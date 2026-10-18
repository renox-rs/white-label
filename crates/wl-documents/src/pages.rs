//! The back-office pages (wl-documents §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /documents/review` | `wl_documents.review` | `documents.review` |
//! | `GET /documents/{id}/file` | `wl_documents.file` | `documents.review` |
//! | `POST /documents/{id}/accept` | `wl_documents.accept` | `documents.review` |
//! | `POST /documents/{id}/reject` | `wl_documents.reject` | `documents.review` (`reason` required) |
//! | `GET /document-kinds` | `wl_documents.kinds` | `document_kinds.manage` |
//! | `GET /document-kinds/{key}` | `wl_documents.kind` | `document_kinds.manage` |
//! | `POST /document-kinds/{key}/blocks` | `wl_documents.blocks.store` | `document_kinds.manage` |
//! | `POST /document-kinds/{key}/blocks/{guarded}/mode` | `wl_documents.blocks.mode` | `document_kinds.manage` |
//! | `DELETE /document-kinds/{key}/blocks/{guarded}` | `wl_documents.blocks.destroy` | `document_kinds.manage` |
//!
//! The review queue lists the documents in `pending_review`: first those a
//! refused transition waits for (`review_priority_at`, newest first), then
//! the others by the date they were submitted, oldest first. The kind page
//! changes `blocks` only through [`crate::kinds`], so a locked (legal-block)
//! entry is refused there too, with its message; the page shows that
//! message instead of the actions. Every template is compiled in and can
//! be replaced by the app (`resources/views/wl-documents/…`).

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use renox::Download;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use renox::validation::{Errors, ValidationError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{BlockCondition, BlockMode, OverrideMode};

use crate::documents::{Document, DocumentStatus, DocumentVersion, Documents};
use crate::kinds::{self, BlockChangeRefused, DocumentKind, KindBlock};
use crate::permissions;
use crate::texts;

/// The texts a template reads as `tx['fields.kind']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_documents.` prefix) with its
/// placeholders filled in.
type TextWith<'a> = &'a dyn Fn(&str, &[(&str, &dyn std::fmt::Display)]) -> String;

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/documents/review", review_queue_page)
        .name("wl_documents.review")
        .get("/documents/{id}/file", file)
        .name("wl_documents.file")
        .post("/documents/{id}/accept", accept)
        .name("wl_documents.accept")
        .post("/documents/{id}/reject", reject)
        .name("wl_documents.reject")
        .require_permission(permissions::REVIEW)
        .merge(
            Routes::new()
                .get("/document-kinds", kinds_index)
                .name("wl_documents.kinds")
                .get("/document-kinds/{key}", kind_page)
                .name("wl_documents.kind")
                .post("/document-kinds/{key}/blocks", store_block)
                .name("wl_documents.blocks.store")
                .post("/document-kinds/{key}/blocks/{guarded}/mode", update_mode)
                .name("wl_documents.blocks.mode")
                .delete("/document-kinds/{key}/blocks/{guarded}", destroy_block)
                .name("wl_documents.blocks.destroy")
                .require_permission(permissions::KINDS_MANAGE),
        )
}

/// The package text `key` (without the `wl_documents.` prefix) in `lang`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_documents.{key}"))
}

/// The review order: documents a refused transition waits for first
/// (`review_priority_at`, newest first), then by submission, oldest first.
pub fn queue_order(a: &Document, b: &Document) -> Ordering {
    b.review_priority_at
        .cmp(&a.review_priority_at)
        .then_with(|| match (a.submitted_at, b.submitted_at) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| a.id.cmp(&b.id))
}

/// The documents in `pending_review`, in review order ([`queue_order`]).
pub async fn review_queue(db: &Db) -> Result<Vec<Document>> {
    let mut documents = Document::query()
        .where_(Document::STATUS.eq(DocumentStatus::PendingReview))
        .get(db)
        .await?;
    documents.sort_by(queue_order);
    Ok(documents)
}

/// One document as the review queue shows it.
#[derive(Serialize, Debug, Clone)]
pub struct QueueRow {
    /// The document's id.
    pub id: i64,
    /// The kind's name in the page's language.
    pub kind: String,
    /// The record it is attached to, e.g. `employee #12`.
    pub subject: String,
    /// The document's title.
    pub title: String,
    /// Its number, if any.
    pub number: Option<String>,
    /// Its expiry date, if any.
    pub expiry_date: Option<NaiveDate>,
    /// The version under review.
    pub version: i64,
    /// The file name of that version, if it is stored.
    pub file_name: Option<String>,
    /// Who uploaded it, or `—`.
    pub uploaded_by: String,
    /// When it was submitted.
    pub submitted_at: Option<DateTime>,
    /// Since when a refused transition waits for it, if one does.
    pub priority_since: Option<DateTime>,
}

/// What the review queue needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/review_queue.html")]
pub(crate) struct ReviewQueuePage {
    tx: Texts,
    documents: Vec<QueueRow>,
}

/// The queue rows of `documents`, in their order.
async fn queue_rows(db: &Db, lang: &Lang, documents: Vec<Document>) -> Result<Vec<QueueRow>> {
    let kinds: HashMap<i64, DocumentKind> = DocumentKind::query()
        .get(db)
        .await?
        .into_iter()
        .map(|kind| (kind.id, kind))
        .collect();
    let ids: Vec<i64> = documents.iter().map(|d| d.id).collect();
    let files: HashMap<(i64, i64), String> = if ids.is_empty() {
        HashMap::new()
    } else {
        DocumentVersion::query()
            .where_(DocumentVersion::DOCUMENT_ID.is_in(ids))
            .get(db)
            .await?
            .into_iter()
            .map(|v| ((v.document_id, v.version_no), v.file_name))
            .collect()
    };
    let mut names: HashMap<i64, String> = HashMap::new();
    let mut rows = Vec::with_capacity(documents.len());
    for document in documents {
        let uploaded_by = match document.uploaded_by {
            Some(id) => match names.get(&id) {
                Some(name) => name.clone(),
                None => {
                    let name = User::find(db, id)
                        .await?
                        .map_or_else(|| format!("#{id}"), |user| user.name);
                    names.insert(id, name.clone());
                    name
                }
            },
            None => text(lang, "none"),
        };
        let kind = kinds
            .get(&document.kind_id)
            .map_or_else(String::new, |k| k.name(&lang.locale).to_owned());
        rows.push(QueueRow {
            id: document.id,
            kind,
            subject: format!("{} #{}", document.subject_type, document.subject_id),
            file_name: files.get(&(document.id, document.current_version)).cloned(),
            title: document.title,
            number: document.number,
            expiry_date: document.expiry_date,
            version: document.current_version,
            uploaded_by,
            submitted_at: document.submitted_at,
            priority_since: document.review_priority_at,
        });
    }
    Ok(rows)
}

/// `GET /documents/review`: the documents waiting for review, priority
/// first, with the accept and reject actions.
async fn review_queue_page(State(db): State<Db>, lang: Lang) -> Result<View> {
    let documents = review_queue(&db).await?;
    Ok(View::typed(ReviewQueuePage {
        tx: texts::all(&lang),
        documents: queue_rows(&db, &lang, documents).await?,
    }))
}

/// `GET /documents/{id}/file`: the current version's file, shown inline.
async fn file(State(state): State<AppState>, Found(document): Found<Document>) -> Result<Download> {
    let version = document
        .current_file(&state.db)
        .await?
        .ok_or(Error::NotFound)?;
    let bytes = version.read(&state).await?.ok_or(Error::NotFound)?;
    Ok(Download::bytes(version.file_name, version.mime_type, bytes.to_vec()).inline())
}

/// A 422 with `message` under `field`, keeping what was typed.
fn invalid(field: &str, message: String, input: &impl Serialize) -> Error {
    let mut errors = Errors::new();
    errors.add(field, message);
    ValidationError::new(errors).with_input(input).into()
}

/// The form of the accept action (nothing to fill in).
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct AcceptForm {}

/// `POST /documents/{id}/accept`: accepts the document (the previous
/// accepted one of its kind and subject is superseded) and reloads the
/// queue. A refusal (not pending, a number or expiry date missing) is a
/// 422 with the message.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(document): Found<Document>,
    Valid(form): Valid<AcceptForm>,
) -> Result<Response> {
    match Documents::accept(&state, &user, &document).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "review.accepted")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(error @ (TransitionError::Guard(_) | TransitionError::NotFrom(_))) => {
            Err(invalid("document", error.to_string(), &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The form of the reject action.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RejectForm {
    /// Why, for the uploader.
    #[validate(max = 2000)]
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /documents/{id}/reject`: rejects the document for `reason` and
/// reloads the queue; a blank reason is a 422 under the field.
async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(document): Found<Document>,
    Valid(form): Valid<RejectForm>,
) -> Result<Response> {
    let reason = form.reason.as_deref().map_or("", str::trim);
    if reason.is_empty() {
        return Err(invalid(
            "reason",
            text(&lang, "review.reason_required"),
            &form,
        ));
    }
    match Documents::reject(&state, &user, &document, reason).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "review.rejected")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(
            error @ (TransitionError::Guard(_)
            | TransitionError::NotFrom(_)
            | TransitionError::ReasonRequired),
        ) => Err(invalid("reason", error.to_string(), &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// One kind as the kinds list shows it.
#[derive(Serialize, Debug, Clone)]
pub struct KindRow {
    /// The kind's key.
    pub key: String,
    /// Its name in the page's language.
    pub name: String,
    /// The subject types, e.g. `employee, vehicle`.
    pub subject_types: String,
    /// Whether it is a legal block.
    pub legal_block: bool,
}

/// What the kinds list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/kinds.html")]
pub(crate) struct KindsPage {
    tx: Texts,
    kinds: Vec<KindRow>,
}

/// `GET /document-kinds`: every kind, linking to its page.
async fn kinds_index(State(db): State<Db>, lang: Lang) -> Result<View> {
    let kinds = DocumentKind::ordered(&db)
        .await?
        .iter()
        .map(|kind| KindRow {
            key: kind.key.clone(),
            name: kind.name(&lang.locale).to_owned(),
            subject_types: kind.subject_types.0.join(", "),
            legal_block: kind.legal_block,
        })
        .collect();
    Ok(View::typed(KindsPage {
        tx: texts::all(&lang),
        kinds,
    }))
}

/// One entry of a kind's `blocks` as the kind page shows it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct EntryRow {
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// `block` or `flag`.
    pub mode: String,
    /// "Block" or "Flag".
    pub mode_label: String,
    /// "category is one of fire_alarm, sprinkler", or "Every record".
    pub condition: String,
    /// A legal-block entry, which cannot be removed or changed.
    pub locked: bool,
    /// For a locked entry, the message a change is refused with.
    pub locked_message: Option<String>,
}

/// The row of `entry` of a kind named `kind_name`, with the texts of `t`.
pub(crate) fn entry_row(t: TextWith<'_>, kind_name: &str, entry: &KindBlock) -> EntryRow {
    let condition = match entry.condition() {
        Some(BlockCondition { field, values }) => t(
            "kinds.condition",
            &[("field", &field), ("values", &values.join(", "))],
        ),
        None => t("kinds.every_record", &[]),
    };
    EntryRow {
        guarded: entry.guarded.clone(),
        mode: entry.mode.as_str().to_owned(),
        mode_label: t(&format!("mode.{}", entry.mode.as_str()), &[]),
        condition,
        locked: entry.locked,
        locked_message: entry.locked.then(|| {
            t(
                "kinds.locked",
                &[("kind", &kind_name), ("guarded", &entry.guarded)],
            )
        }),
    }
}

/// What the kind page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-documents/kind.html")]
pub(crate) struct KindPage {
    tx: Texts,
    key: String,
    name: String,
    subject_types: String,
    legal_block: bool,
    override_label: String,
    responsible_role: String,
    entries: Vec<EntryRow>,
    /// `(value, label)` pairs for the mode select.
    modes: Vec<(String, String)>,
}

/// A role's label: the text `roles.<key>`, else the key.
fn role_label(lang: &Lang, key: &str) -> String {
    let text_key = format!("roles.{key}");
    let found = lang.t(&text_key, &[]);
    if found == text_key {
        key.to_owned()
    } else {
        found
    }
}

/// `(value, label)` pairs of the block modes.
fn mode_options(lang: &Lang) -> Vec<(String, String)> {
    BlockMode::ALL
        .iter()
        .map(|mode| {
            (
                mode.as_str().to_owned(),
                text(lang, &format!("mode.{}", mode.as_str())),
            )
        })
        .collect()
}

/// `GET /document-kinds/{key}`: the kind and its `blocks`, with the actions
/// to add an entry, switch an entry's mode and remove an entry. A locked
/// entry shows why it cannot be changed instead.
async fn kind_page(State(db): State<Db>, lang: Lang, Path(key): Path<String>) -> Result<View> {
    let kind = DocumentKind::by_key(&db, &key)
        .await?
        .ok_or(Error::NotFound)?;
    let name = kind.name(&lang.locale).to_owned();
    let t = |key: &str, params: &[(&str, &dyn std::fmt::Display)]| {
        texts::t_with(&lang, &format!("wl_documents.{key}"), params)
    };
    let entries = kind
        .blocks(&db)
        .await?
        .iter()
        .map(|entry| entry_row(&t, &name, entry))
        .collect();
    let override_key = match kind.override_mode {
        OverrideMode::Never => "override.never",
        OverrideMode::WithApproval => "override.with_approval",
    };
    Ok(View::typed(KindPage {
        tx: texts::all(&lang),
        subject_types: kind.subject_types.0.join(", "),
        legal_block: kind.legal_block,
        override_label: text(&lang, override_key),
        responsible_role: kind
            .responsible_role
            .as_deref()
            .filter(|role| !role.trim().is_empty())
            .map_or_else(|| text(&lang, "none"), |role| role_label(&lang, role)),
        entries,
        modes: mode_options(&lang),
        key: kind.key,
        name,
    }))
}

/// The add-entry form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct BlockForm {
    /// The transition, `<entity>.<transition>`.
    #[validate(required, max = 128)]
    pub guarded: String,
    /// Block or flag.
    #[serde(default)]
    pub mode: BlockMode,
    /// The condition's field, if any.
    #[validate(max = 64)]
    #[serde(default)]
    pub condition_field: Option<String>,
    /// The condition's values.
    #[serde(default)]
    pub condition_values: Vec<String>,
}

impl BlockForm {
    /// The condition the form gives: `Ok(None)` without one, `Err(())` when
    /// only the field or only the values are given.
    pub(crate) fn condition(&self) -> std::result::Result<Option<BlockCondition>, ()> {
        let field = self
            .condition_field
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty());
        let values: Vec<String> = self
            .condition_values
            .iter()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect();
        match (field, values.is_empty()) {
            (None, true) => Ok(None),
            (Some(field), false) => Ok(Some(BlockCondition::new(field, values))),
            _ => Err(()),
        }
    }
}

/// A refused change to `blocks` as a 422 under `field`, in `lang`.
fn refused(
    lang: &Lang,
    field: &str,
    refusal: &BlockChangeRefused,
    input: &impl Serialize,
) -> Error {
    invalid(field, refusal.message(lang), input)
}

/// `POST /document-kinds/{key}/blocks`: adds an entry and reloads the
/// page. An invalid transition, one no module guards, one the kind already
/// guards or a half-given condition is a 422 under the field.
async fn store_block(
    State(db): State<Db>,
    lang: Lang,
    Path(key): Path<String>,
    Valid(form): Valid<BlockForm>,
) -> Result<Response> {
    let Ok(condition) = form.condition() else {
        return Err(invalid(
            "condition_values",
            text(&lang, "kinds.condition_incomplete"),
            &form,
        ));
    };
    match kinds::add_block(&db, &key, &form.guarded, form.mode, condition).await? {
        Ok(_) => Ok((Toast::success(text(&lang, "kinds.added")), renox::HxRefresh).into_response()),
        Err(refusal) => Err(refused(&lang, "guarded", &refusal, &form)),
    }
}

/// The change-mode form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ModeForm {
    /// The new mode.
    #[serde(default)]
    pub mode: BlockMode,
}

/// `POST /document-kinds/{key}/blocks/{guarded}/mode`: switches the entry's
/// mode and reloads the page; a locked entry is refused (422 with its
/// message, the entry unchanged).
async fn update_mode(
    State(db): State<Db>,
    lang: Lang,
    Path((key, guarded)): Path<(String, String)>,
    Valid(form): Valid<ModeForm>,
) -> Result<Response> {
    match kinds::set_block_mode(&db, &key, &guarded, form.mode).await? {
        Ok(_) => Ok((
            Toast::success(text(&lang, "kinds.switched")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(refusal) => Err(refused(&lang, "mode", &refusal, &form)),
    }
}

/// `DELETE /document-kinds/{key}/blocks/{guarded}`: removes the entry and
/// goes back to the page; a locked entry is refused (422 with its message,
/// the entry unchanged).
async fn destroy_block(
    State(db): State<Db>,
    lang: Lang,
    back: Back,
    Path((key, guarded)): Path<(String, String)>,
) -> Result<Response> {
    match kinds::remove_block(&db, &key, &guarded).await? {
        Ok(()) => Ok((Toast::success(text(&lang, "kinds.removed")), back).into_response()),
        Err(refusal) => Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            refusal.message(&lang),
        )),
    }
}

#[cfg(test)]
mod tests {
    use renox::chrono::{TimeZone, Utc};
    use renox::db::Json;

    use super::*;

    fn at(day: u32) -> Option<DateTime> {
        Some(Utc.with_ymd_and_hms(2026, 11, day, 9, 0, 0).unwrap())
    }

    fn pending(id: i64, submitted: u32, priority: Option<u32>) -> Document {
        Document {
            id,
            submitted_at: at(submitted),
            review_priority_at: priority.and_then(at),
            ..Default::default()
        }
    }

    #[test]
    fn a_renewal_a_refused_transition_waits_for_comes_first() {
        let mut queue = [
            pending(1, 1, None),
            pending(2, 20, Some(25)),
            pending(3, 2, None),
            pending(4, 21, Some(28)),
        ];
        queue.sort_by(queue_order);
        let ids: Vec<i64> = queue.iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![4, 2, 1, 3]);
    }

    fn english(key: &str, params: &[(&str, &dyn std::fmt::Display)]) -> String {
        texts::english(&format!("wl_documents.{key}"), params)
    }

    #[test]
    fn a_locked_entry_shows_why_it_cannot_change() {
        let entry = KindBlock {
            guarded: "deployment.activate".into(),
            mode: BlockMode::Block,
            locked: true,
            ..Default::default()
        };
        let row = entry_row(&english, "Work permit", &entry);
        assert_eq!(row.mode_label, "Block");
        assert_eq!(row.condition, "Every record");
        assert_eq!(
            row.locked_message.as_deref(),
            Some("Work permit is a legal block: deployment.activate cannot be removed or changed")
        );
    }

    #[test]
    fn a_conditioned_entry_names_its_values() {
        let entry = KindBlock {
            guarded: "ticket.resolve".into(),
            mode: BlockMode::Flag,
            condition_field: Some("category".into()),
            condition_values: Json(vec!["fire_alarm".into(), "sprinkler".into()]),
            ..Default::default()
        };
        let row = entry_row(&english, "Civil Defence approval", &entry);
        assert_eq!(row.mode, "flag");
        assert_eq!(row.condition, "category is one of fire_alarm, sprinkler");
        assert!(row.locked_message.is_none());
    }

    #[test]
    fn a_condition_needs_both_its_field_and_values() {
        let form = |field: Option<&str>, values: &[&str]| BlockForm {
            guarded: "ticket.resolve".into(),
            condition_field: field.map(str::to_owned),
            condition_values: values.iter().map(|v| (*v).to_owned()).collect(),
            ..Default::default()
        };
        assert_eq!(form(None, &[]).condition(), Ok(None));
        assert_eq!(form(Some(" "), &[" "]).condition(), Ok(None));
        assert_eq!(
            form(Some("category"), &["fire_alarm"]).condition(),
            Ok(Some(BlockCondition::new("category", ["fire_alarm"])))
        );
        assert!(form(Some("category"), &[]).condition().is_err());
        assert!(form(None, &["fire_alarm"]).condition().is_err());
    }
}
