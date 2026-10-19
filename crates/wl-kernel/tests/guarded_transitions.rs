//! Checks of #37 part 3: `.block_guarded(..)`, `run_guarded`,
//! `request_exception_in`, the final re-check and the refusal partial.
//!
//! The subject is a test-only supplier payment with a `pay` transition
//! (money out). The guard is a test double that refuses the parties listed in
//! a table; the real one lives in `wl-parties`.

use renox::auth::{Permissions, permissions};
use renox::db::{Migration, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, history};
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, Step};
use wl_kernel::block_guard::{
    self, ActionKind, BLOCK_EXCEPTION_ACTION, BlockFuture, BlockGuard, BlockRefusal, GuardOutcome,
    GuardedError,
};
use wl_kernel::{
    ApprovalRequest, ApprovalStatus, ClientConfig, PartyId, PolicyDefault, Subject, SubjectMachine,
};

const PAYMENT: &str = "payments.payment";
const EXCEPTION: &str = "parties.block_exception";

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum PaymentStatus {
    #[default]
    Requested,
    Paid,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_supplier_payments", state_machine)]
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

/// Refuses the parties in `test_blocked`.
struct TableGuard;

impl BlockGuard for TableGuard {
    fn check_in<'a>(
        &'a self,
        tx: &'a mut Transaction,
        _viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome> {
        Box::pin(async move {
            if !kind.is_guarded() {
                return Ok(GuardOutcome::Allowed);
            }
            let blocked = sql("SELECT COUNT(*) FROM test_blocked WHERE party_id = ?")
                .bind(party.0)
                .scalar::<i64>(&mut *tx)
                .await?;
            Ok(if blocked > 0 {
                GuardOutcome::Blocked(BlockRefusal::neutral(party, kind, "Finance manager"))
            } else {
                GuardOutcome::Allowed
            })
        })
    }
}

const UP: &str = r#"
CREATE TABLE "test_supplier_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'requested',
    party_id INTEGER NOT NULL DEFAULT 0,
    amount INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_blocked" (party_id INTEGER PRIMARY KEY);
"#;

const DOWN: &str = r#"
DROP TABLE "test_blocked";
DROP TABLE "test_supplier_payments";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261018130098_create_test_supplier_payments",
    UP,
    Some(DOWN),
)];

#[derive(Deserialize)]
struct PayForm {
    amount: i64,
    block_exception: Option<String>,
}

/// Runs `pay` guarded, or asks for the exception when the form says so.
async fn pay(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    lang: Lang,
    Form(form): Form<PayForm>,
) -> Result<Response> {
    use renox::prelude::IntoResponse;

    let payment = Payment::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let input = json!({ "amount": form.amount });
    if form.block_exception.is_some() {
        let mut tx = state.db.begin().await?;
        let request = block_guard::request_exception_in(
            &state,
            &mut tx,
            &user,
            &Subject::new(PAYMENT, id),
            "pay",
            input,
        )
        .await?;
        tx.commit().await?;
        approvals::announce_request(&state, &request).await?;
        return Ok("exception requested".into_response());
    }
    match block_guard::run_guarded(&state, &user, &payment, "pay", input).await {
        Ok(outcome) if outcome.is_pending() => Ok("pending".into_response()),
        Ok(_) => Ok("paid".into_response()),
        Err(GuardedError::Blocked(refusal)) => {
            let amount = form.amount.to_string();
            let refusal = refusal.view(
                &lang,
                &format!("/payments/{id}/pay"),
                &[("amount", &amount)],
            );
            Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                view("payments/refused.html", context! { refusal }),
            )
                .into_response())
        }
        Err(other) => Err(other.into_error()),
    }
}

struct Payments;

impl Module for Payments {
    fn name(&self) -> &'static str {
        "test-payments"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/payments/{id}/pay", pay)
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Payment>();
        block_guard::provide_block_guard(app, TableGuard);
        SubjectMachine::of::<Payment>(PAYMENT)
            .block_guarded("pay", ActionKind::MoneyOut, |payment: &Payment| {
                Some(PartyId(payment.party_id))
            })
            .register();
        PolicyDefault::new(BLOCK_EXCEPTION_ACTION)
            .with_step(Step::one_of(["finance_manager"]).requiring(EXCEPTION))
            .register();
        PolicyDefault::new("payments.pay")
            .amount_above(500_000)
            .step(["payment_manager"])
            .register();
    }
}

const CLIENT: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = []

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
default = true
"#;

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const REFUSED: &str = r#"<rx-page layout="layouts/app.html" title="On hold">
  <rx-include template="wl-kernel/block_refusal.html"/>
</rx-page>
"#;

struct World {
    app: TestApp,
    accountant: User,
    finance_manager: User,
    payment_manager: User,
    _views: tempfile::TempDir,
}

async fn world() -> World {
    let views = tempfile::tempdir().expect("a temporary views directory");
    for (name, text) in [
        ("layouts/app.html", LAYOUT),
        ("payments/refused.html", REFUSED),
    ] {
        let path = views.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Payments),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::with_config(app, |c| c.views_path = views.path().to_path_buf()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.fake_events();

    let db = app.db();
    let view = wl_kernel::permissions::APPROVALS_VIEW;
    permissions::define_role(db, "accountant", &[view])
        .await
        .unwrap();
    permissions::define_role(db, "payment_manager", &[view])
        .await
        .unwrap();
    permissions::define_role(db, "finance_manager", &[view, EXCEPTION])
        .await
        .unwrap();
    let user = |name: &'static str, role: &'static str| async move {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(db, name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(db, role).await.unwrap();
        user
    };
    World {
        accountant: user("Aisha", "accountant").await,
        finance_manager: user("Farid", "finance_manager").await,
        payment_manager: user("Pia", "payment_manager").await,
        app,
        _views: views,
    }
}

impl World {
    async fn payment(&self, party: i64, amount: i64) -> Payment {
        Payment::create(
            self.app.db(),
            Payment {
                party_id: party,
                amount,
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    async fn block(&self, party: i64) {
        sql("INSERT OR IGNORE INTO test_blocked (party_id) VALUES (?)")
            .bind(party)
            .execute(self.app.db())
            .await
            .unwrap();
    }

    async fn pay(
        &self,
        user: &User,
        payment: &Payment,
        extra: &[(&str, &str)],
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        let amount = payment.amount.to_string();
        let mut form = vec![("amount", amount.as_str())];
        form.extend_from_slice(extra);
        self.app
            .post(&format!("/payments/{}/pay", payment.id), &form)
            .await
    }

    async fn status(&self, payment: &Payment) -> PaymentStatus {
        Payment::find(self.app.db(), payment.id)
            .await
            .unwrap()
            .unwrap()
            .status
    }

    async fn history(&self, payment: &Payment) -> Vec<String> {
        history(self.app.db(), Payment::TABLE, payment.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.transition)
            .collect()
    }

    async fn requests(&self, payment: &Payment) -> Vec<ApprovalRequest> {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(PAYMENT))
            .where_(ApprovalRequest::SUBJECT_ID.eq(payment.id))
            .get(self.app.db())
            .await
            .unwrap()
    }
}

#[renox::test]
async fn a_blocked_party_is_refused_with_the_partial_and_nothing_is_written() {
    let w = world().await;
    w.block(11).await;
    let payment = w.payment(11, 100_000).await;

    let res = w.pay(&w.accountant, &payment, &[]).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body = res.text();
    assert!(body.contains("This party is on hold"), "{body}");
    assert!(
        body.contains("Request exception (approved by Finance manager)"),
        "{body}"
    );
    assert!(body.contains(r#"name="block_exception""#), "{body}");
    assert!(body.contains(r#"action="/payments/"#), "{body}");
    assert_eq!(w.status(&payment).await, PaymentStatus::Requested);
    assert!(w.history(&payment).await.is_empty());
    assert!(w.requests(&payment).await.is_empty());
}

#[renox::test]
async fn an_unblocked_party_is_paid() {
    let w = world().await;
    let payment = w.payment(12, 100_000).await;
    let res = w.pay(&w.accountant, &payment, &[]).await;
    assert_eq!(res.text(), "paid");
    assert_eq!(w.status(&payment).await, PaymentStatus::Paid);
    assert_eq!(w.history(&payment).await, ["pay"]);
}

#[renox::test]
async fn an_exception_has_the_exception_step_and_runs_the_payment_once() {
    let w = world().await;
    w.block(13).await;
    let payment = w.payment(13, 100_000).await;

    let res = w
        .pay(&w.accountant, &payment, &[("block_exception", "1")])
        .await;
    assert_eq!(res.text(), "exception requested");
    let requests = w.requests(&payment).await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(request.transition, "pay");
    assert_eq!(request.payload.0["amount"], 100_000);
    assert_eq!(
        request.steps.0,
        [Step::one_of(["finance_manager"]).requiring(EXCEPTION)]
    );
    assert_eq!(w.status(&payment).await, PaymentStatus::Requested);
    assert!(w.history(&payment).await.is_empty());

    // The requester cannot approve; the finance manager can.
    assert!(
        approvals::approve(w.app.state(), request.id, &w.accountant, "")
            .await
            .is_err()
    );
    approvals::approve(w.app.state(), request.id, &w.finance_manager, "ok")
        .await
        .unwrap()
        .expect("the request ended");
    assert_eq!(w.status(&payment).await, PaymentStatus::Paid);
    assert_eq!(w.history(&payment).await, ["pay_approved"]);
    let after = ApprovalRequest::find(w.app.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, ApprovalStatus::Approved);

    // The approval was for that payment only: the next one is refused again.
    let next = w.payment(13, 100_000).await;
    let res = w.pay(&w.accountant, &next, &[]).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[renox::test]
async fn an_exception_for_a_party_that_is_not_blocked_is_refused() {
    let w = world().await;
    let payment = w.payment(14, 100_000).await;
    let res = w
        .pay(&w.accountant, &payment, &[("block_exception", "1")])
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(res.text().contains("no exception is needed"));
    assert!(w.requests(&payment).await.is_empty());
}

#[renox::test]
async fn an_exception_keeps_the_actions_own_policy_steps_first() {
    let w = world().await;
    w.block(15).await;
    // Above AED 5,000: the payment's own policy needs the payment manager.
    let payment = w.payment(15, 840_000).await;
    w.pay(&w.accountant, &payment, &[("block_exception", "1")])
        .await;
    let requests = w.requests(&payment).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].steps.0,
        [
            Step::one_of(["payment_manager"]),
            Step::one_of(["finance_manager"]).requiring(EXCEPTION),
        ]
    );

    // A second request for the same payment waits behind the first.
    let again = w
        .pay(&w.accountant, &payment, &[("block_exception", "1")])
        .await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);

    approvals::approve(w.app.state(), requests[0].id, &w.payment_manager, "")
        .await
        .unwrap();
    assert_eq!(w.status(&payment).await, PaymentStatus::Requested);
    approvals::approve(w.app.state(), requests[0].id, &w.finance_manager, "")
        .await
        .unwrap();
    assert_eq!(w.status(&payment).await, PaymentStatus::Paid);
}

#[renox::test]
async fn a_request_waiting_when_the_party_gets_blocked_fails_at_the_final_approval() {
    let w = world().await;
    let payment = w.payment(16, 840_000).await;
    // Not blocked yet: the payment's own policy defers the run.
    let res = w.pay(&w.accountant, &payment, &[]).await;
    assert_eq!(res.text(), "pending");
    let request = w.requests(&payment).await.pop().expect("a request");
    assert_eq!(request.action, "payments.pay");

    w.block(16).await;
    let decided = approvals::approve(w.app.state(), request.id, &w.payment_manager, "")
        .await
        .unwrap()
        .expect("the request ended");
    assert_eq!(decided.decision, approvals::ApprovalEnd::Failed);
    let after = ApprovalRequest::find(w.app.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, ApprovalStatus::Failed);
    assert_eq!(w.status(&payment).await, PaymentStatus::Requested);
    assert!(!w.history(&payment).await.contains(&"pay_approved".into()));
}

#[renox::test]
async fn without_a_provided_guard_the_machine_still_runs_directly() {
    // The hook and the guard use the same declaration: an undeclared
    // transition is not guarded at all.
    let machine = wl_kernel::machines::get(PAYMENT).expect("registered");
    assert_eq!(machine.guarded_kind("pay"), Some(ActionKind::MoneyOut));
    assert_eq!(machine.guarded_kind("pay_approved"), None);
}

#[test]
#[should_panic(expected = "money in is never refused")]
fn money_in_cannot_be_declared_guarded() {
    let _ = SubjectMachine::of::<Payment>(PAYMENT).block_guarded(
        "pay",
        ActionKind::MoneyIn,
        |payment: &Payment| Some(PartyId(payment.party_id)),
    );
}

#[test]
#[should_panic(expected = "not a user transition")]
fn a_system_transition_cannot_be_declared_guarded() {
    let _ = SubjectMachine::of::<Payment>(PAYMENT).block_guarded(
        "pay_approved",
        ActionKind::MoneyOut,
        |payment: &Payment| Some(PartyId(payment.party_id)),
    );
}
