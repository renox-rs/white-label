//! The production report (07 Manufacturing §6 *Production report*, §7.1): the
//! good quantity and the scrap a line reports for a work order that is
//! `in_progress` (`MFG.ACL.01`: the line's tablet and operators report only on
//! the work orders of their line). The good quantity is posted as a `produce`
//! movement into the work order's output lot, which the first report opens in
//! `quarantine` at the finished goods location of the line (the lot cannot be
//! picked or issued before QC releases it, `MFG.ACL.02`).
//!
//! The report machine declares both states, `posted` and `voided`; the `void`
//! with its reversing movements comes with BL-MFG-04, and so does the
//! `wastage` movement of a report's scrap (the scrap is kept on the report
//! and on the work order). While an abort waits, reports are refused (*abort
//! pending*).

use renox::auth::permissions::Scope;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{ApprovalRequest, PolicyDefault, register_final_check};
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE};
use wl_kernel::machines::BoxFuture;
use wl_kernel::production::{ProduceOutput, VoidOutput, WasteMaterial, production_stock};
use wl_kernel::voids::{
    Effect, EffectKind, POSTED_KEY, RecordVoided, Reverse, ReverseError, VOID, VoidOutcome,
    VoidRecord, Voidable,
};
use wl_kernel::{Subject, SubjectMachine};

use crate::abort;
use crate::cost::value_of;
use crate::db::{answer, lock_order, refused, set_order};
use crate::permissions;
use crate::plant::Line;
use crate::texts::english;
use crate::work_order::{WorkOrder, WorkOrderStatus};

/// The subject type of a production report.
pub const SUBJECT_TYPE: &str = "manufacturing.production_report";

/// The approval action of the void of a report (`PLT.ACL.09`).
pub const VOID_ACTION: &str = "manufacturing.report_void";

/// Where a production report stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReportStatus {
    /// Counted in the totals of the work order.
    #[default]
    Posted,
    /// Entered on the wrong work order and taken back (terminal).
    Voided,
}

/// Output reported from a line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_production_reports",
    state_machine,
    index(work_order_id)
)]
pub struct ProductionReport {
    /// The primary key.
    pub id: i64,
    /// The work order.
    pub work_order_id: i64,
    /// The good quantity, in thousandths.
    pub good_milli: i64,
    /// The scrap, in thousandths.
    pub scrap_milli: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: ReportStatus,
    /// The user who reported.
    pub operator_id: Option<i64>,
    /// When the output was made (the business time).
    pub reported_at: DateTime,
    /// A note.
    pub note: Option<String>,
    /// The plant of the work order (where the void is approved when it has
    /// no line).
    pub plant_id: i64,
    /// The line it was reported on (where the void is approved).
    pub line_id: Option<i64>,
    /// The output lot the good quantity went into.
    pub lot_id: Option<i64>,
    /// The finished goods location it went to.
    pub location_id: Option<i64>,
    /// The cost of one unit it was posted at, in fils.
    pub unit_cost: i64,
    /// When the reversing stock movements of a void were posted.
    pub stock_reversed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ProductionReport {
    /// The report as a [`Subject`]: `manufacturing.production_report:9`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The number of the report, `PR-9`; a voided report keeps it.
    pub fn number(&self) -> String {
        format!("PR-{}", self.id)
    }

    /// Where a void is asked and approved: the line, else the plant.
    pub fn scope(&self) -> Scope {
        match self.line_id {
            Some(line) => permissions::line_scope(line),
            None => permissions::plant_scope(self.plant_id),
        }
    }

    /// The finished item, the lot and the location of the good output, when
    /// the report made some.
    fn output(&self, order: &WorkOrder) -> Option<(Subject, Option<Subject>, Subject)> {
        let location = self.location_id?;
        (self.good_milli > 0).then(|| {
            (
                Subject::new(ITEM_SUBJECT_TYPE, order.item_id),
                self.lot_id.map(|lot| Subject::new(LOT_SUBJECT_TYPE, lot)),
                Subject::new(LOCATION_SUBJECT_TYPE, location),
            )
        })
    }
}

impl StateMachine for ProductionReport {
    type State = ReportStatus;
    const COLUMN: renox::db::Col<ProductionReport, ReportStatus> = ProductionReport::STATUS;

    fn machine() -> Machine<Self> {
        let [void, approved] = wl_kernel::voids::transitions::<ProductionReport, _>(
            SUBJECT_TYPE,
            [ReportStatus::Posted],
        );
        Machine::new()
            .initial([ReportStatus::Posted])
            .state(ReportStatus::Posted)
            .tone("success")
            .state(ReportStatus::Voided)
            .tone("neutral")
            .terminal()
            .transition(void)
            .transition(approved)
    }
}

/// What a void of `report` would reverse: the good output and the scrap, as
/// stock movements.
fn effects_of(report: &ProductionReport) -> Vec<Effect> {
    let mut effects = Vec::new();
    if report.good_milli > 0 {
        effects.push(
            Effect::new(
                EffectKind::StockMove,
                format!("produce {}", report.number()),
            )
            .quantity(report.good_milli)
            .amount(value_of(report.good_milli, report.unit_cost)),
        );
    }
    if report.scrap_milli > 0 {
        effects.push(
            Effect::new(
                EffectKind::StockMove,
                format!("wastage {}", report.number()),
            )
            .quantity(report.scrap_milli)
            .amount(value_of(report.scrap_milli, report.unit_cost)),
        );
    }
    effects
}

/// The work order of a report is still open: its totals can be corrected.
fn locked_message(order: &WorkOrder) -> String {
    english(
        "manufacturing.void.locked",
        &[("status", &order.status.as_str().replace('_', " "))],
    )
}

fn pending_effects<'a>(
    tx: &'a mut renox::db::Transaction,
    id: i64,
) -> BoxFuture<'a, Result<Vec<Effect>>> {
    Box::pin(async move {
        let report = ProductionReport::find_or_404(&mut *tx, id).await?;
        Ok(effects_of(&report))
    })
}

fn reverse_effects<'a>(
    tx: &'a mut renox::db::Transaction,
    id: i64,
) -> BoxFuture<'a, std::result::Result<Vec<Effect>, ReverseError>> {
    Box::pin(async move {
        let report = ProductionReport::find_or_404(&mut *tx, id).await?;
        let order = lock_order(tx, report.work_order_id).await?;
        if !order.status.is_open() {
            return Err(ReverseError::Failed(locked_message(&order)));
        }
        set_order(
            tx,
            order.id,
            &[
                ("good_milli", &(order.good_milli - report.good_milli).max(0)),
                (
                    "scrap_milli",
                    &(order.scrap_milli - report.scrap_milli).max(0),
                ),
            ],
        )
        .await?;
        Ok(effects_of(&report))
    })
}

/// Registers the machine, the void and its approval with the kernel, and the
/// machine with `rnx state:diagram`.
pub(crate) fn register(app: &mut Registry) {
    renox::state_machine::register::<ProductionReport>();
    SubjectMachine::of::<ProductionReport>(SUBJECT_TYPE)
        .action(VOID, VOID_ACTION)
        .scope(|report: &ProductionReport| report.scope())
        .register();
    Voidable::of::<ProductionReport>(SUBJECT_TYPE)
        .permission(permissions::WORK_ORDERS_REPORT)
        .scope(|report: &ProductionReport| report.scope())
        .number("production_report", |report: &ProductionReport| {
            Some(report.number())
        })
        .reverse(
            EffectKind::StockMove,
            Reverse::new(pending_effects, reverse_effects),
        )
        .register();
    // The kernel's default policy has anyone who may void decide it; here it
    // is the line's supervisor or the production manager (`MFG.ACL.10`).
    PolicyDefault::new(VOID_ACTION)
        .amount_above(0)
        .input_key(POSTED_KEY)
        .step_permission(permissions::WORK_ORDERS_VOID_APPROVE)
        .register();
    // The work order was completed while the void waited: the request fails.
    register_final_check(SUBJECT_TYPE, |tx, request| Box::pin(recheck(tx, request)));
    app.listen(|event: RecordVoided, state: AppState| async move {
        if event.subject.subject_type == SUBJECT_TYPE {
            after_void(&state, event.subject.subject_id).await?;
        }
        Ok(())
    });
}

/// The final check of a void request: the work order is still open.
async fn recheck(
    tx: &mut renox::db::Transaction,
    request: &ApprovalRequest,
) -> Result<Option<String>> {
    if request.action != VOID_ACTION {
        return Ok(None);
    }
    let report = ProductionReport::find_or_404(&mut *tx, request.subject_id).await?;
    let order = WorkOrder::find_or_404(&mut *tx, report.work_order_id).await?;
    if order.status.is_open() {
        Ok(None)
    } else {
        Ok(Some(locked_message(&order)))
    }
}

/// What the line reports.
#[derive(Debug, Clone, Default)]
pub struct ReportAsk {
    /// The good quantity, in thousandths.
    pub good_milli: i64,
    /// The scrap, in thousandths.
    pub scrap_milli: i64,
    /// A note.
    pub note: Option<String>,
}

/// `by` reports output on work order `id`.
///
/// # Errors
///
/// `Forbidden` without `work_orders.report` in the plant or the line of the
/// work order (`MFG.ACL.01`); a 422 unless it is `in_progress`, while an
/// abort waits, for a quantity below zero or nothing reported, or a line
/// without a finished goods location.
pub async fn record(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: ReportAsk,
) -> Result<ProductionReport> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if !order.may(by, permissions::WORK_ORDERS_REPORT) {
        return Err(Error::Forbidden);
    }
    post(state, by.id, id, ask, None).await
}

/// Posts the output `ask` on work order `id` as user `operator`, made at
/// `made_at` (the business time; now when `None`). The permission was checked
/// by the caller: [`record`], or the late entry that was approved
/// ([`crate::late_report`]).
///
/// # Errors
///
/// The 422 refusals of [`record`].
pub(crate) async fn post(
    state: &AppState,
    operator: i64,
    id: i64,
    ask: ReportAsk,
    made_at: Option<DateTime>,
) -> Result<ProductionReport> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if order.status != WorkOrderStatus::InProgress {
        return Err(refused(english("manufacturing.report.state", &[])));
    }
    if abort::is_pending(state, &order).await? {
        return Err(refused(english("manufacturing.abort.pending", &[])));
    }
    if ask.good_milli < 0 || ask.scrap_milli < 0 || ask.good_milli + ask.scrap_milli == 0 {
        return Err(refused(english("manufacturing.report.quantity", &[])));
    }
    let finished = match order.line_id {
        Some(line) => Line::find(&state.db, line)
            .await?
            .and_then(|line| line.finished_goods()),
        None => None,
    };
    if ask.good_milli > 0 && finished.is_none() {
        return Err(refused(english("manufacturing.report.no_location", &[])));
    }
    let stock = production_stock(state);
    let mut tx = state.db.begin().await?;
    let order = lock_order(&mut tx, id).await?;
    if order.status != WorkOrderStatus::InProgress {
        return Err(refused(english("manufacturing.report.state", &[])));
    }
    let mut lot = order
        .output_lot_id
        .map(|lot| wl_kernel::Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, lot));
    if ask.good_milli > 0
        && let Some(location) = finished.clone()
    {
        let opened = stock
            .produce(
                &mut tx,
                ProduceOutput {
                    source: order.subject(),
                    item: order.item(),
                    location,
                    lot: lot.clone(),
                    lot_number: format!("F-{}", order.number),
                    quantity_milli: ask.good_milli,
                    unit_cost: order.std_unit_cost,
                    user: Some(operator),
                },
            )
            .await?;
        if opened.is_some() {
            lot = opened;
        }
    }
    let mut report = ProductionReport {
        work_order_id: id,
        good_milli: ask.good_milli,
        scrap_milli: ask.scrap_milli,
        operator_id: Some(operator),
        reported_at: made_at.unwrap_or_else(renox::db::now),
        note: ask.note.filter(|note| !note.trim().is_empty()),
        plant_id: order.plant_id,
        line_id: order.line_id,
        lot_id: lot.as_ref().map(|lot| lot.subject_id),
        location_id: finished.as_ref().map(|location| location.subject_id),
        unit_cost: order.std_unit_cost,
        ..ProductionReport::default()
    };
    report.insert(&mut tx).await?;
    if ask.scrap_milli > 0 {
        stock
            .waste_material(&mut tx, scrap_of(&report, &order, "Scrap".to_owned()))
            .await?;
    }
    set_order(
        &mut tx,
        id,
        &[
            ("good_milli", &(order.good_milli + ask.good_milli)),
            ("scrap_milli", &(order.scrap_milli + ask.scrap_milli)),
            ("output_lot_id", &lot.map(|lot| lot.subject_id)),
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(report)
}

/// The `wastage` of the scrap of `report`, valued at the standard cost of a
/// unit; `what` starts its reason (`Scrap`, `Void of`).
fn scrap_of(report: &ProductionReport, order: &WorkOrder, what: String) -> WasteMaterial {
    WasteMaterial {
        source: order.subject(),
        item: order.item(),
        lot: None,
        quantity_milli: report.scrap_milli,
        value: value_of(report.scrap_milli, report.unit_cost),
        reason: format!("{what}, production report {}", report.number()),
        user: report.operator_id,
    }
}

/// `by` voids production report `id` with `reason` (`PLT.ACL.09`).
///
/// The kernel defers it as an approval request (it moves stock): the answer
/// is [`VoidOutcome::Pending`] until the line's supervisor or the production
/// manager approves.
///
/// # Errors
///
/// `Forbidden` without `work_orders.report` on the line of the report; a 422
/// without a reason, unless the report is posted, once the work order is
/// completed or otherwise stopped ("output already used"), while an abort or
/// another void of the report waits, or when the output lot holds less than
/// the report's good quantity.
pub async fn void(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<VoidOutcome> {
    let report = ProductionReport::find_or_404(&state.db, id).await?;
    if !by.has_permission_in(permissions::WORK_ORDERS_REPORT, &report.scope()) {
        return Err(Error::Forbidden);
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(english("manufacturing.void.reason", &[])));
    }
    let order = WorkOrder::find_or_404(&state.db, report.work_order_id).await?;
    if report.status != ReportStatus::Posted {
        return Err(refused(english("manufacturing.void.not_posted", &[])));
    }
    if !order.status.is_open() {
        return Err(refused(locked_message(&order)));
    }
    if abort::is_pending(state, &order).await? {
        return Err(refused(english("manufacturing.abort.pending", &[])));
    }
    if waiting(state, &report).await? {
        return Err(refused(english("manufacturing.void.waiting", &[])));
    }
    if let Some((item, lot, location)) = report.output(&order)
        && let Some(left) = production_stock(state)
            .output_available(state, &item, lot.as_ref(), &location)
            .await?
        && left < report.good_milli
    {
        return Err(refused(used_message(left, report.good_milli)));
    }
    wl_kernel::voids::void(state, by, &report.subject(), reason)
        .await
        .map_err(answer)
}

/// The refusal of a void whose output is gone.
fn used_message(left: i64, good: i64) -> String {
    english(
        "manufacturing.void.used",
        &[
            ("left", &crate::screens::quantity_text(left)),
            ("good", &crate::screens::quantity_text(good)),
        ],
    )
}

/// Whether a void of `report` waits for its approval.
pub async fn waiting(state: &AppState, report: &ProductionReport) -> Result<bool> {
    Ok(pending_request(state, report).await?.is_some())
}

/// The approval request that voids `report`, while it waits.
pub async fn pending_request(
    state: &AppState,
    report: &ProductionReport,
) -> Result<Option<ApprovalRequest>> {
    Ok(ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(SUBJECT_TYPE))
        .where_(ApprovalRequest::SUBJECT_ID.eq(report.id))
        .where_(ApprovalRequest::ACTION.eq(VOID_ACTION))
        .order_by_desc(ApprovalRequest::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .find(ApprovalRequest::is_pending))
}

/// After the commit of a void: the reversing movements, once.
///
/// The claim (`stock_reversed_at`) and the movements share a transaction: a
/// second run finds nothing to do, and a failure (the output was used
/// meanwhile) leaves the claim for the sweep. Returns whether this run
/// posted.
///
/// # Errors
///
/// The database's and the stock's errors.
pub async fn after_void(state: &AppState, id: i64) -> Result<bool> {
    let report = ProductionReport::find_or_404(&state.db, id).await?;
    if report.status != ReportStatus::Voided {
        return Ok(false);
    }
    let order = WorkOrder::find_or_404(&state.db, report.work_order_id).await?;
    let record = VoidRecord::of_subject(&state.db, &report.subject()).await?;
    let (reason, by) = record.map_or((String::new(), None), |void| {
        (void.reason, Some(void.voided_by))
    });
    let stock = production_stock(state);
    let mut tx = state.db.begin().await?;
    let claimed = ProductionReport::query()
        .where_(ProductionReport::ID.eq(id))
        .where_(ProductionReport::STOCK_REVERSED_AT.is_null())
        .update(&mut tx, &[("stock_reversed_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    if let Some((item, lot, location)) = report.output(&order) {
        stock
            .void_output(
                &mut tx,
                VoidOutput {
                    source: order.subject(),
                    item,
                    lot,
                    location,
                    quantity_milli: report.good_milli,
                    unit_cost: report.unit_cost,
                    reason: format!("Void of production report {}: {reason}", report.number()),
                    user: by,
                },
            )
            .await?;
    }
    if report.scrap_milli > 0 {
        let mut waste = scrap_of(&report, &order, "Void of the scrap".to_owned());
        waste.reason = format!("{}: {reason}", waste.reason);
        waste.user = by;
        stock.void_waste(&mut tx, waste).await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// The voids whose reversing movements did not post (the claim rolled
/// back): tried again by the sweep.
///
/// # Errors
///
/// The database's.
pub async fn sweep(state: &AppState) -> Result<usize> {
    let mut done = 0;
    for report in ProductionReport::query()
        .where_(ProductionReport::STATUS.eq(ReportStatus::Voided))
        .where_(ProductionReport::STOCK_REVERSED_AT.is_null())
        .order_by(ProductionReport::ID)
        .get(&state.db)
        .await?
    {
        // One that cannot post yet (the lot is short) does not hold up the
        // others; it is tried again at the next run.
        match after_void(state, report.id).await {
            Ok(true) => done += 1,
            Ok(false) => {}
            Err(error) => eprintln!(
                "manufacturing: the stock reversal of {} did not post: {error}",
                report.number()
            ),
        }
    }
    Ok(done)
}
