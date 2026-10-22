//! The money documents of a withdrawal (10 §4.3 *When it goes wrong* rows
//! 1-2, §7.1 *Enrolment* `withdraw`, BL-EDU-03; also the withdrawals of a
//! batch `cancel` and `terminate`, BL-EDU-13 and BL-EDU-12).
//!
//! The state of the enrolment changes when the withdrawal is approved
//! (`FIN.ACL.04`, [`crate::enrolment_machine::withdraw_policy`]); the documents
//! follow it, after the commit, in [`settle`]:
//!
//! - a membership's subscription is cancelled at once, with its own pro rata
//!   credit (not repeated here), and the payment link of the first invoice is
//!   cancelled;
//! - drafts of the enrolment's invoices are discarded and, unless the student
//!   withdrew from `pending_payment` (nothing is credited then), issued
//!   invoices nothing was paid on are credited in full;
//! - the refund the policy gave ([`crate::refund`], stored on the enrolment) is
//!   credited on the invoices that were paid, newest first ([`plan_credits`]),
//!   with a credit note per invoice, and each credit note is asked back through
//!   `PaymentRecorder::request_refund` (it waits for another user's approval,
//!   `FIN.ACL.04`, recorded by the user who asked for the withdrawal);
//! - when the sessions used and the fees came to more than was paid, one
//!   invoice for the amount owed is drafted;
//! - instalments not yet invoiced were cancelled by the transition itself.
//!
//! The steps are kept on the enrolment ([`Settlement`]) as they are done, so
//! a step that fails (payments off, a refused credit note) is repeated by
//! [`settle`] without doing the others twice; the reason is shown on the
//! enrolment page. A refund that later fails (`RefundFailed`) changes nothing
//! here: the enrolment stays `withdrawn` and wl-invoicing reopens the credit
//! note (BL-INV-08); the retry is wl-payments' (`FIN.RUL.21`).
//!
//! Decisions: the package `refund` of wl-sales is not built, so a package
//! enrolment is not credited here (`DEFERRED`: #438); the documents are made by
//! the contracts one after the other, not in one transaction, because each
//! contract commits on its own.

use renox::db::{Json, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{CreditLine, InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::payments::{DEFAULT_CURRENCY, RefundRequest, RefundSource, payment_recorder};
use wl_kernel::subscriptions::{CancelSubscription, subscriptions};
use wl_kernel::{LegalEntity, PartyId};

use crate::enrolment::{Enrolment, EnrolmentKind, EnrolmentStatus, Instalment, InstalmentStatus};
use crate::fees;
use crate::lapse;
use crate::refund::net_of_gross;
use crate::texts::english;

/// What a withdrawal has done so far, kept on the enrolment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settlement {
    /// The user who asked for the withdrawal: the recorder of the refunds.
    pub requested_by: Option<i64>,
    /// The student left from `pending_payment`: nothing is credited.
    pub from_pending: bool,
    /// The refund is the whole of what was paid, registration fee included
    /// (a batch `cancel`).
    pub full_refund: bool,
    /// The subscription of a membership was cancelled.
    pub membership_cancelled: bool,
    /// The payment link was cancelled.
    pub link_cancelled: bool,
    /// The invoices closed (a draft discarded, an unpaid invoice credited).
    pub closed_invoices: Vec<i64>,
    /// The invoices a credit note of the refund was made on.
    pub credited_invoices: Vec<i64>,
    /// The credit notes made for the refund.
    pub credit_notes: Vec<i64>,
    /// The credit notes whose refund was asked.
    pub refunded_notes: Vec<i64>,
    /// The refunds asked of wl-payments.
    pub refunds: Vec<i64>,
    /// The credit notes whose balance was given to their refund, so the same
    /// credit is not refunded twice and a failed refund gives it back
    /// (BL-INV-08).
    pub taken_notes: Vec<i64>,
    /// The invoice for the amount owed.
    pub owed_invoice: Option<i64>,
    /// Every step is done.
    pub done: bool,
    /// Why the last run stopped.
    pub problem: Option<String>,
}

/// An invoice of the enrolment that was paid, with the lines a refund may
/// credit: position and gross (with VAT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creditable {
    /// The invoice.
    pub invoice: i64,
    /// What was paid on it, in fils.
    pub paid: i64,
    /// The lines that may be credited: position on the invoice, gross.
    pub lines: Vec<(i64, i64)>,
}

/// How a refund of `refund` fils (with VAT) is credited on `items`, in the
/// order given (the newest invoice first): per invoice, up to what was paid on
/// it and what its lines hold, line by line. Each credit is the net of the
/// gross taken from the line, half-up, so a whole line is credited by its own
/// net. Invoices that take nothing are left out.
pub fn plan_credits(refund: i64, items: &[Creditable]) -> Vec<(i64, Vec<CreditLine>)> {
    let mut left = refund.max(0);
    let mut plan = Vec::new();
    for item in items {
        if left == 0 {
            break;
        }
        let holds: i64 = item.lines.iter().map(|(_, gross)| gross).sum();
        let mut take = left.min(item.paid.max(0)).min(holds);
        if take == 0 {
            continue;
        }
        left -= take;
        let mut lines = Vec::new();
        for (position, gross) in &item.lines {
            if take == 0 {
                break;
            }
            let part = take.min(*gross);
            take -= part;
            let net = net_of_gross(part);
            if net > 0 {
                lines.push(CreditLine::new(*position, net));
            }
        }
        if !lines.is_empty() {
            plan.push((item.invoice, lines));
        }
    }
    plan
}

/// Stores `plan` on enrolment `id`.
async fn store(state: &AppState, id: i64, plan: &Settlement) -> Result {
    Enrolment::query()
        .where_(Enrolment::ID.eq(id))
        .update(
            &state.db,
            &[("settlement", &Json(plan.clone())), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// Whether an invoice in `status` can take a credit note of money that was paid
/// on it: not a draft, and not one that was credited or written off already.
fn can_credit(status: InvoiceStatus) -> bool {
    matches!(
        status,
        InvoiceStatus::Issued
            | InvoiceStatus::PartiallyPaid
            | InvoiceStatus::Paid
            | InvoiceStatus::Overdue
    )
}

/// The invoices of `enrolment` that may be credited, newest first: the later
/// instalments, then the first invoice. The lines are those `fees::raise`
/// drafted: the course fee (or the instalment, or the package), then the
/// registration fee, which only a `full` refund credits.
async fn creditable(
    state: &AppState,
    enrolment: &Enrolment,
    full: bool,
) -> Result<Vec<Creditable>> {
    let books = invoicer(state);
    let rows = Instalment::of_enrolment(&state.db, enrolment.id).await?;
    let mut items = Vec::new();
    for row in rows.iter().rev() {
        let (Some(invoice), true) = (row.invoice_id, row.number > 1) else {
            continue;
        };
        if row.status == InstalmentStatus::Cancelled {
            continue;
        }
        if let Some(summary) = books.find(&state.db, InvoiceId(invoice)).await?
            && can_credit(summary.status)
        {
            items.push(Creditable {
                invoice,
                paid: (summary.gross_total - summary.balance).max(0),
                lines: vec![(1, row.amount)],
            });
        }
    }
    if let Some(invoice) = enrolment.first_invoice_id
        && let Some(summary) = books.find(&state.db, InvoiceId(invoice)).await?
        && can_credit(summary.status)
    {
        let course = match enrolment.kind {
            EnrolmentKind::Course => rows.first().map_or(enrolment.fee(), |r| r.amount),
            EnrolmentKind::Package => enrolment.fee(),
            EnrolmentKind::Membership => 0,
        };
        let mut lines = Vec::new();
        let mut position = 1;
        if course > 0 {
            lines.push((position, course));
            position += 1;
        }
        if enrolment.registration_fee > 0 && full {
            lines.push((position, enrolment.registration_fee));
        }
        items.push(Creditable {
            invoice,
            paid: (summary.gross_total - summary.balance).max(0),
            lines,
        });
    }
    Ok(items)
}

/// The invoices of the enrolment (the first, the instalments), each once.
async fn invoice_ids(state: &AppState, enrolment: &Enrolment) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = enrolment.first_invoice_id.into_iter().collect();
    for row in Instalment::of_enrolment(&state.db, enrolment.id).await? {
        if let Some(id) = row.invoice_id
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Closes what nothing was paid on: a draft is discarded and, unless the
/// student left from `pending_payment`, an issued invoice is credited in full.
async fn close_unpaid(state: &AppState, e: &Enrolment, plan: &mut Settlement) -> Result {
    let books = invoicer(state);
    for id in invoice_ids(state, e).await? {
        if plan.closed_invoices.contains(&id) {
            continue;
        }
        let Some(summary) = books.find(&state.db, InvoiceId(id)).await? else {
            continue;
        };
        match summary.status {
            InvoiceStatus::Draft => {
                books.discard_draft(&state.db, InvoiceId(id)).await?;
            }
            InvoiceStatus::Issued | InvoiceStatus::Overdue
                if !plan.from_pending && summary.balance == summary.gross_total =>
            {
                let reason = english("edtech.settle.unpaid_reason", &[("enrolment", &e.number())]);
                books
                    .credit_in_full(state, InvoiceId(id), reason, plan.requested_by)
                    .await?;
            }
            _ => continue,
        }
        plan.closed_invoices.push(id);
        store(state, e.id, plan).await?;
    }
    Ok(())
}

/// Credits the refund on the paid invoices and asks each credit note back.
async fn credit_and_refund(state: &AppState, e: &Enrolment, plan: &mut Settlement) -> Result {
    let books = invoicer(state);
    let reason = english(
        "edtech.settle.credit_reason",
        &[
            ("enrolment", &e.number()),
            ("reason", &e.withdrawal_reason.clone().unwrap_or_default()),
        ],
    );
    if plan.credit_notes.is_empty() && e.refund_amount > 0 && e.kind != EnrolmentKind::Package {
        let mut items = creditable(state, e, plan.full_refund).await?;
        // The value that moved here with a transfer was paid on the invoices of
        // the enrolment it replaced.
        let mut previous = e.clone();
        for _ in 0..8 {
            let Some(from) = previous.transferred_from_id else {
                break;
            };
            let Some(found) = Enrolment::find(&state.db, from).await? else {
                break;
            };
            items.extend(creditable(state, &found, false).await?);
            previous = found;
        }
        for (invoice, lines) in plan_credits(e.refund_amount, &items) {
            if plan.credited_invoices.contains(&invoice) {
                continue;
            }
            let note = books
                .credit_partly(
                    state,
                    InvoiceId(invoice),
                    lines,
                    reason.clone(),
                    plan.requested_by,
                )
                .await?;
            plan.credited_invoices.push(invoice);
            plan.credit_notes.push(note.0);
            store(state, e.id, plan).await?;
        }
    }
    let notes = plan.credit_notes.clone();
    for note in notes {
        let id = wl_kernel::invoicing::CreditNoteId(note);
        let Some(summary) = books.find_credit_note(&state.db, id).await? else {
            continue;
        };
        if !plan.refunded_notes.contains(&note) {
            if summary.remaining <= 0 {
                continue;
            }
            let entity = match summary.legal_entity_id {
                Some(entity) => entity,
                None => LegalEntity::default_entity(&state.db).await?.id,
            };
            let refund = payment_recorder(state)
                .request_refund(
                    state,
                    RefundRequest {
                        source: RefundSource::CreditNote(id.subject()),
                        party: PartyId(e.payer_party_id),
                        legal_entity_id: entity,
                        amount: summary.remaining,
                        currency: DEFAULT_CURRENCY.to_owned(),
                        method: None,
                        gateway_txn: None,
                        reason: reason.clone(),
                    },
                    plan.requested_by,
                )
                .await?;
            plan.refunded_notes.push(note);
            plan.refunds.push(refund.0);
            store(state, e.id, plan).await?;
        }
        // The refund exists: the credit note gives its balance to it, so the
        // same credit is not refunded twice, and a refund that fails gives
        // it back (BL-INV-08).
        if !plan.taken_notes.contains(&note)
            && summary.remaining > 0
            && let Some(index) = plan.refunded_notes.iter().position(|n| *n == note)
            && let Some(refund) = plan.refunds.get(index).copied()
        {
            books
                .take_credit_for_refund(
                    state,
                    id,
                    wl_kernel::payments::RefundId(refund).subject(),
                    1,
                    summary.remaining,
                )
                .await?;
            plan.taken_notes.push(note);
            store(state, e.id, plan).await?;
        }
    }
    Ok(())
}

/// Drafts the one invoice for the amount owed.
async fn bill_owed(state: &AppState, e: &Enrolment, plan: &mut Settlement) -> Result {
    if e.amount_owed <= 0 || plan.owed_invoice.is_some() {
        return Ok(());
    }
    let description = english("edtech.fees.owed", &[("enrolment", &e.number())]);
    let line = fees::line(&description, e.amount_owed, e);
    if let Some(invoice) = fees::bill(state, e, vec![line]).await? {
        plan.owed_invoice = Some(invoice.0);
        store(state, e.id, plan).await?;
    }
    Ok(())
}

/// The steps in order; the first that fails stops the run.
async fn steps(state: &AppState, e: &Enrolment, plan: &mut Settlement) -> Result {
    if e.kind == EnrolmentKind::Membership && !plan.membership_cancelled {
        let on = e.withdrawn_on.unwrap_or_else(today);
        let _ended = subscriptions(state)
            .cancel(state, &e.subject(), CancelSubscription::new(on))
            .await?;
        plan.membership_cancelled = true;
        store(state, e.id, plan).await?;
    }
    if !plan.link_cancelled {
        lapse::cancel_link(state, e).await;
        plan.link_cancelled = true;
        store(state, e.id, plan).await?;
    }
    close_unpaid(state, e, plan).await?;
    if !plan.from_pending {
        credit_and_refund(state, e, plan).await?;
        bill_owed(state, e, plan).await?;
    }
    Ok(())
}

/// Settles the money of withdrawn enrolment `id`: the steps not done yet are
/// done, and the result is kept on the enrolment. One already settled, or not
/// withdrawn, is left as it is.
///
/// # Errors
///
/// The database's errors and the contracts' (the reason is also stored on the
/// enrolment, [`Settlement::problem`], and a later call repeats the step).
pub async fn settle(state: &AppState, id: i64) -> Result<Enrolment> {
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    if enrolment.status != EnrolmentStatus::Withdrawn || enrolment.settlement.0.done {
        return Ok(enrolment);
    }
    let mut plan = enrolment.settlement.0.clone();
    match steps(state, &enrolment, &mut plan).await {
        Ok(()) => {
            plan.done = true;
            plan.problem = None;
            store(state, id, &plan).await?;
            Enrolment::find_or_404(&state.db, id).await
        }
        Err(error) => {
            plan.problem = Some(error.to_string());
            store(state, id, &plan).await?;
            Err(error)
        }
    }
}

/// Settles after a commit: a failure is kept on the enrolment and shown on its
/// page, and does not undo the withdrawal or fail the approval that caused it.
pub async fn settle_quietly(state: &AppState, id: i64) {
    if let Err(error) = settle(state, id).await {
        eprintln!("edtech: the money of enrolment {id} is not settled yet: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(invoice: i64, paid: i64, lines: &[(i64, i64)]) -> Creditable {
        Creditable {
            invoice,
            paid,
            lines: lines.to_vec(),
        }
    }

    fn nets(plan: &[(i64, Vec<CreditLine>)]) -> Vec<(i64, i64, i64)> {
        plan.iter()
            .flat_map(|(invoice, lines)| {
                lines.iter().map(move |l| (*invoice, l.invoice_line, l.net))
            })
            .collect()
    }

    #[test]
    fn a_refund_on_one_paid_invoice_is_the_net_of_its_gross() {
        // AED 1,785.00 refunded of AED 2,520.00 paid upfront.
        let plan = plan_credits(178_500, &[item(100, 252_000, &[(1, 252_000)])]);
        assert_eq!(nets(&plan), [(100, 1, 170_000)]);
    }

    #[test]
    fn the_newest_invoice_is_credited_first_and_never_above_what_was_paid() {
        let items = [
            item(102, 60_000, &[(1, 126_000)]),
            item(100, 126_000, &[(1, 126_000)]),
        ];
        let plan = plan_credits(100_000, &items);
        assert_eq!(nets(&plan), [(102, 1, 57_143), (100, 1, 38_095)]);
    }

    #[test]
    fn a_whole_line_is_credited_by_its_own_net_and_the_registration_line_is_last() {
        // Full refund of AED 2,520.00 + AED 210.00 on one invoice.
        let plan = plan_credits(273_000, &[item(100, 273_000, &[(1, 252_000), (2, 21_000)])]);
        assert_eq!(nets(&plan), [(100, 1, 240_000), (100, 2, 20_000)]);
    }

    #[test]
    fn nothing_is_credited_for_nothing_paid_or_no_refund() {
        assert!(plan_credits(0, &[item(1, 100, &[(1, 100)])]).is_empty());
        assert!(plan_credits(100, &[item(1, 0, &[(1, 100)])]).is_empty());
        assert!(plan_credits(100, &[]).is_empty());
    }
}
