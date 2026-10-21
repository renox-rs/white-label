//! Planning and dispatch (wl-delivery §4.1 `assign`, `unassign`, `dispatch`,
//! `cancel`; §4.2 `dispatch`; §5.1), the services the dispatcher's pages call:
//!
//! - [`create_run`] makes a `planned` run for a driver (and a vehicle).
//! - [`assign`] puts a `pending` delivery on a `planned` run: the run is
//!   planned, the driver is available (not on leave on the run's date), and
//!   the driver's and vehicle's documents pass the guard of
//!   `delivery.assign` on the run's date ([`crate::doc_guard`], #157).
//!   `DeliveryAssigned` is emitted.
//! - [`unassign`] takes it off while the run is not dispatched.
//! - [`dispatch_run`] dispatches a `planned` run: the documents pass the
//!   guard of `run.dispatch` today, the run moves to `dispatched` and each
//!   of its `assigned` deliveries to `out_for_delivery` (as the system),
//!   all in one transaction; then `DeliveryOutForDelivery` per delivery and
//!   `RunDispatched` are emitted. A refused dispatch moves nothing.
//! - [`cancel`] / [`cancel_as_system`] cancel a `pending` or `assigned`
//!   delivery with a reason.
//!
//! Renox guards get no `AppState`, so the checks that need the database run
//! in the services and the machine's guards read what the service worked out
//! (the run's status in an [`AssignInput`]).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{DeliveryAssigned, DeliveryOutForDelivery, RunDispatched};
use wl_kernel::documents::today;
use wl_kernel::workforce::workforce;

use crate::delivery::{Delivery, DeliveryStatus};
use crate::doc_guard;
use crate::permissions;
use crate::run::{RUN_SERIES, Run, RunStatus, VehicleType};
use crate::run_machine;

/// `assign`: pending → assigned.
pub const ASSIGN: &str = "assign";
/// `unassign`: assigned → pending.
pub const UNASSIGN: &str = "unassign";
/// `dispatch`: assigned → out_for_delivery, by the system with the run.
pub const DISPATCH: &str = "dispatch";
/// `cancel`: pending or assigned → cancelled.
pub const CANCEL: &str = "cancel";
/// `cancel`, run by the system (the source was cancelled).
pub const AUTO_CANCEL: &str = "auto_cancel";

/// What the services work out for `assign` and `unassign`, for the guards
/// and effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AssignInput {
    /// The run.
    pub run_id: i64,
    /// The run's status when the service looked.
    pub run_status: RunStatus,
    /// The place on the run.
    pub sequence: i64,
}

fn input(ctx: &TransitionCtx) -> Option<AssignInput> {
    ctx.input_as::<AssignInput>().ok()
}

/// Clears the run link and, for a cancel, stores the reason.
fn leaving_the_run(transition: Transition<Delivery>) -> Transition<Delivery> {
    transition.effect(|delivery: &Delivery, ctx, tx| {
        let id = delivery.id;
        let reason = ctx.reason().map(|r| r.trim().to_owned());
        let cancelled = reason.is_some();
        Box::pin(async move {
            let none: Option<i64> = None;
            let mut values: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> =
                vec![("run_id", &none), ("run_sequence", &none)];
            if cancelled {
                values.push(("cancel_reason", &reason));
            }
            Delivery::query()
                .where_(Delivery::ID.eq(id))
                .update(&mut *tx, &values)
                .await?;
            Ok(())
        })
    })
}

/// The delivery transitions of planning, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Delivery>> {
    vec![
        assign_transition(),
        unassign_transition(),
        dispatch_transition(),
        leaving_the_run(
            Transition::new(CANCEL)
                .label("Cancel")
                .tone("danger")
                .from([DeliveryStatus::Pending, DeliveryStatus::Assigned])
                .to(DeliveryStatus::Cancelled)
                .permission(permissions::MANAGE)
                .reason_required(),
        ),
        leaving_the_run(
            Transition::new(AUTO_CANCEL)
                .label("Cancel (system)")
                .from([DeliveryStatus::Pending, DeliveryStatus::Assigned])
                .to(DeliveryStatus::Cancelled)
                .system()
                .reason_required(),
        ),
    ]
}

/// `assign`: the run is `planned` (the service worked it out); the delivery
/// gets its run and its place on it.
fn assign_transition() -> Transition<Delivery> {
    Transition::new(ASSIGN)
        .label("Assign")
        .tone("primary")
        .from([DeliveryStatus::Pending])
        .to(DeliveryStatus::Assigned)
        .permission(permissions::DISPATCH)
        .guard("run_planned", |_delivery: &Delivery, ctx| {
            let planned = input(ctx).is_some_and(|i| i.run_status == RunStatus::Planned);
            Box::pin(async move {
                if planned {
                    Ok(())
                } else {
                    Err("The run is not planned.".to_owned())
                }
            })
        })
        .effect(|delivery: &Delivery, ctx, tx| {
            let id = delivery.id;
            let input = input(ctx);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("an assign without its run"))
                })?;
                let run = Some(input.run_id);
                let sequence = Some(input.sequence);
                Delivery::query()
                    .where_(Delivery::ID.eq(id))
                    .update(&mut *tx, &[("run_id", &run), ("run_sequence", &sequence)])
                    .await?;
                Ok(())
            })
        })
}

/// `unassign`: while the run is not dispatched.
fn unassign_transition() -> Transition<Delivery> {
    leaving_the_run(
        Transition::new(UNASSIGN)
            .label("Unassign")
            .tone("warning")
            .from([DeliveryStatus::Assigned])
            .to(DeliveryStatus::Pending)
            .permission(permissions::DISPATCH)
            .guard("run_not_dispatched", |_delivery: &Delivery, ctx| {
                let open = input(ctx).is_some_and(|i| i.run_status == RunStatus::Planned);
                Box::pin(async move {
                    if open {
                        Ok(())
                    } else {
                        Err("The run is already dispatched.".to_owned())
                    }
                })
            }),
    )
}

/// `dispatch`: the system, with the run's `dispatch`.
fn dispatch_transition() -> Transition<Delivery> {
    Transition::new(DISPATCH)
        .label("Dispatch")
        .tone("primary")
        .from([DeliveryStatus::Assigned])
        .to(DeliveryStatus::OutForDelivery)
        .system()
        // The goods left: the untraced clock of `WRK.RUL.17` starts here.
        .effect(|delivery: &Delivery, _ctx, tx| {
            let id = delivery.id;
            Box::pin(async move {
                crate::trace::insert_in(
                    &mut *tx,
                    id,
                    crate::trace::DISPATCH,
                    "",
                    "",
                    renox::db::now(),
                    None,
                )
                .await
            })
        })
}

/// What makes a run.
#[derive(Debug, Clone)]
pub struct NewRun {
    /// The day it is driven.
    pub run_date: NaiveDate,
    /// The depot it leaves from.
    pub depot: String,
    /// The driver (a user).
    pub driver_user_id: i64,
    /// What it is driven with.
    pub vehicle_type: VehicleType,
    /// The vehicle (an asset); none for an on-foot rider.
    pub vehicle_asset_id: Option<i64>,
    /// The estimated duration in minutes.
    pub estimated_minutes: Option<i64>,
    /// The planned start.
    pub planned_start: Option<DateTime>,
}

impl NewRun {
    /// A run on `run_date` for `driver_user_id` in a van.
    pub fn new(run_date: NaiveDate, driver_user_id: i64) -> Self {
        NewRun {
            run_date,
            depot: String::new(),
            driver_user_id,
            vehicle_type: VehicleType::Van,
            vehicle_asset_id: None,
            estimated_minutes: None,
            planned_start: None,
        }
    }

    /// Drives with `vehicle_type`, an asset `vehicle_asset_id` for a
    /// vehicle.
    pub fn vehicle(mut self, vehicle_type: VehicleType, vehicle_asset_id: Option<i64>) -> Self {
        self.vehicle_type = vehicle_type;
        self.vehicle_asset_id = vehicle_asset_id;
        self
    }
}

fn unprocessable(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// Makes a `planned` run.
///
/// # Errors
///
/// 422 for a vehicle type that needs a vehicle and has none, or an on-foot
/// rider with one.
pub async fn create_run(state: &AppState, new: NewRun) -> Result<Run> {
    match (new.vehicle_type, new.vehicle_asset_id) {
        (VehicleType::OnFoot, Some(_)) => {
            return Err(unprocessable("An on-foot rider has no vehicle."));
        }
        (VehicleType::OnFoot, None) => {}
        (_, None) => return Err(unprocessable("A run needs its vehicle.")),
        (_, Some(_)) => {}
    }
    let number = RUN_SERIES
        .next_allowing_gaps(&state.db, None, None, new.run_date)
        .await?;
    let mut run = Run {
        number,
        run_date: new.run_date,
        depot: new.depot.trim().to_owned(),
        driver_user_id: new.driver_user_id,
        vehicle_type: new.vehicle_type,
        vehicle_asset_id: new.vehicle_asset_id,
        estimated_minutes: new.estimated_minutes,
        planned_start: new.planned_start,
        status: RunStatus::Planned,
        ..Run::default()
    };
    run.insert(&state.db).await?;
    Ok(run)
}

/// Why the driver of `run` cannot be planned for, if they cannot: on leave
/// on the run's date, or no longer an active employee (wl-hr's workforce).
/// A driver wl-hr does not know is available.
async fn driver_unavailable(state: &AppState, run: &Run) -> Result<Option<String>> {
    let workforce = workforce(state);
    let Some(employee) = workforce
        .employee_of_user(&state.db, run.driver_user_id)
        .await?
    else {
        return Ok(None);
    };
    let Some(profile) = workforce.staff(state, employee).await? else {
        return Ok(None);
    };
    if !profile.listed {
        return Ok(Some("The driver is not an active employee.".to_owned()));
    }
    if profile.on_leave && profile.returns_on.is_none_or(|back| back > run.run_date) {
        return Ok(Some(format!(
            "The driver is on leave on {}.",
            run.run_date.format("%Y-%m-%d")
        )));
    }
    Ok(None)
}

/// Puts `delivery` on `run`.
///
/// # Errors
///
/// `Guard` (422) when the run is not planned, the driver is not available,
/// or a document of the driver or vehicle the kinds' `blocks` list for
/// `delivery.assign` is not valid on the run's date; the transition's own
/// errors otherwise (`Forbidden` without `deliveries.dispatch`, `NotFrom`
/// unless the delivery is `pending`).
pub async fn assign(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    run: &Run,
) -> std::result::Result<(), TransitionError> {
    if run.status != RunStatus::Planned {
        return Err(TransitionError::Guard("The run is not planned.".to_owned()));
    }
    if let Some(message) = driver_unavailable(state, run)
        .await
        .map_err(TransitionError::Failed)?
    {
        return Err(TransitionError::Guard(message));
    }
    let on_run = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .count(&state.db)
        .await
        .map_err(TransitionError::Failed)?;
    let input = AssignInput {
        run_id: run.id,
        run_status: run.status,
        sequence: i64::try_from(on_run).unwrap_or(i64::MAX) + 1,
    };
    doc_guard::run(
        state,
        doc_guard::ASSIGN,
        run.id,
        run.run_date,
        Some(user.id),
        || async {
            delivery
                .transition(ASSIGN)
                .by(user)
                .input(&input)
                .run(state)
                .await
                .map(|_| ())
        },
    )
    .await?;
    state
        .emit(DeliveryAssigned {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            run: run.run_id(),
            driver: run.driver_user_id,
        })
        .await
        .map_err(TransitionError::Failed)
}

/// Takes `delivery` off its run while the run is not dispatched.
///
/// # Errors
///
/// `Guard` (422) when the run is dispatched; the transition's own errors
/// otherwise.
pub async fn unassign(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
) -> std::result::Result<(), TransitionError> {
    let run = match delivery.run_id {
        Some(id) => Run::find(&state.db, id)
            .await
            .map_err(TransitionError::Failed)?,
        None => None,
    };
    let input = AssignInput {
        run_id: delivery.run_id.unwrap_or_default(),
        run_status: run.map_or(RunStatus::Planned, |run| run.status),
        sequence: 0,
    };
    let _outcome = delivery
        .transition(UNASSIGN)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

/// Dispatches `run`: the documents are checked today, then the run and each
/// of its `assigned` deliveries move in one transaction.
///
/// # Errors
///
/// `Guard` (422) when the run is not planned or a document of the driver or
/// vehicle the kinds' `blocks` list for `run.dispatch` is not valid today;
/// the run is then `planned` still and no delivery moved.
pub async fn dispatch_run(
    state: &AppState,
    user: &AuthUser,
    run: &Run,
) -> std::result::Result<(), TransitionError> {
    if run.status != RunStatus::Planned {
        return Err(TransitionError::Guard("The run is not planned.".to_owned()));
    }
    let deliveries = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .where_(Delivery::STATUS.eq(DeliveryStatus::Assigned))
        .order_by(Delivery::RUN_SEQUENCE)
        .get(&state.db)
        .await
        .map_err(TransitionError::Failed)?;
    doc_guard::run(
        state,
        doc_guard::DISPATCH,
        run.id,
        today(),
        Some(user.id),
        || async {
            let mut tx = state
                .db
                .begin()
                .await
                .map_err(|e| TransitionError::Failed(e.into()))?;
            let mut outcomes = Vec::new();
            let first = run
                .transition(run_machine::DISPATCH)
                .by(user)
                .run_in(&mut tx)
                .await?;
            for delivery in &deliveries {
                outcomes.push(
                    delivery
                        .transition(DISPATCH)
                        .by_system()
                        .run_in(&mut tx)
                        .await?,
                );
            }
            tx.commit()
                .await
                .map_err(|e| TransitionError::Failed(e.into()))?;
            first.emit(state).await.map_err(TransitionError::Failed)?;
            for outcome in outcomes {
                outcome.emit(state).await.map_err(TransitionError::Failed)?;
            }
            Ok(())
        },
    )
    .await?;
    for delivery in &deliveries {
        state
            .emit(DeliveryOutForDelivery {
                delivery: delivery.delivery_id(),
                source: delivery.source(),
                window_from: delivery.window_from,
                window_to: delivery.window_to,
            })
            .await
            .map_err(TransitionError::Failed)?;
    }
    state
        .emit(RunDispatched {
            run: run.run_id(),
            driver: run.driver_user_id,
            vehicle: run.vehicle_asset_id,
            deliveries: deliveries.iter().map(Delivery::delivery_id).collect(),
        })
        .await
        .map_err(TransitionError::Failed)
}

/// Cancels `delivery` (`deliveries.manage`) with a reason.
///
/// # Errors
///
/// The transition's errors: `ReasonRequired` without a reason, `NotFrom`
/// unless it is `pending` or `assigned`.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    let _outcome = delivery
        .transition(CANCEL)
        .by(user)
        .reason(reason)
        .run(state)
        .await?;
    Ok(())
}

/// Cancels `delivery` as the system (its source was cancelled).
///
/// # Errors
///
/// As [`cancel`].
pub async fn cancel_as_system(
    state: &AppState,
    delivery: &Delivery,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    let _outcome = delivery
        .transition(AUTO_CANCEL)
        .by_system()
        .reason(reason)
        .run(state)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_run_defaults_to_a_van_without_a_vehicle_yet() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let new = NewRun::new(day, 7);
        assert_eq!(new.vehicle_type, VehicleType::Van);
        assert_eq!(new.vehicle_asset_id, None);
        let new = new.vehicle(VehicleType::Motorbike, Some(3));
        assert_eq!(new.vehicle_asset_id, Some(3));
    }
}
