//! Returned and refused goods: inspect them and ask sales to deliver again or
//! cancel (07 Manufacturing §4.1 "When it goes wrong" rows "accepts only
//! part" and "whole delivery is refused", §7.1 *Dispatch* `confirm_delivery`
//! and `return`, §7.2 `MFG.RUL.10`; BL-MFG-07).
//!
//! **When it runs.** wl-inventory says `DispatchGoodsQuarantined` once goods
//! of a dispatch are booked into quarantine: the whole quantity of a dispatch
//! that came back (`return`, with its complaint ticket) or the refused
//! quantity received back at run close. It is not the driver recording the
//! refusal that starts anything.
//!
//! **What it does.**
//! - **One inspection per lot per event** ([`ReturnInspection`]). The lot is
//!   inspected with a checklist run of the template [`INSPECTION_TEMPLATE`]
//!   when `wl-ticketing` provides checklists (`WRK.CHK.01`); where it does
//!   not, the QC inspector gets a task for the lot and records the result on
//!   `/manufacturing/inspections` (*passed*: the lot is released; *failed*: an
//!   NCR is raised on it). A lot with an inspection open is not released
//!   ([`ReturnReleaseGuard`], `MFG.ACL.02`: refused goods are never released to
//!   stock without the inspection). The inspection of a returned dispatch is
//!   linked to its complaint ticket (a comment on it); a partial refusal has no
//!   complaint ticket, its reason stays on the inspection and on the sales
//!   decision.
//! - **One sales decision per dispatch and order line** ([`ReturnDecision`]):
//!   the order, the line, the refused quantity and the reasons, with the
//!   actions *New dispatch* (a draft dispatch copied from the original,
//!   `DispatchMaker`) and *Cancel the line* (`cancel_line` on the order). The
//!   task goes to the account's sales person (the owner of the order), else to
//!   the sales role; the same person is notified. Nothing is cancelled and no
//!   dispatch is made by the rule.
//! - **The decision closes by itself.** Its open quantity goes down by what a
//!   new dispatch that reached `dispatched` carries for the line
//!   (`DispatchSent`) and by what `cancel_line` cancels
//!   (`SalesOrderLineCancelled`); at 0 the task is completed. A partial
//!   re-dispatch or cancel (10 of 20) lowers the open quantity to 10 and keeps
//!   the task open. A cancelled order closes its decisions.
//!
//! **Idempotent.** Every event has a key (the dispatch, the lines and
//! quantities booked; the dispatch; the order line and what is open after the
//! cancellation); a replay finds it and creates nothing.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::{
    DispatchGoodsQuarantined, DispatchSent, LOT_SUBJECT_TYPE, QuarantinedLine, RedispatchLine,
    dispatch_directory, dispatch_maker,
};
use wl_kernel::lot_disposal::{LotFuture, LotReleaseGuard};
use wl_kernel::production::order_lines;
use wl_kernel::sales::{
    SALES_ORDER_SUBJECT_TYPE, SalesOrderCancelled, SalesOrderId, SalesOrderLineCancelled,
};
use wl_kernel::tickets::{
    Assignee, OpenTicket, TASK_CATEGORY, TICKET_SUBJECT_TYPE, TaskComment, TicketId, checklists,
    task_creator,
};
use wl_kernel::{Subject, role_tasks};

use crate::db::refused;
use crate::notices;
use crate::permissions;
use crate::screens::quantity_text;
use crate::texts::english;

/// The rule's identifier.
pub const RULE: &str = "MFG.RUL.10";

/// The checklist template a returned lot is inspected with (`WRK.CHK.01`).
pub const INSPECTION_TEMPLATE: &str = "returned_goods_inspection";

/// The states of an inspection.
pub mod inspection {
    /// Waiting for QC.
    pub const OPEN: &str = "open";
    /// Passed: the lot was released.
    pub const PASSED: &str = "passed";
    /// Failed: an NCR was raised.
    pub const FAILED: &str = "failed";
}

/// The states of a decision.
pub mod decision {
    /// Waiting for sales.
    pub const OPEN: &str = "open";
    /// Everything delivered again or cancelled.
    pub const CLOSED: &str = "closed";
}

/// A returned lot waiting for its inspection.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_return_inspections",
    unique(event_key, item_id, lot_key),
    index(lot_id),
    index(status)
)]
pub struct ReturnInspection {
    /// The primary key.
    pub id: i64,
    /// The event it answers.
    pub event_key: String,
    /// The dispatch the goods came back from.
    pub dispatch_id: i64,
    /// Its number.
    pub dispatch_number: String,
    /// The sales order.
    pub order_id: i64,
    /// The sale line, when the dispatch line names one.
    pub line_id: Option<i64>,
    /// The item.
    pub item_id: i64,
    /// The lot.
    pub lot_id: Option<i64>,
    /// The lot, or 0 for an item without one (the unique key).
    pub lot_key: i64,
    /// The quantity booked into quarantine, in thousandths.
    pub quantity_milli: i64,
    /// The refusal or return reason.
    pub reason: Option<String>,
    /// Whether the whole dispatch came back.
    pub returned: bool,
    /// The checklist run, when `wl-ticketing` started one.
    pub run_id: Option<i64>,
    /// The complaint ticket of a returned dispatch.
    pub complaint_id: Option<i64>,
    /// The QC task, when no checklist run was started.
    pub task_id: Option<i64>,
    /// `open`, `passed` or `failed`.
    pub status: String,
    /// The NCR a failed inspection raised.
    pub ncr_id: Option<i64>,
    /// The inspector's note.
    pub note: Option<String>,
    /// Who recorded the result.
    pub decided_by: Option<i64>,
    /// When.
    pub decided_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ReturnInspection {
    /// The lot as a subject, when there is one.
    pub fn lot(&self) -> Option<Subject> {
        self.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id))
    }

    /// Whether the inspection still waits.
    pub fn is_open(&self) -> bool {
        self.status == inspection::OPEN
    }

    /// The inspections waiting on lot `lot`.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn open_on(db: &Db, lot: i64) -> Result<Vec<ReturnInspection>> {
        ReturnInspection::query()
            .where_(ReturnInspection::LOT_ID.eq(Some(lot)))
            .where_(ReturnInspection::STATUS.eq(inspection::OPEN))
            .order_by(ReturnInspection::ID)
            .get(db)
            .await
    }
}

/// What sales has to decide about refused goods of one order line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_return_decisions",
    unique(dispatch_id, line_id),
    index(order_id, line_id),
    index(status)
)]
pub struct ReturnDecision {
    /// The primary key.
    pub id: i64,
    /// The dispatch the goods came back from.
    pub dispatch_id: i64,
    /// Its number.
    pub dispatch_number: String,
    /// The sales order.
    pub order_id: i64,
    /// Its number.
    pub order_number: String,
    /// The sale line.
    pub line_id: i64,
    /// The item.
    pub item_id: i64,
    /// The customer.
    pub customer_id: Option<i64>,
    /// The sales person it was given to.
    pub owner_id: Option<i64>,
    /// The quantity refused or returned, in thousandths.
    pub refused_milli: i64,
    /// What is still to deliver again or cancel, in thousandths.
    pub open_milli: i64,
    /// The reasons.
    pub reason: Option<String>,
    /// The sales task.
    pub task_id: Option<i64>,
    /// `open` or `closed`.
    pub status: String,
    /// How it closed: *delivered again*, *cancelled* or *order cancelled*.
    pub closed_why: Option<String>,
    /// When it closed.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ReturnDecision {
    /// Whether the decision still waits.
    pub fn is_open(&self) -> bool {
        self.status == decision::OPEN
    }

    /// The source key of its task.
    pub fn task_key(&self) -> String {
        format!("manufacturing.return_decision:{}", self.id)
    }
}

/// A quantity taken off a decision: delivered again or cancelled.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_return_moves", unique(decision_id, move_key))]
pub struct ReturnMove {
    /// The primary key.
    pub id: i64,
    /// The decision.
    pub decision_id: i64,
    /// The event it answers.
    pub move_key: String,
    /// `dispatch` or `cancel`.
    pub kind: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// Words (the number of the new dispatch).
    pub note: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The item in English words: its name and code, else `Item 6`.
async fn item_name(state: &AppState, id: i64) -> String {
    let subject = Subject::new(wl_kernel::inventory::ITEM_SUBJECT_TYPE, id);
    match wl_kernel::inventory::stock_service(state)
        .item_label(state, &subject)
        .await
    {
        Ok(Some(label)) => english(
            "manufacturing.screen.item_name",
            &[("name", &label.name), ("code", &label.code)],
        ),
        _ => english("manufacturing.screen.item_unknown", &[("id", &id)]),
    }
}

/// The lot in English words: `Lot L-001`, else `Lot 10`.
async fn lot_name(state: &AppState, lot: &Subject) -> String {
    let number = match wl_kernel::inventory::stock_service(state)
        .lot_number(state, lot)
        .await
    {
        Ok(Some(number)) => number,
        _ => lot.subject_id.to_string(),
    };
    english("manufacturing.screen.ncr.lot_name", &[("number", &number)])
}

/// The key of a quarantine event: the dispatch, whether it came back whole,
/// and the lines and quantities booked.
fn event_key(event: &DispatchGoodsQuarantined) -> String {
    let mut booked: Vec<String> = event
        .lines
        .iter()
        .map(|line| format!("{}x{}", line.line.subject_id, line.quantity_milli))
        .collect();
    booked.sort();
    format!(
        "quarantine:{}:{}:{}",
        event.dispatch.subject_id,
        u8::from(event.returned),
        booked.join(",")
    )
}

/// The sale line a quarantined line fulfils.
fn sale_line(line: &QuarantinedLine) -> Option<i64> {
    line.source_line
        .as_ref()
        .filter(|source| source.subject_type == "sales.sale_line")
        .map(|source| source.subject_id)
}

/// The sales person of `order`: its owner, else a holder of the sales role.
async fn sales_person(state: &AppState, owner: Option<i64>) -> Result<Option<i64>> {
    if owner.is_some() {
        return Ok(owner);
    }
    if let Some(user) = notices::holder_anywhere(state, permissions::SALES_ROLE).await? {
        return Ok(Some(user));
    }
    role_tasks::assignee(&state.db, permissions::SALES_ROLE).await
}

/// The QC inspector a task is given to.
async fn inspector(state: &AppState) -> Result<Option<i64>> {
    if let Some(user) = notices::holder_anywhere(state, permissions::QC_ROLE).await? {
        return Ok(Some(user));
    }
    role_tasks::assignee(&state.db, permissions::QC_ROLE).await
}

/// A comment on the ticket `id`, when there is one.
async fn comment(state: &AppState, ticket: Option<i64>, text: &str) -> Result {
    if let Some(ticket) = ticket {
        task_creator(state)
            .comment(state, TicketId(ticket), TaskComment::internal(text))
            .await?;
    }
    Ok(())
}

/// `event` booked goods into quarantine: the inspections, the complaint link
/// and the sales decisions. Answers whether this run did it (a replay does
/// not).
///
/// # Errors
///
/// The database's, the ticketing's and the notifier's errors.
pub async fn on_quarantined(state: &AppState, event: &DispatchGoodsQuarantined) -> Result<bool> {
    if event.source.subject_type != SALES_ORDER_SUBJECT_TYPE || event.lines.is_empty() {
        return Ok(false);
    }
    let key = event_key(event);
    let seen = ReturnInspection::query()
        .where_(ReturnInspection::EVENT_KEY.eq(key.as_str()))
        .first(&state.db)
        .await?;
    if seen.is_some() {
        return Ok(false);
    }
    let number = dispatch_directory(state)
        .facts(state, &event.dispatch)
        .await?
        .map(|facts| facts.number)
        .unwrap_or_default();
    let number = if number.is_empty() {
        event.dispatch.subject_id.to_string()
    } else {
        number
    };
    let complaint = event
        .complaint
        .as_ref()
        .filter(|ticket| ticket.subject_type == TICKET_SUBJECT_TYPE)
        .map(|ticket| ticket.subject_id);
    for line in &event.lines {
        inspect(state, event, &key, &number, complaint, line).await?;
    }
    decide(state, event, &number).await?;
    Ok(true)
}

/// The inspection of one returned lot: the row, the checklist run or the QC
/// task, and the link to the complaint ticket.
async fn inspect(
    state: &AppState,
    event: &DispatchGoodsQuarantined,
    key: &str,
    dispatch_number: &str,
    complaint: Option<i64>,
    line: &QuarantinedLine,
) -> Result {
    let lot_id = line.lot.as_ref().map(|lot| lot.subject_id);
    let mut row = ReturnInspection::create(
        &state.db,
        ReturnInspection {
            event_key: key.to_owned(),
            dispatch_id: event.dispatch.subject_id,
            dispatch_number: dispatch_number.to_owned(),
            order_id: event.source.subject_id,
            line_id: sale_line(line),
            item_id: line.item.subject_id,
            lot_id,
            lot_key: lot_id.unwrap_or_default(),
            quantity_milli: line.quantity_milli,
            reason: line.reason.clone(),
            returned: event.returned,
            complaint_id: complaint,
            status: inspection::OPEN.to_owned(),
            ..ReturnInspection::default()
        },
    )
    .await?;
    let on = line.lot.clone().unwrap_or_else(|| line.item.clone());
    let run = checklists(state)
        .start(state, INSPECTION_TEMPLATE, on.clone())
        .await?;
    let lot_name = match &line.lot {
        Some(lot) => lot_name(state, lot).await,
        None => item_name(state, line.item.subject_id).await,
    };
    let reason = line.reason.clone().unwrap_or_default();
    if let Some(run) = run {
        row.run_id = Some(run.0);
    } else {
        let title = english(
            "manufacturing.returns.inspect_title",
            &[("lot", &lot_name), ("dispatch", &dispatch_number)],
        );
        let body = english(
            "manufacturing.returns.inspect_body",
            &[
                ("lot", &lot_name),
                ("quantity", &quantity_text(line.quantity_milli)),
                ("dispatch", &dispatch_number),
                ("reason", &reason),
            ],
        );
        let mut ticket = OpenTicket::new(on, TASK_CATEGORY, &title)
            .description(&body)
            .source_key(&format!("manufacturing.return_inspection:{}", row.id));
        if let Some(user) = inspector(state).await? {
            ticket = ticket.assign_to(Assignee::user(user));
        }
        row.task_id = task_creator(state)
            .open(state, ticket)
            .await?
            .map(|t| t.id.0);
    }
    ReturnInspection::query()
        .where_(ReturnInspection::ID.eq(row.id))
        .update(
            &state.db,
            &[("run_id", &row.run_id), ("task_id", &row.task_id)],
        )
        .await?;
    comment(
        state,
        complaint,
        &english(
            "manufacturing.returns.complaint_link",
            &[
                ("lot", &lot_name),
                ("quantity", &quantity_text(line.quantity_milli)),
            ],
        ),
    )
    .await
}

/// The sales decisions of the lines booked: one per dispatch and sale line.
async fn decide(state: &AppState, event: &DispatchGoodsQuarantined, number: &str) -> Result {
    let mut lines: Vec<i64> = event.lines.iter().filter_map(sale_line).collect();
    lines.sort_unstable();
    lines.dedup();
    for line_id in lines {
        let booked: Vec<&QuarantinedLine> = event
            .lines
            .iter()
            .filter(|line| sale_line(line) == Some(line_id))
            .collect();
        let quantity: i64 = booked.iter().map(|line| line.quantity_milli).sum();
        let item = booked[0].item.subject_id;
        let mut reasons: Vec<String> = booked
            .iter()
            .filter_map(|line| line.reason.clone())
            .collect();
        reasons.sort();
        reasons.dedup();
        let reason = reasons.join(", ");
        let existing = ReturnDecision::query()
            .where_(ReturnDecision::DISPATCH_ID.eq(event.dispatch.subject_id))
            .where_(ReturnDecision::LINE_ID.eq(line_id))
            .first(&state.db)
            .await?;
        if let Some(found) = existing {
            // More of the same line came back (another receipt at run close).
            ReturnDecision::query()
                .where_(ReturnDecision::ID.eq(found.id))
                .update(
                    &state.db,
                    &[
                        ("refused_milli", &(found.refused_milli + quantity)),
                        ("open_milli", &(found.open_milli + quantity)),
                        ("status", &decision::OPEN),
                        ("closed_why", &None::<String>),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            comment(
                state,
                found.task_id,
                &english(
                    "manufacturing.returns.more_came_back",
                    &[("quantity", &quantity_text(quantity))],
                ),
            )
            .await?;
            continue;
        }
        let info = order_lines(state)
            .line(state, SalesOrderId(event.source.subject_id), line_id)
            .await?;
        let owner = sales_person(state, info.as_ref().and_then(|info| info.owner)).await?;
        let order_number = info
            .as_ref()
            .and_then(|info| info.number.clone())
            .unwrap_or_else(|| event.source.subject_id.to_string());
        let mut made = ReturnDecision::create(
            &state.db,
            ReturnDecision {
                dispatch_id: event.dispatch.subject_id,
                dispatch_number: number.to_owned(),
                order_id: event.source.subject_id,
                order_number: order_number.clone(),
                line_id,
                item_id: item,
                customer_id: event.recipient.map(|party| party.0),
                owner_id: owner,
                refused_milli: quantity,
                open_milli: quantity,
                reason: Some(reason.clone()).filter(|reason| !reason.is_empty()),
                status: decision::OPEN.to_owned(),
                ..ReturnDecision::default()
            },
        )
        .await?;
        let item_text = item_name(state, item).await;
        let params: [(&str, &(dyn std::fmt::Display + Sync)); 6] = [
            ("order", &order_number),
            ("line", &line_id),
            ("item", &item_text),
            ("quantity", &quantity_text(quantity)),
            ("dispatch", &number),
            ("reason", &reason),
        ];
        let title = english("manufacturing.returns.decide_title", &params);
        let body = english("manufacturing.returns.decide_body", &params);
        let mut ticket = OpenTicket::new(
            Subject::new(SALES_ORDER_SUBJECT_TYPE, event.source.subject_id),
            TASK_CATEGORY,
            &title,
        )
        .description(&body)
        .source_key(&made.task_key());
        if let Some(user) = owner {
            ticket = ticket.assign_to(Assignee::user(user));
        }
        made.task_id = task_creator(state)
            .open(state, ticket)
            .await?
            .map(|t| t.id.0);
        ReturnDecision::query()
            .where_(ReturnDecision::ID.eq(made.id))
            .update(&state.db, &[("task_id", &made.task_id)])
            .await?;
        notices::tell(
            state,
            Subject::new(SALES_ORDER_SUBJECT_TYPE, event.source.subject_id),
            &order_number,
            &owner.into_iter().collect::<Vec<_>>(),
            &title,
            &body,
        )
        .await?;
    }
    Ok(())
}

/// Takes `quantity` off the open decisions of line `line` of `order`, oldest
/// first, under the key `key` (a key used already takes nothing). Closes a
/// decision that reaches 0 and completes its task; comments on the task of one
/// that stays open.
async fn consume(
    state: &AppState,
    order: i64,
    line: i64,
    key: &str,
    kind: &str,
    quantity: i64,
    note: &str,
) -> Result {
    let mut left = quantity;
    let open = ReturnDecision::query()
        .where_(ReturnDecision::ORDER_ID.eq(order))
        .where_(ReturnDecision::LINE_ID.eq(line))
        .where_(ReturnDecision::STATUS.eq(decision::OPEN))
        .order_by(ReturnDecision::ID)
        .get(&state.db)
        .await?;
    for found in open {
        if left <= 0 {
            break;
        }
        let used = ReturnMove::query()
            .where_(ReturnMove::DECISION_ID.eq(found.id))
            .where_(ReturnMove::MOVE_KEY.eq(key))
            .first(&state.db)
            .await?;
        if used.is_some() {
            return Ok(());
        }
        let take = left.min(found.open_milli);
        if take <= 0 {
            continue;
        }
        left -= take;
        ReturnMove::create(
            &state.db,
            ReturnMove {
                decision_id: found.id,
                move_key: key.to_owned(),
                kind: kind.to_owned(),
                quantity_milli: take,
                note: note.to_owned(),
                ..ReturnMove::default()
            },
        )
        .await?;
        let open_now = found.open_milli - take;
        if open_now == 0 {
            let why = if kind == "dispatch" {
                "delivered_again"
            } else {
                "cancelled"
            };
            close(state, &found, why).await?;
        } else {
            ReturnDecision::query()
                .where_(ReturnDecision::ID.eq(found.id))
                .update(
                    &state.db,
                    &[("open_milli", &open_now), ("updated_at", &renox::db::now())],
                )
                .await?;
            comment(
                state,
                found.task_id,
                &english(
                    "manufacturing.returns.open_now",
                    &[
                        ("quantity", &quantity_text(open_now)),
                        ("done", &quantity_text(take)),
                    ],
                ),
            )
            .await?;
        }
    }
    Ok(())
}

/// Closes `found` for `why` (`delivered_again`, `cancelled`,
/// `order_cancelled`) and completes its task.
async fn close(state: &AppState, found: &ReturnDecision, why: &str) -> Result {
    ReturnDecision::query()
        .where_(ReturnDecision::ID.eq(found.id))
        .update(
            &state.db,
            &[
                ("open_milli", &0_i64),
                ("status", &decision::CLOSED),
                ("closed_why", &Some(why.to_owned())),
                ("closed_at", &Some(renox::db::now())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if let Some(task) = found.task_id {
        let note = english(&format!("manufacturing.returns.closed.{why}"), &[]);
        task_creator(state)
            .complete(state, TicketId(task), &note)
            .await?;
    }
    Ok(())
}

/// A new dispatch left: the open decisions of its order lines go down by what
/// it carries for them.
///
/// # Errors
///
/// The database's and the ticketing's errors.
pub async fn on_sent(state: &AppState, event: &DispatchSent) -> Result {
    if event.source.subject_type != SALES_ORDER_SUBJECT_TYPE {
        return Ok(());
    }
    let number = dispatch_directory(state)
        .facts(state, &event.dispatch)
        .await?
        .map(|facts| facts.number)
        .unwrap_or_default();
    for line in &event.lines {
        let Some(source) = line
            .source_line
            .as_ref()
            .filter(|source| source.subject_type == "sales.sale_line")
        else {
            continue;
        };
        consume(
            state,
            event.source.subject_id,
            source.subject_id,
            &format!("sent:{}:{}", event.dispatch.subject_id, source.subject_id),
            "dispatch",
            line.quantity_milli,
            &number,
        )
        .await?;
    }
    Ok(())
}

/// A line was cancelled: the open decisions of the line go down by the
/// quantity cancelled.
///
/// # Errors
///
/// The database's and the ticketing's errors.
pub async fn on_line_cancelled(state: &AppState, event: &SalesOrderLineCancelled) -> Result {
    let open_after = order_lines(state)
        .line(state, event.order, event.line)
        .await?
        .map_or(0, |info| if info.cancelled { 0 } else { info.open_milli });
    consume(
        state,
        event.order.0,
        event.line,
        &format!(
            "cancel:{}:{}:{}:{}",
            event.order.0, event.line, event.quantity_milli, open_after
        ),
        "cancel",
        event.quantity_milli,
        &event.reason,
    )
    .await
}

/// The whole order was cancelled: its open decisions close.
///
/// # Errors
///
/// The database's and the ticketing's errors.
pub async fn on_order_cancelled(state: &AppState, event: &SalesOrderCancelled) -> Result {
    let open = ReturnDecision::query()
        .where_(ReturnDecision::ORDER_ID.eq(event.order.0))
        .where_(ReturnDecision::STATUS.eq(decision::OPEN))
        .get(&state.db)
        .await?;
    for found in open {
        close(state, &found, "order_cancelled").await?;
    }
    Ok(())
}

/// What `new_dispatch` made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redispatched {
    /// The draft dispatch.
    pub dispatch: Subject,
}

/// `by` makes a draft dispatch for the open quantity of decision `id`
/// (`quantity_milli` when given, at most what is open).
///
/// # Errors
///
/// `Forbidden` without `returned_goods.decide`; a 422 when the decision is closed,
/// the quantity is not above 0 or more than is open, or no dispatch can be
/// made.
pub async fn new_dispatch(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    quantity_milli: Option<i64>,
) -> Result<Redispatched> {
    if !by.has_permission(permissions::RETURNED_GOODS_DECIDE) {
        return Err(Error::Forbidden);
    }
    let found = ReturnDecision::find_or_404(&state.db, id).await?;
    if !found.is_open() {
        return Err(refused(english("manufacturing.returns.closed", &[])));
    }
    let quantity = quantity_milli.unwrap_or(found.open_milli);
    if quantity <= 0 || quantity > found.open_milli {
        return Err(refused(english(
            "manufacturing.returns.quantity",
            &[("open", &quantity_text(found.open_milli))],
        )));
    }
    let made = dispatch_maker(state)
        .redispatch(
            state,
            Some(by.id),
            &Subject::new(
                wl_kernel::inventory::DISPATCH_SUBJECT_TYPE,
                found.dispatch_id,
            ),
            &[RedispatchLine {
                source_line: Subject::new("sales.sale_line", found.line_id),
                quantity_milli: quantity,
            }],
        )
        .await?
        .ok_or_else(|| refused(english("manufacturing.returns.cannot_dispatch", &[])))?;
    comment(
        state,
        found.task_id,
        &english(
            "manufacturing.returns.dispatch_made",
            &[("quantity", &quantity_text(quantity))],
        ),
    )
    .await?;
    Ok(Redispatched { dispatch: made })
}

/// Whether `user` holds `permission` in any scope.
pub(crate) fn allows(user: &AuthUser, permission: &str) -> bool {
    user.has_permission(permission) || user.has_permission_anywhere(permission)
}

/// The open inspection `id`, for `user` who records results.
async fn open_inspection(state: &AppState, user: &AuthUser, id: i64) -> Result<ReturnInspection> {
    if !allows(user, permissions::INSPECTIONS_RECORD) {
        return Err(Error::Forbidden);
    }
    let found = ReturnInspection::find_or_404(&state.db, id).await?;
    if !found.is_open() {
        return Err(refused(english("manufacturing.returns.inspected", &[])));
    }
    if crate::late_inspection::waiting(state, found.id)
        .await?
        .is_some()
    {
        return Err(refused(english("manufacturing.returns.waiting", &[])));
    }
    Ok(found)
}

/// Marks the inspection `found` with `status`, the note and the user; its QC
/// task is completed and the complaint ticket told.
async fn finish(
    state: &AppState,
    (by, at): (i64, renox::db::DateTime),
    found: &ReturnInspection,
    status: &str,
    note: &str,
    ncr: Option<i64>,
    said: &str,
) -> Result<ReturnInspection> {
    ReturnInspection::query()
        .where_(ReturnInspection::ID.eq(found.id))
        .update(
            &state.db,
            &[
                ("status", &status),
                (
                    "note",
                    &Some(note.trim().to_owned()).filter(|n| !n.is_empty()),
                ),
                ("ncr_id", &ncr),
                ("decided_by", &Some(by)),
                ("decided_at", &Some(at)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if let Some(task) = found.task_id {
        task_creator(state)
            .complete(state, TicketId(task), said)
            .await?;
    }
    comment(state, found.complaint_id, said).await?;
    ReturnInspection::find_or_404(&state.db, found.id).await
}

/// `user` passes the inspection `id`: the lot is released. The goods are
/// still in the quarantine location; the storekeeper moves them to a store
/// with a transfer.
///
/// # Errors
///
/// `Forbidden` without `inspections.record`; a 422 when the inspection is not
/// open or the lot cannot be released (an NCR is open on it).
pub async fn pass(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> Result<ReturnInspection> {
    let found = open_inspection(state, user, id).await?;
    apply_pass(state, user.id, renox::db::now(), &found, note).await
}

/// Passes the open inspection `found` as `by`, decided at `at` (its business
/// time): the lot is released.
pub(crate) async fn apply_pass(
    state: &AppState,
    by: i64,
    at: renox::db::DateTime,
    found: &ReturnInspection,
    note: &str,
) -> Result<ReturnInspection> {
    // The inspection is passed first: it is what keeps the lot from release.
    ReturnInspection::query()
        .where_(ReturnInspection::ID.eq(found.id))
        .update(&state.db, &[("status", &inspection::PASSED)])
        .await?;
    if let Some(lot) = found.lot() {
        if let Some(why) = wl_kernel::lot_disposal::lot_release_blocked(state, &lot).await? {
            ReturnInspection::query()
                .where_(ReturnInspection::ID.eq(found.id))
                .update(&state.db, &[("status", &inspection::OPEN)])
                .await?;
            return Err(refused(why));
        }
        wl_kernel::lot_disposal::lot_disposal(state)
            .release(state, &lot)
            .await?;
    }
    finish(
        state,
        (by, at),
        found,
        inspection::PASSED,
        note,
        None,
        &english("manufacturing.returns.passed_said", &[]),
    )
    .await
}

/// `user` fails the inspection `id`: an NCR is raised on the lot (or the item)
/// for the quantity that came back.
///
/// # Errors
///
/// `Forbidden` without `inspections.record` and `ncrs.raise` in a plant; a 422
/// when the inspection is not open or the NCR cannot be raised.
pub async fn fail(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    defect: &str,
) -> Result<(ReturnInspection, crate::ncr::Ncr)> {
    let found = open_inspection(state, user, id).await?;
    let plant_id = ncr_plant(state, user).await?;
    apply_fail(state, user.id, renox::db::now(), &found, plant_id, defect).await
}

/// The first active plant where `user` may raise an NCR.
///
/// # Errors
///
/// `Forbidden` when there is none.
pub(crate) async fn ncr_plant(state: &AppState, user: &AuthUser) -> Result<i64> {
    for candidate in crate::plant::Plant::query()
        .order_by(crate::plant::Plant::ID)
        .get(&state.db)
        .await?
    {
        if candidate.active && user.has_permission_in(permissions::NCRS_RAISE, &candidate.scope()) {
            return Ok(candidate.id);
        }
    }
    Err(Error::Forbidden)
}

/// Fails the open inspection `found` as `by` (who may raise NCRs in plant
/// `plant_id`), decided at `at` (its business time): an NCR is raised.
pub(crate) async fn apply_fail(
    state: &AppState,
    by: i64,
    at: renox::db::DateTime,
    found: &ReturnInspection,
    plant_id: i64,
    defect: &str,
) -> Result<(ReturnInspection, crate::ncr::Ncr)> {
    let defect = if defect.trim().is_empty() {
        found.reason.clone().unwrap_or_default()
    } else {
        defect.trim().to_owned()
    };
    let ncr = crate::ncrs::raise_as(
        state,
        by,
        crate::ncrs::NewNcr {
            plant_id,
            lot: found.lot(),
            item: Some(Subject::new(
                wl_kernel::inventory::ITEM_SUBJECT_TYPE,
                found.item_id,
            )),
            quantity_milli: found.quantity_milli,
            defect,
            cause: None,
            origin: crate::ncr::Origin::Other,
            source: Some(Subject::new(
                wl_kernel::inventory::DISPATCH_SUBJECT_TYPE,
                found.dispatch_id,
            )),
        },
    )
    .await?;
    let done = finish(
        state,
        (by, at),
        found,
        inspection::FAILED,
        "",
        Some(ncr.id),
        &english("manufacturing.returns.failed_said", &[("ncr", &ncr.number)]),
    )
    .await?;
    Ok((done, ncr))
}

/// Keeps a lot from release while a returned-goods inspection waits on it
/// (`MFG.ACL.02`). Provided to wl-inventory with the NCR guard.
pub struct ReturnReleaseGuard;

impl LotReleaseGuard for ReturnReleaseGuard {
    fn blocked<'a>(&'a self, db: &'a Db, lot: &'a Subject) -> LotFuture<'a, Option<String>> {
        Box::pin(async move {
            if lot.subject_type != LOT_SUBJECT_TYPE {
                return Ok(None);
            }
            let waiting = ReturnInspection::open_on(db, lot.subject_id).await?;
            Ok(waiting.first().map(|found| {
                english(
                    "manufacturing.returns.keeps_lot",
                    &[("dispatch", &found.dispatch_number)],
                )
            }))
        })
    }
}

/// Registers the listeners of `MFG.RUL.10`.
pub fn listen(app: &mut Registry) {
    app.listen(
        |event: DispatchGoodsQuarantined, state: AppState| async move {
            on_quarantined(&state, &event).await.map(|_| ())
        },
    );
    app.listen(|event: DispatchSent, state: AppState| async move { on_sent(&state, &event).await });
    app.listen(
        |event: SalesOrderLineCancelled, state: AppState| async move {
            on_line_cancelled(&state, &event).await
        },
    );
    app.listen(|event: SalesOrderCancelled, state: AppState| async move {
        on_order_cancelled(&state, &event).await
    });
}
