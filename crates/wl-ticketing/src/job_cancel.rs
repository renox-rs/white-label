//! Cancelling a job after work started and guarding compliance visits (01
//! Property §3.3 `PRP.ACL.15`, §4.2 rows "The job is cancelled after work
//! started"; BL-PRP-24).
//!
//! - **Cancel with a charge.** A coordinator cancels a job (also from
//!   `on_hold`) with a reason. Nothing is charged to the requester unless the
//!   coordinator marks the job chargeable with a reason and charge lines
//!   ([`cancel_job`]); the lines are kept, the ticket becomes chargeable and
//!   one draft invoice is made through the `Invoicer` as for `confirm`
//!   ([`crate::charge::ensure_draft`]). The draft is issued at once by
//!   wl-invoicing's setting for the ticket source (automatic issue), there is
//!   no confirmation step for a cancelled job. The vendor's bill for work
//!   already done references the cancelled ticket and goes to the owner.
//! - **A compliance-critical planned visit** (a planned visit of a category
//!   marked compliance-critical) is not skipped: a user's `cancel` is refused
//!   ([`ensure_not_skipped`]); only the system's `auto_cancel` of `WRK.RUL.18`
//!   cancels it, for visits due after the contract's end. It is rescheduled
//!   with a reason instead ([`reschedule`]), kept in the ticket's comments.
//! - **Service report.** Such a visit is resolved only with the vendor's service
//!   report attached ([`attach_service_report`], [`ensure_service_report`]);
//!   the coordinator may attach it for the vendor.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;

use crate::charge::{self, ChargeInput, ChargeLine};
use crate::comments::{Comment, CommentVisibility};
use crate::config::Category;
use crate::permissions;
use crate::texts::english;
use crate::ticket::Ticket;
use crate::work;

/// A 422 with the English text of `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key).to_owned())
}

/// Whether `ticket` is a planned visit of a compliance-critical category.
///
/// # Errors
///
/// The database's errors.
pub async fn is_compliance_visit(db: &Db, ticket: &Ticket) -> Result<bool> {
    if ticket.planned_due_date.is_none() {
        return Ok(false);
    }
    Ok(Category::find(db, ticket.category_id)
        .await?
        .is_some_and(|category| category.compliance_critical))
}

/// A user cannot cancel a compliance-critical planned visit.
///
/// # Errors
///
/// `Guard` (422) for such a visit.
pub async fn ensure_not_skipped(
    state: &AppState,
    ticket: &Ticket,
) -> std::result::Result<(), TransitionError> {
    if is_compliance_visit(&state.db, ticket).await? {
        return Err(TransitionError::Guard(
            english("wl_ticketing.visit.no_skip").to_owned(),
        ));
    }
    Ok(())
}

/// A compliance-critical planned visit is resolved only with the vendor's
/// service report attached.
///
/// # Errors
///
/// `Guard` (422) without it.
pub async fn ensure_service_report(
    state: &AppState,
    ticket: &Ticket,
) -> std::result::Result<(), TransitionError> {
    let attached = ticket
        .service_report
        .as_deref()
        .is_some_and(|reference| !reference.trim().is_empty());
    if !attached && is_compliance_visit(&state.db, ticket).await? {
        return Err(TransitionError::Guard(
            english("wl_ticketing.visit.report_needed").to_owned(),
        ));
    }
    Ok(())
}

/// An internal comment of `user` on ticket `id`.
async fn note(db: &Db, id: i64, user: &AuthUser, text: String) -> Result {
    Comment::create(
        db,
        Comment {
            ticket_id: id,
            author_user_id: Some(user.id),
            visibility: CommentVisibility::Internal,
            text,
            ..Comment::default()
        },
    )
    .await?;
    Ok(())
}

/// `user` attaches the reference of the vendor's service report to the visit
/// `id` (the coordinator on the vendor's behalf, or the assignee).
///
/// # Errors
///
/// `Forbidden` unless [`charge::may_change`]; 422 for a blank reference or a
/// ticket that is not a planned visit.
pub async fn attach_service_report(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reference: &str,
) -> Result<Ticket> {
    let ticket = Ticket::find_or_404(&state.db, id).await?;
    if !charge::may_change(&ticket, user) {
        return Err(Error::Forbidden);
    }
    let reference = reference.trim();
    if reference.is_empty() || ticket.planned_due_date.is_none() {
        return Err(refused("wl_ticketing.visit.report_blank"));
    }
    Ticket::query()
        .where_(Ticket::ID.eq(id))
        .update(
            &state.db,
            &[
                ("service_report", &Some(reference.to_owned())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    note(
        &state.db,
        id,
        user,
        english("wl_ticketing.visit.report_attached").replace(":reference", reference),
    )
    .await?;
    Ticket::find_or_404(&state.db, id).await
}

/// `user` moves the due date of the planned visit `id` to `to`, with a
/// reason kept in the ticket's comments (`PRP.ACL.15`: rescheduled, not
/// skipped).
///
/// # Errors
///
/// `Forbidden` without `tickets.assign`; 422 for a blank reason, a ticket
/// that is not an open planned visit.
pub async fn reschedule(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    to: NaiveDate,
    reason: &str,
) -> Result<Ticket> {
    if !user.has_permission(permissions::ASSIGN) {
        return Err(Error::Forbidden);
    }
    let ticket = Ticket::find_or_404(&state.db, id).await?;
    let reason = reason.trim();
    let Some(from) = ticket.planned_due_date else {
        return Err(refused("wl_ticketing.visit.not_planned"));
    };
    if reason.is_empty() || !ticket.is_open() {
        return Err(refused("wl_ticketing.visit.reschedule_refused"));
    }
    let none: Option<renox::db::DateTime> = None;
    Ticket::query()
        .where_(Ticket::ID.eq(id))
        .update(
            &state.db,
            &[
                ("planned_due_date", &Some(to)),
                ("missed_notified_at", &none),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    note(
        &state.db,
        id,
        user,
        english("wl_ticketing.visit.rescheduled")
            .replace(":from", &from.to_string())
            .replace(":to", &to.to_string())
            .replace(":reason", reason),
    )
    .await?;
    Ticket::find_or_404(&state.db, id).await
}

/// What the coordinator ticks to charge the requester for work done on a
/// cancelled job.
#[derive(Debug, Clone)]
pub struct ChargeOnCancel {
    /// Why the requester is charged.
    pub reason: String,
    /// The charge lines.
    pub lines: Vec<ChargeInput>,
}

/// `user` cancels `ticket` for `reason`. With `charge`, the lines are kept on
/// the ticket and one draft invoice is made; the id of the draft is returned.
/// Without it nothing is invoiced.
///
/// # Errors
///
/// As [`work::cancel`]; `Guard` (422) for a compliance-critical planned visit
/// (it is rescheduled, not skipped), and for a charge without a reason, without
/// a line or with a line that is not valid.
pub async fn cancel_job(
    state: &AppState,
    user: &AuthUser,
    ticket: &Ticket,
    reason: &str,
    charge: Option<ChargeOnCancel>,
) -> std::result::Result<Option<i64>, TransitionError> {
    if let Some(wanted) = &charge {
        let invalid = wanted.reason.trim().is_empty()
            || wanted.lines.is_empty()
            || wanted.lines.iter().any(|line| charge::input_problem(line).is_some());
        if invalid {
            return Err(TransitionError::Guard(
                english("wl_ticketing.visit.charge_invalid").to_owned(),
            ));
        }
    }
    work::cancel(state, user, ticket, reason).await?;
    let Some(wanted) = charge else {
        return Ok(None);
    };
    Ticket::query()
        .where_(Ticket::ID.eq(ticket.id))
        .update(
            &state.db,
            &[("chargeable", &true), ("updated_at", &renox::db::now())],
        )
        .await?;
    for (index, line) in wanted.lines.iter().enumerate() {
        ChargeLine::create(
            &state.db,
            ChargeLine {
                ticket_id: ticket.id,
                position: i64::try_from(index).unwrap_or(0) + 1,
                description: line.description.trim().to_owned(),
                quantity_milli: line.quantity_milli,
                unit_price: line.unit_price,
                vat_category: line.vat_category,
                ..ChargeLine::default()
            },
        )
        .await?;
    }
    note(
        &state.db,
        ticket.id,
        user,
        english("wl_ticketing.visit.charged").replace(":reason", wanted.reason.trim()),
    )
    .await?;
    Ok(charge::ensure_draft(state, ticket.id).await?)
}
