//! The Site (09 Manpower §6 *Site*): a client location where workers are
//! deployed. It is the scope a site supervisor is assigned in (§3.2). The
//! licences and passes a site requires (`MPW.CMP.01`) come with the guard on
//! them.

use renox::auth::permissions::Scope;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

use crate::db::refused;
use crate::permissions;
use crate::texts::english;

/// The subject type of a site.
pub const SUBJECT_TYPE: &str = "manpower.site";

/// A client site.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manpower_sites", index(client_party_id))]
pub struct Site {
    /// The primary key.
    pub id: i64,
    /// The client company, a wl-parties party.
    pub client_party_id: i64,
    /// The name.
    pub name: String,
    /// The address.
    pub address: Option<String>,
    /// Whether workers can still be deployed there.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Site {
    /// The site as a [`Subject`]: `manpower.site:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The client as a [`PartyId`].
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The scope a site supervisor is assigned in.
    pub fn scope(&self) -> Scope {
        permissions::site_scope(self.id)
    }
}

/// `by` adds a site of `client` named `name`.
///
/// # Errors
///
/// `Forbidden` without `sites.manage` for the client; a 422 for a blank name
/// or a client that is not a known party.
pub async fn create(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    name: &str,
    address: Option<&str>,
) -> Result<Site> {
    if !by.has_permission_in(
        permissions::SITES_MANAGE,
        &permissions::client_scope(client.0),
    ) {
        return Err(Error::Forbidden);
    }
    if name.trim().is_empty() {
        return Err(refused(english("manpower.problems.site_name", &[])));
    }
    if wl_kernel::parties::directory(state)
        .find(&state.db, client)
        .await?
        .is_none()
    {
        return Err(refused(english("manpower.problems.client_unknown", &[])));
    }
    Site::create(
        &state.db,
        Site {
            client_party_id: client.0,
            name: name.trim().to_owned(),
            address: address
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty()),
            active: true,
            ..Site::default()
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_a_subject_and_a_scope() {
        let site = Site {
            id: 4,
            client_party_id: 9,
            ..Site::default()
        };
        assert_eq!(site.subject().to_string(), "manpower.site:4");
        assert_eq!(site.scope(), Scope::new("manpower.site", 4));
        assert_eq!(site.client(), PartyId(9));
    }
}
