//! The inspections of the returned lots (`GET /manufacturing/inspections`,
//! BL-MFG-07, `MFG.RUL.10`, `MFG.ACL.02`): per lot that came back into
//! quarantine the quantity, the reason, the order and dispatch it came from,
//! where it stands and, for QC, the result: *Pass* (the lot is released; the
//! storekeeper then moves the goods to a store) or *Fail* (an NCR is raised
//! on the lot). The page is what the QC inspector works from on a phone: one
//! card per lot, the actions as sheets. The page is a `renox::grid` searched
//! by dispatch and reason and filtered by status and quantity; a
//! `?status=open|passed|failed` link narrows the rows to that status under
//! the grid's own filters.
//! JSON keeps answering the rows in words.

use std::collections::HashMap;

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};
use wl_kernel::lot_disposal::lot_disposal;

use super::{Act, Texts, date_text, done, grids, quantity_text, time_text};
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
    lots: GridPage<ReturnInspection>,
    /// The lots of the page QC may decide now, `[id, reason]`, for the
    /// *Pass* and *Fail* sheets.
    sheets: Vec<(i64, String)>,
}

/// The returned lots grid: the lot first and frozen, the item, the status in
/// words, the quantity, whether the whole dispatch came back, the reason, the
/// dispatch, what else is known (late, waiting, the complaint and the NCR)
/// and the actions; newest first.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let text = |key: &str| t(lang, &format!("manufacturing.screen.{key}"));
    let statuses: Vec<(String, String)> =
        [inspection::OPEN, inspection::PASSED, inspection::FAILED]
            .iter()
            .map(|status| ((*status).to_owned(), word(lang, "inspection_status", status)))
            .collect();
    let came_back = vec![
        ("whole".to_owned(), text("inspections.whole")),
        ("refused".to_owned(), text("inspections.refused")),
    ];
    grids::baseline(Grid::new("manufacturing-inspections"))
        .title(&text("inspections.title"))
        .column(
            Column::custom("lot", &text("inspections.lot"))
                .frozen()
                .mobile(),
        )
        .column(Column::custom("item", &text("grid.item")).mobile())
        .column(
            Column::select("status", &text("grid.status"), statuses)
                .badges(&[
                    (inspection::OPEN, "warning"),
                    (inspection::PASSED, "success"),
                    (inspection::FAILED, "danger"),
                ])
                .mobile(),
        )
        .column(
            Column::quantity("quantity_milli", &text("inspections.quantity"))
                .scale(3)
                .mobile(),
        )
        .column(
            Column::select("came_back", &text("grid.came_back"), came_back)
                .computed("CASE WHEN {T}.\"returned\" THEN 'whole' ELSE 'refused' END"),
        )
        .column(Column::text("reason", &text("grid.reason")).searchable())
        .column(
            Column::text("dispatch_number", &text("inspections.dispatch"))
                .searchable()
                .link("/dispatches/{dispatch_id}"),
        )
        .column(Column::datetime("decided_at", &text("grid.decided_at")).hidden())
        .column(Column::text("note", &text("inspections.note")).hidden())
        .column(Column::custom("details", &text("grid.details")))
        .column(Column::custom("actions", "").frozen_right())
        .sort_by("-id")
        .empty_state(
            &text("inspections.empty"),
            Some(&text("inspections.empty_text")),
        )
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
    request: GridRequest,
) -> Result<Response> {
    let may_record = allows(&user, permissions::INSPECTIONS_RECORD);
    if !may_record && !allows(&user, permissions::INSPECTIONS_VIEW) {
        return Err(Error::Forbidden);
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for found in ReturnInspection::query()
            .order_by_desc(ReturnInspection::ID)
            .get(&state.db)
            .await?
        {
            if !filter.status.is_empty() && found.status != filter.status {
                continue;
            }
            rows.push(row(&state, &lang, &found, may_record).await?);
        }
        return Ok(Json(&rows).into_response());
    }
    let mut query = ReturnInspection::query();
    if !filter.status.is_empty() {
        query = query.where_eq("status", filter.status.clone());
    }
    // QC may export the list; the lot, the item and the notes are custom
    // columns and stay on screen.
    let grid = grids::exporting(grid(&lang), true);
    if let Some(file) =
        grids::export_file(&grid, true, query.clone(), &request).await?
    {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra: HashMap<i64, Value> = HashMap::new();
    let mut sheets = Vec::new();
    for found in page.items() {
        let line = row(&state, &lang, found, may_record).await?;
        if line.record.shown && line.record.blocked.is_empty() {
            sheets.push((found.id, line.reason.clone()));
        }
        extra.insert(
            found.id,
            json!({
                "lot": line.lot,
                "item": line.item,
                "late": line.late,
                "waits": line.waits,
                "complaint_url": line.complaint_url,
                "ncr_url": line.ncr_url,
                "record": line.record,
            }),
        );
    }
    let lots = page.extend(|found| extra.get(&found.id).cloned().unwrap_or(Value::Null));
    Ok(View::typed(InspectionsPage {
        tx: screen_texts(&lang),
        lots,
        sheets,
    })
    .into_response())
}

/// A returned lot in words, for the JSON answer and the grid's custom cells.
async fn row(
    state: &AppState,
    lang: &Lang,
    found: &ReturnInspection,
    may_record: bool,
) -> Result<Row> {
    let lot = match found.lot() {
        Some(subject) => lot_disposal(state)
            .facts(state, &subject)
            .await?
            .map(|facts| facts.number)
            .unwrap_or_else(|| subject.subject_id.to_string()),
        None => String::new(),
    };
    let lot = if lot.is_empty() {
        t(lang, "manufacturing.screen.not_set")
    } else {
        tf(
            lang,
            "manufacturing.screen.ncr.lot_name",
            &[("number", &lot)],
        )
    };
    let open = found.is_open();
    let waiting = if open {
        late_inspection::waiting(state, found.id).await?
    } else {
        None
    };
    let waits = match &waiting {
        Some(entry) => tf(
            lang,
            "manufacturing.screen.inspections.waits",
            &[
                (
                    "decision",
                    &decision_word(lang, entry.payload.0["decision"].as_str().unwrap_or("")),
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
    let late = match late_inspection::flagged(state, found.id).await? {
        Some(entry) => tf(
            lang,
            "manufacturing.screen.inspections.late",
            &[("reason", &entry.late_reason.unwrap_or_default())],
        ),
        None => String::new(),
    };
    Ok(Row {
        id: found.id,
        lot,
        item: super::names::item_text(state, lang, found.item_id).await,
        quantity: quantity_text(found.quantity_milli),
        reason: found.reason.clone().unwrap_or_default(),
        dispatch: found.dispatch_number.clone(),
        dispatch_url: format!("/dispatches/{}", found.dispatch_id),
        order_url: format!("/sales-orders/{}", found.order_id),
        came_back: if found.returned {
            t(lang, "manufacturing.screen.inspections.whole")
        } else {
            t(lang, "manufacturing.screen.inspections.refused")
        },
        complaint_url: found
            .complaint_id
            .map(|id| wl_kernel::tickets::ticket_path(wl_kernel::tickets::TicketId(id)))
            .unwrap_or_default(),
        ncr_url: found.ncr_id.map(super::ncr_page::url).unwrap_or_default(),
        status: word(lang, "inspection_status", &found.status),
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
            Act::blocked(t(lang, "manufacturing.screen.inspections.blocked_waits"))
        } else {
            Act::ready()
        },
        waits,
    })
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
