//! Booting wl-invoicing in a `TestApp`, as `crates/wl-parties/tests/support`
//! boots wl-parties: a party directory of its own (the crate does not depend
//! on wl-parties), a notifier that records what it is asked for, and the
//! kernel events recorded.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_invoicing::bill_machine;
use wl_invoicing::bill_submit;
use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::vat_summary::{self, VatSummary, VatSummaryStatus};
use wl_invoicing::{
    AdjustmentKind, BudgetCategory, CompanyBudget, Invoice, InvoicingConfig, StartChoice,
    VatAdjustment, WlInvoicing, permissions as perm,
};
use wl_invoicing::{BillLineDraft, DrawingApproval, DrawingStatus, SupplierBill};
use wl_kernel::LegalEntity;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::bank_statements::{BankBalance, BankStatementFuture, BankStatements};
use wl_kernel::budgets::{CompanyBudgetApproved, CompanyBudgetVarianceReached};
use wl_kernel::cash_forecast::CashForecastBelowThreshold;
use wl_kernel::cheques::{
    ChequeFuture, ChequeId, ChequeSchedule, ChequeSummary, Cheques, OpenCheque, RegisterCheque,
};
use wl_kernel::documents::today;
use wl_kernel::drawings::OwnerDrawingDecided;
use wl_kernel::einvoice::{EInvoiceAccepted, EInvoiceRejected};
use wl_kernel::invoicing::{
    BadDebtRecovered, CreditNoteReopened, DraftLine, InvoiceDisputeResolved, InvoiceDisputed,
    InvoiceDraft, InvoiceId, InvoiceIssued, InvoiceOverdue, InvoicePaid, InvoiceReopened,
    InvoiceWrittenOff, SupplierBillApproved, SupplierBillReopened, SupplierBillVoided, VatCategory,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest};
use wl_kernel::parties::{BillingIdentity, PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::payments::{
    AllocationTarget, BankPayment, Direction, PaymentFuture, PaymentId, PaymentMethod,
    PaymentRecorder, PaymentStatus, PaymentSummary, RecordPayment, RefundId, RefundRequest,
    ReversalReason, UnpaidRefund,
};
use wl_kernel::tickets::{
    ContractCover, OpenTicket, ServiceContractCoverage, ServiceContractId, TaskComment,
    TaskCreator, TicketFuture, TicketId, TicketRef,
};
use wl_kernel::{ClientConfig, PartyId, Subject};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the invoicing module (and the parties it needs) on: a
/// VAT-registered mainland company and a free-zone one without a TRN.
pub const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true

[[legal_entities]]
code = "PGFZ"
name_en = "Palm Grove Facilities FZ-LLC"
"#;

/// Blue Dunes LLC: has a billing e-mail.
pub const BLUE_DUNES: PartyId = PartyId(1);
/// Blue Dunes' billing e-mail.
pub const BLUE_DUNES_EMAIL: &str = "ap@bluedunes.example";
/// Quiet Sands LLC: no billing e-mail.
pub const QUIET_SANDS: PartyId = PartyId(2);

/// A party whose status is `blocked`; it exists only for status reads.
pub const BLOCKED_PARTY: PartyId = PartyId(3);

/// An individual: a B2C recipient, outside the e-invoicing mandate.
pub const OMAR: PartyId = PartyId(4);

/// The parties the tests bill. Cloning shares the recipient identifiers a
/// test sets.
#[derive(Debug, Clone, Default)]
pub struct TestDirectory(Arc<Mutex<std::collections::BTreeMap<i64, String>>>);

impl TestDirectory {
    /// Sets (or clears) the e-invoicing recipient identifier of `party`.
    pub fn set_recipient(&self, party: PartyId, identifier: Option<&str>) {
        let mut map = self.0.lock().unwrap();
        match identifier {
            Some(id) => map.insert(party.0, id.to_owned()),
            None => map.remove(&party.0),
        };
    }
}

/// The summary of a test party, by id.
fn summary_of(party: PartyId) -> Option<PartySummary> {
    let (code, name) = match party {
        BLUE_DUNES => ("P-00001", "Blue Dunes LLC"),
        QUIET_SANDS => ("P-00002", "Quiet Sands LLC"),
        OMAR => ("P-00004", "Omar Hassan"),
        _ => return None,
    };
    Some(PartySummary {
        id: party,
        code: code.to_owned(),
        kind: if party == OMAR {
            wl_kernel::parties::PartyKind::Person
        } else {
            wl_kernel::parties::PartyKind::Company
        },
        display_name: name.to_owned(),
        status: PartyStatus::Active,
        display_block_reason: None,
    })
}

impl PartyDirectory for TestDirectory {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move { Ok(summary_of(party)) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok([BLUE_DUNES, QUIET_SANDS]
                .into_iter()
                .filter_map(summary_of)
                .find(|p| p.code == code))
        })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        // Party 3 is blocked (the cash-flow forecast tests).
        let status = if party == BLOCKED_PARTY {
            PartyStatus::Blocked
        } else {
            PartyStatus::Active
        };
        Box::pin(async move { Ok(Some(status)) })
    }

    fn billing_identity<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<BillingIdentity>> {
        let (name, email) = match party {
            BLUE_DUNES => ("Blue Dunes LLC", Some(BLUE_DUNES_EMAIL.to_owned())),
            QUIET_SANDS => ("Quiet Sands LLC", None),
            OMAR => ("Omar Hassan", None),
            _ => return Box::pin(async { Ok(None) }),
        };
        let recipient = self.0.lock().unwrap().get(&party.0).cloned();
        Box::pin(async move {
            Ok(Some(BillingIdentity {
                party,
                legal_name_en: name.to_owned(),
                legal_name_ar: None,
                trn: Some("100000000000011".to_owned()),
                billing_address: Some("Office 12, Business Bay, Dubai".to_owned()),
                billing_email: email,
                einvoice_recipient_id: recipient,
            }))
        })
    }
}

/// A test-only module with two probes of what a user is offered; a test
/// cannot build an `AuthUser`, so these read it from the request.
struct Probe;

impl Module for Probe {
    fn name(&self) -> &'static str {
        "invoicing-probe"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/probe/{id}/available", probe_available)
            .get("/probe/{id}/offered", probe_offered)
            .post("/probe/{id}/contract-issue", probe_contract_issue)
    }
}

async fn probe_available(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let invoice = Invoice::find_or_404(&state.db, id).await?;
    let names: Vec<String> = invoice
        .available(&user, &state.db)
        .await?
        .into_iter()
        .map(|action| action.name)
        .collect();
    Ok(names.join(","))
}

/// `Invoicer::issue` for the signed-in user: the answer is `ok` or the error's
/// status.
async fn probe_contract_issue(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> String {
    match wl_kernel::invoicing::invoicer(&state)
        .issue(&state, wl_kernel::invoicing::InvoiceId(id), Some(&user))
        .await
    {
        Ok(()) => "ok".to_owned(),
        Err(Error::Status(code, _)) => code.as_u16().to_string(),
        Err(Error::Forbidden) => "403".to_owned(),
        Err(_) => "error".to_owned(),
    }
}

async fn probe_offered(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<String> {
    let invoice = Invoice::find_or_404(&state.db, id).await?;
    let config = state
        .provided::<wl_invoicing::InvoicingConfig>()
        .expect("the module provides its settings");
    let offered = wl_invoicing::write_off::offered(&config, &user, &invoice).await;
    Ok(if offered { "yes" } else { "no" }.to_owned())
}

/// A notifier that records every request and queues nothing.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>, Arc<Mutex<Vec<i64>>>);

impl Recording {
    /// Every request so far, oldest first.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// From now on the notifier fails (and records nothing) for a request
    /// to any of the users `ids`; an empty list makes it work again.
    pub fn fail_for(&self, ids: &[i64]) {
        *self.1.lock().unwrap() = ids.to_vec();
    }

    /// The requests of one notification type, oldest first.
    pub fn of_type(&self, type_key: &str) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|r| r.type_key == type_key)
            .collect()
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        let failing = self.1.lock().unwrap().clone();
        let hit = request.recipients.iter().any(|r| {
            matches!(r, wl_kernel::notify::NotifyRecipient::User { id } if failing.contains(id))
        });
        if hit {
            return Box::pin(async {
                Err(Error::Status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The notifier is down.".into(),
                ))
            });
        }
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// A refund asked of the desk, with the user who asked.
pub type Asked = (RefundRequest, Option<i64>);

/// A `PaymentRecorder` for the refunds a credit note asks for: it records
/// each request with its user and answers a new refund id, or refuses (422)
/// while `refuse` is set. It records and allocates nothing.
#[derive(Clone, Default)]
pub struct RefundDesk {
    asked: Arc<Mutex<Vec<Asked>>>,
    refuse: Arc<AtomicBool>,
    bank: Arc<Mutex<Vec<(i64, BankPayment)>>>,
    unpaid: Arc<Mutex<Vec<UnpaidRefund>>>,
}

impl RefundDesk {
    /// A payment of `entity` in `account` for the cash-flow reads.
    pub fn bank_payment(
        &self,
        entity: i64,
        account: &str,
        direction: Direction,
        amount: i64,
        date: NaiveDate,
        reference: Option<&str>,
    ) {
        let mut bank = self.bank.lock().unwrap();
        let id = bank.len() as i64 + 1;
        bank.push((
            entity,
            BankPayment {
                payment: PaymentSummary {
                    id: PaymentId(id),
                    number: None,
                    party: BLUE_DUNES,
                    legal_entity_id: entity,
                    direction,
                    method: PaymentMethod::Transfer,
                    amount,
                    allocated: 0,
                    unallocated: amount,
                    currency: "AED".to_owned(),
                    status: PaymentStatus::Received,
                    date,
                },
                bank_account: account.to_owned(),
                reference: reference.map(str::to_owned),
            },
        ));
    }

    /// A refund approved or failed and not paid, for the cash-flow reads.
    pub fn unpaid_refund(&self, entity: i64, amount: i64, failed: bool) {
        let mut unpaid = self.unpaid.lock().unwrap();
        let id = unpaid.len() as i64 + 1;
        unpaid.push(UnpaidRefund {
            id: RefundId(id),
            party: BLUE_DUNES,
            legal_entity_id: entity,
            amount,
            currency: "AED".to_owned(),
            failed,
            approved_on: None,
            source: Subject::new("invoicing.credit_note", id),
        });
    }

    /// The requests so far, oldest first, each with its user.
    pub fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }

    /// Whether the next requests are refused.
    pub fn refuse(&self, refuse: bool) {
        self.refuse.store(refuse, Ordering::SeqCst);
    }
}

fn desk_off(what: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, what.to_owned())
}

impl PaymentRecorder for RefundDesk {
    fn record<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: RecordPayment,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        Box::pin(async { Err(desk_off("not recorded")) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _target: AllocationTarget,
        _amount: i64,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(desk_off("not allocated")) })
    }

    fn reverse<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _reason: ReversalReason,
        _note: Option<String>,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(desk_off("not reversed")) })
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
        by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        Box::pin(async move {
            if self.refuse.load(Ordering::SeqCst) {
                return Err(desk_off("the refund is refused"));
            }
            let mut asked = self.asked.lock().unwrap();
            asked.push((request, by));
            Ok(RefundId(asked.len() as i64))
        })
    }

    fn payments_after<'a>(
        &'a self,
        _db: &'a Db,
        legal_entity_id: i64,
        bank_account: &'a str,
        after: NaiveDate,
    ) -> PaymentFuture<'a, Vec<BankPayment>> {
        Box::pin(async move {
            Ok(self
                .bank
                .lock()
                .unwrap()
                .iter()
                .filter(|(entity, p)| {
                    *entity == legal_entity_id
                        && p.bank_account == bank_account
                        && p.payment.date > after
                })
                .map(|(_, p)| p.clone())
                .collect())
        })
    }

    fn refunds_unpaid<'a>(
        &'a self,
        _db: &'a Db,
        legal_entity_id: i64,
    ) -> PaymentFuture<'a, Vec<UnpaidRefund>> {
        Box::pin(async move {
            Ok(self
                .unpaid
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.legal_entity_id == legal_entity_id)
                .cloned()
                .collect())
        })
    }
}

/// The cheques and the statement balances the cash-flow forecast reads,
/// scripted by the test.
#[derive(Clone, Default)]
pub struct ForecastDesk {
    open: Arc<Mutex<Vec<(i64, OpenCheque)>>>,
    bounced: Arc<Mutex<Vec<(i64, OpenCheque)>>>,
    balances: Arc<Mutex<Vec<BankBalance>>>,
}

impl ForecastDesk {
    /// A cheque not cleared, of `entity`.
    pub fn open_cheque(&self, entity: i64, cheque: OpenCheque) {
        self.open.lock().unwrap().push((entity, cheque));
    }

    /// A cheque that bounced, of `entity`.
    pub fn bounced_cheque(&self, entity: i64, cheque: OpenCheque) {
        self.bounced.lock().unwrap().push((entity, cheque));
    }

    /// A completed statement's closing balance; it replaces the one the
    /// account had (the latest closing balance is read).
    pub fn statement(&self, entity: i64, account: &str, amount: i64, as_of: NaiveDate) {
        self.balances
            .lock()
            .unwrap()
            .retain(|b| !(b.legal_entity_id == entity && b.bank_account == account));
        self.balances.lock().unwrap().push(BankBalance {
            legal_entity_id: entity,
            bank_account: account.to_owned(),
            amount,
            currency: "AED".to_owned(),
            as_of,
            import_id: 1,
        });
    }
}

/// A scripted [`OpenCheque`] of `amount` fils dated `date` paying
/// `documents` (each with the part applied).
pub fn open_cheque(
    id: i64,
    number: &str,
    direction: Direction,
    date: NaiveDate,
    amount: i64,
    documents: &[(Subject, i64)],
) -> OpenCheque {
    OpenCheque {
        cheque: ChequeSummary {
            id: ChequeId(id),
            direction,
            party: BLUE_DUNES,
            legal_entity_id: 0,
            bank: "ENBD".to_owned(),
            number: number.to_owned(),
            date,
            amount,
            honoured_amount: 0,
            open_amount: amount,
            currency: "AED".to_owned(),
            presentations: 1,
            status: wl_kernel::cheques::ChequeStatus::Received,
            agreement: None,
            linked: documents.iter().map(|(d, _)| d.clone()).collect(),
        },
        documents: documents
            .iter()
            .map(|(document, amount)| wl_kernel::cheques::ChequeDocument {
                document: document.clone(),
                amount: *amount,
            })
            .collect(),
    }
}

impl Cheques for ForecastDesk {
    fn register<'a>(
        &'a self,
        _state: &'a AppState,
        _cheque: RegisterCheque,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, ChequeSummary> {
        Box::pin(async { Err(desk_off("not registered")) })
    }

    fn register_schedule<'a>(
        &'a self,
        _state: &'a AppState,
        _schedule: ChequeSchedule,
        _by: Option<i64>,
    ) -> ChequeFuture<'a, Vec<ChequeSummary>> {
        Box::pin(async { Err(desk_off("not registered")) })
    }

    fn find<'a>(&'a self, _db: &'a Db, _id: ChequeId) -> ChequeFuture<'a, Option<ChequeSummary>> {
        Box::pin(async { Ok(None) })
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

    fn not_cleared<'a>(
        &'a self,
        _state: &'a AppState,
        legal_entity_id: i64,
        direction: Direction,
    ) -> ChequeFuture<'a, Vec<OpenCheque>> {
        Box::pin(async move {
            Ok(self
                .open
                .lock()
                .unwrap()
                .iter()
                .filter(|(entity, c)| *entity == legal_entity_id && c.cheque.direction == direction)
                .map(|(_, c)| c.clone())
                .collect())
        })
    }

    fn bounced<'a>(
        &'a self,
        _state: &'a AppState,
        legal_entity_id: i64,
    ) -> ChequeFuture<'a, Vec<OpenCheque>> {
        Box::pin(async move {
            Ok(self
                .bounced
                .lock()
                .unwrap()
                .iter()
                .filter(|(entity, _)| *entity == legal_entity_id)
                .map(|(_, c)| c.clone())
                .collect())
        })
    }
}

impl BankStatements for ForecastDesk {
    fn latest_closing_balances<'a>(
        &'a self,
        _db: &'a Db,
        legal_entity_id: i64,
    ) -> BankStatementFuture<'a, Vec<BankBalance>> {
        Box::pin(async move {
            let mut found: Vec<BankBalance> = self
                .balances
                .lock()
                .unwrap()
                .iter()
                .filter(|b| b.legal_entity_id == legal_entity_id)
                .cloned()
                .collect();
            found.sort_by(|a, b| a.bank_account.cmp(&b.bank_account));
            Ok(found)
        })
    }
}

/// A `TaskCreator` that records the tickets it is asked to open and to
/// resolve, and answers `TKT-n`; switched off, it opens nothing (as without
/// the ticketing module).
#[derive(Clone, Default)]
pub struct TicketDesk {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
    resolved: Arc<Mutex<Vec<(TicketId, String)>>>,
    comments: Arc<Mutex<Vec<(TicketId, TaskComment)>>>,
    off: Arc<AtomicBool>,
}

impl TicketDesk {
    /// The tickets opened, oldest first (ticket `n` is the n-th).
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }

    /// The tickets resolved by their source, with the note.
    pub fn resolved(&self) -> Vec<(TicketId, String)> {
        self.resolved.lock().unwrap().clone()
    }

    /// The comments added to tickets, with their ticket.
    pub fn comments(&self) -> Vec<(TicketId, TaskComment)> {
        self.comments.lock().unwrap().clone()
    }

    /// Switches ticketing off (`true`) or on again.
    pub fn off(&self, off: bool) {
        self.off.store(off, Ordering::SeqCst);
    }
}

impl TaskCreator for TicketDesk {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            if self.off.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let mut opened = self.opened.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn resolve_from_source<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            self.resolved
                .lock()
                .unwrap()
                .push((ticket, note.to_owned()));
            Ok(true)
        })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(async move {
            self.comments.lock().unwrap().push((ticket, comment));
            Ok(())
        })
    }
}

/// Everything of one type `E` emitted so far.
#[derive(Clone)]
pub struct Recorded<E>(Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    fn new() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }

    /// The events, oldest first.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

fn record<E: Event + Clone>(app: App, into: &Recorded<E>) -> App {
    let into = into.clone();
    app.listen(move |event: E, _state: AppState| {
        let into = into.clone();
        async move {
            into.0.lock().unwrap().push(event);
            Ok(())
        }
    })
}

/// The service contracts' coverage of a day, as ticketing answers it: a
/// contract covers the days up to the last day the test sets; an unset
/// contract is unknown.
#[derive(Debug, Clone, Default)]
pub struct ContractDesk(Arc<Mutex<std::collections::BTreeMap<i64, NaiveDate>>>);

impl ContractDesk {
    /// Sets the last day `contract` covers (its end or termination date).
    pub fn ends_on(&self, contract: i64, last_day: NaiveDate) {
        self.0.lock().unwrap().insert(contract, last_day);
    }
}

impl ServiceContractCoverage for ContractDesk {
    fn covers<'a>(
        &'a self,
        _db: &'a Db,
        contract: ServiceContractId,
        on: NaiveDate,
    ) -> TicketFuture<'a, ContractCover> {
        Box::pin(async move {
            let ends = self.0.lock().unwrap().get(&contract.0).copied();
            Ok(match ends {
                None => ContractCover::Unknown,
                Some(last) if on <= last => ContractCover::Covered,
                Some(last) => ContractCover::AfterEnd { ends_on: last },
            })
        })
    }
}

/// The app under test with what it emitted.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The reminders (and other notices) asked of the kernel's notifier.
    pub notices: Recording,
    /// `InvoiceIssued`.
    pub issued: Recorded<InvoiceIssued>,
    /// `InvoiceOverdue`.
    pub overdue: Recorded<InvoiceOverdue>,
    /// `InvoicePaid`.
    pub paid: Recorded<InvoicePaid>,
    /// `InvoiceWrittenOff`.
    pub written_off: Recorded<InvoiceWrittenOff>,
    /// `CreditNoteReopened`.
    pub reopened: Recorded<CreditNoteReopened>,
    /// `InvoiceReopened`.
    pub invoice_reopened: Recorded<InvoiceReopened>,
    /// The refunds asked of payments.
    pub refunds: RefundDesk,
    /// The cheques and statement balances the cash-flow forecast reads.
    pub forecast: ForecastDesk,
    /// The coverage of service contracts the match asks of ticketing.
    pub contracts: ContractDesk,
    /// `BadDebtRecovered`.
    pub recovered: Recorded<BadDebtRecovered>,
    /// `InvoiceDisputeResolved`.
    pub resolved: Recorded<InvoiceDisputeResolved>,
    /// `InvoiceDisputed`.
    pub disputed: Recorded<InvoiceDisputed>,
    /// The tickets asked of ticketing.
    pub tickets: TicketDesk,
    /// `SupplierBillApproved`.
    pub bill_approved: Recorded<SupplierBillApproved>,
    /// `SupplierBillVoided`.
    pub bill_voided: Recorded<SupplierBillVoided>,
    /// `SupplierBillReopened`.
    pub bill_reopened: Recorded<SupplierBillReopened>,
    /// `CompanyBudgetApproved`.
    pub budget_approved: Recorded<CompanyBudgetApproved>,
    /// `CompanyBudgetVarianceReached`.
    pub variance_reached: Recorded<CompanyBudgetVarianceReached>,
    /// `CashForecastBelowThreshold`.
    pub below_threshold: Recorded<CashForecastBelowThreshold>,
    /// `OwnerDrawingDecided`.
    pub drawing_decided: Recorded<OwnerDrawingDecided>,
    /// `EInvoiceAccepted`.
    pub einvoice_accepted: Recorded<EInvoiceAccepted>,
    /// `EInvoiceRejected`.
    pub einvoice_rejected: Recorded<EInvoiceRejected>,
    /// The test parties' directory: set a recipient identifier here.
    pub directory: TestDirectory,
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

    /// A user holding the roles of the host: `receivables` (and `admin`
    /// too when `admin`).
    async fn staff(&self, name: &str, admin: bool) -> User {
        let user = User::register(
            self.db(),
            name,
            &format!("{}@example.com", name.to_lowercase()),
            "password123",
        )
        .await
        .unwrap();
        user.assign_role(self.db(), "receivables").await.unwrap();
        if admin {
            user.assign_role(self.db(), "admin").await.unwrap();
        }
        user
    }

    /// User A: a receivables clerk.
    pub async fn user_a(&self) -> User {
        self.staff("Amal", false).await
    }

    /// User A, who is also an admin.
    pub async fn admin_a(&self) -> User {
        self.staff("Amal", true).await
    }

    /// User B: another receivables clerk.
    pub async fn user_b(&self) -> User {
        self.staff("Bilal", false).await
    }

    /// A user who may only see invoices.
    pub async fn viewer(&self) -> User {
        self.user("Vera", &[perm::VIEW]).await
    }

    /// A user of the host's `accountant` role (the ones the relief notice
    /// goes to).
    pub async fn accountant(&self, name: &str) -> User {
        let user = User::register(
            self.db(),
            name,
            &format!("{}@example.com", name.to_lowercase()),
            "password123",
        )
        .await
        .unwrap();
        user.assign_role(self.db(), "accountant").await.unwrap();
        user
    }

    /// The user `name` with `grants` (through a role of their own), made
    /// once: asked again, the same user.
    pub async fn helper(&self, name: &str, grants: &[&str]) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        match User::find_by_email(self.db(), &email).await.unwrap() {
            Some(user) => user,
            None => self.user(name, grants).await,
        }
    }

    /// A receivables clerk `name`, made once: asked again, the same user.
    pub async fn clerk(&self, name: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        match User::find_by_email(self.db(), &email).await.unwrap() {
            Some(user) => user,
            None => self.staff(name, false).await,
        }
    }

    /// The module's settings, as the app has them.
    pub fn config(&self) -> std::sync::Arc<InvoicingConfig> {
        self.state()
            .provided::<InvoicingConfig>()
            .expect("the module provides its settings")
    }

    /// The legal entity `code`.
    pub async fn entity(&self, code: &str) -> LegalEntity {
        LegalEntity::query()
            .where_(LegalEntity::CODE.eq(code))
            .first(self.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no legal entity {code}"))
    }

    /// A new active, VAT-registered legal entity.
    pub async fn new_entity(&self, code: &str) -> LegalEntity {
        let mut entity = LegalEntity {
            code: code.to_owned(),
            name_en: format!("{code} LLC"),
            trn: Some("100000000000029".to_owned()),
            vat_registered: true,
            status: "active".to_owned(),
            ..Default::default()
        };
        entity.insert(self.db()).await.unwrap();
        entity
    }

    /// A user who prepares budgets (and sees them), made once.
    pub async fn budget_preparer(&self, name: &str) -> User {
        self.helper(name, &[perm::BUDGETS_VIEW, perm::BUDGETS_MANAGE])
            .await
    }

    /// A user who approves budgets (and sees them), made once.
    pub async fn budget_approver(&self, name: &str) -> User {
        self.helper(name, &[perm::BUDGETS_VIEW, perm::BUDGETS_APPROVE])
            .await
    }

    /// The budget category with this code.
    pub async fn category(&self, code: &str) -> BudgetCategory {
        BudgetCategory::by_code(self.db(), code)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("no budget category {code}"))
    }

    /// Budget `id`, as stored now.
    pub async fn budget(&self, id: i64) -> CompanyBudget {
        CompanyBudget::find(self.db(), id).await.unwrap().unwrap()
    }

    /// `budget`, as stored now.
    pub async fn budget_now(&self, budget: &CompanyBudget) -> CompanyBudget {
        self.budget(budget.id).await
    }

    /// A draft version for the entity `code` and `year`, started as `start`,
    /// made by `by`.
    pub async fn new_budget(
        &self,
        code: &str,
        year: i64,
        start: StartChoice,
        by: &User,
    ) -> CompanyBudget {
        let entity = self.entity(code).await;
        budget_lines::create(
            self.state(),
            NewBudget {
                legal_entity_id: entity.id,
                fiscal_year: year,
                name: String::new(),
                start,
                revision_reason: None,
            },
            Some(by.id),
        )
        .await
        .expect("the draft is made")
    }

    /// Sets `amount` fils for the category `code` in each month of
    /// `budget` from the `from`-th to the `to`-th (1 to 12, of the fiscal
    /// year).
    pub async fn fill(
        &self,
        budget: &CompanyBudget,
        code: &str,
        amount: i64,
        from: usize,
        to: usize,
    ) {
        let category = self.category(code).await;
        let months = budget.months();
        let lines: Vec<LineInput> = months[from - 1..to]
            .iter()
            .map(|month| LineInput {
                category_id: category.id,
                month: *month,
                amount,
                note: None,
            })
            .collect();
        budget_lines::set_lines(self.db(), budget.id, &lines)
            .await
            .expect("the lines are set");
    }

    /// `POST path` as `by` with `fields`, as a sheet or a button sends it.
    pub async fn post_as(
        &self,
        path: &str,
        by: &User,
        fields: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app.htmx().post(path, fields).await
    }

    /// Submits `budget` for approval as `by` (the page's button).
    pub async fn submit_budget(&self, budget: &CompanyBudget, by: &User) {
        self.post_as(&format!("/budgets/{}/submit", budget.id), by, &[])
            .await
            .assert_redirect(&format!("/budgets/{}", budget.id));
    }

    /// The pending approval request about `budget`.
    pub async fn budget_request(&self, budget: &CompanyBudget) -> ApprovalRequest {
        let mut pending = approvals::pending_for(self.db(), &budget.subject())
            .await
            .unwrap();
        assert_eq!(pending.len(), 1, "one pending request");
        pending.remove(0)
    }

    /// The names of the transitions `budget` went through, in order.
    pub async fn budget_history(&self, budget: &CompanyBudget) -> Vec<String> {
        renox::state_machine::history(self.db(), CompanyBudget::TABLE, budget.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.transition)
            .collect()
    }

    /// `user`'s notices of `kind`, newest first, as JSON text.
    pub async fn notices_of(&self, user: &User, kind: &str) -> Vec<String> {
        user.notifications(self.db(), 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.kind == kind)
            .map(|n| n.data.to_string())
            .collect()
    }

    /// A user holding `grants` through a role given in the scope of the legal
    /// entity `entity_id` only, made once.
    pub async fn scoped_user(&self, name: &str, grants: &[&str], entity_id: i64) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        if let Some(user) = User::find_by_email(self.db(), &email).await.unwrap() {
            return user;
        }
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        let role = format!("scoped_{}", name.to_lowercase());
        permissions::define_role(self.db(), &role, grants)
            .await
            .unwrap();
        let scope = renox::auth::permissions::Scope::of_id::<LegalEntity>(entity_id);
        user.assign_role_in(self.db(), &role, &scope).await.unwrap();
        user
    }

    /// An approved budget of the entity `code` for `year` with the lines
    /// `(category code, month 1 to 12, amount in fils)`.
    pub async fn approved_budget(
        &self,
        code: &str,
        year: i64,
        lines: &[(&str, usize, i64)],
    ) -> CompanyBudget {
        let preparer = self.budget_preparer("Budgeter").await;
        let approver = self.budget_approver("Boss").await;
        let draft = self
            .new_budget(code, year, StartChoice::Empty, &preparer)
            .await;
        for (category, month, amount) in lines {
            self.fill(&draft, category, *amount, *month, *month).await;
        }
        self.submit_budget(&draft, &preparer).await;
        let request = self.budget_request(&draft).await;
        approvals::approve(self.state(), request.id, &approver, "")
            .await
            .unwrap()
            .expect("the approval runs");
        self.budget_now(&draft).await
    }

    /// A supplier bill of Blue Dunes to the entity `entity`, dated `date`,
    /// approved, with the lines `(description, price in fils, cost code,
    /// VAT category)`.
    pub async fn approved_bill(
        &self,
        entity: &LegalEntity,
        number: &str,
        date: NaiveDate,
        lines: &[(&str, i64, &str, VatCategory)],
    ) -> SupplierBill {
        let mut draft = BillDraft::new(BLUE_DUNES, number, date);
        draft.legal_entity_id = Some(entity.id);
        for (description, price, code, category) in lines {
            draft = draft.line(BillLineDraft::new(description, *price, *category).cost_code(code));
        }
        // The user who enters and submits the bill asks for the approval of
        // a drawing on it; another user, a holder of `drawings.approve`,
        // gives it.
        let drawer = self.helper("Drawer", &[perm::SUPPLIER_BILLS_CREATE]).await;
        let bill = supplier_bill::create(self.db(), &draft, Some(drawer.id))
            .await
            .expect("the bill is made");
        bill_submit::submit(self.state(), By::System(Some(drawer.id)), bill.id)
            .await
            .expect("the bill is submitted");
        let drawings = DrawingApproval::for_source(self.db(), &bill.subject())
            .await
            .unwrap();
        if let Some(request) = drawings
            .iter()
            .find(|d| d.status == DrawingStatus::Requested)
            .and_then(|d| d.approval_request_id)
        {
            let owner = self
                .helper(
                    "Downer",
                    &[perm::SUPPLIER_BILLS_VIEW, perm::DRAWINGS_APPROVE],
                )
                .await;
            approvals::approve(self.state(), request, &owner, "")
                .await
                .unwrap()
                .expect("the drawing is approved");
        }
        bill_machine::approve(self.state(), By::System(Some(drawer.id)), bill.id, "")
            .await
            .expect("the bill is approved")
    }

    /// An issued invoice of `entity` with a line of `price` fils (exempt)
    /// billed under the charge type `charge_type`, with the date of supply
    /// `supply`.
    pub async fn invoice_with_charge(
        &self,
        entity: &LegalEntity,
        charge_type: &str,
        price: i64,
        supply: NaiveDate,
        by: &User,
    ) -> Invoice {
        let draft = self
            .draft_of(
                InvoiceDraft::new(BLUE_DUNES, Subject::new("property.lease", 9))
                    .line(
                        DraftLine::new("Rent", price, VatCategory::Exempt).charge_type(charge_type),
                    )
                    .legal_entity(entity.id),
            )
            .await;
        let issued = self.issue(&draft, by).await;
        Invoice::query()
            .where_(Invoice::ID.eq(issued.id))
            .update(self.db(), &[("date_of_supply", &Some(supply))])
            .await
            .unwrap();
        self.reload(&issued).await
    }

    /// The summary of `entity` for the period that contains `date`, taken
    /// to `status` through its transitions (so the period must be over for
    /// anything past `open`).
    pub async fn summary(
        &self,
        entity: &LegalEntity,
        date: NaiveDate,
        status: VatSummaryStatus,
    ) -> VatSummary {
        let config = self.config();
        vat_summary::ensure_open(self.db(), &config, entity, date)
            .await
            .unwrap();
        let summary = VatSummary::containing(self.db(), &config, entity, date)
            .await
            .unwrap()
            .expect("the summary exists");
        let preparer = self
            .helper("Prep", &[perm::VAT_VIEW, perm::VAT_PREPARE])
            .await;
        let reviewer = self
            .helper("Rev", &[perm::VAT_VIEW, perm::VAT_REVIEW, perm::VAT_LOCK])
            .await;
        let steps = [
            ("prepare", VatSummaryStatus::Prepared, &preparer),
            ("review", VatSummaryStatus::Reviewed, &reviewer),
            ("lock", VatSummaryStatus::Locked, &reviewer),
        ];
        for (name, reaches, by) in steps {
            if summary_rank(reaches) > summary_rank(status) {
                break;
            }
            self.post_transition(&summary, name, by, "")
                .await
                .assert_status(303);
        }
        self.summary_now(summary.id).await
    }

    /// Summary `id`, as stored now.
    pub async fn summary_now(&self, id: i64) -> VatSummary {
        VatSummary::find(self.db(), id).await.unwrap().unwrap()
    }

    /// `POST /vat-summaries/{id}/transitions/{name}` as `by`, as the page's
    /// actions send it.
    pub async fn post_transition(
        &self,
        summary: &VatSummary,
        name: &str,
        by: &User,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!("/vat-summaries/{}/transitions/{name}", summary.id),
                &[("reason", reason)],
            )
            .await
    }

    /// An invoice of `entity` (the default one when `None`) for AED
    /// 1,000.00 plus 5% VAT, written off for its whole balance through the
    /// approval, with the date of supply `supply` and the write-off notice
    /// date `notice_on`. Returns it with its pending bad-debt relief of
    /// AED 50.00.
    pub async fn written_off(
        &self,
        entity: Option<&LegalEntity>,
        supply: NaiveDate,
        notice_on: Option<NaiveDate>,
    ) -> (Invoice, VatAdjustment) {
        let clerk = self.clerk("Wendy").await;
        let approver = self.clerk("Wade").await;
        let mut draft = InvoiceDraft::new(BLUE_DUNES, Subject::new("property.lease", 9))
            .line(DraftLine::new("Line 1", 100_000, VatCategory::Standard))
            .due_date(self.day(-5));
        if let Some(entity) = entity {
            draft = draft.legal_entity(entity.id);
        }
        let draft = self.draft_of(draft).await;
        self.issue(&draft, &clerk).await;
        wl_invoicing::overdue::run(self.state())
            .await
            .expect("the overdue run");
        let invoice = self.reload(&draft).await;
        self.post_write_off(&invoice, &clerk, "insolvent", "")
            .await
            .assert_ok();
        let request = self.pending(&invoice).await.remove(0);
        approvals::approve(self.state(), request.id, &approver, "")
            .await
            .unwrap()
            .expect("the approval runs");
        Invoice::query()
            .where_(Invoice::ID.eq(invoice.id))
            .update(
                self.db(),
                &[
                    ("date_of_supply", &Some(supply)),
                    ("write_off_notice_on", &notice_on),
                ],
            )
            .await
            .unwrap();
        let invoice = self.reload(&invoice).await;
        let relief = VatAdjustment::relief_of(self.db(), &invoice)
            .await
            .unwrap()
            .expect("the write-off made the relief");
        assert_eq!(relief.kind, AdjustmentKind::BadDebtRelief);
        (invoice, relief)
    }

    /// The adjustment `id`, as stored now.
    pub async fn adjustment(&self, id: i64) -> VatAdjustment {
        VatAdjustment::find(self.db(), id).await.unwrap().unwrap()
    }

    /// `POST` of the include or exclude sheet (`action`) of `adjustment`.
    pub async fn post_decision(
        &self,
        summary: &VatSummary,
        adjustment: &VatAdjustment,
        action: &str,
        by: &User,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!(
                    "/vat-summaries/{}/adjustments/{}/{action}",
                    summary.id, adjustment.id
                ),
                &[("reason", reason)],
            )
            .await
    }

    /// `n` days from today (negative: before).
    pub fn day(&self, n: i64) -> NaiveDate {
        let days = Days::new(n.unsigned_abs());
        if n >= 0 {
            today().checked_add_days(days).unwrap()
        } else {
            today().checked_sub_days(days).unwrap()
        }
    }

    /// Moves the clock forward to `date` at `hour:minute` UTC.
    pub fn travel_to(&self, date: &str, hour: u32, minute: u32) {
        let target = date
            .parse::<NaiveDate>()
            .expect("a date")
            .and_hms_opt(hour, minute, 0)
            .expect("a time")
            .and_utc();
        // `now()` outside `at_travelled_time` is the real clock, so a second
        // trip starts from the present again instead of adding to the first.
        self.app.travel_back();
        let seconds = (target - renox::db::now()).num_seconds();
        assert!(seconds > 0, "the target {target} is in the past");
        self.app
            .travel(std::time::Duration::from_secs(seconds as u64));
    }

    /// A draft to Blue Dunes with lines of `(unit price in fils, category)`,
    /// through the kernel contract.
    pub async fn draft(&self, lines: &[(i64, VatCategory)], due: Option<NaiveDate>) -> Invoice {
        let mut draft = InvoiceDraft::new(BLUE_DUNES, Subject::new("property.lease", 9));
        for (i, (price, category)) in lines.iter().enumerate() {
            draft = draft.line(DraftLine::new(
                &format!("Line {}", i + 1),
                *price,
                *category,
            ));
        }
        if let Some(due) = due {
            draft = draft.due_date(due);
        }
        self.draft_of(draft).await
    }

    /// `draft` through the kernel contract, as stored.
    pub async fn draft_of(&self, draft: InvoiceDraft) -> Invoice {
        let id = wl_kernel::invoicing::invoicer(self.state())
            .draft(self.db(), draft)
            .await
            .expect("the draft is made")
            .expect("invoicing is on");
        self.invoice(id.0).await
    }

    /// Invoice `id`, as stored now.
    pub async fn invoice(&self, id: i64) -> Invoice {
        Invoice::find(self.db(), id).await.unwrap().unwrap()
    }

    /// `invoice`, as stored now.
    pub async fn reload(&self, invoice: &Invoice) -> Invoice {
        self.invoice(invoice.id).await
    }

    /// `POST /invoices/{id}/issue` as `by`, as the page sends it.
    pub async fn post_issue(&self, draft: &Invoice, by: &User) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/invoices/{}/issue", draft.id), &[])
            .await
    }

    /// `POST /invoices/{id}/write-off` as `by`, as the write-off sheet
    /// sends it.
    pub async fn post_write_off(
        &self,
        invoice: &Invoice,
        by: &User,
        code: &str,
        text: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!("/invoices/{}/write-off", invoice.id),
                &[("reason_code", code), ("reason_text", text)],
            )
            .await
    }

    /// `Invoicer::issue` on `draft` as `by`: `ok`, or the status that refused.
    pub async fn contract_issue(&self, draft: &Invoice, by: &User) -> String {
        self.app.acting_as(by);
        self.app
            .post(&format!("/probe/{}/contract-issue", draft.id), &[])
            .await
            .text()
    }

    /// Issues `draft` as `by`.
    pub async fn issue(&self, draft: &Invoice, by: &User) -> Invoice {
        // Issued: back to the invoice's page.
        self.post_issue(draft, by)
            .await
            .assert_redirect(&format!("/invoices/{}", draft.id));
        self.reload(draft).await
    }

    /// The names of the transitions `by` is offered on `invoice`
    /// (`available()`), comma separated.
    pub async fn available(&self, invoice: &Invoice, by: &User) -> String {
        self.app.acting_as(by);
        self.app
            .get(&format!("/probe/{}/available", invoice.id))
            .await
            .text()
    }

    /// Whether the write-off is offered to `by` now (the page's button).
    pub async fn write_off_offered(&self, invoice: &Invoice, by: &User) -> bool {
        self.app.acting_as(by);
        self.app
            .get(&format!("/probe/{}/offered", invoice.id))
            .await
            .text()
            == "yes"
    }

    /// An issued invoice to Blue Dunes with `lines`, issued by user A.
    pub async fn issued(
        &self,
        lines: &[(i64, VatCategory)],
        due: Option<NaiveDate>,
        by: &User,
    ) -> Invoice {
        let draft = self.draft(lines, due).await;
        self.issue(&draft, by).await
    }

    /// An issued invoice of AED 1,000.00 plus 5% VAT (AED 1,050.00) that
    /// fell due 5 days ago and was marked `overdue`.
    pub async fn overdue_invoice(&self, by: &User) -> Invoice {
        let invoice = self
            .issued(&[(100_000, VatCategory::Standard)], Some(self.day(-5)), by)
            .await;
        wl_invoicing::overdue::run(self.state())
            .await
            .expect("the overdue run");
        self.reload(&invoice).await
    }

    /// Allocates `amount` fils of payment `n` to `invoice`.
    pub async fn pay(&self, invoice: &Invoice, n: i64, amount: i64) {
        wl_kernel::invoicing::invoicer(self.state())
            .allocate(
                self.state(),
                InvoiceId(invoice.id),
                Subject::new("payments.payment", n),
                amount,
            )
            .await
            .expect("the payment is allocated");
    }

    /// The approval requests about `invoice`, oldest first.
    pub async fn requests(&self, invoice: &Invoice) -> Vec<ApprovalRequest> {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(invoice.subject().subject_type.as_str()))
            .where_(ApprovalRequest::SUBJECT_ID.eq(invoice.id))
            .order_by(ApprovalRequest::ID)
            .get(self.db())
            .await
            .unwrap()
    }

    /// The pending approval requests about `invoice`.
    pub async fn pending(&self, invoice: &Invoice) -> Vec<ApprovalRequest> {
        approvals::pending_for(self.db(), &invoice.subject())
            .await
            .unwrap()
    }

    /// The names of the transitions `invoice` went through, in order.
    pub async fn history(&self, invoice: &Invoice) -> Vec<String> {
        renox::state_machine::history(self.db(), Invoice::TABLE, invoice.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.transition)
            .collect()
    }
}

/// The position of a summary status in its lifecycle.
fn summary_rank(status: VatSummaryStatus) -> usize {
    match status {
        VatSummaryStatus::Open => 0,
        VatSummaryStatus::Prepared => 1,
        VatSummaryStatus::Reviewed => 2,
        VatSummaryStatus::Locked => 3,
    }
}

/// wl-invoicing with the §7 defaults, booted.
pub async fn boot() -> Booted {
    boot_with(WlInvoicing::new()).await
}

/// `module` booted in an app with auth and the kernel, the roles of the
/// host (`receivables`, `admin`) defined.
pub async fn boot_with(module: WlInvoicing) -> Booted {
    boot_with_poster(module, None).await
}

/// [`boot_with`] with `poster` as the app's `CostPoster`, when given.
pub async fn boot_with_poster(
    module: WlInvoicing,
    poster: Option<Arc<dyn wl_kernel::invoicing::CostPoster>>,
) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let issued = Recorded::new();
    let overdue = Recorded::new();
    let paid = Recorded::new();
    let written_off = Recorded::new();
    let reopened = Recorded::new();
    let invoice_reopened = Recorded::new();
    let recovered = Recorded::new();
    let resolved = Recorded::new();
    let disputed = Recorded::new();
    let bill_approved = Recorded::new();
    let bill_voided = Recorded::new();
    let bill_reopened = Recorded::new();
    let budget_approved = Recorded::new();
    let variance_reached = Recorded::new();
    let below_threshold = Recorded::new();
    let drawing_decided = Recorded::new();
    let einvoice_accepted = Recorded::new();
    let einvoice_rejected = Recorded::new();
    let tickets = TicketDesk::default();
    let creator: Arc<dyn TaskCreator> = Arc::new(tickets.clone());
    let contracts = ContractDesk::default();
    let coverage: Arc<dyn ServiceContractCoverage> = Arc::new(contracts.clone());
    let notices = Recording::default();
    let refunds = RefundDesk::default();
    let recorder: Arc<dyn PaymentRecorder> = Arc::new(refunds.clone());
    let forecast = ForecastDesk::default();
    let cheque_book: Arc<dyn Cheques> = Arc::new(forecast.clone());
    let statements: Arc<dyn BankStatements> = Arc::new(forecast.clone());
    let directory_state = TestDirectory::default();
    let directory: Arc<dyn PartyDirectory> = Arc::new(directory_state.clone());
    let notifier: Arc<dyn Notifier> = Arc::new(notices.clone());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(module)
        .module(Probe)
        .provide(directory)
        .provide(recorder)
        .provide(cheque_book)
        .provide(statements)
        .provide(creator)
        .provide(coverage)
        .provide(notifier);
    let app = match poster {
        Some(poster) => app.provide(poster),
        None => app,
    };
    let app = record(app, &issued);
    let app = record(app, &overdue);
    let app = record(app, &paid);
    let app = record(app, &written_off);
    let app = record(app, &reopened);
    let app = record(app, &invoice_reopened);
    let app = record(app, &recovered);
    let app = record(app, &resolved);
    let app = record(app, &disputed);
    let app = record(app, &bill_approved);
    let app = record(app, &bill_voided);
    let app = record(app, &bill_reopened);
    let app = record(app, &budget_approved);
    let app = record(app, &variance_reached);
    let app = record(app, &below_threshold);
    let app = record(app, &drawing_decided);
    let app = record(app, &einvoice_accepted);
    let app = record(app, &einvoice_rejected);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    wl_invoicing::budget_category::install(app.db())
        .await
        .expect("the budget categories are seeded");
    let all = [
        perm::VIEW,
        perm::CREATE,
        perm::ISSUE,
        perm::CREDIT,
        perm::WRITE_OFF,
        perm::DISPUTE,
        perm::RECEIVABLES_VIEW,
        perm::VAT_VIEW,
        perm::VAT_PREPARE,
        perm::VAT_REVIEW,
        perm::VAT_LOCK,
        perm::SUPPLIER_BILLS_VIEW,
        perm::SUPPLIER_BILLS_CREATE,
        perm::SUPPLIER_BILLS_APPROVE,
        perm::BUDGETS_VIEW,
        perm::BUDGETS_MANAGE,
        perm::BUDGETS_APPROVE,
        perm::BUDGET_CATEGORIES_MANAGE,
        perm::CASHFLOW_VIEW,
        perm::CASHFLOW_MANAGE,
    ];
    permissions::define_role(app.db(), "admin", &all)
        .await
        .unwrap();
    // The host's `accountant` role: the VAT permissions and the invoices.
    permissions::define_role(
        app.db(),
        "accountant",
        &[
            perm::VIEW,
            perm::VAT_VIEW,
            perm::VAT_PREPARE,
            perm::VAT_REVIEW,
            perm::VAT_LOCK,
            perm::SUPPLIER_BILLS_VIEW,
            perm::SUPPLIER_BILLS_CREATE,
            perm::SUPPLIER_BILLS_APPROVE,
            perm::BUDGETS_VIEW,
            perm::BUDGETS_MANAGE,
            perm::BUDGET_CATEGORIES_MANAGE,
            perm::CASHFLOW_VIEW,
            perm::CASHFLOW_MANAGE,
        ],
    )
    .await
    .unwrap();
    // The role a seeded category tells (marketing).
    permissions::define_role(app.db(), "marketing_manager", &[perm::VIEW])
        .await
        .unwrap();
    // The host's `finance_manager` role: sees budgets and approves them.
    permissions::define_role(
        app.db(),
        "finance_manager",
        &[perm::VIEW, perm::BUDGETS_VIEW, perm::BUDGETS_APPROVE],
    )
    .await
    .unwrap();
    permissions::define_role(
        app.db(),
        "receivables",
        &[
            perm::VIEW,
            perm::CREATE,
            perm::ISSUE,
            perm::WRITE_OFF,
            perm::DISPUTE,
            perm::RECEIVABLES_VIEW,
        ],
    )
    .await
    .unwrap();
    Booted {
        app,
        _views: views,
        notices,
        issued,
        overdue,
        paid,
        written_off,
        reopened,
        invoice_reopened,
        refunds,
        forecast,
        contracts,
        recovered,
        resolved,
        disputed,
        tickets,
        bill_approved,
        bill_voided,
        bill_reopened,
        budget_approved,
        variance_reached,
        below_threshold,
        drawing_decided,
        einvoice_accepted,
        einvoice_rejected,
        directory: directory_state,
    }
}
