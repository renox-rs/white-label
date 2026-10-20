//! The portfolio pages: the buildings with their occupancy, a building with
//! its units, and the forms that add them (`PRP.PRT.01`, `PRP.PRT.02`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /property` | `property.index` | `units.view` in a building |
//! | `GET /property/buildings/{id}` | `property.building` | `units.view` in the building |
//! | `POST /property/buildings` | `property.building_create` | `units.manage` (global) |
//! | `POST /property/buildings/{id}/units` | `property.unit_create` | `units.manage` in the building |
//! | `POST /property/units/{id}/transitions/{name}` | `property.unit_transitions.*` | the transition's permission in the building |
//!
//! The create-lease sheet on a unit is `property.lease_create`
//! ([`super::leases`]).

use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};

use super::{Texts, buildings_with, text, unit_kind};
use crate::building::{Building, NewBuilding, create_building};
use crate::parse::{parse_aed, parse_milli};
use crate::permissions;
use crate::texts;
use crate::unit::{NewUnit, Unit, UnitStatus, UnitType, UnitUse, create_unit};
use crate::unit_machine::{MARK_READY, RELIST, WITHDRAW};

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/property", index)
        .name("property.index")
        .get("/property/buildings/{id}", building)
        .name("property.building")
        .post("/property/buildings", building_create)
        .name("property.building_create")
        .post("/property/buildings/{id}/units", unit_create)
        .name("property.unit_create")
        .group(
            "/property/units",
            "property.unit_transitions.",
            wl_kernel::machines::routes::<Unit>(),
        )
        .require_auth()
}

/// One building of the list.
#[derive(Serialize, Debug, Clone)]
pub struct BuildingRow {
    /// The primary key.
    pub id: i64,
    /// The code.
    pub code: String,
    /// The name in the page's language.
    pub name: String,
    /// The emirate.
    pub emirate: String,
    /// How many units.
    pub units: i64,
    /// How many are occupied (or on notice).
    pub occupied: i64,
    /// How many can be leased.
    pub available: i64,
    /// Occupied of all, as a percentage text, e.g. `83%`.
    pub occupancy: String,
}

/// Occupied of `units`, as a percentage text; `-` for none.
pub fn occupancy_text(occupied: i64, units: i64) -> String {
    if units <= 0 {
        return "-".to_owned();
    }
    format!("{}%", (occupied * 100 + units / 2) / units)
}

/// What the portfolio page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/index.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    buildings: Vec<BuildingRow>,
    emirates: Vec<(String, String)>,
    can_manage: bool,
    non_compliant: usize,
    can_see_compliance: bool,
}

/// The emirates of the building form, `(key, label)`.
fn emirates(lang: &Lang) -> Vec<(String, String)> {
    [
        "dubai",
        "abu_dhabi",
        "sharjah",
        "ajman",
        "umm_al_quwain",
        "ras_al_khaimah",
        "fujairah",
    ]
    .into_iter()
    .map(|key| (key.to_owned(), text(lang, &format!("emirates.{key}"))))
    .collect()
}

/// `GET /property`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let scopes = buildings_with(&user, permissions::UNITS_VIEW)?;
    let mut rows = Vec::new();
    for building in Building::query()
        .order_by(Building::CODE)
        .get(&state.db)
        .await?
    {
        if !scopes.contains(&building.id) {
            continue;
        }
        let units = Unit::of_building(&state.db, building.id).await?;
        let count = |wanted: &[UnitStatus]| {
            i64::try_from(units.iter().filter(|u| wanted.contains(&u.status)).count())
                .unwrap_or(i64::MAX)
        };
        let occupied = count(&[UnitStatus::Occupied, UnitStatus::NoticeGiven]);
        let total = i64::try_from(units.len()).unwrap_or(i64::MAX);
        rows.push(BuildingRow {
            id: building.id,
            code: building.code.clone(),
            name: building.display_name(&lang.locale),
            emirate: text(&lang, &format!("emirates.{}", building.emirate)),
            units: total,
            occupied,
            available: count(&[UnitStatus::Available]),
            occupancy: occupancy_text(occupied, total),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        buildings: rows,
        emirates: emirates(&lang),
        can_manage: user.has_permission(permissions::UNITS_MANAGE),
        non_compliant: super::compliance::count_for(&state, &user).await?,
        can_see_compliance: !user
            .scopes_with::<Building>(permissions::COMPLIANCE_VIEW)
            .is_empty(),
    }))
}

/// A transition offered on a unit.
#[derive(Serialize, Debug, Clone)]
pub struct ActionView {
    /// The transition's name.
    pub name: String,
    /// Its label.
    pub label: String,
}

/// One unit of a building page.
#[derive(Serialize, Debug, Clone)]
pub struct UnitRow {
    /// The primary key.
    pub id: i64,
    /// The unit number.
    pub number: String,
    /// The kind of space.
    pub unit_type: String,
    /// Residential or commercial.
    pub usage: String,
    /// The size in square feet.
    pub area: String,
    /// The bedrooms.
    pub bedrooms: String,
    /// The asking rent.
    pub asking_rent: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// The tenant of the running lease, with a link to it.
    pub lease_id: Option<i64>,
    /// The lease's tenant name.
    pub tenant: String,
    /// The transitions the user may run.
    pub actions: Vec<ActionView>,
    /// Whether the user may draft a lease on it.
    pub can_lease: bool,
}

/// What a building page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/building.html")]
pub(crate) struct BuildingPage {
    tx: Texts,
    building: BuildingView,
    units: Vec<UnitRow>,
    unit_types: Vec<(String, String)>,
    usages: Vec<(String, String)>,
    can_manage: bool,
    today: String,
}

/// The building as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct BuildingView {
    /// The primary key.
    pub id: i64,
    /// The code.
    pub code: String,
    /// The name in the page's language.
    pub name: String,
    /// The emirate.
    pub emirate: String,
    /// The community.
    pub community: String,
    /// The plot number.
    pub plot_number: String,
    /// The premise number.
    pub premise_number: String,
}

/// The formatted size, `1,250.5`; empty for none.
fn area_text(milli: Option<i64>) -> String {
    milli.map_or_else(String::new, |m| {
        let whole = m / 1000;
        let fraction = m % 1000;
        if fraction == 0 {
            whole.to_string()
        } else {
            format!("{whole}.{}", format!("{fraction:03}").trim_end_matches('0'))
        }
    })
}

/// The unit types and uses of the unit form.
fn kinds(lang: &Lang) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let types = ["apartment", "villa", "office", "shop", "warehouse"]
        .into_iter()
        .map(|k| (k.to_owned(), text(lang, &format!("units.type.{k}"))))
        .collect();
    let usages = ["residential", "commercial"]
        .into_iter()
        .map(|k| (k.to_owned(), text(lang, &format!("units.usage.{k}"))))
        .collect();
    (types, usages)
}

/// The transitions of a unit the page offers.
const OFFERED: [&str; 3] = [MARK_READY, WITHDRAW, RELIST];

/// `GET /property/buildings/{id}`.
async fn building(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let building = Building::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::UNITS_VIEW, &building.scope()) {
        return Err(Error::Forbidden);
    }
    let can_lease_here = user.has_permission_in(permissions::LEASES_CREATE, &building.scope());
    let mut rows = Vec::new();
    for unit in Unit::of_building(&state.db, id).await? {
        let actions = unit
            .available(&user, &state.db)
            .await?
            .into_iter()
            .filter(|a| OFFERED.contains(&a.name.as_str()))
            .map(|a| ActionView {
                name: a.name,
                label: a.label,
            })
            .collect();
        let lease = running_lease(&state, unit.id).await?;
        let tenant = match &lease {
            Some(lease) => wl_kernel::parties::directory(&state)
                .display_name(&state.db, lease.tenant(), &lang.locale)
                .await?
                .unwrap_or_else(|| format!("#{}", lease.tenant_party_id)),
            None => String::new(),
        };
        rows.push(UnitRow {
            id: unit.id,
            number: unit.number.clone(),
            unit_type: text(&lang, &format!("units.type.{}", unit.unit_type.as_str())),
            usage: text(&lang, &format!("units.usage.{}", unit.usage.as_str())),
            area: area_text(unit.area_sqft_milli),
            bedrooms: unit.bedrooms.map(|b| b.to_string()).unwrap_or_default(),
            asking_rent: unit.asking_rent.map(crate::money_text).unwrap_or_default(),
            status: text(&lang, &format!("units.status.{}", unit.status.as_str())),
            status_kind: unit_kind(unit.status),
            lease_id: lease.as_ref().map(|l| l.id),
            tenant,
            actions,
            can_lease: can_lease_here && unit.status == UnitStatus::Available,
        });
    }
    let (unit_types, usages) = kinds(&lang);
    Ok(View::typed(BuildingPage {
        tx: texts::all(&lang),
        building: BuildingView {
            id: building.id,
            code: building.code.clone(),
            name: building.display_name(&lang.locale),
            emirate: text(&lang, &format!("emirates.{}", building.emirate)),
            community: building.community.clone().unwrap_or_default(),
            plot_number: building.plot_number.clone().unwrap_or_default(),
            premise_number: building.premise_number.clone().unwrap_or_default(),
        },
        units: rows,
        unit_types,
        usages,
        can_manage: user.has_permission_in(permissions::UNITS_MANAGE, &building.scope()),
        today: wl_kernel::documents::today().to_string(),
    }))
}

/// The lease in force on a unit, if any.
async fn running_lease(state: &AppState, unit_id: i64) -> Result<Option<crate::lease::Lease>> {
    use crate::lease::{Lease, LeaseStatus};
    Lease::query()
        .where_(Lease::UNIT_ID.eq(unit_id))
        .where_(Lease::STATUS.is_in(LeaseStatus::RUNNING))
        .order_by_desc(Lease::VERSION)
        .first(&state.db)
        .await
}

/// The form of a new building.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct BuildingForm {
    /// The short code.
    #[serde(default)]
    pub code: String,
    /// The name in English.
    #[serde(default)]
    pub name_en: String,
    /// The name in Arabic.
    #[serde(default)]
    pub name_ar: String,
    /// The emirate.
    #[serde(default)]
    pub emirate: String,
    /// The community.
    #[serde(default)]
    pub community: String,
    /// The plot number.
    #[serde(default)]
    pub plot_number: String,
    /// The premise number.
    #[serde(default)]
    pub premise_number: String,
}

/// `POST /property/buildings`.
async fn building_create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BuildingForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::UNITS_MANAGE) {
        return Err(Error::Forbidden);
    }
    create_building(
        &state.db,
        NewBuilding {
            code: form.code,
            name_en: form.name_en,
            name_ar: Some(form.name_ar),
            emirate: form.emirate,
            community: Some(form.community),
            plot_number: Some(form.plot_number),
            premise_number: Some(form.premise_number),
        },
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "buildings.created")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form of a new unit.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct UnitForm {
    /// The unit number.
    #[serde(default)]
    pub number: String,
    /// The kind of space.
    #[serde(default)]
    pub unit_type: String,
    /// Residential or commercial.
    #[serde(default)]
    pub usage: String,
    /// The size in square feet.
    #[serde(default)]
    pub area: String,
    /// The bedrooms.
    #[serde(default)]
    pub bedrooms: String,
    /// The asking rent per year, in dirhams.
    #[serde(default)]
    pub asking_rent: String,
    /// Notes.
    #[serde(default)]
    pub notes: String,
}

/// The enum for a form value, a 400 when it is none of the variants.
fn parsed<T: std::str::FromStr>(value: &str, field: &str) -> Result<T> {
    value
        .trim()
        .parse()
        .map_err(|_| Error::BadRequest(field.to_owned()))
}

/// `POST /property/buildings/{id}/units`.
async fn unit_create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<UnitForm>,
) -> Result<Response> {
    let building = Building::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::UNITS_MANAGE, &building.scope()) {
        return Err(Error::Forbidden);
    }
    let area = match form.area.trim() {
        "" => None,
        value => Some(parse_milli(value).ok_or_else(|| Error::BadRequest("area".to_owned()))?),
    };
    let rent = match form.asking_rent.trim() {
        "" => None,
        value => Some(parse_aed(value).ok_or_else(|| Error::BadRequest("asking_rent".to_owned()))?),
    };
    let bedrooms = match form.bedrooms.trim() {
        "" => None,
        value => Some(parsed::<i64>(value, "bedrooms")?),
    };
    create_unit(
        &state.db,
        NewUnit {
            building_id: id,
            floor_id: None,
            number: form.number,
            unit_type: if form.unit_type.trim().is_empty() {
                UnitType::default()
            } else {
                parsed(&form.unit_type, "unit_type")?
            },
            usage: if form.usage.trim().is_empty() {
                UnitUse::default()
            } else {
                parsed(&form.usage, "usage")?
            },
            area_sqft_milli: area,
            bedrooms,
            asking_rent: rent,
            notes: Some(form.notes),
        },
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "units.created")),
        renox::HxRefresh,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupancy_is_a_rounded_percentage() {
        assert_eq!(occupancy_text(5, 6), "83%");
        assert_eq!(occupancy_text(0, 4), "0%");
        assert_eq!(occupancy_text(4, 4), "100%");
        assert_eq!(occupancy_text(0, 0), "-");
    }

    #[test]
    fn an_area_reads_without_trailing_zeros() {
        assert_eq!(area_text(None), "");
        assert_eq!(area_text(Some(900_000)), "900");
        assert_eq!(area_text(Some(1_250_500)), "1250.5");
        assert_eq!(area_text(Some(1_250_050)), "1250.05");
    }
}
