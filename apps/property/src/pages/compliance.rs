//! The "Buildings non-compliant" tile and list (01 Property §4.5, §10;
//! BL-PRP-25).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /property/compliance` | `property.compliance` | `compliance.view` in a building |
//!
//! A property manager sees the buildings they hold the permission in, the
//! maintenance coordinator and the company admin all. Leasing is not blocked
//! by anything listed here.

use renox::prelude::*;
use serde::Serialize;

use super::{Texts, buildings_with, text};
use crate::building::Building;
use crate::compliance::{BuildingFlag, non_compliant};
use crate::permissions;
use crate::texts;

/// The routes of the page.
pub fn routes() -> Routes {
    Routes::new()
        .get("/property/compliance", index)
        .name("property.compliance")
        .require_auth()
}

/// A row of the list: building, cause, since.
#[derive(Serialize, Debug, Clone)]
pub struct FlagRow {
    /// The building's name.
    pub building: String,
    /// The cause in the page's language.
    pub cause: String,
    /// The category or the certificate kind.
    pub category: String,
    /// The day it was raised.
    pub since: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/compliance.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    count: usize,
    flags: Vec<FlagRow>,
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
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let scopes = buildings_with(&user, permissions::COMPLIANCE_VIEW)?;
    let mut rows = Vec::new();
    for flag in BuildingFlag::open(&state.db).await? {
        if !scopes.contains(&flag.building_id) {
            continue;
        }
        let Some(building) = Building::find(&state.db, flag.building_id).await? else {
            continue;
        };
        rows.push(FlagRow {
            building: building.display_name(&lang.locale),
            cause: text(&lang, &format!("compliance.cause_{}", flag.cause)),
            category: flag.category.clone(),
            since: flag.raised_on.to_string(),
        });
    }
    let mut buildings: Vec<&str> = rows.iter().map(|r| r.building.as_str()).collect();
    buildings.sort_unstable();
    buildings.dedup();
    let count = buildings.len();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        count,
        flags: rows,
    }))
}
