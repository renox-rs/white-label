//! A delivery whose goods are neither delivered nor received back is `lost`;
//! one that turns up at a depot is found again (wl-delivery §4.1
//! `mark_lost`, `found`, §5.5 step 4, `WRK.ACL.10`, `WRK.RUL.17`, issue
//! #162).
//!
//! - [`run`] is the rule `WRK.RUL.17`, daily at 02:00 Asia/Dubai: a delivery
//!   `out_for_delivery`, `attempted` or `under_review` whose goods were last
//!   traced N days (7) of 24 hours ago or more is marked `lost` by the
//!   system, with liability *none*. The clock starts at the latest of the
//!   dispatch, the last visit, a review action, a scan, a note or a depot
//!   receipt ([`crate::trace`]); any of them restarts it. A delivery that is
//!   already lost, or `delivered`, is never picked.
//! - [`mark_lost`] is the manual request of a holder of `deliveries.reverse`
//!   with a reason and a liability. The run's driver can neither request nor
//!   approve it (the driver holds no `deliveries.reverse`; the guard also
//!   refuses the driver as the requester). The kernel approval policy
//!   `delivery.mark_lost` (`WRK.ACL.10`) makes another holder of
//!   `deliveries.reverse` approve it (never the requester). At the final
//!   approval the request is re-checked: a scan, a visit or a depot receipt
//!   recorded after the request fails it, and the delivery stays where it is
//!   (the kernel's final check below).
//! - [`found`] puts a lost delivery back to `attempted` for a holder of
//!   `deliveries.manage`, with the depot where the goods were scanned or
//!   confirmed: a visit with the reason *found* that does not count against
//!   the maximum, a comment on the claim ticket, `DeliveryFound`. The frozen
//!   flag of an earlier review is cleared here, since the delivery is not
//!   `delivered` any more.
//!
//! After the commit of any of the three ways to `lost`, a listener opens one
//! claim ticket (the ticket service writes on its own connection, so never
//! inside the transaction) and emits `DeliveryLost` with the state before,
//! the review ticket and the liability. A `delivered` delivery is never lost
//! directly: it goes through a review first. Nothing is deleted or
//! cancelled.

use renox::chrono::Duration;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{
    ApprovalRequest, PolicyDefault, register_final_check, submit::REQUEST_KEY,
};
use wl_kernel::delivery::{DELIVERY_SUBJECT_TYPE, DeliveryFound, DeliveryLost, RunId};
use wl_kernel::machines::SubjectMachine;
use wl_kernel::tickets::{self, OpenTicket, TaskComment, TicketId};

use crate::delivery::{Attempt, AttemptOutcome, Delivery, DeliveryStatus};
use crate::permissions;
use crate::proof::next_attempt_number;
use crate::review::{TICKET_CATEGORY, source_lines};
use crate::run::Run;
use crate::settings::settings;
use crate::texts::english;
use crate::trace;

/// `mark_lost`: out_for_delivery, attempted or under_review → lost, by a user.
pub const MARK_LOST: &str = "mark_lost";
/// `mark_lost`, run by the kernel once the request is approved.
pub const MARK_LOST_APPROVED: &str = "mark_lost_approved";
/// `mark_lost`, run by the system for `WRK.RUL.17`.
pub const MARK_LOST_SYSTEM: &str = "mark_lost_system";
/// `found`: lost → attempted.
pub const FOUND: &str = "found";

/// The approval policy's action key of a manual `mark_lost` (`WRK.ACL.10`).
pub const ACTION: &str = "delivery.mark_lost";

/// The rule, as the specification names it.
pub const RULE: &str = "WRK.RUL.17";

/// The task that runs the rule.
pub const TASK: &str = "delivery:lost-untraced";

/// The time of day the rule runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// Who bears a loss.
pub const LIABILITIES: [&str; 4] = ["driver", "business", "client", "none"];

/// The failure reason code of the visit `found` records.
pub const FOUND_REASON: &str = "found";

/// Hours an approval request for a manual loss may wait.
pub const EXPIRY_HOURS: i64 = 72;

/// The states a delivery can be lost from.
const LOSABLE: [DeliveryStatus; 3] = [
    DeliveryStatus::OutForDelivery,
    DeliveryStatus::Attempted,
    DeliveryStatus::UnderReview,
];

/// What a manual `mark_lost` runs on; it is also the approval request's
/// payload, which the kernel hands to `mark_lost_approved`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct LostInput {
    /// Why the delivery is lost.
    #[serde(default)]
    pub reason: String,
    /// Who bears the loss.
    #[serde(default)]
    pub liability: String,
    /// The driver of the delivery's run, or 0.
    #[serde(default)]
    pub driver_user_id: i64,
}

/// What `found` runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FoundInput {
    /// The depot where the goods were scanned or confirmed.
    pub depot: String,
    /// The number the visit gets.
    pub attempt_number: i64,
}

fn lost_input(ctx: &TransitionCtx) -> Option<LostInput> {
    ctx.input_as::<LostInput>().ok()
}

/// The guard "reason and liability" of a manual loss.
pub(crate) fn reason_and_liability(input: Option<&LostInput>) -> std::result::Result<(), String> {
    match input {
        None => Err("The loss was not prepared by the lost service.".to_owned()),
        Some(input) if input.reason.trim().is_empty() => {
            Err("Give the reason the delivery is lost.".to_owned())
        }
        Some(input) if !LIABILITIES.contains(&input.liability.as_str()) => {
            Err("Choose who bears the loss: driver, business, client or none.".to_owned())
        }
        Some(_) => Ok(()),
    }
}

/// What the three ways to `lost` share: the states, the reason stored with
/// the delivery and the liability.
fn losing(transition: Transition<Delivery>) -> Transition<Delivery> {
    transition
        .from(LOSABLE)
        .to(DeliveryStatus::Lost)
        .effect(|delivery: &Delivery, ctx, tx| {
            let id = delivery.id;
            let input = lost_input(ctx);
            let reason = ctx
                .reason()
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty())
                .or_else(|| input.as_ref().map(|i| i.reason.trim().to_owned()));
            let liability = input
                .map(|i| i.liability)
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "none".to_owned());
            Box::pin(async move {
                let at = Some(renox::db::now());
                let liability = Some(liability);
                Delivery::query()
                    .where_(Delivery::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("lost_reason", &reason),
                            ("lost_liability", &liability),
                            ("lost_at", &at),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The manual rules, the same for the user's run and the kernel's.
fn manual(transition: Transition<Delivery>) -> Transition<Delivery> {
    losing(transition).guard("reason_and_liability", |_delivery: &Delivery, ctx| {
        let checked = reason_and_liability(lost_input(ctx).as_ref());
        Box::pin(async move { checked })
    })
}

/// The loss transitions and `found`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Delivery>> {
    vec![
        manual(
            Transition::new(MARK_LOST)
                .label("Mark lost")
                .tone("danger")
                .permission(permissions::REVERSE),
        )
        .guard("not_the_driver", |_delivery: &Delivery, ctx| {
            let driver = lost_input(ctx).map_or(0, |i| i.driver_user_id);
            let is_driver = matches!(ctx.actor(), Some(user) if driver != 0 && user.id == driver);
            Box::pin(async move {
                if is_driver {
                    Err(
                        "The driver of the run cannot request that its delivery is lost."
                            .to_owned(),
                    )
                } else {
                    Ok(())
                }
            })
        }),
        manual(
            Transition::new(MARK_LOST_APPROVED)
                .label("Mark lost (approved)")
                .system(),
        ),
        losing(
            Transition::new(MARK_LOST_SYSTEM)
                .label("Mark lost (untraced)")
                .system()
                .reason_required(),
        ),
        found_transition(),
    ]
}

fn found_transition() -> Transition<Delivery> {
    Transition::new(FOUND)
        .label("Found")
        .tone("success")
        .from([DeliveryStatus::Lost])
        .to(DeliveryStatus::Attempted)
        .permission(permissions::MANAGE)
        .guard("depot_given", |_delivery: &Delivery, ctx| {
            let given = ctx
                .input_as::<FoundInput>()
                .is_ok_and(|input| !input.depot.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err("Name the depot where the goods were scanned or confirmed.".to_owned())
                }
            })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let delivery_id = delivery.id;
            let run_id = delivery.run_id;
            let actor = ctx.actor().map(|user| user.id);
            let input = ctx.input_as::<FoundInput>();
            Box::pin(async move {
                let input = input.map_err(|_| {
                    Error::from(renox::anyhow::anyhow!("a found without its input"))
                })?;
                let now = renox::db::now();
                // The visit does not count against the maximum attempts.
                let mut attempt = Attempt {
                    delivery_id,
                    run_id,
                    number: input.attempt_number,
                    outcome: AttemptOutcome::Failed,
                    failure_reason: Some(FOUND_REASON.to_owned()),
                    counts: false,
                    occurred_at: now,
                    actor_user_id: actor,
                    ..Attempt::default()
                };
                attempt.insert(&mut *tx).await?;
                let frozen = false;
                Delivery::query()
                    .where_(Delivery::ID.eq(delivery_id))
                    .update(&mut *tx, &[("frozen", &frozen)])
                    .await?;
                trace::insert_in(
                    &mut *tx,
                    delivery_id,
                    trace::FOUND,
                    "",
                    &input.depot,
                    now,
                    actor,
                )
                .await
            })
        })
}

/// The driver of the run `delivery` is on, or 0.
async fn driver_of(db: &Db, delivery: &Delivery) -> Result<i64> {
    match delivery.run_id {
        Some(id) => Ok(Run::find(db, id).await?.map_or(0, |run| run.driver_user_id)),
        None => Ok(0),
    }
}

/// What a manual request came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostRequest {
    /// No approval was needed: the delivery is lost.
    Lost,
    /// An approval request (its id) waits for a second manager.
    Requested(i64),
}

/// A holder of `deliveries.reverse` asks to mark `delivery` lost, with a
/// `reason` and a `liability` (`driver`, `business`, `client` or `none`).
/// The kernel's policy `delivery.mark_lost` defers it as an approval request
/// that another holder of `deliveries.reverse` decides; the request is
/// announced after the commit.
///
/// # Errors
///
/// `Forbidden` without `deliveries.reverse`; `NotFrom` for a delivery that is
/// not `out_for_delivery`, `attempted` or `under_review` (a `delivered` one
/// goes through a review first); `Guard` (422) without a reason, with an
/// unknown liability, for the run's driver, or when a request for the
/// delivery is already pending.
pub async fn mark_lost(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    reason: &str,
    liability: &str,
) -> std::result::Result<LostRequest, TransitionError> {
    let input = LostInput {
        reason: reason.trim().to_owned(),
        liability: liability.trim().to_owned(),
        driver_user_id: driver_of(&state.db, delivery)
            .await
            .map_err(TransitionError::Failed)?,
    };
    let outcome = delivery
        .transition(MARK_LOST)
        .by(user)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(match &outcome {
        Outcome::Pending(pending) => match pending.data.get(REQUEST_KEY).and_then(Value::as_i64) {
            Some(id) => LostRequest::Requested(id),
            None => LostRequest::Lost,
        },
        _ => LostRequest::Lost,
    })
}

/// Marks `delivery` lost as the system (`WRK.RUL.17`), with liability *none*.
///
/// # Errors
///
/// `NotFrom` for a delivery that cannot be lost.
pub async fn mark_lost_as_system(
    state: &AppState,
    delivery: &Delivery,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    let _outcome = delivery
        .transition(MARK_LOST_SYSTEM)
        .by_system()
        .reason(reason.trim())
        .run(state)
        .await?;
    Ok(())
}

/// The rule `WRK.RUL.17` at `now`: marks lost, as the system, every delivery
/// `out_for_delivery`, `attempted` or `under_review` whose goods were last
/// traced at least N × 24 hours before `now` (N is the setting, 7). Returns
/// how many it marked. A delivery already lost or delivered is not picked,
/// so a second run on the same day does nothing.
///
/// # Errors
///
/// Database errors; a delivery that moved meanwhile is skipped.
pub async fn run(state: &AppState, now: DateTime) -> Result<usize> {
    let days = settings(state).untraced_days();
    let candidates = Delivery::query()
        .where_(Delivery::STATUS.is_in(LOSABLE))
        .order_by(Delivery::ID)
        .get(&state.db)
        .await?;
    let reason = english("wl_delivery.lost.untraced").replace("{days}", &days.to_string());
    let mut marked = 0;
    for delivery in candidates {
        let Some(last) = trace::last_traced_at(&state.db, &delivery).await? else {
            continue;
        };
        if last + Duration::days(days) > now {
            continue;
        }
        match mark_lost_as_system(state, &delivery, &reason).await {
            Ok(()) => marked += 1,
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(marked)
}

/// Puts the lost `delivery` back to `attempted` for `user` (a holder of
/// `deliveries.manage`): the goods were scanned or confirmed at `depot`.
///
/// # Errors
///
/// `Forbidden` without `deliveries.manage`; `Guard` (422) without a depot;
/// `NotFrom` unless the delivery is `lost`.
pub async fn found(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    depot: &str,
) -> std::result::Result<(), TransitionError> {
    let input = FoundInput {
        depot: depot.trim().to_owned(),
        attempt_number: next_attempt_number(&state.db, delivery)
            .await
            .map_err(TransitionError::Failed)?,
    };
    let _outcome = delivery
        .transition(FOUND)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    let claim = delivery.claim_ticket_id.map(TicketId);
    if let Some(ticket) = claim {
        let text = english("wl_delivery.lost.found_comment").replace("{depot}", &input.depot);
        tickets::task_creator(state)
            .comment(state, ticket, TaskComment::internal(&text))
            .await
            .map_err(TransitionError::Failed)?;
    }
    state
        .emit(DeliveryFound {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            depot: input.depot,
            claim_ticket: claim,
            lines: source_lines(&state.db, delivery)
                .await
                .map_err(TransitionError::Failed)?,
        })
        .await
        .map_err(TransitionError::Failed)
}

/// After the commit of a loss: opens the claim ticket, keeps its id on the
/// delivery and emits `DeliveryLost`.
async fn after_lost(state: &AppState, event: &Transitioned<Delivery>) -> Result {
    if ![MARK_LOST, MARK_LOST_APPROVED, MARK_LOST_SYSTEM].contains(&event.transition.as_str()) {
        return Ok(());
    }
    let delivery = Delivery::find_or_404(&state.db, event.key).await?;
    let liability = delivery
        .lost_liability
        .clone()
        .unwrap_or_else(|| "none".to_owned());
    let description = english("wl_delivery.lost.claim_description")
        .replace(
            "{reason}",
            delivery.lost_reason.as_deref().unwrap_or_default(),
        )
        .replace("{liability}", &liability);
    let mut claim = OpenTicket::new(
        delivery.delivery_id().subject(),
        TICKET_CATEGORY,
        &english("wl_delivery.lost.claim_title").replace("{number}", &delivery.number),
    )
    .description(&description)
    .source(delivery.source());
    if let Some(party) = delivery.recipient() {
        claim = claim.requester_party(party);
    }
    let opened = tickets::task_creator(state).open(state, claim).await?;
    if let Some(ticket) = &opened {
        let id = Some(ticket.id.0);
        Delivery::query()
            .where_(Delivery::ID.eq(delivery.id))
            .update(&state.db, &[("claim_ticket_id", &id)])
            .await?;
    }
    let was_under_review = event.from == DeliveryStatus::UnderReview.as_str();
    state
        .emit(DeliveryLost {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            was_under_review,
            ticket: delivery
                .review_ticket_id
                .filter(|_| was_under_review)
                .map(TicketId),
            liability,
            run: delivery.run_id.map(RunId),
            lines: source_lines(&state.db, &delivery).await?,
        })
        .await
}

/// The final re-check of a manual loss (`WRK.ACL.10`): no scan, visit or
/// depot receipt may have been recorded after the request; the request then
/// fails and the delivery stays where it is.
async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    if request.action != ACTION {
        return Ok(None);
    }
    let Some(since) = request.created_at else {
        return Ok(None);
    };
    if trace::recorded_after(tx, request.subject_id, since).await? {
        Ok(Some(english("wl_delivery.lost.traced").to_owned()))
    } else {
        Ok(None)
    }
}

/// Registers the approval set-up, the listener and the daily rule.
pub(crate) fn register(app: &mut Registry) {
    // The kernel runs `mark_lost_approved` by name on this subject.
    SubjectMachine::of::<Delivery>(DELIVERY_SUBJECT_TYPE).register();
    // `WRK.ACL.10`: another holder of `deliveries.reverse` approves, never
    // the requester.
    PolicyDefault::new(ACTION)
        .step_permission(permissions::REVERSE)
        .expiry_hours(Some(EXPIRY_HOURS))
        .register();
    // At the final approval: nothing may have been traced after the request.
    register_final_check(DELIVERY_SUBJECT_TYPE, |tx, request| {
        Box::pin(recheck(tx, request))
    });
    app.listen(|event: Transitioned<Delivery>, state: AppState| async move {
        after_lost(&state, &event).await
    });
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, renox::db::now()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loss_needs_a_reason_and_a_known_liability() {
        assert!(reason_and_liability(None).is_err());
        let mut input = LostInput {
            reason: "Parcel untraced".into(),
            liability: "driver".into(),
            driver_user_id: 3,
        };
        assert!(reason_and_liability(Some(&input)).is_ok());
        input.liability = "carrier".into();
        assert!(reason_and_liability(Some(&input)).is_err());
        input.liability = "none".into();
        input.reason = "  ".into();
        assert!(reason_and_liability(Some(&input)).is_err());
    }

    #[test]
    fn a_delivered_delivery_is_not_a_losable_state() {
        assert!(!LOSABLE.contains(&DeliveryStatus::Delivered));
        assert!(!LOSABLE.contains(&DeliveryStatus::Lost));
        assert!(LOSABLE.contains(&DeliveryStatus::UnderReview));
    }
}
