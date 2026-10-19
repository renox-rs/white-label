//! The cash-flow forecast (wl-invoicing §3, §5.14, `FIN.CFL.01`): a read-only
//! projection of the cash position by week, computed when it is opened and
//! never stored.
//!
//! [`assemble`] turns what [`crate::cash_forecast_load`] read into the grid:
//!
//! - **Opening balance** per bank account: the later of the latest statement
//!   balance and the latest manual balance (the statement wins on the same
//!   day), then the payments of the account dated after that day, in and
//!   out. An account with neither shows a warning and counts as zero; one
//!   older than the stale limit is used with a warning.
//! - **Receipts:** issued invoices by due date for their balance less the
//!   part covered by a linked cheque, the disputed part and the part covered
//!   by a bounced cheque; received cheques not cleared, by cheque date for
//!   their open amount; the lines of the registered
//!   [`CashForecastSource`](wl_kernel::cash_forecast::CashForecastSource)s.
//! - **Payments:** supplier bills by due date for their balance (the not yet
//!   approved ones marked, when the setting counts them), issued cheques not
//!   cleared by cheque date, refunds approved or failed in week 1, the VAT
//!   payable at the return due date, and the lines of the sources.
//! - **Memo lines**, shown and not counted: disputed amounts, the invoices a
//!   bounced cheque covered, receivables of blocked parties, supplier bills
//!   held as a possible duplicate.
//! - A date already past counts in week 1; money is AED fils; a foreign
//!   amount without a stored rate is left out with a warning.
//!
//! The days-late adjustment and the budget fill come with BL-INV-29. Payroll,
//! final settlements, subscription cycles, payer bills and scheduled charges
//! belong to modules this build does not have; they join the forecast through
//! `CashForecastSource`.

use std::collections::BTreeSet;

use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use wl_kernel::cash_forecast::ForecastLine;
use wl_kernel::payments::{BankPayment, Direction, UnpaidRefund};
use wl_kernel::{PartyId, Subject};

/// Where an opening balance comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceSource {
    /// The closing balance of a completed statement import.
    Statement,
    /// An opening cash balance entered by hand.
    Manual,
    /// None: the account counts as zero.
    None,
}

/// A bank balance as of the end of a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Balance {
    /// In AED fils.
    pub amount: i64,
    /// The day it is as of.
    pub as_of: NaiveDate,
    /// Where it comes from.
    pub source: BalanceSource,
}

/// The balance to start from: the one with the later day; on the same day
/// the statement.
pub fn choose_balance(statement: Option<Balance>, manual: Option<Balance>) -> Option<Balance> {
    match (statement, manual) {
        (Some(s), Some(m)) => Some(if m.as_of > s.as_of { m } else { s }),
        (Some(s), None) => Some(s),
        (None, Some(m)) => Some(m),
        (None, None) => None,
    }
}

/// One bank account of an entity with what the opening balance needs.
#[derive(Debug, Clone)]
pub struct AccountInput {
    /// The account label.
    pub account: String,
    /// The balance chosen, if any.
    pub balance: Option<Balance>,
    /// The payments dated after the balance day.
    pub payments: Vec<BankPayment>,
}

/// An opening balance of the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The account label.
    pub account: String,
    /// The balance after the payments, in AED fils.
    pub amount: i64,
    /// The balance it started from.
    pub balance: Option<Balance>,
    /// The payments added to it, in fils (in less out).
    pub movements: i64,
}

/// An issued invoice that is open.
#[derive(Debug, Clone)]
pub struct InvoiceInput {
    /// The invoice as a subject.
    pub subject: Subject,
    /// Its number, if issued.
    pub label: String,
    /// The party that owes it.
    pub party: PartyId,
    /// The day it falls due.
    pub due: NaiveDate,
    /// The balance in AED fils; `None` for a foreign amount without a rate.
    pub balance: Option<i64>,
    /// The disputed part in AED fils.
    pub disputed: i64,
    /// Whether the party is blocked.
    pub blocked: bool,
}

/// A supplier bill that is open.
#[derive(Debug, Clone)]
pub struct BillInput {
    /// The bill as a subject.
    pub subject: Subject,
    /// The supplier's invoice number.
    pub label: String,
    /// The supplier.
    pub party: PartyId,
    /// The day it falls due.
    pub due: NaiveDate,
    /// The balance in AED fils; `None` for a foreign amount without a rate.
    pub balance: Option<i64>,
    /// Whether it is approved.
    pub approved: bool,
    /// Whether it is held as a possible duplicate.
    pub duplicate: bool,
    /// Whether it carries owner drawings.
    pub drawings: bool,
}

/// A cheque not cleared, or bounced, with what it pays.
#[derive(Debug, Clone)]
pub struct ChequeInput {
    /// The cheque as a subject.
    pub subject: Subject,
    /// The cheque number.
    pub label: String,
    /// The drawer or payee.
    pub party: PartyId,
    /// The date on the cheque.
    pub date: NaiveDate,
    /// What is still expected, in fils.
    pub open_amount: i64,
    /// The documents it pays with the part applied to each.
    pub documents: Vec<(Subject, i64)>,
}

/// The VAT payable of one tax period.
#[derive(Debug, Clone)]
pub struct VatInput {
    /// The summary of the period, if one exists.
    pub summary: Option<Subject>,
    /// The first day of the period.
    pub period_start: NaiveDate,
    /// The last day of the period.
    pub period_end: NaiveDate,
    /// The return due date.
    pub due: NaiveDate,
    /// The net payable in AED fils (above 0).
    pub amount: i64,
    /// Whether it is an estimate from the documents of the period.
    pub estimate: bool,
    /// Whether a payment naming the summary was found.
    pub paid: bool,
}

/// Everything about one entity the grid needs.
#[derive(Debug, Clone, Default)]
pub struct EntityInput {
    /// The entity.
    pub legal_entity_id: i64,
    /// Its code.
    pub code: String,
    /// Its bank accounts.
    pub accounts: Vec<AccountInput>,
    /// Its open invoices.
    pub invoices: Vec<InvoiceInput>,
    /// Its open supplier bills.
    pub bills: Vec<BillInput>,
    /// Received cheques not cleared.
    pub cheques_in: Vec<ChequeInput>,
    /// Issued cheques not cleared.
    pub cheques_out: Vec<ChequeInput>,
    /// Received cheques that bounced and are still owed.
    pub bounced: Vec<ChequeInput>,
    /// Refunds approved or failed.
    pub refunds: Vec<UnpaidRefund>,
    /// The VAT periods.
    pub vat: Vec<VatInput>,
    /// The lines of the registered sources: `(source name, line)`.
    pub sources: Vec<(String, ForecastLine)>,
    /// Months of the horizon without an approved budget.
    pub months_without_budget: Vec<NaiveDate>,
    /// Foreign-currency documents left out for a missing rate.
    pub unpriced: Vec<Subject>,
}

/// What [`assemble`] reads.
#[derive(Debug, Clone)]
pub struct Inputs {
    /// The day the forecast is opened (Asia/Dubai).
    pub today: NaiveDate,
    /// How many weeks.
    pub weeks: usize,
    /// The first day of a week.
    pub week_start: Weekday,
    /// The stale limit, in days.
    pub stale_days: i64,
    /// Whether the disputed part counts.
    pub disputed_counted: bool,
    /// Whether the VAT payable counts.
    pub vat_counted: bool,
    /// Whether the bills not yet approved count.
    pub unapproved_counted: bool,
    /// The entities.
    pub entities: Vec<EntityInput>,
}

/// What a counted line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// An issued invoice.
    Invoice,
    /// A received cheque.
    ChequeIn,
    /// A supplier bill.
    Bill,
    /// An issued cheque.
    ChequeOut,
    /// A refund.
    Refund,
    /// VAT payable.
    Vat,
    /// A line of a [`CashForecastSource`](wl_kernel::cash_forecast::CashForecastSource).
    Source,
}

impl LineKind {
    /// The code used in texts and queries.
    pub fn code(self) -> &'static str {
        match self {
            LineKind::Invoice => "invoice",
            LineKind::ChequeIn => "cheque_in",
            LineKind::Bill => "bill",
            LineKind::ChequeOut => "cheque_out",
            LineKind::Refund => "refund",
            LineKind::Vat => "vat",
            LineKind::Source => "source",
        }
    }
}

/// An expected receipt or payment that is counted.
#[derive(Debug, Clone)]
pub struct Line {
    /// The entity.
    pub legal_entity_id: i64,
    /// What it is.
    pub kind: LineKind,
    /// The text of the grid.
    pub label: String,
    /// The category (the source's, else the kind).
    pub category: String,
    /// The source's name, for a source line.
    pub source: Option<String>,
    /// The party, if any.
    pub party: Option<PartyId>,
    /// The date it was expected.
    pub date: NaiveDate,
    /// The week it counts in, from 0.
    pub week: usize,
    /// The amount in AED fils, above 0.
    pub amount: i64,
    /// In or out.
    pub direction: Direction,
    /// The documents behind it.
    pub documents: Vec<Subject>,
    /// A supplier bill not yet approved.
    pub not_approved: bool,
    /// A VAT line estimated from the documents so far.
    pub estimate: bool,
    /// Shown as a total only (drawings, payroll).
    pub total_only: bool,
}

/// Why an amount is shown and not counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoKind {
    /// The disputed part of an invoice.
    Disputed,
    /// An invoice a bounced cheque covered.
    BouncedCheque,
    /// A receivable of a blocked party.
    BlockedParty,
    /// A supplier bill held as a possible duplicate.
    DuplicateBill,
}

impl MemoKind {
    /// The code used in texts and queries.
    pub fn code(self) -> &'static str {
        match self {
            MemoKind::Disputed => "disputed",
            MemoKind::BouncedCheque => "bounced_cheque",
            MemoKind::BlockedParty => "blocked_party",
            MemoKind::DuplicateBill => "duplicate_bill",
        }
    }

    /// Every kind, in the order of the grid.
    pub const ALL: [MemoKind; 4] = [
        MemoKind::Disputed,
        MemoKind::BouncedCheque,
        MemoKind::BlockedParty,
        MemoKind::DuplicateBill,
    ];
}

/// An amount shown and not counted.
#[derive(Debug, Clone)]
pub struct Memo {
    /// The entity.
    pub legal_entity_id: i64,
    /// Why.
    pub kind: MemoKind,
    /// The text of the grid.
    pub label: String,
    /// The party, if any.
    pub party: Option<PartyId>,
    /// The week it falls in, from 0.
    pub week: usize,
    /// The amount in AED fils.
    pub amount: i64,
    /// The document it is about.
    pub documents: Vec<Subject>,
}

/// Something the owner should know before trusting the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A bank account with no balance: it counts as zero.
    NoBalance {
        /// The entity.
        legal_entity_id: i64,
        /// The account.
        account: String,
    },
    /// A balance older than the stale limit.
    Stale {
        /// The entity.
        legal_entity_id: i64,
        /// The account.
        account: String,
        /// The day it is as of.
        as_of: NaiveDate,
    },
    /// A foreign-currency document left out for a missing rate.
    Unpriced {
        /// The document.
        document: Subject,
    },
    /// A VAT return past due with no payment found: the line is dropped.
    VatPastDue {
        /// The entity.
        legal_entity_id: i64,
        /// The last day of the period.
        period_end: NaiveDate,
    },
    /// Months of the horizon with no approved budget.
    NoBudget {
        /// The entity.
        legal_entity_id: i64,
        /// The months.
        months: Vec<NaiveDate>,
    },
}

/// One week of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Week {
    /// From 1.
    pub number: usize,
    /// The first day.
    pub start: NaiveDate,
    /// The last day.
    pub end: NaiveDate,
}

/// The forecast.
#[derive(Debug, Clone)]
pub struct Forecast {
    /// The day it was computed for.
    pub today: NaiveDate,
    /// The weeks.
    pub weeks: Vec<Week>,
    /// The opening balances.
    pub openings: Vec<Opening>,
    /// The counted lines.
    pub lines: Vec<Line>,
    /// The memo lines.
    pub memos: Vec<Memo>,
    /// The warnings.
    pub warnings: Vec<Warning>,
}

impl Forecast {
    /// The opening balances added up, in AED fils.
    pub fn opening_total(&self) -> i64 {
        self.openings.iter().map(|o| o.amount).sum()
    }

    /// The counted receipts of week `week` (from 0), in fils.
    pub fn receipts(&self, week: usize) -> i64 {
        self.sum(week, Direction::In)
    }

    /// The counted payments of week `week` (from 0), in fils.
    pub fn payments(&self, week: usize) -> i64 {
        self.sum(week, Direction::Out)
    }

    fn sum(&self, week: usize, direction: Direction) -> i64 {
        self.lines
            .iter()
            .filter(|l| l.week == week && l.direction == direction)
            .map(|l| l.amount)
            .sum()
    }

    /// The projected balance at the end of week `week` (from 0).
    pub fn balance(&self, week: usize) -> i64 {
        self.opening_total()
            + (0..=week)
                .map(|w| self.receipts(w) - self.payments(w))
                .sum::<i64>()
    }

    /// The memo total of `kind` in week `week` (from 0).
    pub fn memo(&self, kind: MemoKind, week: usize) -> i64 {
        self.memos
            .iter()
            .filter(|m| m.kind == kind && m.week == week)
            .map(|m| m.amount)
            .sum()
    }
}

/// The first day of week 1: the latest `start` day on or before `today`.
pub fn week_one_start(today: NaiveDate, start: Weekday) -> NaiveDate {
    let back = (today.weekday().num_days_from_monday() + 7 - start.num_days_from_monday()) % 7;
    today - Days::new(u64::from(back))
}

/// The weeks of a forecast opened on `today`.
pub fn weeks_of(today: NaiveDate, start: Weekday, count: usize) -> Vec<Week> {
    let first = week_one_start(today, start);
    (0..count)
        .map(|i| {
            let from = first + Days::new(7 * i as u64);
            Week {
                number: i + 1,
                start: from,
                end: from + Days::new(6),
            }
        })
        .collect()
}

/// The week (from 0) a date counts in: a date already past counts in week 1;
/// `None` beyond the horizon.
pub fn week_of(date: NaiveDate, weeks: &[Week]) -> Option<usize> {
    let first = weeks.first()?;
    if date <= first.end {
        return Some(0);
    }
    weeks.iter().position(|w| date >= w.start && date <= w.end)
}

/// The day a return is due: 28 days after the period ends.
pub fn vat_due(period_end: NaiveDate) -> NaiveDate {
    period_end + Days::new(28)
}

/// The grid for `inputs`.
pub fn assemble(inputs: &Inputs) -> Forecast {
    let weeks = weeks_of(inputs.today, inputs.week_start, inputs.weeks);
    let mut forecast = Forecast {
        today: inputs.today,
        weeks,
        openings: Vec::new(),
        lines: Vec::new(),
        memos: Vec::new(),
        warnings: Vec::new(),
    };
    for entity in &inputs.entities {
        add_openings(&mut forecast, inputs, entity);
        add_entity(&mut forecast, inputs, entity);
    }
    forecast
}

fn add_openings(forecast: &mut Forecast, inputs: &Inputs, entity: &EntityInput) {
    for account in &entity.accounts {
        let movements: i64 = account
            .payments
            .iter()
            .map(|p| match p.payment.direction {
                Direction::In => p.payment.amount,
                Direction::Out => -p.payment.amount,
            })
            .sum();
        match account.balance {
            None => forecast.warnings.push(Warning::NoBalance {
                legal_entity_id: entity.legal_entity_id,
                account: account.account.clone(),
            }),
            Some(balance) => {
                if (inputs.today - balance.as_of).num_days() > inputs.stale_days {
                    forecast.warnings.push(Warning::Stale {
                        legal_entity_id: entity.legal_entity_id,
                        account: account.account.clone(),
                        as_of: balance.as_of,
                    });
                }
            }
        }
        let base = account.balance.map_or(0, |b| b.amount);
        let movements = if account.balance.is_some() { movements } else { 0 };
        forecast.openings.push(Opening {
            legal_entity_id: entity.legal_entity_id,
            account: account.account.clone(),
            amount: base + movements,
            balance: account.balance,
            movements,
        });
    }
}

/// What a set of cheques covers of each document, in fils.
fn covered(cheques: &[ChequeInput], document: &Subject) -> i64 {
    cheques
        .iter()
        .flat_map(|c| c.documents.iter())
        .filter(|(d, _)| d == document)
        .map(|(_, amount)| *amount)
        .sum()
}

fn add_entity(forecast: &mut Forecast, inputs: &Inputs, entity: &EntityInput) {
    let id = entity.legal_entity_id;
    let mut counted: BTreeSet<Subject> = BTreeSet::new();
    let weeks = forecast.weeks.clone();
    let push_line = |forecast: &mut Forecast, line: Line| {
        if line.week < weeks.len() && line.amount > 0 {
            forecast.lines.push(line);
        }
    };

    for document in &entity.unpriced {
        forecast.warnings.push(Warning::Unpriced {
            document: document.clone(),
        });
    }

    // Receipts: invoices.
    for invoice in &entity.invoices {
        counted.insert(invoice.subject.clone());
        let Some(balance) = invoice.balance else {
            continue;
        };
        let pending = covered(&entity.cheques_in, &invoice.subject);
        let bounced = covered(&entity.bounced, &invoice.subject);
        let week = week_of(invoice.due, &weeks);
        let memo_week = week.unwrap_or(0);
        let after_pending = (balance - pending).max(0);
        let bounced_part = bounced.min(after_pending);
        let remaining = after_pending - bounced_part;
        let memo = |kind: MemoKind, amount: i64| Memo {
            legal_entity_id: id,
            kind,
            label: invoice.label.clone(),
            party: Some(invoice.party),
            week: memo_week,
            amount,
            documents: vec![invoice.subject.clone()],
        };
        if bounced_part > 0 {
            forecast.memos.push(memo(MemoKind::BouncedCheque, bounced_part));
        }
        if invoice.blocked {
            if remaining > 0 {
                forecast.memos.push(memo(MemoKind::BlockedParty, remaining));
            }
            continue;
        }
        let disputed = if inputs.disputed_counted {
            0
        } else {
            invoice.disputed.min(remaining)
        };
        if disputed > 0 {
            forecast.memos.push(memo(MemoKind::Disputed, disputed));
        }
        if let Some(week) = week {
            push_line(
                forecast,
                Line {
                    legal_entity_id: id,
                    kind: LineKind::Invoice,
                    label: invoice.label.clone(),
                    category: LineKind::Invoice.code().to_owned(),
                    source: None,
                    party: Some(invoice.party),
                    date: invoice.due,
                    week,
                    amount: remaining - disputed,
                    direction: Direction::In,
                    documents: vec![invoice.subject.clone()],
                    not_approved: false,
                    estimate: false,
                    total_only: false,
                },
            );
        }
    }

    // Receipts and payments: cheques not cleared.
    for (cheques, kind, direction) in [
        (&entity.cheques_in, LineKind::ChequeIn, Direction::In),
        (&entity.cheques_out, LineKind::ChequeOut, Direction::Out),
    ] {
        for cheque in cheques {
            counted.insert(cheque.subject.clone());
            for (document, _) in &cheque.documents {
                counted.insert(document.clone());
            }
            let Some(week) = week_of(cheque.date, &weeks) else {
                continue;
            };
            push_line(
                forecast,
                Line {
                    legal_entity_id: id,
                    kind,
                    label: cheque.label.clone(),
                    category: kind.code().to_owned(),
                    source: None,
                    party: Some(cheque.party),
                    date: cheque.date,
                    week,
                    amount: cheque.open_amount,
                    direction,
                    documents: std::iter::once(cheque.subject.clone())
                        .chain(cheque.documents.iter().map(|(d, _)| d.clone()))
                        .collect(),
                    not_approved: false,
                    estimate: false,
                    total_only: false,
                },
            );
        }
    }

    // Payments: supplier bills.
    for bill in &entity.bills {
        counted.insert(bill.subject.clone());
        let Some(balance) = bill.balance else {
            continue;
        };
        let left = (balance - covered(&entity.cheques_out, &bill.subject)).max(0);
        let week = week_of(bill.due, &weeks);
        if bill.duplicate {
            if left > 0 {
                forecast.memos.push(Memo {
                    legal_entity_id: id,
                    kind: MemoKind::DuplicateBill,
                    label: bill.label.clone(),
                    party: Some(bill.party),
                    week: week.unwrap_or(0),
                    amount: left,
                    documents: vec![bill.subject.clone()],
                });
            }
            continue;
        }
        if !bill.approved && !inputs.unapproved_counted {
            continue;
        }
        if let Some(week) = week {
            push_line(
                forecast,
                Line {
                    legal_entity_id: id,
                    kind: LineKind::Bill,
                    label: bill.label.clone(),
                    category: LineKind::Bill.code().to_owned(),
                    source: None,
                    party: Some(bill.party),
                    date: bill.due,
                    week,
                    amount: left,
                    direction: Direction::Out,
                    documents: vec![bill.subject.clone()],
                    not_approved: !bill.approved,
                    estimate: false,
                    total_only: bill.drawings,
                },
            );
        }
    }

    // Payments: refunds approved or failed, in week 1.
    for refund in &entity.refunds {
        if !refund.currency.eq_ignore_ascii_case("AED") {
            forecast.warnings.push(Warning::Unpriced {
                document: refund.id.subject(),
            });
            continue;
        }
        push_line(
            forecast,
            Line {
                legal_entity_id: id,
                kind: LineKind::Refund,
                label: refund.id.subject().to_string(),
                category: LineKind::Refund.code().to_owned(),
                source: None,
                party: Some(refund.party),
                date: inputs.today,
                week: 0,
                amount: refund.amount,
                direction: Direction::Out,
                documents: vec![refund.id.subject()],
                not_approved: false,
                estimate: false,
                total_only: false,
            },
        );
    }

    // Payments: VAT at the return due date.
    if inputs.vat_counted {
        for vat in &entity.vat {
            if vat.paid {
                continue;
            }
            if vat.due < inputs.today {
                forecast.warnings.push(Warning::VatPastDue {
                    legal_entity_id: id,
                    period_end: vat.period_end,
                });
                continue;
            }
            let Some(week) = week_of(vat.due, &weeks) else {
                continue;
            };
            push_line(
                forecast,
                Line {
                    legal_entity_id: id,
                    kind: LineKind::Vat,
                    label: format!(
                        "{} - {}",
                        vat.period_start.format("%Y-%m-%d"),
                        vat.period_end.format("%Y-%m-%d")
                    ),
                    category: LineKind::Vat.code().to_owned(),
                    source: None,
                    party: None,
                    date: vat.due,
                    week,
                    amount: vat.amount,
                    direction: Direction::Out,
                    documents: vat.summary.iter().cloned().collect(),
                    not_approved: false,
                    estimate: vat.estimate,
                    total_only: false,
                },
            );
        }
    }

    // Lines other modules add; one standing for a document counted above is
    // dropped.
    for (name, line) in &entity.sources {
        if line.documents.iter().any(|d| counted.contains(d)) || line.amount <= 0 {
            continue;
        }
        let Some(week) = week_of(line.date, &weeks) else {
            continue;
        };
        push_line(
            forecast,
            Line {
                legal_entity_id: id,
                kind: LineKind::Source,
                label: line.label.clone(),
                category: line.category.clone(),
                source: Some(name.clone()),
                party: line.party,
                date: line.date,
                week,
                amount: line.amount,
                direction: line.direction,
                documents: line.documents.clone(),
                not_approved: false,
                estimate: false,
                total_only: line.total_only,
            },
        );
    }

    if !entity.months_without_budget.is_empty() {
        forecast.warnings.push(Warning::NoBudget {
            legal_entity_id: id,
            months: entity.months_without_budget.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::payments::{PaymentId, PaymentMethod, PaymentStatus, PaymentSummary};

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn inputs(entity: EntityInput) -> Inputs {
        Inputs {
            today: day(2027, 1, 13),
            weeks: 13,
            week_start: Weekday::Mon,
            stale_days: 7,
            disputed_counted: false,
            vat_counted: true,
            unapproved_counted: true,
            entities: vec![entity],
        }
    }

    fn invoice(id: i64, due: NaiveDate, balance: i64) -> InvoiceInput {
        InvoiceInput {
            subject: Subject::new("invoicing.invoice", id),
            label: format!("INV-{id}"),
            party: PartyId(1),
            due,
            balance: Some(balance),
            disputed: 0,
            blocked: false,
        }
    }

    fn bill(id: i64, due: NaiveDate, balance: i64, approved: bool) -> BillInput {
        BillInput {
            subject: Subject::new("invoicing.supplier_bill", id),
            label: format!("SB-{id}"),
            party: PartyId(2),
            due,
            balance: Some(balance),
            approved,
            duplicate: false,
            drawings: false,
        }
    }

    fn payment(direction: Direction, amount: i64, on: NaiveDate) -> BankPayment {
        BankPayment {
            payment: PaymentSummary {
                id: PaymentId(1),
                number: None,
                party: PartyId(1),
                legal_entity_id: 1,
                direction,
                method: PaymentMethod::Transfer,
                amount,
                allocated: 0,
                unallocated: amount,
                currency: "AED".into(),
                status: PaymentStatus::Received,
                date: on,
            },
            bank_account: "BA1".into(),
            reference: None,
        }
    }

    fn entity() -> EntityInput {
        EntityInput {
            legal_entity_id: 1,
            code: "E1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn week_one_is_the_week_containing_today() {
        let monday = week_one_start(day(2027, 1, 13), Weekday::Mon);
        assert_eq!(monday, day(2027, 1, 11));
        let sunday = week_one_start(day(2027, 1, 13), Weekday::Sun);
        assert_eq!(sunday, day(2027, 1, 10));
        let weeks = weeks_of(day(2027, 1, 13), Weekday::Sun, 13);
        assert_eq!((weeks[0].start, weeks[0].end), (day(2027, 1, 10), day(2027, 1, 16)));
        assert_eq!(weeks.len(), 13);
    }

    #[test]
    fn a_past_date_counts_in_week_one_and_a_far_one_is_out() {
        let weeks = weeks_of(day(2027, 1, 13), Weekday::Mon, 13);
        assert_eq!(week_of(day(2027, 1, 2), &weeks), Some(0));
        assert_eq!(week_of(day(2027, 1, 25), &weeks), Some(2));
        assert_eq!(week_of(day(2027, 1, 31), &weeks), Some(2));
        assert_eq!(week_of(day(2027, 2, 1), &weeks), Some(3));
        assert_eq!(week_of(day(2027, 6, 1), &weeks), None);
    }

    #[test]
    fn the_later_balance_wins_and_the_statement_wins_a_tie() {
        let statement = Balance { amount: 5_000_000, as_of: day(2027, 1, 10), source: BalanceSource::Statement };
        let manual = Balance { amount: 6_000_000, as_of: day(2027, 1, 11), source: BalanceSource::Manual };
        assert_eq!(choose_balance(Some(statement), Some(manual)), Some(manual));
        let same_day = Balance { as_of: day(2027, 1, 10), ..manual };
        assert_eq!(choose_balance(Some(statement), Some(same_day)), Some(statement));
        assert_eq!(choose_balance(None, None), None);
    }

    #[test]
    fn the_payments_after_the_balance_day_are_added_to_the_opening() {
        let mut e = entity();
        e.accounts.push(AccountInput {
            account: "BA1".into(),
            balance: Some(Balance { amount: 5_000_000, as_of: day(2027, 1, 10), source: BalanceSource::Statement }),
            payments: vec![
                payment(Direction::In, 500_000, day(2027, 1, 12)),
                payment(Direction::Out, 100_000, day(2027, 1, 12)),
            ],
        });
        e.accounts.push(AccountInput { account: "BA2".into(), balance: None, payments: vec![] });
        let forecast = assemble(&inputs(e));
        assert_eq!(forecast.openings[0].amount, 5_400_000);
        assert_eq!(forecast.openings[1].amount, 0);
        assert!(forecast.warnings.contains(&Warning::NoBalance { legal_entity_id: 1, account: "BA2".into() }));
        assert_eq!(forecast.opening_total(), 5_400_000);
    }

    #[test]
    fn a_stale_balance_is_used_with_a_warning() {
        let mut e = entity();
        e.accounts.push(AccountInput {
            account: "BA1".into(),
            balance: Some(Balance { amount: 100, as_of: day(2027, 1, 3), source: BalanceSource::Manual }),
            payments: vec![],
        });
        let forecast = assemble(&inputs(e));
        assert_eq!(forecast.openings[0].amount, 100);
        assert!(matches!(forecast.warnings[0], Warning::Stale { .. }));
    }

    #[test]
    fn invoices_and_bills_count_in_the_week_of_their_due_date() {
        let mut e = entity();
        e.invoices.push(invoice(1, day(2027, 1, 25), 1_050_000));
        e.invoices.push(invoice(2, day(2027, 1, 2), 200_000));
        e.bills.push(bill(5, day(2027, 2, 10), 1_200_000, true));
        let forecast = assemble(&inputs(e));
        assert_eq!(forecast.receipts(2), 1_050_000);
        assert_eq!(forecast.receipts(0), 200_000);
        assert_eq!(forecast.payments(4), 1_200_000);
        assert_eq!(forecast.balance(4), 1_050_000 + 200_000 - 1_200_000);
    }

    #[test]
    fn a_disputed_part_is_a_memo_unless_the_setting_counts_it() {
        let mut e = entity();
        let mut i = invoice(1, day(2027, 1, 20), 600_000);
        i.disputed = 200_000;
        e.invoices.push(i);
        let forecast = assemble(&inputs(e.clone()));
        assert_eq!(forecast.receipts(1), 400_000);
        assert_eq!(forecast.memo(MemoKind::Disputed, 1), 200_000);
        let mut counting = inputs(e);
        counting.disputed_counted = true;
        let forecast = assemble(&counting);
        assert_eq!(forecast.receipts(1), 600_000);
        assert_eq!(forecast.memo(MemoKind::Disputed, 1), 0);
    }

    #[test]
    fn a_cheque_counts_by_its_date_and_its_invoice_by_the_rest() {
        let mut e = entity();
        e.invoices.push(invoice(3, day(2027, 1, 20), 800_000));
        e.cheques_in.push(ChequeInput {
            subject: Subject::new("payments.cheque", 7),
            label: "CHQ-7".into(),
            party: PartyId(1),
            date: day(2027, 2, 1),
            open_amount: 800_000,
            documents: vec![(Subject::new("invoicing.invoice", 3), 800_000)],
        });
        let forecast = assemble(&inputs(e));
        assert_eq!(forecast.receipts(3), 800_000);
        assert_eq!(forecast.receipts(1), 0);
    }

    #[test]
    fn a_bounced_cheque_moves_its_invoice_to_the_memo_line() {
        let mut e = entity();
        e.invoices.push(invoice(5, day(2027, 1, 1), 700_000));
        e.bounced.push(ChequeInput {
            subject: Subject::new("payments.cheque", 8),
            label: "CHQ-8".into(),
            party: PartyId(1),
            date: day(2027, 1, 1),
            open_amount: 700_000,
            documents: vec![(Subject::new("invoicing.invoice", 5), 700_000)],
        });
        let forecast = assemble(&inputs(e));
        assert_eq!(forecast.receipts(0), 0);
        assert_eq!(forecast.memo(MemoKind::BouncedCheque, 0), 700_000);
    }

    #[test]
    fn a_blocked_partys_receivable_and_a_duplicate_bill_are_memo_lines() {
        let mut e = entity();
        let mut blocked = invoice(1, day(2027, 1, 20), 300_000);
        blocked.blocked = true;
        e.invoices.push(blocked);
        let mut held = bill(6, day(2027, 2, 3), 900_000, false);
        held.duplicate = true;
        e.bills.push(held);
        e.bills.push(bill(7, day(2027, 2, 3), 400_000, false));
        let forecast = assemble(&inputs(e));
        assert_eq!(forecast.receipts(1), 0);
        assert_eq!(forecast.memo(MemoKind::BlockedParty, 1), 300_000);
        assert_eq!(forecast.memo(MemoKind::DuplicateBill, 3), 900_000);
        assert_eq!(forecast.payments(3), 400_000);
        assert!(forecast.lines.iter().any(|l| l.not_approved));
    }

    #[test]
    fn bills_not_yet_approved_can_be_left_out() {
        let mut e = entity();
        e.bills.push(bill(7, day(2027, 2, 3), 400_000, false));
        let mut left_out = inputs(e);
        left_out.unapproved_counted = false;
        assert_eq!(assemble(&left_out).payments(3), 0);
    }

    #[test]
    fn vat_counts_at_the_due_date_and_a_past_one_is_dropped_with_a_warning() {
        let mut e = entity();
        e.vat.push(VatInput {
            summary: Some(Subject::new("invoicing.vat_summary", 1)),
            period_start: day(2026, 10, 1),
            period_end: day(2026, 12, 31),
            due: day(2027, 1, 28),
            amount: 1_400_000,
            estimate: false,
            paid: false,
        });
        e.vat.push(VatInput {
            summary: None,
            period_start: day(2026, 7, 1),
            period_end: day(2026, 9, 30),
            due: day(2026, 10, 28),
            amount: 100,
            estimate: true,
            paid: false,
        });
        let forecast = assemble(&inputs(e));
        assert_eq!(forecast.payments(2), 1_400_000);
        assert_eq!(forecast.lines.len(), 1);
        assert!(forecast.warnings.iter().any(|w| matches!(w, Warning::VatPastDue { .. })));
    }

    #[test]
    fn a_source_line_for_a_counted_document_is_dropped() {
        let mut e = entity();
        e.bills.push(bill(5, day(2027, 2, 10), 100_000, true));
        e.sources.push((
            "pm.payouts".into(),
            ForecastLine::new("Payout", "owner_payout", day(2027, 1, 20), 50_000, Direction::Out)
                .document(Subject::new("invoicing.supplier_bill", 5)),
        ));
        e.sources.push((
            "pm.payouts".into(),
            ForecastLine::new("Payout", "owner_payout", day(2027, 1, 20), 70_000, Direction::Out),
        ));
        let forecast = assemble(&inputs(e));
        assert_eq!(forecast.payments(1), 70_000);
        assert_eq!(forecast.payments(4), 100_000);
    }

    #[test]
    fn refunds_count_in_week_one() {
        let mut e = entity();
        e.refunds.push(UnpaidRefund {
            id: wl_kernel::payments::RefundId(4),
            party: PartyId(1),
            legal_entity_id: 1,
            amount: 90_000,
            currency: "AED".into(),
            failed: false,
            approved_on: None,
            source: Subject::new("invoicing.credit_note", 2),
        });
        assert_eq!(assemble(&inputs(e)).payments(0), 90_000);
    }
}
