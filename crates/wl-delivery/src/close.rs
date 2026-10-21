//! A run back at the depot and closed (wl-delivery §4.2 `return_to_depot` and
//! `close`, §5.4, issue #167; `return_to_depot` is the small base the close
//! needs).
//!
//! - [`return_to_depot`] is the driver's: every stop has an outcome (a stop
//!   still `out_for_delivery` is refused: mark it failed first), and the run
//!   is `returned_to_depot`.
//! - At the depot the list of the run's items ([`items_of`]) is every
//!   undelivered delivery (`attempted`) and every refused line of a delivered
//!   one. The depot user ticks each *received* or *missing* with [`receive`]:
//!   an undelivered delivery as a whole, a refused line by quantity (part
//!   received and part missing is allowed, and the two add up to the refused
//!   quantity). The cash the driver handed over is counted by someone else
//!   with `cash.count` in `wl-payments`: the driver's handover of the run is
//!   in the cash custody (`FIN.CSH.01`, [`wl_kernel::custody`]), filled at
//!   each `deliver`, and the run closes on `CashHandoverCounted` (counted or
//!   short: a shortage does not block the close). Without the payments
//!   module nothing is held, and [`count_cash`] keeps the amount counted as
//!   a record of this module (#167); with it that record is refused.
//! - [`try_close`] is the system's `close`: it runs when the cash is counted
//!   (or no cash was collected) and every item is accounted for, and never
//!   before. It records the received and missing refused quantities on the
//!   delivery lines, leaves a depot receipt as a trace for each undelivered
//!   delivery received back (a missing one leaves none: its untraced clock of
//!   `WRK.RUL.17` does not restart), and, after the commit, emits
//!   `DeliveryRefusedReceived` per delivery with refused lines and
//!   `RunClosed`, and opens a task for each item reported missing. A delivery
//!   reported missing stays `attempted`; a delivery with refused lines stays
//!   `delivered`. [`receive`], [`count_cash`] and [`return_to_depot`] each try
//!   to close at the end, so the run closes by itself as soon as the last
//!   condition is met.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::custody::{
    CashHandoverCounted, HandoverStatus, HandoverSummary, custody, custody_provided,
};
use wl_kernel::delivery::{
    DeliveryRefusedReceived, RUN_SUBJECT_TYPE, RefusedLineReceived, RunClosed,
};
use wl_kernel::tickets::{self, OpenTicket, TASK_CATEGORY};

use crate::delivery::{Delivery, DeliveryLine, DeliveryStatus};
use crate::permissions;
use crate::run::{Run, RunStatus};
use crate::texts::english;
use crate::trace;

/// `return_to_depot`: dispatched → returned_to_depot, by the driver.
pub const RETURN_TO_DEPOT: &str = "return_to_depot";
/// `close`: returned_to_depot → closed, by the system.
pub const CLOSE: &str = "close";

/// The quantity of an undelivered delivery as one item: all of it received or
/// all of it missing.
pub const WHOLE: i64 = 1000;

/// What the depot ticked for one item of a run.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_delivery_receipts",
    unique(run_id, delivery_id, delivery_line_id),
    index(run_id)
)]
pub struct Receipt {
    /// The primary key.
    pub id: i64,
    /// The run.
    #[model(references = "wl_delivery_runs")]
    pub run_id: i64,
    /// The delivery.
    #[model(references = "wl_delivery_deliveries")]
    pub delivery_id: i64,
    /// The refused line, or 0 for an undelivered delivery as a whole.
    pub delivery_line_id: i64,
    /// The quantity received back (thousandths; a whole delivery: 1000).
    pub received_milli: i64,
    /// The quantity reported missing.
    pub missing_milli: i64,
    /// The depot user.
    pub recorded_by: Option<i64>,
    /// When it was ticked.
    pub recorded_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One item the depot accounts for.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Item {
    /// The delivery.
    pub delivery_id: i64,
    /// The refused line, or 0 for an undelivered delivery as a whole.
    pub line_id: i64,
    /// The delivery's number.
    pub number: String,
    /// What it is.
    pub description: String,
    /// The quantity to account for (thousandths; a whole delivery: 1000).
    pub expected_milli: i64,
    /// What was ticked received.
    pub received_milli: i64,
    /// What was ticked missing.
    pub missing_milli: i64,
    /// Whether the item is accounted for.
    pub accounted: bool,
}

/// What the depot ticks for an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The delivery.
    pub delivery_id: i64,
    /// The refused line, or 0 for an undelivered delivery as a whole.
    pub line_id: i64,
    /// The quantity received back.
    pub received_milli: i64,
    /// The quantity reported missing.
    pub missing_milli: i64,
}

/// What `return_to_depot` runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReturnInput {
    pub driver_user_id: i64,
    pub open_stops: i64,
}

/// What `close` runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CloseInput {
    pub cash_expected: i64,
    pub cash_counted: bool,
    pub open_items: i64,
}

/// The guard "cash handover counted (or no cash)".
pub(crate) fn cash_ok(input: Option<&CloseInput>) -> std::result::Result<(), String> {
    match input {
        Some(input) if input.cash_expected <= 0 || input.cash_counted => Ok(()),
        Some(_) => Err(english("wl_delivery.close.cash_not_counted").to_owned()),
        None => Err("The close was not prepared by the run service.".to_owned()),
    }
}

/// The guard "undelivered and refused goods received back, or reported
/// missing".
pub(crate) fn items_ok(input: Option<&CloseInput>) -> std::result::Result<(), String> {
    match input {
        Some(input) if input.open_items == 0 => Ok(()),
        Some(input) => Err(english("wl_delivery.close.items_open")
            .replace("{count}", &input.open_items.to_string())),
        None => Err("The close was not prepared by the run service.".to_owned()),
    }
}

/// The guard "every stop has an outcome".
pub(crate) fn stops_ok(input: Option<&ReturnInput>) -> std::result::Result<(), String> {
    match input {
        Some(input) if input.open_stops == 0 => Ok(()),
        Some(input) => Err(english("wl_delivery.close.stops_open")
            .replace("{count}", &input.open_stops.to_string())),
        None => Err("The return was not prepared by the run service.".to_owned()),
    }
}

fn return_input(ctx: &TransitionCtx) -> Option<ReturnInput> {
    ctx.input_as::<ReturnInput>().ok()
}

fn close_input(ctx: &TransitionCtx) -> Option<CloseInput> {
    ctx.input_as::<CloseInput>().ok()
}

/// The run transitions of this module, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Run>> {
    vec![
        Transition::new(RETURN_TO_DEPOT)
            .label("Return to the depot")
            .tone("primary")
            .from([RunStatus::Dispatched])
            .to(RunStatus::ReturnedToDepot)
            .permission(permissions::RUNS_DRIVE)
            .guard("by_the_runs_driver", |_run: &Run, ctx| {
                let driver = return_input(ctx).map(|i| i.driver_user_id);
                let allowed =
                    matches!((ctx.actor(), driver), (Some(user), Some(d)) if user.id == d);
                Box::pin(async move {
                    if allowed {
                        Ok(())
                    } else {
                        Err("Only the driver of the run brings it back to the depot.".to_owned())
                    }
                })
            })
            .guard("every_stop_has_an_outcome", |_run: &Run, ctx| {
                let checked = stops_ok(return_input(ctx).as_ref());
                Box::pin(async move { checked })
            })
            .effect(|run: &Run, _ctx, tx| {
                let id = run.id;
                Box::pin(async move {
                    let at = Some(renox::db::now());
                    Run::query()
                        .where_(Run::ID.eq(id))
                        .update(&mut *tx, &[("returned_at", &at)])
                        .await?;
                    Ok(())
                })
            }),
        Transition::new(CLOSE)
            .label("Close")
            .tone("success")
            .from([RunStatus::ReturnedToDepot])
            .to(RunStatus::Closed)
            .system()
            .guard("cash_counted", |_run: &Run, ctx| {
                let checked = cash_ok(close_input(ctx).as_ref());
                Box::pin(async move { checked })
            })
            .guard("items_accounted", |_run: &Run, ctx| {
                let checked = items_ok(close_input(ctx).as_ref());
                Box::pin(async move { checked })
            })
            .effect(|run: &Run, _ctx, tx| {
                let id = run.id;
                Box::pin(async move { record_close(tx, id).await })
            }),
    ]
}

/// The effect of `close`: the lines get what was ticked, a received
/// undelivered delivery leaves a depot receipt, the run its closing time.
async fn record_close(tx: &mut Transaction, run_id: i64) -> Result<()> {
    let now = renox::db::now();
    let receipts = Receipt::query()
        .where_(Receipt::RUN_ID.eq(run_id))
        .order_by(Receipt::ID)
        .get(&mut *tx)
        .await?;
    for receipt in receipts {
        if receipt.delivery_line_id != 0 {
            let at = Some(now);
            DeliveryLine::query()
                .where_(DeliveryLine::ID.eq(receipt.delivery_line_id))
                .update(
                    &mut *tx,
                    &[
                        ("refused_received_milli", &receipt.received_milli),
                        ("refused_missing_milli", &receipt.missing_milli),
                        ("accounted_at", &at),
                        ("accounted_by", &receipt.recorded_by),
                    ],
                )
                .await?;
        } else if receipt.received_milli > 0 {
            trace::insert_in(
                &mut *tx,
                receipt.delivery_id,
                trace::DEPOT_RECEIPT,
                "",
                "",
                now,
                receipt.recorded_by,
            )
            .await?;
        }
    }
    let at = Some(now);
    Run::query()
        .where_(Run::ID.eq(run_id))
        .update(&mut *tx, &[("closed_at", &at)])
        .await?;
    Ok(())
}

/// Closes the run once the driver's handover of it is counted: a listener on
/// the cash custody's `CashHandoverCounted` (counted or short).
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: CashHandoverCounted, state: AppState| async move {
        match event.period {
            Some(period) if period.subject_type == RUN_SUBJECT_TYPE => {
                try_close(&state, period.subject_id)
                    .await
                    .map(|_| ())
                    .map_err(TransitionError::into_error)
            }
            _ => Ok(()),
        }
    });
}

fn unprocessable(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// The items the depot accounts for on `run`: every `attempted` delivery as a
/// whole, and every refused line of a delivered one, with what was ticked.
///
/// # Errors
///
/// Database errors.
pub async fn items_of(db: &Db, run: &Run) -> Result<Vec<Item>> {
    let deliveries = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .order_by(Delivery::RUN_SEQUENCE)
        .get(db)
        .await?;
    let receipts = Receipt::query()
        .where_(Receipt::RUN_ID.eq(run.id))
        .get(db)
        .await?;
    let ticked = |delivery_id: i64, line_id: i64| {
        receipts
            .iter()
            .find(|r| r.delivery_id == delivery_id && r.delivery_line_id == line_id)
    };
    let mut items = Vec::new();
    for delivery in &deliveries {
        match delivery.status {
            DeliveryStatus::Attempted => {
                let receipt = ticked(delivery.id, 0);
                let (received, missing) =
                    receipt.map_or((0, 0), |r| (r.received_milli, r.missing_milli));
                items.push(Item {
                    delivery_id: delivery.id,
                    line_id: 0,
                    number: delivery.number.clone(),
                    description: delivery.recipient_name.clone(),
                    expected_milli: WHOLE,
                    received_milli: received,
                    missing_milli: missing,
                    accounted: received + missing == WHOLE,
                });
            }
            DeliveryStatus::Delivered => {
                for line in delivery.lines(db).await? {
                    if line.refused_milli <= 0 {
                        continue;
                    }
                    let receipt = ticked(delivery.id, line.id);
                    let (received, missing) =
                        receipt.map_or((0, 0), |r| (r.received_milli, r.missing_milli));
                    items.push(Item {
                        delivery_id: delivery.id,
                        line_id: line.id,
                        number: delivery.number.clone(),
                        description: line.description.clone(),
                        expected_milli: line.refused_milli,
                        received_milli: received,
                        missing_milli: missing,
                        accounted: received + missing == line.refused_milli,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(items)
}

/// The cash collected on the run's deliveries that the driver hands over, in
/// fils: the cash of the saved proofs of the deliveries still `delivered` or
/// under review.
///
/// # Errors
///
/// Database errors.
pub async fn cash_expected(db: &Db, run: &Run) -> Result<i64> {
    let deliveries = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .where_(Delivery::STATUS.is_in([DeliveryStatus::Delivered, DeliveryStatus::UnderReview]))
        .get(db)
        .await?;
    let mut total = 0;
    for delivery in deliveries {
        if let Some(proof) = delivery.latest_proof(db).await? {
            total += proof.cash_collected;
        }
    }
    Ok(total)
}

/// The driver's handover of `run` in the cash custody, when a custody is
/// provided and cash was collected on the run.
///
/// # Errors
///
/// Database errors.
pub async fn handover_of(state: &AppState, run: &Run) -> Result<Option<HandoverSummary>> {
    if !custody_provided(state) {
        return Ok(None);
    }
    custody(state)
        .handover_of(&state.db, run.driver_user_id, &run.subject())
        .await
}

/// The cash the depot must count for `run` and whether it has been:
/// with the cash custody the driver's handover of the run (counted or short
/// is counted, `open` is not), else the amount `collected` and the record of
/// [`count_cash`].
async fn cash_to_count(state: &AppState, run: &Run, collected: i64) -> Result<(i64, bool)> {
    if !custody_provided(state) {
        return Ok((collected, run.cash_counted_at.is_some()));
    }
    Ok(match handover_of(state, run).await? {
        Some(handover) => (handover.expected, handover.status != HandoverStatus::Open),
        None => (0, true),
    })
}

/// The driver brings `run` back to the depot.
///
/// # Errors
///
/// `Forbidden` without `runs.drive`; `Guard` (422) for anyone but the run's
/// driver, or while a stop has no outcome; `NotFrom` unless the run is
/// `dispatched`.
pub async fn return_to_depot(
    state: &AppState,
    user: &AuthUser,
    run: &Run,
) -> std::result::Result<(), TransitionError> {
    let open_stops = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .where_(Delivery::STATUS.eq(DeliveryStatus::OutForDelivery))
        .count(&state.db)
        .await
        .map_err(TransitionError::Failed)?;
    let input = ReturnInput {
        driver_user_id: run.driver_user_id,
        open_stops: i64::try_from(open_stops).unwrap_or(i64::MAX),
    };
    let _outcome = run
        .transition(RETURN_TO_DEPOT)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    // The driver's cash period ends now (or at its planned end, whichever is
    // earlier): `wl-payments` starts its escalation clock.
    custody(state)
        .end_period(state, run.driver_user_id, run.subject(), renox::db::now())
        .await
        .map_err(TransitionError::Failed)?;
    // A run with nothing to account for closes at once.
    try_close(state, run.id).await.map(|_| ())
}

/// Ticks the items of `entries` for `user` (a holder of `runs.dispatch`, at
/// the depot): each undelivered delivery is wholly received or wholly
/// missing; each refused line has its received and missing quantities, which
/// add up to the refused quantity. Ticking an item again replaces it. The run
/// closes when this was the last thing missing.
///
/// # Errors
///
/// `Forbidden` without `runs.dispatch`; `Guard` (422) when the run is not back
/// at the depot, an entry is not an item of the run, or its quantities do not
/// add up.
pub async fn receive(
    state: &AppState,
    user: &AuthUser,
    run: &Run,
    entries: &[Entry],
) -> std::result::Result<CloseState, TransitionError> {
    if !user.has_permission(permissions::RUNS_DISPATCH) {
        return Err(TransitionError::Forbidden);
    }
    if run.status != RunStatus::ReturnedToDepot {
        return Err(unprocessable(english("wl_delivery.close.not_back")));
    }
    let items = items_of(&state.db, run)
        .await
        .map_err(TransitionError::Failed)?;
    for entry in entries {
        let item = items
            .iter()
            .find(|i| i.delivery_id == entry.delivery_id && i.line_id == entry.line_id)
            .ok_or_else(|| unprocessable(english("wl_delivery.close.not_an_item")))?;
        if entry.received_milli < 0 || entry.missing_milli < 0 {
            return Err(unprocessable(format!(
                "{}: quantities cannot be negative.",
                item.description
            )));
        }
        if entry.received_milli + entry.missing_milli != item.expected_milli {
            return Err(unprocessable(format!(
                "{} ({}): received plus missing must be {}.",
                item.description,
                item.number,
                crate::proof::milli_text(item.expected_milli)
            )));
        }
        if item.line_id == 0 && entry.received_milli != 0 && entry.missing_milli != 0 {
            return Err(unprocessable(format!(
                "{} ({}): an undelivered delivery is received or missing as a whole.",
                item.description, item.number
            )));
        }
    }
    let now = renox::db::now();
    for entry in entries {
        let existing = Receipt::query()
            .where_(Receipt::RUN_ID.eq(run.id))
            .where_(Receipt::DELIVERY_ID.eq(entry.delivery_id))
            .where_(Receipt::DELIVERY_LINE_ID.eq(entry.line_id))
            .first(&state.db)
            .await
            .map_err(TransitionError::Failed)?;
        let mut receipt = existing.unwrap_or_else(|| Receipt {
            run_id: run.id,
            delivery_id: entry.delivery_id,
            delivery_line_id: entry.line_id,
            ..Receipt::default()
        });
        receipt.received_milli = entry.received_milli;
        receipt.missing_milli = entry.missing_milli;
        receipt.recorded_by = Some(user.id);
        receipt.recorded_at = now;
        if receipt.id == 0 {
            receipt
                .insert(&state.db)
                .await
                .map_err(TransitionError::Failed)?;
        } else {
            receipt
                .save(&state.db)
                .await
                .map_err(TransitionError::Failed)?;
        }
    }
    try_close(state, run.id).await
}

/// Records the cash the driver handed over, as counted by `user` (a holder of
/// `runs.dispatch` other than the run's driver). The run closes when this was
/// the last thing missing.
///
/// # Errors
///
/// `Forbidden` without `runs.dispatch`; `Guard` (422) for the run's driver,
/// when the run is not back at the depot, or for a negative amount.
pub async fn count_cash(
    state: &AppState,
    user: &AuthUser,
    run: &Run,
    amount: i64,
) -> std::result::Result<CloseState, TransitionError> {
    if !user.has_permission(permissions::RUNS_DISPATCH) {
        return Err(TransitionError::Forbidden);
    }
    if user.id == run.driver_user_id {
        return Err(unprocessable(english(
            "wl_delivery.close.counted_by_another",
        )));
    }
    if run.status != RunStatus::ReturnedToDepot {
        return Err(unprocessable(english("wl_delivery.close.not_back")));
    }
    if amount < 0 {
        return Err(unprocessable("The cash counted cannot be negative."));
    }
    if custody_provided(state) {
        return Err(unprocessable(english(
            "wl_delivery.close.counted_in_payments",
        )));
    }
    let counted = Some(amount);
    let by = Some(user.id);
    let at = Some(renox::db::now());
    Run::query()
        .where_(Run::ID.eq(run.id))
        .update(
            &state.db,
            &[
                ("cash_counted_amount", &counted),
                ("cash_counted_by", &by),
                ("cash_counted_at", &at),
            ],
        )
        .await
        .map_err(TransitionError::Failed)?;
    try_close(state, run.id).await
}

/// Whether a run closed, and what it still waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseState {
    /// The run is closed (now, or it was).
    Closed,
    /// The run waits for these (as sentences).
    Waiting(Vec<String>),
}

/// The system's `close` of run `run_id`: runs when the cash is counted (or
/// none was collected) and every item is accounted for; otherwise says what
/// is missing and changes nothing.
///
/// # Errors
///
/// `NotFrom` is not an error: a run that is not back at the depot is
/// `Waiting`. Database errors.
pub async fn try_close(
    state: &AppState,
    run_id: i64,
) -> std::result::Result<CloseState, TransitionError> {
    let run = Run::find_or_404(&state.db, run_id)
        .await
        .map_err(TransitionError::Failed)?;
    match run.status {
        RunStatus::Closed => return Ok(CloseState::Closed),
        RunStatus::ReturnedToDepot => {}
        _ => {
            return Ok(CloseState::Waiting(vec![
                english("wl_delivery.close.not_back").to_owned(),
            ]));
        }
    }
    let items = items_of(&state.db, &run)
        .await
        .map_err(TransitionError::Failed)?;
    let expected = cash_expected(&state.db, &run)
        .await
        .map_err(TransitionError::Failed)?;
    let (to_count, counted) = cash_to_count(state, &run, expected)
        .await
        .map_err(TransitionError::Failed)?;
    let input = CloseInput {
        cash_expected: to_count,
        cash_counted: counted,
        open_items: i64::try_from(items.iter().filter(|i| !i.accounted).count())
            .unwrap_or(i64::MAX),
    };
    let mut waiting = Vec::new();
    if let Err(message) = cash_ok(Some(&input)) {
        waiting.push(message);
    }
    if let Err(message) = items_ok(Some(&input)) {
        waiting.push(message);
    }
    if !waiting.is_empty() {
        return Ok(CloseState::Waiting(waiting));
    }
    let _outcome = run
        .transition(CLOSE)
        .by_system()
        .input(&input)
        .run(state)
        .await?;
    announce_close(state, run_id, expected)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(CloseState::Closed)
}

/// After the commit of `close`: `DeliveryRefusedReceived` per delivery with
/// refused lines, a task for each item reported missing, and `RunClosed`.
async fn announce_close(state: &AppState, run_id: i64, cash_collected: i64) -> Result<()> {
    let run = Run::find_or_404(&state.db, run_id).await?;
    let items = items_of(&state.db, &run).await?;
    let deliveries = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .order_by(Delivery::RUN_SEQUENCE)
        .get(&state.db)
        .await?;
    let mut delivered = 0;
    let mut undelivered = 0;
    for delivery in &deliveries {
        match delivery.status {
            DeliveryStatus::Delivered | DeliveryStatus::UnderReview => delivered += 1,
            DeliveryStatus::Attempted => undelivered += 1,
            _ => {}
        }
        let refused: Vec<(DeliveryLine, &Item)> = delivery
            .lines(&state.db)
            .await?
            .into_iter()
            .filter_map(|line| {
                let item = items
                    .iter()
                    .find(|i| i.delivery_id == delivery.id && i.line_id == line.id)?;
                Some((line, item))
            })
            .collect();
        if refused.is_empty() {
            continue;
        }
        state
            .emit(DeliveryRefusedReceived {
                delivery: delivery.delivery_id(),
                source: delivery.source(),
                run: run.run_id(),
                lines: refused
                    .iter()
                    .map(|(line, item)| RefusedLineReceived {
                        line: line.source_line(),
                        description: line.description.clone(),
                        refused_milli: line.refused_milli,
                        received_milli: item.received_milli,
                        missing_milli: item.missing_milli,
                        refusal_reason: line.refusal_reason.clone(),
                    })
                    .collect(),
            })
            .await?;
    }
    let mut missing = 0;
    for item in items.iter().filter(|i| i.missing_milli > 0) {
        missing += 1;
        open_missing_task(state, &run, item).await?;
    }
    state
        .emit(RunClosed {
            run: run.run_id(),
            driver: run.driver_user_id,
            vehicle: run.vehicle_asset_id,
            delivered,
            undelivered,
            missing,
            cash_collected,
        })
        .await
}

/// The dispatcher's task for an item reported missing: a ticket of the
/// kernel's task category on the delivery, once per item. An undelivered
/// delivery is traced by `WRK.RUL.17` from then on.
async fn open_missing_task(state: &AppState, run: &Run, item: &Item) -> Result<()> {
    let subject = wl_kernel::delivery::DeliveryId(item.delivery_id).subject();
    let title = english("wl_delivery.close.missing_title")
        .replace("{number}", &item.number)
        .replace("{item}", &item.description);
    let description = english(if item.line_id == 0 {
        "wl_delivery.close.missing_delivery"
    } else {
        "wl_delivery.close.missing_line"
    })
    .replace("{run}", &run.number)
    .replace("{quantity}", &crate::proof::milli_text(item.missing_milli));
    let ticket = OpenTicket::new(subject, TASK_CATEGORY, &title)
        .description(&description)
        .source(run.subject())
        .source_key(&format!(
            "delivery.missing:{}:{}",
            item.delivery_id, item.line_id
        ));
    tickets::task_creator(state).open(state, ticket).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(expected: i64, counted: bool, open: i64) -> CloseInput {
        CloseInput {
            cash_expected: expected,
            cash_counted: counted,
            open_items: open,
        }
    }

    #[test]
    fn cash_must_be_counted_only_when_some_was_collected() {
        assert!(cash_ok(Some(&input(0, false, 0))).is_ok());
        assert!(cash_ok(Some(&input(10_000, true, 0))).is_ok());
        assert!(cash_ok(Some(&input(10_000, false, 0))).is_err());
        assert!(cash_ok(None).is_err());
    }

    #[test]
    fn every_item_must_be_accounted_for() {
        assert!(items_ok(Some(&input(0, false, 0))).is_ok());
        let refused = items_ok(Some(&input(0, false, 2))).unwrap_err();
        assert!(refused.contains('2'), "{refused}");
    }

    #[test]
    fn a_stop_without_an_outcome_keeps_the_run_out() {
        let ready = ReturnInput {
            driver_user_id: 1,
            open_stops: 0,
        };
        assert!(stops_ok(Some(&ready)).is_ok());
        let open = ReturnInput {
            open_stops: 3,
            ..ready
        };
        assert!(stops_ok(Some(&open)).unwrap_err().contains('3'));
    }
}
