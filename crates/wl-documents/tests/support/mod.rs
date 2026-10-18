//! Booting wl-documents in a `TestApp`, as `crates/wl-parties/tests/support`
//! boots wl-parties: the kinds of the acceptance checks, the guarded
//! transitions' `BlockSubjects` providers (no module guards a transition
//! yet, #45 A1) and the few actions that need a logged-in user.

#![allow(dead_code)]

use std::sync::{Arc, Mutex, PoisonError};

use renox::auth::permissions::Scope;
use renox::auth::{Permissions, permissions};
use renox::axum::extract::Form;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use serde::Deserialize;
use wl_documents::{
    Document, DocumentKind, Documents, KindSeed, NewDocument, SopInput, Sops, WlDocuments,
    permissions as perm,
};
use wl_kernel::documents::{
    BlockCondition, BlockEntry, BlockMode, BlockSubjects, DocumentAccepted, DocumentFuture,
    FutureWork, OverrideMode, PlannedRecord, ReminderSchedule, SopReaderRef,
    register_block_subjects, register_future_work, register_reminder_schedule, today,
};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::tickets::{
    OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::{ClientConfig, Subject};

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
        .override_mode(OverrideMode::Never)
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

    fn scope<'a>(
        &'a self,
        _state: &'a AppState,
        _entry: &'a BlockEntry,
        _record_id: i64,
    ) -> DocumentFuture<'a, Option<Scope>> {
        Box::pin(async { Ok(Some(site())) })
    }
}

/// The site of every deployment (#50): who may approve an override of its
/// block is decided here.
pub fn site() -> Scope {
    Scope::new("sites", 1)
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

// ---- The expiry run (#48): kinds, planned visits, schedule, notifier, roles.

/// The valve "Valve V1" an ECAS certificate belongs to (no scope).
pub const VALVE: i64 = 21;

/// Tower A, a building (scope type `buildings`).
pub fn tower_a() -> Scope {
    Scope::new("buildings", 1)
}

/// Tower B, a building.
pub fn tower_b() -> Scope {
    Scope::new("buildings", 2)
}

/// Plant 1.
pub fn plant1() -> Scope {
    Scope::new("plants", 1)
}

/// Plant 2.
pub fn plant2() -> Scope {
    Scope::new("plants", 2)
}

/// "Valve V1": an item with no scope.
pub fn valve() -> Subject {
    Subject::new("inventory.item", VALVE)
}

/// The Civil Defence approval of the expiry run: it blocks `ticket.resolve`
/// for fire alarm and sprinkler tickets; the `coordinator`s are told.
pub fn civil_defence_approval() -> KindSeed {
    KindSeed::new("civil_defence_approval", "Civil Defence approval")
        .subject_type("party.vendor")
        .expires()
        .responsible_role("coordinator")
        .look_ahead(30)
        .legal_block()
        .blocks_when(
            "ticket.resolve",
            BlockMode::Block,
            BlockCondition::new("category", ["fire_alarm", "sprinkler"]),
        )
}

/// The ECAS certificate: no `blocks`, reminders at 90, 30 and 7 days, the
/// `qc_inspector`s are told.
pub fn ecas_certificate() -> KindSeed {
    KindSeed::new("ecas_certificate", "ECAS certificate")
        .subject_type("inventory.item")
        .expires()
        .reminders(&[90, 30, 7])
        .responsible_role("qc_inspector")
}

/// The work permit of the expiry rule (#49): a legal block, hence
/// compliance-critical, on `deployment.activate` and `deployment.resume`;
/// PRO renews it and HR is told as well.
pub fn permit() -> KindSeed {
    KindSeed::new("work_permit", "Work permit")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("pro")
        .also_notify(&["hr"])
        .legal_block()
        .blocks("deployment.activate", BlockMode::Block)
        .blocks("deployment.resume", BlockMode::Block)
}

/// A kind that blocks `deployment.activate` but is not compliance-critical.
pub fn site_card() -> KindSeed {
    KindSeed::new("site_card", "Site card")
        .subject_type("hr.employee")
        .expires()
        .responsible_role("pro")
        .blocks("deployment.activate", BlockMode::Block)
}

/// The owner role of the `client.toml` of the expiry run.
pub const OWNER: &str = "md";

/// A planned fire-safety visit of a vendor.
#[derive(Debug, Clone)]
pub struct Visit {
    /// The record's id (a `ticketing.ticket`).
    pub id: i64,
    /// The vendor who does it.
    pub vendor: Subject,
    /// The day it is planned for.
    pub date: NaiveDate,
    /// The building it is in.
    pub scope: Option<Scope>,
    /// The ticket category the condition of the `blocks` entry looks at.
    pub category: String,
}

/// What the app under test plans and whom it asks: the work the test
/// `FutureWork` of `ticket.resolve` lists and the scopes the test
/// `ReminderSchedule` of the ECAS certificate names. They live in the app
/// (provided to it), because the provider registries are process-wide and
/// the tests run side by side.
#[derive(Clone, Default)]
pub struct Plan {
    visits: Arc<Mutex<Vec<Visit>>>,
    schedule: Arc<Mutex<Option<Vec<Scope>>>>,
    deployments: Arc<Mutex<Vec<Deployment>>>,
    failing: Arc<Mutex<Vec<i64>>>,
}

/// A deployment of a worker: running now (`date` is when it started) or
/// planned for `date`.
#[derive(Debug, Clone)]
pub struct Deployment {
    /// The record's id.
    pub id: i64,
    /// The worker's id (an `hr.employee`).
    pub worker: i64,
    /// Running now, else planned.
    pub active: bool,
    /// The start date.
    pub date: NaiveDate,
}

impl Plan {
    /// Ravi's deployment `id` is running since `date`.
    pub fn active_deployment(&self, worker: i64, id: i64, date: NaiveDate) {
        self.push_deployment(worker, id, true, date);
    }

    /// The deployment `id` of `worker` is planned for `date`.
    pub fn planned_deployment(&self, worker: i64, id: i64, date: NaiveDate) {
        self.push_deployment(worker, id, false, date);
    }

    fn push_deployment(&self, worker: i64, id: i64, active: bool, date: NaiveDate) {
        self.deployments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Deployment {
                id,
                worker,
                active,
                date,
            });
    }

    /// Makes the `FutureWork` of the deployments fail for `worker`'s
    /// documents; `None` lets it answer again.
    pub fn fail_for(&self, worker: Option<i64>) {
        let mut failing = self.failing.lock().unwrap_or_else(PoisonError::into_inner);
        failing.clear();
        failing.extend(worker);
    }

    /// Plans a fire-alarm visit of SafeFire in `scope` on `date`; returns
    /// the visit's id (1, 2, ... in the order planned).
    pub fn visit(&self, scope: Scope, date: NaiveDate) -> i64 {
        let mut visits = self.visits.lock().unwrap_or_else(PoisonError::into_inner);
        let id = visits.len() as i64 + 1;
        visits.push(Visit {
            id,
            vendor: safefire(),
            date,
            scope: Some(scope),
            category: "fire_alarm".to_owned(),
        });
        id
    }

    /// Moves visit `id` to `date`.
    pub fn move_visit(&self, id: i64, date: NaiveDate) {
        let mut visits = self.visits.lock().unwrap_or_else(PoisonError::into_inner);
        visits
            .iter_mut()
            .find(|visit| visit.id == id)
            .unwrap_or_else(|| panic!("no visit {id}"))
            .date = date;
    }

    /// The planned visits.
    pub fn visits(&self) -> Vec<Visit> {
        self.visits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Has the schedule of the ECAS certificate name `scopes`; without a
    /// call it gives no answer (every scope).
    pub fn schedule(&self, scopes: Vec<Scope>) {
        *self.schedule.lock().unwrap_or_else(PoisonError::into_inner) = Some(scopes);
    }
}

/// The test `FutureWork` of `ticket.resolve`: the [`Plan`]'s visits of the
/// vendor, dated on or after `from`, that match the entry's condition.
struct PlannedVisits;

impl FutureWork for PlannedVisits {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            let visits = state
                .provided::<Plan>()
                .map(|plan| plan.visits())
                .unwrap_or_default();
            Ok(visits
                .into_iter()
                .filter(|visit| {
                    visit.vendor == *subject
                        && visit.date >= from
                        && entry.applies_to(Some(&visit.category))
                })
                .map(|visit| PlannedRecord {
                    record: Subject::new("ticketing.ticket", visit.id),
                    date: visit.date,
                    label: format!("Fire-safety visit {}", visit.id),
                    scope: visit.scope,
                })
                .collect())
        })
    }
}

/// The test `FutureWork` of `deployment.activate` and `deployment.resume`:
/// the [`Plan`]'s running deployments of the worker (`open_for`) and those
/// planned from a date (`planned_from`). It fails for the workers the plan
/// names.
struct PlannedDeployments;

impl PlannedDeployments {
    fn records(
        state: &AppState,
        subject: &Subject,
        active: bool,
        from: Option<NaiveDate>,
    ) -> Result<Vec<PlannedRecord>> {
        let Some(plan) = state.provided::<Plan>() else {
            return Ok(Vec::new());
        };
        let failing = plan.failing.lock().unwrap_or_else(PoisonError::into_inner);
        if failing.contains(&subject.subject_id) {
            return Err(Error::Status(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the deployments are not available".into(),
            ));
        }
        drop(failing);
        let deployments = plan
            .deployments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(deployments
            .into_iter()
            .filter(|d| d.worker == subject.subject_id && d.active == active)
            .filter(|d| from.is_none_or(|from| d.date >= from))
            .map(|d| PlannedRecord {
                record: Subject::new("ops.deployment", d.id),
                date: d.date,
                label: format!("Deployment {}", d.id),
                scope: None,
            })
            .collect())
    }
}

impl FutureWork for PlannedDeployments {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move { Self::records(state, subject, false, Some(from)) })
    }

    fn open_for<'a>(
        &'a self,
        state: &'a AppState,
        _entry: &'a BlockEntry,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move { Self::records(state, subject, true, None) })
    }
}

/// The test `ReminderSchedule` of the ECAS certificate: what the [`Plan`]
/// names, else no answer.
struct PlanSchedule;

impl ReminderSchedule for PlanSchedule {
    fn scopes<'a>(
        &'a self,
        state: &'a AppState,
        _subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<Vec<Scope>>> {
        Box::pin(async move {
            Ok(state.provided::<Plan>().and_then(|plan| {
                plan.schedule
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            }))
        })
    }
}

/// A [`Notifier`] that records each request instead of sending it.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request so far, oldest first.
    pub fn sent(&self) -> Vec<NotifyRequest> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The requests addressed to the user `id`.
    pub fn to_user(&self, id: i64) -> Vec<NotifyRequest> {
        self.sent()
            .into_iter()
            .filter(|request| {
                request
                    .recipients
                    .iter()
                    .any(|r| matches!(r, NotifyRecipient::User { id: to } if *to == id))
            })
            .collect()
    }

    /// Forgets what was recorded.
    pub fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            let mut sent = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            sent.push(request);
            Ok(vec![MessageId(sent.len() as i64)])
        })
    }
}

/// A [`TaskCreator`] that records the tasks opened and completed instead of
/// opening tickets. A task is found by its source key until it is
/// completed.
#[derive(Clone, Default)]
pub struct Tasks(Arc<Mutex<TaskLog>>);

#[derive(Default)]
struct TaskLog {
    opened: Vec<OpenTicket>,
    completed: Vec<(TicketId, String)>,
}

impl Tasks {
    /// Every task opened so far, oldest first; task `n` has the ticket id
    /// `n` (from 1).
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.log().opened.clone()
    }

    /// Every `complete` call that closed a task, with its note.
    pub fn completed(&self) -> Vec<(TicketId, String)> {
        self.log().completed.clone()
    }

    fn log(&self) -> std::sync::MutexGuard<'_, TaskLog> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TaskCreator for Tasks {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut log = self.log();
            log.opened.push(ticket);
            let n = log.opened.len();
            Ok(Some(TicketRef {
                id: TicketId(n as i64),
                number: format!("TKT-2026-{n:04}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let log = self.log();
            Ok(log
                .opened
                .iter()
                .position(|t| t.source_key.as_deref() == Some(source_key))
                .filter(|n| !log.completed.iter().any(|(id, _)| id.0 == *n as i64 + 1))
                .map(|n| TicketRef {
                    id: TicketId(n as i64 + 1),
                    number: format!("TKT-2026-{:04}", n + 1),
                }))
        })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            let mut log = self.log();
            let known = ticket.0 >= 1 && ticket.0 as usize <= log.opened.len();
            let done = log.completed.iter().any(|(id, _)| *id == ticket);
            if !known || done {
                return Ok(false);
            }
            log.completed.push((ticket, note.to_owned()));
            Ok(true)
        })
    }
}

/// What the app under test boots with.
#[derive(Clone)]
pub struct Options {
    /// wl-notify is on (a recording notifier), else the kernel's.
    pub notify: bool,
    /// wl-ticketing is on (the recording [`Tasks`]), else the kernel's
    /// `NoTickets`.
    pub tickets: bool,
    /// `[roles] owner` of the `client.toml`; without it no config is
    /// provided and the owner is the company admin.
    pub owner: Option<&'static str>,
    /// Events are recorded, not delivered; off, the listeners run and
    /// [`Booted::accepted_events`] lists the `DocumentAccepted` seen.
    pub fake_events: bool,
    /// The kernel module is on (approvals, their tables and policies), and
    /// the default approval policies are installed.
    pub kernel: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            notify: true,
            tickets: true,
            owner: None,
            fake_events: true,
            kernel: false,
        }
    }
}

/// The app under test.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// What the app's task creator recorded.
    pub tasks: Tasks,
    /// The `DocumentAccepted` seen by the live listener.
    accepted_seen: Arc<Mutex<Vec<DocumentAccepted>>>,
    /// What the app's notifier recorded.
    pub notices: Recording,
    /// The planned work and the reminder schedule of the app.
    pub plan: Plan,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
}

impl Booted {
    /// The `DocumentAccepted` events the live listener saw, oldest first.
    pub fn accepted_events(&self) -> Vec<DocumentAccepted> {
        self.accepted_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

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

    /// Defines the roles `names` (no grants).
    pub async fn roles(&self, names: &[&str]) {
        for name in names {
            permissions::define_role(self.db(), name, &[])
                .await
                .unwrap();
        }
    }

    /// A user named `name` holding `role` in `scope` (globally for `None`).
    /// The role must be defined ([`Booted::roles`]).
    pub async fn staff(&self, name: &str, role: &str, scope: Option<&Scope>) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        match scope {
            Some(scope) => user.assign_role_in(self.db(), role, scope).await,
            None => user.assign_role(self.db(), role).await,
        }
        .unwrap();
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
struct Harness {
    notices: Recording,
    plan: Plan,
    tasks: Tasks,
    options: Options,
}

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

/// A form for the SOP routes.
#[derive(Deserialize)]
struct SopForm {
    #[serde(default)]
    code: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    owner_role: String,
    #[serde(default)]
    entity_id: i64,
    /// `all`, `role` or `role@type:id`, separated by `;`.
    #[serde(default)]
    readers: String,
}

/// Parses the `readers` of a [`SopForm`].
fn sop_reader_refs(text: &str) -> Vec<SopReaderRef> {
    text.split(';')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let part = part.trim();
            if part == "all" {
                return SopReaderRef {
                    all_staff: true,
                    role: String::new(),
                    scope_type: String::new(),
                    scope_id: String::new(),
                };
            }
            let (role, scope) = part.split_once('@').unwrap_or((part, ""));
            let (scope_type, scope_id) = scope.split_once(':').unwrap_or((scope, ""));
            SopReaderRef {
                all_staff: false,
                role: role.to_owned(),
                scope_type: scope_type.to_owned(),
                scope_id: scope_id.to_owned(),
            }
        })
        .collect()
}

fn sop_input(form: &SopForm) -> SopInput {
    let input = SopInput::new(&form.code, &form.title, &form.owner_role);
    if form.entity_id != 0 {
        input.legal_entity("legal_entity", form.entity_id)
    } else {
        input
    }
}

async fn sop_create(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<SopForm>,
) -> Result<String> {
    Ok(Sops::create(&state, &user, sop_input(&form))
        .await?
        .id
        .to_string())
}

async fn sop_update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<SopForm>,
) -> Result<String> {
    Ok(Sops::update(&state, &user, id, sop_input(&form))
        .await?
        .code)
}

async fn sop_readers(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<SopForm>,
) -> Result<String> {
    let readers = sop_reader_refs(&form.readers);
    Ok(Sops::set_readers(&state, &user, id, &readers)
        .await?
        .len()
        .to_string())
}

async fn generic_accept(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let document = Document::find_or_404(&state.db, id).await?;
    match Documents::accept(&state, &user, &document).await {
        Ok(document) => Ok(document.status.as_str().to_owned()),
        Err(error) => Err(error.into_error()),
    }
}

async fn generic_reject(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<FileForm>,
) -> Result<String> {
    let document = Document::find_or_404(&state.db, id).await?;
    let reason = form.reason.unwrap_or_default();
    match Documents::reject(&state, &user, &document, &reason).await {
        Ok(document) => Ok(document.status.as_str().to_owned()),
        Err(error) => Err(error.into_error()),
    }
}

async fn sop_upload(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<FileForm>,
) -> Result<String> {
    match Sops::upload_version(&state, &user, id, form.upload(), None).await? {
        Ok(document) => Ok(document.id.to_string()),
        Err(refused) => Err(refused.into_error()),
    }
}

async fn sop_accept(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let document = Document::find_or_404(&state.db, id).await?;
    match Sops::accept_version(&state, &user, &document).await {
        Ok(document) => Ok(document.status.as_str().to_owned()),
        Err(error) => Err(error.into_error()),
    }
}

async fn sop_reject(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<FileForm>,
) -> Result<String> {
    let document = Document::find_or_404(&state.db, id).await?;
    let reason = form.reason.unwrap_or_default();
    match Sops::reject_version(&state, &user, &document, &reason).await {
        Ok(document) => Ok(document.status.as_str().to_owned()),
        Err(error) => Err(error.into_error()),
    }
}

async fn sop_withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<FileForm>,
) -> Result<String> {
    match Sops::withdraw(&state, &user, id, form.reason.as_deref()).await {
        Ok(sop) => Ok(sop.id.to_string()),
        Err(error) => Err(error.into_error()),
    }
}

impl Module for Harness {
    fn name(&self) -> &'static str {
        "documents-test-harness"
    }

    fn register(&self, app: &mut Registry) {
        if self.options.notify {
            provide_notifier(app, self.notices.clone());
        }
        if self.options.tickets {
            provide_task_creator(app, self.tasks.clone());
        }
        if let Some(owner) = self.options.owner {
            let text = format!(
                "[client]\nname = \"Acme Staffing LLC\"\napp = \"staffing\"\n\n[roles]\nowner = \"{owner}\"\n"
            );
            let config = ClientConfig::parse(&text).expect("a valid client.toml");
            app.provide(Arc::new(config));
        }
        app.provide(self.plan.clone());
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/test/documents/record", record)
            .post("/test/documents/{id}/resubmit", resubmit)
            .post("/test/documents/{id}/archive", archive)
            .post("/test/sops", sop_create)
            .post("/test/sops/{id}", sop_update)
            .post("/test/sops/{id}/readers", sop_readers)
            .post("/test/documents/{id}/accept", generic_accept)
            .post("/test/documents/{id}/reject", generic_reject)
            .post("/test/sops/{id}/versions", sop_upload)
            .post("/test/sops/{id}/withdraw", sop_withdraw)
            .post("/test/sop-versions/{id}/accept", sop_accept)
            .post("/test/sop-versions/{id}/reject", sop_reject)
    }
}

/// wl-documents booted in an app with auth, with the providers of the
/// guarded transitions registered and no kind installed yet.
pub async fn boot() -> Booted {
    boot_with(true).await
}

/// [`boot`], with the recording notifier only when `notify` (wl-notify on);
/// without it the kernel's `NoNotifier` is the app's notifier.
pub async fn boot_with(notify: bool) -> Booted {
    boot_options(Options {
        notify,
        ..Options::default()
    })
    .await
}

/// [`boot`], as `options` say.
pub async fn boot_options(options: Options) -> Booted {
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
    register_block_subjects("deployment.resume", Deployments);

    register_future_work("ticket.resolve", PlannedVisits);
    register_future_work("deployment.activate", PlannedDeployments);
    register_future_work("deployment.resume", PlannedDeployments);
    register_reminder_schedule("ecas_certificate", PlanSchedule);

    let notices = Recording::default();
    let plan = Plan::default();
    let tasks = Tasks::default();
    let accepted_seen: Arc<Mutex<Vec<DocumentAccepted>>> = Arc::default();
    let seen = accepted_seen.clone();
    let fake_events = options.fake_events;
    let options_kernel = options.kernel;
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlDocuments)
        .module(Harness {
            notices: notices.clone(),
            plan: plan.clone(),
            tasks: tasks.clone(),
            options,
        })
        .listen(move |event: DocumentAccepted, _state: AppState| {
            let seen = seen.clone();
            async move {
                seen.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event);
                Ok(())
            }
        });
    let kernel = options_kernel;
    let app = if kernel {
        let text = "[client]\nname = \"Acme Staffing LLC\"\napp = \"staffing\"\n";
        let config = ClientConfig::parse(text).expect("a valid client.toml");
        wl_kernel::apply(app, config).expect("the kernel applies")
    } else {
        app
    };
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    if kernel {
        wl_kernel::approvals::install(app.db())
            .await
            .expect("the approval policies install");
    }
    if fake_events {
        // Events are recorded for `assert_emitted`; no listener runs.
        app.fake_events();
    }
    Booted {
        app,
        tasks,
        accepted_seen,
        notices,
        plan,
        _views: views,
    }
}
