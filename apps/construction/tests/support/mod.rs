//! Booting the Construction vertical in a `TestApp` with auth, the kernel and
//! the real attendance and inventory modules: the roles of the project
//! manager and of the MD, the kernel events recorded, and fakes for what the
//! vertical reaches through the kernel (the workforce, the tickets, the
//! notifier and the invoicer). A test cannot build an `AuthUser`, so a
//! request is made through the vertical's own routes as the user.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use construction::permissions as perm;
use construction::{Construction, Ipa, IpaStatus, Project, ProjectStatus, ProjectTerminated};
use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_attendance::{WlAttendance, WorkLocation};
use wl_inventory::{Location, PurchaseOrder, PurchaseOrderStatus, WlInventory};
use wl_kernel::block_guard::{
    ActionKind, BlockFuture, BlockGuard, BlockRefusal, ExceptionStep, GuardOutcome,
    provide_block_guard,
};
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
    provide_invoicer,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest, provide_notifier};
use wl_kernel::tickets::{
    OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::workforce::{
    EmployeeId, StaffProfile, Workforce, WorkforceFuture, provide_workforce,
};
use wl_kernel::{BusinessCalendar, PartyId, Subject};

/// A contracting client with attendance and stock on.
const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "construction"

[modules]
enabled = ["parties", "hr", "attendance", "inventory"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
default = true
"#;

/// The client of the projects.
pub const CLIENT_PARTY: PartyId = PartyId(31);

/// A date.
pub fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// The Asia/Dubai wall-clock time `hour:minute` on `date`.
pub fn dubai(date: NaiveDate, hour: u32, minute: u32) -> DateTime {
    BusinessCalendar::default().from_local(date.and_hms_opt(hour, minute, 0).unwrap())
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

/// A ticket opened through the kernel's task creator.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The ticket.
    pub ticket: OpenTicket,
    /// The id the fake gave it.
    pub id: i64,
    /// When it was opened (the travelled clock).
    pub at: DateTime,
    /// Why it was cancelled, when it was.
    pub cancelled: Option<String>,
}

/// The tickets other modules open, and the snags a test puts there.
#[derive(Clone, Default)]
pub struct Tasks(Arc<Mutex<Vec<Opened>>>);

impl Tasks {
    /// Every ticket opened or seeded, oldest first.
    pub fn all(&self) -> Vec<Opened> {
        self.0.lock().unwrap().clone()
    }

    /// The tickets whose source key starts with `prefix`.
    pub fn with_key(&self, prefix: &str) -> Vec<Opened> {
        self.all()
            .into_iter()
            .filter(|opened| {
                opened
                    .ticket
                    .source_key
                    .as_deref()
                    .is_some_and(|key| key.starts_with(prefix))
            })
            .collect()
    }

    /// The tickets of `category` about `subject` that were cancelled.
    pub fn cancelled_of(&self, subject: &Subject, category: &str) -> Vec<Opened> {
        self.all()
            .into_iter()
            .filter(|opened| {
                opened.ticket.subject == *subject
                    && opened.ticket.category_key == category
                    && opened.cancelled.is_some()
            })
            .collect()
    }

    /// An open ticket of `category` about `subject` (a snag, a defect).
    pub fn seed(&self, subject: &Subject, category: &str, title: &str) {
        let mut all = self.0.lock().unwrap();
        let id = i64::try_from(all.len()).unwrap() + 1;
        all.push(Opened {
            ticket: OpenTicket::new(subject.clone(), category, title),
            id,
            at: renox::db::now(),
            cancelled: None,
        });
    }
}

impl TaskCreator for Tasks {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            let id = i64::try_from(all.len()).unwrap() + 1;
            all.push(Opened {
                ticket,
                id,
                at: renox::db::now(),
                cancelled: None,
            });
            Ok(Some(TicketRef {
                id: TicketId(id),
                number: format!("TKT-2027-{id:04}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let all = self.0.lock().unwrap();
            Ok(all
                .iter()
                .find(|opened| {
                    opened.cancelled.is_none()
                        && opened.ticket.source_key.as_deref() == Some(source_key)
                })
                .map(|opened| TicketRef {
                    id: TicketId(opened.id),
                    number: format!("TKT-2027-{:04}", opened.id),
                }))
        })
    }

    fn open_of<'a>(
        &'a self,
        _state: &'a AppState,
        subject: &'a Subject,
        category_key: &'a str,
    ) -> TicketFuture<'a, Vec<TicketRef>> {
        Box::pin(async move {
            let all = self.0.lock().unwrap();
            Ok(all
                .iter()
                .filter(|opened| {
                    opened.cancelled.is_none()
                        && opened.ticket.subject == *subject
                        && opened.ticket.category_key == category_key
                })
                .map(|opened| TicketRef {
                    id: TicketId(opened.id),
                    number: format!("TKT-2027-{:04}", opened.id),
                })
                .collect())
        })
    }

    fn cancel_from_source<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        reason: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            let found = all
                .iter_mut()
                .find(|opened| opened.id == ticket.0 && opened.cancelled.is_none());
            Ok(match found {
                Some(opened) => {
                    opened.cancelled = Some(reason.to_owned());
                    true
                }
                None => false,
            })
        })
    }
}

/// A notifier that records every request and queues nothing.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request so far, oldest first.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The requests of the type `key`.
    pub fn of_type(&self, key: &str) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|request| request.type_key == key)
            .collect()
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The employees of HR, as the workforce of the kernel answers for them.
#[derive(Clone, Default)]
pub struct Staff(Arc<Mutex<BTreeMap<i64, StaffProfile>>>);

impl Staff {
    /// Puts an employee who is at work among the staff.
    pub fn employ(&self, id: i64) -> EmployeeId {
        self.0.lock().unwrap().insert(
            id,
            StaffProfile {
                employee: EmployeeId(id),
                party: PartyId(1000 + id),
                user_id: None,
                role: "labourer".into(),
                scopes: Vec::new(),
                legal_entity_id: None,
                listed: true,
                on_leave: false,
                returns_on: None,
                managers: Vec::new(),
            },
        );
        EmployeeId(id)
    }
}

impl Workforce for Staff {
    fn staff<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&employee.0).cloned()) })
    }
}

/// The invoices of wl-invoicing, as the vertical reads them.
#[derive(Clone, Default)]
pub struct Invoices(Arc<Mutex<BTreeMap<i64, InvoiceSummary>>>);

impl Invoices {
    /// An invoice in `status`; returns its id.
    pub fn put(&self, status: InvoiceStatus) -> i64 {
        let mut all = self.0.lock().unwrap();
        let id = i64::try_from(all.len()).unwrap() + 1;
        all.insert(
            id,
            InvoiceSummary {
                id: InvoiceId(id),
                number: Some(format!("INV-2027-{id:04}")),
                status,
                party: CLIENT_PARTY,
                legal_entity_id: None,
                currency: "AED".into(),
                gross_total: 1_000_000,
                balance: 1_000_000,
                due_date: None,
                disputed: false,
                chasing_paused_until: None,
            },
        );
        id
    }
}

impl Invoicer for Invoices {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        _draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async { Ok(None) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&id.0).cloned()) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _id: InvoiceId,
        _source: Subject,
        _amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// The parties wl-parties blocks, as the block guard answers for them: a
/// blocked party is refused every action but a receipt, and an exception is
/// approved by the compliance role.
#[derive(Clone, Default)]
pub struct Blocked(Arc<Mutex<Vec<PartyId>>>);

impl Blocked {
    /// `party` is blocked.
    pub fn block(&self, party: PartyId) {
        self.0.lock().unwrap().push(party);
    }

    /// `party` is not blocked any more.
    pub fn unblock(&self, party: PartyId) {
        self.0.lock().unwrap().retain(|blocked| *blocked != party);
    }
}

impl BlockGuard for Blocked {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        _viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome> {
        let blocked = self.0.lock().unwrap().contains(&party);
        let outcome = if blocked && kind.is_guarded() {
            GuardOutcome::Blocked(BlockRefusal::neutral(party, kind, "compliance"))
        } else {
            GuardOutcome::Allowed
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn exception_step(&self) -> ExceptionStep {
        ExceptionStep::new("compliance")
    }
}

/// Records the events and provides the fakes.
struct Fakes {
    blocked: Blocked,
    staff: Staff,
    tasks: Tasks,
    notices: Recording,
    invoices: Invoices,
    terminated: Arc<Mutex<Vec<ProjectTerminated>>>,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_workforce(app, self.staff.clone());
        provide_task_creator(app, self.tasks.clone());
        provide_notifier(app, self.notices.clone());
        provide_invoicer(app, self.invoices.clone());
        provide_block_guard(app, self.blocked.clone());
        record(app, &self.terminated);
    }
}

/// The app under test with the fakes and the events.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// The workforce.
    pub staff: Staff,
    /// The tickets opened.
    pub tasks: Tasks,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// The invoices of wl-invoicing.
    pub invoices: Invoices,
    /// The parties wl-parties blocks.
    pub blocked: Blocked,
    /// `ProjectTerminated`.
    pub terminated: Recorded<ProjectTerminated>,
    /// Days the clock was moved forward from the real one.
    clock_days: std::sync::atomic::AtomicI64,
    /// The projects made, for their codes.
    made: std::sync::atomic::AtomicI64,
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

    /// A user holding the role `role` everywhere.
    pub async fn user_with(&self, name: &str, role: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(self.db(), role).await.unwrap();
        user
    }

    /// A project manager.
    pub async fn project_manager(&self, name: &str) -> User {
        self.user_with(name, perm::PROJECT_MANAGER_ROLE).await
    }

    /// A quantity surveyor.
    pub async fn qs(&self, name: &str) -> User {
        self.user_with(name, perm::QS_ROLE).await
    }

    /// The MD.
    pub async fn md(&self, name: &str) -> User {
        self.user_with(name, "md").await
    }

    /// Runs `work` with the clock on `day` (Asia/Dubai) at midday, moved
    /// forward from the real one; days only go forward within a test.
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

    /// A project in `status`, inserted as a fixture, managed by `manager`.
    pub async fn project(&self, status: ProjectStatus, manager: Option<&User>) -> Project {
        let n = self.made.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let mut project =
            Project::new(&format!("TWR-{n:02}"), "Marina Tower fit-out", CLIENT_PARTY)
                .with_budget(50_000_000);
        project.manager_user_id = manager.map(|user| user.id);
        project.status = status;
        let mut tx = self.db().begin().await.unwrap();
        project
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        project
    }

    /// IPA `number` of `project` in `status`, inserted as a fixture.
    pub async fn ipa(
        &self,
        project: &Project,
        number: i64,
        status: IpaStatus,
        claimed: i64,
        certified: Option<i64>,
    ) -> Ipa {
        let mut ipa = Ipa::new(project.id);
        ipa.number = number;
        ipa.status = status;
        ipa.claimed_amount = claimed;
        ipa.certified_amount = certified;
        let mut tx = self.db().begin().await.unwrap();
        ipa.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        ipa
    }

    /// The IPA as it is stored now.
    pub async fn reload_ipa(&self, id: i64) -> Ipa {
        Ipa::find_or_404(self.db(), id).await.unwrap()
    }

    /// The IPA `number` of `project` as it is stored now.
    pub async fn ipa_numbered(&self, project: &Project, number: i64) -> Ipa {
        Ipa::query()
            .where_(Ipa::PROJECT_ID.eq(project.id))
            .where_(Ipa::NUMBER.eq(number))
            .first(self.db())
            .await
            .unwrap()
            .expect("the IPA")
    }

    /// The project as it is stored now.
    pub async fn reload(&self, project: &Project) -> Project {
        Project::find_or_404(self.db(), project.id).await.unwrap()
    }

    /// A store of the project in wl-inventory.
    pub async fn store(&self, project: &Project, code: &str) -> Location {
        let mut location = Location::new(code, code).in_scope(&project.subject());
        location.save(self.db()).await.unwrap();
        location
    }

    /// A work location of the project in wl-attendance.
    pub async fn work_location(&self, project: &Project) -> WorkLocation {
        WorkLocation::create(
            self.db(),
            WorkLocation::named("Site 1").in_scope(&project.subject()),
        )
        .await
        .unwrap()
    }

    /// An issued purchase order to a supplier, delivering to `location`.
    pub async fn order(&self, location: &Location, number: &str) -> PurchaseOrder {
        let mut order = PurchaseOrder::new(PartyId(7), location.id);
        order.status = PurchaseOrderStatus::Issued;
        order.number = Some(number.to_owned());
        let mut tx = self.db().begin().await.unwrap();
        order
            .insert_in_state(&mut tx, PurchaseOrderStatus::Issued, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        order
    }

    /// A POST as `by`.
    pub async fn post(&self, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
        self.app.acting_as(by);
        self.app.htmx().post(path, form).await
    }

    /// A GET as `by`.
    pub async fn get(&self, by: &User, path: &str) -> TestResponse {
        self.app.acting_as(by);
        self.app.get(path).await
    }

    /// `by` asks for the termination of `project`.
    pub async fn ask_to_terminate(
        &self,
        by: &User,
        project: &Project,
        reason: &str,
        effective_on: &str,
    ) -> TestResponse {
        self.post(
            by,
            &format!("/construction/projects/{}/terminate", project.id),
            &[
                ("reason", reason),
                ("notice", "TERMINATION-NOTICE-17.pdf"),
                ("effective_on", effective_on),
            ],
        )
        .await
    }

    /// The approval request waiting about `project`.
    pub async fn pending_request(
        &self,
        project: &Project,
    ) -> wl_kernel::approvals::ApprovalRequest {
        wl_kernel::approvals::ApprovalRequest::pending_about(self.db(), &project.subject())
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("an approval request")
    }

    /// `approver` approves the request waiting about `project`.
    pub async fn approve(&self, approver: &User, project: &Project) {
        let request = self.pending_request(project).await;
        wl_kernel::approvals::approve(self.state(), request.id, approver, "")
            .await
            .unwrap();
    }
}

/// Boots the vertical with the default attendance (no permit check) and
/// inventory modules.
pub async fn boot() -> Booted {
    let staff = Staff::default();
    let tasks = Tasks::default();
    let notices = Recording::default();
    let invoices = Invoices::default();
    let blocked = Blocked::default();
    let terminated = Arc::new(Mutex::new(Vec::new()));
    let fakes = Fakes {
        blocked: blocked.clone(),
        staff: staff.clone(),
        tasks: tasks.clone(),
        notices: notices.clone(),
        invoices: invoices.clone(),
        terminated: terminated.clone(),
    };
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(WlAttendance::new().permit_check(false))
        .module(WlInventory::new())
        .module(Construction)
        .module(fakes);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    // The project manager runs the project and asks for its termination; the
    // MD approves it and cancels a project before work (`CON.ACL.15`).
    permissions::define_role(
        app.db(),
        perm::PROJECT_MANAGER_ROLE,
        &[
            perm::PROJECTS_VIEW,
            perm::PROJECTS_MANAGE,
            perm::PROJECTS_TERMINATE,
            perm::PROJECTS_AWARD,
            perm::IPAS_SUBMIT,
            perm::IPAS_DEDUCTIONS_DECIDE,
        ],
    )
    .await
    .unwrap();
    // The QS records the certificates.
    permissions::define_role(
        app.db(),
        perm::QS_ROLE,
        &[
            perm::PROJECTS_VIEW,
            perm::IPAS_CERTIFY,
            perm::IPAS_DEDUCTIONS_RECORD,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "md",
        &[perm::PROJECTS_VIEW, perm::PROJECTS_CANCEL],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "stock_manager",
        &[wl_inventory::permissions::STOCK_APPROVE],
    )
    .await
    .unwrap();
    Booted {
        app,
        staff,
        tasks,
        notices,
        invoices,
        blocked,
        terminated: Recorded(terminated),
        clock_days: std::sync::atomic::AtomicI64::new(0),
        made: std::sync::atomic::AtomicI64::new(0),
    }
}
