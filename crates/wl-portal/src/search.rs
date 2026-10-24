//! The portal search (BL-AGT-05, wl-kernel §5.8 point 2): a tenant or a
//! client finds their own invoices (and the other types the modules mark
//! portal-visible) by number or words, without ever seeing another party's
//! records.
//!
//! | Route | Name | Who |
//! |---|---|---|
//! | `GET /portal/search?q=` | `wl_portal.search` | a portal user with a readable grant |
//!
//! The query runs on the kernel's index ([`wl_kernel::search::portal_search`])
//! filtered **by party**, never by the staff permissions alone: for each
//! party the user acts for (a readable grant, `active` or `read_only`), the
//! portal-visible types whose portal permission the grants' roles give, less
//! the types the client turned off (`client.toml`, `[portal]
//! search_hidden_types = ["invoicing.invoice"]`, wl-kernel §7 *Searchable
//! types shown in the portal*). A user acting for several parties (a
//! guardian, a company contact) sees the records of each, grouped by party.
//! When exactly one record has an identifier equal to the query, the page
//! opens it in the portal.
//!
//! The party's block is never read here (`PTY.ACL.02`).

use std::collections::BTreeMap;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::search::{PORTAL_LIMIT, PortalHit, PortalReach, portal_search, portal_types};
use wl_kernel::{ClientConfig, PartyId};

use crate::access::PortalAccess;
use crate::texts;

/// The name of the search route.
pub const ROUTE: &str = "wl_portal.search";

/// The page's template.
pub const TEMPLATE: &str = "wl-portal/search.html";

/// The key of `client.toml`'s `[portal]` section listing the searchable
/// types the client does not show in the portal.
pub const HIDDEN_TYPES_KEY: &str = "search_hidden_types";

/// The query string of the page.
#[derive(Debug, Default, Deserialize)]
pub struct SearchQuery {
    /// What the user typed.
    #[serde(default)]
    pub q: String,
}

/// One result on the page.
#[derive(Debug, Clone, Serialize)]
pub struct ResultRow {
    /// What it is called.
    pub title: String,
    /// The line under it.
    pub secondary: String,
    /// The type's group, e.g. `Invoices`.
    pub label: String,
    /// Its status.
    pub status: Option<String>,
    /// Where it opens in the portal.
    pub url: String,
}

/// The results of one party.
#[derive(Debug, Clone, Serialize)]
pub struct PartyResults {
    /// The party's name.
    pub name: String,
    /// The party's portal home.
    pub home_url: String,
    /// Its records found.
    pub rows: Vec<ResultRow>,
}

/// What the search page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-portal/search.html")]
pub struct PortalSearchPage {
    tx: BTreeMap<&'static str, String>,
    query: String,
    hint: Option<String>,
    empty: Option<String>,
    party_results: Vec<PartyResults>,
}

/// The routes of the portal search.
pub fn routes() -> Routes {
    Routes::new()
        .get("/portal/search", page)
        .name(ROUTE)
        .require_auth()
}

/// The types the client turned off for the portal search.
fn hidden_types(state: &AppState) -> Vec<String> {
    state
        .provided::<Arc<ClientConfig>>()
        .and_then(|config| {
            config
                .section("portal")
                .and_then(|portal| portal.get(HIDDEN_TYPES_KEY))
                .and_then(|value| value.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
        })
        .unwrap_or_default()
}

/// What `user` may find in the portal on `day`: for each party they act
/// for, the portal-visible types their grants allow and the client shows.
/// Empty when their portal is switched off or they act for nobody.
pub async fn reach_of(state: &AppState, user: i64, day: NaiveDate) -> Result<Vec<PortalReach>> {
    if PortalAccess::disabled(state, user).await? {
        return Ok(Vec::new());
    }
    let hidden = hidden_types(state);
    let types = portal_types();
    let mut reach = Vec::new();
    for party in PortalAccess::parties_for(&state.db, user, day).await? {
        let grants = PortalAccess::readable_grants(&state.db, user, party, day).await?;
        if grants.is_empty() {
            continue;
        }
        let permissions = crate::audiences::permissions_of(&state.db, &grants).await?;
        let allowed: Vec<String> = types
            .iter()
            .filter(|t| !hidden.iter().any(|h| h == t.subject_type()))
            .filter(|t| t.portal_permission().is_none_or(|p| permissions.contains(p)))
            .map(|t| t.subject_type().to_owned())
            .collect();
        reach.push(PortalReach {
            party_id: party.0,
            types: allowed,
        });
    }
    Ok(reach)
}

/// The records `user` finds for `text` today, best first.
pub async fn search_for(state: &AppState, user: i64, text: &str) -> Result<Vec<PortalHit>> {
    let reach = reach_of(state, user, today()).await?;
    portal_search(&state.db, &reach, text, PORTAL_LIMIT).await
}

/// `GET /portal/search?q=`: the user's own records matching the query,
/// grouped by party; the record itself when exactly one has an identifier
/// equal to the query. 403 for a user who may open the portal for nobody.
async fn page(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<SearchQuery>,
) -> Result<Response> {
    let day = today();
    let reach = reach_of(&state, user.id, day).await?;
    if reach.is_empty() {
        return Err(Error::Forbidden);
    }
    let text = query.q.trim().to_owned();
    let tx = texts::all(&lang);
    if !wl_kernel::search::searchable_query(&text) {
        let hint = (!text.is_empty()).then(|| texts::t(&lang, "wl_portal.search.short"));
        return Ok(View::typed(PortalSearchPage {
            tx,
            query: text,
            hint,
            empty: None,
            party_results: Vec::new(),
        })
        .into_response());
    }
    let hits = portal_search(&state.db, &reach, &text, PORTAL_LIMIT).await?;
    let exact: Vec<&PortalHit> = hits.iter().filter(|h| h.identifier_match).collect();
    if let [only] = exact.as_slice() {
        return Ok(Redirect::to(&only.url).into_response());
    }
    let mut party_results: Vec<PartyResults> = Vec::new();
    for r in &reach {
        let rows: Vec<ResultRow> = hits
            .iter()
            .filter(|h| h.party_id == r.party_id)
            .map(|h| ResultRow {
                title: h.title.clone(),
                secondary: h.secondary.clone(),
                label: h.label.clone(),
                status: h.status.clone(),
                url: h.url.clone(),
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let name = wl_kernel::parties::directory(&state)
            .display_name(&state.db, PartyId(r.party_id), &lang.locale)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("#{}", r.party_id));
        party_results.push(PartyResults {
            name,
            home_url: format!("/portal/{}", r.party_id),
            rows,
        });
    }
    let empty = party_results
        .is_empty()
        .then(|| texts::t(&lang, "wl_portal.search.empty"));
    Ok(View::typed(PortalSearchPage {
        tx,
        query: text,
        hint: None,
        empty,
        party_results,
    })
    .into_response())
}
