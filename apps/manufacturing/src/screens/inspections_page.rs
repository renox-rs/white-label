//! The inspections of the returned lots (`GET /manufacturing/inspections`,
//! BL-MFG-07, `MFG.RUL.10`, `MFG.ACL.02`): per lot that came back into
//! quarantine the quantity, the reason, the order and dispatch it came from,
//! where it stands and, for QC, the result: *Pass* (the lot is released; the
//! storekeeper then moves the goods to a store) or *Fail* (an NCR is raised
//! on the lot). The page is what the QC inspector works from on a phone: one
//! card per lot, the actions as sheets. The list is the standard grid (search
//! on the dispatch, the item, the reason and the note; filters by status,
//! date and quantity); `?status=open|passed|failed` still narrows the query.
//! The lot, the item, the notes and the buttons are custom cells; the dispatch
//! is a link only for who may open dispatches.

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::lot_disposal::lot_disposal;

use super::{
    Act, Texts, date_text, done, exported, may_open_dispatch, quantity_text, time_text,
    with_item_search,
};
use crate::late_inspection::{self, Decision, Made};
use crate::permissions;
use crate::returns::{self, ReturnInspection, inspection};
use crate::texts::{screen_texts, t, tf, word};

/// The address of the page.
pub(crate) const URL: &str = "/manufacturing/inspections";

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

/// The sheets of the *Pass* and *Fail* buttons of one inspection.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Sheet {
    id: i64,
    reason: String,
}

/// A status link of the header.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Tab {
    url: String,
    label: String,
    current: bool,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/inspections.html")]
pub(crate) struct InspectionsPage {
    tx: Texts,
    rows: GridPage<ReturnInspection>,
    status_links: Vec<Tab>,
    sheets: Vec<Sheet>,
}

/// The inspections grid. The dispatch is a custom cell, a link only for who
/// may open dispatches; its number is a searchable hidden column.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let tx = |key: &str| t(lang, &format!("manufacturing.screen.inspections.{key}"));
    let grid = Grid::new("manufacturing-inspections")
        .title(&tx("title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(Column::custom("lot", &tx("lot")).frozen().mobile())
        .column(Column::custom("item", &t(lang, "manufacturing.screen.fact.item")).mobile())
        .column(
            Column::quantity("quantity_milli", &tx("quantity"))
                .scale(3)
                .mobile(),
        )
        .column(Column::bool("returned", &tx("whole_dispatch")).hidden())
        .column(Column::text("reason", &tx("reason")).searchable())
        .column(Column::custom("dispatch", &tx("dispatch")))
        .column(
            Column::text("dispatch_number", &tx("dispatch"))
                .hidden()
                .searchable(),
        )
        .column(Column::custom("links", &tx("links")).hidden())
        .column(
            Column::select(
                "status",
                &t(lang, "manufacturing.screen.fields.status"),
                [inspection::OPEN, inspection::PASSED, inspection::FAILED]
                    .into_iter()
                    .map(|status| (status, word(lang, "inspection_status", status)))
                    .collect::<Vec<_>>(),
            )
            .badges(&[
                (inspection::OPEN, "warning"),
                (inspection::PASSED, "success"),
                (inspection::FAILED, "error"),
            ])
            .mobile(),
        )
        .column(Column::datetime("decided_at", &tx("decided")))
        .column(Column::text("note", &tx("note")).hidden().searchable())
        .column(Column::datetime("created_at", &tx("came_back_on")).hidden())
        .column(Column::custom("notes", &tx("notes")).hidden())
        .column(Column::custom("actions", &tx("actions")).frozen_right())
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&tx("empty"), Some(&tx("empty_text")));
    let grid = with_item_search(grid, lang, "item_id", true);
    if can_export { grid.exports() } else { grid }
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

/// The waiting entry, the late note and the buttons of an inspection.
struct Extras {
    waits: String,
    late: String,
    record: Act,
}

async fn extras_of(
    state: &AppState,
    lang: &Lang,
    found: &ReturnInspection,
    may_record: bool,
) -> Result<Extras> {
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
    let record = if !(open && may_record) {
        Act::hidden()
    } else if waiting.is_some() {
        Act::blocked(t(lang, "manufacturing.screen.inspections.blocked_waits"))
    } else {
        Act::ready()
    };
    Ok(Extras {
        waits,
        late,
        record,
    })
}

/// The lot of an inspection in words.
async fn lot_of(state: &AppState, lang: &Lang, found: &ReturnInspection) -> Result<String> {
    let lot = match found.lot() {
        Some(subject) => lot_disposal(state)
            .facts(state, &subject)
            .await?
            .map(|facts| facts.number)
            .unwrap_or_else(|| subject.subject_id.to_string()),
        None => String::new(),
    };
    Ok(if lot.is_empty() {
        t(lang, "manufacturing.screen.not_set")
    } else {
        tf(
            lang,
            "manufacturing.screen.ncr.lot_name",
            &[("number", &lot)],
        )
    })
}

/// `GET /manufacturing/inspections`.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    request: GridRequest,
) -> Result<Response> {
    let may_record = allows(&user, permissions::INSPECTIONS_RECORD);
    if !may_record && !allows(&user, permissions::INSPECTIONS_VIEW) {
        return Err(Error::Forbidden);
    }
    let status = request.param("status").unwrap_or_default().to_owned();
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for found in ReturnInspection::query()
            .order_by_desc(ReturnInspection::ID)
            .get(&state.db)
            .await?
        {
            if !status.is_empty() && found.status != status {
                continue;
            }
            let extras = extras_of(&state, &lang, &found, may_record).await?;
            rows.push(Row {
                id: found.id,
                lot: lot_of(&state, &lang, &found).await?,
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
                status_kind: status_kind_of(&found.status),
                decided: found
                    .decided_at
                    .map(|at| format!("{} {}", date_text(at.date_naive()), time_text(at)))
                    .unwrap_or_default(),
                note: found.note.clone().unwrap_or_default(),
                late: extras.late,
                record: extras.record,
                waits: extras.waits,
            });
        }
        return Ok(Json(&rows).into_response());
    }
    let can_export = may_record || allows(&user, permissions::INSPECTIONS_VIEW);
    let grid = grid(&lang, can_export);
    let link_dispatch = may_open_dispatch(&user);
    let mut query = ReturnInspection::query();
    if !status.is_empty() {
        query = query.where_(ReturnInspection::STATUS.eq(status.clone()));
    }
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    let mut sheets = Vec::new();
    for found in page.items() {
        let extras = extras_of(&state, &lang, found, may_record).await?;
        let ready = extras.record.shown && extras.record.blocked.is_empty();
        if ready {
            sheets.push(Sheet {
                id: found.id,
                reason: found.reason.clone().unwrap_or_default(),
            });
        }
        extra.insert(
            found.id,
            json!({
                "dispatch_url": if link_dispatch { format!("/dispatches/{}", found.dispatch_id) } else { String::new() },
                "lot": lot_of(&state, &lang, found).await?,
                "item": super::names::item_text(&state, &lang, found.item_id).await,
                "complaint_url": found.complaint_id
                    .map(|id| wl_kernel::tickets::ticket_path(wl_kernel::tickets::TicketId(id)))
                    .unwrap_or_default(),
                "ncr_url": found.ncr_id.map(super::ncr_page::url).unwrap_or_default(),
                "late": extras.late,
                "waits": extras.waits,
                "can_record": ready,
                "record_blocked": if extras.record.shown { extras.record.blocked } else { String::new() },
            }),
        );
    }
    let page = page.extend(|found| extra.get(&found.id).cloned().unwrap_or_else(|| json!({})));
    let tabs = [(String::new(), t(&lang, "manufacturing.screen.filter.all"))]
        .into_iter()
        .chain(
            [inspection::OPEN, inspection::PASSED, inspection::FAILED]
                .into_iter()
                .map(|key| (key.to_owned(), word(&lang, "inspection_status", key))),
        )
        .map(|(key, label)| {
            let url = if key.is_empty() {
                URL.to_owned()
            } else {
                format!("{URL}?status={key}")
            };
            Tab {
                url,
                label,
                current: key == status,
            }
        })
        .collect();
    Ok(View::typed(InspectionsPage {
        tx: screen_texts(&lang),
        rows: page,
        status_links: tabs,
        sheets,
    })
    .into_response())
}

fn status_kind_of(status: &str) -> &'static str {
    match status {
        inspection::OPEN => "warning",
        inspection::PASSED => "success",
        _ => "error",
    }
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
