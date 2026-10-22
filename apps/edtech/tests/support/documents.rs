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
    DocumentFuture, DocumentGuard, FlagFailure, InvalidReason, OverrideMode, RecordFlag, Validity,
    block_subjects,
};

/// The expiry date of each `(subject, kind)` document.
type Expiries = BTreeMap<(String, String), Option<NaiveDate>>;

/// The kinds' `blocks` and the expiry dates of the documents.
#[derive(Clone, Default)]
pub struct Documents {
    entries: Arc<Mutex<Vec<BlockEntry>>>,
    expiries: Arc<Mutex<Expiries>>,
    types: Arc<Mutex<BTreeMap<String, Vec<String>>>>,
}

/// The name of a kind in a refusal.
fn kind_name(kind: &str) -> &str {
    match kind {
        "centre_permit" => "Centre permit",
        "course_approval" => "Course approval",
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
            for entry in entries {
                for subject in provider.subjects(state, &entry, record_id).await? {
                    let validity = self.validity_of(&subject, &entry.kind, on_date);
                    if validity.is_valid() {
                        continue;
                    }
                    let name = kind_name(&entry.kind);
                    let what = match &validity {
                        Validity::Invalid {
                            expired_on: Some(date),
                            ..
                        } => format!("{name} expired on {date}"),
                        _ => format!("{name} missing"),
                    };
                    failures.push(BlockFailure {
                        entry: entry.clone(),
                        subject,
                        validity,
                        message: format!("{what}, judged on {on_date}; override: not allowed"),
                    });
                }
            }
            if failures.is_empty() {
                Ok(Ok(CheckOutcome::default()))
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
        Box::pin(async { Err(Error::NotFound) })
    }
}
