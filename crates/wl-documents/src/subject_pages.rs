//! The documents of one record, and the form that adds one: the page a
//! person goes to when a transition needs a document the record does not
//! have (`/documents/of/{subject_type}/{subject_id}`, e.g. the tenant's
//! Emirates ID, passport and residence visa before a lease is submitted).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /documents/of/{type}/{id}` | `wl_documents.subject` | `documents.view`, in the record's scope |
//! | `POST /documents/of/{type}/{id}` | `wl_documents.subject.store` | `documents.upload`, in the record's scope |
//!
//! An uploaded document waits for review (`/documents/review`); a kind with
//! `blocks` is accepted by someone other than the uploader (`DOC.ACL.03`).

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::documents::{Document, DocumentStatus, DocumentVersion, Documents, NewDocument};
use crate::kinds::DocumentKind;
use crate::pages::{blank_to_none, invalid, parse_date, text};
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['fields.kind']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .merge(wl_kernel::scopes::anywhere(
            Routes::new()
                .get("/documents/of/{subject_type}/{subject_id}", show)
                .name("wl_documents.subject"),
            permissions::VIEW,
        ))
        .merge(wl_kernel::scopes::anywhere(
            Routes::new()
                .post("/documents/of/{subject_type}/{subject_id}", store)
                .name("wl_documents.subject.store"),
            permissions::UPLOAD,
        ))
}

/// One document of the record.
#[derive(Serialize, Debug, Clone)]
pub struct SubjectRow {
    id: i64,
    kind: String,
    title: String,
    number: String,
    expiry_date: String,
    status: String,
    status_kind: &'static str,
    file_name: Option<String>,
    rejection_reason: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/subject.html")]
pub(crate) struct SubjectPage {
    tx: Texts,
    label: String,
    back: Option<String>,
    documents: Vec<SubjectRow>,
    /// The kinds the record takes, `(key, name)`.
    kinds: Vec<(String, String)>,
    can_upload: bool,
    action: String,
}

/// The name of the record for the page's title, and the page that shows it.
pub(crate) async fn label_of(state: &AppState, subject: &Subject) -> (String, Option<String>) {
    if subject.subject_type == wl_kernel::parties::PARTY_SUBJECT_TYPE {
        let party = wl_kernel::parties::directory(state)
            .find(&state.db, wl_kernel::PartyId(subject.subject_id))
            .await
            .ok()
            .flatten();
        if let Some(party) = party {
            return (
                format!("{} · {}", party.code, party.display_name),
                Some(format!("/parties/{}", subject.subject_id)),
            );
        }
    }
    if let Ok(Some(label)) = wl_kernel::documents::subject_label(&state.db, subject).await {
        return (label, None);
    }
    (wl_kernel::documents::default_subject_label(subject), None)
}

/// `Forbidden` unless the record is in the user's reach.
async fn ensure_reach(
    state: &AppState,
    user: &AuthUser,
    permission: &str,
    subject: &Subject,
) -> Result<()> {
    let reach = wl_kernel::scopes::reach(&state.db, user, permission).await?;
    if reach.is_all() || reach.contains(&state.db, subject).await? {
        return Ok(());
    }
    // A record that is not a scope's own (a tenant): the registered members.
    match reach.ids(&state.db, &subject.subject_type).await? {
        Some(ids) if ids.contains(&subject.subject_id) => Ok(()),
        _ => Err(Error::Forbidden),
    }
}

/// `GET /documents/of/{subject_type}/{subject_id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((subject_type, subject_id)): Path<(String, i64)>,
) -> Result<View> {
    let subject = Subject::new(&subject_type, subject_id);
    ensure_reach(&state, &user, permissions::VIEW, &subject).await?;
    let db = &state.db;
    let kinds = DocumentKind::ordered(db).await?;
    let names: BTreeMap<i64, &DocumentKind> = kinds.iter().map(|k| (k.id, k)).collect();
    let mut rows = Vec::new();
    for document in Document::query()
        .where_eq("subject_type", subject_type.as_str())
        .where_eq("subject_id", subject_id)
        .order_by("kind_id")
        .order_by_desc("id")
        .get(db)
        .await?
    {
        if matches!(
            document.status,
            DocumentStatus::Superseded | DocumentStatus::Archived
        ) {
            continue;
        }
        let file_name = DocumentVersion::query()
            .where_eq("document_id", document.id)
            .where_eq("version_no", document.current_version)
            .first(db)
            .await?
            .map(|v| v.file_name);
        rows.push(SubjectRow {
            id: document.id,
            kind: names
                .get(&document.kind_id)
                .map_or_else(String::new, |k| k.name(&lang.locale).to_owned()),
            title: document.title.clone(),
            number: document.number.clone().unwrap_or_default(),
            expiry_date: crate::pages::date_value(document.expiry_date),
            status: text(
                &lang,
                &format!("subject.status.{}", status_key(document.status)),
            ),
            status_kind: match document.status {
                DocumentStatus::Accepted => "success",
                DocumentStatus::PendingReview => "warning",
                DocumentStatus::Rejected => "error",
                _ => "neutral",
            },
            file_name,
            rejection_reason: document.rejection_reason.clone().unwrap_or_default(),
        });
    }
    let mut offered = Vec::new();
    for kind in &kinds {
        if !kind.enabled || !kind.allows_subject_type(&subject_type) {
            continue;
        }
        let label = kind.name(&lang.locale).to_owned();
        offered.push((kind.key.clone(), label));
    }
    let (label, back) = label_of(&state, &subject).await;
    Ok(View::typed(SubjectPage {
        tx: texts::all(&lang),
        label,
        back,
        documents: rows,
        kinds: offered,
        can_upload: user.has_permission_anywhere(permissions::UPLOAD),
        action: format!("/documents/of/{subject_type}/{subject_id}"),
    }))
}

fn status_key(status: DocumentStatus) -> &'static str {
    match status {
        DocumentStatus::PendingReview => "pending_review",
        DocumentStatus::Accepted => "accepted",
        DocumentStatus::Rejected => "rejected",
        DocumentStatus::Superseded => "superseded",
        DocumentStatus::Archived => "archived",
    }
}

/// The upload form (multipart).
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct SubjectForm {
    /// The kind's key.
    #[serde(default)]
    pub kind: String,
    /// The title.
    #[validate(max = 255)]
    #[serde(default)]
    pub title: Option<String>,
    /// The issuing authority.
    #[validate(max = 255)]
    #[serde(default)]
    pub authority: Option<String>,
    /// The number.
    #[validate(max = 255)]
    #[serde(default)]
    pub number: Option<String>,
    /// The issue date, `YYYY-MM-DD`.
    #[serde(default)]
    pub issue_date: Option<String>,
    /// The expiry date, `YYYY-MM-DD`.
    #[serde(default)]
    pub expiry_date: Option<String>,
    /// A note on the file.
    #[validate(max = 2000)]
    #[serde(default)]
    pub note: Option<String>,
    /// The file.
    #[serde(default)]
    pub files: Vec<Upload>,
}

/// `POST /documents/of/{subject_type}/{subject_id}`: attaches the file for
/// review. A missing kind or file, a bad date or a refusal (the kind does not
/// take this record, a bad file) is a 422 under its field.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    htmx: Htmx,
    Path((subject_type, subject_id)): Path<(String, i64)>,
    Valid(form): Valid<SubjectForm>,
) -> Result<Response> {
    let subject = Subject::new(&subject_type, subject_id);
    ensure_reach(&state, &user, permissions::UPLOAD, &subject).await?;
    if form.kind.trim().is_empty() {
        return Err(invalid("kind", text(&lang, "subject.kind_required"), &form));
    }
    let Some(file) = form.files.first().cloned() else {
        return Err(invalid(
            "files",
            text(&lang, "correction.file_required"),
            &form,
        ));
    };
    let mut new = NewDocument::new(subject, form.kind.trim(), file);
    new.title = blank_to_none(form.title.as_deref());
    new.number = blank_to_none(form.number.as_deref());
    new.issuing_authority = blank_to_none(form.authority.as_deref());
    new.note = blank_to_none(form.note.as_deref());
    for (field, value, target) in [
        ("issue_date", &form.issue_date, &mut new.issue_date),
        ("expiry_date", &form.expiry_date, &mut new.expiry_date),
    ] {
        if let Some(value) = value {
            match parse_date(value) {
                Some(date) => *target = date,
                None => return Err(invalid(field, text(&lang, "form.date_invalid"), &form)),
            }
        }
    }
    match Documents::attach(&state, Some(&user), new).await? {
        // A multipart form is sent by the browser itself: back to the page.
        Ok(_) => Ok((
            Toast::success(text(&lang, "subject.uploaded")),
            htmx.redirect(&format!("/documents/of/{subject_type}/{subject_id}")),
        )
            .into_response()),
        Err(refused) => Err(invalid("kind", refused.message(&lang), &form)),
    }
}
