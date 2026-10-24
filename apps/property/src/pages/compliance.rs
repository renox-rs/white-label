//! The "Buildings non-compliant" tile and list (01 Property §4.5, §10;
//! BL-PRP-25).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /property/compliance` | `property.compliance` | `compliance.view` in a building |
//! | `GET /property/compliance/{id}` | `property.compliance_flag` | `compliance.view` in the flag's building: opens the building |
//!
//! A property manager sees the buildings they hold the permission in, the
//! maintenance coordinator and the company admin all. Leasing is not blocked
//! by anything listed here.

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use serde::Serialize;

use super::{Texts, building_name_sql, buildings_with, text};
use crate::building::Building;
use crate::compliance::{BuildingFlag, non_compliant};
use crate::permissions;
use crate::texts;

/// The routes of the page.
pub fn routes() -> Routes {
    Routes::new()
        .get("/property/compliance", index)
        .name("property.compliance")
        .get("/property/compliance/{id}", open_flag)
        .name("property.compliance_flag")
        .require_auth()
}

/// The causes of a flag.
const CAUSES: [&str; 4] = [
    "visit_missed",
    "no_contract",
    "vendor_document_expired",
    "certificate_expired",
];

/// The compliance grid: search on the building and the category, filter by
/// cause, the oldest flag first. A flag has no page of its own: its row opens
/// the building (`/property/compliance/{id}` redirects there, as the grid's
/// links put the row's id only).
pub(crate) fn grid(lang: &Lang) -> Grid {
    let t = |key: &str| text(lang, key);
    let causes: Vec<(String, String)> = CAUSES
        .iter()
        .map(|cause| ((*cause).to_owned(), t(&format!("compliance.cause_{cause}"))))
        .collect();
    Grid::new("property-compliance")
        .title(&t("compliance.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("building", &t("compliance.building"))
                .computed(&format!(
                    "(SELECT {} FROM \"property_buildings\" \
                     WHERE \"property_buildings\".\"id\" = {{T}}.\"building_id\")",
                    building_name_sql(lang, "\"property_buildings\"")
                ))
                .frozen()
                .mobile()
                .searchable()
                .link("/property/compliance/{id}"),
        )
        .column(Column::select("cause", &t("compliance.cause"), causes).mobile())
        .column(
            Column::text("category", &t("compliance.category"))
                .mobile()
                .searchable(),
        )
        .column(Column::date("raised_on", &t("compliance.since")).mobile())
        .sort_by("raised_on")
        .row_url("/property/compliance/{id}")
        .cards_on_mobile()
        .empty_state(&t("compliance.empty"), Some(&t("compliance.empty_hint")))
        .exports()
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/compliance.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    count: usize,
    flags: GridPage<BuildingFlag>,
}

/// How many of the buildings `user` sees are non-compliant.
pub(crate) async fn count_for(state: &AppState, user: &AuthUser) -> Result<usize> {
    let scopes = user.scopes_with::<Building>(permissions::COMPLIANCE_VIEW);
    if scopes.is_empty() {
        return Ok(0);
    }
    Ok(non_compliant(&state.db)
        .await?
        .iter()
        .filter(|id| scopes.contains(id))
        .count())
}

/// `GET /property/compliance`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let scopes = buildings_with(&user, permissions::COMPLIANCE_VIEW)?;
    // The open flags of the buildings the user sees.
    let query = scopes.apply(
        BuildingFlag::query().where_(BuildingFlag::CLEARED_ON.is_null()),
        &["building_id"],
    );
    let grid = grid(&lang);
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let mut buildings: Vec<i64> = query
        .clone()
        .get(&state.db)
        .await?
        .iter()
        .map(|flag| flag.building_id)
        .collect();
    buildings.sort_unstable();
    buildings.dedup();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        count: buildings.len(),
        flags: grid.page(query, &request).await?,
    })
    .into_response())
}

/// `GET /property/compliance/{id}`: the building of flag `id`, for the
/// grid's row link; 404 for a flag of a building the user does not see.
async fn open_flag(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    let scopes = buildings_with(&user, permissions::COMPLIANCE_VIEW)?;
    let flag = BuildingFlag::find_or_404(&state.db, id).await?;
    if !scopes.contains(&flag.building_id) {
        return Err(Error::NotFound);
    }
    Ok(Redirect::route("property.building", &[&flag.building_id])?.into_response())
}
