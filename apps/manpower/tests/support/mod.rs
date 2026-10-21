//! Booting the Manpower vertical in a `TestApp` with auth and the kernel, and
//! fakes for what the vertical reaches through the kernel contracts: the party
//! directory, the rate cards, the workforce, the notifier and the portal. A
//! test cannot build an `AuthUser`, so a request is made through the
//! vertical's own routes as the user.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use manpower::permissions as perm;
use manpower::{
    ClientContract, ContractActivated, ContractEnded, ContractRenewed, ContractStatus,
    CoverRequested, Deployment, DeploymentActivated, DeploymentCancelled, DeploymentEnded,
    DeploymentStatus, Manpower, ManpowerRequest, RequestStatus, Site,
};
use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_kernel::attendance::{AttendanceHistory, LabourFuture, provide_attendance_history};
use wl_kernel::cheques::{
    ChequeBounceResolved, ChequeBounced, ChequeFuture, ChequeId, ChequeOutcome, ChequeStatus,
    ChequeSummary, Cheques, NoCheques, provide_cheques,
};
use wl_kernel::documents::{
    BlockEntry, CheckOutcome, DocumentFuture, DocumentGuard, FlagFailure, InvalidReason,
    RecordFlag, Validity, provide_guard,
};
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
    provide_invoicer,
};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, provide_directory};
use wl_kernel::payments::Direction;
use wl_kernel::portal::{PortalFuture, PortalUser, PortalUsers, provide_portal_users};
use wl_kernel::rate_cards::{RateCardFuture, RateCardVersion, RateCards, provide_rate_cards};
use wl_kernel::tickets::{
    OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::timesheets::{TimesheetClosing, TimesheetClosure, provide_timesheet_closing};
use wl_kernel::workforce::{
    AbsenceCaseClosed, AbsenceCaseOpened, EmployeeId, StaffProfile, Workforce, WorkforceFuture,
    provide_workforce,
};
use wl_kernel::{BusinessCalendar, PartyId, PartyKind, PartyStatus, PartySummary, Subject};

/// A manpower agency with the parties module standing in for the directory.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

/// The first client company.
pub const TOWER: PartyId = PartyId(31);

/// The second client company.
pub const FOODS: PartyId = PartyId(32);

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
    /// What was recorded, oldest first.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

impl<E> Default for Recorded<E> {
    fn default() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }
}

/// Records `event`s of type `E` into `log`.
fn record<E: Event + Clone + Send + Sync + 'static>(app: &mut Registry, log: &Recorded<E>) {
    let log = log.0.clone();
    app.listen(move |event: E, _state: AppState| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(event);
            Ok(())
        }
    });
}

/// The client companies the party directory knows.
#[derive(Clone, Default)]
pub struct Parties(Arc<Mutex<BTreeMap<i64, String>>>);

impl Parties {
    /// Knows the party `id` as `name`.
    pub fn know(&self, id: PartyId, name: &str) {
        self.0.lock().unwrap().insert(id.0, name.to_owned());
    }
}

impl PartyDirectory for Parties {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&party.0)
                .map(|name| PartySummary {
                    id: party,
                    code: format!("P-{:05}", party.0),
                    kind: PartyKind::Company,
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
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&party.0).cloned()) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .contains_key(&party.0)
                .then_some(PartyStatus::Active))
        })
    }
}

/// The employees of HR, as the workforce of the kernel answers for them.
#[derive(Clone, Default)]
pub struct Staff(
    Arc<Mutex<BTreeMap<i64, (StaffProfile, bool)>>>,
    Arc<Mutex<BTreeMap<i64, Vec<NaiveDate>>>>,
);

impl Staff {
    /// An employee `id` named `name`, `active` in HR or not (onboarding).
    pub fn employ(&self, parties: &Parties, id: i64, name: &str, active: bool) -> EmployeeId {
        let party = PartyId(1_000 + id);
        parties.know(party, name);
        self.0.lock().unwrap().insert(
            id,
            (
                StaffProfile {
                    employee: EmployeeId(id),
                    party,
                    user_id: None,
                    role: "guard".into(),
                    scopes: Vec::new(),
                    legal_entity_id: None,
                    listed: true,
                    on_leave: false,
                    returns_on: None,
                    managers: Vec::new(),
                },
                active,
            ),
        );
        EmployeeId(id)
    }

    /// HR recorded a failed attempt to reach the employee `id` on `day`.
    pub fn failed_to_reach(&self, id: i64, day: NaiveDate) {
        self.1.lock().unwrap().entry(id).or_default().push(day);
    }

    /// Puts the employee `id` on or off the active list of HR.
    pub fn set_active(&self, id: i64, active: bool) {
        if let Some(entry) = self.0.lock().unwrap().get_mut(&id) {
            entry.1 = active;
        }
    }
}

impl Workforce for Staff {
    fn staff<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&employee.0)
                .map(|(profile, _)| profile.clone()))
        })
    }

    fn is_active<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, bool> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&employee.0)
                .is_some_and(|(_, active)| *active))
        })
    }

    fn failed_contact_days<'a>(
        &'a self,
        _db: &'a Db,
        employee: EmployeeId,
        after: NaiveDate,
    ) -> WorkforceFuture<'a, Vec<NaiveDate>> {
        Box::pin(async move {
            let mut days: Vec<NaiveDate> = self
                .1
                .lock()
                .unwrap()
                .get(&employee.0)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|day| *day > after)
                .collect();
            days.sort_unstable();
            days.dedup();
            Ok(days)
        })
    }
}

/// The days the employees were present, as wl-attendance answers for them.
#[derive(Clone, Default)]
pub struct History(Arc<Mutex<BTreeMap<i64, Vec<NaiveDate>>>>);

impl History {
    /// The employee `id` attended on `day`.
    pub fn attended(&self, id: i64, day: NaiveDate) {
        self.0.lock().unwrap().entry(id).or_default().push(day);
    }
}

impl AttendanceHistory for History {
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
                .get(&employee.0)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|day| from <= *day && *day <= to)
                .collect();
            days.sort_unstable();
            days.dedup();
            Ok(days)
        })
    }
}

/// A ticket another module opened, as the tickets fake keeps it.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The ticket.
    pub ticket: OpenTicket,
    /// Its id.
    pub id: i64,
    /// Why it was completed; none while open.
    pub completed: Option<String>,
    /// The comments on it.
    pub comments: Vec<String>,
}

/// The tickets of wl-ticketing, as the vertical opens them.
#[derive(Clone, Default)]
pub struct Tickets(Arc<Mutex<Vec<Opened>>>);

impl Tickets {
    /// Every ticket opened, oldest first.
    pub fn all(&self) -> Vec<Opened> {
        self.0.lock().unwrap().clone()
    }
}

impl TaskCreator for Tickets {
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
                completed: None,
                comments: Vec::new(),
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
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|opened| {
                    opened.completed.is_none()
                        && opened.ticket.source_key.as_deref() == Some(source_key)
                })
                .map(|opened| TicketRef {
                    id: TicketId(opened.id),
                    number: format!("TKT-2027-{:04}", opened.id),
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
            let mut all = self.0.lock().unwrap();
            let found = all
                .iter_mut()
                .find(|opened| opened.id == ticket.0 && opened.completed.is_none());
            Ok(match found {
                Some(opened) => {
                    opened.completed = Some(note.to_owned());
                    true
                }
                None => false,
            })
        })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(async move {
            if let Some(opened) = self
                .0
                .lock()
                .unwrap()
                .iter_mut()
                .find(|opened| opened.id == ticket.0)
            {
                opened.comments.push(comment.text);
            }
            Ok(())
        })
    }
}

/// The portal grants: who acts for which client company, with which roles.
#[derive(Clone, Default)]
pub struct Portal(Arc<Mutex<Vec<PortalUser>>>);

impl Portal {
    /// `user` acts for `party` with the portal `roles`.
    pub fn grant(&self, user: &User, party: PartyId, roles: &[&str]) {
        self.0.lock().unwrap().push(PortalUser {
            user_id: user.id,
            person: PartyId(9_000 + user.id),
            acting_for: party,
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
        });
    }
}

impl PortalUsers for Portal {
    fn users_for<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Vec<PortalUser>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|user| user.acting_for == party)
                .cloned()
                .collect())
        })
    }

    fn acting_for<'a>(
        &'a self,
        _db: &'a Db,
        user: i64,
        party: PartyId,
        _on: NaiveDate,
    ) -> PortalFuture<'a, Option<PortalUser>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|grant| grant.user_id == user && grant.acting_for == party)
                .cloned())
        })
    }
}

/// An accepted rate card version and the roles and shifts it prices.
#[derive(Clone)]
struct Version {
    source: Subject,
    effective_from: NaiveDate,
    lines: Vec<(String, String)>,
}

/// The rate cards of wl-invoicing, as the vertical asks for them.
#[derive(Clone, Default)]
pub struct Cards(Arc<Mutex<Vec<Version>>>);

impl Cards {
    /// The client accepted a version of the card of `source` effective from
    /// `effective_from`, pricing `lines` (role, shift type).
    pub fn accept(&self, source: &Subject, effective_from: NaiveDate, lines: &[(&str, &str)]) {
        self.0.lock().unwrap().push(Version {
            source: source.clone(),
            effective_from,
            lines: lines
                .iter()
                .map(|(role, shift)| ((*role).to_owned(), (*shift).to_owned()))
                .collect(),
        });
    }

    /// The accepted version in force on `on`: the latest effective on or
    /// before it.
    fn in_force(&self, source: &Subject, on: NaiveDate) -> Option<Version> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.source == *source && v.effective_from <= on)
            .max_by_key(|v| v.effective_from)
            .cloned()
    }
}

impl RateCards for Cards {
    fn accepted_version<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> RateCardFuture<'a, Option<RateCardVersion>> {
        Box::pin(async move {
            Ok(self.in_force(source, on).map(|v| RateCardVersion {
                rate_card: source.subject_id,
                version: 1,
                effective_from: v.effective_from,
            }))
        })
    }

    fn covers<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
        role: &'a str,
        shift: &'a str,
        on: NaiveDate,
    ) -> RateCardFuture<'a, bool> {
        Box::pin(async move {
            Ok(self.in_force(source, on).is_some_and(|v| {
                v.lines
                    .iter()
                    .any(|(r, s)| r.eq_ignore_ascii_case(role) && s.eq_ignore_ascii_case(shift))
            }))
        })
    }
}

/// The closing of timesheets of wl-attendance, as the vertical asks for it:
/// it records each call and answers that the worker's open timesheet was
/// submitted.
#[derive(Clone, Default)]
pub struct Closings(Arc<Mutex<Vec<(Subject, EmployeeId, NaiveDate)>>>);

impl Closings {
    /// Every call so far, oldest first: the context, the owner and the last day.
    pub fn all(&self) -> Vec<(Subject, EmployeeId, NaiveDate)> {
        self.0.lock().unwrap().clone()
    }
}

impl TimesheetClosing for Closings {
    fn close_open<'a>(
        &'a self,
        _state: &'a AppState,
        context: &'a Subject,
        owner: EmployeeId,
        last_day: NaiveDate,
    ) -> wl_kernel::attendance::LabourFuture<'a, Vec<TimesheetClosure>> {
        self.0
            .lock()
            .unwrap()
            .push((context.clone(), owner, last_day));
        Box::pin(async move {
            Ok(vec![TimesheetClosure {
                timesheet: Subject::new("attendance.timesheet", owner.0),
                period_to: last_day,
                submitted: true,
                message: None,
            }])
        })
    }
}

/// The document guard of wl-documents, as the vertical asks it: a kind is
/// valid unless a test expired it for the employee; no kind blocks anything
/// (the guard of `MPW.ACL.07` is tested with the real module in the host).
#[derive(Clone, Default)]
pub struct Documents(Arc<Mutex<BTreeSet<(i64, String)>>>);

impl Documents {
    /// The `kind` of employee `employee` is expired from now on.
    pub fn expire(&self, employee: i64, kind: &str) {
        self.0.lock().unwrap().insert((employee, kind.to_owned()));
    }

    /// A renewed `kind` of employee `employee` is accepted.
    pub fn renew(&self, employee: i64, kind: &str) {
        self.0.lock().unwrap().remove(&(employee, kind.to_owned()));
    }
}

impl DocumentGuard for Documents {
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
        let expired = self
            .0
            .lock()
            .unwrap()
            .contains(&(subject.subject_id, kind.to_owned()));
        Box::pin(async move {
            Ok(if expired {
                Validity::Invalid {
                    reason: InvalidReason::Expired,
                    document: None,
                    expired_on: None,
                    pending: None,
                }
            } else {
                Validity::Valid {
                    document: None,
                    expires_on: None,
                }
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
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, wl_kernel::documents::BlockRefusal>>
    {
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

/// A notifier that records every request and queues nothing.
#[derive(Clone, Default)]
pub struct Recording(
    Arc<Mutex<Vec<NotifyRequest>>>,
    Arc<AtomicBool>,
    Arc<Mutex<Vec<String>>>,
);

impl Recording {
    /// Requests of the type `key` work even while the notifier is failing (the
    /// kernel's alert to the owner is a different channel from the client's).
    pub fn always_works(&self, key: &str) {
        self.2.lock().unwrap().push(key.to_owned());
    }

    /// Makes every further request fail (or work again).
    pub fn fail(&self, failing: bool) {
        self.1.store(failing, Ordering::SeqCst);
    }

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

/// The users a request is for.
pub fn users_of(request: &NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|recipient| match recipient {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        let exempt = self.2.lock().unwrap().contains(&request.type_key);
        if self.1.load(Ordering::SeqCst) && !exempt {
            return Box::pin(async { Err(Error::permanent_message("the notifier is down")) });
        }
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The invoices of wl-invoicing, as the vertical reads them: each bills the
/// contract it was issued under.
#[derive(Clone, Default)]
pub struct Invoices(Arc<Mutex<Vec<(InvoiceSummary, Subject)>>>);

impl Invoices {
    /// An invoice `number` of AED `balance` fils issued under `contract`, due
    /// on `due`, still unpaid.
    pub fn issue(
        &self,
        contract: &ClientContract,
        number: &str,
        balance: i64,
        due: NaiveDate,
    ) -> InvoiceId {
        let mut all = self.0.lock().unwrap();
        let id = InvoiceId(all.len() as i64 + 1);
        all.push((
            InvoiceSummary {
                id,
                number: Some(number.to_owned()),
                status: InvoiceStatus::Overdue,
                party: PartyId(contract.client_party_id),
                legal_entity_id: None,
                currency: "AED".into(),
                gross_total: balance,
                balance,
                due_date: Some(due),
                disputed: false,
                chasing_paused_until: None,
            },
            contract.subject(),
        ));
        id
    }

    /// Changes the invoice `id`.
    pub fn change(&self, id: InvoiceId, change: impl FnOnce(&mut InvoiceSummary)) {
        let mut all = self.0.lock().unwrap();
        let (summary, _) = all
            .iter_mut()
            .find(|(summary, _)| summary.id == id)
            .expect("a known invoice");
        change(summary);
    }

    /// `amount` fils of the invoice `id` are paid.
    pub fn pay(&self, id: InvoiceId, amount: i64) {
        self.change(id, |summary| {
            summary.balance -= amount;
            summary.status = if summary.balance == 0 {
                InvoiceStatus::Paid
            } else {
                InvoiceStatus::PartiallyPaid
            };
        });
    }

    /// The invoice `id` is under a billing dispute.
    pub fn dispute(&self, id: InvoiceId) {
        self.change(id, |summary| summary.disputed = true);
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
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|(summary, _)| summary.id == id)
                .map(|(summary, _)| summary.clone()))
        })
    }

    fn billing<'a>(
        &'a self,
        _db: &'a Db,
        origin: &'a Subject,
    ) -> InvoiceFuture<'a, Vec<InvoiceId>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, source)| source == origin)
                .map(|(summary, _)| summary.id)
                .collect())
        })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        _source: Subject,
        amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async move {
            self.pay(id, amount);
            Ok(())
        })
    }
}

/// The cheques of wl-payments, as the vertical looks them up.
#[derive(Clone, Default)]
pub struct Register(Arc<Mutex<Vec<ChequeSummary>>>);

impl Register {
    /// A cheque `number` of `client` for `amount` fils bounced: it is in the
    /// register as `bounced`, and the event is what wl-payments emits.
    pub fn bounce(&self, id: i64, client: PartyId, number: &str, amount: i64) -> ChequeBounced {
        self.0.lock().unwrap().push(ChequeSummary {
            id: ChequeId(id),
            direction: Direction::In,
            party: client,
            legal_entity_id: 1,
            bank: "Emirates NBD".into(),
            number: number.to_owned(),
            date: date(2026, 10, 1),
            amount,
            honoured_amount: 0,
            open_amount: amount,
            currency: "AED".into(),
            presentations: 1,
            status: ChequeStatus::Bounced,
            agreement: None,
            linked: Vec::new(),
        });
        ChequeBounced {
            cheque: ChequeId(id),
            party: client,
            credit_group: None,
            amount,
            honoured_amount: 0,
            open_amount: amount,
            currency: "AED".into(),
            reason: "insufficient funds".into(),
            presentation: 1,
            linked: Vec::new(),
        }
    }

    /// The cheque `id` was cleared after a re-presentation: the register says
    /// so and the event is what wl-payments emits.
    pub fn clear(&self, id: i64) -> ChequeBounceResolved {
        let mut all = self.0.lock().unwrap();
        let cheque = all
            .iter_mut()
            .find(|cheque| cheque.id == ChequeId(id))
            .expect("a known cheque");
        cheque.status = ChequeStatus::Cleared;
        cheque.open_amount = 0;
        ChequeBounceResolved {
            cheque: cheque.id,
            party: cheque.party,
            outcome: ChequeOutcome::Cleared,
            others_open: Vec::new(),
        }
    }
}

impl Cheques for Register {
    fn register<'a>(
        &'a self,
        state: &'a AppState,
        cheque: wl_kernel::cheques::RegisterCheque,
        by: Option<i64>,
    ) -> ChequeFuture<'a, ChequeSummary> {
        NoCheques.register(state, cheque, by)
    }

    fn register_schedule<'a>(
        &'a self,
        state: &'a AppState,
        schedule: wl_kernel::cheques::ChequeSchedule,
        by: Option<i64>,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        NoCheques.register_schedule(state, schedule, by)
    }

    fn find<'a>(&'a self, _db: &'a Db, id: ChequeId) -> ChequeFuture<'a, Option<ChequeSummary>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|cheque| cheque.id == id)
                .cloned())
        })
    }

    fn of_agreement<'a>(
        &'a self,
        _db: &'a Db,
        _agreement: &'a Subject,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn open_bounced_amount<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> ChequeFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }
}

/// Provides the fakes.
struct Fakes {
    invoices: Invoices,
    register: Register,
    parties: Parties,
    cards: Cards,
    portal: Portal,
    staff: Staff,
    notices: Recording,
    activated: Recorded<ContractActivated>,
    deployed: Recorded<DeploymentActivated>,
    ended: Recorded<DeploymentEnded>,
    cancelled: Recorded<DeploymentCancelled>,
    covers: Recorded<CoverRequested>,
    renewed: Recorded<ContractRenewed>,
    contracts_ended: Recorded<ContractEnded>,
    closings: Closings,
    documents: Documents,
    history: History,
    tickets: Tickets,
    case_opened: Recorded<AbsenceCaseOpened>,
    case_closed: Recorded<AbsenceCaseClosed>,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_directory(app, self.parties.clone());
        provide_rate_cards(app, self.cards.clone());
        provide_portal_users(app, self.portal.clone());
        provide_notifier(app, self.notices.clone());
        provide_workforce(app, self.staff.clone());
        provide_invoicer(app, self.invoices.clone());
        provide_cheques(app, self.register.clone());
        record(app, &self.activated);
        record(app, &self.deployed);
        record(app, &self.ended);
        record(app, &self.cancelled);
        record(app, &self.covers);
        record(app, &self.renewed);
        record(app, &self.contracts_ended);
        provide_timesheet_closing(app, self.closings.clone());
        provide_guard(app, self.documents.clone());
        provide_attendance_history(app, self.history.clone());
        provide_task_creator(app, self.tickets.clone());
        record(app, &self.case_opened);
        record(app, &self.case_closed);
    }
}

/// The app under test with the fakes.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// The party directory.
    pub parties: Parties,
    /// The rate cards.
    pub cards: Cards,
    /// The portal grants.
    pub portal: Portal,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// The workforce of HR.
    pub staff: Staff,
    /// The invoices of wl-invoicing.
    pub invoices: Invoices,
    /// The cheques of wl-payments.
    pub register: Register,
    /// `ContractActivated`.
    pub activated: Recorded<ContractActivated>,
    /// `DeploymentActivated`.
    pub deployed: Recorded<DeploymentActivated>,
    /// `DeploymentEnded`.
    pub ended: Recorded<DeploymentEnded>,
    /// `DeploymentCancelled`.
    pub cancelled: Recorded<DeploymentCancelled>,
    /// `CoverRequested`.
    pub covers: Recorded<CoverRequested>,
    /// The events `ContractRenewed`.
    pub renewed: Recorded<ContractRenewed>,
    /// `ContractEnded`.
    pub contracts_ended: Recorded<ContractEnded>,
    /// The calls to close an open timesheet.
    pub closings: Closings,
    /// The documents of the employees.
    pub documents: Documents,
    /// The days the employees attended.
    pub history: History,
    /// The tickets opened.
    pub tickets: Tickets,
    /// `AbsenceCaseOpened`.
    pub case_opened: Recorded<AbsenceCaseOpened>,
    /// `AbsenceCaseClosed`.
    pub case_closed: Recorded<AbsenceCaseClosed>,
    /// Days the clock was moved forward from the real one.
    clock_secs: std::sync::atomic::AtomicI64,
    /// The contracts made, for their references.
    made: std::sync::atomic::AtomicI64,
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

    /// A user with no role.
    pub async fn user(&self, name: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        User::register(self.db(), name, &email, "password123")
            .await
            .unwrap()
    }

    /// A user holding the role `role` everywhere.
    pub async fn user_with(&self, name: &str, role: &str) -> User {
        let user = self.user(name).await;
        user.assign_role(self.db(), role).await.unwrap();
        user
    }

    /// An account manager of the client company `client` only.
    pub async fn account_manager(&self, name: &str, client: PartyId) -> User {
        let user = self.user(name).await;
        user.assign_role_in(
            self.db(),
            perm::ACCOUNT_MANAGER_ROLE,
            &perm::client_scope(client.0),
        )
        .await
        .unwrap();
        user
    }

    /// The MD.
    pub async fn md(&self, name: &str) -> User {
        self.user_with(name, perm::MD_ROLE).await
    }

    /// Runs `work` with the clock on `day` (Asia/Dubai) at midday, moved
    /// forward from the real one; days only go forward within a test. The
    /// time of day is pinned too: a message sent in quiet hours would be
    /// deferred, so a test that expects one at once must not depend on the
    /// hour it runs.
    pub async fn on<F: std::future::Future>(&self, day: NaiveDate, work: F) -> F::Output {
        use renox::chrono::Duration;
        use std::sync::atomic::Ordering;
        let moved = self.clock_secs.load(Ordering::SeqCst);
        let now = renox::db::now() + Duration::seconds(moved);
        // Midday in Asia/Dubai (UTC+4) is 08:00 UTC.
        let midday = day.and_hms_opt(8, 0, 0).unwrap().and_utc();
        let need = (midday - now).num_seconds();
        if need > 0 {
            self.app
                .travel(std::time::Duration::from_secs(u64::try_from(need).unwrap()));
            self.clock_secs.store(moved + need, Ordering::SeqCst);
        } else {
            assert_eq!(
                wl_kernel::documents::dubai_date(now),
                day,
                "the clock cannot go back to {day}"
            );
        }
        self.app.at_travelled_time(work).await
    }

    /// A contract of `client` in `status`, from 1 January to 31 December
    /// 2027, inserted as a fixture, managed by `manager`.
    pub async fn contract(
        &self,
        client: PartyId,
        status: ContractStatus,
        manager: Option<&User>,
    ) -> ClientContract {
        let n = self.made.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let mut contract = ClientContract {
            reference: format!("CC-{n:04}"),
            client_party_id: client.0,
            start_date: date(2027, 1, 1),
            end_date: date(2027, 12, 31),
            payment_terms_days: 30,
            notice_period_days: 30,
            account_manager_user_id: manager.map(|user| user.id),
            status,
            ..ClientContract::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        contract
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        contract
    }

    /// A client user of the portal who acts for `party` with the portal
    /// `roles` (`owner`, `approver`, `viewer`...).
    pub async fn client_user(&self, name: &str, party: PartyId, roles: &[&str]) -> User {
        let user = self.user(name).await;
        self.portal.grant(&user, party, roles);
        user
    }

    /// A site of `client`, in use.
    pub async fn site(&self, client: PartyId, name: &str) -> Site {
        Site::create(
            self.db(),
            Site {
                client_party_id: client.0,
                name: name.to_owned(),
                active: true,
                ..Site::default()
            },
        )
        .await
        .unwrap()
    }

    /// A request for `headcount` guards on the day shift at `site`, from 1
    /// February 2027 to 31 July 2027, in `status` (a fixture).
    pub async fn request(
        &self,
        contract: &ClientContract,
        site: &Site,
        headcount: i64,
        status: RequestStatus,
    ) -> ManpowerRequest {
        let mut request = ManpowerRequest {
            client_contract_id: contract.id,
            client_party_id: contract.client_party_id,
            site_id: site.id,
            role: "guard".into(),
            headcount,
            shift_type: "day".into(),
            shift_start: Some("07:00".into()),
            shift_end: Some("19:00".into()),
            start_date: date(2027, 2, 1),
            end_date: Some(date(2027, 7, 31)),
            status,
            ..ManpowerRequest::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        request
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        request
    }

    /// A deployment of `employee` as a guard on the day shift at `site`, from
    /// 1 February 2027 to `end`, in `status` (a fixture).
    pub async fn deployment(
        &self,
        contract: &ClientContract,
        site: &Site,
        employee: EmployeeId,
        end: Option<NaiveDate>,
        status: DeploymentStatus,
    ) -> Deployment {
        let mut deployment = Deployment {
            employee_id: employee.0,
            client_contract_id: contract.id,
            client_party_id: contract.client_party_id,
            site_id: site.id,
            role: "guard".into(),
            shift_type: "day".into(),
            shift_start: Some("07:00".into()),
            shift_end: Some("19:00".into()),
            start_date: date(2027, 2, 1),
            end_date: end,
            status,
            ..Deployment::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        deployment
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        deployment
    }

    /// Links the deployment to the request it fills (a fixture).
    pub async fn fills(&self, deployment: &Deployment, request: &ManpowerRequest) {
        Deployment::query()
            .where_(Deployment::ID.eq(deployment.id))
            .update(self.db(), &[("request_id", &Some(request.id))])
            .await
            .unwrap();
    }

    /// The deployment `id` as it is now.
    pub async fn reload_deployment(&self, id: i64) -> Deployment {
        Deployment::find_or_404(self.db(), id).await.unwrap()
    }

    /// The request `id` as it is now.
    pub async fn reload_request(&self, id: i64) -> ManpowerRequest {
        ManpowerRequest::find_or_404(self.db(), id).await.unwrap()
    }

    /// The contract `id` as it is now.
    pub async fn reload_contract(&self, id: i64) -> ClientContract {
        ClientContract::find_or_404(self.db(), id).await.unwrap()
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

    /// A GET as `by` the way a browser sends it.
    pub async fn browse(&self, by: &User, path: &str) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .request()
            .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
            .get(path)
            .await
    }
}

/// Boots the app.
pub async fn boot() -> Booted {
    let parties = Parties::default();
    parties.know(TOWER, "Gulf Tower LLC");
    parties.know(FOODS, "Desert Foods LLC");
    let cards = Cards::default();
    let portal = Portal::default();
    let staff = Staff::default();
    let notices = Recording::default();
    let activated = Recorded::default();
    let deployed = Recorded::default();
    let ended = Recorded::default();
    let cancelled = Recorded::default();
    let covers = Recorded::default();
    let renewed = Recorded::default();
    let contracts_ended = Recorded::default();
    let closings = Closings::default();
    let documents = Documents::default();
    let history = History::default();
    let tickets = Tickets::default();
    let case_opened = Recorded::default();
    let case_closed = Recorded::default();
    let invoices = Invoices::default();
    let register = Register::default();
    let fakes = Fakes {
        invoices: invoices.clone(),
        register: register.clone(),
        parties: parties.clone(),
        cards: cards.clone(),
        portal: portal.clone(),
        staff: staff.clone(),
        notices: notices.clone(),
        activated: activated.clone(),
        deployed: deployed.clone(),
        ended: ended.clone(),
        cancelled: cancelled.clone(),
        covers: covers.clone(),
        renewed: renewed.clone(),
        contracts_ended: contracts_ended.clone(),
        closings: closings.clone(),
        documents: documents.clone(),
        history: history.clone(),
        tickets: tickets.clone(),
        case_opened: case_opened.clone(),
        case_closed: case_closed.clone(),
    };
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(Manpower)
        .module(fakes);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    // The pages extend the app's layout; a vertical's test has no host, so it
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
    // The account manager runs the client accounts; the MD only sees them.
    permissions::define_role(
        app.db(),
        perm::ACCOUNT_MANAGER_ROLE,
        &[
            perm::CONTRACTS_VIEW,
            perm::CONTRACTS_MANAGE,
            perm::SITES_MANAGE,
            perm::REQUESTS_VIEW,
            perm::REQUESTS_CREATE,
            perm::REQUESTS_MANAGE,
            perm::DEPLOYMENTS_VIEW,
            perm::DEPLOYMENTS_MANAGE,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(app.db(), perm::MD_ROLE, &[perm::CONTRACTS_VIEW])
        .await
        .unwrap();
    permissions::define_role(app.db(), perm::SUPERVISOR_ROLE, &[perm::DEPLOYMENTS_VIEW])
        .await
        .unwrap();
    Booted {
        app,
        parties,
        cards,
        portal,
        staff,
        invoices,
        register,
        notices,
        activated,
        deployed,
        ended,
        cancelled,
        covers,
        renewed,
        contracts_ended,
        closings,
        documents,
        history,
        tickets,
        case_opened,
        case_closed,
        clock_secs: std::sync::atomic::AtomicI64::new(0),
        made: std::sync::atomic::AtomicI64::new(0),
        _views: views,
    }
}
