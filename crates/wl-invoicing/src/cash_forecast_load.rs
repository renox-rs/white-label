//! What the cash-flow forecast reads (wl-invoicing §5.14, `FIN.CFL.01`): the
//! balances and payments of wl-payments through the kernel, the open invoices
//! and supplier bills, the VAT periods, and the lines of the registered
//! sources, for the legal entities asked for. Nothing is written. The grid
//! itself is [`crate::cash_forecast::assemble`].

use std::collections::{BTreeMap, BTreeSet};

use renox::chrono::{Days, Months, NaiveDate};
use renox::prelude::*;
use wl_kernel::bank_statements::bank_statements;
use wl_kernel::cash_forecast::cash_forecast_sources;
use wl_kernel::cheques::{OpenCheque, cheques as cheque_contract};
use wl_kernel::documents::dubai_date;
use wl_kernel::invoicing::{CreditNoteStatus, InvoiceStatus};
use wl_kernel::parties::{PartyStatus, directory};
use wl_kernel::payments::{Direction, payment_recorder};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::allocation::Allocation;
use crate::bill_line::BillLine;
use crate::budget::{CompanyBudget, CompanyBudgetLine, month_start};
use crate::budget_actual::{self, Mapped};
use crate::budget_category::{BudgetCategory, BudgetKind, MappingKind};
use crate::cash_balance;
use crate::cash_forecast::{
    AccountInput, Balance, BalanceSource, BillInput, ChequeInput, Cover, EntityInput, FillBudget,
    FillCategory, FillInput, Inputs, InvoiceInput, VatInput, average_days_late, choose_balance,
    vat_due, week_one_start, weeks_of,
};
use crate::config::InvoicingConfig;
use crate::credit_note::CreditNote;
use crate::dispute::Dispute;
use crate::drawings;
use crate::duplicates::BillException;
use crate::invoice::Invoice;
use crate::supplier_bill::{POSSIBLE_DUPLICATE, SupplierBill, SupplierBillStatus};
use crate::vat::{aed_rate, to_aed};
use crate::vat_period::{TaxPeriod, containing};
use crate::vat_summary::{VatSummary, VatSummaryStatus};

/// How long after its due date a VAT return is still looked at: a payment
/// that names the summary removes the line, otherwise the forecast warns.
const VAT_LOOK_BACK_DAYS: u64 = 60;

/// The tax periods whose return falls due from `today` less the look-back to
/// `horizon_end`, as `(start, end)`.
pub(crate) fn vat_periods(
    period: TaxPeriod,
    today: NaiveDate,
    horizon_end: NaiveDate,
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut periods = Vec::new();
    let earliest = today - Days::new(VAT_LOOK_BACK_DAYS);
    let (mut start, mut end) = containing(period, today - Days::new(120));
    loop {
        let due = vat_due(end);
        if due > horizon_end {
            break;
        }
        if due >= earliest {
            periods.push((start, end));
        }
        let next = end + Days::new(1);
        (start, end) = containing(period, next);
    }
    periods
}

/// An amount in AED fils: `None` for a foreign amount without a rate.
fn in_aed(amount: i64, currency: &str, rate_micro: Option<i64>) -> Option<i64> {
    aed_rate(currency, rate_micro).map(|rate| to_aed(amount, rate))
}

fn cheque_input(open: &OpenCheque) -> ChequeInput {
    ChequeInput {
        subject: open.cheque.id.subject(),
        label: open.cheque.number.clone(),
        party: open.cheque.party,
        date: open.cheque.date,
        open_amount: open.cheque.open_amount,
        documents: open
            .documents
            .iter()
            .map(|d| (d.document.clone(), d.amount))
            .collect(),
    }
}

/// Reads what the forecast of `entities` needs, as of `today`.
///
/// # Errors
///
/// A database error, or one of a source.
pub async fn load(
    state: &AppState,
    config: &InvoicingConfig,
    entities: &[LegalEntity],
    today: NaiveDate,
) -> Result<Inputs> {
    let weeks = weeks_of(
        today,
        config.cash_week_start,
        config.cash_forecast_weeks as usize,
    );
    let horizon_end = weeks.last().map_or(today, |w| w.end);
    let first = week_one_start(today, config.cash_week_start);
    let mut blocked: BTreeMap<i64, bool> = BTreeMap::new();
    let mapped = Mapped::load(&state.db).await?;
    let mut loaded = Vec::with_capacity(entities.len());
    for entity in entities {
        loaded.push(
            load_entity(
                state,
                config,
                entity,
                today,
                (first, horizon_end),
                &mut blocked,
                &mapped,
            )
            .await?,
        );
    }
    Ok(Inputs {
        today,
        weeks: weeks.len(),
        week_start: config.cash_week_start,
        stale_days: config.cash_stale_days,
        disputed_counted: config.cash_disputed_counted,
        vat_counted: config.cash_vat_counted,
        unapproved_counted: config.cash_unapproved_bills_counted,
        days_late: config.cash_days_late,
        entities: loaded,
    })
}

async fn load_entity(
    state: &AppState,
    config: &InvoicingConfig,
    entity: &LegalEntity,
    today: NaiveDate,
    (first, horizon_end): (NaiveDate, NaiveDate),
    blocked: &mut BTreeMap<i64, bool>,
    mapped: &Mapped,
) -> Result<EntityInput> {
    let db = &state.db;
    let mut input = EntityInput {
        legal_entity_id: entity.id,
        code: entity.code.clone(),
        ..Default::default()
    };

    // Bank accounts and their opening balances.
    let statements = bank_statements(state)
        .latest_closing_balances(db, entity.id)
        .await?;
    let manual = cash_balance::latest(db, entity.id).await?;
    let mut names: BTreeSet<String> = config
        .cash_accounts
        .iter()
        .filter(|(code, _)| *code == entity.code)
        .map(|(_, account)| account.clone())
        .collect();
    names.extend(statements.iter().map(|b| b.bank_account.clone()));
    names.extend(manual.keys().cloned());
    let recorder = payment_recorder(state);
    for account in &names {
        let statement = statements
            .iter()
            .find(|b| &b.bank_account == account)
            .map(|b| Balance {
                amount: b.amount,
                as_of: b.as_of,
                source: BalanceSource::Statement,
            });
        let by_hand = manual.get(account).map(|b| Balance {
            amount: b.amount,
            as_of: b.as_of,
            source: BalanceSource::Manual,
        });
        let balance = choose_balance(statement, by_hand);
        let payments = match balance {
            Some(balance) => {
                recorder
                    .payments_after(db, entity.id, account, balance.as_of)
                    .await?
            }
            None => Vec::new(),
        };
        input.accounts.push(AccountInput {
            account: account.clone(),
            balance,
            payments,
        });
    }

    // Invoices.
    let invoices = Invoice::query()
        .where_(Invoice::LEGAL_ENTITY_ID.eq(entity.id))
        .where_(Invoice::STATUS.is_in([
            InvoiceStatus::Issued,
            InvoiceStatus::PartiallyPaid,
            InvoiceStatus::Overdue,
        ]))
        .where_(Invoice::BALANCE.gt(0))
        .order_by(Invoice::ID)
        .get(db)
        .await?;
    let directory = directory(state);
    for invoice in invoices {
        let subject = invoice.subject();
        let is_blocked = match blocked.get(&invoice.party_id) {
            Some(known) => *known,
            None => {
                let status = directory.status(db, PartyId(invoice.party_id)).await?;
                let value = status == Some(PartyStatus::Blocked);
                blocked.insert(invoice.party_id, value);
                value
            }
        };
        let balance = in_aed(
            invoice.balance,
            &invoice.currency,
            invoice.exchange_rate_micro,
        );
        if balance.is_none() {
            input.unpriced.push(subject.clone());
        }
        let mut disputed = 0;
        if invoice.disputed
            && let Some(id) = invoice.dispute_id
            && let Some(dispute) = Dispute::find(db, id).await?
        {
            disputed = in_aed(
                dispute.amount,
                &invoice.currency,
                invoice.exchange_rate_micro,
            )
            .unwrap_or(0);
        }
        input.invoices.push(InvoiceInput {
            label: invoice
                .number
                .clone()
                .unwrap_or_else(|| subject.to_string()),
            subject,
            party: PartyId(invoice.party_id),
            due: invoice.due_date.or(invoice.issue_date).unwrap_or(today),
            balance,
            disputed,
            blocked: is_blocked,
        });
    }

    // How late each party pays, for the days-late adjustment.
    if config.cash_days_late {
        let parties: BTreeSet<i64> = input.invoices.iter().map(|i| i.party.0).collect();
        input.late_days = late_days(db, &parties, today).await?;
    }

    // Supplier bills.
    let bills = SupplierBill::query()
        .where_(SupplierBill::LEGAL_ENTITY_ID.eq(entity.id))
        .where_(SupplierBill::STATUS.is_in([
            SupplierBillStatus::Approved,
            SupplierBillStatus::PartiallyPaid,
            SupplierBillStatus::Matched,
            SupplierBillStatus::Exception,
        ]))
        .where_(SupplierBill::BALANCE.gt(0))
        .order_by(SupplierBill::ID)
        .get(db)
        .await?;
    let bill_ids: Vec<i64> = bills.iter().map(|b| b.id).collect();
    let mut duplicates: BTreeSet<i64> = BTreeSet::new();
    let mut drawing_bills: BTreeSet<i64> = BTreeSet::new();
    if !bill_ids.is_empty() {
        duplicates = BillException::query()
            .where_(BillException::BILL_ID.is_in(bill_ids.clone()))
            .where_(BillException::REASON_CODE.eq(POSSIBLE_DUPLICATE))
            .get(db)
            .await?
            .into_iter()
            .map(|e| e.bill_id)
            .collect();
        drawing_bills = BillLine::query()
            .where_(BillLine::BILL_ID.is_in(bill_ids))
            .get(db)
            .await?
            .into_iter()
            .filter(|l| mapped.is_drawings_code(l.cost_code.as_deref()))
            .map(|l| l.bill_id)
            .collect();
    }
    for bill in bills {
        let subject = bill.subject();
        let balance = in_aed(bill.balance, &bill.currency, bill.exchange_rate_micro);
        if balance.is_none() {
            input.unpriced.push(subject.clone());
        }
        input.bills.push(BillInput {
            label: bill.supplier_invoice_number.clone(),
            party: PartyId(bill.party_id),
            due: bill.due_date.unwrap_or(bill.bill_date),
            balance,
            approved: matches!(
                bill.status,
                SupplierBillStatus::Approved | SupplierBillStatus::PartiallyPaid
            ),
            duplicate: bill.status == SupplierBillStatus::Exception
                && duplicates.contains(&bill.id),
            drawings: drawing_bills.contains(&bill.id),
            subject,
        });
    }

    // Cheques and refunds, from wl-payments.
    let book = cheque_contract(state);
    input.cheques_in = book
        .not_cleared(state, entity.id, Direction::In)
        .await?
        .iter()
        .map(cheque_input)
        .collect();
    input.cheques_out = book
        .not_cleared(state, entity.id, Direction::Out)
        .await?
        .iter()
        .map(cheque_input)
        .collect();
    input.bounced = book
        .bounced(state, entity.id)
        .await?
        .iter()
        .map(cheque_input)
        .collect();
    input.refunds = recorder.refunds_unpaid(db, entity.id).await?;

    // VAT at the return due date.
    if config.cash_vat_counted {
        input.vat = load_vat(state, config, entity, today, horizon_end, &names).await?;
    }

    // The lines other modules add.
    for (name, source) in cash_forecast_sources() {
        for line in source.lines(state, entity.id, first, horizon_end).await? {
            input.sources.push((name.clone(), line));
        }
    }

    // The approved budget of each month of the horizon, and what covers it.
    load_fill(db, entity.id, (first, horizon_end), mapped, &mut input).await?;
    Ok(input)
}

/// The days-late average of each of `parties` (their ids): over the invoices
/// fully paid (`paid`, never written off or credited) in the 12 months before
/// `today`, the days from the due date to the day of the last allocation,
/// the one that brought the balance to 0. A party with fewer than three has
/// no entry. Payer bills (`FIN.TPP.01`) join through the same average when
/// their module exists.
async fn late_days(
    db: &Db,
    parties: &BTreeSet<i64>,
    today: NaiveDate,
) -> Result<BTreeMap<i64, i64>> {
    let mut found = BTreeMap::new();
    if parties.is_empty() {
        return Ok(found);
    }
    let since = today
        .checked_sub_months(Months::new(12))
        .unwrap_or(NaiveDate::MIN);
    let paid = Invoice::query()
        .where_(Invoice::PARTY_ID.is_in(parties.iter().copied().collect::<Vec<_>>()))
        .where_(Invoice::STATUS.eq(InvoiceStatus::Paid))
        .get(db)
        .await?;
    if paid.is_empty() {
        return Ok(found);
    }
    let allocations = Allocation::query()
        .where_(Allocation::INVOICE_ID.is_in(paid.iter().map(|i| i.id).collect::<Vec<_>>()))
        .where_(Allocation::REMOVED_AT.is_null())
        .get(db)
        .await?;
    let mut samples: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for invoice in &paid {
        let Some(due) = invoice.due_date else {
            continue;
        };
        let Some(last) = allocations
            .iter()
            .filter(|a| a.invoice_id == invoice.id)
            .map(|a| a.allocated_at)
            .max()
        else {
            continue;
        };
        let on = dubai_date(last);
        if on < since || on > today {
            continue;
        }
        samples
            .entry(invoice.party_id)
            .or_default()
            .push((on - due).num_days());
    }
    for (party, days) in samples {
        if let Some(average) = average_days_late(&days) {
            found.insert(party, average);
        }
    }
    Ok(found)
}

/// The category a line of a source counts in: the category with the line's
/// category as its code, else the one that maps the line's category as a key
/// (payroll, expense category or cost code for a payment; charge type or item
/// group for a receipt), of a kind that fits the direction.
fn source_category(
    mapped: &Mapped,
    categories: &[BudgetCategory],
    category: &str,
    income: bool,
) -> Option<i64> {
    if let Some(found) = categories
        .iter()
        .find(|c| c.code == category && (c.kind == BudgetKind::Income) == income)
    {
        return Some(found.id);
    }
    let kinds: &[MappingKind] = if income {
        &[MappingKind::ChargeType, MappingKind::ItemGroup]
    } else {
        &[
            MappingKind::Payroll,
            MappingKind::ExpenseCategory,
            MappingKind::CostCode,
        ]
    };
    kinds.iter().find_map(|kind| {
        mapped
            .find(*kind, Some(category))
            .filter(|(_, budget_kind)| (*budget_kind == BudgetKind::Income) == income)
            .map(|(id, _)| id)
    })
}

/// Reads what the budget fill needs (§5.14 step 7): the lines of the approved
/// budget of each month of the horizon (a month with none goes to
/// `months_without_budget`), the actual of those months, the supplier bills
/// not yet approved with the category and month their lines will count in,
/// and the category of each source line.
async fn load_fill(
    db: &Db,
    entity: i64,
    (first, horizon_end): (NaiveDate, NaiveDate),
    mapped: &Mapped,
    input: &mut EntityInput,
) -> Result<()> {
    let mut months = Vec::new();
    let mut month = month_start(first);
    while month <= horizon_end {
        months.push(month);
        month = month_start(month + Days::new(32));
    }
    let categories = BudgetCategory::query().get(db).await?;
    let mut fill = FillInput {
        categories: categories
            .iter()
            .map(|c| FillCategory {
                id: c.id,
                code: c.code.clone(),
                name: c.name_en.clone(),
                kind: c.kind,
            })
            .collect(),
        ..Default::default()
    };
    let mut lines: BTreeMap<i64, Vec<CompanyBudgetLine>> = BTreeMap::new();
    let mut budgeted = Vec::new();
    for month in &months {
        let Some(budget) = CompanyBudget::approved_on(db, entity, *month).await? else {
            input.months_without_budget.push(*month);
            continue;
        };
        budgeted.push(*month);
        if let std::collections::btree_map::Entry::Vacant(slot) = lines.entry(budget.id) {
            slot.insert(CompanyBudgetLine::of(db, budget.id).await?);
        }
        for line in lines.get(&budget.id).into_iter().flatten() {
            if line.month == *month {
                fill.budget.push(FillBudget {
                    category_id: line.category_id,
                    month: *month,
                    amount: line.amount,
                });
            }
        }
    }
    if !budgeted.is_empty() {
        fill.actual = budget_actual::per_category_month(db, entity, &budgeted).await?;
    }
    // The bills not yet approved that the forecast counts, by line.
    let pending: Vec<SupplierBill> = SupplierBill::query()
        .where_(SupplierBill::LEGAL_ENTITY_ID.eq(entity))
        .where_(
            SupplierBill::STATUS
                .is_in([SupplierBillStatus::Matched, SupplierBillStatus::Exception]),
        )
        .get(db)
        .await?
        .into_iter()
        .filter(|b| {
            !input
                .bills
                .iter()
                .any(|i| i.subject == b.subject() && i.duplicate)
        })
        .collect();
    for item in budget_actual::bill_items(db, mapped, &pending).await? {
        if let Some(category) = item.category_id {
            fill.bill_covers.push(Cover {
                document: item.source,
                category_id: category,
                month: month_start(item.month),
                amount: item.amount,
            });
        }
    }
    for (_, line) in &input.sources {
        let income = line.direction == Direction::In;
        if let Some(category) = source_category(mapped, &categories, &line.category, income) {
            fill.source_categories
                .insert((line.category.clone(), income), category);
        }
    }
    input.fill = fill;
    Ok(())
}

/// The VAT payable of the tax periods whose return is due in the horizon.
async fn load_vat(
    state: &AppState,
    config: &InvoicingConfig,
    entity: &LegalEntity,
    today: NaiveDate,
    horizon_end: NaiveDate,
    accounts: &BTreeSet<String>,
) -> Result<Vec<VatInput>> {
    let db = &state.db;
    let mut found = Vec::new();
    for (start, end) in vat_periods(config.tax_period_of(&entity.code), today, horizon_end) {
        let summary = VatSummary::for_period(db, entity.id, start).await?;
        let subject = summary
            .as_ref()
            .map(|s| Subject::new("invoicing.vat_summary", s.id));
        let amount = payable(db, entity.id, start, end).await?;
        let mut paid = false;
        if let Some(subject) = &subject {
            let reference = subject.to_string();
            for account in accounts {
                let payments = payment_recorder(state)
                    .payments_after(db, entity.id, account, end)
                    .await?;
                if payments.iter().any(|p| {
                    p.payment.direction == Direction::Out
                        && p.reference.as_deref().map(str::trim) == Some(reference.as_str())
                }) {
                    paid = true;
                }
            }
        }
        if amount <= 0 && !paid {
            // A net refundable is not a receipt.
            continue;
        }
        found.push(VatInput {
            estimate: summary
                .as_ref()
                .is_none_or(|s| s.status == VatSummaryStatus::Open),
            summary: subject,
            period_start: start,
            period_end: end,
            due: vat_due(end),
            amount,
            paid,
        });
    }
    Ok(found)
}

/// The net VAT payable of the period from the documents: the VAT of the
/// invoices less credit notes, less the input VAT of the approved supplier
/// bills that is recovered (the VAT on owner drawings is not), in AED fils.
pub async fn payable(db: &Db, entity: i64, start: NaiveDate, end: NaiveDate) -> Result<i64> {
    let invoices = Invoice::query()
        .where_(Invoice::LEGAL_ENTITY_ID.eq(entity))
        .where_(Invoice::STATUS.ne(InvoiceStatus::Draft))
        .get(db)
        .await?;
    let output: i64 = invoices
        .iter()
        .filter(|i| {
            i.date_of_supply
                .or(i.issue_date)
                .is_some_and(|d| d >= start && d <= end)
        })
        .map(|i| {
            i.vat_total_aed.unwrap_or_else(|| {
                in_aed(i.vat_total, &i.currency, i.exchange_rate_micro).unwrap_or(0)
            })
        })
        .sum();
    let notes = CreditNote::query()
        .where_(CreditNote::LEGAL_ENTITY_ID.eq(entity))
        .where_(CreditNote::STATUS.ne(CreditNoteStatus::Draft))
        .get(db)
        .await?;
    let credited: i64 = notes
        .iter()
        .filter(|n| n.issue_date.is_some_and(|d| d >= start && d <= end))
        .map(|n| n.vat_total_aed.unwrap_or(0))
        .sum();
    let bills = SupplierBill::query()
        .where_(SupplierBill::LEGAL_ENTITY_ID.eq(entity))
        .where_(SupplierBill::STATUS.is_in([
            SupplierBillStatus::Approved,
            SupplierBillStatus::PartiallyPaid,
            SupplierBillStatus::Paid,
        ]))
        .where_(SupplierBill::BILL_DATE.gte(start))
        .where_(SupplierBill::BILL_DATE.lte(end))
        .get(db)
        .await?;
    let input: i64 = bills
        .iter()
        .map(|b| in_aed(b.vat_total, &b.currency, b.exchange_rate_micro).unwrap_or(0))
        .sum();
    let blocked = drawings::blocked_input_vat(db, entity, start, end).await?;
    Ok(output - credited - (input - blocked))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_periods_are_those_whose_return_falls_in_the_window() {
        // Quarterly on calendar quarters; today 2027-01-13, 13 weeks ahead.
        let periods = vat_periods(TaxPeriod::default(), day(2027, 1, 13), day(2027, 4, 11));
        // Q4 2026 is due 2027-01-28; Q1 2027 is due 2027-04-28, beyond the
        // horizon; Q3 2026 was due 2026-10-28, more than 60 days ago.
        assert_eq!(periods, vec![(day(2026, 10, 1), day(2026, 12, 31))]);
    }

    #[test]
    fn a_return_just_past_due_is_still_looked_at() {
        let periods = vat_periods(TaxPeriod::default(), day(2026, 11, 20), day(2027, 2, 20));
        assert_eq!(
            periods,
            vec![
                (day(2026, 7, 1), day(2026, 9, 30)),
                (day(2026, 10, 1), day(2026, 12, 31)),
            ]
        );
    }

    #[test]
    fn monthly_periods_follow_each_other() {
        let periods = vat_periods(TaxPeriod::Monthly, day(2027, 1, 13), day(2027, 3, 31));
        assert!(!periods.is_empty());
        assert!(periods.windows(2).all(|w| w[1].0 == w[0].1 + Days::new(1)));
    }

    #[test]
    fn a_foreign_amount_needs_a_rate() {
        assert_eq!(in_aed(1_000, "AED", None), Some(1_000));
        assert_eq!(in_aed(1_000, "USD", None), None);
        assert_eq!(in_aed(1_000, "USD", Some(3_672_500)), Some(3_673));
    }
}
