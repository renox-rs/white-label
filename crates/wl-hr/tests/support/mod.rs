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
    Adjustment, AdjustmentKind, Employee, EmploymentType, FinalSettlement, LineStatus, NewEmployee,
    Onboarding, OnboardingStep, PayAccount, PayAccountStatus, PayrollLine, PayrollRun,
    PayrollRunStatus, SalaryStructure, SettlementPayment, SettlementStatus, WlHr, employees,
    template::{self, StepInput, TemplateInput},
};
use wl_kernel::attendance::{AttendanceHistory, LabourFuture, provide_attendance_history};
use wl_kernel::documents::{
    BlockEntry, CheckOutcome, DocumentFuture, DocumentGuard, FlagFailure, RecordFlag,
    SopAcknowledgements, SopFuture, SopStaff, Validity, provide_guard,
    provide_sop_acknowledgements,
};
use wl_kernel::invoicing::{CostPoster, PostCost, SupplierBillFuture, provide_cost_poster};
use wl_kernel::notify::{
    ContactKind, MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest,
    PartyContactPoint, PartyContacts, provide_notifier,
};
use wl_kernel::parties::{
    PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary, provide_directory,
};
use wl_kernel::tickets::{
    ChecklistRunId, Checklists, OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef,
    provide_checklists, provide_task_creator,
};
use wl_kernel::timesheets::{
    HoursLine, PayrollHours, PayrollPeriod, Timesheets, provide_timesheets,
};
use wl_kernel::workforce::{
    AbsenceCaseOpened, EmployeeAbsconded, EmployeeActivated, EmployeeAssignmentChanged,
    EmployeeDeceased, EmployeeExited, EmployeeId, EmployeeNoticeWithdrawn,
    EmployeeOnboardingCancelled, EmployeeReinstated, LeaveApproved, LeaveCancelled,
    OnboardingCompleted, OnboardingStarted, PayAccountChanged, PayrollLinesFailed, PayrollRunPaid,
    PayrollRunReturned, QualificationExpired, QualificationExpiring, SettlementDisputed,
    SettlementPaid, VisaCaseRejected,
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

/// The recipients of one stop, with its reason.
type Stop = (Vec<NotifyRecipient>, String);

/// Records what the notifier is asked for.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>, Arc<Mutex<Vec<Stop>>>);

impl Recording {
    /// The recipients messages were stopped for, with the reason, in order.
    pub fn stopped(&self) -> Vec<Stop> {
        self.1.lock().unwrap().clone()
    }

    /// The requests not cancelled by a stop: those with no recipient that was
    /// stopped afterwards.
    pub fn waiting_of_type(&self, key: &str) -> Vec<NotifyRequest> {
        let stopped: Vec<NotifyRecipient> = self
            .stopped()
            .into_iter()
            .flat_map(|(recipients, _)| recipients)
            .collect();
        self.of_type(key)
            .into_iter()
            .filter(|r| !r.recipients.iter().any(|to| stopped.contains(to)))
            .collect()
    }

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

    fn stop_messages_to<'a>(
        &'a self,
        _state: &'a AppState,
        recipients: Vec<NotifyRecipient>,
        reason: &'a str,
    ) -> NotifyFuture<'a, usize> {
        Box::pin(async move {
            self.1.lock().unwrap().push((recipients, reason.to_owned()));
            Ok(0)
        })
    }
}

/// The party directory: the names and the contact points of the employees'
/// parties, as wl-parties would answer.
#[derive(Clone, Default)]
pub struct PartyDesk {
    names: Arc<Mutex<BTreeMap<i64, String>>>,
    contacts: Arc<Mutex<BTreeMap<i64, Vec<PartyContactPoint>>>>,
}

impl PartyDesk {
    /// Party `party` is called `name`.
    pub fn name(&self, party: i64, name: &str) {
        self.names.lock().unwrap().insert(party, name.to_owned());
    }

    /// Party `party` has the mobile number `number` on record (replacing the
    /// one before).
    pub fn mobile(&self, party: i64, number: &str) {
        self.point(party, ContactKind::Mobile, number);
    }

    /// Party `party` has the e-mail address `address` on record.
    pub fn email(&self, party: i64, address: &str) {
        self.point(party, ContactKind::Email, address);
    }

    /// Party `party` has no mobile number on record.
    pub fn no_mobile(&self, party: i64) {
        if let Some(points) = self.contacts.lock().unwrap().get_mut(&party) {
            points.retain(|p| p.kind != ContactKind::Mobile);
        }
    }

    fn point(&self, party: i64, kind: ContactKind, address: &str) {
        let mut all = self.contacts.lock().unwrap();
        let points = all.entry(party).or_default();
        points.retain(|p| p.kind != kind);
        let id = i64::try_from(points.len()).unwrap_or(0) + party * 10 + 1;
        points.push(PartyContactPoint {
            id,
            kind,
            address: address.to_owned(),
            primary: true,
            verified: true,
            valid: true,
        });
    }
}

impl PartyDirectory for PartyDesk {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok(self
                .names
                .lock()
                .unwrap()
                .get(&party.0)
                .map(|name| PartySummary {
                    id: party,
                    code: format!("P-{:05}", party.0),
                    kind: PartyKind::Person,
                    display_name: name.clone(),
                    status: PartyStatus::Active,
                    display_block_reason: None,
                }))
        })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async move { Ok(self.names.lock().unwrap().get(&party.0).cloned()) })
    }

    fn status<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async { Ok(None) })
    }

    fn contacts<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        Box::pin(async move {
            Ok(self
                .contacts
                .lock()
                .unwrap()
                .get(&party.0)
                .map(|points| PartyContacts {
                    party,
                    language: "en".into(),
                    preferred_channel: None,
                    points: points.clone(),
                }))
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

/// The days employees attended, as wl-attendance would answer
/// `AttendanceHistory`.
#[derive(Clone, Default)]
pub struct AttendanceDesk(Arc<Mutex<Vec<(i64, NaiveDate)>>>);

impl AttendanceDesk {
    /// `employee` attended on `day`.
    pub fn attended(&self, employee: &Employee, day: NaiveDate) {
        self.0.lock().unwrap().push((employee.id, day));
    }
}

impl AttendanceHistory for AttendanceDesk {
    fn attended_days<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> LabourFuture<'a, Vec<NaiveDate>> {
        Box::pin(async move {
            let mut days: Vec<NaiveDate> = self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(id, day)| *id == employee.0 && *day >= from && *day <= to)
                .map(|(_, day)| *day)
                .collect();
            days.sort_unstable();
            days.dedup();
            Ok(days)
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

/// The real modules that take the place of a fake in a booted app.
#[derive(Clone, Copy, Default)]
struct Real {
    /// wl-payments records the payments HR makes.
    payments: bool,
    /// wl-attendance provides the `Timesheets`.
    attendance: bool,
    /// wl-ticketing provides the `TaskCreator`.
    ticketing: bool,
    /// wl-notify provides the `Notifier`.
    notify: bool,
}

/// Records the events and provides the fakes.
struct Fakes {
    real: Real,
    notices: Recording,
    checklists: ChecklistDesk,
    sops: SopDesk,
    docs: DocDesk,
    timesheets: TimesheetDesk,
    costs: CostDesk,
    parties: PartyDesk,
    pay_changed: Arc<Mutex<Vec<PayAccountChanged>>>,
    started: Arc<Mutex<Vec<OnboardingStarted>>>,
    completed: Arc<Mutex<Vec<OnboardingCompleted>>>,
    activated: Arc<Mutex<Vec<EmployeeActivated>>>,
    changed: Arc<Mutex<Vec<EmployeeAssignmentChanged>>>,
    qualification_expired: Arc<Mutex<Vec<QualificationExpired>>>,
    qualification_expiring: Arc<Mutex<Vec<QualificationExpiring>>>,
    absconded: Arc<Mutex<Vec<EmployeeAbsconded>>>,
    onboarding_cancelled: Arc<Mutex<Vec<EmployeeOnboardingCancelled>>>,
    notice_withdrawn: Arc<Mutex<Vec<EmployeeNoticeWithdrawn>>>,
    reinstated: Arc<Mutex<Vec<EmployeeReinstated>>>,
    exited: Arc<Mutex<Vec<EmployeeExited>>>,
    deceased: Arc<Mutex<Vec<EmployeeDeceased>>>,
    visa_rejected: Arc<Mutex<Vec<VisaCaseRejected>>>,
    attendance: AttendanceDesk,
    returned: Arc<Mutex<Vec<PayrollRunReturned>>>,
    failed: Arc<Mutex<Vec<PayrollLinesFailed>>>,
    paid: Arc<Mutex<Vec<PayrollRunPaid>>>,
    settlement_paid: Arc<Mutex<Vec<SettlementPaid>>>,
    settlement_disputed: Arc<Mutex<Vec<SettlementDisputed>>>,
    tasks: TicketDesk,
    calendar: Option<Arc<dyn wl_kernel::payroll_calendar::PayrollCalendar>>,
    leave_approved: Arc<Mutex<Vec<LeaveApproved>>>,
    leave_cancelled: Arc<Mutex<Vec<LeaveCancelled>>>,
}

/// A `TaskCreator` that records the tickets it opens. A ticket opened with a
/// source key that is still open is the same ticket (`find_open`).
#[derive(Clone, Default)]
pub struct TicketDesk {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
    completed: Arc<Mutex<Vec<(i64, String)>>>,
    resolved: Arc<Mutex<Vec<i64>>>,
}

impl TicketDesk {
    /// The tickets opened, oldest first.
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }

    /// `(ticket, note)` of each ticket closed on behalf of its source.
    pub fn completed(&self) -> Vec<(i64, String)> {
        self.completed.lock().unwrap().clone()
    }

    /// Whether ticket number `id` (1 for the first opened) was resolved from
    /// its source.
    pub fn is_resolved(&self, id: i64) -> bool {
        self.resolved.lock().unwrap().contains(&id)
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

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            self.completed
                .lock()
                .unwrap()
                .push((ticket.0, note.to_owned()));
            Ok(true)
        })
    }

    fn resolve_from_source<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        _note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            let mut resolved = self.resolved.lock().unwrap();
            if resolved.contains(&ticket.0) {
                return Ok(false);
            }
            resolved.push(ticket.0);
            Ok(true)
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let opened = self.opened.lock().unwrap();
            let resolved = self.resolved.lock().unwrap();
            Ok(opened
                .iter()
                .position(|t| t.source_key.as_deref() == Some(source_key))
                .filter(|n| !resolved.contains(&(i64::try_from(*n).unwrap() + 1)))
                .map(|n| TicketRef {
                    id: TicketId(n as i64 + 1),
                    number: format!("TKT-{}", n + 1),
                }))
        })
    }
}

/// What `record_payroll_use` was told: the run and the hours it used.
type PayrollUse = (Subject, Vec<PayrollHours>);

/// The timesheets of wl-attendance: the hours payroll is given at `calculate`
/// and a log of what `submit` told it was used.
#[derive(Clone, Default)]
pub struct TimesheetDesk {
    hours: Arc<Mutex<Vec<PayrollHours>>>,
    used: Arc<Mutex<Vec<PayrollUse>>>,
}

impl TimesheetDesk {
    /// `hours` are given to payroll from now on.
    pub fn give(&self, hours: PayrollHours) {
        self.hours.lock().unwrap().push(hours);
    }

    /// `(run, hours)` of each `record_payroll_use`.
    pub fn used(&self) -> Vec<PayrollUse> {
        self.used.lock().unwrap().clone()
    }
}

impl Timesheets for TimesheetDesk {
    fn hours_for_payroll<'a>(
        &'a self,
        _state: &'a AppState,
        _period: &'a PayrollPeriod,
    ) -> LabourFuture<'a, Vec<PayrollHours>> {
        Box::pin(async move { Ok(self.hours.lock().unwrap().clone()) })
    }

    fn record_payroll_use<'a>(
        &'a self,
        _state: &'a AppState,
        run: &'a Subject,
        used: &'a [PayrollHours],
    ) -> LabourFuture<'a, ()> {
        Box::pin(async move {
            self.used.lock().unwrap().push((run.clone(), used.to_vec()));
            Ok(())
        })
    }
}

/// The cost poster: the cost posted to cost objects, once per key.
#[derive(Clone, Default)]
pub struct CostDesk {
    posted: Arc<Mutex<Vec<PostCost>>>,
}

impl CostDesk {
    /// What was posted, oldest first.
    pub fn posted(&self) -> Vec<PostCost> {
        self.posted.lock().unwrap().clone()
    }
}

impl CostPoster for CostDesk {
    fn post<'a>(&'a self, _state: &'a AppState, request: PostCost) -> SupplierBillFuture<'a, ()> {
        Box::pin(async move {
            let mut posted = self.posted.lock().unwrap();
            if !posted.iter().any(|p| p.key == request.key) {
                posted.push(request);
            }
            Ok(())
        })
    }
}

/// The hours of timesheet `timesheet` of `employee` in November: `(day,
/// category, minutes)` rows.
pub fn hours_of(
    employee: &Employee,
    timesheet: i64,
    provisional: bool,
    rows: &[(u32, &str, i64)],
) -> PayrollHours {
    PayrollHours {
        timesheet: Subject::new("attendance.timesheet", timesheet),
        owner: employee.employee_id(),
        period_from: nov(1),
        period_to: nov(30),
        context: None,
        approver_kind: "internal".into(),
        provisional,
        lines: rows
            .iter()
            .map(|(day, category, minutes)| HoursLine {
                date: nov(*day),
                category: (*category).to_owned(),
                minutes: *minutes,
            })
            .collect(),
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
        if !self.real.notify {
            provide_notifier(app, self.notices.clone());
        }
        provide_checklists(app, self.checklists.clone());
        provide_sop_acknowledgements(app, self.sops.clone());
        provide_guard(app, self.docs.clone());
        if !self.real.attendance {
            provide_timesheets(app, self.timesheets.clone());
        }
        provide_cost_poster(app, self.costs.clone());
        provide_directory(app, self.parties.clone());
        record(app, &self.pay_changed);
        record(app, &self.started);
        record(app, &self.completed);
        record(app, &self.activated);
        record(app, &self.changed);
        record(app, &self.qualification_expired);
        record(app, &self.qualification_expiring);
        record(app, &self.absconded);
        record(app, &self.leave_approved);
        record(app, &self.leave_cancelled);
        record(app, &self.onboarding_cancelled);
        record(app, &self.notice_withdrawn);
        record(app, &self.reinstated);
        record(app, &self.exited);
        record(app, &self.deceased);
        record(app, &self.visa_rejected);
        record(app, &self.returned);
        record(app, &self.failed);
        record(app, &self.paid);
        record(app, &self.settlement_paid);
        record(app, &self.settlement_disputed);
        if !self.real.attendance {
            provide_attendance_history(app, self.attendance.clone());
        }
        if !self.real.ticketing {
            provide_task_creator(app, self.tasks.clone());
        }
        if let Some(calendar) = &self.calendar {
            app.provide(Arc::clone(calendar));
        }
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
    /// The hours of the timesheets.
    pub timesheets: TimesheetDesk,
    /// The cost posted to cost objects.
    pub costs: CostDesk,
    /// The names and contact points of the parties.
    pub parties: PartyDesk,
    /// `PayAccountChanged`.
    pub pay_changed: Recorded<PayAccountChanged>,
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
    /// `EmployeeOnboardingCancelled`.
    pub onboarding_cancelled: Recorded<EmployeeOnboardingCancelled>,
    /// `EmployeeNoticeWithdrawn`.
    pub notice_withdrawn: Recorded<EmployeeNoticeWithdrawn>,
    /// `EmployeeReinstated`.
    pub reinstated: Recorded<EmployeeReinstated>,
    /// `EmployeeExited` events.
    pub exited: Recorded<EmployeeExited>,
    /// `EmployeeDeceased` events.
    pub deceased: Recorded<EmployeeDeceased>,
    /// `VisaCaseRejected`.
    pub visa_rejected: Recorded<VisaCaseRejected>,
    /// The days employees attended.
    pub attendance: AttendanceDesk,
    /// `PayrollRunReturned`.
    pub returned: Recorded<PayrollRunReturned>,
    /// `PayrollLinesFailed`.
    pub failed: Recorded<PayrollLinesFailed>,
    /// `PayrollRunPaid`.
    pub paid: Recorded<PayrollRunPaid>,
    /// `SettlementPaid`.
    pub settlement_paid: Recorded<SettlementPaid>,
    /// `SettlementDisputed`.
    pub settlement_disputed: Recorded<SettlementDisputed>,
    /// The tasks (tickets) opened for staff.
    pub tasks: TicketDesk,
    /// `LeaveApproved`.
    pub leave_approved: Recorded<LeaveApproved>,
    /// `LeaveCancelled`.
    pub leave_cancelled: Recorded<LeaveCancelled>,
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

    /// `employee` is recorded present on `day` (07:00 to 15:00) through the
    /// real wl-attendance (`boot_with_attendance`): the record is saved by the
    /// service, so `AttendanceRecorded` is emitted, with the clock on `day`.
    pub async fn check_in(&self, employee: &Employee, day: NaiveDate) {
        let site = match wl_attendance::WorkLocation::query()
            .first(self.db())
            .await
            .unwrap()
        {
            Some(site) => site,
            None => wl_attendance::WorkLocation::create(
                self.db(),
                wl_attendance::WorkLocation::named("Site 1"),
            )
            .await
            .unwrap(),
        };
        let at = |hour| {
            wl_kernel::BusinessCalendar::default().from_local(day.and_hms_opt(hour, 0, 0).unwrap())
        };
        self.on(day, async {
            wl_attendance::service::record(
                self.state(),
                wl_attendance::NewRecord::import(
                    employee.employee_id(),
                    site.id,
                    at(7),
                    Some(at(15)),
                ),
            )
            .await
            .unwrap();
        })
        .await;
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

    /// The tickets of the real wl-ticketing, oldest first, each with the key
    /// of its category.
    pub async fn tickets(&self) -> Vec<(wl_ticketing::Ticket, String)> {
        let all = wl_ticketing::Ticket::query()
            .order_by(wl_ticketing::Ticket::ID)
            .get(self.db())
            .await
            .unwrap();
        let mut found = Vec::new();
        for ticket in all {
            let category = wl_ticketing::Category::find(self.db(), ticket.category_id)
                .await
                .unwrap()
                .expect("the category of a ticket");
            found.push((ticket, category.key));
        }
        found
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

    /// A `paid` line of `run` for `employee` with a fixed wage and a net, in
    /// fils, whose cost is posted to `cost_object`.
    pub async fn paid_line(
        &self,
        run: &PayrollRun,
        employee: &Employee,
        wage: i64,
        net: i64,
        cost_object: &Subject,
    ) -> PayrollLine {
        PayrollLine::create(
            self.db(),
            PayrollLine {
                run_id: run.id,
                employee_id: employee.id,
                fixed_wage: wage,
                net_amount: net,
                status: LineStatus::Paid,
                cost_object_type: Some(cost_object.subject_type.clone()),
                cost_object_id: Some(cost_object.subject_id),
                ..PayrollLine::default()
            },
        )
        .await
        .unwrap()
    }

    /// A salary structure for `employee` from 1 Jan 2026: a basic wage of
    /// `wage` fils and `hourly` fils an hour.
    pub async fn salary(&self, employee: &Employee, wage: i64, hourly: i64) -> SalaryStructure {
        SalaryStructure::create(
            self.db(),
            SalaryStructure {
                employee_id: employee.id,
                effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                basic: wage,
                hourly_rate: Some(hourly),
                ..SalaryStructure::default()
            },
        )
        .await
        .unwrap()
    }

    /// The line of `employee` in `run`, as stored now.
    pub async fn line_of(&self, run: &PayrollRun, employee: &Employee) -> PayrollLine {
        PayrollLine::query()
            .where_(PayrollLine::RUN_ID.eq(run.id))
            .where_(PayrollLine::EMPLOYEE_ID.eq(employee.id))
            .first(self.db())
            .await
            .unwrap()
            .expect("the run has a line for the employee")
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

    /// A run of `period` that was approved and submitted as SIF version 1, with
    /// a line of `net` fils (and the same fixed wage) for each employee.
    pub async fn submitted_run(
        &self,
        period: &str,
        employees: &[&Employee],
        net: i64,
    ) -> (PayrollRun, Vec<PayrollLine>) {
        let run = self.payroll_run(period, PayrollRunStatus::Approved).await;
        let mut lines = Vec::new();
        for employee in employees {
            lines.push(self.payroll_line(&run, employee, net, net).await);
        }
        wl_hr::payroll::submit(self.state(), run.id).await.unwrap();
        (self.run(&run).await, lines)
    }

    /// The default legal entity's id.
    pub async fn entity_id(&self) -> i64 {
        wl_kernel::entities::LegalEntity::default_entity(self.db())
            .await
            .unwrap()
            .id
    }

    /// `user` records the bank's rejection of the file of `run`.
    pub async fn reject(
        &self,
        user: &User,
        run: &PayrollRun,
        reason: &str,
        notice: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/hr/payroll/{}/rejection", run.id),
                &[("reason", reason), ("notice", notice)],
            )
            .await
    }

    /// `user` resubmits the rejected file of `run` with `note`.
    pub async fn resubmit(
        &self,
        user: &User,
        run: &PayrollRun,
        note: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/hr/payroll/{}/resubmit", run.id),
                &[("note", note)],
            )
            .await
    }

    /// `user` records the bank's result of `run`: `result_file` and the failed
    /// lines as `<line id>=<reason>` rows.
    pub async fn partial(
        &self,
        user: &User,
        run: &PayrollRun,
        result_file: &str,
        failed: &[(&PayrollLine, &str)],
    ) -> renox::testing::TestResponse {
        let rows: Vec<String> = failed
            .iter()
            .map(|(line, reason)| format!("{}={reason}", line.id))
            .collect();
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/hr/payroll/{}/partial", run.id),
                &[("result_file", result_file), ("failed", &rows.join("\n"))],
            )
            .await
    }

    /// `user` confirms payment of `run` with `confirmation`.
    pub async fn confirm(
        &self,
        user: &User,
        run: &PayrollRun,
        confirmation: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/hr/payroll/{}/confirm", run.id),
                &[("confirmation", confirmation)],
            )
            .await
    }

    /// `user` runs an action of a failed `line`: `hold` or `pay-other`.
    pub async fn line_action(
        &self,
        user: &User,
        line: &PayrollLine,
        action: &str,
        fields: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(&format!("/hr/payroll/lines/{}/{action}", line.id), fields)
            .await
    }

    /// A run of `period` for `employees` with a net of `net` fils each,
    /// calculated by `prep`, approved by `appr` and sent (SIF version 1).
    pub async fn sent_run(
        &self,
        prep: &User,
        appr: &User,
        period: &str,
        employees: &[Employee],
        net: i64,
    ) -> (PayrollRun, Vec<PayrollLine>) {
        let run = self.payroll_run(period, PayrollRunStatus::Draft).await;
        for employee in employees {
            self.payroll_line(&run, employee, net, net).await;
        }
        self.transition(prep, &run, "calculate", "")
            .await
            .assert_status(303);
        self.transition(appr, &run, "approve", "")
            .await
            .assert_status(303);
        wl_hr::payroll::submit(self.state(), run.id).await.unwrap();
        let run = self.run(&run).await;
        let lines = run.lines(self.db()).await.unwrap();
        (run, lines)
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

    /// A pay account of `employee` in `status` with the IBAN `iban`, inserted
    /// as a fixture (the machine would not take it there).
    pub async fn pay_account_in(
        &self,
        employee: &Employee,
        status: PayAccountStatus,
        iban: &str,
    ) -> PayAccount {
        let mut account = PayAccount {
            employee_id: employee.id,
            channel: "bank_account".into(),
            routing_code: Some("123456789".into()),
            account_number: iban.to_owned(),
            last4: iban[iban.len() - 4..].to_owned(),
            holder_name: "Ravi Kumar".into(),
            cooling_days: 5,
            status,
            ..PayAccount::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        account
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        account
    }

    /// `user` asks for a new pay account of `employee` through the page route,
    /// with the form `fields`.
    pub async fn request_account(
        &self,
        user: &User,
        employee: &Employee,
        fields: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/hr/employees/{}/pay-accounts", employee.id),
                fields,
            )
            .await
    }

    /// `user` runs the transition `name` of `account` through its page route.
    pub async fn pay_transition(
        &self,
        user: &User,
        account: &PayAccount,
        name: &str,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/hr/pay-accounts/{}/transitions/{name}", account.id),
                &[("reason", reason)],
            )
            .await
    }

    /// `account`, as stored now.
    pub async fn account(&self, account: &PayAccount) -> PayAccount {
        PayAccount::find_or_404(self.db(), account.id)
            .await
            .unwrap()
    }

    /// The pay accounts of `employee`, oldest first.
    pub async fn accounts_of(&self, employee: &Employee) -> Vec<PayAccount> {
        PayAccount::of_employee(self.db(), employee.id)
            .await
            .unwrap()
    }

    /// A final settlement of `employee` in `status` with the figures `set`
    /// gives it, inserted as a fixture (the machine would not take it there).
    pub async fn settlement_in(
        &self,
        employee: &Employee,
        status: SettlementStatus,
        set: impl FnOnce(&mut FinalSettlement),
    ) -> FinalSettlement {
        let mut settlement = FinalSettlement {
            employee_id: employee.id,
            reason: "resignation".into(),
            payee: "employee".into(),
            statement_version: 1,
            status,
            ..FinalSettlement::default()
        };
        set(&mut settlement);
        let mut tx = self.db().begin().await.unwrap();
        settlement
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        settlement
    }

    /// A visa case of `employee` of `case_type` in `status` with the fields
    /// `set` gives it, inserted as a fixture (the machine would not take it
    /// there).
    pub async fn visa_in(
        &self,
        employee: &Employee,
        case_type: &str,
        status: wl_hr::VisaCaseStatus,
        set: impl FnOnce(&mut wl_hr::VisaCase),
    ) -> wl_hr::VisaCase {
        let mut case = wl_hr::VisaCase {
            employee_id: employee.id,
            case_type: case_type.to_owned(),
            variant: "standard".into(),
            status,
            ..wl_hr::VisaCase::default()
        };
        set(&mut case);
        let mut tx = self.db().begin().await.unwrap();
        case.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        case
    }

    /// Step `step_type` of `case` done on 2 Nov.
    pub async fn step_done(&self, case: &wl_hr::VisaCase, step_type: &str) {
        wl_hr::VisaCaseStep::create(
            self.db(),
            wl_hr::VisaCaseStep {
                case_id: case.id,
                step_type: step_type.to_owned(),
                done_on: Some(nov(2)),
                ..wl_hr::VisaCaseStep::default()
            },
        )
        .await
        .unwrap();
    }

    /// `case`, as stored now.
    pub async fn case(&self, case: &wl_hr::VisaCase) -> wl_hr::VisaCase {
        wl_hr::VisaCase::find_or_404(self.db(), case.id)
            .await
            .unwrap()
    }

    /// `settlement`, as stored now.
    pub async fn settlement(&self, settlement: &FinalSettlement) -> FinalSettlement {
        FinalSettlement::find_or_404(self.db(), settlement.id)
            .await
            .unwrap()
    }

    /// A payment of `fils` already made against `settlement` (a fixture).
    pub async fn paid_against(&self, settlement: &FinalSettlement, fils: i64) {
        SettlementPayment::create(
            self.db(),
            SettlementPayment {
                settlement_id: settlement.id,
                amount: fils,
                method: "transfer".into(),
                reference: "FIXTURE".into(),
                paid_on: nov(1),
                ..SettlementPayment::default()
            },
        )
        .await
        .unwrap();
    }

    /// `user` posts `fields` to the settlement route `path` (`pay`,
    /// `dispute`, `payments`, ...).
    pub async fn settle(
        &self,
        user: &User,
        settlement: &FinalSettlement,
        path: &str,
        fields: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(&format!("/hr/settlements/{}/{path}", settlement.id), fields)
            .await
    }

    /// `user` runs the transition `name` of `settlement` through its page
    /// route.
    pub async fn settle_transition(
        &self,
        user: &User,
        settlement: &FinalSettlement,
        name: &str,
    ) -> renox::testing::TestResponse {
        self.settle(
            user,
            settlement,
            &format!("transitions/{name}"),
            &[("reason", "")],
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
    boot_full(module, induction, Real::default(), None).await
}

/// [`boot`] with `calendar` as the app's payroll calendar.
pub async fn boot_with_calendar(
    calendar: impl wl_kernel::payroll_calendar::PayrollCalendar,
) -> Booted {
    boot_full(WlHr::new(), true, Real::default(), Some(Arc::new(calendar))).await
}

/// [`boot`] with the real wl-payments module, to which the payments HR makes
/// are recorded.
pub async fn boot_with_payments() -> Booted {
    let real = Real {
        payments: true,
        ..Real::default()
    };
    boot_full(WlHr::new(), true, real, None).await
}

/// [`boot`] with the real wl-attendance module in place of the fake
/// `Timesheets`: the hours payroll is given and the timesheets `submit` marks
/// are wl-attendance's own.
pub async fn boot_with_attendance() -> Booted {
    let real = Real {
        attendance: true,
        ..Real::default()
    };
    boot_full(WlHr::new(), true, real, None).await
}

/// [`boot`] with the real wl-ticketing module in place of the fake
/// `TaskCreator`: the labour case, the security task and the HR tasks are
/// tickets of wl-ticketing, seeded as `db:seed` seeds for a client with HR.
pub async fn boot_with_ticketing() -> Booted {
    let real = Real {
        ticketing: true,
        ..Real::default()
    };
    boot_full(WlHr::new(), true, real, None).await
}

/// [`boot`] with the real wl-notify module in place of the recording
/// `Notifier`: a message asked for is a stored `Message`, and
/// `stop_messages_to` cancels the waiting ones (`notices` records nothing).
pub async fn boot_with_notify() -> Booted {
    let real = Real {
        notify: true,
        ..Real::default()
    };
    boot_full(WlHr::new(), true, real, None).await
}

async fn boot_full(
    module: WlHr,
    induction: bool,
    real: Real,
    calendar: Option<Arc<dyn wl_kernel::payroll_calendar::PayrollCalendar>>,
) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let fakes = Fakes {
        real,
        notices: Recording::default(),
        checklists: ChecklistDesk::default(),
        sops: SopDesk::default(),
        docs: DocDesk::default(),
        timesheets: TimesheetDesk::default(),
        costs: CostDesk::default(),
        parties: PartyDesk::default(),
        pay_changed: Arc::default(),
        started: Arc::default(),
        completed: Arc::default(),
        activated: Arc::default(),
        changed: Arc::default(),
        qualification_expired: Arc::default(),
        qualification_expiring: Arc::default(),
        absconded: Arc::default(),
        onboarding_cancelled: Arc::default(),
        notice_withdrawn: Arc::default(),
        reinstated: Arc::default(),
        exited: Arc::default(),
        deceased: Arc::default(),
        visa_rejected: Arc::default(),
        attendance: AttendanceDesk::default(),
        returned: Arc::default(),
        failed: Arc::default(),
        paid: Arc::default(),
        settlement_paid: Arc::default(),
        settlement_disputed: Arc::default(),
        tasks: TicketDesk::default(),
        calendar,
        leave_approved: Arc::default(),
        leave_cancelled: Arc::default(),
    };
    let (qualification_expired, qualification_expiring, absconded, returned, failed, paid, tasks) = (
        Recorded(fakes.qualification_expired.clone()),
        Recorded(fakes.qualification_expiring.clone()),
        Recorded(fakes.absconded.clone()),
        Recorded(fakes.returned.clone()),
        Recorded(fakes.failed.clone()),
        Recorded(fakes.paid.clone()),
        fakes.tasks.clone(),
    );
    let (leave_approved, leave_cancelled) = (
        Recorded(fakes.leave_approved.clone()),
        Recorded(fakes.leave_cancelled.clone()),
    );
    let (deceased, exited) = (
        Recorded(fakes.deceased.clone()),
        Recorded(fakes.exited.clone()),
    );
    let (onboarding_cancelled, notice_withdrawn, reinstated, visa_rejected, attendance) = (
        Recorded(fakes.onboarding_cancelled.clone()),
        Recorded(fakes.notice_withdrawn.clone()),
        Recorded(fakes.reinstated.clone()),
        Recorded(fakes.visa_rejected.clone()),
        fakes.attendance.clone(),
    );
    let (settlement_paid, settlement_disputed) = (
        Recorded(fakes.settlement_paid.clone()),
        Recorded(fakes.settlement_disputed.clone()),
    );
    let (parties, pay_changed) = (fakes.parties.clone(), Recorded(fakes.pay_changed.clone()));
    let (notices, checklists, sops, docs, timesheets, costs) = (
        fakes.notices.clone(),
        fakes.checklists.clone(),
        fakes.sops.clone(),
        fakes.docs.clone(),
        fakes.timesheets.clone(),
        fakes.costs.clone(),
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
    let app = if real.payments {
        app.module(wl_payments::WlPayments::new())
    } else {
        app
    };
    let app = if real.attendance {
        app.module(wl_attendance::WlAttendance::new())
    } else {
        app
    };
    let app = if real.ticketing {
        app.module(wl_ticketing::WlTicketing::new())
    } else {
        app
    };
    let app = if real.notify {
        app.module(wl_notify::WlNotify::new())
    } else {
        app
    };
    let mut modules = vec!["parties", "hr"];
    if real.payments {
        modules.extend(["invoicing", "payments"]);
    }
    if real.attendance {
        modules.push("attendance");
    }
    if real.ticketing {
        modules.push("ticketing");
    }
    if real.notify {
        modules.push("notify");
    }
    let listed = modules
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let client = CLIENT.replace(r#"["parties", "hr"]"#, &format!("[{listed}]"));
    let config = wl_kernel::ClientConfig::parse(&client).expect("a valid client.toml");
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
        perm::LEAVE_VIEW,
        perm::LEAVE_APPROVE,
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
    // Pay accounts (`HRM.ACL.05`): one user approves what another asked for;
    // `hr_pay` holds both permissions, to show the requester cannot approve.
    permissions::define_role(
        app.db(),
        "pay_approver",
        &[perm::EMPLOYEES_VIEW, perm::PAY_ACCOUNTS_APPROVE],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "hr_pay",
        &[
            perm::EMPLOYEES_VIEW,
            perm::EMPLOYEES_MANAGE,
            perm::PAY_ACCOUNTS_APPROVE,
        ],
    )
    .await
    .unwrap();
    // The PRO runs the visa cases.
    permissions::define_role(app.db(), "pro", &[perm::VISAS_MANAGE])
        .await
        .unwrap();
    permissions::define_role(app.db(), "accountant", &[perm::PAYROLL_VIEW])
        .await
        .unwrap();
    // Final settlements (`HRM.ACL.04`): one user prepares, another approves, a
    // third pays; `settle_all` holds all three, to show the preparer cannot
    // approve their own figures.
    permissions::define_role(
        app.db(),
        "settle_prep",
        &[perm::EMPLOYEES_VIEW, perm::SETTLEMENTS_PREPARE],
    )
    .await
    .unwrap();
    permissions::define_role(app.db(), "settle_appr", &[perm::SETTLEMENTS_APPROVE])
        .await
        .unwrap();
    permissions::define_role(app.db(), "settle_pay", &[perm::SETTLEMENTS_PAY])
        .await
        .unwrap();
    permissions::define_role(
        app.db(),
        "settle_all",
        &[
            perm::SETTLEMENTS_PREPARE,
            perm::SETTLEMENTS_APPROVE,
            perm::SETTLEMENTS_PAY,
        ],
    )
    .await
    .unwrap();
    if induction {
        wl_hr::seed::install(app.db()).await.unwrap().total();
    }
    if real.ticketing {
        app.kernel()
            .call("ticketing:install", Vec::<String>::new())
            .await
            .expect("ticketing:install runs");
    }
    Booted {
        app,
        _views: views,
        notices,
        checklists,
        sops,
        docs,
        timesheets,
        costs,
        parties,
        pay_changed,
        started,
        completed,
        activated,
        changed,
        qualification_expired,
        qualification_expiring,
        absconded,
        onboarding_cancelled,
        notice_withdrawn,
        reinstated,
        deceased,
        exited,
        visa_rejected,
        attendance,
        returned,
        failed,
        paid,
        settlement_paid,
        settlement_disputed,
        tasks,
        leave_approved,
        leave_cancelled,
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
