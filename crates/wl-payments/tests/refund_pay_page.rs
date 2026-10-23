//! The refund page pays an approved refund: a sheet with the method and the
//! payment reference posts to `/refunds/{id}/pay`; a missing method or reference
//! is refused in words under its field, and the paid refund shows its status.

mod support;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::payments::RefundSource;
use wl_payments::refund::RefundStatus;

use support::{BLUE_DUNES, boot_gateway};
use wl_payments::gateway::FakeGateway;

fn credit_note() -> RefundSource {
    RefundSource::CreditNote(Subject::new("invoicing.credit_note", 7))
}

#[renox::test]
async fn an_approved_refund_is_paid_from_its_page_with_a_method_and_a_reference() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let refund = t
        .request_for_source(&a, BLUE_DUNES, &credit_note(), 50_000, "transfer", "")
        .await;
    // Requested: the page offers no payment yet.
    t.app.acting_as(&b);
    t.app
        .get(&format!("/refunds/{}", refund.id))
        .await
        .assert_ok()
        .assert_dont_see(&format!("pay-refund-{}", refund.id));
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.app.acting_as(&b);
    t.app
        .get(&format!("/refunds/{}", refund.id))
        .await
        .assert_ok()
        .assert_see(&format!("pay-refund-{}", refund.id));

    let url = format!("/refunds/{}/pay", refund.id);
    t.app
        .htmx()
        .post(&url, &[("reference", "BANK-89")])
        .await
        .assert_status(422)
        .assert_see("Choose how the refund is paid.");
    t.app
        .htmx()
        .post(&url, &[("method", "transfer"), ("reference", "  ")])
        .await
        .assert_status(422)
        .assert_see("Give the payment reference.");
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Approved);

    t.app
        .htmx()
        .post(&url, &[("method", "transfer"), ("reference", "BANK-89")])
        .await
        .assert_ok();
    let paid = t.refund(refund.id).await;
    assert_eq!(paid.status, RefundStatus::Paid);
    t.app
        .get(&format!("/refunds/{}", refund.id))
        .await
        .assert_ok()
        .assert_dont_see(&format!("pay-refund-{}", refund.id));
}

#[renox::test]
async fn a_user_who_may_not_pay_refunds_cannot_post_the_payment() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let refund = t
        .request_for_source(&a, BLUE_DUNES, &credit_note(), 50_000, "transfer", "")
        .await;
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    let viewer = t.user("Vera", &[wl_payments::permissions::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post(
            &format!("/refunds/{}/pay", refund.id),
            &[("method", "transfer"), ("reference", "BANK-1")],
        )
        .await
        .assert_status(403);
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Approved);
}
