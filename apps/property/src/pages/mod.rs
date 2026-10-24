//! The back-office pages of the vertical: the portfolio (buildings and units)
//! and the leases. Every route needs a signed-in user; what a user may see
//! and do is decided per building, because roles are assigned per building
//! (§3.2) and a route guard counts global roles only.

pub mod agreements;
pub mod compliance;
pub mod enquiries;
pub mod leases;
pub mod portfolio;
pub mod statements;

use std::collections::BTreeMap;

use renox::auth::permissions::Scopes;
use renox::prelude::*;

use crate::building::Building;
use crate::lease::LeaseStatus;
use crate::texts;
use crate::unit::UnitStatus;

/// The texts a template reads as `tx['portfolio.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(portfolio::routes())
        .merge(leases::routes())
        .merge(statements::routes())
        .merge(agreements::routes())
        .merge(compliance::routes())
        .merge(enquiries::routes())
}

/// A 422 for one field of a form: the kit shows `message` next to the input
/// named `field` (Renox answers an htmx post with the errors as JSON), and a
/// plain post goes back to the form with it.
pub(crate) fn field_error(field: &str, message: impl Into<String>) -> Error {
    let mut errors = renox::validation::Errors::new();
    errors.add(field, message);
    Error::Validation(renox::validation::ValidationError::new(errors))
}

/// SQL for a grid column: the integer column `column` of the row, left-padded
/// with zeros to `width` digits (`L-000012`), the same in SQLite and Postgres.
pub(crate) fn padded(column: &str, width: usize) -> String {
    let zeros = "0".repeat(width);
    let text = format!("'{zeros}' || CAST({{T}}.\"{column}\" AS TEXT)");
    format!("substr({text}, length({text}) - {} )", width - 1)
}

/// SQL for the name of a building in the page's language: the Arabic name
/// when the language is `ar` and the building has one. `table` is the
/// (quoted) table or alias of the buildings.
pub(crate) fn building_name_sql(lang: &Lang, table: &str) -> String {
    if lang.locale == "ar" {
        format!("COALESCE(NULLIF({table}.\"name_ar\", ''), {table}.\"name_en\")")
    } else {
        format!("{table}.\"name_en\"")
    }
}

/// SQL for a grid column: the display name of the party whose id is in the
/// row's column `column`, read from the parties directory's table
/// (`wl_parties_parties`, the module every Property client runs). The grid
/// searches, sorts, filters and exports on it; the cell on screen still shows
/// the name the directory gives in the page's language.
pub(crate) fn party_name_sql(column: &str) -> String {
    format!(
        "(SELECT \"wl_parties_parties\".\"display_name\" FROM \"wl_parties_parties\" \
         WHERE \"wl_parties_parties\".\"id\" = {{T}}.\"{column}\")"
    )
}

/// The package text `key` (without the `property.` prefix).
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("property.{key}"))
}

/// The buildings in which the user holds `permission`; 403 when none.
pub(crate) fn buildings_with(user: &AuthUser, permission: &str) -> Result<Scopes<i64>> {
    let scopes = user.scopes_with::<Building>(permission);
    if scopes.is_empty() {
        return Err(Error::Forbidden);
    }
    Ok(scopes)
}

/// The party a form names in `field`: the number shown in the UI (`P-00012`,
/// any case, `P-12` too) or the id the picker sends (`12`). A 422 on the
/// field when no party has it.
pub(crate) async fn party_of(
    state: &AppState,
    value: &str,
    field: &str,
) -> Result<wl_kernel::PartyId> {
    let value = value.trim();
    let directory = wl_kernel::parties::directory(state);
    let found = if let Ok(id) = value.parse::<i64>() {
        directory.find(&state.db, wl_kernel::PartyId(id)).await?
    } else {
        let digits = value
            .get(..2)
            .filter(|prefix| prefix.eq_ignore_ascii_case("p-"))
            .map(|_| &value[2..])
            .and_then(|rest| rest.trim().parse::<u32>().ok());
        match digits {
            Some(number) => {
                directory
                    .find_by_code(&state.db, &format!("P-{number:05}"))
                    .await?
            }
            None => None,
        }
    };
    found.map(|party| party.id).ok_or_else(|| {
        field_error(
            field,
            crate::texts::english("property.leases.tenant_unknown", &[]),
        )
    })
}

/// The badge kind of a lease status.
pub(crate) fn lease_kind(status: LeaseStatus) -> &'static str {
    match status {
        LeaseStatus::Active | LeaseStatus::Renewed => "success",
        LeaseStatus::PendingSignature | LeaseStatus::Draft => "info",
        LeaseStatus::RenewalDue | LeaseStatus::Expired | LeaseStatus::NoticeGiven => "warning",
        LeaseStatus::Amended | LeaseStatus::Terminated | LeaseStatus::Cancelled => "info",
    }
}

/// The badge kind of a unit status.
pub(crate) fn unit_kind(status: UnitStatus) -> &'static str {
    match status {
        UnitStatus::Occupied => "success",
        UnitStatus::Available | UnitStatus::Reserved => "info",
        UnitStatus::NoticeGiven | UnitStatus::UnderMaintenance => "warning",
        UnitStatus::OffMarket => "info",
    }
}

/// The name of the user `id`, else `#id`.
pub(crate) async fn user_name(db: &Db, id: i64) -> String {
    User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| format!("#{id}"), |u| u.name)
}
