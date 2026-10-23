//! The record page of a work order (`GET /manufacturing/work-orders/{id}`,
//! `docs/UX.md` §2.2).
//!
//! The page shows the work order with its status, the facts (item, line,
//! quantity, source, planned dates, start and stop, output lot), the actions
//! for the people who may run them (a guard that fails now draws the button
//! disabled and says why in words), and the tabs *Overview*, *Materials* (the
//! pick list and what was issued), *Output* (the production reports),
//! *Approvals* and *Activity*.
//!
//! Decisions where the spec is silent:
//!
//! - **Which actions are drawn.** An action is drawn for a person who holds
//!   its permission (`WorkOrder::may`) when the work order is in a state the
//!   transition leaves; `abort` only when it is offered (material issued or
//!   output reported); `unlink` only while the source is the sales order.
//!   *Cancel* of a started work order is drawn disabled, naming `abort`.
//! - **Costs** (actual cost, scrap value and loss) show only to holders of
//!   `work_orders.cost_view` (`FIN.ACL.08`); the JSON leaves them out too.
//! - The item shows as `Item 9` until the page can read the item catalogue
//!   through a kernel contract.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, awaiting::action_label, decide};
use wl_kernel::voids::VoidRecord;

use super::{
    Act, Texts, date_text, period_text, quantity_text, time_text, transition_text, user_text,
};
use crate::abort::{self, AbortLine};
use crate::cost;
use crate::issue::MaterialIssue;
use crate::order_changes::{self, Adjustment};
use crate::permissions;
use crate::pick_list;
use crate::plant::Line;
use crate::report::{ProductionReport, ReportStatus};
use crate::texts::{screen_texts, t, tf, word};
use crate::unlink;
use crate::work_order::{HOLD_REASONS, WorkOrder, WorkOrderSource, WorkOrderStatus};
use crate::work_orders::takes_material;

/// The address of a work order's page.
pub(crate) fn url(id: i64) -> String {
    format!("/manufacturing/work-orders/{id}")
}

/// The source of a work order in words: the number of its sales order (the
/// order's id while the sales side does not tell it), or stock.
pub(crate) async fn source_text(
    state: &AppState,
    lang: &Lang,
    order: &WorkOrder,
) -> Result<String> {
    if order.source != WorkOrderSource::SalesOrder {
        return Ok(t(lang, "manufacturing.screen.source.stock"));
    }
    let id = order.sales_order_id.unwrap_or_default();
    let number = match (order.order(), order.sales_order_line_id) {
        (Some(sales), Some(line)) => wl_kernel::production::order_lines(state)
            .line(state, sales, line)
            .await?
            .and_then(|info| info.number),
        _ => None,
    };
    Ok(tf(
        lang,
        "manufacturing.screen.source.sales_order",
        &[("order", &number.unwrap_or_else(|| id.to_string()))],
    ))
}

/// The item of a work order in words.
pub(crate) fn item_text(lang: &Lang, item_id: i64) -> String {
    tf(lang, "manufacturing.screen.item_name", &[("id", &item_id)])
}

/// A fact of the overview.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Fact {
    label: String,
    value: String,
}

/// What the person may do with the work order now.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Actions {
    release: Act,
    start: Act,
    hold: Act,
    resume: Act,
    complete: Act,
    close: Act,
    cancel: Act,
    unlink: Act,
    abort: Act,
    issue: Act,
    report: Act,
}

/// A material issued, or waiting as a pending issue.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct IssueLine {
    item: String,
    lot: String,
    quantity: String,
    returned: String,
    cost: String,
    waiting: bool,
}

/// A line of the pick list.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct PickView {
    item: String,
    required: String,
}

/// A production report.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ReportLine {
    id: i64,
    number: String,
    at: String,
    good: String,
    scrap: String,
    by: String,
    note: String,
    voided: bool,
    status: String,
    status_kind: &'static str,
    reason: String,
    waits: bool,
    can_void: bool,
    reversal_open: bool,
}

/// What the abort did with a part of the work in progress.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct OutcomeLine {
    what: String,
    item: String,
    quantity: String,
    value: String,
}

/// An approval request of the work order.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ApprovalLine {
    id: i64,
    title: String,
    status: String,
    status_kind: &'static str,
    requester: String,
    requested: String,
    decided: String,
    can_decide: bool,
    note: String,
}

/// A change of state.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ActivityLine {
    at: String,
    what: String,
    by: String,
    reason: String,
}

/// A change `MFG.RUL.09` made because the sales order changed.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ChangeLine {
    at: String,
    what: String,
    detail: String,
}

/// A material that can go back to the store in the *Abort* sheet.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ReturnRow {
    name: String,
    label: String,
    hint: String,
}

/// What the record page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/work_order.html")]
pub(crate) struct WorkOrderPage {
    tx: Texts,
    id: i64,
    number: String,
    subtitle: String,
    status: String,
    status_key: String,
    status_kind: &'static str,
    facts: Vec<Fact>,
    costs: Vec<Fact>,
    notes: Vec<String>,
    actions: Actions,
    hold_reasons: Vec<(String, String)>,
    hold_selected: String,
    returns: Vec<ReturnRow>,
    show_costs: bool,
    pick: Vec<PickView>,
    issues: Vec<IssueLine>,
    reports: Vec<ReportLine>,
    outcome: Vec<OutcomeLine>,
    approvals: Vec<ApprovalLine>,
    activity: Vec<ActivityLine>,
    changes: Vec<ChangeLine>,
    abort_prefill: String,
}

fn approval_kind(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Approved => "success",
        ApprovalStatus::Pending => "warning",
        ApprovalStatus::Rejected | ApprovalStatus::Failed => "error",
        _ => "info",
    }
}

/// The reasons the actions of `order` are blocked now, for the person.
struct Gate {
    waiting: bool,
    release_problems: Vec<String>,
}

fn actions(user: &AuthUser, lang: &Lang, order: &WorkOrder, gate: &Gate) -> Actions {
    use WorkOrderStatus::{Completed, InProgress, OnHold, Planned, Released};
    let status = order.status;
    let may = |permission: &str| order.may(user, permission);
    let pending_reason = || t(lang, "manufacturing.abort.pending");
    // While an abort waits nothing else runs on the work order.
    let guarded = |act: Act| {
        if gate.waiting && act.shown && act.blocked.is_empty() {
            Act::blocked(pending_reason())
        } else {
            act
        }
    };
    let release = if status == Planned && may(permissions::WORK_ORDERS_RELEASE) {
        Act::unless((!gate.release_problems.is_empty()).then(|| gate.release_problems.join(" ")))
    } else {
        Act::hidden()
    };
    let report_state = |from: &[WorkOrderStatus]| {
        if from.contains(&status) && may(permissions::WORK_ORDERS_REPORT) {
            Act::ready()
        } else {
            Act::hidden()
        }
    };
    let complete = if status == InProgress && may(permissions::WORK_ORDERS_REPORT) {
        Act::unless(
            (order.good_milli <= 0).then(|| t(lang, "manufacturing.complete.nothing_reported")),
        )
    } else {
        Act::hidden()
    };
    let close = if status == Completed && may(permissions::WORK_ORDERS_CLOSE) {
        Act::ready()
    } else {
        Act::hidden()
    };
    let cancel = if status.is_open() && may(permissions::WORK_ORDERS_RELEASE) {
        Act::unless(
            order
                .has_work_in_progress()
                .then(|| t(lang, "manufacturing.cancel.work_started")),
        )
    } else {
        Act::hidden()
    };
    let unlink_act =
        if may(permissions::WORK_ORDERS_RELEASE) && unlink::not_offered(order).is_none() {
            Act::ready()
        } else {
            Act::hidden()
        };
    let abort_act = if may(permissions::WORK_ORDERS_RELEASE) && abort::not_offered(order).is_none()
    {
        Act::ready()
    } else {
        Act::hidden()
    };
    let issue = if takes_material(status) && may(permissions::WORK_ORDERS_ISSUE) {
        Act::ready()
    } else {
        Act::hidden()
    };
    Actions {
        release: guarded(release),
        start: guarded(report_state(&[Released])),
        hold: guarded(report_state(&[InProgress])),
        resume: guarded(report_state(&[OnHold])),
        complete: guarded(complete),
        close: guarded(close),
        cancel: guarded(cancel),
        unlink: guarded(unlink_act),
        abort: if gate.waiting && abort_act.shown {
            Act::blocked(pending_reason())
        } else {
            abort_act
        },
        issue: guarded(issue),
        report: guarded(
            if status == InProgress && may(permissions::WORK_ORDERS_REPORT) {
                Act::ready()
            } else {
                Act::hidden()
            },
        ),
    }
}

/// `GET /manufacturing/work-orders/{id}`: the page for a browser, the same
/// data as JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if !order.may_see(&user) {
        return Err(Error::Forbidden);
    }
    let page = build(&state, &user, &lang, &order).await?;
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(page).into_response());
    }
    let mut data = renox::serde_json::to_value(&page)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("tx");
    }
    Ok(Json(data).into_response())
}

async fn build(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    order: &WorkOrder,
) -> Result<WorkOrderPage> {
    let not_set = t(lang, "manufacturing.screen.not_set");
    let show_costs = order.may(user, permissions::WORK_ORDERS_COST_VIEW);
    let line = match order.line_id {
        Some(line) => Line::find(&state.db, line).await?,
        None => None,
    };
    let waiting = abort::is_pending(state, order).await?;
    let release_problems = if order.status == WorkOrderStatus::Planned {
        crate::work_orders::release_problems(state, order).await?
    } else {
        Vec::new()
    };
    let gate = Gate {
        waiting,
        release_problems,
    };

    let mut facts = vec![
        Fact {
            label: t(lang, "manufacturing.screen.fact.item"),
            value: item_text(lang, order.item_id),
        },
        Fact {
            label: t(lang, "manufacturing.screen.fact.line"),
            value: line.as_ref().map_or_else(
                || t(lang, "manufacturing.screen.no_line"),
                |l| format!("{} - {}", l.code, l.name),
            ),
        },
        Fact {
            label: t(lang, "manufacturing.screen.fact.quantity"),
            value: tf(
                lang,
                "manufacturing.screen.fact.quantity_of",
                &[
                    ("made", &quantity_text(order.good_milli)),
                    ("quantity", &quantity_text(order.quantity_milli)),
                ],
            ),
        },
        Fact {
            label: t(lang, "manufacturing.screen.fact.scrap"),
            value: quantity_text(order.scrap_milli),
        },
        Fact {
            label: t(lang, "manufacturing.screen.fact.source"),
            value: source_text(state, lang, order).await?,
        },
        Fact {
            label: t(lang, "manufacturing.screen.fact.planned"),
            value: period_text(lang, order.planned_start, order.planned_end),
        },
    ];
    if let Some(at) = order.started_at {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.started"),
            value: time_text(at),
        });
    }
    if let Some(at) = order.stopped_at {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.stopped"),
            value: time_text(at),
        });
    }
    if let Some(reason) = order.hold_reason.as_deref() {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.hold_reason"),
            value: word(lang, "hold_reason", reason),
        });
    }
    if order.quantity_for_stock_milli > 0 {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.for_stock"),
            value: quantity_text(order.quantity_for_stock_milli),
        });
    }
    if let Some(lot) = order.output_lot_id {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.output_lot"),
            value: tf(lang, "manufacturing.screen.fact.lot_name", &[("id", &lot)]),
        });
    }
    if let Some(reason) = order.unlink_reason.as_deref() {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.unlinked"),
            value: reason.to_owned(),
        });
    }
    if let Some(reason) = order.cancel_reason.as_deref() {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.cancelled"),
            value: reason.to_owned(),
        });
    }
    if let Some(reason) = order.abort_reason.as_deref() {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.aborted"),
            value: reason.to_owned(),
        });
    }

    let mut costs = Vec::new();
    if show_costs {
        costs.push(Fact {
            label: t(lang, "manufacturing.screen.cost.standard"),
            value: super::aed(order.std_unit_cost),
        });
        if let Some(actual) = order.actual_cost {
            costs.push(Fact {
                label: t(lang, "manufacturing.screen.cost.actual"),
                value: super::aed(actual),
            });
        }
        if let Some(scrap) = order.abort_scrap_value {
            costs.push(Fact {
                label: t(lang, "manufacturing.screen.cost.scrap"),
                value: super::aed(scrap),
            });
        }
        if let Some(loss) = order.abort_loss {
            costs.push(Fact {
                label: t(lang, "manufacturing.screen.cost.loss"),
                value: super::aed(loss),
            });
        }
    }

    // Pick list, issues.
    let pick = pick_list::of_order(&state.db, order.id)
        .await?
        .into_iter()
        .filter(|line| !line.withdrawn)
        .map(|line| PickView {
            item: item_text(lang, line.item_id),
            required: quantity_text(line.required_milli),
        })
        .collect();
    let issued = MaterialIssue::of_order(&state.db, order.id).await?;
    let issues: Vec<IssueLine> = issued
        .iter()
        .map(|issue| IssueLine {
            item: item_text(lang, issue.item_id),
            lot: issue.lot_id.map_or_else(
                || not_set.clone(),
                |lot| tf(lang, "manufacturing.screen.fact.lot_name", &[("id", &lot)]),
            ),
            quantity: quantity_text(issue.quantity_milli),
            returned: quantity_text(issue.returned_milli),
            cost: if show_costs {
                super::aed(cost::value_of(issue.quantity_milli, issue.unit_cost))
            } else {
                String::new()
            },
            waiting: issue.pending_issue_id.is_some(),
        })
        .collect();

    // What can go back to the store in the abort sheet.
    let mut returns = Vec::new();
    let mut seen: Vec<(i64, Option<i64>)> = Vec::new();
    for issue in issued.iter().filter(|i| i.pending_issue_id.is_none()) {
        let key = (issue.item_id, issue.lot_id);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let left = cost::issued_quantity(&issued, key.0, key.1)
            - cost::returned_quantity(&issued, key.0, key.1);
        returns.push(ReturnRow {
            name: format!("return_{}_{}", key.0, key.1.unwrap_or(0)),
            label: tf(
                lang,
                "manufacturing.screen.abort.return_label",
                &[("item", &item_text(lang, key.0))],
            ),
            hint: tf(
                lang,
                "manufacturing.screen.abort.return_hint",
                &[("left", &quantity_text(left))],
            ),
        });
    }

    let mut reports = Vec::new();
    for report in ProductionReport::query()
        .where_(ProductionReport::WORK_ORDER_ID.eq(order.id))
        .order_by(ProductionReport::ID)
        .get(&state.db)
        .await?
    {
        let voided = report.status == ReportStatus::Voided;
        let waits = !voided && crate::report::waiting(state, &report).await?;
        let reason = if voided {
            VoidRecord::of_subject(&state.db, &report.subject())
                .await?
                .map(|void| void.reason)
                .unwrap_or_default()
        } else {
            String::new()
        };
        reports.push(ReportLine {
            id: report.id,
            number: report.number(),
            at: time_text(report.reported_at),
            good: quantity_text(report.good_milli),
            scrap: quantity_text(report.scrap_milli),
            by: user_text(state, report.operator_id, &not_set).await,
            note: report.note.clone().unwrap_or_default(),
            voided,
            status: if voided {
                t(lang, "manufacturing.screen.output.voided")
            } else if waits {
                t(lang, "manufacturing.screen.output.void_waits")
            } else {
                word(lang, "status", "posted")
            },
            status_kind: if voided {
                "info"
            } else if waits {
                "warning"
            } else {
                "success"
            },
            reason,
            waits,
            // Void is not offered once the work order is completed, closed or
            // aborted (`MFG.ACL.10`), nor while an abort or a void waits.
            can_void: !voided
                && !waits
                && !gate.waiting
                && order.status.is_open()
                && user.has_permission_in(permissions::WORK_ORDERS_REPORT, &report.scope()),
            reversal_open: voided && report.stock_reversed_at.is_none(),
        });
    }

    let mut outcome = Vec::new();
    for line in AbortLine::query()
        .where_(AbortLine::WORK_ORDER_ID.eq(order.id))
        .order_by(AbortLine::ID)
        .get(&state.db)
        .await?
    {
        outcome.push(OutcomeLine {
            what: word(lang, "outcome", &line.kind),
            item: line
                .item_id
                .map_or_else(|| not_set.clone(), |item| item_text(lang, item)),
            quantity: quantity_text(line.quantity_milli),
            value: if show_costs {
                super::aed(line.value)
            } else {
                String::new()
            },
        });
    }

    // Approvals, newest first.
    let mut approvals = Vec::new();
    let mut notes = Vec::new();
    if issues.iter().any(|issue| issue.waiting) {
        notes.push(t(lang, "manufacturing.screen.materials.short_note"));
    }
    let mut requests = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(order.subject().subject_type.as_str()))
        .where_(ApprovalRequest::SUBJECT_ID.eq(order.id))
        .order_by_desc(ApprovalRequest::ID)
        .get(&state.db)
        .await?;
    // The voids of its production reports wait in the same list.
    for line in &reports {
        requests.extend(
            ApprovalRequest::query()
                .where_(ApprovalRequest::SUBJECT_TYPE.eq(crate::report::SUBJECT_TYPE))
                .where_(ApprovalRequest::SUBJECT_ID.eq(line.id))
                .get(&state.db)
                .await?,
        );
    }
    requests.sort_by_key(|request| std::cmp::Reverse(request.id));
    for request in requests {
        let mut title = action_label(lang, &request.action);
        if request.subject_type == crate::report::SUBJECT_TYPE
            && let Some(line) = reports.iter().find(|line| line.id == request.subject_id)
        {
            title = format!("{title} {}", line.number);
        }
        let is_waiting = request.is_pending();
        if is_waiting {
            notes.push(tf(
                lang,
                "manufacturing.screen.approval.waiting_for",
                &[("action", &title)],
            ));
        }
        approvals.push(ApprovalLine {
            id: request.id,
            title,
            status: if is_waiting {
                t(lang, "manufacturing.screen.approval.waiting")
            } else {
                word(lang, "approval", &request.status.to_string())
            },
            status_kind: approval_kind(request.status),
            requester: user_text(state, Some(request.requester_id), &not_set).await,
            requested: request.created_at.map_or_else(
                || not_set.clone(),
                |at| date_text(wl_kernel::documents::dubai_date(at)),
            ),
            decided: request.decided_at.map_or_else(
                || not_set.clone(),
                |at| date_text(wl_kernel::documents::dubai_date(at)),
            ),
            note: if is_waiting {
                String::new()
            } else {
                request
                    .decisions(&state.db)
                    .await?
                    .into_iter()
                    .rev()
                    .find_map(|decision| decision.comment.filter(|note| !note.trim().is_empty()))
                    .unwrap_or_default()
            },
            can_decide: is_waiting
                && decide::ensure_approver(&state.db, &request, user)
                    .await
                    .is_ok(),
        });
    }

    let mut activity = Vec::new();
    for row in renox::state_machine::history(&state.db, WorkOrder::TABLE, order.id).await? {
        activity.push(ActivityLine {
            at: time_text(row.at),
            what: transition_text(lang, &row.transition),
            by: match row.actor_id {
                Some(id) => user_text(state, Some(id), &not_set).await,
                None => t(lang, "manufacturing.screen.activity.system"),
            },
            reason: row.reason.unwrap_or_default(),
        });
    }

    // What the sales order changed (`MFG.RUL.09`), and the reason the abort
    // sheet is filled with when the production manager was asked for it.
    let mut changes = Vec::new();
    let mut abort_prefill = String::new();
    for row in Adjustment::of_work_order(&state.db, order.id).await? {
        if row.action == order_changes::action::ABORT_TASK {
            let number = wl_kernel::production::order_lines(state)
                .line(
                    state,
                    wl_kernel::sales::SalesOrderId(row.order_id),
                    row.line_id,
                )
                .await?
                .and_then(|info| info.number)
                .unwrap_or_else(|| row.order_id.to_string());
            abort_prefill = order_changes::abort_reason(&number, &row.note);
            if order.status.is_open() {
                notes.push(tf(
                    lang,
                    "manufacturing.screen.change.abort_note",
                    &[("reason", &abort_prefill)],
                ));
            }
        }
        changes.push(ChangeLine {
            at: row.created_at.map(time_text).unwrap_or_default(),
            what: word(lang, "change", &row.action),
            detail: tf(
                lang,
                &format!("manufacturing.screen.change.detail.{}", row.action),
                &[
                    ("from", &quantity_text(row.from_milli)),
                    ("to", &quantity_text(row.to_milli)),
                    ("note", &row.note),
                ],
            ),
        });
    }

    let status = order.status;
    Ok(WorkOrderPage {
        tx: screen_texts(lang),
        id: order.id,
        number: order.number.clone(),
        subtitle: item_text(lang, order.item_id),
        status: word(lang, "status", status.as_str()),
        status_key: status.as_str().to_owned(),
        status_kind: super::work_orders_page::status_kind(status),
        facts,
        costs,
        notes,
        actions: actions(user, lang, order, &gate),
        hold_reasons: super::options(lang, "hold_reason", HOLD_REASONS),
        hold_selected: "material".to_owned(),
        returns,
        show_costs,
        pick,
        issues,
        reports,
        outcome,
        approvals,
        activity,
        changes,
        abort_prefill,
    })
}
