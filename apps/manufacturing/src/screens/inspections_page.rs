//! The inspections of the returned lots (`GET /manufacturing/inspections`,
//! BL-MFG-07, `MFG.RUL.10`, `MFG.ACL.02`): per lot that came back into
//! quarantine the quantity, the reason, the order and dispatch it came from,
//! where it stands and, for QC, the result: *Pass* (the lot is released; the
//! storekeeper then moves the goods to a store) or *Fail* (an NCR is raised
//! on the lot). The page is what the QC inspector works from on a phone: one
//! card per lot, the actions as sheets. `?status=open|passed|failed`
//! filters.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::lot_disposal::lot_disposal;

use super::{Act, Texts, date_text, done, quantity_text, time_text};
use crate::late_inspection::{self, Decision, Made};
use crate::permissions;
use crate::returns::{self, ReturnInspection, inspection};
use crate::texts::{screen_texts, t, tf, word};

/// The address of the page.
pub(crate) const URL: &str = "/manufacturing/inspections";

/// The page's filter.
#[derive(Deserialize, Default)]
pub(crate) struct Filter {
    /// `open`, `passed`, `failed`, or empty for all.
    #[serde(default)]
    status: String,
}

/// An inspection of the list.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    id: i64,
    lot: String,
    item: String,
    quantity: String,
    reason: String,
    dispatch: String,
    dispatch_url: String,
    order_url: String,
    came_back: String,
    complaint_url: String,
    ncr_url: String,
    status: String,
    status_kind: &'static str,
    decided: String,
    note: String,
    late: String,
    waits: String,
    record: Act,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/inspections.html")]
pub(crate) struct InspectionsPage {
    tx: Texts,
    rows: Vec<Row>,
    status_tabs: Vec<(String, String)>,
    status_current: String,
}

fn tabs(lang: &Lang) -> Vec<(String, String)> {
    let mut tabs = vec![(URL.to_owned(), t(lang, "manufacturing.screen.filter.all"))];
    for status in [inspection::OPEN, inspection::PASSED, inspection::FAILED] {
        tabs.push((
            format!("{URL}?status={status}"),
            word(lang, "inspection_status", status),
        ));
    }
    tabs
}

fn decision_word(lang: &Lang, decision: &str) -> String {
    t(
        lang,
        &format!(
            "manufacturing.screen.inspections.{}",
            if decision == "fail" { "fail" } else { "pass" }
        ),
    )
    .to_lowercase()
}

fn allows(user: &AuthUser, permission: &str) -> bool {
    user.has_permission(permission) || user.has_permission_anywhere(permission)
}

/// `GET /manufacturing/inspections`.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    let may_record = allows(&user, permissions::INSPECTIONS_RECORD);
    if !may_record && !allows(&user, permissions::INSPECTIONS_VIEW) {
        return Err(Error::Forbidden);
    }
    let mut rows = Vec::new();
    for found in ReturnInspection::query()
        .order_by_desc(ReturnInspection::ID)
        .get(&state.db)
        .await?
    {
        if !filter.status.is_empty() && found.status != filter.status {
            continue;
        }
        let lot = match found.lot() {
            Some(subject) => lot_disposal(&state)
                .facts(&state, &subject)
                .await?
                .map(|facts| facts.number)
                .unwrap_or_else(|| subject.subject_id.to_string()),
            None => String::new(),
        };
        let lot = if lot.is_empty() {
            t(&lang, "manufacturing.screen.not_set")
        } else {
            tf(
                &lang,
                "manufacturing.screen.ncr.lot_name",
                &[("number", &lot)],
            )
        };
        let open = found.is_open();
        let waiting = if open {
            late_inspection::waiting(&state, found.id).await?
        } else {
            None
        };
        let waits = match &waiting {
            Some(entry) => tf(
                &lang,
                "manufacturing.screen.inspections.waits",
                &[
                    (
                        "decision",
                        &decision_word(&lang, entry.payload.0["decision"].as_str().unwrap_or("")),
                    ),
                    (
                        "at",
                        &format!(
                            "{} {}",
                            date_text(entry.occurred_at.date_naive()),
                            time_text(entry.occurred_at)
                        ),
                    ),
                ],
            ),
            None => String::new(),
        };
        let late = match late_inspection::flagged(&state, found.id).await? {
            Some(entry) => tf(
                &lang,
                "manufacturing.screen.inspections.late",
                &[("reason", &entry.late_reason.unwrap_or_default())],
            ),
            None => String::new(),
        };
        rows.push(Row {
            id: found.id,
            lot,
            item: super::names::item_text(&state, &lang, found.item_id).await,
            quantity: quantity_text(found.quantity_milli),
            reason: found.reason.clone().unwrap_or_default(),
            dispatch: found.dispatch_number.clone(),
            dispatch_url: format!("/dispatches/{}", found.dispatch_id),
            order_url: format!("/sales-orders/{}", found.order_id),
            came_back: if found.returned {
                t(&lang, "manufacturing.screen.inspections.whole")
            } else {
                t(&lang, "manufacturing.screen.inspections.refused")
            },
            complaint_url: found
                .complaint_id
                .map(|id| wl_kernel::tickets::ticket_path(wl_kernel::tickets::TicketId(id)))
                .unwrap_or_default(),
            ncr_url: found.ncr_id.map(super::ncr_page::url).unwrap_or_default(),
            status: word(&lang, "inspection_status", &found.status),
            status_kind: match found.status.as_str() {
                inspection::OPEN => "warning",
                inspection::PASSED => "success",
                _ => "error",
            },
            decided: found
                .decided_at
                .map(|at| format!("{} {}", date_text(at.date_naive()), time_text(at)))
                .unwrap_or_default(),
            note: found.note.clone().unwrap_or_default(),
            late,
            record: if !(open && may_record) {
                Act::hidden()
            } else if waiting.is_some() {
                Act::blocked(t(&lang, "manufacturing.screen.inspections.blocked_waits"))
            } else {
                Act::ready()
            },
            waits,
        });
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
    let current = match filter.status.as_str() {
        "" => URL.to_owned(),
        key => format!("{URL}?status={key}"),
    };
    Ok(View::typed(InspectionsPage {
        tx: screen_texts(&lang),
        rows,
        status_tabs: tabs(&lang),
        status_current: current,
    })
    .into_response())
}

/// The note or defect field of a form.
fn text_of(form: &std::collections::BTreeMap<String, String>, name: &str) -> String {
    form.get(name)
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

/// What the late-entry fields of the sheet say: the time the decision was
/// made (Asia/Dubai wall time), the reason and the outage reference.
struct Late {
    at: renox::db::DateTime,
    reason: String,
    outage: String,
}

/// The late-entry fields of a form, when the decision is entered later.
fn late_of(lang: &Lang, form: &std::collections::BTreeMap<String, String>) -> Result<Option<Late>> {
    let time = text_of(form, "occurred_at");
    if time.is_empty() {
        return Ok(None);
    }
    let at = crate::pages::parse_local(&time)
        .ok_or_else(|| crate::db::refused(t(lang, "manufacturing.bad_input")))?;
    Ok(Some(Late {
        at,
        reason: text_of(form, "late_reason"),
        outage: text_of(form, "outage_reference"),
    }))
}

/// `POST /manufacturing/inspections/{id}/pass`: `note`; and, for a result
/// entered later, `occurred_at`, `late_reason` and `outage_reference`.
pub(crate) async fn pass(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<std::collections::BTreeMap<String, String>>,
) -> Result<Response> {
    let note = text_of(&form, "note");
    let key = match late_of(&lang, &form)? {
        None => {
            returns::pass(&state, &user, id, &note).await?;
            "manufacturing.screen.inspections.pass_done"
        }
        Some(late) => {
            let entered = late_inspection::enter(
                &state,
                &user,
                id,
                Decision::Pass,
                &note,
                Made {
                    at: late.at,
                    reason: Some(&late.reason),
                    outage: Some(&late.outage),
                },
            )
            .await?;
            if entered.is_pending() {
                "manufacturing.screen.inspections.late_requested"
            } else {
                "manufacturing.screen.inspections.pass_late_done"
            }
        }
    };
    Ok(done(&lang, URL, key))
}

/// `POST /manufacturing/inspections/{id}/fail`: `defect`; and, for a result
/// entered later, `occurred_at`, `late_reason` and `outage_reference`.
pub(crate) async fn fail(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<std::collections::BTreeMap<String, String>>,
) -> Result<Response> {
    let defect = text_of(&form, "defect");
    let Some(late) = late_of(&lang, &form)? else {
        let (_, ncr) = returns::fail(&state, &user, id, &defect).await?;
        return Ok(done(
            &lang,
            &super::ncr_page::url(ncr.id),
            "manufacturing.screen.inspections.fail_done",
        ));
    };
    let entered = late_inspection::enter(
        &state,
        &user,
        id,
        Decision::Fail,
        &defect,
        Made {
            at: late.at,
            reason: Some(&late.reason),
            outage: Some(&late.outage),
        },
    )
    .await?;
    let key = if entered.is_pending() {
        "manufacturing.screen.inspections.late_requested"
    } else {
        "manufacturing.screen.inspections.fail_late_done"
    };
    Ok(done(&lang, URL, key))
}
