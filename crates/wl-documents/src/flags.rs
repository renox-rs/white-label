//! Flags (wl-documents §3, #46): what a guarded transition in `flag` mode
//! leaves on its record when a document is not valid.
//!
//! A [`DocumentFlag`] is permanent: it is reviewed with a note but never
//! removed, and a later renewal leaves it unchanged. [`insert`] writes the
//! flags of a transition that ran (a repeat of an unreviewed flag adds no
//! row), [`flags_for`] and [`open_flag_counts`] read them, and [`review`]
//! marks one reviewed.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::{DocumentId, FlagFailure, FlagReason, RecordFlag};

use crate::texts;

/// One flag on a record.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_documents_flags",
    index(record_type, record_id),
    index(reviewed_at)
)]
pub struct DocumentFlag {
    /// The primary key.
    pub id: i64,
    /// The flagged record's type: the entity part of `guarded`.
    pub record_type: String,
    /// The flagged record's id.
    pub record_id: i64,
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// The document kind's key.
    pub kind: String,
    /// The document subject that failed: the type.
    pub subject_type: String,
    /// The document subject that failed: the id.
    pub subject_id: i64,
    /// The expired, rejected or pending document; `None` when missing.
    pub document_id: Option<i64>,
    /// Why the document was not valid.
    pub reason: FlagReason,
    /// The expiry date of an expired document.
    pub expires_on: Option<NaiveDate>,
    /// The business date it was judged on.
    pub on_date: NaiveDate,
    /// The label, e.g. "Work permit expired 2026-11-30".
    pub label: String,
    /// When it was reviewed; `None` while open.
    pub reviewed_at: Option<DateTime>,
    /// The reviewer's user id.
    pub reviewed_by: Option<i64>,
    /// The reviewer's note.
    pub review_note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DocumentFlag {
    /// Whether a reviewer has marked it reviewed.
    pub fn is_reviewed(&self) -> bool {
        self.reviewed_at.is_some()
    }

    /// The flag in the kernel's shape; `reviewer_name` is the reviewer's name.
    pub fn to_record_flag(&self, reviewer_name: Option<String>) -> RecordFlag {
        RecordFlag {
            id: self.id,
            record_type: self.record_type.clone(),
            record_id: self.record_id,
            guarded: self.guarded.clone(),
            kind: self.kind.clone(),
            subject: Subject::new(&self.subject_type, self.subject_id),
            document: self.document_id.map(DocumentId),
            reason: self.reason,
            expires_on: self.expires_on,
            on_date: self.on_date,
            label: self.label.clone(),
            created_at: self.created_at.unwrap_or_else(now),
            reviewed_at: self.reviewed_at,
            reviewed_by: self.reviewed_by,
            reviewer_name,
            review_note: self.review_note.clone(),
        }
    }
}

/// The label of a flag: `<Kind name> expired <YYYY-MM-DD>`, `<Kind name>
/// missing`, `<Kind name> rejected` or `<Kind name> renewal pending review`.
pub fn label(
    lang: &Lang,
    reason: FlagReason,
    kind_name: &str,
    expired_on: Option<NaiveDate>,
) -> String {
    let key = match reason {
        FlagReason::Expired => "wl_documents.flag.expired",
        FlagReason::Missing => "wl_documents.flag.missing",
        FlagReason::Rejected => "wl_documents.flag.rejected",
        FlagReason::PendingReview => "wl_documents.flag.pending_review",
    };
    let date = expired_on.map(|d| d.to_string()).unwrap_or_default();
    texts::t_with(lang, key, &[("kind", &kind_name), ("date", &date)])
}

/// Writes `failures` on record `record_id` of `guarded`. A failure that
/// repeats an unreviewed flag (same entry, kind, subject and reason) adds
/// no row. Returns the rows written.
pub async fn insert(
    db: &Db,
    guarded: &str,
    record_id: i64,
    failures: &[FlagFailure],
) -> Result<Vec<DocumentFlag>> {
    let record_type = guarded
        .split_once('.')
        .map_or(guarded, |(entity, _)| entity);
    let existing = DocumentFlag::query()
        .where_(DocumentFlag::RECORD_TYPE.eq(record_type))
        .where_(DocumentFlag::RECORD_ID.eq(record_id))
        .where_(DocumentFlag::GUARDED.eq(guarded))
        .get(db)
        .await?;
    let mut written: Vec<DocumentFlag> = Vec::new();
    for failure in failures {
        let repeats = |flag: &DocumentFlag| {
            !flag.is_reviewed()
                && flag.kind == failure.entry.kind
                && flag.subject_type == failure.subject.subject_type
                && flag.subject_id == failure.subject.subject_id
                && flag.reason == failure.reason
        };
        if existing.iter().chain(written.iter()).any(repeats) {
            continue;
        }
        let mut flag = DocumentFlag {
            record_type: record_type.to_owned(),
            record_id,
            guarded: guarded.to_owned(),
            kind: failure.entry.kind.clone(),
            subject_type: failure.subject.subject_type.clone(),
            subject_id: failure.subject.subject_id,
            document_id: failure.document.map(|d| d.0),
            reason: failure.reason,
            expires_on: failure.expired_on,
            on_date: failure.on_date,
            label: failure.label.clone(),
            ..DocumentFlag::default()
        };
        flag.save(db).await?;
        written.push(flag);
    }
    Ok(written)
}

/// `flag` in the kernel's shape, with its reviewer's name (`#<id>` when the
/// user is gone).
pub async fn to_record_flag(db: &Db, flag: &DocumentFlag) -> Result<RecordFlag> {
    let name = match flag.reviewed_by {
        Some(id) => Some(
            User::find(db, id)
                .await?
                .map_or_else(|| format!("#{id}"), |user| user.name),
        ),
        None => None,
    };
    Ok(flag.to_record_flag(name))
}

/// Every flag of a record, reviewed or not, oldest first.
pub async fn flags_for(db: &Db, record_type: &str, record_id: i64) -> Result<Vec<RecordFlag>> {
    let rows = DocumentFlag::query()
        .where_(DocumentFlag::RECORD_TYPE.eq(record_type))
        .where_(DocumentFlag::RECORD_ID.eq(record_id))
        .order_by(DocumentFlag::ID)
        .get(db)
        .await?;
    let mut flags = Vec::with_capacity(rows.len());
    for row in &rows {
        flags.push(to_record_flag(db, row).await?);
    }
    Ok(flags)
}

/// The number of unreviewed flags per record id; every id in `ids` has an
/// entry (0 when it has none).
pub async fn open_flag_counts(
    db: &Db,
    record_type: &str,
    ids: &[i64],
) -> Result<BTreeMap<i64, u64>> {
    let mut counts: BTreeMap<i64, u64> = ids.iter().map(|id| (*id, 0)).collect();
    if ids.is_empty() {
        return Ok(counts);
    }
    let rows = DocumentFlag::query()
        .where_(DocumentFlag::RECORD_TYPE.eq(record_type))
        .where_(DocumentFlag::RECORD_ID.is_in(ids.to_vec()))
        .get(db)
        .await?;
    for row in rows.iter().filter(|row| !row.is_reviewed()) {
        *counts.entry(row.record_id).or_insert(0) += 1;
    }
    Ok(counts)
}

/// Marks flag `flag_id` reviewed by user `reviewer` with `note`. The note is
/// required, and a reviewed flag cannot be reviewed again (422). An unknown
/// flag is a 404.
pub async fn review(db: &Db, flag_id: i64, reviewer: i64, note: &str) -> Result<RecordFlag> {
    let note = note.trim();
    if note.is_empty() {
        return Err(refused("wl_documents.flag.note_required"));
    }
    let mut flag = DocumentFlag::find_or_404(db, flag_id).await?;
    if flag.is_reviewed() {
        return Err(refused("wl_documents.flag.already_reviewed"));
    }
    flag.reviewed_at = Some(now());
    flag.reviewed_by = Some(reviewer);
    flag.review_note = Some(note.to_owned());
    flag.save(db).await?;
    to_record_flag(db, &flag).await
}

/// A 422 with the English text for `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, texts::english(key, &[]))
}
