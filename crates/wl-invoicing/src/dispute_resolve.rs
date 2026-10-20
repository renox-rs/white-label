//! Resolving an invoice dispute (wl-invoicing §5.10 `resolve_dispute`, #67
//! A10-A13): [`resolve`] closes the open dispute of an invoice `upheld` or
//! `credited`.
//!
//! The invoice may be in any state while it has an open dispute, and the
//! note is required. Who may: `upheld` needs `invoices.dispute`, `credited`
//! also `invoices.credit`; the system may resolve `upheld`.
//!
//! - `upheld` clears the flag, stores the outcome, and sets the grace period
//!   (`dispute_grace_days`, 7 by default) on the dispute and on the invoice,
//!   which `FIN.RUL.01` honours; the due date never moves.
//! - `credited` turns the agreed gross amount into a credit note through
//!   #71: a `full` note when it equals the gross total of an invoice with
//!   nothing allocated, else a partial one spread over the disputed lines
//!   ([`dispute_credit::split`]). A line billed from a timesheet is refused
//!   unless the resolve names an adjustment timesheet that gave a credit note
//!   against the invoice ([`crate::usage_adjustment`]): the dispute then links
//!   to that credit note and makes no second one. The draft is
//!   checked against the `invoicing.dispute_credit` policy (`FIN.ACL.07`):
//!   no request needed, and the note is issued at once; otherwise the
//!   request is pending, the dispute keeps the draft and the request, and
//!   the flag and ticket stay until the final approval
//!   ([`dispute_credit::approved_in`]). After a rejection the next resolve
//!   discards the old draft first.
//!
//! After the commit `InvoiceDisputeResolved` is emitted; its listener
//! resolves the ticket and tells the party (`invoicing.dispute_resolved`).

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::TransitionError;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest, Require, Required};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    CreditLine, CreditNoteStatus, DisputeBy, DisputeOutcome, InvoiceDisputeResolved, InvoiceStatus,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyRequest, fill};
use wl_kernel::scopes;
use wl_kernel::tickets::{self, TicketId};

use crate::allocate::amount_text;
use crate::config_of;
use crate::credit_note::{CreditNote, CreditNoteKind};
use crate::credit_notes::{
    By, Outcomes, credited_by_line, discard, insert_in, issue_in, issue_input, prepare,
};
use crate::dispute::{Dispute, dispute_type, holds};
use crate::dispute_credit::{
    ACTION, Payload, SplitLine, finish_in, settle_in, split, timesheet_guard,
};
use crate::invoice::Invoice;
use crate::invoicer::lines_of;
use crate::line::InvoiceLine;
use crate::permissions;
use crate::texts::english;
use crate::usage_adjustment;
use crate::vat::remaining_credit;

/// The notification type that tells the party its dispute was resolved.
pub const RESOLVED: &str = "invoicing.dispute_resolved";

/// What a resolve is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveInput {
    /// How the dispute is settled.
    pub outcome: DisputeOutcome,
    /// The resolution note (required).
    pub note: String,
    /// For `credited`: the agreed gross amount, in fils.
    pub agreed_amount: Option<i64>,
    /// For `credited` on a timesheet line: the approved adjustment timesheet
    /// (its credit note, if it gave one against the invoice, is the resolution).
    pub adjustment_timesheet: Option<Subject>,
}

impl ResolveInput {
    /// `upheld` with `note`.
    pub fn upheld(note: &str) -> Self {
        ResolveInput {
            outcome: DisputeOutcome::Upheld,
            note: note.to_owned(),
            agreed_amount: None,
            adjustment_timesheet: None,
        }
    }

    /// `credited` with `note` for the agreed gross `amount`.
    pub fn credited(note: &str, amount: i64) -> Self {
        ResolveInput {
            outcome: DisputeOutcome::Credited,
            note: note.to_owned(),
            agreed_amount: Some(amount),
            adjustment_timesheet: None,
        }
    }
}

/// What a resolve did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The dispute.
    pub dispute: i64,
    /// The credit note (issued, or the draft while a request is pending).
    pub credit_note: Option<i64>,
    /// The approval request, when the credit waits for one: the dispute is
    /// then still open.
    pub pending_request: Option<i64>,
}

impl Resolution {
    /// Whether the dispute is resolved now.
    pub fn is_resolved(&self) -> bool {
        self.pending_request.is_none()
    }
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The permissions a user needs to settle with `outcome`.
pub fn permissions_for(outcome: DisputeOutcome) -> &'static [&'static str] {
    match outcome {
        DisputeOutcome::Upheld => &[permissions::DISPUTE],
        DisputeOutcome::Credited => &[permissions::DISPUTE, permissions::CREDIT],
    }
}

/// The last day chasing stays paused after an `upheld` on `on`.
pub fn grace_until(on: NaiveDate, days: u32) -> NaiveDate {
    on + Days::new(u64::from(days))
}

/// `date` as `18 Oct 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// Who may resolve with `outcome`; the user's id when it is a user. A 403
/// otherwise.
async fn authorize(
    state: &AppState,
    invoice: &Invoice,
    outcome: DisputeOutcome,
    by: DisputeBy,
) -> Result<Option<i64>> {
    match by {
        DisputeBy::System if outcome == DisputeOutcome::Upheld => Ok(None),
        DisputeBy::User { id } => {
            let scope = scopes::scope_of(&state.db, &invoice.subject()).await?;
            for permission in permissions_for(outcome) {
                if !holds(&state.db, id, permission, &scope).await? {
                    return Err(Error::Forbidden);
                }
            }
            Ok(Some(id))
        }
        _ => Err(Error::Forbidden),
    }
}

/// Discards the draft credit note a rejected, expired or withdrawn request
/// left on `dispute` (#71 A7); an issued note stays.
async fn discard_old_draft(state: &AppState, dispute: &Dispute) -> Result {
    if let Some(id) = dispute.credit_note_id
        && let Some(note) = CreditNote::find(&state.db, id).await?
        && note.status == CreditNoteStatus::Draft
    {
        discard(&state.db, &note).await?;
    }
    Ok(())
}

/// Settles the open dispute of `invoice_id` as `by`. See the module docs.
///
/// # Errors
///
/// 403 without the permissions of the outcome; 404 for an unknown invoice;
/// 422 when the invoice has no open dispute, a request for it is pending,
/// the note is blank, the amount is missing, above 0 or above what the
/// disputed lines can still be credited by, a line needs a timesheet, or
/// the credit services refuse.
pub async fn resolve(
    state: &AppState,
    invoice_id: i64,
    input: ResolveInput,
    by: DisputeBy,
) -> Result<Resolution> {
    let invoice = Invoice::find_or_404(&state.db, invoice_id).await?;
    let user = authorize(state, &invoice, input.outcome, by).await?;
    let dispute = match invoice.dispute_id {
        Some(id) => Dispute::find(&state.db, id).await?.filter(Dispute::is_open),
        None => None,
    }
    .ok_or_else(|| refused("The invoice has no open dispute."))?;
    let note = input.note.trim();
    if note.is_empty() {
        return Err(refused("Give a note for the resolution."));
    }
    if let Some(id) = dispute.approval_request_id
        && ApprovalRequest::find(&state.db, id)
            .await?
            .is_some_and(|request| request.is_pending())
    {
        return Err(refused("A credit for this dispute waits for approval."));
    }
    match input.outcome {
        DisputeOutcome::Upheld => uphold(state, &invoice, &dispute, note, user).await,
        DisputeOutcome::Credited => {
            let user = user.ok_or(Error::Forbidden)?;
            credit(state, &invoice, &dispute, &input, note, user).await
        }
    }
}

/// `upheld`: the flag is cleared and chasing stays paused through the grace
/// period.
async fn uphold(
    state: &AppState,
    invoice: &Invoice,
    dispute: &Dispute,
    note: &str,
    user: Option<i64>,
) -> Result<Resolution> {
    discard_old_draft(state, dispute).await?;
    let on = today();
    let until = grace_until(on, config_of(state).dispute_grace_days);
    let now = renox::db::now();
    let mut tx = state.db.begin().await?;
    let cleared = renox::db::sql(format!(
        "UPDATE {} SET disputed = 0, dispute_id = NULL, dispute_grace_until = ?, updated_at = ? \
         WHERE id = ? AND disputed = 1 AND dispute_id = ?",
        Invoice::TABLE
    ))
    .bind(until)
    .bind(now)
    .bind(invoice.id)
    .bind(dispute.id)
    .execute(&mut tx)
    .await?;
    if cleared == 0 {
        return Err(refused("The dispute was already resolved."));
    }
    let settled = renox::db::sql(format!(
        "UPDATE {} SET outcome = ?, resolution_note = ?, credit_note_id = NULL, \
         agreed_amount = NULL, approval_request_id = NULL, resolved_by_user_id = ?, \
         resolved_at = ?, grace_until = ?, updated_at = ? WHERE id = ? AND outcome IS NULL",
        Dispute::TABLE
    ))
    .bind(DisputeOutcome::Upheld.as_str())
    .bind(note)
    .bind(user)
    .bind(now)
    .bind(until)
    .bind(now)
    .bind(dispute.id)
    .execute(&mut tx)
    .await?;
    if settled == 0 {
        return Err(refused("The dispute was already resolved."));
    }
    tx.commit().await?;
    announce(state, dispute.id).await?;
    Ok(Resolution {
        dispute: dispute.id,
        credit_note: None,
        pending_request: None,
    })
}

/// The lines the credit is about: the disputed ones, or every line when the
/// dispute is not tied to lines.
fn disputed_lines<'a>(dispute: &Dispute, lines: &'a [InvoiceLine]) -> Vec<&'a InvoiceLine> {
    lines
        .iter()
        .filter(|line| {
            dispute.positions().is_empty()
                || i32::try_from(line.position).is_ok_and(|p| dispute.positions().contains(&p))
        })
        .collect()
}

/// `credited`: the agreed amount becomes a credit note, issued at once or
/// after the `FIN.ACL.07` approval.
async fn credit(
    state: &AppState,
    invoice: &Invoice,
    dispute: &Dispute,
    input: &ResolveInput,
    note: &str,
    user: i64,
) -> Result<Resolution> {
    // The credit note an approved adjustment timesheet already gave against
    // this invoice is the resolution (§5.2 step 5, §5.10 step 4).
    if let Some(sheet) = &input.adjustment_timesheet
        && let Some(note_id) =
            usage_adjustment::credit_note_of(&state.db, sheet, invoice.id).await?
    {
        return link_adjustment_credit(state, invoice, dispute, input, note, user, note_id).await;
    }
    let agreed = input
        .agreed_amount
        .filter(|amount| *amount > 0)
        .ok_or_else(|| refused("Give the agreed amount, above 0."))?;
    let lines = lines_of(&state.db, invoice.id).await?;
    // Every line when the whole invoice is credited, else the disputed ones.
    let full = agreed == invoice.gross_total
        && invoice.allocated_total == 0
        && [InvoiceStatus::Issued, InvoiceStatus::Overdue].contains(&invoice.status);
    let in_scope: Vec<&InvoiceLine> = if full {
        lines.iter().collect()
    } else {
        disputed_lines(dispute, &lines)
    };
    timesheet_guard(in_scope.iter().map(|line| line.origin_type.as_deref())).map_err(refused)?;
    let (requested, kind) = if full {
        let requested = lines
            .iter()
            .filter(|line| line.net_amount > 0)
            .map(|line| CreditLine::new(line.position, line.net_amount))
            .collect();
        (requested, CreditNoteKind::Full)
    } else {
        let credited = credited_by_line(&state.db, invoice.id).await?;
        let split_lines: Vec<SplitLine> = in_scope
            .iter()
            .map(|line| SplitLine {
                position: line.position,
                category: line.vat_category,
                remaining_net: remaining_credit(
                    line.net_amount,
                    credited.get(&line.id).copied().unwrap_or(0),
                ),
            })
            .collect();
        (
            split(&split_lines, agreed).map_err(refused)?,
            CreditNoteKind::Partial,
        )
    };

    // A draft left by a rejected request goes first (#71 A7).
    discard_old_draft(state, dispute).await?;
    let prepared = prepare(&state.db, invoice.id, &requested, note, kind, Some(user)).await?;
    let issue = issue_input(&state.db, &prepared.invoice, prepared.lines.len(), None).await?;
    let amount = prepared.note.gross_total;

    let into_error = TransitionError::into_error;
    let by = By::System(Some(user));
    let mut outcomes = Outcomes::default();
    let mut tx = state.db.begin().await?;
    let (draft, _) = insert_in(&mut tx, prepared).await?;
    let now = renox::db::now();
    let held = renox::db::sql(format!(
        "UPDATE {} SET resolution_note = ?, agreed_amount = ?, credit_note_id = ?, \
         approval_request_id = NULL, updated_at = ? WHERE id = ? AND outcome IS NULL",
        Dispute::TABLE
    ))
    .bind(note)
    .bind(agreed)
    .bind(draft.id)
    .bind(now)
    .bind(dispute.id)
    .execute(&mut tx)
    .await?;
    if held == 0 {
        return Err(refused("The dispute was already resolved."));
    }
    let payload = Payload {
        amount,
        dispute: dispute.id,
    };
    let require = Require::new(ACTION, draft.subject(), "issue", user, "draft").payload(json!({
        "amount": payload.amount,
        "dispute": payload.dispute,
    }));
    match approvals::require_in(&mut tx, &require).await? {
        Required::Allowed => {
            let issued = issue_in(&mut tx, &draft, &issue, &by, &mut outcomes)
                .await
                .map_err(into_error)?;
            finish_in(&mut tx, issued, dispute.id, Some(user), &by, &mut outcomes)
                .await
                .map_err(into_error)?;
            tx.commit().await?;
            outcomes.emit(state).await.map_err(into_error)?;
            announce(state, dispute.id).await?;
            Ok(Resolution {
                dispute: dispute.id,
                credit_note: Some(draft.id),
                pending_request: None,
            })
        }
        Required::Pending(request) => {
            renox::db::sql(format!(
                "UPDATE {} SET approval_request_id = ?, updated_at = ? WHERE id = ?",
                Dispute::TABLE
            ))
            .bind(request.id)
            .bind(now)
            .bind(dispute.id)
            .execute(&mut tx)
            .await?;
            tx.commit().await?;
            approvals::announce_request(state, &request).await?;
            Ok(Resolution {
                dispute: dispute.id,
                credit_note: Some(draft.id),
                pending_request: Some(request.id),
            })
        }
    }
}

/// `credited` with the credit note that an approved adjustment timesheet
/// gave against the invoice (`credit_note_id`): the dispute links to it, no
/// second credit is made, and the `FIN.ACL.07` limit does not apply, since
/// the hours were approved as an adjustment. An agreed amount, when given,
/// must be that note's total.
async fn link_adjustment_credit(
    state: &AppState,
    invoice: &Invoice,
    dispute: &Dispute,
    input: &ResolveInput,
    note: &str,
    user: i64,
    credit_note_id: i64,
) -> Result<Resolution> {
    let credit = CreditNote::find_or_404(&state.db, credit_note_id).await?;
    if input
        .agreed_amount
        .is_some_and(|amount| amount != credit.gross_total)
    {
        return Err(refused(format!(
            "The credit note of the adjustment timesheet is {} fils with VAT.",
            credit.gross_total
        )));
    }
    discard_old_draft(state, dispute).await?;
    let mut tx = state.db.begin().await?;
    let held = renox::db::sql(format!(
        "UPDATE {} SET resolution_note = ?, agreed_amount = ?, approval_request_id = NULL, \
         updated_at = ? WHERE id = ? AND outcome IS NULL",
        Dispute::TABLE
    ))
    .bind(note)
    .bind(credit.gross_total)
    .bind(renox::db::now())
    .bind(dispute.id)
    .execute(&mut tx)
    .await?;
    if held == 0 {
        return Err(refused("The dispute was already resolved."));
    }
    settle_in(&mut tx, invoice.id, dispute.id, credit.id, Some(user)).await?;
    tx.commit().await?;
    announce(state, dispute.id).await?;
    Ok(Resolution {
        dispute: dispute.id,
        credit_note: Some(credit.id),
        pending_request: None,
    })
}

/// Emits `InvoiceDisputeResolved` for the now resolved dispute `dispute_id`,
/// after the commit that resolved it.
pub(crate) async fn announce(state: &AppState, dispute_id: i64) -> Result {
    let dispute = Dispute::find_or_404(&state.db, dispute_id).await?;
    let Some(outcome) = dispute.outcome else {
        return Ok(());
    };
    let invoice = Invoice::find_or_404(&state.db, dispute.invoice_id).await?;
    let note = match dispute.credit_note_id {
        Some(id) if outcome == DisputeOutcome::Credited => CreditNote::find(&state.db, id).await?,
        _ => None,
    };
    state
        .emit(InvoiceDisputeResolved {
            invoice: invoice.invoice_id(),
            party: invoice.party(),
            dispute: dispute.id,
            outcome,
            credited_amount: note.as_ref().map(|n| n.gross_total),
            credit_note: note.as_ref().map(CreditNote::subject),
        })
        .await
}

/// The template variables of the party's notice.
fn data(invoice: &Invoice, dispute: &Dispute, note: Option<&CreditNote>) -> Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    let result = match (dispute.outcome, note) {
        (Some(DisputeOutcome::Credited), Some(note)) => fill(
            english("wl_invoicing.dispute.result_credited"),
            &json!({
                "credit_note": note.number.clone().unwrap_or_default(),
                "amount": amount_text(&note.currency, note.gross_total),
            }),
        ),
        _ => fill(
            english("wl_invoicing.dispute.result_upheld"),
            &json!({
                "grace_until": dispute.grace_until.map(date_text).unwrap_or_default(),
            }),
        ),
    };
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "result": result,
        "note": dispute.resolution_note.clone().unwrap_or_default(),
        "link": format!("/invoices/{}", invoice.id),
    })
}

/// After the commit: resolves the dispute's ticket and tells the party.
async fn after_resolved(state: &AppState, event: &InvoiceDisputeResolved) -> Result {
    let dispute = Dispute::find_or_404(&state.db, event.dispute).await?;
    let invoice = Invoice::find_or_404(&state.db, dispute.invoice_id).await?;
    if let Some(ticket) = dispute.ticket_id {
        let note = dispute.resolution_note.clone().unwrap_or_default();
        tickets::task_creator(state)
            .resolve_from_source(state, TicketId(ticket), &note)
            .await?;
    }
    let credit = match event.credit_note.as_ref() {
        Some(subject) => CreditNote::find(&state.db, subject.subject_id).await?,
        None => None,
    };
    let notice = NotifyRequest::new(RESOLVED, invoice.subject())
        .to_party(event.party)
        .data(data(&invoice, &dispute, credit.as_ref()));
    notify::notifier(state).notify(state, notice).await?;
    Ok(())
}

/// The notification type [`RESOLVED`] (to the party).
pub fn notification_type() -> NotificationTypeDef {
    dispute_type(RESOLVED, "resolved", Category::Transactional)
}

/// Registers the type with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

/// Registers the listener on `InvoiceDisputeResolved`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: InvoiceDisputeResolved, state: AppState| async move {
            after_resolved(&state, &event).await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    #[test]
    fn credited_needs_the_credit_permission_besides_the_dispute_one() {
        assert_eq!(
            permissions_for(DisputeOutcome::Upheld),
            [permissions::DISPUTE]
        );
        assert_eq!(
            permissions_for(DisputeOutcome::Credited),
            [permissions::DISPUTE, permissions::CREDIT]
        );
    }

    #[test]
    fn the_grace_period_ends_the_given_days_after_the_resolution() {
        assert_eq!(grace_until(day(20), 7), day(27));
        assert_eq!(grace_until(day(20), 0), day(20));
        assert_eq!(date_text(day(27)), "27 Nov 2026");
    }

    #[test]
    fn the_inputs_name_their_outcome() {
        let upheld = ResolveInput::upheld("Checked");
        assert_eq!(
            (upheld.outcome, upheld.agreed_amount),
            (DisputeOutcome::Upheld, None)
        );
        let credited = ResolveInput::credited("Agreed", 120_000);
        assert_eq!(
            (credited.outcome, credited.agreed_amount),
            (DisputeOutcome::Credited, Some(120_000))
        );
    }

    #[test]
    fn a_dispute_without_lines_is_about_every_line() {
        let lines: Vec<InvoiceLine> = (1..=3)
            .map(|position| InvoiceLine {
                position,
                ..Default::default()
            })
            .collect();
        let mut dispute = Dispute::default();
        assert_eq!(disputed_lines(&dispute, &lines).len(), 3);
        dispute.lines = renox::db::Json(vec![2, 3]);
        let picked: Vec<i64> = disputed_lines(&dispute, &lines)
            .iter()
            .map(|l| l.position)
            .collect();
        assert_eq!(picked, [2, 3]);
    }

    #[test]
    fn the_party_notice_tells_the_outcome() {
        let invoice = Invoice {
            id: 5,
            number: Some("INV-1".into()),
            ..Default::default()
        };
        let upheld = Dispute {
            outcome: Some(DisputeOutcome::Upheld),
            grace_until: Some(day(27)),
            resolution_note: Some("Hours verified".into()),
            ..Default::default()
        };
        let text = data(&invoice, &upheld, None);
        assert_eq!(
            text["result"],
            "The invoice stands. Payment reminders resume after 27 Nov 2026."
        );
        assert_eq!(text["note"], "Hours verified");
        let credited = Dispute {
            outcome: Some(DisputeOutcome::Credited),
            ..Default::default()
        };
        let note = CreditNote {
            number: Some("CN-1".into()),
            currency: "AED".into(),
            gross_total: 120_000,
            ..Default::default()
        };
        let text = data(&invoice, &credited, Some(&note));
        assert_eq!(
            text["result"],
            "We issued credit note CN-1 for AED 1,200.00."
        );
        assert_eq!(notification_type().key, RESOLVED);
    }
}
