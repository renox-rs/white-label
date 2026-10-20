//! The SOP pages (wl-documents §3 *SOP*, #52 A13):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /documents/sops` | `wl_documents.sops` | `documents.view` or `sops.manage` |
//! | `GET /documents/sops/new` | `wl_documents.sops.new` | `sops.manage` |
//! | `POST /documents/sops` | `wl_documents.sops.store` | `sops.manage` |
//! | `GET /documents/sops/{id}` | `wl_documents.sop` | `documents.view` or `sops.manage` |
//! | `GET /documents/sop-files/{id}` | `wl_documents.sops.file` | `documents.view` or `sops.manage` |
//! | `POST /documents/sops/{id}/readers` | `wl_documents.sops.readers` | `sops.manage` |
//! | `POST /documents/sops/{id}/versions` | `wl_documents.sops.versions.store` | `sops.manage` (multipart) |
//! | `POST /documents/sops/{id}/withdraw` | `wl_documents.sops.withdraw` | `sops.manage` |
//! | `POST /documents/sop-versions/{id}/accept` | `wl_documents.sops.versions.accept` | `sops.manage` and the owner role |
//! | `POST /documents/sop-versions/{id}/reject` | `wl_documents.sops.versions.reject` | `sops.manage` and the owner role |
//! | `POST /documents/sop-versions/{id}/translations` | `wl_documents.sops.translations.store` | `sops.manage` (multipart) |
//! | `POST /documents/sop-translations/{id}/accept` | `wl_documents.sops.translations.accept` | `sops.manage` and the owner role |
//! | `POST /documents/sop-translations/{id}/reject` | `wl_documents.sops.translations.reject` | `sops.manage` and the owner role |
//!
//! The pages only call [`Sops`], which keeps every rule (owner role,
//! `sops.manage`); the buttons are hidden for a viewer who could not use
//! them. Times are shown in Asia/Dubai.

use std::collections::BTreeMap;

use renox::Download;
use renox::axum::extract::Query;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, TransitionError};
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::SopReaderRef;

use super::model::{Sop, SopStatus};
use super::versions::holds_owner_role;
use super::{KIND_SOP, SUBJECT_TYPE, Sops};
use crate::acknowledgements::web::{VersionReport, report as acknowledgement_report};
use crate::documents::{Document, DocumentStatus};
use crate::kinds::DocumentKind;
use crate::overrides::web::dubai;
use crate::permissions;
use crate::{SopInput, texts};

/// The texts a template reads as `tx['fields.kind']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's SOP routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/documents/sops", index)
        .name("wl_documents.sops")
        .get("/documents/sops/new", new)
        .name("wl_documents.sops.new")
        .post("/documents/sops", store)
        .name("wl_documents.sops.store")
        .get("/documents/sop-files/{id}", file)
        .name("wl_documents.sops.file")
        .get("/documents/sops/{id}", show)
        .name("wl_documents.sop")
        .post("/documents/sops/{id}/readers", set_readers)
        .name("wl_documents.sops.readers")
        .post("/documents/sops/{id}/versions", upload_version)
        .name("wl_documents.sops.versions.store")
        .post("/documents/sops/{id}/withdraw", withdraw)
        .name("wl_documents.sops.withdraw")
        .post("/documents/sop-versions/{id}/accept", accept_version)
        .name("wl_documents.sops.versions.accept")
        .post("/documents/sop-versions/{id}/reject", reject_version)
        .name("wl_documents.sops.versions.reject")
        .post(
            "/documents/sop-versions/{id}/translations",
            upload_translation,
        )
        .name("wl_documents.sops.translations.store")
        .post(
            "/documents/sop-translations/{id}/accept",
            accept_translation,
        )
        .name("wl_documents.sops.translations.accept")
        .post(
            "/documents/sop-translations/{id}/reject",
            reject_translation,
        )
        .name("wl_documents.sops.translations.reject")
        .require_auth()
}

/// The package text `key` (without the `wl_documents.` prefix) in `lang`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_documents.{key}"))
}

/// A 422 under `field`, keeping what was typed.
fn invalid(field: &str, message: String, input: &impl Serialize) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    ValidationError::new(errors).with_input(input).into()
}

/// Whether `user` may see the SOP pages.
fn may_view(user: &AuthUser) -> bool {
    full_access(user)
        || user.has_permission_anywhere(permissions::VIEW)
        || user.has_permission_anywhere(permissions::SOPS_MANAGE)
}

/// Whether `user` sees every SOP, drafts included: a company-wide holder. A
/// holder in some buildings only reads the active and withdrawn ones; the
/// SOPs are the company's, not a building's.
fn full_access(user: &AuthUser) -> bool {
    user.has_permission(permissions::SOPS_MANAGE) || user.has_permission(permissions::VIEW)
}

/// `Forbidden` unless `user` may see the SOP pages.
fn require_view(user: &AuthUser) -> Result<()> {
    if may_view(user) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// The title in the page's language.
fn title_of(sop: &Sop, lang: &Lang) -> String {
    if lang.locale == "ar" && !sop.title_ar.is_empty() {
        sop.title_ar.clone()
    } else {
        sop.title_en.clone()
    }
}

/// The badge colour of an SOP status.
fn status_kind(status: SopStatus) -> &'static str {
    match status {
        SopStatus::Draft => "neutral",
        SopStatus::Active => "success",
        SopStatus::Withdrawn => "warning",
    }
}

/// The badge colour of a document status.
fn document_status_kind(status: DocumentStatus) -> &'static str {
    match status {
        DocumentStatus::PendingReview => "warning",
        DocumentStatus::Accepted => "success",
        DocumentStatus::Rejected => "error",
        DocumentStatus::Superseded | DocumentStatus::Archived => "neutral",
    }
}

/// A reader in words: `All staff`, `site_worker` or `site_worker (site #4)`.
fn reader_label(lang: &Lang, reader: &SopReaderRef) -> String {
    if reader.all_staff {
        text(lang, "sops.readers.all_staff")
    } else if reader.scope_type.is_empty() {
        reader.role.clone()
    } else {
        format!(
            "{} ({} #{})",
            reader.role, reader.scope_type, reader.scope_id
        )
    }
}

/// The readers as the textarea's lines: `all`, `role` or `role@type:id`.
fn readers_text(readers: &[SopReaderRef]) -> String {
    readers
        .iter()
        .map(|r| {
            if r.all_staff {
                "all".to_owned()
            } else if r.scope_type.is_empty() {
                r.role.clone()
            } else {
                format!("{}@{}:{}", r.role, r.scope_type, r.scope_id)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The readers of the textarea's lines (or commas); the inverse of
/// [`readers_text`]. A line the service finds wrong is refused there.
pub fn parse_readers(text: &str) -> Vec<SopReaderRef> {
    text.split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if line.eq_ignore_ascii_case("all") {
                return SopReaderRef {
                    all_staff: true,
                    role: String::new(),
                    scope_type: String::new(),
                    scope_id: String::new(),
                };
            }
            let (role, scope) = line.split_once('@').unwrap_or((line, ""));
            let (scope_type, scope_id) = scope.split_once(':').unwrap_or((scope, ""));
            SopReaderRef {
                all_staff: false,
                role: role.trim().to_owned(),
                scope_type: scope_type.trim().to_owned(),
                scope_id: scope_id.trim().to_owned(),
            }
        })
        .collect()
}

/// One SOP in the library.
#[derive(Serialize, Debug, Clone)]
pub struct SopRow {
    /// Its page.
    pub url: String,
    /// The code.
    pub code: String,
    /// The title in the page's language.
    pub title: String,
    /// The category.
    pub category: String,
    /// The owner role.
    pub owner_role: String,
    /// The required readers, in words.
    pub readers: String,
    /// The status, in words.
    pub status_label: String,
    /// The status badge's colour.
    pub status_kind: &'static str,
    /// The current version, e.g. `v2`.
    pub version: Option<String>,
    /// The review state, in words.
    pub review_label: String,
    /// The review badge's colour.
    pub review_kind: &'static str,
    /// The review date, while accepted.
    pub review_date: Option<String>,
}

/// What the library needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/sops.html")]
pub struct SopsPage {
    tx: Texts,
    sops: Vec<SopRow>,
    /// The status filter now applied (`all` for none).
    filter: String,
    can_manage: bool,
}

/// The library's query.
#[derive(Deserialize, Debug, Default)]
pub struct IndexQuery {
    /// `draft`, `active` or `withdrawn`; anything else lists all.
    #[serde(default)]
    pub status: String,
}

/// The current version document of `sop`.
async fn current_of(db: &Db, sop: &Sop) -> Result<Option<Document>> {
    match sop.current_document_id {
        Some(id) => Document::find(db, id).await,
        None => Ok(None),
    }
}

/// `GET /documents/sops?status=`: the library, by code.
async fn index(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<IndexQuery>,
) -> Result<View> {
    require_view(&user)?;
    let wanted = match query.status.as_str() {
        "draft" => Some(SopStatus::Draft),
        "active" => Some(SopStatus::Active),
        "withdrawn" => Some(SopStatus::Withdrawn),
        _ => None,
    };
    let today = wl_kernel::documents::dubai_date(renox::db::now());
    let mut rows = Vec::new();
    for sop in Sops::list(&db, wanted).await? {
        let status = Sops::status(&db, &sop).await?;
        if status == SopStatus::Draft && !full_access(&user) {
            continue;
        }
        let review = Sops::review_state(&db, &sop, today).await?;
        let current = current_of(&db, &sop).await?;
        let version = match &current {
            Some(document) => Some(format!("v{}", Sops::version_number(&db, document).await?)),
            None => None,
        };
        let readers = Sops::readers(&db, sop.id).await?;
        let summary = readers
            .iter()
            .map(|r| reader_label(&lang, r))
            .collect::<Vec<_>>()
            .join(", ");
        rows.push(SopRow {
            url: format!("/documents/sops/{}", sop.id),
            code: sop.code.clone(),
            title: title_of(&sop, &lang),
            category: sop.category.clone(),
            owner_role: sop.owner_role.clone(),
            readers: if summary.is_empty() {
                text(&lang, "none")
            } else {
                summary
            },
            status_label: text(&lang, &format!("sops.status.{}", status.key())),
            status_kind: status_kind(status),
            version,
            review_label: text(&lang, &format!("sops.review.{}", review.key())),
            review_kind: review.tone(),
            review_date: current
                .filter(|d| d.status == DocumentStatus::Accepted)
                .and_then(|d| d.expiry_date)
                .map(|date| date.format("%Y-%m-%d").to_string()),
        });
    }
    Ok(View::typed(SopsPage {
        tx: texts::all(&lang),
        sops: rows,
        filter: wanted.map_or_else(|| "all".to_owned(), |s| s.key().to_owned()),
        can_manage: user.has_permission(permissions::SOPS_MANAGE),
    }))
}

/// What the create form needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/sop_form.html")]
pub struct SopFormPage {
    tx: Texts,
}

/// `GET /documents/sops/new`: the create form.
async fn new(user: AuthUser, lang: Lang) -> Result<View> {
    if !user.has_permission(permissions::SOPS_MANAGE) {
        return Err(Error::Forbidden);
    }
    Ok(View::typed(SopFormPage {
        tx: texts::all(&lang),
    }))
}

/// The create form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct SopForm {
    /// The code, e.g. `SOP-HSE-01`.
    #[validate(required, max = 64)]
    pub code: String,
    /// The English title.
    #[validate(required, max = 200)]
    pub title_en: String,
    /// The Arabic title.
    #[validate(max = 200)]
    #[serde(default)]
    pub title_ar: String,
    /// The category.
    #[validate(max = 64)]
    #[serde(default)]
    pub category: String,
    /// The role that owns the SOP.
    #[validate(required, max = 128)]
    pub owner_role: String,
    /// The legal entity's type; empty with no id for every entity.
    #[validate(max = 64)]
    #[serde(default)]
    pub legal_entity_type: String,
    /// The legal entity's id.
    #[serde(default)]
    pub legal_entity_id: Option<i64>,
    /// Months between reviews; 12 when empty.
    #[serde(default)]
    pub review_interval_months: Option<i64>,
    /// Days to acknowledge a new version; 7 when empty.
    #[serde(default)]
    pub ack_due_days: Option<i64>,
}

/// The service input of `form`.
fn input_of(form: &SopForm) -> SopInput {
    let mut input = SopInput::new(&form.code, &form.title_en, &form.owner_role)
        .title_ar(&form.title_ar)
        .category(&form.category);
    let entity_id = form.legal_entity_id.unwrap_or(0);
    if !form.legal_entity_type.trim().is_empty() || entity_id != 0 {
        input = input.legal_entity(&form.legal_entity_type, entity_id);
    }
    if let Some(months) = form.review_interval_months {
        input = input.review_every(months);
    }
    if let Some(days) = form.ack_due_days {
        input = input.ack_due_days(days);
    }
    input
}

/// `POST /documents/sops`: creates the SOP and opens its page. A refusal of
/// the service is a 422 under the field it is about.
async fn store(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<SopForm>,
) -> Result<Response> {
    match Sops::create(&state, &user, input_of(&form)).await {
        Ok(sop) => Ok((
            Toast::success(text(&lang, "sops.created")),
            Redirect::route("wl_documents.sop", &[&sop.id])?,
        )
            .into_response()),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            Err(invalid("code", message, &form))
        }
        Err(other) => Err(other),
    }
}

/// One version or translation on the details page.
#[derive(Serialize, Debug, Clone)]
pub struct DocumentRow {
    /// The document's id.
    pub id: i64,
    /// `v2`, or the language of a translation.
    pub label: String,
    /// The status, in words.
    pub status_label: String,
    /// The status badge's colour.
    pub status_kind: &'static str,
    /// Who uploaded it.
    pub uploaded_by: String,
    /// When it was submitted (Asia/Dubai).
    pub submitted_at: Option<String>,
    /// The review date, for an accepted version.
    pub review_date: Option<String>,
    /// The name of its file, if stored.
    pub file_name: Option<String>,
    /// Its file's address.
    pub file_url: String,
    /// Whether the viewer may accept or reject it now.
    pub can_decide: bool,
    /// Whether the viewer may add a translation of it.
    pub can_translate: bool,
    /// Its translations (versions only).
    pub translations: Vec<DocumentRow>,
}

/// What the details page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/sop.html")]
pub struct SopPage {
    tx: Texts,
    id: i64,
    title: String,
    code: String,
    category: String,
    owner_role: String,
    entity: String,
    review_interval_months: i64,
    ack_due_days: i64,
    status_label: String,
    status_kind: &'static str,
    version: Option<String>,
    review_label: String,
    review_kind: &'static str,
    review_date: Option<String>,
    readers: Vec<String>,
    readers_text: String,
    versions: Vec<DocumentRow>,
    timeline: Vec<Value>,
    /// The acknowledgements per version (`DOC.RUL.04`).
    acks: Vec<VersionReport>,
    can_manage: bool,
    can_withdraw: bool,
    can_readers: bool,
}

/// The names of users, looked up once per page.
async fn user_name(db: &Db, id: Option<i64>, lang: &Lang) -> Result<String> {
    Ok(match id {
        Some(id) => User::find(db, id)
            .await?
            .map_or_else(|| format!("#{id}"), |user| user.name),
        None => text(lang, "none"),
    })
}

/// The row of `document`.
async fn document_row(
    db: &Db,
    lang: &Lang,
    label: String,
    document: &Document,
    can_decide: bool,
) -> Result<DocumentRow> {
    let file = document.current_file(db).await?;
    Ok(DocumentRow {
        id: document.id,
        label,
        status_label: text(
            lang,
            &format!("sops.doc_status.{}", document.status.as_str()),
        ),
        status_kind: document_status_kind(document.status),
        uploaded_by: user_name(db, document.uploaded_by, lang).await?,
        submitted_at: document.submitted_at.map(dubai),
        review_date: (document.status == DocumentStatus::Accepted)
            .then_some(document.expiry_date)
            .flatten()
            .map(|date| date.format("%Y-%m-%d").to_string()),
        file_name: file.map(|f| f.file_name),
        file_url: format!("/documents/sop-files/{}", document.id),
        can_decide,
        can_translate: false,
        translations: Vec::new(),
    })
}

/// `GET /documents/sops/{id}`: the SOP, its versions with their
/// translations, readers and history; 403 for anyone who may not see SOPs.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(sop): Found<Sop>,
) -> Result<View> {
    require_view(&user)?;
    let db = state.db.clone();
    if !full_access(&user) && Sops::status(&db, &sop).await? == SopStatus::Draft {
        return Err(Error::NotFound);
    }
    let manage = user.has_permission(permissions::SOPS_MANAGE);
    let owner = manage && holds_owner_role(&db, &user, &sop).await?;
    let today = wl_kernel::documents::dubai_date(renox::db::now());
    let status = Sops::status(&db, &sop).await?;
    let review = Sops::review_state(&db, &sop, today).await?;
    let current = current_of(&db, &sop).await?;
    let readers = Sops::readers(&db, sop.id).await?;

    let mut versions = Vec::new();
    let mut timeline: Vec<Value> = Vec::new();
    let info = state_machine::info_of::<Document>();
    let mut documents = Vec::new();
    if let Some(kind) = DocumentKind::by_key(&db, KIND_SOP).await? {
        documents = Document::query()
            .where_(Document::SUBJECT_TYPE.eq(SUBJECT_TYPE))
            .where_(Document::SUBJECT_ID.eq(sop.id))
            .where_(Document::KIND_ID.eq(kind.id))
            .order_by_desc(Document::ID)
            .get(&db)
            .await?;
    }
    for document in &documents {
        let number = Sops::version_number(&db, document).await?;
        let pending = document.status == DocumentStatus::PendingReview;
        let mut row =
            document_row(&db, &lang, format!("v{number}"), document, owner && pending).await?;
        row.can_translate = manage
            && matches!(
                document.status,
                DocumentStatus::PendingReview | DocumentStatus::Accepted
            );
        let current_accepted = sop.current_document_id == Some(document.id)
            && document.status == DocumentStatus::Accepted;
        add_history(&db, document, &format!("v{number}"), &info, &mut timeline).await?;
        for link in Sops::translations(&db, document.id).await? {
            let Some(translation) = Document::find(&db, link.document_id).await? else {
                continue;
            };
            let decide =
                owner && current_accepted && translation.status == DocumentStatus::PendingReview;
            let label = format!("v{number} {}", link.language);
            row.translations
                .push(document_row(&db, &lang, link.language.clone(), &translation, decide).await?);
            add_history(&db, &translation, &label, &info, &mut timeline).await?;
        }
        versions.push(row);
    }
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    for item in &mut timeline {
        let shown = item["time"]
            .as_str()
            .and_then(|time| renox::chrono::DateTime::parse_from_rfc3339(time).ok())
            .map(|time| dubai(time.with_timezone(&renox::chrono::Utc)));
        if let Some(shown) = shown {
            item["time"] = Value::String(shown);
        }
    }

    let version = match &current {
        Some(document) => Some(format!("v{}", Sops::version_number(&db, document).await?)),
        None => None,
    };
    Ok(View::typed(SopPage {
        tx: texts::all(&lang),
        id: sop.id,
        title: title_of(&sop, &lang),
        code: sop.code.clone(),
        category: sop.category.clone(),
        owner_role: sop.owner_role.clone(),
        entity: match sop.legal_entity() {
            Some(id) => format!("{} #{id}", sop.legal_entity_type),
            None => text(&lang, "sops.every_entity"),
        },
        review_interval_months: sop.review_interval_months,
        ack_due_days: sop.ack_due_days,
        status_label: text(&lang, &format!("sops.status.{}", status.key())),
        status_kind: status_kind(status),
        version,
        review_label: text(&lang, &format!("sops.review.{}", review.key())),
        review_kind: review.tone(),
        review_date: current
            .filter(|d| d.status == DocumentStatus::Accepted)
            .and_then(|d| d.expiry_date)
            .map(|date| date.format("%Y-%m-%d").to_string()),
        readers: readers.iter().map(|r| reader_label(&lang, r)).collect(),
        readers_text: readers_text(&readers),
        versions,
        timeline,
        acks: acknowledgement_report(&state, &lang, &sop, today).await?,
        can_manage: manage,
        can_withdraw: manage && status == SopStatus::Active,
        can_readers: manage && status != SopStatus::Withdrawn,
    }))
}

/// Adds the machine's history of `document` to `timeline`, each title
/// starting with `label`.
async fn add_history(
    db: &Db,
    document: &Document,
    label: &str,
    info: &state_machine::MachineInfo,
    timeline: &mut Vec<Value>,
) -> Result<()> {
    let history = state_machine::history(db, Document::TABLE, document.id).await?;
    for mut item in state_machine::timeline(&history, Some(info)) {
        let title = item["title"].as_str().unwrap_or_default().to_owned();
        item["title"] = Value::String(format!("{label}: {title}"));
        timeline.push(item);
    }
    Ok(())
}

/// `GET /documents/sop-files/{id}`: the file of a version or translation of
/// an SOP, shown inline. 404 for any other document.
async fn file(
    State(state): State<AppState>,
    user: AuthUser,
    Found(document): Found<Document>,
) -> Result<Download> {
    require_view(&user)?;
    if document.subject_type != SUBJECT_TYPE {
        return Err(Error::NotFound);
    }
    let version = document
        .current_file(&state.db)
        .await?
        .ok_or(Error::NotFound)?;
    let bytes = version.read(&state).await?.ok_or(Error::NotFound)?;
    Ok(Download::bytes(version.file_name, version.mime_type, bytes.to_vec()).inline())
}

/// The readers form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReadersForm {
    /// One reader per line: `all`, `role` or `role@type:id`.
    #[validate(max = 4000)]
    #[serde(default)]
    pub readers: String,
}

/// A refusal of a transition as a 422 under `field`; the rest unchanged.
fn refusal(error: TransitionError, field: &str, input: &impl Serialize) -> Error {
    match error {
        TransitionError::Guard(_)
        | TransitionError::NotFrom(_)
        | TransitionError::ReasonRequired => invalid(field, error.to_string(), input),
        other => other.into_error(),
    }
}

/// `POST /documents/sops/{id}/readers`: saves the required readers and
/// reloads the page.
async fn set_readers(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReadersForm>,
) -> Result<Response> {
    match Sops::set_readers(&state, &user, id, &parse_readers(&form.readers)).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "sops.readers.saved")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            Err(invalid("readers", message, &form))
        }
        Err(other) => Err(other),
    }
}

/// The upload form of a version.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct VersionForm {
    /// A note for the reviewer.
    #[validate(max = 2000)]
    #[serde(default)]
    pub note: String,
    /// The file.
    #[serde(default)]
    pub files: Vec<Upload>,
}

/// `POST /documents/sops/{id}/versions`: uploads a new version (multipart)
/// and reloads the page. A bad or missing file is a 422 under `files`.
async fn upload_version(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<VersionForm>,
) -> Result<Response> {
    let Some(file) = form.files.first().cloned() else {
        return Err(invalid("files", text(&lang, "sops.file_required"), &form));
    };
    match Sops::upload_version(&state, &user, id, file, Some(&form.note)).await? {
        Ok(_) => Ok((
            Toast::success(text(&lang, "sops.version_uploaded")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(refused) => Err(invalid("files", refused.message(&lang), &form)),
    }
}

/// The form of the withdraw action.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReasonForm {
    /// Why; required to reject, optional to withdraw.
    #[validate(max = 2000)]
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /documents/sops/{id}/withdraw`: withdraws the SOP and reloads.
async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    match Sops::withdraw(&state, &user, id, form.reason.as_deref()).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "sops.withdrawn")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(error) => Err(refusal(error, "reason", &form)),
    }
}

/// The form of the accept actions (nothing to fill in).
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct AcceptForm {}

/// `POST /documents/sop-versions/{id}/accept`: accepts the version.
async fn accept_version(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(document): Found<Document>,
    Valid(form): Valid<AcceptForm>,
) -> Result<Response> {
    match Sops::accept_version(&state, &user, &document).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "sops.accepted")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(error) => Err(refusal(error, "document", &form)),
    }
}

/// `POST /documents/sop-versions/{id}/reject`: rejects the version for a
/// reason.
async fn reject_version(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(document): Found<Document>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    let reason = form.reason.as_deref().map_or("", str::trim);
    if reason.is_empty() {
        return Err(invalid(
            "reason",
            text(&lang, "review.reason_required"),
            &form,
        ));
    }
    match Sops::reject_version(&state, &user, &document, reason).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "sops.rejected")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(error) => Err(refusal(error, "reason", &form)),
    }
}

/// The upload form of a translation.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct TranslationForm {
    /// The language, e.g. `ar`.
    #[validate(required, max = 16)]
    pub language: String,
    /// The file.
    #[serde(default)]
    pub files: Vec<Upload>,
}

/// `POST /documents/sop-versions/{id}/translations`: uploads a translation
/// of the version (multipart) and reloads the page.
async fn upload_translation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(version): Found<Document>,
    Valid(form): Valid<TranslationForm>,
) -> Result<Response> {
    let Some(file) = form.files.first().cloned() else {
        return Err(invalid("files", text(&lang, "sops.file_required"), &form));
    };
    match Sops::upload_translation(&state, &user, &version, &form.language, file, None).await {
        Ok(Ok(_)) => Ok((
            Toast::success(text(&lang, "sops.translation_uploaded")),
            renox::HxRefresh,
        )
            .into_response()),
        Ok(Err(refused)) => Err(invalid("files", refused.message(&lang), &form)),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            Err(invalid("language", message, &form))
        }
        Err(other) => Err(other),
    }
}

/// `POST /documents/sop-translations/{id}/accept`: accepts the translation.
async fn accept_translation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(document): Found<Document>,
    Valid(form): Valid<AcceptForm>,
) -> Result<Response> {
    match Sops::accept_translation(&state, &user, &document).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "sops.accepted")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(error) => Err(refusal(error, "document", &form)),
    }
}

/// `POST /documents/sop-translations/{id}/reject`: rejects the translation.
async fn reject_translation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(document): Found<Document>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    let reason = form.reason.as_deref().map_or("", str::trim);
    if reason.is_empty() {
        return Err(invalid(
            "reason",
            text(&lang, "review.reason_required"),
            &form,
        ));
    }
    match Sops::reject_translation(&state, &user, &document, reason).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "sops.rejected")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(error) => Err(refusal(error, "reason", &form)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_round_trip_through_the_textarea() {
        let readers = parse_readers("all\nsite_worker@site:4, hse_officer ;  ");
        assert_eq!(readers.len(), 3);
        assert!(readers[0].all_staff);
        assert_eq!(readers[1].role, "site_worker");
        assert_eq!(readers[1].scope_type, "site");
        assert_eq!(readers[1].scope_id, "4");
        assert_eq!(readers[2].role, "hse_officer");
        assert_eq!(
            readers_text(&readers),
            "all\nsite_worker@site:4\nhse_officer"
        );
    }
}
