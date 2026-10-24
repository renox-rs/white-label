//! The NCR list (`GET /manufacturing/ncrs`, `docs/UX.md` §2.1) and its
//! *Raise an NCR* sheet.
//!
//! The list is the standard grid over the non-conformance reports of the
//! plants the person works for (the scope is in the query): the number, the
//! item and lot, the quantity, the cause, the disposition and the status,
//! filtered by status, cause, disposition and date. `?status=active` still
//! narrows the query to the NCRs that are neither closed nor cancelled (the
//! header's *To work on*); one state is the status filter of the grid, so no
//! label is written twice.

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;

use super::{Texts, date_text, exported, ids_or_none, options, quantity_text};
use crate::ncr::{Cause, Ncr, NcrStatus};
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
    rows: GridPage<Ncr>,
    all_url: String,
    active_url: String,
    active_now: bool,
    can_raise: bool,
    form: NewForm,
}

/// The NCR grid: search on the number, the lot and the defect, filters by
/// status, cause, disposition, quantity and date. The item is drawn by the
/// template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let pick = |group: &str, values: &[&str]| options(lang, group, values);
    let grid = Grid::new("manufacturing-ncrs")
        .title(&t(lang, "manufacturing.screen.ncrs.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &t(lang, "manufacturing.screen.ncrs.ncr"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/manufacturing/ncrs/{id}"),
        )
        .column(Column::custom("item", &t(lang, "manufacturing.screen.fact.item")).mobile())
        .column(Column::text("lot_number", &t(lang, "manufacturing.screen.ncrs.lot")).searchable())
        .column(
            Column::quantity(
                "quantity_milli",
                &t(lang, "manufacturing.screen.ncrs.quantity"),
            )
            .scale(3)
            .mobile(),
        )
        .column(
            Column::text("defect", &t(lang, "manufacturing.screen.ncrs.defect"))
                .hidden()
                .searchable()
                .limit(80),
        )
        .column(Column::select(
            "cause",
            &t(lang, "manufacturing.screen.ncrs.cause"),
            pick("cause", &["supplier", "internal_process", "unknown"]),
        ))
        .column(Column::select(
            "disposition",
            &t(lang, "manufacturing.screen.ncrs.disposition"),
            pick(
                "disposition",
                &["rework", "scrap", "use_as_is", "return_to_supplier"],
            ),
        ))
        .column(Column::date(
            "raised_on",
            &t(lang, "manufacturing.screen.ncrs.raised"),
        ))
        .column(
            Column::select(
                "status",
                &t(lang, "manufacturing.screen.fields.status"),
                pick(
                    "ncr_status",
                    &["open", "dispositioned", "closed", "cancelled"],
                ),
            )
            .badges(&[
                ("open", "error"),
                ("dispositioned", "warning"),
                ("closed", "success"),
                ("cancelled", "info"),
            ])
            .mobile(),
        )
        .sort_by("-id")
        .row_url("/manufacturing/ncrs/{id}")
        .cards_on_mobile()
        .empty_state(
            &t(lang, "manufacturing.screen.ncrs.empty"),
            Some(&t(lang, "manufacturing.screen.ncrs.empty_text")),
        );
    let grid = super::with_item_search(grid, lang, "item_id", true);
    if can_export { grid.exports() } else { grid }
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

/// `GET /manufacturing/ncrs`: the page for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    request: GridRequest,
) -> Result<Response> {
    let status = request.param("status").unwrap_or_default().to_owned();
    let active = status == "active";
    let visible: Vec<Ncr> = Ncr::query()
        .order_by_desc(Ncr::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|ncr| may_see(&user, ncr))
        .collect();
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
            let wanted = match status.as_str() {
                "" => true,
                "active" => matches!(ncr.status, NcrStatus::Open | NcrStatus::Dispositioned),
                key => ncr.status.as_str() == key,
            };
            if !wanted {
                continue;
            }
            rows.push(Row {
                id: ncr.id,
                url: super::ncr_page::url(ncr.id),
                number: ncr.number.clone(),
                item: item_text(&state, &lang, ncr.item_id).await,
                lot: lot_text(&state, &lang, ncr).await,
                quantity: quantity_text(ncr.quantity_milli),
                cause: word(&lang, "cause", ncr.cause.as_str()),
                disposition: ncr.disposition.map_or_else(
                    || t(&lang, "manufacturing.screen.not_set"),
                    |d| word(&lang, "disposition", d.as_str()),
                ),
                raised: date_text(ncr.raised_on),
                status: word(&lang, "ncr_status", ncr.status.as_str()),
                status_kind: status_kind(ncr.status),
            });
        }
        return Ok(Json(&rows).into_response());
    }
    let can_export = user.has_permission_anywhere(permissions::NCRS_DISPOSITION)
        || user.has_permission_anywhere(permissions::NCRS_APPROVE);
    let grid = grid(&lang, can_export);
    let mut query =
        Ncr::query().where_(Ncr::ID.is_in(ids_or_none(visible.iter().map(|ncr| ncr.id).collect())));
    if active {
        query = query.where_(Ncr::STATUS.is_in([NcrStatus::Open, NcrStatus::Dispositioned]));
    } else if let Some(one) = [
        NcrStatus::Open,
        NcrStatus::Dispositioned,
        NcrStatus::Closed,
        NcrStatus::Cancelled,
    ]
    .into_iter()
    .find(|s| s.as_str() == status)
    {
        query = query.where_(Ncr::STATUS.eq(one));
    }
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    for ncr in page.items() {
        extra.insert(
            ncr.id,
            json!({ "item": item_text(&state, &lang, ncr.item_id).await }),
        );
    }
    let page = page.extend(|ncr| extra.get(&ncr.id).cloned().unwrap_or_else(|| json!({})));
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
        rows: page,
        all_url: URL.to_owned(),
        active_url: format!("{URL}?status=active"),
        active_now: active,
        can_raise,
        form,
    };
    Ok(View::typed(page).into_response())
}
