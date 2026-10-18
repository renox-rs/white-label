//! Booting wl-documents in a `TestApp`, as `crates/wl-parties/tests/support`
//! boots wl-parties: the kinds of the acceptance checks, the guarded
//! transitions' `BlockSubjects` providers (no module guards a transition
//! yet, #45 A1) and the few actions that need a logged-in user.

#![allow(dead_code)]

use renox::auth::{Permissions, permissions};
use renox::axum::extract::Form;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use serde::Deserialize;
use wl_documents::{
    Document, DocumentKind, Documents, KindSeed, NewDocument, WlDocuments, permissions as perm,
};
use wl_kernel::Subject;
use wl_kernel::documents::{
    BlockCondition, BlockEntry, BlockMode, BlockSubjects, DocumentFuture, register_block_subjects,
    today,
};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// The employee "Ravi S." (deployment 7, run 7).
pub const RAVI: i64 = 7;
/// The vendor "SafeFire LLC".
pub const SAFEFIRE: i64 = 9;
/// The fire alarm ticket of SafeFire.
pub const FIRE_TICKET: i64 = 1;
/// The plumbing ticket of SafeFire.
pub const PLUMBING_TICKET: i64 = 2;

/// Ravi, the worker.
pub fn ravi() -> Subject {
    Subject::new("hr.employee", RAVI)
}

/// Ravi's vehicle.
pub fn truck() -> Subject {
    Subject::new("fleet.vehicle", RAVI)
}

/// SafeFire LLC.
pub fn safefire() -> Subject {
    Subject::new("party.vendor", SAFEFIRE)
}

/// `days` after today (Asia/Dubai); before today when negative.
pub fn day(days: i64) -> NaiveDate {
    let today = today();
    if days >= 0 {
        today + Days::new(days.unsigned_abs())
    } else {
        today - Days::new(days.unsigned_abs())
    }
}

/// A small PDF.
pub fn pdf(name: &str) -> Upload {
    Upload::new(name, "application/pdf", b"%PDF-1.7 acceptance".to_vec())
}

/// The residence visa: a legal block on `deployment.activate`, renewed by PRO.
pub fn residence_visa() -> KindSeed {
    KindSeed::new("residence_visa", "Residence visa")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("PRO")
        .legal_block()
        .blocks("deployment.activate", BlockMode::Block)
}

/// The work permit: a legal block on `deployment.activate`.
pub fn work_permit() -> KindSeed {
    KindSeed::new("work_permit", "Work permit")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("PRO")
        .legal_block()
        .blocks("deployment.activate", BlockMode::Block)
}

/// The driver's licence, guarding `run.dispatch`.
pub fn driving_licence() -> KindSeed {
    KindSeed::new("driving_licence", "Driving licence")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("PRO")
        .blocks("run.dispatch", BlockMode::Block)
}

/// The vehicle insurance, guarding `run.dispatch`.
pub fn vehicle_insurance() -> KindSeed {
    KindSeed::new("vehicle_insurance", "Vehicle insurance")
        .subject_type("fleet.vehicle")
        .expires()
        .responsible_role("fleet_manager")
        .blocks("run.dispatch", BlockMode::Block)
}

/// The Civil Defence approval of a vendor: only fire and life safety tickets.
pub fn civil_defence() -> KindSeed {
    KindSeed::new("civil_defence_approval", "Civil Defence approval")
        .subject_type("party.vendor")
        .expires()
        .responsible_role("vendor_manager")
        .legal_block()
        .blocks_when(
            "ticket.resolve",
            BlockMode::Block,
            BlockCondition::new("category", ["fire_alarm", "sprinkler"]),
        )
}

/// `deployment.activate`: the deployment's worker (deployment `n` is worker
/// `n`), labelled "Ravi S." for Ravi.
struct Deployments;

impl BlockSubjects for Deployments {
    fn label<'a>(
        &'a self,
        _state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        let label = (subject.subject_id == RAVI).then(|| "Ravi S.".to_owned());
        Box::pin(async move { Ok(label) })
    }

    fn subjects<'a>(
        &'a self,
        _state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move { Ok(vec![Subject::new("hr.employee", record_id)]) })
    }
}

/// `run.dispatch`: the driver for a licence, the vehicle for insurance.
struct Runs;

impl BlockSubjects for Runs {
    fn subjects<'a>(
        &'a self,
        _state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            Ok(vec![match entry.kind.as_str() {
                "vehicle_insurance" => Subject::new("fleet.vehicle", record_id),
                _ => Subject::new("hr.employee", record_id),
            }])
        })
    }
}

/// `ticket.resolve`: the vendor, for the tickets that match the condition.
struct Tickets;

impl BlockSubjects for Tickets {
    fn subjects<'a>(
        &'a self,
        _state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let category = match record_id {
                FIRE_TICKET => "fire_alarm",
                _ => "plumbing",
            };
            Ok(if entry.applies_to(Some(category)) {
                vec![Subject::new("party.vendor", SAFEFIRE)]
            } else {
                Vec::new()
            })
        })
    }
}

/// The practitioner whose visits are booked.
pub const PRACTITIONER: i64 = 5;
/// The tutor whose sessions are planned.
pub const TUTOR: i64 = 3;

/// The practitioner licence, blocking `booking.book`.
pub fn practitioner_licence() -> KindSeed {
    KindSeed::new("practitioner_licence", "Practitioner licence")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("HR")
        .blocks("booking.book", BlockMode::Block)
}

/// The tutor's clearance, blocking `session.start`.
pub fn tutor_clearance() -> KindSeed {
    KindSeed::new("tutor_clearance", "Tutor clearance")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("HR")
        .blocks("session.start", BlockMode::Block)
}

/// `booking.book`: the practitioner of the visit; a bookable slot.
struct Visits;

impl BlockSubjects for Visits {
    fn subjects<'a>(
        &'a self,
        _state: &'a AppState,
        _entry: &'a BlockEntry,
        _record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move { Ok(vec![Subject::new("hr.employee", PRACTITIONER)]) })
    }

    fn bookable(&self) -> bool {
        true
    }
}

/// `session.start`: the tutor of every session.
struct Sessions;

impl BlockSubjects for Sessions {
    fn subjects<'a>(
        &'a self,
        _state: &'a AppState,
        _entry: &'a BlockEntry,
        _record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move { Ok(vec![Subject::new("hr.employee", TUTOR)]) })
    }
}

/// Ravi's attendance of 2026-12-02 (the attendance `n` of worker `n`).
pub const RAVI_ATTENDANCE: i64 = 7;
/// Another worker's attendance, whose documents are in order.
pub const MONA_ATTENDANCE: i64 = 8;

/// `attendance_record.accept`: the worker (attendance `n` is worker `n`).
struct Attendance;

impl BlockSubjects for Attendance {
    fn subjects<'a>(
        &'a self,
        _state: &'a AppState,
        _entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move { Ok(vec![Subject::new("hr.employee", record_id)]) })
    }
}

/// The work permit in flag mode on `attendance_record.accept`.
pub fn flagged_work_permit() -> KindSeed {
    KindSeed::new("work_permit", "Work permit")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("PRO")
        .blocks("attendance_record.accept", BlockMode::Flag)
}

/// A medical certificate in block mode on `attendance_record.accept`.
pub fn medical_certificate() -> KindSeed {
    KindSeed::new("medical_certificate", "Medical certificate")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("HR")
        .blocks("attendance_record.accept", BlockMode::Block)
}

/// The app under test.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
}

impl Booted {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// Installs `seeds`.
    pub async fn install(&self, seeds: &[KindSeed]) {
        wl_documents::kinds::install_seeds(self.db(), seeds)
            .await
            .expect("the kinds install");
    }

    /// A user holding exactly `grants` (through a role of their own).
    pub async fn user(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        user.assign_role(self.db(), &role).await.unwrap();
        user
    }

    /// Reviews documents.
    pub async fn reviewer(&self) -> User {
        self.user("Rita", &[perm::VIEW, perm::REVIEW]).await
    }

    /// Edits the kinds.
    pub async fn kind_admin(&self) -> User {
        self.user("Kamal", &[perm::VIEW, perm::KINDS_MANAGE]).await
    }

    /// Attaches a file to `subject` for review, expiring on `expiry`.
    pub async fn attach(
        &self,
        subject: Subject,
        kind: &str,
        expiry: Option<NaiveDate>,
    ) -> Document {
        let mut new = NewDocument::new(subject, kind, pdf("scan.pdf"));
        if let Some(expiry) = expiry {
            new = new.expires_on(expiry);
        }
        Documents::attach(self.state(), None, new)
            .await
            .unwrap()
            .expect("the document is attached")
    }

    /// `document`, as stored now.
    pub async fn reload(&self, document: &Document) -> Document {
        Document::find(self.db(), document.id)
            .await
            .unwrap()
            .unwrap()
    }

    /// The kind with this key.
    pub async fn kind(&self, key: &str) -> DocumentKind {
        DocumentKind::by_key(self.db(), key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no kind `{key}`"))
    }

    /// `reviewer` accepts `document` on the review queue, as the page does.
    pub async fn accept(&self, reviewer: &User, document: &Document) -> Document {
        self.app.acting_as(reviewer);
        self.app
            .htmx()
            .post(&format!("/documents/{}/accept", document.id), &[])
            .await
            .assert_ok();
        self.reload(document).await
    }

    /// Attaches a document and has `reviewer` accept it.
    pub async fn accepted(
        &self,
        reviewer: &User,
        subject: Subject,
        kind: &str,
        expiry: Option<NaiveDate>,
    ) -> Document {
        let document = self.attach(subject, kind, expiry).await;
        self.accept(reviewer, &document).await
    }
}

/// `record`, `resubmit` and `archive` take the logged-in user, and only a
/// request has one; these routes run them for the tests.
struct Harness;

#[derive(Deserialize)]
struct FileForm {
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    size: usize,
    #[serde(default)]
    subject_id: i64,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    expires_on: Option<NaiveDate>,
    #[serde(default)]
    reason: Option<String>,
}

impl FileForm {
    fn upload(&self) -> Upload {
        let mut bytes = b"%PDF-1.7 acceptance".to_vec();
        bytes.resize(self.size.max(bytes.len()), b'.');
        Upload::new(&self.file_name, "application/pdf", bytes)
    }
}

async fn record(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<FileForm>,
) -> Result<String> {
    let mut new = NewDocument::new(
        Subject::new("hr.employee", form.subject_id),
        &form.kind,
        form.upload(),
    );
    if let Some(expiry) = form.expires_on {
        new = new.expires_on(expiry);
    }
    match Documents::record(&state, &user, new).await? {
        Ok(document) => Ok(document.id.to_string()),
        Err(refused) => Err(refused.into_error()),
    }
}

async fn resubmit(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<FileForm>,
) -> Result<String> {
    let document = Document::find_or_404(&state.db, id).await?;
    match Documents::resubmit(&state, &user, &document, form.upload(), None).await {
        Ok(document) => Ok(document.current_version.to_string()),
        Err(error) => Err(error.into_error()),
    }
}

async fn archive(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<FileForm>,
) -> Result<String> {
    let document = Document::find_or_404(&state.db, id).await?;
    match Documents::archive(&state, &user, &document, form.reason.as_deref()).await {
        Ok(document) => Ok(document.status.as_str().to_owned()),
        Err(error) => Err(error.into_error()),
    }
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "documents-test-harness"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/test/documents/record", record)
            .post("/test/documents/{id}/resubmit", resubmit)
            .post("/test/documents/{id}/archive", archive)
    }
}

/// wl-documents booted in an app with auth, with the providers of the
/// guarded transitions registered and no kind installed yet.
pub async fn boot() -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    register_block_subjects("deployment.activate", Deployments);
    register_block_subjects("run.dispatch", Runs);
    register_block_subjects("ticket.resolve", Tickets);
    register_block_subjects("attendance_record.accept", Attendance);
    register_block_subjects("booking.book", Visits);
    register_block_subjects("session.start", Sessions);

    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlDocuments)
        .module(Harness);
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    // Events are recorded for `assert_emitted`; nothing here listens.
    app.fake_events();
    Booted { app, _views: views }
}
