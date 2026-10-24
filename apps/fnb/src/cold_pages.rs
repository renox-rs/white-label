//! The cold unit screens (06 F&B §4.6, §7.2 `FNB.RUL.05`; BL-FNB-12): the units
//! of the outlets with their last reading and the hold they are on, the sheet
//! that sets a unit up (with its stock location and range) and the sheet that
//! records a reading; a unit's page with its readings, the lines of its hold
//! and the sheet where the chef decides each line (release, discard or return
//! to the supplier) with the reading and the minutes out of range.
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /cold-units` | `fnb.cold_units` | `temperature.view` |
//! | `POST /cold-units` | `fnb.cold_unit_create` | `cold_units.manage`, fields `code`, `name`, `kind`, `location`, `min`, `max` |
//! | `GET /cold-units/{id}` | `fnb.cold_unit` | `temperature.view` |
//! | `POST /cold-units/{id}/readings` | `fnb.cold_unit_reading` | `temperature.log`, fields `temperature`, `note` |
//! | `POST /cold-units/{id}/lines/{line}/decide` | `fnb.cold_unit_decide` | `lots.release` (the stock module's), fields `decision`, `reading`, `minutes` |
//!
//! A refused action answers 422 with the reason; the decision of a line needs
//! the reading and the minutes.

use renox::grid::{Column, Grid, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::inventory::stock_hold_service;

use crate::catalogue::Outlet;
use crate::cold::{
    self, ColdUnit, DecideAsk, Decided, HoldDecisionRecord, TemperatureReading, UnitAsk, kind,
};
use crate::money::{milli_text, parse_tenths, tenths_text};
use crate::pages::{Fields, Texts, field, tx};
use crate::permissions;
use crate::placing::refused;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "fnb/cold_units.html",
        include_str!("../views/fnb/cold_units.html"),
    ),
    (
        "fnb/cold_unit.html",
        include_str!("../views/fnb/cold_unit.html"),
    ),
];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/cold-units", index)
        .name("fnb.cold_units")
        .post("/cold-units", create)
        .name("fnb.cold_unit_create")
        .get("/cold-units/{id}", show)
        .name("fnb.cold_unit")
        .post("/cold-units/{id}/readings", reading)
        .name("fnb.cold_unit_reading")
        .post("/cold-units/{id}/lines/{line}/decide", decide)
        .name("fnb.cold_unit_decide")
        .require_auth()
}

fn kind_options(lang: &Lang) -> Vec<[String; 2]> {
    kind::ALL
        .iter()
        .map(|k| {
            [
                (*k).to_owned(),
                crate::texts::t(lang, &format!("fnb.screen.cold.kind.{k}")),
            ]
        })
        .collect()
}

/// The cold units grid: search on the name and code, filter by kind. The
/// range, the last reading, the hold and the record button are drawn by the
/// template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = tx(lang);
    let kinds: Vec<(String, String)> = kind::ALL
        .iter()
        .map(|k| {
            (
                (*k).to_owned(),
                crate::texts::t(lang, &format!("fnb.screen.cold.kind.{k}")),
            )
        })
        .collect();
    let grid = Grid::new("fnb-cold-units")
        .title(&t["cold.title"])
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("name", &t["cold.name"])
                .frozen()
                .mobile()
                .searchable()
                .link("/cold-units/{id}"),
        )
        .column(Column::text("code", &t["cold.code"]).searchable())
        .column(Column::select("kind", &t["cold.kind"], kinds).mobile())
        .column(Column::custom("range", &t["cold.range"]))
        .column(Column::custom("reading", &t["cold.reading"]).mobile())
        .column(Column::custom("read_at", &t["cold.read_at"]))
        .column(Column::datetime("out_since", &t["cold.out_since"]).hidden())
        .column(Column::custom("state", &t["state"]).mobile())
        .column(Column::custom("actions", &t["actions"]))
        .sort_by("code")
        .row_url("/cold-units/{id}")
        .cards_on_mobile()
        .empty_state(&t["cold.empty"], Some(&t["cold.empty_text"]));
    if can_export { grid.exports() } else { grid }
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/cold_units.html")]
pub(crate) struct ColdUnitsPage {
    tx: Texts,
    units: renox::grid::GridPage<ColdUnit>,
    sheets: Vec<i64>,
    kinds: Vec<[String; 2]>,
    locations: Vec<[String; 2]>,
    can_manage: bool,
    can_log: bool,
}

async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::TEMPERATURE_VIEW) {
        return Err(Error::Forbidden);
    }
    let can_manage = user.has_permission_anywhere(permissions::COLD_UNITS_MANAGE);
    let grid = grid(&lang, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    let active = ColdUnit::query().where_(ColdUnit::ACTIVE.eq(true));
    if let Some(file) = grid.export(active.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(active, &request).await?;
    // The last reading and the hold of each unit of the page.
    let mut latest = std::collections::BTreeMap::new();
    for unit in page.items() {
        let last = TemperatureReading::of_unit(&state.db, unit.id, 1)
            .await?
            .pop();
        let held = cold::on_hold(&state, unit).await?;
        latest.insert(unit.id, (last, held));
    }
    let sheets = page.items().iter().map(|unit| unit.id).collect();
    let units = page.extend(|unit| {
        let (last, held) = match latest.get(&unit.id) {
            Some((last, held)) => (last.as_ref(), *held),
            None => (None, false),
        };
        json!({
            "range": unit.range_text(),
            "reading": last.map(|r| tenths_text(r.tenths)).unwrap_or_default(),
            "reading_kind": match last {
                Some(r) if r.in_range => "success",
                Some(_) => "error",
                None => "info",
            },
            "read_at": last
                .map(|r| wl_kernel::documents::dubai_text(r.recorded_at))
                .unwrap_or_default(),
            "on_hold": held,
            "ticket": unit.ticket_number.clone().unwrap_or_default(),
        })
    });
    let locations = stock_hold_service(&state)
        .locations(&state)
        .await?
        .into_iter()
        .map(|l| {
            [
                l.location.subject_id.to_string(),
                format!("{} / {}", l.code, l.name),
            ]
        })
        .collect();
    Ok(View::typed(ColdUnitsPage {
        tx: tx(&lang),
        units,
        sheets,
        kinds: kind_options(&lang),
        locations,
        can_manage,
        can_log: user.has_permission_anywhere(permissions::TEMPERATURE_LOG),
    })
    .into_response())
}

/// `POST /cold-units`: sets a unit up.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Fields,
) -> Result<Response> {
    let tenths = |name: &str| -> Result<Option<i64>> {
        let text = field(&form, name);
        if text.trim().is_empty() {
            return Ok(None);
        }
        parse_tenths(&text)
            .map(Some)
            .ok_or_else(|| refused(t(&lang, "fnb.cold.problem.temperature")))
    };
    let ask = UnitAsk {
        code: field(&form, "code"),
        name: field(&form, "name"),
        kind: field(&form, "kind"),
        outlet_id: Outlet::query()
            .order_by(Outlet::ID)
            .first(&state.db)
            .await?
            .map(|outlet| outlet.id),
        location_id: field(&form, "location").trim().parse().unwrap_or(0),
        min_tenths: tenths("min")?,
        max_tenths: tenths("max")?,
    };
    let unit = cold::create_unit(&state, &user, &ask).await?;
    Ok((
        Toast::success(t(&lang, "fnb.cold.created")),
        Redirect::route("fnb.cold_unit", &[&unit.id])?,
    )
        .into_response())
}

/// One reading of a unit's page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ReadingRow {
    tenths: String,
    in_range: bool,
    kind: &'static str,
    at: String,
    note: String,
}

/// One line of the unit's hold.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    id: i64,
    item: String,
    quantity: String,
    value: i64,
    status: String,
    status_kind: &'static str,
    undecided: bool,
    waiting: bool,
    reading: String,
    minutes: String,
}

/// What a unit's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "fnb/cold_unit.html")]
pub(crate) struct ShowColdUnitPage {
    tx: Texts,
    unit: ColdUnit,
    kind_label: String,
    range: String,
    out_since: String,
    on_hold: bool,
    hold_number: String,
    hold_open: bool,
    readings: Vec<ReadingRow>,
    lines: Vec<LineRow>,
    decisions: Vec<[String; 2]>,
    notes: Vec<String>,
    can_log: bool,
    can_decide: bool,
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::TEMPERATURE_VIEW) {
        return Err(Error::Forbidden);
    }
    let unit = ColdUnit::find_or_404(&state.db, id).await?;
    let readings = TemperatureReading::of_unit(&state.db, unit.id, 30)
        .await?
        .into_iter()
        .map(|r| ReadingRow {
            tenths: tenths_text(r.tenths),
            in_range: r.in_range,
            kind: if r.in_range { "success" } else { "error" },
            at: wl_kernel::documents::dubai_text(r.recorded_at),
            note: r.note,
        })
        .collect();
    let hold = cold::hold_of(&state, &unit).await?;
    let mut lines = Vec::new();
    if let Some(view) = &hold {
        for line in &view.lines {
            let record = HoldDecisionRecord::of_line(&state.db, line.line.subject_id).await?;
            let undecided = line.status == "undecided";
            lines.push(LineRow {
                id: line.line.subject_id,
                item: line.item_name.clone(),
                quantity: milli_text(line.quantity_milli),
                value: line.value,
                status: t(&lang, &format!("fnb.screen.cold.line.{}", line.status)),
                status_kind: match line.status.as_str() {
                    "undecided" => "warning",
                    "released" => "success",
                    _ => "info",
                },
                undecided,
                waiting: undecided && record.as_ref().is_some_and(|r| r.waiting),
                reading: record
                    .as_ref()
                    .map(|r| tenths_text(r.reading_tenths))
                    .unwrap_or_default(),
                minutes: record
                    .as_ref()
                    .map(|r| r.minutes_out.to_string())
                    .unwrap_or_default(),
            });
        }
    }
    let on_hold = hold.as_ref().is_some_and(|view| view.open);
    let mut notes = Vec::new();
    if on_hold {
        notes.push(t(&lang, "fnb.screen.cold.note_on_hold"));
    }
    let decisions = cold::decision::ALL
        .iter()
        .map(|d| {
            [
                (*d).to_owned(),
                t(&lang, &format!("fnb.screen.cold.decision.{d}")),
            ]
        })
        .collect();
    Ok(View::typed(ShowColdUnitPage {
        tx: tx(&lang),
        kind_label: t(&lang, &format!("fnb.screen.cold.kind.{}", unit.kind)),
        range: unit.range_text(),
        out_since: unit
            .out_since
            .map(wl_kernel::documents::dubai_text)
            .unwrap_or_default(),
        on_hold,
        hold_number: hold.as_ref().map(|v| v.number.clone()).unwrap_or_default(),
        hold_open: on_hold,
        readings,
        lines,
        decisions,
        notes,
        can_log: user.has_permission_anywhere(permissions::TEMPERATURE_LOG),
        can_decide: user.has_permission_anywhere(permissions::LOTS_RELEASE),
        unit,
    })
    .into_response())
}

/// `POST /cold-units/{id}/readings`: records a reading.
async fn reading(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let tenths = parse_tenths(&field(&form, "temperature"))
        .ok_or_else(|| refused(t(&lang, "fnb.cold.problem.temperature")))?;
    let recorded = cold::record(&state, &user, id, tenths, &field(&form, "note")).await?;
    let key = if recorded.in_range {
        "fnb.cold.recorded"
    } else {
        "fnb.cold.recorded_out_of_range"
    };
    Ok((
        Toast::success(t(&lang, key)),
        Redirect::route("fnb.cold_unit", &[&id])?,
    )
        .into_response())
}

/// `POST /cold-units/{id}/lines/{line}/decide`: the chef decides a line.
async fn decide(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, line)): Path<(i64, i64)>,
    Form(form): Fields,
) -> Result<Response> {
    let reading = field(&form, "reading");
    let minutes = field(&form, "minutes");
    let ask = DecideAsk {
        decision: field(&form, "decision"),
        reading_tenths: parse_tenths(&reading),
        minutes_out: minutes.trim().parse().ok(),
    };
    let key = match cold::decide(&state, &user, id, line, &ask).await? {
        Decided::Done => "fnb.cold.decided",
        Decided::Waiting => "fnb.cold.decision_waiting",
    };
    Ok((
        Toast::success(t(&lang, key)),
        Redirect::route("fnb.cold_unit", &[&id])?,
    )
        .into_response())
}
