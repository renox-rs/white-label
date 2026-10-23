//! Production batches in use (06 F&B §4.4 *Supplier or authority recalls an
//! ingredient lot*, §7.1 *Production batch*; BL-FNB-11).
//!
//! The chef plans a batch of a recipe, starts it picking the ingredient lots
//! (the earliest expiry is proposed; a lot on hold, expired or short is
//! refused) and finishes it with the yield and the expiry. The stock module
//! consumes the lots and makes the output lot ([`BatchStock`]); the batch
//! keeps the ingredient lots it used (the genealogy a recall follows).
//!
//! The lot decides the batch afterwards, through the stock module's events:
//!
//! | Event | The batch |
//! |---|---|
//! | `LotHeld` of its lot | `produced` becomes `on_hold`; the chefs are told |
//! | `LotReleased` | `on_hold` becomes `produced` |
//! | `LotDiscarded` | becomes `discarded`; the wastage entry links the lot's own wastage and posts no second movement |
//!
//! A hold on a location (a temperature hold) emits no `LotHeld`: the batch
//! stays `produced` and only that stock is blocked. A batch `in_production`
//! has no lot, so `finish` holds the output lot at once when an ingredient
//! lot was on hold at that moment, with the same reason.
//!
//! A chef discards a `produced` batch from its page: the batch posts the
//! `wastage` movement for what is left, once. Above the wastage limit
//! (`STK.ACL.02`) a second holder of `stock.approve` approves first. A batch
//! on hold is not discarded here: its hold line is decided *discard*, so the
//! hold's approvals apply.
//!
//! Decisions: one lot per ingredient line (an ingredient spread over several
//! lots is not picked yet); the output lot has the number of the batch; the
//! cost of the output is the cost of the ingredient lots used.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::today;
use wl_kernel::inventory::{
    ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, LotDiscarded, LotHeld, LotReleased, ProduceBatch,
    StockChoice, batch_stock, stock_service,
};

use crate::batch::{
    BATCH_SERIES, BatchRecipe, BatchRecipeLine, BatchStatus, BatchWastage, ProductionBatch,
    posted_by,
};
use crate::batch_machine::{
    BatchRun, DISCARD, DISCARD_APPROVED, DISCARD_SYSTEM, FINISH, HOLD, PickedLot, RELEASE, START,
};
use crate::catalogue::Outlet;
use crate::notices::tell_holders;
use crate::permissions;
use crate::placing::refused;
use crate::texts::english;

/// What the chef asks for when planning a batch.
#[derive(Debug, Clone, Default)]
pub struct PlanAsk {
    /// The recipe.
    pub recipe_id: i64,
    /// What to make, in thousandths; the recipe's standard yield when 0.
    pub planned_yield_milli: i64,
    /// The outlet whose kitchen makes it; the first active outlet when none.
    pub outlet_id: Option<i64>,
}

/// What the chef enters at `finish`.
#[derive(Debug, Clone, Default)]
pub struct FinishAsk {
    /// What came out, in thousandths.
    pub actual_yield_milli: i64,
    /// The last day the output can be used.
    pub expiry: Option<NaiveDate>,
}

/// What a discard did.
#[derive(Debug, Clone)]
pub enum Discarded {
    /// The batch is discarded and its wastage posted.
    Done(ProductionBatch),
    /// A second holder of `stock.approve` must approve first.
    Waiting(ApprovalRequest),
}

/// One ingredient of the start sheet: what is needed and which lots can be
/// taken.
#[derive(Debug, Clone)]
pub struct Proposal {
    /// The recipe line.
    pub line: BatchRecipeLine,
    /// What the batch needs of it, in thousandths.
    pub needed_milli: i64,
    /// The lots that can be taken, earliest expiry first.
    pub choice: Option<StockChoice>,
}

fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission_anywhere(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The lot subject of `id`.
fn lot_subject(id: i64) -> Subject {
    Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, id)
}

/// Plans a batch of the recipe.
///
/// # Errors
///
/// `Forbidden` without `production.manage`; 422 for an unknown recipe or an
/// outlet without a stock location (the first active outlet that has one is
/// taken when none is named).
pub async fn plan(state: &AppState, user: &AuthUser, ask: &PlanAsk) -> Result<ProductionBatch> {
    require(user, permissions::PRODUCTION_MANAGE)?;
    let Some(recipe) = BatchRecipe::find(&state.db, ask.recipe_id).await? else {
        return Err(refused(english("fnb.batch.problem.no_recipe", &[])));
    };
    let planned = if ask.planned_yield_milli > 0 {
        ask.planned_yield_milli
    } else {
        recipe.yield_milli
    };
    let outlet = match ask.outlet_id {
        Some(id) => Outlet::find(&state.db, id).await?,
        None => Outlet::query()
            .where_(Outlet::ACTIVE.eq(true))
            .order_by(Outlet::ID)
            .get(&state.db)
            .await?
            .into_iter()
            .find(|outlet| outlet.location_id.is_some()),
    };
    let Some(outlet) = outlet else {
        return Err(refused(english("fnb.batch.problem.no_location", &[])));
    };
    let Some(location_id) = outlet.location_id else {
        return Err(refused(english("fnb.batch.problem.no_location", &[])));
    };
    let number = BATCH_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let mut batch = ProductionBatch {
        number,
        recipe_id: recipe.id,
        outlet_id: Some(outlet.id),
        location_id,
        planned_yield_milli: planned,
        planned_by: Some(user.id),
        ..ProductionBatch::default()
    };
    let mut tx = state.db.begin().await?;
    let history = english("fnb.batch.history.planned", &[("recipe", &recipe.name)]);
    batch
        .insert_in_state(&mut tx, BatchStatus::Planned, &history)
        .await?;
    tx.commit().await?;
    ProductionBatch::find_or_404(&state.db, batch.id).await
}

/// Cancels a planned batch: it is deleted.
///
/// # Errors
///
/// `Forbidden` without `production.manage`; 422 unless the batch is planned.
pub async fn cancel(state: &AppState, user: &AuthUser, id: i64) -> Result {
    require(user, permissions::PRODUCTION_MANAGE)?;
    let batch = ProductionBatch::find_or_404(&state.db, id).await?;
    if batch.status != BatchStatus::Planned {
        return Err(refused(english("fnb.batch.problem.not_planned", &[])));
    }
    ProductionBatch::query()
        .where_(ProductionBatch::ID.eq(id))
        .delete(&state.db)
        .await?;
    Ok(())
}

/// The ingredients of a batch with what it needs and the lots that can be
/// taken: the earliest expiry first, never a lot on hold or expired.
///
/// # Errors
///
/// Database errors and the stock module's.
pub async fn proposals(state: &AppState, batch: &ProductionBatch) -> Result<Vec<Proposal>> {
    let Some(recipe) = BatchRecipe::find(&state.db, batch.recipe_id).await? else {
        return Ok(Vec::new());
    };
    let location = Subject::new(LOCATION_SUBJECT_TYPE, batch.location_id);
    let mut found = Vec::new();
    for line in recipe.lines(&state.db).await? {
        let choice = stock_service(state)
            .choices(state, &line.item_code, &location)
            .await?;
        found.push(Proposal {
            needed_milli: line.needed(&recipe, batch.planned_yield_milli),
            line,
            choice,
        });
    }
    Ok(found)
}

/// What is wrong with picking `lot_id` for `line`, in words; none when the lot
/// is fine.
async fn problem_with(
    state: &AppState,
    line: &BatchRecipeLine,
    needed_milli: i64,
    lot_id: i64,
) -> Result<Option<String>> {
    let Some(info) = batch_stock(state)
        .lot_info(state, &lot_subject(lot_id))
        .await?
    else {
        return Ok(Some(english(
            "fnb.batch.problem.lot_unknown",
            &[("item", &line.item_code)],
        )));
    };
    if info.item.subject_id != line.item_id {
        return Ok(Some(english(
            "fnb.batch.problem.lot_wrong",
            &[("lot", &info.number), ("item", &line.item_code)],
        )));
    }
    if info.status == "on_hold" {
        return Ok(Some(english(
            "fnb.batch.problem.lot_on_hold",
            &[("lot", &info.number)],
        )));
    }
    if info.status != "released" {
        return Ok(Some(english(
            "fnb.batch.problem.lot_not_usable",
            &[("lot", &info.number), ("status", &info.status)],
        )));
    }
    if info.expiry_date.is_some_and(|day| day < today()) {
        return Ok(Some(english(
            "fnb.batch.problem.lot_expired",
            &[("lot", &info.number)],
        )));
    }
    if info.left_milli < needed_milli {
        return Ok(Some(english(
            "fnb.batch.problem.lot_short",
            &[
                ("lot", &info.number),
                ("item", &line.item_code),
                ("needed", &crate::money::milli_text(needed_milli)),
                ("left", &crate::money::milli_text(info.left_milli)),
            ],
        )));
    }
    Ok(None)
}

/// Starts the batch with the lots `chosen` by recipe line id (the proposed lot
/// of a line that has none chosen). The lots are recorded for the recall
/// trace.
///
/// # Errors
///
/// `Forbidden` without `production.manage`; 422 when a lot is on hold,
/// expired, of another item or short, or no lot can be proposed; the
/// transition's errors.
pub async fn start(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    chosen: &BTreeMap<i64, i64>,
) -> Result<ProductionBatch> {
    require(user, permissions::PRODUCTION_MANAGE)?;
    let batch = ProductionBatch::find_or_404(&state.db, id).await?;
    let mut run = BatchRun {
        by: Some(user.id),
        ..BatchRun::default()
    };
    let mut problems = Vec::new();
    for proposal in proposals(state, &batch).await? {
        let line = &proposal.line;
        let lot_id = chosen.get(&line.id).copied().or_else(|| {
            proposal
                .choice
                .as_ref()
                .and_then(|choice| choice.lots.first())
                .map(|lot| lot.lot.subject_id)
        });
        let Some(lot_id) = lot_id else {
            problems.push(english(
                "fnb.batch.problem.lot_missing",
                &[("item", &line.item_code)],
            ));
            continue;
        };
        if let Some(problem) = problem_with(state, line, proposal.needed_milli, lot_id).await? {
            problems.push(problem);
            continue;
        }
        run.picked.push(PickedLot {
            item_id: line.item_id,
            lot_id,
            quantity_milli: proposal.needed_milli,
        });
    }
    if !problems.is_empty() {
        run.problem = Some(problems.join(" "));
    }
    let history = english("fnb.batch.history.started", &[("number", &batch.number)]);
    let _outcome = batch
        .transition(START)
        .by(user)
        .reason(history)
        .input(&run)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    ProductionBatch::find_or_404(&state.db, id).await
}

/// Finishes the batch: the ingredient lots are consumed, the output lot is made
/// with the expiry the chef entered, and when an ingredient lot is on hold at
/// that moment the output lot is held at once with the same reason (the batch
/// then goes `on_hold` by the `LotHeld` event).
///
/// # Errors
///
/// `Forbidden` without `production.manage`; 422 without a yield or an expiry,
/// for an expiry later than the earliest expiry of the ingredient lots, or
/// when the stock module refuses; the transition's errors.
pub async fn finish(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    ask: &FinishAsk,
) -> Result<ProductionBatch> {
    require(user, permissions::PRODUCTION_MANAGE)?;
    let batch = ProductionBatch::find_or_404(&state.db, id).await?;
    if batch.status != BatchStatus::InProduction {
        return Err(refused(english("fnb.batch.problem.not_in_production", &[])));
    }
    if ask.actual_yield_milli <= 0 {
        return Err(refused(english("fnb.batch.problem.yield", &[])));
    }
    let Some(expiry) = ask.expiry else {
        return Err(refused(english("fnb.batch.problem.expiry_needed", &[])));
    };
    if expiry < today() {
        return Err(refused(english(
            "fnb.batch.problem.expiry_past",
            &[("expiry", &expiry)],
        )));
    }
    let inputs = batch.inputs(&state.db).await?;
    let mut earliest: Option<NaiveDate> = None;
    for input in &inputs {
        if let Some(info) = batch_stock(state).lot_info(state, &input.lot()).await?
            && let Some(day) = info.expiry_date
        {
            earliest = Some(earliest.map_or(day, |found| found.min(day)));
        }
    }
    if let Some(earliest) = earliest
        && expiry > earliest
    {
        return Err(refused(english(
            "fnb.batch.problem.expiry_later",
            &[("expiry", &expiry), ("earliest", &earliest)],
        )));
    }
    let Some(recipe) = BatchRecipe::find(&state.db, batch.recipe_id).await? else {
        return Err(refused(english("fnb.batch.problem.no_recipe", &[])));
    };
    let produced = batch_stock(state)
        .produce(
            state,
            ProduceBatch {
                item: Subject::new(ITEM_SUBJECT_TYPE, recipe.output_item_id),
                location: Subject::new(LOCATION_SUBJECT_TYPE, batch.location_id),
                quantity_milli: ask.actual_yield_milli,
                lot_number: batch.number.clone(),
                expiry,
                source: batch.subject(),
                inputs: inputs
                    .iter()
                    .map(|input| wl_kernel::inventory::BatchInput {
                        lot: input.lot(),
                        quantity_milli: input.quantity_milli,
                    })
                    .collect(),
                by: Some(user.id),
            },
        )
        .await?;
    let Some(produced) = produced else {
        return Err(refused(english("fnb.batch.problem.stock_off", &[])));
    };
    let run = BatchRun {
        actual_yield_milli: ask.actual_yield_milli,
        expiry: Some(expiry),
        earliest_input_expiry: earliest,
        lot_id: Some(produced.lot.subject_id),
        by: Some(user.id),
        ..BatchRun::default()
    };
    let history = english("fnb.batch.history.finished", &[("number", &batch.number)]);
    let _outcome = batch
        .transition(FINISH)
        .by(user)
        .reason(history)
        .input(&run)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    // An ingredient lot was on hold: the output lot is held with the same
    // reason, and `LotHeld` runs the batch's `hold`.
    if let Some(reason) = &produced.held_reason {
        batch_stock(state)
            .hold_lot(state, &produced.lot, reason, &batch.subject())
            .await?;
    }
    ProductionBatch::find_or_404(&state.db, id).await
}

/// Discards a `produced` batch: its wastage is posted once, and above the
/// wastage limit a second holder of `stock.approve` approves first.
///
/// # Errors
///
/// `Forbidden` without `production.manage`; 422 without a reason or unless the
/// batch is `produced` (a batch on hold is discarded through its hold line).
pub async fn discard(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<Discarded> {
    require(user, permissions::PRODUCTION_MANAGE)?;
    let batch = ProductionBatch::find_or_404(&state.db, id).await?;
    let reason = reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refused(english("fnb.batch.problem.discard_reason", &[])));
    }
    if batch.status == BatchStatus::OnHold {
        return Err(refused(english("fnb.batch.problem.discard_on_hold", &[])));
    }
    if batch.status != BatchStatus::Produced {
        return Err(refused(english("fnb.batch.problem.not_produced", &[])));
    }
    let amount = match batch.lot() {
        Some(lot) => batch_stock(state)
            .lot_info(state, &lot)
            .await?
            .map_or(0, |info| info.value),
        None => 0,
    };
    let run = BatchRun {
        amount,
        by: Some(user.id),
        ..BatchRun::default()
    };
    let outcome = batch
        .transition(DISCARD)
        .by(user)
        .reason(reason)
        .input(&run)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    approvals::announce(state, &outcome).await?;
    match outcome {
        Outcome::Done(_) => Ok(Discarded::Done(
            ProductionBatch::find_or_404(&state.db, id).await?,
        )),
        Outcome::Pending(_) => {
            let request = ApprovalRequest::pending_about(&state.db, &batch.subject())
                .await?
                .pop()
                .ok_or(Error::NotFound)?;
            Ok(Discarded::Waiting(request))
        }
    }
}

/// Tells the chefs about `batch`.
async fn tell_chefs(state: &AppState, batch: &ProductionBatch, title: &str, body: &str) -> Result {
    tell_holders(
        state,
        batch.subject(),
        &batch.number,
        permissions::PRODUCTION_MANAGE,
        None,
        title,
        body,
        &format!("/batches/{}", batch.id),
    )
    .await
    .map(|_| ())
}

/// `LotHeld`: the batch whose lot is held goes `on_hold` and the chefs are
/// told. A hold on a location emits no such event, so a temperature hold
/// leaves the batch `produced`.
///
/// # Errors
///
/// Database errors, the machine's and the notifier's.
pub(crate) async fn on_lot_held(state: &AppState, event: LotHeld) -> Result {
    let Some(batch) = ProductionBatch::of_lot(&state.db, event.lot.subject_id).await? else {
        return Ok(());
    };
    if batch.status != BatchStatus::Produced {
        return Ok(());
    }
    let run = BatchRun {
        hold_reason: Some(event.reason.clone()),
        ..BatchRun::default()
    };
    let history = english("fnb.batch.history.held", &[("reason", &event.reason)]);
    let _outcome = batch
        .transition(HOLD)
        .by_system()
        .reason(history)
        .input(&run)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    tell_chefs(
        state,
        &batch,
        &english("fnb.batch.hold_title", &[("number", &batch.number)]),
        &english(
            "fnb.batch.hold_body",
            &[("number", &batch.number), ("reason", &event.reason)],
        ),
    )
    .await
}

/// `LotReleased`: the batch is `produced` again.
///
/// # Errors
///
/// Database errors and the machine's.
pub(crate) async fn on_lot_released(state: &AppState, event: LotReleased) -> Result {
    let Some(batch) = ProductionBatch::of_lot(&state.db, event.lot.subject_id).await? else {
        return Ok(());
    };
    if batch.status != BatchStatus::OnHold {
        return Ok(());
    }
    let history = english("fnb.batch.history.released", &[("number", &batch.number)]);
    let _outcome = batch
        .transition(RELEASE)
        .by_system()
        .reason(history)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    tell_chefs(
        state,
        &batch,
        &english("fnb.batch.release_title", &[("number", &batch.number)]),
        &english("fnb.batch.release_body", &[("number", &batch.number)]),
    )
    .await
}

/// `LotDiscarded`: the batch is `discarded`. The lot's own discard is the
/// posting of the wastage; the batch's entry links it and posts no second
/// movement.
///
/// # Errors
///
/// Database errors, the machine's and the notifier's.
pub(crate) async fn on_lot_discarded(state: &AppState, event: LotDiscarded) -> Result {
    let Some(batch) = ProductionBatch::of_lot(&state.db, event.lot.subject_id).await? else {
        return Ok(());
    };
    if !matches!(batch.status, BatchStatus::OnHold | BatchStatus::Produced) {
        return Ok(());
    }
    let _outcome = batch
        .transition(DISCARD_SYSTEM)
        .by_system()
        .reason(event.reason.clone())
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    if BatchWastage::of_batch(&state.db, batch.id).await?.is_none() {
        let wasted = batch_stock(state).wasted_of_lot(state, &event.lot).await?;
        let mut entry = BatchWastage {
            batch_id: batch.id,
            lot_id: event.lot.subject_id,
            quantity_milli: event.quantity_milli.max(wasted.quantity_milli),
            value: wasted.value,
            reason: event.reason.clone(),
            posted_by: posted_by::LOT.to_owned(),
            ..BatchWastage::default()
        };
        entry.insert(&state.db).await?;
    }
    tell_chefs(
        state,
        &batch,
        &english("fnb.batch.discard_title", &[("number", &batch.number)]),
        &english(
            "fnb.batch.discard_body",
            &[("number", &batch.number), ("reason", &event.reason)],
        ),
    )
    .await
}

/// After a committed discard by the chef (or the second person's yes): the
/// batch posts the `wastage` movement for what is left, once, and keeps the
/// entry.
///
/// # Errors
///
/// Database errors and the stock module's.
pub(crate) async fn after_batch(state: &AppState, event: &Transitioned<ProductionBatch>) -> Result {
    if event.transition != DISCARD && event.transition != DISCARD_APPROVED {
        return Ok(());
    }
    let batch = ProductionBatch::find_or_404(&state.db, event.key).await?;
    if batch.status != BatchStatus::Discarded
        || BatchWastage::of_batch(&state.db, batch.id).await?.is_some()
    {
        return Ok(());
    }
    let Some(lot) = batch.lot() else {
        return Ok(());
    };
    let reason = batch
        .discard_reason
        .clone()
        .unwrap_or_else(|| english("fnb.batch.discard_default", &[]));
    let wasted = batch_stock(state)
        .waste_lot(state, &lot, &reason, event.actor_id)
        .await?;
    let mut entry = BatchWastage {
        batch_id: batch.id,
        lot_id: lot.subject_id,
        quantity_milli: wasted.quantity_milli,
        value: wasted.value,
        reason,
        posted_by: posted_by::BATCH.to_owned(),
        recorded_by: event.actor_id,
        ..BatchWastage::default()
    };
    entry.insert(&state.db).await?;
    Ok(())
}

/// The batches with `status`, newest first.
///
/// # Errors
///
/// Database errors.
pub async fn with_status(db: &Db, status: BatchStatus) -> Result<Vec<ProductionBatch>> {
    ProductionBatch::query()
        .where_(ProductionBatch::STATUS.eq(status))
        .order_by_desc(ProductionBatch::ID)
        .get(db)
        .await
}

/// Registers the listeners of the lot events and of a committed discard.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: LotHeld, state: AppState| async move { on_lot_held(&state, event).await });
    app.listen(|event: LotReleased, state: AppState| async move {
        on_lot_released(&state, event).await
    });
    app.listen(|event: LotDiscarded, state: AppState| async move {
        on_lot_discarded(&state, event).await
    });
    app.listen(
        |event: Transitioned<ProductionBatch>, state: AppState| async move {
            after_batch(&state, &event).await
        },
    );
}
