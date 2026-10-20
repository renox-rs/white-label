//! Booting wl-payments in a `TestApp`, as `crates/wl-ticketing/tests/support`
//! boots wl-ticketing: a party directory of its own (the crate does not
//! depend on wl-parties), an `Invoicer` and a `Notifier` that record what
//! they are asked for, the kernel events recorded, the `FakeGateway`, and a
//! test-only module whose routes act as the logged-in user for the refund
//! transitions (a test cannot build an `AuthUser`). Recording and reversing
//! a payment use the module's own routes.

#![allow(dead_code)]

pub mod cash;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::auth::{Permissions, permissions};
use renox::axum::extract::Form;
use renox::chrono::NaiveDate;
use renox::db::sql;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use renox::testing::{TestApp, TestResponse};
use serde::Deserialize;
use wl_kernel::approvals::{ApprovalDecided, ApprovalEnd};
use wl_kernel::cheques::{
    BounceFeeCharge, BounceFees, ChargeInvoicer, ChequeBounceResolved, ChequeBounced, ChequeFuture,
    ChequeReferredToLegal, RegisterCheque, cheques as cheque_contract,
};
use wl_kernel::deposits::{
    DepositAdjusted, DepositFuture, DepositSettled, DepositTopUps, DisputeDeductions, TopUpCharge,
    deposit_deductions,
};
use wl_kernel::custody::{CashHandoverCounted, CashShortage, CashShortageSettled, ShortageRecovery};
use wl_kernel::documents::today;
use wl_kernel::drawings::{
    DrawingCategory, DrawingFuture, DrawingRequest, DrawingStart, OwnerDrawings,
};
use wl_kernel::invoicing::{
    InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, InvoiceWrittenOff, Invoicer,
    WriteOffAnswer,
};
use wl_kernel::tickets::TaskCreator;
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyKind, PartyStatus, PartySummary};
use wl_kernel::payments::{
    AllocationTarget, PaymentId, PaymentMethod, PaymentReceived, PaymentReversed, PaymentSummary,
    RecordPayment, RefundFailed, RefundFailureKind, RefundPaid, payment_recorder,
};
use wl_kernel::{ClientConfig, LegalEntity, PartyId, Subject};
use wl_payments::deposit::HoldDeposit;
use wl_payments::gateway::FakeGateway;
use wl_payments::refund::{RefundRequest, RefundSource, RequestRefund};
use wl_payments::{
    Allocation, Cheque, Deposit, FollowupTask, Payment, Refund, WlPayments, permissions as perm,
};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A client with the payments module (and what it requires) on: two legal
/// entities, so numbers carry the entity code.
pub const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing", "payments"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true

[[legal_entities]]
code = "PGFZ"
name_en = "Palm Grove Facilities FZ-LLC"
"#;

/// Blue Dunes LLC (active).
pub const BLUE_DUNES: PartyId = PartyId(1);
/// The code of Blue Dunes.
pub const BLUE_DUNES_CODE: &str = "P-00001";
/// Quiet Sands LLC (active).
pub const QUIET_SANDS: PartyId = PartyId(2);
/// The code of Quiet Sands.
pub const QUIET_SANDS_CODE: &str = "P-00002";
/// Dusk Holdings LLC (blocked): automatic messages to it are suppressed.
pub const DUSK_HOLDINGS: PartyId = PartyId(3);
/// The code of Dusk Holdings.
pub const DUSK_HOLDINGS_CODE: &str = "P-00003";

/// The parties that pay.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestDirectory;

fn summary(party: PartyId) -> Option<PartySummary> {
    let (code, name) = match party {
        BLUE_DUNES => (BLUE_DUNES_CODE, "Blue Dunes LLC"),
        QUIET_SANDS => (QUIET_SANDS_CODE, "Quiet Sands LLC"),
        DUSK_HOLDINGS => (DUSK_HOLDINGS_CODE, "Dusk Holdings LLC"),
        _ => return None,
    };
    let status = if party == DUSK_HOLDINGS {
        PartyStatus::Blocked
    } else {
        PartyStatus::Active
    };
    Some(PartySummary {
        id: party,
        code: code.to_owned(),
        kind: PartyKind::Company,
        display_name: name.to_owned(),
        status,
        display_block_reason: None,
    })
}

impl PartyDirectory for TestDirectory {
    fn find<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move { Ok(summary(party)) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async move {
            Ok([BLUE_DUNES, QUIET_SANDS, DUSK_HOLDINGS]
                .into_iter()
                .filter_map(summary)
                .find(|party| party.code == code))
        })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async move { Ok(summary(party).map(|p| p.display_name)) })
    }

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async move { Ok(summary(party).map(|p| p.status)) })
    }
}

/// One call of `Invoicer::allocate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocateCall {
    /// The invoice.
    pub invoice: InvoiceId,
    /// The source (the payment).
    pub source: Subject,
    /// The amount, in fils.
    pub amount: i64,
}

#[derive(Default)]
struct Ledger {
    invoices: Vec<InvoiceSummary>,
    allocations: Vec<AllocateCall>,
    drafts: usize,
    /// Whether a write-off waits for an approval (above the limit).
    approval: bool,
    /// The write-offs asked for, with the approval request when one waits.
    write_offs: Vec<(InvoiceId, Option<i64>)>,
}

/// An `Invoicer` that holds invoices in memory and records every call.
#[derive(Clone, Default)]
pub struct RecordingInvoicer(Arc<Mutex<Ledger>>);

impl RecordingInvoicer {
    /// Adds an issued invoice of `party` with `balance` fils open (AED).
    pub fn add(&self, party: PartyId, number: &str, balance: i64) -> InvoiceId {
        let mut ledger = self.0.lock().unwrap();
        let id = InvoiceId(ledger.invoices.len() as i64 + 1);
        ledger.invoices.push(InvoiceSummary {
            id,
            number: Some(number.to_owned()),
            status: InvoiceStatus::Issued,
            party,
            legal_entity_id: None,
            currency: "AED".to_owned(),
            gross_total: balance,
            balance,
            due_date: None,
            disputed: false,
            chasing_paused_until: None,
        });
        id
    }

    /// Every successful `allocate` call so far, oldest first.
    pub fn calls(&self) -> Vec<AllocateCall> {
        self.0.lock().unwrap().allocations.clone()
    }

    /// How many invoices were drafted.
    pub fn drafts(&self) -> usize {
        self.0.lock().unwrap().drafts
    }

    /// Whether the next write-offs wait for an approval, as above the
    /// `FIN.ACL.15` limit (the default: they are made at once).
    pub fn require_approval(&self, on: bool) {
        self.0.lock().unwrap().approval = on;
    }

    /// The invoices a write-off was asked for, oldest first.
    pub fn write_offs(&self) -> Vec<InvoiceId> {
        self.0
            .lock()
            .unwrap()
            .write_offs
            .iter()
            .map(|(id, _)| *id)
            .collect()
    }

    /// The approval request of the write-off asked for `id`, if one waits.
    pub fn request_of(&self, id: InvoiceId) -> Option<i64> {
        self.0
            .lock()
            .unwrap()
            .write_offs
            .iter()
            .rev()
            .find(|(invoice, _)| *invoice == id)
            .and_then(|(_, request)| *request)
    }

    /// Flags the invoice `id` as disputed.
    pub fn set_disputed(&self, id: InvoiceId, disputed: bool) {
        let mut ledger = self.0.lock().unwrap();
        if let Some(invoice) = ledger.invoices.iter_mut().find(|i| i.id == id) {
            invoice.disputed = disputed;
        }
    }

    /// Sets the balance of the invoice `id`, as when the party pays.
    pub fn set_balance(&self, id: InvoiceId, balance: i64) {
        let mut ledger = self.0.lock().unwrap();
        if let Some(invoice) = ledger.invoices.iter_mut().find(|i| i.id == id) {
            invoice.balance = balance;
        }
    }

    /// Marks the invoice `id` written off; its summary, if it exists.
    fn mark_written_off(&self, id: InvoiceId) -> Option<InvoiceSummary> {
        let mut ledger = self.0.lock().unwrap();
        let invoice = ledger.invoices.iter_mut().find(|i| i.id == id)?;
        invoice.status = InvoiceStatus::WrittenOff;
        Some(invoice.clone())
    }

    /// The status of the invoice `id`.
    pub fn status_of(&self, id: InvoiceId) -> Option<InvoiceStatus> {
        self.0
            .lock()
            .unwrap()
            .invoices
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.status)
    }
}

impl Invoicer for RecordingInvoicer {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        _draft: wl_kernel::invoicing::InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        self.0.lock().unwrap().drafts += 1;
        Box::pin(async { Ok(None) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .invoices
            .iter()
            .find(|i| i.id == id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        id: InvoiceId,
        source: Subject,
        amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        let mut ledger = self.0.lock().unwrap();
        let result = match ledger.invoices.iter_mut().find(|i| i.id == id) {
            Some(invoice) if amount > 0 && amount <= invoice.balance => {
                invoice.balance -= amount;
                Ok(())
            }
            Some(_) => Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "The amount is above the balance.".to_owned(),
            )),
            None => Err(Error::NotFound),
        };
        if result.is_ok() {
            ledger.allocations.push(AllocateCall {
                invoice: id,
                source,
                amount,
            });
        }
        Box::pin(async move { result })
    }

    fn write_off<'a>(
        &'a self,
        state: &'a AppState,
        _by: &'a AuthUser,
        id: InvoiceId,
        _reason_code: &'a str,
        _reason_text: Option<&'a str>,
    ) -> InvoiceFuture<'a, WriteOffAnswer> {
        Box::pin(async move {
            let (refusal, approval, request) = {
                let mut ledger = self.0.lock().unwrap();
                let found = ledger.invoices.iter().find(|i| i.id == id).cloned();
                let refusal = match found {
                    None => Some("The invoice was not found.".to_owned()),
                    Some(i) if i.disputed => Some("The invoice is disputed.".to_owned()),
                    Some(i) if i.balance <= 0 || !i.status.is_open() => {
                        Some("The invoice has no balance to write off.".to_owned())
                    }
                    Some(_) => None,
                };
                let approval = ledger.approval;
                let request = (refusal.is_none() && approval).then(|| {
                    ledger
                        .write_offs
                        .iter()
                        .filter(|(_, r)| r.is_some())
                        .count() as i64
                        + 1
                });
                if refusal.is_none() {
                    ledger.write_offs.push((id, request));
                }
                (refusal, approval, request)
            };
            if let Some(message) = refusal {
                return Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message));
            }
            if approval {
                return Ok(WriteOffAnswer::Pending {
                    request: request.unwrap_or_default(),
                });
            }
            let summary = self.mark_written_off(id).expect("the invoice exists");
            state.emit(written_off_event(&summary)).await?;
            Ok(WriteOffAnswer::WrittenOff)
        })
    }
}

fn written_off_event(invoice: &InvoiceSummary) -> InvoiceWrittenOff {
    InvoiceWrittenOff {
        invoice: invoice.id,
        party: invoice.party,
        source: Subject::new("property.lease", 9),
        amount: invoice.balance,
        reason_code: "other".to_owned(),
        approver: None,
    }
}

/// Charges that can be invoiced: `register` a charge, and `invoice` makes an
/// issued invoice of its amount in the shared ledger.
#[derive(Clone)]
pub struct RecordingCharges {
    invoices: RecordingInvoicer,
    charges: Arc<Mutex<Vec<(i64, PartyId, i64)>>>,
}

impl RecordingCharges {
    /// Knows the charge `id` of `party` for `amount` fils.
    pub fn register(&self, id: i64, party: PartyId, amount: i64) {
        self.charges.lock().unwrap().push((id, party, amount));
    }
}

impl ChargeInvoicer for RecordingCharges {
    fn invoice<'a>(
        &'a self,
        _state: &'a AppState,
        charge: &'a Subject,
    ) -> ChequeFuture<'a, Option<InvoiceId>> {
        let known = self
            .charges
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _, _)| *id == charge.subject_id)
            .copied();
        let invoice = known
            .map(|(id, party, amount)| self.invoices.add(party, &format!("INV-CHG-{id}"), amount));
        Box::pin(async move { Ok(invoice) })
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

    /// The requests of one notification type, oldest first.
    pub fn of_type(&self, type_key: &str) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|r| r.type_key == type_key)
            .collect()
    }

    /// The recipients of every request of `type_key`, oldest first.
    pub fn recipients(&self, type_key: &str) -> Vec<NotifyRecipient> {
        self.of_type(type_key)
            .into_iter()
            .flat_map(|r| r.recipients)
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

/// A [`BounceFees`] that records every charge it is asked for and makes it
/// (a charge of the next number) unless told not to.
#[derive(Clone)]
pub struct RecordingFees {
    charges: Arc<Mutex<Vec<BounceFeeCharge>>>,
    makes: Arc<Mutex<bool>>,
}

impl Default for RecordingFees {
    fn default() -> Self {
        RecordingFees {
            charges: Arc::new(Mutex::new(Vec::new())),
            makes: Arc::new(Mutex::new(true)),
        }
    }
}

impl RecordingFees {
    /// The charges asked for, oldest first.
    pub fn charges(&self) -> Vec<BounceFeeCharge> {
        self.charges.lock().unwrap().clone()
    }

    /// Whether the next charges are made (the default) or left out, as when
    /// invoicing cannot schedule charges.
    pub fn make_charges(&self, makes: bool) {
        *self.makes.lock().unwrap() = makes;
    }
}

impl BounceFees for RecordingFees {
    fn add<'a>(
        &'a self,
        _state: &'a AppState,
        charge: BounceFeeCharge,
    ) -> ChequeFuture<'a, Option<Subject>> {
        let mut charges = self.charges.lock().unwrap();
        charges.push(charge);
        let made = *self.makes.lock().unwrap();
        let number = charges.len() as i64;
        Box::pin(async move { Ok(made.then(|| Subject::new("invoicing.charge", number))) })
    }
}

/// A [`DepositTopUps`] that records every charge it is asked for and makes it
/// (a charge of the next number) unless told not to.
#[derive(Clone)]
pub struct RecordingTopUps {
    charges: Arc<Mutex<Vec<TopUpCharge>>>,
    makes: Arc<Mutex<bool>>,
}

impl Default for RecordingTopUps {
    fn default() -> Self {
        RecordingTopUps {
            charges: Arc::new(Mutex::new(Vec::new())),
            makes: Arc::new(Mutex::new(true)),
        }
    }
}

impl RecordingTopUps {
    /// The charges asked for, oldest first.
    pub fn charges(&self) -> Vec<TopUpCharge> {
        self.charges.lock().unwrap().clone()
    }

    /// Whether the next charges are made (the default) or left out, as when
    /// nothing can schedule charges.
    pub fn make_charges(&self, makes: bool) {
        *self.makes.lock().unwrap() = makes;
    }
}

impl DepositTopUps for RecordingTopUps {
    fn request<'a>(
        &'a self,
        _state: &'a AppState,
        charge: TopUpCharge,
    ) -> DepositFuture<'a, Option<Subject>> {
        let mut charges = self.charges.lock().unwrap();
        charges.push(charge);
        let made = *self.makes.lock().unwrap();
        let number = charges.len() as i64;
        Box::pin(async move { Ok(made.then(|| Subject::new("invoicing.charge", 100 + number))) })
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

/// The owner drawings approval wl-invoicing provides, scripted by the test: the
/// category `owner_drawings` has an owner party (Quiet Sands unless changed),
/// and a request waits for the approvers unless the desk approves at once.
#[derive(Clone)]
pub struct DrawingsDesk(Arc<Mutex<DeskState>>);

struct DeskState {
    owner: Option<PartyId>,
    start: DrawingStart,
    asked: Vec<DrawingRequest>,
}

impl Default for DrawingsDesk {
    fn default() -> Self {
        DrawingsDesk(Arc::new(Mutex::new(DeskState {
            owner: Some(QUIET_SANDS),
            start: DrawingStart::Pending,
            asked: Vec::new(),
        })))
    }
}

impl DrawingsDesk {
    /// Sets the owner party of the category (`None`: it has none yet).
    pub fn owner(&self, party: Option<PartyId>) {
        self.0.lock().unwrap().owner = party;
    }

    /// From now on a request is approved at once (the sole owner,
    /// `FIN.ACL.23`), flagged or not.
    pub fn approves_at_once(&self, self_approved: bool) {
        self.0.lock().unwrap().start = DrawingStart::Approved { self_approved };
    }

    /// From now on a request waits for the approvers.
    pub fn waits(&self) {
        self.0.lock().unwrap().start = DrawingStart::Pending;
    }

    /// The requests asked so far, oldest first.
    pub fn asked(&self) -> Vec<DrawingRequest> {
        self.0.lock().unwrap().asked.clone()
    }
}

impl OwnerDrawings for DrawingsDesk {
    fn category<'a>(
        &'a self,
        _db: &'a Db,
        code: &'a str,
    ) -> DrawingFuture<'a, Option<DrawingCategory>> {
        Box::pin(async move {
            Ok((code == "owner_drawings").then(|| DrawingCategory {
                code: code.to_owned(),
                name_en: "Owner drawings".to_owned(),
                owner_party: self.0.lock().unwrap().owner,
            }))
        })
    }

    fn request<'a>(
        &'a self,
        _state: &'a AppState,
        request: DrawingRequest,
    ) -> DrawingFuture<'a, DrawingStart> {
        Box::pin(async move {
            let mut desk = self.0.lock().unwrap();
            desk.asked.push(request);
            Ok(desk.start)
        })
    }
}

/// What the refund routes take.
#[derive(Deserialize)]
struct RefundForm {
    #[serde(default)]
    payment: Option<i64>,
    #[serde(default)]
    party: Option<i64>,
    #[serde(default)]
    entity: Option<i64>,
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    reference: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    source_type: Option<String>,
    #[serde(default)]
    source_id: Option<i64>,
    #[serde(default)]
    gateway_txn: Option<String>,
    #[serde(default)]
    details: Option<String>,
}

/// What the deposit routes take.
#[derive(Deserialize)]
struct DepositForm {
    #[serde(default)]
    party: Option<i64>,
    #[serde(default)]
    entity: Option<i64>,
    #[serde(default)]
    source_type: Option<String>,
    #[serde(default)]
    source_id: Option<i64>,
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    payment: Option<i64>,
    #[serde(default)]
    method: Option<String>,
}

fn method_of(code: Option<&str>) -> Result<Option<PaymentMethod>> {
    match code.filter(|c| !c.is_empty()) {
        None => Ok(None),
        Some(code) => PaymentMethod::ALL
            .iter()
            .copied()
            .find(|m| m.as_str() == code)
            .map(Some)
            .ok_or(Error::NotFound),
    }
}

async fn refund_request_for_source(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<RefundForm>,
) -> Result<String> {
    let subject = Subject::new(
        form.source_type.as_deref().unwrap_or_default(),
        form.source_id.ok_or(Error::NotFound)?,
    );
    let source = wl_kernel::payments::RefundSource::from_kind(
        form.source_kind.as_deref().unwrap_or_default(),
        &subject,
    )
    .ok_or(Error::NotFound)?;
    let request = RefundRequest {
        source,
        party: PartyId(form.party.ok_or(Error::NotFound)?),
        legal_entity_id: form.entity.ok_or(Error::NotFound)?,
        amount: form.amount.unwrap_or_default(),
        currency: "AED".to_owned(),
        method: method_of(form.method.as_deref())?,
        gateway_txn: form.gateway_txn,
        reason: form.reason.unwrap_or_default(),
    };
    let refund = wl_payments::refund::request_for_source(&state, &request, &user)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(refund.id.to_string())
}

async fn deposit_hold(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<DepositForm>,
) -> Result<String> {
    let request = HoldDeposit {
        party: PartyId(form.party.ok_or(Error::NotFound)?),
        legal_entity_id: form.entity.ok_or(Error::NotFound)?,
        source: Subject::new(
            form.source_type.as_deref().unwrap_or_default(),
            form.source_id.ok_or(Error::NotFound)?,
        ),
        amount: form.amount.unwrap_or_default(),
        currency: "AED".to_owned(),
        received_payment: form.payment.map(PaymentId),
    };
    let deposit = wl_payments::deposit::hold(&state, &user, &request)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(deposit.id.to_string())
}

async fn deposit_act(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, action)): Path<(i64, String)>,
    Form(form): Form<DepositForm>,
) -> Result<String> {
    let deposit = Deposit::find_or_404(&state.db, id).await?;
    let done = match action.as_str() {
        "start_settlement" => {
            wl_payments::deposit::start_settlement(&state, Some(&user), &deposit).await
        }
        "refund" => {
            let method = method_of(form.method.as_deref())?;
            return match wl_payments::deposit::refund_or_ask(&state, &user, &deposit, method)
                .await
                .map_err(TransitionError::into_error)?
            {
                wl_payments::deposit::RefundOutcome::Refunded(done) => {
                    Ok(format!("{:?}", done.status))
                }
                wl_payments::deposit::RefundOutcome::ExceptionRequested(request) => {
                    Ok(format!("exception:{}", request.id))
                }
            };
        }
        _ => return Err(Error::NotFound),
    };
    done.map(|d| format!("{:?}", d.status))
        .map_err(TransitionError::into_error)
}

async fn refund_request(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<RefundForm>,
) -> Result<String> {
    let source = match form.payment {
        Some(id) => RefundSource::Payment(PaymentId(id)),
        None => RefundSource::PartyCredit {
            party: PartyId(form.party.ok_or(Error::NotFound)?),
            legal_entity_id: form.entity.ok_or(Error::NotFound)?,
            currency: "AED".to_owned(),
        },
    };
    let request = RequestRefund {
        source,
        amount: form.amount.unwrap_or_default(),
        reason: form.reason.unwrap_or_default(),
    };
    let refund = wl_payments::refund::request(&state, &user, &request)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(refund.id.to_string())
}

async fn refund_act(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, action)): Path<(i64, String)>,
    Form(form): Form<RefundForm>,
) -> Result<String> {
    let refund = Refund::find_or_404(&state.db, id).await?;
    let done = match action.as_str() {
        "approve" => wl_payments::refund::approve(&state, &user, &refund).await,
        "reject" => {
            let reason = form.reason.unwrap_or_default();
            wl_payments::refund::reject(&state, &user, &refund, &reason).await
        }
        "pay" => {
            let code = form.method.unwrap_or_default();
            let method = PaymentMethod::ALL
                .iter()
                .copied()
                .find(|m| m.as_str() == code)
                .ok_or(Error::NotFound)?;
            let reference = form.reference.unwrap_or_default();
            wl_payments::refund::pay(&state, &user, &refund, method, &reference).await
        }
        "send" => {
            return wl_payments::refund::send_to_gateway(&state, &user, &refund)
                .await
                .map(|attempt| format!("attempt {}", attempt.number))
                .map_err(TransitionError::into_error);
        }
        "fail" => {
            let code = form.kind.unwrap_or_default();
            let kind = RefundFailureKind::ALL
                .iter()
                .copied()
                .find(|k| k.as_str() == code)
                .ok_or(Error::NotFound)?;
            let reason = form.reason.unwrap_or_default();
            wl_payments::refund::fail(&state, &user, &refund, kind, &reason).await
        }
        "retry" => wl_payments::refund_recover::retry(&state, &user, &refund).await,
        "change_method" => {
            let method = method_of(form.method.as_deref())?.ok_or(Error::NotFound)?;
            wl_payments::refund_recover::change_method(
                &state,
                &user,
                &refund,
                method,
                form.details.as_deref(),
            )
            .await
        }
        "cancel" => {
            let reason = form.reason.unwrap_or_default();
            wl_payments::refund_recover::cancel(&state, &user, &refund, &reason).await
        }
        _ => return Err(Error::NotFound),
    };
    done.map(|r| format!("{:?}", r.status))
        .map_err(TransitionError::into_error)
}

/// The parties the block guard of the test refuses (`PTY.ACL.03`).
#[derive(Clone, Default)]
pub struct BlockedParties(Arc<Mutex<Vec<i64>>>);

impl BlockedParties {
    /// The party is on hold from now on.
    pub fn block(&self, party: PartyId) {
        self.0.lock().unwrap().push(party.0);
    }

    /// The party is free again.
    pub fn free(&self, party: PartyId) {
        self.0.lock().unwrap().retain(|id| *id != party.0);
    }
}

impl wl_kernel::block_guard::BlockGuard for BlockedParties {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        _viewer: Option<&'a User>,
        party: PartyId,
        kind: wl_kernel::block_guard::ActionKind,
    ) -> wl_kernel::block_guard::BlockFuture<'a, wl_kernel::block_guard::GuardOutcome> {
        Box::pin(async move {
            let blocked = kind.is_guarded() && self.0.lock().unwrap().contains(&party.0);
            Ok(if blocked {
                wl_kernel::block_guard::GuardOutcome::Blocked(
                    wl_kernel::block_guard::BlockRefusal::neutral(party, kind, "Compliance"),
                )
            } else {
                wl_kernel::block_guard::GuardOutcome::Allowed
            })
        })
    }

    fn exception_step(&self) -> wl_kernel::block_guard::ExceptionStep {
        wl_kernel::block_guard::ExceptionStep::new("compliance")
    }
}

/// The test-only routes.
struct Harness;

impl Module for Harness {
    fn name(&self) -> &'static str {
        "payments-test-harness"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/test/refunds", refund_request)
            .post("/test/refunds/source", refund_request_for_source)
            .post("/test/deposits", deposit_hold)
            .post("/test/deposits/{id}/{action}", deposit_act)
            .post("/test/refunds/{id}/{action}", refund_act)
    }
}

/// The app under test with what it emitted and was asked.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
    /// The notices asked of the kernel's notifier.
    pub notices: Recording,
    /// The invoices and the calls made to the invoicer.
    pub invoices: RecordingInvoicer,
    /// The payment gateway.
    pub gateway: FakeGateway,
    /// `PaymentReceived`.
    pub received: Recorded<PaymentReceived>,
    /// `PaymentReversed`.
    pub reversed: Recorded<PaymentReversed>,
    /// `RefundPaid`.
    pub refund_paid: Recorded<RefundPaid>,
    /// `RefundFailed`.
    pub refund_failed: Recorded<RefundFailed>,
    /// `ChequeBounced`.
    pub cheque_bounced: Recorded<ChequeBounced>,
    /// `ChequeBounceResolved`.
    pub cheque_resolved: Recorded<ChequeBounceResolved>,
    /// `ChequeReferredToLegal`.
    pub cheque_referred: Recorded<ChequeReferredToLegal>,
    /// `DepositSettled`.
    pub deposit_settled: Recorded<DepositSettled>,
    /// `DepositAdjusted`.
    pub deposit_adjusted: Recorded<DepositAdjusted>,
    /// The parties on hold (`PTY.ACL.03`).
    pub blocked: BlockedParties,
    /// The top-up charges asked for.
    pub top_ups: RecordingTopUps,
    /// The bounce fee charges asked of invoicing.
    pub fees: RecordingFees,
    /// The charges that can be invoiced.
    pub charges: RecordingCharges,
    /// The owner drawings approval.
    pub drawings: DrawingsDesk,
    /// The tickets asked of the task creator.
    pub cash_tickets: cash::CashTickets,
    /// The salary deductions passed to wl-hr.
    pub hr: cash::HrDesk,
    /// `CashShortage`.
    pub cash_shortage: Recorded<CashShortage>,
    /// `CashHandoverCounted`.
    pub cash_counted: Recorded<CashHandoverCounted>,
    /// `CashShortageSettled`.
    pub cash_settled: Recorded<CashShortageSettled>,
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

    /// A user with every payments permission.
    pub async fn staff(&self, name: &str) -> User {
        self.user(
            name,
            &[
                perm::VIEW,
                perm::RECORD,
                perm::REVERSE,
                perm::REFUNDS_REQUEST,
                perm::REFUNDS_APPROVE,
                perm::REFUNDS_PAY,
                perm::DEPOSITS_VIEW,
                perm::DEPOSITS_MANAGE,
                perm::DEPOSITS_REFUND,
            ],
        )
        .await
    }

    /// User A.
    pub async fn user_a(&self) -> User {
        self.staff("Amal").await
    }

    /// User B.
    pub async fn user_b(&self) -> User {
        self.staff("Bilal").await
    }

    /// A user of the `accountant` role (the ones follow-up notices go to).
    pub async fn accountant(&self, name: &str) -> User {
        let user = User::register(
            self.db(),
            name,
            &format!("{}@example.com", name.to_lowercase()),
            "password123",
        )
        .await
        .unwrap();
        permissions::define_role(self.db(), "accountant", &[perm::VIEW, perm::RECORD])
            .await
            .unwrap();
        user.assign_role(self.db(), "accountant").await.unwrap();
        user
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

    /// Records an incoming transfer of `amount` fils from `party` to PGP,
    /// through the kernel contract.
    pub async fn receive(&self, party: PartyId, amount: i64) -> Payment {
        let entity = self.entity("PGP").await;
        let mut request = RecordPayment::new(party, entity.id, PaymentMethod::Transfer, amount);
        request.reference = Some("TRF-1".to_owned());
        self.receive_request(request).await
    }

    /// Records `request` through the kernel contract.
    pub async fn receive_request(&self, request: RecordPayment) -> Payment {
        let summary = payment_recorder(self.state())
            .record(self.state(), request, None)
            .await
            .expect("the payment is recorded");
        self.payment(summary.id.0).await
    }

    /// The payment `id`, as stored now.
    pub async fn payment(&self, id: i64) -> Payment {
        Payment::find_or_404(self.db(), id).await.unwrap()
    }

    /// `payment`, as stored now.
    pub async fn reload(&self, payment: &Payment) -> Payment {
        self.payment(payment.id).await
    }

    /// The summary the contract gives for `payment`.
    pub async fn summary(&self, payment: &Payment) -> PaymentSummary {
        payment_recorder(self.state())
            .find(self.db(), payment.payment_id())
            .await
            .unwrap()
            .expect("the payment exists")
    }

    /// Allocates `amount` fils of `payment` to `invoice`.
    pub async fn allocate(
        &self,
        payment: &Payment,
        invoice: InvoiceId,
        amount: i64,
    ) -> Result<Allocation> {
        wl_payments::allocate::allocate(
            self.state(),
            payment.payment_id(),
            AllocationTarget::Invoice(invoice),
            amount,
            None,
        )
        .await
    }

    /// Every allocation row of `payment`, removed ones too.
    pub async fn allocations(&self, payment: &Payment) -> Vec<Allocation> {
        Allocation::all_of(self.db(), payment.id).await.unwrap()
    }

    /// The follow-up tasks of `payment`.
    pub async fn tasks(&self, payment: &Payment) -> Vec<FollowupTask> {
        FollowupTask::of_payment(self.db(), payment.id)
            .await
            .unwrap()
    }

    /// `by` reverses `payment` through the module's route.
    pub async fn reverse(
        &self,
        by: &User,
        payment: &Payment,
        reason: &str,
        note: &str,
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!("/payments/{}/reverse", payment.id),
                &[("reason", reason), ("note", note)],
            )
            .await
    }

    /// `by` asks to refund `amount` fils out of `payment`; the refund.
    pub async fn request_refund(&self, by: &User, payment: &Payment, amount: i64) -> Refund {
        self.app.acting_as(by);
        let payment_id = payment.id.to_string();
        let amount = amount.to_string();
        let response = self
            .app
            .htmx()
            .post(
                "/test/refunds",
                &[
                    ("payment", payment_id.as_str()),
                    ("amount", amount.as_str()),
                    ("reason", "Duplicate payment"),
                ],
            )
            .await;
        response.assert_ok();
        let id: i64 = response.text().trim().parse().expect("a refund id");
        self.refund(id).await
    }

    /// `by` runs `action` (`approve`, `reject` or `pay`) on `refund`.
    pub async fn refund_action(
        &self,
        by: &User,
        refund: &Refund,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/test/refunds/{}/{action}", refund.id), pairs)
            .await
    }

    /// `by` asks, for the owner of a source outside this module, to refund
    /// `amount` fils of PGP to `party` out of `source_kind` `source`
    /// (`method` and `gateway_txn` as given); the refund.
    pub async fn request_for_source(
        &self,
        by: &User,
        party: PartyId,
        source: &wl_kernel::payments::RefundSource,
        amount: i64,
        method: &str,
        gateway_txn: &str,
    ) -> Refund {
        let entity = self.entity("PGP").await;
        self.app.acting_as(by);
        let party = party.0.to_string();
        let entity = entity.id.to_string();
        let subject = source.subject();
        let source_id = subject.subject_id.to_string();
        let amount = amount.to_string();
        let response = self
            .app
            .htmx()
            .post(
                "/test/refunds/source",
                &[
                    ("source_kind", source.kind()),
                    ("source_type", subject.subject_type.as_str()),
                    ("source_id", source_id.as_str()),
                    ("party", party.as_str()),
                    ("entity", entity.as_str()),
                    ("amount", amount.as_str()),
                    ("method", method),
                    ("gateway_txn", gateway_txn),
                    ("reason", "Order cancelled"),
                ],
            )
            .await;
        response.assert_ok();
        let id: i64 = response.text().trim().parse().expect("a refund id");
        self.refund(id).await
    }

    /// `by` fails `refund` with the kind code `kind` and `reason`.
    pub async fn fail_refund(
        &self,
        by: &User,
        refund: &Refund,
        kind: &str,
        reason: &str,
    ) -> TestResponse {
        self.refund_action(by, refund, "fail", &[("kind", kind), ("reason", reason)])
            .await
    }

    /// `by` holds a deposit of `amount` fils of `party` for the lease 9,
    /// funded by `payment` when given; the deposit.
    pub async fn hold_deposit(
        &self,
        by: &User,
        party: PartyId,
        amount: i64,
        payment: Option<&Payment>,
    ) -> Deposit {
        let response = self.hold_deposit_response(by, party, amount, payment).await;
        response.assert_ok();
        let id: i64 = response.text().trim().parse().expect("a deposit id");
        self.deposit(id).await
    }

    /// The answer of the hold route for the same request.
    pub async fn hold_deposit_response(
        &self,
        by: &User,
        party: PartyId,
        amount: i64,
        payment: Option<&Payment>,
    ) -> TestResponse {
        let entity = self.entity("PGP").await;
        self.app.acting_as(by);
        let party = party.0.to_string();
        let entity = entity.id.to_string();
        let amount = amount.to_string();
        let payment = payment.map(|p| p.id.to_string());
        let mut pairs = vec![
            ("party", party.as_str()),
            ("entity", entity.as_str()),
            ("source_type", "property.lease"),
            ("source_id", "9"),
            ("amount", amount.as_str()),
        ];
        if let Some(payment) = payment.as_deref() {
            pairs.push(("payment", payment));
        }
        self.app.htmx().post("/test/deposits", &pairs).await
    }

    /// `by` runs `action` (`start_settlement` or `refund`) on `deposit`.
    pub async fn deposit_action(
        &self,
        by: &User,
        deposit: &Deposit,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/test/deposits/{}/{action}", deposit.id), pairs)
            .await
    }

    /// The deposit `id`, as stored now.
    pub async fn deposit(&self, id: i64) -> Deposit {
        Deposit::find_or_404(self.db(), id).await.unwrap()
    }

    /// The refund `id`, as stored now.
    pub async fn refund(&self, id: i64) -> Refund {
        Refund::find_or_404(self.db(), id).await.unwrap()
    }

    /// A user with every cheque permission (and the payments ones to view).
    pub async fn cheque_staff(&self, name: &str) -> User {
        self.user(
            name,
            &[
                perm::VIEW,
                perm::CHEQUES_VIEW,
                perm::CHEQUES_DEPOSIT,
                perm::CHEQUES_MANAGE,
                perm::CHEQUES_LEGAL,
            ],
        )
        .await
    }

    /// Registers `request` through the kernel contract; the cheque.
    pub async fn register_cheque(&self, request: RegisterCheque) -> Cheque {
        let summary = cheque_contract(self.state())
            .register(self.state(), request, None)
            .await
            .expect("the cheque is registered");
        self.cheque(summary.id.0).await
    }

    /// A received cheque of `amount` fils of `party`, dated today, paying
    /// `invoice` when given.
    pub async fn received_cheque(
        &self,
        party: PartyId,
        number: &str,
        amount: i64,
        invoice: Option<InvoiceId>,
    ) -> Cheque {
        let entity = self.entity("PGP").await;
        let mut request = RegisterCheque::new(party, entity.id, "ENBD", number, today(), amount);
        if let Some(invoice) = invoice {
            request = request.link(invoice.subject());
        }
        self.register_cheque(request).await
    }

    /// The cheque `id`, as stored now.
    pub async fn cheque(&self, id: i64) -> Cheque {
        Cheque::find_or_404(self.db(), id).await.unwrap()
    }

    /// `by` posts `action` (`deposit`, `clear`, `bounce`, ...) to the page
    /// route of `cheque`.
    pub async fn cheque_action(
        &self,
        by: &User,
        cheque: &Cheque,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/cheques/{}/{action}", cheque.id), pairs)
            .await
    }

    /// `by` deposits `cheque`; the cheque.
    pub async fn deposit_cheque(&self, by: &User, cheque: &Cheque) -> Cheque {
        self.cheque_action(by, cheque, "deposit", &[])
            .await
            .assert_ok();
        self.cheque(cheque.id).await
    }

    /// `by` records the bounce of the deposited `cheque`.
    pub async fn bounce_cheque(
        &self,
        by: &User,
        cheque: &Cheque,
        reason: &str,
        certificate: &str,
        honoured: &str,
    ) -> TestResponse {
        self.cheque_action(
            by,
            cheque,
            "bounce",
            &[
                ("reason", reason),
                ("certificate", certificate),
                ("honoured", honoured),
            ],
        )
        .await
    }

    /// `by` presents the bounced `cheque` again on `date` (`2027-02-05`),
    /// with the party's confirmation of funds when `funds_confirmed`, and
    /// `note`.
    pub async fn represent_cheque(
        &self,
        by: &User,
        cheque: &Cheque,
        date: &str,
        funds_confirmed: bool,
        note: &str,
    ) -> TestResponse {
        let mut pairs = vec![("date", date), ("note", note)];
        if funds_confirmed {
            pairs.push(("funds_confirmed", "on"));
        }
        self.cheque_action(by, cheque, "represent", &pairs).await
    }

    /// A received cheque of `amount` fils paying `invoice`, dated today,
    /// deposited by `by` and bounced for "insufficient funds" with the
    /// certificate "CERT-1" and `honoured` dirhams paid (`""` for nothing);
    /// the cheque as stored after the bounce.
    pub async fn bounced_cheque(
        &self,
        by: &User,
        party: PartyId,
        number: &str,
        amount: i64,
        invoice: Option<InvoiceId>,
        honoured: &str,
    ) -> Cheque {
        let cheque = self.received_cheque(party, number, amount, invoice).await;
        let cheque = self.deposit_cheque(by, &cheque).await;
        self.bounce_cheque(by, &cheque, "insufficient_funds", "CERT-1", honoured)
            .await
            .assert_ok();
        self.cheque(cheque.id).await
    }

    /// `by` records a part payment of `amount` dirhams (`"4000.00"`) by
    /// `method` towards the bounced `cheque`.
    pub async fn settle_cheque(
        &self,
        by: &User,
        cheque: &Cheque,
        method: &str,
        amount: &str,
        reference: &str,
    ) -> TestResponse {
        self.cheque_action(
            by,
            cheque,
            "settle",
            &[
                ("method", method),
                ("amount", amount),
                ("reference", reference),
            ],
        )
        .await
    }

    /// `by` links the cheque or payment `id` (`kind` is `cheque` or
    /// `payment`) as a replacement of the bounced `cheque`.
    pub async fn link_replacement(
        &self,
        by: &User,
        cheque: &Cheque,
        kind: &str,
        id: i64,
    ) -> TestResponse {
        let id = id.to_string();
        self.cheque_action(
            by,
            cheque,
            "replacements",
            &[("kind", kind), ("ref_id", id.as_str())],
        )
        .await
    }

    /// `by` marks the bounced `cheque` replaced.
    pub async fn replace_cheque(&self, by: &User, cheque: &Cheque) -> TestResponse {
        self.cheque_action(by, cheque, "replace", &[]).await
    }

    /// `by` refers the bounced `cheque` to a legal case (`case_type` is
    /// `civil_execution` or `criminal_complaint`), owned by user `owner`
    /// when given.
    pub async fn refer_cheque(
        &self,
        by: &User,
        cheque: &Cheque,
        case_type: &str,
        owner: Option<i64>,
    ) -> TestResponse {
        let owner = owner.map(|id| id.to_string());
        let mut pairs = vec![("case_type", case_type)];
        if let Some(owner) = owner.as_deref() {
            pairs.push(("case_owner", owner));
        }
        self.cheque_action(by, cheque, "refer", &pairs).await
    }

    /// A user of the `finance_manager` role (told of a legal referral).
    pub async fn finance_manager(&self, name: &str) -> User {
        let user = User::register(
            self.db(),
            name,
            &format!("{}@example.com", name.to_lowercase()),
            "password123",
        )
        .await
        .unwrap();
        permissions::define_role(self.db(), "finance_manager", &[perm::VIEW])
            .await
            .unwrap();
        user.assign_role(self.db(), "finance_manager")
            .await
            .unwrap();
        user
    }

    /// A deposit of `amount` fils of `party`, held with a received payment
    /// and `settlement_pending`.
    pub async fn settling_deposit(&self, by: &User, party: PartyId, amount: i64) -> Deposit {
        let payment = self.receive(party, amount).await;
        let deposit = self.hold_deposit(by, party, amount, Some(&payment)).await;
        self.deposit_action(by, &deposit, "start_settlement", &[])
            .await
            .assert_ok();
        self.deposit(deposit.id).await
    }

    /// `by` posts to the page route `action` of `deposit`.
    pub async fn deposit_page(
        &self,
        by: &User,
        deposit: &Deposit,
        action: &str,
        pairs: &[(&str, &str)],
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/deposits/{}/{action}", deposit.id), pairs)
            .await
    }

    /// `by` publishes the deductions `lines` (`description | amount |
    /// evidence` per row), or the forfeiture when `forfeiture` is not empty.
    pub async fn publish_deductions(
        &self,
        by: &User,
        deposit: &Deposit,
        lines: &str,
        forfeiture: &str,
    ) -> TestResponse {
        self.deposit_page(
            by,
            deposit,
            "deductions",
            &[("lines", lines), ("forfeiture_reason", forfeiture)],
        )
        .await
    }

    /// `by` answers the open disputes of `deposit`; `changes` has a
    /// `line id | new amount` per row (empty: upheld).
    pub async fn answer_dispute(
        &self,
        by: &User,
        deposit: &Deposit,
        changes: &str,
    ) -> TestResponse {
        self.deposit_page(
            by,
            deposit,
            "answer",
            &[
                ("response", "The inspection report shows the damage."),
                ("evidence", "inspection-2027-02"),
                ("changes", changes),
            ],
        )
        .await
    }

    /// `by` forfeits `deposit` for `reason`, approved by `approver`.
    pub async fn forfeit_deposit(
        &self,
        by: &User,
        deposit: &Deposit,
        reason: &str,
        approver: Option<i64>,
    ) -> TestResponse {
        let approver = approver.map(|id| id.to_string());
        let mut pairs = vec![("reason", reason)];
        if let Some(approver) = approver.as_deref() {
            pairs.push(("approver", approver));
        }
        self.deposit_page(by, deposit, "forfeit", &pairs).await
    }

    /// `party` disputes the lines `lines` of version `version` of the
    /// deductions of `deposit`, through the kernel contract the portal uses.
    pub async fn dispute_lines(
        &self,
        party: PartyId,
        deposit: &Deposit,
        version: i32,
        lines: Vec<i64>,
    ) -> Result<()> {
        let request = DisputeDeductions {
            deposit: deposit.deposit_id(),
            version,
            lines,
            comment: "The wall was already damaged when I moved in.".to_owned(),
        };
        // `today()` reads the clock only inside `at_travelled_time`.
        self.app
            .at_travelled_time(deposit_deductions(self.state()).dispute(
                self.state(),
                party,
                request,
            ))
            .await
    }

    /// What the portal shows `party` of the deductions now.
    pub async fn published_for(
        &self,
        party: PartyId,
    ) -> Vec<wl_kernel::deposits::PublishedDeductions> {
        // `today()` reads the clock only inside `at_travelled_time`.
        self.app
            .at_travelled_time(deposit_deductions(self.state()).published_for(self.state(), party))
            .await
            .unwrap()
    }

    /// `by` adjusts the held `deposit` to `amount` dirhams (`"6000.00"`) for
    /// version `version` of its agreement, effective on `date`.
    pub async fn adjust_deposit(
        &self,
        by: &User,
        deposit: &Deposit,
        amount: &str,
        version: &str,
        date: &str,
    ) -> TestResponse {
        self.deposit_page(
            by,
            deposit,
            "adjust",
            &[
                ("new_amount", amount),
                ("source_version", version),
                ("effective_on", date),
            ],
        )
        .await
    }

    /// Allocates `amount` fils of `payment` to the held `deposit`.
    pub async fn allocate_to_deposit(
        &self,
        payment: &Payment,
        deposit: &Deposit,
        amount: i64,
    ) -> Result<Allocation> {
        wl_payments::allocate::allocate(
            self.state(),
            payment.payment_id(),
            AllocationTarget::Deposit(deposit.deposit_id()),
            amount,
            None,
        )
        .await
    }

    /// `by` writes off the bounced `cheque` for `reason`.
    pub async fn write_off_cheque(&self, by: &User, cheque: &Cheque, reason: &str) -> TestResponse {
        self.cheque_action(by, cheque, "write_off", &[("reason", reason)])
            .await
    }

    /// The approvers say yes to the waiting write-off of `invoice`: the
    /// invoice is written off and `InvoiceWrittenOff` arrives.
    pub async fn approve_invoice_write_off(&self, invoice: InvoiceId) {
        let summary = self
            .invoices
            .mark_written_off(invoice)
            .expect("the invoice exists");
        self.state()
            .emit(written_off_event(&summary))
            .await
            .expect("the event is handled");
    }

    /// The approvers end the waiting write-off request of `invoice` with
    /// `decision`.
    pub async fn end_invoice_write_off(&self, invoice: InvoiceId, decision: ApprovalEnd) {
        let request = self
            .invoices
            .request_of(invoice)
            .expect("a write-off waits for the invoice");
        self.state()
            .emit(ApprovalDecided {
                request,
                decision,
                approvers: Vec::new(),
                subject: invoice.subject(),
                state_returned_to: None,
            })
            .await
            .expect("the event is handled");
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
        self.app.travel(Duration::from_secs(seconds as u64));
    }
}

/// The year the receipts of the tests carry (the tests run in real time
/// unless they travel).
pub fn year() -> String {
    wl_kernel::documents::today().format("%Y").to_string()
}

/// wl-payments booted in an app with auth and the kernel, seeded.
pub async fn boot() -> Booted {
    boot_gateway(FakeGateway::new()).await
}

/// `WlPayments` with the default methods and `gateway`, booted.
pub async fn boot_gateway(gateway: FakeGateway) -> Booted {
    boot_with(WlPayments::new().gateway(gateway.clone()), gateway).await
}

/// `module` booted; `gateway` is the fake the module was built with.
pub async fn boot_with(module: WlPayments, gateway: FakeGateway) -> Booted {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();

    let received = Recorded::new();
    let reversed = Recorded::new();
    let refund_paid = Recorded::new();
    let refund_failed = Recorded::new();
    let cheque_bounced = Recorded::new();
    let cheque_resolved = Recorded::new();
    let cheque_referred = Recorded::new();
    let deposit_settled = Recorded::new();
    let deposit_adjusted = Recorded::new();
    let cash_shortage = Recorded::new();
    let cash_counted = Recorded::new();
    let cash_settled = Recorded::new();
    let cash_tickets = cash::CashTickets::default();
    let task_creator: Arc<dyn TaskCreator> = Arc::new(cash_tickets.clone());
    let hr = cash::HrDesk::default();
    let shortage_recovery: Arc<dyn ShortageRecovery> = Arc::new(hr.clone());
    let top_ups = RecordingTopUps::default();
    let top_up_provider: Arc<dyn DepositTopUps> = Arc::new(top_ups.clone());
    let fees = RecordingFees::default();
    let notices = Recording::default();
    let invoices = RecordingInvoicer::default();
    let directory: Arc<dyn PartyDirectory> = Arc::new(TestDirectory);
    let notifier: Arc<dyn Notifier> = Arc::new(notices.clone());
    let invoicer: Arc<dyn Invoicer> = Arc::new(invoices.clone());
    let bounce_fees: Arc<dyn BounceFees> = Arc::new(fees.clone());
    let charges = RecordingCharges {
        invoices: invoices.clone(),
        charges: Arc::new(Mutex::new(Vec::new())),
    };
    let charge_invoicer: Arc<dyn ChargeInvoicer> = Arc::new(charges.clone());
    let drawings = DrawingsDesk::default();
    let owner_drawings: Arc<dyn OwnerDrawings> = Arc::new(drawings.clone());
    let blocked = BlockedParties::default();
    let block_guard: Arc<dyn wl_kernel::block_guard::BlockGuard> = Arc::new(blocked.clone());
    // What wl-parties registers: an exception is decided by the compliance role.
    wl_kernel::PolicyDefault::new(wl_kernel::BLOCK_EXCEPTION_ACTION)
        .with_step(
            wl_kernel::approvals::Step::one_of(["compliance"])
                .requiring(wl_kernel::BLOCK_EXCEPTION_ACTION),
        )
        .register();
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(module)
        .module(Harness)
        .provide(directory)
        .provide(notifier)
        .provide(invoicer)
        .provide(bounce_fees)
        .provide(charge_invoicer)
        .provide(top_up_provider)
        .provide(owner_drawings)
        .provide(task_creator)
        .provide(shortage_recovery)
        .provide(block_guard);
    let app = record(app, &received);
    let app = record(app, &reversed);
    let app = record(app, &refund_paid);
    let app = record(app, &refund_failed);
    let app = record(app, &cheque_bounced);
    let app = record(app, &cheque_resolved);
    let app = record(app, &cheque_referred);
    let app = record(app, &deposit_settled);
    let app = record(app, &deposit_adjusted);
    let app = record(app, &cash_shortage);
    let app = record(app, &cash_counted);
    let app = record(app, &cash_settled);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    // The payment list reads the names of parties from wl-parties' table,
    // which this app does not have (the crate does not depend on
    // wl-parties): a stand-in with the two parties of `TestDirectory`.
    sql(
        r#"CREATE TABLE "wl_parties_parties" (id INTEGER PRIMARY KEY, display_name TEXT NOT NULL)"#,
    )
    .execute(app.db())
    .await
    .expect("the stand-in table");
    sql(r#"INSERT INTO "wl_parties_parties" (id, display_name) VALUES (1, 'Blue Dunes LLC'), (2, 'Quiet Sands LLC'), (3, 'Dusk Holdings LLC')"#)
        .execute(app.db())
        .await
        .expect("the stand-in rows");
    Booted {
        app,
        _views: views,
        notices,
        invoices,
        gateway,
        received,
        reversed,
        refund_paid,
        refund_failed,
        cheque_bounced,
        cheque_resolved,
        cheque_referred,
        deposit_settled,
        deposit_adjusted,
        blocked,
        top_ups,
        fees,
        charges,
        drawings,
        cash_tickets,
        hr,
        cash_shortage,
        cash_counted,
        cash_settled,
    }
}
