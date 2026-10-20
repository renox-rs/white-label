//! A document guard for the ticketing tests: the module does not depend on
//! `wl-documents`, so the tests provide a `DocumentGuard` that answers
//! `document_valid` from a table the test fills, and checks a guarded
//! transition through the module's own `BlockSubjects` provider, as
//! `wl-documents` does.

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

/// The key of 01's Civil Defence approval kind.
pub const CIVIL_DEFENCE: &str = "civil_defence_approval";

/// The expiry dates of the vendors' documents, and the kinds' `blocks`.
#[derive(Clone, Default)]
pub struct Documents {
    entries: Arc<Mutex<Vec<BlockEntry>>>,
    expiries: Arc<Mutex<BTreeMap<(String, String), Option<NaiveDate>>>>,
}

impl Documents {
    /// 01's seed: the Civil Defence approval blocks `ticket.resolve` and
    /// `service_contract.activate` for fire and life safety, as a legal block
    /// (override mode `never`, locked).
    pub fn civil_defence() -> Self {
        let documents = Documents::default();
        for guarded in ["ticket.resolve", "service_contract.activate"] {
            documents.block(BlockEntry {
                kind: CIVIL_DEFENCE.into(),
                guarded: guarded.into(),
                mode: BlockMode::Block,
                override_mode: OverrideMode::Never,
                condition: Some(BlockCondition::new("category", ["fire_safety"])),
                locked: true,
            });
        }
        documents
    }

    /// Every entry of the kinds' `blocks`.
    pub fn all_entries(&self) -> Vec<BlockEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// Adds `entry` to the kinds' `blocks`.
    pub fn block(&self, entry: BlockEntry) {
        self.entries.lock().unwrap().push(entry);
    }

    /// The vendor `subject` has an approval of `kind` valid through `expires`
    /// (`None`: it does not expire).
    pub fn set(&self, subject: &Subject, kind: &str, expires: Option<NaiveDate>) {
        self.expiries
            .lock()
            .unwrap()
            .insert((subject.to_string(), kind.to_owned()), expires);
    }

    /// The vendor `subject` has no approval of `kind`.
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

/// The name of a kind in a refusal.
fn kind_name(kind: &str) -> &str {
    if kind == CIVIL_DEFENCE {
        "Civil Defence approval"
    } else {
        kind
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

    fn check_as<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        record_id: i64,
        on_date: NaiveDate,
        _actor: Option<i64>,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>> {
        Box::pin(async move {
            let mut failures = Vec::new();
            let Some(provider) = block_subjects(guarded) else {
                return Err(Error::Internal(renox::anyhow::anyhow!(
                    "no BlockSubjects provider for {guarded}"
                )));
            };
            for entry in self.entries_of(guarded) {
                for subject in provider.subjects(state, &entry, record_id).await? {
                    let validity = self.validity_of(&subject, &entry.kind, on_date);
                    if validity.is_valid() {
                        continue;
                    }
                    let name = kind_name(&entry.kind);
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
                        "{what}; override: {}",
                        if allowed {
                            "with approval"
                        } else {
                            "not allowed"
                        }
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
