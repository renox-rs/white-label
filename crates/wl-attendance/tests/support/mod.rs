//! Booting wl-attendance in a `TestApp` with auth, as `crates/wl-hr/tests/support`
//! boots wl-hr: the roles attendance uses, a notifier that records what it is
//! asked for, the kernel events recorded, and fakes for the workforce of HR,
//! its qualifications and the document guard of documents.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_attendance::permissions as perm;
use wl_attendance::{
    AttendanceMethod, AttendanceRecord, AttendanceStatus, WlAttendance, WorkLocation,
};
use wl_kernel::attendance::{
    AttendanceFlagged, AttendanceRecorded, CostRateResolver, LabourFuture, TimesheetApproved,
    provide_cost_rates,
};
use wl_kernel::documents::{
    BlockEntry, CheckOutcome, DocumentFuture, DocumentGuard, FlagFailure, RecordFlag, Validity,
    provide_guard,
};
use wl_kernel::invoicing::{
    CostPoster, DisputeOutcome, DisputeState, InvoiceDraft, InvoiceFuture, InvoiceId,
    InvoiceSummary, Invoicer, PostCost, SupplierBillFuture, provide_cost_poster, provide_invoicer,
};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::qualifications::{
    QualificationFuture, QualificationStatus, Qualifications, provide_qualifications,
};
use wl_kernel::tickets::{
    OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::timesheets::{
    ApproverResolver, TimesheetReconciliationNeeded, provide_approver_resolver,
};
use wl_kernel::workforce::{
    EmployeeId, StaffProfile, Workforce, WorkforceFuture, provide_workforce,
};
use wl_kernel::{BusinessCalendar, PartyId, Subject};

/// A client with the attendance module (and the modules it needs) on.
const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "property"

[modules]
enabled = ["parties", "hr", "attendance"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
default = true
"#;

/// A date in November 2026.
pub fn nov(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
}

/// The Asia/Dubai wall-clock time `hour:minute` on `date`.
pub fn dubai(date: NaiveDate, hour: u32, minute: u32) -> DateTime {
    BusinessCalendar::default().from_local(date.and_hms_opt(hour, minute, 0).unwrap())
}

/// `hour:minute` on `day` November 2026.
pub fn at(day: u32, hour: u32, minute: u32) -> DateTime {
    dubai(nov(day), hour, minute)
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

    /// The flag notifications of the flag type `flag`.
    pub fn flagged(&self, flag: &str) -> Vec<NotifyRequest> {
        self.of_type("attendance.record_flagged")
            .into_iter()
            .filter(|r| r.data["flag"] == flag)
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

/// The user ids a request goes to.
pub fn users_of(request: &NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// The employees of HR, as the workforce of the kernel answers for them.
#[derive(Clone, Default)]
pub struct Staff {
    profiles: Arc<Mutex<BTreeMap<i64, StaffProfile>>>,
    accounts: Arc<Mutex<BTreeMap<i64, i64>>>,
}

impl Staff {
    /// Puts `profile` among the staff.
    pub fn set(&self, profile: StaffProfile) {
        self.profiles
            .lock()
            .unwrap()
            .insert(profile.employee.0, profile);
    }

    /// Changes the profile of `employee`.
    pub fn change(&self, employee: EmployeeId, change: impl FnOnce(&mut StaffProfile)) {
        change(self.profiles.lock().unwrap().get_mut(&employee.0).unwrap());
    }

    /// The account `user` is `employee`'s.
    pub fn account(&self, user: i64, employee: EmployeeId) {
        self.accounts.lock().unwrap().insert(user, employee.0);
    }
}

impl Workforce for Staff {
    fn staff<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        Box::pin(async move { Ok(self.profiles.lock().unwrap().get(&employee.0).cloned()) })
    }

    fn employee_of_user<'a>(
        &'a self,
        _db: &'a Db,
        user_id: i64,
    ) -> WorkforceFuture<'a, Option<EmployeeId>> {
        Box::pin(async move {
            Ok(self
                .accounts
                .lock()
                .unwrap()
                .get(&user_id)
                .copied()
                .map(EmployeeId))
        })
    }
}

/// The labour cost posted through the kernel's cost poster.
#[derive(Clone, Default)]
pub struct Posted(Arc<Mutex<Vec<PostCost>>>);

impl Posted {
    /// Every posting so far.
    pub fn all(&self) -> Vec<PostCost> {
        self.0.lock().unwrap().clone()
    }
}

impl CostPoster for Posted {
    fn post<'a>(&'a self, _state: &'a AppState, request: PostCost) -> SupplierBillFuture<'a, ()> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            // A key that was used changes nothing.
            if !all.iter().any(|p| p.key == request.key) {
                all.push(request);
            }
            Ok(())
        })
    }
}

/// Construction posts cost from hours at the trade rate (AED 25 an hour
/// normal, AED 31.25 overtime); manpower and every other context do not.
#[derive(Clone, Copy, Default)]
pub struct TradeRates;

impl CostRateResolver for TradeRates {
    fn posts_cost<'a>(
        &'a self,
        _state: &'a AppState,
        context: Option<&'a Subject>,
    ) -> LabourFuture<'a, bool> {
        Box::pin(
            async move { Ok(context.is_some_and(|c| c.subject_type == "construction.project")) },
        )
    }

    fn hourly_rate<'a>(
        &'a self,
        _state: &'a AppState,
        _employee: EmployeeId,
        _context: Option<&'a Subject>,
        category: &'a str,
        _date: NaiveDate,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async move {
            Ok(match category {
                "normal" => Some(2_500),
                "overtime" => Some(3_125),
                _ => None,
            })
        })
    }
}

/// The invoice disputes of wl-invoicing, as the kernel's `Invoicer` answers
/// for them: a dispute is open until it is settled.
#[derive(Clone, Default)]
pub struct Disputes(Arc<Mutex<BTreeMap<i64, DisputeState>>>);

impl Disputes {
    /// Dispute `id` of invoice `invoice` is open.
    pub fn open(&self, id: i64, invoice: i64) {
        self.0.lock().unwrap().insert(
            id,
            DisputeState {
                invoice: InvoiceId(invoice),
                outcome: None,
            },
        );
    }

    /// Dispute `id` was settled with `outcome`.
    pub fn settle(&self, id: i64, outcome: DisputeOutcome) {
        if let Some(found) = self.0.lock().unwrap().get_mut(&id) {
            found.outcome = Some(outcome);
        }
    }
}

impl Invoicer for Disputes {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        _draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async { Ok(None) })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: InvoiceId,
    ) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async { Ok(None) })
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

    fn dispute_state<'a>(
        &'a self,
        _db: &'a Db,
        dispute: i64,
    ) -> InvoiceFuture<'a, Option<DisputeState>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&dispute).copied()) })
    }
}

/// The tasks opened through the kernel's task creator.
#[derive(Clone, Default)]
pub struct Tasks {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
    closed: Arc<Mutex<Vec<i64>>>,
}

impl Tasks {
    /// Every task opened so far.
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }

    /// The tasks opened with `source_key` that were closed.
    pub fn closed_with_key(&self, source_key: &str) -> usize {
        let closed = self.closed.lock().unwrap();
        self.opened
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(n, t)| {
                t.source_key.as_deref() == Some(source_key)
                    && closed.contains(&(i64::try_from(*n).unwrap() + 1))
            })
            .count()
    }
}

impl TaskCreator for Tasks {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut all = self.opened.lock().unwrap();
            all.push(ticket);
            let n = all.len();
            Ok(Some(TicketRef {
                id: TicketId(i64::try_from(n).unwrap()),
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
            let all = self.opened.lock().unwrap();
            let closed = self.closed.lock().unwrap();
            Ok(all
                .iter()
                .position(|t| t.source_key.as_deref() == Some(source_key))
                .filter(|n| !closed.contains(&(i64::try_from(*n).unwrap() + 1)))
                .map(|n| TicketRef {
                    id: TicketId(i64::try_from(n).unwrap() + 1),
                    number: format!("TKT-2026-{:04}", n + 1),
                }))
        })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        _note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            let mut closed = self.closed.lock().unwrap();
            if closed.contains(&ticket.0) {
                return Ok(false);
            }
            closed.push(ticket.0);
            Ok(true)
        })
    }
}

/// The contract of a deployment, as the app's `ApproverResolver` answers for
/// it: the account manager, the client's escalation contact and approvers,
/// and the days after which silence is approval (none when the contract has
/// no such clause).
#[derive(Clone, Default)]
pub struct Approvers(Arc<Mutex<ApproverSetup>>);

#[derive(Default)]
struct ApproverSetup {
    account_manager: Option<i64>,
    escalation: Vec<NotifyRecipient>,
    clients: Vec<NotifyRecipient>,
    deemed_days: Option<i64>,
}

impl Approvers {
    /// User `user` is the account manager.
    pub fn account_manager(&self, user: i64) {
        self.0.lock().unwrap().account_manager = Some(user);
    }

    /// User `user` is the client's escalation contact.
    pub fn escalation_contact(&self, user: i64) {
        self.0
            .lock()
            .unwrap()
            .escalation
            .push(NotifyRecipient::User { id: user });
    }

    /// User `user` is one of the client's approvers.
    pub fn client_approver(&self, user: i64) {
        self.0
            .lock()
            .unwrap()
            .clients
            .push(NotifyRecipient::User { id: user });
    }

    /// The contract deems a timesheet approved after `days` days of silence
    /// (`None` removes the clause).
    pub fn deemed_after(&self, days: Option<i64>) {
        self.0.lock().unwrap().deemed_days = days;
    }
}

impl ApproverResolver for Approvers {
    fn account_manager<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().account_manager) })
    }

    fn escalation_contacts<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().escalation.clone()) })
    }

    fn client_approvers<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().clients.clone()) })
    }

    fn deemed_approval_days<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().deemed_days) })
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

/// One qualification of an employee, as the HR module would hold it.
#[derive(Clone)]
struct QualEntry {
    employee: i64,
    kind: String,
    roles: Vec<String>,
    scopes: Vec<Subject>,
    valid_to: Option<NaiveDate>,
}

/// The qualifications of the employees: `qualification_valid` answers yes
/// through the expiry date, and for a role-bound kind only to a caller that
/// names a role and scope the qualification covers.
#[derive(Clone, Default)]
pub struct Quals(Arc<Mutex<Vec<QualEntry>>>);

impl Quals {
    /// `employee` holds a qualification of `kind` valid through `valid_to`,
    /// for any role and scope.
    pub fn grant(&self, employee: EmployeeId, kind: &str, valid_to: Option<NaiveDate>) {
        self.bound(employee, kind, &[], &[], valid_to);
    }

    /// The same, covering only `roles` and `scopes`.
    pub fn bound(
        &self,
        employee: EmployeeId,
        kind: &str,
        roles: &[&str],
        scopes: &[Subject],
        valid_to: Option<NaiveDate>,
    ) {
        self.0.lock().unwrap().push(QualEntry {
            employee: employee.0,
            kind: kind.to_owned(),
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
            scopes: scopes.to_vec(),
            valid_to,
        });
    }

    fn answer(
        &self,
        employee: EmployeeId,
        kind: &str,
        on_date: NaiveDate,
        asked: Option<(&str, &Subject)>,
    ) -> bool {
        let all: Vec<QualEntry> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|q| q.employee == employee.0 && q.kind == kind)
            .cloned()
            .collect();
        all.iter().any(|q| {
            let bound = !q.roles.is_empty() || !q.scopes.is_empty();
            let covers = match asked {
                Some((role, scope)) => {
                    (q.roles.is_empty() || q.roles.iter().any(|r| r == role))
                        && (q.scopes.is_empty() || q.scopes.contains(scope))
                }
                None => !bound,
            };
            covers && q.valid_to.is_none_or(|day| day >= on_date)
        })
    }
}

impl Qualifications for Quals {
    fn qualification_valid<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
        kind: &'a str,
        on_date: NaiveDate,
        role: Option<&'a str>,
        scope: Option<&'a Subject>,
    ) -> QualificationFuture<'a, QualificationStatus> {
        let asked = role.zip(scope);
        let valid = self.answer(employee, kind, on_date, asked);
        Box::pin(async move {
            Ok(QualificationStatus {
                valid,
                ..QualificationStatus::default()
            })
        })
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

/// Records the events and provides the fakes.
struct Fakes {
    notices: Recording,
    staff: Staff,
    tasks: Tasks,
    docs: DocDesk,
    quals: Quals,
    posted: Posted,
    disputes: Disputes,
    approvers: Approvers,
    recorded: Arc<Mutex<Vec<AttendanceRecorded>>>,
    flagged: Arc<Mutex<Vec<AttendanceFlagged>>>,
    approved: Arc<Mutex<Vec<TimesheetApproved>>>,
    reconciled: Arc<Mutex<Vec<TimesheetReconciliationNeeded>>>,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.notices.clone());
        provide_task_creator(app, self.tasks.clone());
        provide_workforce(app, self.staff.clone());
        provide_guard(app, self.docs.clone());
        provide_qualifications(app, self.quals.clone());
        provide_cost_poster(app, self.posted.clone());
        provide_cost_rates(app, TradeRates);
        provide_invoicer(app, self.disputes.clone());
        provide_approver_resolver(app, self.approvers.clone());
        record(app, &self.recorded);
        record(app, &self.flagged);
        record(app, &self.approved);
        record(app, &self.reconciled);
    }
}

/// The app under test with the fakes and the events.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// The workforce.
    pub staff: Staff,
    /// The tasks opened.
    pub tasks: Tasks,
    /// The accepted documents.
    pub docs: DocDesk,
    /// The qualifications.
    pub quals: Quals,
    /// `AttendanceRecorded`.
    pub recorded: Recorded<AttendanceRecorded>,
    /// `AttendanceFlagged`.
    pub flagged: Recorded<AttendanceFlagged>,
    /// The labour cost posted.
    pub posted: Posted,
    /// `TimesheetApproved`.
    pub approved: Recorded<TimesheetApproved>,
    /// `TimesheetReconciliationNeeded`.
    pub reconciled: Recorded<TimesheetReconciliationNeeded>,
    /// The invoice disputes.
    pub disputes: Disputes,
    /// The contract of the deployments.
    pub approvers: Approvers,
    /// Days the clock was moved forward from the real one.
    clock_days: std::sync::atomic::AtomicI64,
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

    /// A user holding `role` everywhere.
    pub async fn user_with(&self, name: &str, role: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(self.db(), role).await.unwrap();
        user
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

    /// An employee who is at work (listed, not on leave), managed by
    /// `managers`, with a work permit and a residence visa that do not
    /// expire.
    pub fn employee(&self, id: i64, managers: &[i64]) -> EmployeeId {
        self.staff.set(StaffProfile {
            employee: EmployeeId(id),
            party: PartyId(1000 + id),
            user_id: None,
            role: "labourer".into(),
            scopes: Vec::new(),
            legal_entity_id: None,
            listed: true,
            on_leave: false,
            returns_on: None,
            managers: managers.to_vec(),
        });
        // Papers in order, so that only the tests of `HRM.RUL.14` have a reason
        // to flag.
        self.papers(EmployeeId(id), None);
        EmployeeId(id)
    }

    /// The work permit and the residence visa of `employee`, valid until
    /// `expires` (never when none).
    pub fn papers(&self, employee: EmployeeId, expires: Option<NaiveDate>) {
        self.docs.accepted_until(
            &employee.subject(),
            "work_permit",
            100 + employee.0,
            expires,
        );
        self.docs.accepted_until(
            &employee.subject(),
            "residence_visa",
            200 + employee.0,
            expires,
        );
    }

    /// A work location called `name`.
    pub async fn location(&self, name: &str) -> WorkLocation {
        WorkLocation::create(self.db(), WorkLocation::named(name))
            .await
            .unwrap()
    }

    /// A work location made from `location`.
    pub async fn location_of(&self, location: WorkLocation) -> WorkLocation {
        WorkLocation::create(self.db(), location).await.unwrap()
    }

    /// A supervisor-marked record in `status`, inserted as a fixture (the
    /// machine would not take it there).
    pub async fn fixture(
        &self,
        employee: EmployeeId,
        location: &WorkLocation,
        check_in: DateTime,
        check_out: Option<DateTime>,
        status: AttendanceStatus,
    ) -> AttendanceRecord {
        self.fixture_by(employee, location, check_in, check_out, status, None)
            .await
    }

    /// [`fixture`](Self::fixture) marked by the supervisor `by`.
    pub async fn fixture_by(
        &self,
        employee: EmployeeId,
        location: &WorkLocation,
        check_in: DateTime,
        check_out: Option<DateTime>,
        status: AttendanceStatus,
        by: Option<i64>,
    ) -> AttendanceRecord {
        let mut record = AttendanceRecord {
            employee_id: employee.0,
            location_id: location.id,
            date: wl_kernel::documents::dubai_date(check_in),
            check_in,
            check_out,
            method: AttendanceMethod::Supervisor,
            recorded_by: by,
            recorded_at: check_in,
            status,
            ..AttendanceRecord::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        record
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        record
    }

    /// A timesheet in `status`, inserted as a fixture (the machine would not
    /// take it there).
    pub async fn sheet_in(
        &self,
        sheet: wl_attendance::Timesheet,
        status: wl_attendance::TimesheetStatus,
    ) -> wl_attendance::Timesheet {
        let mut sheet = sheet;
        sheet.status = status;
        let mut tx = self.db().begin().await.unwrap();
        sheet
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        sheet
    }

    /// `sheet`, as stored now.
    pub async fn reload_sheet(&self, sheet: &wl_attendance::Timesheet) -> wl_attendance::Timesheet {
        wl_attendance::Timesheet::find_or_404(self.db(), sheet.id)
            .await
            .unwrap()
    }

    /// The transitions of a timesheet, oldest first, as `(transition, actor,
    /// reason)`.
    pub async fn sheet_history(
        &self,
        sheet: &wl_attendance::Timesheet,
    ) -> Vec<(String, Option<i64>, Option<String>)> {
        renox::state_machine::history(self.db(), wl_attendance::Timesheet::TABLE, sheet.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.transition, row.actor_id, row.reason))
            .collect()
    }

    /// `record`, as stored now.
    pub async fn reload(&self, record: &AttendanceRecord) -> AttendanceRecord {
        AttendanceRecord::find_or_404(self.db(), record.id)
            .await
            .unwrap()
    }

    /// Every record of `employee`, oldest first.
    pub async fn records_of(&self, employee: EmployeeId) -> Vec<AttendanceRecord> {
        AttendanceRecord::query()
            .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
            .order_by(AttendanceRecord::ID)
            .get(self.db())
            .await
            .unwrap()
    }

    /// The flag types of a record, in the order they were added.
    pub async fn flags_of(&self, record: &AttendanceRecord) -> Vec<String> {
        wl_attendance::AttendanceFlag::of_record(self.db(), record.id)
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.flag_type.as_str().to_owned())
            .collect()
    }

    /// The transitions of a record, oldest first, as `(transition, actor,
    /// reason, flag type of the metadata)`.
    pub async fn history_of(
        &self,
        record: &AttendanceRecord,
    ) -> Vec<(String, Option<i64>, Option<String>, Option<String>)> {
        renox::state_machine::history(self.db(), AttendanceRecord::TABLE, record.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| {
                let flag = row.metadata["flag"].as_str().map(str::to_owned);
                (row.transition, row.actor_id, row.reason, flag)
            })
            .collect()
    }
}

/// `WlAttendance` booted with auth, the roles attendance uses and the kernel.
pub async fn boot() -> Booted {
    boot_with(WlAttendance::new(), "").await
}

/// [`boot`] with `module` and `extra` appended to the client's `client.toml`
/// (for the `[calendar]`).
pub async fn boot_with(module: WlAttendance, extra: &str) -> Booted {
    let fakes = Fakes {
        notices: Recording::default(),
        staff: Staff::default(),
        tasks: Tasks::default(),
        docs: DocDesk::default(),
        quals: Quals::default(),
        posted: Posted::default(),
        disputes: Disputes::default(),
        approvers: Approvers::default(),
        recorded: Arc::default(),
        flagged: Arc::default(),
        approved: Arc::default(),
        reconciled: Arc::default(),
    };
    let (notices, staff, tasks) = (
        fakes.notices.clone(),
        fakes.staff.clone(),
        fakes.tasks.clone(),
    );
    let (docs, quals) = (fakes.docs.clone(), fakes.quals.clone());
    let (posted, approved) = (fakes.posted.clone(), Recorded(fakes.approved.clone()));
    let (disputes, approvers) = (fakes.disputes.clone(), fakes.approvers.clone());
    let reconciled = Recorded(fakes.reconciled.clone());
    let (recorded, flagged) = (
        Recorded(fakes.recorded.clone()),
        Recorded(fakes.flagged.clone()),
    );
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(module)
        .module(fakes);
    let config =
        wl_kernel::ClientConfig::parse(&format!("{CLIENT}\n{extra}")).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    // The pages extend the app's layout; a package test has no app, so it
    // brings the smallest one.
    let views = tempfile::tempdir().expect("a temporary views directory");
    let layout = views.path().join("layouts/app.html");
    std::fs::create_dir_all(layout.parent().unwrap()).unwrap();
    std::fs::write(
        &layout,
        "<!doctype html>\n<html><body><rx-outlet name=\"content\"/></body></html>\n",
    )
    .unwrap();
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    // The supervisor marks and corrects; the reviewer accepts flagged
    // records; HR is told of a missing permit.
    permissions::define_role(
        app.db(),
        "supervisor",
        &[perm::VIEW, perm::MARK, perm::CORRECT],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "reviewer",
        &[perm::VIEW, perm::REVIEW, perm::CORRECT],
    )
    .await
    .unwrap();
    // A site lead who marks and also reviews: the separation of duties
    // (`HRM.ACL.09`) still keeps them from accepting what they marked.
    permissions::define_role(
        app.db(),
        "lead",
        &[perm::VIEW, perm::MARK, perm::CORRECT, perm::REVIEW],
    )
    .await
    .unwrap();
    // The timekeeper submits timesheets and reopens approved ones; the
    // adjuster opens adjustments on locked ones (`HRM.ACL.08`).
    permissions::define_role(
        app.db(),
        "timekeeper",
        &[perm::VIEW, perm::TIMESHEETS_SUBMIT, perm::TIMESHEETS_REOPEN],
    )
    .await
    .unwrap();
    permissions::define_role(app.db(), "adjuster", &[perm::VIEW, perm::TIMESHEETS_ADJUST])
        .await
        .unwrap();
    // The approver approves internal timesheets and rejects them; staff with
    // `approve_on_behalf` approve for the client; the client's approver holds
    // a role of the timesheet's `approver_role` and no permission.
    permissions::define_role(
        app.db(),
        "approver",
        &[
            perm::VIEW,
            perm::TIMESHEETS_SUBMIT,
            perm::TIMESHEETS_APPROVE,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "account_manager",
        &[perm::VIEW, perm::TIMESHEETS_APPROVE_ON_BEHALF],
    )
    .await
    .unwrap();
    permissions::define_role(app.db(), "client_approver", &[])
        .await
        .unwrap();
    permissions::define_role(app.db(), "worker", &[perm::SELF_CHECK_IN])
        .await
        .unwrap();
    permissions::define_role(app.db(), "hr", &[perm::VIEW])
        .await
        .unwrap();
    permissions::define_role(app.db(), "hse", &[perm::VIEW])
        .await
        .unwrap();
    Booted {
        app,
        notices,
        staff,
        tasks,
        docs,
        quals,
        recorded,
        flagged,
        posted,
        approved,
        reconciled,
        disputes,
        approvers,
        clock_days: std::sync::atomic::AtomicI64::new(0),
        _views: views,
    }
}

/// A subject for a project.
pub fn project(id: i64) -> Subject {
    Subject::new("construction.project", id)
}

/// The cost code of the examples, as a form sends it.
pub const COST_CODE: &str = "construction.cost_code:3";

/// A locked timesheet of `employee` for `context` with 8 h normal on 2
/// November against the cost code, used by payroll run 7 (`paid_by`) and
/// approved by `kind`.
pub async fn locked_eight_hours(
    t: &Booted,
    employee: EmployeeId,
    context: &Subject,
    kind: wl_attendance::timesheet::ApproverKind,
) -> wl_attendance::Timesheet {
    let mut new = wl_attendance::Timesheet::regular(employee, nov(2), nov(8)).in_context(context);
    new.paid_by = Some("hr.payroll_run:7".into());
    new.approver_kind = kind;
    if kind == wl_attendance::timesheet::ApproverKind::External {
        new.approver_role = Some("client_approver".into());
    }
    let sheet = t
        .sheet_in(new, wl_attendance::TimesheetStatus::Locked)
        .await;
    wl_attendance::TimesheetEntry::create(
        t.db(),
        wl_attendance::TimesheetEntry {
            timesheet_id: sheet.id,
            employee_id: employee.0,
            date: nov(2),
            cost_object_type: Some("construction.cost_code".into()),
            cost_object_id: Some(3),
            normal_minutes: 480,
            billable: true,
            ..wl_attendance::TimesheetEntry::default()
        },
    )
    .await
    .unwrap();
    sheet
}

/// The entries of a form that takes 8 h of normal hours off 2 November.
pub fn minus_eight_hours() -> String {
    format!(
        r#"[{{"date": "2026-11-02", "category": "normal", "minutes": -480, "cost_object": "{COST_CODE}"}}]"#
    )
}

/// `user` posts `form` to `/timesheets/{id}/{action}`.
pub async fn post_to(
    t: &Booted,
    user: &User,
    sheet: &wl_attendance::Timesheet,
    action: &str,
    form: &[(&str, &str)],
) -> TestResponse {
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(&format!("/timesheets/{}/{action}", sheet.id), form)
        .await
}

/// An 8 h accepted record (07:00 to 15:00) of `employee` on each of `days` of
/// November, at a site of its own.
pub async fn accepted_days(t: &Booted, employee: EmployeeId, days: &[u32]) {
    let site = t.location(&format!("Site {}", employee.0)).await;
    for day in days {
        t.fixture(
            employee,
            &site,
            at(*day, 7, 0),
            Some(at(*day, 15, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    }
}

/// `minutes` of `category` on `day` of November, as an entry of `sheet`
/// against the deployment.
pub async fn put_hours(
    t: &Booted,
    sheet: &wl_attendance::Timesheet,
    day: u32,
    category: &str,
    minutes: i64,
) {
    let mut entry = wl_attendance::TimesheetEntry {
        timesheet_id: sheet.id,
        employee_id: sheet.owner_employee_id,
        date: nov(day),
        cost_object_type: Some("manpower.deployment".into()),
        cost_object_id: Some(9),
        billable: true,
        ..wl_attendance::TimesheetEntry::default()
    };
    match category {
        "normal" => entry.normal_minutes = minutes,
        "overtime" => entry.overtime_minutes = minutes,
        other => panic!("category {other}"),
    }
    wl_attendance::TimesheetEntry::create(t.db(), entry)
        .await
        .unwrap();
}

/// A November timesheet of `employee` for the deployment whose client
/// approves it (role `client_approver`), in `status`.
pub async fn client_timesheet(
    t: &Booted,
    employee: EmployeeId,
    status: wl_attendance::TimesheetStatus,
) -> wl_attendance::Timesheet {
    let mut new = wl_attendance::Timesheet::regular(employee, nov(1), nov(30))
        .in_context(&Subject::new("manpower.deployment", 9));
    new.approver_kind = wl_attendance::timesheet::ApproverKind::External;
    new.approver_role = Some("client_approver".into());
    t.sheet_in(new, status).await
}

/// Payroll run 7 pays the provisional hours, as `wl-hr` would at
/// its `submit`: it asks the hours of November (cut-off on the 25th) and tells
/// the service it used them.
pub async fn pay_provisionally(t: &Booted) {
    use wl_kernel::timesheets::{PayrollPeriod, timesheets};
    let service = timesheets(t.state());
    let period = PayrollPeriod::new(nov(1), nov(30), nov(25));
    let hours = service.hours_for_payroll(t.state(), &period).await.unwrap();
    service
        .record_payroll_use(t.state(), &Subject::new("hr.payroll_run", 7), &hours)
        .await
        .unwrap();
}
