//! The vehicle pages (08 `AUT.VEH.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/vehicles` | `auto.vehicles` | `vehicles.view` |
//! | `POST /auto/vehicles` | `auto.vehicle_create` | `vehicles.manage` |
//! | `GET /auto/vehicles/{id}` | `auto.vehicle` | `vehicles.view` |
//! | `POST /auto/vehicles/{id}` | `auto.vehicle_update` | `vehicles.manage` |
//! | `GET /auto/parties/options` | `auto.party_options` | `vehicles.manage`, `job_cards.advise`, `insurance.manage`, `fleet_contracts.manage` or `supplier_claims.manage` |
//!
//! The list is searched by plate, VIN, make or model and narrowed to fleet
//! or private vehicles with link tabs. The owner is picked from the parties
//! (`PartyDirectory::search`), never typed as an id; a party code such as
//! `P-00023` is accepted too.

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use super::lists;
use super::{
    Texts, parse_km, parse_number, parse_optional_id, party_choices, party_name, party_of, require,
    text,
};
use crate::fleet_contract::FleetContract;
use crate::number_text;
use crate::permissions;
use crate::texts;
use crate::vehicle::{Emirate, Vehicle};
use crate::vehicles::{self, NewVehicle};

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/vehicles", index)
        .name("auto.vehicles")
        .post("/auto/vehicles", create)
        .name("auto.vehicle_create")
        .get("/auto/parties/options", party_options)
        .name("auto.party_options")
        .get("/auto/vehicles/{id}", show)
        .name("auto.vehicle")
        .post("/auto/vehicles/{id}", update)
        .name("auto.vehicle_update")
        .require_auth()
}

/// The odometer as people read it: `52,000 km`.
pub(crate) fn km_text(km: i64) -> String {
    format!("{} km", number_text(km))
}

/// The emirates as options, `[stored name, name on a plate]`.
fn emirate_options() -> Vec<[String; 2]> {
    Emirate::FORM_ORDER
        .into_iter()
        .map(|emirate| [emirate.as_str().to_owned(), emirate.label().to_owned()])
        .collect()
}

/// `Toyota Hilux 2022`.
pub(crate) fn vehicle_text(vehicle: &Vehicle) -> String {
    match vehicle.model_year {
        Some(year) => format!("{} {} {year}", vehicle.make, vehicle.model_name),
        None => format!("{} {}", vehicle.make, vehicle.model_name),
    }
}

/// The vehicles grid: the plate first and frozen, what the vehicle is, the
/// VIN, whose it is, private or fleet, and the odometer in kilometres;
/// searched by plate, VIN, make or model.
pub(crate) fn grid(t: lists::Text<'_>) -> Grid {
    let kinds = lists::options(t, "vehicles.kind", ["fleet", "private"]);
    lists::baseline(Grid::new("auto-vehicles"))
        .title(&t("vehicles.title"))
        .column(
            Column::text("plate", &t("vehicles.plate"))
                .computed(&lists::own_plate_sql())
                .frozen()
                .mobile()
                .searchable()
                .link("/auto/vehicles/{id}"),
        )
        .column(
            Column::text("vehicle", &t("vehicles.make"))
                .computed(
                    "{T}.\"make\" || ' ' || {T}.\"model_name\" || \
                     CASE WHEN {T}.\"model_year\" IS NULL THEN '' \
                     ELSE ' ' || CAST({T}.\"model_year\" AS TEXT) END",
                )
                .mobile()
                .searchable(),
        )
        .column(
            Column::text("vin", &t("vehicles.vin"))
                .searchable()
                .copyable(),
        )
        .column(Column::custom("owner", &t("vehicles.owner")).mobile())
        .column(
            Column::select("kind", &t("vehicles.kind"), kinds)
                .computed(
                    "CASE WHEN {T}.\"fleet_contract_id\" IS NULL THEN 'private' ELSE 'fleet' END",
                )
                .badges(&[("fleet", "info"), ("private", "neutral")]),
        )
        .column(Column::number("odometer_km", &t("vehicles.odometer")).mobile())
        .sort_by("plate")
        .row_url("/auto/vehicles/{id}")
        .cards_on_mobile()
        .empty_state(&t("vehicles.empty"), Some(&t("vehicles.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/vehicles.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    vehicles: GridPage<Vehicle>,
    emirate_options: Vec<[String; 2]>,
    can_manage: bool,
}

/// `GET /auto/vehicles`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    require(&user, permissions::VEHICLES_VIEW)?;
    let can_export = user.has_permission(permissions::VEHICLES_MANAGE);
    let grid = lists::exporting(grid(&lists::texts_in(&lang)), can_export);
    if let Some(file) = lists::export_file(&grid, can_export, Vehicle::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(Vehicle::query(), &request).await?;
    let ids: Vec<i64> = page.items().iter().map(|v| v.owner_party_id).collect();
    let names = lists::party_names(&state, &lang, ids).await;
    let vehicles = page.extend(|vehicle| {
        json!({ "owner": names.get(&vehicle.owner_party_id).cloned().unwrap_or_default() })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        vehicles,
        emirate_options: emirate_options(),
        can_manage: user.has_permission(permissions::VEHICLES_MANAGE),
    })
    .into_response())
}

/// `GET /auto/parties/options`: the parties the picker offers as the person
/// types (`?q=`), or the labels of the values it holds (`?values=`).
async fn party_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    if !(user.has_permission(permissions::VEHICLES_MANAGE)
        || user.has_permission(permissions::JOB_CARDS_ADVISE)
        || user.has_permission(permissions::INSURANCE_MANAGE)
        || user.has_permission(permissions::FLEET_CONTRACTS_MANAGE)
        || user.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE))
    {
        return Err(Error::Forbidden);
    }
    Ok(Json(party_choices(&state, &query).await?))
}

/// The form that registers or changes a vehicle.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct VehicleForm {
    /// The emirate of the plate.
    #[serde(default)]
    pub emirate: String,
    /// The plate code.
    #[serde(default)]
    pub plate_code: String,
    /// The plate number.
    #[serde(default)]
    pub plate_number: String,
    /// The VIN.
    #[serde(default)]
    pub vin: String,
    /// The make.
    #[serde(default)]
    pub make: String,
    /// The model.
    #[serde(default)]
    pub model_name: String,
    /// The model year.
    #[serde(default)]
    pub model_year: String,
    /// The engine.
    #[serde(default)]
    pub engine: String,
    /// The colour.
    #[serde(default)]
    pub colour: String,
    /// The owner, picked from the parties.
    #[serde(default)]
    pub owner: String,
    /// The fleet contract.
    #[serde(default)]
    pub fleet_contract: String,
    /// The odometer reading in kilometres.
    #[serde(default)]
    pub odometer: String,
}

/// The vehicle the form asks for.
async fn new_vehicle(state: &AppState, form: &VehicleForm) -> Result<NewVehicle> {
    let emirate = Emirate::parse(&form.emirate)
        .ok_or_else(|| super::invalid(&texts::english("auto.vehicles.problem.emirate", &[])))?;
    let owner = party_of(state, &form.owner).await?;
    let mut new = NewVehicle::new(
        emirate,
        &form.plate_code,
        &form.plate_number,
        &form.make,
        &form.model_name,
        owner,
    );
    if !form.vin.trim().is_empty() {
        new = new.vin(&form.vin);
    }
    if !form.model_year.trim().is_empty() {
        new = new.year(parse_number(&form.model_year)?);
    }
    if !form.engine.trim().is_empty() {
        new = new.engine(&form.engine);
    }
    if !form.colour.trim().is_empty() {
        new = new.colour(&form.colour);
    }
    if let Some(fleet) = parse_optional_id(&form.fleet_contract)? {
        new = new.fleet(fleet);
    }
    if !form.odometer.trim().is_empty() {
        new = new.odometer(parse_km(&form.odometer)?);
    }
    Ok(new)
}

/// `POST /auto/vehicles`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<VehicleForm>,
) -> Result<Response> {
    require(&user, permissions::VEHICLES_MANAGE)?;
    let new = new_vehicle(&state, &form).await?;
    let vehicle = vehicles::create(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "vehicles.registered")),
        renox::HxRedirect(format!("/auto/vehicles/{}", vehicle.id)),
    )
        .into_response())
}

/// `POST /auto/vehicles/{id}`.
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<VehicleForm>,
) -> Result<Response> {
    require(&user, permissions::VEHICLES_MANAGE)?;
    let new = new_vehicle(&state, &form).await?;
    vehicles::update(&state, &user, id, new).await?;
    Ok((
        Toast::success(text(&lang, "vehicles.saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// One job card on the vehicle's history.
#[derive(Serialize, Debug, Clone)]
pub struct HistoryRow {
    /// The primary key.
    pub id: i64,
    /// `JOB-00007`.
    pub number: String,
    /// The day it was checked in.
    pub date: String,
    /// The complaint.
    pub complaint: String,
    /// The odometer reading at check-in.
    pub odometer: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the page of one vehicle needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/vehicle.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    title: String,
    plate: String,
    vin: String,
    vehicle: String,
    engine: String,
    colour: String,
    owner: String,
    fleet: String,
    odometer: String,
    odometer_at: String,
    next_service: String,
    active: bool,
    history: Vec<HistoryRow>,
    can_manage: bool,
    emirate_options: Vec<[String; 2]>,
    owner_options: Vec<[String; 2]>,
    fleet_options: Vec<[String; 2]>,
    emirate: String,
    plate_code: String,
    plate_number: String,
    raw_vin: String,
    make: String,
    model_name: String,
    model_year: String,
    raw_engine: String,
    raw_colour: String,
    owner_id: String,
    fleet_contract: String,
    odometer_value: String,
}

/// `GET /auto/vehicles/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::VEHICLES_VIEW)?;
    let vehicle = Vehicle::find_or_404(&state.db, id).await?;
    let owner = party_name(&state, &lang.locale, vehicle.owner_party_id).await;
    let next_service = match (vehicle.next_service_date, vehicle.next_service_odometer) {
        (None, None) => text(&lang, "vehicles.next_service_none"),
        (date, km) => {
            let date = date.map_or_else(String::new, |d| d.to_string());
            let km = km.map_or_else(String::new, number_text);
            [date, km]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ")
        }
    };
    let history = super::job_cards::history_rows(&state, &lang, vehicle.id).await?;
    let contract = match vehicle.fleet_contract_id {
        Some(contract_id) => FleetContract::find(&state.db, contract_id).await?,
        None => None,
    };
    let contract_label = contract
        .as_ref()
        .map(|c| format!("{} · {}", c.number(), c.title));
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: vehicle.id,
        title: vehicle.plate(),
        plate: vehicle.plate(),
        vin: vehicle.vin.clone().unwrap_or_else(|| text(&lang, "none")),
        vehicle: vehicle_text(&vehicle),
        engine: vehicle
            .engine
            .clone()
            .unwrap_or_else(|| text(&lang, "none")),
        colour: vehicle
            .colour
            .clone()
            .unwrap_or_else(|| text(&lang, "none")),
        owner: owner.clone(),
        fleet: contract_label
            .clone()
            .unwrap_or_else(|| text(&lang, "none")),
        odometer: km_text(vehicle.odometer_km),
        odometer_at: vehicle
            .odometer_at
            .map_or_else(|| text(&lang, "none"), wl_kernel::documents::dubai_text),
        next_service,
        active: vehicle.active,
        history,
        can_manage: user.has_permission(permissions::VEHICLES_MANAGE),
        emirate_options: emirate_options(),
        owner_options: vec![[vehicle.owner_party_id.to_string(), owner]],
        fleet_options: match (vehicle.fleet_contract_id, contract_label) {
            (Some(contract_id), Some(label)) => vec![[contract_id.to_string(), label]],
            _ => Vec::new(),
        },
        emirate: vehicle.plate_emirate.as_str().to_owned(),
        plate_code: vehicle.plate_code.clone(),
        plate_number: vehicle.plate_number.clone(),
        raw_vin: vehicle.vin.clone().unwrap_or_default(),
        make: vehicle.make.clone(),
        model_name: vehicle.model_name.clone(),
        model_year: vehicle
            .model_year
            .map(|y| y.to_string())
            .unwrap_or_default(),
        raw_engine: vehicle.engine.clone().unwrap_or_default(),
        raw_colour: vehicle.colour.clone().unwrap_or_default(),
        owner_id: vehicle.owner_party_id.to_string(),
        fleet_contract: vehicle
            .fleet_contract_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        odometer_value: vehicle.odometer_km.to_string(),
    }))
}
