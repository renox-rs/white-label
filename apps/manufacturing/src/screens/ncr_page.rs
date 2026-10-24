//! The record page of an NCR (`GET /manufacturing/ncrs/{id}`,
//! `docs/UX.md` §2.2).
//!
//! The page shows the NCR with its status, the facts (item, lot, quantity,
//! defect, cause, supplier, disposition and what became of it), the actions
//! for the people who may run them (a guard that fails now draws the button
//! disabled and says why in words), and the tabs *Overview*,
//! *Dispositions* (every one the NCR had, a reopened NCR keeps the earlier
//! one), *Approvals* and *Activity*.
//!
//! Decisions where the spec is silent:
//!
//! - **Which dispositions are drawn.** The four dispositions are drawn for a
//!   holder of `ncrs.disposition` while the NCR is `open`; *return to
//!   supplier* is not drawn for a lot made in-house and is disabled, saying
//!   why, for a lot with no supplier lot; scrap, use as is and return are
//!   disabled for an NCR that is not on a lot. While a decision waits for its
//!   approval every disposition is disabled.
//! - **The value of a scrap** shows to the holders of `ncrs.disposition`.
//! - The item shows as `Item 9` and the lot by the number on the pack until
//!   the page can read the item catalogue through a kernel contract.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, awaiting::action_label, decide};
use wl_kernel::lot_disposal::lot_disposal;

use super::ncrs_page::{item_text, lot_text, status_kind};
use super::{
    Act, Texts, aed, date_text, options, party_text, quantity_text, time_text, transition_text,
    user_text,
};
use crate::ncr::{Cause, Disposition, Ncr, NcrDecision, NcrStatus};
use crate::ncrs;
use crate::permissions;
use crate::plant::Line;
use crate::texts::{screen_texts, t, tf, word};
use crate::work_order::WorkOrder;

/// The address of an NCR's page.
pub(crate) fn url(id: i64) -> String {
    format!("/manufacturing/ncrs/{id}")
}

/// A fact of the overview.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Fact {
    label: String,
    value: String,
}

/// What the person may do with the NCR now.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Actions {
    rework: Act,
    scrap: Act,
    use_as_is: Act,
    return_to_supplier: Act,
    close: Act,
    reopen: Act,
    cancel: Act,
    cause: Act,
}

/// A disposition the NCR had.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct DecisionLine {
    what: String,
    outcome: String,
    quantity: String,
    by: String,
    at: String,
    ended: String,
    in_force: bool,
}

/// An approval request of the NCR.
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

/// What the record page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/ncr.html")]
pub(crate) struct NcrPage {
    tx: Texts,
    id: i64,
    number: String,
    subtitle: String,
    status: String,
    status_key: String,
    status_kind: &'static str,
    facts: Vec<Fact>,
    notes: Vec<String>,
    actions: Actions,
    outcomes: Vec<(String, String)>,
    outcome_selected: String,
    lines: Vec<(String, String)>,
    line_selected: String,
    causes: Vec<(String, String)>,
    cause_selected: String,
    decisions: Vec<DecisionLine>,
    approvals: Vec<ApprovalLine>,
    activity: Vec<ActivityLine>,
    rework_url: String,
    return_url: String,
}

fn approval_kind(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Approved => "success",
        ApprovalStatus::Pending => "warning",
        ApprovalStatus::Rejected | ApprovalStatus::Failed => "error",
        _ => "info",
    }
}

/// What the actions of `ncr` are blocked by now.
struct Gate {
    waiting: bool,
    needs_lot: bool,
    own_production: bool,
    no_supplier_lot: bool,
    action_done: bool,
    return_alive: bool,
    rework_alive: bool,
}

fn actions(user: &AuthUser, lang: &Lang, ncr: &Ncr, gate: &Gate) -> Actions {
    let may = |permission: &str| ncr.may(user, permission);
    let open = ncr.status == NcrStatus::Open;
    let waiting_reason = || t(lang, "manufacturing.ncr.waiting");
    let lot_reason = || t(lang, "manufacturing.ncr.needs_lot");
    let dispose = |needs_lot: bool| {
        if !(open && may(permissions::NCRS_DISPOSITION)) {
            Act::hidden()
        } else if gate.waiting {
            Act::blocked(waiting_reason())
        } else if needs_lot && gate.needs_lot {
            Act::blocked(lot_reason())
        } else {
            Act::ready()
        }
    };
    let return_to_supplier = if gate.own_production {
        Act::hidden()
    } else if !gate.needs_lot && gate.no_supplier_lot && open && may(permissions::NCRS_DISPOSITION)
    {
        Act::blocked(t(lang, "manufacturing.ncr.no_supplier_lot"))
    } else {
        dispose(true)
    };
    let dispositioned = ncr.status == NcrStatus::Dispositioned;
    let close = if dispositioned && may(permissions::NCRS_DISPOSITION) {
        Act::unless((!gate.action_done).then(|| t(lang, "manufacturing.ncr.close_not_done")))
    } else {
        Act::hidden()
    };
    let reopen = if dispositioned && !gate.action_done && may(permissions::NCRS_DISPOSITION) {
        if gate.return_alive {
            Act::blocked(t(lang, "manufacturing.ncr.return_alive"))
        } else if gate.rework_alive {
            Act::blocked(t(lang, "manufacturing.ncr.rework_alive"))
        } else {
            Act::ready()
        }
    } else {
        Act::hidden()
    };
    let cancel = if open && may(permissions::NCRS_DISPOSITION) {
        Act::unless(gate.waiting.then(waiting_reason))
    } else {
        Act::hidden()
    };
    let cause = if matches!(ncr.status, NcrStatus::Open | NcrStatus::Dispositioned)
        && may(permissions::NCRS_RAISE)
    {
        Act::ready()
    } else {
        Act::hidden()
    };
    Actions {
        rework: dispose(false),
        scrap: dispose(true),
        use_as_is: dispose(true),
        return_to_supplier,
        close,
        reopen,
        cancel,
        cause,
    }
}

/// `GET /manufacturing/ncrs/{id}`: the page for a browser, the same data as
/// JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let ncr = Ncr::find_or_404(&state.db, id).await?;
    if !super::ncrs_page::may_see(&user, &ncr) {
        return Err(Error::Forbidden);
    }
    let page = build(&state, &user, &lang, &ncr).await?;
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(page).into_response());
    }
    let mut data = renox::serde_json::to_value(&page)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("tx");
    }
    Ok(Json(data).into_response())
}

async fn build(state: &AppState, user: &AuthUser, lang: &Lang, ncr: &Ncr) -> Result<NcrPage> {
    let not_set = t(lang, "manufacturing.screen.not_set");
    let facts_of_lot = match ncr.lot() {
        Some(lot) => lot_disposal(state).facts(state, &lot).await?,
        None => None,
    };
    let waiting = ncrs::waiting(state, ncr).await?;
    let action_done = ncrs::action_done(state, ncr).await?;
    let return_alive = match ncr.supplier_return() {
        Some(found) => lot_disposal(state)
            .return_status(state, &found)
            .await?
            .is_some_and(|status| status != "cancelled"),
        None => false,
    };
    let rework_order = match ncr.rework_work_order_id {
        Some(id) => WorkOrder::find(&state.db, id).await?,
        None => None,
    };
    let rework_alive = rework_order.as_ref().is_some_and(|order| {
        !matches!(
            order.status,
            crate::work_order::WorkOrderStatus::Cancelled
                | crate::work_order::WorkOrderStatus::Aborted
        )
    });
    let gate = Gate {
        waiting,
        needs_lot: ncr.lot_id.is_none(),
        own_production: facts_of_lot
            .as_ref()
            .is_some_and(|facts| facts.produced_by.is_some()),
        no_supplier_lot: facts_of_lot
            .as_ref()
            .is_some_and(|facts| facts.supplier.is_none() || facts.supplier_lot.is_none()),
        action_done,
        return_alive,
        rework_alive,
    };
    let may_decide = ncr.may(user, permissions::NCRS_DISPOSITION);

    let mut facts = vec![
        Fact {
            label: t(lang, "manufacturing.screen.fact.item"),
            value: item_text(state, lang, ncr.item_id).await,
        },
        Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.lot"),
            value: lot_text(state, lang, ncr).await,
        },
        Fact {
            label: t(lang, "manufacturing.screen.fact.quantity"),
            value: quantity_text(ncr.quantity_milli),
        },
        Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.defect"),
            value: ncr.defect.clone(),
        },
        Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.cause"),
            value: word(lang, "cause", ncr.cause.as_str()),
        },
        Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.origin"),
            value: word(lang, "origin", ncr.origin.as_str()),
        },
        Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.raised"),
            value: tf(
                lang,
                "manufacturing.screen.ncr.fact.raised_by",
                &[
                    ("date", &date_text(ncr.raised_on)),
                    (
                        "who",
                        &user_text(
                            state,
                            ncr.raised_by,
                            &t(lang, "manufacturing.screen.activity.system"),
                        )
                        .await,
                    ),
                ],
            ),
        },
    ];
    if let Some(reason) = ncr.cause_reason.as_deref() {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.cause_reason"),
            value: reason.to_owned(),
        });
    }
    if let Some(supplier) = ncr.supplier_party_id {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.supplier"),
            value: party_text(state, lang, Some(supplier), &not_set).await,
        });
    }
    if let Some(kind) = ncr.disposition {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.disposition"),
            value: word(lang, "disposition", kind.as_str()),
        });
        if let Some(outcome) = ncr.outcome.as_deref() {
            facts.push(Fact {
                label: t(lang, "manufacturing.screen.ncr.fact.outcome"),
                value: word(lang, "return_outcome", outcome),
            });
        }
        if let Some(note) = ncr.disposition_note.as_deref() {
            facts.push(Fact {
                label: t(lang, "manufacturing.screen.ncr.fact.note"),
                value: note.to_owned(),
            });
        }
        if kind == Disposition::Scrap && may_decide {
            facts.push(Fact {
                label: t(lang, "manufacturing.screen.ncr.fact.scrap_value"),
                value: aed(ncr.scrap_value),
            });
        }
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.action"),
            value: if action_done {
                t(lang, "manufacturing.screen.ncr.action.done")
            } else {
                t(lang, "manufacturing.screen.ncr.action.pending")
            },
        });
    }
    if let Some(reason) = ncr.cancel_reason.as_deref() {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.fact.cancelled"),
            value: reason.to_owned(),
        });
    }
    let (rework_url, return_url) = (
        rework_order
            .as_ref()
            .map(|order| super::work_order_page::url(order.id))
            .unwrap_or_default(),
        ncr.return_id
            .map(|id| format!("/supplier-returns/{id}"))
            .unwrap_or_default(),
    );
    if let Some(order) = &rework_order {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.rework"),
            value: order.number.clone(),
        });
    }
    if let Some(id) = ncr.return_id {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.return"),
            value: tf(
                lang,
                "manufacturing.screen.ncr.fact.return_name",
                &[("id", &id)],
            ),
        });
    }

    for link in crate::disputes::links_of(&state.db, ncr.id).await? {
        facts.push(Fact {
            label: t(lang, "manufacturing.screen.ncr.fact.dispute"),
            value: tf(
                lang,
                "manufacturing.screen.ncr.fact.dispute_on",
                &[("dispute", &link.dispute_id), ("invoice", &link.invoice_id)],
            ),
        });
        if let Some(text) = link.text.as_deref() {
            facts.push(Fact {
                label: t(lang, "manufacturing.screen.ncr.fact.customer_says"),
                value: text.to_owned(),
            });
        }
    }

    let mut notes = Vec::new();
    if waiting {
        notes.push(t(lang, "manufacturing.screen.ncr.note.waiting"));
    }
    if ncr.keeps_lot()
        && ncr.lot_id.is_some()
        && facts_of_lot
            .as_ref()
            .is_some_and(|facts| facts.quantity_milli > ncr.quantity_milli)
    {
        notes.push(t(lang, "manufacturing.screen.ncr.note.partial"));
    }
    if ncr.status == NcrStatus::Open && ncr.disposition.is_none() {
        let reopened = !ncrs::decisions(&state.db, ncr.id).await?.is_empty();
        if reopened {
            notes.push(t(lang, "manufacturing.screen.ncr.note.reopened"));
        }
    }

    let mut decisions = Vec::new();
    for decision in ncrs::decisions(&state.db, ncr.id).await? {
        decisions.push(decision_line(state, lang, &decision, &not_set).await);
    }

    let requests = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(ncr.subject().subject_type.as_str()))
        .where_(ApprovalRequest::SUBJECT_ID.eq(ncr.id))
        .order_by_desc(ApprovalRequest::ID)
        .get(&state.db)
        .await?;
    let mut approvals = Vec::new();
    for request in requests {
        let title = action_label(lang, &request.action);
        let is_waiting = request.is_pending();
        // `MFG.ACL.03`: the inspector who raised the NCR does not approve it.
        let may_approve = is_waiting
            && decide::ensure_approver(&state.db, &request, user)
                .await
                .is_ok()
            && ncr.raised_by != Some(user.id);
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
            can_decide: may_approve,
        });
    }

    let mut activity = Vec::new();
    for row in renox::state_machine::history(&state.db, Ncr::TABLE, ncr.id).await? {
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

    let mut lines = vec![(
        String::new(),
        t(lang, "manufacturing.screen.ncr.rework.no_line"),
    )];
    for line in Line::query()
        .where_(Line::PLANT_ID.eq(ncr.plant_id))
        .order_by(Line::ID)
        .get(&state.db)
        .await?
    {
        if line.active {
            lines.push((
                line.id.to_string(),
                format!("{} - {}", line.code, line.name),
            ));
        }
    }
    let status = ncr.status;
    Ok(NcrPage {
        tx: screen_texts(lang),
        id: ncr.id,
        number: ncr.number.clone(),
        subtitle: item_text(state, lang, ncr.item_id).await,
        status: word(lang, "ncr_status", status.as_str()),
        status_key: status.as_str().to_owned(),
        status_kind: status_kind(status),
        facts,
        notes,
        actions: actions(user, lang, ncr, &gate),
        outcomes: options(lang, "return_outcome", &["credit", "replace"]),
        outcome_selected: "credit".to_owned(),
        lines,
        line_selected: String::new(),
        causes: options(
            lang,
            "cause",
            &[
                Cause::Supplier.as_str(),
                Cause::InternalProcess.as_str(),
                Cause::Unknown.as_str(),
            ],
        ),
        cause_selected: ncr.cause.as_str().to_owned(),
        decisions,
        approvals,
        activity,
        rework_url,
        return_url,
    })
}

async fn decision_line(
    state: &AppState,
    lang: &Lang,
    decision: &NcrDecision,
    not_set: &str,
) -> DecisionLine {
    DecisionLine {
        what: word(lang, "disposition", decision.disposition.as_str()),
        outcome: decision
            .outcome
            .as_deref()
            .map_or_else(String::new, |o| word(lang, "return_outcome", o)),
        quantity: quantity_text(decision.quantity_milli),
        by: user_text(state, decision.decided_by, not_set).await,
        at: time_text(decision.decided_at),
        ended: match (decision.ended_at, decision.ended_reason.as_deref()) {
            (Some(at), Some(reason)) => format!("{} - {reason}", time_text(at)),
            (Some(at), None) => time_text(at),
            _ => String::new(),
        },
        in_force: decision.ended_at.is_none(),
    }
}
