//! Documents and their versions (wl-documents §3 *Document*, *Document
//! version*, §4.1, §5.1).
//!
//! A [`Document`] is one logical document of a [`DocumentKind`] attached to
//! a [`Subject`] (an employee, a vehicle, a vendor party…). Each file is a
//! [`DocumentVersion`]: stored privately on the app's default disk under
//! `documents/<subject_type>/<subject_id>/<document_id>/v<n>/` with a random
//! name, with its file name, MIME type, size and SHA-256. A file is never
//! overwritten: a new file is a new version.
//!
//! [`Documents`] is the service the other parts of the module and the pages
//! call:
//!
//! - [`Documents::attach`] (`submit`) checks that the kind allows the
//!   subject type, the file type and the size, and creates the document in
//!   `pending_review`; [`Documents::record`] (`record`) creates it
//!   `accepted` for a kind without review;
//! - [`Documents::accept`], [`Documents::reject`], [`Documents::resubmit`]
//!   and [`Documents::archive`] run the machine's transitions
//!   ([`crate::machine`]); accepting (or recording) supersedes the previous
//!   accepted document of the same kind and subject, in the same
//!   transaction;
//! - [`Documents::current`] is the accepted document of a kind for a
//!   subject.
//!
//! After each commit the kernel events are emitted: `DocumentUploaded`
//! (`submit`, `resubmit`), `DocumentAccepted` (`accept`, `record`),
//! `DocumentRejected` and `DocumentSuperseded`, next to Renox's
//! `Transitioned<Document>`. The notifications of §4.1 (uploader,
//! reviewers) and the expiry reminders come with their own issues.

use std::fmt;

use renox::chrono::NaiveDate;
use renox::db::{Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError, TransitionRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wl_kernel::Subject;
use wl_kernel::documents::{
    DocumentAccepted, DocumentId, DocumentRejected, DocumentSuperseded, DocumentUploaded,
};

use crate::expiry::ReminderLog;
use crate::kinds::DocumentKind;
use crate::machine::{
    ACCEPT, ARCHIVE, AcceptInput, KindInput, RECORD, REJECT, RESUBMIT, ResubmitInput, SUBMIT,
    SUPERSEDE, SupersedeInput,
};
use crate::{permissions, texts};

/// Where a document's status stands (§4.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DocumentStatus {
    /// Uploaded, not yet relied on.
    #[default]
    PendingReview,
    /// The valid record of the document: guards and reminders use it.
    Accepted,
    /// Not acceptable; the uploader is told why.
    Rejected,
    /// Replaced by a newer document of the same kind and subject.
    Superseded,
    /// No longer current; kept for retention.
    Archived,
}

/// How a document came to be (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DocumentSource {
    /// A file someone uploaded.
    #[default]
    Upload,
    /// Generated from a template.
    Generated,
    /// The signed result of a signature request.
    Signed,
}

/// Where an upload came from when nothing else is said.
pub const BACK_OFFICE: &str = "back_office";

/// One logical document attached to a record. Its status moves only
/// through the machine ([`crate::machine`]).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_documents_documents",
    state_machine,
    index(subject_type, subject_id, kind_id),
    index(status)
)]
pub struct Document {
    /// The primary key.
    pub id: i64,
    /// What it is attached to: the type, e.g. `hr.employee`.
    pub subject_type: String,
    /// What it is attached to: the record's id.
    pub subject_id: i64,
    /// Its kind.
    #[model(references = "wl_documents_kinds")]
    pub kind_id: i64,
    /// The title (the kind's name unless given).
    pub title: String,
    /// The number, e.g. a passport no.
    pub number: Option<String>,
    /// The issue date.
    pub issue_date: Option<NaiveDate>,
    /// The expiry date (Asia/Dubai); valid through it.
    pub expiry_date: Option<NaiveDate>,
    /// The issuing authority.
    pub issuing_authority: Option<String>,
    /// The number of the version in force.
    pub current_version: i64,
    /// Where it stands.
    pub status: DocumentStatus,
    /// Uploaded, generated or signed.
    pub source: DocumentSource,
    /// Where it came from, e.g. `back_office`, `portal`.
    pub channel: String,
    /// The user who uploaded it; `None` for the system.
    pub uploaded_by: Option<i64>,
    /// When it was last submitted (or recorded).
    pub submitted_at: Option<DateTime>,
    /// The reviewer of the last `accept` or `reject`.
    pub reviewed_by: Option<i64>,
    /// When it was last reviewed.
    pub reviewed_at: Option<DateTime>,
    /// The reviewer's reason for the last `reject`.
    pub rejection_reason: Option<String>,
    /// When it became accepted.
    pub accepted_at: Option<DateTime>,
    /// The document that superseded it.
    pub superseded_by: Option<i64>,
    /// When it was superseded.
    pub superseded_at: Option<DateTime>,
    /// When it was archived.
    pub archived_at: Option<DateTime>,
    /// Set when a refused guard names this pending renewal: the review
    /// queue shows it first.
    pub review_priority_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Document {
    /// What it is attached to.
    pub fn subject(&self) -> Subject {
        Subject {
            subject_type: self.subject_type.clone(),
            subject_id: self.subject_id,
        }
    }

    /// Its id, as the kernel names documents.
    pub fn document_id(&self) -> DocumentId {
        DocumentId(self.id)
    }

    /// Its kind.
    pub async fn kind(&self, db: &Db) -> Result<DocumentKind> {
        DocumentKind::find_or_404(db, self.kind_id).await
    }

    /// Its versions, oldest first.
    pub async fn versions(&self, db: &Db) -> Result<Vec<DocumentVersion>> {
        DocumentVersion::query()
            .where_(DocumentVersion::DOCUMENT_ID.eq(self.id))
            .order_by(DocumentVersion::VERSION_NO)
            .get(db)
            .await
    }

    /// The version in force.
    pub async fn current_file(&self, db: &Db) -> Result<Option<DocumentVersion>> {
        DocumentVersion::query()
            .where_(DocumentVersion::DOCUMENT_ID.eq(self.id))
            .where_(DocumentVersion::VERSION_NO.eq(self.current_version))
            .first(db)
            .await
    }
}

/// One file of a document.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_documents_versions", unique(document_id, version_no))]
pub struct DocumentVersion {
    /// The primary key.
    pub id: i64,
    /// The document.
    #[model(references = "wl_documents_documents")]
    pub document_id: i64,
    /// 1, 2, …
    pub version_no: i64,
    /// The private key of the file on the app's default disk.
    pub storage_key: String,
    /// The name the uploader gave the file.
    pub file_name: String,
    /// The MIME type (from the content when known, else as declared).
    pub mime_type: String,
    /// The size in bytes.
    pub size_bytes: i64,
    /// The SHA-256 of the file, lower-case hex.
    pub checksum: String,
    /// The user who uploaded it; `None` for the system.
    pub uploaded_by: Option<i64>,
    /// When it was uploaded.
    pub uploaded_at: DateTime,
    /// The uploader's note.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DocumentVersion {
    /// The file's content, if it is still stored.
    pub async fn read(&self, state: &AppState) -> Result<Option<renox::axum::body::Bytes>> {
        state.storage.get(&self.storage_key).await
    }
}

/// The SHA-256 of `bytes`, lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A document to attach: what [`Documents::attach`] and
/// [`Documents::record`] take.
///
/// ```ignore
/// let visa = NewDocument::new(Subject::new("hr.employee", 7), "residence_visa", file)
///     .number("201/2026/1234567")
///     .expires_on("2027-11-30".parse()?);
/// ```
#[derive(Debug, Clone)]
pub struct NewDocument {
    /// What it is attached to.
    pub subject: Subject,
    /// The kind's key.
    pub kind: String,
    /// The file.
    pub file: Upload,
    /// The title; the kind's name when `None`.
    pub title: Option<String>,
    /// The number.
    pub number: Option<String>,
    /// The issue date.
    pub issue_date: Option<NaiveDate>,
    /// The expiry date.
    pub expiry_date: Option<NaiveDate>,
    /// The issuing authority.
    pub issuing_authority: Option<String>,
    /// The uploader's note on the file.
    pub note: Option<String>,
    /// Where it comes from ([`BACK_OFFICE`] unless set).
    pub channel: String,
    /// Uploaded, generated or signed.
    pub source: DocumentSource,
}

impl NewDocument {
    /// `file` as a document of kind `kind` on `subject`.
    pub fn new(subject: Subject, kind: &str, file: Upload) -> Self {
        NewDocument {
            subject,
            kind: kind.to_owned(),
            file,
            title: None,
            number: None,
            issue_date: None,
            expiry_date: None,
            issuing_authority: None,
            note: None,
            channel: BACK_OFFICE.to_owned(),
            source: DocumentSource::Upload,
        }
    }

    /// The title.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_owned());
        self
    }

    /// The number.
    pub fn number(mut self, number: &str) -> Self {
        self.number = Some(number.to_owned());
        self
    }

    /// The issue date.
    pub fn issued_on(mut self, date: NaiveDate) -> Self {
        self.issue_date = Some(date);
        self
    }

    /// The expiry date.
    pub fn expires_on(mut self, date: NaiveDate) -> Self {
        self.expiry_date = Some(date);
        self
    }

    /// The issuing authority.
    pub fn authority(mut self, authority: &str) -> Self {
        self.issuing_authority = Some(authority.to_owned());
        self
    }

    /// The uploader's note on the file.
    pub fn note(mut self, note: &str) -> Self {
        self.note = Some(note.to_owned());
        self
    }

    /// Where it comes from, e.g. `portal`.
    pub fn channel(mut self, channel: &str) -> Self {
        self.channel = channel.to_owned();
        self
    }

    /// Generated or signed instead of uploaded.
    pub fn source(mut self, source: DocumentSource) -> Self {
        self.source = source;
        self
    }
}

/// Why a file or a document was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttachReason {
    /// The kind does not attach to the subject's type.
    SubjectType,
    /// The kind does not allow the file's type.
    FileType,
    /// The file is larger than the kind allows.
    TooLarge,
    /// The file is empty.
    Empty,
    /// `record` on a kind that needs review.
    NeedsReview,
    /// `record` without the number the kind has.
    NumberMissing,
    /// `record` without the expiry date the kind has.
    ExpiryMissing,
    /// `correct` on a document that is not accepted.
    NotAccepted,
    /// `correct` while another document of the kind and subject waits for
    /// review.
    CorrectionOpen,
    /// `attach` or `record` on a kind that is available and off.
    KindOff,
}

impl AttachReason {
    /// The text key of the message.
    pub fn text_key(self) -> &'static str {
        match self {
            AttachReason::SubjectType => "wl_documents.attach.subject_type",
            AttachReason::FileType => "wl_documents.attach.file_type",
            AttachReason::TooLarge => "wl_documents.attach.too_large",
            AttachReason::Empty => "wl_documents.attach.empty",
            AttachReason::NeedsReview => "wl_documents.attach.needs_review",
            AttachReason::NumberMissing => "wl_documents.attach.number_missing",
            AttachReason::ExpiryMissing => "wl_documents.attach.expiry_missing",
            AttachReason::NotAccepted => "wl_documents.correction.not_accepted",
            AttachReason::CorrectionOpen => "wl_documents.correction.open",
            AttachReason::KindOff => "wl_documents.attach.kind_off",
        }
    }
}

/// A document or file that was refused; nothing was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRefused {
    /// Why.
    pub reason: AttachReason,
    /// The kind's key.
    pub kind: String,
    /// The kind's English name.
    pub kind_name: String,
    /// The subject's type.
    pub subject_type: String,
    /// The file's name.
    pub file_name: String,
    /// The file's extension (`?` when it has none).
    pub extension: String,
    /// The kind's allowed extensions, comma separated.
    pub allowed: String,
    /// The kind's largest file, in MB.
    pub max_mb: String,
}

impl AttachRefused {
    fn new(reason: AttachReason, kind: &DocumentKind, subject_type: &str, file: &Upload) -> Self {
        AttachRefused {
            reason,
            kind: kind.key.clone(),
            kind_name: kind.name_en.clone(),
            subject_type: subject_type.to_owned(),
            file_name: file.file_name().to_owned(),
            extension: file.extension().unwrap_or_else(|| "?".to_owned()),
            allowed: kind.allowed_file_types.0.join(", "),
            max_mb: megabytes(kind.max_size_bytes),
        }
    }

    fn params(&self) -> [(&str, &dyn fmt::Display); 6] {
        [
            ("kind", &self.kind_name),
            ("subject_type", &self.subject_type),
            ("file", &self.file_name),
            ("extension", &self.extension),
            ("allowed", &self.allowed),
            ("max", &self.max_mb),
        ]
    }

    /// The message in `lang` (the app's translation, else English).
    pub fn message(&self, lang: &Lang) -> String {
        texts::t_with(lang, self.reason.text_key(), &self.params())
    }

    /// The refusal as a 422 with the English message.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
    }
}

impl fmt::Display for AttachRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&texts::english(self.reason.text_key(), &self.params()))
    }
}

impl std::error::Error for AttachRefused {}

/// The result of attaching: the outer `Err` is a failure (an unknown kind,
/// the database, the disk), the inner one a refusal.
pub type Attach<T> = Result<std::result::Result<T, AttachRefused>>;

/// `bytes` in MB for a message: `20`, or `2.5`.
fn megabytes(bytes: i64) -> String {
    const MB: i64 = 1024 * 1024;
    if bytes % MB == 0 {
        (bytes / MB).to_string()
    } else {
        // Display only; never money.
        #[expect(clippy::cast_precision_loss, reason = "a size shown to one decimal")]
        let mb = bytes as f64 / MB as f64;
        format!("{mb:.1}")
    }
}

/// The refusal of `new` when its kind is off (available, not enabled), else
/// `None`. Documents the kind already has stay, and a correction of one is
/// not a new document, so only `attach` and `record` ask.
///
/// # Errors
///
/// `NotFound` for an unknown kind; the database's errors.
async fn kind_off(state: &AppState, new: &NewDocument) -> Result<Option<AttachRefused>> {
    let kind = DocumentKind::by_key(&state.db, &new.kind)
        .await?
        .ok_or(Error::NotFound)?;
    Ok((!kind.enabled).then(|| {
        AttachRefused::new(
            AttachReason::KindOff,
            &kind,
            &new.subject.subject_type,
            &new.file,
        )
    }))
}

/// Why `kind` refuses `file` on a subject of `subject_type`, if it does:
/// the subject type, an empty file, the file type, the size.
pub fn file_refusal(
    kind: &DocumentKind,
    subject_type: &str,
    file: &Upload,
) -> Option<AttachReason> {
    if !kind.allows_subject_type(subject_type) {
        return Some(AttachReason::SubjectType);
    }
    if file.size() == 0 {
        return Some(AttachReason::Empty);
    }
    if !file
        .extension()
        .is_some_and(|extension| kind.allows_file_type(&extension))
    {
        return Some(AttachReason::FileType);
    }
    if i64::try_from(file.size()).unwrap_or(i64::MAX) > kind.max_size_bytes {
        return Some(AttachReason::TooLarge);
    }
    None
}

/// The MIME type to keep for `file`: from its content when Renox knows the
/// format, else as declared.
fn mime_type(file: &Upload) -> String {
    match file.sniffed_type() {
        Some(sniffed) => sniffed.to_owned(),
        None if !file.content_type().trim().is_empty() => file.content_type().trim().to_owned(),
        None => "application/octet-stream".to_owned(),
    }
}

/// Stores `file` privately as version `version_no` of `document` and adds
/// its row in `tx`. The file is written first, under a new random name, so
/// nothing is ever overwritten; a rolled-back transaction leaves it
/// unreferenced.
async fn store_version(
    state: &AppState,
    tx: &mut Transaction,
    document: &Document,
    version_no: i64,
    file: &Upload,
    uploaded_by: Option<i64>,
    note: Option<String>,
) -> Result<DocumentVersion> {
    let dir = format!(
        "documents/{}/{}/{}/v{version_no}",
        document.subject_type, document.subject_id, document.id
    );
    let storage_key = file.store(&state.storage, &dir).await?;
    DocumentVersion::create(
        &mut *tx,
        DocumentVersion {
            document_id: document.id,
            version_no,
            storage_key,
            file_name: file.file_name().trim().to_owned(),
            mime_type: mime_type(file),
            size_bytes: i64::try_from(file.size()).unwrap_or(i64::MAX),
            checksum: sha256_hex(file.bytes()),
            uploaded_by,
            uploaded_at: now(),
            note: note.filter(|n| !n.trim().is_empty()),
            ..DocumentVersion::default()
        },
    )
    .await
}

/// A superseded document and the run that moved it, to emit after the
/// commit.
pub(crate) type Superseded = (i64, Outcome<Document>);

/// Supersedes, in `tx`, every other accepted document of `accepted`'s kind
/// and subject (`supersede`, by the system).
pub(crate) async fn supersede_previous(
    tx: &mut Transaction,
    accepted: &Document,
) -> std::result::Result<Vec<Superseded>, TransitionError> {
    let previous = Document::query()
        .where_(Document::SUBJECT_TYPE.eq(accepted.subject_type.as_str()))
        .where_(Document::SUBJECT_ID.eq(accepted.subject_id))
        .where_(Document::KIND_ID.eq(accepted.kind_id))
        .where_(Document::STATUS.eq(DocumentStatus::Accepted))
        .where_(Document::ID.ne(accepted.id))
        .get(&mut *tx)
        .await?;
    let mut superseded = Vec::with_capacity(previous.len());
    for document in previous {
        let outcome = document
            .transition(SUPERSEDE)
            .by_system()
            .input(&SupersedeInput {
                replaced_by: accepted.id,
            })
            .run_in(&mut *tx)
            .await?;
        superseded.push((document.id, outcome));
    }
    Ok(superseded)
}

/// After the commit: Renox's `Transitioned` and the kernel's
/// `DocumentSuperseded` for each superseded document.
pub(crate) async fn emit_superseded(
    state: &AppState,
    superseded: Vec<Superseded>,
    replaced_by: i64,
) -> Result {
    for (id, outcome) in superseded {
        let done = !outcome.is_pending();
        outcome.emit(state).await?;
        if done {
            state
                .emit(DocumentSuperseded {
                    document: DocumentId(id),
                    replaced_by: DocumentId(replaced_by),
                })
                .await?;
        }
    }
    Ok(())
}

/// The kernel's `DocumentAccepted` for `document`.
pub(crate) async fn emit_accepted(
    state: &AppState,
    document: &Document,
    kind: &DocumentKind,
) -> Result {
    state
        .emit(DocumentAccepted {
            document: document.document_id(),
            subject: document.subject(),
            kind: kind.key.clone(),
            expires_on: document.expiry_date,
        })
        .await
}

/// The kernel's `DocumentUploaded` for `document`.
async fn emit_uploaded(state: &AppState, document: &Document, kind: &DocumentKind) -> Result {
    state
        .emit(DocumentUploaded {
            document: document.document_id(),
            subject: document.subject(),
            kind: kind.key.clone(),
            uploaded_by: document.uploaded_by,
            source: document.channel.clone(),
        })
        .await
}

/// What a correction of an accepted document carries (`DOC.ACL.03`, #51):
/// the new values and the new file. The kind and subject are the old
/// document's.
#[derive(Debug)]
pub struct CorrectionInput {
    /// The new number.
    pub number: Option<String>,
    /// The new issue date.
    pub issue_date: Option<NaiveDate>,
    /// The new expiry date.
    pub expiry_date: Option<NaiveDate>,
    /// The new issuing authority.
    pub authority: Option<String>,
    /// The new title (the old one's when none).
    pub title: Option<String>,
    /// A note on the file.
    pub note: Option<String>,
    /// The new file.
    pub file: Upload,
}

impl CorrectionInput {
    /// A correction with `file` and nothing else yet.
    #[must_use]
    pub fn new(file: Upload) -> Self {
        CorrectionInput {
            number: None,
            issue_date: None,
            expiry_date: None,
            authority: None,
            title: None,
            note: None,
            file,
        }
    }
}

/// The fields of a document that can be edited in place (`DOC.ACL.03`,
/// #51): every field is replaced by the value given, so start from
/// [`FieldsUpdate::of`] and change what is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldsUpdate {
    /// The title.
    pub title: String,
    /// The issuing authority.
    pub issuing_authority: Option<String>,
    /// The number; refused for a blocking kind when changed.
    pub number: Option<String>,
    /// The issue date.
    pub issue_date: Option<NaiveDate>,
    /// The expiry date; refused for a blocking kind when changed.
    pub expiry_date: Option<NaiveDate>,
}

impl FieldsUpdate {
    /// The values `document` has now.
    #[must_use]
    pub fn of(document: &Document) -> Self {
        FieldsUpdate {
            title: document.title.clone(),
            issuing_authority: document.issuing_authority.clone(),
            number: document.number.clone(),
            issue_date: document.issue_date,
            expiry_date: document.expiry_date,
        }
    }
}

/// Records, after the commit, that `document` replaced `old`: the old and
/// new ids, number and expiry date, the uploader and the accepting user.
async fn audit_corrected(
    state: &AppState,
    by: &AuthUser,
    old: &Document,
    document: &Document,
) -> Result {
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new("document.corrected")
            .user(by.id)
            .subject(Document::TABLE, document.id)
            .data(json!({
                "old_document": old.id,
                "new_document": document.id,
                "old_number": old.number,
                "new_number": document.number,
                "old_expiry_date": old.expiry_date,
                "new_expiry_date": document.expiry_date,
                "uploaded_by": document.uploaded_by,
                "accepted_by": by.id,
            })),
    )
    .await
}

/// The documents service (§6).
pub struct Documents;

impl Documents {
    /// `submit`: attaches `new` for review, uploaded by `by` (`None` for the
    /// system). Refused when the kind does not attach to the subject's type,
    /// the file is empty, its type is not allowed or it is too large; then
    /// nothing is stored. Emits `DocumentUploaded`.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown kind; the database's or the disk's errors.
    pub async fn attach(
        state: &AppState,
        by: Option<&AuthUser>,
        new: NewDocument,
    ) -> Attach<Document> {
        if let Some(refused) = kind_off(state, &new).await? {
            return Ok(Err(refused));
        }
        create(
            state,
            by.map(|user| user.id),
            new,
            DocumentStatus::PendingReview,
        )
        .await
    }

    /// `record`: attaches `new` as accepted at once, by `by` holding
    /// `documents.upload`, for a kind that does not need review (a kind with
    /// `blocks` always does). Refused as [`Documents::attach`] is, and when
    /// the number or expiry date the kind has is missing. Supersedes the
    /// previous accepted document of the kind and subject and emits
    /// `DocumentAccepted`.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `documents.upload`, `NotFound` for an unknown
    /// kind; the database's or the disk's errors.
    pub async fn record(state: &AppState, by: &AuthUser, new: NewDocument) -> Attach<Document> {
        if !by.has_permission(permissions::UPLOAD) {
            return Err(Error::Forbidden);
        }
        if let Some(refused) = kind_off(state, &new).await? {
            return Ok(Err(refused));
        }
        create(state, Some(by.id), new, DocumentStatus::Accepted).await
    }

    /// `correct`: `by` (holding `documents.upload`) submits `input` as a
    /// correction of the accepted `document`: a new document of the same
    /// kind and subject, in `pending_review`, uploaded by `by`. `document`
    /// stays accepted and in force until the correction is accepted, by
    /// someone else for a blocking kind. Refused as [`Documents::attach`] is,
    /// when `document` is not accepted, and while another document of the
    /// kind and subject waits for review (one correction at a time).
    /// Records `document.correction_submitted` and emits `DocumentUploaded`.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `documents.upload`; the database's or the disk's
    /// errors.
    pub async fn correct(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
        input: CorrectionInput,
    ) -> Attach<Document> {
        if !by.has_permission(permissions::UPLOAD) {
            return Err(Error::Forbidden);
        }
        let kind = document.kind(&state.db).await?;
        let current = Document::find_or_404(&state.db, document.id).await?;
        let refusal = if current.status != DocumentStatus::Accepted {
            Some(AttachReason::NotAccepted)
        } else if Document::query()
            .where_(Document::SUBJECT_TYPE.eq(current.subject_type.as_str()))
            .where_(Document::SUBJECT_ID.eq(current.subject_id))
            .where_(Document::KIND_ID.eq(current.kind_id))
            .where_(Document::STATUS.eq(DocumentStatus::PendingReview))
            .first(&state.db)
            .await?
            .is_some()
        {
            Some(AttachReason::CorrectionOpen)
        } else {
            None
        };
        if let Some(reason) = refusal {
            return Ok(Err(AttachRefused::new(
                reason,
                &kind,
                &current.subject_type,
                &input.file,
            )));
        }
        let mut new = NewDocument::new(current.subject(), &kind.key, input.file);
        new.title = input.title.or_else(|| Some(current.title.clone()));
        new.number = input.number;
        new.issue_date = input.issue_date;
        new.expiry_date = input.expiry_date;
        new.issuing_authority = input.authority;
        new.note = input.note;
        let created = match create(state, Some(by.id), new, DocumentStatus::PendingReview).await? {
            Ok(created) => created,
            Err(refused) => return Ok(Err(refused)),
        };
        renox::audit::record(
            &state.db,
            renox::audit::Entry::new("document.correction_submitted")
                .user(by.id)
                .subject(Document::TABLE, created.id)
                .data(json!({
                    "old_document": current.id,
                    "new_document": created.id,
                    "old_number": current.number,
                    "new_number": created.number,
                    "old_expiry_date": current.expiry_date,
                    "new_expiry_date": created.expiry_date,
                    "uploaded_by": by.id,
                })),
        )
        .await?;
        Ok(Ok(created))
    }

    /// `update_fields`: `by` (holding `documents.manage`) edits `document`
    /// in place, accepted or pending only. Title and issuing authority are
    /// always editable. For a blocking kind any change to the number or
    /// expiry date is refused with a 422, whatever the user's roles: that
    /// takes a correction ([`Documents::correct`]). Records
    /// `document.updated` with the old and new values of what changed. For
    /// another kind a changed expiry date forgets the reminders already
    /// sent, so the nightly run (#48) reaches them again. Nothing is
    /// stored when nothing changed. Returns the document as stored.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `documents.manage`; a 422 when refused; the
    /// database's errors.
    pub async fn update_fields(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
        update: FieldsUpdate,
    ) -> Result<Document> {
        if !by.has_permission(permissions::MANAGE) {
            return Err(Error::Forbidden);
        }
        let kind = document.kind(&state.db).await?;
        let current = Document::find_or_404(&state.db, document.id).await?;
        let refuse = |key: &str| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                texts::english(key, &[("kind", &kind.name_en)]),
            )
        };
        if !matches!(
            current.status,
            DocumentStatus::Accepted | DocumentStatus::PendingReview
        ) {
            return Err(refuse("wl_documents.update.status"));
        }
        let key_fields_changed =
            update.number != current.number || update.expiry_date != current.expiry_date;
        let blocking = kind.is_blocking(&state.db).await?;
        if blocking && key_fields_changed {
            return Err(refuse("wl_documents.update.blocking_fields"));
        }
        if update == FieldsUpdate::of(&current) {
            return Ok(current);
        }
        let mut tx = state.db.begin().await?;
        Document::query()
            .where_(Document::ID.eq(current.id))
            .update(
                &mut tx,
                &[
                    ("title", &update.title),
                    ("issuing_authority", &update.issuing_authority),
                    ("number", &update.number),
                    ("issue_date", &update.issue_date),
                    ("expiry_date", &update.expiry_date),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        if update.expiry_date != current.expiry_date {
            ReminderLog::reset_for(&mut tx, current.id).await?;
        }
        tx.commit().await?;
        renox::audit::record(
            &state.db,
            renox::audit::Entry::new("document.updated")
                .user(by.id)
                .subject(Document::TABLE, current.id)
                .data(json!({
                    "old": {
                        "title": current.title,
                        "issuing_authority": current.issuing_authority,
                        "number": current.number,
                        "issue_date": current.issue_date,
                        "expiry_date": current.expiry_date,
                    },
                    "new": {
                        "title": update.title,
                        "issuing_authority": update.issuing_authority,
                        "number": update.number,
                        "issue_date": update.issue_date,
                        "expiry_date": update.expiry_date,
                    },
                })),
        )
        .await?;
        Document::find_or_404(&state.db, current.id).await
    }

    /// `accept`: `by` (holding `documents.review`) accepts the pending
    /// `document`, once the number and expiry date its kind has are filled.
    /// The previous accepted document of the same kind and subject is
    /// superseded in the same transaction. Emits `DocumentAccepted` and
    /// `DocumentSuperseded`. Returns the document as stored.
    ///
    /// # Errors
    ///
    /// `Forbidden`, `NotFrom` (not pending), `Guard` (a field missing).
    pub async fn accept(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
    ) -> std::result::Result<Document, TransitionError> {
        let kind = document.kind(&state.db).await?;
        let input = AcceptInput {
            needs_number: kind.has_number,
            needs_expiry: kind.has_expiry,
            sop_kind: crate::sops::is_sop_kind(&kind.key),
            blocking: kind.is_blocking(&state.db).await?,
            uploaded_by: document.uploaded_by,
        };
        let mut tx = state.db.begin().await?;
        let outcome = document
            .transition(ACCEPT)
            .by(by)
            .input(&input)
            .run_in(&mut tx)
            .await?;
        let done = !outcome.is_pending();
        let superseded = if done {
            supersede_previous(&mut tx, document).await?
        } else {
            Vec::new()
        };
        tx.commit().await?;
        outcome.emit(state).await?;
        let accepted = Document::find_or_404(&state.db, document.id).await?;
        if done {
            emit_accepted(state, &accepted, &kind).await?;
            if input.blocking {
                for (old_id, _) in &superseded {
                    let old = Document::find_or_404(&state.db, *old_id).await?;
                    audit_corrected(state, by, &old, &accepted).await?;
                }
            }
            emit_superseded(state, superseded, accepted.id).await?;
        }
        Ok(accepted)
    }

    /// `reject`: `by` (holding `documents.review`) rejects the pending
    /// `document` for `reason`, which the document keeps for the uploader.
    /// Emits `DocumentRejected`.
    ///
    /// # Errors
    ///
    /// `Forbidden`, `NotFrom` (not pending), `ReasonRequired`.
    pub async fn reject(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
        reason: &str,
    ) -> std::result::Result<Document, TransitionError> {
        let kind = document.kind(&state.db).await?;
        let outcome = document
            .transition(REJECT)
            .by(by)
            .input(&KindInput {
                sop_kind: crate::sops::is_sop_kind(&kind.key),
            })
            .reason(reason.trim())
            .run(state)
            .await?;
        let rejected = Document::find_or_404(&state.db, document.id).await?;
        if !outcome.is_pending() {
            state
                .emit(DocumentRejected {
                    document: rejected.document_id(),
                    subject: rejected.subject(),
                    kind: kind.key.clone(),
                    reason: reason.trim().to_owned(),
                })
                .await?;
        }
        Ok(rejected)
    }

    /// `resubmit`: `by` sends `file` as a new version of the rejected
    /// `document`, which goes back to `pending_review`. The file is checked
    /// as for [`Documents::attach`]. Emits `DocumentUploaded`.
    ///
    /// # Errors
    ///
    /// `NotFrom` (not rejected); `Guard` with the refusal's message when the
    /// file is refused (nothing is stored then).
    pub async fn resubmit(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
        file: Upload,
        note: Option<&str>,
    ) -> std::result::Result<Document, TransitionError> {
        let kind = document.kind(&state.db).await?;
        if let Some(reason) = file_refusal(&kind, &document.subject_type, &file) {
            let refused = AttachRefused::new(reason, &kind, &document.subject_type, &file);
            return Err(TransitionError::Guard(refused.to_string()));
        }
        // Checked before the file is stored; the run checks it again.
        let machine = Document::machine();
        machine
            .find(RESUBMIT)?
            .check_from(document.status.as_str())?;
        let version = document.current_version + 1;
        let mut tx = state.db.begin().await?;
        store_version(
            state,
            &mut tx,
            document,
            version,
            &file,
            Some(by.id),
            note.map(str::to_owned),
        )
        .await?;
        let outcome = document
            .transition(RESUBMIT)
            .by(by)
            .input(&ResubmitInput { version })
            .run_in(&mut tx)
            .await?;
        tx.commit().await?;
        let done = !outcome.is_pending();
        outcome.emit(state).await?;
        let resubmitted = Document::find_or_404(&state.db, document.id).await?;
        if done {
            state
                .emit(DocumentUploaded {
                    document: resubmitted.document_id(),
                    subject: resubmitted.subject(),
                    kind: kind.key.clone(),
                    uploaded_by: Some(by.id),
                    source: resubmitted.channel.clone(),
                })
                .await?;
        }
        Ok(resubmitted)
    }

    /// `archive`: `by` (holding `documents.manage`) archives the accepted
    /// or rejected `document`, with an optional reason for the history.
    ///
    /// # Errors
    ///
    /// `Forbidden`, `NotFrom`.
    pub async fn archive(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
        reason: Option<&str>,
    ) -> std::result::Result<Document, TransitionError> {
        let kind = document.kind(&state.db).await?;
        let mut run = document.transition(ARCHIVE).by(by).input(&KindInput {
            sop_kind: crate::sops::is_sop_kind(&kind.key),
        });
        if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
            run = run.reason(reason);
        }
        let _outcome = run.run(state).await?;
        Ok(Document::find_or_404(&state.db, document.id).await?)
    }

    /// `generate` (`DOC.GEN.01`, §5.3): the branded PDF of a template for a
    /// subject, stored as an accepted document with source `generated`
    /// ([`crate::generation`]).
    ///
    /// # Errors
    ///
    /// The database's or the disk's errors; refusals are `Ok(Err(..))`.
    pub async fn generate(
        state: &AppState,
        request: wl_kernel::generation::GenerateRequest,
    ) -> Result<wl_kernel::generation::Generated> {
        crate::generation::generate(state, request).await
    }

    /// The accepted document of kind `kind` on `subject` (the latest
    /// accepted, should two ever be), if any. An unknown kind has none.
    pub async fn current(db: &Db, subject: &Subject, kind: &str) -> Result<Option<Document>> {
        let Some(kind) = DocumentKind::by_key(db, kind).await? else {
            return Ok(None);
        };
        Document::query()
            .where_(Document::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(Document::SUBJECT_ID.eq(subject.subject_id))
            .where_(Document::KIND_ID.eq(kind.id))
            .where_(Document::STATUS.eq(DocumentStatus::Accepted))
            .order_by_desc(Document::ACCEPTED_AT)
            .order_by_desc(Document::ID)
            .first(db)
            .await
    }

    /// Every document of `subject`, newest first.
    pub async fn of_subject(db: &Db, subject: &Subject) -> Result<Vec<Document>> {
        Document::query()
            .where_(Document::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(Document::SUBJECT_ID.eq(subject.subject_id))
            .order_by_desc(Document::ID)
            .get(db)
            .await
    }
}

/// `submit` (into `pending_review`) or `record` (into `accepted`): checks,
/// inserts the document, stores version 1, writes the history row under the
/// transition's name and, for `record`, supersedes the previous accepted
/// document; then emits the events.
async fn create(
    state: &AppState,
    by: Option<i64>,
    new: NewDocument,
    status: DocumentStatus,
) -> Attach<Document> {
    let kind = DocumentKind::by_key(&state.db, &new.kind)
        .await?
        .ok_or(Error::NotFound)?;
    let subject_type = new.subject.subject_type.as_str();
    let refuse = |reason| {
        Ok(Err(AttachRefused::new(
            reason,
            &kind,
            subject_type,
            &new.file,
        )))
    };
    if let Some(reason) = file_refusal(&kind, subject_type, &new.file) {
        return refuse(reason);
    }
    let recording = status == DocumentStatus::Accepted;
    let at = now();
    let document = Document {
        subject_type: subject_type.to_owned(),
        subject_id: new.subject.subject_id,
        kind_id: kind.id,
        title: new
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| kind.name_en.clone()),
        number: new.number.clone().filter(|n| !n.trim().is_empty()),
        issue_date: new.issue_date,
        expiry_date: new.expiry_date,
        issuing_authority: new.issuing_authority.clone(),
        current_version: 1,
        status,
        source: new.source,
        channel: new.channel.clone(),
        uploaded_by: by,
        submitted_at: Some(at),
        accepted_at: recording.then_some(at),
        ..Document::default()
    };
    if recording {
        if kind.needs_review || kind.is_blocking(&state.db).await? {
            return refuse(AttachReason::NeedsReview);
        }
        let rules = AcceptInput {
            needs_number: kind.has_number,
            needs_expiry: kind.has_expiry,
            ..AcceptInput::default()
        };
        if rules.missing(&document).is_some() {
            let reason = if rules.needs_number && document.number.is_none() {
                AttachReason::NumberMissing
            } else {
                AttachReason::ExpiryMissing
            };
            return refuse(reason);
        }
    }

    let mut tx = state.db.begin().await?;
    let document = Document::create(&mut tx, document).await?;
    store_version(
        state,
        &mut tx,
        &document,
        1,
        &new.file,
        by,
        new.note.clone(),
    )
    .await?;
    let name = if recording { RECORD } else { SUBMIT };
    let mut history = TransitionRecord::new(Document::TABLE, document.id, status.as_str(), name);
    if let Some(user) = by {
        history = history.actor(user);
    }
    history.record(&mut tx).await?;
    let superseded = if recording {
        supersede_previous(&mut tx, &document)
            .await
            .map_err(TransitionError::into_error)?
    } else {
        Vec::new()
    };
    tx.commit().await?;

    if recording {
        emit_accepted(state, &document, &kind).await?;
        emit_superseded(state, superseded, document.id).await?;
    } else {
        emit_uploaded(state, &document, &kind).await?;
    }
    Ok(Ok(document))
}

/// `record` of a generated document (§3: generated documents are created
/// directly as accepted, by the system or the user who asked), in the
/// caller's transaction `tx`, so the number taken for it and the generation
/// row commit or roll back together. The caller has checked the file against
/// `kind` ([`file_refusal`]) and emits [`emit_accepted`] and
/// [`emit_superseded`] after the commit.
pub(crate) async fn record_generated_in(
    state: &AppState,
    tx: &mut Transaction,
    by: Option<i64>,
    new: &NewDocument,
    kind: &DocumentKind,
) -> Result<(Document, Vec<Superseded>)> {
    let at = now();
    let document = Document {
        subject_type: new.subject.subject_type.clone(),
        subject_id: new.subject.subject_id,
        kind_id: kind.id,
        title: new
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| kind.name_en.clone()),
        number: new.number.clone().filter(|n| !n.trim().is_empty()),
        issue_date: new.issue_date,
        expiry_date: new.expiry_date,
        issuing_authority: new.issuing_authority.clone(),
        current_version: 1,
        status: DocumentStatus::Accepted,
        source: new.source,
        channel: new.channel.clone(),
        uploaded_by: by,
        submitted_at: Some(at),
        accepted_at: Some(at),
        ..Document::default()
    };
    let document = Document::create(&mut *tx, document).await?;
    store_version(state, tx, &document, 1, &new.file, by, new.note.clone()).await?;
    let mut history = TransitionRecord::new(
        Document::TABLE,
        document.id,
        DocumentStatus::Accepted.as_str(),
        RECORD,
    );
    if let Some(user) = by {
        history = history.actor(user);
    }
    history.record(&mut *tx).await?;
    let superseded = supersede_previous(tx, &document)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((document, superseded))
}

#[cfg(test)]
mod tests {
    use renox::state_machine;
    use renox::testing::TestApp;

    use super::*;
    use crate::kinds::{KindSeed, install_seeds};

    fn pdf(name: &str) -> Upload {
        Upload::new(name, "application/pdf", b"%PDF-1.7 visa".to_vec())
    }

    fn visa() -> KindSeed {
        KindSeed::new("residence_visa", "Residence visa")
            .subject_type("hr.employee")
            .expires()
            .numbered()
            .file_types(&["pdf", "jpg"])
            .max_size(2 * 1024 * 1024 + 512 * 1024)
    }

    fn ravi() -> Subject {
        Subject::new("hr.employee", 7)
    }

    #[test]
    fn sha256_is_lower_case_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_file_is_checked_against_its_kind() {
        let kind = DocumentKind::from_seed(&visa());
        assert_eq!(file_refusal(&kind, "hr.employee", &pdf("visa.pdf")), None);
        assert_eq!(
            file_refusal(&kind, "fleet.vehicle", &pdf("visa.pdf")),
            Some(AttachReason::SubjectType)
        );
        assert_eq!(
            file_refusal(&kind, "hr.employee", &pdf("visa.exe")),
            Some(AttachReason::FileType)
        );
        assert_eq!(
            file_refusal(&kind, "hr.employee", &pdf("visa")),
            Some(AttachReason::FileType)
        );
        let empty = Upload::new("visa.pdf", "application/pdf", Vec::new());
        assert_eq!(
            file_refusal(&kind, "hr.employee", &empty),
            Some(AttachReason::Empty)
        );
        let big = Upload::new("visa.pdf", "application/pdf", vec![0_u8; 3 * 1024 * 1024]);
        assert_eq!(
            file_refusal(&kind, "hr.employee", &big),
            Some(AttachReason::TooLarge)
        );
        let refused = AttachRefused::new(AttachReason::TooLarge, &kind, "hr.employee", &big);
        assert_eq!(
            refused.to_string(),
            "visa.pdf is larger than 2.5 MB, the most Residence visa allows"
        );
        let refused = AttachRefused::new(
            AttachReason::FileType,
            &kind,
            "hr.employee",
            &pdf("visa.exe"),
        );
        assert_eq!(
            refused.to_string(),
            "Residence visa does not accept .exe files (allowed: pdf, jpg)"
        );
    }

    #[test]
    fn megabytes_are_whole_or_one_decimal() {
        assert_eq!(megabytes(20 * 1024 * 1024), "20");
        assert_eq!(megabytes(1024 * 1024 + 512 * 1024), "1.5");
    }

    #[renox::test]
    async fn attach_stores_the_file_privately_with_its_checksum() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let state = app.state();
        install_seeds(app.db(), [&visa()]).await.unwrap();

        let refused = Documents::attach(
            state,
            None,
            NewDocument::new(
                Subject::new("fleet.vehicle", 3),
                "residence_visa",
                pdf("v.pdf"),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(refused.reason, AttachReason::SubjectType);
        assert!(
            Documents::of_subject(app.db(), &Subject::new("fleet.vehicle", 3))
                .await
                .unwrap()
                .is_empty()
        );

        let file = pdf("Ravi visa.pdf");
        let document = Documents::attach(
            state,
            None,
            NewDocument::new(ravi(), "residence_visa", file.clone())
                .number("201/2026/1234567")
                .expires_on("2026-11-30".parse().unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(document.status, DocumentStatus::PendingReview);
        assert_eq!(document.title, "Residence visa");
        assert_eq!(document.channel, BACK_OFFICE);

        let versions = document.versions(app.db()).await.unwrap();
        assert_eq!(versions.len(), 1);
        let version = &versions[0];
        assert_eq!(version.version_no, 1);
        assert_eq!(version.file_name, "Ravi visa.pdf");
        assert_eq!(version.mime_type, "application/pdf");
        assert_eq!(version.checksum, sha256_hex(file.bytes()));
        let prefix = format!("documents/hr.employee/7/{}/v1/", document.id);
        assert!(
            version.storage_key.starts_with(&prefix),
            "{}",
            version.storage_key
        );
        assert!(!version.storage_key.starts_with("public/"));
        let stored = version.read(state).await.unwrap().unwrap();
        assert_eq!(&stored[..], &file.bytes()[..]);

        let history = state_machine::history(app.db(), Document::TABLE, document.id)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].transition, SUBMIT);
        assert_eq!(history[0].to_state, "pending_review");

        // Pending is not current.
        assert!(
            Documents::current(app.db(), &ravi(), "residence_visa")
                .await
                .unwrap()
                .is_none()
        );
    }
}
