//! Booting wl-hr in a `TestApp` with auth, as `crates/wl-sales/tests/support`
//! boots wl-sales: the roles onboarding uses, a notifier that records what it
//! is asked for, the kernel events recorded, and fakes for the checklist
//! runs of ticketing, the SOP acknowledgements and the document guard of
//! documents.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::auth::permissions::Scope;
use renox::auth::{Auth, Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_hr::permissions as perm;
use wl_hr::{
    Employee, EmploymentType, NewEmployee, Onboarding, OnboardingStep, WlHr, employees,
    template::{self, StepInput, TemplateInput},
};
use wl_kernel::documents::{
    BlockEntry, CheckOutcome, DocumentFuture, DocumentGuard, FlagFailure, RecordFlag, SopFuture,
    SopStaff, Validity, provide_guard, provide_sop_acknowledgements, SopAcknowledgements,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest, provide_notifier};
use wl_kernel::tickets::{ChecklistRunId, Checklists, TicketFuture, provide_checklists};
use wl_kernel::workforce::{
    EmployeeActivated, EmployeeAssignmentChanged, OnboardingCompleted, OnboardingStarted,
};
use wl_kernel::{PartyId, SopId, Subject};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the HR module (and the parties it needs) on.
const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "hr"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
default = true
"#;

/// Branch B, where the tests' employees work.
pub fn branch() -> Subject {
    Subject::new("test.branch", 2)
}

/// Another branch.
pub fn other_branch() -> Subject {
    Subject::new("test.branch", 3)
}

/// The scope of [`branch`].
pub fn branch_scope() -> Scope {
    Scope::new("test.branch", 2)
}

/// A date in November 2026.
pub fn nov(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
}

/// Records what the notifier is asked for.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request so far.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The requests of type `key`.
    pub fn of_type(&self, key: &str) -> Vec<NotifyRequest> {
        self.all().into_iter().filter(|r| r.type_key == key).collect()
    }

    /// Forgets what was recorded.
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }
}

/// The checklist runs ticketing would start: a counter, and a log.
#[derive(Clone, Default)]
pub struct ChecklistDesk {
    /// `(template, subject, run)` of each run started.
    pub started: Arc<Mutex<Vec<(String, Subject, i64)>>>,
    /// `(run, reason)` of each run voided.
    pub voided: Arc<Mutex<Vec<(i64, String)>>>,
}

impl Checklists for ChecklistDesk {
    fn start<'a>(
        &'a self,
        _state: &'a AppState,
        template: &'a str,
        subject: Subject,
    ) -> TicketFuture<'a, Option<ChecklistRunId>> {
        Box::pin(async move {
            let mut started = self.started.lock().unwrap();
            let run = i64::try_from(started.len()).unwrap_or(0) + 100;
            started.push((template.to_owned(), subject, run));
            Ok(Some(ChecklistRunId(run)))
        })
    }

    fn void<'a>(
        &'a self,
        _state: &'a AppState,
        run: ChecklistRunId,
        reason: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            self.voided.lock().unwrap().push((run.0, reason.to_owned()));
            Ok(true)
        })
    }
}

/// The SOPs pending for every employee, and the acknowledgements recorded in
/// person.
#[derive(Clone, Default)]
pub struct SopDesk {
    /// The SOPs pending.
    pub pending: Arc<Mutex<Vec<SopId>>>,
    /// `(who, signature, witness)` of each in-person acknowledgement.
    pub in_person: Arc<Mutex<Vec<(SopStaff, String, i64)>>>,
}

impl SopDesk {
    /// Makes `sops` pending for everyone.
    pub fn set_pending(&self, sops: &[i64]) {
        *self.pending.lock().unwrap() = sops.iter().map(|id| SopId(*id)).collect();
    }
}

impl SopAcknowledgements for SopDesk {
    fn pending_for<'a>(
        &'a self,
        _state: &'a AppState,
        _who: &'a SopStaff,
    ) -> SopFuture<'a, Vec<SopId>> {
        Box::pin(async move { Ok(self.pending.lock().unwrap().clone()) })
    }

    fn acknowledge_in_person<'a>(
        &'a self,
        _state: &'a AppState,
        who: &'a SopStaff,
        signature: &'a str,
        witness: i64,
    ) -> SopFuture<'a, Vec<SopId>> {
        Box::pin(async move {
            let done = std::mem::take(&mut *self.pending.lock().unwrap());
            self.in_person
                .lock()
                .unwrap()
                .push((who.clone(), signature.to_owned(), witness));
            Ok(done)
        })
    }
}

/// The accepted documents of the employees: `(subject, kind)` to a document id.
#[derive(Clone, Default)]
pub struct DocDesk(Arc<Mutex<BTreeMap<(String, String), i64>>>);

impl DocDesk {
    /// `subject` has an accepted document `document` of `kind`.
    pub fn accepted(&self, subject: &Subject, kind: &str, document: i64) {
        self.0
            .lock()
            .unwrap()
            .insert((subject.to_string(), kind.to_owned()), document);
    }
}

impl DocumentGuard for DocDesk {
    fn blocks_for<'a>(
        &'a self,
        _db: &'a Db,
        _guarded: &'a str,
    ) -> DocumentFuture<'a, Vec<BlockEntry>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn is_valid<'a>(
        &'a self,
        _db: &'a Db,
        subject: &'a Subject,
        kind: &'a str,
        _on_date: NaiveDate,
    ) -> DocumentFuture<'a, Validity> {
        Box::pin(async move {
            let found = self
                .0
                .lock()
                .unwrap()
                .get(&(subject.to_string(), kind.to_owned()))
                .copied();
            Ok(match found {
                Some(id) => Validity::Valid {
                    document: Some(wl_kernel::DocumentId(id)),
                    expires_on: None,
                },
                None => Validity::Invalid {
                    reason: wl_kernel::InvalidReason::Missing,
                    document: None,
                    expired_on: None,
                    pending: None,
                },
            })
        })
    }

    fn check_as<'a>(
        &'a self,
        _state: &'a AppState,
        _guarded: &'a str,
        _record_id: i64,
        _on_date: NaiveDate,
        _actor: Option<i64>,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, wl_kernel::BlockRefusal>> {
        Box::pin(async { Ok(Ok(CheckOutcome::default())) })
    }

    fn record_flags<'a>(
        &'a self,
        _db: &'a Db,
        _guarded: &'a str,
        _record_id: i64,
        _flags: &'a [FlagFailure],
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn flags_for<'a>(
        &'a self,
        _db: &'a Db,
        _record_type: &'a str,
        _record_id: i64,
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn open_flag_counts<'a>(
        &'a self,
        _db: &'a Db,
        _record_type: &'a str,
        _ids: &'a [i64],
    ) -> DocumentFuture<'a, BTreeMap<i64, u64>> {
        Box::pin(async { Ok(BTreeMap::new()) })
    }

    fn review_flag<'a>(
        &'a self,
        _db: &'a Db,
        _flag_id: i64,
        _reviewer: i64,
        _note: &'a str,
    ) -> DocumentFuture<'a, RecordFlag> {
        Box::pin(async { Err(Error::NotFound) })
    }
}

/// An event log.
#[derive(Clone)]
pub struct Recorded<E>(pub Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    /// What was recorded.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

/// Records the events and provides the fakes.
struct Fakes {
    notices: Recording,
    checklists: ChecklistDesk,
    sops: SopDesk,
    docs: DocDesk,
    started: Arc<Mutex<Vec<OnboardingStarted>>>,
    completed: Arc<Mutex<Vec<OnboardingCompleted>>>,
    activated: Arc<Mutex<Vec<EmployeeActivated>>>,
    changed: Arc<Mutex<Vec<EmployeeAssignmentChanged>>>,
}

/// Records `event`s of type `E` into `log`.
fn record<E: Event + Clone + Send + Sync + 'static>(app: &mut Registry, log: &Arc<Mutex<Vec<E>>>) {
    let log = log.clone();
    app.listen(move |event: E, _state: AppState| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(event);
            Ok(())
        }
    });
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.notices.clone());
        provide_checklists(app, self.checklists.clone());
        provide_sop_acknowledgements(app, self.sops.clone());
        provide_guard(app, self.docs.clone());
        record(app, &self.started);
        record(app, &self.completed);
        record(app, &self.activated);
        record(app, &self.changed);
    }
}

/// The app under test with the fakes and the events.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// The checklist runs.
    pub checklists: ChecklistDesk,
    /// The SOP acknowledgements.
    pub sops: SopDesk,
    /// The accepted documents.
    pub docs: DocDesk,
    /// `OnboardingStarted`.
    pub started: Recorded<OnboardingStarted>,
    /// `OnboardingCompleted`.
    pub completed: Recorded<OnboardingCompleted>,
    /// `EmployeeActivated`.
    pub activated: Recorded<EmployeeActivated>,
    /// `EmployeeAssignmentChanged`.
    pub changed: Recorded<EmployeeAssignmentChanged>,
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

    /// A user with no role.
    pub async fn user(&self, name: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        User::register(self.db(), name, &email, "password123")
            .await
            .unwrap()
    }

    /// A user holding `role` in the scope of [`branch`].
    pub async fn user_in_branch(&self, name: &str, role: &str) -> User {
        let user = self.user(name).await;
        user.assign_role_in(self.db(), role, &branch_scope())
            .await
            .unwrap();
        user
    }

    /// A user holding `role` everywhere.
    pub async fn user_with(&self, name: &str, role: &str) -> User {
        let user = self.user(name).await;
        user.assign_role(self.db(), role).await.unwrap();
        user
    }

    /// Ravi K., a driver hired on 2 Nov at branch B, with an account.
    pub async fn ravi(&self) -> (Employee, User) {
        let account = self.user("Ravi").await;
        let employee = employees::create(
            self.state(),
            NewEmployee::new(PartyId(50), "driver", nov(2))
                .scope(branch())
                .user(account.id)
                .job_title("Driver")
                .status(wl_hr::EmployeeStatus::Onboarding),
        )
        .await
        .unwrap();
        (employee, account)
    }

    /// The onboardings of `employee`, oldest first.
    pub async fn onboardings(&self, employee: &Employee) -> Vec<Onboarding> {
        let mut all = Onboarding::of_employee(self.db(), employee.id).await.unwrap();
        all.reverse();
        all
    }

    /// The step of `onboarding` titled `title`.
    pub async fn step(&self, onboarding: &Onboarding, title: &str) -> OnboardingStep {
        onboarding
            .steps(self.db())
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.title_en == title)
            .unwrap_or_else(|| panic!("no step titled {title}"))
    }

    /// `step`, as stored now.
    pub async fn reload(&self, step: &OnboardingStep) -> OnboardingStep {
        OnboardingStep::find_or_404(self.db(), step.id).await.unwrap()
    }
}

/// `WlHr` booted with auth, the roles onboarding uses and the induction
/// template installed.
pub async fn boot() -> Booted {
    boot_with(WlHr::new(), true).await
}

/// [`boot`] with `module`, and the default induction template when
/// `induction` is true.
pub async fn boot_with(module: WlHr, induction: bool) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let fakes = Fakes {
        notices: Recording::default(),
        checklists: ChecklistDesk::default(),
        sops: SopDesk::default(),
        docs: DocDesk::default(),
        started: Arc::default(),
        completed: Arc::default(),
        activated: Arc::default(),
        changed: Arc::default(),
    };
    let (notices, checklists, sops, docs) = (
        fakes.notices.clone(),
        fakes.checklists.clone(),
        fakes.sops.clone(),
        fakes.docs.clone(),
    );
    let (started, completed, activated, changed) = (
        Recorded(fakes.started.clone()),
        Recorded(fakes.completed.clone()),
        Recorded(fakes.activated.clone()),
        Recorded(fakes.changed.clone()),
    );
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(module)
        .module(fakes);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    // HR holds the onboarding permissions; the owner role holds
    // `onboarding.manage` as the second person; `manager` holds none.
    let hr = [
        perm::EMPLOYEES_VIEW,
        perm::EMPLOYEES_MANAGE,
        perm::ONBOARDING_VIEW,
        perm::ONBOARDING_MANAGE,
    ];
    permissions::define_role(app.db(), "hr", &hr).await.unwrap();
    permissions::define_role(app.db(), "owner", &[perm::ONBOARDING_VIEW, perm::ONBOARDING_MANAGE])
        .await
        .unwrap();
    permissions::define_role(app.db(), "manager", &[perm::EMPLOYEES_VIEW])
        .await
        .unwrap();
    if induction {
        wl_hr::seed::install(app.db()).await.unwrap().total();
    }
    Booted {
        app,
        _views: views,
        notices,
        checklists,
        sops,
        docs,
        started,
        completed,
        activated,
        changed,
    }
}

/// The "Driver" template: one step each of the kinds.
pub fn driver_template() -> TemplateInput {
    TemplateInput::for_role("Driver", "driver")
        .step(StepInput::task("Read the handbook", "employee", 5))
        .step(StepInput::task("Copy of driving licence", "employee", 5).evidence())
        .step(StepInput::document("Emirates ID", "hr", 5, "emirates_id"))
        .step(StepInput::task("Optional extra", "hr", 6).optional())
}

/// The "Supervisor" template for a manual onboarding.
pub fn supervisor_template() -> TemplateInput {
    TemplateInput::for_role("Supervisor", "supervisor")
        .step(StepInput::task("Supervisor briefing", "manager", 2))
}

/// Saves `input` as a template and returns it.
pub async fn add_template(t: &Booted, input: TemplateInput) -> wl_hr::OnboardingTemplate {
    template::create(t.db(), input)
        .await
        .unwrap()
        .expect("the template is valid")
}

/// A full-time employee type, for readability in the tests.
pub const FULL_TIME: EmploymentType = EmploymentType::FullTime;
