//! The portal's sections and menu (wl-portal §6, `PortalSections`, issue #44,
//! BL-POR-02): the lease, the invoices, the statements and the requests of
//! the party a user acts for. A module registers a [`PortalSection`] with its
//! audience, the portal permission it needs, its order and its label; the
//! portal home `GET /portal/{party}` shows the menu of the sections the
//! user's grants for the party allow, in order, and the page
//! `GET /portal/{party}/{section}` lists a section's rows. A section of
//! another audience, or one the grants' roles do not allow, is a 403 even by
//! URL. `GET /portal` opens the first party the user acts for; a user with
//! grants for several parties switches between them on the home page.
//!
//! The menu is built from the grants and the registered sections only: the
//! party's block is never read for it (`PTY.ACL.02`). A `read_only` grant
//! (BL-POR-03) shows every section it allows with the notice *Your access is
//! read-only since (date)*; its write actions are refused by
//! [`PortalGuard`](crate::PortalGuard).
//!
//! Reading is never refused because the party is blocked, and the page shows
//! each row with the status its module stored (*Submitted*, *In review*,
//! *Paid*): the portal does not look at the block here, so no block status,
//! badge or reason can appear (`PTY.ACL.02`, no tipping off).

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use crate::access::PortalAccess;
use crate::grants::AccessGrant;
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

/// The order of a section that does not say (after the ones that do).
pub const DEFAULT_ORDER: i32 = 1000;

/// A section a module offers in the portal.
///
/// Only [`title`](PortalSection::title) and [`rows`](PortalSection::rows) are
/// required. The other fields default to every audience, no permission,
/// order [`DEFAULT_ORDER`] and the title as label; a section of a real
/// vertical names its audience and permission.
pub trait PortalSection: Send + Sync + 'static {
    /// The English title.
    fn title(&self) -> String;

    /// The rows for `party`, with the statuses the module stored.
    fn rows<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> SectionFuture<'a, Vec<SectionRow>>;

    /// The audience it is for (`tenant`, `owner`, `client`); `None` for
    /// every audience.
    fn audience(&self) -> Option<&str> {
        None
    }

    /// The portal permission a grant's roles must give, e.g.
    /// `portal.invoices.view`; `None` for none.
    fn permission(&self) -> Option<&str> {
        None
    }

    /// Its place in the menu, smallest first.
    fn order(&self) -> i32 {
        DEFAULT_ORDER
    }

    /// The menu label in `lang` (English and Arabic); the title by default.
    fn label(&self, lang: &Lang) -> String {
        let _ = lang;
        self.title()
    }
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

/// Every registered section with its key.
fn all_sections() -> Vec<(String, Arc<dyn PortalSection>)> {
    SECTIONS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(key, section)| (key.clone(), section.clone()))
        .collect()
}

/// Whether `section` shows for grants of `audiences` giving `permissions`.
fn allowed(
    section: &dyn PortalSection,
    audiences: &BTreeSet<String>,
    permissions: &BTreeSet<String>,
) -> bool {
    section.audience().is_none_or(|a| audiences.contains(a))
        && section.permission().is_none_or(|p| permissions.contains(p))
}

/// One entry of the portal menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MenuEntry {
    /// The section key, e.g. `invoices`.
    pub key: String,
    /// The label in the user's language.
    pub label: String,
    /// The section page.
    pub url: String,
    /// Its place in the menu.
    pub order: i32,
}

/// What a user may see of the portal for one party.
#[derive(Debug, Clone)]
pub struct PortalMenu {
    /// The party.
    pub party: PartyId,
    /// The allowed sections, in order.
    pub entries: Vec<MenuEntry>,
    /// The audiences of the user's grants for the party.
    pub audiences: BTreeSet<String>,
    /// The union of the permissions of the grants' roles.
    pub permissions: BTreeSet<String>,
    /// When every grant for the party is `read_only`: the day it became so.
    pub read_only_since: Option<NaiveDate>,
}

impl PortalMenu {
    /// The menu of `user` for `party` on `day`; `None` when the user may not
    /// open the portal for it (no `active` or `read_only` grant, or their
    /// portal is switched off).
    pub async fn of(
        state: &AppState,
        user: i64,
        party: PartyId,
        day: NaiveDate,
        lang: &Lang,
    ) -> Result<Option<PortalMenu>> {
        if PortalAccess::disabled(state, user).await? {
            return Ok(None);
        }
        let grants = PortalAccess::readable_grants(&state.db, user, party, day).await?;
        if grants.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self::from_grants(&state.db, party, &grants, day, lang).await?))
    }

    /// The menu the readable `grants` for `party` give.
    pub async fn from_grants(
        db: &Db,
        party: PartyId,
        grants: &[AccessGrant],
        day: NaiveDate,
        lang: &Lang,
    ) -> Result<PortalMenu> {
        let audiences: BTreeSet<String> = grants.iter().map(|g| g.audience.clone()).collect();
        let permissions = crate::audiences::permissions_of(db, grants).await?;
        let read_only_since = if grants.iter().any(|g| g.is_active_on(day)) {
            None
        } else {
            Some(grants.iter().filter_map(|g| g.ended_on).max().unwrap_or(day))
        };
        let mut entries: Vec<MenuEntry> = all_sections()
            .into_iter()
            .filter(|(_, section)| allowed(section.as_ref(), &audiences, &permissions))
            .map(|(key, section)| MenuEntry {
                url: format!("/portal/{}/{key}", party.0),
                label: section.label(lang),
                order: section.order(),
                key,
            })
            .collect();
        entries.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.key.cmp(&b.key)));
        Ok(PortalMenu {
            party,
            entries,
            audiences,
            permissions,
            read_only_since,
        })
    }

    /// Whether the section `key` is in the menu.
    pub fn allows(&self, key: &str) -> bool {
        self.entries.iter().any(|e| e.key == key)
    }

    /// The section keys, in menu order.
    pub fn keys(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.key.clone()).collect()
    }
}

/// A party the user may switch to.
#[derive(Debug, Clone, Serialize)]
pub struct PartyLink {
    /// The party's name.
    pub name: String,
    /// Its portal home.
    pub url: String,
    /// Whether it is the party shown.
    pub current: bool,
}

/// What the section page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-portal/section.html")]
pub struct SectionPage {
    tx: BTreeMap<&'static str, String>,
    title: String,
    rows: Vec<SectionRow>,
    read_only_notice: Option<String>,
    home_url: String,
}

/// What the portal home needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-portal/home.html")]
pub struct HomePage {
    tx: BTreeMap<&'static str, String>,
    party_name: String,
    menu_entries: Vec<MenuEntry>,
    party_links: Vec<PartyLink>,
    read_only_notice: Option<String>,
}

/// The routes of the portal home and the sections.
pub fn routes() -> Routes {
    Routes::new()
        .get("/portal", start)
        .name("wl_portal.start")
        .get("/portal/{party}", home)
        .name("wl_portal.home")
        .get("/portal/{party}/{section}", show)
        .name("wl_portal.section")
        .require_auth()
}

/// The display name of `party`.
async fn name_of(state: &AppState, party: PartyId, lang: &Lang) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, party, &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", party.0))
}

/// `GET /portal`: the home of the first party the user acts for; 403 for a
/// user with no grant to read.
async fn start(State(state): State<AppState>, user: AuthUser) -> Result<Response> {
    if PortalAccess::disabled(&state, user.id).await? {
        return Err(Error::Forbidden);
    }
    let parties = PortalAccess::parties_for(&state.db, user.id, today()).await?;
    let first = parties.first().ok_or(Error::Forbidden)?;
    Ok(Redirect::to(&format!("/portal/{}", first.0)).into_response())
}

/// `GET /portal/{party}`: the menu of the sections the user's grants for the
/// party allow, in order, the parties to switch to and, for a `read_only`
/// grant, the notice; 403 without a grant to read.
async fn home(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(party): Path<i64>,
) -> Result<View> {
    let day = today();
    let menu = PortalMenu::of(&state, user.id, PartyId(party), day, &lang)
        .await?
        .ok_or(Error::Forbidden)?;
    let mut party_links = Vec::new();
    for other in PortalAccess::parties_for(&state.db, user.id, day).await? {
        party_links.push(PartyLink {
            name: name_of(&state, other, &lang).await,
            url: format!("/portal/{}", other.0),
            current: other.0 == party,
        });
    }
    Ok(View::typed(HomePage {
        tx: texts::all(&lang),
        party_name: name_of(&state, PartyId(party), &lang).await,
        menu_entries: menu.entries.clone(),
        party_links,
        read_only_notice: menu
            .read_only_since
            .map(|since| texts::read_only_notice(&lang, since)),
    }))
}

/// `GET /portal/{party}/{section}`: the rows, for a user whose grants for
/// the party (`active` or `read_only`) allow the section; 403 otherwise,
/// also for a section of another audience. The block is not asked.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((party, key)): Path<(i64, String)>,
) -> Result<View> {
    let menu = PortalMenu::of(&state, user.id, PartyId(party), today(), &lang)
        .await?
        .ok_or(Error::Forbidden)?;
    let section = section(&key).ok_or(Error::NotFound)?;
    if !menu.allows(&key) {
        return Err(Error::Forbidden);
    }
    let rows = section.rows(&state, PartyId(party)).await?;
    Ok(View::typed(SectionPage {
        tx: texts::all(&lang),
        title: section.label(&lang),
        rows,
        read_only_notice: menu
            .read_only_since
            .map(|since| texts::read_only_notice(&lang, since)),
        home_url: format!("/portal/{party}"),
    }))
}
