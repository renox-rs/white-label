//! The portal settings of the back office (wl-portal §7, BL-POR-02): the
//! portal roles of each audience and their portal permissions, seeded per
//! vertical, which the company admin (`portal_users.manage`) changes.
//!
//! | Route | Name | Who |
//! |---|---|---|
//! | `GET /portal/roles` | `wl_portal.roles` | `portal_users.manage` |
//! | `POST /portal/roles/{id}` | `wl_portal.roles.update` | `portal_users.manage` (form field `permissions`) |

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::audiences::{self, Audience, PortalRole};
use crate::permissions;
use crate::texts;

/// One role on the page.
#[derive(Debug, Clone, Serialize)]
pub struct RoleRow {
    /// The role's id.
    pub id: i64,
    /// The audience key.
    pub audience: String,
    /// The role key.
    pub key: String,
    /// The label.
    pub label: String,
    /// The permissions, comma-separated.
    pub permissions: String,
    /// The audience's close-after period.
    pub close_after_days: i64,
}

/// What the roles page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-portal/roles.html")]
pub struct RolesPage {
    tx: BTreeMap<&'static str, String>,
    role_rows: Vec<RoleRow>,
}

/// The form of a role's permissions.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct PermissionsForm {
    /// The portal permissions, separated by commas, spaces or lines.
    #[serde(default)]
    pub permissions: Option<String>,
}

/// The routes of the portal settings.
pub fn routes() -> Routes {
    Routes::new()
        .get("/portal/roles", index)
        .name("wl_portal.roles")
        .post("/portal/roles/{id}", update)
        .name("wl_portal.roles.update")
        .require_permission(permissions::USERS_MANAGE)
}

/// `GET /portal/roles`: every audience's roles and their permissions.
async fn index(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let audience_rows = Audience::query().get(&state.db).await?;
    let mut role_rows = Vec::new();
    for role in PortalRole::all(&state.db).await? {
        let close_after_days = audience_rows
            .iter()
            .find(|a| a.key == role.audience)
            .map_or(audiences::DEFAULT_CLOSE_AFTER_DAYS, |a| a.close_after_days);
        role_rows.push(RoleRow {
            id: role.id,
            permissions: role.permission_keys().join(", "),
            audience: role.audience,
            key: role.key,
            label: role.label,
            close_after_days,
        });
    }
    Ok(View::typed(RolesPage {
        tx: texts::all(&lang),
        role_rows,
    }))
}

/// `POST /portal/roles/{id}`: sets the role's permissions and reloads the
/// page.
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<PermissionsForm>,
) -> Result<Response> {
    let text = form.permissions.unwrap_or_default();
    audiences::set_permissions(&state.db, &user, id, &text).await?;
    let saved = texts::t(&lang, "wl_portal.roles.saved");
    Ok((Toast::success(saved), renox::HxRefresh).into_response())
}
