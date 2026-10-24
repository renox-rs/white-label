//! BL-PAY-28 (#467) with the real modules of the Property host: the
//! accountant pays vendor CoolAir LLC by transfer against its approved bills,
//! oldest first; a payment to the blocked supplier waits for an approved
//! exception and is then recorded once, as requested; a reversal reopens the
//! bills.
//!
//! In the Property host the exception step is the finance manager's (the
//! block guard's approver role of the client); the spec's "compliance
//! officer" is that role here.

mod support;

use std::sync::{Arc, Mutex};

use host::roles::{ACCOUNTANT, FINANCE_MANAGER, PROPERTY_MANAGER};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::bill_machine::approve as approve_bill;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{BillLineDraft, SupplierBill, SupplierBillStatus};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::payments::{Direction, PaymentReceived, PaymentStatus};
use wl_kernel::{BLOCK_EXCEPTION_ACTION, PartyId, VatCategory};
use wl_payments::Payment;

/// Records every `PaymentReceived` of the app.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<PaymentReceived>>>);

impl Module for Seen {
    fn name(&self) -> &'static str {
        "seen_payments"
    }

    fn register(&self, app: &mut Registry) {
        let seen = Arc::clone(&self.0);
        app.listen(move |event: PaymentReceived, _state: AppState| {
            let seen = Arc::clone(&seen);
            async move {
                seen.lock().unwrap().push(event);
                Ok(())
            }
        });
    }
}

/// The Property host with the recorder.
async fn booted_seeing(seen: &Seen) -> TestApp {
    let app = TestApp::new(host::app().provide(noon_clock()).module(seen.clone())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// An approved bill `number` of `net` fils plus 5% VAT from `supplier`.
async fn approved_bill(
    app: &TestApp,
    supplier: PartyId,
    number: &str,
    day: u32,
    net: i64,
) -> SupplierBill {
    let entity = entity(app, "PGP").await;
    let mut draft = BillDraft::new(
        supplier,
        number,
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap(),
    )
    .line(BillLineDraft::new("AC maintenance", net, VatCategory::Standard));
    draft.legal_entity_id = Some(entity.id);
    let bill = supplier_bill::create(app.db(), &draft, Some(7))
        .await
        .unwrap();
    submit(app.state(), By::System(Some(7)), bill.id)
        .await
        .unwrap();
    approve_bill(app.state(), By::System(Some(8)), bill.id, "")
        .await
        .unwrap()
}

struct Scene {
    app: TestApp,
    accountant: User,
    supplier: PartyId,
    b101: SupplierBill,
    b102: SupplierBill,
    seen: Seen,
}

/// CoolAir LLC with B-101 (AED 1,050) and B-102 (AED 2,100), both approved.
async fn scene() -> Scene {
    let seen = Seen::default();
    let app = booted_seeing(&seen).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let supplier = company(&app, "CoolAir LLC").await;
    let b101 = approved_bill(&app, supplier, "B-101", 1, 100_000).await;
    let b102 = approved_bill(&app, supplier, "B-102", 5, 200_000).await;
    assert_eq!((b101.gross_total, b102.gross_total), (105_000, 210_000));
    Scene {
        app,
        accountant,
        supplier,
        b101,
        b102,
        seen,
    }
}

impl Scene {
    fn path(&self) -> String {
        format!("/payments/suppliers/{}/pay", self.supplier.0)
    }

    /// Posts the pay form with `amounts` per bill (`exception` asks for one).
    async fn pay(&self, b101: &str, b102: &str, exception: bool) -> renox::testing::TestResponse {
        let entity = entity(&self.app, "PGP").await.id.to_string();
        let f101 = format!("bill_{}", self.b101.id);
        let f102 = format!("bill_{}", self.b102.id);
        let mut form = vec![
            ("legal_entity_id", entity.as_str()),
            ("method", "transfer"),
            ("bank_account", "ENBD-1"),
            ("reference", "TRF-COOL-1"),
            (f101.as_str(), b101),
            (f102.as_str(), b102),
        ];
        if exception {
            form.push(("exception", "1"));
        }
        self.app.acting_as(&self.accountant);
        self.app.htmx().post(&self.path(), &form).await
    }

    async fn bill(&self, bill: &SupplierBill) -> SupplierBill {
        SupplierBill::find_or_404(self.app.db(), bill.id).await.unwrap()
    }

    async fn payments(&self) -> Vec<Payment> {
        Payment::query()
            .where_(Payment::PARTY_ID.eq(self.supplier.0))
            .order_by(Payment::ID)
            .get(self.app.db())
            .await
            .unwrap()
    }
}

#[renox::test]
async fn a_transfer_of_3150_pays_both_bills_with_one_voucher() {
    let s = scene().await;
    s.pay("1050", "2100", false).await.assert_ok();
    assert_eq!(s.bill(&s.b101).await.status, SupplierBillStatus::Paid);
    assert_eq!(s.bill(&s.b102).await.status, SupplierBillStatus::Paid);
    let payments = s.payments().await;
    assert_eq!(payments.len(), 1, "one payment, one voucher number");
    let payment = &payments[0];
    assert_eq!(payment.direction, Direction::Out);
    assert_eq!(payment.amount, 315_000);
    assert_eq!(payment.status, PaymentStatus::Received);
    assert!(payment.number.is_some(), "a voucher number");
    let seen = s.seen.0.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|e| e.payment.0 == payment.id && e.amount == 315_000),
        "PaymentReceived is emitted for the money out"
    );
}

#[renox::test]
async fn a_transfer_of_1500_pays_the_oldest_bill_first() {
    let s = scene().await;
    s.app.acting_as(&s.accountant);
    s.app
        .get(&format!("{}?amount=1500", s.path()))
        .await
        .assert_ok()
        .assert_see("B-101")
        .assert_see("1050.00")
        .assert_see("450.00");
    s.pay("1050", "450", false).await.assert_ok();
    assert_eq!(s.bill(&s.b101).await.status, SupplierBillStatus::Paid);
    let b102 = s.bill(&s.b102).await;
    assert_eq!(b102.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!(b102.balance, 210_000 - 45_000);
}

#[renox::test]
async fn a_blocked_supplier_is_paid_once_when_the_exception_is_approved() {
    let s = scene().await;
    let finance = person(&s.app, "Fiona", FINANCE_MANAGER).await;
    block(&s.app, s.supplier).await;
    let refused = s.pay("1050", "2100", false).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "the on-hold text");
    assert!(
        s.payments()
            .await
            .iter()
            .all(|p| p.status != PaymentStatus::Received),
        "nothing is recorded"
    );

    s.pay("1050", "2100", true).await.assert_ok();
    let requests = ApprovalRequest::query()
        .where_(ApprovalRequest::ACTION.eq(BLOCK_EXCEPTION_ACTION))
        .get(s.app.db())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1, "one exception request");
    assert_eq!(s.bill(&s.b101).await.status, SupplierBillStatus::Approved);

    approvals::approve(s.app.state(), requests[0].id, &finance, "approved")
        .await
        .unwrap();
    let received: Vec<Payment> = s
        .payments()
        .await
        .into_iter()
        .filter(|p| p.status == PaymentStatus::Received)
        .collect();
    assert_eq!(received.len(), 1, "recorded once");
    assert_eq!(received[0].recorded_by, Some(s.accountant.id), "as the accountant");
    assert_eq!(received[0].amount, 315_000, "with the stored amounts");
    assert_eq!(s.bill(&s.b101).await.status, SupplierBillStatus::Paid);
    assert_eq!(s.bill(&s.b102).await.status, SupplierBillStatus::Paid);
}

#[renox::test]
async fn a_rejected_exception_records_nothing_and_tells_the_accountant() {
    let s = scene().await;
    let finance = person(&s.app, "Fiona", FINANCE_MANAGER).await;
    block(&s.app, s.supplier).await;
    s.pay("1050", "2100", true).await.assert_ok();
    let request = ApprovalRequest::query()
        .where_(ApprovalRequest::ACTION.eq(BLOCK_EXCEPTION_ACTION))
        .first(s.app.db())
        .await
        .unwrap()
        .expect("the request");
    s.app.fake_notifications();
    approvals::reject(s.app.state(), request.id, &finance, "still under review")
        .await
        .unwrap();
    s.app.assert_notified(&s.accountant, "approval-decided");
    assert!(
        s.payments()
            .await
            .iter()
            .all(|p| p.status != PaymentStatus::Received)
    );
    assert_eq!(s.bill(&s.b101).await.status, SupplierBillStatus::Approved);
    assert_eq!(s.bill(&s.b102).await.status, SupplierBillStatus::Approved);
}

#[renox::test]
async fn reversing_the_3150_payment_reopens_both_bills() {
    let s = scene().await;
    let manager = person(&s.app, "Pam", PROPERTY_MANAGER).await;
    s.pay("1050", "2100", false).await.assert_ok();
    let payment = s.payments().await.remove(0);
    s.app.acting_as(&s.accountant);
    s.app
        .htmx()
        .post(
            &format!("/payments/{}/reverse", payment.id),
            &[("reason", "entered_in_error"), ("note", "Paid the wrong vendor")],
        )
        .await
        .assert_ok();
    // In 01 a property manager approves the reversal (`PRP.ACL.16`).
    let mut pending = approvals::pending_for(s.app.db(), &payment.subject())
        .await
        .unwrap();
    if let Some(request) = pending.pop() {
        approvals::approve(s.app.state(), request.id, &manager, "agreed")
            .await
            .unwrap();
    }
    assert_eq!(
        Payment::find_or_404(s.app.db(), payment.id).await.unwrap().status,
        PaymentStatus::Reversed
    );
    assert_eq!(s.bill(&s.b101).await.status, SupplierBillStatus::Approved);
    assert_eq!(s.bill(&s.b102).await.status, SupplierBillStatus::Approved);
}

#[renox::test]
async fn an_amount_above_a_bill_balance_is_refused() {
    let s = scene().await;
    s.pay("1100", "", false).await.assert_invalid("bills");
    assert!(s.payments().await.is_empty());
    assert_eq!(s.bill(&s.b101).await.status, SupplierBillStatus::Approved);
}
