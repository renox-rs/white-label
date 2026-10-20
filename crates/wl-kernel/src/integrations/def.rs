//! The registry of integration connections (wl-kernel §4.5, §5.14 step 1),
//! keyed by connection key like [`crate::voids`].
//!
//! A module registers each external system it calls once, in `register()`,
//! with the screens the connection affects, a manual fallback (EN/AR), the
//! role responsible for it and the handler that sends one queued call:
//!
//! ```ignore
//! IntegrationDef::of("asp.acme", "invoicing", |state, call| {
//!     Box::pin(async move { send_to_asp(state, call).await })
//! })
//! .owner_role("accountant")
//! .screens(["invoicing.invoice_list", "invoicing.invoice_detail"])
//! .fallback("File the invoice on the ASP portal by hand.", Some("قدّم الفاتورة يدويًا."))
//! .register();
//! ```
//!
//! `wl:install` stores a connection row for each registered definition the
//! database lacks; what an admin changes later (the fallback text, N) is kept.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use renox::chrono::Duration;
use renox::db::DateTime;
use renox::prelude::{Error, StatusCode};
use renox::serde_json::Value;
use renox::{AppState, Result};

use crate::late_entries::EntrySpec;
use crate::machines::BoxFuture;
use crate::subject::Subject;

/// N: minutes without a successful call after which a connection goes
/// `down` (`PLT.RUL.05`).
pub const DEFAULT_DOWN_AFTER_MINUTES: i64 = 15;

/// The wait before the next attempt after `failed` failed attempts of an
/// item (the first call counts as attempt 1): 1, 2, 4 and 8 minutes, then
/// 15 minutes for ever (wl-kernel §5.14 step 2). There is no attempt limit.
pub fn retry_wait(failed: i64) -> Duration {
    Duration::minutes(match failed {
        ..=1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        _ => 15,
    })
}

/// One outbound call, as the module's handlers see it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Outbound {
    /// The queued item's id; 0 for a call that was not queued yet.
    pub id: i64,
    /// The connection's key.
    pub connection: String,
    /// What the call is, in the module's words, e.g. `refund`.
    pub kind: String,
    /// What the module needs to send it.
    pub payload: Value,
    /// The module's natural key of the call (an external order number, a
    /// transaction reference), if it has one.
    pub natural_key: Option<String>,
    /// The record the call is about, if any.
    pub subject: Option<Subject>,
    /// How many attempts failed so far.
    pub attempts: i64,
}

impl Outbound {
    /// A call of `kind` on `connection` with `payload`, not about any
    /// record and without a natural key.
    pub fn new(connection: &str, kind: &str, payload: Value) -> Self {
        Outbound {
            id: 0,
            connection: connection.to_owned(),
            kind: kind.to_owned(),
            payload,
            natural_key: None,
            subject: None,
            attempts: 0,
        }
    }

    /// With the module's natural key.
    pub fn natural_key(mut self, key: &str) -> Self {
        self.natural_key = Some(key.to_owned());
        self
    }

    /// About `subject`.
    pub fn subject(mut self, subject: Subject) -> Self {
        self.subject = Some(subject);
        self
    }
}

/// Data that arrives from the external system after the fact (a gateway
/// callback, an aggregator order), with its own business time. It is
/// processed under the late-entry rules ([`crate::late_entries`]).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Inbound {
    /// The connection's key.
    pub connection: String,
    /// The module's natural key of the thing, e.g. the gateway's transaction
    /// reference: it prevents a second record.
    pub natural_key: String,
    /// The business time: when it happened at the other side.
    pub occurred_at: DateTime,
    /// What the external system says (amount, status…).
    pub payload: Value,
    /// The user the record is entered for (the late entry's author).
    pub user_id: i64,
}

impl Inbound {
    /// An inbound item of `connection` with `natural_key`, that happened at
    /// `occurred_at`, entered for `user_id`.
    pub fn new(
        connection: &str,
        natural_key: &str,
        occurred_at: DateTime,
        payload: Value,
        user_id: i64,
    ) -> Self {
        Inbound {
            connection: connection.to_owned(),
            natural_key: natural_key.to_owned(),
            occurred_at,
            payload,
            user_id,
        }
    }
}

/// What the module's matcher found for an inbound item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    /// No record has this natural key: the item is entered as a new record.
    NoRecord,
    /// A record done by hand has this natural key and says the same: the
    /// item is linked to it, no second record is created.
    Same(Subject),
    /// A record done by hand has this natural key but its values differ; the
    /// detail (for example `AED 250 manual vs AED 205 gateway`) goes on the
    /// reconciliation task.
    Differs {
        /// The record done by hand.
        subject: Subject,
        /// How the two differ.
        detail: String,
    },
}

/// The module's handler that finds the record done by hand for an inbound
/// item, by its natural key, and compares the two.
pub type MatchFn =
    Arc<dyn for<'a> Fn(&'a AppState, &'a Inbound) -> BoxFuture<'a, Result<Match>> + Send + Sync>;

/// The module's description of the record an inbound item creates when no
/// record done by hand matches (a late entry of an opted-in entity).
pub type EntryFn = Arc<dyn Fn(&Inbound) -> EntrySpec + Send + Sync>;

/// What a module's send handler found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The external system took the call.
    Sent,
    /// The record changed while the call was queued (voided, cancelled): it
    /// must not be sent. The item is closed and listed on the
    /// reconciliation task.
    Obsolete,
}

/// The module's handler that sends one call. `Ok` is a successful call; an
/// `Err` is a failed call (the message is kept as the last error).
pub type SendFn = Arc<
    dyn for<'a> Fn(&'a AppState, &'a Outbound) -> BoxFuture<'a, Result<Delivery>> + Send + Sync,
>;

/// The module's handler that says whether a queued call is still due; asked
/// before each queued item is sent.
pub type DueFn =
    Arc<dyn for<'a> Fn(&'a AppState, &'a Outbound) -> BoxFuture<'a, Result<bool>> + Send + Sync>;

/// An external system a module calls. Built with [`IntegrationDef::of`].
#[derive(Clone)]
#[must_use = "a connection is only tracked once registered"]
pub struct IntegrationDef {
    key: String,
    module: String,
    owner_role: String,
    screens: Vec<String>,
    fallback_en: String,
    fallback_ar: Option<String>,
    down_after_minutes: i64,
    send: SendFn,
    due: Option<DueFn>,
    matcher: Option<MatchFn>,
    entry: Option<EntryFn>,
}

impl fmt::Debug for IntegrationDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IntegrationDef")
            .field("key", &self.key)
            .field("module", &self.module)
            .field("owner_role", &self.owner_role)
            .field("screens", &self.screens)
            .field("down_after_minutes", &self.down_after_minutes)
            .field("inbound", &self.matcher.is_some())
            .finish()
    }
}

impl IntegrationDef {
    /// Starts the connection `key` (e.g. `asp.acme`) of `module`, with the
    /// handler that sends one call. The owner role is the company admin
    /// until [`IntegrationDef::owner_role`] says otherwise.
    pub fn of<F>(key: &str, module: &str, send: F) -> Self
    where
        F: for<'a> Fn(&'a AppState, &'a Outbound) -> BoxFuture<'a, Result<Delivery>>
            + Send
            + Sync
            + 'static,
    {
        IntegrationDef {
            key: key.to_owned(),
            module: module.to_owned(),
            owner_role: crate::scopes::COMPANY_ADMIN_ROLE.to_owned(),
            screens: Vec::new(),
            fallback_en: String::new(),
            fallback_ar: None,
            down_after_minutes: DEFAULT_DOWN_AFTER_MINUTES,
            send: Arc::new(send),
            due: None,
            matcher: None,
            entry: None,
        }
    }

    /// The role responsible for the connection (e.g. `accountant` for the
    /// payment gateway), alerted with the company admin when it goes down.
    pub fn owner_role(mut self, role: &str) -> Self {
        self.owner_role = role.to_owned();
        self
    }

    /// The screens the connection affects; each shows the banner while it is
    /// `down`.
    pub fn screens<S: Into<String>>(mut self, screens: impl IntoIterator<Item = S>) -> Self {
        self.screens = screens.into_iter().map(Into::into).collect();
        self
    }

    /// The manual fallback, in English and, if there is one, Arabic.
    pub fn fallback(mut self, en: &str, ar: Option<&str>) -> Self {
        self.fallback_en = en.to_owned();
        self.fallback_ar = ar.map(str::to_owned);
        self
    }

    /// N: minutes without a success before the connection goes `down`
    /// (default 15).
    pub fn down_after_minutes(mut self, minutes: i64) -> Self {
        self.down_after_minutes = minutes;
        self
    }

    /// How the module says a queued call is still due. Without it every
    /// queued call is due.
    pub fn due<F>(mut self, due: F) -> Self
    where
        F: for<'a> Fn(&'a AppState, &'a Outbound) -> BoxFuture<'a, Result<bool>>
            + Send
            + Sync
            + 'static,
    {
        self.due = Some(Arc::new(due));
        self
    }

    /// How inbound data that arrives late is handled: `matcher` finds the
    /// record done by hand with the same natural key and compares the two;
    /// when there is none, `entry` describes the record to enter, which goes
    /// through the late-entry rules with the item's own business time.
    pub fn inbound<M, E>(mut self, matcher: M, entry: E) -> Self
    where
        M: for<'a> Fn(&'a AppState, &'a Inbound) -> BoxFuture<'a, Result<Match>>
            + Send
            + Sync
            + 'static,
        E: Fn(&Inbound) -> EntrySpec + Send + Sync + 'static,
    {
        self.matcher = Some(Arc::new(matcher));
        self.entry = Some(Arc::new(entry));
        self
    }

    /// The connection's key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The module that owns the connection.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// The role responsible for the connection.
    pub fn owner_role_name(&self) -> &str {
        &self.owner_role
    }

    /// The screens the connection affects.
    pub fn screen_keys(&self) -> &[String] {
        &self.screens
    }

    /// The English fallback text.
    pub fn fallback_en(&self) -> &str {
        &self.fallback_en
    }

    /// The Arabic fallback text, if any.
    pub fn fallback_ar(&self) -> Option<&str> {
        self.fallback_ar.as_deref()
    }

    /// N for this connection when first stored.
    pub fn down_after(&self) -> i64 {
        self.down_after_minutes
    }

    /// Sends `call` through the module's handler.
    pub async fn send(&self, state: &AppState, call: &Outbound) -> Result<Delivery> {
        (self.send)(state, call).await
    }

    /// Whether the module says `call` is still due (`true` without a
    /// handler).
    pub async fn is_due(&self, state: &AppState, call: &Outbound) -> Result<bool> {
        match &self.due {
            Some(due) => due(state, call).await,
            None => Ok(true),
        }
    }

    /// Whether the connection takes inbound data ([`IntegrationDef::inbound`]).
    pub fn takes_inbound(&self) -> bool {
        self.matcher.is_some() && self.entry.is_some()
    }

    /// Asks the module's matcher about `item`; [`Match::NoRecord`] without
    /// one.
    pub async fn match_manual(&self, state: &AppState, item: &Inbound) -> Result<Match> {
        match &self.matcher {
            Some(matcher) => matcher(state, item).await,
            None => Ok(Match::NoRecord),
        }
    }

    /// The record `item` creates when nothing matches it.
    pub fn entry_for(&self, item: &Inbound) -> Option<EntrySpec> {
        self.entry.as_ref().map(|entry| entry(item))
    }

    /// Adds this definition to the process-wide registry, replacing one of
    /// the same key.
    ///
    /// # Panics
    ///
    /// When the key is empty, or N is not above 0: programming errors.
    pub fn register(self) {
        register(self);
    }
}

static REGISTRY: RwLock<BTreeMap<String, Arc<IntegrationDef>>> = RwLock::new(BTreeMap::new());

/// Adds `def` to the process-wide registry; registering the same key again
/// replaces the entry.
///
/// # Panics
///
/// When the key is empty, or N is not above 0.
pub fn register(def: IntegrationDef) {
    assert!(!def.key.trim().is_empty(), "an integration needs a key");
    assert!(
        def.down_after_minutes > 0,
        "{}: down after N minutes needs N above 0",
        def.key
    );
    REGISTRY
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(def.key.clone(), Arc::new(def));
}

/// The definition of `key`, if registered.
pub fn get(key: &str) -> Option<Arc<IntegrationDef>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .cloned()
}

/// The registered definitions, by key.
pub fn all() -> Vec<Arc<IntegrationDef>> {
    REGISTRY
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
}

/// A failed call, for tests and modules that build errors: a 502 with
/// `message`.
pub fn call_failed(message: &str) -> Error {
    Error::Status(StatusCode::BAD_GATEWAY, message.to_owned())
}

/// When the connection with `failed_since` goes `down` if nothing succeeds
/// before: N minutes after the first failure.
pub fn down_at(failed_since: DateTime, down_after_minutes: i64) -> DateTime {
    failed_since + Duration::minutes(down_after_minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(key: &str) -> IntegrationDef {
        IntegrationDef::of(key, "invoicing", |_state, _call| {
            Box::pin(async { Ok(Delivery::Sent) })
        })
    }

    #[test]
    fn the_wait_is_1_2_4_8_then_15_minutes_for_ever() {
        let minutes: Vec<i64> = (1..=8).map(|n| retry_wait(n).num_minutes()).collect();
        assert_eq!(minutes, [1, 2, 4, 8, 15, 15, 15, 15]);
        assert_eq!(retry_wait(0).num_minutes(), 1);
    }

    #[test]
    fn a_connection_goes_down_n_minutes_after_the_first_failure() {
        let first: DateTime = "2026-11-03T06:00:00Z".parse().unwrap();
        assert_eq!(
            down_at(first, 15),
            "2026-11-03T06:15:00Z".parse::<DateTime>().unwrap()
        );
        assert_eq!(
            down_at(first, 5),
            "2026-11-03T06:05:00Z".parse::<DateTime>().unwrap()
        );
    }

    #[test]
    fn the_registry_keeps_what_the_module_declared() {
        def("tests.asp")
            .owner_role("accountant")
            .screens(["invoicing.invoice_list", "invoicing.invoice_detail"])
            .fallback("File by hand.", Some("قدّم يدويًا."))
            .down_after_minutes(5)
            .register();
        let found = get("tests.asp").expect("registered");
        assert_eq!(found.key(), "tests.asp");
        assert_eq!(found.module(), "invoicing");
        assert_eq!(found.owner_role_name(), "accountant");
        assert_eq!(found.screen_keys().len(), 2);
        assert_eq!(found.fallback_en(), "File by hand.");
        assert_eq!(found.fallback_ar(), Some("قدّم يدويًا."));
        assert_eq!(found.down_after(), 5);
        assert!(all().iter().any(|d| d.key() == "tests.asp"));
        assert!(get("tests.unknown").is_none());
    }

    #[test]
    fn the_defaults_are_the_company_admin_and_15_minutes() {
        let plain = def("tests.plain");
        assert_eq!(plain.owner_role_name(), "admin");
        assert_eq!(plain.down_after(), 15);
        assert_eq!(plain.fallback_ar(), None);
        assert!(plain.screen_keys().is_empty());
    }

    #[test]
    #[should_panic(expected = "needs N above 0")]
    fn a_limit_of_zero_minutes_panics() {
        def("tests.zero").down_after_minutes(0).register();
    }

    #[test]
    fn a_connection_takes_inbound_data_only_with_both_handlers() {
        let plain = def("tests.inbound_plain");
        assert!(!plain.takes_inbound());
        let at: DateTime = "2026-11-03T06:20:00Z".parse().unwrap();
        let with = def("tests.inbound_with").inbound(
            |_state, _item| Box::pin(async { Ok(Match::NoRecord) }),
            move |item| EntrySpec::new("payments.payment", item.user_id, item.occurred_at),
        );
        assert!(with.takes_inbound());
        let item = Inbound::new("tests.inbound_with", "TX-889", at, Value::Null, 7);
        let spec = with.entry_for(&item).expect("an entry");
        assert_eq!(spec.subject_type, "payments.payment");
        assert_eq!(spec.occurred_at, at);
        assert_eq!(spec.user_id, 7);
        assert!(plain.entry_for(&item).is_none());
    }

    #[test]
    fn a_call_names_its_natural_key_and_subject() {
        let call = Outbound::new(
            "gateway.acme",
            "refund",
            renox::serde_json::json!({ "n": 1 }),
        )
        .natural_key("TX-889")
        .subject(Subject::new("payments.refund", 3));
        assert_eq!(call.natural_key.as_deref(), Some("TX-889"));
        assert_eq!(call.subject, Some(Subject::new("payments.refund", 3)));
        assert_eq!(call.attempts, 0);
    }
}
