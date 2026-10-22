//! A document guard for the edtech tests: the app does not depend on
//! `wl-documents`, so the tests provide a `DocumentGuard` that answers
//! `document_valid` from a table the test fills, and checks a guarded
//! transition through the app's own `BlockSubjects` provider, as `wl-documents`
//! does. With no entry it guards nothing, like a client without the module.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockCondition, BlockEntry, BlockFailure, BlockMode, BlockRefusal, CheckOutcome,
    DocumentFuture, DocumentGuard, FlagFailure, FlagReason, InvalidReason, OverrideMode,
    RecordFlag, Validity, block_subjects,
};

/// The expiry date of each `(subject, kind)` document.
type Expiries = BTreeMap<(String, String), Option<NaiveDate>>;

/// The kinds' `blocks` and the expiry dates of the documents.
#[derive(Clone, Default)]
pub struct Documents {
    entries: Arc<Mutex<Vec<BlockEntry>>>,
    expiries: Arc<Mutex<Expiries>>,
    types: Arc<Mutex<BTreeMap<String, Vec<String>>>>,
    flags: Arc<Mutex<Vec<RecordFlag>>>,
}

/// The name of a kind in a refusal.
fn kind_name(kind: &str) -> &str {
    match kind {
        "centre_permit" => "Centre permit",
        "course_approval" => "Course approval",
        "police_clearance" => "Police clearance",
        "tutor_permit" => "Tutor permit",
        other => other,
    }
}

impl Documents {
    /// 10's seed: the centre permit blocks `batch.open` and `certificate.issue`,
    /// the course approval blocks `batch.open` and, for a certificate that
    /// states it, `certificate.issue`; every override is never (locked).
    pub fn edtech_blocks(&self) {
        let entry = |kind: &str, guarded: &str, condition: Option<BlockCondition>| BlockEntry {
            kind: kind.into(),
            guarded: guarded.into(),
            mode: BlockMode::Block,
            override_mode: OverrideMode::Never,
            condition,
            locked: true,
        };
        self.block(entry("centre_permit", "batch.open", None));
        self.block(entry("centre_permit", "certificate.issue", None));
        self.block(entry("course_approval", "batch.open", None));
        self.block(entry(
            "course_approval",
            "certificate.issue",
            Some(BlockCondition::new("states_approval", ["true"])),
        ));
        self.attached("centre_permit", &["kernel.legal_entity"]);
        self.attached("course_approval", &["edtech.course"]);
    }

    /// 10's tutor kinds (§9): the police clearance and the permit or visa block
    /// the assignments (`batch.open`, `batch.change_tutor`, `session.change_tutor`,
    /// `session.reschedule`) and **flag** the attendance (`session.submit_attendance`);
    /// both are attached to employees.
    pub fn tutor_blocks(&self) {
        for kind in ["police_clearance", "tutor_permit"] {
            for guarded in [
                "batch.open",
                "batch.change_tutor",
                "session.change_tutor",
                "session.reschedule",
            ] {
                self.block(BlockEntry {
                    kind: kind.into(),
                    guarded: guarded.into(),
                    mode: BlockMode::Block,
                    override_mode: OverrideMode::Never,
                    condition: None,
                    locked: false,
                });
            }
            self.block(BlockEntry {
                kind: kind.into(),
                guarded: "session.submit_attendance".into(),
                mode: BlockMode::Flag,
                override_mode: OverrideMode::Never,
                condition: None,
                locked: false,
            });
            self.attached(kind, &["hr.employee"]);
        }
    }

    /// Adds `entry` to the kinds' `blocks`.
    pub fn block(&self, entry: BlockEntry) {
        self.entries.lock().unwrap().push(entry);
    }

    /// The kind `kind` is attached to subjects of `types`.
    pub fn attached(&self, kind: &str, types: &[&str]) {
        self.types
            .lock()
            .unwrap()
            .insert(kind.to_owned(), types.iter().map(|t| (*t).to_owned()).collect());
    }

    /// `subject` has a document of `kind` valid through `expires` (`None`: it
    /// does not expire).
    pub fn set(&self, subject: &Subject, kind: &str, expires: Option<NaiveDate>) {
        self.expiries
            .lock()
            .unwrap()
            .insert((subject.to_string(), kind.to_owned()), expires);
    }

    /// `subject` has one more document of `kind`, valid through `expires`: it
    /// is valid while any of its documents is (a permit that ran out and a
    /// freelance visa that did not).
    pub fn also(&self, subject: &Subject, kind: &str, expires: NaiveDate) {
        let key = (subject.to_string(), kind.to_owned());
        let mut all = self.expiries.lock().unwrap();
        let later = match all.get(&key) {
            Some(None) => return,
            Some(Some(old)) => (*old).max(expires),
            None => expires,
        };
        all.insert(key, Some(later));
    }

    /// `subject` has no document of `kind`.
    pub fn remove(&self, subject: &Subject, kind: &str) {
        self.expiries
            .lock()
            .unwrap()
            .remove(&(subject.to_string(), kind.to_owned()));
    }

    fn validity_of(&self, subject: &Subject, kind: &str, on: NaiveDate) -> Validity {
        let found = self
            .expiries
            .lock()
            .unwrap()
            .get(&(subject.to_string(), kind.to_owned()))
            .copied();
        match found {
            None => Validity::Invalid {
                reason: InvalidReason::Missing,
                document: None,
                expired_on: None,
                pending: None,
            },
            Some(Some(expires)) if expires < on => Validity::Invalid {
                reason: InvalidReason::Expired,
                document: None,
                expired_on: Some(expires),
                pending: None,
            },
            Some(expires) => Validity::Valid {
                document: None,
                expires_on: expires,
            },
        }
    }

    fn entries_of(&self, guarded: &str) -> Vec<BlockEntry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.guarded == guarded)
            .cloned()
            .collect()
    }
}

impl DocumentGuard for Documents {
    fn blocks_for<'a>(
        &'a self,
        _db: &'a Db,
        guarded: &'a str,
    ) -> DocumentFuture<'a, Vec<BlockEntry>> {
        Box::pin(async move { Ok(self.entries_of(guarded)) })
    }

    fn is_valid<'a>(
        &'a self,
        _db: &'a Db,
        subject: &'a Subject,
        kind: &'a str,
        on_date: NaiveDate,
    ) -> DocumentFuture<'a, Validity> {
        Box::pin(async move { Ok(self.validity_of(subject, kind, on_date)) })
    }

    fn kind_subject_types<'a>(
        &'a self,
        _db: &'a Db,
        kind: &'a str,
    ) -> DocumentFuture<'a, Vec<String>> {
        let found = self.types.lock().unwrap().get(kind).cloned().unwrap_or_default();
        Box::pin(async move { Ok(found) })
    }

    fn check_as<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        record_id: i64,
        on_date: NaiveDate,
        _actor: Option<i64>,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>> {
        Box::pin(async move {
            let entries = self.entries_of(guarded);
            if entries.is_empty() {
                return Ok(Ok(CheckOutcome::default()));
            }
            let Some(provider) = block_subjects(guarded) else {
                return Err(Error::Internal(renox::anyhow::anyhow!(
                    "no BlockSubjects provider for {guarded}"
                )));
            };
            let mut failures = Vec::new();
            let mut outcome = CheckOutcome::default();
            for entry in entries {
                for subject in provider.subjects(state, &entry, record_id).await? {
                    let validity = self.validity_of(&subject, &entry.kind, on_date);
                    if validity.is_valid() {
                        continue;
                    }
                    let name = kind_name(&entry.kind);
                    let (what, expired_on, reason) = match &validity {
                        Validity::Invalid {
                            expired_on: Some(date),
                            ..
                        } => (
                            format!("{name} expired on {date}"),
                            Some(*date),
                            FlagReason::Expired,
                        ),
                        _ => (format!("{name} missing"), None, FlagReason::Missing),
                    };
                    if entry.mode == BlockMode::Flag {
                        outcome.flags.push(FlagFailure {
                            entry: entry.clone(),
                            subject,
                            reason,
                            document: None,
                            expired_on,
                            on_date,
                            label: what,
                        });
                        continue;
                    }
                    failures.push(BlockFailure {
                        entry: entry.clone(),
                        subject,
                        validity,
                        message: format!("{what}, judged on {on_date}; override: not allowed"),
                    });
                }
            }
            if failures.is_empty() {
                Ok(Ok(outcome))
            } else {
                Ok(Err(BlockRefusal {
                    guarded: guarded.to_owned(),
                    record_id,
                    failures,
                }))
            }
        })
    }

    fn record_flags<'a>(
        &'a self,
        _db: &'a Db,
        guarded: &'a str,
        record_id: i64,
        flags: &'a [FlagFailure],
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        let mut all = self.flags.lock().unwrap();
        let mut written = Vec::new();
        for flag in flags {
            let repeats = all.iter().any(|old| {
                old.guarded == guarded
                    && old.record_id == record_id
                    && old.kind == flag.entry.kind
                    && old.subject == flag.subject
                    && old.reason == flag.reason
                    && !old.is_reviewed()
            });
            if repeats {
                continue;
            }
            let row = RecordFlag {
                id: i64::try_from(all.len()).unwrap() + 1,
                record_type: guarded.split_once('.').map_or(guarded, |(entity, _)| entity).to_owned(),
                record_id,
                guarded: guarded.to_owned(),
                kind: flag.entry.kind.clone(),
                subject: flag.subject.clone(),
                document: None,
                reason: flag.reason,
                expires_on: flag.expired_on,
                on_date: flag.on_date,
                label: flag.label.clone(),
                created_at: renox::db::now(),
                reviewed_at: None,
                reviewed_by: None,
                reviewer_name: None,
                review_note: None,
            };
            all.push(row.clone());
            written.push(row);
        }
        Box::pin(async move { Ok(written) })
    }

    fn flags_for<'a>(
        &'a self,
        _db: &'a Db,
        record_type: &'a str,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        let found: Vec<RecordFlag> = self
            .flags
            .lock()
            .unwrap()
            .iter()
            .filter(|flag| flag.record_type == record_type && flag.record_id == record_id)
            .cloned()
            .collect();
        Box::pin(async move { Ok(found) })
    }

    fn open_flag_counts<'a>(
        &'a self,
        _db: &'a Db,
        record_type: &'a str,
        ids: &'a [i64],
    ) -> DocumentFuture<'a, BTreeMap<i64, u64>> {
        let mut counts: BTreeMap<i64, u64> = ids.iter().map(|id| (*id, 0)).collect();
        for flag in self.flags.lock().unwrap().iter() {
            if flag.record_type == record_type
                && !flag.is_reviewed()
                && let Some(count) = counts.get_mut(&flag.record_id)
            {
                *count += 1;
            }
        }
        Box::pin(async move { Ok(counts) })
    }

    fn review_flag<'a>(
        &'a self,
        _db: &'a Db,
        flag_id: i64,
        reviewer: i64,
        note: &'a str,
    ) -> DocumentFuture<'a, RecordFlag> {
        let mut all = self.flags.lock().unwrap();
        let outcome = match all.iter_mut().find(|flag| flag.id == flag_id) {
            None => Err(Error::NotFound),
            Some(_) if note.trim().is_empty() => Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "A note is needed".to_owned(),
            )),
            Some(flag) if flag.is_reviewed() => Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "The flag is already reviewed".to_owned(),
            )),
            Some(flag) => {
                flag.reviewed_at = Some(renox::db::now());
                flag.reviewed_by = Some(reviewer);
                flag.reviewer_name = Some(format!("Reviewer {reviewer}"));
                flag.review_note = Some(note.trim().to_owned());
                Ok(flag.clone())
            }
        };
        Box::pin(async move { outcome })
    }
}
