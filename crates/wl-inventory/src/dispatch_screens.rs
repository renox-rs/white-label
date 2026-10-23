//! The dispatch screens: the list of dispatches and one dispatch with its
//! lines and the buttons that pick, dispatch, cancel, re-pick, confirm a
//! courier's delivery and record the refused goods that came back (issues
//! #240 and #241; §4 *Dispatch*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /dispatches` | `wl_inventory.dispatches` | `dispatch.manage` or `stock.view` somewhere; a page for a browser, JSON for the rest |
//! | `GET /dispatches/{id}` | `wl_inventory.dispatch_page` | `dispatch.manage` or `stock.view` at the location; a page for a browser, JSON for the rest |
//!
//! The buttons post to the routes of [`crate::dispatch_pages`]. A button is
//! shown to a user who holds `dispatch.manage` at the location and only for
//! the states it applies to; one that cannot run now is shown disabled with
//! the reason (a picked lot on hold: *re-pick* first; an own delivery is
//! confirmed by the driver's proof, not by hand). A refusal of the service
//! (a lot on hold, a failed credit re-check) is the 422 sentence in the sheet
//! that sent the form.
//!
//! Decision: the screen makes no dispatch; the module that sells (a sales
//! order) makes the draft with [`crate::dispatch::create_draft`].

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::documents::dubai_text;
use wl_kernel::inventory::DISPATCH_SUBJECT_TYPE;

use crate::balance;
use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, Handover, lines_of};
use crate::dispatch_pick::lot_problem;
use crate::dispatch_texts::DISPATCH_TEXTS;
use crate::item::Item;
use crate::location::Location;
use crate::lot::{Lot, LotStatus};
use crate::permissions;
use crate::stock_screens::{amount as quantity_with_unit, item_name, label};
use crate::texts::{said, t};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "wl-inventory/dispatches.html",
        include_str!("../views/wl-inventory/dispatches.html"),
    ),
    (
        "wl-inventory/dispatch.html",
        include_str!("../views/wl-inventory/dispatch.html"),
    ),
];

/// The texts a template reads as `tx['title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The prefix of the keys of these screens' texts.
pub(crate) const PREFIX: &str = "wl_inventory.dispatch.ui.";

/// Every text of the dispatch screens in `lang`, by the name a template reads.
pub(crate) fn tx(lang: &Lang) -> Texts {
    DISPATCH_TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix(PREFIX)?, t(lang, key))))
        .collect()
}

/// The word for `value` in `group` (`status`, `handover`, `reason`).
fn word(lang: &Lang, group: &str, value: &str) -> String {
    let key = format!("{PREFIX}{group}.{value}");
    let found = t(lang, &key);
    if found == key {
        value.replace('_', " ")
    } else {
        found
    }
}

/// The routes of the dispatch screens.
pub fn routes() -> Routes {
    Routes::new()
        .get("/dispatches", list)
        .name("wl_inventory.dispatches")
        .get("/dispatches/{id}", show)
        .name("wl_inventory.dispatch_page")
        .require_auth()
}

/// A dispatch's label for pages that only hold its id.
pub(crate) fn register_labels() {
    wl_kernel::documents::register_subject_label(DISPATCH_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Dispatch::find(db, id).await?.and_then(|row| row.number)) })
    });
}

/// The badge colour of a status.
fn status_kind(status: DispatchStatus) -> &'static str {
    match status {
        DispatchStatus::Draft => "info",
        DispatchStatus::Picked | DispatchStatus::Dispatched => "warning",
        DispatchStatus::Delivered => "success",
        DispatchStatus::Returned | DispatchStatus::Discarded | DispatchStatus::Lost => "error",
        DispatchStatus::Cancelled => "info",
    }
}

/// One dispatch in the list.
#[derive(Serialize, Debug, Clone)]
pub struct DispatchRow {
    id: i64,
    number: String,
    recipient: String,
    source: String,
    store: String,
    lines: String,
    status: String,
    status_code: String,
    status_kind: &'static str,
    when: String,
}

/// What the dispatches page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/dispatches.html")]
pub(crate) struct DispatchesPage {
    tx: Texts,
    rows: Vec<DispatchRow>,
}

/// Whether `user` may see the dispatch: the right to manage or to see stock
/// at its location.
fn may_see(user: &AuthUser, dispatch: &Dispatch) -> bool {
    user.has_permission_in(permissions::DISPATCH_MANAGE, &dispatch.scope())
        || user.has_permission_in(permissions::STOCK_VIEW, &dispatch.scope())
}

/// `GET /dispatches`.
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::DISPATCH_MANAGE)
        && !user.has_permission_anywhere(permissions::STOCK_VIEW)
    {
        return Err(Error::Forbidden);
    }
    let mut rows = Vec::new();
    for dispatch in Dispatch::query()
        .order_by_desc(Dispatch::ID)
        .limit(200)
        .get(&state.db)
        .await?
    {
        if !may_see(&user, &dispatch) {
            continue;
        }
        let location = Location::find_or_404(&state.db, dispatch.location_id).await?;
        let mut lines = Vec::new();
        for line in lines_of(&state.db, dispatch.id).await? {
            let item = Item::find_or_404(&state.db, line.item_id).await?;
            lines.push(format!(
                "{} · {}",
                item_name(&item, &lang),
                quantity_with_unit(line.quantity_milli, &item.base_unit)
            ));
        }
        rows.push(DispatchRow {
            id: dispatch.id,
            number: dispatch.number.clone().unwrap_or_default(),
            recipient: dispatch.recipient_name.clone(),
            source: source_text(&state, &dispatch).await,
            store: location.name_en,
            lines: lines.join(", "),
            status: word(&lang, "status", dispatch.status.as_str()),
            status_code: dispatch.status.as_str().to_owned(),
            status_kind: status_kind(dispatch.status),
            when: dispatch.updated_at.map(dubai_text).unwrap_or_default(),
        });
    }
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(DispatchesPage {
            tx: tx(&lang),
            rows,
        })
        .into_response());
    }
    json(&rows)
}

/// `value` as a JSON answer.
fn json<T: Serialize>(value: &T) -> Result<Response> {
    let body = renox::serde_json::to_string(value)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// The record the dispatch fulfils, in words: its label, else its number.
async fn source_text(state: &AppState, dispatch: &Dispatch) -> String {
    let source: Subject = dispatch.source();
    let found = label(state, &source).await;
    if found.is_empty() {
        format!("#{}", dispatch.source_id)
    } else {
        found
    }
}

/// An action button: shown or not, and the reason when it cannot run now.
#[derive(Serialize, Debug, Clone, Default)]
pub struct Act {
    shown: bool,
    blocked: Option<String>,
}

impl Act {
    fn hidden() -> Self {
        Act::default()
    }

    fn ready() -> Self {
        Act {
            shown: true,
            blocked: None,
        }
    }

    fn unless(reason: Option<String>) -> Self {
        Act {
            shown: true,
            blocked: reason,
        }
    }
}

/// The buttons of a dispatch.
#[derive(Serialize, Debug, Clone)]
pub struct Actions {
    pick: Act,
    dispatch: Act,
    cancel: Act,
    confirm: Act,
    receive_back: Act,
}

/// One line of the record page.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    id: i64,
    position: i64,
    item: String,
    lot: String,
    quantity: String,
    delivered: String,
    refused: String,
    reason: String,
    in_return: String,
    received: String,
    missing: String,
    held: bool,
    can_repick: bool,
    lots: Vec<(String, String)>,
    unit: String,
}

/// What the record page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/dispatch.html")]
pub(crate) struct DispatchPage {
    tx: Texts,
    id: i64,
    title: String,
    status: String,
    status_code: String,
    status_kind: &'static str,
    recipient: String,
    address: String,
    source: String,
    store: String,
    handover: String,
    tracking: String,
    signature: String,
    delivery: String,
    has_note: bool,
    picked_at: String,
    dispatched_at: String,
    delivered_at: String,
    loss: String,
    alerts: Vec<String>,
    lines: Vec<LineRow>,
    actions: Actions,
    handovers: Vec<(String, String)>,
    reasons: Vec<(String, String)>,
}

/// The released lots of `item` with something available at `location_id`,
/// as options of a select, the first to expire first.
async fn lot_options(
    state: &AppState,
    lang: &Lang,
    item: &Item,
    location_id: i64,
    leave_out: Option<i64>,
) -> Result<Vec<(String, String)>> {
    let mut options = vec![(String::new(), t(lang, &format!("{PREFIX}suggest")))];
    if !item.lot_tracked {
        return Ok(options);
    }
    for candidate in balance::candidates(&state.db, item, location_id).await? {
        let Some(lot_id) = candidate.lot_id else {
            continue;
        };
        if Some(lot_id) == leave_out {
            continue;
        }
        let lot = Lot::find_or_404(&state.db, lot_id).await?;
        let expiry = lot
            .expiry_date
            .map(|day| format!(" ({day})"))
            .unwrap_or_default();
        options.push((lot.id.to_string(), format!("{}{expiry}", lot.number)));
    }
    Ok(options)
}

async fn line_row(
    state: &AppState,
    lang: &Lang,
    dispatch: &Dispatch,
    line: &DispatchLine,
    manage: bool,
    alerts: &mut Vec<String>,
) -> Result<LineRow> {
    let item = Item::find_or_404(&state.db, line.item_id).await?;
    let lot = match line.lot_id {
        Some(id) => Some(Lot::find_or_404(&state.db, id).await?),
        None => None,
    };
    let held = lot
        .as_ref()
        .is_some_and(|lot| lot_problem(lot, line.position).is_some());
    if held
        && dispatch.status == DispatchStatus::Picked
        && let Some(lot) = &lot
    {
        let key = if lot.status == LotStatus::OnHold {
            "lot_held"
        } else {
            "lot_unusable"
        };
        alerts.push(said(
            &format!("wl_inventory.dispatch.ui.{key}"),
            &[
                ("position", &line.position.to_string()),
                ("lot", &lot.number),
            ],
        ));
    }
    let unit = item.base_unit.clone();
    let shown = |milli: i64| quantity_with_unit(milli, &unit);
    let lots = if manage && dispatch.status == DispatchStatus::Picked {
        lot_options(state, lang, &item, dispatch.location_id, line.lot_id).await?
    } else {
        Vec::new()
    };
    Ok(LineRow {
        id: line.id,
        position: line.position,
        item: item_name(&item, lang),
        lot: lot
            .map(|lot| lot.number)
            .unwrap_or_else(|| t(lang, &format!("{PREFIX}no_lot"))),
        quantity: shown(line.quantity_milli),
        delivered: shown(line.delivered_milli),
        refused: shown(line.refused_milli),
        reason: line
            .refusal_reason
            .as_deref()
            .map(|reason| word(lang, "reason", reason))
            .unwrap_or_default(),
        in_return: shown(line.in_return_milli()),
        received: shown(line.returned_milli),
        missing: shown(line.missing_milli),
        held,
        can_repick: manage && dispatch.status == DispatchStatus::Picked,
        lots,
        unit,
    })
}

/// `GET /dispatches/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    if !may_see(&user, &dispatch) {
        return Err(Error::Forbidden);
    }
    let manage = user.has_permission_in(permissions::DISPATCH_MANAGE, &dispatch.scope());
    let location = Location::find_or_404(&state.db, dispatch.location_id).await?;
    let lines = lines_of(&state.db, id).await?;
    let mut alerts = Vec::new();
    let mut rows = Vec::new();
    for line in &lines {
        rows.push(line_row(&state, &lang, &dispatch, line, manage, &mut alerts).await?);
    }
    if wl_kernel::negotiate::wants_html(&headers) {
        let in_return: i64 = lines.iter().map(DispatchLine::in_return_milli).sum();
        let own_delivery = dispatch.handover.as_deref() == Some(Handover::OwnDelivery.as_str());
        let status = dispatch.status;
        let actions = Actions {
            pick: if manage && status == DispatchStatus::Draft {
                Act::ready()
            } else {
                Act::hidden()
            },
            dispatch: if manage && status == DispatchStatus::Picked {
                Act::unless(alerts.first().cloned())
            } else {
                Act::hidden()
            },
            cancel: if manage && matches!(status, DispatchStatus::Draft | DispatchStatus::Picked) {
                Act::ready()
            } else {
                Act::hidden()
            },
            confirm: if manage && status == DispatchStatus::Dispatched {
                Act::unless(
                    own_delivery.then(|| t(&lang, &format!("{PREFIX}blocked_own_delivery"))),
                )
            } else {
                Act::hidden()
            },
            receive_back: if manage
                && matches!(
                    status,
                    DispatchStatus::Delivered | DispatchStatus::Dispatched | DispatchStatus::Lost
                )
                && lines.iter().any(|line| line.refused_milli > 0)
            {
                Act::unless(
                    (in_return == 0)
                        .then(|| t(&lang, &format!("{PREFIX}blocked_nothing_in_return"))),
                )
            } else {
                Act::hidden()
            },
        };
        let loss = if dispatch.loss_open {
            said(
                "wl_inventory.dispatch.ui.lost",
                &[("value", &crate::po_revise::money_text(dispatch.loss_fils))],
            )
        } else {
            String::new()
        };
        let mut notes = alerts;
        if in_return > 0 && status != DispatchStatus::Picked {
            let unit = rows.first().map(|row| row.unit.clone()).unwrap_or_default();
            let quantity = quantity_with_unit(in_return, &unit);
            notes.push(said(
                "wl_inventory.dispatch.ui.in_return_note",
                &[("quantity", &quantity)],
            ));
        }
        let page = DispatchPage {
            tx: tx(&lang),
            id,
            title: dispatch.number.clone().unwrap_or_default(),
            status: word(&lang, "status", status.as_str()),
            status_code: status.as_str().to_owned(),
            status_kind: status_kind(status),
            recipient: dispatch.recipient_name.clone(),
            address: dispatch.address.clone(),
            source: source_text(&state, &dispatch).await,
            store: location.name_en.clone(),
            handover: dispatch
                .handover
                .as_deref()
                .map(|kind| word(&lang, "handover", kind))
                .unwrap_or_default(),
            tracking: dispatch.tracking_number.clone().unwrap_or_default(),
            signature: dispatch.collector_signature.clone().unwrap_or_default(),
            delivery: dispatch
                .delivery_id
                .map(|delivery| format!("#{delivery}"))
                .unwrap_or_default(),
            has_note: dispatch.delivery_note.is_some(),
            picked_at: dispatch.picked_at.map(dubai_text).unwrap_or_default(),
            dispatched_at: dispatch.dispatched_at.map(dubai_text).unwrap_or_default(),
            delivered_at: dispatch.delivered_at.map(dubai_text).unwrap_or_default(),
            loss,
            alerts: notes,
            lines: rows,
            actions,
            handovers: [
                Handover::OwnDelivery,
                Handover::Courier,
                Handover::Collection,
            ]
            .iter()
            .map(|handover| {
                (
                    handover.as_str().to_owned(),
                    word(&lang, "handover", handover.as_str()),
                )
            })
            .collect(),
            reasons: [
                "damaged",
                "wrong_item",
                "not_ordered",
                "short_dated",
                "temperature",
                "other",
            ]
            .iter()
            .map(|reason| ((*reason).to_owned(), word(&lang, "reason", reason)))
            .collect(),
        };
        return Ok(View::typed(page).into_response());
    }
    json(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tx['name']` a template reads is a text of the crate: a missing
    /// one would show as a raw key on the page.
    #[test]
    fn every_text_a_dispatch_template_reads_exists() {
        for (name, source) in VIEWS {
            let mut rest = *source;
            while let Some(at) = rest.find("tx['") {
                rest = &rest[at + 4..];
                let end = rest.find('\'').expect("a closed key");
                let key = format!("{PREFIX}{}", &rest[..end]);
                assert!(
                    DISPATCH_TEXTS.iter().any(|(known, _)| *known == key),
                    "{name} reads `{key}`, which has no text"
                );
            }
        }
    }

    #[test]
    fn a_status_has_a_badge_colour_and_a_word() {
        assert_eq!(status_kind(DispatchStatus::Delivered), "success");
        assert_eq!(status_kind(DispatchStatus::Lost), "error");
        for status in DispatchStatus::ALL {
            let key = format!("{PREFIX}status.{}", status.as_str());
            assert!(
                DISPATCH_TEXTS.iter().any(|(known, _)| *known == key),
                "{key}"
            );
        }
    }
}
