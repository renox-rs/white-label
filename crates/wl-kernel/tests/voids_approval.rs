//! Checks of #27 part 3: the approval a void needs when money or stock was
//! posted. The `posted` count comes from the entity's `pending` probes, the
//! default policy has one step for holders of the void permission, and the
//! approved void takes who and which request from the history metadata.
//!
//! The subject is a test-only receipt allocated to an invoice (as in
//! `voids_run.rs`); a user cannot be built for `available()`, so the
//! checks run through the HTTP routes and the approvals API.

use renox::auth::{Permissions, permissions};
use renox::db::{Migration, sql};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, history};
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalRequested, ApprovalStatus};
use wl_kernel::machines::BoxFuture;
use wl_kernel::voids::{self, VoidOutcome};
use wl_kernel::{
    ClientConfig, Effect, EffectKind, RecordVoided, Reverse, ReverseError, Subject, SubjectMachine,
    VoidRecord, Voidable,
};

const RECEIPT: &str = "payments.receipt";
const VOID_PERMISSION: &str = "receipts.void";

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum ReceiptStatus {
    #[default]
    Draft,
    Issued,
    Voided,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_receipts", state_machine)]
struct Receipt {
    id: i64,
    status: ReceiptStatus,
    number: Option<String>,
    amount: i64,
    given: bool,
    block: String,
}

impl StateMachine for Receipt {
    type State = ReceiptStatus;
    const COLUMN: renox::db::Col<Receipt, ReceiptStatus> = Receipt::STATUS;

    fn machine() -> Machine<Self> {
        use ReceiptStatus::{Draft, Issued, Voided};
        voids::transitions::<Receipt, _>(RECEIPT, [Issued])
            .into_iter()
            .fold(
                Machine::new()
                    .initial([Draft, Issued])
                    .state(Draft)
                    .state(Issued)
                    .state(Voided)
                    .tone("neutral")
                    .terminal(),
                Machine::transition,
            )
    }
}

fn allocated<'a>(
    tx: &'a mut renox::db::Transaction,
    id: i64,
) -> BoxFuture<'a, Result<Vec<Effect>>> {
    Box::pin(async move {
        let amount =
            sql("SELECT COALESCE(SUM(amount), 0) FROM test_allocations WHERE receipt_id = ?")
                .bind(id)
                .scalar::<i64>(&mut *tx)
                .await?;
        Ok(if amount > 0 {
            vec![Effect::new(EffectKind::Allocation, "invoice INV-ACME-2026-0101").amount(amount)]
        } else {
            Vec::new()
        })
    })
}

fn unallocate<'a>(
    tx: &'a mut renox::db::Transaction,
    id: i64,
) -> BoxFuture<'a, Result<Vec<Effect>, ReverseError>> {
    Box::pin(async move {
        let block = sql("SELECT block FROM test_receipts WHERE id = ?")
            .bind(id)
            .scalar::<String>(&mut *tx)
            .await?;
        match block.as_str() {
            "stock" => {
                return Err(ReverseError::Failed("stock already issued onward".into()));
            }
            "period" => {
                return Err(ReverseError::LockedPeriod(
                    "the period 2026-09 is locked".into(),
                ));
            }
            _ => {}
        }
        let effects = allocated(tx, id).await?;
        let amount: i64 = effects.iter().map(|e| e.amount).sum();
        sql("UPDATE test_invoice SET balance = balance + ?")
            .bind(amount)
            .execute(&mut *tx)
            .await?;
        sql("DELETE FROM test_allocations WHERE receipt_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        Ok(effects)
    })
}

const UP: &str = r#"
CREATE TABLE "test_receipts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'draft',
    number TEXT,
    amount INTEGER NOT NULL DEFAULT 0,
    given INTEGER NOT NULL DEFAULT 0,
    block TEXT NOT NULL DEFAULT ''
);
CREATE TABLE "test_allocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt_id INTEGER NOT NULL,
    amount INTEGER NOT NULL
);
CREATE TABLE "test_invoice" (balance INTEGER NOT NULL);
INSERT INTO "test_invoice" (balance) VALUES (0);
"#;

const DOWN: &str = r#"
DROP TABLE "test_invoice";
DROP TABLE "test_allocations";
DROP TABLE "test_receipts";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261018130199_create_test_receipts",
    UP,
    Some(DOWN),
)];

#[derive(Deserialize)]
struct VoidForm {
    reason: Option<String>,
}

async fn void_receipt(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<VoidForm>,
) -> Result<Response> {
    use renox::prelude::IntoResponse;

    let reason = form.reason.unwrap_or_default();
    match voids::void(&state, &user, &Subject::new(RECEIPT, id), &reason)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?
    {
        VoidOutcome::Voided(_) => Ok("voided".into_response()),
        VoidOutcome::Pending(_) => Ok("pending".into_response()),
    }
}

struct Payments;

impl Module for Payments {
    fn name(&self) -> &'static str {
        "test-payments"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/receipts/{id}/void", void_receipt)
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Receipt>();
        SubjectMachine::of::<Receipt>(RECEIPT).register();
        Voidable::of::<Receipt>(RECEIPT)
            .permission(VOID_PERMISSION)
            .number("receipt", |receipt: &Receipt| receipt.number.clone())
            .given_to_third_party(|receipt: &Receipt| receipt.given)
            .reverse(EffectKind::Allocation, Reverse::new(allocated, unallocate))
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

struct World {
    app: TestApp,
    voider: User,
    approver: User,
    clerk: User,
}

async fn world() -> World {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Payments),
        config,
    )
    .expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.fake_events();

    let db = app.db();

    permissions::define_role(db, "voider", &[VOID_PERMISSION])
        .await
        .unwrap();
    permissions::define_role(db, "clerk", &[wl_kernel::permissions::APPROVALS_VIEW])
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
        voider: user("Vera", "voider").await,
        approver: user("Walt", "voider").await,
        clerk: user("Carl", "clerk").await,
        app,
    }
}

impl World {
    /// An issued receipt of AED 3,000 allocated to the invoice.
    async fn receipt(&self, number: &str) -> Receipt {
        let receipt = Receipt::create(
            self.app.db(),
            Receipt {
                status: ReceiptStatus::Issued,
                number: Some(number.to_owned()),
                amount: 300_000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        sql("INSERT INTO test_allocations (receipt_id, amount) VALUES (?, 300000)")
            .bind(receipt.id)
            .execute(self.app.db())
            .await
            .unwrap();
        receipt
    }

    async fn set(&self, receipt: &Receipt, column: &str, value: &str) {
        sql(format!(
            "UPDATE test_receipts SET {column} = ? WHERE id = ?"
        ))
        .bind(value)
        .bind(receipt.id)
        .execute(self.app.db())
        .await
        .unwrap();
    }

    async fn void(
        &self,
        user: &User,
        receipt: &Receipt,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/receipts/{}/void", receipt.id),
                &[("reason", reason)],
            )
            .await
    }

    async fn status(&self, receipt: &Receipt) -> ReceiptStatus {
        Receipt::find(self.app.db(), receipt.id)
            .await
            .unwrap()
            .unwrap()
            .status
    }

    async fn balance(&self) -> i64 {
        sql("SELECT balance FROM test_invoice")
            .scalar::<i64>(self.app.db())
            .await
            .unwrap()
    }

    async fn record(&self, receipt: &Receipt) -> Option<VoidRecord> {
        VoidRecord::of_subject(self.app.db(), &Subject::new(RECEIPT, receipt.id))
            .await
            .unwrap()
    }
}

impl World {
    async fn request_about(&self, receipt: &Receipt) -> Option<ApprovalRequest> {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(RECEIPT))
            .where_(ApprovalRequest::SUBJECT_ID.eq(receipt.id))
            .order_by_desc(ApprovalRequest::ID)
            .first(self.app.db())
            .await
            .unwrap()
    }

    /// A receipt with an allocation, voided by Vera: its pending request.
    async fn sent(&self, number: &str) -> (Receipt, ApprovalRequest) {
        let receipt = self.receipt(number).await;
        let res = self.void(&self.voider, &receipt, "duplicate").await;
        assert_eq!(res.status.as_u16(), 200, "{}", res.text());
        assert_eq!(res.text(), "pending");
        let request = self.request_about(&receipt).await.expect("a request");
        (receipt, request)
    }
}

#[track_caller]
fn assert_forbidden<T: std::fmt::Debug>(result: Result<T>) {
    match result {
        Err(Error::Forbidden) => {}
        other => panic!("expected 403, got {other:?}"),
    }
}

#[renox::test]
async fn a_void_with_an_allocation_waits_for_approval_and_changes_nothing() {
    let w = world().await;
    let (receipt, request) = w.sent("RCP-ACME-2026-0042").await;

    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(request.action, "payments.void");
    assert_eq!(request.transition, "void");
    assert_eq!(request.requester_id, w.voider.id);
    assert_eq!(request.steps.0.len(), 1);
    assert_eq!(
        request.steps.0[0].permission.as_deref(),
        Some(VOID_PERMISSION)
    );
    assert!(request.steps.0[0].roles.is_empty());
    assert_eq!(request.payload.0["posted"], 1);

    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert_eq!(w.balance().await, 0);
    assert!(w.record(&receipt).await.is_none());
    w.app.assert_not_emitted::<RecordVoided>();
    // The request was announced after the commit.
    w.app
        .assert_emitted::<ApprovalRequested>(|e| e.request == request.id);
}

#[renox::test]
async fn only_another_holder_of_the_void_permission_approves_and_it_then_voids() {
    let w = world().await;
    let (receipt, request) = w.sent("RCP-ACME-2026-0042").await;
    let state = w.app.state();

    // Vera asked (SoD); Carl does not hold the permission.
    assert_forbidden(approvals::approve(state, request.id, &w.voider, "").await);
    assert_forbidden(approvals::approve(state, request.id, &w.clerk, "").await);
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);

    let decided = approvals::approve(state, request.id, &w.approver, "ok")
        .await
        .unwrap()
        .expect("the last step ends the request");
    assert_eq!(decided.request, request.id);

    assert_eq!(w.status(&receipt).await, ReceiptStatus::Voided);
    assert_eq!(w.balance().await, 300_000);
    let record = w.record(&receipt).await.expect("a Void record");
    assert_eq!(record.voided_by, w.voider.id, "the requester voided it");
    assert_eq!(record.approval_request_id, Some(request.id));
    assert_eq!(record.reason, "duplicate");
    assert_eq!(record.state_before, "issued");
    assert_eq!(record.effects.0.len(), 1);

    let rows = history(w.app.db(), Receipt::TABLE, receipt.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].transition, "void_approved");
    assert_eq!(rows[0].reason.as_deref(), Some("duplicate"));

    w.app.assert_emitted::<RecordVoided>(|e| {
        e.subject == Subject::new(RECEIPT, receipt.id)
            && e.user_id == w.voider.id
            && e.approval_request_id == Some(request.id)
            && e.reason == "duplicate"
    });
}

#[renox::test]
async fn the_approvers_are_the_other_holders_of_the_permission() {
    let w = world().await;
    let (_, request) = w.sent("RCP-ACME-2026-0042").await;
    let approvers = wl_kernel::approvals::notify::step_approvers(w.app.db(), &request)
        .await
        .unwrap();
    assert_eq!(
        approvers.iter().map(|u| u.id).collect::<Vec<_>>(),
        vec![w.approver.id]
    );
}

#[renox::test]
async fn a_void_with_nothing_posted_runs_at_once() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;
    sql("DELETE FROM test_allocations")
        .execute(w.app.db())
        .await
        .unwrap();

    let res = w.void(&w.voider, &receipt, "mistake").await;
    assert_eq!(res.text(), "voided", "{}", res.text());
    assert!(w.request_about(&receipt).await.is_none());
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Voided);
    let record = w.record(&receipt).await.unwrap();
    assert_eq!(record.approval_request_id, None);
    w.app
        .assert_emitted::<RecordVoided>(|e| e.approval_request_id.is_none());
}

#[renox::test]
async fn a_receipt_given_out_while_waiting_fails_the_request() {
    let w = world().await;
    let (receipt, request) = w.sent("RCP-ACME-2026-0042").await;
    w.set(&receipt, "given", "1").await;

    let decided = approvals::approve(w.app.state(), request.id, &w.approver, "")
        .await
        .unwrap()
        .expect("the request ends");
    assert_eq!(decided.decision, approvals::ApprovalEnd::Failed);
    let request = ApprovalRequest::find(w.app.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Failed);
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert!(w.record(&receipt).await.is_none());
    assert_eq!(w.balance().await, 0);
    w.app.assert_not_emitted::<RecordVoided>();
}

#[renox::test]
async fn a_forged_posted_count_in_the_input_is_not_trusted() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;
    let mut tx = w.app.db().begin().await.unwrap();
    let forged = renox::serde_json::json!({ "posted": 0, "reason": "x" });
    let subject = Subject::new(RECEIPT, receipt.id);
    let input = voids::approval::with_posted(&mut tx, &subject, "void", Some("dup"), &forged)
        .await
        .unwrap();
    assert_eq!(input["posted"], 1);
    assert_eq!(input["reason"], "dup");
    let other = voids::approval::with_posted(&mut tx, &subject, "other", None, &forged)
        .await
        .unwrap();
    assert_eq!(other, forged);
}

#[renox::test]
async fn the_default_policy_is_stored_per_voidable_type() {
    let w = world().await;
    let policy = approvals::ApprovalPolicy::query()
        .where_(approvals::ApprovalPolicy::ACTION.eq("payments.void"))
        .first(w.app.db())
        .await
        .unwrap()
        .expect("the default policy");
    assert_eq!(policy.condition, approvals::ConditionKind::AmountAbove);
    assert_eq!(policy.threshold, 0);
    assert_eq!(policy.input_key.as_deref(), Some("posted"));
    assert!(policy.sod);
    assert_eq!(policy.steps.0.len(), 1);
}
