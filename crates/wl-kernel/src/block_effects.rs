//! The block-effect contract ([ARCHITECTURE.md] §5, wl-parties §5.6, §7): what
//! blocking a party does to each kind of active agreement it holds.
//!
//! Blocking a party ends nothing by itself. Every kind of agreement
//! (subscription, booking, lease, ...) has a [`BlockEffect`] and a decider
//! role. The module that owns the kind registers it with [`AgreementKind`],
//! the vertical overrides it with [`BlockEffectOverride`], both in their
//! `register()`, and the module that reacts to `PartyBlocked` asks
//! [`effect_for`]:
//!
//! ```ignore
//! AgreementKind::new("subscription")
//!     .owner("wl-invoicing")
//!     .effect(BlockEffect::Pause)
//!     .decider("subscriptions_manager")
//!     .source(SubscriptionAgreements)
//!     .register();
//!
//! // in the vertical:
//! BlockEffectOverride::kind("deployment")
//!     .effect(BlockEffect::Continue)
//!     .decider("managing_director")
//!     .register();
//!
//! let rule = block_effects::effect_for("subscription", &scope);
//! ```
//!
//! The kernel ships the 12 kinds of wl-parties §5.6 as built-in defaults
//! ([`BUILT_IN`]), so [`kinds`] lists them before any owning module exists.
//! Precedence, per field: the vertical's override, then the module's
//! registration, then the built-in default. A kind nobody registered gets
//! [`BlockEffect::Review`], the safest, and no decider.
//!
//! The registries are process-wide and independent of registration order:
//! overrides are stored apart from the registrations.
//!
//! [ARCHITECTURE.md]: https://github.com/renox-rs/white-label/blob/main/docs/ARCHITECTURE.md

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::auth::permissions::Scope;
use renox::{AppState, Result};
use serde::{Deserialize, Serialize};

use crate::parties::{PartyId, PartyStatus, directory};
use crate::subject::Subject;

/// What a block does to an agreement of one kind (wl-parties §5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockEffect {
    /// The agreement stays, service and billing stop, and it resumes on
    /// unblock. Money out's "stop until unblock" is stored as this.
    Pause,
    /// The agreement runs to its normal end; nothing changes. Refusing
    /// renewal and extension is the block guard's work, not this registry's.
    Continue,
    /// Nothing changes on its own; the decider gets a task to pause,
    /// continue or end each agreement.
    Review,
}

impl BlockEffect {
    /// The stored name: `pause`, `continue`, `review`.
    pub fn as_str(self) -> &'static str {
        match self {
            BlockEffect::Pause => "pause",
            BlockEffect::Continue => "continue",
            BlockEffect::Review => "review",
        }
    }
}

impl fmt::Display for BlockEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a [`BlockEffectRule`] came from. Precedence: the vertical, then the
/// module, then the built-in default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleSource {
    /// The kernel's built-in default, or an unregistered kind.
    Default,
    /// The owning module's registration.
    Module,
    /// The vertical's [`BlockEffectOverride`].
    Vertical,
}

impl RuleSource {
    /// The stored name: `default`, `module`, `vertical`.
    pub fn as_str(self) -> &'static str {
        match self {
            RuleSource::Default => "default",
            RuleSource::Module => "module",
            RuleSource::Vertical => "vertical",
        }
    }
}

/// The answer of [`effect_for`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockEffectRule {
    /// The agreement kind's key.
    pub kind: String,
    /// What the block does to agreements of this kind.
    pub effect: BlockEffect,
    /// The role key that decides for this kind; `None` for an unregistered
    /// kind. The role itself is defined by the host or the vertical.
    pub decider_role: Option<String>,
    /// The scope the question was asked for: the decider holds the role in it.
    pub scope: Scope,
    /// Which layer the answer comes from.
    pub source: RuleSource,
}

/// What the decider chooses for one agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Stop service and billing; resumes on unblock.
    Pause,
    /// Leave the agreement running.
    Continue,
    /// End the agreement through its own transition, with its normal money
    /// effects.
    End,
}

impl Decision {
    /// Every decision.
    pub const ALL: [Decision; 3] = [Decision::Pause, Decision::Continue, Decision::End];

    /// The stored name: `pause`, `continue`, `end`.
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Pause => "pause",
            Decision::Continue => "continue",
            Decision::End => "end",
        }
    }
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One open agreement of a party, as listed in a review task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgreementRef {
    /// The agreement, e.g. `scheduling.booking:12`.
    pub subject: Subject,
    /// A one-line label for the task.
    pub label: String,
    /// Where the agreement lives (branch, clinic, ...); `None` when global.
    /// Tasks are grouped by it and go to the decider role in it.
    pub scope: Option<Scope>,
}

/// The future an [`AgreementSource`] method returns.
pub type AgreementFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The owning module's side of a review: it lists the party's open
/// agreements and carries out the decider's choice.
///
/// The contract:
/// - **No automatic ending.** `decide` runs only when a decider chose; nothing
///   in the block ends an agreement by itself.
/// - The pause or hold reason recorded on the agreement is
///   [`NEUTRAL_BLOCK_REASON`](crate::parties::NEUTRAL_BLOCK_REASON), never the
///   real reason.
/// - [`Decision::End`] goes through the agreement's own `end`, `cancel` or
///   `withdraw` transition with its normal money effects (final invoice,
///   refund or credit). `Pause` and `Continue` go through its own
///   transitions too; the caller never changes an agreement itself.
/// - A refusal is an `Err`: the caller records no decision for it.
pub trait AgreementSource: Send + Sync + 'static {
    /// The party's open agreements of this kind.
    fn open_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> AgreementFuture<'a, Vec<AgreementRef>>;

    /// Carries out `decision` on `subject` as the user with id `user`.
    fn decide<'a>(
        &'a self,
        state: &'a AppState,
        user: i64,
        subject: &'a Subject,
        decision: Decision,
    ) -> AgreementFuture<'a, ()>;
}

/// A built-in kind of wl-parties §5.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltInKind {
    /// The kind's key.
    pub key: &'static str,
    /// The module or vertical that owns agreements of this kind.
    pub owner: &'static str,
    /// The default effect.
    pub effect: BlockEffect,
    /// The default decider role key.
    pub decider_role: &'static str,
}

const fn built_in(
    key: &'static str,
    owner: &'static str,
    effect: BlockEffect,
    decider_role: &'static str,
) -> BuiltInKind {
    BuiltInKind {
        key,
        owner,
        effect,
        decider_role,
    }
}

/// The 12 kinds of wl-parties §5.6 with their default effect and decider
/// role. Money out's "stop until unblock" is [`BlockEffect::Pause`].
pub const BUILT_IN: [BuiltInKind; 12] = [
    built_in("engagement", "vertical", BlockEffect::Pause, "compliance"),
    built_in(
        "subscription",
        "wl-invoicing",
        BlockEffect::Pause,
        "subscriptions_manager",
    ),
    built_in(
        "enrolment",
        "vertical",
        BlockEffect::Pause,
        "centre_manager",
    ),
    built_in(
        "lease",
        "vertical",
        BlockEffect::Continue,
        "property_manager",
    ),
    built_in(
        "deployment",
        "vertical",
        BlockEffect::Continue,
        "managing_director",
    ),
    built_in(
        "project",
        "vertical",
        BlockEffect::Continue,
        "managing_director",
    ),
    built_in(
        "sales_order",
        "wl-sales",
        BlockEffect::Pause,
        "sales_manager",
    ),
    built_in(
        "booking",
        "wl-scheduling",
        BlockEffect::Review,
        "branch_manager",
    ),
    built_in(
        "purchase_order",
        "wl-inventory",
        BlockEffect::Continue,
        "procurement",
    ),
    built_in(
        "service_contract",
        "wl-ticketing",
        BlockEffect::Review,
        "facility_manager",
    ),
    built_in("money_out", "wl-payments", BlockEffect::Pause, "compliance"),
    built_in(
        "money_in",
        "wl-payments",
        BlockEffect::Continue,
        "compliance",
    ),
];

/// A kind as the settings list shows it: the effective values at the
/// deployment level.
#[derive(Clone)]
pub struct KindInfo {
    /// The kind's key.
    pub key: String,
    /// The owning module or vertical.
    pub owner: String,
    /// The text key of the label: `wl_kernel.block_kind.<key>`.
    pub label_key: String,
    /// The effective effect.
    pub effect: BlockEffect,
    /// The effective decider role.
    pub decider_role: Option<String>,
    /// Which layer the effective values come from.
    pub source: RuleSource,
    /// Whether a module registered an [`AgreementSource`]. Without one the
    /// kind is listed but gets no review task.
    pub has_source: bool,
}

impl fmt::Debug for KindInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KindInfo")
            .field("key", &self.key)
            .field("owner", &self.owner)
            .field("effect", &self.effect)
            .field("decider_role", &self.decider_role)
            .field("source", &self.source)
            .field("has_source", &self.has_source)
            .finish()
    }
}

/// The text key of a kind's label.
pub fn label_key(kind: &str) -> String {
    format!("wl_kernel.block_kind.{kind}")
}

#[derive(Clone)]
struct Registered {
    owner: Option<String>,
    effect: Option<BlockEffect>,
    decider_role: Option<String>,
    source: Option<Arc<dyn AgreementSource>>,
}

#[derive(Clone, Default)]
struct Overridden {
    effect: Option<BlockEffect>,
    decider_role: Option<String>,
}

/// The registrations and overrides, kept apart so that registration order
/// never matters.
struct Book {
    modules: BTreeMap<String, Registered>,
    overrides: BTreeMap<String, Overridden>,
    announcing: BTreeSet<String>,
}

impl Book {
    const fn new() -> Self {
        Book {
            modules: BTreeMap::new(),
            overrides: BTreeMap::new(),
            announcing: BTreeSet::new(),
        }
    }

    fn rule(&self, kind: &str, scope: Scope) -> BlockEffectRule {
        let built = BUILT_IN.iter().find(|b| b.key == kind);
        let module = self.modules.get(kind);
        let over = self.overrides.get(kind);

        let effect = over
            .and_then(|o| o.effect)
            .or_else(|| module.and_then(|m| m.effect))
            .or_else(|| built.map(|b| b.effect))
            .unwrap_or(BlockEffect::Review);
        let decider_role = over
            .and_then(|o| o.decider_role.clone())
            .or_else(|| module.and_then(|m| m.decider_role.clone()))
            .or_else(|| built.map(|b| b.decider_role.to_owned()));
        let source = if over.is_some_and(|o| o.effect.is_some() || o.decider_role.is_some()) {
            RuleSource::Vertical
        } else if module.is_some_and(|m| m.effect.is_some() || m.decider_role.is_some()) {
            RuleSource::Module
        } else {
            RuleSource::Default
        };
        BlockEffectRule {
            kind: kind.to_owned(),
            effect,
            decider_role,
            scope,
            source,
        }
    }

    fn kinds(&self) -> Vec<KindInfo> {
        let keys: BTreeSet<String> = BUILT_IN
            .iter()
            .map(|b| b.key.to_owned())
            .chain(self.modules.keys().cloned())
            .collect();
        let mut out: Vec<KindInfo> = keys
            .into_iter()
            .map(|key| {
                let built = BUILT_IN.iter().find(|b| b.key == key);
                let module = self.modules.get(&key);
                let rule = self.rule(&key, Scope::global());
                let owner = module
                    .and_then(|m| m.owner.clone())
                    .or_else(|| built.map(|b| b.owner.to_owned()))
                    .unwrap_or_default();
                KindInfo {
                    label_key: label_key(&key),
                    owner,
                    effect: rule.effect,
                    decider_role: rule.decider_role,
                    source: rule.source,
                    has_source: module.is_some_and(|m| m.source.is_some()),
                    key,
                }
            })
            .collect();
        // Built-in kinds first, in the order of §5.6, then the added ones by key.
        let rank = |key: &str| BUILT_IN.iter().position(|b| b.key == key);
        out.sort_by(|a, b| match (rank(&a.key), rank(&b.key)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.key.cmp(&b.key),
        });
        out
    }
}

static BOOK: RwLock<Book> = RwLock::new(Book::new());

fn read<T>(f: impl FnOnce(&Book) -> T) -> T {
    f(&BOOK.read().unwrap_or_else(PoisonError::into_inner))
}

fn write<T>(f: impl FnOnce(&mut Book) -> T) -> T {
    f(&mut BOOK.write().unwrap_or_else(PoisonError::into_inner))
}

/// Registers an agreement kind: the owning module's side. Registering the
/// same key again replaces the earlier registration. Fields left unset keep
/// the built-in default (or, for a new key, mean *review* and no decider).
pub struct AgreementKind {
    key: String,
    entry: Registered,
}

impl AgreementKind {
    /// A kind with this key, e.g. `subscription`.
    pub fn new(key: impl Into<String>) -> Self {
        AgreementKind {
            key: key.into(),
            entry: Registered {
                owner: None,
                effect: None,
                decider_role: None,
                source: None,
            },
        }
    }

    /// The owning module, e.g. `wl-invoicing`.
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.entry.owner = Some(owner.into());
        self
    }

    /// The module's default effect.
    pub fn effect(mut self, effect: BlockEffect) -> Self {
        self.entry.effect = Some(effect);
        self
    }

    /// The module's default decider role key.
    pub fn decider(mut self, role: impl Into<String>) -> Self {
        self.entry.decider_role = Some(role.into());
        self
    }

    /// Lists the party's open agreements and carries out decisions. Needed
    /// for a *review* kind to get a task.
    pub fn source(mut self, source: impl AgreementSource) -> Self {
        self.entry.source = Some(Arc::new(source));
        self
    }

    /// Adds the kind to the process-wide registry.
    pub fn register(self) {
        write(|book| book.modules.insert(self.key, self.entry));
    }
}

/// The vertical's override of one kind's effect and decider. Overrides are
/// per kind for the whole deployment and win over the module and the
/// built-in default. Registering the same key again replaces the earlier
/// override.
pub struct BlockEffectOverride {
    key: String,
    entry: Overridden,
}

impl BlockEffectOverride {
    /// An override of the kind `key`.
    pub fn kind(key: impl Into<String>) -> Self {
        BlockEffectOverride {
            key: key.into(),
            entry: Overridden::default(),
        }
    }

    /// Overrides the effect.
    pub fn effect(mut self, effect: BlockEffect) -> Self {
        self.entry.effect = Some(effect);
        self
    }

    /// Overrides the decider role key.
    pub fn decider(mut self, role: impl Into<String>) -> Self {
        self.entry.decider_role = Some(role.into());
        self
    }

    /// Adds the override to the process-wide registry.
    pub fn register(self) {
        write(|book| book.overrides.insert(self.key, self.entry));
    }
}

/// What a block does to agreements of `kind` in `scope`: the override, then
/// the module's registration, then the built-in default. An unknown kind is
/// `{ effect: Review, decider_role: None, source: Default }`.
///
/// `scope` is carried into the result, so the task goes to the decider role
/// in that scope; overrides are per kind for now.
pub fn effect_for(kind: &str, scope: &Scope) -> BlockEffectRule {
    read(|book| book.rule(kind, scope.clone()))
}

/// Every known kind (the 12 built-in ones, then those modules added), with
/// the effective effect and decider at the deployment level.
pub fn kinds() -> Vec<KindInfo> {
    read(Book::kinds)
}

/// The [`AgreementSource`] a module registered for `kind`, if any.
pub fn source_for(kind: &str) -> Option<Arc<dyn AgreementSource>> {
    read(|book| book.modules.get(kind).and_then(|m| m.source.clone()))
}

/// Marks the notification type `type_key` as one that announces a pause (or
/// hold) to the party. While the party is blocked such a message is
/// suppressed (wl-parties §5.6): the decider chooses what to tell the party.
/// Owning modules mark their types; `party.on_hold.neutral` is never marked.
pub fn mark_announces_pause(type_key: &str) {
    write(|book| book.announcing.insert(type_key.to_owned()));
}

/// Whether `type_key` was marked with [`mark_announces_pause`].
pub fn announces_pause(type_key: &str) -> bool {
    read(|book| book.announcing.contains(type_key))
}

/// Whether a message of `type_key` to `party` must be suppressed: the type
/// announces a pause and the party is `blocked` according to the
/// [`PartyDirectory`](crate::parties::PartyDirectory). `wl-notify` checks
/// this when it resolves a party recipient and ends the message `suppressed`
/// with the reason `party_on_hold`.
pub async fn suppressed_while_blocked(
    state: &AppState,
    type_key: &str,
    party: PartyId,
) -> Result<bool> {
    if !announces_pause(type_key) {
        return Ok(false);
    }
    let status = directory(state).status(&state.db, party).await?;
    Ok(status == Some(PartyStatus::Blocked))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> Scope {
        Scope::new("branches", 7)
    }

    fn rule(book: &Book, kind: &str) -> BlockEffectRule {
        book.rule(kind, here())
    }

    fn module(effect: Option<BlockEffect>, decider: Option<&str>) -> Registered {
        Registered {
            owner: Some("test".into()),
            effect,
            decider_role: decider.map(Into::into),
            source: None,
        }
    }

    #[test]
    fn the_twelve_built_in_kinds_have_their_defaults() {
        let book = Book::new();
        let all = book.kinds();
        let keys: Vec<_> = all.iter().map(|k| k.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "engagement",
                "subscription",
                "enrolment",
                "lease",
                "deployment",
                "project",
                "sales_order",
                "booking",
                "purchase_order",
                "service_contract",
                "money_out",
                "money_in",
            ]
        );
        let booking = rule(&book, "booking");
        assert_eq!(booking.effect, BlockEffect::Review);
        assert_eq!(booking.decider_role.as_deref(), Some("branch_manager"));
        assert_eq!(booking.source, RuleSource::Default);
        assert_eq!(booking.scope, here());
        assert_eq!(rule(&book, "lease").effect, BlockEffect::Continue);
        assert_eq!(rule(&book, "money_out").effect, BlockEffect::Pause);
        assert_eq!(
            rule(&book, "money_out").decider_role.as_deref(),
            Some("compliance")
        );
        assert!(all.iter().all(|k| !k.has_source));
        assert_eq!(all[0].label_key, "wl_kernel.block_kind.engagement");
    }

    #[test]
    fn an_unknown_kind_is_reviewed_with_no_decider() {
        let book = Book::new();
        let unknown = rule(&book, "unregistered_kind");
        assert_eq!(unknown.effect, BlockEffect::Review);
        assert_eq!(unknown.decider_role, None);
        assert_eq!(unknown.source, RuleSource::Default);
    }

    #[test]
    fn the_vertical_beats_the_module_which_beats_the_default() {
        let mut book = Book::new();
        book.modules.insert(
            "subscription".into(),
            module(Some(BlockEffect::Continue), Some("billing_clerk")),
        );
        let by_module = rule(&book, "subscription");
        assert_eq!(by_module.effect, BlockEffect::Continue);
        assert_eq!(by_module.decider_role.as_deref(), Some("billing_clerk"));
        assert_eq!(by_module.source, RuleSource::Module);

        book.overrides.insert(
            "subscription".into(),
            Overridden {
                effect: None,
                decider_role: Some("finance_manager".into()),
            },
        );
        let by_vertical = rule(&book, "subscription");
        // Per field: the effect still comes from the module.
        assert_eq!(by_vertical.effect, BlockEffect::Continue);
        assert_eq!(by_vertical.decider_role.as_deref(), Some("finance_manager"));
        assert_eq!(by_vertical.source, RuleSource::Vertical);
    }

    #[test]
    fn a_module_that_sets_nothing_keeps_the_built_in_default() {
        let mut book = Book::new();
        book.modules
            .insert("subscription".into(), module(None, None));
        let r = rule(&book, "subscription");
        assert_eq!(r.effect, BlockEffect::Pause);
        assert_eq!(r.decider_role.as_deref(), Some("subscriptions_manager"));
        assert_eq!(r.source, RuleSource::Default);
    }

    #[test]
    fn an_override_alone_works_on_a_built_in_and_on_an_unregistered_kind() {
        let mut book = Book::new();
        book.overrides.insert(
            "deployment".into(),
            Overridden {
                effect: Some(BlockEffect::Continue),
                decider_role: Some("managing_director".into()),
            },
        );
        book.overrides.insert(
            "ghost".into(),
            Overridden {
                effect: Some(BlockEffect::Pause),
                decider_role: None,
            },
        );
        let deployment = rule(&book, "deployment");
        assert_eq!(deployment.source, RuleSource::Vertical);
        assert_eq!(deployment.effect, BlockEffect::Continue);
        let ghost = rule(&book, "ghost");
        assert_eq!(
            (ghost.effect, ghost.decider_role),
            (BlockEffect::Pause, None)
        );
        // An override does not add a kind to the list.
        assert_eq!(book.kinds().len(), 12);
    }

    #[test]
    fn a_new_key_adds_a_kind_after_the_built_in_ones() {
        let mut book = Book::new();
        book.modules
            .insert("warranty".into(), module(Some(BlockEffect::Pause), None));
        book.modules
            .insert("alpha".into(), module(None, Some("lead")));
        let all = book.kinds();
        assert_eq!(all.len(), 14);
        assert_eq!(all[12].key, "alpha");
        assert_eq!(all[12].owner, "test");
        assert_eq!(all[13].key, "warranty");
    }

    #[test]
    fn the_global_registry_applies_the_override_whatever_the_order() {
        BlockEffectOverride::kind("t_order_kind")
            .effect(BlockEffect::Continue)
            .decider("vertical_role")
            .register();
        AgreementKind::new("t_order_kind")
            .owner("wl-test")
            .effect(BlockEffect::Pause)
            .decider("module_role")
            .register();
        let r = effect_for("t_order_kind", &here());
        assert_eq!(r.effect, BlockEffect::Continue);
        assert_eq!(r.decider_role.as_deref(), Some("vertical_role"));
        assert_eq!(r.source, RuleSource::Vertical);
        assert!(kinds().iter().any(|k| k.key == "t_order_kind"));
        assert!(source_for("t_order_kind").is_none());
        assert_eq!(
            effect_for("t_never_registered", &here()).effect,
            BlockEffect::Review
        );
    }

    struct Fake;

    impl AgreementSource for Fake {
        fn open_for<'a>(
            &'a self,
            _state: &'a AppState,
            _party: PartyId,
        ) -> AgreementFuture<'a, Vec<AgreementRef>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn decide<'a>(
            &'a self,
            _state: &'a AppState,
            _user: i64,
            _subject: &'a Subject,
            _decision: Decision,
        ) -> AgreementFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn a_registered_source_is_found_by_kind() {
        AgreementKind::new("t_with_source").source(Fake).register();
        assert!(source_for("t_with_source").is_some());
        let info = kinds().into_iter().find(|k| k.key == "t_with_source");
        assert!(info.is_some_and(|k| k.has_source && k.effect == BlockEffect::Review));
    }

    #[test]
    fn announcing_types_are_a_registry() {
        assert!(!announces_pause("t.pause_notice"));
        mark_announces_pause("t.pause_notice");
        assert!(announces_pause("t.pause_notice"));
        assert!(!announces_pause("party.on_hold.neutral"));
    }

    #[test]
    fn names_are_stored_in_snake_case() {
        assert_eq!(BlockEffect::Review.to_string(), "review");
        assert_eq!(Decision::End.as_str(), "end");
        assert_eq!(RuleSource::Vertical.as_str(), "vertical");
        assert_eq!(
            renox::serde_json::to_string(&Decision::Pause).unwrap(),
            "\"pause\""
        );
        assert_eq!(Decision::ALL.len(), 3);
    }

    #[renox::test]
    async fn without_parties_nothing_is_suppressed() {
        use renox::testing::TestApp;
        let app = TestApp::new(renox::App::new()).await;
        mark_announces_pause("t.suppress_notice");
        let hit = suppressed_while_blocked(app.state(), "t.suppress_notice", PartyId(1)).await;
        assert!(!hit.unwrap());
        let other = suppressed_while_blocked(app.state(), "t.other", PartyId(1)).await;
        assert!(!other.unwrap());
    }
}
