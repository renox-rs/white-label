//! Booting wl-payments in a `TestApp`, as `crates/wl-ticketing/tests/support`
//! boots wl-ticketing: a party directory of its own (the crate does not
//! depend on wl-parties), an `Invoicer` and a `Notifier` that record what
//! they are asked for, the kernel events recorded, the `FakeGateway`, and a
//! test-only module whose routes act as the logged-in user for the refund
//! transitions (a test cannot build an `AuthUser`). Recording and reversing
//! a payment use the module's own routes.

#![allow(dead_code)]

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
use wl_kernel::invoicing::{InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer};
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
    Allocation, Deposit, FollowupTask, Payment, Refund, WlPayments, permissions as perm,
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
            wl_payments::deposit::refund(&state, &user, &deposit, method).await
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
        _ => return Err(Error::NotFound),
    };
    done.map(|r| format!("{:?}", r.status))
        .map_err(TransitionError::into_error)
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
    let notices = Recording::default();
    let invoices = RecordingInvoicer::default();
    let directory: Arc<dyn PartyDirectory> = Arc::new(TestDirectory);
    let notifier: Arc<dyn Notifier> = Arc::new(notices.clone());
    let invoicer: Arc<dyn Invoicer> = Arc::new(invoices.clone());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(module)
        .module(Harness)
        .provide(directory)
        .provide(notifier)
        .provide(invoicer);
    let app = record(app, &received);
    let app = record(app, &reversed);
    let app = record(app, &refund_paid);
    let app = record(app, &refund_failed);
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
    }
}
