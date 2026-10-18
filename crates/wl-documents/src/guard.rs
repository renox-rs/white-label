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
//!   first, then (when none fails) its `flag` entries, which only collect
//!   [`FlagFailure`]s (#46). It asks the transition's `BlockSubjects`
//!   provider for the subjects, and refuses with one line per failing
//!   subject and kind, e.g. "Residence visa expired on 2026-11-30; renew:
//!   PRO; override: not allowed". A pending renewal it names is moved to the
//!   top of the review queue (`review_priority_at`). An entry whose
//!   transition has no provider is an error, never a *yes*;
//! - [`DocumentGuard::check_planned`] warns, after planned work was saved,
//!   about each document that will not be valid on a planned start date
//!   (#47). It looks at the `block` entries only, never refuses and never
//!   bumps a review; a bookable slot is not warned about but refused with
//!   [`PlannedError::BookableSlot`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockEntry, BlockFailure, BlockMode, BlockRefusal, CheckOutcome, DocumentFuture, DocumentGuard,
    DocumentId, FlagFailure, FlagReason, InvalidReason, OverrideMode, PendingRenewal, PlannedError,
    PlannedItem, PlannedWarning, PlannedWarningLine, RecordFlag, Validity, block_subjects,
    default_subject_label, dubai_date, today,
};

use crate::documents::{Document, DocumentStatus};
use crate::flags;
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

    fn check_as<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        record_id: i64,
        on_date: NaiveDate,
        _actor: Option<i64>,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>> {
        // Overrides are not stored yet: nothing is covered.
        Box::pin(check(state, guarded, record_id, on_date))
    }

    fn check_planned<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        items: &'a [PlannedItem],
    ) -> DocumentFuture<'a, std::result::Result<Option<PlannedWarning>, PlannedError>> {
        Box::pin(check_planned(state, guarded, items))
    }

    fn record_flags<'a>(
        &'a self,
        db: &'a Db,
        guarded: &'a str,
        record_id: i64,
        flags: &'a [FlagFailure],
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        Box::pin(async move {
            let rows = flags::insert(db, guarded, record_id, flags).await?;
            let mut written = Vec::with_capacity(rows.len());
            for row in &rows {
                written.push(flags::to_record_flag(db, row).await?);
            }
            Ok(written)
        })
    }

    fn flags_for<'a>(
        &'a self,
        db: &'a Db,
        record_type: &'a str,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        Box::pin(flags::flags_for(db, record_type, record_id))
    }

    fn open_flag_counts<'a>(
        &'a self,
        db: &'a Db,
        record_type: &'a str,
        ids: &'a [i64],
    ) -> DocumentFuture<'a, BTreeMap<i64, u64>> {
        Box::pin(flags::open_flag_counts(db, record_type, ids))
    }

    fn review_flag<'a>(
        &'a self,
        db: &'a Db,
        flag_id: i64,
        reviewer: i64,
        note: &'a str,
    ) -> DocumentFuture<'a, RecordFlag> {
        Box::pin(flags::review(db, flag_id, reviewer, note))
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

/// One failing document found by [`failing`].
struct Failing {
    entry: BlockEntry,
    kind: DocumentKind,
    subject: Subject,
    validity: Validity,
}

/// The documents of `entries` that are not valid on `on_date`, and the
/// pending renewals they name. An entry whose transition has no provider is
/// an error, never a *yes*.
async fn failing(
    state: &AppState,
    guarded: &str,
    entries: Vec<BlockEntry>,
    record_id: i64,
    on_date: NaiveDate,
) -> Result<(Vec<Failing>, Vec<DocumentId>)> {
    let mut failures = Vec::new();
    let mut prioritised: Vec<DocumentId> = Vec::new();
    if entries.is_empty() {
        return Ok((failures, prioritised));
    }
    let Some(provider) = block_subjects(guarded) else {
        let message = texts::english("wl_documents.guard.no_provider", &[("guarded", &guarded)]);
        return Err(Error::Internal(renox::anyhow::anyhow!(message)));
    };
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
            failures.push(Failing {
                entry: entry.clone(),
                kind: kind.clone(),
                subject,
                validity,
            });
        }
    }
    Ok((failures, prioritised))
}

/// Moves the pending renewals `documents` to the top of the review queue.
async fn prioritise(db: &Db, documents: &[DocumentId]) -> Result<()> {
    if !documents.is_empty() {
        Document::query()
            .where_(Document::ID.is_in(documents.iter().map(|d| d.0)))
            .update(db, &[("review_priority_at", &now())])
            .await?;
    }
    Ok(())
}

/// The checks of [`DocumentGuard::check`]: the `block` entries first, and
/// when any fails the transition is refused with only those failures (block
/// wins, nothing is flagged). Otherwise the `flag` entries are judged on
/// `on_date` and each failure becomes a [`FlagFailure`] of the outcome; a
/// flag never refuses.
pub async fn check(
    state: &AppState,
    guarded: &str,
    record_id: i64,
    on_date: NaiveDate,
) -> Result<std::result::Result<CheckOutcome, BlockRefusal>> {
    let (blocks, flags): (Vec<_>, Vec<_>) = kinds::blocks_for(&state.db, guarded)
        .await?
        .into_iter()
        .partition(|entry| entry.mode == BlockMode::Block);
    let lang = state.current_lang();

    let (failures, prioritised) = failing(state, guarded, blocks, record_id, on_date).await?;
    if !failures.is_empty() {
        prioritise(&state.db, &prioritised).await?;
        let failures = failures
            .into_iter()
            .map(|f| BlockFailure {
                message: failure_message(&lang, &f.kind, &f.entry, &f.validity),
                entry: f.entry,
                subject: f.subject,
                validity: f.validity,
            })
            .collect();
        return Ok(Err(BlockRefusal {
            guarded: guarded.to_owned(),
            record_id,
            failures,
        }));
    }

    let (failures, prioritised) = failing(state, guarded, flags, record_id, on_date).await?;
    prioritise(&state.db, &prioritised).await?;
    let mut outcome = CheckOutcome::default();
    for f in failures {
        let Validity::Invalid {
            reason,
            document,
            expired_on,
            ..
        } = f.validity
        else {
            continue;
        };
        let reason = FlagReason::from(reason);
        let label = flags::label(&lang, reason, f.kind.name(&lang.locale), expired_on);
        outcome.flags.push(FlagFailure {
            entry: f.entry,
            subject: f.subject,
            reason,
            document,
            expired_on,
            on_date,
            label,
        });
    }
    Ok(Ok(outcome))
}

/// The line text of a document that will not be valid on a planned date
/// (A6): "Residence visa expires 2026-12-15" while the expiry date is not
/// past, "Residence visa expired on 2026-11-30" once it is.
pub(crate) fn planned_text(
    lang: &Lang,
    kind_name: &str,
    reason: InvalidReason,
    expires_on: Option<NaiveDate>,
    today: NaiveDate,
) -> String {
    let (key, date) = match (reason, expires_on) {
        (InvalidReason::Expired, Some(date)) if date >= today => {
            ("wl_documents.planned.expires", Some(date))
        }
        (InvalidReason::Expired, Some(date)) => ("wl_documents.planned.expired", Some(date)),
        (InvalidReason::Expired | InvalidReason::Missing, _) => {
            ("wl_documents.planned.missing", None)
        }
        (InvalidReason::Rejected, _) => ("wl_documents.planned.rejected", None),
        (InvalidReason::PendingReview, _) => ("wl_documents.planned.pending_review", None),
    };
    let date = date.map(|d| d.to_string()).unwrap_or_default();
    texts::t_with(lang, key, &[("kind", &kind_name), ("date", &date)])
}

/// What one subject and kind fails on, while the items are checked.
struct PlannedFailure {
    kind_name: String,
    reason: InvalidReason,
    expires_on: Option<NaiveDate>,
    dates: BTreeSet<NaiveDate>,
}

/// The checks of [`DocumentGuard::check_planned`]: a bookable provider is
/// refused; otherwise `is_valid` runs for every item, subject and kind of
/// the `block` entries of `guarded`, and the failures are grouped by
/// subject and kind (ordered by both) with their sorted dates. Flag
/// entries, entries whose condition does not match, and valid documents
/// add nothing. Nothing is written.
pub async fn check_planned(
    state: &AppState,
    guarded: &str,
    items: &[PlannedItem],
) -> Result<std::result::Result<Option<PlannedWarning>, PlannedError>> {
    let provider = block_subjects(guarded);
    if provider.as_ref().is_some_and(|p| p.bookable()) {
        return Ok(Err(PlannedError::BookableSlot {
            guarded: guarded.to_owned(),
        }));
    }
    let blocks: Vec<BlockEntry> = kinds::blocks_for(&state.db, guarded)
        .await?
        .into_iter()
        .filter(|entry| entry.mode == BlockMode::Block)
        .collect();
    if blocks.is_empty() || items.is_empty() {
        return Ok(Ok(None));
    }
    let Some(provider) = provider else {
        let message = texts::english("wl_documents.guard.no_provider", &[("guarded", &guarded)]);
        return Err(Error::Internal(renox::anyhow::anyhow!(message)));
    };
    let lang = state.current_lang();
    let today = today();

    // Keyed by subject, then kind, so the lines come out in that order.
    let mut failures: BTreeMap<(String, i64, String), PlannedFailure> = BTreeMap::new();
    for entry in &blocks {
        let kind = DocumentKind::by_key(&state.db, &entry.kind)
            .await?
            .ok_or(Error::NotFound)?;
        let kind_name = kind.name(&lang.locale);
        for item in items {
            for subject in provider.subjects(state, entry, item.record_id).await? {
                let validity = is_valid(&state.db, &subject, &entry.kind, item.on_date).await?;
                let Validity::Invalid {
                    reason, expired_on, ..
                } = validity
                else {
                    continue;
                };
                let key = (
                    subject.subject_type.clone(),
                    subject.subject_id,
                    entry.kind.clone(),
                );
                let failure = failures.entry(key).or_insert_with(|| PlannedFailure {
                    kind_name: kind_name.to_owned(),
                    reason,
                    expires_on: expired_on,
                    dates: BTreeSet::new(),
                });
                // The reason of the earliest failing date describes the line.
                if failure
                    .dates
                    .first()
                    .is_none_or(|first| item.on_date < *first)
                {
                    failure.reason = reason;
                    failure.expires_on = expired_on;
                }
                failure.dates.insert(item.on_date);
            }
        }
    }
    if failures.is_empty() {
        return Ok(Ok(None));
    }

    let mut labels: BTreeMap<(String, i64), String> = BTreeMap::new();
    let mut lines = Vec::with_capacity(failures.len());
    for ((subject_type, subject_id, kind), failure) in failures {
        let subject = Subject::new(&subject_type, subject_id);
        let label = match labels.get(&(subject_type.clone(), subject_id)) {
            Some(label) => label.clone(),
            None => {
                let label = provider
                    .label(state, &subject)
                    .await?
                    .unwrap_or_else(|| default_subject_label(&subject));
                labels.insert((subject_type, subject_id), label.clone());
                label
            }
        };
        lines.push(PlannedWarningLine {
            subject,
            label,
            kind,
            text: planned_text(
                &lang,
                &failure.kind_name,
                failure.reason,
                failure.expires_on,
                today,
            ),
            kind_name: failure.kind_name,
            reason: failure.reason,
            expires_on: failure.expires_on,
            dates: failure.dates.into_iter().collect(),
        });
    }
    Ok(Ok(Some(PlannedWarning {
        guarded: guarded.to_owned(),
        lines,
    })))
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

        // A flag entry without a provider is never a yes either.
        assert!(
            guard
                .check(state, "guard_unit.flagged", 7, date("2026-12-01"))
                .await
                .is_err()
        );
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

    async fn save_document(
        db: &Db,
        kind: &str,
        subject_id: i64,
        status: DocumentStatus,
        expiry: Option<&str>,
    ) -> Document {
        let kind = DocumentKind::by_key(db, kind).await.unwrap().unwrap();
        let mut document = Document {
            subject_type: "hr.employee".into(),
            subject_id,
            kind_id: kind.id,
            title: kind.name_en.clone(),
            status,
            accepted_at: Some(at("2025-12-01T08:00:00Z")),
            submitted_at: Some(at("2025-12-01T08:00:00Z")),
            expiry_date: expiry.map(date),
            ..Document::default()
        };
        document.save(db).await.unwrap();
        document
    }

    #[renox::test]
    async fn flag_mode_collects_flags_and_never_refuses() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let state = app.state();
        let db = app.db();
        install_seeds(
            db,
            [&KindSeed::new("flag_unit_permit", "Work permit")
                .subject_type("hr.employee")
                .expires()
                .blocks("flag_unit.accept", BlockMode::Flag)],
        )
        .await
        .unwrap();
        register_block_subjects("flag_unit.accept", Worker);
        save_document(
            db,
            "flag_unit_permit",
            7,
            DocumentStatus::Accepted,
            Some("2026-11-30"),
        )
        .await;
        let guard = wl_kernel::documents::guard(state);

        // Valid on the expiry date: no flag.
        let outcome = guard
            .check(state, "flag_unit.accept", 7, date("2026-11-30"))
            .await
            .unwrap()
            .unwrap();
        assert!(outcome.flags.is_empty());

        // Expired the day after: allowed, with a flag.
        let outcome = guard
            .check(state, "flag_unit.accept", 7, date("2026-12-02"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.flags.len(), 1);
        let failure = &outcome.flags[0];
        assert_eq!(failure.label, "Work permit expired 2026-11-30");
        assert_eq!(failure.reason, FlagReason::Expired);
        assert_eq!(failure.subject, Subject::new("hr.employee", 7));
        assert_eq!(failure.expired_on, Some(date("2026-11-30")));
        assert_eq!(failure.on_date, date("2026-12-02"));

        // A worker without any document is flagged as missing.
        let outcome = guard
            .check(state, "flag_unit.accept", 8, date("2026-12-02"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.flags[0].label, "Work permit missing");
    }

    #[renox::test]
    async fn block_wins_over_flag_and_a_pending_renewal_is_bumped() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let state = app.state();
        let db = app.db();
        install_seeds(
            db,
            [
                &KindSeed::new("mix_unit_permit", "Work permit")
                    .subject_type("hr.employee")
                    .expires()
                    .blocks("mix_unit.accept", BlockMode::Flag),
                &KindSeed::new("mix_unit_visa", "Residence visa")
                    .subject_type("hr.employee")
                    .expires()
                    .blocks("mix_unit.accept", BlockMode::Block),
            ],
        )
        .await
        .unwrap();
        register_block_subjects("mix_unit.accept", Worker);
        let renewal = save_document(
            db,
            "mix_unit_permit",
            7,
            DocumentStatus::PendingReview,
            Some("2028-11-30"),
        )
        .await;
        let guard = wl_kernel::documents::guard(state);

        // Only the flag entry fails: allowed, and its renewal is bumped.
        save_document(
            db,
            "mix_unit_visa",
            7,
            DocumentStatus::Accepted,
            Some("2028-01-01"),
        )
        .await;
        let outcome = guard
            .check(state, "mix_unit.accept", 7, date("2026-12-02"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.flags.len(), 1);
        assert_eq!(outcome.flags[0].reason, FlagReason::PendingReview);
        assert_eq!(outcome.flags[0].label, "Work permit renewal pending review");
        let bumped = Document::find(db, renewal.id).await.unwrap().unwrap();
        assert!(bumped.review_priority_at.is_some());

        // Worker 9 has no visa (block) and no permit (flag): refused with
        // the block failure only.
        let refusal = guard
            .check(state, "mix_unit.accept", 9, date("2026-12-02"))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(refusal.failures.len(), 1);
        assert_eq!(refusal.failures[0].entry.kind, "mix_unit_visa");
        assert!(!refusal.message().contains("Work permit"));
    }

    #[renox::test]
    async fn the_guard_records_reviews_and_counts_flags() {
        let app = TestApp::new(App::new().module(Auth::new()).module(crate::WlDocuments)).await;
        let state = app.state();
        let db = app.db();
        install_seeds(
            db,
            [&KindSeed::new("rec_unit_permit", "Work permit")
                .subject_type("hr.employee")
                .expires()
                .blocks("rec_unit.accept", BlockMode::Flag)],
        )
        .await
        .unwrap();
        register_block_subjects("rec_unit.accept", Worker);
        save_document(
            db,
            "rec_unit_permit",
            7,
            DocumentStatus::Accepted,
            Some("2026-11-30"),
        )
        .await;
        let guard = wl_kernel::documents::guard(state);
        let outcome = guard
            .check(state, "rec_unit.accept", 7, date("2026-12-02"))
            .await
            .unwrap()
            .unwrap();

        let written = guard
            .record_flags(db, "rec_unit.accept", 7, &outcome.flags)
            .await
            .unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].label, "Work permit expired 2026-11-30");
        // Again while unreviewed: no second row.
        let again = guard
            .record_flags(db, "rec_unit.accept", 7, &outcome.flags)
            .await
            .unwrap();
        assert!(again.is_empty());

        assert_eq!(guard.flags_for(db, "rec_unit", 7).await.unwrap().len(), 1);
        assert!(guard.flags_for(db, "rec_unit", 8).await.unwrap().is_empty());
        let counts = guard
            .open_flag_counts(db, "rec_unit", &[7, 8])
            .await
            .unwrap();
        assert_eq!(counts.get(&7), Some(&1));
        assert_eq!(counts.get(&8), Some(&0));

        // A note is required; then it is reviewed, once.
        assert!(guard.review_flag(db, written[0].id, 1, " ").await.is_err());
        let reviewed = guard
            .review_flag(db, written[0].id, 1, "Permit renewal in progress")
            .await
            .unwrap();
        assert_eq!(reviewed.label, "Work permit expired 2026-11-30");
        assert_eq!(reviewed.reviewed_by, Some(1));
        assert_eq!(
            reviewed.review_note.as_deref(),
            Some("Permit renewal in progress")
        );
        assert!(
            guard
                .review_flag(db, written[0].id, 1, "again")
                .await
                .is_err()
        );
        let counts = guard.open_flag_counts(db, "rec_unit", &[7]).await.unwrap();
        assert_eq!(counts.get(&7), Some(&0));
    }

    struct Crew;

    impl BlockSubjects for Crew {
        fn subjects<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a BlockEntry,
            record_id: i64,
        ) -> DocumentFuture<'a, Vec<Subject>> {
            Box::pin(async move {
                // Record 99 does not match the entry's condition.
                Ok(if record_id == 99 {
                    Vec::new()
                } else {
                    vec![Subject::new("hr.employee", 7)]
                })
            })
        }

        fn label<'a>(
            &'a self,
            _state: &'a AppState,
            subject: &'a Subject,
        ) -> DocumentFuture<'a, Option<String>> {
            Box::pin(async move { Ok((subject.subject_id == 7).then(|| "Ravi S.".to_owned())) })
        }
    }

    struct Bookable;

    impl BlockSubjects for Bookable {
        fn subjects<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a BlockEntry,
            record_id: i64,
        ) -> DocumentFuture<'a, Vec<Subject>> {
            Box::pin(async move { Ok(vec![Subject::new("hr.employee", record_id)]) })
        }

        fn bookable(&self) -> bool {
            true
        }
    }

    #[renox::test]
    async fn the_planned_text_follows_the_reason_and_the_expiry_date() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let lang = app.state().lang("en");
        let today = date("2026-11-01");
        let text = |reason, expires| planned_text(&lang, "Residence visa", reason, expires, today);
        assert_eq!(
            text(InvalidReason::Expired, Some(date("2026-12-15"))),
            "Residence visa expires 2026-12-15"
        );
        assert_eq!(
            text(InvalidReason::Expired, Some(date("2026-10-31"))),
            "Residence visa expired on 2026-10-31"
        );
        assert_eq!(text(InvalidReason::Missing, None), "Residence visa missing");
        assert_eq!(
            text(InvalidReason::Rejected, None),
            "Residence visa rejected"
        );
        assert_eq!(
            text(InvalidReason::PendingReview, None),
            "Residence visa renewal pending review"
        );
    }

    #[renox::test]
    async fn planned_work_past_the_expiry_gives_one_grouped_warning() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let state = app.state();
        let db = app.db();
        install_seeds(
            db,
            [
                &KindSeed::new("plan_unit_visa", "Residence visa")
                    .subject_type("hr.employee")
                    .expires()
                    .blocks("plan_unit.activate", BlockMode::Block),
                &KindSeed::new("plan_unit_permit", "Work permit")
                    .subject_type("hr.employee")
                    .expires()
                    .blocks("plan_unit.activate", BlockMode::Block),
                &KindSeed::new("plan_unit_flag", "Medical card")
                    .subject_type("hr.employee")
                    .blocks("plan_unit.activate", BlockMode::Flag),
            ],
        )
        .await
        .unwrap();
        register_block_subjects("plan_unit.activate", Crew);
        save_document(
            db,
            "plan_unit_visa",
            7,
            DocumentStatus::Accepted,
            Some("2099-12-15"),
        )
        .await;
        let guard = wl_kernel::documents::guard(state);
        let item = |id, on| PlannedItem::new(id, date(on));

        // Valid through the expiry date; only the missing permit warns (the
        // flag-mode medical card never counts).
        let warning = guard
            .check_planned(state, "plan_unit.activate", &[item(1, "2099-12-15")])
            .await
            .unwrap()
            .unwrap()
            .expect("a warning");
        assert_eq!(warning.guarded, "plan_unit.activate");
        assert_eq!(warning.lines.len(), 1);
        assert_eq!(warning.lines[0].text, "Work permit missing");
        assert_eq!(warning.lines[0].label, "Ravi S.");

        // Past the expiry the visa joins, grouped with sorted, unique dates.
        let items = [
            item(3, "2100-01-18"),
            item(1, "2099-12-15"),
            item(2, "2100-01-04"),
            item(4, "2100-01-04"),
            item(99, "2100-02-01"),
        ];
        let warning = guard
            .check_planned(state, "plan_unit.activate", &items)
            .await
            .unwrap()
            .unwrap()
            .expect("a warning");
        assert_eq!(warning.lines.len(), 2);
        let (permit, visa) = (&warning.lines[0], &warning.lines[1]);
        assert_eq!(permit.kind, "plan_unit_permit");
        assert_eq!(visa.kind, "plan_unit_visa");
        assert_eq!(visa.text, "Residence visa expires 2099-12-15");
        assert_eq!(visa.expires_on, Some(date("2099-12-15")));
        assert_eq!(visa.dates, [date("2100-01-04"), date("2100-01-18")]);
        assert_eq!(permit.dates.len(), 3);
        assert!(warning.message().contains(
            "Ravi S.: Residence visa expires 2099-12-15; planned: 2100-01-04, 2100-01-18"
        ));

        // Nothing planned, nothing guarded, or a record the entry does not
        // match: no warning.
        for (guarded, items) in [
            ("plan_unit.activate", vec![]),
            ("plan_unit.unguarded", vec![item(1, "2100-01-04")]),
            ("plan_unit.activate", vec![item(99, "2100-01-04")]),
        ] {
            assert!(
                guard
                    .check_planned(state, guarded, &items)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[renox::test]
    async fn a_bookable_slot_is_refused_never_warned_about() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let state = app.state();
        install_seeds(
            app.db(),
            [&KindSeed::new("plan_book_licence", "Licence")
                .subject_type("hr.employee")
                .expires()
                .blocks("plan_book.book", BlockMode::Block)],
        )
        .await
        .unwrap();
        register_block_subjects("plan_book.book", Bookable);
        let guard = wl_kernel::documents::guard(state);
        let error = guard
            .check_planned(
                state,
                "plan_book.book",
                &[PlannedItem::new(5, date("2100-01-04"))],
            )
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error,
            PlannedError::BookableSlot {
                guarded: "plan_book.book".into()
            }
        );
        // The answer the booking refusal rests on is still a no.
        assert!(
            !is_valid(
                &state.db,
                &Subject::new("hr.employee", 5),
                "plan_book_licence",
                date("2100-01-04")
            )
            .await
            .unwrap()
            .is_valid()
        );
    }
}
