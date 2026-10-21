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
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_hr::permissions as perm;
use wl_hr::{
    Adjustment, AdjustmentKind, Employee, EmploymentType, LineStatus, NewEmployee, Onboarding,
    OnboardingStep, PayrollLine, PayrollRun, PayrollRunStatus, WlHr, employees,
    template::{self, StepInput, TemplateInput},
};
use wl_kernel::documents::{
    BlockEntry, CheckOutcome, DocumentFuture, DocumentGuard, FlagFailure, RecordFlag,
    SopAcknowledgements, SopFuture, SopStaff, Validity, provide_guard,
    provide_sop_acknowledgements,
};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::tickets::{
    ChecklistRunId, Checklists, OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef,
    provide_checklists, provide_task_creator,
};
use wl_kernel::workforce::{
    AbsenceCaseOpened, EmployeeAbsconded, EmployeeActivated, EmployeeAssignmentChanged,
    OnboardingCompleted, OnboardingStarted, PayrollLinesFailed, PayrollRunReturned,
    QualificationExpired, QualificationExpiring,
};
use wl_kernel::{PartyId, SopId, Subject};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// The users a notice is for, sorted.
pub fn users(request: &NotifyRequest) -> Vec<i64> {
    let mut ids: Vec<i64> = request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids
}

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

/// The scope of `subject`.
pub fn branch_scope_of(subject: &Subject) -> Scope {
    Scope::new(&subject.subject_type, subject.subject_id)
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
        self.all()
            .into_iter()
            .filter(|r| r.type_key == key)
            .collect()
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

/// What the document desk holds for one subject and kind.
#[derive(Clone, Default)]
struct DocEntry {
    /// The accepted document and its expiry date.
    accepted: Option<(i64, Option<NaiveDate>)>,
    /// A renewal waiting for review.
    pending: Option<i64>,
}

/// The documents of the employees: `(subject, kind)` to what is held. Its
/// `is_valid` follows `document_valid` (wl-documents §5.6): valid on or
/// before the expiry date, a renewal that waits for review is no answer.
#[derive(Clone, Default)]
pub struct DocDesk(Arc<Mutex<BTreeMap<(String, String), DocEntry>>>);

impl DocDesk {
    /// `subject` has an accepted document `document` of `kind` that does not
    /// expire.
    pub fn accepted(&self, subject: &Subject, kind: &str, document: i64) {
        self.accepted_until(subject, kind, document, None);
    }

    /// `subject` has an accepted document `document` of `kind`, valid
    /// through `expires_on`; any renewal waiting is settled.
    pub fn accepted_until(
        &self,
        subject: &Subject,
        kind: &str,
        document: i64,
        expires_on: Option<NaiveDate>,
    ) {
        let mut all = self.0.lock().unwrap();
        let entry = all
            .entry((subject.to_string(), kind.to_owned()))
            .or_default();
        entry.accepted = Some((document, expires_on));
        entry.pending = None;
    }

    /// A renewal `document` of `kind` was uploaded and waits for review.
    pub fn pending_review(&self, subject: &Subject, kind: &str, document: i64) {
        let mut all = self.0.lock().unwrap();
        all.entry((subject.to_string(), kind.to_owned()))
            .or_default()
            .pending = Some(document);
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
        on_date: NaiveDate,
    ) -> DocumentFuture<'a, Validity> {
        Box::pin(async move {
            let entry = self
                .0
                .lock()
                .unwrap()
                .get(&(subject.to_string(), kind.to_owned()))
                .cloned()
                .unwrap_or_default();
            Ok(match entry {
                DocEntry {
                    accepted: Some((id, expires_on)),
                    ..
                } if expires_on.is_none_or(|day| day >= on_date) => Validity::Valid {
                    document: Some(wl_kernel::DocumentId(id)),
                    expires_on,
                },
                DocEntry {
                    pending: Some(id),
                    accepted,
                    ..
                } => Validity::Invalid {
                    reason: wl_kernel::InvalidReason::PendingReview,
                    document: Some(wl_kernel::DocumentId(id)),
                    expired_on: accepted.and_then(|(_, expires_on)| expires_on),
                    pending: Some(wl_kernel::documents::PendingRenewal {
                        document: wl_kernel::DocumentId(id),
                        submitted_on: on_date,
                    }),
                },
                DocEntry {
                    accepted: Some((id, expires_on)),
                    ..
                } => Validity::Invalid {
                    reason: wl_kernel::InvalidReason::Expired,
                    document: Some(wl_kernel::DocumentId(id)),
                    expired_on: expires_on,
                    pending: None,
                },
                _ => Validity::Invalid {
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
    qualification_expired: Arc<Mutex<Vec<QualificationExpired>>>,
    qualification_expiring: Arc<Mutex<Vec<QualificationExpiring>>>,
    absconded: Arc<Mutex<Vec<EmployeeAbsconded>>>,
    returned: Arc<Mutex<Vec<PayrollRunReturned>>>,
    failed: Arc<Mutex<Vec<PayrollLinesFailed>>>,
    tasks: TicketDesk,
}

/// A `TaskCreator` that records the tickets it opens. A ticket opened with a
/// source key that is still open is the same ticket (`find_open`).
#[derive(Clone, Default)]
pub struct TicketDesk {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
}

impl TicketDesk {
    /// The tickets opened, oldest first.
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }

    /// The tickets opened with a source key that starts with `prefix`.
    pub fn with_key(&self, prefix: &str) -> Vec<OpenTicket> {
        self.opened()
            .into_iter()
            .filter(|t| {
                t.source_key
                    .as_deref()
                    .is_some_and(|k| k.starts_with(prefix))
            })
            .collect()
    }
}

impl TaskCreator for TicketDesk {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut opened = self.opened.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let opened = self.opened.lock().unwrap();
            Ok(opened
                .iter()
                .position(|t| t.source_key.as_deref() == Some(source_key))
                .map(|n| TicketRef {
                    id: TicketId(n as i64 + 1),
                    number: format!("TKT-{}", n + 1),
                }))
        })
    }
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
        record(app, &self.qualification_expired);
        record(app, &self.qualification_expiring);
        record(app, &self.absconded);
        record(app, &self.returned);
        record(app, &self.failed);
        provide_task_creator(app, self.tasks.clone());
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
    /// `QualificationExpired`.
    pub qualification_expired: Recorded<QualificationExpired>,
    /// `QualificationExpiring`.
    pub qualification_expiring: Recorded<QualificationExpiring>,
    /// `EmployeeAbsconded`.
    pub absconded: Recorded<EmployeeAbsconded>,
    /// `PayrollRunReturned`.
    pub returned: Recorded<PayrollRunReturned>,
    /// `PayrollLinesFailed`.
    pub failed: Recorded<PayrollLinesFailed>,
    /// The tasks (tickets) opened for staff.
    pub tasks: TicketDesk,
    /// Days the clock was moved forward from the real one.
    clock_days: std::sync::atomic::AtomicI64,
    /// The absence cases opened so far.
    cases: std::sync::atomic::AtomicI64,
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

    /// An employee in `status`, hired on 2 Nov at branch B, inserted as a
    /// fixture (the machine would not take it there).
    pub async fn employee_in(&self, party: i64, status: wl_hr::EmployeeStatus) -> Employee {
        let mut employee = Employee {
            party_id: party,
            job_role: "driver".into(),
            job_title: "Driver".into(),
            hire_date: nov(2),
            status,
            ..Employee::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        employee
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        wl_hr::EmployeeScope::create(
            &mut tx,
            wl_hr::EmployeeScope {
                employee_id: employee.id,
                scope_type: branch().subject_type.clone(),
                scope_id: branch().subject_id.to_string(),
                ..wl_hr::EmployeeScope::default()
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        employee
    }

    /// Runs `work` with the clock on `day` (Asia/Dubai), moved forward from
    /// the real one; days only go forward within a test.
    pub async fn on<F: std::future::Future>(&self, day: NaiveDate, work: F) -> F::Output {
        use std::sync::atomic::Ordering;
        let real = wl_kernel::documents::dubai_date(renox::db::now());
        let wanted = (day - real).num_days();
        let moved = self.clock_days.load(Ordering::SeqCst);
        assert!(wanted >= moved, "the clock cannot go back to {day}");
        if wanted > moved {
            let seconds = u64::try_from(wanted - moved).unwrap() * 24 * 3600;
            self.app.travel(std::time::Duration::from_secs(seconds));
            self.clock_days.store(wanted, Ordering::SeqCst);
        }
        self.app.at_travelled_time(work).await
    }

    /// An absence case of a vertical opened for `employee` on `opened_on`,
    /// counting from `last_present`, as the vertical tells wl-hr.
    pub async fn open_case(
        &self,
        employee: &Employee,
        opened_on: NaiveDate,
        last_present: NaiveDate,
    ) {
        let id = self.cases.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.state()
            .emit(AbsenceCaseOpened {
                employee: employee.employee_id(),
                case: Subject::new("ticketing.ticket", id),
                opened_on,
                last_present_on: last_present,
            })
            .await
            .unwrap();
    }

    /// A failed attempt (no answer) to reach `employee` on `day`.
    pub async fn attempt(&self, employee: &Employee, day: NaiveDate) {
        wl_hr::ContactAttempt::create(
            self.db(),
            wl_hr::ContactAttempt {
                employee_id: employee.id,
                attempted_on: day,
                method: "phone".into(),
                outcome: wl_hr::ContactOutcome::NoAnswer,
                ..wl_hr::ContactAttempt::default()
            },
        )
        .await
        .unwrap();
    }

    /// `employee` has been absent for long: an open absence case that counts
    /// from 2026-01-01 and two failed attempts on 2026-01-05 and 2026-01-06,
    /// whatever the clock says.
    pub async fn make_absent(&self, employee: &Employee) {
        let day = |m: u32, d: u32| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        self.open_case(employee, day(1, 9), day(1, 1)).await;
        self.attempt(employee, day(1, 5)).await;
        self.attempt(employee, day(1, 6)).await;
    }

    /// `employee`, as stored now.
    pub async fn employee(&self, employee: &Employee) -> Employee {
        Employee::find_or_404(self.db(), employee.id).await.unwrap()
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

    /// A payroll run of `period` in `status`, inserted as a fixture (the
    /// machine would not take it there).
    pub async fn payroll_run(&self, period: &str, status: PayrollRunStatus) -> PayrollRun {
        let mut run = PayrollRun {
            period: period.to_owned(),
            run_type: "regular".into(),
            status,
            ..PayrollRun::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        run.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        run
    }

    /// A line of `run` for `employee` with a fixed wage and a net, in fils.
    pub async fn payroll_line(
        &self,
        run: &PayrollRun,
        employee: &Employee,
        wage: i64,
        net: i64,
    ) -> PayrollLine {
        PayrollLine::create(
            self.db(),
            PayrollLine {
                run_id: run.id,
                employee_id: employee.id,
                fixed_wage: wage,
                net_amount: net,
                status: LineStatus::Included,
                ..PayrollLine::default()
            },
        )
        .await
        .unwrap()
    }

    /// A deduction of `fils` of `kind` from the pay of `employee` in `period`.
    pub async fn deduct(
        &self,
        employee: &Employee,
        kind: AdjustmentKind,
        fils: i64,
        period: &str,
    ) -> Adjustment {
        Adjustment::create(
            self.db(),
            Adjustment {
                employee_id: employee.id,
                kind,
                amount: fils,
                period: period.to_owned(),
                ..Adjustment::default()
            },
        )
        .await
        .unwrap()
    }

    /// `user` runs the transition `name` of `run` through its page route,
    /// with `reason`.
    pub async fn transition(
        &self,
        user: &User,
        run: &PayrollRun,
        name: &str,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/hr/payroll/{}/transitions/{name}", run.id),
                &[("reason", reason)],
            )
            .await
    }

    /// `user` comments on `line` (the approver, before `return`).
    pub async fn comment(
        &self,
        user: &User,
        line: &PayrollLine,
        comment: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/hr/payroll/lines/{}/comment", line.id),
                &[("comment", comment)],
            )
            .await
    }

    /// `run`, as stored now.
    pub async fn run(&self, run: &PayrollRun) -> PayrollRun {
        PayrollRun::find_or_404(self.db(), run.id).await.unwrap()
    }

    /// `line`, as stored now.
    pub async fn line(&self, line: &PayrollLine) -> PayrollLine {
        PayrollLine::find_or_404(self.db(), line.id).await.unwrap()
    }

    /// The onboardings of `employee`, oldest first.
    pub async fn onboardings(&self, employee: &Employee) -> Vec<Onboarding> {
        let mut all = Onboarding::of_employee(self.db(), employee.id)
            .await
            .unwrap();
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
        OnboardingStep::find_or_404(self.db(), step.id)
            .await
            .unwrap()
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
        qualification_expired: Arc::default(),
        qualification_expiring: Arc::default(),
        absconded: Arc::default(),
        returned: Arc::default(),
        failed: Arc::default(),
        tasks: TicketDesk::default(),
    };
    let (qualification_expired, qualification_expiring, absconded, returned, failed, tasks) = (
        Recorded(fakes.qualification_expired.clone()),
        Recorded(fakes.qualification_expiring.clone()),
        Recorded(fakes.absconded.clone()),
        Recorded(fakes.returned.clone()),
        Recorded(fakes.failed.clone()),
        fakes.tasks.clone(),
    );
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
        .module(renox::audit::Audit)
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
    permissions::define_role(
        app.db(),
        "owner",
        &[perm::ONBOARDING_VIEW, perm::ONBOARDING_MANAGE],
    )
    .await
    .unwrap();
    permissions::define_role(app.db(), "manager", &[perm::EMPLOYEES_VIEW])
        .await
        .unwrap();
    // The MD approves the non-standard exits (`HRM.ACL.07`) and may also ask
    // for them as HR.
    permissions::define_role(
        app.db(),
        "md",
        &[perm::EMPLOYEES_VIEW, perm::EMPLOYEES_EXIT_APPROVE],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "hr_md",
        &[
            perm::EMPLOYEES_VIEW,
            perm::EMPLOYEES_MANAGE,
            perm::EMPLOYEES_EXIT_APPROVE,
        ],
    )
    .await
    .unwrap();
    // Payroll: HR prepares, the MD approves (`HRM.ACL.04`); one role holds
    // everything, to show that no permission gets past the caps.
    permissions::define_role(
        app.db(),
        "payroll_prep",
        &[
            perm::PAYROLL_VIEW,
            perm::PAYROLL_PREPARE,
            perm::ADJUSTMENTS_MANAGE,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "payroll_appr",
        &[perm::PAYROLL_VIEW, perm::PAYROLL_APPROVE],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "payroll_all",
        &[
            perm::PAYROLL_VIEW,
            perm::PAYROLL_PREPARE,
            perm::PAYROLL_APPROVE,
            perm::ADJUSTMENTS_MANAGE,
            perm::EMPLOYEES_VIEW,
            perm::EMPLOYEES_MANAGE,
            perm::EMPLOYEES_EXIT_APPROVE,
            perm::ONBOARDING_VIEW,
            perm::ONBOARDING_MANAGE,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(app.db(), "accountant", &[perm::PAYROLL_VIEW])
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
        qualification_expired,
        qualification_expiring,
        absconded,
        returned,
        failed,
        tasks,
        clock_days: std::sync::atomic::AtomicI64::new(0),
        cases: std::sync::atomic::AtomicI64::new(0),
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
    TemplateInput::for_role("Supervisor", "supervisor").step(StepInput::task(
        "Supervisor briefing",
        "manager",
        2,
    ))
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
