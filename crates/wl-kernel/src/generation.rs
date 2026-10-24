//! Generated documents (wl-documents §5.3, §6 `Documents::generate`, §8 data
//! providers, `DOC.GEN.01`): how a module asks for a branded PDF of one of
//! its records without depending on the `wl-documents` crate.
//!
//! - The module that owns a subject type registers a [`DataProvider`] for it
//!   with [`register_data_provider`]: it returns the data of one record as a
//!   JSON object, and the legal entity that issues the document when the
//!   template's number series is per entity.
//! - A transition (or a user action) calls [`DocumentGenerator::generate`]
//!   through [`generator`] with a [`GenerateRequest`] (template key, subject,
//!   language mode). `wl-documents` provides the generator; it renders the
//!   PDF with the client's branding, stores it as an accepted document with
//!   source `generated` and emits [`DocumentGenerated`].
//! - Asking twice with the same data returns the same document
//!   ([`GeneratedDocument::reused`]); asking again with changed data for an
//!   issued template is refused ([`GenerateRefusal::Issued`]): a correction
//!   is a new document (a credit note), never an edit.
//!
//! When the `documents` module is off, [`generator`] gives [`NoGenerator`],
//! which refuses every request with [`GenerateRefusal::Unavailable`].

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::documents::DocumentId;
use crate::subject::Subject;

/// A boxed future of the generation contract.
pub type GenerationFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The languages of a generated document (wl-documents §3 *Document
/// template*, §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageMode {
    /// English only (internal documents).
    En,
    /// Arabic only, right to left.
    Ar,
    /// English and Arabic side by side (documents sent to parties).
    #[default]
    Bilingual,
}

impl LanguageMode {
    /// `en`, `ar` or `bilingual`.
    pub fn as_str(self) -> &'static str {
        match self {
            LanguageMode::En => "en",
            LanguageMode::Ar => "ar",
            LanguageMode::Bilingual => "bilingual",
        }
    }

    /// The mode named `text`, if it is one.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "en" => Some(LanguageMode::En),
            "ar" => Some(LanguageMode::Ar),
            "bilingual" => Some(LanguageMode::Bilingual),
            _ => None,
        }
    }

    /// Whether the document has an English part.
    pub fn has_english(self) -> bool {
        matches!(self, LanguageMode::En | LanguageMode::Bilingual)
    }

    /// Whether the document has an Arabic part.
    pub fn has_arabic(self) -> bool {
        matches!(self, LanguageMode::Ar | LanguageMode::Bilingual)
    }
}

impl fmt::Display for LanguageMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a [`DataProvider`] returns for one record.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GenerationData {
    /// The values the template reads (`{{ data.period }}`), a JSON object.
    pub values: serde_json::Value,
    /// The legal entity that issues the document: its number series (when
    /// per entity), names and footer.
    pub entity_id: Option<i64>,
    /// The document's title; the template kind's name when `None`.
    pub title: Option<String>,
}

impl GenerationData {
    /// `values` (a JSON object) and nothing else.
    pub fn new(values: serde_json::Value) -> Self {
        GenerationData {
            values,
            entity_id: None,
            title: None,
        }
    }

    /// Issued by legal entity `entity_id`.
    pub fn entity(mut self, entity_id: i64) -> Self {
        self.entity_id = Some(entity_id);
        self
    }

    /// With this title.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_owned());
        self
    }
}

/// A module's function that returns the data of one record of its subject
/// type for a template (wl-documents §8).
pub trait DataProvider: Send + Sync + 'static {
    /// The data of `subject` for `template`, or `None` when the record does
    /// not exist.
    fn data<'a>(
        &'a self,
        state: &'a AppState,
        template: &'a str,
        subject: &'a Subject,
    ) -> GenerationFuture<'a, Option<GenerationData>>;
}

static PROVIDERS: RwLock<BTreeMap<String, Arc<dyn DataProvider>>> = RwLock::new(BTreeMap::new());

/// Adds `provider` for `subject_type` to the process-wide registry;
/// registering the same type again replaces it.
pub fn register_data_provider(subject_type: &str, provider: impl DataProvider) {
    PROVIDERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.to_owned(), Arc::new(provider));
}

/// The provider of `subject_type`, if registered.
pub fn data_provider(subject_type: &str) -> Option<Arc<dyn DataProvider>> {
    PROVIDERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .cloned()
}

/// A request for a generated document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GenerateRequest {
    /// The template's key, e.g. `owner_statement`.
    pub template: String,
    /// The record the document is about.
    pub subject: Subject,
    /// The languages; the template's own mode when `None`.
    pub language: Option<LanguageMode>,
    /// The user who asked; `None` for a transition run by the system.
    pub by: Option<i64>,
}

impl GenerateRequest {
    /// The document of `template` for `subject`, in the template's languages,
    /// by the system.
    pub fn new(template: &str, subject: Subject) -> Self {
        GenerateRequest {
            template: template.to_owned(),
            subject,
            language: None,
            by: None,
        }
    }

    /// In `language` instead of the template's mode.
    pub fn language(mut self, language: LanguageMode) -> Self {
        self.language = Some(language);
        self
    }

    /// Asked by user `user`.
    pub fn by(mut self, user: i64) -> Self {
        self.by = Some(user);
        self
    }
}

/// The document a [`GenerateRequest`] produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedDocument {
    /// The stored document (accepted, source `generated`).
    pub document: DocumentId,
    /// The number assigned from the template's series, if it has one.
    pub number: Option<String>,
    /// The SHA-256 of the data the PDF was built from.
    pub data_hash: String,
    /// `true` when an earlier generation with the same data was returned.
    pub reused: bool,
}

/// Why a document was not generated. Nothing is stored in any case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenerateRefusal {
    /// The `documents` module is off.
    Unavailable,
    /// No active template has this key.
    UnknownTemplate(String),
    /// No data provider is registered for the subject type.
    NoProvider(String),
    /// The provider found no record for the subject.
    NoData,
    /// The template's document was issued for the subject with other data;
    /// a correction is a new document.
    Issued {
        /// The issued document.
        document: DocumentId,
    },
    /// The PDF could not be rendered.
    Render(String),
    /// The template's document kind refuses the file.
    Refused(String),
}

impl fmt::Display for GenerateRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenerateRefusal::Unavailable => f.write_str("documents are not enabled"),
            GenerateRefusal::UnknownTemplate(key) => write!(f, "no active template `{key}`"),
            GenerateRefusal::NoProvider(subject_type) => {
                write!(f, "no data provider for `{subject_type}`")
            }
            GenerateRefusal::NoData => f.write_str("the record has no data for the template"),
            GenerateRefusal::Issued { document } => write!(
                f,
                "document {document} was issued with other data; issue a correction instead"
            ),
            GenerateRefusal::Render(reason) => write!(f, "the PDF could not be rendered: {reason}"),
            GenerateRefusal::Refused(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for GenerateRefusal {}

impl GenerateRefusal {
    /// The refusal as an error: 422 with the reason.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
    }
}

/// The answer of [`DocumentGenerator::generate`].
pub type Generated = std::result::Result<GeneratedDocument, GenerateRefusal>;

/// Produces branded PDF documents (`DOC.GEN.01`). `wl-documents` provides it
/// with [`provide_generator`].
pub trait DocumentGenerator: Send + Sync + 'static {
    /// Generates (or returns the earlier generation of) the document
    /// `request` asks for.
    fn generate<'a>(
        &'a self,
        state: &'a AppState,
        request: GenerateRequest,
    ) -> GenerationFuture<'a, Generated>;
}

/// The generator when the `documents` module is off: refuses everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoGenerator;

impl DocumentGenerator for NoGenerator {
    fn generate<'a>(
        &'a self,
        _state: &'a AppState,
        _request: GenerateRequest,
    ) -> GenerationFuture<'a, Generated> {
        Box::pin(async { Ok(Err(GenerateRefusal::Unavailable)) })
    }
}

/// Makes `generator` the app's [`DocumentGenerator`]; `wl-documents` calls
/// it in `register`.
pub fn provide_generator(app: &mut Registry, generator: impl DocumentGenerator) {
    let generator: Arc<dyn DocumentGenerator> = Arc::new(generator);
    app.provide(generator);
}

/// The app's [`DocumentGenerator`], or [`NoGenerator`].
pub fn generator(state: &AppState) -> Arc<dyn DocumentGenerator> {
    match state.provided::<Arc<dyn DocumentGenerator>>() {
        Some(generator) => Arc::clone(&generator),
        None => Arc::new(NoGenerator),
    }
}

/// A PDF was generated and stored (wl-documents §10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentGenerated {
    /// The stored document.
    pub document: DocumentId,
    /// The template's key.
    pub template: String,
    /// The template's version.
    pub template_version: i64,
    /// The record it is about.
    pub subject: Subject,
    /// The number assigned, if any.
    pub number: Option<String>,
}

impl Event for DocumentGenerated {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_modes_read_and_print() {
        for mode in [LanguageMode::En, LanguageMode::Ar, LanguageMode::Bilingual] {
            assert_eq!(LanguageMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(LanguageMode::parse("fr"), None);
        assert!(LanguageMode::Bilingual.has_arabic() && LanguageMode::Bilingual.has_english());
        assert!(!LanguageMode::En.has_arabic());
    }

    #[test]
    fn a_refusal_reads_in_words() {
        let refusal = GenerateRefusal::Issued {
            document: DocumentId(7),
        };
        assert!(refusal.to_string().contains("issue a correction"));
    }
}
