//! The document guard contract ([ARCHITECTURE.md] §5, wl-documents §5.6,
//! §6): how a module that owns a guarded transition asks whether the
//! documents it needs are valid, without depending on the `wl-documents`
//! crate.
//!
//! A document kind lists the transitions it guards as [`BlockEntry`]s
//! (`<entity>.<transition>`, a [`BlockMode`], an optional
//! [`BlockCondition`]). The guarded module registers a [`BlockSubjects`]
//! provider for each such transition with [`register_block_subjects`]; it
//! maps a record to the document subjects (a run → its driver and vehicle).
//! Before `record.transition(..)` it calls
//! [`DocumentGuard::check`] through [`guard`] and answers a
//! [`BlockRefusal`] with [`BlockRefusal::into_error`] (422). Renox guards get
//! no `AppState`, so the check cannot run inside the machine's guard.
//!
//! `wl-documents` provides the guard with [`provide_guard`] and emits
//! [`DocumentUploaded`], [`DocumentAccepted`], [`DocumentRejected`] and
//! [`DocumentSuperseded`]. When the `documents` module is off, [`guard`]
//! gives [`NoDocuments`]: no blocks, always valid.
//!
//! Dates are calendar dates in Asia/Dubai ([`today`]); a document is valid
//! through its expiry date.
//!
//! [ARCHITECTURE.md]: https://github.com/renox-rs/white-label/blob/main/docs/ARCHITECTURE.md

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::chrono::{FixedOffset, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// Asia/Dubai's offset from UTC, in seconds: UTC+4 all year (no DST).
pub const DUBAI_OFFSET_SECONDS: i32 = 4 * 60 * 60;

/// The Asia/Dubai calendar date of the instant `at`.
pub fn dubai_date(at: DateTime) -> NaiveDate {
    let dubai = FixedOffset::east_opt(DUBAI_OFFSET_SECONDS).expect("UTC+4 is a valid offset");
    at.with_timezone(&dubai).date_naive()
}

/// Today in Asia/Dubai, from [`renox::db::now`] (so a test that travels
/// moves it too): the default `on_date` of the guard.
pub fn today() -> NaiveDate {
    dubai_date(renox::db::now())
}

/// The id of a document.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct DocumentId(pub i64);

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for DocumentId {
    fn from(id: i64) -> Self {
        DocumentId(id)
    }
}

/// What a failing document does to the transition it guards.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlockMode {
    /// The transition is refused.
    #[default]
    Block,
    /// The transition runs and the record is flagged.
    Flag,
}

/// Whether a refused transition may be overridden.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OverrideMode {
    /// Never (every legal block).
    #[default]
    Never,
    /// With a second user's approval (`DOC.ACL.02`).
    WithApproval,
}

/// The condition of a [`BlockEntry`] on the guarded record: one field the
/// guarded module exposes for conditions and the values it must have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockCondition {
    /// The field, e.g. `category`.
    pub field: String,
    /// The values that make the entry apply, e.g. the fire and life safety
    /// categories.
    pub values: Vec<String>,
}

impl BlockCondition {
    /// A condition on `field` with these values.
    pub fn new<I, S>(field: &str, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        BlockCondition {
            field: field.to_owned(),
            values: values.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether a record whose field has `value` matches.
    pub fn matches(&self, value: &str) -> bool {
        self.values.iter().any(|v| v == value)
    }
}

/// One transition a document kind guards (an entry of its `blocks`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEntry {
    /// The document kind's key, e.g. `residence_visa`.
    pub kind: String,
    /// The guarded transition, `<entity>.<transition>`, e.g.
    /// `deployment.activate`.
    pub guarded: String,
    /// Block or flag.
    pub mode: BlockMode,
    /// The kind's override mode.
    pub override_mode: OverrideMode,
    /// The condition on the guarded record; `None` applies to every record.
    pub condition: Option<BlockCondition>,
    /// A legal-block entry: a client may not remove it or switch it to
    /// `flag`.
    pub locked: bool,
}

impl BlockEntry {
    /// The entity part of [`guarded`](Self::guarded): `deployment`.
    pub fn entity(&self) -> &str {
        split_guarded(&self.guarded).0
    }

    /// The transition part of [`guarded`](Self::guarded): `activate`.
    pub fn transition(&self) -> &str {
        split_guarded(&self.guarded).1
    }

    /// Whether the entry applies to a record whose condition field reads
    /// `value`: always without a condition, else when the value matches.
    pub fn applies_to(&self, value: Option<&str>) -> bool {
        match &self.condition {
            None => true,
            Some(condition) => value.is_some_and(|v| condition.matches(v)),
        }
    }
}

/// `<entity>.<transition>` split at its first dot.
fn split_guarded(guarded: &str) -> (&str, &str) {
    guarded.split_once('.').unwrap_or((guarded, ""))
}

/// Why a document is not valid on a date.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InvalidReason {
    /// Its expiry date is before the date.
    #[default]
    Expired,
    /// There is no document of the kind.
    Missing,
    /// The only document was rejected.
    Rejected,
    /// The only document still waits for review.
    PendingReview,
}

/// A renewal uploaded and still `pending_review`: named in the refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRenewal {
    /// The pending document.
    pub document: DocumentId,
    /// When it was submitted (Asia/Dubai).
    pub submitted_on: NaiveDate,
}

/// The answer of [`DocumentGuard::is_valid`] (the guard `document_valid`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Validity {
    /// Valid on the date.
    Valid {
        /// The accepted document that makes it valid; `None` from
        /// [`NoDocuments`].
        document: Option<DocumentId>,
        /// Its expiry date; `None` when it does not expire.
        expires_on: Option<NaiveDate>,
    },
    /// Not valid on the date.
    Invalid {
        /// Why.
        reason: InvalidReason,
        /// The expired, rejected or pending document; `None` when missing.
        document: Option<DocumentId>,
        /// The expiry date of an expired document.
        expired_on: Option<NaiveDate>,
        /// A renewal waiting for review.
        pending: Option<PendingRenewal>,
    },
}

impl Validity {
    /// Whether the guard answers *yes*.
    pub fn is_valid(&self) -> bool {
        matches!(self, Validity::Valid { .. })
    }
}

/// One failing document of a refused transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockFailure {
    /// The entry that guards the transition.
    pub entry: BlockEntry,
    /// The document subject that fails, e.g. the driver.
    pub subject: Subject,
    /// Why it fails.
    pub validity: Validity,
    /// The line shown to the user, e.g. "Residence visa expired on
    /// 2026-11-30; renew: PRO; override: not allowed".
    pub message: String,
}

/// A guarded transition refused because documents are expired or missing:
/// one [`BlockFailure`] per failing subject and kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRefusal {
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// The record whose transition was refused.
    pub record_id: i64,
    /// Every failing document.
    pub failures: Vec<BlockFailure>,
}

impl BlockRefusal {
    /// The message: one line per failing document.
    pub fn message(&self) -> String {
        self.failures
            .iter()
            .map(|f| f.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The refusal as a 422 with [`message`](Self::message), like a Renox
    /// `TransitionError::Guard`.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

impl fmt::Display for BlockRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for BlockRefusal {}

/// What an allowed check leaves to do after the transition. Flag-mode
/// entries (#46) add the flags to write here; build it with
/// `CheckOutcome::default()`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CheckOutcome {}

/// The future a [`DocumentGuard`] or [`BlockSubjects`] method returns.
pub type DocumentFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The standard document guard. `wl-documents` provides it; without that
/// module, [`guard`] gives [`NoDocuments`].
pub trait DocumentGuard: Send + Sync + 'static {
    /// The entries that guard `guarded` (`<entity>.<transition>`), with
    /// their mode, override mode and condition.
    fn blocks_for<'a>(
        &'a self,
        db: &'a Db,
        guarded: &'a str,
    ) -> DocumentFuture<'a, Vec<BlockEntry>>;

    /// The guard `document_valid(subject, kind, on_date)`: whether `subject`
    /// has an accepted document of `kind` valid on `on_date` ([`today`] for
    /// the present; past and future dates allowed). Nothing is cached.
    fn is_valid<'a>(
        &'a self,
        db: &'a Db,
        subject: &'a Subject,
        kind: &'a str,
        on_date: NaiveDate,
    ) -> DocumentFuture<'a, Validity>;

    /// Checks record `record_id` before its transition `guarded`: runs
    /// [`blocks_for`](Self::blocks_for), the transition's [`BlockSubjects`]
    /// provider and [`is_valid`](Self::is_valid) for every subject and
    /// kind. The inner `Err` refuses the transition; the outer one is a
    /// failure (e.g. an entry whose transition has no provider), never a
    /// *yes*.
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        record_id: i64,
        on_date: NaiveDate,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>>;
}

/// The guard when the `documents` module is off: nothing is guarded and
/// every document is valid.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDocuments;

impl DocumentGuard for NoDocuments {
    fn blocks_for<'a>(
        &'a self,
        _db: &'a Db,
        _guarded: &'a str,
    ) -> DocumentFuture<'a, Vec<BlockEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn is_valid<'a>(
        &'a self,
        _db: &'a Db,
        _subject: &'a Subject,
        _kind: &'a str,
        _on_date: NaiveDate,
    ) -> DocumentFuture<'a, Validity> {
        Box::pin(async {
            Ok(Validity::Valid {
                document: None,
                expires_on: None,
            })
        })
    }

    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _guarded: &'a str,
        _record_id: i64,
        _on_date: NaiveDate,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>> {
        Box::pin(async { Ok(Ok(CheckOutcome::default())) })
    }
}

/// Makes `guard` the app's [`DocumentGuard`]; `wl-documents` calls it in
/// `register`.
pub fn provide_guard(app: &mut Registry, guard: impl DocumentGuard) {
    let guard: Arc<dyn DocumentGuard> = Arc::new(guard);
    app.provide(guard);
}

/// The app's [`DocumentGuard`], or [`NoDocuments`] when none is provided.
pub fn guard(state: &AppState) -> Arc<dyn DocumentGuard> {
    match state.provided::<Arc<dyn DocumentGuard>>() {
        Some(guard) => Arc::clone(&guard),
        None => Arc::new(NoDocuments),
    }
}

/// Maps a guarded record to its document subjects. The guarded module
/// registers one per `<entity>.<transition>` with
/// [`register_block_subjects`].
pub trait BlockSubjects: Send + Sync + 'static {
    /// The subjects of record `record_id` whose documents of `entry.kind`
    /// must be valid (a run → its driver for a licence, its vehicle for
    /// insurance). Returns none when the record does not match
    /// `entry.condition`, so the guard answers *yes* for it.
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>>;
}

static PROVIDERS: RwLock<BTreeMap<String, Arc<dyn BlockSubjects>>> = RwLock::new(BTreeMap::new());

/// Adds `provider` for `guarded` (`<entity>.<transition>`) to the
/// process-wide registry; registering the same transition again replaces
/// it.
pub fn register_block_subjects(guarded: &str, provider: impl BlockSubjects) {
    PROVIDERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(guarded.to_owned(), Arc::new(provider));
}

/// The provider of `guarded`, if registered.
pub fn block_subjects(guarded: &str) -> Option<Arc<dyn BlockSubjects>> {
    PROVIDERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(guarded)
        .cloned()
}

/// The transitions with a registered provider, sorted.
pub fn guarded_transitions() -> Vec<String> {
    PROVIDERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect()
}

/// A file was submitted for review (`submit`, `resubmit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentUploaded {
    /// The document.
    pub document: DocumentId,
    /// What it is attached to.
    pub subject: Subject,
    /// The kind's key.
    pub kind: String,
    /// The user who uploaded it; `None` for the system.
    pub uploaded_by: Option<i64>,
    /// Where it came from, e.g. `back_office`, `portal`.
    pub source: String,
}

impl Event for DocumentUploaded {}

/// A document became accepted (`accept`, `record`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentAccepted {
    /// The document.
    pub document: DocumentId,
    /// What it is attached to.
    pub subject: Subject,
    /// The kind's key.
    pub kind: String,
    /// The expiry date; `None` when it does not expire.
    pub expires_on: Option<NaiveDate>,
}

impl Event for DocumentAccepted {}

/// A review rejected a document (`reject`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRejected {
    /// The document.
    pub document: DocumentId,
    /// What it is attached to.
    pub subject: Subject,
    /// The kind's key.
    pub kind: String,
    /// The reviewer's reason.
    pub reason: String,
}

impl Event for DocumentRejected {}

/// A newer document of the same kind and subject replaced this one
/// (`supersede`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSuperseded {
    /// The replaced document.
    pub document: DocumentId,
    /// The document that replaced it.
    pub replaced_by: DocumentId,
}

impl Event for DocumentSuperseded {}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use renox::testing::TestApp;

    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn instant(text: &str) -> DateTime {
        text.parse().expect("an RFC 3339 instant")
    }

    fn visa_entry() -> BlockEntry {
        BlockEntry {
            kind: "residence_visa".into(),
            guarded: "deployment.activate".into(),
            mode: BlockMode::Block,
            override_mode: OverrideMode::Never,
            condition: None,
            locked: true,
        }
    }

    #[test]
    fn a_dubai_date_turns_at_20_00_utc() {
        assert_eq!(
            dubai_date(instant("2026-11-30T19:59:59Z")),
            date("2026-11-30")
        );
        assert_eq!(
            dubai_date(instant("2026-11-30T20:00:00Z")),
            date("2026-12-01")
        );
        // No daylight saving: the same in summer.
        assert_eq!(
            dubai_date(instant("2026-06-30T20:00:00Z")),
            date("2026-07-01")
        );
    }

    #[renox::test]
    async fn today_follows_the_travelled_clock() {
        let app = TestApp::new(App::new()).await;
        let now_today = today();
        app.travel(Duration::from_secs(2 * 24 * 60 * 60));
        let (later, later_today) = app
            .at_travelled_time(async { (renox::db::now(), today()) })
            .await;
        assert_eq!(later_today, dubai_date(later));
        assert!(later_today >= now_today + renox::chrono::Days::new(2));
    }

    #[renox::test]
    async fn without_documents_the_guard_is_the_noop() {
        let app = TestApp::new(App::new()).await;
        let guard = guard(app.state());
        let on = date("2026-12-01");
        assert!(
            guard
                .blocks_for(app.db(), "deployment.activate")
                .await
                .unwrap()
                .is_empty()
        );
        let ravi = Subject::new("hr.employee", 1);
        let validity = guard
            .is_valid(app.db(), &ravi, "residence_visa", on)
            .await
            .unwrap();
        assert!(validity.is_valid());
        let outcome = guard
            .check(app.state(), "deployment.activate", 7, on)
            .await
            .unwrap();
        assert_eq!(outcome, Ok(CheckOutcome::default()));
    }

    #[test]
    fn a_refusal_is_a_422_with_one_line_per_document() {
        let failure = |kind: &str, message: &str| BlockFailure {
            entry: BlockEntry {
                kind: kind.into(),
                guarded: "run.dispatch".into(),
                ..visa_entry()
            },
            subject: Subject::new("fleet.vehicle", 3),
            validity: Validity::Invalid {
                reason: InvalidReason::Expired,
                document: Some(DocumentId(9)),
                expired_on: Some(date("2026-11-30")),
                pending: None,
            },
            message: message.into(),
        };
        let refusal = BlockRefusal {
            guarded: "run.dispatch".into(),
            record_id: 4,
            failures: vec![
                failure(
                    "vehicle_insurance",
                    "Vehicle insurance expired on 2026-11-30",
                ),
                failure("driver_licence", "Driver licence missing"),
            ],
        };
        assert_eq!(
            refusal.to_string(),
            "Vehicle insurance expired on 2026-11-30\nDriver licence missing"
        );
        match refusal.into_error() {
            Error::Status(code, message) => {
                assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
                assert!(
                    message.starts_with("Vehicle insurance expired"),
                    "{message}"
                );
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn an_entry_splits_its_transition_and_applies_its_condition() {
        let entry = visa_entry();
        assert_eq!(
            (entry.entity(), entry.transition()),
            ("deployment", "activate")
        );
        assert!(entry.applies_to(None));
        let fire = BlockEntry {
            guarded: "service_contract.activate".into(),
            condition: Some(BlockCondition::new("category", ["fire_alarm", "sprinkler"])),
            ..visa_entry()
        };
        assert_eq!(fire.entity(), "service_contract");
        assert!(fire.applies_to(Some("fire_alarm")));
        assert!(!fire.applies_to(Some("plumbing")));
        assert!(!fire.applies_to(None));
    }

    #[test]
    fn modes_and_reasons_are_stored_in_snake_case() {
        assert_eq!(BlockMode::Flag.as_str(), "flag");
        assert_eq!(BlockMode::default(), BlockMode::Block);
        assert_eq!(OverrideMode::WithApproval.as_str(), "with_approval");
        assert_eq!(
            "never".parse::<OverrideMode>().unwrap(),
            OverrideMode::Never
        );
        assert_eq!(InvalidReason::PendingReview.as_str(), "pending_review");
    }

    struct Nobody;

    impl BlockSubjects for Nobody {
        fn subjects<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a BlockEntry,
            _record_id: i64,
        ) -> DocumentFuture<'a, Vec<Subject>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    #[test]
    fn providers_are_registered_by_transition() {
        register_block_subjects("test_unit.guarded", Nobody);
        assert!(block_subjects("test_unit.guarded").is_some());
        assert!(block_subjects("test_unit.unknown").is_none());
        assert!(guarded_transitions().contains(&"test_unit.guarded".to_owned()));
    }

    #[test]
    fn events_serialize_with_plain_ids() {
        let accepted = DocumentAccepted {
            document: DocumentId(5),
            subject: Subject::new("hr.employee", 1),
            kind: "residence_visa".into(),
            expires_on: Some(date("2027-11-30")),
        };
        let json = renox::serde_json::to_value(&accepted).unwrap();
        assert_eq!(json["document"], 5);
        assert_eq!(json["expires_on"], "2027-11-30");
        let superseded = DocumentSuperseded {
            document: DocumentId(5),
            replaced_by: DocumentId(8),
        };
        let json = renox::serde_json::to_string(&superseded).unwrap();
        assert_eq!(json, r#"{"document":5,"replaced_by":8}"#);
    }
}
