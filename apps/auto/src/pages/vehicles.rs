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

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    Texts, parse_km, parse_number, parse_optional_id, party_choices, party_name, party_of, require,
    text,
};
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

/// What the list can be narrowed by.
#[derive(Deserialize, Default, Debug, Clone)]
pub struct Filter {
    /// Text in the plate, VIN, make or model.
    #[serde(default)]
    q: String,
    /// `fleet` or `private`; blank for every vehicle.
    #[serde(default)]
    kind: String,
}

impl Filter {
    /// The list's address with this filter.
    fn href(&self) -> String {
        if self.kind.is_empty() {
            "/auto/vehicles".to_owned()
        } else {
            format!("/auto/vehicles?kind={}", self.kind)
        }
    }

    fn with_kind(&self, kind: &str) -> Filter {
        Filter {
            kind: kind.to_owned(),
            ..self.clone()
        }
    }
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

/// One vehicle of the list.
#[derive(Serialize, Debug, Clone)]
pub struct VehicleRow {
    /// The primary key.
    pub id: i64,
    /// `Dubai A 12345`.
    pub plate: String,
    /// The VIN, or a dash.
    pub vin: String,
    /// `Toyota Hilux 2022`.
    pub vehicle: String,
    /// The owner's name.
    pub owner: String,
    /// Fleet or private.
    pub kind: String,
    /// `52,000 km`.
    pub odometer: String,
}

/// `Toyota Hilux 2022`.
pub(crate) fn vehicle_text(vehicle: &Vehicle) -> String {
    match vehicle.model_year {
        Some(year) => format!("{} {} {year}", vehicle.make, vehicle.model_name),
        None => format!("{} {}", vehicle.make, vehicle.model_name),
    }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/vehicles.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<VehicleRow>,
    kind_tabs: Vec<(String, String)>,
    kind_current: String,
    kind: String,
    q: String,
    emirate_options: Vec<[String; 2]>,
    can_manage: bool,
}

/// `GET /auto/vehicles`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    require(&user, permissions::VEHICLES_VIEW)?;
    let found = vehicles::search(&state.db, &filter.q).await?;
    let mut rows = Vec::new();
    for vehicle in found {
        let fleet = vehicle.is_fleet();
        match filter.kind.as_str() {
            "fleet" if !fleet => continue,
            "private" if fleet => continue,
            _ => {}
        }
        rows.push(VehicleRow {
            id: vehicle.id,
            plate: vehicle.plate(),
            vin: vehicle.vin.clone().unwrap_or_else(|| text(&lang, "none")),
            vehicle: vehicle_text(&vehicle),
            owner: party_name(&state, &lang.locale, vehicle.owner_party_id).await,
            kind: text(
                &lang,
                if fleet {
                    "vehicles.kind.fleet"
                } else {
                    "vehicles.kind.private"
                },
            ),
            odometer: km_text(vehicle.odometer_km),
        });
    }
    let kind_tabs = vec![
        (
            filter.with_kind("").href(),
            text(&lang, "vehicles.filter.all"),
        ),
        (
            filter.with_kind("fleet").href(),
            text(&lang, "vehicles.filter.fleet"),
        ),
        (
            filter.with_kind("private").href(),
            text(&lang, "vehicles.filter.private"),
        ),
    ];
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        kind_tabs,
        kind_current: filter.href(),
        kind: filter.kind.clone(),
        q: filter.q.clone(),
        emirate_options: emirate_options(),
        can_manage: user.has_permission(permissions::VEHICLES_MANAGE),
    }))
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
        fleet: vehicle
            .fleet_contract_id
            .map_or_else(|| text(&lang, "none"), |id| format!("#{id}")),
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
