//! The HTML pages of the supplier screens: the suppliers of an item, a
//! reorder suggestion, a supplier's score and the preferred-supplier
//! requests.
//!
//! These routes began as JSON endpoints and still answer JSON; they answer a
//! page when the request names `text/html` (a browser)
//! ([`wl_kernel::negotiate::wants_html`]). The templates are compiled into
//! the crate (`views/wl-inventory/`); an app replaces one with a file of the
//! same name under its own `resources/views/wl-inventory/`.
//!
//! Money in a template is fils, shown through the `money` filter in the
//! currency of the document it comes from; a price is left out for a person
//! without `stock.cost.view`, as in the JSON.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid};
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;

use crate::item::Item;
use crate::item_supplier::{LastPrice, PreferredSupplierRequest, RequestStatus};
use crate::location::Location;
use crate::preferred_change::{RequestView, SideView};
use crate::reorder::{ReorderSuggestion, SuggestionStatus};
use crate::screen_texts::SCREEN_TEXTS;
use crate::supplier_pages::{ScoreView, SuggestionView, SupplierView};
use crate::texts::{said, t};

/// An instant as `23 Oct 2026 22:15` in Asia/Dubai: the form every screen of
/// the suite uses.
pub(crate) fn dubai_text(at: renox::db::DateTime) -> String {
    at.with_timezone(
        &renox::chrono::FixedOffset::east_opt(wl_kernel::documents::DUBAI_OFFSET_SECONDS)
            .expect("UTC+4 is a valid offset"),
    )
    .format("%d %b %Y %H:%M")
    .to_string()
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "wl-inventory/item_suppliers.html",
        include_str!("../views/wl-inventory/item_suppliers.html"),
    ),
    (
        "wl-inventory/reorder_suggestion.html",
        include_str!("../views/wl-inventory/reorder_suggestion.html"),
    ),
    (
        "wl-inventory/supplier_score.html",
        include_str!("../views/wl-inventory/supplier_score.html"),
    ),
    (
        "wl-inventory/preferred_requests.html",
        include_str!("../views/wl-inventory/preferred_requests.html"),
    ),
    (
        "wl-inventory/preferred_request.html",
        include_str!("../views/wl-inventory/preferred_request.html"),
    ),
];

/// The texts a template reads as `tx['suppliers.rank']`.
type Texts = BTreeMap<&'static str, String>;

/// The prefix of the keys of the screens' texts.
const PREFIX: &str = "wl_inventory.screen.";

/// Every text of the screens, by the name a template reads
/// (`tx['suppliers.rank']` is the key `wl_inventory.screen.suppliers.rank`). A
/// page hands over all of them, so a template an app replaces may read any.
fn tx(lang: &Lang) -> Texts {
    SCREEN_TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix(PREFIX)?, t(lang, key))))
        .collect()
}

/// The word for `value` in `group` (`role`, `band`, `state`, `request_kind`).
fn word(lang: &Lang, group: &str, value: &str) -> String {
    let key = format!("wl_inventory.screen.{group}.{value}");
    let found = t(lang, &key);
    if found == key {
        value.replace('_', " ")
    } else {
        found
    }
}

/// The name of a supplier, else `#id`.
async fn party_name(state: &AppState, lang: &Lang, id: i64) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}

/// `12.5` for 12 500 thousandths, `3` for 3 000.
pub(crate) fn quantity_text(milli: i64) -> String {
    let sign = if milli < 0 { "-" } else { "" };
    let magnitude = milli.unsigned_abs();
    let (whole, fraction) = (magnitude / 1000, magnitude % 1000);
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        let fraction = format!("{fraction:03}");
        format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
    }
}

/// `92.50%`; a dash without a figure.
pub(crate) fn percent_text(basis_points: Option<i64>) -> String {
    match basis_points {
        Some(bp) => {
            let sign = if bp < 0 { "-" } else { "" };
            format!("{sign}{}.{:02}%", bp.abs() / 100, bp.abs() % 100)
        }
        None => "\u{2014}".to_owned(),
    }
}

/// The badge colour of a score band.
fn band_kind(band: &str) -> &'static str {
    match band {
        "good" => "success",
        "watch" => "warning",
        "below_threshold" => "error",
        _ => "info",
    }
}

/// The page of an item's suppliers.
pub(crate) fn item_url(id: i64) -> String {
    format!("/items/{id}/suppliers")
}

/// The page of a supplier's score.
pub(crate) fn score_url(party: i64) -> String {
    format!("/suppliers/{party}/score")
}

/// The page of a preferred-supplier request.
pub(crate) fn request_url(id: i64) -> String {
    format!("/preferred-requests/{id}")
}

/// A price as the templates read it.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Price {
    unit_price: i64,
    currency: String,
}

impl Price {
    fn of(price: &LastPrice) -> Self {
        Price {
            unit_price: price.unit_price,
            currency: price.currency.clone(),
        }
    }
}

/// A flag on a supplier.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Flag {
    text: String,
    kind: &'static str,
}

/// One supplier of an item.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct SupplierRow {
    name: String,
    score_url: Option<String>,
    role: String,
    role_kind: &'static str,
    rank: i64,
    lead_time_days: i64,
    min_order: String,
    code: Option<String>,
    price: Option<Price>,
    flags: Vec<Flag>,
}

/// What the suppliers page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/item_suppliers.html")]
pub(crate) struct SuppliersPage {
    tx: Texts,
    title: String,
    subtitle: String,
    rows: Vec<SupplierRow>,
    request_url: Option<String>,
    costs: bool,
}

/// What the person may follow from the suppliers page.
pub(crate) struct Reach {
    /// May open a supplier's score.
    pub scores: bool,
    /// May see the last prices.
    pub costs: bool,
    /// The pending request about the item, if the person may open it.
    pub request: Option<i64>,
}

/// The suppliers page of `item`.
pub(crate) async fn suppliers(
    state: &AppState,
    lang: &Lang,
    item: &Item,
    views: &[SupplierView],
    reach: &Reach,
) -> Response {
    let mut rows = Vec::with_capacity(views.len());
    for view in views {
        let mut flags = Vec::new();
        if view.on_hold {
            flags.push(Flag {
                text: t(lang, "wl_inventory.screen.suppliers.on_hold"),
                kind: "error",
            });
        }
        if view.watched {
            flags.push(Flag {
                text: t(lang, "wl_inventory.screen.suppliers.watched"),
                kind: "warning",
            });
        }
        if !view.active {
            flags.push(Flag {
                text: t(lang, "wl_inventory.screen.suppliers.inactive"),
                kind: "info",
            });
        }
        for kind in &view.warnings {
            flags.push(Flag {
                text: said("wl_inventory.screen.suppliers.missing", &[("kind", kind)]),
                kind: "warning",
            });
        }
        let unit = view.purchase_unit.clone().unwrap_or_default();
        rows.push(SupplierRow {
            name: party_name(state, lang, view.supplier_party_id).await,
            score_url: reach.scores.then(|| score_url(view.supplier_party_id)),
            role: word(lang, "role", view.role),
            role_kind: if view.role == "preferred" {
                "success"
            } else {
                "info"
            },
            rank: view.rank,
            lead_time_days: view.lead_time_days,
            min_order: if view.min_order_qty_milli == 0 {
                String::new()
            } else {
                format!("{} {}", quantity_text(view.min_order_qty_milli), unit)
                    .trim()
                    .to_owned()
            },
            code: view.supplier_item_code.clone(),
            price: view.last_price.as_ref().map(Price::of),
            flags,
        });
    }
    View::typed(SuppliersPage {
        tx: tx(lang),
        title: said(
            "wl_inventory.screen.suppliers.title",
            &[("item", &item.name_en)],
        ),
        subtitle: format!(
            "{} · {}",
            item.code,
            t(lang, "wl_inventory.screen.suppliers.subtitle")
        ),
        rows,
        request_url: reach.request.map(request_url),
        costs: reach.costs,
    })
    .into_response()
}

/// An alternative on a suggestion.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct AlternativeRow {
    name: String,
    score_url: Option<String>,
    role: String,
    lead_time_days: i64,
    price: Option<Price>,
    score: Option<i64>,
    on_hold: bool,
    warnings: Vec<String>,
}

/// What the suggestion page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/reorder_suggestion.html")]
pub(crate) struct ReorderPage {
    tx: Texts,
    title: String,
    subtitle: String,
    status: String,
    status_kind: &'static str,
    item_url: String,
    location: String,
    quantity: String,
    rounded_from: Option<String>,
    supplier: Option<String>,
    supplier_role: Option<String>,
    reason: Option<String>,
    lead_time_days: Option<i64>,
    warnings: Vec<String>,
    alternatives: Vec<AlternativeRow>,
}

/// The suggestion page of `suggestion` for `item`.
pub(crate) async fn reorder(
    state: &AppState,
    lang: &Lang,
    item: &Item,
    suggestion: &ReorderSuggestion,
    view: &SuggestionView,
    reach: &Reach,
) -> Result<Response> {
    let location = Location::find(&state.db, suggestion.location_id)
        .await?
        .map_or_else(
            || format!("#{}", suggestion.location_id),
            |location| format!("{} · {}", location.code, location.name_en),
        );
    let mut alternatives = Vec::with_capacity(view.alternatives.len());
    for other in &view.alternatives {
        alternatives.push(AlternativeRow {
            name: party_name(state, lang, other.supplier_party_id).await,
            score_url: reach.scores.then(|| score_url(other.supplier_party_id)),
            role: word(lang, "role", &other.role),
            lead_time_days: other.lead_time_days,
            price: match (other.last_price, &other.currency) {
                (Some(unit_price), Some(currency)) => Some(Price {
                    unit_price,
                    currency: currency.clone(),
                }),
                _ => None,
            },
            score: other.score,
            on_hold: other.on_hold,
            warnings: other.warnings.clone(),
        });
    }
    let supplier = match view.supplier_party_id {
        Some(id) => Some(party_name(state, lang, id).await),
        None => None,
    };
    let status_kind = match suggestion.status {
        SuggestionStatus::Open => "warning",
        SuggestionStatus::Converted => "success",
        SuggestionStatus::Dismissed => "info",
    };
    let unit = &item.base_unit;
    Ok(View::typed(ReorderPage {
        tx: tx(lang),
        title: said(
            "wl_inventory.screen.reorder.title",
            &[("id", &suggestion.id.to_string())],
        ),
        subtitle: format!("{} · {}", item.code, item.name_en),
        status: word(lang, "state", view.status),
        status_kind,
        item_url: item_url(item.id),
        location,
        quantity: format!("{} {unit}", quantity_text(view.suggested_qty_milli)),
        rounded_from: view
            .rounded_from_milli
            .map(|milli| format!("{} {unit}", quantity_text(milli))),
        supplier,
        supplier_role: view.supplier_role.as_deref().map(|r| word(lang, "role", r)),
        reason: view.reason.clone(),
        lead_time_days: view.lead_time_days,
        warnings: view.warnings.clone(),
        alternatives,
    })
    .into_response())
}

/// The numbers of one window.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Window {
    label: String,
    window: String,
    score: Option<i64>,
    band: String,
    band_kind: &'static str,
    receipts: i64,
    on_time: String,
    rejected: String,
    incidents: i64,
    debit_notes: i64,
    price_variance: String,
    received_value: Option<i64>,
    rejected_value: Option<i64>,
    debit_note_value: Option<i64>,
    notes: Vec<String>,
}

impl Window {
    fn of(lang: &Lang, view: &ScoreView, label: String) -> Self {
        Window {
            label,
            window: window_label(view.from, view.to),
            score: view.score,
            band: word(lang, "band", view.band),
            band_kind: band_kind(view.band),
            receipts: view.receipts,
            on_time: match view.on_time_bp {
                Some(bp) => format!(
                    "{} ({} / {})",
                    percent_text(Some(bp)),
                    view.on_time,
                    view.receipts
                ),
                None => percent_text(None),
            },
            rejected: percent_text(view.rejected_bp),
            incidents: view.incidents,
            debit_notes: view.debit_notes,
            price_variance: percent_text(view.price_variance_bp),
            received_value: view.received_value,
            rejected_value: view.rejected_value,
            debit_note_value: view.debit_note_value,
            notes: view.notes.clone(),
        }
    }
}

/// `2026-10-01 to 2026-10-31`.
fn window_label(from: NaiveDate, to: NaiveDate) -> String {
    format!("{from} \u{2192} {to}")
}

/// What the score page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/supplier_score.html")]
pub(crate) struct ScorePage {
    tx: Texts,
    title: String,
    subtitle: String,
    live: Window,
    snapshots: Vec<Window>,
    watched: bool,
    review_open: bool,
    costs: bool,
}

/// What the score page shows of a supplier.
pub(crate) struct ScoreFacts<'a> {
    /// The live window.
    pub live: &'a ScoreView,
    /// The monthly snapshots, newest first.
    pub snapshots: &'a [ScoreView],
    /// Whether the supplier is on the watch list.
    pub watched: bool,
    /// Whether a review of the supplier is open.
    pub review_open: bool,
    /// Whether the person may see the values (`stock.cost.view`).
    pub costs: bool,
}

/// The score page of `supplier`.
pub(crate) async fn score(
    state: &AppState,
    lang: &Lang,
    supplier: i64,
    facts: &ScoreFacts<'_>,
) -> Response {
    let ScoreFacts {
        live,
        snapshots,
        watched,
        review_open,
        costs,
    } = *facts;
    let name = party_name(state, lang, supplier).await;
    View::typed(ScorePage {
        tx: tx(lang),
        title: said("wl_inventory.screen.score.title", &[("supplier", &name)]),
        subtitle: t(lang, "wl_inventory.screen.score.subtitle"),
        live: Window::of(lang, live, t(lang, "wl_inventory.screen.score.live")),
        snapshots: snapshots
            .iter()
            .map(|view| {
                let day = view.snapshot_date.map(|day| day.to_string());
                Window::of(lang, view, day.unwrap_or_default())
            })
            .collect(),
        watched,
        review_open,
        costs,
    })
    .into_response()
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/preferred_requests.html")]
pub(crate) struct RequestsPage {
    tx: Texts,
    rows: renox::grid::GridPage<PreferredSupplierRequest>,
}

/// The requests grid: search on the item, filters by the change asked for.
/// The item links to its suppliers (`custom`); the suppliers are the names of
/// the parties.
pub(crate) fn requests_grid(lang: &Lang) -> Grid {
    let text = |key: &str| t(lang, &format!("wl_inventory.screen.{key}"));
    Grid::new("inventory-preferred-requests")
        .title(&text("requests.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::number("id", &text("requests.number"))
                .frozen()
                .mobile()
                .link("/preferred-requests/{id}"),
        )
        .column(Column::custom("item", &text("item")).mobile())
        .column(
            Column::related(
                "item_name",
                &text("item"),
                "wl_inventory_items",
                "item_id",
                "name_en",
            )
            .hidden()
            .searchable(),
        )
        .column(Column::select(
            "kind",
            &text("requests.kind"),
            ["replace", "deactivate"]
                .map(|kind| (kind.to_owned(), word(lang, "request_kind", kind))),
        ))
        .column(Column::custom("from", &text("requests.from")))
        .column(Column::custom("to", &text("requests.to")).mobile())
        .column(
            Column::select(
                "status",
                &text("requests.status"),
                ["pending"].map(|status| (status.to_owned(), word(lang, "state", status))),
            )
            .badges(&[("pending", "warning")])
            .mobile(),
        )
        .sort_by("id")
        .row_url("/preferred-requests/{id}")
        .cards_on_mobile()
        .empty_state(
            &text("requests.empty"),
            Some(&text("requests.empty_text")),
        )
}

/// The text of a request's status.
fn request_status(lang: &Lang, status: &str) -> (String, &'static str) {
    let kind = match status {
        "pending" => "warning",
        "approved" => "success",
        "rejected" => "error",
        _ => "info",
    };
    (word(lang, "state", status), kind)
}

/// The name of the item of a request: `CODE · Name`.
async fn item_label(state: &AppState, id: i64) -> Result<String> {
    Ok(Item::find(&state.db, id).await?.map_or_else(
        || format!("#{id}"),
        |item| format!("{} · {}", item.code, item.name_en),
    ))
}

/// The list of pending requests, a page of the grid.
pub(crate) async fn requests(
    state: &AppState,
    lang: &Lang,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    let grid = requests_grid(lang);
    let query = PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::STATUS.eq(RequestStatus::Pending));
    let page = grid.page(query, &request).await?;
    let nobody = t(lang, "wl_inventory.screen.request.none");
    let mut names = std::collections::HashMap::new();
    for found in page.items() {
        let supplier = |id: Option<i64>| {
            let nobody = nobody.clone();
            async move {
                match id.filter(|id| *id != 0) {
                    Some(id) => party_name(state, lang, id).await,
                    None => nobody,
                }
            }
        };
        names.insert(
            found.id,
            renox::serde_json::json!({
                "item_label": item_label(state, found.item_id).await?,
                "item_url": item_url(found.item_id),
                "from": supplier(Some(found.from_supplier_party_id)).await,
                "to": supplier(found.to_supplier_party_id).await,
            }),
        );
    }
    let page = page.extend(|found| names.get(&found.id).cloned().unwrap_or_default());
    Ok(View::typed(RequestsPage { tx: tx(lang), rows: page }).into_response())
}

/// One side of a request.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Side {
    name: String,
    score_url: Option<String>,
    lead_time_days: Option<i64>,
    on_hold: bool,
    score: Option<i64>,
    band: String,
    band_kind: &'static str,
    price: Option<Price>,
}

/// What the request page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/preferred_request.html")]
pub(crate) struct RequestPage {
    tx: Texts,
    id: i64,
    title: String,
    subtitle: String,
    status: String,
    status_kind: &'static str,
    kind: String,
    item: String,
    item_url: String,
    requested_by: String,
    decided_by: Option<String>,
    decision_reason: Option<String>,
    returned_note: Option<String>,
    from: Option<Side>,
    to: Option<Side>,
    can_decide: bool,
}

/// The name of the user `id`, else a dash.
async fn user_name(state: &AppState, id: Option<i64>, nobody: &str) -> String {
    match id {
        Some(id) => renox::auth::User::find(&state.db, id)
            .await
            .ok()
            .flatten()
            .map_or_else(|| format!("#{id}"), |user| user.name),
        None => nobody.to_owned(),
    }
}

/// The page of one request; `can_decide` shows the buttons of an approver.
pub(crate) async fn request(
    state: &AppState,
    lang: &Lang,
    view: &RequestView,
    reach: &Reach,
    can_decide: bool,
) -> Result<Response> {
    let side = |side: &Option<SideView>| {
        side.as_ref().map(|side| {
            (
                side.supplier_party_id,
                side.lead_time_days,
                side.on_hold,
                side.score,
                side.band,
                side.last_price.as_ref().map(Price::of),
            )
        })
    };
    let mut sides = Vec::new();
    for found in [side(&view.from), side(&view.to)] {
        sides.push(match found {
            Some((party, lead, on_hold, score, band, price)) => Some(Side {
                name: party_name(state, lang, party).await,
                score_url: reach.scores.then(|| score_url(party)),
                lead_time_days: lead,
                on_hold,
                score,
                band: word(lang, "band", band),
                band_kind: band_kind(band),
                price,
            }),
            None => None,
        });
    }
    let to = sides.pop().flatten();
    let from = sides.pop().flatten();
    let (status, status_kind) = request_status(lang, view.status);
    let nobody = t(lang, "wl_inventory.screen.requests.nobody");
    let decided_by = match view.decided_by {
        Some(id) => Some(user_name(state, Some(id), &nobody).await),
        None => None,
    };
    Ok(View::typed(RequestPage {
        tx: tx(lang),
        id: view.id,
        title: said(
            "wl_inventory.screen.request.title",
            &[("id", &view.id.to_string())],
        ),
        subtitle: item_label(state, view.item_id).await?,
        status,
        status_kind,
        kind: word(lang, "request_kind", &view.kind),
        item: item_label(state, view.item_id).await?,
        item_url: item_url(view.item_id),
        requested_by: user_name(state, view.requested_by, &nobody).await,
        decided_by,
        decision_reason: view.decision_reason.clone(),
        returned_note: view.returned_note.clone(),
        from,
        to,
        can_decide: can_decide && view.status == "pending",
    })
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tx['name']` a template reads is a text of the crate: a missing
    /// one would show as a raw key on the page.
    #[test]
    fn every_text_a_template_reads_exists() {
        for (name, source) in VIEWS {
            let mut rest = *source;
            while let Some(at) = rest.find("tx['") {
                rest = &rest[at + 4..];
                let end = rest.find('\'').expect("a closed key");
                let key = format!("wl_inventory.screen.{}", &rest[..end]);
                assert!(
                    SCREEN_TEXTS.iter().any(|(known, _)| *known == key),
                    "{name} reads `{key}`, which has no text"
                );
            }
        }
    }

    #[test]
    fn quantities_and_shares_read_as_numbers() {
        assert_eq!(quantity_text(3_000), "3");
        assert_eq!(quantity_text(12_500), "12.5");
        assert_eq!(quantity_text(-1_050), "-1.05");
        assert_eq!(percent_text(Some(9_250)), "92.50%");
        assert_eq!(percent_text(Some(-5)), "-0.05%");
        assert_eq!(percent_text(None), "\u{2014}");
    }
}
