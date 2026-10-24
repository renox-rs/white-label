//! Audiences and portal roles (wl-portal §3 *Audience*, *Portal role*, §7,
//! BL-POR-02, BL-POR-03).
//!
//! A vertical declares its audiences ([`AudienceDef`]: the key, the
//! close-after period, the portal roles and their permissions) and seeds them
//! with [`seed`]: what the database lacks is added, what the company admin
//! changed is kept, so a re-run changes nothing. The company admin edits a
//! role's permissions in the portal settings ([`set_permissions`]).
//!
//! The permissions of a user for a party are the union of the permissions of
//! the roles of their grants for it ([`permissions_of`]): two grants for the
//! same party add up. A role key with no row has no permission.

use std::collections::BTreeSet;

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::grants::AccessGrant;

/// The close-after period of an audience without a row (§7: 90 days).
pub const DEFAULT_CLOSE_AFTER_DAYS: i64 = 90;

/// A kind of external user, per vertical (`tenant`, `owner`, `client`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_portal_audiences", unique(key))]
pub struct Audience {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `tenant`.
    pub key: String,
    /// The English label.
    pub label: String,
    /// Days from `end` to `close`; 0 closes at once.
    pub close_after_days: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A named set of portal permissions inside an audience.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_portal_roles", unique(audience, key))]
pub struct PortalRole {
    /// The primary key.
    pub id: i64,
    /// The audience key.
    pub audience: String,
    /// The role key, e.g. `finance_contact`.
    pub key: String,
    /// The English label.
    pub label: String,
    /// The portal permissions, comma-separated.
    pub permissions: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PortalRole {
    /// The permission keys.
    pub fn permission_keys(&self) -> Vec<String> {
        split(&self.permissions)
    }

    /// The roles of `audience`, by key.
    pub async fn of_audience(db: &Db, audience: &str) -> Result<Vec<PortalRole>> {
        PortalRole::query()
            .where_(PortalRole::AUDIENCE.eq(audience))
            .order_by(PortalRole::ID)
            .get(db)
            .await
    }

    /// Every role, by audience then in the order they were seeded.
    pub async fn all(db: &Db) -> Result<Vec<PortalRole>> {
        let mut roles = PortalRole::query().order_by(PortalRole::ID).get(db).await?;
        roles.sort_by(|a, b| a.audience.cmp(&b.audience).then(a.id.cmp(&b.id)));
        Ok(roles)
    }
}

/// Comma-separated keys, trimmed, without blanks or repeats, in order.
fn split(text: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for key in text.split([',', '\n', ' ']).map(str::trim) {
        if !key.is_empty() && !keys.iter().any(|k| k == key) {
            keys.push(key.to_owned());
        }
    }
    keys
}

/// A portal role a vertical declares.
#[derive(Debug, Clone, Copy)]
pub struct RoleDef {
    /// The key, e.g. `primary_contact`.
    pub key: &'static str,
    /// The English label.
    pub label: &'static str,
    /// The portal permissions.
    pub permissions: &'static [&'static str],
}

/// An audience a vertical declares (§8 `AudienceDefinition`).
#[derive(Debug, Clone, Copy)]
pub struct AudienceDef {
    /// The key, e.g. `tenant`.
    pub key: &'static str,
    /// The English label.
    pub label: &'static str,
    /// Days from `end` to `close`.
    pub close_after_days: i64,
    /// The portal roles.
    pub roles: &'static [RoleDef],
}

/// Stores `def` where the database lacks it: the audience row and each role
/// row missing by key. A role the company admin changed keeps its
/// permissions. Returns how many rows were created.
pub async fn seed(db: &Db, def: &AudienceDef) -> Result<usize> {
    let mut created = 0;
    let exists = Audience::query()
        .where_(Audience::KEY.eq(def.key))
        .first(db)
        .await?
        .is_some();
    if !exists {
        Audience::create(
            db,
            Audience {
                key: def.key.to_owned(),
                label: def.label.to_owned(),
                close_after_days: def.close_after_days,
                ..Audience::default()
            },
        )
        .await?;
        created += 1;
    }
    let roles = PortalRole::of_audience(db, def.key).await?;
    for role in def.roles {
        if roles.iter().any(|r| r.key == role.key) {
            continue;
        }
        PortalRole::create(
            db,
            PortalRole {
                audience: def.key.to_owned(),
                key: role.key.to_owned(),
                label: role.label.to_owned(),
                permissions: role.permissions.join(","),
                ..PortalRole::default()
            },
        )
        .await?;
        created += 1;
    }
    Ok(created)
}

/// The close-after period of `audience`, in days (§7; 90 without a row).
pub async fn close_after_days(db: &Db, audience: &str) -> Result<i64> {
    Ok(Audience::query()
        .where_(Audience::KEY.eq(audience))
        .first(db)
        .await?
        .map_or(DEFAULT_CLOSE_AFTER_DAYS, |a| a.close_after_days))
}

/// The union of the permissions of the roles of `grants`, each role looked
/// up in its grant's audience (two grants for one party add up).
pub async fn permissions_of(db: &Db, grants: &[AccessGrant]) -> Result<BTreeSet<String>> {
    let mut all = BTreeSet::new();
    let mut audiences: Vec<&str> = grants.iter().map(|g| g.audience.as_str()).collect();
    audiences.sort_unstable();
    audiences.dedup();
    for audience in audiences {
        let roles = PortalRole::of_audience(db, audience).await?;
        for grant in grants.iter().filter(|g| g.audience == audience) {
            for key in grant.role_keys() {
                if let Some(role) = roles.iter().find(|r| r.key == key) {
                    all.extend(role.permission_keys());
                }
            }
        }
    }
    Ok(all)
}

/// The company admin (`by`, holding `portal_users.manage`) sets the
/// permissions of the role `id` to `permissions` (comma-, space- or
/// line-separated). Only portal permissions (`portal.*`) are kept: a portal
/// role never carries a staff permission (`PLT.ACL.05`).
pub async fn set_permissions(
    db: &Db,
    by: &AuthUser,
    id: i64,
    permissions: &str,
) -> Result<PortalRole> {
    if !by.has_permission(crate::permissions::USERS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let role = PortalRole::find_or_404(db, id).await?;
    let keys: Vec<String> = split(permissions)
        .into_iter()
        .filter(|k| k.starts_with("portal."))
        .collect();
    let joined = keys.join(",");
    let stamp = now();
    PortalRole::query()
        .where_(PortalRole::ID.eq(role.id))
        .update(db, &[("permissions", &joined), ("updated_at", &stamp)])
        .await?;
    PortalRole::find_or_404(db, id).await
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn permissions_are_split_without_blanks_or_repeats() {
        assert_eq!(
            split("portal.invoices.view, portal.invoices.pay\nportal.invoices.view,,"),
            ["portal.invoices.view", "portal.invoices.pay"]
        );
    }
}
