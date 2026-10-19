#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod block;
pub mod block_effects;
pub mod block_guard;
pub mod block_record;
pub mod compliance_tasks;
pub mod confirm_match;
pub mod contacts;
pub mod create;
pub mod details;
pub mod directory;
pub mod events;
pub mod evidence;
pub mod feed;
pub mod freshness;
pub mod freshness_alert;
mod machine;
pub mod notify;
pub mod pages;
pub mod party;
pub mod reasons;
pub mod relationships;
pub mod rescreen;
pub mod restore;
pub mod review_tasks;
pub mod roles;
pub mod sanctions_lists;
pub mod screening;
pub mod screening_task;
pub mod seed;
pub mod settings;
mod texts;
pub mod unblock;
pub mod visibility;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use block::{block, block_by_system, block_by_system_for_screening};
pub use block_record::BlockRecord;
pub use compliance_tasks::{
    ComplianceTask, ComplianceTaskFund, ComplianceTaskKind, ComplianceTaskParty,
    ComplianceTaskStatus, PartyStanding,
};
pub use confirm_match::{apply_confirmed_match, confirm_match};
pub use contacts::{ContactPoint, ContactStatus, ContactType, update_contact, verify_contact};
pub use create::{NewParty, create};
pub use details::{CompanyDetails, PersonDetails};
pub use directory::Directory;
pub use events::{SanctionsListStale, SanctionsListUpdated};
pub use evidence::{UnblockBasis, UnblockEvidence};
pub use feed::{FeedError, ListFeed, ListVersion, NoListFeed};
pub use party::{Channel, Language, Party};
pub use reasons::{BlockReason, BlockReasons};
pub use relationships::{
    NewRelationship, Relationship, RelationshipType, relate, related_companies,
};
pub use restore::restore;
pub use review_tasks::{
    ItemDecision, ReviewTask, ReviewTaskItem, ReviewTaskNote, ReviewTaskStatus,
};
pub use roles::PartyRole;
pub use sanctions_lists::{ListStatus, SanctionsList, UpdateSource, is_stale};
pub use screening::{
    ListUsed, NewCandidate, ProviderError, Screening, ScreeningCandidate, ScreeningMethod,
    ScreeningProvider, ScreeningRun, ScreeningTrigger, record_run,
};
pub use screening_task::{ScreeningTask, ScreeningTaskStatus};
pub use settings::ConfirmedMatchSettings;
pub use unblock::{UnblockRequest, unblock};

/// The module's permissions (wl-parties §6).
pub mod permissions {
    /// See parties, their contacts, roles and history.
    pub const VIEW: &str = "parties.view";
    /// Create parties.
    pub const CREATE: &str = "parties.create";
    /// Change a party's details.
    pub const UPDATE: &str = "parties.update";
    /// Block and unblock parties.
    pub const BLOCK: &str = "parties.block";
    /// See and use restricted block reason codes and Block records
    /// (`PTY.ACL.02`; MLRO and deputy by default).
    pub const BLOCK_REASON_VIEW: &str = "parties.block_reason.view";
    /// Export the parties list.
    pub const EXPORT: &str = "parties.export";
    /// Approve an exception that lets a guarded action go ahead for a
    /// blocked party (wl-parties §6; compliance and the finance manager).
    pub const BLOCK_EXCEPTION: &str = "parties.block_exception";
    /// Record a manual sanctions check and decide on screening matches
    /// (compliance role).
    pub const SCREENING_REVIEW: &str = "screening.review";
    /// Run a screening and see the sanctions lists and their freshness
    /// (wl-parties §6).
    pub const SCREENING_RUN: &str = "screening.run";
    /// Archive and restore parties.
    pub const ARCHIVE: &str = "parties.archive";
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-parties/index.html",
        include_str!("../views/wl-parties/index.html"),
    ),
    (
        "wl-parties/new.html",
        include_str!("../views/wl-parties/new.html"),
    ),
    (
        "wl-parties/show.html",
        include_str!("../views/wl-parties/show.html"),
    ),
    (
        "wl-parties/_card.html",
        include_str!("../views/wl-parties/_card.html"),
    ),
    (
        "wl-parties/_block_sheet.html",
        include_str!("../views/wl-parties/_block_sheet.html"),
    ),
    (
        "wl-parties/_unblock_sheet.html",
        include_str!("../views/wl-parties/_unblock_sheet.html"),
    ),
    (
        "wl-parties/_contact_sheet.html",
        include_str!("../views/wl-parties/_contact_sheet.html"),
    ),
    (
        "wl-parties/block_effects.html",
        include_str!("../views/wl-parties/block_effects.html"),
    ),
    (
        "wl-parties/review_tasks.html",
        include_str!("../views/wl-parties/review_tasks.html"),
    ),
    (
        "wl-parties/review_task.html",
        include_str!("../views/wl-parties/review_task.html"),
    ),
    (
        "wl-parties/screening_tasks.html",
        include_str!("../views/wl-parties/screening_tasks.html"),
    ),
    (
        "wl-parties/screening_task.html",
        include_str!("../views/wl-parties/screening_task.html"),
    ),
];

/// The parties module. The host adds it with `.module(wl_parties::WlParties)`
/// when `client.toml` enables `parties`; a vertical that adds block reason
/// codes uses `WlParties::new().block_reason("tax_evasion", true)`.
#[derive(Debug, Clone, Default)]
pub struct WlParties {
    /// The block reason codes (wl-parties §7): the defaults plus the
    /// vertical's; `None` until one is added, meaning the defaults.
    reasons: Option<BlockReasons>,
    /// Whether the app screens parties; the exception is then approved by
    /// `compliance` instead of `finance_manager`.
    screening: bool,
    /// The score at or above which a screening candidate is a possible
    /// match; `None` means the default (0.85).
    threshold: Option<f64>,
    /// The role that approves a block exception, when the app sets one.
    approver: Option<String>,
    /// The hours after which a sanctions list without a successful update is
    /// stale; `None` means the default (24).
    freshness_hours: Option<i64>,
    /// Downloads the sanctions lists; `None` means [`NoListFeed`], which
    /// always fails.
    list_feed: Option<std::sync::Arc<dyn ListFeed>>,
    /// Matches a party against the lists; `None` leaves a run `pending`.
    screening_provider: Option<std::sync::Arc<dyn ScreeningProvider>>,
    /// What a confirmed sanctions match does (wl-parties §7).
    confirmed: ConfirmedMatchSettings,
}

/// The module with the default block reason codes, so the host can write
/// `.module(wl_parties::WlParties)`.
#[allow(non_upper_case_globals)]
pub const WlParties: WlParties = WlParties {
    reasons: None,
    screening: false,
    threshold: None,
    approver: None,
    freshness_hours: None,
    list_feed: None,
    screening_provider: None,
    confirmed: ConfirmedMatchSettings::DEFAULT,
};

impl WlParties {
    /// The module with the default block reason codes.
    pub fn new() -> Self {
        WlParties::default()
    }

    /// Adds the block reason `code`, restricted or not (wl-parties §7). A
    /// restricted code is seen and used only under
    /// `parties.block_reason.view`; everyone else sees `on_hold`.
    ///
    /// # Panics
    ///
    /// When `code` is `on_hold` (always present, never restricted), is not
    /// a `snake_case` key, or would un-restrict a restricted code; see
    /// [`BlockReasons::add`].
    pub fn block_reason(mut self, code: &str, restricted: bool) -> Self {
        self.reasons
            .get_or_insert_with(BlockReasons::default)
            .add(code, restricted);
        self
    }

    /// Turns screening on or off (off by default). With screening on, a
    /// block exception is approved by `compliance`, else by
    /// `finance_manager`; [`WlParties::exception_approver`] overrides both.
    pub fn screening(mut self, on: bool) -> Self {
        self.screening = on;
        self
    }

    /// Sets the match threshold (wl-parties §7): a candidate scoring at or
    /// above it (`>=`) makes a screening a possible match. The default is
    /// 0.85.
    ///
    /// # Panics
    ///
    /// When `threshold` is not between 0 and 1.
    pub fn match_threshold(mut self, threshold: f64) -> Self {
        assert!(
            (0.0..=1.0).contains(&threshold),
            "the match threshold is between 0 and 1"
        );
        self.threshold = Some(threshold);
        self
    }

    /// Sets the hours after which a sanctions list without a successful
    /// update is stale (wl-parties §7; default 24).
    ///
    /// # Panics
    ///
    /// When `hours` is not positive.
    pub fn sanctions_freshness_hours(mut self, hours: i64) -> Self {
        assert!(hours > 0, "the freshness limit is at least one hour");
        self.freshness_hours = Some(hours);
        self
    }

    /// The hours after which a sanctions list is stale.
    pub fn freshness_hours(&self) -> i64 {
        self.freshness_hours
            .unwrap_or(sanctions_lists::DEFAULT_FRESHNESS_HOURS)
    }

    /// Sets the feed that downloads the sanctions lists. Without one every
    /// download fails and the lists show as stale.
    pub fn list_feed(mut self, feed: impl ListFeed) -> Self {
        self.list_feed = Some(std::sync::Arc::new(feed));
        self
    }

    /// Sets the provider that matches a party against the lists in
    /// [`screening::run`]. Without one a run stays `pending`.
    pub fn screening_provider(mut self, provider: impl ScreeningProvider) -> Self {
        self.screening_provider = Some(std::sync::Arc::new(provider));
        self
    }

    /// The score at or above which a candidate is a possible match.
    pub fn threshold(&self) -> f64 {
        self.threshold.unwrap_or(screening::DEFAULT_MATCH_THRESHOLD)
    }

    /// The role that approves a block exception, whatever the screening
    /// setting.
    pub fn exception_approver(mut self, role: &str) -> Self {
        self.approver = Some(role.to_owned());
        self
    }

    /// The role that approves a block exception in this setup.
    pub fn approver(&self) -> String {
        match (&self.approver, self.screening) {
            (Some(role), _) => role.clone(),
            (None, true) => block_guard::SCREENING_APPROVER.to_owned(),
            (None, false) => block_guard::DEFAULT_APPROVER.to_owned(),
        }
    }

    /// Whether a confirmed sanctions match also blocks the companies the
    /// person is a UBO, director or signatory of (default on).
    pub fn related_company_block(mut self, on: bool) -> Self {
        self.confirmed.related_company_block = on;
        self
    }

    /// Hours after the flag until the freeze task is due (default 24).
    ///
    /// # Panics
    ///
    /// When `hours` is negative.
    pub fn freeze_task_due_hours(mut self, hours: i64) -> Self {
        assert!(hours >= 0, "the freeze task due hours are not negative");
        self.confirmed.freeze_task_due_hours = hours;
        self
    }

    /// Hours after the flag until the report task is due (default 24, the
    /// open L-30 parameter).
    ///
    /// # Panics
    ///
    /// When `hours` is negative.
    pub fn report_task_due_hours(mut self, hours: i64) -> Self {
        assert!(hours >= 0, "the report task due hours are not negative");
        self.confirmed.report_task_due_hours = hours;
        self
    }

    /// The block reason codes this module is set up with.
    pub fn reasons(&self) -> BlockReasons {
        self.reasons.clone().unwrap_or_default()
    }
}

impl Module for WlParties {
    fn name(&self) -> &'static str {
        "wl-parties"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Party>()
            .model::<PersonDetails>()
            .model::<CompanyDetails>()
            .model::<PartyRole>()
            .model::<ContactPoint>()
            .model::<Relationship>()
            .model::<BlockRecord>()
            .model::<UnblockEvidence>()
            .model::<ReviewTask>()
            .model::<ReviewTaskItem>()
            .model::<ReviewTaskNote>()
            .model::<Screening>()
            .model::<ScreeningCandidate>()
            .model::<ScreeningTask>()
            .model::<ComplianceTask>()
            .model::<ComplianceTaskParty>()
            .model::<ComplianceTaskFund>()
            .model::<SanctionsList>()
            .view_data::<pages::IndexPage>()
            .view_data::<pages::NewPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<pages::BlockEffectsPage>()
            .view_data::<pages::ReviewTasksPage>()
            .view_data::<pages::ReviewTaskPage>()
            .view_data::<pages::ScreeningTasksPage>()
            .view_data::<pages::ScreeningTaskPage>();
        // `rnx state:diagram Party` and the admin panel find the machine.
        renox::state_machine::register::<Party>();
        renox::state_machine::register::<ReviewTask>();
        renox::state_machine::register::<Screening>();
        renox::state_machine::register::<ComplianceTask>();
        // Whether screening is on, and its match threshold.
        app.provide(screening::ScreeningSettings {
            on: self.screening,
            threshold: self.threshold(),
            freshness_hours: self.freshness_hours(),
        });
        // The list feed; the default fails, so lists show as stale.
        let feed: std::sync::Arc<dyn ListFeed> = self
            .list_feed
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(NoListFeed));
        app.provide(feed);
        // What a confirmed sanctions match does.
        app.provide(self.confirmed);
        // `PTY.RUL.02`: the hourly list check and its command.
        freshness::schedule(app);
        // Screenings made on a stale list are re-run after the list updates.
        rescreen::register(app);
        if let Some(provider) = &self.screening_provider {
            app.provide(std::sync::Arc::clone(provider));
        }
        // The compliance alert of a stale list.
        freshness_alert::register(app);
        // `confirm_match` with its second reviewer, and its effects.
        confirm_match::register(app);
        // The block reason codes; `on_hold` must be there and not restricted.
        let reasons = self.reasons();
        reasons.check();
        app.provide(reasons);
        // `PartyBlocked` after a block commits.
        block::register(app);
        // Review tasks for the agreements of a blocked party.
        block_effects::register(app);
        // The neutral on-hold message (the decider sends it).
        notify::register();
        // Other modules refuse a blocked party through the kernel guard, and
        // an exception is an approval of the action `parties.block_exception`.
        let guard = block_guard::PartiesBlockGuard::new(self.approver());
        guard.policy().register();
        wl_kernel::block_guard::provide_block_guard(app, guard);
        // Other modules read parties through the kernel (wl-parties §6).
        wl_kernel::parties::provide_directory(app, Directory);
        // `db:seed`: the demo parties (DEMO-DATA.md).
        app.seeder(|state: AppState| async move {
            seed::run(&state).await?;
            seed::lists(&state).await.map(|_| ())
        });
        // In the app group of the staff sidebar; the label is the app's
        // `nav.parties` text.
        app.nav(
            "app",
            NavLink::route("wl_parties.index", "nav.parties")
                .icon("handshake")
                .group("nav.app")
                .order(30)
                .permission(permissions::VIEW),
        );
        // What a block does to each kind of agreement (read-only), and the
        // review tasks the user decides on.
        app.nav(
            "app",
            NavLink::route("wl_parties.block_effects", "nav.block_effects")
                .icon("shield")
                .group("nav.app")
                .order(31)
                .permission(permissions::BLOCK),
        );
        app.nav(
            "app",
            NavLink::route("wl_parties.review_tasks", "nav.review_tasks")
                .icon("list-checks")
                .group("nav.app")
                .order(32),
        );
        // The possible-match tasks, for the compliance role only (#38 A10).
        app.nav(
            "app",
            NavLink::route("wl_parties.screening_tasks", "nav.screening_tasks")
                .icon("shield-alert")
                .group("nav.app")
                .order(33)
                .permission(permissions::BLOCK_REASON_VIEW),
        );
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_has_the_default_reasons_unless_a_vertical_adds_one() {
        assert_eq!(WlParties.reasons(), BlockReasons::default());
        assert_eq!(WlParties::new().reasons(), BlockReasons::default());
        let module = WlParties::new().block_reason("tax_evasion", true);
        let reasons = module.reasons();
        assert!(reasons.is_restricted("tax_evasion"));
        assert_eq!(reasons.all().len(), 8);
        reasons.check();
    }

    #[test]
    fn the_exception_approver_follows_screening_unless_set() {
        assert_eq!(WlParties::new().approver(), "finance_manager");
        assert_eq!(WlParties::new().screening(true).approver(), "compliance");
        let set = WlParties::new().screening(true).exception_approver("cfo");
        assert_eq!(set.approver(), "cfo");
    }

    #[test]
    fn the_confirmed_match_settings_default_and_can_be_set() {
        let defaults = WlParties::new().confirmed;
        assert!(defaults.related_company_block);
        assert_eq!(defaults.freeze_task_due_hours, 24);
        assert_eq!(defaults.report_task_due_hours, 24);
        let module = WlParties::new()
            .related_company_block(false)
            .freeze_task_due_hours(12)
            .report_task_due_hours(48);
        assert!(!module.confirmed.related_company_block);
        assert_eq!(module.confirmed.freeze_task_due_hours, 12);
        assert_eq!(module.confirmed.report_task_due_hours, 48);
    }

    #[test]
    fn the_match_threshold_defaults_to_085_unless_set() {
        assert_eq!(WlParties::new().threshold(), 0.85);
        assert_eq!(WlParties::new().match_threshold(0.9).threshold(), 0.9);
    }

    #[test]
    fn the_freshness_limit_defaults_to_24_hours_unless_set() {
        assert_eq!(WlParties::new().freshness_hours(), 24);
        let module = WlParties::new().sanctions_freshness_hours(12);
        assert_eq!(module.freshness_hours(), 12);
    }

    #[test]
    #[should_panic(expected = "at least one hour")]
    fn the_freshness_limit_must_be_positive() {
        let _ = WlParties::new().sanctions_freshness_hours(0);
    }

    #[test]
    #[should_panic(expected = "between 0 and 1")]
    fn the_match_threshold_must_be_a_score() {
        let _ = WlParties::new().match_threshold(1.5);
    }

    #[test]
    #[should_panic(expected = "on_hold")]
    fn a_vertical_cannot_restrict_on_hold() {
        let _ = WlParties::new().block_reason("on_hold", true);
    }
}
