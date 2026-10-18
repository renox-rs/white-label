//! The standard document guard (wl-documents §5.6, §6): this module's
//! [`DocumentGuard`], which [`crate::WlDocuments`] provides in `register`.
//!
//! - [`DocumentGuard::blocks_for`] lists the entries that guard a
//!   transition ([`crate::kinds::blocks_for`]);
//! - [`DocumentGuard::is_valid`] is `document_valid(subject, kind,
//!   on_date)`. On [`today`] or a later date only the current `accepted`
//!   document counts; on a past date an `accepted` or `superseded` document
//!   counts when it was accepted on or before that date. Either way it must
//!   expire on or after the date, or not expire. A missing, `rejected` or
//!   `pending_review` document answers *no*. Nothing is cached;
//! - [`DocumentGuard::check`] runs the `block` entries of a transition
//!   (`flag` entries come with #46), asks the transition's `BlockSubjects`
//!   provider for the subjects, and refuses with one line per failing
//!   subject and kind, e.g. "Residence visa expired on 2026-11-30; renew:
//!   PRO; override: not allowed". A pending renewal it names is moved to the
//!   top of the review queue (`review_priority_at`). An entry whose
//!   transition has no provider is an error, never a *yes*.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockEntry, BlockFailure, BlockMode, BlockRefusal, CheckOutcome, DocumentFuture, DocumentGuard,
    DocumentId, FlagFailure, InvalidReason, OverrideMode, PendingRenewal, RecordFlag, Validity,
    block_subjects, dubai_date, today,
};

use crate::documents::{Document, DocumentStatus};
use crate::kinds::{self, DocumentKind};
use crate::texts;

/// The guard of the documents module.
#[derive(Debug, Clone, Copy, Default)]
pub struct DocumentsGuard;

impl DocumentGuard for DocumentsGuard {
    fn blocks_for<'a>(
        &'a self,
        db: &'a Db,
        guarded: &'a str,
    ) -> DocumentFuture<'a, Vec<BlockEntry>> {
        Box::pin(kinds::blocks_for(db, guarded))
    }

    fn is_valid<'a>(
        &'a self,
        db: &'a Db,
        subject: &'a Subject,
        kind: &'a str,
        on_date: NaiveDate,
    ) -> DocumentFuture<'a, Validity> {
        Box::pin(is_valid(db, subject, kind, on_date))
    }

    fn check<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        record_id: i64,
        on_date: NaiveDate,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>> {
        Box::pin(check(state, guarded, record_id, on_date))
    }

    // Flag mode (#46): the flags table and the real methods come with
    // parts 2 and 3. Until then nothing is flagged, as in `NoDocuments`.
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

/// `document_valid(subject, kind, on_date)`; an unknown kind is missing.
pub async fn is_valid(
    db: &Db,
    subject: &Subject,
    kind: &str,
    on_date: NaiveDate,
) -> Result<Validity> {
    let Some(kind) = DocumentKind::by_key(db, kind).await? else {
        return Ok(validity_of(&[], on_date, today()));
    };
    let documents = Document::query()
        .where_(Document::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(Document::SUBJECT_ID.eq(subject.subject_id))
        .where_(Document::KIND_ID.eq(kind.id))
        .get(db)
        .await?;
    Ok(validity_of(&documents, on_date, today()))
}

/// The Asia/Dubai date `document` was accepted on; `None` when unknown.
fn accepted_on(document: &Document) -> Option<NaiveDate> {
    document
        .accepted_at
        .or(document.submitted_at)
        .or(document.created_at)
        .map(dubai_date)
}

/// The Asia/Dubai date `document` was submitted on.
fn submitted_on(document: &Document, today: NaiveDate) -> NaiveDate {
    document
        .submitted_at
        .or(document.created_at)
        .map_or(today, dubai_date)
}

/// Whether `document` does not expire before `on_date` (valid through its
/// expiry date).
fn unexpired_on(document: &Document, on_date: NaiveDate) -> bool {
    document.expiry_date.is_none_or(|expiry| expiry >= on_date)
}

/// The A6 date rules over the documents of one kind and subject, with
/// `today` the present date in Asia/Dubai.
pub(crate) fn validity_of(
    documents: &[Document],
    on_date: NaiveDate,
    today: NaiveDate,
) -> Validity {
    let past = on_date < today;
    // The documents that count on `on_date`, whether expired or not.
    let mut counted: Vec<&Document> = documents
        .iter()
        .filter(|d| match d.status {
            DocumentStatus::Accepted if !past => true,
            DocumentStatus::Accepted | DocumentStatus::Superseded if past => {
                accepted_on(d).is_none_or(|accepted| accepted <= on_date)
            }
            _ => false,
        })
        .collect();
    // The latest accepted first.
    counted.sort_by_key(|d| std::cmp::Reverse((d.accepted_at, d.id)));

    if let Some(valid) = counted.iter().find(|d| unexpired_on(d, on_date)) {
        return Validity::Valid {
            document: Some(valid.document_id()),
            expires_on: valid.expiry_date,
        };
    }

    let pending = documents
        .iter()
        .filter(|d| d.status == DocumentStatus::PendingReview)
        .max_by_key(|d| (d.submitted_at, d.id));
    let renewal = pending.map(|d| PendingRenewal {
        document: d.document_id(),
        submitted_on: submitted_on(d, today),
    });

    if let Some(expired) = counted.first() {
        return Validity::Invalid {
            reason: InvalidReason::Expired,
            document: Some(expired.document_id()),
            expired_on: expired.expiry_date,
            pending: renewal,
        };
    }
    if let Some(pending) = pending {
        return Validity::Invalid {
            reason: InvalidReason::PendingReview,
            document: Some(pending.document_id()),
            expired_on: None,
            pending: renewal,
        };
    }
    let rejected = documents
        .iter()
        .filter(|d| d.status == DocumentStatus::Rejected)
        .max_by_key(|d| d.id);
    match rejected {
        Some(rejected) => Validity::Invalid {
            reason: InvalidReason::Rejected,
            document: Some(rejected.document_id()),
            expired_on: None,
            pending: None,
        },
        None => Validity::Invalid {
            reason: InvalidReason::Missing,
            document: None,
            expired_on: None,
            pending: None,
        },
    }
}

/// The checks of [`DocumentGuard::check`].
pub async fn check(
    state: &AppState,
    guarded: &str,
    record_id: i64,
    on_date: NaiveDate,
) -> Result<std::result::Result<CheckOutcome, BlockRefusal>> {
    let entries: Vec<BlockEntry> = kinds::blocks_for(&state.db, guarded)
        .await?
        .into_iter()
        // Flag mode comes with #46.
        .filter(|entry| entry.mode == BlockMode::Block)
        .collect();
    if entries.is_empty() {
        return Ok(Ok(CheckOutcome::default()));
    }
    let Some(provider) = block_subjects(guarded) else {
        let message = texts::english("wl_documents.guard.no_provider", &[("guarded", &guarded)]);
        return Err(Error::Internal(renox::anyhow::anyhow!(message)));
    };
    let lang = state.current_lang();
    let mut failures = Vec::new();
    let mut prioritised: Vec<DocumentId> = Vec::new();
    for entry in entries {
        let subjects = provider.subjects(state, &entry, record_id).await?;
        if subjects.is_empty() {
            continue;
        }
        let kind = DocumentKind::by_key(&state.db, &entry.kind)
            .await?
            .ok_or(Error::NotFound)?;
        for subject in subjects {
            let validity = is_valid(&state.db, &subject, &entry.kind, on_date).await?;
            let Validity::Invalid { pending, .. } = &validity else {
                continue;
            };
            if let Some(renewal) = pending
                && !prioritised.contains(&renewal.document)
            {
                prioritised.push(renewal.document);
            }
            let message = failure_message(&lang, &kind, &entry, &validity);
            failures.push(BlockFailure {
                entry: entry.clone(),
                subject,
                validity,
                message,
            });
        }
    }
    if failures.is_empty() {
        return Ok(Ok(CheckOutcome::default()));
    }
    if !prioritised.is_empty() {
        Document::query()
            .where_(Document::ID.is_in(prioritised.iter().map(|d| d.0)))
            .update(&state.db, &[("review_priority_at", &now())])
            .await?;
    }
    Ok(Err(BlockRefusal {
        guarded: guarded.to_owned(),
        record_id,
        failures,
    }))
}

/// The label of role `key`: the text `roles.<key>`, else the key itself.
fn role_label(lang: &Lang, key: &str) -> String {
    let text_key = format!("roles.{key}");
    let label = lang.t(&text_key, &[]);
    if label == text_key {
        key.to_owned()
    } else {
        label
    }
}

/// The line of one failing document (A5): what is wrong, who renews it,
/// whether an override is allowed and, when there is one, the pending
/// renewal.
pub(crate) fn failure_message(
    lang: &Lang,
    kind: &DocumentKind,
    entry: &BlockEntry,
    validity: &Validity,
) -> String {
    let Validity::Invalid {
        reason,
        expired_on,
        pending,
        ..
    } = validity
    else {
        return String::new();
    };
    let name = kind.name(&lang.locale);
    let mut parts = Vec::with_capacity(4);
    parts.push(match (reason, expired_on) {
        (InvalidReason::Expired, Some(date)) => texts::t_with(
            lang,
            "wl_documents.guard.expired",
            &[("kind", &name), ("date", date as &dyn Display)],
        ),
        (InvalidReason::Expired | InvalidReason::Missing, _) => {
            texts::t_with(lang, "wl_documents.guard.missing", &[("kind", &name)])
        }
        (InvalidReason::Rejected, _) => {
            texts::t_with(lang, "wl_documents.guard.rejected", &[("kind", &name)])
        }
        (InvalidReason::PendingReview, _) => texts::t_with(
            lang,
            "wl_documents.guard.pending_review",
            &[("kind", &name)],
        ),
    });
    if let Some(role) = kind.responsible_role.as_deref().filter(|r| !r.is_empty()) {
        let role = role_label(lang, role);
        parts.push(texts::t_with(
            lang,
            "wl_documents.guard.renew",
            &[("role", &role)],
        ));
    }
    parts.push(texts::t(
        lang,
        match entry.override_mode {
            OverrideMode::Never => "wl_documents.guard.override_never",
            OverrideMode::WithApproval => "wl_documents.guard.override_with_approval",
        },
    ));
    if let Some(renewal) = pending {
        parts.push(texts::t_with(
            lang,
            "wl_documents.guard.renewal_pending",
            &[("date", &renewal.submitted_on)],
        ));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use renox::testing::TestApp;
    use wl_kernel::documents::{BlockSubjects, register_block_subjects};

    use super::*;
    use crate::kinds::{KindSeed, install_seeds};

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn at(text: &str) -> DateTime {
        text.parse().expect("an RFC 3339 instant")
    }

    fn document(id: i64, status: DocumentStatus, accepted: &str, expiry: Option<&str>) -> Document {
        Document {
            id,
            status,
            accepted_at: Some(at(accepted)),
            submitted_at: Some(at(accepted)),
            expiry_date: expiry.map(date),
            ..Document::default()
        }
    }

    fn visa_kind() -> DocumentKind {
        DocumentKind {
            key: "residence_visa".into(),
            name_en: "Residence visa".into(),
            responsible_role: Some("PRO".into()),
            ..DocumentKind::default()
        }
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
    fn a_document_is_valid_through_its_expiry_date() {
        let today = date("2026-10-18");
        let visa = [document(
            1,
            DocumentStatus::Accepted,
            "2025-12-01T08:00:00Z",
            Some("2026-11-30"),
        )];
        assert!(validity_of(&visa, date("2026-11-30"), today).is_valid());
        assert_eq!(
            validity_of(&visa, date("2026-12-01"), today),
            Validity::Invalid {
                reason: InvalidReason::Expired,
                document: Some(DocumentId(1)),
                expired_on: Some(date("2026-11-30")),
                pending: None,
            }
        );
        let open = [document(
            2,
            DocumentStatus::Accepted,
            "2025-12-01T08:00:00Z",
            None,
        )];
        assert!(validity_of(&open, date("2030-01-01"), today).is_valid());
    }

    #[test]
    fn a_past_date_counts_the_superseded_document_valid_then() {
        let today = date("2026-12-05");
        let documents = [
            document(
                1,
                DocumentStatus::Superseded,
                "2025-12-01T08:00:00Z",
                Some("2026-11-30"),
            ),
            document(
                2,
                DocumentStatus::Accepted,
                "2026-12-02T08:00:00Z",
                Some("2028-11-30"),
            ),
        ];
        assert_eq!(
            validity_of(&documents, date("2026-11-15"), today),
            Validity::Valid {
                document: Some(DocumentId(1)),
                expires_on: Some(date("2026-11-30")),
            }
        );
        // Between the old visa's expiry and the renewal's acceptance.
        assert_eq!(
            validity_of(&documents, date("2026-12-01"), today),
            Validity::Invalid {
                reason: InvalidReason::Expired,
                document: Some(DocumentId(1)),
                expired_on: Some(date("2026-11-30")),
                pending: None,
            }
        );
        // From today on, only the current accepted one counts.
        assert!(validity_of(&documents, today, today).is_valid());
    }

    #[test]
    fn missing_rejected_and_pending_answer_no() {
        let today = date("2026-10-18");
        let on = date("2026-12-01");
        assert!(matches!(
            validity_of(&[], on, today),
            Validity::Invalid {
                reason: InvalidReason::Missing,
                document: None,
                ..
            }
        ));
        let rejected = [document(
            3,
            DocumentStatus::Rejected,
            "2026-10-01T08:00:00Z",
            Some("2027-11-30"),
        )];
        assert!(matches!(
            validity_of(&rejected, on, today),
            Validity::Invalid {
                reason: InvalidReason::Rejected,
                ..
            }
        ));
        let pending = [document(
            4,
            DocumentStatus::PendingReview,
            "2026-10-01T08:00:00Z",
            Some("2027-11-30"),
        )];
        assert_eq!(
            validity_of(&pending, on, today),
            Validity::Invalid {
                reason: InvalidReason::PendingReview,
                document: Some(DocumentId(4)),
                expired_on: None,
                pending: Some(PendingRenewal {
                    document: DocumentId(4),
                    submitted_on: date("2026-10-01"),
                }),
            }
        );
    }

    #[renox::test]
    async fn the_message_names_the_document_the_role_and_the_override() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let lang = app.state().lang("en");
        let kind = visa_kind();
        let expired = Validity::Invalid {
            reason: InvalidReason::Expired,
            document: Some(DocumentId(1)),
            expired_on: Some(date("2026-11-30")),
            pending: None,
        };
        assert_eq!(
            failure_message(&lang, &kind, &visa_entry(), &expired),
            "Residence visa expired on 2026-11-30; renew: PRO; override: not allowed"
        );
        let missing = Validity::Invalid {
            reason: InvalidReason::Missing,
            document: None,
            expired_on: None,
            pending: None,
        };
        let entry = BlockEntry {
            override_mode: OverrideMode::WithApproval,
            ..visa_entry()
        };
        assert_eq!(
            failure_message(&lang, &kind, &entry, &missing),
            "Residence visa missing; renew: PRO; override: with approval"
        );
        let renewing = Validity::Invalid {
            reason: InvalidReason::Expired,
            document: Some(DocumentId(1)),
            expired_on: Some(date("2026-11-30")),
            pending: Some(PendingRenewal {
                document: DocumentId(2),
                submitted_on: date("2026-11-28"),
            }),
        };
        assert_eq!(
            failure_message(&lang, &kind, &visa_entry(), &renewing),
            "Residence visa expired on 2026-11-30; renew: PRO; override: not allowed; \
             renewal pending review since 2026-11-28"
        );
    }

    struct Worker;

    impl BlockSubjects for Worker {
        fn subjects<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a BlockEntry,
            record_id: i64,
        ) -> DocumentFuture<'a, Vec<Subject>> {
            Box::pin(async move { Ok(vec![Subject::new("hr.employee", record_id)]) })
        }
    }

    fn guarded_visa(guarded: &str) -> KindSeed {
        KindSeed::new("guard_unit_visa", "Residence visa")
            .subject_type("hr.employee")
            .expires()
            .responsible_role("PRO")
            .legal_block()
            .blocks(guarded, BlockMode::Block)
    }

    #[renox::test]
    async fn a_missing_document_refuses_and_a_missing_provider_fails() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let state = app.state();
        install_seeds(
            app.db(),
            [
                &guarded_visa("guard_unit.activate"),
                &KindSeed::new("guard_unit_flag", "Permit")
                    .subject_type("hr.employee")
                    .blocks("guard_unit.flagged", BlockMode::Flag),
                &KindSeed::new("guard_unit_orphan", "Orphan")
                    .subject_type("hr.employee")
                    .blocks("guard_unit.orphan", BlockMode::Block),
            ],
        )
        .await
        .unwrap();
        register_block_subjects("guard_unit.activate", Worker);

        let guard = wl_kernel::documents::guard(state);
        let refusal = guard
            .check(state, "guard_unit.activate", 7, date("2026-12-01"))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            refusal.message(),
            "Residence visa missing; renew: PRO; override: not allowed"
        );
        assert_eq!(refusal.failures.len(), 1);
        assert_eq!(refusal.failures[0].subject, Subject::new("hr.employee", 7));

        // Flag entries are skipped (#46), so nothing needs a provider.
        let outcome = guard
            .check(state, "guard_unit.flagged", 7, date("2026-12-01"))
            .await
            .unwrap();
        assert_eq!(outcome, Ok(CheckOutcome::default()));
        // Nothing guards it.
        assert!(
            guard
                .check(state, "guard_unit.unguarded", 7, date("2026-12-01"))
                .await
                .unwrap()
                .is_ok()
        );
        // A block entry without a provider is never a yes.
        assert!(
            guard
                .check(state, "guard_unit.orphan", 7, date("2026-12-01"))
                .await
                .is_err()
        );
    }
}
