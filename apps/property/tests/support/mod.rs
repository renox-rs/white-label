//! Booting Property in a `TestApp` with auth, the kernel, and fakes for the
//! contracts it calls: the party directory, the document guard, the invoicer,
//! the cheque register, the deposit book, the payment recorder, the task
//! creator and the notifier. Each fake records what it is asked for.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use property::permissions as perm;
use property::{
    Building, Lease, LeaseActivated, LeaseAmended, LeaseCancelled, LeaseForceTerminated,
    LeaseNoticeWithdrawn, LeaseStatus, LeaseTerminated, NewBuilding, NewLease, NewUnit, Unit,
    UnitUse, create_building, create_unit,
};
use renox::auth::permissions::{self, Scope};
use renox::auth::{Auth, Permissions};
use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::block_guard::{
    ActionKind, BlockFuture, BlockGuard, BlockRefusal as PartyBlockRefusal, GuardOutcome,
    provide_block_guard,
};
use wl_kernel::cheques::{
    ChequeFuture, ChequeId, ChequeSchedule, ChequeStatus, ChequeSummary, Cheques, RegisterCheque,
    ReturnCheques, ReturnedCheques, provide_cheques,
};
use wl_kernel::deposits::{
    AdjustDeposit, ApplyDeposit, ApplyToInvoice, CancelSettlement, DeductionKind,
    DeductionLineView, DepositAdjusted, DepositApplied, DepositDeductions, DepositFuture,
    DepositTransferred, Deposits, HeldDeposit, PublishedDeductions, SettlementStarted, TopUpCharge,
    TransferDeposit, deposit_top_ups, provide_deposit_deductions, provide_deposits,
};
use wl_kernel::documents::{
    BlockEntry, BlockFailure, BlockMode, BlockRefusal, CheckOutcome, DocumentFuture, DocumentGuard,
    FlagFailure, InvalidReason, OverrideMode, RecordFlag, Validity, block_subjects, provide_guard,
    today,
};
use wl_kernel::invoicing::{
    BadDebtRecovered, CreditLine, CreditNoteId, InvoiceDraft, InvoiceFuture, InvoiceId,
    InvoiceStatus, InvoiceSummary, InvoiceWrittenOff, Invoicer, ReopenReason, WriteOffAnswer,
    invoicer, provide_invoicer,
};
use wl_kernel::notify::{
    MessageId, NoticeStatus, Notifier, NotifyFuture, NotifyRequest, provide_notifier,
};
use wl_kernel::parties::{
    PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary, provide_directory,
};
use wl_kernel::payments::{
    AllocationTarget, PaymentFuture, PaymentId, PaymentRecorder, PaymentSummary, RecordPayment,
    RefundId, RefundRequest, RefundSource, ReversalReason, payment_recorder,
    provide_payment_recorder,
};
use wl_kernel::tickets::{
    Assignee, OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::workforce::{
    EmployeeId, StaffProfile, Workforce, WorkforceFuture, provide_workforce,
};
use wl_kernel::{PartyId, Subject};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with one default legal entity and the optional `modules` on.
fn client(modules: &str) -> String {
    format!(
        r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = [{modules}]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true
"#
    )
}

/// Which of the contracts the real module answers instead of the fake.
#[derive(Debug, Clone, Copy, Default)]
pub struct Real {
    /// wl-invoicing answers the `Invoicer` (invoices, credit notes, overdue,
    /// reminders).
    pub invoicing: bool,
    /// wl-payments answers the payment recorder, the deposits and the deposit
    /// deductions (receipts, refunds, deposit settlement and its window).
    pub payments: bool,
    /// wl-documents answers whether the tenant's acceptance of changed terms
    /// is attached (the rest of the document guard stays the fake).
    pub documents: bool,
}

impl Real {
    /// The money modules: invoicing and payments.
    pub const MONEY: Real = Real {
        invoicing: true,
        payments: true,
        documents: false,
    };
    /// Payments only: refunds, deposits and their dispute window.
    pub const PAYMENTS: Real = Real {
        invoicing: false,
        payments: true,
        documents: false,
    };
    /// Invoicing only: invoices, overdue and reminders.
    pub const INVOICING: Real = Real {
        invoicing: true,
        payments: false,
        documents: false,
    };
    /// The documents module: the tenant's acceptance is a real document.
    pub const DOCUMENTS: Real = Real {
        invoicing: false,
        payments: false,
        documents: true,
    };

    /// The `modules.enabled` list of the client.
    fn enabled(self) -> String {
        // The client's manifest: payments needs invoicing and invoicing needs
        // parties, whichever answers the contract.
        let mut list = Vec::new();
        if self.invoicing || self.payments {
            list.push("\"parties\"");
            list.push("\"invoicing\"");
        }
        if self.payments {
            list.push("\"payments\"");
        }
        if self.documents {
            list.push("\"documents\"");
        }
        list.join(", ")
    }
}

/// Today plus `days` (negative for the past), in Asia/Dubai.
pub fn day(days: i64) -> NaiveDate {
    today() + Duration::days(days)
}

/// A date in 2027.
pub fn d(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, month, day).unwrap()
}

/// AED in fils.
pub const fn aed(dirhams: i64) -> i64 {
    dirhams * 100
}

// ---- the party directory -------------------------------------------------

/// The parties the tests know.
#[derive(Clone, Default)]
pub struct Parties(Arc<Mutex<BTreeMap<i64, PartySummary>>>);

impl Parties {
    /// Adds party `id`, a person or a company.
    pub fn add(&self, id: i64, name: &str, kind: PartyKind) {
        self.0.lock().unwrap().insert(
            id,
            PartySummary {
                id: PartyId(id),
                code: format!("P-{id:05}"),
                kind,
                display_name: name.to_owned(),
                status: PartyStatus::Active,
                display_block_reason: None,
            },
        );
    }
}

impl PartyDirectory for Parties {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&party.0).cloned()) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .values()
                .find(|p| p.code == code)
                .cloned())
        })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&party.0)
                .map(|p| p.display_name.clone()))
        })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&party.0).map(|p| p.status)) })
    }

    fn billing_identity<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<wl_kernel::parties::BillingIdentity>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&party.0)
                .map(|p| wl_kernel::parties::BillingIdentity {
                    party,
                    legal_name_en: p.display_name.clone(),
                    legal_name_ar: None,
                    trn: None,
                    billing_address: Some("Tower A, Dubai".to_owned()),
                    billing_email: None,
                    einvoice_recipient_id: None,
                }))
        })
    }
}

// ---- the document guard --------------------------------------------------

/// A document guard that asks the vertical's own providers whose documents a
/// transition needs, and holds the entries (`blocks`) and the accepted
/// documents of the test.
#[derive(Clone, Default)]
pub struct Docs {
    entries: Arc<Mutex<Vec<BlockEntry>>>,
    accepted: Arc<Mutex<Vec<(String, String)>>>,
    /// The kinds whose validity wl-documents answers.
    real_kinds: Arc<Mutex<Vec<String>>>,
}

impl Docs {
    /// A kind `kind` blocks the transition `guarded` (`lease.activate`).
    pub fn block(&self, kind: &str, guarded: &str) {
        self.block_if(kind, guarded, None);
    }

    /// Like [`Docs::block`], for the records whose condition field has one of
    /// `values`.
    pub fn block_if(&self, kind: &str, guarded: &str, condition: Option<(&str, &[&str])>) {
        self.entries.lock().unwrap().push(BlockEntry {
            kind: kind.to_owned(),
            guarded: guarded.to_owned(),
            mode: BlockMode::Block,
            override_mode: OverrideMode::Never,
            condition: condition.map(|(field, values)| {
                wl_kernel::documents::BlockCondition::new(field, values.iter().copied())
            }),
            locked: false,
        });
    }

    /// `subject` has an accepted document of `kind`.
    pub fn accept(&self, subject: &Subject, kind: &str) {
        self.accepted
            .lock()
            .unwrap()
            .push((subject.to_string(), kind.to_owned()));
    }

    /// The validity of documents of `kind` is the real module's.
    pub fn use_real_for(&self, kind: &str) {
        self.real_kinds.lock().unwrap().push(kind.to_owned());
    }

    fn is_real(&self, kind: &str) -> bool {
        self.real_kinds.lock().unwrap().iter().any(|k| k == kind)
    }

    fn has(&self, subject: &Subject, kind: &str) -> bool {
        self.accepted
            .lock()
            .unwrap()
            .iter()
            .any(|(s, k)| *s == subject.to_string() && k == kind)
    }
}

impl DocumentGuard for Docs {
    fn blocks_for<'a>(
        &'a self,
        _db: &'a Db,
        guarded: &'a str,
    ) -> DocumentFuture<'a, Vec<BlockEntry>> {
        Box::pin(async move {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.guarded == guarded)
                .cloned()
                .collect())
        })
    }

    fn is_valid<'a>(
        &'a self,
        db: &'a Db,
        subject: &'a Subject,
        kind: &'a str,
        on_date: NaiveDate,
    ) -> DocumentFuture<'a, Validity> {
        Box::pin(async move {
            if self.is_real(kind) {
                return wl_documents::guard::is_valid(db, subject, kind, on_date).await;
            }
            Ok(if self.has(subject, kind) {
                Validity::Valid {
                    document: None,
                    expires_on: None,
                }
            } else {
                Validity::Invalid {
                    reason: InvalidReason::Missing,
                    document: None,
                    expired_on: None,
                    pending: None,
                }
            })
        })
    }

    fn check_as<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        record_id: i64,
        _on_date: NaiveDate,
        _actor: Option<i64>,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, BlockRefusal>> {
        Box::pin(async move {
            let entries: Vec<BlockEntry> = self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.guarded == guarded)
                .cloned()
                .collect();
            let Some(provider) = block_subjects(guarded) else {
                return Ok(Ok(CheckOutcome::default()));
            };
            let mut failures = Vec::new();
            for entry in entries {
                for subject in provider.subjects(state, &entry, record_id).await? {
                    if self.has(&subject, &entry.kind) {
                        continue;
                    }
                    failures.push(BlockFailure {
                        message: format!("{} is missing for {subject}", entry.kind),
                        entry: entry.clone(),
                        subject,
                        validity: Validity::Invalid {
                            reason: InvalidReason::Missing,
                            document: None,
                            expired_on: None,
                            pending: None,
                        },
                    });
                }
            }
            if failures.is_empty() {
                Ok(Ok(CheckOutcome::default()))
            } else {
                Ok(Err(BlockRefusal {
                    guarded: guarded.to_owned(),
                    record_id,
                    failures,
                }))
            }
        })
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

// ---- the notifier and the tasks -------------------------------------------

/// Records what the notifier is asked for, can be told to fail on a type, and
/// answers the delivery status of the statutory notices the test sets.
#[derive(Clone, Default)]
pub struct Recording(
    Arc<Mutex<Vec<NotifyRequest>>>,
    Arc<Mutex<Option<String>>>,
    Arc<Mutex<BTreeMap<String, NoticeStatus>>>,
);

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

    /// From now on a request of type `key` fails (the notifier is down);
    /// `None` makes it work again.
    pub fn fail_on(&self, key: Option<&str>) {
        *self.1.lock().unwrap() = key.map(str::to_owned);
    }

    /// The statutory notice of type `key` about `subject` is `status` from
    /// now on (the provider's receipt, or the proof staff recorded).
    pub fn set_notice(&self, subject: &Subject, key: &str, status: NoticeStatus) {
        self.2
            .lock()
            .unwrap()
            .insert(format!("{subject}|{key}"), status);
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            if self.1.lock().unwrap().as_deref() == Some(request.type_key.as_str()) {
                return Err(Error::Status(
                    StatusCode::BAD_GATEWAY,
                    "the notifier is down".to_owned(),
                ));
            }
            self.0.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }

    fn notice_status<'a>(
        &'a self,
        _state: &'a AppState,
        subject: &'a Subject,
        type_key: &'a str,
    ) -> NotifyFuture<'a, NoticeStatus> {
        Box::pin(async move {
            Ok(self
                .2
                .lock()
                .unwrap()
                .get(&format!("{subject}|{type_key}"))
                .cloned()
                .unwrap_or(NoticeStatus::NotDelivered))
        })
    }
}

/// A task a rule opened.
#[derive(Debug, Clone)]
pub struct Task {
    /// The ticket's id.
    pub id: i64,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// The key it is found by while open.
    pub key: Option<String>,
    /// The record it is about.
    pub subject: Subject,
    /// The ticket category it was opened in.
    pub category: String,
    /// Who it is assigned to.
    pub assignee: Option<Assignee>,
    /// Whether it is still open.
    pub open: bool,
    /// How it was completed.
    pub note: String,
    /// When it is due, if it has a due time of its own.
    pub due_at: Option<renox::db::DateTime>,
}

/// The tasks the vertical opens.
#[derive(Clone, Default)]
pub struct Tasks(Arc<Mutex<Vec<Task>>>);

impl Tasks {
    /// Every task, oldest first.
    pub fn all(&self) -> Vec<Task> {
        self.0.lock().unwrap().clone()
    }

    /// The open tasks.
    pub fn open(&self) -> Vec<Task> {
        self.all().into_iter().filter(|t| t.open).collect()
    }

    /// The tasks whose key starts with `prefix`.
    pub fn with_key(&self, prefix: &str) -> Vec<Task> {
        self.all()
            .into_iter()
            .filter(|t| t.key.as_deref().is_some_and(|k| k.starts_with(prefix)))
            .collect()
    }
}

impl TaskCreator for Tasks {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut tasks = self.0.lock().unwrap();
            let id = i64::try_from(tasks.len()).unwrap_or(0) + 1;
            tasks.push(Task {
                id,
                title: ticket.title.clone(),
                description: ticket.description.clone(),
                key: ticket.source_key.clone(),
                subject: ticket.subject.clone(),
                category: ticket.category_key.clone(),
                assignee: ticket.assignee,
                open: true,
                note: String::new(),
                due_at: ticket.due_at,
            });
            Ok(Some(TicketRef {
                id: TicketId(id),
                number: format!("TKT-{id}"),
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
                .find(|t| t.open && t.key.as_deref() == Some(source_key))
                .map(|t| TicketRef {
                    id: TicketId(t.id),
                    number: format!("TKT-{}", t.id),
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
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|t| t.open && t.subject == *subject && t.category == category_key)
                .map(|t| TicketRef {
                    id: TicketId(t.id),
                    number: format!("TKT-{}", t.id),
                })
                .collect())
        })
    }

    fn resolve_from_source<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            let mut tasks = self.0.lock().unwrap();
            match tasks.iter_mut().find(|t| t.id == ticket.0 && t.open) {
                Some(task) => {
                    task.open = false;
                    task.note = note.to_owned();
                    Ok(true)
                }
                None => Ok(false),
            }
        })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            let mut tasks = self.0.lock().unwrap();
            match tasks.iter_mut().find(|t| t.id == ticket.0 && t.open) {
                Some(task) => {
                    task.open = false;
                    task.note = note.to_owned();
                    Ok(true)
                }
                None => Ok(false),
            }
        })
    }
}

// ---- the cheque register --------------------------------------------------

/// An in-memory cheque register.
#[derive(Clone, Default)]
pub struct ChequeBook(
    Arc<Mutex<Vec<ChequeSummary>>>,
    Arc<Mutex<Vec<ReturnCheques>>>,
    Arc<Mutex<Vec<ChequeId>>>,
);

impl ChequeBook {
    /// The requests to hand cheques back, oldest first.
    pub fn returns(&self) -> Vec<ReturnCheques> {
        self.1.lock().unwrap().clone()
    }

    /// Every cheque.
    pub fn all(&self) -> Vec<ChequeSummary> {
        self.0.lock().unwrap().clone()
    }

    /// The cheques held as evidence.
    pub fn evidence(&self) -> Vec<ChequeId> {
        self.2.lock().unwrap().clone()
    }

    /// Cheque `id`, as stored now.
    pub fn get(&self, id: ChequeId) -> ChequeSummary {
        self.all()
            .into_iter()
            .find(|c| c.id == id)
            .expect("a cheque")
    }

    /// Moves cheque `id` to `status` (the register's own transitions).
    pub fn set_status(&self, id: ChequeId, status: ChequeStatus) {
        for cheque in self.0.lock().unwrap().iter_mut() {
            if cheque.id == id {
                cheque.status = status;
            }
        }
    }

    /// Adds a cheque received with `agreement`, linked to `links`.
    pub fn receive(
        &self,
        party: PartyId,
        agreement: &Subject,
        number: &str,
        date: NaiveDate,
        amount: i64,
        links: &[Subject],
    ) -> ChequeId {
        let mut all = self.0.lock().unwrap();
        let id = ChequeId(i64::try_from(all.len()).unwrap_or(0) + 1);
        all.push(ChequeSummary {
            id,
            direction: wl_kernel::payments::Direction::In,
            party,
            legal_entity_id: 1,
            bank: "ENBD".to_owned(),
            number: number.to_owned(),
            date,
            amount,
            honoured_amount: 0,
            open_amount: amount,
            currency: "AED".to_owned(),
            presentations: 0,
            status: ChequeStatus::Received,
            agreement: Some(agreement.clone()),
            linked: links.to_vec(),
        });
        id
    }
}

impl Cheques for ChequeBook {
    fn register<'a>(
        &'a self,
        _state: &'a AppState,
        cheque: RegisterCheque,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, ChequeSummary> {
        Box::pin(async move {
            let id = self.receive(
                cheque.party,
                cheque.agreement.as_ref().expect("an agreement"),
                &cheque.number,
                cheque.date,
                cheque.amount,
                &cheque.linked,
            );
            Ok(self.get(id))
        })
    }

    fn register_schedule<'a>(
        &'a self,
        _state: &'a AppState,
        schedule: ChequeSchedule,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async move {
            let ids: Vec<ChequeId> = schedule
                .cheques
                .iter()
                .map(|c| {
                    self.receive(
                        schedule.party,
                        &schedule.agreement,
                        &c.number,
                        c.date,
                        c.amount,
                        &c.linked,
                    )
                })
                .collect();
            Ok(ids.into_iter().map(|id| self.get(id)).collect())
        })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: ChequeId) -> ChequeFuture<'a, Option<ChequeSummary>> {
        Box::pin(async move { Ok(self.all().into_iter().find(|c| c.id == id)) })
    }

    fn of_agreement<'a>(
        &'a self,
        _db: &'a Db,
        agreement: &'a Subject,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async move {
            Ok(self
                .all()
                .into_iter()
                .filter(|c| c.agreement.as_ref() == Some(agreement))
                .collect())
        })
    }

    fn open_bounced_amount<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> ChequeFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    fn return_undeposited<'a>(
        &'a self,
        _state: &'a AppState,
        request: ReturnCheques,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, Option<ReturnedCheques>> {
        Box::pin(async move {
            self.1.lock().unwrap().push(request.clone());
            let mut returned = Vec::new();
            let mut skipped = Vec::new();
            for id in &request.cheques {
                let status = self.get(*id).status;
                if matches!(status, ChequeStatus::Received | ChequeStatus::DueForDeposit) {
                    self.set_status(*id, ChequeStatus::Returned);
                    returned.push(*id);
                } else {
                    skipped.push(*id);
                }
            }
            if returned.is_empty() {
                return Ok(None);
            }
            Ok(Some(ReturnedCheques {
                receipt: format!("HOR-TEST-{}", returned[0]),
                returned,
                skipped,
            }))
        })
    }

    fn hold_as_evidence<'a>(
        &'a self,
        _state: &'a AppState,
        cheques: Vec<ChequeId>,
        _note: String,
    ) -> ChequeFuture<'a, Vec<ChequeId>> {
        Box::pin(async move {
            let mut marked = Vec::new();
            for id in cheques {
                let status = self.get(id).status;
                if matches!(status, ChequeStatus::Received | ChequeStatus::DueForDeposit) {
                    self.2.lock().unwrap().push(id);
                    marked.push(id);
                }
            }
            Ok(marked)
        })
    }

    fn relink<'a>(
        &'a self,
        _state: &'a AppState,
        cheque: ChequeId,
        from: Subject,
        to: Subject,
    ) -> ChequeFuture<'a, ()> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            let Some(found) = all.iter_mut().find(|c| c.id == cheque) else {
                return Err(Error::NotFound);
            };
            let Some(link) = found.linked.iter_mut().find(|s| **s == from) else {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "not linked".to_owned(),
                ));
            };
            *link = to;
            Ok(())
        })
    }
}

// ---- the deposit book -----------------------------------------------------

/// An in-memory deposit book.
#[derive(Clone, Default)]
pub struct DepositBook {
    held: Arc<Mutex<BTreeMap<String, HeldDeposit>>>,
    settled: Arc<Mutex<Vec<(CancelSettlement, SettlementStarted)>>>,
    started: Arc<Mutex<Vec<Subject>>>,
    legal: Arc<Mutex<Vec<(Subject, bool)>>>,
    adjusted: Arc<Mutex<Vec<AdjustDeposit>>>,
    settling: Arc<Mutex<BTreeMap<String, HeldDeposit>>>,
    published: Arc<Mutex<BTreeMap<String, PublishedDeductions>>>,
    applied: Arc<Mutex<Vec<(ApplyDeposit, i64)>>>,
    transfers: Arc<Mutex<Vec<(TransferDeposit, i64)>>>,
}

/// What the portal contract shows of the deductions the deposit book holds.
#[derive(Clone)]
pub struct PublishedBook(pub DepositBook);

impl DepositDeductions for PublishedBook {
    fn published_for<'a>(
        &'a self,
        _state: &'a AppState,
        party: PartyId,
    ) -> DepositFuture<'a, Vec<PublishedDeductions>> {
        Box::pin(async move {
            Ok(self
                .0
                .published
                .lock()
                .unwrap()
                .values()
                .filter(|shown| self.0.party_of(shown) == Some(party))
                .cloned()
                .collect())
        })
    }
}

impl DepositBook {
    /// The party of the settling deposit that `shown` is about, while it is being settled.
    fn party_of(&self, shown: &PublishedDeductions) -> Option<PartyId> {
        self.settling
            .lock()
            .unwrap()
            .get(&shown.source.to_string())
            .map(|held| held.party)
    }

    /// The deposit held for `source` is being settled: it holds `held` fils,
    /// and the deductions published `published_days` days from today (a
    /// negative number is in the past) carry the damage `lines`, whose window
    /// of 14 days ends 14 days after.
    pub fn settle_with(
        &self,
        source: &Subject,
        party: PartyId,
        held: i64,
        published_days: i64,
        lines: &[(&str, i64)],
    ) {
        let id = i64::try_from(self.settling.lock().unwrap().len()).unwrap_or(0) + 1;
        let deposit = wl_kernel::payments::DepositId(id);
        self.settling.lock().unwrap().insert(
            source.to_string(),
            HeldDeposit {
                deposit,
                party,
                required: held,
                held,
                currency: "AED".to_owned(),
            },
        );
        let published_on = day(published_days);
        self.published.lock().unwrap().insert(
            source.to_string(),
            PublishedDeductions {
                deposit,
                source: source.clone(),
                version: 1,
                kind: DeductionKind::Forfeiture,
                reason: Some("applied to deductions".to_owned()),
                currency: "AED".to_owned(),
                published_on,
                window_ends_on: Some(published_on + Duration::days(14)),
                lines: lines
                    .iter()
                    .enumerate()
                    .map(|(index, (description, amount))| DeductionLineView {
                        id: i64::try_from(index).unwrap_or(0) + 1,
                        description: (*description).to_owned(),
                        amount: *amount,
                        evidence: "photos".to_owned(),
                    })
                    .collect(),
                disputed_lines: Vec::new(),
                can_dispute: false,
            },
        );
    }

    /// The tenant disputes line `line` (from 1) of the published deductions.
    pub fn dispute_line(&self, source: &Subject, line: i64) {
        if let Some(shown) = self.published.lock().unwrap().get_mut(&source.to_string()) {
            shown.disputed_lines.push(line);
        }
    }

    /// The applications made, with the user who forfeited.
    pub fn applied(&self) -> Vec<(ApplyDeposit, i64)> {
        self.applied.lock().unwrap().clone()
    }

    /// Whether a deposit is still being settled for `source`.
    pub fn is_settling(&self, source: &Subject) -> bool {
        self.settling
            .lock()
            .unwrap()
            .contains_key(&source.to_string())
    }

    /// `amount` fils are held for `source`.
    pub fn hold(&self, source: &Subject, party: PartyId, amount: i64) {
        let mut held = self.held.lock().unwrap();
        let id = i64::try_from(held.len()).unwrap_or(0) + 1;
        held.insert(
            source.to_string(),
            HeldDeposit {
                deposit: wl_kernel::payments::DepositId(id),
                party,
                required: amount,
                held: amount,
                currency: "AED".to_owned(),
            },
        );
    }

    /// The settlements started.
    pub fn settled(&self) -> Vec<(CancelSettlement, SettlementStarted)> {
        self.settled.lock().unwrap().clone()
    }

    /// The adjustments asked for (a lease amendment or renewal with another
    /// deposit), oldest first.
    pub fn adjusted(&self) -> Vec<AdjustDeposit> {
        self.adjusted.lock().unwrap().clone()
    }

    /// The sources whose settlement started because the agreement ended.
    pub fn started(&self) -> Vec<Subject> {
        self.started.lock().unwrap().clone()
    }

    /// The legal case notes made: the source and whether a case is open.
    pub fn legal(&self) -> Vec<(Subject, bool)> {
        self.legal.lock().unwrap().clone()
    }

    /// The deposits handed to a new holder, with the user who did it.
    pub fn transferred(&self) -> Vec<(TransferDeposit, i64)> {
        self.transfers.lock().unwrap().clone()
    }

    /// Whether a deposit is still held (not settled, not transferred) for `source`.
    pub fn is_held(&self, source: &Subject) -> bool {
        self.held.lock().unwrap().contains_key(&source.to_string())
    }
}

impl Deposits for DepositBook {
    fn held_for<'a>(
        &'a self,
        _db: &'a Db,
        source: &'a Subject,
    ) -> DepositFuture<'a, Option<HeldDeposit>> {
        Box::pin(async move { Ok(self.held.lock().unwrap().get(&source.to_string()).cloned()) })
    }

    fn set_legal_case<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
        open: bool,
    ) -> DepositFuture<'a, bool> {
        Box::pin(async move {
            self.legal.lock().unwrap().push((source.clone(), open));
            Ok(true)
        })
    }

    fn transfer<'a>(
        &'a self,
        _state: &'a AppState,
        by: &'a AuthUser,
        request: TransferDeposit,
    ) -> DepositFuture<'a, Option<DepositTransferred>> {
        Box::pin(async move {
            if request.receipt.trim().is_empty() {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "the receipt is required".to_owned(),
                ));
            }
            let Some(held) = self
                .held
                .lock()
                .unwrap()
                .remove(&request.source.to_string())
            else {
                return Ok(None);
            };
            let moved = DepositTransferred {
                deposit: held.deposit,
                held: held.held,
                new_holder: request.new_holder,
            };
            self.transfers.lock().unwrap().push((request, by.id));
            Ok(Some(moved))
        })
    }

    fn in_settlement<'a>(&'a self, _db: &'a Db, source: &'a Subject) -> DepositFuture<'a, bool> {
        Box::pin(async move {
            Ok(self
                .settling
                .lock()
                .unwrap()
                .contains_key(&source.to_string()))
        })
    }

    fn start_settlement<'a>(
        &'a self,
        _state: &'a AppState,
        source: &'a Subject,
    ) -> DepositFuture<'a, Option<SettlementStarted>> {
        Box::pin(async move {
            let Some(held) = self.held.lock().unwrap().remove(&source.to_string()) else {
                return Ok(None);
            };
            self.started.lock().unwrap().push(source.clone());
            Ok(Some(SettlementStarted {
                deposit: held.deposit,
                held: held.held,
                forfeit: 0,
                version: 0,
            }))
        })
    }

    fn adjust<'a>(
        &'a self,
        state: &'a AppState,
        request: AdjustDeposit,
        _by: Option<i64>,
    ) -> DepositFuture<'a, Option<DepositAdjusted>> {
        Box::pin(async move {
            let held = self
                .held
                .lock()
                .unwrap()
                .get(&request.source.to_string())
                .cloned();
            let Some(held) = held else {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "no deposit is held".to_owned(),
                ));
            };
            if held.held == request.new_amount {
                return Ok(None);
            }
            self.adjusted.lock().unwrap().push(request.clone());
            let mut top_up_charge = None;
            let mut part_refund = None;
            if request.new_amount > held.held {
                top_up_charge = deposit_top_ups(state)
                    .request(
                        state,
                        TopUpCharge {
                            deposit: held.deposit,
                            party: held.party,
                            legal_entity_id: 1,
                            source: request.source.clone(),
                            source_version: request.source_version,
                            amount: request.new_amount - held.held,
                            currency: "AED".to_owned(),
                            due: request.effective_on,
                        },
                    )
                    .await?;
            } else {
                part_refund = Some(
                    payment_recorder(state)
                        .request_refund(
                            state,
                            RefundRequest {
                                source: RefundSource::Deposit(held.deposit),
                                party: held.party,
                                legal_entity_id: 1,
                                amount: held.held - request.new_amount,
                                currency: "AED".to_owned(),
                                method: None,
                                gateway_txn: None,
                                reason: "Deposit lowered".to_owned(),
                            },
                            None,
                        )
                        .await?,
                );
            }
            Ok(Some(DepositAdjusted {
                deposit: held.deposit,
                source: request.source,
                party: held.party,
                source_version: request.source_version,
                old_amount: held.held,
                new_amount: request.new_amount,
                effective_on: request.effective_on,
                top_up_charge,
                part_refund,
            }))
        })
    }

    fn apply_to_deductions<'a>(
        &'a self,
        state: &'a AppState,
        by: &'a AuthUser,
        request: ApplyDeposit,
    ) -> DepositFuture<'a, DepositApplied> {
        Box::pin(async move {
            let key = request.source.to_string();
            let refused =
                |message: &str| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned());
            let held = self.settling.lock().unwrap().get(&key).cloned();
            let Some(held) = held else {
                return Err(refused("no deposit is being settled"));
            };
            let damages: i64 = self
                .published
                .lock()
                .unwrap()
                .get(&key)
                .map_or(0, |shown| shown.lines.iter().map(|l| l.amount).sum());
            let unpaid: i64 = request.rent.iter().map(|r| r.amount).sum();
            if unpaid + damages < held.held {
                return Err(refused("the total deductions are below the deposit"));
            }
            let mut left = held.held;
            let mut shares = Vec::new();
            for invoice in &request.rent {
                let take = left.min(invoice.amount);
                if take > 0 {
                    shares.push(ApplyToInvoice {
                        invoice: invoice.invoice,
                        amount: take,
                    });
                    left -= take;
                }
            }
            for share in &shares {
                invoicer(state)
                    .allocate(
                        state,
                        share.invoice,
                        Subject::new("payments.payment", 900 + share.invoice.0),
                        share.amount,
                    )
                    .await?;
            }
            self.settling.lock().unwrap().remove(&key);
            self.applied.lock().unwrap().push((request, by.id));
            let to_invoices: i64 = shares.iter().map(|s| s.amount).sum();
            Ok(DepositApplied {
                deposit: held.deposit,
                party: held.party,
                legal_entity_id: 1,
                held: held.held,
                to_invoices,
                shares,
                kept: held.held - to_invoices,
                version: 1,
            })
        })
    }

    fn settle_cancelled<'a>(
        &'a self,
        _state: &'a AppState,
        request: CancelSettlement,
        _by: Option<i64>,
    ) -> DepositFuture<'a, Option<SettlementStarted>> {
        Box::pin(async move {
            let Some(held) = self
                .held
                .lock()
                .unwrap()
                .remove(&request.source.to_string())
            else {
                return Ok(None);
            };
            let started = SettlementStarted {
                deposit: held.deposit,
                held: held.held,
                forfeit: request.forfeit,
                version: 1,
            };
            self.settled
                .lock()
                .unwrap()
                .push((request, started.clone()));
            Ok(Some(started))
        })
    }
}

// ---- the invoicer -----------------------------------------------------------

/// An invoice of the fake invoicer.
#[derive(Debug, Clone)]
pub struct FakeInvoice {
    /// What it was drafted from.
    pub draft: InvoiceDraft,
    /// Its summary.
    pub summary: InvoiceSummary,
    /// What is allocated to it: the source and the fils.
    pub allocations: Vec<(Subject, i64)>,
}

/// A partial credit note of the fake invoicer.
#[derive(Debug, Clone)]
pub struct Credit {
    /// The invoice credited.
    pub invoice: i64,
    /// The lines and the net credited on each.
    pub lines: Vec<CreditLine>,
    /// Why.
    pub reason: String,
}

/// An in-memory invoicer.
#[derive(Clone, Default)]
pub struct InvoiceDesk(
    Arc<Mutex<BTreeMap<i64, FakeInvoice>>>,
    Arc<Mutex<Vec<Credit>>>,
    Arc<Mutex<Vec<WriteOffAsk>>>,
);

/// A write-off the fake invoicer was asked for; it waits for an approval.
#[derive(Debug, Clone)]
pub struct WriteOffAsk {
    /// The invoice.
    pub invoice: i64,
    /// The user who asked.
    pub by: i64,
    /// The reason code.
    pub reason: String,
}

impl InvoiceDesk {
    /// The write-offs asked for, oldest first.
    pub fn write_offs(&self) -> Vec<WriteOffAsk> {
        self.2.lock().unwrap().clone()
    }

    /// The approver says yes to the write-off of invoice `id`: it is
    /// `written_off` and `InvoiceWrittenOff` is emitted.
    pub async fn approve_write_off(&self, state: &AppState, id: i64, approver: i64) {
        let found = {
            let mut all = self.0.lock().unwrap();
            all.get_mut(&id).map(|invoice| {
                invoice.summary.status = InvoiceStatus::WrittenOff;
                (
                    invoice.summary.party,
                    invoice.draft.source.clone(),
                    invoice.summary.balance,
                )
            })
        };
        let (party, source, amount) = found.expect("the invoice exists");
        let reason_code = self
            .write_offs()
            .iter()
            .rev()
            .find(|ask| ask.invoice == id)
            .map_or_else(|| "insolvent".to_owned(), |ask| ask.reason.clone());
        state
            .emit(InvoiceWrittenOff {
                invoice: InvoiceId(id),
                party,
                source,
                amount,
                reason_code,
                approver: Some(approver),
            })
            .await
            .unwrap();
    }

    /// `amount` fils are recovered on the written-off invoice `id` on `on`:
    /// `BadDebtRecovered` is emitted.
    pub async fn recover(&self, state: &AppState, id: i64, amount: i64, on: NaiveDate) {
        let party = {
            let mut all = self.0.lock().unwrap();
            let invoice = all.get_mut(&id).expect("the invoice exists");
            invoice.summary.balance -= amount;
            if invoice.summary.balance <= 0 {
                invoice.summary.status = InvoiceStatus::Paid;
            }
            invoice.summary.party
        };
        state
            .emit(BadDebtRecovered {
                invoice: InvoiceId(id),
                party,
                source: Subject::new("payments.payment", 700 + id * 1_000 + amount / 100),
                amount,
                relief_claimed: false,
                recovered_on: on,
            })
            .await
            .unwrap();
    }

    /// The partial credit notes made, oldest first.
    pub fn credits(&self) -> Vec<Credit> {
        self.1.lock().unwrap().clone()
    }

    /// The net credited on invoice `id` in all, in fils.
    pub fn credited(&self, id: i64) -> i64 {
        self.credits()
            .iter()
            .filter(|c| c.invoice == id)
            .flat_map(|c| c.lines.iter().map(|l| l.net))
            .sum()
    }

    /// Every invoice.
    pub fn all(&self) -> Vec<FakeInvoice> {
        self.0.lock().unwrap().values().cloned().collect()
    }

    /// Invoice `id`, if it still exists.
    pub fn get(&self, id: i64) -> Option<FakeInvoice> {
        self.0.lock().unwrap().get(&id).cloned()
    }

    /// Issues draft `id`.
    pub fn issue(&self, id: i64) {
        if let Some(invoice) = self.0.lock().unwrap().get_mut(&id) {
            invoice.summary.status = InvoiceStatus::Issued;
        }
    }

    /// Invoice `id` is disputed (or not).
    pub fn dispute(&self, id: i64, disputed: bool) {
        if let Some(invoice) = self.0.lock().unwrap().get_mut(&id) {
            invoice.summary.disputed = disputed;
        }
    }

    /// Invoice `id` is in `status` (written off, credited...).
    pub fn set_status(&self, id: i64, status: InvoiceStatus) {
        if let Some(invoice) = self.0.lock().unwrap().get_mut(&id) {
            invoice.summary.status = status;
        }
    }

    /// `amount` fils of `source` are allocated to invoice `id`.
    pub fn pay(&self, id: i64, source: &Subject, amount: i64) {
        if let Some(invoice) = self.0.lock().unwrap().get_mut(&id) {
            invoice.allocations.push((source.clone(), amount));
            invoice.summary.balance -= amount;
            invoice.summary.status = if invoice.summary.balance <= 0 {
                InvoiceStatus::Paid
            } else {
                InvoiceStatus::PartiallyPaid
            };
        }
    }
}

impl Invoicer for InvoiceDesk {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            let id = i64::try_from(all.len()).unwrap_or(0) + 1;
            let gross: i64 = draft.lines.iter().map(|l| l.gross()).sum();
            let summary = InvoiceSummary {
                id: InvoiceId(id),
                number: None,
                status: InvoiceStatus::Draft,
                party: draft.party,
                legal_entity_id: draft.legal_entity_id,
                currency: draft.currency.clone(),
                gross_total: gross,
                balance: gross,
                due_date: draft.due_date,
                disputed: false,
                chasing_paused_until: None,
            };
            all.insert(
                id,
                FakeInvoice {
                    draft,
                    summary,
                    allocations: Vec::new(),
                },
            );
            Ok(Some(InvoiceId(id)))
        })
    }

    fn discard_draft<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, bool> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            let is_draft = all
                .get(&id.0)
                .is_some_and(|i| i.summary.status == InvoiceStatus::Draft);
            if is_draft {
                all.remove(&id.0);
            }
            Ok(is_draft)
        })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async move { Ok(self.get(id.0).map(|i| i.summary)) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        source: Subject,
        amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async move {
            self.pay(id.0, &source, amount);
            Ok(())
        })
    }

    fn credit_partly<'a>(
        &'a self,
        _state: &'a AppState,
        invoice: InvoiceId,
        lines: Vec<CreditLine>,
        reason: String,
        _by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move {
            if self.get(invoice.0).is_none() {
                return Err(Error::NotFound);
            }
            let mut credits = self.1.lock().unwrap();
            credits.push(Credit {
                invoice: invoice.0,
                lines,
                reason,
            });
            Ok(CreditNoteId(i64::try_from(credits.len()).unwrap_or(0)))
        })
    }

    fn unallocate_all<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        _reason: ReopenReason,
    ) -> InvoiceFuture<'a, Vec<(Subject, i64)>> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            let Some(invoice) = all.get_mut(&id.0) else {
                return Ok(Vec::new());
            };
            let taken = std::mem::take(&mut invoice.allocations);
            invoice.summary.balance = invoice.summary.gross_total;
            invoice.summary.status = InvoiceStatus::Issued;
            Ok(taken)
        })
    }

    fn write_off<'a>(
        &'a self,
        _state: &'a AppState,
        by: &'a AuthUser,
        id: InvoiceId,
        reason_code: &'a str,
        _reason_text: Option<&'a str>,
    ) -> InvoiceFuture<'a, WriteOffAnswer> {
        Box::pin(async move {
            let open = self
                .get(id.0)
                .is_some_and(|i| i.summary.balance > 0 && !i.summary.disputed);
            if !open {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "the invoice cannot be written off".to_owned(),
                ));
            }
            let mut asked = self.2.lock().unwrap();
            asked.push(WriteOffAsk {
                invoice: id.0,
                by: by.id,
                reason: reason_code.to_owned(),
            });
            Ok(WriteOffAnswer::Pending {
                request: i64::try_from(asked.len()).unwrap_or(0),
            })
        })
    }

    fn credit_in_full<'a>(
        &'a self,
        _state: &'a AppState,
        invoice: InvoiceId,
        _reason: String,
        _by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            let Some(found) = all.get_mut(&invoice.0) else {
                return Err(Error::NotFound);
            };
            if !found.allocations.is_empty() {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "something is allocated".to_owned(),
                ));
            }
            found.summary.status = InvoiceStatus::Credited;
            found.summary.balance = 0;
            Ok(CreditNoteId(invoice.0))
        })
    }
}

// ---- the payment recorder -----------------------------------------------------

/// A payment recorder that takes refund requests and records the payments the
/// vertical makes (an owner payout).
#[derive(Clone, Default)]
pub struct Refunds(
    Arc<Mutex<Vec<RefundRequest>>>,
    Arc<Mutex<PaymentLog>>,
    Arc<Mutex<bool>>,
);

/// The payments recorded, each with the refund it settles.
type PaymentLog = Vec<(RecordPayment, Option<i64>)>;

impl Refunds {
    /// The refunds asked for.
    pub fn asked(&self) -> Vec<RefundRequest> {
        self.0.lock().unwrap().clone()
    }

    /// From now on the recorder refuses the refunds it is asked for (`true`),
    /// or takes them again.
    pub fn refuse(&self, refuse: bool) {
        *self.2.lock().unwrap() = refuse;
    }

    /// The payments recorded, oldest first, with the user who recorded them.
    pub fn payments(&self) -> Vec<(RecordPayment, Option<i64>)> {
        self.1.lock().unwrap().clone()
    }
}

/// The 422 of the fake's unsupported calls.
fn unsupported() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        "the test recorder takes refund requests only".to_owned(),
    )
}

impl PaymentRecorder for Refunds {
    fn record<'a>(
        &'a self,
        _state: &'a AppState,
        payment: RecordPayment,
        by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        Box::pin(async move {
            let mut all = self.1.lock().unwrap();
            all.push((payment.clone(), by));
            let id = i64::try_from(all.len()).unwrap_or(0);
            Ok(PaymentSummary {
                id: PaymentId(id),
                number: Some(format!("PV-{id:04}")),
                party: payment.party,
                legal_entity_id: payment.legal_entity_id,
                direction: payment.direction,
                method: payment.method,
                amount: payment.amount,
                allocated: 0,
                unallocated: payment.amount,
                currency: payment.currency.clone(),
                status: wl_kernel::payments::PaymentStatus::Received,
                date: payment.date.unwrap_or_else(today),
            })
        })
    }

    /// As the real recorder: an invoice target takes the money through the
    /// invoicer's own contract.
    fn allocate<'a>(
        &'a self,
        state: &'a AppState,
        payment: PaymentId,
        target: AllocationTarget,
        amount: i64,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async move {
            match target {
                AllocationTarget::Invoice(invoice) => {
                    invoicer(state)
                        .allocate(state, invoice, payment.subject(), amount)
                        .await
                }
                _ => Err(unsupported()),
            }
        })
    }

    fn reverse<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _reason: ReversalReason,
        _note: Option<String>,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(unsupported()) })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: PaymentId,
    ) -> PaymentFuture<'a, Option<PaymentSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn credit_balance<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> PaymentFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    fn payments_for<'a>(
        &'a self,
        _db: &'a Db,
        _prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn request_refund<'a>(
        &'a self,
        _state: &'a AppState,
        request: RefundRequest,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        Box::pin(async move {
            if *self.2.lock().unwrap() {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "the refund is refused".to_owned(),
                ));
            }
            let mut all = self.0.lock().unwrap();
            all.push(request);
            Ok(RefundId(i64::try_from(all.len()).unwrap_or(0)))
        })
    }
}

// ---- the block guard ------------------------------------------------------------

/// A block guard that refuses the parties of the test (`PTY.ACL.03`) and
/// remembers what it was asked.
#[derive(Clone, Default)]
pub struct Blocks(Arc<Mutex<Vec<i64>>>, Arc<Mutex<Vec<(i64, ActionKind)>>>);

impl Blocks {
    /// The party is on hold from now on.
    pub fn block(&self, party: PartyId) {
        self.0.lock().unwrap().push(party.0);
    }

    /// The party is free again.
    pub fn free(&self, party: PartyId) {
        self.0.lock().unwrap().retain(|id| *id != party.0);
    }

    /// What the guard was asked, oldest first.
    pub fn asked(&self) -> Vec<(i64, ActionKind)> {
        self.1.lock().unwrap().clone()
    }
}

impl BlockGuard for Blocks {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        _viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome> {
        Box::pin(async move {
            self.1.lock().unwrap().push((party.0, kind));
            let blocked = kind.is_guarded() && self.0.lock().unwrap().contains(&party.0);
            Ok(if blocked {
                GuardOutcome::Blocked(PartyBlockRefusal::neutral(party, kind, "Compliance"))
            } else {
                GuardOutcome::Allowed
            })
        })
    }
}

// ---- the workforce ----------------------------------------------------------------

/// A workforce that knows the managers of the users the test sets.
#[derive(Clone, Default)]
pub struct Staff(Arc<Mutex<BTreeMap<i64, Vec<i64>>>>);

impl Staff {
    /// The user `user` is an employee whose managers are the users `managers`.
    pub fn set_managers(&self, user: i64, managers: &[i64]) {
        self.0.lock().unwrap().insert(user, managers.to_vec());
    }
}

impl Workforce for Staff {
    fn employee_of_user<'a>(
        &'a self,
        _db: &'a Db,
        user_id: i64,
    ) -> WorkforceFuture<'a, Option<EmployeeId>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .contains_key(&user_id)
                .then_some(EmployeeId(user_id)))
        })
    }

    fn staff<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        Box::pin(async move {
            let managers = self.0.lock().unwrap().get(&employee.0).cloned();
            Ok(managers.map(|managers| StaffProfile {
                employee,
                party: PartyId(employee.0),
                user_id: Some(employee.0),
                role: "property_manager".to_owned(),
                scopes: Vec::new(),
                legal_entity_id: None,
                listed: true,
                on_leave: false,
                returns_on: None,
                managers,
            }))
        })
    }
}

// ---- the booted app -----------------------------------------------------------

/// An event log.
#[derive(Clone)]
pub struct Recorded<E>(pub Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    /// What was recorded.
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

/// Provides the fakes.
struct Fakes {
    real: Real,
    /// wl-payments answers the cheque register too (needs `real.payments`).
    real_cheques: bool,
    activated: Recorded<LeaseActivated>,
    cancelled: Recorded<LeaseCancelled>,
    withdrawn: Recorded<LeaseNoticeWithdrawn>,
    terminated: Recorded<LeaseTerminated>,
    amended: Recorded<LeaseAmended>,
    forced: Recorded<LeaseForceTerminated>,
    adjusted: Recorded<DepositAdjusted>,
    parties: Parties,
    docs: Docs,
    notices: Recording,
    tasks: Tasks,
    cheques: ChequeBook,
    deposits: DepositBook,
    invoices: InvoiceDesk,
    refunds: Refunds,
    blocks: Blocks,
    staff: Staff,
}

/// What the test route that holds a deposit takes.
#[derive(serde::Deserialize)]
struct HoldForm {
    party: i64,
    entity: i64,
    source_type: String,
    source_id: i64,
    amount: i64,
    #[serde(default)]
    payment: Option<i64>,
}

/// `POST /test/deposits`: holds a deposit through the real wl-payments (the
/// function needs the user of a request).
async fn hold_route(
    State(state): State<AppState>,
    user: AuthUser,
    renox::axum::extract::Form(form): renox::axum::extract::Form<HoldForm>,
) -> Result<String> {
    let request = wl_payments::deposit::HoldDeposit {
        party: PartyId(form.party),
        legal_entity_id: form.entity,
        source: Subject::new(&form.source_type, form.source_id),
        amount: form.amount,
        currency: "AED".to_owned(),
        received_payment: form.payment.map(PaymentId),
    };
    let deposit = wl_payments::deposit::hold(&state, &user, &request)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    Ok(deposit.id.to_string())
}

/// What the test route that approves or pays a refund takes.
#[derive(serde::Deserialize, Default)]
struct RefundForm {
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    reference: Option<String>,
}

/// `POST /test/refunds/{id}/{action}`: `approve` or `pay` a refund of the real
/// wl-payments (the functions need the user of a request).
async fn refund_route(
    State(state): State<AppState>,
    user: AuthUser,
    renox::axum::extract::Path((id, action)): renox::axum::extract::Path<(i64, String)>,
    renox::axum::extract::Form(form): renox::axum::extract::Form<RefundForm>,
) -> Result<String> {
    let refund = wl_payments::Refund::find_or_404(&state.db, id).await?;
    let done = match action.as_str() {
        "approve" => wl_payments::refund::approve(&state, &user, &refund).await,
        "pay" => {
            let code = form.method.unwrap_or_default();
            let method = wl_kernel::payments::PaymentMethod::ALL
                .iter()
                .copied()
                .find(|m| m.as_str() == code)
                .ok_or(Error::NotFound)?;
            let reference = form.reference.unwrap_or_default();
            wl_payments::refund::pay(&state, &user, &refund, method, &reference).await
        }
        _ => return Err(Error::NotFound),
    };
    done.map(|refund| refund.id.to_string())
        .map_err(renox::state_machine::TransitionError::into_error)
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn routes(&self) -> Routes {
        if self.real.payments {
            Routes::new()
                .post("/test/deposits", hold_route)
                .post("/test/refunds/{id}/{action}", refund_route)
        } else {
            Routes::new()
        }
    }

    fn register(&self, app: &mut Registry) {
        provide_directory(app, self.parties.clone());
        provide_guard(app, self.docs.clone());
        provide_notifier(app, self.notices.clone());
        provide_task_creator(app, self.tasks.clone());
        if !self.real_cheques {
            provide_cheques(app, self.cheques.clone());
        }
        if !self.real.payments {
            provide_deposits(app, self.deposits.clone());
            provide_deposit_deductions(app, PublishedBook(self.deposits.clone()));
            provide_payment_recorder(app, self.refunds.clone());
        }
        if !self.real.invoicing {
            provide_invoicer(app, self.invoices.clone());
        }
        provide_block_guard(app, self.blocks.clone());
        provide_workforce(app, self.staff.clone());
        record(app, &self.activated);
        record(app, &self.cancelled);
        record(app, &self.withdrawn);
        record(app, &self.terminated);
        record(app, &self.amended);
        record(app, &self.forced);
        record(app, &self.adjusted);
    }
}

/// The app under test and its fakes.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The parties.
    pub parties: Parties,
    /// The document guard.
    pub docs: Docs,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// The tasks opened.
    pub tasks: Tasks,
    /// The cheque register.
    pub cheques: ChequeBook,
    /// The deposit book.
    pub deposits: DepositBook,
    /// The invoicer.
    pub invoices: InvoiceDesk,
    /// The refunds asked for.
    pub refunds: Refunds,
    /// `LeaseActivated`.
    pub activated: Recorded<LeaseActivated>,
    /// `LeaseCancelled`.
    pub cancelled: Recorded<LeaseCancelled>,
    /// `LeaseNoticeWithdrawn`.
    pub withdrawn: Recorded<LeaseNoticeWithdrawn>,
    /// `LeaseTerminated`.
    pub terminated: Recorded<LeaseTerminated>,
    /// The `LeaseAmended` events.
    pub amended: Recorded<LeaseAmended>,
    /// The `LeaseForceTerminated` events.
    pub forced: Recorded<LeaseForceTerminated>,
    /// The `DepositAdjusted` events of the deposit book in use.
    pub deposit_adjusted: Recorded<DepositAdjusted>,
    /// The parties on hold.
    pub blocks: Blocks,
    /// The managers of the staff.
    pub staff: Staff,
}

/// The tenant of the tests: a person.
pub const TENANT: PartyId = PartyId(7);

/// A company tenant.
pub const COMPANY: PartyId = PartyId(8);

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

    /// A user holding `role` in the building `building_id`.
    pub async fn user_in(&self, name: &str, role: &str, building_id: i64) -> User {
        let user = self.user(name).await;
        user.assign_role_in(self.db(), role, &Building::scope_of(building_id))
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

    /// A building `TWR-A`.
    pub async fn building(&self) -> Building {
        self.building_coded("TWR-A").await
    }

    /// A building with `code`.
    pub async fn building_coded(&self, code: &str) -> Building {
        create_building(
            self.db(),
            NewBuilding {
                code: code.to_owned(),
                name_en: format!("Tower {code}"),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    /// A residential unit `number` in `building`.
    pub async fn unit(&self, building: &Building, number: &str) -> Unit {
        self.unit_of(building, number, UnitUse::Residential).await
    }

    /// A unit `number` in `building` with `usage`.
    pub async fn unit_of(&self, building: &Building, number: &str, usage: UnitUse) -> Unit {
        create_unit(
            self.db(),
            NewUnit {
                building_id: building.id,
                number: number.to_owned(),
                usage,
                asking_rent: Some(aed(100_000)),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    /// A draft lease of `unit` for [`TENANT`]: one year starting `start_in`
    /// days from today, AED 100,000 a year in 4 instalments, a deposit of
    /// AED 5,000.
    pub async fn lease(&self, unit: &Unit, start_in: i64) -> Lease {
        let new = NewLease::new(
            unit.id,
            TENANT,
            day(start_in),
            day(start_in + 364),
            aed(100_000),
            4,
        )
        .deposit(aed(5_000));
        property::leases::create_draft(self.state(), None, new)
            .await
            .unwrap()
    }

    /// `lease`, as stored now.
    pub async fn reload(&self, lease: &Lease) -> Lease {
        Lease::find_or_404(self.db(), lease.id).await.unwrap()
    }

    /// The unit `unit`, as stored now.
    pub async fn unit_now(&self, unit: &Unit) -> Unit {
        Unit::find_or_404(self.db(), unit.id).await.unwrap()
    }

    /// `POST path` as `user`, the way a sheet posts it (htmx).
    pub async fn post(
        &self,
        user: &User,
        path: &str,
        pairs: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app.htmx().post(path, pairs).await
    }

    /// `POST path` as `user`, as a browser form posts it (a redirect answers).
    pub async fn post_plain(
        &self,
        user: &User,
        path: &str,
        pairs: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app.post(path, pairs).await
    }

    /// Submits `lease` as `user`.
    pub async fn submit(&self, user: &User, lease: &Lease) -> renox::testing::TestResponse {
        self.post(user, &format!("/property/leases/{}/submit", lease.id), &[])
            .await
    }

    /// `user` waives the deposit of `lease`.
    pub async fn waive(&self, user: &User, lease: &Lease) {
        self.post(
            user,
            &format!("/property/leases/{}/waive-deposit", lease.id),
            &[],
        )
        .await
        .assert_ok();
    }

    /// Activates `lease` as `user`.
    pub async fn activate(&self, user: &User, lease: &Lease) -> renox::testing::TestResponse {
        self.post(
            user,
            &format!("/property/leases/{}/activate", lease.id),
            &[],
        )
        .await
    }

    /// A lease of `unit` submitted by `agent`: `pending_signature`.
    pub async fn pending(&self, agent: &User, unit: &Unit, start_in: i64) -> Lease {
        let lease = self.lease(unit, start_in).await;
        self.submit(agent, &lease).await.assert_ok();
        self.reload(&lease).await
    }

    /// The status of `lease`.
    pub async fn status(&self, lease: &Lease) -> LeaseStatus {
        self.reload(lease).await.status
    }

    /// The tenant's signed acceptance of changed terms is attached to the
    /// draft version `lease` (`PRP.ACL.17`).
    pub fn accept(&self, lease: &Lease) {
        self.docs
            .accept(&lease.subject(), property::terms::TENANT_ACCEPTANCE_KIND);
    }

    /// Writes columns of `lease` directly: a fixture's date.
    pub async fn set(&self, lease: &Lease, columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))]) {
        Lease::query()
            .where_(Lease::ID.eq(lease.id))
            .update(self.db(), columns)
            .await
            .unwrap();
    }
}

/// The roles of the tests: the property manager, the leasing agent and the
/// accountant, with the permissions the host gives them.
pub const MANAGER: &str = perm::PROPERTY_MANAGER_ROLE;
/// The leasing agent.
pub const AGENT: &str = perm::LEASING_AGENT_ROLE;
/// The accountant.
pub const ACCOUNTANT: &str = perm::ACCOUNTANT_ROLE;

/// Property booted with auth, the kernel and the fakes, and the three roles.
pub async fn boot() -> Booted {
    boot_with(Real::default()).await
}

/// Like [`boot`], with the modules of `real` answering their contracts instead
/// of the fakes (the cheque register stays the fake one).
pub async fn boot_with(real: Real) -> Booted {
    boot_custom(
        real,
        wl_invoicing::WlInvoicing::new().auto_issue_source("property.lease"),
        wl_payments::WlPayments::new(),
        false,
    )
    .await
}

/// The money modules (wl-invoicing set up as `invoicing`, which should
/// issue the lease source at once as the Property client does) with
/// wl-payments answering the cheque register too: the cash-flow forecast
/// reads real cheques linked to real charges.
pub async fn boot_money(invoicing: wl_invoicing::WlInvoicing) -> Booted {
    boot_money_with(invoicing, wl_payments::WlPayments::new()).await
}

/// [`boot_money`] with wl-payments set up as `payments` (the deductions
/// dispute of the Property client is added to it).
pub async fn boot_money_with(
    invoicing: wl_invoicing::WlInvoicing,
    payments: wl_payments::WlPayments,
) -> Booted {
    boot_custom(Real::MONEY, invoicing, payments, true).await
}

/// Like [`boot_with`], with wl-invoicing set up as `invoicing` (used when
/// `real.invoicing`), wl-payments as `payments` (used when `real.payments`)
/// and, with `real_cheques`, the cheques of wl-payments.
async fn boot_custom(
    real: Real,
    invoicing: wl_invoicing::WlInvoicing,
    payments: wl_payments::WlPayments,
    real_cheques: bool,
) -> Booted {
    assert!(
        !real_cheques || real.payments,
        "real cheques need wl-payments"
    );
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let fakes = Fakes {
        real,
        real_cheques,
        activated: Recorded::default(),
        cancelled: Recorded::default(),
        withdrawn: Recorded::default(),
        terminated: Recorded::default(),
        amended: Recorded::default(),
        forced: Recorded::default(),
        adjusted: Recorded::default(),
        parties: Parties::default(),
        docs: Docs::default(),
        notices: Recording::default(),
        tasks: Tasks::default(),
        cheques: ChequeBook::default(),
        deposits: DepositBook::default(),
        invoices: InvoiceDesk::default(),
        refunds: Refunds::default(),
        blocks: Blocks::default(),
        staff: Staff::default(),
    };
    let logs = (
        fakes.activated.clone(),
        fakes.cancelled.clone(),
        fakes.withdrawn.clone(),
        fakes.terminated.clone(),
        fakes.amended.clone(),
        fakes.forced.clone(),
        fakes.adjusted.clone(),
    );
    let handles = (
        fakes.parties.clone(),
        fakes.docs.clone(),
        fakes.notices.clone(),
        fakes.tasks.clone(),
        fakes.cheques.clone(),
        fakes.deposits.clone(),
        fakes.invoices.clone(),
        fakes.refunds.clone(),
        fakes.blocks.clone(),
        fakes.staff.clone(),
    );
    let mut app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(property::Property);
    if real.invoicing {
        // As the Property client: the system issues what 01 drafts for a
        // lease at once (the damages invoice, a reissued invoice).
        app = app.module(invoicing);
    }
    if real.documents {
        app = app
            .module(renox::audit::Audit)
            .module(wl_documents::WlDocuments);
    }
    if real.payments {
        // The vertical turns the deductions dispute on: 14 days, 7 to answer.
        app = app.module(payments.deductions_dispute(14, 7));
    }
    let app = app.module(fakes);
    // What wl-parties registers: an exception for a blocked party is decided by
    // the compliance role.
    wl_kernel::PolicyDefault::new(wl_kernel::BLOCK_EXCEPTION_ACTION)
        .with_step(
            wl_kernel::approvals::Step::one_of(["compliance"])
                .requiring(wl_kernel::BLOCK_EXCEPTION_ACTION),
        )
        .register();
    let config =
        wl_kernel::ClientConfig::parse(&client(&real.enabled())).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    let manager = [
        perm::UNITS_VIEW,
        perm::UNITS_MANAGE,
        perm::UNITS_RESERVE,
        perm::LEASES_VIEW,
        perm::LEASES_CREATE,
        perm::LEASES_CANCEL,
        perm::LEASES_ACTIVATE,
        perm::LEASES_MANAGE,
        perm::OWNER_STATEMENTS_APPROVE,
        perm::AGREEMENTS_MANAGE,
        perm::COMPLIANCE_VIEW,
    ];
    permissions::define_role(app.db(), MANAGER, &manager)
        .await
        .unwrap();
    let agent = [
        perm::UNITS_VIEW,
        perm::LEASES_VIEW,
        perm::LEASES_CREATE,
        perm::LEASES_CANCEL,
    ];
    permissions::define_role(app.db(), AGENT, &agent)
        .await
        .unwrap();
    let accountant = [
        perm::UNITS_VIEW,
        perm::LEASES_VIEW,
        perm::LEASES_CANCEL,
        perm::DEPOSITS_REFUND,
        perm::OWNER_STATEMENTS_PREPARE,
    ];
    permissions::define_role(app.db(), ACCOUNTANT, &accountant)
        .await
        .unwrap();
    // The roles that give the second approval of a forfeiture (`PRP.ACL.05`).
    for role in perm::SECOND_APPROVER_ROLES {
        permissions::define_role(app.db(), role, &[perm::UNITS_VIEW])
            .await
            .unwrap();
    }
    // The admin role the tasks fall back to.
    let mut admin = manager.to_vec();
    admin.push(perm::LEASES_FORCE_TERMINATE);
    permissions::define_role(app.db(), wl_kernel::COMPANY_ADMIN_ROLE, &admin)
        .await
        .unwrap();
    if real.documents {
        docs_handle_real(&handles.1);
    }
    let (parties, docs, notices, tasks, cheques, deposits, invoices, refunds, blocks, staff) =
        handles;
    parties.add(TENANT.0, "Tariq Hassan", PartyKind::Person);
    parties.add(COMPANY.0, "Blue Dunes LLC", PartyKind::Company);
    Booted {
        app,
        _views: views,
        parties,
        docs,
        notices,
        tasks,
        cheques,
        deposits,
        invoices,
        refunds,
        blocks,
        staff,
        activated: logs.0,
        cancelled: logs.1,
        withdrawn: logs.2,
        terminated: logs.3,
        amended: logs.4,
        forced: logs.5,
        deposit_adjusted: logs.6,
    }
}

/// The tenant acceptance of changed terms is answered by wl-documents.
fn docs_handle_real(docs: &Docs) {
    docs.use_real_for(property::terms::TENANT_ACCEPTANCE_KIND);
}

/// The scope of a building, for assertions.
pub fn scope_of(building: &Building) -> Scope {
    building.scope()
}

impl Booted {
    /// A lease of `unit` for [`TENANT`] that is active: its term began 300
    /// days ago and ends in 64 days, AED 100,000 a year in 4 instalments.
    pub async fn active_lease(&self, unit: &Unit, agent: &User, manager: &User) -> Lease {
        let lease = self.pending(agent, unit, -300).await;
        self.waive(manager, &lease).await;
        self.activate(manager, &lease).await.assert_ok();
        let active = self.reload(&lease).await;
        assert_eq!(active.status, LeaseStatus::Active);
        active
    }

    /// Holds a deposit of `amount` fils for `lease` through the real
    /// wl-payments: a transfer from the tenant is recorded and allocated to
    /// it. (Needs [`Real::payments`].)
    pub async fn hold_real_deposit(&self, lease: &Lease, amount: i64) {
        permissions::define_role(
            self.db(),
            "payments_teller",
            &[
                wl_payments::permissions::DEPOSITS_MANAGE,
                wl_payments::permissions::RECORD,
            ],
        )
        .await
        .unwrap();
        let teller = self
            .user_with(&format!("Teller {}", lease.id), "payments_teller")
            .await;
        let entity = wl_kernel::LegalEntity::query()
            .where_(wl_kernel::LegalEntity::CODE.eq("PGP"))
            .first(self.db())
            .await
            .unwrap()
            .expect("the default entity");
        let mut request = RecordPayment::new(
            TENANT,
            entity.id,
            wl_kernel::payments::PaymentMethod::Transfer,
            amount,
        );
        request.reference = Some(format!("TRF-DEP-{}", lease.id));
        let payment = payment_recorder(self.state())
            .record(self.state(), request, Some(teller.id))
            .await
            .expect("the payment is recorded");
        let (entity, amount, id, payment) = (
            entity.id.to_string(),
            amount.to_string(),
            lease.agreement().subject_id.to_string(),
            payment.id.0.to_string(),
        );
        self.post(
            &teller,
            "/test/deposits",
            &[
                ("party", &TENANT.0.to_string()),
                ("entity", &entity),
                ("source_type", &lease.agreement().subject_type),
                ("source_id", &id),
                ("amount", &amount),
                ("payment", &payment),
            ],
        )
        .await
        .assert_ok();
    }

    /// The tenant's signed acceptance of changed terms is uploaded to the
    /// version `lease` as a real `tenant_acceptance` document and accepted on
    /// the review queue (`PRP.ACL.17`). (Needs [`Real::documents`].)
    pub async fn attach_acceptance(&self, lease: &Lease) -> wl_documents::Document {
        wl_documents::kinds::install_seeds(
            self.db(),
            &[wl_documents::KindSeed::new(
                property::terms::TENANT_ACCEPTANCE_KIND,
                "Tenant acceptance of changed terms",
            )
            .subject_type("property.lease")],
        )
        .await
        .expect("the kind installs");
        let reviewer = self
            .user_with_grants(
                "Rita",
                &[
                    wl_documents::permissions::VIEW,
                    wl_documents::permissions::REVIEW,
                ],
            )
            .await;
        let new = wl_documents::documents::NewDocument::new(
            lease.subject(),
            property::terms::TENANT_ACCEPTANCE_KIND,
            Upload::new(
                "acceptance.pdf",
                "application/pdf",
                b"%PDF-1.7 signed acceptance".to_vec(),
            ),
        );
        let document = wl_documents::Documents::attach(self.state(), None, new)
            .await
            .unwrap()
            .expect("the document is attached");
        self.app.acting_as(&reviewer);
        self.app
            .htmx()
            .post(&format!("/documents/{}/accept", document.id), &[])
            .await
            .assert_ok();
        wl_documents::Document::find(self.db(), document.id)
            .await
            .unwrap()
            .unwrap()
    }

    /// A user holding exactly `grants` through a role of their own.
    pub async fn user_with_grants(&self, name: &str, grants: &[&str]) -> User {
        let role = format!("role_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        self.user_with(name, &role).await
    }

    /// Runs `work` with the clock at 08:00 UTC of `date` (a date after today):
    /// `today()` and `now()` read that day inside it, the way the tests of the
    /// issues' literal dates need.
    pub async fn at<F: std::future::Future>(&self, date: NaiveDate, work: F) -> F::Output {
        // Outside `at_travelled_time` the clock is the real one, so a second
        // trip starts from the present again instead of adding to the first.
        self.app.travel_back();
        let target = date.and_hms_opt(8, 0, 0).expect("a time").and_utc();
        let seconds = (target - renox::db::now()).num_seconds();
        assert!(seconds > 0, "{date} is not after today");
        self.app
            .travel(std::time::Duration::from_secs(seconds as u64));
        let done = self.app.at_travelled_time(work).await;
        self.app.travel_back();
        done
    }

    /// Runs the system transition `name` on `lease`.
    pub async fn system(&self, lease: &Lease, name: &str) {
        use renox::state_machine::StateMachine;
        let current = self.reload(lease).await;
        let _outcome = current
            .transition(name)
            .by_system()
            .run(self.state())
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
