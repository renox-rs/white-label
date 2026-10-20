//! A document guard for the delivery tests: the module does not depend on
//! `wl-documents`, so the tests provide a `DocumentGuard` that answers
//! `document_valid` from a table the test fills, and checks a guarded
//! transition through the module's own `BlockSubjects` provider, as
//! `wl-documents` does. It counts how often `document_valid` was asked.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockEntry, BlockFailure, BlockMode, BlockRefusal, CheckOutcome, DocumentFuture,
    DocumentGuard, FlagFailure, InvalidReason, OverrideMode, RecordFlag, Validity, block_subjects,
};

/// The driver's licence (attaches to the employee).
pub const LICENCE: &str = "driving_licence";
/// The rider permit (attaches to the employee).
pub const RIDER_PERMIT: &str = "rider_permit";
/// The vehicle registration, Mulkiya (attaches to the asset).
pub const MULKIYA: &str = "mulkiya";
/// The vehicle insurance (attaches to the asset).
pub const INSURANCE: &str = "vehicle_insurance";

/// The expiry date of each `(subject, kind)` document.
type Expiries = BTreeMap<(String, String), Option<NaiveDate>>;

/// The kinds' `blocks` and the expiry dates of the documents.
#[derive(Clone, Default)]
pub struct Documents {
    entries: Arc<Mutex<Vec<BlockEntry>>>,
    expiries: Arc<Mutex<Expiries>>,
    asked: Arc<AtomicUsize>,
}

/// The subject types a kind attaches to.
fn types_of(kind: &str) -> Vec<String> {
    match kind {
        LICENCE | RIDER_PERMIT => vec!["hr.employee".to_owned()],
        MULKIYA | INSURANCE => vec!["inventory.asset".to_owned()],
        _ => Vec::new(),
    }
}

/// The name and the staff who renew a kind, as wl-documents puts them in a
/// refusal.
fn kind_name(kind: &str) -> (&'static str, &'static str) {
    match kind {
        LICENCE => ("Driving licence", "HR"),
        RIDER_PERMIT => ("Rider permit", "HR"),
        MULKIYA => ("Mulkiya", "Fleet"),
        INSURANCE => ("Vehicle insurance", "Fleet"),
        _ => ("Document", "Admin"),
    }
}

impl Documents {
    /// 03's seed: the four kinds block `delivery.assign` and `run.dispatch`,
    /// legal blocks (override mode `never`, locked).
    pub fn logistics() -> Self {
        let documents = Documents::default();
        for kind in [LICENCE, RIDER_PERMIT, MULKIYA, INSURANCE] {
            for guarded in ["delivery.assign", "run.dispatch"] {
                documents.block(kind, guarded, OverrideMode::Never, true);
            }
        }
        documents
    }

    /// No kind blocks anything.
    pub fn none() -> Self {
        Documents::default()
    }

    /// Adds an entry to the kinds' `blocks`.
    pub fn block(&self, kind: &str, guarded: &str, override_mode: OverrideMode, locked: bool) {
        self.entries.lock().unwrap().push(BlockEntry {
            kind: kind.to_owned(),
            guarded: guarded.to_owned(),
            mode: BlockMode::Block,
            override_mode,
            condition: None,
            locked,
        });
    }

    /// `subject` has a document of `kind` valid through `expires` (`None`:
    /// it does not expire).
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

    /// How often `document_valid` was asked.
    pub fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    fn validity_of(&self, subject: &Subject, kind: &str, on: NaiveDate) -> Validity {
        self.asked.fetch_add(1, Ordering::SeqCst);
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
        Box::pin(async move { Ok(types_of(kind)) })
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
                    let (name, renewer) = kind_name(&entry.kind);
                    let allowed =
                        entry.override_mode == OverrideMode::WithApproval && !entry.locked;
                    let what = match &validity {
                        Validity::Invalid {
                            expired_on: Some(date),
                            ..
                        } => format!("{name} expired on {date}"),
                        _ => format!("{name} missing"),
                    };
                    let message = format!(
                        "{what}; renew: {renewer}; override: {}",
                        if allowed { "with approval" } else { "not allowed" }
                    );
                    failures.push(BlockFailure {
                        entry: entry.clone(),
                        subject,
                        validity,
                        message,
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
