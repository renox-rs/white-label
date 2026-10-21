//! Tracing a shipment that is not on a delivery, and marking it lost with an
//! approval (03 Logistics §4.1 row *Shipment cannot be found*, §7.1 row
//! `mark_lost`, §7.2 `LOG.RUL.12`, §3.3 `LOG.ACL.14`; BL-LOG-12).
//!
//! - **The rule** ([`run`], daily at 02:00 Asia/Dubai): a shipment `picked_up`,
//!   `at_hub`, `on_hold` or `returning` (a return in progress on a run is a
//!   delivery, `WRK.RUL.17`) whose latest scan, tracking event or note is
//!   N = 3 days of 24 hours old or more gets one trace task for the
//!   operations manager ([`TraceTask`]); M = 7 days after the same instant
//!   the task is escalated as *request `mark_lost` now*. The rule never
//!   requests the loss itself: a user does, and another approves. A scan, a
//!   note or any change of the shipment closes the task and restarts the
//!   clock. Only one task is open per shipment, so a second run on the same
//!   day opens nothing.
//! - **The request** ([`request`]): a user with `shipments.manage`
//!   asks, with a reason and a liability, at any time (the trace task only
//!   prompts it). The kernel defers it as an approval request that any holder
//!   of `shipments.approve_loss` (the operations manager, and the company admin
//!   who holds every permission) decides, never the requester. At the final
//!   approval nothing may have been scanned or noted since the request
//!   ([`recheck`]), or the request fails and the shipment stays.
//! - **Scans and notes** ([`scan_at_depot`], [`note`]): a scan of a shipment at
//!   a depot, and a note, are tracking events that count as activity; a scan
//!   of a lost shipment runs `found` ([`crate::loss::found`]).
//!
//! There is no depot scope unit yet, so the task goes to the holders of the
//! operations manager role, not to the manager of the last depot.

use renox::chrono::Duration;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{
    ApprovalRequest, PolicyDefault, announce, register_final_check, submit::REQUEST_KEY,
};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{self, TaskComment, TicketId};

use crate::loss::{LossInput, OFF_DELIVERY};
use crate::permissions;
use crate::shipment::{SUBJECT_TYPE, Shipment, ShipmentStatus, TrackingEvent, track};
use crate::shipment_machine::{MARK_LOST, ShipmentInput};
use crate::texts::english;

/// The rule, as the specification names it.
pub const RULE: &str = "LOG.RUL.12";

/// The task that runs the rule.
pub const TASK: &str = "logistics:trace";

/// The time of day the rule runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// N: days without a scan, tracking event or note before the trace task.
pub const TRACE_DAYS: i64 = 3;

/// M: days, counted from the same instant, before the task is escalated to
/// *request `mark_lost` now*.
pub const ESCALATE_DAYS: i64 = 7;

/// The approval policy's action key of a request to mark lost (`LOG.ACL.14`).
pub const ACTION: &str = "logistics.mark_lost";

/// Hours an approval request to mark a shipment lost may wait.
pub const EXPIRY_HOURS: i64 = 72;

/// The states the rule traces: not on a delivery.
const TRACED: [ShipmentStatus; 4] = [
    ShipmentStatus::PickedUp,
    ShipmentStatus::AtHub,
    ShipmentStatus::OnHold,
    ShipmentStatus::Returning,
];

/// The tracking events the module writes about its own work: they are not a
/// scan or a note, so they do not restart the clock.
const NOT_ACTIVITY: [&str; 11] = [
    "trace_task",
    "trace_escalated",
    "trace_closed",
    "claim_ticket",
    "claim_ticket_skipped",
    "claim_ticket_failed",
    "credit_due",
    "claim_invoice_skipped",
    "delivery_hold_failed",
    "delivery_cancel_failed",
    "delivery_request_failed",
];

/// The trace task of a shipment: one open at a time.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_trace_tasks", index(shipment_id))]
pub struct TraceTask {
    /// The primary key.
    pub id: i64,
    /// The shipment.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// The ticket of the task, when ticketing is on.
    pub ticket_id: Option<i64>,
    /// The latest scan, tracking event or note when the task was opened.
    pub last_seen_at: DateTime,
    /// When it was escalated to *request `mark_lost` now*.
    pub escalated_at: Option<DateTime>,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a run of the rule did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TraceRun {
    /// Trace tasks opened.
    pub opened: usize,
    /// Trace tasks escalated.
    pub escalated: usize,
    /// Trace tasks closed because the shipment was scanned or moved.
    pub closed: usize,
}

/// The latest scan, tracking event or note of `shipment`; its creation when it
/// has none.
///
/// # Errors
///
/// Database errors.
pub async fn last_activity(db: &Db, shipment: &Shipment) -> Result<DateTime> {
    let events = TrackingEvent::query()
        .where_(TrackingEvent::SHIPMENT_ID.eq(shipment.id))
        .order_by_desc(TrackingEvent::OCCURRED_AT)
        .get(db)
        .await?;
    Ok(events
        .into_iter()
        .find(|event| !NOT_ACTIVITY.contains(&event.kind.as_str()))
        .map(|event| event.occurred_at)
        .or(shipment.created_at)
        .unwrap_or_else(renox::db::now))
}

/// The open trace task of shipment `id`, if there is one.
async fn open_task(db: &Db, id: i64) -> Result<Option<TraceTask>> {
    TraceTask::query()
        .where_(TraceTask::SHIPMENT_ID.eq(id))
        .where_(TraceTask::CLOSED_AT.is_null())
        .order_by(TraceTask::ID)
        .first(db)
        .await
}

/// Closes `task`, with `note` on its ticket.
async fn close(state: &AppState, task: &TraceTask, note: &str) -> Result {
    let at = Some(renox::db::now());
    TraceTask::query()
        .where_(TraceTask::ID.eq(task.id))
        .update(&state.db, &[("closed_at", &at)])
        .await?;
    track(&state.db, task.shipment_id, "trace_closed", note, None).await?;
    if let Some(ticket) = task.ticket_id {
        tickets::task_creator(state)
            .complete(state, TicketId(ticket), note)
            .await?;
    }
    Ok(())
}

/// Closes the open trace task of shipment `id`, if any: the shipment was
/// scanned, noted or changed.
///
/// # Errors
///
/// Database errors and the ticketing ones.
pub(crate) async fn close_open(state: &AppState, id: i64, note: &str) -> Result {
    if let Some(task) = open_task(&state.db, id).await? {
        close(state, &task, note).await?;
    }
    Ok(())
}

/// Opens the trace task of `shipment`, whose latest activity was `last`.
async fn open(state: &AppState, shipment: &Shipment, last: DateTime) -> Result<TraceTask> {
    let title = english("logistics.trace.title", &[("number", &shipment.number)]);
    let description = english(
        "logistics.trace.description",
        &[
            ("number", &shipment.number),
            ("days", &TRACE_DAYS),
            ("status", &shipment.status.as_str()),
        ],
    );
    let task = RoleTask::new(
        shipment.subject(),
        permissions::OPERATIONS_MANAGER_ROLE,
        &title,
        &format!("logistics.trace:{}", shipment.id),
    )
    .description(&description);
    let ticket = role_tasks::open(state, task).await?.map(|t| t.id.0);
    let mut row = TraceTask {
        shipment_id: shipment.id,
        ticket_id: ticket,
        last_seen_at: last,
        ..TraceTask::default()
    };
    row.insert(&state.db).await?;
    track(&state.db, shipment.id, "trace_task", &title, None).await?;
    Ok(row)
}

/// Escalates `task` to *request `mark_lost` now*.
async fn escalate(state: &AppState, shipment: &Shipment, task: &TraceTask) -> Result {
    let at = Some(renox::db::now());
    TraceTask::query()
        .where_(TraceTask::ID.eq(task.id))
        .update(&state.db, &[("escalated_at", &at)])
        .await?;
    let text = english(
        "logistics.trace.escalated",
        &[("number", &shipment.number), ("days", &ESCALATE_DAYS)],
    );
    track(&state.db, shipment.id, "trace_escalated", &text, None).await?;
    if let Some(ticket) = task.ticket_id {
        tickets::task_creator(state)
            .comment(state, TicketId(ticket), TaskComment::internal(&text))
            .await?;
    }
    Ok(())
}

/// The rule `LOG.RUL.12` at `now`: closes the tasks of shipments that were
/// scanned or moved, opens one task per shipment untraced for N days and
/// escalates the open ones untraced for M days. Returns what it did.
///
/// # Errors
///
/// Database errors and the ticketing ones.
pub async fn run(state: &AppState, now: DateTime) -> Result<TraceRun> {
    let mut done = TraceRun::default();
    // A scan, a note or a change since the task was opened closes it.
    let open_tasks = TraceTask::query()
        .where_(TraceTask::CLOSED_AT.is_null())
        .order_by(TraceTask::ID)
        .get(&state.db)
        .await?;
    for task in open_tasks {
        let shipment = Shipment::find_or_404(&state.db, task.shipment_id).await?;
        let last = last_activity(&state.db, &shipment).await?;
        if !TRACED.contains(&shipment.status) || last > task.last_seen_at {
            close(state, &task, &english("logistics.trace.closed", &[])).await?;
            done.closed += 1;
        }
    }
    let candidates = Shipment::query()
        .where_(Shipment::STATUS.is_in(TRACED))
        .order_by(Shipment::ID)
        .get(&state.db)
        .await?;
    for shipment in candidates {
        let last = last_activity(&state.db, &shipment).await?;
        if last + Duration::days(TRACE_DAYS) > now {
            continue;
        }
        let task = match open_task(&state.db, shipment.id).await? {
            Some(task) => task,
            None => {
                done.opened += 1;
                open(state, &shipment, last).await?
            }
        };
        if task.escalated_at.is_none() && last + Duration::days(ESCALATE_DAYS) <= now {
            escalate(state, &shipment, &task).await?;
            done.escalated += 1;
        }
    }
    Ok(done)
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

async fn load(state: &AppState, id: i64) -> std::result::Result<Shipment, TransitionError> {
    Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (`shipments.scan`) scans shipment `id` at `depot`: a tracking event
/// that restarts the untraced clock and closes the trace task; for a lost
/// shipment it is `found`.
///
/// # Errors
///
/// `Forbidden` without `shipments.scan`; `Guard` (422) without a depot and for
/// a shipment that is not at a hub, picked up, held, returning or lost.
pub async fn scan_at_depot(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    depot: &str,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_SCAN) {
        return Err(TransitionError::Forbidden);
    }
    let shipment = load(state, id).await?;
    if shipment.status == ShipmentStatus::Lost {
        return crate::loss::found(state, user, id, depot).await;
    }
    if !TRACED.contains(&shipment.status) {
        return Err(guard(english("logistics.trace.not_traced", &[])));
    }
    if depot.trim().is_empty() {
        return Err(guard(english("logistics.loss.depot_needed", &[])));
    }
    track(&state.db, id, "depot_scan", depot.trim(), Some(user.id)).await?;
    close_open(state, id, &english("logistics.trace.scanned", &[])).await?;
    load(state, id).await
}

/// `user` (`shipments.manage`) adds a note to shipment `id`: a tracking event
/// that restarts the untraced clock and closes the trace task.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `Guard` (422) for a blank note.
pub async fn note(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    text: &str,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let shipment = load(state, id).await?;
    if text.trim().is_empty() {
        return Err(guard(english("logistics.trace.note_needed", &[])));
    }
    track(&state.db, shipment.id, "note", text.trim(), Some(user.id)).await?;
    close_open(state, id, &english("logistics.trace.noted", &[])).await?;
    load(state, id).await
}

/// What a request to mark lost came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostRequest {
    /// No approval was needed: the shipment is lost.
    Lost,
    /// An approval request (its id) waits for an operations manager or the
    /// company admin other than the requester.
    Requested(i64),
}

/// `user` (`shipments.manage`) asks to mark shipment `id`, which is on no
/// delivery, lost, with a `reason` and a `liability` (`driver`, `business`,
/// `client` or `none`). The kernel's policy `logistics.mark_lost`
/// (`LOG.ACL.14`) defers it as an approval request that a holder of
/// `shipments.approve_loss` other than the requester decides; the request is
/// announced after the commit. A request may be made at any time, also before
/// a trace task: the task only prompts it.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `NotFrom` for a shipment that is
/// not `picked_up`, `at_hub`, `on_hold` or `returning`; `Guard` (422) without
/// a reason, with an unknown liability, for a shipment on a delivery (the
/// delivery module marks that lost), and when a request is already pending.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
    liability: &str,
) -> std::result::Result<LostRequest, TransitionError> {
    let shipment = load(state, id).await?;
    if matches!(
        shipment.status,
        ShipmentStatus::OutForDelivery | ShipmentStatus::Attempted | ShipmentStatus::UnderReview
    ) {
        return Err(guard(english("logistics.trace.on_delivery", &[])));
    }
    let input = ShipmentInput {
        loss: Some(LossInput {
            reason: reason.trim().to_owned(),
            liability: liability.trim().to_owned(),
            claim_ticket: None,
        }),
        ..ShipmentInput::default()
    };
    let outcome = shipment
        .transition(MARK_LOST)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    announce(state, &outcome)
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

/// Whether a scan, a depot scan or a note was recorded for shipment `id`
/// after `since`.
async fn activity_after(tx: &mut Transaction, id: i64, since: DateTime) -> Result<bool> {
    let events = TrackingEvent::query()
        .where_(TrackingEvent::SHIPMENT_ID.eq(id))
        .where_(TrackingEvent::OCCURRED_AT.gt(since))
        .get(&mut *tx)
        .await?;
    Ok(events
        .iter()
        .any(|event| !NOT_ACTIVITY.contains(&event.kind.as_str())))
}

/// The final re-check of a request to mark lost (`LOG.ACL.14`): no scan, note
/// or other tracking event may have been recorded after the request; the
/// request then fails and the shipment stays where it is.
async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    if request.action != ACTION {
        return Ok(None);
    }
    let Some(since) = request.created_at else {
        return Ok(None);
    };
    if activity_after(tx, request.subject_id, since).await? {
        Ok(Some(english("logistics.trace.scanned_since", &[])))
    } else {
        Ok(None)
    }
}

/// Registers the approval policy, its final check and the daily rule.
pub(crate) fn register(app: &mut Registry) {
    // `LOG.ACL.14`: a holder of `shipments.approve_loss` other than the
    // requester approves (the operations manager, or the company admin).
    PolicyDefault::new(ACTION)
        .step_permission(permissions::SHIPMENTS_APPROVE_LOSS)
        .expiry_hours(Some(EXPIRY_HOURS))
        .register();
    register_final_check(SUBJECT_TYPE, |tx, request| Box::pin(recheck(tx, request)));
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, renox::db::now()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
}

/// The states a shipment can be marked lost from by a user.
pub fn requestable(status: ShipmentStatus) -> bool {
    OFF_DELIVERY.contains(&status)
}
