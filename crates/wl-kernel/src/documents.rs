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
//! Look-ahead: before a blocking document expires, `wl-documents` asks the
//! [`FutureWork`] provider of each guarded transition (registered with
//! [`register_future_work`]) for the planned records that fall after the
//! document stops being valid. The `from` date it passes is **expiry + 1
//! day**: a document is valid through its expiry date, so a record planned
//! on that date is fine and is never listed. The result is emitted as
//! [`DocumentExpiring`], grouped by guarded transition ([`AffectedGroup`]).
//! The staff to tell come from the subject's scope, else the scopes of the
//! affected records, else the kind's [`ReminderSchedule`] (registered with
//! [`register_reminder_schedule`]), else every scope.
//!
//! [ARCHITECTURE.md]: https://github.com/renox-rs/white-label/blob/main/docs/ARCHITECTURE.md

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::auth::permissions::Scope;
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

/// `at` as `2026-11-03 09:15` in Asia/Dubai: how times are shown in alerts,
/// digests and cards.
pub fn dubai_text(at: DateTime) -> String {
    let dubai = FixedOffset::east_opt(DUBAI_OFFSET_SECONDS).expect("UTC+4 is a valid offset");
    at.with_timezone(&dubai).format("%Y-%m-%d %H:%M").to_string()
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

/// The id of a block override (a one-use approval to pass a document
/// block), as stored by `wl-documents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OverrideId(pub i64);

impl fmt::Display for OverrideId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for OverrideId {
    fn from(id: i64) -> Self {
        OverrideId(id)
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

impl BlockFailure {
    /// Whether the failure may be overridden with a second user's approval:
    /// the kind's override mode is `with_approval` and it is not a legal
    /// block.
    pub fn override_allowed(&self) -> bool {
        self.entry.override_mode == OverrideMode::WithApproval && !self.entry.locked
    }
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

    /// The links that request an override, one per kind whose failure
    /// [may be overridden](BlockFailure::override_allowed):
    /// `/documents/overrides/new?kind=..&guarded=..&record=..`. A `never`
    /// kind and a legal block give none.
    pub fn override_links(&self) -> Vec<String> {
        let mut links = Vec::new();
        for failure in self.failures.iter().filter(|f| f.override_allowed()) {
            let link = format!(
                "/documents/overrides/new?kind={}&guarded={}&record={}",
                query_encode(&failure.entry.kind),
                query_encode(&self.guarded),
                self.record_id
            );
            if !links.contains(&link) {
                links.push(link);
            }
        }
        links
    }

    /// The refusal as a 422 with [`message`](Self::message), like a Renox
    /// `TransitionError::Guard`.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

/// Percent-encodes everything but the RFC 3986 unreserved characters.
fn query_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl fmt::Display for BlockRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for BlockRefusal {}

/// Why a flagged document was not valid: the reasons of
/// [`InvalidReason`], stored on a [`RecordFlag`].
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlagReason {
    /// Its expiry date was before the date.
    #[default]
    Expired,
    /// There was no document of the kind.
    Missing,
    /// The only document was rejected.
    Rejected,
    /// The only document still waited for review.
    PendingReview,
}

impl From<InvalidReason> for FlagReason {
    fn from(reason: InvalidReason) -> Self {
        match reason {
            InvalidReason::Expired => FlagReason::Expired,
            InvalidReason::Missing => FlagReason::Missing,
            InvalidReason::Rejected => FlagReason::Rejected,
            InvalidReason::PendingReview => FlagReason::PendingReview,
        }
    }
}

/// One failing document of an entry in flag mode: the transition still
/// runs and the record gets a flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagFailure {
    /// The `flag` mode entry that guards the transition.
    pub entry: BlockEntry,
    /// The document subject that fails, e.g. the worker.
    pub subject: Subject,
    /// Why it fails.
    pub reason: FlagReason,
    /// The expired, rejected or pending document; `None` when missing.
    pub document: Option<DocumentId>,
    /// The expiry date of an expired document.
    pub expired_on: Option<NaiveDate>,
    /// The business date the document was judged on.
    pub on_date: NaiveDate,
    /// The flag's label, e.g. "Work permit expired 2026-11-30".
    pub label: String,
}

/// What an allowed check leaves to do after the transition: the flags to
/// write with [`DocumentGuard::record_flags`] once it ran (see
/// [`guarded_transition`]). Build it with `CheckOutcome::default()`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CheckOutcome {
    /// One per failing flag-mode document; empty when nothing fails.
    pub flags: Vec<FlagFailure>,
    /// The failing `block` entries that approved overrides cover (see
    /// [`DocumentGuard::check_as`]); empty when nothing is overridden.
    pub overrides: Vec<CoveredBlock>,
}

/// A failing `block` entry that an approved override covers: the guard let
/// the transition through for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveredBlock {
    /// The override that covers the failure.
    pub override_id: OverrideId,
    /// The entry that failed.
    pub entry: BlockEntry,
    /// The document subject that failed.
    pub subject: Subject,
}

/// One planned item to check with [`DocumentGuard::check_planned`]: a saved
/// record and the date its guarded transition will run.
///
/// An item that spans several days passes only its start date as
/// `on_date` (a deployment its first day, a visit its day); the guard does
/// not know about end dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlannedItem {
    /// The saved record.
    pub record_id: i64,
    /// Its start date (Asia/Dubai calendar date).
    pub on_date: NaiveDate,
}

impl PlannedItem {
    /// The planned item `record_id` starting on `on_date`.
    pub fn new(record_id: i64, on_date: NaiveDate) -> Self {
        PlannedItem { record_id, on_date }
    }
}

/// The label of a subject without a [`BlockSubjects::label`]:
/// `<subject_type> #<id>`.
pub fn default_subject_label(subject: &Subject) -> String {
    format!("{} #{}", subject.subject_type, subject.subject_id)
}

/// One subject and document kind that will not be valid on some planned
/// dates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedWarningLine {
    /// The document subject, e.g. the worker.
    pub subject: Subject,
    /// Its label, e.g. "Ravi S."
    pub label: String,
    /// The document kind's key, e.g. `residence_visa`.
    pub kind: String,
    /// The kind's name, e.g. "Residence visa".
    pub kind_name: String,
    /// Why it will not be valid.
    pub reason: InvalidReason,
    /// The expiry date of the expired document; `None` when missing,
    /// rejected or pending.
    pub expires_on: Option<NaiveDate>,
    /// The line text, e.g. "Residence visa expires 2026-12-15".
    pub text: String,
    /// The planned dates it fails on, sorted, each once.
    pub dates: Vec<NaiveDate>,
}

impl PlannedWarningLine {
    /// `<subject label>: <text>; planned: <date>, <date>, ...`
    pub fn message(&self) -> String {
        let dates = self
            .dates
            .iter()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}: {}; planned: {}", self.label, self.text, dates)
    }
}

/// The documents that will not be valid on the dates of planned work: one
/// [`PlannedWarningLine`] per subject and kind, ordered by subject, then
/// kind. It never refuses anything; the module shows it after saving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedWarning {
    /// The guarded transition the items will pass, `<entity>.<transition>`.
    pub guarded: String,
    /// The lines, never empty.
    pub lines: Vec<PlannedWarningLine>,
}

impl PlannedWarning {
    /// The message: one line per subject and kind.
    pub fn message(&self) -> String {
        self.lines
            .iter()
            .map(PlannedWarningLine::message)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A persistent warning toast: the first line is the text, the other
    /// lines its body. The module returns it with its redirect after
    /// saving.
    pub fn toast(&self) -> Toast {
        let mut lines = self.lines.iter().map(PlannedWarningLine::message);
        let first = lines.next().unwrap_or_default();
        let rest = lines.collect::<Vec<_>>().join("\n");
        let toast = Toast::warning(first).persistent();
        if rest.is_empty() {
            toast
        } else {
            toast.body(rest)
        }
    }
}

impl fmt::Display for PlannedWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

/// Why [`DocumentGuard::check_planned`] gives no answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlannedError {
    /// The transition is a bookable slot: it is refused through
    /// `is_valid` (`SCH.ACL.04`), never turned into a warning.
    BookableSlot {
        /// The guarded transition, `<entity>.<transition>`.
        guarded: String,
    },
}

impl fmt::Display for PlannedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlannedError::BookableSlot { guarded } => write!(
                f,
                "{guarded} is a bookable slot: it is refused, not warned about"
            ),
        }
    }
}

impl std::error::Error for PlannedError {}

/// A flag on a record: permanent, it is reviewed but never removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordFlag {
    /// The flag's id.
    pub id: i64,
    /// The flagged record's type: the entity part of `guarded`.
    pub record_type: String,
    /// The flagged record's id.
    pub record_id: i64,
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// The document kind's key.
    pub kind: String,
    /// The document subject that failed.
    pub subject: Subject,
    /// The expired, rejected or pending document; `None` when missing.
    pub document: Option<DocumentId>,
    /// Why the document was not valid.
    pub reason: FlagReason,
    /// The expiry date of an expired document.
    pub expires_on: Option<NaiveDate>,
    /// The business date it was judged on.
    pub on_date: NaiveDate,
    /// The label, e.g. "Work permit expired 2026-11-30".
    pub label: String,
    /// When the flag was written.
    pub created_at: DateTime,
    /// When it was reviewed; `None` while open.
    pub reviewed_at: Option<DateTime>,
    /// The reviewer's user id.
    pub reviewed_by: Option<i64>,
    /// The reviewer's name.
    pub reviewer_name: Option<String>,
    /// The reviewer's note.
    pub review_note: Option<String>,
}

impl RecordFlag {
    /// Whether a reviewer has marked it reviewed.
    pub fn is_reviewed(&self) -> bool {
        self.reviewed_at.is_some()
    }
}

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
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>> {
        self.check_as(state, guarded, record_id, on_date, None)
    }

    /// [`check`](Self::check) for the user `actor` who runs the
    /// transition. A failing `block` entry is covered, and listed in
    /// [`CheckOutcome::overrides`], only when an approved, unused, valid
    /// override of that kind, subject, transition and record was requested
    /// by `actor`. `None` (no actor) covers nothing.
    fn check_as<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        record_id: i64,
        on_date: NaiveDate,
        actor: Option<i64>,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>>;

    /// Claims the overrides `covered` for one use, all or none: each must
    /// still be unused and valid, and is stamped used. `false` when any
    /// cannot be claimed (e.g. a concurrent attempt got it first); then
    /// none stays claimed. The default is the no-op fallback (`true`).
    fn claim_overrides<'a>(
        &'a self,
        _db: &'a Db,
        _covered: &'a [CoveredBlock],
        _actor: i64,
    ) -> DocumentFuture<'a, bool> {
        Box::pin(async { Ok(true) })
    }

    /// Gives the claimed overrides `covered` back (used at empty, approved
    /// again) after the transition failed. The default does nothing.
    fn unclaim_overrides<'a>(
        &'a self,
        _db: &'a Db,
        _covered: &'a [CoveredBlock],
    ) -> DocumentFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// Reports that the transition `guarded` ran on record `record_id`
    /// under the claimed overrides `covered`: emits one
    /// [`DocumentBlockOverridden`] each. The default does nothing.
    fn overrides_used<'a>(
        &'a self,
        _state: &'a AppState,
        _guarded: &'a str,
        _record_id: i64,
        _covered: &'a [CoveredBlock],
    ) -> DocumentFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// Warns about planned work: after the module saved `items` (a run, a
    /// deployment, a batch of sessions), checks each item's start date
    /// `on_date` against the `block` entries of `guarded` (the
    /// `<entity>.<transition>` the items will pass later). A flag-mode
    /// entry gives no warning, and an entry whose condition does not match
    /// a record gives none for it. Every subject and kind that is not
    /// valid on a date adds that date to its [`PlannedWarningLine`];
    /// `Ok(None)` when all are valid. It never refuses, so a warning never
    /// stops the save.
    ///
    /// A provider that is [`bookable`](BlockSubjects::bookable) gives
    /// [`PlannedError::BookableSlot`] and no warning.
    ///
    /// The default is the no-op fallback (`Ok(None)`), as without
    /// `wl-documents`.
    fn check_planned<'a>(
        &'a self,
        _state: &'a AppState,
        _guarded: &'a str,
        _items: &'a [PlannedItem],
    ) -> DocumentFuture<'a, std::result::Result<Option<PlannedWarning>, PlannedError>> {
        Box::pin(async { Ok(Ok(None)) })
    }

    /// Writes `flags` on record `record_id` of the transition `guarded`,
    /// after the transition ran. A flag that repeats an unreviewed one
    /// (same entry, kind, subject and reason) adds no row. Returns the
    /// rows written.
    fn record_flags<'a>(
        &'a self,
        db: &'a Db,
        guarded: &'a str,
        record_id: i64,
        flags: &'a [FlagFailure],
    ) -> DocumentFuture<'a, Vec<RecordFlag>>;

    /// Every flag of a record, reviewed or not, oldest first.
    fn flags_for<'a>(
        &'a self,
        db: &'a Db,
        record_type: &'a str,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<RecordFlag>>;

    /// The number of unreviewed flags per record id, for list badges. A
    /// provided guard has an entry for every id in `ids`; an id missing
    /// from the map has none.
    fn open_flag_counts<'a>(
        &'a self,
        db: &'a Db,
        record_type: &'a str,
        ids: &'a [i64],
    ) -> DocumentFuture<'a, BTreeMap<i64, u64>>;

    /// Marks flag `flag_id` reviewed by user `reviewer` with a required
    /// `note`. A reviewed flag cannot be reviewed again, and the flag is
    /// never removed.
    fn review_flag<'a>(
        &'a self,
        db: &'a Db,
        flag_id: i64,
        reviewer: i64,
        note: &'a str,
    ) -> DocumentFuture<'a, RecordFlag>;
}

/// Runs a guarded transition: [`DocumentGuard::check`], then `run` (the
/// module's `record.transition(..)` call), then
/// [`DocumentGuard::record_flags`] for the flag-mode failures.
///
/// A [`BlockRefusal`] is answered with [`BlockRefusal::into_error`] (422)
/// and `run` does not run; if `run` fails, no flag is written. Without
/// `wl-documents` it just runs `run`.
pub async fn guarded_transition<T, F, Fut>(
    state: &AppState,
    guarded: &str,
    record_id: i64,
    on_date: NaiveDate,
    run: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    guarded_transition_as(state, guarded, record_id, on_date, None, run).await
}

/// [`guarded_transition`] for the user `actor`, who may hold approved
/// block overrides ([`DocumentGuard::check_as`]).
///
/// The overrides the check covers are claimed for one use before `run`
/// ([`DocumentGuard::claim_overrides`]); if a claim fails (a concurrent
/// attempt won), the transition is refused with the block message and `run`
/// does not run. If `run` fails the claims are given back
/// ([`DocumentGuard::unclaim_overrides`]); if it succeeds,
/// [`DocumentGuard::overrides_used`] emits [`DocumentBlockOverridden`].
/// Without `wl-documents` it just runs `run`.
pub async fn guarded_transition_as<T, F, Fut>(
    state: &AppState,
    guarded: &str,
    record_id: i64,
    on_date: NaiveDate,
    actor: Option<i64>,
    run: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let guard = guard(state);
    let outcome = guard
        .check_as(state, guarded, record_id, on_date, actor)
        .await?
        .map_err(BlockRefusal::into_error)?;
    let claimed = match actor {
        Some(actor) if !outcome.overrides.is_empty() => {
            if !guard
                .claim_overrides(&state.db, &outcome.overrides, actor)
                .await?
            {
                // Lost the claim: the check now sees the override as used.
                return Err(
                    match guard
                        .check_as(state, guarded, record_id, on_date, Some(actor))
                        .await?
                    {
                        Err(refusal) => refusal.into_error(),
                        Ok(_) => Error::Status(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            "The override could not be used".to_owned(),
                        ),
                    },
                );
            }
            &outcome.overrides[..]
        }
        _ => &[],
    };
    let done = match run().await {
        Ok(done) => done,
        Err(err) => {
            if !claimed.is_empty() {
                guard.unclaim_overrides(&state.db, claimed).await?;
            }
            return Err(err);
        }
    };
    if !claimed.is_empty() {
        guard
            .overrides_used(state, guarded, record_id, claimed)
            .await?;
    }
    if !outcome.flags.is_empty() {
        guard
            .record_flags(&state.db, guarded, record_id, &outcome.flags)
            .await?;
    }
    Ok(done)
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

    fn check_as<'a>(
        &'a self,
        _state: &'a AppState,
        _guarded: &'a str,
        _record_id: i64,
        _on_date: NaiveDate,
        _actor: Option<i64>,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>> {
        Box::pin(async { Ok(Ok(CheckOutcome::default())) })
    }

    fn record_flags<'a>(
        &'a self,
        _db: &'a Db,
        _guarded: &'a str,
        _record_id: i64,
        _flags: &'a [FlagFailure],
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn flags_for<'a>(
        &'a self,
        _db: &'a Db,
        _record_type: &'a str,
        _record_id: i64,
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn open_flag_counts<'a>(
        &'a self,
        _db: &'a Db,
        _record_type: &'a str,
        _ids: &'a [i64],
    ) -> DocumentFuture<'a, BTreeMap<i64, u64>> {
        Box::pin(async { Ok(BTreeMap::new()) })
    }

    fn review_flag<'a>(
        &'a self,
        _db: &'a Db,
        _flag_id: i64,
        _reviewer: i64,
        _note: &'a str,
    ) -> DocumentFuture<'a, RecordFlag> {
        // Nothing is ever flagged without the module.
        Box::pin(async { Err(Error::NotFound) })
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

    /// Whether the transition books a slot (`booking.book`,
    /// `booking.reschedule`, `booking.reassign`). A bookable transition is
    /// refused through [`DocumentGuard::is_valid`] (`SCH.ACL.04`), and
    /// [`DocumentGuard::check_planned`] gives
    /// [`PlannedError::BookableSlot`] for it. Default: `false`.
    fn bookable(&self) -> bool {
        false
    }

    /// The label of `subject` in a [`PlannedWarning`], e.g. "Ravi S.".
    /// Default: `None`, which gives [`default_subject_label`].
    fn label<'a>(
        &'a self,
        _state: &'a AppState,
        _subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    /// The scope of record `record_id`, e.g. the deployment's site: who may
    /// approve an override of its block. Default: `None` (global).
    fn scope<'a>(
        &'a self,
        _state: &'a AppState,
        _entry: &'a BlockEntry,
        _record_id: i64,
    ) -> DocumentFuture<'a, Option<Scope>> {
        Box::pin(async { Ok(None) })
    }
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

/// A guarded transition ran under an approved block override (emitted once,
/// after the transition committed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentBlockOverridden {
    /// The override that was used.
    pub override_id: OverrideId,
    /// The overridden kind's key.
    pub kind: String,
    /// The document subject that failed.
    pub subject: Subject,
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// The guarded record.
    pub record_id: i64,
    /// The reason given with the request.
    pub reason: String,
    /// The user who requested the override and ran the transition.
    pub requested_by: i64,
    /// The user who approved it.
    pub approved_by: i64,
}

impl Event for DocumentBlockOverridden {}

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

/// The id of an SOP, as stored by `wl-documents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SopId(pub i64);

impl fmt::Display for SopId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for SopId {
    fn from(id: i64) -> Self {
        SopId(id)
    }
}

/// Who must read an SOP: everyone, or a role in a scope. Plain strings so
/// listeners need not depend on `wl-documents`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SopReaderRef {
    /// Every member of staff, whatever the role.
    pub all_staff: bool,
    /// The role or trade key; empty for an all-staff reader.
    pub role: String,
    /// The scope type; empty (with `scope_id`) for every scope.
    pub scope_type: String,
    /// The scope id; empty for every scope.
    pub scope_id: String,
}

/// A member of staff as the SOP acknowledgements see them: the employee, the
/// job role or trade, and the scopes they work in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SopStaff {
    /// The employee, e.g. `hr.employee:7`.
    pub employee: Subject,
    /// The job role or trade key.
    pub role: String,
    /// The scopes the employee works in.
    pub scopes: Vec<Subject>,
}

/// An employee acknowledged the current version of an SOP (`DOC.RUL.04`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SopAcknowledged {
    /// The SOP.
    pub sop: SopId,
    /// The employee.
    pub employee: Subject,
    /// The staff user who recorded it in person, when the employee did not
    /// do it in their own session.
    pub witness: Option<i64>,
}

impl Event for SopAcknowledged {}

/// The future the [`SopAcknowledgements`] methods return.
pub type SopFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What the onboarding asks of the SOP acknowledgements (wl-documents §6).
/// `wl-documents` provides it with [`provide_sop_acknowledgements`]; without
/// it, [`sop_acknowledgements`] gives [`NoSopAcknowledgements`]: no SOP is
/// pending for anyone.
pub trait SopAcknowledgements: Send + Sync + 'static {
    /// The SOPs whose current version `who` has not acknowledged and whose
    /// required readers list the role and scope of `who`. The default has
    /// none.
    fn pending_for<'a>(
        &'a self,
        _state: &'a AppState,
        _who: &'a SopStaff,
    ) -> SopFuture<'a, Vec<SopId>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Records the acknowledgement of every pending SOP of `who` in person,
    /// with the drawn `signature`, `witness` being the staff user. Returns
    /// the SOPs acknowledged. The default records nothing.
    fn acknowledge_in_person<'a>(
        &'a self,
        _state: &'a AppState,
        _who: &'a SopStaff,
        _signature: &'a str,
        _witness: i64,
    ) -> SopFuture<'a, Vec<SopId>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The SOP acknowledgements when the `documents` module is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSopAcknowledgements;

impl SopAcknowledgements for NoSopAcknowledgements {}

/// Makes `acknowledgements` the app's [`SopAcknowledgements`].
pub fn provide_sop_acknowledgements(
    app: &mut Registry,
    acknowledgements: impl SopAcknowledgements,
) {
    let acknowledgements: Arc<dyn SopAcknowledgements> = Arc::new(acknowledgements);
    app.provide(acknowledgements);
}

/// The app's [`SopAcknowledgements`], or [`NoSopAcknowledgements`] when none
/// is provided.
pub fn sop_acknowledgements(state: &AppState) -> Arc<dyn SopAcknowledgements> {
    match state.provided::<Arc<dyn SopAcknowledgements>>() {
        Some(acknowledgements) => Arc::clone(&acknowledgements),
        None => Arc::new(NoSopAcknowledgements),
    }
}

/// An SOP version was accepted and is now the SOP's current version
/// (`accept_sop`). Acknowledgements (#53) and onboarding listen to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SopPublished {
    /// The SOP.
    pub sop: SopId,
    /// The accepted version document.
    pub document: DocumentId,
    /// The SOP's running version number (1, 2, ...).
    pub version: i64,
    /// Days a reader has to acknowledge it.
    pub ack_due_days: i64,
    /// The required readers at the time of publication.
    pub readers: Vec<SopReaderRef>,
}

impl Event for SopPublished {}

/// An SOP was withdrawn (its current version was archived).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SopWithdrawn {
    /// The SOP.
    pub sop: SopId,
    /// The version that was current.
    pub document: DocumentId,
    /// The running number of that version.
    pub version: i64,
    /// The pending acknowledgements the withdrawal cancelled.
    pub cancelled_acknowledgements: Vec<i64>,
}

impl Event for SopWithdrawn {}

/// The required readers of an active SOP changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SopReadersChanged {
    /// The SOP.
    pub sop: SopId,
    /// Readers that are new (a changed scope counts as added).
    pub added: Vec<SopReaderRef>,
    /// Readers that are gone (a changed scope counts as removed).
    pub removed: Vec<SopReaderRef>,
}

impl Event for SopReadersChanged {}

/// A planned record a blocking document would not cover: it is dated after
/// the document's expiry date.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlannedRecord {
    /// The planned record, e.g. `ticketing.ticket #12`.
    pub record: Subject,
    /// The date it is planned for (Asia/Dubai calendar date).
    pub date: NaiveDate,
    /// What to show to the staff, e.g. "Fire-safety visit".
    pub label: String,
    /// The scope the record lives in; `None` when it has none.
    pub scope: Option<Scope>,
}

/// The planned records found for one guarded transition.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AffectedGroup {
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// Its planned records, in provider order.
    pub records: Vec<PlannedRecord>,
}

/// Lists the planned work of a guarded transition that a document would
/// not cover. The guarded module registers one per
/// `<entity>.<transition>` with [`register_future_work`]. Unlike
/// [`BlockSubjects`] it is optional: an entry without a provider is
/// skipped.
pub trait FutureWork: Send + Sync + 'static {
    /// The records planned on or after `from` whose guarded transition
    /// needs `subject`'s document of `entry.kind`. `entry` is the `blocks`
    /// entry (`block` or `flag`) with its condition.
    ///
    /// The caller passes `from` = the document's expiry date + 1 day: the
    /// document is valid through its expiry date, so records planned on
    /// that date are never listed. A record that does not match
    /// `entry.condition` is left out, as [`BlockSubjects::subjects`] does.
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>>;

    /// The records already running that need `subject`'s document of
    /// `entry.kind`, e.g. an active deployment. Used when the document has
    /// expired. The default lists none. A record that does not match
    /// `entry.condition` is left out.
    fn open_for<'a>(
        &'a self,
        _state: &'a AppState,
        _entry: &'a BlockEntry,
        _subject: &'a Subject,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

static FUTURE_WORK: RwLock<BTreeMap<String, Arc<dyn FutureWork>>> = RwLock::new(BTreeMap::new());

/// Adds `provider` for `guarded` (`<entity>.<transition>`) to the
/// process-wide registry; registering the same transition again replaces
/// it.
pub fn register_future_work(guarded: &str, provider: impl FutureWork) {
    FUTURE_WORK
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(guarded.to_owned(), Arc::new(provider));
}

/// The [`FutureWork`] provider of `guarded`, if registered.
pub fn future_work(guarded: &str) -> Option<Arc<dyn FutureWork>> {
    FUTURE_WORK
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(guarded)
        .cloned()
}

/// Names the scopes whose staff get a document kind's reminders when the
/// subject and the affected records have no scope. The vertical registers
/// one per document kind key with [`register_reminder_schedule`].
pub trait ReminderSchedule: Send + Sync + 'static {
    /// The scopes for `subject`'s document. `None` means "no answer": the
    /// reminder goes to every scope.
    fn scopes<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<Vec<Scope>>>;
}

static REMINDER_SCHEDULES: RwLock<BTreeMap<String, Arc<dyn ReminderSchedule>>> =
    RwLock::new(BTreeMap::new());

/// Adds `schedule` for the document kind `kind` to the process-wide
/// registry; registering the same kind again replaces it.
pub fn register_reminder_schedule(kind: &str, schedule: impl ReminderSchedule) {
    REMINDER_SCHEDULES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(kind.to_owned(), Arc::new(schedule));
}

/// The [`ReminderSchedule`] of the document kind `kind`, if registered.
pub fn reminder_schedule(kind: &str) -> Option<Arc<dyn ReminderSchedule>> {
    REMINDER_SCHEDULES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(kind)
        .cloned()
}

/// An accepted document is about to expire: a reminder offset was reached,
/// or planned work now falls after its expiry date. Emitted at most once
/// per document and night. It changes no planned item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentExpiring {
    /// The document.
    pub document_id: DocumentId,
    /// What it is attached to.
    pub subject: Subject,
    /// The kind's key.
    pub kind: String,
    /// The expiry date; the document is valid through it.
    pub expires_on: NaiveDate,
    /// Expiry date minus today.
    pub days_left: i64,
    /// The reminder offset reached tonight, in days; `None` when only
    /// planned work triggered the event.
    pub offset: Option<u32>,
    /// The full current list of planned records dated after the expiry
    /// date, grouped by guarded transition (not only the new ones).
    pub affected: Vec<AffectedGroup>,
}

impl Event for DocumentExpiring {}

/// The records a guarded transition has with an expired document: those
/// already running and those planned after the expiry date.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlockedGroup {
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// How the document's entry for this transition acts.
    pub mode: BlockMode,
    /// The records already running (from [`FutureWork::open_for`]).
    pub open: Vec<PlannedRecord>,
    /// The records planned after the expiry date (from
    /// [`FutureWork::planned_from`]).
    pub planned: Vec<PlannedRecord>,
}

/// A blocking document has expired: emitted once, on the first nightly run
/// after its expiry date. It changes no record; consumers react to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentExpired {
    /// The document.
    pub document: DocumentId,
    /// What it is attached to.
    pub subject: Subject,
    /// The kind's key.
    pub kind: String,
    /// The expiry date; the document was valid through it.
    pub expires_on: NaiveDate,
    /// Every `blocks` entry of the kind, `block` and `flag` alike.
    pub blocks: Vec<BlockEntry>,
    /// The records found per guarded transition, in entry order.
    pub blocked: Vec<BlockedGroup>,
}

impl Event for DocumentExpired {}

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
        assert!(outcome.unwrap().flags.is_empty());
    }

    fn flag_failure() -> FlagFailure {
        FlagFailure {
            entry: BlockEntry {
                guarded: "attendance_record.accept".into(),
                mode: BlockMode::Flag,
                ..visa_entry()
            },
            subject: Subject::new("hr.employee", 1),
            reason: FlagReason::Expired,
            document: Some(DocumentId(3)),
            expired_on: Some(date("2026-11-30")),
            on_date: date("2026-12-02"),
            label: "Work permit expired 2026-11-30".into(),
        }
    }

    #[renox::test]
    async fn without_documents_nothing_is_flagged() {
        let app = TestApp::new(App::new()).await;
        let guard = guard(app.state());
        let written = guard
            .record_flags(app.db(), "attendance_record.accept", 5, &[flag_failure()])
            .await
            .unwrap();
        assert!(written.is_empty());
        assert!(
            guard
                .flags_for(app.db(), "attendance_record", 5)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            guard
                .open_flag_counts(app.db(), "attendance_record", &[5, 6])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            guard.review_flag(app.db(), 1, 2, "Seen").await,
            Err(Error::NotFound)
        ));
    }

    #[renox::test]
    async fn without_documents_a_guarded_transition_just_runs() {
        let app = TestApp::new(App::new()).await;
        let ran = std::sync::atomic::AtomicBool::new(false);
        let value = guarded_transition(
            app.state(),
            "attendance_record.accept",
            5,
            date("2026-12-02"),
            || async {
                ran.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(42)
            },
        )
        .await
        .unwrap();
        assert_eq!(value, 42);
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));

        // The closure's error comes back as it is.
        let failed: Result<()> = guarded_transition(
            app.state(),
            "attendance_record.accept",
            5,
            date("2026-12-02"),
            || async { Err(Error::NotFound) },
        )
        .await;
        assert!(matches!(failed, Err(Error::NotFound)));
    }

    #[test]
    fn a_flag_reason_follows_the_invalid_reason() {
        assert_eq!(
            FlagReason::from(InvalidReason::PendingReview),
            FlagReason::PendingReview
        );
        assert_eq!(FlagReason::PendingReview.as_str(), "pending_review");
        assert_eq!(
            FlagReason::from(InvalidReason::Missing),
            FlagReason::Missing
        );
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

    fn planned_line(label: &str, text: &str, dates: &[&str]) -> PlannedWarningLine {
        PlannedWarningLine {
            subject: Subject::new("hr.employee", 7),
            label: label.into(),
            kind: "residence_visa".into(),
            kind_name: "Residence visa".into(),
            reason: InvalidReason::Expired,
            expires_on: Some(date("2026-12-15")),
            text: text.into(),
            dates: dates.iter().map(|d| date(d)).collect(),
        }
    }

    #[renox::test]
    async fn without_documents_nothing_is_warned_about() {
        let app = TestApp::new(App::new()).await;
        let items = [PlannedItem::new(3, date("2026-12-20"))];
        assert_eq!(
            guard(app.state())
                .check_planned(app.state(), "deployment.activate", &items)
                .await
                .expect("no failure"),
            Ok(None)
        );
        assert_eq!(
            NoDocuments
                .check_planned(app.state(), "booking.book", &[])
                .await
                .expect("no failure"),
            Ok(None)
        );
    }

    #[renox::test]
    async fn a_provider_is_not_bookable_and_has_no_label_by_default() {
        let app = TestApp::new(App::new()).await;
        assert!(!Nobody.bookable());
        assert_eq!(
            Nobody
                .label(app.state(), &Subject::new("hr.employee", 7))
                .await
                .expect("no failure"),
            None
        );
        assert_eq!(
            default_subject_label(&Subject::new("hr.employee", 7)),
            "hr.employee #7"
        );
    }

    #[test]
    fn a_planned_warning_gives_one_line_per_subject_and_kind() {
        let warning = PlannedWarning {
            guarded: "deployment.activate".into(),
            lines: vec![
                planned_line(
                    "Ravi S.",
                    "Residence visa expires 2026-12-15",
                    &["2027-01-04", "2027-01-11"],
                ),
                planned_line("worker #8", "Residence visa missing", &["2027-01-04"]),
            ],
        };
        assert_eq!(
            warning.message(),
            "Ravi S.: Residence visa expires 2026-12-15; planned: 2027-01-04, 2027-01-11\n\
             worker #8: Residence visa missing; planned: 2027-01-04"
        );
        assert_eq!(warning.to_string(), warning.message());
    }

    #[test]
    fn a_planned_warning_is_a_persistent_warning_toast() {
        let one = PlannedWarning {
            guarded: "deployment.activate".into(),
            lines: vec![planned_line(
                "Ravi S.",
                "Residence visa expires 2026-12-15",
                &["2026-12-20"],
            )],
        };
        let toast = one.toast();
        assert_eq!(toast.kind, renox::ToastKind::Warning);
        assert_eq!(
            toast.message,
            "Ravi S.: Residence visa expires 2026-12-15; planned: 2026-12-20"
        );
        assert_eq!(toast.body, None);
        assert_eq!(toast.duration, Some(0));

        let two = PlannedWarning {
            guarded: one.guarded.clone(),
            lines: vec![
                one.lines[0].clone(),
                planned_line("Mia T.", "Residence visa missing", &["2026-12-21"]),
            ],
        };
        let toast = two.toast();
        assert_eq!(toast.message, one.lines[0].message());
        assert_eq!(
            toast.body.as_deref(),
            Some("Mia T.: Residence visa missing; planned: 2026-12-21")
        );
    }

    #[test]
    fn a_bookable_slot_error_names_the_transition() {
        let error = PlannedError::BookableSlot {
            guarded: "booking.book".into(),
        };
        assert!(error.to_string().contains("booking.book"));
    }

    #[test]
    fn providers_are_registered_by_transition() {
        register_block_subjects("test_unit.guarded", Nobody);
        assert!(block_subjects("test_unit.guarded").is_some());
        assert!(block_subjects("test_unit.unknown").is_none());
        assert!(guarded_transitions().contains(&"test_unit.guarded".to_owned()));
    }

    struct OneVisit;

    impl FutureWork for OneVisit {
        fn planned_from<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a BlockEntry,
            subject: &'a Subject,
            from: NaiveDate,
        ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
            let record = PlannedRecord {
                record: Subject::new("test.visit", 1),
                date: from,
                label: format!("visit for {}", subject.subject_id),
                scope: None,
            };
            Box::pin(async move { Ok(vec![record]) })
        }
    }

    struct AnyScope;

    impl ReminderSchedule for AnyScope {
        fn scopes<'a>(
            &'a self,
            _state: &'a AppState,
            _subject: &'a Subject,
        ) -> DocumentFuture<'a, Option<Vec<Scope>>> {
            Box::pin(async { Ok(None) })
        }
    }

    #[test]
    fn future_work_is_registered_by_transition() {
        register_future_work("test_unit.planned", OneVisit);
        assert!(future_work("test_unit.planned").is_some());
        assert!(future_work("test_unit.unknown").is_none());
        // Independent of the block-subjects registry.
        assert!(block_subjects("test_unit.planned").is_none());
    }

    #[test]
    fn reminder_schedules_are_registered_by_kind() {
        register_reminder_schedule("test_unit_kind", AnyScope);
        assert!(reminder_schedule("test_unit_kind").is_some());
        assert!(reminder_schedule("test_unit_other").is_none());
    }

    #[renox::test]
    async fn a_provider_gets_the_day_after_expiry() {
        let app = TestApp::new(App::new()).await;
        let entry = BlockEntry {
            kind: "civil_defence".into(),
            guarded: "ticket.resolve".into(),
            mode: BlockMode::Block,
            override_mode: OverrideMode::Never,
            condition: None,
            locked: false,
        };
        let from = date("2026-12-15").succ_opt().unwrap();
        let subject = Subject::new("parties.vendor", 4);
        let found = OneVisit
            .planned_from(app.state(), &entry, &subject, from)
            .await
            .expect("no failure");
        assert_eq!(found[0].date, date("2026-12-16"));
        assert_eq!(found[0].label, "visit for 4");
        assert_eq!(
            AnyScope
                .scopes(app.state(), &subject)
                .await
                .expect("no failure"),
            None
        );
    }

    #[test]
    fn expiring_events_serialize_with_plain_ids() {
        let expiring = DocumentExpiring {
            document_id: DocumentId(5),
            subject: Subject::new("parties.vendor", 4),
            kind: "civil_defence".into(),
            expires_on: date("2026-12-15"),
            days_left: 30,
            offset: None,
            affected: vec![AffectedGroup {
                guarded: "ticket.resolve".into(),
                records: vec![PlannedRecord {
                    record: Subject::new("test.visit", 1),
                    date: date("2026-12-20"),
                    label: "Fire-safety visit".into(),
                    scope: None,
                }],
            }],
        };
        let json = renox::serde_json::to_value(&expiring).unwrap();
        assert_eq!(json["document_id"], 5);
        assert_eq!(json["days_left"], 30);
        assert_eq!(json["affected"][0]["records"][0]["date"], "2026-12-20");
    }

    #[test]
    fn expired_events_serialize_with_plain_ids() {
        let record = PlannedRecord {
            record: Subject::new("test.deployment", 3),
            date: date("2026-12-10"),
            label: "Deployment".into(),
            scope: None,
        };
        let expired = DocumentExpired {
            document: DocumentId(5),
            subject: Subject::new("hr.employee", 1),
            kind: "work_permit".into(),
            expires_on: date("2026-11-30"),
            blocks: vec![visa_entry()],
            blocked: vec![BlockedGroup {
                guarded: "deployment.resume".into(),
                mode: BlockMode::Block,
                open: Vec::new(),
                planned: vec![record],
            }],
        };
        let json = renox::serde_json::to_value(&expired).unwrap();
        assert_eq!(json["document"], 5);
        assert_eq!(json["expires_on"], "2026-11-30");
        assert_eq!(json["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(json["blocked"][0]["guarded"], "deployment.resume");
        assert_eq!(json["blocked"][0]["open"].as_array().unwrap().len(), 0);
        assert_eq!(json["blocked"][0]["planned"][0]["date"], "2026-12-10");
    }

    #[renox::test]
    async fn open_for_lists_nothing_by_default() {
        let app = TestApp::new(App::new()).await;
        let entry = visa_entry();
        let subject = Subject::new("hr.employee", 1);
        let open = OneVisit
            .open_for(app.state(), &entry, &subject)
            .await
            .expect("no failure");
        assert!(open.is_empty());
    }

    #[test]
    fn sop_events_serialize_with_plain_ids() {
        let reader = SopReaderRef {
            all_staff: false,
            role: "site_worker".into(),
            scope_type: String::new(),
            scope_id: String::new(),
        };
        let published = SopPublished {
            sop: SopId(3),
            document: DocumentId(9),
            version: 2,
            ack_due_days: 7,
            readers: vec![reader.clone()],
        };
        let json = renox::serde_json::to_value(&published).unwrap();
        assert_eq!(json["sop"], 3);
        assert_eq!(json["document"], 9);
        assert_eq!(json["readers"][0]["role"], "site_worker");
        let back: SopPublished = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, published);
        let changed = SopReadersChanged {
            sop: SopId(3),
            added: vec![],
            removed: vec![reader],
        };
        assert_eq!(renox::serde_json::to_value(&changed).unwrap()["sop"], 3);
        let withdrawn = SopWithdrawn {
            sop: SopId(3),
            document: DocumentId(9),
            version: 2,
            cancelled_acknowledgements: vec![],
        };
        let json = renox::serde_json::to_value(&withdrawn).unwrap();
        assert_eq!(
            json["cancelled_acknowledgements"].as_array().unwrap().len(),
            0
        );
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

    fn failure(kind: &str, override_mode: OverrideMode, locked: bool) -> BlockFailure {
        BlockFailure {
            entry: BlockEntry {
                kind: kind.into(),
                guarded: "deployment.activate".into(),
                mode: BlockMode::Block,
                override_mode,
                condition: None,
                locked,
            },
            subject: Subject::new("hr.employee", 1),
            validity: Validity::Invalid {
                reason: InvalidReason::Missing,
                document: None,
                expired_on: None,
                pending: None,
            },
            message: format!("{kind} missing"),
        }
    }

    #[test]
    fn only_a_with_approval_kind_that_is_not_a_legal_block_offers_an_override() {
        let health = failure("health_card", OverrideMode::WithApproval, false);
        let never = failure("noc", OverrideMode::Never, false);
        let legal = failure("sira_licence", OverrideMode::WithApproval, true);
        assert!(health.override_allowed());
        assert!(!never.override_allowed());
        assert!(!legal.override_allowed());
        let refusal = BlockRefusal {
            guarded: "deployment.activate".into(),
            record_id: 7,
            failures: vec![health, never, legal],
        };
        assert_eq!(
            refusal.override_links(),
            vec!["/documents/overrides/new?kind=health_card&guarded=deployment.activate&record=7"]
        );
    }

    #[renox::test]
    async fn without_documents_nothing_is_covered_or_claimed() {
        let app = TestApp::new(App::new()).await;
        let guard = guard(app.state());
        let outcome = guard
            .check_as(
                app.state(),
                "deployment.activate",
                7,
                date("2026-12-01"),
                Some(3),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(outcome.overrides.is_empty());
        let covered = [CoveredBlock {
            override_id: OverrideId(4),
            entry: failure("health_card", OverrideMode::WithApproval, false).entry,
            subject: Subject::new("hr.employee", 1),
        }];
        assert!(guard.claim_overrides(app.db(), &covered, 3).await.unwrap());
        guard.unclaim_overrides(app.db(), &covered).await.unwrap();
        guard
            .overrides_used(app.state(), "deployment.activate", 7, &covered)
            .await
            .unwrap();
    }

    #[renox::test]
    async fn without_documents_a_guarded_transition_as_just_runs() {
        let app = TestApp::new(App::new()).await;
        let value = guarded_transition_as(
            app.state(),
            "deployment.activate",
            7,
            date("2026-12-02"),
            Some(3),
            || async { Ok(9) },
        )
        .await
        .unwrap();
        assert_eq!(value, 9);
        let failed: Result<()> = guarded_transition_as(
            app.state(),
            "deployment.activate",
            7,
            date("2026-12-02"),
            Some(3),
            || async { Err(Error::NotFound) },
        )
        .await;
        assert!(matches!(failed, Err(Error::NotFound)));
    }

    #[test]
    fn the_override_event_serializes_with_plain_ids() {
        let event = DocumentBlockOverridden {
            override_id: OverrideId(4),
            kind: "health_card".into(),
            subject: Subject::new("hr.employee", 1),
            guarded: "deployment.activate".into(),
            record_id: 7,
            reason: "renewal booked".into(),
            requested_by: 2,
            approved_by: 3,
        };
        let json = renox::serde_json::to_value(&event).unwrap();
        assert_eq!(json["override_id"], 4);
        assert_eq!(json["kind"], "health_card");
        assert_eq!(json["guarded"], "deployment.activate");
        assert_eq!(json["record_id"], 7);
        assert_eq!(json["requested_by"], 2);
        assert_eq!(json["approved_by"], 3);
        assert_eq!(json["reason"], "renewal booked");
    }
}
