//! Checks of #27 part 2: `voids::transitions`, `voids::void`, the reverse
//! handlers' refusals and the helpers.
//!
//! The subject is a test-only receipt that is allocated to an invoice (the
//! invoice balance is a test table). A `block` column makes the reverse
//! handler refuse, like a stock that was issued onward or a locked period.

use renox::auth::{Permissions, permissions};
use renox::db::{Migration, sql};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transitioned, history};
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
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

/// Whether `void` is open on `receipt` for a user with the permission: its
/// state and its guards (the permission guard passes a run without an actor).
async fn offered(receipt: &Receipt) -> bool {
    let machine = Receipt::machine();
    let transition = machine.find("void").unwrap();
    let ctx = renox::state_machine::TransitionCtx::new(None);
    transition.check_from(receipt.status).is_ok()
        && transition.check_guards(receipt, &ctx).await.is_ok()
}

#[renox::test]
async fn a_void_reverses_the_allocation_and_writes_the_void_record() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;

    let res = w.void(&w.voider, &receipt, "duplicate").await;
    assert_eq!(res.status.as_u16(), 200, "{}", res.text());
    assert_eq!(res.text(), "voided");

    assert_eq!(w.status(&receipt).await, ReceiptStatus::Voided);
    assert_eq!(w.balance().await, 300_000);
    let record = w.record(&receipt).await.expect("a Void record");
    assert_eq!(record.number.as_deref(), Some("RCP-ACME-2026-0042"));
    assert_eq!(record.series_key.as_deref(), Some("receipt"));
    assert_eq!(record.state_before, "issued");
    assert_eq!(record.reason, "duplicate");
    assert_eq!(record.voided_by, w.voider.id);
    assert_eq!(record.approval_request_id, None);
    assert_eq!(record.effects.0.len(), 1);
    assert_eq!(record.effects.0[0].kind, EffectKind::Allocation);
    assert_eq!(record.effects.0[0].amount, 300_000);

    // The history row is the engine's own, with the void reason.
    let rows = history(w.app.db(), Receipt::TABLE, receipt.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].transition, "void");
    assert_eq!(rows[0].reason.as_deref(), Some("duplicate"));

    w.app
        .assert_emitted::<Transitioned<Receipt>>(|e| e.transition == "void" && e.to == "voided");
    w.app.assert_emitted::<RecordVoided>(|e| {
        e.subject == Subject::new(RECEIPT, receipt.id)
            && e.state_before == "issued"
            && e.reason == "duplicate"
            && e.user_id == w.voider.id
            && e.approval_request_id.is_none()
    });
}

#[renox::test]
async fn the_voided_number_is_listed_and_the_record_offers_nothing_more() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;
    let other = w.receipt("RCP-ACME-2026-0043").await;
    w.void(&w.voider, &receipt, "duplicate").await;

    let numbers = voids::void_numbers(w.app.db(), "receipt").await.unwrap();
    assert_eq!(numbers, vec!["RCP-ACME-2026-0042"]);
    assert!(
        voids::void_numbers(w.app.db(), "invoice")
            .await
            .unwrap()
            .is_empty()
    );

    // exclude_voided leaves the voided one out of a total.
    let kept = voids::exclude_voided(Receipt::query())
        .get(w.app.db())
        .await
        .unwrap();
    assert_eq!(
        kept.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![other.id]
    );

    let voided = Receipt::find(w.app.db(), receipt.id)
        .await
        .unwrap()
        .unwrap();
    assert!(voids::is_voided(&voided));
    assert!(!voids::is_voided(&other));
    let refused = voids::ensure_editable(&voided).unwrap_err();
    assert!(matches!(refused, Error::Status(s, _) if s == StatusCode::FORBIDDEN));
    voids::ensure_editable(&other).unwrap();

    // No transition leaves `voided`, for anyone.
    assert!(!offered(&voided).await);
    let again = w.void(&w.voider, &receipt, "again").await;
    assert_eq!(again.status.as_u16(), 422, "{}", again.text());
}

#[renox::test]
async fn a_void_needs_a_reason() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;
    for reason in ["", "   "] {
        let res = w.void(&w.voider, &receipt, reason).await;
        assert!(res.status.is_client_error(), "{}", res.text());
    }
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert!(w.record(&receipt).await.is_none());
    assert_eq!(w.balance().await, 0);
}

#[renox::test]
async fn only_a_holder_of_the_void_permission_may_void_and_is_offered_it() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;

    let res = w.void(&w.clerk, &receipt, "duplicate").await;
    assert!(res.status.is_client_error(), "{}", res.text());
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);

    let loaded = Receipt::find(w.app.db(), receipt.id)
        .await
        .unwrap()
        .unwrap();
    // (The permission itself is checked by the run above; a test cannot
    // build an `AuthUser` for `available()`.)
    assert!(offered(&loaded).await);
}

#[renox::test]
async fn a_record_given_to_a_third_party_cannot_be_voided() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;
    w.set(&receipt, "given", "1").await;

    let res = w.void(&w.voider, &receipt, "duplicate").await;
    assert_eq!(res.status.as_u16(), 422, "{}", res.text());
    assert!(
        res.text().contains("given to a third party"),
        "{}",
        res.text()
    );
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert!(w.record(&receipt).await.is_none());
    let loaded = Receipt::find(w.app.db(), receipt.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!offered(&loaded).await);
}

#[renox::test]
async fn a_state_the_module_does_not_allow_is_refused() {
    let w = world().await;
    let draft = Receipt::create(w.app.db(), Receipt::default())
        .await
        .unwrap();
    let res = w.void(&w.voider, &draft, "mistake").await;
    assert!(res.status.is_client_error(), "{}", res.text());
    assert_eq!(w.status(&draft).await, ReceiptStatus::Draft);
}

#[renox::test]
async fn a_handler_that_refuses_rolls_everything_back_with_its_message() {
    let w = world().await;
    for (block, message) in [
        ("stock", "stock already issued onward"),
        ("period", "the period 2026-09 is locked"),
    ] {
        let receipt = w.receipt("RCP-ACME-2026-0042").await;
        w.set(&receipt, "block", block).await;

        let res = w.void(&w.voider, &receipt, "duplicate").await;
        assert_eq!(res.status.as_u16(), 422, "{}", res.text());
        assert!(res.text().contains(message), "{}", res.text());
        assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
        assert!(w.record(&receipt).await.is_none());
        assert_eq!(w.balance().await, 0);
        assert!(
            history(w.app.db(), Receipt::TABLE, receipt.id)
                .await
                .unwrap()
                .is_empty()
        );
    }
    w.app.assert_not_emitted::<RecordVoided>();
}

#[renox::test]
async fn the_approved_void_takes_who_and_which_request_from_the_metadata() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;
    let machine = wl_kernel::machines::get(RECEIPT).expect("registered");
    // The Void record points at a real request (a foreign key).
    sql("INSERT INTO wl_approval_policies (action) VALUES ('payments.void')")
        .execute(w.app.db())
        .await
        .unwrap();
    let request = sql(
        "INSERT INTO wl_approval_requests (policy_id, subject_type, subject_id, action, \
         transition, requester_id, subject_state_before) \
         VALUES (1, 'payments.receipt', ?, 'payments.void', 'void', ?, 'issued') RETURNING id",
    )
    .bind(receipt.id)
    .bind(w.clerk.id)
    .scalar::<i64>(w.app.db())
    .await
    .unwrap();

    let mut tx = w.app.db().begin().await.unwrap();
    let run = wl_kernel::machines::Run::new().reason("duplicate").meta(
        renox::serde_json::json!({ "approval_request": request, "requester_id": w.clerk.id }),
    );
    let ran = machine
        .run_in(&mut tx, receipt.id, "void_approved", run)
        .await
        .expect("the system voids it");
    tx.commit().await.unwrap();
    assert!(ran.moved());

    let record = w.record(&receipt).await.expect("a Void record");
    assert_eq!(record.voided_by, w.clerk.id);
    assert_eq!(record.approval_request_id, Some(request));
    assert_eq!(record.reason, "duplicate");
    assert_eq!(w.balance().await, 300_000);
}

#[test]
#[should_panic(expected = "cannot be voided")]
fn a_machine_without_a_terminal_voided_state_cannot_be_registered() {
    #[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    enum Status {
        #[default]
        Issued,
        Voided,
    }

    #[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
    #[model(table = "test_never", state_machine)]
    struct Never {
        id: i64,
        status: Status,
    }

    impl StateMachine for Never {
        type State = Status;
        const COLUMN: renox::db::Col<Never, Status> = Never::STATUS;

        fn machine() -> Machine<Self> {
            let machine = Machine::new()
                .initial([Status::Issued])
                .state(Status::Issued);
            voids::transitions::<Never, _>("payments.never", [Status::Issued])
                .into_iter()
                .fold(machine.state(Status::Voided), Machine::transition)
        }
    }

    // `voided` is not terminal.
    let _ = Voidable::of::<Never>("payments.never");
}
