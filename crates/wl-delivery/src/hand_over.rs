//! Handing open stops over to another driver's run (wl-delivery §3 *Run*
//! hand-overs, §4.2 `hand_over`, §5.3 step 1, issue #165).
//!
//! After a breakdown, an accident, a driver falling ill, or to balance the
//! load, the dispatcher moves chosen open stops from a `dispatched` run to
//! the run of **another** driver. It takes two steps, so no parcel and no
//! dirham changes hands unseen:
//!
//! 1. [`propose`] (`runs.dispatch`): the dispatcher chooses the stops, the
//!    receiving run and a reason. The receiving driver's and vehicle's
//!    documents are checked at once, today, with the same kinds as
//!    `run.dispatch` ([`crate::doc_guard`]); a refusal creates nothing. The
//!    proposal is a [`HandOver`] record that waits for the receiver.
//! 2. [`confirm`] (`runs.drive`, the receiving run's driver): the receiving
//!    driver scans or ticks each parcel and counts the cash. The documents
//!    are checked again, then the giving run's `hand_over` transition runs
//!    ([`transitions`]; dispatched to dispatched, by the system on the
//!    receiver's confirmation) and, in the same transaction, the stops still
//!    open move to the receiving run and stay `out_for_delivery`. A stop
//!    delivered, failed or moved meanwhile is skipped. After the commit
//!    `DeliveryAssigned` is emitted for each stop that moved and
//!    `RunHandedOver` once.
//!
//! The cash does not move here: `RunHandedOver` carries the amount the
//! receiver counted and `wl-payments` moves it between the two drivers'
//! handovers (BL-PAY-20, `FIN.CSH.01`); any difference stays on the giving
//! driver's handover and shows as a shortage at its count. Without the
//! counting setting ([`DeliverySettings::hand_over_counts_cash`] off) the
//! amount the giving driver's handover holds moves as recorded.
//!
//! The spec lets the receiving run be `planned`: the moved stops are
//! physically out, so they stay `out_for_delivery` and its `dispatch` skips
//! them (it dispatches `assigned` stops only). A run `cancel` is not built
//! yet; when it is, it must refuse a run that holds handed-over stops. The
//! window for perishable stops is the subject of the next paragraph.
//!
//! **Perishable stops (BL-DLV-10).** Cooked food handed to another rider must
//! still arrive within the brand's delivery promise. The dispatcher enters
//! the arrival the receiving run expects for each perishable stop
//! ([`Proposal::arrivals`]; there is no route optimisation in the MVP, the run's
//! estimated duration and sequence are only a hint on the form). The window
//! ends at the source's placed time plus its delivery promise
//! ([`window_end`], Asia/Dubai time is the display of the same instant): an
//! arrival after it, or none, refuses the proposal and the stop stays on the
//! giving run, where an `abort` fails it with *run aborted* and `WRK.RUL.16`
//! applies. An arrival exactly at the window end is accepted. A stop whose
//! source gave no promise has no end to compare with, but still needs its
//! arrival. Non-perishable stops need none.
//!
//! [`DeliverySettings::hand_over_counts_cash`]: crate::settings::DeliverySettings::hand_over_counts_cash

use std::sync::Mutex;

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{DeliveryAssigned, RunHandedOver};
use wl_kernel::documents::{dubai_date, dubai_text, today};

use crate::delivery::{Delivery, DeliveryStatus};
use crate::doc_guard;
use crate::permissions;
use crate::run::{Run, RunStatus};
use crate::settings::settings;
use crate::texts::english;
use crate::trace;

/// `hand_over`: dispatched → dispatched, on the giving run.
pub const HAND_OVER: &str = "hand_over";

/// Why stops are handed over (§4.2).
pub const REASONS: [&str; 4] = ["breakdown", "accident", "driver_unwell", "load_balancing"];

/// The latest arrival a receiving run may promise for the perishable
/// `delivery`: the source's placed time plus its delivery promise. `None`
/// when the source gave no promise.
pub fn window_end(delivery: &Delivery) -> Option<DateTime> {
    let placed = delivery.placed_at?;
    let minutes = delivery.promise_minutes?;
    Some(placed + renox::chrono::Duration::minutes(minutes))
}

/// Why the perishable `delivery` cannot be handed over with the expected
/// `arrival`: none given, or after the window end. `None` for a stop that may
/// go (and for a non-perishable one, which needs no arrival).
pub fn arrival_problem(delivery: &Delivery, arrival: Option<DateTime>) -> Option<String> {
    if !delivery.perishable {
        return None;
    }
    let Some(arrival) = arrival else {
        return Some(
            english("wl_delivery.hand_over.arrival_needed").replace("{number}", &delivery.number),
        );
    };
    match window_end(delivery) {
        Some(end) if arrival > end => Some(
            english("wl_delivery.hand_over.outside_window")
                .replace("{number}", &delivery.number)
                .replace("{end}", &dubai_text(end)),
        ),
        _ => None,
    }
}

/// An arrival as the dispatcher types it, in Asia/Dubai time (UTC+4): a full
/// `2026-06-01 19:40` (or with a `T`), or just `19:40`, which takes the day
/// of `base`. `None` when it reads as neither.
pub fn parse_arrival(text: &str, base: DateTime) -> Option<DateTime> {
    use renox::chrono::{Duration, NaiveDateTime, NaiveTime};
    let text = text.trim();
    let dubai = Duration::hours(4);
    for format in ["%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(local) = NaiveDateTime::parse_from_str(text, format) {
            return Some(local.and_utc() - dubai);
        }
    }
    let time = NaiveTime::parse_from_str(text, "%H:%M").ok()?;
    Some(dubai_date(base).and_time(time).and_utc() - dubai)
}

/// A hand-over from the giving run to the receiving run. It waits until the
/// receiving driver confirms ([`confirm`]); only then do the stops move.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_hand_overs", index(run_id), index(to_run_id))]
pub struct HandOver {
    /// The primary key.
    pub id: i64,
    /// The giving run.
    #[model(references = "wl_delivery_runs")]
    pub run_id: i64,
    /// The receiving run.
    #[model(references = "wl_delivery_runs")]
    pub to_run_id: i64,
    /// Why: one of [`REASONS`].
    pub reason: String,
    /// A note of the dispatcher.
    pub note: String,
    /// The dispatcher who proposed it.
    pub requested_by: i64,
    /// When it was proposed.
    pub requested_at: DateTime,
    /// The receiving driver who confirmed it.
    pub confirmed_by: Option<i64>,
    /// When it was confirmed.
    pub confirmed_at: Option<DateTime>,
    /// The cash the receiving driver counted, in fils.
    pub cash_counted: Option<i64>,
    /// The cheques the receiving driver confirmed (references, comma
    /// separated).
    pub cheques: String,
    /// Where it was confirmed, in millionths of a degree.
    pub latitude_micro: Option<i64>,
    /// Where it was confirmed, in millionths of a degree.
    pub longitude_micro: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A stop a hand-over names.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_delivery_hand_over_stops",
    unique(hand_over_id, delivery_id)
)]
pub struct HandOverStop {
    /// The primary key.
    pub id: i64,
    /// The hand-over.
    #[model(references = "wl_delivery_hand_overs")]
    pub hand_over_id: i64,
    /// The delivery.
    #[model(references = "wl_delivery_deliveries")]
    pub delivery_id: i64,
    /// Whether the stop moved at the confirmation (it was still open).
    pub moved: bool,
    /// The arrival the dispatcher expects on the receiving run; needed for a
    /// perishable stop (BL-DLV-10).
    pub expected_arrival: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl HandOver {
    /// Whether the receiving driver confirmed it.
    pub fn is_confirmed(&self) -> bool {
        self.confirmed_at.is_some()
    }

    /// The hand-overs of the giving run `run_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_run(db: &Db, run_id: i64) -> Result<Vec<HandOver>> {
        HandOver::query()
            .where_(HandOver::RUN_ID.eq(run_id))
            .order_by(HandOver::ID)
            .get(db)
            .await
    }

    /// The deliveries the hand-over names, in the order they were chosen.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn deliveries(&self, db: &Db) -> Result<Vec<Delivery>> {
        let stops = HandOverStop::query()
            .where_(HandOverStop::HAND_OVER_ID.eq(self.id))
            .order_by(HandOverStop::ID)
            .get(db)
            .await?;
        let mut deliveries = Vec::new();
        for stop in stops {
            if let Some(delivery) = Delivery::find(db, stop.delivery_id).await? {
                deliveries.push(delivery);
            }
        }
        Ok(deliveries)
    }
}

/// What the dispatcher chooses.
#[derive(Debug, Clone, Default)]
pub struct Proposal {
    /// The receiving run.
    pub to_run_id: i64,
    /// One of [`REASONS`].
    pub reason: String,
    /// A note.
    pub note: String,
    /// The open stops (deliveries) to hand over.
    pub delivery_ids: Vec<i64>,
    /// The arrival the receiving run expects per stop, `(delivery id,
    /// arrival)`: needed for each perishable stop (BL-DLV-10).
    pub arrivals: Vec<(i64, DateTime)>,
}

/// What the receiving driver confirms.
#[derive(Debug, Clone, Default)]
pub struct Confirmation {
    /// The deliveries whose parcel was scanned or ticked.
    pub parcels: Vec<i64>,
    /// The cash counted, in fils; none when it was not entered.
    pub cash: Option<i64>,
    /// The references of the cheques received.
    pub cheques: Vec<String>,
    /// Where the driver is, in millionths of a degree.
    pub location: (Option<i64>, Option<i64>),
}

/// What `hand_over` runs on, worked out by [`confirm`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HandOverInput {
    pub to_run_status: RunStatus,
    pub to_driver: i64,
    pub from_driver: i64,
    pub stops_left: i64,
    pub unconfirmed: i64,
}

fn input(ctx: &TransitionCtx) -> Option<HandOverInput> {
    ctx.input_as::<HandOverInput>().ok()
}

fn guard(problem: Option<String>) -> std::result::Result<(), String> {
    problem.map_or(Ok(()), Err)
}

/// The run transition of this module, for the machine: dispatched to
/// dispatched, run by the system once the receiving driver confirmed.
pub(crate) fn transitions() -> Vec<Transition<Run>> {
    vec![
        Transition::new(HAND_OVER)
            .label("Hand over")
            .tone("warning")
            .from([RunStatus::Dispatched])
            .to(RunStatus::Dispatched)
            .system()
            .reason_required()
            .guard("receiving_run_open", |_run: &Run, ctx| {
                let problem = match input(ctx) {
                    Some(i)
                        if matches!(
                            i.to_run_status,
                            RunStatus::Dispatched | RunStatus::Planned
                        ) =>
                    {
                        None
                    }
                    _ => Some(english("wl_delivery.hand_over.to_run_closed").to_owned()),
                };
                Box::pin(async move { guard(problem) })
            })
            .guard("to_another_driver", |run: &Run, ctx| {
                let problem = match input(ctx) {
                    Some(i) if i.to_driver != run.driver_user_id => None,
                    _ => Some(english("wl_delivery.hand_over.same_driver").to_owned()),
                };
                Box::pin(async move { guard(problem) })
            })
            .guard("a_stop_is_left", |_run: &Run, ctx| {
                let problem = match input(ctx) {
                    Some(i) if i.stops_left > 0 => None,
                    _ => Some(english("wl_delivery.hand_over.nothing_left").to_owned()),
                };
                Box::pin(async move { guard(problem) })
            })
            .guard("receiver_confirmed_each_parcel", |_run: &Run, ctx| {
                let problem = match input(ctx) {
                    Some(i) if i.unconfirmed == 0 => None,
                    Some(i) => Some(
                        english("wl_delivery.hand_over.parcels_unconfirmed")
                            .replace("{count}", &i.unconfirmed.to_string()),
                    ),
                    None => Some("The hand-over was not prepared by the run service.".to_owned()),
                };
                Box::pin(async move { guard(problem) })
            }),
    ]
}

fn refused(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// The reason's words, for the history and the notices.
pub fn reason_text(code: &str) -> String {
    english(&format!("wl_delivery.hand_over.reason.{code}")).to_owned()
}

/// The dispatcher proposes to hand `proposal.delivery_ids` of `run` over to
/// `proposal.to_run_id`. Nothing moves until the receiving driver confirms.
///
/// # Errors
///
/// `Forbidden` without `runs.dispatch`; `Guard` (422) unless `run` is
/// `dispatched`, the reason is one of [`REASONS`], the receiving run is
/// another run of another driver and `planned` or `dispatched`, every stop is
/// an open stop of `run`, and the receiving driver's and vehicle's documents
/// are valid today (the refusal names the document).
pub async fn propose(
    state: &AppState,
    user: &AuthUser,
    run: &Run,
    proposal: &Proposal,
) -> std::result::Result<HandOver, TransitionError> {
    if !user.has_permission(permissions::RUNS_DISPATCH) {
        return Err(TransitionError::Forbidden);
    }
    if run.status != RunStatus::Dispatched {
        return Err(refused(english("wl_delivery.hand_over.not_dispatched")));
    }
    if !REASONS.contains(&proposal.reason.as_str()) {
        return Err(refused(english("wl_delivery.hand_over.reason_unknown")));
    }
    if proposal.delivery_ids.is_empty() {
        return Err(refused(english("wl_delivery.hand_over.no_stops")));
    }
    if proposal.to_run_id == run.id {
        return Err(refused(english("wl_delivery.hand_over.same_run")));
    }
    let to_run = Run::find(&state.db, proposal.to_run_id)
        .await
        .map_err(TransitionError::Failed)?
        .ok_or_else(|| refused(english("wl_delivery.hand_over.to_run_unknown")))?;
    if to_run.driver_user_id == run.driver_user_id {
        return Err(refused(english("wl_delivery.hand_over.same_driver")));
    }
    if !matches!(to_run.status, RunStatus::Dispatched | RunStatus::Planned) {
        return Err(refused(english("wl_delivery.hand_over.to_run_closed")));
    }
    let mut stops = Vec::new();
    for id in &proposal.delivery_ids {
        let delivery = Delivery::find(&state.db, *id)
            .await
            .map_err(TransitionError::Failed)?;
        match delivery {
            Some(d) if d.run_id == Some(run.id) && d.status == DeliveryStatus::OutForDelivery => {
                stops.push(d);
            }
            Some(d) => {
                return Err(refused(
                    english("wl_delivery.hand_over.not_open_stop").replace("{number}", &d.number),
                ));
            }
            None => return Err(refused(english("wl_delivery.hand_over.no_stops"))),
        }
    }
    // A perishable stop needs the arrival the receiving run expects, within
    // the window; a refusal creates nothing and the stop stays on this run.
    let arrival_of = |id: i64| {
        proposal
            .arrivals
            .iter()
            .find(|(stop, _)| *stop == id)
            .map(|(_, at)| *at)
    };
    for stop in &stops {
        if let Some(problem) = arrival_problem(stop, arrival_of(stop.id)) {
            return Err(refused(problem));
        }
    }
    // The receiving driver and vehicle pass the document check of
    // `run.dispatch` today; a refusal creates nothing.
    let made: Mutex<Option<HandOver>> = Mutex::new(None);
    doc_guard::run(
        state,
        doc_guard::DISPATCH,
        to_run.id,
        today(),
        Some(user.id),
        || async {
            let mut hand_over = HandOver {
                run_id: run.id,
                to_run_id: to_run.id,
                reason: proposal.reason.clone(),
                note: proposal.note.trim().to_owned(),
                requested_by: user.id,
                requested_at: renox::db::now(),
                ..HandOver::default()
            };
            hand_over
                .insert(&state.db)
                .await
                .map_err(TransitionError::Failed)?;
            for stop in &stops {
                let mut row = HandOverStop {
                    hand_over_id: hand_over.id,
                    delivery_id: stop.id,
                    expected_arrival: stop.perishable.then(|| arrival_of(stop.id)).flatten(),
                    ..HandOverStop::default()
                };
                row.insert(&state.db)
                    .await
                    .map_err(TransitionError::Failed)?;
            }
            *made
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hand_over);
            Ok(())
        },
    )
    .await?;
    made.into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .ok_or_else(|| refused(english("wl_delivery.hand_over.no_stops")))
}

/// The receiving driver `user` confirms `hand_over`: each parcel scanned or
/// ticked, the cash counted. The giving run's `hand_over` runs, the stops
/// still open move to the receiving run, and `DeliveryAssigned` (each stop)
/// and `RunHandedOver` are emitted after the commit.
///
/// # Errors
///
/// `Forbidden` without `runs.drive`; `Guard` (422) for anyone but the
/// receiving run's driver, a hand-over already confirmed, a giving run that
/// is no longer `dispatched`, a receiving run that is closed, a parcel not
/// confirmed, cash not counted (or negative), no stop left open, or a
/// document of the receiving driver or vehicle that is not valid today.
pub async fn confirm(
    state: &AppState,
    user: &AuthUser,
    hand_over: &HandOver,
    confirmation: &Confirmation,
) -> std::result::Result<(), TransitionError> {
    if !user.has_permission(permissions::RUNS_DRIVE) {
        return Err(TransitionError::Forbidden);
    }
    if hand_over.is_confirmed() {
        return Err(refused(english("wl_delivery.hand_over.already_confirmed")));
    }
    let run = Run::find_or_404(&state.db, hand_over.run_id)
        .await
        .map_err(TransitionError::Failed)?;
    let to_run = Run::find_or_404(&state.db, hand_over.to_run_id)
        .await
        .map_err(TransitionError::Failed)?;
    if user.id != to_run.driver_user_id {
        return Err(refused(english("wl_delivery.hand_over.only_receiver")));
    }
    if run.status != RunStatus::Dispatched {
        return Err(refused(english("wl_delivery.hand_over.not_dispatched")));
    }
    if !matches!(to_run.status, RunStatus::Dispatched | RunStatus::Planned) {
        return Err(refused(english("wl_delivery.hand_over.to_run_closed")));
    }
    // Only the stops still open count: one delivered on the giving driver's
    // phone meanwhile is skipped.
    let open: Vec<Delivery> = hand_over
        .deliveries(&state.db)
        .await
        .map_err(TransitionError::Failed)?
        .into_iter()
        .filter(|d| d.run_id == Some(run.id) && d.status == DeliveryStatus::OutForDelivery)
        .collect();
    let unconfirmed = open
        .iter()
        .filter(|d| !confirmation.parcels.contains(&d.id))
        .count();
    let cash = cash_to_move(state, &run, confirmation).await?;
    let reason = reason_text(&hand_over.reason);
    let input = HandOverInput {
        to_run_status: to_run.status,
        to_driver: to_run.driver_user_id,
        from_driver: run.driver_user_id,
        stops_left: i64::try_from(open.len()).unwrap_or(i64::MAX),
        unconfirmed: i64::try_from(unconfirmed).unwrap_or(i64::MAX),
    };
    let moved_ids: Vec<i64> = open.iter().map(|d| d.id).collect();
    doc_guard::run(
        state,
        doc_guard::DISPATCH,
        to_run.id,
        today(),
        Some(user.id),
        || async {
            let mut tx = state
                .db
                .begin()
                .await
                .map_err(|e| TransitionError::Failed(e.into()))?;
            let outcome = run
                .transition(HAND_OVER)
                .by_system()
                .reason(reason.clone())
                .input(&input)
                .run_in(&mut tx)
                .await?;
            let now = renox::db::now();
            let mut sequence = i64::try_from(
                Delivery::query()
                    .where_(Delivery::RUN_ID.eq(to_run.id))
                    .count(&mut tx)
                    .await
                    .map_err(TransitionError::Failed)?,
            )
            .unwrap_or(i64::MAX);
            for delivery in &open {
                sequence += 1;
                let run_id = Some(to_run.id);
                let place = Some(sequence);
                Delivery::query()
                    .where_(Delivery::ID.eq(delivery.id))
                    .update(&mut tx, &[("run_id", &run_id), ("run_sequence", &place)])
                    .await
                    .map_err(TransitionError::Failed)?;
                trace::insert_in(
                    &mut tx,
                    delivery.id,
                    trace::HAND_OVER,
                    &reason,
                    "",
                    now,
                    Some(user.id),
                )
                .await
                .map_err(TransitionError::Failed)?;
            }
            let moved = true;
            HandOverStop::query()
                .where_(HandOverStop::HAND_OVER_ID.eq(hand_over.id))
                .where_(HandOverStop::DELIVERY_ID.is_in(moved_ids.clone()))
                .update(&mut tx, &[("moved", &moved)])
                .await
                .map_err(TransitionError::Failed)?;
            let by = Some(user.id);
            let at = Some(now);
            let counted = Some(cash);
            let cheques = confirmation.cheques.join(",");
            HandOver::query()
                .where_(HandOver::ID.eq(hand_over.id))
                .update(
                    &mut tx,
                    &[
                        ("confirmed_by", &by),
                        ("confirmed_at", &at),
                        ("cash_counted", &counted),
                        ("cheques", &cheques),
                        ("latitude_micro", &confirmation.location.0),
                        ("longitude_micro", &confirmation.location.1),
                    ],
                )
                .await
                .map_err(TransitionError::Failed)?;
            // The moved deliveries and both runs are found where they are
            // now (BL-AGT-11).
            for delivery in &open {
                crate::search::index_delivery_in(&mut tx, delivery.id, delivery.run_id)
                    .await
                    .map_err(TransitionError::Failed)?;
            }
            crate::search::index_run_in(&mut tx, run.id)
                .await
                .map_err(TransitionError::Failed)?;
            tx.commit()
                .await
                .map_err(|e| TransitionError::Failed(e.into()))?;
            outcome.emit(state).await.map_err(TransitionError::Failed)?;
            Ok(())
        },
    )
    .await?;
    for delivery in &open {
        state
            .emit(DeliveryAssigned {
                delivery: delivery.delivery_id(),
                source: delivery.source(),
                run: to_run.run_id(),
                driver: to_run.driver_user_id,
            })
            .await
            .map_err(TransitionError::Failed)?;
    }
    state
        .emit(RunHandedOver {
            hand_over: hand_over.id,
            run: run.run_id(),
            to_run: to_run.run_id(),
            from_driver: run.driver_user_id,
            to_driver: to_run.driver_user_id,
            requested_by: hand_over.requested_by,
            deliveries: open.iter().map(Delivery::delivery_id).collect(),
            cash_counted: cash,
            cheques: confirmation.cheques.clone(),
            reason: hand_over.reason.clone(),
            to_period: to_run.cash_period(settings(state).replace_cash_escalation()),
        })
        .await
        .map_err(TransitionError::Failed)
}

/// The cash that moves with the stops: what the receiving driver counted, or,
/// with the counting setting off, what the giving driver's handover holds.
async fn cash_to_move(
    state: &AppState,
    run: &Run,
    confirmation: &Confirmation,
) -> std::result::Result<i64, TransitionError> {
    let held = crate::close::handover_of(state, run)
        .await
        .map_err(TransitionError::Failed)?
        .map_or(0, |handover| handover.expected);
    if !settings(state).hand_over_counts_cash() {
        return Ok(held);
    }
    match confirmation.cash {
        Some(cash) if cash < 0 => Err(refused(english("wl_delivery.hand_over.negative_cash"))),
        Some(cash) => Ok(cash),
        None if held > 0 => Err(refused(english("wl_delivery.hand_over.count_cash"))),
        None => Ok(0),
    }
}

/// The hand-overs waiting for `driver_user_id` to confirm them: proposed for
/// a run of theirs whose giving run is still `dispatched`.
///
/// # Errors
///
/// Database errors.
pub async fn waiting_for(db: &Db, driver_user_id: i64) -> Result<Vec<HandOver>> {
    let runs: Vec<i64> = Run::query()
        .where_(Run::DRIVER_USER_ID.eq(driver_user_id))
        .get(db)
        .await?
        .into_iter()
        .map(|run| run.id)
        .collect();
    if runs.is_empty() {
        return Ok(Vec::new());
    }
    let mut waiting = Vec::new();
    for hand_over in HandOver::query()
        .where_(HandOver::TO_RUN_ID.is_in(runs))
        .where_(HandOver::CONFIRMED_AT.is_null())
        .order_by(HandOver::ID)
        .get(db)
        .await?
    {
        let giving = Run::find(db, hand_over.run_id).await?;
        if giving.is_some_and(|run| run.status == RunStatus::Dispatched) {
            waiting.push(hand_over);
        }
    }
    Ok(waiting)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(to_driver: i64, left: i64, unconfirmed: i64, status: RunStatus) -> HandOverInput {
        HandOverInput {
            to_run_status: status,
            to_driver,
            from_driver: 1,
            stops_left: left,
            unconfirmed,
        }
    }

    fn food(placed: &str, promise: i64) -> Delivery {
        Delivery {
            number: "DLV-1".into(),
            perishable: true,
            placed_at: Some(placed.parse().unwrap()),
            promise_minutes: Some(promise),
            ..Delivery::default()
        }
    }

    #[test]
    fn a_perishable_stop_may_arrive_up_to_the_window_end_and_not_after() {
        // Placed 19:00 Dubai (15:00 UTC), a 45-minute promise: 19:45.
        let delivery = food("2026-06-01T15:00:00Z", 45);
        let at = |hhmm: &str| parse_arrival(hhmm, delivery.placed_at.unwrap());
        assert!(arrival_problem(&delivery, at("19:40")).is_none());
        assert!(arrival_problem(&delivery, at("19:45")).is_none());
        assert!(arrival_problem(&delivery, at("19:50")).is_some());
        assert!(arrival_problem(&delivery, None).is_some());
    }

    #[test]
    fn a_stop_that_is_not_perishable_needs_no_arrival() {
        let mut delivery = food("2026-06-01T15:00:00Z", 45);
        delivery.perishable = false;
        assert!(arrival_problem(&delivery, None).is_none());
    }

    #[test]
    fn an_arrival_is_read_in_dubai_time() {
        let base: DateTime = "2026-06-01T15:00:00Z".parse().unwrap();
        let expected: DateTime = "2026-06-01T15:40:00Z".parse().unwrap();
        assert_eq!(parse_arrival("19:40", base), Some(expected));
        assert_eq!(parse_arrival("2026-06-01 19:40", base), Some(expected));
        assert_eq!(parse_arrival("soon", base), None);
    }

    #[test]
    fn the_transition_keeps_the_run_dispatched_and_is_the_systems() {
        let machine = Run::machine();
        let hand_over = machine.find(HAND_OVER).expect("hand_over");
        assert_eq!(hand_over.from_states(), ["dispatched"]);
        assert_eq!(hand_over.to_state(), Some("dispatched"));
        assert!(hand_over.is_system());
    }

    #[test]
    fn there_are_four_reasons_and_each_has_words() {
        assert_eq!(REASONS.len(), 4);
        for reason in REASONS {
            assert_ne!(
                reason_text(reason),
                format!("wl_delivery.hand_over.reason.{reason}")
            );
        }
    }

    #[test]
    fn the_input_round_trips_through_json() {
        let value = renox::serde_json::to_value(input(2, 3, 0, RunStatus::Planned)).unwrap();
        let back: HandOverInput = renox::serde_json::from_value(value).unwrap();
        assert_eq!(back.stops_left, 3);
        assert_eq!(back.to_run_status, RunStatus::Planned);
    }
}
