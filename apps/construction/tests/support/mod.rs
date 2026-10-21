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
use construction::{
    Construction, DeductionKind, DeductionStatus, Guarantee, GuaranteeKind, GuaranteeStatus, Ipa,
    IpaDeduction, IpaStatus, Project, ProjectStatus, ProjectTerminated, Retention, RetentionPart,
    RetentionStatus,
};
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
use wl_kernel::documents::{
    BlockEntry, CheckOutcome, DocumentFuture, DocumentGuard, FlagFailure, InvalidReason,
    RecordFlag, Validity, provide_guard,
};
use wl_kernel::invoicing::{
    BillRequest, CostPoster, CreditLine, CreditNoteId, DisputeRequest, InvoiceDraft, InvoiceFuture,
    InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer, PostCost, SupplierBillFuture,
    SupplierBillId, SupplierBillLine, SupplierBillLineId, SupplierBills, provide_cost_poster,
    provide_invoicer, provide_supplier_bills,
};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, provide_directory};
use wl_kernel::tickets::{
    OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::workforce::{
    AbsenceCaseOpened, EmployeeId, LeaveCalendar, StaffProfile, Workforce, WorkforceFuture,
    provide_leave_calendar, provide_workforce,
};
use wl_kernel::{BusinessCalendar, PartyId, PartyStatus, PartySummary, Subject};

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

impl Recording {
    /// The notices of the vertical (`construction.notice`) whose title
    /// contains `text`, oldest first.
    pub fn notices_titled(&self, text: &str) -> Vec<NotifyRequest> {
        self.of_type(construction::notices::NOTICE)
            .into_iter()
            .filter(|request| {
                request.data["title"]
                    .as_str()
                    .is_some_and(|title| title.contains(text))
            })
            .collect()
    }
}

impl Recording {
    /// The urgent notices of the vertical (`construction.urgent_notice`)
    /// whose title contains `text`, oldest first.
    pub fn urgent_titled(&self, text: &str) -> Vec<NotifyRequest> {
        self.of_type(construction::notices::URGENT_NOTICE)
            .into_iter()
            .filter(|request| {
                request.data["title"]
                    .as_str()
                    .is_some_and(|title| title.contains(text))
            })
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
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The employees of HR, as the workforce of the kernel answers for them, with
/// their approved leave and the absence cases the vertical announced.
#[derive(Clone, Default)]
pub struct Staff {
    people: Arc<Mutex<BTreeMap<i64, StaffProfile>>>,
    leave: Arc<Mutex<Vec<(i64, NaiveDate, NaiveDate)>>>,
    /// `AbsenceCaseOpened`.
    pub opened: Arc<Mutex<Vec<AbsenceCaseOpened>>>,
}

impl Staff {
    /// Puts an employee who is at work among the staff.
    pub fn employ(&self, id: i64) -> EmployeeId {
        self.people.lock().unwrap().insert(
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

    /// Assigns the employee `id` to `scope` (a project).
    pub fn assign(&self, id: i64, scope: Subject) {
        if let Some(profile) = self.people.lock().unwrap().get_mut(&id) {
            profile.scopes.push(scope);
        }
    }

    /// Approves leave of the employee `id` from `from` to `to`.
    pub fn approve_leave(&self, id: i64, from: NaiveDate, to: NaiveDate) {
        self.leave.lock().unwrap().push((id, from, to));
    }

    /// The absence cases announced.
    pub fn opened(&self) -> Vec<AbsenceCaseOpened> {
        self.opened.lock().unwrap().clone()
    }
}

impl Workforce for Staff {
    fn all_staff<'a>(&'a self, _db: &'a Db) -> WorkforceFuture<'a, Vec<EmployeeId>> {
        Box::pin(async move {
            Ok(self
                .people
                .lock()
                .unwrap()
                .keys()
                .map(|id| EmployeeId(*id))
                .collect())
        })
    }

    fn staff<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        Box::pin(async move { Ok(self.people.lock().unwrap().get(&employee.0).cloned()) })
    }
}

impl LeaveCalendar for Staff {
    fn approved_leave<'a>(
        &'a self,
        _db: &'a Db,
        employee: EmployeeId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> WorkforceFuture<'a, Vec<(NaiveDate, NaiveDate)>> {
        Box::pin(async move {
            Ok(self
                .leave
                .lock()
                .unwrap()
                .iter()
                .filter(|(id, first, last)| *id == employee.0 && *first <= to && *last >= from)
                .map(|(_, first, last)| (*first, *last))
                .collect())
        })
    }
}

/// What the fake invoicing was asked for and holds.
#[derive(Default)]
struct InvoiceBook {
    summaries: BTreeMap<i64, InvoiceSummary>,
    drafts: Vec<(i64, InvoiceDraft)>,
    issued: Vec<i64>,
    credits: Vec<CreditAsked>,
    disputes: Vec<DisputeRequest>,
    next: i64,
}

/// A credit note the vertical asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditAsked {
    /// The invoice credited.
    pub invoice: InvoiceId,
    /// The lines credited.
    pub lines: Vec<CreditLine>,
    /// Why.
    pub reason: String,
}

/// The invoices of wl-invoicing, as the vertical reads them and asks for
/// them.
#[derive(Clone, Default)]
pub struct Invoices(Arc<Mutex<InvoiceBook>>);

impl Invoices {
    /// An invoice in `status`; returns its id.
    pub fn put(&self, status: InvoiceStatus) -> i64 {
        let mut book = self.0.lock().unwrap();
        book.next += 1;
        let id = book.next;
        book.summaries.insert(
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

    /// Puts invoice `id` in `status`, as wl-invoicing would after a payment,
    /// a credit note or a write-off.
    pub fn set_status(&self, id: i64, status: InvoiceStatus) {
        let mut book = self.0.lock().unwrap();
        if let Some(summary) = book.summaries.get_mut(&id) {
            summary.status = status;
        }
    }

    /// The invoice as it is held now.
    pub fn get(&self, id: i64) -> InvoiceSummary {
        self.0.lock().unwrap().summaries[&id].clone()
    }

    /// The drafts the vertical made, with their ids.
    pub fn drafts(&self) -> Vec<(i64, InvoiceDraft)> {
        self.0.lock().unwrap().drafts.clone()
    }

    /// The ids of the invoices issued.
    pub fn issued(&self) -> Vec<i64> {
        self.0.lock().unwrap().issued.clone()
    }

    /// The credit notes asked for.
    pub fn credits(&self) -> Vec<CreditAsked> {
        self.0.lock().unwrap().credits.clone()
    }

    /// The disputes raised.
    pub fn disputes(&self) -> Vec<DisputeRequest> {
        self.0.lock().unwrap().disputes.clone()
    }
}

impl Invoicer for Invoices {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        let mut book = self.0.lock().unwrap();
        book.next += 1;
        let id = book.next;
        let gross: i64 = draft.lines.iter().map(|line| line.gross()).sum();
        book.summaries.insert(
            id,
            InvoiceSummary {
                id: InvoiceId(id),
                number: None,
                status: InvoiceStatus::Draft,
                party: draft.party,
                legal_entity_id: draft.legal_entity_id,
                currency: draft.currency.clone(),
                gross_total: gross,
                balance: gross,
                due_date: None,
                disputed: false,
                chasing_paused_until: None,
            },
        );
        book.drafts.push((id, draft));
        Box::pin(async move { Ok(Some(InvoiceId(id))) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        let found = self.0.lock().unwrap().summaries.get(&id.0).cloned();
        Box::pin(async move { Ok(found) })
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

    fn issue<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        _by: Option<&'a AuthUser>,
    ) -> InvoiceFuture<'a, ()> {
        let mut book = self.0.lock().unwrap();
        if let Some(summary) = book.summaries.get_mut(&id.0) {
            summary.status = InvoiceStatus::Issued;
            summary.number = Some(format!("INV-2027-{:04}", id.0));
        }
        book.issued.push(id.0);
        Box::pin(async { Ok(()) })
    }

    fn credit_partly<'a>(
        &'a self,
        _state: &'a AppState,
        invoice: InvoiceId,
        lines: Vec<CreditLine>,
        reason: String,
        _by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        let mut book = self.0.lock().unwrap();
        book.credits.push(CreditAsked {
            invoice,
            lines,
            reason,
        });
        let id = i64::try_from(book.credits.len()).unwrap();
        Box::pin(async move { Ok(CreditNoteId(id)) })
    }

    fn dispute<'a>(
        &'a self,
        _state: &'a AppState,
        request: DisputeRequest,
    ) -> InvoiceFuture<'a, i64> {
        let mut book = self.0.lock().unwrap();
        if let Some(summary) = book.summaries.get_mut(&request.invoice.0) {
            summary.disputed = true;
        }
        book.disputes.push(request);
        let id = i64::try_from(book.disputes.len()).unwrap();
        Box::pin(async move { Ok(id) })
    }
}

/// The supplier bills the vertical asked wl-invoicing to record.
#[derive(Clone, Default)]
pub struct Bills(Arc<Mutex<Vec<BillRequest>>>);

impl Bills {
    /// The bills asked for, oldest first; the first has the id 1.
    pub fn requests(&self) -> Vec<BillRequest> {
        self.0.lock().unwrap().clone()
    }
}

impl SupplierBills for Bills {
    fn line<'a>(
        &'a self,
        _db: &'a Db,
        _id: SupplierBillLineId,
    ) -> SupplierBillFuture<'a, Option<SupplierBillLine>> {
        Box::pin(async { Ok(None) })
    }

    fn create_submitted<'a>(
        &'a self,
        _state: &'a AppState,
        request: BillRequest,
    ) -> SupplierBillFuture<'a, Option<SupplierBillId>> {
        let mut asked = self.0.lock().unwrap();
        asked.push(request);
        let id = i64::try_from(asked.len()).unwrap();
        Box::pin(async move { Ok(Some(SupplierBillId(id))) })
    }
}

/// The cost posted on cost objects, as the cost poster is asked for it.
#[derive(Clone, Default)]
pub struct Costs(Arc<Mutex<Vec<PostCost>>>);

impl Costs {
    /// The cost posted, oldest first.
    pub fn posted(&self) -> Vec<PostCost> {
        self.0.lock().unwrap().clone()
    }
}

impl CostPoster for Costs {
    fn post<'a>(&'a self, _state: &'a AppState, request: PostCost) -> SupplierBillFuture<'a, ()> {
        let mut posted = self.0.lock().unwrap();
        // A key that was used already changes nothing.
        if !posted.iter().any(|done| done.key == request.key) {
            posted.push(request);
        }
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

/// The subcontractors among the parties and the Dubai Municipality
/// registrations on file (the party directory and the document guard of the
/// kernel, as wl-parties and wl-documents answer them).
#[derive(Clone, Default)]
pub struct Registrations {
    subcontractors: Arc<Mutex<Vec<i64>>>,
    registered_until: Arc<Mutex<BTreeMap<i64, NaiveDate>>>,
}

impl Registrations {
    /// The party `id` has the role `subcontractor`.
    pub fn subcontractor(&self, id: PartyId) {
        self.subcontractors.lock().unwrap().push(id.0);
    }

    /// The party `id` has a registration valid through `expires_on`.
    pub fn register(&self, id: PartyId, expires_on: NaiveDate) {
        self.registered_until
            .lock()
            .unwrap()
            .insert(id.0, expires_on);
    }
}

impl PartyDirectory for Registrations {
    fn find<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
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
        _party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn status<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async { Ok(None) })
    }

    fn has_role<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        role_key: &'a str,
        _on_date: NaiveDate,
    ) -> PartyFuture<'a, bool> {
        Box::pin(async move {
            Ok(role_key == "subcontractor"
                && self.subcontractors.lock().unwrap().contains(&party.0))
        })
    }
}

impl DocumentGuard for Registrations {
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
        _kind: &'a str,
        on_date: NaiveDate,
    ) -> DocumentFuture<'a, Validity> {
        Box::pin(async move {
            let until = self
                .registered_until
                .lock()
                .unwrap()
                .get(&subject.subject_id)
                .copied();
            Ok(match until {
                Some(until) if until >= on_date => Validity::Valid {
                    document: None,
                    expires_on: Some(until),
                },
                Some(until) => Validity::Invalid {
                    reason: InvalidReason::Expired,
                    document: None,
                    expired_on: Some(until),
                    pending: None,
                },
                None => Validity::Invalid {
                    reason: InvalidReason::Missing,
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

/// Records the events and provides the fakes.
struct Fakes {
    blocked: Blocked,
    staff: Staff,
    tasks: Tasks,
    notices: Recording,
    invoices: Invoices,
    bills: Bills,
    costs: Costs,
    terminated: Arc<Mutex<Vec<ProjectTerminated>>>,
    registrations: Registrations,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_workforce(app, self.staff.clone());
        provide_directory(app, self.registrations.clone());
        provide_guard(app, self.registrations.clone());
        provide_leave_calendar(app, self.staff.clone());
        record(app, &self.staff.opened);
        provide_task_creator(app, self.tasks.clone());
        provide_notifier(app, self.notices.clone());
        provide_invoicer(app, self.invoices.clone());
        provide_supplier_bills(app, self.bills.clone());
        provide_cost_poster(app, self.costs.clone());
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
    /// The supplier bills asked for.
    pub bills: Bills,
    /// The cost posted on cost objects.
    pub costs: Costs,
    /// The parties wl-parties blocks.
    pub blocked: Blocked,
    /// The subcontractors and their registrations.
    pub registrations: Registrations,
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

    /// A user holding a role that only names who is told (no permission).
    pub async fn named(&self, name: &str, role: &str) -> User {
        permissions::define_role(self.db(), role, &[perm::PROJECTS_VIEW])
            .await
            .unwrap();
        self.user_with(name, role).await
    }

    /// The employee `id` was recorded present at `site` on `day`.
    pub async fn present(&self, id: i64, site: &WorkLocation, day: NaiveDate) {
        let mut record = wl_attendance::AttendanceRecord {
            employee_id: id,
            location_id: site.id,
            date: day,
            check_in: dubai(day, 7, 0),
            check_out: Some(dubai(day, 15, 0)),
            recorded_at: dubai(day, 15, 5),
            ..wl_attendance::AttendanceRecord::default()
        };
        record.save(self.db()).await.unwrap();
    }

    /// A project manager.
    pub async fn project_manager(&self, name: &str) -> User {
        self.user_with(name, perm::PROJECT_MANAGER_ROLE).await
    }

    /// A quantity surveyor.
    pub async fn qs(&self, name: &str) -> User {
        self.user_with(name, perm::QS_ROLE).await
    }

    /// The accountant, who issues invoices.
    pub async fn accountant(&self, name: &str) -> User {
        self.user_with(name, perm::ACCOUNTANT_ROLE).await
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

    /// A deduction line of `ipa` in `status`, inserted as a fixture.
    pub async fn deduction(
        &self,
        ipa: &Ipa,
        kind: DeductionKind,
        reference: &str,
        amount: i64,
        status: DeductionStatus,
    ) -> IpaDeduction {
        let mut line = IpaDeduction {
            project_id: ipa.project_id,
            ipa_id: ipa.id,
            kind,
            reference: reference.to_owned(),
            amount,
            status,
            recorded_by: 1,
            ..IpaDeduction::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        line.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        line
    }

    /// A retention part of `project` in `status`, inserted as a fixture.
    pub async fn retention(
        &self,
        project: &Project,
        part: RetentionPart,
        amount: i64,
        status: RetentionStatus,
    ) -> Retention {
        let mut row = Retention::new(project.id, part, amount);
        row.status = status;
        let mut tx = self.db().begin().await.unwrap();
        row.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        row
    }

    /// The retention part as it is stored now.
    pub async fn reload_part(&self, id: i64) -> Retention {
        Retention::find_or_404(self.db(), id).await.unwrap()
    }

    /// The part `which` of `project` as it is stored now.
    pub async fn part_of(&self, project: &Project, which: RetentionPart) -> Retention {
        Retention::query()
            .where_(Retention::PROJECT_ID.eq(project.id))
            .where_(Retention::PART.eq(which))
            .first(self.db())
            .await
            .unwrap()
            .expect("the retention part")
    }

    /// A guarantee of `project` in `status`, inserted as a fixture.
    pub async fn guarantee(
        &self,
        project: &Project,
        kind: GuaranteeKind,
        amount: i64,
        status: GuaranteeStatus,
        expires_on: Option<NaiveDate>,
    ) -> Guarantee {
        let mut row = Guarantee::new(project.id, kind, amount);
        row.bank = "Emirates NBD".into();
        row.status = status;
        row.expires_on = expires_on;
        let mut tx = self.db().begin().await.unwrap();
        row.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        row
    }

    /// The guarantee as it is stored now.
    pub async fn reload_guarantee(&self, id: i64) -> Guarantee {
        Guarantee::find_or_404(self.db(), id).await.unwrap()
    }

    /// The variation as it is stored now.
    pub async fn reload_variation(&self, id: i64) -> construction::Variation {
        construction::Variation::find_or_404(self.db(), id)
            .await
            .unwrap()
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

    /// The approval request waiting about `subject` (a variation).
    pub async fn pending_about(&self, subject: &Subject) -> wl_kernel::approvals::ApprovalRequest {
        wl_kernel::approvals::ApprovalRequest::pending_about(self.db(), subject)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("an approval request")
    }

    /// Whether an approval request waits about `subject`.
    pub async fn has_pending(&self, subject: &Subject) -> bool {
        !wl_kernel::approvals::ApprovalRequest::pending_about(self.db(), subject)
            .await
            .unwrap()
            .is_empty()
    }

    /// `approver` approves the request waiting about `subject`.
    pub async fn approve_about(&self, approver: &User, subject: &Subject) {
        let request = self.pending_about(subject).await;
        wl_kernel::approvals::approve(self.state(), request.id, approver, "")
            .await
            .unwrap();
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
    let bills = Bills::default();
    let costs = Costs::default();
    let blocked = Blocked::default();
    let terminated = Arc::new(Mutex::new(Vec::new()));
    let registrations = Registrations::default();
    let fakes = Fakes {
        registrations: registrations.clone(),
        blocked: blocked.clone(),
        staff: staff.clone(),
        tasks: tasks.clone(),
        notices: notices.clone(),
        invoices: invoices.clone(),
        bills: bills.clone(),
        costs: costs.clone(),
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
            perm::VARIATIONS_SUBMIT,
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
            perm::RETENTIONS_MANAGE,
            perm::VARIATIONS_RECORD,
        ],
    )
    .await
    .unwrap();
    // The accountant issues the invoice of a certified IPA, invoices the
    // retention, links a bond to it and registers the guarantees.
    permissions::define_role(
        app.db(),
        perm::ACCOUNTANT_ROLE,
        &[
            perm::INVOICES_ISSUE,
            perm::RETENTIONS_MANAGE,
            perm::GUARANTEES_MANAGE,
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
        bills,
        costs,
        blocked,
        registrations,
        terminated: Recorded(terminated),
        clock_days: std::sync::atomic::AtomicI64::new(0),
        made: std::sync::atomic::AtomicI64::new(0),
    }
}
