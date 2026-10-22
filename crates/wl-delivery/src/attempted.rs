//! What follows a failed attempt (wl-delivery §4.1 `reattempt`, `return` and
//! `discard`, §5.2 step 5, §5.4 step 3, `WRK.RUL.08`, `WRK.RUL.16`; issue #163).
//!
//! A delivery that `fail`ed (or whose run was aborted) is `attempted`. From
//! there:
//!
//! - **`reattempt`** (`deliveries.manage`, or the system) puts it back to
//!   `pending`, queued for the next run. Refused unless attempts are left,
//!   the last failure reason allows a reattempt and the delivery is **not
//!   perishable** (a perishable delivery has one attempt and is never
//!   reattempted, by hand or by a rule).
//! - **`return`** (`deliveries.manage`, or the system) brings it back to the
//!   origin: `returned`, `DeliveryReturned`. Refused until the goods are
//!   confirmed at the origin (the dispatcher ticks it; the system runs it
//!   when the depot ticks the goods *received* at run close). It is always
//!   open to `deliveries.manage`, also for cooked food.
//! - **`discard`** (the run's driver, `deliveries.complete`) destroys the
//!   perishable goods: refused without a photo and a reason. The wastage
//!   goes to the inventory through the kernel's
//!   [`DeliveryWastage`](wl_kernel::delivery::DeliveryWastage) with the
//!   source as reference. A wastage above the limit (`STK.ACL.02`, AED 1,000;
//!   06: AED 200) is an approval request that a holder of `stock.approve`
//!   other than the driver decides; until then the delivery stays
//!   `attempted`. A rejected request leaves it `attempted`: the driver
//!   submits again (new photo or reason, new approval), or the dispatcher
//!   runs `return` by hand. The kernel runs `discard_approved` on the last
//!   yes.
//!
//! **The rules** run once per failed attempt ([`after_failed_attempt`], from
//! `fail` and from a run `abort`), and a delivery that already has a
//! resolution is left alone:
//!
//! - `WRK.RUL.16` on a perishable delivery: the policy of its source type,
//!   read now ([`PerishablePolicy`](crate::settings::PerishablePolicy)),
//!   after a failed visit or after an abort, sets the delivery's
//!   `resolution` to `return` (the goods must be confirmed at the origin,
//!   then the system runs `return`) or `discard` (the driver gets the
//!   "discard with photo" task on the stop screen). The dispatchers are told.
//!   Nothing about the money is decided here; the source reacts to
//!   `DeliveryAttemptFailed`, then `DeliveryReturned` or `DeliveryDiscarded`.
//! - `WRK.RUL.08` on any other delivery, when the client turned it on
//!   ([`DeliverySettings::attempt_rule`](crate::settings::DeliverySettings::attempt_rule),
//!   off by default because 03 shipments follow their own rule): reattempt
//!   while attempts remain and the reason allows it, else the resolution is
//!   `return`.

use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, PolicyDefault, submit::REQUEST_KEY};
use wl_kernel::delivery::{
    DeliveryDiscarded, DeliveryReturned, DiscardLine, DiscardWastage, delivery_wastage,
};

use crate::close;
use crate::delivery::{Delivery, DeliveryStatus};
use crate::notices;
use crate::permissions;
use crate::reasons::FailureReason;
use crate::settings::{PerishableAction, settings};
use crate::texts::english;

/// `reattempt`: attempted → pending, by a manager.
pub const REATTEMPT: &str = "reattempt";
/// `reattempt`, by the system (`WRK.RUL.08`).
pub const REATTEMPT_BY_SYSTEM: &str = "reattempt_by_system";
/// `return`: attempted → returned, by a manager.
pub const RETURN: &str = "return";
/// `return`, by the system once the goods are confirmed at the origin.
pub const RETURN_BY_SYSTEM: &str = "return_by_system";
/// `discard`: attempted → discarded, by the run's driver.
pub const DISCARD: &str = "discard";
/// `discard`, run by the kernel once the wastage is approved.
pub const DISCARD_APPROVED: &str = "discard_approved";

/// The approval policy's action key of a discard (`STK.ACL.02`).
pub const ACTION: &str = "delivery.discard";

/// The permission of the second person who approves a wastage above the
/// limit: the storekeeper's manager of `wl-inventory` (`stock.approve`).
pub const APPROVER_PERMISSION: &str = "stock.approve";

/// The resolution *return*: the goods go back to the origin.
pub const RESOLUTION_RETURN: &str = "return";
/// The resolution *discard*: the driver has the "discard with photo" task.
pub const RESOLUTION_DISCARD: &str = "discard";

/// The failure reason of a stop that was not attempted.
const NO_REASON: &str = "";

/// What `reattempt` runs on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ReattemptInput {
    /// Whether the last failure reason allows a reattempt.
    pub reason_allows: bool,
}

/// What `return` runs on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct BackInput {
    /// Whether the goods are back at the origin, scanned or confirmed.
    pub goods_confirmed: bool,
}

/// What `discard` runs on; it is also the approval request's payload, which
/// the kernel hands to `discard_approved`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DiscardInput {
    /// Why the goods are destroyed.
    #[serde(default)]
    pub reason: String,
    /// The photo of the goods.
    #[serde(default)]
    pub photo: String,
    /// The wastage at cost, in fils: what the approval policy reads.
    #[serde(default)]
    pub amount: i64,
    /// The driver of the delivery's run, or 0.
    #[serde(default)]
    pub driver_user_id: i64,
    /// When the driver submitted it.
    #[serde(default)]
    pub at: Option<DateTime>,
}

fn reattempt_input(ctx: &TransitionCtx) -> Option<ReattemptInput> {
    ctx.input_as::<ReattemptInput>().ok()
}

fn back_input(ctx: &TransitionCtx) -> Option<BackInput> {
    ctx.input_as::<BackInput>().ok()
}

fn discard_input(ctx: &TransitionCtx) -> Option<DiscardInput> {
    ctx.input_as::<DiscardInput>().ok()
}

/// Why `delivery` cannot be reattempted, if it cannot: perishable, no
/// attempts left, or a reason that does not allow it.
pub(crate) fn reattempt_problem(
    delivery: &Delivery,
    input: Option<&ReattemptInput>,
) -> Option<String> {
    let Some(input) = input else {
        return Some("The reattempt was not prepared by the delivery service.".to_owned());
    };
    if delivery.perishable {
        return Some(english("wl_delivery.attempted.perishable_no_reattempt").to_owned());
    }
    if delivery.attempts >= delivery.max_attempts {
        return Some(english("wl_delivery.attempted.no_attempts_left").to_owned());
    }
    if !input.reason_allows {
        return Some(english("wl_delivery.attempted.reason_no_reattempt").to_owned());
    }
    None
}

/// Why `delivery` cannot be discarded with `input`, if it cannot.
pub(crate) fn discard_problem(delivery: &Delivery, input: Option<&DiscardInput>) -> Option<String> {
    let Some(input) = input else {
        return Some("The discard was not prepared by the delivery service.".to_owned());
    };
    if !delivery.perishable {
        return Some(english("wl_delivery.attempted.not_perishable").to_owned());
    }
    if input.reason.trim().is_empty() {
        return Some(english("wl_delivery.attempted.reason_needed").to_owned());
    }
    if input.photo.trim().is_empty() {
        return Some(english("wl_delivery.attempted.photo_needed").to_owned());
    }
    None
}

fn reattempting(transition: Transition<Delivery>) -> Transition<Delivery> {
    transition
        .from([DeliveryStatus::Attempted])
        .to(DeliveryStatus::Pending)
        .guard("attempts_left_and_allowed", |delivery: &Delivery, ctx| {
            let problem = reattempt_problem(delivery, reattempt_input(ctx).as_ref());
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|delivery: &Delivery, _ctx, tx| {
            let id = delivery.id;
            Box::pin(async move {
                // Back in the queue: off its run, nothing decided any more.
                let run_id: Option<i64> = None;
                let sequence: Option<i64> = None;
                let resolution: Option<String> = None;
                Delivery::query()
                    .where_(Delivery::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("run_id", &run_id),
                            ("run_sequence", &sequence),
                            ("resolution", &resolution),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

fn returning(transition: Transition<Delivery>) -> Transition<Delivery> {
    transition
        .from([DeliveryStatus::Attempted])
        .to(DeliveryStatus::Returned)
        .guard("goods_back_at_the_origin", |_delivery: &Delivery, ctx| {
            let confirmed = back_input(ctx).is_some_and(|input| input.goods_confirmed);
            Box::pin(async move {
                if confirmed {
                    Ok(())
                } else {
                    Err(english("wl_delivery.attempted.confirm_goods").to_owned())
                }
            })
        })
}

fn discarding(transition: Transition<Delivery>) -> Transition<Delivery> {
    transition
        .from([DeliveryStatus::Attempted])
        .to(DeliveryStatus::Discarded)
        .guard("perishable_reason_and_photo", |delivery: &Delivery, ctx| {
            let problem = discard_problem(delivery, discard_input(ctx).as_ref());
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let id = delivery.id;
            let input = discard_input(ctx);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a discard without its input"))
                })?;
                let reason = Some(input.reason.trim().to_owned());
                let photo = Some(input.photo.trim().to_owned());
                let at = Some(input.at.unwrap_or_else(renox::db::now));
                let resolution: Option<String> = None;
                Delivery::query()
                    .where_(Delivery::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("discard_reason", &reason),
                            ("discard_photo_ref", &photo),
                            ("discarded_at", &at),
                            ("resolution", &resolution),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The transitions out of `attempted`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Delivery>> {
    vec![
        reattempting(
            Transition::new(REATTEMPT)
                .label("Reattempt")
                .tone("primary")
                .permission(permissions::MANAGE),
        ),
        reattempting(
            Transition::new(REATTEMPT_BY_SYSTEM)
                .label("Reattempt")
                .system(),
        ),
        returning(
            Transition::new(RETURN)
                .label("Return to the origin")
                .tone("warning")
                .permission(permissions::MANAGE),
        ),
        returning(
            Transition::new(RETURN_BY_SYSTEM)
                .label("Return to the origin")
                .system(),
        ),
        discarding(
            Transition::new(DISCARD)
                .label("Discard")
                .tone("danger")
                .permission(permissions::COMPLETE),
        )
        .guard("by_assigned_driver", |_delivery: &Delivery, ctx| {
            let driver = discard_input(ctx).map_or(0, |input| input.driver_user_id);
            let allowed = matches!(ctx.actor(), Some(user) if driver != 0 && user.id == driver);
            Box::pin(async move {
                if allowed {
                    Ok(())
                } else {
                    Err(english("wl_delivery.attempted.only_the_driver").to_owned())
                }
            })
        }),
        discarding(
            Transition::new(DISCARD_APPROVED)
                .label("Discard (approved)")
                .system(),
        ),
    ]
}

fn refused(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// Whether the last failure reason of `delivery` allows a reattempt (true
/// when there is none to judge by).
async fn last_reason_allows(db: &Db, delivery: &Delivery) -> Result<bool> {
    let attempts = delivery.attempts_made(db).await?;
    let Some(code) = attempts
        .iter()
        .rev()
        .find_map(|attempt| attempt.failure_reason.clone())
    else {
        return Ok(true);
    };
    Ok(FailureReason::query()
        .where_(FailureReason::CODE.eq(code))
        .first(db)
        .await?
        .is_none_or(|reason| reason.allows_reattempt))
}

/// The last failure reason's code of `delivery`, or empty.
async fn last_reason(db: &Db, delivery: &Delivery) -> Result<String> {
    Ok(delivery
        .attempts_made(db)
        .await?
        .iter()
        .rev()
        .find_map(|attempt| attempt.failure_reason.clone())
        .unwrap_or_else(|| NO_REASON.to_owned()))
}

/// A manager (`deliveries.manage`) puts the `attempted` `delivery` back in
/// the queue for the next run.
///
/// # Errors
///
/// `Forbidden` without `deliveries.manage`; `NotFrom` unless `attempted`;
/// `Guard` (422) for a perishable delivery, one with no attempts left, or one
/// whose last reason does not allow a reattempt.
pub async fn reattempt(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
) -> std::result::Result<(), TransitionError> {
    let input = ReattemptInput {
        reason_allows: last_reason_allows(&state.db, delivery)
            .await
            .map_err(TransitionError::Failed)?,
    };
    let _outcome = delivery
        .transition(REATTEMPT)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

/// A manager (`deliveries.manage`) returns the `attempted` `delivery` to the
/// origin; `goods_confirmed` says the goods are back, scanned or ticked. Open
/// to the manager also for cooked food.
///
/// # Errors
///
/// `Forbidden` without `deliveries.manage`; `NotFrom` unless `attempted`;
/// `Guard` (422) until the goods are confirmed.
pub async fn return_delivery(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    goods_confirmed: bool,
) -> std::result::Result<(), TransitionError> {
    let _outcome = delivery
        .transition(RETURN)
        .by(user)
        .input(&BackInput { goods_confirmed })
        .run(state)
        .await?;
    Ok(())
}

/// What the driver's discard came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardOutcome {
    /// The delivery is `discarded`.
    Discarded,
    /// The wastage is above the limit: an approval request (its id) waits for
    /// a holder of `stock.approve` other than the driver; the delivery stays
    /// `attempted`.
    Requested(i64),
}

/// The goods of `delivery` as a wastage request for `reason` and `photo`.
async fn wastage_request(
    db: &Db,
    delivery: &Delivery,
    reason: &str,
    photo: Option<&str>,
) -> Result<DiscardWastage> {
    let lines = delivery
        .lines(db)
        .await?
        .into_iter()
        .map(|line| DiscardLine {
            source_line: line.source_line(),
            description: line.description.clone(),
            quantity_milli: line.quantity_milli,
        })
        .collect();
    Ok(DiscardWastage {
        delivery: delivery.delivery_id(),
        source: delivery.source(),
        reason: reason.to_owned(),
        photo: photo.map(str::to_owned),
        lines,
    })
}

/// The run's driver `user` submits the photo and the reason of a discard
/// ("discard with photo", `WRK.RUL.16`). Under the policy *discard* the
/// delivery is `discarded` and the wastage recorded; above the limit an
/// approval request waits instead.
///
/// # Errors
///
/// `Forbidden` without `deliveries.complete`; `NotFrom` unless `attempted`;
/// `Guard` (422) for a delivery that is not perishable, one the rule did not
/// ask to discard, anyone but the run's driver, or a missing photo or reason.
pub async fn discard(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    reason: &str,
    photo: &str,
    at: DateTime,
) -> std::result::Result<DiscardOutcome, TransitionError> {
    if delivery.status == DeliveryStatus::Attempted
        && delivery.resolution.as_deref() != Some(RESOLUTION_DISCARD)
    {
        return Err(refused(english("wl_delivery.attempted.no_discard_task")));
    }
    let request = wastage_request(&state.db, delivery, reason.trim(), Some(photo.trim()))
        .await
        .map_err(TransitionError::Failed)?;
    let amount = delivery_wastage(state)
        .estimate(state, &request)
        .await
        .map_err(TransitionError::Failed)?;
    let driver = match delivery.run_id {
        Some(id) => crate::run::Run::find(&state.db, id)
            .await
            .map_err(TransitionError::Failed)?
            .map_or(0, |run| run.driver_user_id),
        None => 0,
    };
    let input = DiscardInput {
        reason: reason.trim().to_owned(),
        photo: photo.trim().to_owned(),
        amount,
        driver_user_id: driver,
        at: Some(at),
    };
    let outcome = delivery
        .transition(DISCARD)
        .by(user)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await?;
    approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(match &outcome {
        Outcome::Pending(pending) => match pending.data.get(REQUEST_KEY).and_then(Value::as_i64) {
            Some(id) => DiscardOutcome::Requested(id),
            None => DiscardOutcome::Discarded,
        },
        _ => DiscardOutcome::Discarded,
    })
}

/// The rules `WRK.RUL.16` and `WRK.RUL.08` after a failed attempt of
/// `delivery_id` (see the module text). `aborted` says the visit failed
/// because the run was aborted. Acts once per failed attempt.
///
/// # Errors
///
/// Database errors, the notifier's, and the transitions' failures.
pub async fn after_failed_attempt(state: &AppState, delivery_id: i64, aborted: bool) -> Result {
    let delivery = Delivery::find_or_404(&state.db, delivery_id).await?;
    if delivery.status != DeliveryStatus::Attempted || delivery.resolution.is_some() {
        return Ok(());
    }
    let settings = settings(state);
    if delivery.perishable {
        let policy = settings.perishable_policy(&delivery.source_type);
        let action = if aborted {
            policy.on_abort
        } else {
            policy.on_failed_visit
        };
        set_resolution(&state.db, delivery.id, action.as_str()).await?;
        return notices::perishable_failed(state, &delivery, action == PerishableAction::Discard)
            .await;
    }
    if !settings.attempt_rule() {
        return Ok(());
    }
    let allows = last_reason_allows(&state.db, &delivery).await?;
    if allows && delivery.attempts < delivery.max_attempts {
        let input = ReattemptInput {
            reason_allows: allows,
        };
        let _outcome = delivery
            .transition(REATTEMPT_BY_SYSTEM)
            .by_system()
            .input(&input)
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
        return Ok(());
    }
    set_resolution(&state.db, delivery.id, RESOLUTION_RETURN).await
}

async fn set_resolution(db: &Db, delivery_id: i64, resolution: &str) -> Result {
    let resolution = Some(resolution.to_owned());
    Delivery::query()
        .where_(Delivery::ID.eq(delivery_id))
        .update(db, &[("resolution", &resolution)])
        .await?;
    Ok(())
}

/// The depot ticked the goods of `delivery_id` *received* at run close: when
/// the rules decided they go back to the origin, the system runs `return`.
///
/// # Errors
///
/// Database errors and the transition's failures.
pub async fn on_goods_received(state: &AppState, delivery_id: i64) -> Result {
    let delivery = Delivery::find_or_404(&state.db, delivery_id).await?;
    if delivery.status != DeliveryStatus::Attempted
        || delivery.resolution.as_deref() != Some(RESOLUTION_RETURN)
    {
        return Ok(());
    }
    let _outcome = delivery
        .transition(RETURN_BY_SYSTEM)
        .by_system()
        .input(&BackInput {
            goods_confirmed: true,
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// After the commit of a `return`: `DeliveryReturned`, the dispatchers told,
/// the run closed when this was the last item.
async fn finish_return(state: &AppState, delivery_id: i64) -> Result {
    let delivery = Delivery::find_or_404(&state.db, delivery_id).await?;
    let reason = last_reason(&state.db, &delivery).await?;
    state
        .emit(DeliveryReturned {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            reason,
        })
        .await?;
    notices::returned(state, &delivery).await?;
    close_run_of(state, &delivery).await
}

/// After the commit of a `discard`: the wastage is recorded with the source
/// as reference, `DeliveryDiscarded` is emitted, the dispatchers are told and
/// the run closed when this was the last item.
async fn finish_discard(state: &AppState, delivery_id: i64) -> Result {
    let delivery = Delivery::find_or_404(&state.db, delivery_id).await?;
    let reason = delivery.discard_reason.clone().unwrap_or_default();
    let request =
        wastage_request(&state.db, &delivery, &reason, delivery.discard_photo_ref.as_deref())
            .await?;
    let recorded = delivery_wastage(state).record(state, &request).await?;
    let (kind, id, value) = match &recorded {
        Some(done) => (
            done.reference.as_ref().map(|r| r.subject_type.clone()),
            done.reference.as_ref().map(|r| r.subject_id),
            done.value,
        ),
        None => (None, None, 0),
    };
    Delivery::query()
        .where_(Delivery::ID.eq(delivery.id))
        .update(
            &state.db,
            &[
                ("wastage_type", &kind),
                ("wastage_id", &id),
                ("wastage_value", &value),
            ],
        )
        .await?;
    state
        .emit(DeliveryDiscarded {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            reason: reason.clone(),
            photo: delivery.discard_photo_ref.clone(),
            wastage: recorded.as_ref().and_then(|done| done.reference.clone()),
            value,
            lines: delivery
                .lines(&state.db)
                .await?
                .iter()
                .filter_map(crate::delivery::DeliveryLine::source_line)
                .collect(),
        })
        .await?;
    notices::discarded(state, &delivery, &reason).await?;
    close_run_of(state, &delivery).await
}

/// Tries to close the run `delivery` was on: it was one of its items.
async fn close_run_of(state: &AppState, delivery: &Delivery) -> Result {
    if let Some(run_id) = delivery.run_id {
        close::try_close(state, run_id)
            .await
            .map_err(TransitionError::into_error)?;
    }
    Ok(())
}

/// Registers the approval policy and the listener that finishes a return or
/// a discard.
pub(crate) fn register(app: &mut Registry, limit_fils: i64) {
    // `STK.ACL.02`: a wastage above the limit needs a holder of
    // `stock.approve`; the kernel keeps the requester (the driver) out.
    PolicyDefault::new(ACTION)
        .amount_above(limit_fils)
        .step_permission(APPROVER_PERMISSION)
        .register();
    app.listen(
        |event: Transitioned<Delivery>, state: AppState| async move {
            match event.transition.as_str() {
                RETURN | RETURN_BY_SYSTEM => finish_return(&state, event.key).await,
                DISCARD | DISCARD_APPROVED => finish_discard(&state, event.key).await,
                _ => Ok(()),
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(perishable: bool, attempts: i64, max: i64) -> Delivery {
        Delivery {
            perishable,
            attempts,
            max_attempts: max,
            ..Delivery::default()
        }
    }

    #[test]
    fn a_perishable_delivery_is_never_reattempted() {
        let input = ReattemptInput {
            reason_allows: true,
        };
        assert!(reattempt_problem(&delivery(true, 0, 1), Some(&input)).is_some());
        assert!(reattempt_problem(&delivery(false, 1, 3), Some(&input)).is_none());
    }

    #[test]
    fn a_reattempt_needs_attempts_left_and_a_reason_that_allows_it() {
        let allows = ReattemptInput {
            reason_allows: true,
        };
        let denies = ReattemptInput {
            reason_allows: false,
        };
        assert!(reattempt_problem(&delivery(false, 3, 3), Some(&allows)).is_some());
        assert!(reattempt_problem(&delivery(false, 1, 3), Some(&denies)).is_some());
        assert!(reattempt_problem(&delivery(false, 1, 3), None).is_some());
    }

    #[test]
    fn a_discard_needs_a_perishable_delivery_a_reason_and_a_photo() {
        let input = |reason: &str, photo: &str| DiscardInput {
            reason: reason.into(),
            photo: photo.into(),
            ..DiscardInput::default()
        };
        let perishable = delivery(true, 1, 1);
        assert!(discard_problem(&perishable, Some(&input("spoilt", "p-1"))).is_none());
        assert!(discard_problem(&perishable, Some(&input(" ", "p-1"))).is_some());
        assert!(discard_problem(&perishable, Some(&input("spoilt", ""))).is_some());
        assert!(discard_problem(&delivery(false, 1, 3), Some(&input("spoilt", "p-1"))).is_some());
        assert!(discard_problem(&perishable, None).is_some());
    }

    #[test]
    fn the_transitions_leave_attempted_and_the_approved_ones_are_the_systems() {
        let machine = Delivery::machine();
        for (name, to, system) in [
            (REATTEMPT, "pending", false),
            (REATTEMPT_BY_SYSTEM, "pending", true),
            (RETURN, "returned", false),
            (RETURN_BY_SYSTEM, "returned", true),
            (DISCARD, "discarded", false),
            (DISCARD_APPROVED, "discarded", true),
        ] {
            let found = machine.find(name).expect(name);
            assert_eq!(found.from_states(), ["attempted"], "{name}");
            assert_eq!(found.to_state(), Some(to), "{name}");
            assert_eq!(found.is_system(), system, "{name}");
        }
    }
}
