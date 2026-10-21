//! A disputed delivery under review (wl-delivery §4.1 `open_review`,
//! `close_review`, §5.5, `WRK.ACL.06`, issue #160).
//!
//! - [`open_review`] puts a `delivered` delivery under review for a user with
//!   `deliveries.manage`: a reason is needed and a ticket is opened (through
//!   the kernel's [`TaskCreator`]; without `wl-ticketing` there is none and
//!   the review runs without), the frozen flag is set (the cash on delivery
//!   is held out of remittances and settlements, the source does not bill the
//!   delivery) and `DeliveryDisputed` is emitted.
//! - [`open_review_as_system`] is the same for a dispute ticket that the
//!   recipient or the client raised: the ticket is the review's ticket. A
//!   delivery that is already under review is not opened again; the new
//!   complaint is linked to the existing ticket with a comment.
//! - [`close_review`] closes it with the outcome *upheld* and a required note:
//!   the delivery is `delivered` again, the flag is cleared, the ticket is
//!   resolved in the same action when it is still open, and
//!   `DeliveryDisputeClosed` is emitted. Any other outcome is a reversal
//!   (BL-DLV-05) or a loss (BL-DLV-06).
//!
//! The proof is not touched while the delivery is under review (the module
//! has no edit route for it). The ticket is opened before the transition and
//! resolved after its commit, because the ticket service writes through its
//! own connection; a race that loses the transition leaves an unlinked
//! ticket, which the dispatcher closes.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{DeliveryDisputeClosed, DeliveryDisputed, ReviewOutcome};
use wl_kernel::tickets::{self, OpenTicket, TaskComment, TicketChannel, TicketId};

use crate::delivery::{Delivery, DeliveryStatus};
use crate::permissions;
use crate::texts::english;
use crate::trace;

/// `open_review`: delivered → under_review, by a user.
pub const OPEN_REVIEW: &str = "open_review";
/// `open_review`, run by the system for a dispute ticket.
pub const OPEN_REVIEW_SYSTEM: &str = "open_review_system";
/// `close_review`: under_review → delivered.
pub const CLOSE_REVIEW: &str = "close_review";

/// The ticket category of a review (and of a claim): the platform category
/// `wl-ticketing` seeds with `delivery`.
pub const TICKET_CATEGORY: &str = "delivery_claim";

/// What `open_review` runs on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct OpenInput {
    /// The review's ticket.
    pub ticket: Option<i64>,
    /// Whether ticketing is off (no ticket could be opened).
    pub ticketing_off: bool,
}

/// What `close_review` runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CloseInput {
    pub outcome: ReviewOutcome,
}

fn open_input(ctx: &TransitionCtx) -> Option<OpenInput> {
    ctx.input_as::<OpenInput>().ok()
}

/// The guard "linked ticket": the review has a ticket, or ticketing is off.
pub(crate) fn ticket_linked(input: Option<&OpenInput>) -> std::result::Result<(), String> {
    match input {
        Some(input) if input.ticket.is_some() || input.ticketing_off => Ok(()),
        _ => Err("Link the review's ticket first.".to_owned()),
    }
}

/// What both open transitions share.
fn opening(transition: Transition<Delivery>) -> Transition<Delivery> {
    transition
        .from([DeliveryStatus::Delivered])
        .to(DeliveryStatus::UnderReview)
        .reason_required()
        .guard("linked_ticket", |_delivery: &Delivery, ctx| {
            let checked = ticket_linked(open_input(ctx).as_ref());
            Box::pin(async move { checked })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let id = delivery.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            let ticket = open_input(ctx).and_then(|i| i.ticket);
            let actor = ctx.actor().map(|u| u.id);
            Box::pin(async move {
                let now = renox::db::now();
                let frozen = true;
                let reviewed_at = Some(now);
                Delivery::query()
                    .where_(Delivery::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("frozen", &frozen),
                            ("review_ticket_id", &ticket),
                            ("review_reason", &reason),
                            ("reviewed_at", &reviewed_at),
                        ],
                    )
                    .await?;
                trace::insert_in(&mut *tx, id, trace::REVIEW, "", "", now, actor).await
            })
        })
}

/// The review transitions, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Delivery>> {
    vec![
        opening(
            Transition::new(OPEN_REVIEW)
                .label("Put under review")
                .tone("warning")
                .permission(permissions::MANAGE),
        ),
        opening(
            Transition::new(OPEN_REVIEW_SYSTEM)
                .label("Put under review (system)")
                .tone("warning")
                .system(),
        ),
        close_transition(),
    ]
}

fn close_transition() -> Transition<Delivery> {
    Transition::new(CLOSE_REVIEW)
        .label("Close the review")
        .tone("success")
        .from([DeliveryStatus::UnderReview])
        .to(DeliveryStatus::Delivered)
        .permission(permissions::MANAGE)
        .reason_required()
        .guard("outcome_upheld", |_delivery: &Delivery, ctx| {
            let upheld = ctx
                .input_as::<CloseInput>()
                .is_ok_and(|input| input.outcome == ReviewOutcome::Upheld);
            Box::pin(async move {
                if upheld {
                    Ok(())
                } else {
                    Err("A review closes only as upheld: otherwise reverse the delivery or mark it lost."
                        .to_owned())
                }
            })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let id = delivery.id;
            let note = ctx.reason().map(|r| r.trim().to_owned());
            let actor = ctx.actor().map(|u| u.id);
            Box::pin(async move {
                let frozen = false;
                Delivery::query()
                    .where_(Delivery::ID.eq(id))
                    .update(&mut *tx, &[("frozen", &frozen), ("review_note", &note)])
                    .await?;
                trace::insert_in(&mut *tx, id, trace::REVIEW, "", "", renox::db::now(), actor)
                    .await
            })
        })
}

fn guard_error(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// The source line references of the delivery's lines, for the events.
pub(crate) async fn source_lines(db: &Db, delivery: &Delivery) -> Result<Vec<wl_kernel::Subject>> {
    Ok(delivery
        .lines(db)
        .await?
        .iter()
        .filter_map(crate::delivery::DeliveryLine::source_line)
        .collect())
}

/// Opens the review's ticket for `delivery`; `None` when ticketing is off.
async fn open_ticket(
    state: &AppState,
    delivery: &Delivery,
    reason: &str,
    channel: TicketChannel,
) -> std::result::Result<Option<TicketId>, TransitionError> {
    let mut ticket = OpenTicket::new(
        delivery.delivery_id().subject(),
        TICKET_CATEGORY,
        &english("wl_delivery.review.ticket_title").replace("{number}", &delivery.number),
    )
    .description(reason.trim())
    .channel(channel)
    .source(delivery.source());
    if let Some(party) = delivery.recipient() {
        ticket = ticket.requester_party(party);
    }
    let opened = tickets::task_creator(state)
        .open(state, ticket)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(opened.map(|t| t.id))
}

/// Emits `DeliveryDisputed` for the committed review of delivery `id`.
async fn emit_disputed(state: &AppState, id: i64) -> std::result::Result<(), TransitionError> {
    let delivery = Delivery::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)?;
    let reason = delivery.review_reason.clone().unwrap_or_default();
    state
        .emit(DeliveryDisputed {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            reason: reason.clone(),
            ticket: delivery.review_ticket_id.map(TicketId),
            lines: source_lines(&state.db, &delivery)
                .await
                .map_err(TransitionError::Failed)?,
        })
        .await
        .map_err(TransitionError::Failed)?;
    crate::notices::disputed(state, &delivery, &reason, delivery.review_ticket_id)
        .await
        .map_err(TransitionError::Failed)
}

/// Puts `delivery` under review for `user` (a holder of `deliveries.manage`)
/// with `reason`; a ticket is opened and linked.
///
/// # Errors
///
/// `Forbidden` without `deliveries.manage`; `ReasonRequired` without a
/// reason; `NotFrom` unless the delivery is `delivered`; `Guard` (422) when
/// it is already under review (the answer names its ticket).
pub async fn open_review(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    if delivery.status == DeliveryStatus::UnderReview {
        return Err(guard_error(already_text(delivery)));
    }
    let ready = delivery.status == DeliveryStatus::Delivered
        && !reason.trim().is_empty()
        && user.has_permission(permissions::MANAGE);
    let input = if ready {
        let ticket = open_ticket(state, delivery, reason, TicketChannel::Staff).await?;
        OpenInput {
            ticket: ticket.map(|t| t.0),
            ticketing_off: ticket.is_none(),
        }
    } else {
        OpenInput::default()
    };
    let _outcome = delivery
        .transition(OPEN_REVIEW)
        .by(user)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await?;
    emit_disputed(state, delivery.id).await
}

/// Puts `delivery` under review as the system, for a dispute `ticket` that the
/// recipient or the client raised (`ticket` is the review's ticket; without
/// one, a ticket is opened). A delivery that is already under review stays
/// as it is and the new complaint is added to its ticket as a comment.
///
/// # Errors
///
/// `ReasonRequired` without a reason; `NotFrom` unless the delivery is
/// `delivered` or under review.
pub async fn open_review_as_system(
    state: &AppState,
    delivery: &Delivery,
    reason: &str,
    ticket: Option<TicketId>,
) -> std::result::Result<(), TransitionError> {
    if delivery.status == DeliveryStatus::UnderReview {
        if let (Some(existing), Some(new)) = (delivery.review_ticket_id, ticket)
            && existing != new.0
        {
            let text = english("wl_delivery.review.linked_complaint")
                .replace("{ticket}", &new.0.to_string())
                .replace("{reason}", reason.trim());
            tickets::task_creator(state)
                .comment(state, TicketId(existing), TaskComment::internal(&text))
                .await
                .map_err(TransitionError::Failed)?;
        }
        return Ok(());
    }
    let ready = delivery.status == DeliveryStatus::Delivered && !reason.trim().is_empty();
    let input = if !ready {
        OpenInput::default()
    } else if let Some(ticket) = ticket {
        OpenInput {
            ticket: Some(ticket.0),
            ticketing_off: false,
        }
    } else {
        let opened = open_ticket(state, delivery, reason, TicketChannel::System).await?;
        OpenInput {
            ticket: opened.map(|t| t.0),
            ticketing_off: opened.is_none(),
        }
    };
    let _outcome = delivery
        .transition(OPEN_REVIEW_SYSTEM)
        .by_system()
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await?;
    emit_disputed(state, delivery.id).await
}

/// The sentence that refuses a second `open_review`.
fn already_text(delivery: &Delivery) -> String {
    match delivery.review_ticket_id {
        Some(ticket) => english("wl_delivery.review.already_ticket")
            .replace("{number}", &delivery.number)
            .replace("{ticket}", &ticket.to_string()),
        None => english("wl_delivery.review.already").replace("{number}", &delivery.number),
    }
}

/// Closes the review of `delivery` for `user` with `outcome` and a `note`:
/// the delivery is `delivered` again and its flag is cleared, the review's
/// ticket is resolved with the note when it is still open, and
/// `DeliveryDisputeClosed` is emitted.
///
/// # Errors
///
/// `Forbidden` without `deliveries.manage`; `ReasonRequired` without a
/// note; `NotFrom` unless the delivery is under review.
pub async fn close_review(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    outcome: ReviewOutcome,
    note: &str,
) -> std::result::Result<(), TransitionError> {
    let _outcome = delivery
        .transition(CLOSE_REVIEW)
        .by(user)
        .reason(note.trim())
        .input(&CloseInput { outcome })
        .run(state)
        .await?;
    let ticket = delivery.review_ticket_id.map(TicketId);
    if let Some(ticket) = ticket {
        tickets::task_creator(state)
            .resolve_from_source(state, ticket, note.trim())
            .await
            .map_err(TransitionError::Failed)?;
    }
    state
        .emit(DeliveryDisputeClosed {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            outcome,
            ticket,
            lines: source_lines(&state.db, delivery)
                .await
                .map_err(TransitionError::Failed)?,
        })
        .await
        .map_err(TransitionError::Failed)
}

/// Whether `delivery`'s cash and billing are held.
pub fn is_frozen(delivery: &Delivery) -> bool {
    delivery.frozen
}

/// The cash collected on `delivery` that a remittance or settlement may take
/// now, in fils: the collected amount of its saved proof when it is
/// `delivered` and not frozen, else 0.
///
/// # Errors
///
/// Database errors.
pub async fn remittable_cod(db: &Db, delivery: &Delivery) -> Result<i64> {
    if delivery.frozen || delivery.status != DeliveryStatus::Delivered {
        return Ok(0);
    }
    Ok(delivery
        .latest_proof(db)
        .await?
        .map_or(0, |proof| proof.cash_collected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_review_needs_a_ticket_unless_ticketing_is_off() {
        assert!(ticket_linked(None).is_err());
        assert!(ticket_linked(Some(&OpenInput::default())).is_err());
        let linked = OpenInput {
            ticket: Some(4),
            ticketing_off: false,
        };
        assert!(ticket_linked(Some(&linked)).is_ok());
        let off = OpenInput {
            ticket: None,
            ticketing_off: true,
        };
        assert!(ticket_linked(Some(&off)).is_ok());
    }

    #[test]
    fn a_frozen_delivery_is_not_remittable_state() {
        let delivery = Delivery {
            frozen: true,
            status: DeliveryStatus::UnderReview,
            ..Delivery::default()
        };
        assert!(is_frozen(&delivery));
    }

    #[test]
    fn the_second_dispute_is_refused_with_the_ticket_named() {
        let delivery = Delivery {
            number: "DLV-2026-0001".into(),
            review_ticket_id: Some(7),
            ..Delivery::default()
        };
        assert!(already_text(&delivery).contains("7"));
        let bare = Delivery {
            number: "DLV-2026-0001".into(),
            ..Delivery::default()
        };
        assert!(already_text(&bare).contains("DLV-2026-0001"));
    }
}
