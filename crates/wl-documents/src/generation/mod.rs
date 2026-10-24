//! Generated documents (`DOC.GEN.01`, wl-documents §3 *Document template*,
//! *Generation*, §5.3, §6 `Documents::generate`, §8 data providers).
//!
//! A module registers a [`TemplateSeed`] (key, document kind, language
//! mode, body, paper size and, when the kind is numbered, the kernel
//! [`Series`]) with [`register_template`], and a
//! [`DataProvider`](wl_kernel::generation::DataProvider) for its subject
//! type with [`wl_kernel::generation::register_data_provider`]. A
//! transition asks for the document through
//! [`wl_kernel::generation::generator`]; this module answers with
//! [`generate`]:
//!
//! 1. the active [`DocumentTemplate`] (stored from its seed on first use;
//!    editing it adds a new version, documents keep theirs);
//! 2. the subject's data from the provider, and its SHA-256: the same data
//!    asked twice returns the earlier document, changed data for an issued
//!    template is refused (a correction is a new document);
//! 3. in one transaction: the next number of the template's series, the
//!    PDF rendered with the client's branding ([`BrandAssets`]) through the
//!    app's [`PdfRendering`] (the built-in [`pdf`] renderer by default), the
//!    accepted document with source `generated` and the [`Generation`] row.
//!    A rendering error rolls everything back: nothing is stored and a
//!    gapless number is given back;
//! 4. after the commit, `DocumentAccepted` and `DocumentGenerated`.
//!
//! Template bodies are MiniJinja text: `{{ data.<field> }}` (the provider's
//! values), `{{ number }}`, `{{ entity.name_en }}`, `{{ entity.name_ar }}`,
//! `{{ entity.trn }}` and `{{ today }}`. An unknown field fails the render
//! rather than printing an empty value.
//!
//! The back-office page that lists and edits templates per kind is not part
//! of this slice: templates are edited with [`DocumentTemplate::revise`].

pub mod pdf;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use renox::db::now;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::config::ClientConfig;
use wl_kernel::documents::{DocumentId, today};
use wl_kernel::entities::LegalEntity;
use wl_kernel::generation::{
    DocumentGenerated, DocumentGenerator, GenerateRefusal, GenerateRequest, Generated,
    GeneratedDocument, GenerationFuture, LanguageMode, data_provider,
};
use wl_kernel::numbering::Series;

use crate::documents::{
    DocumentSource, NewDocument, emit_accepted, emit_superseded, file_refusal,
    record_generated_in, sha256_hex,
};
use crate::kinds::DocumentKind;

/// The paper a template is printed on.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaperSize {
    /// ISO A4.
    #[default]
    A4,
    /// US Letter.
    Letter,
}

/// A layout generated documents are built from (§3 *Document template*).
/// Each edit is a new row with the next version; only the newest is active.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_documents_templates", unique(key, version))]
pub struct DocumentTemplate {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `owner_statement`.
    pub key: String,
    /// The document kind of the generated documents.
    pub kind_key: String,
    /// The name shown in the settings.
    pub name: String,
    /// `en`, `ar` or `bilingual`.
    pub language_mode: String,
    /// The English body (MiniJinja).
    pub body_en: String,
    /// The Arabic body (MiniJinja).
    pub body_ar: String,
    /// The English footer text.
    pub footer_en: Option<String>,
    /// The Arabic footer text.
    pub footer_ar: Option<String>,
    /// The paper.
    pub paper_size: PaperSize,
    /// 1, 2, …
    pub version: i64,
    /// Whether this version is the one in use.
    pub active: bool,
    /// The company admin who made this version; `None` for the seed.
    pub edited_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The wording a company admin may change on a template: a new version.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TemplateWording {
    /// The English body.
    pub body_en: String,
    /// The Arabic body.
    pub body_ar: String,
    /// The English footer.
    pub footer_en: Option<String>,
    /// The Arabic footer.
    pub footer_ar: Option<String>,
}

impl DocumentTemplate {
    /// The language mode (bilingual when the stored text is unknown).
    pub fn mode(&self) -> LanguageMode {
        LanguageMode::parse(&self.language_mode).unwrap_or_default()
    }

    /// The active version of template `key`, stored from its seed when no
    /// version is stored yet.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn active(db: &Db, key: &str) -> Result<Option<DocumentTemplate>> {
        let found = DocumentTemplate::query()
            .where_(DocumentTemplate::KEY.eq(key))
            .where_(DocumentTemplate::ACTIVE.eq(true))
            .order_by_desc(DocumentTemplate::VERSION)
            .first(db)
            .await?;
        if found.is_some() {
            return Ok(found);
        }
        let Some(seed) = template_seed(key) else {
            return Ok(None);
        };
        let any = DocumentTemplate::query()
            .where_(DocumentTemplate::KEY.eq(key))
            .first(db)
            .await?;
        if any.is_some() {
            // Every version was switched off on purpose.
            return Ok(None);
        }
        Ok(Some(DocumentTemplate::create(db, seed.row()).await?))
    }

    /// `template_manage`: stores `wording` as the next version of this
    /// template, by `by`, and switches the older versions off. Documents
    /// already generated keep the version they were built from.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn revise(
        &self,
        db: &Db,
        by: Option<i64>,
        wording: TemplateWording,
    ) -> Result<DocumentTemplate> {
        let mut tx = db.begin().await?;
        let latest = DocumentTemplate::query()
            .where_(DocumentTemplate::KEY.eq(self.key.as_str()))
            .order_by_desc(DocumentTemplate::VERSION)
            .first(&mut tx)
            .await?
            .map_or(self.version, |t| t.version);
        DocumentTemplate::query()
            .where_(DocumentTemplate::KEY.eq(self.key.as_str()))
            .update(&mut tx, &[("active", &false)])
            .await?;
        let next = DocumentTemplate::create(
            &mut tx,
            DocumentTemplate {
                id: 0,
                body_en: wording.body_en,
                body_ar: wording.body_ar,
                footer_en: wording.footer_en,
                footer_ar: wording.footer_ar,
                version: latest + 1,
                active: true,
                edited_by: by,
                created_at: None,
                updated_at: None,
                ..self.clone()
            },
        )
        .await?;
        tx.commit().await?;
        Ok(next)
    }
}

/// A template a module registers in code (§7: the vertical's templates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSeed {
    /// The key, e.g. `owner_statement`.
    pub key: String,
    /// The document kind's key.
    pub kind: String,
    /// The name shown in the settings.
    pub name: String,
    /// The languages.
    pub mode: LanguageMode,
    /// The English body.
    pub body_en: String,
    /// The Arabic body.
    pub body_ar: String,
    /// The English footer.
    pub footer_en: Option<String>,
    /// The Arabic footer.
    pub footer_ar: Option<String>,
    /// The paper.
    pub paper: PaperSize,
    /// The kernel series that numbers its documents, if any.
    pub series: Option<Series>,
    /// Whether a document once generated is issued: asking again with
    /// changed data is refused. On for a numbered template.
    pub issued: bool,
}

impl TemplateSeed {
    /// Template `key` of documents of kind `kind`, bilingual on A4, with no
    /// text yet.
    pub fn new(key: &str, kind: &str) -> Self {
        TemplateSeed {
            key: key.to_owned(),
            kind: kind.to_owned(),
            name: key.replace('_', " "),
            mode: LanguageMode::Bilingual,
            body_en: String::new(),
            body_ar: String::new(),
            footer_en: None,
            footer_ar: None,
            paper: PaperSize::A4,
            series: None,
            issued: false,
        }
    }

    /// The name shown in the settings.
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }

    /// The languages.
    pub fn mode(mut self, mode: LanguageMode) -> Self {
        self.mode = mode;
        self
    }

    /// The English body.
    pub fn english(mut self, body: &str) -> Self {
        self.body_en = body.to_owned();
        self
    }

    /// The Arabic body.
    pub fn arabic(mut self, body: &str) -> Self {
        self.body_ar = body.to_owned();
        self
    }

    /// The footer text in English and Arabic.
    pub fn footer(mut self, english: &str, arabic: &str) -> Self {
        self.footer_en = Some(english.to_owned()).filter(|t| !t.is_empty());
        self.footer_ar = Some(arabic.to_owned()).filter(|t| !t.is_empty());
        self
    }

    /// The paper.
    pub fn paper(mut self, paper: PaperSize) -> Self {
        self.paper = paper;
        self
    }

    /// Numbered from `series`; a numbered document is issued.
    pub fn numbered(mut self, series: Series) -> Self {
        self.series = Some(series);
        self.issued = true;
        self
    }

    /// Issued once generated: never regenerated with other data.
    pub fn issued(mut self) -> Self {
        self.issued = true;
        self
    }

    /// Version 1 of the template, as stored.
    fn row(&self) -> DocumentTemplate {
        DocumentTemplate {
            key: self.key.clone(),
            kind_key: self.kind.clone(),
            name: self.name.clone(),
            language_mode: self.mode.as_str().to_owned(),
            body_en: self.body_en.clone(),
            body_ar: self.body_ar.clone(),
            footer_en: self.footer_en.clone(),
            footer_ar: self.footer_ar.clone(),
            paper_size: self.paper,
            version: 1,
            active: true,
            ..DocumentTemplate::default()
        }
    }
}

static TEMPLATES: RwLock<BTreeMap<String, TemplateSeed>> = RwLock::new(BTreeMap::new());

/// Adds `seed` to the process-wide templates; registering the same key again
/// replaces it. The stored version is made on first use.
pub fn register_template(seed: TemplateSeed) {
    TEMPLATES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(seed.key.clone(), seed);
}

/// The registered seed of template `key`.
pub fn template_seed(key: &str) -> Option<TemplateSeed> {
    TEMPLATES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .cloned()
}

/// The record of one generated document (§3 *Generation*).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_documents_generations",
    index(template_key, subject_type, subject_id)
)]
pub struct Generation {
    /// The primary key.
    pub id: i64,
    /// The template's key.
    pub template_key: String,
    /// The template's version it was built from.
    pub template_version: i64,
    /// What it is about: the type.
    pub subject_type: String,
    /// What it is about: the record's id.
    pub subject_id: i64,
    /// `en`, `ar` or `bilingual`.
    pub language_mode: String,
    /// The SHA-256 of the data it was built from.
    pub data_hash: String,
    /// The number assigned, if any.
    pub number: Option<String>,
    /// The resulting document.
    #[model(references = "wl_documents_documents")]
    pub document_id: i64,
    /// The user who asked; `None` for the system.
    pub generated_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The client's branding as a generated document shows it (DESIGN-SYSTEM.md
/// §4): the company name, the logo and the accent colour. The host may
/// provide one (`app.provide(BrandAssets { .. })`); otherwise it comes from
/// `client.toml` ([`brand_assets`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrandAssets {
    /// The company name in English.
    pub company_name: String,
    /// The company name in Arabic.
    pub company_name_ar: Option<String>,
    /// The logo file (JPEG or PNG).
    pub logo: Option<Vec<u8>>,
    /// The accent colour, `#rrggbb`.
    pub accent: String,
}

/// The branding for generated documents: the provided [`BrandAssets`], else
/// `client.toml`'s name, accent and logo (its path is relative to the
/// directory of `CLIENT_CONFIG`), else the platform's defaults.
pub fn brand_assets(state: &AppState) -> BrandAssets {
    if let Some(brand) = state.provided::<BrandAssets>() {
        return (*brand).clone();
    }
    let Some(config) = state.provided::<Arc<ClientConfig>>() else {
        return BrandAssets {
            accent: "#0b6e66".to_owned(),
            ..BrandAssets::default()
        };
    };
    let logo = config.branding.logo.as_deref().and_then(|path| {
        let base = std::env::var("CLIENT_CONFIG")
            .ok()
            .and_then(|file| {
                std::path::Path::new(&file)
                    .parent()
                    .map(std::path::Path::to_path_buf)
            })
            .unwrap_or_default();
        std::fs::read(base.join(path)).ok()
    });
    BrandAssets {
        company_name: config.client.name.clone(),
        company_name_ar: None,
        logo,
        accent: config.branding.accent.clone(),
    }
}

/// Why a PDF could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(pub String);

impl RenderError {
    /// A rendering error with `reason`.
    pub fn new(reason: impl Into<String>) -> Self {
        RenderError(reason.into())
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

/// What a renderer lays out: the template's text already filled with the
/// data, and the data itself for a renderer that fills its own layout.
#[derive(Debug, Clone)]
pub struct RenderJob {
    /// The template's key.
    pub template: String,
    /// The template's version.
    pub template_version: i64,
    /// The languages.
    pub mode: LanguageMode,
    /// The paper.
    pub paper: PaperSize,
    /// The document's title.
    pub title: String,
    /// The number assigned, if any (in the footer).
    pub number: Option<String>,
    /// The English text.
    pub english: String,
    /// The Arabic text.
    pub arabic: String,
    /// The English footer (the template's, else the entity's).
    pub footer_en: Option<String>,
    /// The Arabic footer.
    pub footer_ar: Option<String>,
    /// The provider's values.
    pub values: renox::serde_json::Value,
    /// The client's branding.
    pub brand: BrandAssets,
}

/// Turns a [`RenderJob`] into PDF bytes.
pub trait PdfRenderer: Send + Sync + 'static {
    /// The PDF of `job`.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when it cannot be rendered; nothing is stored then.
    fn render(&self, job: &RenderJob) -> std::result::Result<Vec<u8>, RenderError>;
}

/// The built-in renderer ([`pdf::render`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinPdf;

impl PdfRenderer for BuiltinPdf {
    fn render(&self, job: &RenderJob) -> std::result::Result<Vec<u8>, RenderError> {
        pdf::render(job)
    }
}

/// The renderer the app uses, when it provides one
/// (`app.provide(PdfRendering::new(..))`); [`BuiltinPdf`] otherwise.
#[derive(Clone)]
pub struct PdfRendering(pub Arc<dyn PdfRenderer>);

impl PdfRendering {
    /// Renders with `renderer`.
    pub fn new(renderer: impl PdfRenderer) -> Self {
        PdfRendering(Arc::new(renderer))
    }
}

fn renderer(state: &AppState) -> Arc<dyn PdfRenderer> {
    match state.provided::<PdfRendering>() {
        Some(rendering) => Arc::clone(&rendering.0),
        None => Arc::new(BuiltinPdf),
    }
}

/// `value` as text with its object keys sorted, so equal data hashes
/// equally whatever order the provider built it in.
fn canonical(value: &renox::serde_json::Value) -> String {
    match value {
        renox::serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        renox::serde_json::Value::String(key.clone()),
                        canonical(&map[key.as_str()])
                    )
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        renox::serde_json::Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}

/// The data hash of a generation: the canonical values and the language.
pub fn data_hash(values: &renox::serde_json::Value, mode: LanguageMode) -> String {
    sha256_hex(format!("{}|{}", canonical(values), mode.as_str()).as_bytes())
}

/// Fills `body` with the context; an unknown field is an error.
fn fill(body: &str, context: &renox::serde_json::Value) -> std::result::Result<String, RenderError> {
    if body.trim().is_empty() {
        return Ok(String::new());
    }
    let mut env = renox::minijinja::Environment::new();
    env.set_undefined_behavior(renox::minijinja::UndefinedBehavior::Strict);
    env.render_str(body, context)
        .map_err(|err| RenderError::new(err.to_string()))
}

/// A file name: the title and number, without characters a disk refuses.
fn file_name(title: &str, number: Option<&str>) -> String {
    let base = match number {
        Some(number) => format!("{title} {number}"),
        None => title.to_owned(),
    };
    let clean: String = base
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .collect();
    format!("{}.pdf", clean.trim())
}

/// `Documents::generate` (§6): see the module documentation.
///
/// # Errors
///
/// The database's or the disk's errors. Refusals (no template, no data,
/// issued with other data, rendering failed) are `Ok(Err(..))` and store
/// nothing.
pub async fn generate(state: &AppState, request: GenerateRequest) -> Result<Generated> {
    let Some(template) = DocumentTemplate::active(&state.db, &request.template).await? else {
        return Ok(Err(GenerateRefusal::UnknownTemplate(request.template)));
    };
    let seed = template_seed(&template.key);
    let mode = request.language.unwrap_or_else(|| template.mode());
    let Some(provider) = data_provider(&request.subject.subject_type) else {
        return Ok(Err(GenerateRefusal::NoProvider(
            request.subject.subject_type.clone(),
        )));
    };
    let Some(data) = provider
        .data(state, &template.key, &request.subject)
        .await?
    else {
        return Ok(Err(GenerateRefusal::NoData));
    };
    let hash = data_hash(&data.values, mode);

    // The same data again: the earlier document. Other data for an issued
    // template: refused.
    let issued = seed.as_ref().is_some_and(|s| s.issued);
    let earlier = Generation::query()
        .where_(Generation::TEMPLATE_KEY.eq(template.key.as_str()))
        .where_(Generation::SUBJECT_TYPE.eq(request.subject.subject_type.as_str()))
        .where_(Generation::SUBJECT_ID.eq(request.subject.subject_id))
        .order_by_desc(Generation::ID)
        .get(&state.db)
        .await?;
    if let Some(same) = earlier.iter().find(|g| g.data_hash == hash) {
        return Ok(Ok(GeneratedDocument {
            document: DocumentId(same.document_id),
            number: same.number.clone(),
            data_hash: hash,
            reused: true,
        }));
    }
    if issued && let Some(first) = earlier.first() {
        return Ok(Err(GenerateRefusal::Issued {
            document: DocumentId(first.document_id),
        }));
    }

    let Some(kind) = DocumentKind::by_key(&state.db, &template.kind_key).await? else {
        return Ok(Err(GenerateRefusal::Refused(format!(
            "no document kind `{}`",
            template.kind_key
        ))));
    };
    let entity = match data.entity_id {
        Some(id) => LegalEntity::find(&state.db, id).await?,
        None => None,
    };
    let title = data
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| kind.name_en.clone());
    let mut brand = brand_assets(state);
    if let Some(entity) = &entity {
        brand.company_name = entity.name_en.clone();
        brand.company_name_ar = entity.name_ar.clone();
    }

    let mut tx = state.db.begin().await?;
    let series = seed.as_ref().and_then(|s| s.series.clone());
    let number = match &series {
        Some(series) if series.gapless => {
            Some(series.next(&mut tx, None, entity.as_ref(), today()).await?)
        }
        Some(series) => Some(
            series
                .next_allowing_gaps(&state.db, None, entity.as_ref(), today())
                .await?,
        ),
        None => None,
    };
    let context = json!({
        "data": data.values,
        "number": number,
        "title": title,
        "today": today().to_string(),
        "entity": entity.as_ref().map(|e| json!({
            "name_en": e.name_en,
            "name_ar": e.name_ar,
            "trn": e.trn,
            "address_en": e.address_en,
            "address_ar": e.address_ar,
        })),
    });
    let filled = fill(&template.body_en, &context).and_then(|english| {
        fill(&template.body_ar, &context).map(|arabic| (english, arabic))
    });
    let (english, arabic) = match filled {
        Ok(text) => text,
        Err(err) => return Ok(Err(GenerateRefusal::Render(err.0))),
    };
    let job = RenderJob {
        template: template.key.clone(),
        template_version: template.version,
        mode,
        paper: template.paper_size,
        title: title.clone(),
        number: number.clone(),
        english,
        arabic,
        footer_en: template
            .footer_en
            .clone()
            .or_else(|| entity.as_ref().and_then(|e| e.footer_en.clone())),
        footer_ar: template
            .footer_ar
            .clone()
            .or_else(|| entity.as_ref().and_then(|e| e.footer_ar.clone())),
        values: data.values.clone(),
        brand,
    };
    let bytes = match renderer(state).render(&job) {
        Ok(bytes) => bytes,
        // The transaction is dropped: the number goes back, nothing is stored.
        Err(err) => return Ok(Err(GenerateRefusal::Render(err.0))),
    };
    let file = Upload::new(
        file_name(&title, number.as_deref()),
        "application/pdf",
        bytes,
    );
    if let Some(reason) = file_refusal(&kind, &request.subject.subject_type, &file) {
        return Ok(Err(GenerateRefusal::Refused(format!(
            "{} refuses the generated file ({reason:?})",
            kind.name_en
        ))));
    }
    let mut new = NewDocument::new(request.subject.clone(), &kind.key, file)
        .title(&title)
        .issued_on(today())
        .channel("generated")
        .source(DocumentSource::Generated);
    if let Some(number) = &number {
        new = new.number(number);
    }
    let (document, superseded) =
        record_generated_in(state, &mut tx, request.by, &new, &kind).await?;
    Generation::create(
        &mut tx,
        Generation {
            template_key: template.key.clone(),
            template_version: template.version,
            subject_type: request.subject.subject_type.clone(),
            subject_id: request.subject.subject_id,
            language_mode: mode.as_str().to_owned(),
            data_hash: hash.clone(),
            number: number.clone(),
            document_id: document.id,
            generated_by: request.by,
            created_at: Some(now()),
            ..Generation::default()
        },
    )
    .await?;
    tx.commit().await?;

    emit_accepted(state, &document, &kind).await?;
    emit_superseded(state, superseded, document.id).await?;
    state
        .emit(DocumentGenerated {
            document: document.document_id(),
            template: template.key.clone(),
            template_version: template.version,
            subject: request.subject.clone(),
            number: number.clone(),
        })
        .await?;
    Ok(Ok(GeneratedDocument {
        document: document.document_id(),
        number,
        data_hash: hash,
        reused: false,
    }))
}

/// The kernel's [`DocumentGenerator`], answered by [`generate`].
#[derive(Debug, Clone, Copy, Default)]
pub struct TemplateGenerator;

impl DocumentGenerator for TemplateGenerator {
    fn generate<'a>(
        &'a self,
        state: &'a AppState,
        request: GenerateRequest,
    ) -> GenerationFuture<'a, Generated> {
        Box::pin(generate(state, request))
    }
}

/// The subject a generation is about.
impl Generation {
    /// What it is about.
    pub fn subject(&self) -> Subject {
        Subject {
            subject_type: self.subject_type.clone(),
            subject_id: self.subject_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_data_hashes_equally_in_any_key_order() {
        let a = json!({ "period": "2026-10", "total": 1250, "lines": [{ "b": 1, "a": 2 }] });
        let b = json!({ "lines": [{ "a": 2, "b": 1 }], "total": 1250, "period": "2026-10" });
        assert_eq!(
            data_hash(&a, LanguageMode::Bilingual),
            data_hash(&b, LanguageMode::Bilingual)
        );
        assert_ne!(
            data_hash(&a, LanguageMode::Bilingual),
            data_hash(&a, LanguageMode::En)
        );
    }

    #[test]
    fn file_names_carry_the_number() {
        assert_eq!(
            file_name("Owner statement", Some("OS/2026/0001")),
            "Owner statement OS-2026-0001.pdf"
        );
        assert_eq!(file_name("Receipt", None), "Receipt.pdf");
    }

    #[test]
    fn an_unknown_field_fails_the_fill() {
        assert!(fill("{{ data.missing }}", &json!({ "data": {} })).is_err());
        assert_eq!(
            fill("Period {{ data.period }}", &json!({ "data": { "period": "2026-10" } })).unwrap(),
            "Period 2026-10"
        );
    }
}
