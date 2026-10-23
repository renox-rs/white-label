//! Aborting a work order after materials were issued (07 Manufacturing §3.3
//! `MFG.ACL.12`, §4.2 row 1, §7.1 *Work order* `abort`; BL-MFG-01).
//!
//! A work order that has issued material or reported output is stopped only
//! by `abort`, never by `cancel`. The planner or the supervisor
//! (`work_orders.release`) asks with a reason and an outcome for the work in
//! progress: the unused materials that go back to the store, per item and lot
//! (never more than issued less returned); what is left of the issued
//! material, less what the good output used, is scrapped; the good output
//! stays in the work order's output lot in `quarantine`.
//!
//! - **One approval request, two rules.** The kernel defers `abort` as an
//!   approval request ([`policies`]): step 1 the production manager
//!   (`work_orders.abort_approve`, `MFG.ACL.12`), never the requester; and,
//!   only when the scrap value is above AED 2,000 ([`SCRAP_APPROVAL_FILS`]),
//!   step 2 a holder of `stock.approve` (`STK.ACL.02`). The work order keeps
//!   its state while the request waits and nothing is posted. Decision: the
//!   kernel's segregation of duties lets nobody decide two steps of one
//!   request, so one production manager who also holds `stock.approve`
//!   cannot decide both steps (stricter than the issue's edge case).
//! - **While it waits**, new issues and production reports are refused
//!   (*abort pending*, [`is_pending`]); a rejected or returned request leaves
//!   the work order as it was.
//! - **At the final approval** the kernel runs `abort_approved`, which has
//!   the same guard and effect ([`effects`]): in the transaction of the
//!   approval the issues are marked returned, the outcome lines are written
//!   ([`AbortLine`]), the actual cost is calculated and the loss kept
//!   ([`crate::cost`]), and the work order is `aborted`. A final check
//!   ([`recheck`]) fails the request when the posted quantities differ from
//!   the request (a late entry); a new request is made.
//! - **After the commit** ([`after_abort`], once per work order, and again by
//!   the daily sweep if it failed): the `return_in` and `wastage` movements
//!   are posted through the kernel's `ProductionStock` in one transaction
//!   with the claim, QC gets a task for the final inspection of the output
//!   lot, sales is told the outcome and `WorkOrderAborted` is emitted.
//!   Decision: the movements follow the commit because a transition's effect
//!   has no access to the other modules; the claim and the movements share a
//!   transaction, so they post all or not at all.
//! - **The scrap**: the value is [`crate::cost::scrap_value`]; it is spread
//!   over the issued items and lots in proportion to what is left of each
//!   after the return, and each share is a `wastage` movement at the quantity
//!   that share stands for.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionCtx, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, PolicyDefault, announce};
use wl_kernel::inventory::LOCATION_SUBJECT_TYPE;
use wl_kernel::machines::approved;
use wl_kernel::production::{ReturnMaterial, WasteMaterial, order_lines, production_stock};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::{Subject, approvals};

use crate::cost;
use crate::db::{answer, refused, set_order};
use crate::events::WorkOrderAborted;
use crate::issue::MaterialIssue;
use crate::notices;
use crate::permissions;
use crate::texts::english;
use crate::work_order::{ABORT, SUBJECT_TYPE, WorkOrder, WorkOrderStatus};

/// The approval action of `abort` (`MFG.ACL.12`).
pub const ACTION: &str = "manufacturing.abort";

/// The scrap value, in fils, above which a second step under `STK.ACL.02`
/// approves: AED 2,000.
pub const SCRAP_APPROVAL_FILS: i64 = 200_000;

/// The key of the scrap value in the transition input, which the policy of
/// the second step reads.
pub const SCRAP_KEY: &str = "scrap_value";

/// The default approval policies of `abort`: every request needs the
/// production manager (`MFG.ACL.12`); one whose scrap value is above the
/// limit needs a holder of `stock.approve` as well (`STK.ACL.02`).
pub fn policies() -> Vec<PolicyDefault> {
    vec![
        PolicyDefault::new(ACTION).step_permission(permissions::WORK_ORDERS_ABORT_APPROVE),
        PolicyDefault::new(ACTION)
            .amount_above(SCRAP_APPROVAL_FILS)
            .input_key(SCRAP_KEY)
            .step_permission(permissions::WORK_ORDERS_ABORT_APPROVE)
            .step_permission(permissions::STOCK_APPROVE),
    ]
}

/// What the abort did with a part of the work in progress.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_abort_lines", index(work_order_id))]
pub struct AbortLine {
    /// The primary key.
    pub id: i64,
    /// The work order.
    pub work_order_id: i64,
    /// `return` (materials back to the store), `scrap` (materials written
    /// off) or `output` (good output kept in quarantine).
    pub kind: String,
    /// The item.
    pub item_id: Option<i64>,
    /// The lot.
    pub lot_id: Option<i64>,
    /// The store a return goes to.
    pub location_id: Option<i64>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The value, in fils.
    pub value: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A material that goes back to the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnLine {
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked material.
    pub lot_id: Option<i64>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
}

/// What an `abort` run carries to its guard and effect; it is also the
/// approval request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortInput {
    /// Why.
    #[serde(default)]
    pub reason: String,
    /// The user who asked.
    #[serde(default)]
    pub requested_by: Option<i64>,
    /// The materials that go back.
    #[serde(default)]
    pub returns: Vec<ReturnLine>,
    /// The scrap value at the request, in fils (the policy's condition).
    #[serde(default)]
    pub scrap_value: i64,
    /// The quantity issued at the request, in thousandths.
    #[serde(default)]
    pub issued_milli: i64,
    /// The good output reported at the request.
    #[serde(default)]
    pub good_milli: i64,
    /// The scrap reported at the request.
    #[serde(default)]
    pub scrap_milli: i64,
}

/// The input of an `abort` run.
pub(crate) fn input_of(ctx: &TransitionCtx) -> AbortInput {
    ctx.input_as::<AbortInput>().unwrap_or_default()
}

/// The guard of `abort` and `abort_approved`: work to stop, and a reason.
pub(crate) fn guard(order: &WorkOrder, ctx: &TransitionCtx) -> Result<(), String> {
    if !order.has_work_in_progress() {
        return Err(english("manufacturing.abort.nothing", &[]));
    }
    if input_of(ctx).reason.trim().is_empty() {
        return Err(english("manufacturing.problems.reason", &[]));
    }
    Ok(())
}

/// The quantity of the issues of `item` and `lot` that can still go back.
fn returnable(issues: &[MaterialIssue], item: i64, lot: Option<i64>) -> i64 {
    cost::issued_quantity(issues, item, lot) - cost::returned_quantity(issues, item, lot)
}

/// The first return that asks for more than is left, as a refusal.
fn too_much(issues: &[MaterialIssue], returns: &[ReturnLine]) -> Option<String> {
    for line in returns {
        let total: i64 = returns
            .iter()
            .filter(|other| other.item_id == line.item_id && other.lot_id == line.lot_id)
            .map(|other| other.quantity_milli)
            .sum();
        if line.quantity_milli <= 0 || total > returnable(issues, line.item_id, line.lot_id) {
            return Some(english(
                "manufacturing.abort.return_too_much",
                &[("item", &line.item_id)],
            ));
        }
    }
    None
}

/// Marks `returns` as returned on `issues`, first issue first.
fn apply_returns(issues: &mut [MaterialIssue], returns: &[ReturnLine]) {
    for line in returns {
        let mut left = line.quantity_milli;
        for issue in issues.iter_mut().filter(|issue| {
            issue.pending_issue_id.is_none()
                && issue.item_id == line.item_id
                && issue.lot_id == line.lot_id
        }) {
            let part = left.min(issue.quantity_milli - issue.returned_milli).max(0);
            issue.returned_milli += part;
            left -= part;
        }
    }
}

/// How the scrap value is spread: `(item, lot, quantity, value)` per issued
/// item and lot, in proportion to what is left of each after the return. The
/// values add up to `scrap_value`.
pub fn scrap_lines(
    issues: &[MaterialIssue],
    scrap_value: i64,
) -> Vec<(i64, Option<i64>, i64, i64)> {
    let mut groups: Vec<(i64, Option<i64>)> = Vec::new();
    for issue in issues.iter().filter(|i| i.pending_issue_id.is_none()) {
        let key = (issue.item_id, issue.lot_id);
        if !groups.contains(&key) {
            groups.push(key);
        }
    }
    let left: Vec<(i64, Option<i64>, i64, i64)> = groups
        .iter()
        .map(|(item, lot)| {
            let issued_q = cost::issued_quantity(issues, *item, *lot);
            let issued_v: i64 = issues
                .iter()
                .filter(|i| i.pending_issue_id.is_none() && i.item_id == *item && i.lot_id == *lot)
                .map(|i| cost::value_of(i.quantity_milli, i.unit_cost))
                .sum();
            let returned_q = cost::returned_quantity(issues, *item, *lot);
            let returned_v = cost::average_value(issues, *item, *lot, returned_q);
            (*item, *lot, issued_q - returned_q, issued_v - returned_v)
        })
        .filter(|(_, _, quantity, value)| *quantity > 0 && *value > 0)
        .collect();
    let total: i64 = left.iter().map(|(_, _, _, value)| *value).sum();
    if total <= 0 || scrap_value <= 0 {
        return Vec::new();
    }
    let mut given = 0;
    let mut out = Vec::new();
    for (index, (item, lot, quantity, value)) in left.iter().enumerate() {
        let share = if index + 1 == left.len() {
            scrap_value - given
        } else {
            i64::try_from(i128::from(scrap_value) * i128::from(*value) / i128::from(total))
                .unwrap_or(0)
        };
        given += share;
        if share <= 0 {
            continue;
        }
        let scrapped = i64::try_from(
            (2 * i128::from(*quantity) * i128::from(share) + i128::from(*value))
                / (2 * i128::from(*value)),
        )
        .unwrap_or(0)
        .min(*quantity);
        out.push((*item, *lot, scrapped.max(1), share));
    }
    out
}

/// The effect of `abort` and `abort_approved` inside their transaction (see
/// the module).
pub(crate) async fn effects(tx: &mut Transaction, order: &WorkOrder, input: &AbortInput) -> Result {
    let mut issues = MaterialIssue::of_order(&mut *tx, order.id).await?;
    if let Some(problem) = too_much(&issues, &input.returns) {
        return Err(refused(problem));
    }
    let before: Vec<i64> = issues.iter().map(|i| i.returned_milli).collect();
    apply_returns(&mut issues, &input.returns);
    for (issue, was) in issues.iter().zip(before) {
        if issue.returned_milli != was {
            MaterialIssue::query()
                .where_(MaterialIssue::ID.eq(issue.id))
                .update(
                    &mut *tx,
                    &[
                        ("returned_milli", &issue.returned_milli),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
    }
    let issued = cost::issued_value(&issues);
    let returned = cost::returned_value(&issues);
    let scrap = cost::scrap_value(issued, returned, order.good_milli, order.std_material_cost);
    let mut lines = Vec::new();
    for line in &input.returns {
        let location = issues
            .iter()
            .find(|i| i.item_id == line.item_id && i.lot_id == line.lot_id)
            .map(|i| i.location_id);
        lines.push(AbortLine {
            work_order_id: order.id,
            kind: "return".into(),
            item_id: Some(line.item_id),
            lot_id: line.lot_id,
            location_id: location,
            quantity_milli: line.quantity_milli,
            value: cost::average_value(&issues, line.item_id, line.lot_id, line.quantity_milli),
            ..AbortLine::default()
        });
    }
    for (item, lot, quantity, value) in scrap_lines(&issues, scrap) {
        lines.push(AbortLine {
            work_order_id: order.id,
            kind: "scrap".into(),
            item_id: Some(item),
            lot_id: lot,
            quantity_milli: quantity,
            value,
            ..AbortLine::default()
        });
    }
    if order.good_milli > 0 {
        lines.push(AbortLine {
            work_order_id: order.id,
            kind: "output".into(),
            item_id: Some(order.item_id),
            lot_id: order.output_lot_id,
            quantity_milli: order.good_milli,
            value: cost::value_of(order.good_milli, order.std_unit_cost),
            ..AbortLine::default()
        });
    }
    for line in lines {
        AbortLine::create(&mut *tx, line).await?;
    }
    let stopped = now();
    let actual = cost::actual_cost(tx, order, stopped).await?;
    let loss = cost::loss(actual, order.good_milli, order.std_unit_cost);
    let none: Option<String> = None;
    set_order(
        &mut *tx,
        order.id,
        &[
            ("abort_reason", &Some(input.reason.trim().to_owned())),
            ("abort_requested_by", &input.requested_by),
            ("abort_scrap_value", &Some(scrap)),
            ("abort_loss", &Some(loss)),
            ("actual_cost", &Some(actual)),
            ("aborted_at", &Some(stopped)),
            ("stopped_at", &Some(stopped)),
            ("hold_reason", &none),
        ],
    )
    .await
}

/// The abort requests of `order` that wait for a decision.
async fn waiting(state: &AppState, order: &WorkOrder) -> Result<Vec<ApprovalRequest>> {
    Ok(ApprovalRequest::pending_about(&state.db, &order.subject())
        .await?
        .into_iter()
        .filter(|request| request.action == ACTION)
        .collect())
}

/// Whether an abort of `order` waits for a decision: new issues and
/// production reports are refused meanwhile.
///
/// # Errors
///
/// A database error.
pub async fn is_pending(state: &AppState, order: &WorkOrder) -> Result<bool> {
    Ok(!waiting(state, order).await?.is_empty())
}

/// What the planner asks.
#[derive(Debug, Clone, Default)]
pub struct AbortAsk {
    /// Why.
    pub reason: String,
    /// The materials that go back to the store.
    pub returns: Vec<ReturnLine>,
}

/// What a request for the abort did.
#[derive(Debug, Clone)]
pub struct Requested {
    /// The work order: unchanged while the production manager decides.
    pub order: WorkOrder,
    /// The approval request.
    pub approval_request: Option<i64>,
    /// The scrap value the request stands for, in fils.
    pub scrap_value: i64,
}

/// Why `abort` is not offered for `order`, if it is not.
pub fn not_offered(order: &WorkOrder) -> Option<String> {
    if !matches!(
        order.status,
        WorkOrderStatus::Released | WorkOrderStatus::InProgress | WorkOrderStatus::OnHold
    ) {
        return Some(english("manufacturing.abort.state", &[]));
    }
    if !order.has_work_in_progress() {
        return Some(english("manufacturing.abort.nothing", &[]));
    }
    None
}

/// `by` asks to abort work order `id` (see the module). The run is deferred
/// as an approval request for the production manager; nothing changes and
/// nothing is posted until it is approved.
///
/// # Errors
///
/// `Forbidden` without `work_orders.release` for the work order; a 422 when
/// abort is not offered (use `cancel` when nothing was issued or reported),
/// while another request waits, without a reason, or for a return of more
/// than was issued and not yet returned.
pub async fn request(state: &AppState, by: &AuthUser, id: i64, ask: AbortAsk) -> Result<Requested> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if !order.may(by, permissions::WORK_ORDERS_RELEASE) {
        return Err(Error::Forbidden);
    }
    if let Some(problem) = not_offered(&order) {
        return Err(refused(problem));
    }
    if is_pending(state, &order).await? {
        return Err(refused(english("manufacturing.abort.already_pending", &[])));
    }
    if ask.reason.trim().is_empty() {
        return Err(refused(english("manufacturing.problems.reason", &[])));
    }
    let issues = MaterialIssue::of_order(&state.db, id).await?;
    if let Some(problem) = too_much(&issues, &ask.returns) {
        return Err(refused(problem));
    }
    let mut after = issues.clone();
    apply_returns(&mut after, &ask.returns);
    let scrap = cost::scrap_value(
        cost::issued_value(&after),
        cost::returned_value(&after),
        order.good_milli,
        order.std_material_cost,
    );
    let input = AbortInput {
        reason: ask.reason.trim().to_owned(),
        requested_by: Some(by.id),
        returns: ask.returns,
        scrap_value: scrap,
        issued_milli: issued_total(&issues),
        good_milli: order.good_milli,
        scrap_milli: order.scrap_milli,
    };
    let outcome = order
        .transition(ABORT)
        .by(by)
        .reason(&input.reason)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending
            .data
            .get(approvals::submit::REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        Outcome::Done(_) => None,
    };
    outcome.emit(state).await?;
    Ok(Requested {
        order: WorkOrder::find_or_404(&state.db, id).await?,
        approval_request,
        scrap_value: scrap,
    })
}

/// The quantity posted as issued, in thousandths.
fn issued_total(issues: &[MaterialIssue]) -> i64 {
    issues
        .iter()
        .filter(|issue| issue.pending_issue_id.is_none())
        .map(|issue| issue.quantity_milli)
        .sum()
}

/// The final check of an abort request (BL-KRN-01): the posted quantities
/// still match the request, else it fails and a new one is made.
pub(crate) async fn recheck(
    tx: &mut Transaction,
    request: &ApprovalRequest,
) -> Result<Option<String>> {
    if request.action != ACTION {
        return Ok(None);
    }
    let order = WorkOrder::find_or_404(&mut *tx, request.subject_id).await?;
    let issues = MaterialIssue::of_order(&mut *tx, order.id).await?;
    let asked: AbortInput = renox::serde_json::from_value(request.payload.0.clone())
        .map_err(|e| Error::from(renox::anyhow::anyhow!(e)))?;
    if asked.issued_milli != issued_total(&issues)
        || asked.good_milli != order.good_milli
        || asked.scrap_milli != order.scrap_milli
    {
        return Ok(Some(english("manufacturing.abort.quantities_changed", &[])));
    }
    Ok(None)
}

/// The approver of the abort of `order`: whoever approved the last step of
/// its approved request.
async fn approver_of(state: &AppState, order: &WorkOrder) -> Result<Option<i64>> {
    let Some(request) = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(SUBJECT_TYPE))
        .where_(ApprovalRequest::SUBJECT_ID.eq(order.id))
        .where_(ApprovalRequest::ACTION.eq(ACTION))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Approved))
        .order_by_desc(ApprovalRequest::ID)
        .first(&state.db)
        .await?
    else {
        return Ok(None);
    };
    Ok(request
        .decisions(&state.db)
        .await?
        .last()
        .map(|decision| decision.approver_id))
}

/// What follows the commit of the abort of work order `id`, once: the
/// movements post, QC is asked for the final inspection of the output lot,
/// sales is told the outcome and `WorkOrderAborted` is emitted. Answers
/// whether this run did it.
///
/// # Errors
///
/// The database's, the kernel's and the notifier's errors.
pub async fn after_abort(state: &AppState, id: i64) -> Result<bool> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if order.status != WorkOrderStatus::Aborted {
        return Ok(false);
    }
    let lines = AbortLine::query()
        .where_(AbortLine::WORK_ORDER_ID.eq(id))
        .order_by(AbortLine::ID)
        .get(&state.db)
        .await?;
    let approver = approver_of(state, &order).await?;
    let stock = production_stock(state);
    let reason = order.abort_reason.clone().unwrap_or_default();
    let mut tx = state.db.begin().await?;
    // Claim and movements share a transaction: a second run finds nothing to
    // do, and a failure leaves the claim for the sweep.
    let claimed = WorkOrder::query()
        .where_(WorkOrder::ID.eq(id))
        .where_(WorkOrder::ABORT_ANNOUNCED_AT.is_null())
        .update(
            &mut tx,
            &[
                ("abort_announced_at", &Some(now())),
                ("abort_approved_by", &approver),
            ],
        )
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let (mut returned, mut scrapped) = (0, 0);
    for line in &lines {
        let (Some(item), quantity) = (line.item_id, line.quantity_milli) else {
            continue;
        };
        let item = Subject::new(wl_kernel::inventory::ITEM_SUBJECT_TYPE, item);
        let lot = line
            .lot_id
            .map(|lot| Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, lot));
        match line.kind.as_str() {
            "return" => {
                returned += line.value;
                let Some(location) = line.location_id else {
                    continue;
                };
                stock
                    .return_material(
                        &mut tx,
                        ReturnMaterial {
                            source: order.subject(),
                            item,
                            lot,
                            location: Subject::new(LOCATION_SUBJECT_TYPE, location),
                            quantity_milli: quantity,
                            unit_cost: unit_cost(line.value, quantity),
                            user: approver,
                        },
                    )
                    .await?;
            }
            "scrap" => {
                scrapped += line.value;
                stock
                    .waste_material(
                        &mut tx,
                        WasteMaterial {
                            source: order.subject(),
                            item,
                            lot,
                            quantity_milli: quantity,
                            value: line.value,
                            reason: reason.clone(),
                            user: approver,
                        },
                    )
                    .await?;
            }
            _ => {}
        }
    }
    tx.commit().await?;
    // The task of `MFG.RUL.09` that asked for this abort is done (after the
    // commit: it reads the database on its own connection).
    crate::order_changes::close_abort_task(
        state,
        id,
        &english("manufacturing.order_change.abort_done", &[]),
    )
    .await?;
    let output_lot = order
        .output_lot_id
        .map(|lot| Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, lot));
    if order.good_milli > 0 {
        let subject = output_lot.clone().unwrap_or_else(|| order.subject());
        role_tasks::open(
            state,
            RoleTask::new(
                subject,
                permissions::QC_ROLE,
                &english(
                    "manufacturing.abort.inspection_title",
                    &[("order", &order.number)],
                ),
                &format!("manufacturing.final_inspection:{}", order.id),
            )
            .description(&english(
                "manufacturing.abort.inspection_body",
                &[("order", &order.number), ("reason", &reason)],
            )),
        )
        .await?;
    }
    let loss = order.abort_loss.unwrap_or_default();
    let outcome = english(
        "manufacturing.abort.sales_body",
        &[
            ("order", &order.number),
            ("reason", &reason),
            ("returned", &crate::screens::aed(returned)),
            ("scrapped", &crate::screens::aed(scrapped)),
        ],
    );
    let mut told = Vec::new();
    if let Some(sales) = order.sales_order_id.map(wl_kernel::sales::SalesOrderId)
        && let Some(line) = order.sales_order_line_id
        && let Some(info) = order_lines(state).line(state, sales, line).await?
        && let Some(owner) = info.owner
    {
        told.push(owner);
    }
    if told.is_empty() && order.sales_order_id.is_some() {
        told = notices::holders(state, permissions::SALES_ROLE, order.plant_id).await?;
    }
    notices::tell(
        state,
        order.subject(),
        &order.number,
        &told,
        &english(
            "manufacturing.abort.sales_title",
            &[("order", &order.number)],
        ),
        &outcome,
    )
    .await?;
    state
        .emit(WorkOrderAborted {
            work_order: order.subject(),
            reason,
            returned,
            scrapped,
            output_lot,
            loss,
            approver,
        })
        .await?;
    Ok(true)
}

fn unit_cost(value: i64, quantity: i64) -> i64 {
    if quantity <= 0 {
        return 0;
    }
    i64::try_from((i128::from(value) * 1_000 + i128::from(quantity) / 2) / i128::from(quantity))
        .unwrap_or(0)
}

/// The abort tells the people once it committed, direct or approved.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: Transitioned<WorkOrder>, state: AppState| async move {
            if [ABORT, approved(ABORT).as_str()].contains(&event.transition.as_str())
                && event.to == WorkOrderStatus::Aborted.as_str()
            {
                after_abort(&state, event.key).await?;
            }
            Ok(())
        },
    );
}

/// The aborts whose movements did not post (the claim rolled back): tried
/// again by the sweep.
///
/// # Errors
///
/// As [`after_abort`].
pub async fn sweep(state: &AppState) -> Result<usize> {
    let mut done = 0;
    for order in WorkOrder::query()
        .where_(WorkOrder::STATUS.eq(WorkOrderStatus::Aborted))
        .where_(WorkOrder::ABORT_ANNOUNCED_AT.is_null())
        .order_by(WorkOrder::ID)
        .get(&state.db)
        .await?
    {
        if after_abort(state, order.id).await? {
            done += 1;
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(item: i64, lot: i64, quantity: i64, cost: i64) -> MaterialIssue {
        MaterialIssue {
            item_id: item,
            lot_id: Some(lot),
            quantity_milli: quantity,
            unit_cost: cost,
            ..MaterialIssue::default()
        }
    }

    #[test]
    fn a_return_cannot_exceed_what_is_issued_less_returned() {
        let mut issues = vec![issue(1, 1, 500_000, 800)];
        let ask = |q| {
            vec![ReturnLine {
                item_id: 1,
                lot_id: Some(1),
                quantity_milli: q,
            }]
        };
        assert!(too_much(&issues, &ask(200_000)).is_none());
        assert!(too_much(&issues, &ask(500_001)).is_some());
        assert!(too_much(&issues, &ask(0)).is_some());
        apply_returns(&mut issues, &ask(200_000));
        assert!(too_much(&issues, &ask(300_000)).is_none());
        assert!(too_much(&issues, &ask(300_001)).is_some());
    }

    #[test]
    fn the_scrap_is_spread_over_what_is_left_and_adds_up() {
        let mut issues = vec![issue(1, 1, 500_000, 800), issue(2, 0, 100_000, 300)];
        apply_returns(
            &mut issues,
            &[ReturnLine {
                item_id: 1,
                lot_id: Some(1),
                quantity_milli: 200_000,
            }],
        );
        // Left: 300 kg of item 1 (AED 2,400) and 100 kg of item 2 (AED 300).
        let lines = scrap_lines(&issues, 150_000);
        assert_eq!(lines.iter().map(|l| l.3).sum::<i64>(), 150_000);
        assert_eq!(lines.len(), 2);
        let single = scrap_lines(&issues[..1], 150_000);
        assert_eq!(single, vec![(1, Some(1), 187_500, 150_000)]);
        assert!(scrap_lines(&issues, 0).is_empty());
    }

    #[test]
    fn the_second_step_policy_reads_the_scrap_value() {
        let input = AbortInput {
            scrap_value: 310_000,
            ..AbortInput::default()
        };
        let json = renox::serde_json::to_value(&input).unwrap();
        assert_eq!(json[SCRAP_KEY], 310_000);
        assert_eq!(policies().len(), 2);
    }
}
