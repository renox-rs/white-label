//! Acceptance checks of #27 (AC1-AC10): voiding a record entered in error.
//!
//! The subjects are test-only models: a receipt allocated to an invoice
//! (numbered by a gapless series), a booking with a reserved slot, a tax
//! invoice with its `issued` flag, and a receivables report. The parts have
//! their own checks (`void_records.rs`, `voids_run.rs`, `voids_approval.rs`,
//! `voids_web.rs`); these follow the acceptance criteria end to end. A user
//! cannot be built outside a request, so `available()` and the runs go
//! through test-only HTTP routes.

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::db::{Migration, sql};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, history};
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::machines::BoxFuture;
use wl_kernel::voids::{self, VoidOutcome};
use wl_kernel::{
    ClientConfig, Effect, EffectKind, LegalEntity, RecordVoided, Reset, Reverse, ReverseError,
    Series, Subject, SubjectMachine, VoidRecord, Voidable,
};

const RECEIPT: &str = "payments.receipt";
const BOOKING: &str = "ticketing.booking";
const INVOICE: &str = "invoicing.tax_invoice";
const RECEIPT_PERMISSION: &str = "receipts.void";
const BOOKING_PERMISSION: &str = "bookings.void";
const INVOICE_PERMISSION: &str = "invoices.void";

/// Receipts take gapless numbers inside the transaction that issues them.
const RECEIPT_SERIES: Series = Series {
    key: "receipt",
    pattern: "{prefix}-{entity}-{yyyy}-{0000}",
    prefix: "RCP",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 19).unwrap()
}

// ---- the receipt, allocated to an invoice ----------------------------------

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

// ---- the booking, holding a reserved slot ----------------------------------

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum BookingStatus {
    #[default]
    Draft,
    Voided,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_bookings", state_machine)]
struct Booking {
    id: i64,
    status: BookingStatus,
}

impl StateMachine for Booking {
    type State = BookingStatus;
    const COLUMN: renox::db::Col<Booking, BookingStatus> = Booking::STATUS;

    fn machine() -> Machine<Self> {
        use BookingStatus::{Draft, Voided};
        voids::transitions::<Booking, _>(BOOKING, [Draft])
            .into_iter()
            .fold(
                Machine::new()
                    .initial([Draft])
                    .state(Draft)
                    .state(Voided)
                    .tone("neutral")
                    .terminal(),
                Machine::transition,
            )
    }
}

fn reserved<'a>(tx: &'a mut renox::db::Transaction, id: i64) -> BoxFuture<'a, Result<Vec<Effect>>> {
    Box::pin(async move {
        let held = sql("SELECT COUNT(*) FROM test_slots WHERE booking_id = ?")
            .bind(id)
            .scalar::<i64>(&mut *tx)
            .await?;
        Ok(if held > 0 {
            vec![Effect::new(EffectKind::ReservedSlot, "slot 2026-10-20 10:00").quantity(held)]
        } else {
            Vec::new()
        })
    })
}

fn release<'a>(
    tx: &'a mut renox::db::Transaction,
    id: i64,
) -> BoxFuture<'a, Result<Vec<Effect>, ReverseError>> {
    Box::pin(async move {
        let effects = reserved(tx, id).await?;
        sql("UPDATE test_slots SET booking_id = NULL WHERE booking_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        Ok(effects)
    })
}

// ---- the tax invoice, issued to a third party ------------------------------

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum InvoiceStatus {
    #[default]
    Draft,
    Voided,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_tax_invoices", state_machine)]
struct TaxInvoice {
    id: i64,
    status: InvoiceStatus,
    number: Option<String>,
    issued: bool,
}

impl StateMachine for TaxInvoice {
    type State = InvoiceStatus;
    const COLUMN: renox::db::Col<TaxInvoice, InvoiceStatus> = TaxInvoice::STATUS;

    fn machine() -> Machine<Self> {
        use InvoiceStatus::{Draft, Voided};
        voids::transitions::<TaxInvoice, _>(INVOICE, [Draft])
            .into_iter()
            .fold(
                Machine::new()
                    .initial([Draft])
                    .state(Draft)
                    .state(Voided)
                    .tone("neutral")
                    .terminal(),
                Machine::transition,
            )
    }
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
CREATE TABLE "test_bookings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'draft'
);
CREATE TABLE "test_slots" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER
);
CREATE TABLE "test_tax_invoices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'draft',
    number TEXT,
    issued INTEGER NOT NULL DEFAULT 0
);
"#;

const DOWN: &str = r#"
DROP TABLE "test_tax_invoices";
DROP TABLE "test_slots";
DROP TABLE "test_bookings";
DROP TABLE "test_invoice";
DROP TABLE "test_allocations";
DROP TABLE "test_receipts";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261018130199_create_test_voidables",
    UP,
    Some(DOWN),
)];

// ---- test-only routes -------------------------------------------------------

#[derive(Deserialize)]
struct VoidForm {
    reason: Option<String>,
}

/// Runs `voids::void` as the user: "voided" or "pending".
async fn void_record(
    State(state): State<AppState>,
    user: AuthUser,
    Path((subject_type, id)): Path<(String, i64)>,
    Form(form): Form<VoidForm>,
) -> Result<Response> {
    use renox::prelude::IntoResponse;

    let reason = form.reason.unwrap_or_default();
    match voids::void(&state, &user, &Subject::new(&subject_type, id), &reason)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?
    {
        VoidOutcome::Voided(_) => Ok("voided".into_response()),
        VoidOutcome::Pending(_) => Ok("pending".into_response()),
    }
}

/// What the user is offered on the record: the names of `available()`.
async fn available(
    State(state): State<AppState>,
    user: AuthUser,
    Path((subject_type, id)): Path<(String, i64)>,
) -> Result<Json<Vec<String>>> {
    let actions = match subject_type.as_str() {
        RECEIPT => {
            let record = Receipt::find(&state.db, id).await?.ok_or(Error::NotFound)?;
            record.available(&user, &state.db).await?
        }
        BOOKING => {
            let record = Booking::find(&state.db, id).await?.ok_or(Error::NotFound)?;
            record.available(&user, &state.db).await?
        }
        INVOICE => {
            let record = TaxInvoice::find(&state.db, id)
                .await?
                .ok_or(Error::NotFound)?;
            record.available(&user, &state.db).await?
        }
        _ => return Err(Error::NotFound),
    };
    Ok(Json(actions.into_iter().map(|a| a.name).collect()))
}

struct Test;

impl Module for Test {
    fn name(&self) -> &'static str {
        "test-voids"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/test/{subject_type}/{id}/void", void_record)
            .get("/test/{subject_type}/{id}/available", available)
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Receipt>();
        app.model::<Booking>();
        app.model::<TaxInvoice>();
        SubjectMachine::of::<Receipt>(RECEIPT).register();
        SubjectMachine::of::<Booking>(BOOKING).register();
        SubjectMachine::of::<TaxInvoice>(INVOICE).register();
        Voidable::of::<Receipt>(RECEIPT)
            .permission(RECEIPT_PERMISSION)
            .number("receipt", |receipt: &Receipt| receipt.number.clone())
            .given_to_third_party(|receipt: &Receipt| receipt.given)
            .reverse(EffectKind::Allocation, Reverse::new(allocated, unallocate))
            .register();
        Voidable::of::<Booking>(BOOKING)
            .permission(BOOKING_PERMISSION)
            .reverse(EffectKind::ReservedSlot, Reverse::new(reserved, release))
            .register();
        Voidable::of::<TaxInvoice>(INVOICE)
            .permission(INVOICE_PERMISSION)
            .number("invoice", |invoice: &TaxInvoice| invoice.number.clone())
            .given_to_third_party(|invoice: &TaxInvoice| invoice.issued)
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
    /// Vera: holds the void permissions, asks for the voids.
    voider: User,
    /// Walt: holds them too, approves.
    approver: User,
    /// Carl: holds none.
    clerk: User,
    /// Beth: holds every void permission, the way an admin does.
    boss: User,
}

async fn world() -> World {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Test),
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
    permissions::define_role(
        db,
        "voider",
        &[RECEIPT_PERMISSION, BOOKING_PERMISSION, INVOICE_PERMISSION],
    )
    .await
    .unwrap();
    permissions::define_role(db, "clerk", &[wl_kernel::permissions::APPROVALS_VIEW])
        .await
        .unwrap();
    permissions::define_role(
        db,
        "boss",
        &[
            wl_kernel::permissions::RECORDS_VOID,
            RECEIPT_PERMISSION,
            BOOKING_PERMISSION,
            INVOICE_PERMISSION,
            wl_kernel::permissions::APPROVALS_VIEW,
        ],
    )
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
    let world = World {
        voider: user("Vera", "voider").await,
        approver: user("Walt", "voider").await,
        clerk: user("Carl", "clerk").await,
        boss: user("Beth", "boss").await,
        app,
    };

    // The receipt series has run up to 41.
    let db = world.app.db();
    let acme = LegalEntity::default_entity(db).await.unwrap();
    let mut tx = db.begin().await.unwrap();
    for _ in 0..41 {
        RECEIPT_SERIES
            .next(&mut tx, None, Some(&acme), day())
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    world
}

impl World {
    /// An issued receipt of AED 3,000 allocated to the invoice, numbered by
    /// the gapless series as the issuing transaction does.
    async fn receipt(&self) -> Receipt {
        let db = self.app.db();
        let acme = LegalEntity::default_entity(db).await.unwrap();
        let mut tx = db.begin().await.unwrap();
        let number = RECEIPT_SERIES
            .next(&mut tx, None, Some(&acme), day())
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let receipt = Receipt::create(
            db,
            Receipt {
                status: ReceiptStatus::Issued,
                number: Some(number),
                amount: 300_000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        sql("INSERT INTO test_allocations (receipt_id, amount) VALUES (?, 300000)")
            .bind(receipt.id)
            .execute(db)
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
        subject_type: &str,
        id: i64,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .post(
                &format!("/test/{subject_type}/{id}/void"),
                &[("reason", reason)],
            )
            .await
    }

    async fn offered(&self, user: &User, subject_type: &str, id: i64) -> Vec<String> {
        self.app.acting_as(user);
        let res = self
            .app
            .get(&format!("/test/{subject_type}/{id}/available"))
            .await;
        res.assert_ok();
        renox::serde_json::from_str(&res.text()).unwrap()
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

    async fn allocations(&self) -> i64 {
        sql("SELECT COUNT(*) FROM test_allocations")
            .scalar::<i64>(self.app.db())
            .await
            .unwrap()
    }

    async fn record(&self, subject_type: &str, id: i64) -> Option<VoidRecord> {
        VoidRecord::of_subject(self.app.db(), &Subject::new(subject_type, id))
            .await
            .unwrap()
    }

    async fn requests(&self) -> Vec<ApprovalRequest> {
        ApprovalRequest::query()
            .order_by(ApprovalRequest::ID)
            .get(self.app.db())
            .await
            .unwrap()
    }

    /// A receipt with an allocation, voided by Vera: its pending request.
    async fn sent(&self) -> (Receipt, ApprovalRequest) {
        let receipt = self.receipt().await;
        let res = self
            .void(&self.voider, RECEIPT, receipt.id, "duplicate")
            .await;
        assert_eq!(res.status.as_u16(), 200, "{}", res.text());
        assert_eq!(res.text(), "pending");
        let request = self
            .requests()
            .await
            .pop()
            .expect("an approval request was created");
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

// ---- AC1 - AC3: the receipt, approval and its number ------------------------

#[renox::test]
async fn ac1_a_void_of_an_allocated_receipt_waits_for_approval_and_changes_nothing() {
    let w = world().await;
    let (receipt, request) = w.sent().await;
    assert_eq!(receipt.number.as_deref(), Some("RCP-ACME-2026-0042"));
    assert_eq!(receipt.amount, 300_000);

    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(request.requester_id, w.voider.id);
    assert_eq!(request.steps.0.len(), 1);
    assert_eq!(
        request.steps.0[0].permission.as_deref(),
        Some(RECEIPT_PERMISSION)
    );
    assert_eq!(w.requests().await.len(), 1);

    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert_eq!(w.allocations().await, 1);
    assert_eq!(w.balance().await, 0);
    assert!(w.record(RECEIPT, receipt.id).await.is_none());
    w.app.assert_not_emitted::<RecordVoided>();
}

#[renox::test]
async fn ac2_another_holder_approves_and_everything_happens_in_one_transaction() {
    let w = world().await;
    let (receipt, request) = w.sent().await;
    let state = w.app.state();

    // Vera asked (SoD); Carl does not hold the permission.
    assert_forbidden(approvals::approve(state, request.id, &w.voider, "").await);
    assert_forbidden(approvals::approve(state, request.id, &w.clerk, "").await);
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert_eq!(w.balance().await, 0);

    approvals::approve(state, request.id, &w.approver, "ok")
        .await
        .unwrap()
        .expect("the last step ends the request");

    assert_eq!(w.status(&receipt).await, ReceiptStatus::Voided);
    assert_eq!(w.balance().await, 300_000, "the invoice balance is up");
    assert_eq!(w.allocations().await, 0);
    let record = w.record(RECEIPT, receipt.id).await.expect("a Void record");
    assert_eq!(record.state_before, "issued");
    assert_eq!(record.reason, "duplicate");
    assert_eq!(record.voided_by, w.voider.id);
    assert_eq!(record.approval_request_id, Some(request.id));
    assert_eq!(record.effects.0.len(), 1);
    assert_eq!(record.effects.0[0].kind, EffectKind::Allocation);
    assert_eq!(record.effects.0[0].reference, "invoice INV-ACME-2026-0101");
    assert_eq!(record.effects.0[0].amount, 300_000);

    let rows = history(w.app.db(), Receipt::TABLE, receipt.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].transition, "void_approved");

    w.app.assert_emitted::<RecordVoided>(|e| {
        e.subject == Subject::new(RECEIPT, receipt.id)
            && e.user_id == w.voider.id
            && e.approval_request_id == Some(request.id)
    });
    // Once: a second run of the approval is refused and emits nothing.
    assert!(
        approvals::approve(state, request.id, &w.approver, "again")
            .await
            .is_err()
    );
    assert_eq!(w.app.emitted::<RecordVoided>().len(), 1);
}

#[renox::test]
async fn ac3_a_voided_receipt_keeps_its_number_and_the_next_one_takes_the_next() {
    let w = world().await;
    let (receipt, request) = w.sent().await;
    approvals::approve(w.app.state(), request.id, &w.approver, "ok")
        .await
        .unwrap();

    let loaded = Receipt::find(w.app.db(), receipt.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.number.as_deref(), Some("RCP-ACME-2026-0042"));
    let next = w.receipt().await;
    assert_eq!(next.number.as_deref(), Some("RCP-ACME-2026-0043"));
    assert_eq!(
        voids::void_numbers(w.app.db(), "receipt").await.unwrap(),
        vec!["RCP-ACME-2026-0042"]
    );
}

// ---- AC4: a booking with a slot and no postings ------------------------------

#[renox::test]
async fn ac4_a_draft_booking_is_voided_at_once_and_its_slot_is_released() {
    let w = world().await;
    let db = w.app.db();
    let booking = Booking::create(db, Booking::default()).await.unwrap();
    sql("INSERT INTO test_slots (booking_id) VALUES (?)")
        .bind(booking.id)
        .execute(db)
        .await
        .unwrap();

    let res = w.void(&w.voider, BOOKING, booking.id, "wrong date").await;
    assert_eq!(res.status.as_u16(), 200, "{}", res.text());
    assert_eq!(res.text(), "voided");

    assert!(w.requests().await.is_empty(), "no approval request");
    let loaded = Booking::find(db, booking.id).await.unwrap().unwrap();
    assert_eq!(loaded.status, BookingStatus::Voided);
    let free = sql("SELECT COUNT(*) FROM test_slots WHERE booking_id IS NULL")
        .scalar::<i64>(db)
        .await
        .unwrap();
    assert_eq!(free, 1, "the slot is free");
    let record = w.record(BOOKING, booking.id).await.expect("a Void record");
    assert_eq!(record.approval_request_id, None);
    assert_eq!(record.effects.0.len(), 1);
    assert_eq!(record.effects.0[0].kind, EffectKind::ReservedSlot);
    assert_eq!(record.effects.0[0].quantity, 1);
    w.app
        .assert_emitted::<RecordVoided>(|e| e.subject == Subject::new(BOOKING, booking.id));
}

// ---- AC5: reason and states --------------------------------------------------

#[renox::test]
async fn ac5_a_void_needs_a_reason_and_a_state_the_module_allows() {
    let w = world().await;
    let receipt = w.receipt().await;
    for reason in ["", "   "] {
        let res = w.void(&w.voider, RECEIPT, receipt.id, reason).await;
        assert!(res.status.is_client_error(), "{}", res.text());
    }
    assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
    assert!(w.record(RECEIPT, receipt.id).await.is_none());
    assert!(w.requests().await.is_empty());
    assert_eq!(w.balance().await, 0);
    w.app.assert_not_emitted::<RecordVoided>();

    // A draft is a state the module does not allow: not offered, refused.
    let draft = Receipt::create(w.app.db(), Receipt::default())
        .await
        .unwrap();
    assert!(
        !w.offered(&w.voider, RECEIPT, draft.id)
            .await
            .contains(&"void".to_owned())
    );
    assert!(
        Receipt::machine()
            .find("void")
            .unwrap()
            .check_from(ReceiptStatus::Draft)
            .is_err(),
        "the transition does not leave a draft"
    );
    let res = w.void(&w.voider, RECEIPT, draft.id, "mistake").await;
    assert!(res.status.is_client_error(), "{}", res.text());
    assert_eq!(w.status(&draft).await, ReceiptStatus::Draft);
    assert!(w.record(RECEIPT, draft.id).await.is_none());

    // The issued one is offered to a holder, and to no one else.
    assert!(
        w.offered(&w.voider, RECEIPT, receipt.id)
            .await
            .contains(&"void".to_owned())
    );
    assert!(
        !w.offered(&w.clerk, RECEIPT, receipt.id)
            .await
            .contains(&"void".to_owned())
    );
}

// ---- AC6: an issued tax invoice ------------------------------------------------

#[renox::test]
async fn ac6_an_issued_tax_invoice_cannot_be_voided_by_anyone() {
    let w = world().await;
    let invoice = TaxInvoice::create(
        w.app.db(),
        TaxInvoice {
            number: Some("INV-ACME-2026-0101".into()),
            issued: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    for user in [&w.voider, &w.approver, &w.boss, &w.clerk] {
        assert!(
            !w.offered(user, INVOICE, invoice.id)
                .await
                .contains(&"void".to_owned()),
            "{} is not offered void",
            user.name
        );
        let direct = w.void(user, INVOICE, invoice.id, "wrong").await;
        assert!(direct.status.is_client_error(), "{}", direct.text());
        w.app.acting_as(user);
        let route = w
            .app
            .post(
                &format!("/voids/{INVOICE}/{}", invoice.id),
                &[("reason", "wrong")],
            )
            .await;
        assert!(
            route.status.as_u16() == 422 || route.status.as_u16() == 403,
            "{} {}",
            route.status,
            route.text()
        );
    }

    let loaded = TaxInvoice::find(w.app.db(), invoice.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.status, InvoiceStatus::Draft);
    assert!(w.record(INVOICE, invoice.id).await.is_none());
    assert!(w.requests().await.is_empty());
    assert!(
        history(w.app.db(), TaxInvoice::TABLE, invoice.id)
            .await
            .unwrap()
            .is_empty()
    );
    w.app.assert_not_emitted::<RecordVoided>();
}

// ---- AC7: a reverse handler refuses --------------------------------------------

#[renox::test]
async fn ac7_a_refusing_handler_rolls_everything_back_and_its_message_is_shown() {
    let w = world().await;
    // The handler's refusal is checked on a run that is not deferred; the
    // approval a posted allocation needs is checked in AC1/AC2.
    sql("UPDATE wl_approval_policies SET active = 0")
        .execute(w.app.db())
        .await
        .unwrap();
    for (block, message) in [
        ("stock", "stock already issued onward"),
        ("period", "the period 2026-09 is locked"),
    ] {
        let receipt = w.receipt().await;
        w.set(&receipt, "block", block).await;

        let res = w.void(&w.voider, RECEIPT, receipt.id, "duplicate").await;
        assert_eq!(res.status.as_u16(), 422, "{}", res.text());
        assert!(res.text().contains(message), "{}", res.text());
        assert_eq!(w.status(&receipt).await, ReceiptStatus::Issued);
        assert!(w.record(RECEIPT, receipt.id).await.is_none());
        assert!(
            history(w.app.db(), Receipt::TABLE, receipt.id)
                .await
                .unwrap()
                .is_empty()
        );
        // Nothing was reversed: the allocation is still there.
        let left = sql("SELECT COUNT(*) FROM test_allocations WHERE receipt_id = ?")
            .bind(receipt.id)
            .scalar::<i64>(w.app.db())
            .await
            .unwrap();
        assert_eq!(left, 1);
    }
    assert_eq!(w.balance().await, 0);
    w.app.assert_not_emitted::<RecordVoided>();
}

// ---- AC8: the receivables report -------------------------------------------------

/// The test receivables report: the total leaves voided receipts out, the
/// list keeps them, marked for the struck-through row.
async fn receivables(db: &Db) -> (i64, Vec<(String, &'static str)>) {
    let total = voids::exclude_voided(Receipt::query())
        .get(db)
        .await
        .unwrap()
        .iter()
        .map(|r| r.amount)
        .sum();
    let rows = Receipt::query()
        .order_by(Receipt::ID)
        .get(db)
        .await
        .unwrap()
        .into_iter()
        .map(|r| {
            let class = if voids::is_voided(&r) {
                wl_kernel::voids::web::VOIDED_CLASS
            } else {
                ""
            };
            (r.number.unwrap_or_default(), class)
        })
        .collect();
    (total, rows)
}

#[renox::test]
async fn ac8_the_receivables_report_leaves_a_voided_receipt_out_of_its_totals() {
    let w = world().await;
    let (_, request) = w.sent().await;
    let other = w.receipt().await;
    assert_eq!(receivables(w.app.db()).await.0, 600_000);

    approvals::approve(w.app.state(), request.id, &w.approver, "ok")
        .await
        .unwrap();

    let (total, rows) = receivables(w.app.db()).await;
    assert_eq!(total, 300_000, "only the receipt that stands");
    assert_eq!(
        rows,
        vec![
            ("RCP-ACME-2026-0042".to_owned(), "wl-voided"),
            (other.number.clone().unwrap(), ""),
        ]
    );
}

// ---- AC9: nothing leaves voided; bad machines do not register --------------------

#[renox::test]
async fn ac9_a_voided_record_offers_nothing_to_anyone_and_cannot_be_edited() {
    let w = world().await;
    let (receipt, request) = w.sent().await;
    approvals::approve(w.app.state(), request.id, &w.approver, "ok")
        .await
        .unwrap();

    for user in [&w.voider, &w.approver, &w.boss, &w.clerk] {
        assert!(
            w.offered(user, RECEIPT, receipt.id).await.is_empty(),
            "{} is offered nothing on a voided record",
            user.name
        );
        let again = w.void(user, RECEIPT, receipt.id, "again").await;
        assert!(again.status.is_client_error(), "{}", again.text());
    }
    let machine = Receipt::machine();
    assert!(
        machine
            .transitions()
            .iter()
            .all(|t| t.check_from(ReceiptStatus::Voided).is_err()),
        "no transition leaves voided"
    );

    let loaded = Receipt::find(w.app.db(), receipt.id)
        .await
        .unwrap()
        .unwrap();
    assert!(voids::is_voided(&loaded));
    let refused = voids::ensure_editable(&loaded).unwrap_err();
    assert!(matches!(refused, Error::Status(s, _) if s == StatusCode::FORBIDDEN));
    assert_eq!(w.app.emitted::<RecordVoided>().len(), 1);
}

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum BadStatus {
    #[default]
    Issued,
    Voided,
}

macro_rules! bad_model {
    ($name:ident, $table:literal, $machine:expr) => {
        #[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
        #[model(table = $table, state_machine)]
        struct $name {
            id: i64,
            status: BadStatus,
        }

        impl StateMachine for $name {
            type State = BadStatus;
            const COLUMN: renox::db::Col<$name, BadStatus> = $name::STATUS;

            fn machine() -> Machine<Self> {
                $machine
            }
        }
    };
}

// `voided` is only implied by the transitions: never declared terminal.
bad_model!(
    NoVoided,
    "test_no_voided",
    voids::transitions::<NoVoided, _>("payments.no_voided", [BadStatus::Issued])
        .into_iter()
        .fold(
            Machine::new()
                .initial([BadStatus::Issued])
                .state(BadStatus::Issued),
            Machine::transition,
        )
);

// `void_approved` is not `.system()`.
bad_model!(
    NotSystem,
    "test_not_system",
    Machine::new()
        .initial([BadStatus::Issued])
        .state(BadStatus::Issued)
        .state(BadStatus::Voided)
        .tone("neutral")
        .terminal()
        .transition(
            Transition::new("void")
                .from([BadStatus::Issued])
                .to(BadStatus::Voided)
                .reason_required()
        )
        .transition(
            Transition::new("void_approved")
                .from([BadStatus::Issued])
                .to(BadStatus::Voided)
        )
);

#[test]
#[should_panic(expected = "`voided` is not terminal")]
fn ac9_a_machine_without_a_terminal_voided_state_cannot_be_registered() {
    let _ = Voidable::of::<NoVoided>("payments.no_voided");
}

#[test]
#[should_panic(expected = "must be `.system()`")]
fn ac9_a_void_approved_that_is_not_system_cannot_be_registered() {
    let _ = Voidable::of::<NotSystem>("payments.not_system");
}

// ---- AC10: issued to a third party while the request waits ----------------------

#[renox::test]
async fn ac10_a_receipt_given_out_while_the_request_waits_ends_it_as_failed() {
    let w = world().await;
    let (receipt, request) = w.sent().await;
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
    assert!(w.record(RECEIPT, receipt.id).await.is_none());
    assert_eq!(w.balance().await, 0);
    assert_eq!(w.allocations().await, 1);
    w.app.assert_not_emitted::<RecordVoided>();
}
