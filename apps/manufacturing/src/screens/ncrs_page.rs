//! The NCR list (`GET /manufacturing/ncrs`, `docs/UX.md` §2.1) and its
//! *Raise an NCR* sheet.
//!
//! The list is a `renox::grid` of the non-conformance reports of the plants
//! the person works for: the number, the item and lot, the status, the
//! quantity, the cause, the disposition and the day it was raised; searched
//! by number, lot and defect, filtered by cause, disposition, status and
//! dates. A `?status=<key>` link (`active` is every NCR that is not closed
//! or cancelled) narrows the rows to those states under the grid's own
//! filters; JSON keeps answering the rows in words, filtered the same way.

use std::collections::HashMap;

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};

use super::{Texts, date_text, grids, options, quantity_text};
use crate::ncr::{Cause, Disposition, Ncr, NcrStatus};
use crate::permissions;
use crate::plant::Plant;
use crate::texts::{screen_texts, t, tf, word};

/// The address of the list.
pub(crate) const URL: &str = "/manufacturing/ncrs";

/// The badge tone of an NCR status.
pub(crate) fn status_kind(status: NcrStatus) -> &'static str {
    match status {
        NcrStatus::Open => "error",
        NcrStatus::Dispositioned => "warning",
        NcrStatus::Closed => "success",
        NcrStatus::Cancelled => "info",
    }
}

/// The list's filter.
#[derive(Deserialize, Default)]
pub(crate) struct ListFilter {
    /// A status key, or `open`.
    #[serde(default)]
    status: String,
}

/// An NCR of the list.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    id: i64,
    url: String,
    number: String,
    item: String,
    lot: String,
    quantity: String,
    cause: String,
    disposition: String,
    raised: String,
    status: String,
    status_kind: &'static str,
}

/// What the *Raise an NCR* sheet needs.
#[derive(Debug, Serialize, Clone, Default)]
pub(crate) struct NewForm {
    plants: Vec<(String, String)>,
    plant_selected: String,
    causes: Vec<(String, String)>,
    cause_selected: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/ncrs.html")]
pub(crate) struct NcrsPage {
    tx: Texts,
    ncrs: GridPage<Ncr>,
    can_raise: bool,
    form: NewForm,
}

/// The NCRs grid: the number first and frozen, the item, the lot, the status
/// in words, the quantity, the cause, the disposition, the day it was raised
/// and the defect (hidden); newest first.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let text = |key: &str| t(lang, &format!("manufacturing.screen.{key}"));
    let statuses: Vec<(String, String)> = NcrStatus::ALL
        .iter()
        .map(|status| {
            (
                status.as_str().to_owned(),
                word(lang, "ncr_status", status.as_str()),
            )
        })
        .collect();
    let tones: Vec<(&str, &str)> = NcrStatus::ALL
        .iter()
        .map(|status| (status.as_str(), grids::tone(status_kind(*status))))
        .collect();
    let causes: Vec<(String, String)> = Cause::ALL
        .iter()
        .map(|cause| {
            (
                cause.as_str().to_owned(),
                word(lang, "cause", cause.as_str()),
            )
        })
        .collect();
    let dispositions: Vec<(String, String)> = Disposition::ALL
        .iter()
        .map(|d| (d.as_str().to_owned(), word(lang, "disposition", d.as_str())))
        .collect();
    grids::baseline(Grid::new("manufacturing-ncrs"))
        .title(&text("ncrs.title"))
        .column(
            Column::text("number", &text("ncrs.ncr"))
                .frozen()
                .mobile()
                .searchable()
                .link(&format!("{URL}/{{id}}")),
        )
        .column(Column::custom("item", &text("grid.item")).mobile())
        .column(Column::custom("lot", &text("grid.lot")))
        .column(
            Column::select("status", &text("grid.status"), statuses)
                .badges(&tones)
                .mobile(),
        )
        .column(
            Column::quantity("quantity_milli", &text("ncrs.quantity"))
                .scale(3)
                .mobile(),
        )
        .column(Column::select("cause", &text("ncrs.cause"), causes))
        .column(Column::select(
            "disposition",
            &text("ncrs.disposition"),
            dispositions,
        ))
        .column(Column::date("raised_on", &text("ncrs.raised")))
        .column(
            Column::text("lot_number", &text("grid.lot_number"))
                .searchable()
                .hidden(),
        )
        .column(
            Column::text("defect", &text("ncrs.form.defect"))
                .searchable()
                .hidden(),
        )
        .sort_by("-id")
        .row_url(&format!("{URL}/{{id}}"))
        .empty_state(&text("ncrs.empty"), Some(&text("ncrs.empty_text")))
}

/// The states a `?status=<key>` link names: `active` is every NCR still
/// to work on; nothing for no filter.
fn linked_statuses(key: &str) -> Vec<&'static str> {
    match key {
        "" => Vec::new(),
        "active" => vec![NcrStatus::Open.as_str(), NcrStatus::Dispositioned.as_str()],
        key => NcrStatus::ALL
            .iter()
            .map(|status| status.as_str())
            .filter(|status| *status == key)
            .collect(),
    }
}

/// Whether `user` may see anything of NCRs in `ncr`'s plant.
pub(crate) fn may_see(user: &AuthUser, ncr: &Ncr) -> bool {
    [
        permissions::NCRS_VIEW,
        permissions::NCRS_RAISE,
        permissions::NCRS_DISPOSITION,
        permissions::NCRS_APPROVE,
    ]
    .iter()
    .any(|permission| ncr.may(user, permission))
}

/// The item of an NCR in words.
pub(crate) async fn item_text(state: &AppState, lang: &Lang, item_id: i64) -> String {
    super::names::item_text(state, lang, item_id).await
}

/// The lot of an NCR in words: the number on the pack, else a dash.
pub(crate) async fn lot_text(state: &AppState, lang: &Lang, ncr: &Ncr) -> String {
    if let Some(number) = &ncr.lot_number {
        return tf(
            lang,
            "manufacturing.screen.ncr.lot_name",
            &[("number", number)],
        );
    }
    match ncr.lot_id {
        Some(id) => super::names::lot_text(state, lang, id).await,
        None => t(lang, "manufacturing.screen.not_set"),
    }
}

/// `GET /manufacturing/ncrs`: the grid for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<ListFilter>,
    request: GridRequest,
) -> Result<Response> {
    let mut visible = Vec::new();
    for ncr in Ncr::query().order_by_desc(Ncr::ID).get(&state.db).await? {
        if may_see(&user, &ncr) {
            visible.push(ncr);
        }
    }
    if visible.is_empty()
        && !user.has_permission_anywhere(permissions::NCRS_VIEW)
        && !user.has_permission_anywhere(permissions::NCRS_RAISE)
        && !user.has_permission_anywhere(permissions::NCRS_DISPOSITION)
    {
        return Err(Error::Forbidden);
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for ncr in &visible {
            let wanted = match filter.status.as_str() {
                "" => true,
                "active" => matches!(ncr.status, NcrStatus::Open | NcrStatus::Dispositioned),
                key => ncr.status.as_str() == key,
            };
            if wanted {
                rows.push(row(&state, &lang, ncr).await);
            }
        }
        return Ok(Json(&rows).into_response());
    }
    // QC and the people who decide the NCRs may export the list (no supplier
    // or cost is in it).
    let can_export = user.has_permission_anywhere(permissions::NCRS_VIEW)
        || user.has_permission_anywhere(permissions::NCRS_DISPOSITION);
    let grid = grids::exporting(grid(&lang), can_export);
    let mut query = Ncr::query().where_in("id", visible.iter().map(|ncr| ncr.id));
    if !filter.status.is_empty() {
        query = query.where_in("status", linked_statuses(&filter.status));
    }
    if let Some(file) = grids::export_file(&grid, can_export, query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra: HashMap<i64, Value> = HashMap::new();
    for ncr in page.items() {
        extra.insert(
            ncr.id,
            json!({
                "item": item_text(&state, &lang, ncr.item_id).await,
                "lot": lot_text(&state, &lang, ncr).await,
            }),
        );
    }
    let ncrs = page.extend(|ncr| extra.get(&ncr.id).cloned().unwrap_or(Value::Null));
    let mut form = NewForm {
        causes: options(
            &lang,
            "cause",
            &[
                Cause::Supplier.as_str(),
                Cause::InternalProcess.as_str(),
                Cause::Unknown.as_str(),
            ],
        ),
        ..NewForm::default()
    };
    form.causes.insert(
        0,
        (
            String::new(),
            t(&lang, "manufacturing.screen.ncr.cause_default"),
        ),
    );
    let mut can_raise = false;
    for plant in Plant::query().order_by(Plant::ID).get(&state.db).await? {
        if plant.active && user.has_permission_in(permissions::NCRS_RAISE, &plant.scope()) {
            can_raise = true;
            form.plants.push((plant.id.to_string(), plant.name.clone()));
        }
    }
    form.plant_selected = form.plants.first().map(|p| p.0.clone()).unwrap_or_default();
    form.cause_selected = String::new();
    let page = NcrsPage {
        tx: screen_texts(&lang),
        ncrs,
        can_raise,
        form,
    };
    Ok(View::typed(page).into_response())
}

/// An NCR in words, for the JSON answer.
async fn row(state: &AppState, lang: &Lang, ncr: &Ncr) -> Row {
    Row {
        id: ncr.id,
        url: super::ncr_page::url(ncr.id),
        number: ncr.number.clone(),
        item: item_text(state, lang, ncr.item_id).await,
        lot: lot_text(state, lang, ncr).await,
        quantity: quantity_text(ncr.quantity_milli),
        cause: word(lang, "cause", ncr.cause.as_str()),
        disposition: ncr.disposition.map_or_else(
            || t(lang, "manufacturing.screen.not_set"),
            |d| word(lang, "disposition", d.as_str()),
        ),
        raised: date_text(ncr.raised_on),
        status: word(lang, "ncr_status", ncr.status.as_str()),
        status_kind: status_kind(ncr.status),
    }
}
