//! Checks of #27 part 4: the back office of voiding. `POST
//! /voids/{subject_type}/{id}` voids the record or sends it for approval
//! (toasts "Voided" / "Sent for approval"), a blank reason is a 422 under
//! the field, and the `void_action.html` partial shows only when `void` is
//! available.
//!
//! The subject is a test-only receipt allocated to an invoice (as in
//! `voids_approval.rs`).

use renox::auth::{Permissions, permissions};
use renox::db::{Migration, sql};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::machines::BoxFuture;
use wl_kernel::voids::{self, web};
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

struct Payments;

impl Module for Payments {
    fn name(&self) -> &'static str {
        "test-payments"
    }

    fn routes(&self) -> Routes {
        Routes::new().get("/receipts/{id}/void-action", void_action)
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

async fn void_action(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Json<Option<renox::serde_json::Value>>> {
    let receipt = Receipt::find(&state.db, id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    let actions = receipt.available(&user, &state.db).await?;
    Ok(Json(web::action_for(
        &Subject::new(RECEIPT, id),
        &actions,
        &lang,
    )))
}

impl World {
    /// The toasts waiting in the session (they show on the page it goes back to).
    fn toasts(&self) -> String {
        self.app
            .session_get::<renox::serde_json::Value>("_toasts")
            .map(|v| v.to_string())
            .unwrap_or_default()
    }

    async fn action(&self, user: &User, receipt: &Receipt) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .get(&format!("/receipts/{}/void-action", receipt.id))
            .await
    }

    /// Posts the void form the way the sheet does.
    async fn sheet(
        &self,
        user: &User,
        receipt: &Receipt,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/voids/{RECEIPT}/{}", receipt.id),
                &[("reason", reason)],
            )
            .await
    }
}

#[renox::test]
async fn a_void_with_nothing_posted_says_voided() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;
    sql("DELETE FROM test_allocations")
        .execute(w.app.db())
        .await
        .unwrap();

    let res = w.sheet(&w.voider, &receipt, "  mistake  ").await;
    assert_eq!(res.status.as_u16(), 303, "{}", res.text());
    let toast = w.toasts();
    assert!(toast.contains("Voided"), "{toast}");
    assert!(!toast.contains("Sent for approval"), "{toast}");
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Voided);
    let record = w.record(&receipt).await.expect("a Void record");
    assert_eq!(record.reason, "mistake");
    w.app
        .assert_emitted::<RecordVoided>(|e| e.reason == "mistake");
}

#[renox::test]
async fn a_void_with_an_allocation_says_sent_for_approval() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;

    let res = w.sheet(&w.voider, &receipt, "duplicate").await;
    assert_eq!(res.status.as_u16(), 303, "{}", res.text());
    let toast = w.toasts();
    assert!(toast.contains("Sent for approval"), "{toast}");
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert_eq!(w.balance().await, 0);
    assert!(w.record(&receipt).await.is_none());
    w.app.assert_not_emitted::<RecordVoided>();
}

#[renox::test]
async fn a_blank_reason_is_refused_under_the_field_and_nothing_changes() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;
    for reason in ["", "   "] {
        // The sheet posts with htmx, which gets the 422 itself.
        w.app.acting_as(&w.voider);
        let res = w
            .app
            .htmx()
            .post(
                &format!("/voids/{RECEIPT}/{}", receipt.id),
                &[("reason", reason)],
            )
            .await;
        assert_eq!(res.status.as_u16(), 422, "{}", res.text());
        res.assert_invalid("reason");
    }
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert!(w.record(&receipt).await.is_none());
    w.app.assert_not_emitted::<RecordVoided>();
}

#[renox::test]
async fn the_route_refuses_without_the_permission_for_a_third_party_and_an_unknown_type() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;

    let res = w.sheet(&w.clerk, &receipt, "duplicate").await;
    assert_eq!(res.status.as_u16(), 422, "{}", res.text());

    w.set(&receipt, "given", "1").await;
    let res = w.sheet(&w.voider, &receipt, "duplicate").await;
    assert_eq!(res.status.as_u16(), 422, "{}", res.text());
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert!(w.record(&receipt).await.is_none());

    w.app.acting_as(&w.voider);
    let res = w
        .app
        .post(
            &format!("/voids/payments.nothing/{}", receipt.id),
            &[("reason", "x")],
        )
        .await;
    assert!(res.status.as_u16() >= 400, "{}", res.text());
    assert!(w.record(&receipt).await.is_none());
}

#[renox::test]
async fn the_void_item_is_offered_only_when_void_is_available() {
    let w = world().await;
    let receipt = w.receipt("RCP-ACME-2026-0042").await;

    let res = w.action(&w.voider, &receipt).await;
    res.assert_ok();
    res.assert_json_path("id", format!("void-payments-receipt-{}", receipt.id));
    res.assert_json_path("subject_type", RECEIPT);
    res.assert_json_path("labels.void", "Void");

    // No permission: not offered.
    w.action(&w.clerk, &receipt)
        .await
        .assert_json(renox::serde_json::Value::Null);

    // A draft is a state the module does not allow.
    w.set(&receipt, "status", "draft").await;
    w.action(&w.voider, &receipt)
        .await
        .assert_json(renox::serde_json::Value::Null);
    w.set(&receipt, "status", "issued").await;

    // Given to a third party.
    w.set(&receipt, "given", "1").await;
    w.action(&w.voider, &receipt)
        .await
        .assert_json(renox::serde_json::Value::Null);
}

#[renox::test]
async fn the_kernel_templates_compile() {
    let w = world().await;
    w.app.assert_views_compile();
}
