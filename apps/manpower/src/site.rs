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
use crate::texts::refusal;

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
    /// The licences and passes the site asks of the workers, in words.
    pub required_licences: Option<String>,
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

/// What a site is made of when it is added or changed.
#[derive(Debug, Clone, Default)]
pub struct SiteInput {
    /// The name.
    pub name: String,
    /// The address.
    pub address: Option<String>,
    /// The licences and passes it asks of the workers, in words.
    pub required_licences: Option<String>,
}

fn clean(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// The checks every site passes: a name, not used by another site of the
/// client (`except`: the site being changed).
async fn check(
    state: &AppState,
    client: PartyId,
    name: &str,
    except: Option<i64>,
) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(refused(refusal("manpower.problems.site_name", &[])));
    }
    let same = Site::query()
        .where_(Site::CLIENT_PARTY_ID.eq(client.0))
        .get(&state.db)
        .await?
        .into_iter()
        .any(|site| Some(site.id) != except && site.name.trim().eq_ignore_ascii_case(name));
    if same {
        return Err(refused(refusal("manpower.problems.site_duplicate", &[])));
    }
    Ok(name.to_owned())
}

/// `by` adds a site of `client` named `name`.
///
/// # Errors
///
/// `Forbidden` without `sites.manage` for the client; a 422 for a blank name,
/// a name another site of the client has, or a client that is not a known
/// party.
pub async fn create(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    name: &str,
    address: Option<&str>,
) -> Result<Site> {
    create_with(
        state,
        by,
        client,
        SiteInput {
            name: name.to_owned(),
            address: address.map(str::to_owned),
            required_licences: None,
        },
    )
    .await
}

/// [`create`] with the licences the site asks for.
///
/// # Errors
///
/// As [`create`].
pub async fn create_with(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    input: SiteInput,
) -> Result<Site> {
    if !by.has_permission_in(
        permissions::SITES_MANAGE,
        &permissions::client_scope(client.0),
    ) {
        return Err(Error::Forbidden);
    }
    let name = check(state, client, &input.name, None).await?;
    if wl_kernel::parties::directory(state)
        .find(&state.db, client)
        .await?
        .is_none()
    {
        return Err(refused(refusal("manpower.problems.client_unknown", &[])));
    }
    Site::create(
        &state.db,
        Site {
            client_party_id: client.0,
            name,
            address: clean(input.address.as_deref()),
            required_licences: clean(input.required_licences.as_deref()),
            active: true,
            ..Site::default()
        },
    )
    .await
}

/// `by` changes site `id`: its name, address and required licences. The
/// deployments stay linked: the site keeps its id.
///
/// # Errors
///
/// `Forbidden` without `sites.manage` for the client; a 422 for a blank name or
/// a name another site of the client has.
pub async fn update(state: &AppState, by: &AuthUser, id: i64, input: SiteInput) -> Result<Site> {
    let site = Site::find_or_404(&state.db, id).await?;
    if !by.has_permission_in(
        permissions::SITES_MANAGE,
        &permissions::client_scope(site.client_party_id),
    ) {
        return Err(Error::Forbidden);
    }
    let name = check(state, site.client(), &input.name, Some(id)).await?;
    let mut site = site;
    site.name = name;
    site.address = clean(input.address.as_deref());
    site.required_licences = clean(input.required_licences.as_deref());
    site.save(&state.db).await?;
    Ok(site)
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
