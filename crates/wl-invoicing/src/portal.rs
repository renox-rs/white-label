//! The party's invoices and statement of account in the portal (wl-invoicing
//! §5.3 point 3, §5.10; BL-INV-31).
//!
//! The data and the pages are here; the routes are mounted by the host,
//! which checks the portal grant first (wl-portal's menu and guard, which
//! this crate does not depend on) and then calls [`invoices_page`],
//! [`statement_page`] and [`dispute`].
//!
//! - **Invoices**: the party's `issued`, `partially_paid`, `overdue`, `paid`
//!   and `written_off` invoices ([`LISTED`]) with number, dates, amount,
//!   balance and status; a disputed one is marked *Disputed*. Drafts are
//!   never shown, nor a credited (cancelled) invoice: the statement shows
//!   its credit note.
//! - **Statement of account** ([`statement_of`]): the party's issued
//!   invoices (debit), issued credit notes (credit), payments allocated to
//!   its invoices (credit; a reversal is a debit on the day it was removed)
//!   and write-offs (credit) in a period, with the opening balance, a running
//!   balance and the closing balance. Money a payment left unallocated (a
//!   credit balance) is not on an invoice and is not shown here.
//! - **Dispute**: on an `issued`, `partially_paid` or `overdue` invoice not
//!   disputed yet, with a reason, the amount and optionally the lines; it
//!   runs the same `raise` as staff with `DisputeBy::Party` (the ticket, the
//!   paused reminders, the acknowledgement), which refuses it when the
//!   client's `portal_disputes` setting is off. The portal never sets an
//!   outcome, and no internal note of the dispute or its ticket is shown.
//! - **Tax invoice PDF**: not offered. No stored PDF of a tax invoice exists
//!   yet (generation belongs to the base lifecycle and BL-DOC-11); the
//!   download link comes with it.

use std::collections::{BTreeMap, HashMap};

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::invoicing::{CreditNoteStatus, DisputeBy, DisputeRequest, InvoiceId, InvoiceStatus};
use wl_kernel::payments::PAYMENT_SUBJECT_TYPE;

use crate::allocate::amount_text;
use crate::allocation::Allocation;
use crate::config_of;
use crate::credit_note::CreditNote;
use crate::credit_note_pages::parse_amount;
use crate::invoice::Invoice;
use crate::invoicer::lines_of;
use crate::texts;

/// The key of the portal section.
pub const SECTION: &str = "invoices";

/// The portal permission that shows the section.
pub const VIEW_PERMISSION: &str = "portal.invoices.view";

/// The statuses the portal lists.
pub const LISTED: [InvoiceStatus; 5] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
    InvoiceStatus::Paid,
    InvoiceStatus::WrittenOff,
];

/// The package text `key` (without the `wl_invoicing.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_invoicing.{key}"))
}

/// `date` as `16 Nov 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The party's invoices the portal lists, newest first.
pub async fn invoices_of(db: &Db, party: PartyId) -> Result<Vec<Invoice>> {
    Invoice::query()
        .where_(Invoice::PARTY_ID.eq(party.0))
        .where_(Invoice::STATUS.is_in(LISTED))
        .order_by_desc(Invoice::ISSUE_DATE)
        .order_by_desc(Invoice::ID)
        .get(db)
        .await
}

/// The status of `invoice` in words: *Disputed* while flagged.
pub fn status_text(lang: &Lang, invoice: &Invoice) -> String {
    if invoice.disputed {
        text(lang, "portal.disputed")
    } else {
        text(lang, &format!("status.{}", invoice.status.as_str()))
    }
}

/// Whether the party may dispute `invoice` now (the setting aside).
pub fn disputable(invoice: &Invoice) -> bool {
    invoice.status.is_open() && !invoice.disputed
}

/// A line the party may tick in the dispute sheet.
#[derive(Debug, Clone, Serialize)]
pub struct PortalLine {
    /// The checkbox name, `line_<position>`.
    pub name: String,
    /// `1. Parking (AED 1,500.00)`.
    pub label: String,
}

/// One invoice as the portal shows it.
#[derive(Debug, Clone, Serialize)]
pub struct PortalInvoice {
    /// The invoice id.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The issue date.
    pub issue_date: String,
    /// The due date.
    pub due_date: String,
    /// The total.
    pub amount: String,
    /// The balance.
    pub balance: String,
    /// The status in words.
    pub status: String,
    /// The badge kind of the status.
    pub status_kind: &'static str,
    /// Whether the *Dispute* action shows.
    pub can_dispute: bool,
    /// The lines to tick.
    pub lines: Vec<PortalLine>,
}

/// The rows of the party's invoices; `disputes_on` offers *Dispute* on the
/// ones that can be disputed.
pub async fn rows(
    db: &Db,
    lang: &Lang,
    party: PartyId,
    disputes_on: bool,
) -> Result<Vec<PortalInvoice>> {
    let mut out = Vec::new();
    for invoice in invoices_of(db, party).await? {
        let can_dispute = disputes_on && disputable(&invoice);
        let lines = if can_dispute {
            lines_of(db, invoice.id)
                .await?
                .into_iter()
                .map(|line| PortalLine {
                    name: format!("line_{}", line.position),
                    label: format!(
                        "{}. {} ({})",
                        line.position,
                        line.description,
                        amount_text(&invoice.currency, line.gross_amount)
                    ),
                })
                .collect()
        } else {
            Vec::new()
        };
        let status_kind = match invoice.status {
            _ if invoice.disputed => "warning",
            InvoiceStatus::Paid => "success",
            InvoiceStatus::Overdue => "error",
            _ => "info",
        };
        out.push(PortalInvoice {
            id: invoice.id,
            number: invoice.number.clone().unwrap_or_default(),
            issue_date: invoice.issue_date.map(date_text).unwrap_or_default(),
            due_date: invoice.due_date.map(date_text).unwrap_or_default(),
            amount: amount_text(&invoice.currency, invoice.gross_total),
            balance: amount_text(&invoice.currency, invoice.balance),
            status: status_text(lang, &invoice),
            status_kind,
            can_dispute,
            lines,
        });
    }
    Ok(out)
}

/// What the host tells the pages about the viewer.
#[derive(Debug, Clone, Default)]
pub struct PortalContext {
    /// The portal home of the party, `/portal/12`.
    pub home_url: String,
    /// Whether the viewer's grant may write (not `read_only`): with the
    /// setting on, the *Dispute* action shows.
    pub can_write: bool,
    /// The notice of a `read_only` grant, if any.
    pub read_only_notice: Option<String>,
}

/// Whether the client lets the party dispute in the portal.
pub fn disputes_allowed(state: &AppState) -> bool {
    config_of(state).portal_disputes
}

/// What the invoices page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/portal_invoices.html")]
pub struct PortalInvoicesPage {
    tx: BTreeMap<&'static str, String>,
    invoices: Vec<PortalInvoice>,
    home_url: String,
    statement_url: String,
    dispute_base: String,
    read_only_notice: Option<String>,
}

/// The *Invoices* page of `party`.
pub async fn invoices_page(
    state: &AppState,
    lang: &Lang,
    party: PartyId,
    context: PortalContext,
) -> Result<View> {
    let disputes_on = disputes_allowed(state) && context.can_write;
    let invoices = rows(&state.db, lang, party, disputes_on).await?;
    Ok(View::typed(PortalInvoicesPage {
        tx: texts::all(lang),
        invoices,
        statement_url: format!("{}/{SECTION}/statement", context.home_url),
        dispute_base: format!("{}/{SECTION}", context.home_url),
        home_url: context.home_url,
        read_only_notice: context.read_only_notice,
    }))
}

/// What a statement line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementKind {
    /// An issued invoice: a debit.
    Invoice,
    /// An issued credit note: a credit.
    CreditNote,
    /// A payment allocated to an invoice: a credit.
    Payment,
    /// A payment allocation removed (reversed): a debit.
    PaymentReversed,
    /// An invoice written off: a credit.
    WriteOff,
}

impl StatementKind {
    /// The text key of the kind.
    fn key(self) -> &'static str {
        match self {
            StatementKind::Invoice => "portal.statement.invoice",
            StatementKind::CreditNote => "portal.statement.credit_note",
            StatementKind::Payment => "portal.statement.payment",
            StatementKind::PaymentReversed => "portal.statement.payment_reversed",
            StatementKind::WriteOff => "portal.statement.write_off",
        }
    }
}

/// A movement of the statement, before the running balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movement {
    /// The day it counts on.
    pub date: NaiveDate,
    /// What it is.
    pub kind: StatementKind,
    /// The document number it refers to.
    pub reference: String,
    /// Fils the party owes more.
    pub debit: i64,
    /// Fils the party owes less.
    pub credit: i64,
}

/// A line of the statement with its running balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementLine {
    /// The movement.
    pub movement: Movement,
    /// The balance after it.
    pub balance: i64,
}

/// The statement of account of a period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    /// The first day.
    pub from: NaiveDate,
    /// The last day.
    pub to: NaiveDate,
    /// What the party owed before `from`.
    pub opening: i64,
    /// The movements of the period, by date.
    pub lines: Vec<StatementLine>,
    /// What the party owed at the end of `to`.
    pub closing: i64,
}

/// Builds the statement of `movements` for `from`..=`to`: the movements
/// before `from` make the opening balance, the ones inside are listed in
/// date order with a running balance, later ones are left out.
pub fn build_statement(mut movements: Vec<Movement>, from: NaiveDate, to: NaiveDate) -> Statement {
    movements.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.reference.cmp(&b.reference))
    });
    let opening: i64 = movements
        .iter()
        .filter(|m| m.date < from)
        .map(|m| m.debit - m.credit)
        .sum();
    let mut balance = opening;
    let mut lines = Vec::new();
    for movement in movements
        .into_iter()
        .filter(|m| m.date >= from && m.date <= to)
    {
        balance += movement.debit - movement.credit;
        lines.push(StatementLine { movement, balance });
    }
    Statement {
        from,
        to,
        opening,
        lines,
        closing: balance,
    }
}

/// Every movement of `party` up to `to`.
pub async fn movements_of(db: &Db, party: PartyId, to: NaiveDate) -> Result<Vec<Movement>> {
    let mut movements = Vec::new();
    let invoices = Invoice::query()
        .where_(Invoice::PARTY_ID.eq(party.0))
        .where_(Invoice::STATUS.ne(InvoiceStatus::Draft))
        .order_by(Invoice::ID)
        .get(db)
        .await?;
    let mut numbers: HashMap<i64, String> = HashMap::new();
    for invoice in &invoices {
        let number = invoice.number.clone().unwrap_or_default();
        numbers.insert(invoice.id, number.clone());
        let Some(issued) = invoice.issue_date else {
            continue;
        };
        movements.push(Movement {
            date: issued,
            kind: StatementKind::Invoice,
            reference: number.clone(),
            debit: invoice.gross_total,
            credit: 0,
        });
        if let (Some(amount), Some(day)) = (invoice.write_off_amount, invoice.write_off_date) {
            movements.push(Movement {
                date: day,
                kind: StatementKind::WriteOff,
                reference: number,
                debit: 0,
                credit: amount,
            });
        }
    }
    let notes = CreditNote::query()
        .where_(CreditNote::PARTY_ID.eq(party.0))
        .where_(CreditNote::STATUS.ne(CreditNoteStatus::Draft))
        .order_by(CreditNote::ID)
        .get(db)
        .await?;
    for note in notes {
        let Some(issued) = note.issue_date else {
            continue;
        };
        movements.push(Movement {
            date: issued,
            kind: StatementKind::CreditNote,
            reference: note.number.clone().unwrap_or_default(),
            debit: 0,
            credit: note.gross_total,
        });
    }
    let ids: Vec<i64> = invoices.iter().map(|i| i.id).collect();
    if !ids.is_empty() {
        let allocations = Allocation::query()
            .where_(Allocation::INVOICE_ID.is_in(ids))
            .where_(Allocation::SOURCE_TYPE.eq(PAYMENT_SUBJECT_TYPE))
            .order_by(Allocation::ID)
            .get(db)
            .await?;
        for allocation in allocations {
            let reference = numbers
                .get(&allocation.invoice_id)
                .cloned()
                .unwrap_or_default();
            movements.push(Movement {
                date: dubai_date(allocation.allocated_at),
                kind: StatementKind::Payment,
                reference: reference.clone(),
                debit: 0,
                credit: allocation.amount,
            });
            if let Some(removed) = allocation.removed_at {
                movements.push(Movement {
                    date: dubai_date(removed),
                    kind: StatementKind::PaymentReversed,
                    reference,
                    debit: allocation.amount,
                    credit: 0,
                });
            }
        }
    }
    // Credit notes are counted when issued, not when applied, so their
    // allocations (source `invoicing.credit_note`) are not movements of
    // their own.
    movements.retain(|m| m.date <= to);
    Ok(movements)
}

/// The statement of account of `party` for `from`..=`to`.
pub async fn statement_of(
    db: &Db,
    party: PartyId,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Statement> {
    Ok(build_statement(
        movements_of(db, party, to).await?,
        from,
        to,
    ))
}

/// The current calendar month, the default period.
pub fn this_month() -> (NaiveDate, NaiveDate) {
    let day = today();
    let first = NaiveDate::from_ymd_opt(day.year(), day.month(), 1).unwrap_or(day);
    let last = first
        .checked_add_months(renox::chrono::Months::new(1))
        .and_then(|next| next.pred_opt())
        .unwrap_or(day);
    (first, last)
}

/// A statement line as the page shows it.
#[derive(Debug, Clone, Serialize)]
pub struct StatementRow {
    /// The date.
    pub date: String,
    /// What it is, in words.
    pub kind: String,
    /// The document number.
    pub reference: String,
    /// The debit, empty when 0.
    pub debit: String,
    /// The credit, empty when 0.
    pub credit: String,
    /// The running balance.
    pub balance: String,
}

/// What the statement page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/portal_statement.html")]
pub struct PortalStatementPage {
    tx: BTreeMap<&'static str, String>,
    statement_rows: Vec<StatementRow>,
    opening: String,
    closing: String,
    from: String,
    to: String,
    period: String,
    invoices_url: String,
    statement_url: String,
    read_only_notice: Option<String>,
}

/// The *Statement of account* page of `party` for `from`..=`to`.
pub async fn statement_page(
    state: &AppState,
    lang: &Lang,
    party: PartyId,
    from: NaiveDate,
    to: NaiveDate,
    context: PortalContext,
) -> Result<View> {
    let statement = statement_of(&state.db, party, from, to).await?;
    let money = |fils: i64| amount_text("AED", fils);
    let shown = |fils: i64| {
        if fils == 0 {
            String::new()
        } else {
            money(fils)
        }
    };
    let statement_rows = statement
        .lines
        .iter()
        .map(|line| StatementRow {
            date: date_text(line.movement.date),
            kind: text(lang, line.movement.kind.key()),
            reference: line.movement.reference.clone(),
            debit: shown(line.movement.debit),
            credit: shown(line.movement.credit),
            balance: money(line.balance),
        })
        .collect();
    Ok(View::typed(PortalStatementPage {
        tx: texts::all(lang),
        statement_rows,
        opening: money(statement.opening),
        closing: money(statement.closing),
        from: from.to_string(),
        to: to.to_string(),
        period: format!("{} - {}", date_text(from), date_text(to)),
        invoices_url: format!("{}/{SECTION}", context.home_url),
        statement_url: format!("{}/{SECTION}/statement", context.home_url),
        read_only_notice: context.read_only_notice,
    }))
}

/// The positions of the ticked `line_<position>` checkboxes.
fn ticked(form: &HashMap<String, String>) -> Vec<i32> {
    let mut positions: Vec<i32> = form
        .iter()
        .filter(|(_, value)| !value.is_empty() && *value != "0" && *value != "false")
        .filter_map(|(key, _)| key.strip_prefix("line_")?.parse().ok())
        .collect();
    positions.sort_unstable();
    positions
}

/// The party disputes invoice `id` from the portal (`reason`, `amount`,
/// `line_<position>`): the same `raise` as staff, by the party. 404 for an
/// invoice of another party or one the portal does not list; 403 when the
/// setting is off; 422 for a failed guard (the message under `reason` or
/// `amount`).
pub async fn dispute(
    state: &AppState,
    lang: &Lang,
    party: PartyId,
    id: i64,
    form: &HashMap<String, String>,
) -> Result<i64> {
    let invoice = Invoice::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if invoice.party_id != party.0 || !LISTED.contains(&invoice.status) {
        return Err(Error::NotFound);
    }
    if !disputes_allowed(state) {
        return Err(Error::Forbidden);
    }
    let invalid = |field: &str, message: String| -> Error {
        let mut errors = renox::validation::Errors::new();
        errors.add(field, message);
        Error::Validation(renox::validation::ValidationError::new(errors).with_input(form))
    };
    let amount = match form
        .get("amount")
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
    {
        None => None,
        Some(typed) => Some(
            parse_amount(typed)
                .ok_or_else(|| invalid("amount", text(lang, "credit_note.errors.amount")))?,
        ),
    };
    let reason = form.get("reason").cloned().unwrap_or_default();
    let mut request = DisputeRequest::new(
        InvoiceId(invoice.id),
        reason,
        DisputeBy::Party { id: party },
    )
    .lines(ticked(form));
    request.amount = amount;
    match crate::dispute::raise(state, request).await {
        Ok(id) => Ok(id),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            let field = if message.contains("reason") {
                "reason"
            } else {
                "amount"
            };
            Err(invalid(field, message))
        }
        Err(other) => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn movement(date: NaiveDate, kind: StatementKind, debit: i64, credit: i64) -> Movement {
        Movement {
            date,
            kind,
            reference: "INV-1".into(),
            debit,
            credit,
        }
    }

    #[test]
    fn the_statement_runs_from_the_opening_to_the_closing_balance() {
        let statement = build_statement(
            vec![
                movement(day(9, 1), StatementKind::Invoice, 100_000, 0),
                movement(day(9, 20), StatementKind::Payment, 0, 100_000),
                movement(day(10, 3), StatementKind::Invoice, 450_000, 0),
                movement(day(10, 12), StatementKind::Payment, 0, 300_000),
                movement(day(11, 2), StatementKind::Invoice, 450_000, 0),
            ],
            day(10, 1),
            day(10, 31),
        );
        assert_eq!(statement.opening, 0);
        assert_eq!(statement.lines.len(), 2);
        assert_eq!(statement.lines[0].balance, 450_000);
        assert_eq!(statement.lines[1].balance, 150_000);
        assert_eq!(statement.closing, 150_000);
    }

    #[test]
    fn only_open_undisputed_invoices_can_be_disputed() {
        let mut invoice = Invoice {
            status: InvoiceStatus::Issued,
            ..Invoice::default()
        };
        assert!(disputable(&invoice));
        invoice.disputed = true;
        assert!(!disputable(&invoice));
        invoice.disputed = false;
        invoice.status = InvoiceStatus::Paid;
        assert!(!disputable(&invoice));
    }

    #[test]
    fn ticked_lines_are_read_from_the_checkboxes() {
        let form: HashMap<String, String> = [
            ("line_2".to_owned(), "on".to_owned()),
            ("line_1".to_owned(), "on".to_owned()),
            ("reason".to_owned(), "x".to_owned()),
        ]
        .into_iter()
        .collect();
        assert_eq!(ticked(&form), [1, 2]);
    }
}
