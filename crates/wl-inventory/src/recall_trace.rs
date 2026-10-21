//! The forward trace of a recall (wl-inventory §4 *Recall* `trace`, issue
//! #234): who got the recalled goods. `trace` runs right after `open` (as the
//! system) and again whenever a holder of `recalls.manage` asks; it follows
//! the immutable movements of every lot in scope and never changes one.
//!
//! | Movement | What the trace makes |
//! |---|---|
//! | `transfer_out` / `transfer_in` | nothing: the lot at the destination is held already (a hold on whole lots follows the lot) |
//! | `consume` / `issue` with a `produce` from the same source | the output lots join the scope and the recall's hold, and are traced in turn (each lot once, so a cycle ends) |
//! | `issue` / `consume` to a cost object, not theoretical | a *confirmed* line for the subject, with its party |
//! | `dispatch` | a *dispatch* line for the recipient, with the delivery status |
//! | a pending issue posted with a lot *assigned on arrival* | an *uncertain* line for its subject, whichever lot it was posted against |
//! | theoretical `consume` of the item at a location while a lot in scope was on hand there | a *possible* line, whichever lot the posting used |
//! | a sale whose party the vertical does not know (walk-in) | one line per lot, outlet and day with the count of sales |
//!
//! The party behind a subject comes from the vertical's
//! [`TracePartyResolver`](wl_kernel::inventory::TracePartyResolver); without
//! one every line has the party *unknown* for manual handling.
//!
//! A lot is on hand at a location from the movement that took its balance
//! there above zero until the one that took it back to zero, or until the
//! recall held it: the window of a lot at a location. A theoretical
//! consumption inside a window is *possible*; one outside it is not listed.
//! A pending issue is *uncertain* when the time from its recording to its
//! posting touches a window of a lot in scope at its location.
//!
//! Decisions:
//! - a re-run is idempotent: each line has a key (kind, subject, lot), only
//!   keys that are new are added and existing lines keep their notice status;
//!   a walk-in count is updated when more sales were found;
//! - a consumption whose source also produced lots is the batch or work order
//!   itself: it gives the output lots and no line;
//! - an output lot joins the recall's open hold as new lines; when that hold
//!   is closed already (a recall of lots all used up) the output lots that
//!   have stock are held by a new hold with reason `recall`, which `close`
//!   waits for as well; an output lot with no stock stays in the scope;
//! - a walk-in count line has no party, so like a line with the party
//!   *unknown* it is `pending` until the owner waives it with a reason (the
//!   decision on a public notice);
//! - the trace of the first run happens after the recall is committed: when
//!   it fails the recall stays open and held and can be traced again.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::documents::dubai_date;
use wl_kernel::inventory::{
    LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE, LotTrace, RECALL_SUBJECT_TYPE, RecallTraced,
    TraceFuture, TraceParty, TracePartyResolver, TracedHolder, TracedPlace, trace_party_resolver,
};
use wl_kernel::{PartyId, Subject};

use crate::balance;
use crate::hold_service::{NewHold, NewScope, announce_added, announce_placed, place_in};
use crate::lot::{self, Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::pending_issue::{PendingIssue, PendingStatus};
use crate::recall::{Recall, RecallLot, RecallStatus, TRACE, TRACE_BY_SYSTEM, lots_in_scope};
use crate::recall_line::{LineKind, NoticeStatus, RecallLine, lines_of};
use crate::stock_hold::{HoldStatus, StockHold, new_line};

/// A line the trace found, before it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Planned {
    key: String,
    kind: LineKind,
    subject: Option<Subject>,
    party: Option<i64>,
    lot_id: Option<i64>,
    location_id: Option<i64>,
    quantity_milli: i64,
    sales: i64,
    on: Option<NaiveDate>,
    delivery: Option<String>,
}

/// What the trace found.
#[derive(Debug, Default)]
struct Plan {
    /// The lots that are not in the recall's scope yet.
    new_lots: Vec<i64>,
    /// The lines.
    lines: Vec<Planned>,
}

/// The holds of recall `recall_id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn holds_of<'c>(
    db: impl renox::db::Executor<'c>,
    recall_id: i64,
) -> Result<Vec<StockHold>> {
    StockHold::query()
        .where_(StockHold::REFERENCE_TYPE.eq(Some(RECALL_SUBJECT_TYPE.to_owned())))
        .where_(StockHold::REFERENCE_ID.eq(Some(recall_id)))
        .order_by(StockHold::ID)
        .get(db)
        .await
}

/// The subject a movement charged: its cost object, else its source unless
/// that is an inventory record (a location, a count), which names nobody.
fn subject_of(moved: &StockMovement) -> Option<Subject> {
    match (&moved.cost_object_type, moved.cost_object_id) {
        (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
        _ => {
            let source = moved.source();
            (source.module() != "inventory").then_some(source)
        }
    }
}

/// Whether `source` also produced lots: a batch or a work order.
async fn is_production(db: &Db, source: &Subject) -> Result<bool> {
    StockMovement::query()
        .where_(StockMovement::SOURCE_TYPE.eq(source.subject_type.as_str()))
        .where_(StockMovement::SOURCE_ID.eq(source.subject_id))
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Produce))
        .exists(db)
        .await
}

/// The lots made from lot `lot_id`: those a `produce` of a source that
/// consumed or issued it made.
async fn outputs_of(db: &Db, lot_id: i64) -> Result<Vec<i64>> {
    let moves = StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot_id))
        .order_by(StockMovement::ID)
        .get(db)
        .await?;
    let mut sources = BTreeSet::new();
    for moved in &moves {
        if matches!(
            moved.movement_type,
            MovementType::Consume | MovementType::Issue
        ) {
            sources.insert((moved.source_type.clone(), moved.source_id));
        }
    }
    let mut outputs = BTreeSet::new();
    for (kind, id) in sources {
        let produced = StockMovement::query()
            .where_(StockMovement::SOURCE_TYPE.eq(kind.as_str()))
            .where_(StockMovement::SOURCE_ID.eq(id))
            .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Produce))
            .order_by(StockMovement::ID)
            .get(db)
            .await?;
        for moved in produced {
            if let Some(out) = moved.lot_id
                && out != lot_id
            {
                outputs.insert(out);
            }
        }
    }
    Ok(outputs.into_iter().collect())
}

/// The windows in which lot `lot_id` was on hand at `location_id`: from the
/// movement that took its balance there above zero to the one that took it
/// back to zero, or to `held_at` when it was held first or still is.
async fn windows(
    db: &Db,
    lot_id: i64,
    location_id: i64,
    held_at: DateTime,
) -> Result<Vec<(DateTime, DateTime)>> {
    let mut moves = StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot_id))
        .get(db)
        .await?;
    moves.sort_by_key(|moved| (moved.moved_at, moved.id));
    let mut stock = 0_i64;
    let mut start: Option<DateTime> = None;
    let mut found = Vec::new();
    for moved in moves {
        let mut delta = 0;
        if moved.to_location_id == Some(location_id) {
            delta += moved.quantity_milli;
        }
        if moved.from_location_id == Some(location_id) {
            delta -= moved.quantity_milli;
        }
        if delta == 0 {
            continue;
        }
        let before = stock;
        stock += delta;
        if before <= 0 && stock > 0 {
            start = Some(moved.moved_at);
        } else if before > 0
            && stock <= 0
            && let Some(from) = start.take()
        {
            found.push((from, moved.moved_at));
        }
    }
    if let Some(from) = start {
        found.push((from, held_at));
    }
    Ok(found
        .into_iter()
        .filter(|(from, _)| *from <= held_at)
        .map(|(from, to)| (from, to.min(held_at)))
        .collect())
}

/// The locations lot `lot_id` ever arrived at.
async fn places_of(db: &Db, lot_id: i64) -> Result<BTreeSet<i64>> {
    Ok(StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot_id))
        .get(db)
        .await?
        .iter()
        .filter_map(|moved| moved.to_location_id)
        .collect())
}

/// Collects the lines, asking the resolver for the party of each subject.
struct Collector<'a> {
    state: &'a AppState,
    resolver: Arc<dyn TracePartyResolver>,
    planned: BTreeMap<String, Planned>,
    /// Walk-in sales: (lot, outlet, day) to the sales and the quantity.
    walk_in: BTreeMap<(i64, i64, NaiveDate), (BTreeSet<Subject>, i64)>,
}

impl<'a> Collector<'a> {
    fn new(state: &'a AppState) -> Self {
        Collector {
            state,
            resolver: trace_party_resolver(state),
            planned: BTreeMap::new(),
            walk_in: BTreeMap::new(),
        }
    }

    /// Lists `subject` as of `kind` for `quantity_milli` of lot `lot_id`
    /// taken from `location_id` at `at`.
    async fn add(
        &mut self,
        kind: LineKind,
        subject: Subject,
        lot_id: i64,
        location_id: Option<i64>,
        quantity_milli: i64,
        at: DateTime,
    ) -> Result {
        let on = dubai_date(at);
        let party = self.resolver.party(self.state, &subject).await?;
        if party == TraceParty::WalkIn {
            let entry = self
                .walk_in
                .entry((lot_id, location_id.unwrap_or(0), on))
                .or_default();
            entry.0.insert(subject);
            entry.1 += quantity_milli;
            return Ok(());
        }
        let key = match kind {
            LineKind::Uncertain => format!("u:{subject}"),
            LineKind::Possible => format!("p:{subject}"),
            LineKind::Dispatch => format!("d:{subject}:{lot_id}"),
            _ => format!("c:{subject}:{lot_id}"),
        };
        if let Some(found) = self.planned.get_mut(&key) {
            found.quantity_milli += quantity_milli;
            found.on = found.on.min(Some(on));
            return Ok(());
        }
        let delivery = if kind == LineKind::Dispatch {
            self.resolver.delivery_status(self.state, &subject).await?
        } else {
            None
        };
        let party = match party {
            TraceParty::Party(PartyId(id)) => Some(id),
            _ => None,
        };
        self.planned.insert(
            key.clone(),
            Planned {
                key,
                kind,
                subject: Some(subject),
                party,
                lot_id: Some(lot_id),
                location_id,
                quantity_milli,
                sales: 1,
                on: Some(on),
                delivery,
            },
        );
        Ok(())
    }

    /// The lines, the walk-in counts last.
    fn finish(mut self) -> Vec<Planned> {
        for ((lot_id, location_id, on), (sales, quantity_milli)) in self.walk_in {
            let key = format!("w:{lot_id}:{location_id}:{on}");
            self.planned.insert(
                key.clone(),
                Planned {
                    key,
                    kind: LineKind::WalkIn,
                    subject: None,
                    party: None,
                    lot_id: Some(lot_id),
                    location_id: (location_id != 0).then_some(location_id),
                    quantity_milli,
                    sales: sales.len() as i64,
                    on: Some(on),
                    delivery: None,
                },
            );
        }
        self.planned.into_values().collect()
    }
}

/// Follows the movements of every lot in scope of `recall` (read only).
async fn plan(state: &AppState, recall: &Recall) -> Result<Plan> {
    let db = &state.db;
    let mut lots = lots_in_scope(db, recall.id).await?;
    let in_scope: BTreeSet<i64> = lots.iter().copied().collect();
    let mut seen = in_scope.clone();
    let mut new_lots = Vec::new();
    let mut index = 0;
    while index < lots.len() {
        let lot_id = lots[index];
        index += 1;
        for out in outputs_of(db, lot_id).await? {
            if seen.insert(out) {
                lots.push(out);
                new_lots.push(out);
            }
        }
    }
    let opened = recall.opened_at.unwrap_or_else(renox::db::now);
    let traced = renox::db::now();
    let held_at = |lot_id: i64| {
        if in_scope.contains(&lot_id) {
            opened
        } else {
            traced
        }
    };
    // The pending issues posted with a lot chosen when stock arrived.
    let arrivals: Vec<PendingIssue> = PendingIssue::query()
        .where_(PendingIssue::STATUS.eq(PendingStatus::Posted))
        .where_(PendingIssue::ASSIGNED_ON_ARRIVAL.eq(true))
        .order_by(PendingIssue::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(|pending| !pending.theoretical)
        .collect();
    let mut collector = Collector::new(state);
    let mut items_of: BTreeMap<i64, i64> = BTreeMap::new();
    for lot_id in &lots {
        let lot = Lot::find_or_404(db, *lot_id).await?;
        items_of.insert(*lot_id, lot.item_id);
        let mut moves = StockMovement::query()
            .where_(StockMovement::LOT_ID.eq(*lot_id))
            .order_by(StockMovement::ID)
            .get(db)
            .await?;
        moves.sort_by_key(|moved| (moved.moved_at, moved.id));
        for moved in moves {
            let kind = match moved.movement_type {
                MovementType::Consume | MovementType::Issue if !moved.theoretical => {
                    LineKind::Confirmed
                }
                MovementType::Dispatch => LineKind::Dispatch,
                _ => continue,
            };
            let Some(subject) = subject_of(&moved) else {
                continue;
            };
            if is_production(db, &moved.source()).await? {
                continue;
            }
            // The posting of a pending issue is listed as uncertain below.
            let from_arrival = arrivals.iter().any(|pending| {
                pending.lot_id == moved.lot_id
                    && pending.source() == moved.source()
                    && pending.closed_at == Some(moved.moved_at)
                    && pending.quantity_milli == moved.quantity_milli
            });
            if from_arrival {
                continue;
            }
            collector
                .add(
                    kind,
                    subject,
                    *lot_id,
                    moved.from_location_id,
                    moved.quantity_milli,
                    moved.moved_at,
                )
                .await?;
        }
    }
    // Theoretical consumption while a lot in scope was on hand, and pending
    // issues that may have used it.
    for lot_id in &lots {
        let item_id = items_of[lot_id];
        for location_id in places_of(db, *lot_id).await? {
            let spans = windows(db, *lot_id, location_id, held_at(*lot_id)).await?;
            if spans.is_empty() {
                continue;
            }
            let theoretical = StockMovement::query()
                .where_(StockMovement::ITEM_ID.eq(item_id))
                .where_(StockMovement::FROM_LOCATION_ID.eq(location_id))
                .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Consume))
                .order_by(StockMovement::ID)
                .get(db)
                .await?;
            for moved in theoretical.into_iter().filter(|moved| moved.theoretical) {
                if !spans
                    .iter()
                    .any(|(from, to)| *from <= moved.moved_at && moved.moved_at <= *to)
                {
                    continue;
                }
                if let Some(subject) = subject_of(&moved) {
                    collector
                        .add(
                            LineKind::Possible,
                            subject,
                            *lot_id,
                            moved.from_location_id,
                            moved.quantity_milli,
                            moved.moved_at,
                        )
                        .await?;
                }
            }
            for pending in arrivals
                .iter()
                .filter(|p| p.item_id == item_id && p.location_id == location_id)
            {
                let posted = pending.closed_at.unwrap_or(pending.recorded_at);
                if !spans
                    .iter()
                    .any(|(from, to)| *from <= posted && pending.recorded_at <= *to)
                {
                    continue;
                }
                let subject = pending.cost_object().or_else(|| {
                    let source = pending.source();
                    (source.module() != "inventory").then_some(source)
                });
                if let Some(subject) = subject {
                    collector
                        .add(
                            LineKind::Uncertain,
                            subject,
                            *lot_id,
                            Some(location_id),
                            pending.quantity_milli,
                            pending.recorded_at,
                        )
                        .await?;
                }
            }
        }
    }
    Ok(Plan {
        new_lots,
        lines: collector.finish(),
    })
}

/// Traces recall `recall_id` (§4 `trace`): builds the affected lines, adds
/// the lots made from the recalled ones to the scope and the hold, and moves
/// the recall to `traced`. `by` is the holder of `recalls.manage` who runs it
/// again; `None` is the system right after `open`.
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` for a recall that is
/// `closed`; a database error.
pub async fn trace(
    state: &AppState,
    by: Option<&AuthUser>,
    recall_id: i64,
) -> std::result::Result<Recall, TransitionError> {
    let recall = Recall::find_or_404(&state.db, recall_id).await?;
    let plan = plan(state, &recall).await?;
    let known: BTreeMap<String, RecallLine> = lines_of(&state.db, recall.id)
        .await?
        .into_iter()
        .map(|line| (line.trace_key.clone(), line))
        .collect();
    let mut tx = state.db.begin().await?;
    for lot_id in &plan.new_lots {
        let mut row = RecallLot {
            recall_id: recall.id,
            lot_id: *lot_id,
            ..RecallLot::default()
        };
        row.insert(&mut tx).await?;
    }
    // Hold the output lots: new lines on the recall's open hold, else a new
    // hold when something of them is in stock.
    let mut lot_outcomes = Vec::new();
    let mut placed = None;
    let mut added_lines = 0;
    let mut open_hold = None;
    if let Some(hold_id) = recall.stock_hold_id {
        let hold = StockHold::find_or_404(&mut tx, hold_id).await?;
        if hold.status == HoldStatus::Open {
            open_hold = Some(hold);
        }
    }
    match &open_hold {
        Some(hold) => {
            for lot_id in &plan.new_lots {
                let lot = Lot::find_or_404(&mut tx, *lot_id).await?;
                for (location_id, quantity) in balance::lot_locations(&mut tx, lot.id).await? {
                    new_line(
                        &mut tx,
                        hold.id,
                        lot.item_id,
                        location_id,
                        Some(lot.id),
                        quantity,
                    )
                    .await?;
                    added_lines += 1;
                }
                if matches!(lot.status, LotStatus::Quarantine | LotStatus::Released) {
                    lot_outcomes.push(
                        lot.transition(lot::HOLD)
                            .by_system()
                            .reason("recall")
                            .run_in(&mut tx)
                            .await?,
                    );
                }
            }
        }
        None => {
            let mut with_stock = Vec::new();
            for lot_id in &plan.new_lots {
                if balance::lot_left(&mut tx, *lot_id).await? > 0 {
                    with_stock.push(*lot_id);
                }
            }
            if !with_stock.is_empty() {
                placed = Some(
                    place_in(
                        &mut tx,
                        &NewHold {
                            reason: "recall".to_owned(),
                            reference: recall.subject(),
                            scope: NewScope::Lots(with_stock),
                            by: None,
                        },
                        false,
                    )
                    .await?,
                );
            }
        }
    }
    for found in &plan.lines {
        match known.get(&found.key) {
            None => {
                let mut row = RecallLine {
                    recall_id: recall.id,
                    trace_key: found.key.clone(),
                    kind: found.kind,
                    subject_type: found.subject.as_ref().map(|s| s.subject_type.clone()),
                    subject_id: found.subject.as_ref().map(|s| s.subject_id),
                    party_id: found.party,
                    lot_id: found.lot_id,
                    location_id: found.location_id,
                    quantity_milli: found.quantity_milli,
                    sales_count: found.sales,
                    occurred_on: found.on,
                    delivery_status: found.delivery.clone(),
                    notice_status: NoticeStatus::Pending,
                    ..RecallLine::default()
                };
                row.insert(&mut tx).await?;
            }
            Some(existing)
                if found.kind == LineKind::WalkIn
                    && (existing.sales_count != found.sales
                        || existing.quantity_milli != found.quantity_milli) =>
            {
                RecallLine::query()
                    .where_(RecallLine::ID.eq(existing.id))
                    .update(
                        &mut tx,
                        &[
                            ("sales_count", &found.sales),
                            ("quantity_milli", &found.quantity_milli),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
            }
            Some(_) => {}
        }
    }
    let outcome = match by {
        Some(user) => {
            recall
                .transition(TRACE)
                .by(user)
                .meta(renox::serde_json::json!({
                    "lines": plan.lines.len(),
                    "new_lots": plan.new_lots.len(),
                }))
                .run_in(&mut tx)
                .await?
        }
        None => {
            recall
                .transition(TRACE_BY_SYSTEM)
                .by_system()
                .meta(renox::serde_json::json!({
                    "lines": plan.lines.len(),
                    "new_lots": plan.new_lots.len(),
                }))
                .run_in(&mut tx)
                .await?
        }
    };
    tx.commit().await?;
    for lot_outcome in lot_outcomes {
        lot_outcome.emit(state).await?;
    }
    if let Some(placed) = placed {
        announce_placed(state, placed).await?;
    }
    if let Some(hold) = &open_hold
        && added_lines > 0
    {
        announce_added(state, hold.id, added_lines).await?;
    }
    outcome.emit(state).await?;
    Ok(Recall::find_or_404(&state.db, recall_id).await?)
}

/// The kernel's [`LotTrace`]: the affected lines of a recall and where its
/// lots are held.
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryTrace;

impl LotTrace for InventoryTrace {
    fn holders<'a>(
        &'a self,
        state: &'a AppState,
        recall: &'a Subject,
    ) -> TraceFuture<'a, Vec<TracedHolder>> {
        Box::pin(async move {
            if recall.subject_type != RECALL_SUBJECT_TYPE {
                return Ok(Vec::new());
            }
            Ok(lines_of(&state.db, recall.subject_id)
                .await?
                .iter()
                .map(RecallLine::holder)
                .collect())
        })
    }

    fn places<'a>(
        &'a self,
        state: &'a AppState,
        recall: &'a Subject,
    ) -> TraceFuture<'a, Vec<TracedPlace>> {
        Box::pin(async move {
            if recall.subject_type != RECALL_SUBJECT_TYPE {
                return Ok(Vec::new());
            }
            let mut places = Vec::new();
            for hold in holds_of(&state.db, recall.subject_id).await? {
                for line in crate::stock_hold::lines_of(&state.db, hold.id).await? {
                    if let Some(lot_id) = line.lot_id {
                        places.push(TracedPlace {
                            lot: Subject::new(LOT_SUBJECT_TYPE, lot_id),
                            location: Subject::new(LOCATION_SUBJECT_TYPE, line.location_id),
                            quantity_milli: line.quantity_milli,
                        });
                    }
                }
            }
            Ok(places)
        })
    }
}

/// The event of the traced recall `recall` with its `lots` and `lines`.
pub fn traced_event(recall: &Recall, lots: &[i64], lines: &[RecallLine]) -> RecallTraced {
    RecallTraced {
        recall: recall.subject(),
        number: recall.number.clone().unwrap_or_default(),
        lots: lots
            .iter()
            .map(|id| Subject::new(LOT_SUBJECT_TYPE, *id))
            .collect(),
        lines: lines.iter().map(RecallLine::holder).collect(),
    }
}

/// Registers the listener that tells the other modules a recall was traced,
/// after the commit.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Recall>, state: AppState| async move {
        if event.transition != TRACE && event.transition != TRACE_BY_SYSTEM {
            return Ok(());
        }
        let recall = Recall::find_or_404(&state.db, event.key).await?;
        if recall.status != RecallStatus::Traced {
            return Ok(());
        }
        let lots = lots_in_scope(&state.db, recall.id).await?;
        let lines = lines_of(&state.db, recall.id).await?;
        state.emit(traced_event(&recall, &lots, &lines)).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_movement_names_its_cost_object_or_a_source_outside_inventory() {
        let moved = StockMovement {
            cost_object_type: Some("clinics.visit".into()),
            cost_object_id: Some(4),
            source_type: "clinics.visit".into(),
            source_id: 4,
            ..StockMovement::default()
        };
        assert_eq!(subject_of(&moved).unwrap().to_string(), "clinics.visit:4");
        let moved = StockMovement {
            source_type: "inventory.location".into(),
            source_id: 2,
            ..StockMovement::default()
        };
        assert_eq!(subject_of(&moved), None);
        let moved = StockMovement {
            source_type: "manufacturing.work_order".into(),
            source_id: 8,
            ..StockMovement::default()
        };
        assert_eq!(
            subject_of(&moved).unwrap().to_string(),
            "manufacturing.work_order:8"
        );
    }
}
