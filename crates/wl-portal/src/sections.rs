//! The portal's read-only sections (wl-portal §6, `PortalSections`, issue #44):
//! the lease, the invoices, the statements and the requests of the party a
//! user acts for. A module registers a [`PortalSection`]; the page
//! `GET /portal/{party}/{section}` lists its rows for a user who holds an
//! `active` grant for the party.
//!
//! Reading is never refused because the party is blocked, and the page shows
//! each row with the status its module stored (*Submitted*, *In review*,
//! *Paid*): the portal does not look at the block here, so no block status,
//! badge or reason can appear (`PTY.ACL.02`, no tipping off).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use crate::access::PortalAccess;
use crate::texts;

/// The future a section answers with.
pub type SectionFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// One line of a section: a lease, an invoice, a statement, a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionRow {
    /// What it is, e.g. `Refund request RR-12`.
    pub label: String,
    /// The status its module stored, e.g. `In review`.
    pub status: String,
}

impl SectionRow {
    /// A row.
    pub fn new(label: &str, status: &str) -> Self {
        SectionRow {
            label: label.to_owned(),
            status: status.to_owned(),
        }
    }
}

/// A section a module offers in the portal.
pub trait PortalSection: Send + Sync + 'static {
    /// The English title.
    fn title(&self) -> String;

    /// The rows for `party`, with the statuses the module stored.
    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>>;
}

static SECTIONS: RwLock<BTreeMap<String, Arc<dyn PortalSection>>> = RwLock::new(BTreeMap::new());

/// Adds `section` under `key` (e.g. `invoices`) to the process-wide registry;
/// registering the same key again replaces it.
pub fn register_section(key: &str, section: impl PortalSection) {
    SECTIONS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key.to_owned(), Arc::new(section));
}

/// The section registered under `key`.
pub fn section(key: &str) -> Option<Arc<dyn PortalSection>> {
    SECTIONS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .cloned()
}

/// What the section page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-portal/section.html")]
pub struct SectionPage {
    tx: BTreeMap<&'static str, String>,
    title: String,
    rows: Vec<SectionRow>,
}

/// The routes of the sections.
pub fn routes() -> Routes {
    Routes::new()
        .get("/portal/{party}/{section}", show)
        .name("wl_portal.section")
        .require_auth()
}

/// `GET /portal/{party}/{section}`: the rows, for a user with an `active`
/// grant for the party; 403 without one. The block is not asked.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((party, key)): Path<(i64, String)>,
) -> Result<View> {
    if PortalAccess::disabled(&state, user.id).await?
        || PortalAccess::active_grant(&state.db, user.id, PartyId(party), today())
            .await?
            .is_none()
    {
        return Err(Error::Forbidden);
    }
    let section = section(&key).ok_or(Error::NotFound)?;
    let rows = section.rows(&state, PartyId(party)).await?;
    Ok(View::typed(SectionPage {
        tx: texts::all(&lang),
        title: section.title(),
        rows,
    }))
}
