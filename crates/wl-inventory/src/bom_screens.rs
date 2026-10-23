//! The bill of materials screens: the list of bills with a form to start
//! one, and one version with its components and the actions that change,
//! release and retire it (`STK.BOM.01`, 07 §7.1 *BOM*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /boms` | `wl_inventory.boms` | `boms.view`, `boms.manage` or `boms.release` anywhere; a page for a browser, JSON for the rest |
//! | `POST /boms` | `wl_inventory.bom_make` | `boms.manage`: starts a draft (303 to its page) |
//! | `GET /boms/{id}` | `wl_inventory.bom_page` | as the list |
//! | `POST /boms/{id}/edit` | `wl_inventory.bom_edit` | `boms.manage`: output, yield, hours of a draft |
//! | `POST /boms/{id}/lines` | `wl_inventory.bom_line_add` | `boms.manage`: a component of a draft |
//! | `POST /boms/{id}/lines/{line}/remove` | `wl_inventory.bom_line_remove` | `boms.manage` |
//! | `POST /boms/{id}/release` | `wl_inventory.bom_release` | `boms.release` |
//! | `POST /boms/{id}/obsolete` | `wl_inventory.bom_obsolete` | `boms.release` |
//! | `POST /boms/{id}/new-version` | `wl_inventory.bom_new_version` | `boms.manage`: a draft copy (303 to its page) |
//!
//! A button is shown to a user who holds the permission and only for the
//! states it applies to; a refusal of the service (a bill without a
//! component, a quantity that is not above zero) is the 422 sentence in the
//! sheet that sent the form. The bill has no scope: the permissions count
//! wherever the user holds them.

use std::collections::BTreeMap;

use crate::screens::dubai_text;
use renox::axum::http::HeaderMap;
use renox::axum::response::Redirect;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bom::{self, Bom, BomStatus};
use crate::bom_texts::BOM_TEXTS;
use crate::item::{Item, quantity_text};
use crate::permissions;
use crate::stock_screens::{amount, item_name};
use crate::texts::t;

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "wl-inventory/boms.html",
        include_str!("../views/wl-inventory/boms.html"),
    ),
    (
        "wl-inventory/bom.html",
        include_str!("../views/wl-inventory/bom.html"),
    ),
];

/// The texts a template reads as `tx['title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The prefix of the keys of these screens' texts.
pub(crate) const PREFIX: &str = "wl_inventory.bom.ui.";

/// Every text of the bill screens in `lang`, by the name a template reads.
pub(crate) fn tx(lang: &Lang) -> Texts {
    BOM_TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix(PREFIX)?, t(lang, key))))
        .collect()
}

/// The text `key` (without the prefix) in `lang`.
fn tr(lang: &Lang, key: &str) -> String {
    t(lang, &format!("{PREFIX}{key}"))
}

/// The word for a status.
fn status_word(lang: &Lang, status: BomStatus) -> String {
    tr(lang, &format!("status.{}", status.as_str()))
}

/// The badge colour of a status.
fn status_kind(status: BomStatus) -> &'static str {
    match status {
        BomStatus::Draft => "info",
        BomStatus::Released => "success",
        BomStatus::Obsolete => "warning",
    }
}

/// `fils` as `AED 1,200.50`.
fn money(fils: i64) -> String {
    format!("AED {}", crate::po_revise::money_text(fils))
}

/// Whether `user` sees what the materials cost.
fn sees_cost(user: &AuthUser) -> bool {
    bom::can_manage(user)
        || bom::may_release(user)
        || user.has_permission_anywhere(permissions::STOCK_COST_VIEW)
}

/// The address of the list.
const URL: &str = "/boms";

/// The address of the page of version `id`.
pub fn url(id: i64) -> String {
    format!("{URL}/{id}")
}

/// The routes of the bill screens.
pub fn routes() -> Routes {
    Routes::new()
        .get("/boms", list)
        .name("wl_inventory.boms")
        .post("/boms", make)
        .name("wl_inventory.bom_make")
        .get("/boms/{id}", show)
        .name("wl_inventory.bom_page")
        .post("/boms/{id}/edit", edit)
        .name("wl_inventory.bom_edit")
        .post("/boms/{id}/lines", add_line)
        .name("wl_inventory.bom_line_add")
        .post("/boms/{id}/lines/{line}/remove", remove_line)
        .name("wl_inventory.bom_line_remove")
        .post("/boms/{id}/release", release)
        .name("wl_inventory.bom_release")
        .post("/boms/{id}/obsolete", obsolete)
        .name("wl_inventory.bom_obsolete")
        .post("/boms/{id}/new-version", new_version)
        .name("wl_inventory.bom_new_version")
        .require_auth()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// `value` as a JSON answer.
fn json<T: Serialize>(value: &T) -> Result<Response> {
    let body = renox::serde_json::to_string(value)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// `4.5` hours or per cent in basis points or thousandths: a decimal with up
/// to three places in thousandths.
fn milli(text: &str) -> Option<i64> {
    crate::pages::parse_quantity(text)
}

/// A per cent like `92.5` in basis points; at most two places.
fn percent_bps(text: &str) -> Option<i64> {
    let thousandths = milli(text)?;
    (thousandths % 10 == 0).then_some(thousandths / 10)
}

/// Basis points as per cent: `9250` is `92.5`.
fn percent_text(bps: i64) -> String {
    quantity_text(bps * 10)
}

/// One version in the list.
#[derive(Serialize, Debug, Clone)]
pub struct BomRow {
    id: i64,
    url: String,
    item: String,
    version: String,
    lines: i64,
    status: String,
    status_code: String,
    status_kind: &'static str,
    cost: String,
    makes: String,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/boms.html")]
pub(crate) struct BomsPage {
    tx: Texts,
    rows: Vec<BomRow>,
    can_manage: bool,
    show_cost: bool,
    items: Vec<(String, String)>,
    status_tabs: Vec<(String, String)>,
    status_current: String,
}

/// The list's filter.
#[derive(Deserialize, Default)]
pub(crate) struct ListFilter {
    /// A status key.
    #[serde(default)]
    status: String,
}

/// The items to choose from, as options of a select.
async fn item_options(
    state: &AppState,
    lang: &Lang,
    leave_out: Option<i64>,
) -> Result<Vec<(String, String)>> {
    let mut options = Vec::new();
    for item in Item::query()
        .order_by(Item::NAME_EN)
        .limit(500)
        .get(&state.db)
        .await?
    {
        if Some(item.id) != leave_out {
            options.push((
                item.id.to_string(),
                format!("{} · {}", item.code, item_name(&item, lang)),
            ));
        }
    }
    Ok(options)
}

/// `GET /boms`.
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<ListFilter>,
) -> Result<Response> {
    if !bom::can_view(&user) {
        return Err(Error::Forbidden);
    }
    let show_cost = sees_cost(&user);
    let mut rows = Vec::new();
    for found in Bom::query()
        .order_by_desc(Bom::ID)
        .limit(500)
        .get(&state.db)
        .await?
    {
        if !filter.status.is_empty() && found.status.as_str() != filter.status {
            continue;
        }
        let item = Item::find_or_404(&state.db, found.item_id).await?;
        let lines = bom::lines_of(&state.db, found.id).await?.len();
        rows.push(BomRow {
            id: found.id,
            url: url(found.id),
            item: format!("{} · {}", item.code, item_name(&item, &lang)),
            version: format!("v{}", found.version),
            lines: i64::try_from(lines).unwrap_or(0),
            status: status_word(&lang, found.status),
            status_code: found.status.as_str().to_owned(),
            status_kind: status_kind(found.status),
            cost: if !show_cost {
                String::new()
            } else if found.status == BomStatus::Draft {
                tr(&lang, "cost_pending")
            } else {
                money(found.material_cost_per_unit)
            },
            makes: amount(found.output_milli, &item.base_unit),
        });
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return json(&rows);
    }
    let mut tabs = vec![(URL.to_owned(), tr(&lang, "all"))];
    for status in ["draft", "released", "obsolete"] {
        tabs.push((
            format!("{URL}?status={status}"),
            tr(&lang, &format!("status.{status}")),
        ));
    }
    Ok(View::typed(BomsPage {
        tx: tx(&lang),
        rows,
        can_manage: bom::can_manage(&user),
        show_cost,
        items: item_options(&state, &lang, None).await?,
        status_tabs: tabs,
        status_current: if filter.status.is_empty() {
            URL.to_owned()
        } else {
            format!("{URL}?status={}", filter.status)
        },
    })
    .into_response())
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

    fn unless(reason: Option<String>) -> Self {
        Act {
            shown: true,
            blocked: reason,
        }
    }
}

/// The buttons of a version.
#[derive(Serialize, Debug, Clone)]
pub struct Actions {
    edit: Act,
    add_line: Act,
    remove_line: Act,
    release: Act,
    obsolete: Act,
    new_version: Act,
}

/// One component on the record page.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    id: i64,
    position: i64,
    item: String,
    quantity: String,
    scrap: String,
    per_unit: String,
    unit: String,
}

/// A version of the same item, for the switcher.
#[derive(Serialize, Debug, Clone)]
pub struct VersionRow {
    id: i64,
    url: String,
    version: String,
    status: String,
    status_kind: &'static str,
    current: bool,
}

/// What the record page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/bom.html")]
pub(crate) struct BomPage {
    tx: Texts,
    id: i64,
    title: String,
    item: String,
    unit: String,
    status: String,
    status_code: String,
    status_kind: &'static str,
    makes: String,
    makes_input: String,
    yield_text: String,
    yield_input: String,
    hours: String,
    hours_input: String,
    show_cost: bool,
    cost: String,
    released_at: String,
    obsoleted_at: String,
    alerts: Vec<String>,
    lines: Vec<LineRow>,
    versions: Vec<VersionRow>,
    actions: Actions,
    components: Vec<(String, String)>,
}

/// `GET /boms/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !bom::can_view(&user) {
        return Err(Error::Forbidden);
    }
    let found = Bom::find_or_404(&state.db, id).await?;
    let item = Item::find_or_404(&state.db, found.item_id).await?;
    let lines = bom::lines_of(&state.db, id).await?;
    if !wl_kernel::negotiate::wants_html(&headers) {
        let version = bom::version_of(&state.db, id).await?;
        return json(&renox::serde_json::json!({
            "bom": found,
            "lines": lines,
            "version": version,
        }));
    }
    let mut rows = Vec::new();
    for line in &lines {
        let component = Item::find_or_404(&state.db, line.item_id).await?;
        rows.push(LineRow {
            id: line.id,
            position: line.position,
            item: format!("{} · {}", component.code, item_name(&component, &lang)),
            quantity: amount(line.quantity_milli, &component.base_unit),
            scrap: format!("{}%", percent_text(line.scrap_bps)),
            per_unit: amount(bom::per_unit_quantity(&found, line), &component.base_unit),
            unit: component.base_unit.clone(),
        });
    }
    let mut versions = Vec::new();
    for other in Bom::query()
        .where_(Bom::ITEM_ID.eq(found.item_id))
        .order_by_desc(Bom::VERSION)
        .get(&state.db)
        .await?
    {
        versions.push(VersionRow {
            id: other.id,
            url: url(other.id),
            version: format!("v{}", other.version),
            status: status_word(&lang, other.status),
            status_kind: status_kind(other.status),
            current: other.id == id,
        });
    }
    let has_draft = versions
        .iter()
        .any(|row| row.status == status_word(&lang, BomStatus::Draft));
    let draft = found.status == BomStatus::Draft;
    let manage = bom::can_manage(&user);
    let release = bom::may_release(&user);
    let not_draft = (!draft).then(|| tr(&lang, "blocked_draft"));
    let alerts = if draft && release {
        bom::release_problems(&state.db, &found).await?
    } else {
        Vec::new()
    };
    let actions = Actions {
        edit: if manage {
            Act::unless(not_draft.clone())
        } else {
            Act::hidden()
        },
        add_line: if manage && draft {
            Act::unless(None)
        } else {
            Act::hidden()
        },
        remove_line: if manage && draft {
            Act::unless(None)
        } else {
            Act::hidden()
        },
        release: if release && draft {
            Act::unless(None)
        } else {
            Act::hidden()
        },
        obsolete: if release && found.status == BomStatus::Released {
            Act::unless(None)
        } else {
            Act::hidden()
        },
        new_version: if manage && !draft {
            Act::unless(has_draft.then(|| tr(&lang, "blocked_has_draft")))
        } else {
            Act::hidden()
        },
    };
    let title = format!("{} · v{}", item_name(&item, &lang), found.version);
    Ok(View::typed(BomPage {
        tx: tx(&lang),
        id,
        title,
        item: format!("{} · {}", item.code, item_name(&item, &lang)),
        unit: item.base_unit.clone(),
        status: status_word(&lang, found.status),
        status_code: found.status.as_str().to_owned(),
        status_kind: status_kind(found.status),
        makes: amount(found.output_milli, &item.base_unit),
        makes_input: quantity_text(found.output_milli),
        yield_text: format!("{}%", percent_text(found.yield_bps)),
        yield_input: percent_text(found.yield_bps),
        hours: format!("{} h", quantity_text(found.standard_milli_hours)),
        hours_input: quantity_text(found.standard_milli_hours),
        show_cost: sees_cost(&user),
        cost: if draft {
            tr(&lang, "cost_pending")
        } else {
            money(found.material_cost_per_unit)
        },
        released_at: found.released_at.map(dubai_text).unwrap_or_default(),
        obsoleted_at: found.obsoleted_at.map(dubai_text).unwrap_or_default(),
        alerts,
        lines: rows,
        versions,
        actions,
        components: item_options(&state, &lang, Some(found.item_id)).await?,
    })
    .into_response())
}

/// What a user enters for the header of a bill.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct HeaderForm {
    /// The id of the item made (a new bill).
    #[validate(max = 20)]
    pub item: Option<String>,
    /// The quantity the components make.
    #[validate(max = 32)]
    pub output: Option<String>,
    /// The yield in per cent.
    #[validate(max = 32)]
    pub yield_percent: Option<String>,
    /// The standard hours.
    #[validate(max = 32)]
    pub hours: Option<String>,
}

fn present(field: &Option<String>) -> Option<&str> {
    field
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The numbers of a header: `(output, yield, hours)`, the defaults for an
/// empty field.
fn header_numbers(lang: &Lang, form: &HeaderForm) -> Result<(i64, i64, i64)> {
    let bad = || refused(tr(lang, "problem.number"));
    let output = match present(&form.output) {
        Some(text) => milli(text).ok_or_else(bad)?,
        None => 1_000,
    };
    let yield_bps = match present(&form.yield_percent) {
        Some(text) => percent_bps(text).ok_or_else(bad)?,
        None => 10_000,
    };
    let hours = match present(&form.hours) {
        Some(text) => milli(text).ok_or_else(bad)?,
        None => 0,
    };
    Ok((output, yield_bps, hours))
}

/// A toast and a refresh of the page the form was sent from.
fn done(lang: &Lang, key: &str) -> Response {
    (
        Toast::success(tr(lang, &format!("done.{key}"))),
        renox::HxRefresh,
    )
        .into_response()
}

/// A toast and a 303 to `path`.
fn done_at(lang: &Lang, key: &str, path: &str) -> Response {
    (
        Toast::success(tr(lang, &format!("done.{key}"))),
        Redirect::to(path),
    )
        .into_response()
}

/// `POST /boms`: starts the draft of an item.
async fn make(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<HeaderForm>,
) -> Result<Response> {
    if !bom::can_manage(&user) {
        return Err(Error::Forbidden);
    }
    let item_id = present(&form.item)
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| refused(tr(&lang, "problem.item")))?;
    let (output_milli, yield_bps, standard_milli_hours) = header_numbers(&lang, &form)?;
    let made = bom::create_draft(
        &state.db,
        &user,
        bom::NewBom {
            item_id,
            output_milli,
            yield_bps,
            standard_milli_hours,
        },
    )
    .await?;
    Ok(done_at(&lang, "made", &url(made.id)))
}

/// `POST /boms/{id}/edit`: the header of a draft.
async fn edit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<HeaderForm>,
) -> Result<Response> {
    let (output, yield_bps, hours) = header_numbers(&lang, &form)?;
    bom::update_draft(&state.db, &user, id, output, yield_bps, hours).await?;
    Ok(done(&lang, "saved"))
}

/// What a user enters for a component.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct LineForm {
    /// The id of the component item.
    #[validate(max = 20)]
    pub item: Option<String>,
    /// The quantity for what the bill makes.
    #[validate(max = 32)]
    pub quantity: Option<String>,
    /// The scrap in per cent.
    #[validate(max = 32)]
    pub scrap: Option<String>,
}

/// `POST /boms/{id}/lines`: adds a component to a draft.
async fn add_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<LineForm>,
) -> Result<Response> {
    let item_id = present(&form.item)
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| refused(tr(&lang, "problem.item")))?;
    let quantity = present(&form.quantity)
        .and_then(milli)
        .ok_or_else(|| refused(tr(&lang, "problem.number")))?;
    let scrap = match present(&form.scrap) {
        Some(text) => percent_bps(text).ok_or_else(|| refused(tr(&lang, "problem.number")))?,
        None => 0,
    };
    bom::add_line(&state.db, &user, id, item_id, quantity, scrap).await?;
    Ok(done(&lang, "line_added"))
}

/// `POST /boms/{id}/lines/{line}/remove`.
async fn remove_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, line)): Path<(i64, i64)>,
) -> Result<Response> {
    bom::remove_line(&state.db, &user, id, line).await?;
    Ok(done(&lang, "line_removed"))
}

/// `POST /boms/{id}/release`.
async fn release(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    bom::release(&state, &user, id).await?;
    Ok(done(&lang, "released"))
}

/// `POST /boms/{id}/obsolete`.
async fn obsolete(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    bom::make_obsolete(&state, &user, id).await?;
    Ok(done(&lang, "obsolete"))
}

/// `POST /boms/{id}/new-version`.
async fn new_version(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let made = bom::new_version(&state.db, &user, id).await?;
    Ok(done_at(&lang, "new_version", &url(made.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tx['name']` a template reads is a text of the crate: a missing
    /// one would show as a raw key on the page.
    #[test]
    fn every_text_a_bom_template_reads_exists() {
        for (name, source) in VIEWS {
            let mut rest = *source;
            while let Some(at) = rest.find("tx['") {
                rest = &rest[at + 4..];
                let end = rest.find('\'').expect("a closed key");
                let key = format!("{PREFIX}{}", &rest[..end]);
                assert!(
                    BOM_TEXTS.iter().any(|(known, _)| *known == key),
                    "{name} reads `{key}`, which has no text"
                );
            }
        }
    }

    #[test]
    fn a_percent_is_kept_in_basis_points() {
        assert_eq!(percent_bps("92.5"), Some(9_250));
        assert_eq!(percent_bps("5"), Some(500));
        assert_eq!(percent_bps("92.555"), None);
        assert_eq!(percent_text(9_250), "92.5");
    }
}
