//! The NCR list (`GET /manufacturing/ncrs`, `docs/UX.md` §2.1) and its
//! *Raise an NCR* sheet.
//!
//! The list shows the non-conformance reports of the plants the person works
//! for: the number, the item and lot, the quantity, the cause, the
//! disposition and the status. The status filter is `?status=<key>` (`active`
//! is every NCR that is not closed or cancelled).

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Texts, date_text, options, quantity_text};
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
    rows: Vec<Row>,
    status_tabs: Vec<(String, String)>,
    status_current: String,
    can_raise: bool,
    form: NewForm,
}

/// The status tabs: all, those still to work on and each state.
fn tabs(lang: &Lang) -> Vec<(String, String)> {
    let mut tabs = vec![
        (URL.to_owned(), t(lang, "manufacturing.screen.filter.all")),
        (
            format!("{URL}?status=active"),
            t(lang, "manufacturing.screen.filter.open"),
        ),
    ];
    for status in ["open", "dispositioned", "closed", "cancelled"] {
        tabs.push((
            format!("{URL}?status={status}"),
            word(lang, "ncr_status", status),
        ));
    }
    tabs
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
    Query(filter): Query<ListFilter>,
) -> Result<Response> {
    let mut rows = Vec::new();
    let mut any = false;
    for ncr in Ncr::query().order_by_desc(Ncr::ID).get(&state.db).await? {
        if !may_see(&user, &ncr) {
            continue;
        }
        any = true;
        let wanted = match filter.status.as_str() {
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
            lot: lot_text(&state, &lang, &ncr).await,
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
    if !any
        && !user.has_permission_anywhere(permissions::NCRS_VIEW)
        && !user.has_permission_anywhere(permissions::NCRS_RAISE)
        && !user.has_permission_anywhere(permissions::NCRS_DISPOSITION)
    {
        return Err(Error::Forbidden);
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
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
    let current = match filter.status.as_str() {
        "" => URL.to_owned(),
        key => format!("{URL}?status={key}"),
    };
    let page = NcrsPage {
        tx: screen_texts(&lang),
        rows,
        status_tabs: tabs(&lang),
        status_current: current,
        can_raise,
        form,
    };
    Ok(View::typed(page).into_response())
}
