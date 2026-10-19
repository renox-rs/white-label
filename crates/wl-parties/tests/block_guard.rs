//! Acceptance checks of #37 (BL-PTY-04): a blocked party gets no new
//! agreement and no money out unless an exception is approved. The unit-level
//! behaviour of the guard itself is in `guard.rs`; the kernel's side is in
//! `crates/wl-kernel/tests/{guarded_transitions,block_guard}.rs`.
//!
//! The supplier payment, refund (with a waiting state and its own amount
//! policy), lease, booking and receipt are test-only models with their own
//! migration. Their routes act as the signed-in user (`acting_as`), because a
//! test cannot build an `AuthUser` itself.

mod support;

use renox::auth::permissions;
use renox::db::{Migration, Transaction, sql};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, history};
use renox::testing::TestResponse;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalEnd, Require, Step, announce_request};
use wl_kernel::block_guard::{
    self, ActionKind, BLOCK_EXCEPTION_ACTION, GuardOutcome, GuardedError, guard,
};
use wl_kernel::machines::BoxFuture;
use wl_kernel::{
    ApprovalPolicy, ApprovalRequest, ApprovalStatus, PolicyDefault, Subject, SubjectMachine,
};
use wl_kernel::{PartyId, PartyKind};
use wl_parties::{BlockRecord, CompanyDetails, NewParty, WlParties, permissions as perm};

use support::{Booted, block_party, boot_custom};

const PAYMENT: &str = "payments.payment";
const REFUND: &str = "payments.refund";
const LEASE: &str = "leases.lease";
const REQUESTED: &str = "approval-requested";
const GULF: &str = "Gulf Spare Parts";

// ---------------------------------------------------------------------------
// The test-only models.
// ---------------------------------------------------------------------------

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum PaymentStatus {
    #[default]
    Requested,
    Paid,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_bg_payments", state_machine)]
struct Payment {
    id: i64,
    status: PaymentStatus,
    party_id: i64,
    amount: i64,
}

impl StateMachine for Payment {
    type State = PaymentStatus;
    const COLUMN: renox::db::Col<Payment, PaymentStatus> = Payment::STATUS;

    fn machine() -> Machine<Self> {
        use PaymentStatus::{Paid, Requested};
        Machine::new()
            .initial([Requested])
            .state(Requested)
            .state(Paid)
            .terminal()
            .transition(Transition::new("pay").from([Requested]).to(Paid))
            .transition(
                Transition::new("pay_approved")
                    .from([Requested])
                    .to(Paid)
                    .system(),
            )
    }
}

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum RefundStatus {
    #[default]
    Requested,
    PendingApproval,
    Refunded,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_bg_refunds", state_machine)]
struct Refund {
    id: i64,
    status: RefundStatus,
    party_id: i64,
    amount: i64,
}

impl StateMachine for Refund {
    type State = RefundStatus;
    const COLUMN: renox::db::Col<Refund, RefundStatus> = Refund::STATUS;

    fn machine() -> Machine<Self> {
        use RefundStatus::{PendingApproval, Refunded, Requested};
        Machine::new()
            .initial([Requested])
            .state(Requested)
            .state(PendingApproval)
            .state(Refunded)
            .terminal()
            .transition(
                Transition::new("refund")
                    .from([Requested])
                    .to(PendingApproval)
                    .effect(submit_refund),
            )
            .transition(
                Transition::new("refund_approved")
                    .from([Requested, PendingApproval])
                    .to(Refunded)
                    .system(),
            )
            .transition(
                Transition::new("back_to_requested")
                    .from([PendingApproval])
                    .to(Requested)
                    .system(),
            )
    }
}

/// Into the waiting state: the effect asks for approval under the refund's
/// own policy.
fn submit_refund<'a>(
    refund: &'a Refund,
    ctx: &'a TransitionCtx,
    tx: &'a mut Transaction,
) -> BoxFuture<'a, Result> {
    Box::pin(async move {
        let requester = ctx.actor().map(|user| user.id).ok_or(Error::Unauthorized)?;
        let require = Require::new(
            REFUND,
            Subject::new(REFUND, refund.id),
            "refund",
            requester,
            refund.status.to_string(),
        )
        .payload(json!({ "amount": refund.amount }));
        approvals::require_in(tx, &require).await?;
        Ok(())
    })
}

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum LeaseStatus {
    #[default]
    Active,
    Renewed,
    Extended,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_bg_leases", state_machine)]
struct Lease {
    id: i64,
    status: LeaseStatus,
    party_id: i64,
}

impl StateMachine for Lease {
    type State = LeaseStatus;
    const COLUMN: renox::db::Col<Lease, LeaseStatus> = Lease::STATUS;

    fn machine() -> Machine<Self> {
        use LeaseStatus::{Active, Extended, Renewed};
        Machine::new()
            .initial([Active])
            .state(Active)
            .state(Renewed)
            .terminal()
            .state(Extended)
            .terminal()
            .transition(Transition::new("renew").from([Active]).to(Renewed))
            .transition(Transition::new("extend").from([Active]).to(Extended))
    }
}

/// A booking: it has no transition, only a check before the insert.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_bg_bookings")]
struct Booking {
    id: i64,
    party_id: i64,
}

/// A receipt of `amount` fils: money in.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_bg_receipts")]
struct Receipt {
    id: i64,
    party_id: i64,
    amount: i64,
}

const UP: &str = r#"
CREATE TABLE "test_bg_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'requested',
    party_id INTEGER NOT NULL DEFAULT 0,
    amount INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_bg_refunds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'requested',
    party_id INTEGER NOT NULL DEFAULT 0,
    amount INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_bg_leases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'active',
    party_id INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_bg_bookings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_bg_receipts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL DEFAULT 0,
    amount INTEGER NOT NULL DEFAULT 0
);
"#;

const DOWN: &str = r#"
DROP TABLE "test_bg_receipts";
DROP TABLE "test_bg_bookings";
DROP TABLE "test_bg_leases";
DROP TABLE "test_bg_refunds";
DROP TABLE "test_bg_payments";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261019130001_create_test_block_guard_models",
    UP,
    Some(DOWN),
)];

// ---------------------------------------------------------------------------
// The test-only routes: each acts as the signed-in user.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ActionForm {
    block_exception: Option<String>,
}

#[derive(Deserialize)]
struct PartyForm {
    party_id: i64,
    amount: Option<i64>,
}

/// A refusal as the page shows it: 422 with the view of the refusal.
fn refused(refusal: block_guard::BlockRefusal, lang: &Lang, action: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(refusal.view(lang, action, &[])),
    )
        .into_response()
}

#[allow(clippy::too_many_arguments)]
/// Runs `transition` guarded, or asks for the exception when the form says
/// so (the handler of a module's "Request exception" button).
async fn act<M>(
    state: AppState,
    user: AuthUser,
    lang: Lang,
    record: M,
    subject_type: &str,
    transition: &str,
    input: Value,
    form: ActionForm,
) -> Result<Response>
where
    M: StateMachine<Key = i64> + Send + Sync,
{
    let id = record.id();
    if form.block_exception.is_some() {
        let mut tx = state.db.begin().await?;
        let request = block_guard::request_exception_in(
            &state,
            &mut tx,
            &user,
            &Subject::new(subject_type, id),
            transition,
            input,
        )
        .await?;
        tx.commit().await?;
        announce_request(&state, &request).await?;
        return Ok("exception requested".into_response());
    }
    let action = format!("/bg/{subject_type}/{id}/{transition}");
    match block_guard::run_guarded(&state, &user, &record, transition, input).await {
        Ok(outcome) if outcome.is_pending() => Ok("pending".into_response()),
        Ok(_) => Ok("done".into_response()),
        Err(GuardedError::Blocked(refusal)) => Ok(refused(refusal, &lang, &action)),
        Err(other) => Err(other.into_error()),
    }
}

async fn pay(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ActionForm>,
) -> Result<Response> {
    let payment = Payment::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let input = json!({ "amount": payment.amount });
    act(state, user, lang, payment, PAYMENT, "pay", input, form).await
}

async fn refund(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ActionForm>,
) -> Result<Response> {
    let refund = Refund::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let input = json!({ "amount": refund.amount });
    act(state, user, lang, refund, REFUND, "refund", input, form).await
}

async fn lease(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, transition)): Path<(i64, String)>,
    Form(form): Form<ActionForm>,
) -> Result<Response> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    act(
        state,
        user,
        lang,
        lease,
        LEASE,
        &transition,
        json!({}),
        form,
    )
    .await
}

/// A new booking: the guard before the insert, in the same transaction.
async fn book(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<PartyForm>,
) -> Result<Response> {
    let mut tx = state.db.begin().await?;
    let outcome = guard(&state)
        .check_in(
            &mut tx,
            Some(&user),
            PartyId(form.party_id),
            ActionKind::Start,
        )
        .await?;
    if let GuardOutcome::Blocked(refusal) = outcome {
        return Ok(refused(refusal, &lang, "/bg/bookings"));
    }
    Booking::create(
        &mut tx,
        Booking {
            party_id: form.party_id,
            ..Default::default()
        },
    )
    .await?;
    tx.commit().await?;
    Ok("booked".into_response())
}

/// A receipt: money in, checked like any other call and never refused.
async fn receive(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<PartyForm>,
) -> Result<Response> {
    let mut tx = state.db.begin().await?;
    let outcome = guard(&state)
        .check_in(
            &mut tx,
            Some(&user),
            PartyId(form.party_id),
            ActionKind::MoneyIn,
        )
        .await?;
    if let GuardOutcome::Blocked(refusal) = outcome {
        return Ok(refused(refusal, &lang, "/bg/receipts"));
    }
    Receipt::create(
        &mut tx,
        Receipt {
            party_id: form.party_id,
            amount: form.amount.unwrap_or_default(),
            ..Default::default()
        },
    )
    .await?;
    tx.commit().await?;
    Ok("received".into_response())
}

struct Models;

impl Module for Models {
    fn name(&self) -> &'static str {
        "block-guard-models"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/bg/payments.payment/{id}/pay", pay)
            .post("/bg/payments.refund/{id}/refund", refund)
            .post("/bg/leases.lease/{id}/{transition}", lease)
            .post("/bg/bookings", book)
            .post("/bg/receipts", receive)
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Payment>()
            .model::<Refund>()
            .model::<Lease>()
            .model::<Booking>()
            .model::<Receipt>();
        SubjectMachine::of::<Payment>(PAYMENT)
            .block_guarded("pay", ActionKind::MoneyOut, |p: &Payment| {
                Some(PartyId(p.party_id))
            })
            .register();
        SubjectMachine::of::<Refund>(REFUND)
            .waiting(RefundStatus::PendingApproval)
            .block_guarded("refund", ActionKind::MoneyOut, |r: &Refund| {
                Some(PartyId(r.party_id))
            })
            .register();
        SubjectMachine::of::<Lease>(LEASE)
            .block_guarded("renew", ActionKind::Renew, |l: &Lease| {
                Some(PartyId(l.party_id))
            })
            .block_guarded("extend", ActionKind::Extend, |l: &Lease| {
                Some(PartyId(l.party_id))
            })
            .register();
        // The refund's own policy: always the finance role, and above
        // AED 5,000 the property manager.
        PolicyDefault::new(REFUND).step(["finance"]).register();
        PolicyDefault::new(REFUND)
            .amount_above(500_000)
            .step(["property_manager"])
            .register();
    }
}

// ---------------------------------------------------------------------------
// The world.
// ---------------------------------------------------------------------------

struct World {
    t: Booted,
    accountant: User,
    finance_manager: User,
    mlro: User,
}

async fn world_with(module: WlParties) -> World {
    let t = boot_custom(module, true, |app| app.module(Models)).await;
    wl_parties::create(
        t.app.state(),
        NewParty {
            kind: PartyKind::Company,
            display_name: GULF.into(),
            legal_name_en: GULF.into(),
            company: Some(CompanyDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let db = t.db();
    let view = wl_kernel::permissions::APPROVALS_VIEW;
    // `finance_manager` holds the role of the exception step; the permission
    // comes with a role of its own, so that a role holder can lack it.
    for (role, grants) in [
        ("accountant", vec![view]),
        ("finance_manager", vec![view]),
        ("exception_approver", vec![perm::BLOCK_EXCEPTION]),
        ("compliance", vec![view, perm::BLOCK_EXCEPTION]),
        ("property_manager", vec![view]),
        ("finance", vec![view]),
    ] {
        permissions::define_role(db, role, &grants).await.unwrap();
    }
    let mlro = t.mlro().await;
    let accountant = member(&t, "Aisha", &["accountant"]).await;
    let finance_manager = member(&t, "Farid", &["finance_manager", "exception_approver"]).await;
    World {
        t,
        accountant,
        finance_manager,
        mlro,
    }
}

async fn member(t: &Booted, name: &str, roles: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    for role in roles {
        user.assign_role(t.db(), role).await.unwrap();
    }
    user
}

impl World {
    async fn gulf(&self) -> wl_parties::Party {
        self.t.party(GULF).await
    }

    /// `GULF` blocked by a user who sees the reason, with `code`.
    async fn block(&self, code: &str) -> wl_parties::Party {
        let party = self.gulf().await;
        block_party(&self.t, &self.mlro, &party, code, "Under review").await;
        party
    }

    async fn post(&self, user: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
        self.t.app.acting_as(user);
        self.t.app.post(path, form).await
    }

    async fn payment(&self, party: i64, amount: i64) -> Payment {
        Payment::create(
            self.t.db(),
            Payment {
                party_id: party,
                amount,
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    async fn refund(&self, party: i64, amount: i64) -> Refund {
        Refund::create(
            self.t.db(),
            Refund {
                party_id: party,
                amount,
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    async fn lease(&self, party: i64) -> Lease {
        Lease::create(
            self.t.db(),
            Lease {
                party_id: party,
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    async fn history<M: StateMachine<Key = i64>>(&self, id: i64) -> Vec<String> {
        history(self.t.db(), M::TABLE, id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.transition)
            .collect()
    }

    async fn requests(&self, subject_type: &str, id: i64) -> Vec<ApprovalRequest> {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(subject_type))
            .where_(ApprovalRequest::SUBJECT_ID.eq(id))
            .get(self.t.db())
            .await
            .unwrap()
    }

    async fn count(&self, table: &str) -> i64 {
        sql(format!("SELECT COUNT(*) FROM {table}"))
            .scalar::<i64>(self.t.db())
            .await
            .unwrap()
    }

    async fn notices(&self, user: &User) -> usize {
        user.notifications(self.t.db(), 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.kind == REQUESTED)
            .count()
    }
}

fn pay_path(payment: &Payment) -> String {
    format!("/bg/{PAYMENT}/{}/pay", payment.id)
}

fn refund_path(refund: &Refund) -> String {
    format!("/bg/{REFUND}/{}/refund", refund.id)
}

// ---------------------------------------------------------------------------
// The acceptance criteria.
// ---------------------------------------------------------------------------

/// AC1: the payment to a blocked party is refused with the offer to request
/// an exception; the request has the exception step only; nothing is paid.
#[renox::test]
async fn ac1_a_payment_to_a_blocked_party_is_refused_and_an_exception_can_be_requested() {
    let w = world_with(WlParties::new()).await;
    let party = w.block("fraud_suspected").await;
    let payment = w.payment(party.id, 840_000).await;

    let res = w.post(&w.accountant, &pay_path(&payment), &[]).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body = res.text();
    assert!(body.contains(r#""code":"on_hold""#), "{body}");
    assert!(
        body.contains("Request exception (approved by Finance manager)"),
        "{body}"
    );
    assert!(w.requests(PAYMENT, payment.id).await.is_empty());

    let res = w
        .post(
            &w.accountant,
            &pay_path(&payment),
            &[("block_exception", "1")],
        )
        .await;
    assert_eq!(res.text(), "exception requested");
    let requests = w.requests(PAYMENT, payment.id).await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(request.payload.0["amount"], 840_000);
    assert_eq!(
        request.steps.0,
        [Step::one_of(["finance_manager"]).requiring(perm::BLOCK_EXCEPTION)]
    );
    assert_eq!(w.notices(&w.finance_manager).await, 1);
    assert_eq!(w.notices(&w.accountant).await, 0);

    let payment = Payment::find(w.t.db(), payment.id).await.unwrap().unwrap();
    assert_eq!(payment.status, PaymentStatus::Requested);
    assert!(w.history::<Payment>(payment.id).await.is_empty());
}

/// AC2: SoD and the permission bind the approver; the approval runs
/// `pay_approved` once; the next payment needs a new request.
#[renox::test]
async fn ac2_the_exception_is_approved_by_the_right_user_and_runs_once() {
    let w = world_with(WlParties::new()).await;
    let party = w.block("fraud_suspected").await;
    let payment = w.payment(party.id, 840_000).await;
    w.post(
        &w.accountant,
        &pay_path(&payment),
        &[("block_exception", "1")],
    )
    .await;
    let request = w.requests(PAYMENT, payment.id).await.remove(0);
    let state = w.t.app.state();

    // SoD: a user who could approve cannot approve his own request.
    let lead = member(&w.t, "Lamia", &["finance_manager", "exception_approver"]).await;
    let led = w.payment(party.id, 100_000).await;
    w.post(&lead, &pay_path(&led), &[("block_exception", "1")])
        .await;
    let own = w.requests(PAYMENT, led.id).await.remove(0);
    assert!(matches!(
        approvals::approve(state, own.id, &lead, "").await,
        Err(Error::Forbidden)
    ));
    // A role holder without `parties.block_exception` is refused too.
    let without = member(&w.t, "Gina", &["finance_manager"]).await;
    assert!(matches!(
        approvals::approve(state, request.id, &without, "").await,
        Err(Error::Forbidden)
    ));
    assert_eq!(
        ApprovalRequest::find(w.t.db(), request.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Pending
    );
    assert_eq!(w.notices(&without).await, 0, "not told, not an approver");

    let decided = approvals::approve(state, request.id, &w.finance_manager, "ok")
        .await
        .unwrap()
        .expect("the request ended");
    assert_eq!(decided.decision, ApprovalEnd::Approved);
    let paid = Payment::find(w.t.db(), payment.id).await.unwrap().unwrap();
    assert_eq!(paid.status, PaymentStatus::Paid);
    assert_eq!(paid.amount, 840_000);
    assert_eq!(w.history::<Payment>(payment.id).await, ["pay_approved"]);
    let last = history(w.t.db(), Payment::TABLE, payment.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(last.actor_id, None, "run as the system");
    assert_eq!(last.metadata["requester_id"], w.accountant.id);
    let after = ApprovalRequest::find(w.t.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, ApprovalStatus::Approved);

    // The approval was for that payment: the next one is refused again.
    let next = w.payment(party.id, 840_000).await;
    let res = w.post(&w.accountant, &pay_path(&next), &[]).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(w.history::<Payment>(next.id).await.is_empty());
    assert_eq!(w.requests(PAYMENT, next.id).await.len(), 0);
}

/// AC3: with screening on and a `sanctions` block, a user without
/// `parties.block_reason.view` sees only "on hold"; the MLRO sees the reason
/// and the link; the exception goes to `compliance`.
#[renox::test]
async fn ac3_a_restricted_reason_is_shown_only_to_its_holders() {
    let w = world_with(WlParties::new().screening(true)).await;
    let party = w.block("sanctions").await;
    let payment = w.payment(party.id, 840_000).await;
    let user_b = w.t.user_b().await;

    let res = w.post(&user_b, &pay_path(&payment), &[]).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body = res.text();
    assert!(body.contains("This party is on hold"), "{body}");
    assert!(body.contains("on_hold"), "{body}");
    assert!(
        body.contains("compliance") || body.contains("Compliance"),
        "{body}"
    );
    let lower = body.to_lowercase();
    assert!(!lower.contains("sanction"), "{body}");
    assert!(!lower.contains("block-records"), "{body}");

    let res = w.post(&w.mlro, &pay_path(&payment), &[]).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body = res.text();
    assert!(body.contains("Blocked: Sanctions"), "{body}");
    let records = BlockRecord::open_for(w.t.db(), party.id).await.unwrap();
    assert_eq!(records.len(), 1);
    assert!(
        body.contains(&format!(
            "/parties/{}/block-records/{}",
            party.id, records[0].id
        )),
        "{body}"
    );

    w.post(&user_b, &pay_path(&payment), &[("block_exception", "1")])
        .await;
    let request = w.requests(PAYMENT, payment.id).await.remove(0);
    assert_eq!(
        request.steps.0,
        [Step::one_of(["compliance"]).requiring(perm::BLOCK_EXCEPTION)]
    );
}

/// AC4: renewing and extending a lease, and creating a booking, are refused
/// with no history row and no inserted row.
#[renox::test]
async fn ac4_new_agreements_with_a_blocked_party_are_refused() {
    let w = world_with(WlParties::new()).await;
    let party = w.block("non_payment").await;
    let lease = w.lease(party.id).await;

    for transition in ["renew", "extend"] {
        let path = format!("/bg/{LEASE}/{}/{transition}", lease.id);
        let res = w.post(&w.accountant, &path, &[]).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{transition}");
        assert!(res.text().contains("on_hold"), "{transition}");
    }
    assert!(w.history::<Lease>(lease.id).await.is_empty());
    let lease = Lease::find(w.t.db(), lease.id).await.unwrap().unwrap();
    assert_eq!(lease.status, LeaseStatus::Active);

    let id = party.id.to_string();
    let res = w
        .post(&w.accountant, "/bg/bookings", &[("party_id", &id)])
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(w.count("test_bg_bookings").await, 0);

    // A party that is not blocked can renew and book.
    let other = w.t.party("Al Noor Trading LLC").await;
    let lease = w.lease(other.id).await;
    let path = format!("/bg/{LEASE}/{}/renew", lease.id);
    assert_eq!(w.post(&w.accountant, &path, &[]).await.text(), "done");
    let id = other.id.to_string();
    let res = w
        .post(&w.accountant, "/bg/bookings", &[("party_id", &id)])
        .await;
    assert_eq!(res.text(), "booked");
    assert_eq!(w.count("test_bg_bookings").await, 1);
}

/// AC5: a receipt from the blocked party is recorded.
#[renox::test]
async fn ac5_a_receipt_from_a_blocked_party_is_recorded() {
    let w = world_with(WlParties::new()).await;
    let party = w.block("fraud_suspected").await;
    let id = party.id.to_string();
    let res = w
        .post(
            &w.accountant,
            "/bg/receipts",
            &[("party_id", &id), ("amount", "200000")],
        )
        .await;
    assert_eq!(res.text(), "received");
    assert_eq!(w.count("test_bg_receipts").await, 1);
    let mut tx = w.t.db().begin().await.unwrap();
    let outcome = guard(w.t.app.state())
        .check_in(
            &mut tx,
            Some(&w.accountant),
            PartyId(party.id),
            ActionKind::MoneyIn,
        )
        .await
        .unwrap();
    assert!(outcome.is_allowed());
}

/// AC6: a refund waiting for approval when the party gets blocked fails at
/// the final approval, goes back to `requested`, and shows no real reason.
#[renox::test]
async fn ac6_a_pending_refund_fails_when_the_party_was_blocked_meanwhile() {
    let w = world_with(WlParties::new()).await;
    let party = w.gulf().await;
    let refund = w.refund(party.id, 50_000).await;

    let res = w.post(&w.accountant, &refund_path(&refund), &[]).await;
    assert_eq!(res.text(), "done", "the refund ran into its waiting state");
    let stored = Refund::find(w.t.db(), refund.id).await.unwrap().unwrap();
    assert_eq!(stored.status, RefundStatus::PendingApproval);
    let request = w.requests(REFUND, refund.id).await.remove(0);
    assert_eq!(request.action, REFUND);

    w.block("sanctions").await;
    let finance = member(&w.t, "Fiona", &["finance"]).await;
    let decided = approvals::approve(w.t.app.state(), request.id, &finance, "")
        .await
        .unwrap()
        .expect("the request ended");
    assert_eq!(decided.decision, ApprovalEnd::Failed);
    assert_eq!(decided.state_returned_to.as_deref(), Some("requested"));

    let after = ApprovalRequest::find(w.t.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, ApprovalStatus::Failed);
    let stored = Refund::find(w.t.db(), refund.id).await.unwrap().unwrap();
    assert_eq!(stored.status, RefundStatus::Requested);
    let rows = history(w.t.db(), Refund::TABLE, refund.id).await.unwrap();
    assert!(rows.iter().all(|row| row.transition != "refund_approved"));
    let back = rows.last().unwrap();
    assert_eq!(back.transition, "back_to_requested");
    assert_eq!(
        back.reason.as_deref(),
        Some("approval failed: this party is on hold")
    );
    let shown = format!("{rows:?} {after:?}").to_lowercase();
    assert!(!shown.contains("sanction"), "{shown}");
    assert!(!shown.contains("fraud"), "{shown}");
}

/// AC7: an exception for a refund above AED 5,000 keeps the refund's own
/// step first, then the exception step.
#[renox::test]
async fn ac7_an_exception_keeps_the_refunds_own_policy_steps_first() {
    let w = world_with(WlParties::new()).await;
    let party = w.block("fraud_suspected").await;
    let refund = w.refund(party.id, 600_000).await;

    let res = w
        .post(
            &w.accountant,
            &refund_path(&refund),
            &[("block_exception", "1")],
        )
        .await;
    assert_eq!(res.text(), "exception requested");
    let request = w.requests(REFUND, refund.id).await.remove(0);
    assert_eq!(request.action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(
        request.steps.0,
        [
            Step::one_of(["property_manager"]),
            Step::one_of(["finance_manager"]).requiring(perm::BLOCK_EXCEPTION),
        ]
    );
    // Both policies are the refund's own: the exception adds a step to them.
    let policy = ApprovalPolicy::for_action(w.t.db(), REFUND, &json!({ "amount": 600_000 }))
        .await
        .unwrap()
        .expect("the refund's policy");
    assert_eq!(policy.steps.0, [Step::one_of(["property_manager"])]);
}
