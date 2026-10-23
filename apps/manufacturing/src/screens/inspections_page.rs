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
        rows.push(Row {
            id: found.id,
            lot,
            item: tf(
                &lang,
                "manufacturing.screen.item_name",
                &[("id", &found.item_id)],
            ),
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
            record: if open && may_record {
                Act::ready()
            } else {
                Act::hidden()
            },
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

/// `POST /manufacturing/inspections/{id}/pass`.
pub(crate) async fn pass(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<std::collections::BTreeMap<String, String>>,
) -> Result<Response> {
    returns::pass(&state, &user, id, &text_of(&form, "note")).await?;
    Ok(done(
        &lang,
        URL,
        "manufacturing.screen.inspections.pass_done",
    ))
}

/// `POST /manufacturing/inspections/{id}/fail`.
pub(crate) async fn fail(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<std::collections::BTreeMap<String, String>>,
) -> Result<Response> {
    let (_, ncr) = returns::fail(&state, &user, id, &text_of(&form, "defect")).await?;
    Ok(done(
        &lang,
        &super::ncr_page::url(ncr.id),
        "manufacturing.screen.inspections.fail_done",
    ))
}
