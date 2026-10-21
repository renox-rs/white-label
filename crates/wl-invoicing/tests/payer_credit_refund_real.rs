//! Refunding a payer credit balance with the real wl-payments (wl-payments
//! #101, wl-invoicing §3 *Payer credit balance*, `FIN.ACL.16`, `FIN.ACL.04`):
//! the refund's approval is the movement's approval, a refund above the
//! balance is refused, and a failed refund gives the amount back.

mod support;

use renox::auth::{Permissions, permissions};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::payer_credit::{self, OVERPAYMENT, PayerCreditMovement, REFUND, REFUND_RESTORED};
use wl_invoicing::{PayerAgreement, VatRecipient, WlInvoicing, permissions as inv};
use wl_kernel::payer_bills::{NewPayerBill, NewPayerBillLine, payer_bills, payer_credit_subject};
use wl_kernel::payments::{
    PaymentMethod, RefundAttemptRef, RefundFailed, RefundFailedFrom, RefundFailureKind, RefundId,
    RefundRequest, RefundSource, payment_recorder,
};
use wl_kernel::{ClientConfig, LegalEntity};
use wl_payments::refund::{Refund, RefundStatus};
use wl_payments::{WlPayments, permissions as pay};

use support::{OMAR, QUIET_SANDS, Recording, TestDirectory};

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
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
"#;

struct Real {
    app: TestApp,
    _views: tempfile::TempDir,
}

/// wl-invoicing and wl-payments in one app.
async fn boot() -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let directory: std::sync::Arc<dyn wl_kernel::parties::PartyDirectory> =
        std::sync::Arc::new(TestDirectory::default());
    let notifier: std::sync::Arc<dyn wl_kernel::notify::Notifier> =
        std::sync::Arc::new(Recording::default());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlInvoicing::new())
        .module(WlPayments::new())
        .provide(directory)
        .provide(notifier);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    Real { app, _views: views }
}

async fn user(t: &Real, name: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.app.db(), name, &email, "password123")
        .await
        .unwrap();
    let role = format!("role_{}", name.to_lowercase());
    permissions::define_role(t.app.db(), &role, grants)
        .await
        .unwrap();
    user.assign_role(t.app.db(), &role).await.unwrap();
    user
}

/// A user who requests, approves and pays refunds.
async fn clerk(t: &Real, name: &str) -> User {
    user(
        t,
        name,
        &[
            pay::VIEW,
            pay::REFUNDS_REQUEST,
            pay::REFUNDS_APPROVE,
            pay::REFUNDS_PAY,
        ],
    )
    .await
}

/// The id of PGP.
async fn entity(t: &Real) -> i64 {
    LegalEntity::query()
        .where_(LegalEntity::CODE.eq("PGP"))
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap()
        .id
}

/// The payer overpaid AED 1,200.00 and nothing was set off.
async fn overpaid(t: &Real, entity: i64) {
    PayerCreditMovement {
        payer_party_id: QUIET_SANDS.0,
        legal_entity_id: entity,
        currency: "AED".to_owned(),
        kind: OVERPAYMENT.to_owned(),
        amount: 120_000,
        ..Default::default()
    }
    .insert(t.app.db())
    .await
    .unwrap();
}

fn request(entity: i64, amount: i64) -> RefundRequest {
    RefundRequest {
        source: RefundSource::PayerCredit(payer_credit_subject(QUIET_SANDS)),
        party: QUIET_SANDS,
        legal_entity_id: entity,
        amount,
        currency: "AED".to_owned(),
        method: Some(PaymentMethod::Transfer),
        gateway_txn: None,
        reason: "Overpayment not set off".to_owned(),
    }
}

async fn balance(t: &Real, entity: i64) -> i64 {
    payer_credit::balance(t.app.db(), QUIET_SANDS, entity, "AED")
        .await
        .unwrap()
}

async fn movements(t: &Real, refund: i64) -> Vec<PayerCreditMovement> {
    PayerCreditMovement::query()
        .where_(PayerCreditMovement::REFUND_ID.eq(refund))
        .order_by(PayerCreditMovement::ID)
        .get(t.app.db())
        .await
        .unwrap()
}

/// Given a payer credit balance of AED 1,200.00, user A requests a refund of
/// AED 1,200.00; user B approves; a payer credit balance movement *refund* of
/// -1,200.00 is recorded with the approval (B's). A refund of AED 1,300.00 is
/// refused.
#[renox::test]
async fn the_approval_of_the_refund_is_the_approval_of_the_movement() {
    let t = boot().await;
    let entity = entity(&t).await;
    overpaid(&t, entity).await;
    let a = clerk(&t, "Amal").await;
    let b = clerk(&t, "Bilal").await;
    let recorder = payment_recorder(t.app.state());

    // Above the balance: refused, nothing is stored.
    let refused = recorder
        .request_refund(t.app.state(), request(entity, 130_000), Some(a.id))
        .await;
    assert!(refused.is_err(), "AED 1,300.00 is above the balance");
    assert_eq!(Refund::query().count(t.app.db()).await.unwrap(), 0);

    let id = recorder
        .request_refund(t.app.state(), request(entity, 120_000), Some(a.id))
        .await
        .expect("the refund is requested")
        .0;
    // Not taken before the approval.
    assert_eq!(balance(&t, entity).await, 120_000);
    assert!(movements(&t, id).await.is_empty());

    // The requester cannot approve it; another user can.
    let approve = format!("/refunds/{id}/transitions/approve");
    t.app.acting_as(&a);
    t.app
        .post(&approve, &[("reason", "")])
        .await
        .assert_status(422);
    assert_eq!(
        Refund::find_or_404(t.app.db(), id).await.unwrap().status,
        RefundStatus::Requested
    );
    t.app.acting_as(&b);
    t.app
        .post(&approve, &[("reason", "")])
        .await
        .assert_status(303);
    assert_eq!(
        Refund::find_or_404(t.app.db(), id).await.unwrap().status,
        RefundStatus::Approved
    );

    // One movement: the refund, -1,200.00, with B's approval.
    let rows = movements(&t, id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, REFUND);
    assert_eq!(rows[0].amount, -120_000);
    assert_eq!(rows[0].approved_by, Some(b.id));
    assert_eq!(rows[0].payer_party_id, QUIET_SANDS.0);
    assert_eq!(rows[0].legal_entity_id, entity);
    assert_eq!(balance(&t, entity).await, 0);

    // Nothing is left to ask for.
    let again = recorder
        .request_refund(t.app.state(), request(entity, 100), Some(a.id))
        .await;
    assert!(again.is_err());
}

/// The approval is given once: approving again changes nothing (the refund
/// is not `requested` any more), and the refund holds the amount once.
#[renox::test]
async fn a_second_approval_takes_nothing_more() {
    let t = boot().await;
    let entity = entity(&t).await;
    overpaid(&t, entity).await;
    let a = clerk(&t, "Amal").await;
    let b = clerk(&t, "Bilal").await;
    let id = payment_recorder(t.app.state())
        .request_refund(t.app.state(), request(entity, 50_000), Some(a.id))
        .await
        .unwrap()
        .0;
    let approve = format!("/refunds/{id}/transitions/approve");
    t.app.acting_as(&b);
    t.app
        .post(&approve, &[("reason", "")])
        .await
        .assert_status(303);
    t.app
        .post(&approve, &[("reason", "")])
        .await
        .assert_status(422);
    assert_eq!(movements(&t, id).await.len(), 1);
    assert_eq!(balance(&t, entity).await, 70_000);
}

/// A refund that fails gives the amount back to the balance (BL-PAY-06).
#[renox::test]
async fn a_failed_refund_gives_the_amount_back_to_the_balance() {
    let t = boot().await;
    let entity = entity(&t).await;
    overpaid(&t, entity).await;
    let a = clerk(&t, "Amal").await;
    let b = clerk(&t, "Bilal").await;
    let id = payment_recorder(t.app.state())
        .request_refund(t.app.state(), request(entity, 120_000), Some(a.id))
        .await
        .unwrap()
        .0;
    t.app.acting_as(&b);
    t.app
        .post(
            &format!("/refunds/{id}/transitions/approve"),
            &[("reason", "")],
        )
        .await
        .assert_status(303);
    assert_eq!(balance(&t, entity).await, 0);

    // The refund fails: wl-payments says so and names its source.
    let failed = RefundFailed {
        refund: RefundId(id),
        party: QUIET_SANDS,
        amount: 120_000,
        currency: "AED".to_owned(),
        source: RefundSource::PayerCredit(payer_credit_subject(QUIET_SANDS)),
        method: PaymentMethod::Transfer,
        kind: RefundFailureKind::BankReturn,
        reason: "Account closed".to_owned(),
        attempt: RefundAttemptRef { id: 1, number: 1 },
        from: RefundFailedFrom::Approved,
        outgoing_payment: None,
    };
    t.app.state().emit(failed.clone()).await.unwrap();
    let rows = movements(&t, id).await;
    let kinds: Vec<_> = rows.iter().map(|m| m.kind.as_str()).collect();
    assert_eq!(kinds, [REFUND, REFUND_RESTORED]);
    assert_eq!(rows[1].amount, 120_000);
    assert_eq!(balance(&t, entity).await, 120_000);
    // The same failure delivered again changes nothing.
    t.app.state().emit(failed).await.unwrap();
    assert_eq!(movements(&t, id).await.len(), 2);
    assert_eq!(balance(&t, entity).await, 120_000);
}

/// The payer bill page of the payer lists the *refund* movement with the
/// name of the user who approved it (#101, "Refund + approver").
#[renox::test]
async fn the_payer_bill_page_lists_the_refund_with_its_approver() {
    let t = boot().await;
    let entity = entity(&t).await;
    overpaid(&t, entity).await;
    let a = clerk(&t, "Amal").await;
    let b = clerk(&t, "Bilal").await;
    let id = payment_recorder(t.app.state())
        .request_refund(t.app.state(), request(entity, 120_000), Some(a.id))
        .await
        .unwrap()
        .0;
    t.app.acting_as(&b);
    t.app
        .post(
            &format!("/refunds/{id}/transitions/approve"),
            &[("reason", "")],
        )
        .await
        .assert_status(303);

    // A payer bill of the same payer, so its page shows the payer's credit.
    let mut agreement = PayerAgreement {
        legal_entity_id: entity,
        payer_party_id: QUIET_SANDS.0,
        name: "Daman Gold".to_owned(),
        active: true,
        vat_recipient: VatRecipient::Customer,
        first_submission_days: 30,
        resubmission_days: 15,
        resubmission_limit: 2,
        write_off_limit: 100_000,
        ..Default::default()
    };
    agreement.insert(t.app.db()).await.expect("the agreement");
    let bill = payer_bills(t.app.state())
        .create(
            t.app.state(),
            NewPayerBill::new(
                agreement.id,
                OMAR,
                wl_kernel::Subject::new("clinics.claim", 1),
                wl_kernel::documents::today(),
            )
            .line(NewPayerBillLine::new("Consultation", 40_000)),
            None,
        )
        .await
        .expect("the payer bill is made");

    let viewer = user(&t, "Vera", &[inv::PAYER_BILLS_VIEW]).await;
    t.app.acting_as(&viewer);
    let page = t.app.get(&format!("/payer-bills/{}", bill.0)).await;
    page.assert_ok()
        .assert_see("Refund")
        .assert_see("Bilal")
        .assert_see("1,200.00");
}
