//! The actual of the company budget (wl-invoicing §5.15 step 2, `FIN.BUD.01`):
//! per legal entity, budget category and month, in AED without recoverable
//! VAT.
//!
//! - **Income:** the lines of issued invoices (every invoice but a draft) by
//!   date of supply (the issue date when there is none), less the lines of
//!   issued credit notes by the credit note's date, mapped through the
//!   charge type of the invoice line.
//! - **Expense:** the lines of approved, partly paid and paid supplier bills
//!   by bill date, less the lines of issued and settled supplier debit notes
//!   by the note's date (in the category of the bill line they reduce),
//!   mapped through the cost code of the bill line.
//! - **Owner drawings:** supplier bill lines coded to a category of kind
//!   `owner_drawings`, with their VAT, which is not recovered (§10, the
//!   default until the accountant answers question 14, `⚠ open: accountant
//!   (A-16)`). They are never counted in expense.
//!
//! A line whose key finds no category is *Uncategorised*, so the totals
//! always agree with the documents. The actual is computed when asked, with
//! the mapping of the day. Foreign amounts are taken at the rate stored on
//! the document; a line whose rate is unknown is left out. The lines of
//! approved expense claims (wl-payments, by expense date, mapped through the
//! expense category) and the employer cost of paid payroll runs (wl-hr, by
//! period, key `payroll`) are read through the kernel's actual cost sources:
//! wl-payments registers the claims; wl-hr has no payroll runs in this build,
//! so the payroll key is stored and counted once a source provides it.
//! The payments with purpose *owner drawings* (wl-payments) are read through
//! the kernel's drawing payment sources: a drawing adds its amount, a repayment
//! by the owner takes it off, in the month of the payment date.

use std::collections::{BTreeMap, BTreeSet};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::budgets::actual_cost_sources;
use wl_kernel::drawings::drawing_payment_sources;
use wl_kernel::invoicing::{CreditNoteStatus, InvoiceStatus, SupplierDebitNoteStatus};
use wl_kernel::payments::Direction;

use crate::bill_line::BillLine;
use crate::budget::{month_end, month_start};
use crate::budget_category::{BudgetCategory, BudgetKind, BudgetMapping, MappingKind};
use crate::credit_note::CreditNote;
use crate::credit_note_line::CreditNoteLine;
use crate::debit_note_line::DebitNoteLine;
use crate::drawings_machine::{DrawingApproval, DrawingStatus};
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};
use crate::supplier_debit_note::SupplierDebitNote;
use crate::texts::english;
use crate::vat;

/// The supplier bill states whose lines count as actual expense.
const COUNTED_BILLS: [SupplierBillStatus; 3] = [
    SupplierBillStatus::Approved,
    SupplierBillStatus::PartiallyPaid,
    SupplierBillStatus::Paid,
];

/// One amount of actual.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ActualItem {
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The first day of the month the amount belongs to.
    pub month: NaiveDate,
    /// Income, expense or owner drawings; an uncategorised item is income or
    /// expense.
    pub kind: BudgetKind,
    /// The budget category; `None`: *Uncategorised*.
    pub category_id: Option<i64>,
    /// The amount in AED fils, negative for a credit note or a debit note.
    pub amount: i64,
    /// What it is: the line's description.
    pub label: String,
    /// The document the amount comes from.
    pub source: Subject,
    /// The supplier of a bill line, if any (the payee of a drawing).
    pub party_id: Option<i64>,
    /// Whether the drawing was recorded without a second approver
    /// (`FIN.ACL.23`).
    pub self_approved: bool,
}

/// What the keys of the mapping say, loaded once per computation.
#[derive(Debug, Clone, Default)]
pub struct Mapped {
    keys: BTreeMap<(MappingKind, String), i64>,
    kinds: BTreeMap<i64, BudgetKind>,
}

impl Mapped {
    /// The categories and their keys, as they are now.
    pub async fn load(db: &Db) -> Result<Mapped> {
        let kinds = BudgetCategory::query()
            .get(db)
            .await?
            .into_iter()
            .map(|c| (c.id, c.kind))
            .collect();
        let keys = BudgetMapping::query()
            .get(db)
            .await?
            .into_iter()
            .map(|m| ((m.key_kind, m.key_value), m.category_id))
            .collect();
        Ok(Mapped { keys, kinds })
    }

    /// [`Mapped::load`] inside the transaction `tx`.
    pub async fn load_in(tx: &mut renox::db::Transaction) -> Result<Mapped> {
        let kinds = BudgetCategory::query()
            .get(&mut *tx)
            .await?
            .into_iter()
            .map(|c| (c.id, c.kind))
            .collect();
        let keys = BudgetMapping::query()
            .get(&mut *tx)
            .await?
            .into_iter()
            .map(|m| ((m.key_kind, m.key_value), m.category_id))
            .collect();
        Ok(Mapped { keys, kinds })
    }

    /// Adds a key, for tests.
    #[cfg(test)]
    pub(crate) fn insert_for_test(
        &mut self,
        kind: MappingKind,
        key: &str,
        id: i64,
        budget_kind: BudgetKind,
    ) {
        self.keys.insert((kind, key.to_owned()), id);
        self.kinds.insert(id, budget_kind);
    }

    /// Whether the cost code `code` is collected by a category of kind
    /// `owner_drawings`.
    pub fn is_drawings_code(&self, code: Option<&str>) -> bool {
        self.find(MappingKind::CostCode, code)
            .is_some_and(|(_, kind)| kind == BudgetKind::OwnerDrawings)
    }

    /// The category that collects `value` of `kind`, with its budget kind.
    pub fn find(&self, kind: MappingKind, value: Option<&str>) -> Option<(i64, BudgetKind)> {
        let value = crate::budget_category::normalise_key(value?);
        let id = *self.keys.get(&(kind, value))?;
        Some((id, *self.kinds.get(&id)?))
    }
}

/// The amount of `net` fils at `rate_micro` to AED; `None` while the rate is
/// unknown.
fn aed(net: i64, rate_micro: Option<i64>) -> Option<i64> {
    rate_micro.map(|rate| vat::to_aed(net, rate))
}

/// The items of the lines of issued invoices and credit notes of `entities`
/// dated from `from` to `to`.
async fn income_items(
    db: &Db,
    mapped: &Mapped,
    entities: &[i64],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<ActualItem>> {
    let mut items = Vec::new();
    let invoices: Vec<Invoice> = Invoice::query()
        .where_(Invoice::LEGAL_ENTITY_ID.is_in(entities.to_vec()))
        .where_(Invoice::STATUS.ne(InvoiceStatus::Draft))
        .get(db)
        .await?
        .into_iter()
        .filter(|i| {
            i.date_of_supply
                .or(i.issue_date)
                .is_some_and(|d| d >= from && d <= to)
        })
        .collect();
    if !invoices.is_empty() {
        let ids: Vec<i64> = invoices.iter().map(|i| i.id).collect();
        let lines = InvoiceLine::query()
            .where_(InvoiceLine::INVOICE_ID.is_in(ids))
            .order_by(InvoiceLine::ID)
            .get(db)
            .await?;
        for line in &lines {
            let Some(invoice) = invoices.iter().find(|i| i.id == line.invoice_id) else {
                continue;
            };
            let Some(amount) = aed(line.net_amount, invoice.aed_rate()) else {
                continue;
            };
            let date = invoice
                .date_of_supply
                .or(invoice.issue_date)
                .unwrap_or(from);
            let category = mapped
                .find(MappingKind::ChargeType, line.charge_type.as_deref())
                .filter(|(_, kind)| *kind == BudgetKind::Income);
            items.push(ActualItem {
                legal_entity_id: invoice.legal_entity_id.unwrap_or_default(),
                month: month_start(date),
                kind: BudgetKind::Income,
                category_id: category.map(|(id, _)| id),
                amount,
                label: line.description.clone(),
                source: invoice.subject(),
                party_id: Some(invoice.party_id),
                self_approved: false,
            });
        }
    }
    let notes: Vec<CreditNote> = CreditNote::query()
        .where_(CreditNote::LEGAL_ENTITY_ID.is_in(entities.to_vec()))
        .where_(CreditNote::STATUS.is_in([CreditNoteStatus::Issued, CreditNoteStatus::Applied]))
        .where_(CreditNote::ISSUE_DATE.gte(from))
        .where_(CreditNote::ISSUE_DATE.lte(to))
        .get(db)
        .await?;
    if !notes.is_empty() {
        let ids: Vec<i64> = notes.iter().map(|n| n.id).collect();
        let note_lines = CreditNoteLine::query()
            .where_(CreditNoteLine::CREDIT_NOTE_ID.is_in(ids))
            .order_by(CreditNoteLine::ID)
            .get(db)
            .await?;
        let invoice_line_ids: Vec<i64> = note_lines.iter().map(|l| l.invoice_line_id).collect();
        let invoice_lines: BTreeMap<i64, InvoiceLine> = InvoiceLine::query()
            .where_(InvoiceLine::ID.is_in(invoice_line_ids))
            .get(db)
            .await?
            .into_iter()
            .map(|l| (l.id, l))
            .collect();
        for line in &note_lines {
            let Some(note) = notes.iter().find(|n| n.id == line.credit_note_id) else {
                continue;
            };
            let Some(amount) = aed(line.net_amount, note.aed_rate()) else {
                continue;
            };
            let charge_type = invoice_lines
                .get(&line.invoice_line_id)
                .and_then(|l| l.charge_type.as_deref());
            let category = mapped
                .find(MappingKind::ChargeType, charge_type)
                .filter(|(_, kind)| *kind == BudgetKind::Income);
            items.push(ActualItem {
                legal_entity_id: note.legal_entity_id.unwrap_or_default(),
                month: month_start(note.issue_date.unwrap_or(from)),
                kind: BudgetKind::Income,
                category_id: category.map(|(id, _)| id),
                amount: -amount,
                label: line.description.clone(),
                source: note.subject(),
                party_id: Some(note.party_id),
                self_approved: false,
            });
        }
    }
    Ok(items)
}

/// One expense item for `line` worth `net` (and `vat`, which an owner
/// drawing keeps): in the category its cost code finds, else
/// *Uncategorised*. The entity, month, document and supplier are set by the
/// caller; `sign` is -1 for a debit note.
fn expense_item(mapped: &Mapped, line: &BillLine, amounts: (i64, i64), sign: i64) -> ActualItem {
    let (net, vat) = amounts;
    let category = mapped
        .find(MappingKind::CostCode, line.cost_code.as_deref())
        .filter(|(_, kind)| *kind != BudgetKind::Income);
    let kind = category.map_or(BudgetKind::Expense, |(_, kind)| kind);
    // The VAT of an owner drawing is not recovered: it stays in the amount.
    let amount = if kind == BudgetKind::OwnerDrawings {
        net + vat
    } else {
        net
    };
    ActualItem {
        legal_entity_id: 0,
        month: NaiveDate::MIN,
        kind,
        category_id: category.map(|(id, _)| id),
        amount: sign * amount,
        label: line.description.clone(),
        source: Subject::new("invoicing.supplier_bill", 0),
        party_id: None,
        self_approved: false,
    }
}

/// The items of the lines of `bills`, by bill date, whatever their state: the
/// caller chooses which bills count. The cash-flow forecast reads the bills
/// not yet approved through it, to know what they will add to the actual.
pub async fn bill_items(
    db: &Db,
    mapped: &Mapped,
    bills: &[SupplierBill],
) -> Result<Vec<ActualItem>> {
    let mut items = Vec::new();
    if bills.is_empty() {
        return Ok(items);
    }
    let ids: Vec<i64> = bills.iter().map(|b| b.id).collect();
    // The bills whose drawing the sole owner approved themselves
    // (`FIN.ACL.23`).
    let self_approved: BTreeSet<i64> = DrawingApproval::query()
        .where_(DrawingApproval::SOURCE_TYPE.eq("invoicing.supplier_bill"))
        .where_(DrawingApproval::SOURCE_ID.is_in(ids.clone()))
        .where_(DrawingApproval::STATUS.eq(DrawingStatus::Approved))
        .where_(DrawingApproval::SELF_APPROVED.eq(true))
        .get(db)
        .await?
        .into_iter()
        .map(|a| a.source_id)
        .collect();
    let lines = BillLine::query()
        .where_(BillLine::BILL_ID.is_in(ids))
        .order_by(BillLine::ID)
        .get(db)
        .await?;
    for line in &lines {
        let Some(bill) = bills.iter().find(|b| b.id == line.bill_id) else {
            continue;
        };
        let Some(net) = line.net_amount_aed else {
            continue;
        };
        let mut item = ActualItem {
            legal_entity_id: bill.legal_entity_id.unwrap_or_default(),
            month: month_start(bill.bill_date),
            source: bill.subject(),
            party_id: Some(bill.party_id),
            ..expense_item(mapped, line, (net, line.vat_amount_aed.unwrap_or(0)), 1)
        };
        item.self_approved =
            item.kind == BudgetKind::OwnerDrawings && self_approved.contains(&bill.id);
        items.push(item);
    }
    Ok(items)
}

/// The items of the lines of approved supplier bills and issued debit notes
/// of `entities` dated from `from` to `to`.
async fn expense_items(
    db: &Db,
    mapped: &Mapped,
    entities: &[i64],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<ActualItem>> {
    let mut items = Vec::new();
    let bills = SupplierBill::query()
        .where_(SupplierBill::LEGAL_ENTITY_ID.is_in(entities.to_vec()))
        .where_(SupplierBill::STATUS.is_in(COUNTED_BILLS))
        .where_(SupplierBill::BILL_DATE.gte(from))
        .where_(SupplierBill::BILL_DATE.lte(to))
        .get(db)
        .await?;
    items.extend(bill_items(db, mapped, &bills).await?);
    let notes: Vec<SupplierDebitNote> = SupplierDebitNote::query()
        .where_(SupplierDebitNote::LEGAL_ENTITY_ID.is_in(entities.to_vec()))
        .where_(SupplierDebitNote::STATUS.is_in([
            SupplierDebitNoteStatus::Issued,
            SupplierDebitNoteStatus::Settled,
        ]))
        .where_(SupplierDebitNote::ISSUE_DATE.gte(from))
        .where_(SupplierDebitNote::ISSUE_DATE.lte(to))
        .get(db)
        .await?;
    if !notes.is_empty() {
        let ids: Vec<i64> = notes.iter().map(|n| n.id).collect();
        let note_lines = DebitNoteLine::query()
            .where_(DebitNoteLine::DEBIT_NOTE_ID.is_in(ids))
            .order_by(DebitNoteLine::ID)
            .get(db)
            .await?;
        let bill_line_ids: Vec<i64> = note_lines.iter().filter_map(|l| l.bill_line_id).collect();
        let bill_lines: BTreeMap<i64, BillLine> = BillLine::query()
            .where_(BillLine::ID.is_in(bill_line_ids))
            .get(db)
            .await?
            .into_iter()
            .map(|l| (l.id, l))
            .collect();
        for line in &note_lines {
            let (Some(note), Some(bill_line)) = (
                notes.iter().find(|n| n.id == line.debit_note_id),
                line.bill_line_id.and_then(|id| bill_lines.get(&id)),
            ) else {
                continue;
            };
            let Some(net) = line.net_amount_aed else {
                continue;
            };
            items.push(ActualItem {
                legal_entity_id: note.legal_entity_id,
                month: month_start(note.issue_date.unwrap_or(from)),
                label: line.description.clone(),
                source: note.subject(),
                party_id: Some(note.party_id),
                ..expense_item(
                    mapped,
                    bill_line,
                    (net, line.vat_amount_aed.unwrap_or(0)),
                    -1,
                )
            });
        }
    }
    Ok(items)
}

/// Every item of actual of `entities` dated from `from` to `to`.
pub async fn items(
    db: &Db,
    entities: &[i64],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<ActualItem>> {
    if entities.is_empty() || to < from {
        return Ok(Vec::new());
    }
    let mapped = Mapped::load(db).await?;
    let mut all = income_items(db, &mapped, entities, from, to).await?;
    all.extend(expense_items(db, &mapped, entities, from, to).await?);
    all.extend(drawing_payment_items(db, entities, from, to).await?);
    all.extend(other_module_items(db, &mapped, entities, from, to).await?);
    Ok(all)
}

/// The items of the amounts other modules own (the employer cost of paid
/// payroll runs of wl-hr, approved expense claim lines of wl-payments), read
/// through the kernel's
/// [`ActualCostSource`](wl_kernel::budgets::ActualCostSource)s and mapped
/// through their key (`payroll` or `expense_category`).
async fn other_module_items(
    db: &Db,
    mapped: &Mapped,
    entities: &[i64],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<ActualItem>> {
    let mut items = Vec::new();
    for source in actual_cost_sources() {
        for cost in source.costs(db, entities, from, to).await? {
            let kind = match cost.key_kind.as_str() {
                "payroll" => MappingKind::Payroll,
                _ => MappingKind::ExpenseCategory,
            };
            let found = mapped
                .find(kind, Some(&cost.key))
                .filter(|(_, kind)| *kind != BudgetKind::Income);
            items.push(ActualItem {
                legal_entity_id: cost.legal_entity_id,
                month: month_start(cost.date),
                kind: found.map_or(BudgetKind::Expense, |(_, k)| k),
                category_id: found.map(|(id, _)| id),
                amount: cost.amount,
                label: cost.label,
                source: cost.source,
                party_id: None,
                self_approved: false,
            });
        }
    }
    Ok(items)
}

/// The items of the payments with purpose *owner drawings* (wl-payments,
/// `FIN.ACL.21`) dated from `from` to `to`, read through the kernel's
/// [`DrawingPaymentSource`](wl_kernel::drawings::DrawingPaymentSource)s: a
/// drawing adds its amount (the payment has no VAT of its own), a repayment
/// by the owner takes it off, both in their category and the month of the
/// payment date.
async fn drawing_payment_items(
    db: &Db,
    entities: &[i64],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<ActualItem>> {
    let sources = drawing_payment_sources();
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let categories: BTreeMap<String, i64> = BudgetCategory::query()
        .get(db)
        .await?
        .into_iter()
        .map(|c| (c.code, c.id))
        .collect();
    let mut items = Vec::new();
    for source in sources {
        for payment in source.payments(db, entities, from, to).await? {
            let repaid = payment.direction == Direction::In;
            items.push(ActualItem {
                legal_entity_id: payment.legal_entity_id,
                month: month_start(payment.date),
                kind: BudgetKind::OwnerDrawings,
                category_id: categories.get(&payment.category).copied(),
                amount: if repaid {
                    -payment.amount
                } else {
                    payment.amount
                },
                label: english(if repaid {
                    "wl_invoicing.drawings.repayment"
                } else {
                    "wl_invoicing.drawings.payment"
                })
                .to_owned(),
                source: payment.payment.subject(),
                party_id: Some(payment.party.0),
                self_approved: payment.self_approved,
            });
        }
    }
    Ok(items)
}

/// The actual of category `category` for `entity` in the month of `month`.
pub async fn category_actual(db: &Db, entity: i64, category: i64, month: NaiveDate) -> Result<i64> {
    let found = items(db, &[entity], month_start(month), month_end(month)).await?;
    Ok(found
        .iter()
        .filter(|i| i.category_id == Some(category))
        .map(|i| i.amount)
        .sum())
}

/// The actual per category and month of `entity` over the months of
/// `months`, for the categories that are active now: a category with no
/// actual in a month has no entry, and an uncategorised amount none.
pub async fn per_category_month(
    db: &Db,
    entity: i64,
    months: &[NaiveDate],
) -> Result<BTreeMap<(i64, NaiveDate), i64>> {
    let (Some(first), Some(last)) = (months.iter().min(), months.iter().max()) else {
        return Ok(BTreeMap::new());
    };
    let wanted: BTreeSet<NaiveDate> = months.iter().copied().map(month_start).collect();
    let found = items(db, &[entity], month_start(*first), month_end(*last)).await?;
    let mut sums: BTreeMap<(i64, NaiveDate), i64> = BTreeMap::new();
    for item in found {
        if let (Some(category), true) = (item.category_id, wanted.contains(&item.month)) {
            *sums.entry((category, item.month)).or_insert(0) += item.amount;
        }
    }
    Ok(sums)
}

/// Whether category `id` has an actual: an invoice line with one of its
/// charge types or a bill line with one of its cost codes exists.
pub async fn category_has_actual(tx: &mut renox::db::Transaction, id: i64) -> Result<bool> {
    let keys = BudgetMapping::query()
        .where_(BudgetMapping::CATEGORY_ID.eq(id))
        .get(&mut *tx)
        .await?;
    let values = |kind: MappingKind| -> Vec<String> {
        keys.iter()
            .filter(|m| m.key_kind == kind)
            .map(|m| m.key_value.clone())
            .collect()
    };
    let charge_types = values(MappingKind::ChargeType);
    if !charge_types.is_empty()
        && InvoiceLine::query()
            .where_in("charge_type", charge_types)
            .exists(&mut *tx)
            .await?
    {
        return Ok(true);
    }
    let cost_codes = values(MappingKind::CostCode);
    Ok(!cost_codes.is_empty()
        && BillLine::query()
            .where_in("cost_code", cost_codes)
            .exists(&mut *tx)
            .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped() -> Mapped {
        let mut mapped = Mapped::default();
        mapped
            .keys
            .insert((MappingKind::CostCode, "marketing".into()), 1);
        mapped
            .keys
            .insert((MappingKind::CostCode, "owner_drawings".into()), 2);
        mapped.kinds.insert(1, BudgetKind::Expense);
        mapped.kinds.insert(2, BudgetKind::OwnerDrawings);
        mapped
    }

    fn line(code: Option<&str>) -> BillLine {
        BillLine {
            description: "Flyers".into(),
            cost_code: code.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn a_key_finds_its_category_ignoring_case_and_spaces() {
        let mapped = mapped();
        assert_eq!(
            mapped.find(MappingKind::CostCode, Some(" Marketing ")),
            Some((1, BudgetKind::Expense))
        );
        assert_eq!(mapped.find(MappingKind::CostCode, Some("fuel")), None);
        assert_eq!(mapped.find(MappingKind::CostCode, None), None);
        assert_eq!(
            mapped.find(MappingKind::ChargeType, Some("marketing")),
            None
        );
    }

    #[test]
    fn a_bill_line_is_expense_without_its_recoverable_vat() {
        let item = expense_item(&mapped(), &line(Some("marketing")), (600_000, 30_000), 1);
        assert_eq!(item.kind, BudgetKind::Expense);
        assert_eq!(item.category_id, Some(1));
        assert_eq!(item.amount, 600_000);
    }

    #[test]
    fn a_drawings_line_keeps_its_vat_and_is_not_expense() {
        let item = expense_item(
            &mapped(),
            &line(Some("owner_drawings")),
            (300_000, 15_000),
            1,
        );
        assert_eq!(item.kind, BudgetKind::OwnerDrawings);
        assert_eq!(item.amount, 315_000);
    }

    #[test]
    fn a_line_without_a_category_is_uncategorised_expense() {
        for code in [None, Some("unmapped")] {
            let item = expense_item(&mapped(), &line(code), (50_000, 2_500), 1);
            assert_eq!((item.kind, item.category_id), (BudgetKind::Expense, None));
            assert_eq!(item.amount, 50_000);
        }
    }

    #[test]
    fn a_debit_note_takes_the_amount_off() {
        let item = expense_item(&mapped(), &line(Some("marketing")), (100_000, 5_000), -1);
        assert_eq!(item.amount, -100_000);
    }

    #[test]
    fn an_unknown_rate_gives_no_amount() {
        assert_eq!(aed(1_000, None), None);
        assert_eq!(aed(1_000, Some(1_000_000)), Some(1_000));
        assert_eq!(aed(1_000, Some(3_672_500)), Some(3_673));
    }
}
