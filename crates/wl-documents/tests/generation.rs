//! BL-DOC-11 (#463, `DOC.GEN.01`): a branded PDF generated from a template,
//! stored as an accepted document with source `generated`, a data hash, the
//! number from the kernel series and never regenerated once issued.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::generation::pdf::{text_lines, visual_order};
use wl_documents::generation::{
    BrandAssets, DocumentTemplate, Generation, PdfRenderer, PdfRendering, RenderError, RenderJob,
    TemplateSeed, TemplateWording, register_template,
};
use wl_documents::kinds::{KindSeed, install_seeds};
use wl_documents::{Document, DocumentSource, DocumentStatus, Documents, WlDocuments};
use wl_kernel::Subject;
use wl_kernel::generation::{
    DataProvider, GenerateRefusal, GenerateRequest, GenerationData, GenerationFuture, LanguageMode,
    generator, register_data_provider,
};
use wl_kernel::numbering::{Reset, Series};

/// The subject type of the test statements.
const STATEMENT: &str = "property.owner_statement";

/// A tiny JPEG header the renderer embeds as the logo (1 x 1 pixel, RGB).
const LOGO: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x22, 0x00, 0x02,
    0x11, 0x01, 0x03, 0x11, 0x01, 0xFF, 0xD9,
];

/// The test statements' data, per subject id: each test uses its own ids.
static STATEMENTS: Mutex<BTreeMap<i64, renox::serde_json::Value>> = Mutex::new(BTreeMap::new());

fn set_statement(id: i64, values: renox::serde_json::Value) {
    STATEMENTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, values);
}

struct Statements;

impl DataProvider for Statements {
    fn data<'a>(
        &'a self,
        _state: &'a AppState,
        _template: &'a str,
        subject: &'a Subject,
    ) -> GenerationFuture<'a, Option<GenerationData>> {
        let values = STATEMENTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&subject.subject_id)
            .cloned();
        Box::pin(async move {
            Ok(values.map(|values| GenerationData::new(values).title("Owner statement")))
        })
    }
}

/// The owner statement series, numbered across the deployment.
const STATEMENT_SERIES: Series = Series {
    key: "test_owner_statement",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "OS",
    reset: Reset::Never,
    gapless: true,
    per_entity: false,
};

fn kinds() -> [KindSeed; 2] {
    [
        KindSeed::new("owner_statement", "Owner statement")
            .subject_type(STATEMENT)
            .numbered()
            .file_types(&["pdf"])
            .without_review(),
        KindSeed::new("statement_note", "Statement note")
            .subject_type(STATEMENT)
            .file_types(&["pdf"])
            .without_review(),
    ]
}

fn register() {
    register_data_provider(STATEMENT, Statements);
    register_template(
        TemplateSeed::new("owner_statement", "owner_statement")
            .name("Owner statement")
            .mode(LanguageMode::Bilingual)
            .english("Owner statement\nPeriod {{ data.period }}\nOwner {{ data.owner }}")
            .arabic("كشف حساب المالك\nالفترة {{ data.period }}")
            .footer("ACME Properties LLC, Dubai", "")
            .numbered(STATEMENT_SERIES),
    );
    register_template(
        TemplateSeed::new("statement_note", "statement_note")
            .mode(LanguageMode::En)
            .english("Note for {{ data.period }}"),
    );
}

async fn boot() -> TestApp {
    register();
    let app = TestApp::new(App::new().module(WlDocuments).provide(BrandAssets {
        company_name: "ACME Properties LLC".into(),
        company_name_ar: Some("أكمي للعقارات".into()),
        logo: Some(LOGO.to_vec()),
        accent: "#0b6e66".into(),
    }))
    .await;
    install_seeds(app.db(), kinds().iter()).await.unwrap();
    app
}

fn statement(id: i64) -> Subject {
    Subject::new(STATEMENT, id)
}

async fn pdf_of(app: &TestApp, document: &Document) -> Vec<u8> {
    let version = document.current_file(app.db()).await.unwrap().unwrap();
    version.read(app.state()).await.unwrap().unwrap().to_vec()
}

/// Done when 1 and 4: one accepted document with source `generated`, a
/// data hash, the next number of the series, and a PDF with the logo, the
/// footer number and "Page 1 of 1".
#[renox::test]
async fn generate_stores_one_accepted_generated_document_with_a_branded_pdf() {
    let app = boot().await;
    set_statement(101, json!({ "period": "2026-10", "owner": "Khalid A." }));

    let generated = generator(app.state())
        .generate(
            app.state(),
            GenerateRequest::new("owner_statement", statement(101)),
        )
        .await
        .unwrap()
        .expect("the statement is generated");
    assert!(!generated.reused);
    assert_eq!(generated.data_hash.len(), 64);

    let documents = Documents::of_subject(app.db(), &statement(101))
        .await
        .unwrap();
    assert_eq!(documents.len(), 1, "one document");
    let document = &documents[0];
    assert_eq!(document.id, generated.document.0);
    assert_eq!(document.status, DocumentStatus::Accepted);
    assert_eq!(document.source, DocumentSource::Generated);
    let number = generated.number.clone().expect("the template is numbered");
    assert!(
        number.starts_with("OS-") && number.ends_with("-0001"),
        "{number}"
    );
    assert_eq!(document.number.as_deref(), Some(number.as_str()));

    let generation = Generation::query()
        .where_(Generation::DOCUMENT_ID.eq(document.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the generation is recorded");
    assert_eq!(generation.data_hash, generated.data_hash);
    assert_eq!(generation.template_version, 1);
    assert_eq!(generation.number.as_deref(), Some(number.as_str()));

    let pdf = pdf_of(&app, document).await;
    assert!(pdf.starts_with(b"%PDF-1."));
    let raw = String::from_utf8_lossy(&pdf);
    assert!(
        raw.contains("/Subtype /Image") && raw.contains("/Logo Do"),
        "the logo is drawn"
    );
    assert!(
        pdf.windows(LOGO.len()).any(|w| w == LOGO),
        "the company logo is embedded"
    );
    let lines = text_lines(&pdf);
    assert!(lines.contains(&number), "the footer number: {lines:?}");
    assert!(lines.contains(&"Page 1 of 1".to_owned()), "{lines:?}");
    assert!(lines.contains(&"ACME Properties LLC".to_owned()));
    assert!(lines.contains(&"Period 2026-10".to_owned()));
    let version = document.current_file(app.db()).await.unwrap().unwrap();
    assert_eq!(version.mime_type, "application/pdf");
    assert!(version.file_name.ends_with(".pdf"));
}

/// Done when 2: the Arabic half is right to left: the text of an Arabic
/// line is in visual order, numbers kept left to right.
#[renox::test]
async fn the_arabic_half_is_right_to_left() {
    let app = boot().await;
    set_statement(102, json!({ "period": "2026-10", "owner": "Khalid A." }));
    let generated = Documents::generate(
        app.state(),
        GenerateRequest::new("owner_statement", statement(102)),
    )
    .await
    .unwrap()
    .unwrap();
    let document = Document::find_or_404(app.db(), generated.document.0)
        .await
        .unwrap();
    let lines = text_lines(&pdf_of(&app, &document).await);
    // The reference: "الفترة 2026-10" drawn from the left.
    let reference = "2026-10 ةرتفلا";
    assert_eq!(visual_order("الفترة 2026-10"), reference);
    assert!(lines.contains(&reference.to_owned()), "{lines:?}");
    assert!(lines.contains(&"كلاملا باسح فشك".to_owned()), "{lines:?}");
    assert!(
        !lines.contains(&"الفترة 2026-10".to_owned()),
        "never left to right"
    );
}

/// Done when 3: the same data returns the same document; changed data for
/// an issued template is refused.
#[renox::test]
async fn the_same_data_returns_the_same_document_and_changed_data_is_refused() {
    let app = boot().await;
    set_statement(103, json!({ "period": "2026-10", "owner": "Khalid A." }));
    let ask = || GenerateRequest::new("owner_statement", statement(103));
    let first = Documents::generate(app.state(), ask())
        .await
        .unwrap()
        .unwrap();
    let again = Documents::generate(app.state(), ask())
        .await
        .unwrap()
        .unwrap();
    assert!(again.reused);
    assert_eq!(again.document, first.document);
    assert_eq!(again.number, first.number);
    assert_eq!(
        Documents::of_subject(app.db(), &statement(103))
            .await
            .unwrap()
            .len(),
        1
    );

    set_statement(
        103,
        json!({ "period": "2026-10", "owner": "Khalid A. (corrected)" }),
    );
    let refused = Documents::generate(app.state(), ask())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        refused,
        GenerateRefusal::Issued {
            document: first.document
        }
    );
    assert_eq!(
        Documents::of_subject(app.db(), &statement(103))
            .await
            .unwrap()
            .len(),
        1,
        "an issued document is never regenerated"
    );
}

/// Done when 4: each new statement takes the next number of the series.
#[renox::test]
async fn a_numbered_template_assigns_the_next_number_of_the_series() {
    let app = boot().await;
    set_statement(104, json!({ "period": "2026-09", "owner": "Mona" }));
    set_statement(105, json!({ "period": "2026-10", "owner": "Mona" }));
    let a = Documents::generate(
        app.state(),
        GenerateRequest::new("owner_statement", statement(104)),
    )
    .await
    .unwrap()
    .unwrap();
    let b = Documents::generate(
        app.state(),
        GenerateRequest::new("owner_statement", statement(105)),
    )
    .await
    .unwrap()
    .unwrap();
    let n = |number: &Option<String>| -> i64 {
        number
            .as_deref()
            .and_then(|n| n.rsplit('-').next())
            .and_then(|n| n.parse().ok())
            .expect("a numbered document")
    };
    assert_eq!(n(&b.number), n(&a.number) + 1);
}

struct Broken;

impl PdfRenderer for Broken {
    fn render(&self, _job: &RenderJob) -> Result<Vec<u8>, RenderError> {
        Err(RenderError::new("the font is missing a glyph"))
    }
}

/// Done when 5: a rendering error stores nothing (and gives the number back).
#[renox::test]
async fn a_rendering_error_stores_nothing() {
    register();
    let app = TestApp::new(
        App::new()
            .module(WlDocuments)
            .provide(PdfRendering::new(Broken)),
    )
    .await;
    install_seeds(app.db(), kinds().iter()).await.unwrap();
    set_statement(106, json!({ "period": "2026-10", "owner": "Khalid A." }));
    let refused = Documents::generate(
        app.state(),
        GenerateRequest::new("owner_statement", statement(106)),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(matches!(refused, GenerateRefusal::Render(_)), "{refused:?}");
    assert!(
        Documents::of_subject(app.db(), &statement(106))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(Generation::query().count(app.db()).await.unwrap(), 0);
}

/// A template whose body names a field the data lacks fails to render and
/// stores nothing.
#[renox::test]
async fn a_template_with_an_unknown_field_stores_nothing() {
    let app = boot().await;
    register_template(
        TemplateSeed::new("broken_note", "statement_note")
            .mode(LanguageMode::En)
            .english("{{ data.nothing_here }}"),
    );
    set_statement(107, json!({ "period": "2026-10" }));
    let refused = Documents::generate(
        app.state(),
        GenerateRequest::new("broken_note", statement(107)),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(matches!(refused, GenerateRefusal::Render(_)), "{refused:?}");
    assert!(
        Documents::of_subject(app.db(), &statement(107))
            .await
            .unwrap()
            .is_empty()
    );
}

/// An edited template is a new version; the earlier document keeps its own.
#[renox::test]
async fn an_edited_template_is_a_new_version_and_old_documents_keep_theirs() {
    let app = boot().await;
    set_statement(108, json!({ "period": "2026-10" }));
    let first = Documents::generate(
        app.state(),
        GenerateRequest::new("statement_note", statement(108)),
    )
    .await
    .unwrap()
    .unwrap();
    let template = DocumentTemplate::active(app.db(), "statement_note")
        .await
        .unwrap()
        .unwrap();
    let revised = template
        .revise(
            app.db(),
            None,
            TemplateWording {
                body_en: "Revised note for {{ data.period }}".into(),
                ..TemplateWording::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(revised.version, 2);
    let active = DocumentTemplate::active(app.db(), "statement_note")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.version, 2);
    let generation = Generation::query()
        .where_(Generation::DOCUMENT_ID.eq(first.document.0))
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(generation.template_version, 1);
}
