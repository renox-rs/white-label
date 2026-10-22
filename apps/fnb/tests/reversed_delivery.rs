//! BL-FNB-07 (#371), 06 §4.2 row *Customer says the order never arrived*, §7.1
//! *Order*, `WRK.ACL.10`, `SAL.ACL.06`: a delivered order the customer says
//! never arrived stays `completed` while its delivery goes under review; if
//! the kitchen manager finds the rider wrong and reverses the delivery (a
//! second manager approves, never the rider), the customer is refunded in full
//! by a sales return of kind *no goods* with the rider as the responsible
//! party; on cash on delivery nobody handed over, the same return is resolved
//! by credit note only; an upheld review changes nothing. The modules are the
//! real wl-delivery, wl-sales, wl-payments (with its cash custody) and
//! wl-inventory.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::FnbOrderStatus;
use fnb::order::payment;
use renox::prelude::*;
use wl_delivery::{Delivery, DeliveryStatus, Run};
use wl_kernel::Subject;
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::custody::custody;
use wl_kernel::sales::SALES_RETURN_SUBJECT_TYPE;
use wl_payments::refund::Refund;
use wl_sales::{ReturnKind, ReturnResolution, ReturnStatus, SalesOrder, SalesReturn};

use support::{World, boot_own_riders};

/// One shawarma plate at AED 95.00 (ORD-901).
const PREPAID: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "95.00")];

/// One falafel wrap at AED 70.00 (ORD-902).
const CASH: [(&str, &str, &str); 1] = [("falafel-wrap", "1", "70.00")];

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

/// An order delivered by the rider, with `cash` dirhams collected.
async fn delivered(
    w: &World,
    mode: &str,
    lines: &[(&str, &str, &str)],
    cash: &str,
) -> (fnb::FnbOrder, Delivery, Run) {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    let customer = w.customer(&format!("Mariam Al Hosani {n}")).await;
    let (order, delivery, run) = w.dispatched(customer, mode, lines).await;
    w.deliver_stop(&delivery, cash).await.assert_status(303);
    let done = w.reload(&order).await;
    assert_eq!(done.status, FnbOrderStatus::Completed);
    (done, delivery, run)
}

/// The kitchen manager puts the delivery under review.
async fn open_review(w: &World, delivery: &Delivery) {
    w.post(
        &w.manager,
        &format!("/deliveries/{}/open-review", delivery.id),
        &[("reason", "The customer says nobody came")],
    )
    .await
    .assert_ok();
    assert_eq!(
        w.delivery(delivery).await.status,
        DeliveryStatus::UnderReview
    );
}

/// The kitchen manager asks to reverse the delivery and the second one
/// approves it.
async fn reverse(w: &World, delivery: &Delivery) {
    w.post(
        &w.manager,
        &format!("/deliveries/{}/reverse", delivery.id),
        &[
            ("reason", "recipient_denies"),
            ("liability", "driver"),
            ("note", "The photo shows another building"),
        ],
    )
    .await
    .assert_ok();
    w.post(
        &w.manager_b,
        &format!("/deliveries/{}/reverse/approve", delivery.id),
        &[("cash_treatment", "")],
    )
    .await
    .assert_ok();
}

/// The returns of `order`'s sales order.
async fn returns_of(w: &World, order: &fnb::FnbOrder) -> Vec<SalesReturn> {
    SalesReturn::query()
        .where_(SalesReturn::SALE_ID.eq(order.sales_order_id))
        .order_by(SalesReturn::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The refunds asked for `order`'s sales order.
async fn refunds_of(w: &World, order: &fnb::FnbOrder) -> Vec<Refund> {
    Refund::query()
        .where_(Refund::SOURCE_ID.eq(order.sales_order_id))
        .where_(Refund::SOURCE_KIND.eq("order"))
        .order_by(Refund::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// AC1: given prepaid ORD-901 (AED 95.00) `completed`, when the kitchen
/// manager reverses the delivery after review, a *no goods* return of AED
/// 95.00 with party *rider* is resolved, AED 95.00 is refunded, and ORD-901
/// stays `completed` with amount refunded AED 95.00.
#[renox::test]
async fn reversing_the_delivery_of_a_prepaid_order_refunds_it_in_full_and_it_stays_completed() {
    let w = boot_own_riders().await;
    let (order, delivery, _run) = delivered(&w, "online", &PREPAID, "0").await;

    open_review(&w, &delivery).await;
    // Under review the order stays completed and nothing is refunded.
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::Completed);
    assert!(returns_of(&w, &order).await.is_empty());

    reverse(&w, &delivery).await;

    let after = w.reload(&order).await;
    assert_eq!(
        after.status,
        FnbOrderStatus::Completed,
        "never reopened or cancelled"
    );
    assert_eq!(after.refund_decision, None);
    let returns = returns_of(&w, &after).await;
    assert_eq!(returns.len(), 1);
    let ret = &returns[0];
    assert_eq!(ret.kind, ReturnKind::NoGoods);
    assert_eq!(ret.status, ReturnStatus::Resolved);
    assert_eq!(ret.resolution, Some(ReturnResolution::Refund));
    assert_eq!(ret.gross_value, 9_500);
    assert_eq!(ret.responsible_party.as_deref(), Some("driver_or_rider"));
    assert_eq!(ret.complaint_category.as_deref(), Some("other"));
    assert!(ret.reason.contains("not delivered"));
    // No second approval: the kitchen manager's reversal counts.
    let waiting =
        ApprovalRequest::pending_about(w.db(), &Subject::new(SALES_RETURN_SUBJECT_TYPE, ret.id))
            .await
            .unwrap();
    assert!(waiting.is_empty());
    // AED 95.00 is asked of wl-payments, and the order's amount refunded is
    // the whole amount.
    let refunds = refunds_of(&w, &after).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 9_500);
    let sales = SalesOrder::find_or_404(w.db(), after.sales_order_id)
        .await
        .unwrap();
    assert_eq!(sales.refunded_amount, 9_500);
    // The order remembers the return.
    assert_eq!(after.reversal_return_id, Some(ret.id));
    assert_eq!(after.reversal_resolution.as_deref(), Some("refund"));
    assert_eq!(after.reversal_amount, 9_500);
    assert_eq!(after.reversal_party.as_deref(), Some("driver_or_rider"));
    // The page shows it.
    let page = w.get(&w.manager, &format!("/orders/{}", order.id)).await;
    page.assert_ok();
    assert!(page.text().contains("Delivery reversed"));
    assert!(page.text().contains("Refunded after completion"));
}

/// AC2: given COD ORD-902 (AED 70.00) with no cash handed over, the reversal
/// creates a *no goods* return of AED 70.00 resolved by credit note, no refund
/// payment, and the rider's expected cash no longer includes ORD-902.
#[renox::test]
async fn reversing_cash_nobody_handed_over_is_a_credit_note_and_leaves_the_riders_expected_cash() {
    let w = boot_own_riders().await;
    let (order, delivery, run) = delivered(&w, payment::CASH_ON_DELIVERY, &CASH, "70.00").await;
    let before = custody(w.state())
        .handover_of(w.db(), w.rider.id, &run.subject())
        .await
        .unwrap()
        .expect("the rider's handover of the run");
    assert_eq!(before.expected, 7_000);

    open_review(&w, &delivery).await;
    reverse(&w, &delivery).await;

    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::Completed);
    let returns = returns_of(&w, &after).await;
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].kind, ReturnKind::NoGoods);
    assert_eq!(returns[0].resolution, Some(ReturnResolution::CreditNote));
    assert_eq!(returns[0].gross_value, 7_000);
    assert_eq!(returns[0].status, ReturnStatus::Resolved);
    assert_eq!(after.reversal_resolution.as_deref(), Some("credit_note"));
    // No refund payment; the amount due is cleared.
    assert!(refunds_of(&w, &after).await.is_empty());
    let sales = SalesOrder::find_or_404(w.db(), after.sales_order_id)
        .await
        .unwrap();
    assert_eq!(sales.refunded_amount, 7_000);
    // The cash left the rider's expected cash.
    let handover = custody(w.state())
        .handover_of(w.db(), w.rider.id, &run.subject())
        .await
        .unwrap()
        .expect("the handover");
    assert_eq!(handover.expected, 0);
}

/// AC3: when the review closes upheld, no return exists and nothing changes.
#[renox::test]
async fn an_upheld_review_raises_no_return() {
    let w = boot_own_riders().await;
    let (order, delivery, _run) = delivered(&w, "online", &PREPAID, "0").await;
    open_review(&w, &delivery).await;

    w.post(
        &w.manager,
        &format!("/deliveries/{}/close-review", delivery.id),
        &[("outcome", "upheld"), ("note", "The photo shows the door")],
    )
    .await
    .assert_ok();

    assert_eq!(
        w.delivery(&delivery).await.status,
        DeliveryStatus::Delivered
    );
    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::Completed);
    assert!(returns_of(&w, &after).await.is_empty());
    assert!(refunds_of(&w, &after).await.is_empty());
    assert_eq!(after.reversal_return_id, None);
    let sales = SalesOrder::find_or_404(w.db(), after.sales_order_id)
        .await
        .unwrap();
    assert_eq!(sales.refunded_amount, 0);
}

/// AC4: the rider cannot open the reversal, nor approve it; the requester
/// cannot approve their own request.
#[renox::test]
async fn the_rider_cannot_open_or_approve_the_reversal() {
    let w = boot_own_riders().await;
    let (order, delivery, _run) = delivered(&w, "online", &PREPAID, "0").await;
    open_review(&w, &delivery).await;
    let form = [
        ("reason", "recipient_denies"),
        ("liability", "driver"),
        ("note", "x"),
    ];
    let path = format!("/deliveries/{}/reverse", delivery.id);

    w.post(&w.rider, &path, &form).await.assert_forbidden();
    w.post(&w.dispatcher, &path, &form).await.assert_forbidden();
    w.post(&w.manager, &path, &form).await.assert_ok();
    // Neither the rider nor the requester approves.
    let approve = format!("/deliveries/{}/reverse/approve", delivery.id);
    w.post(&w.rider, &approve, &[("cash_treatment", "")])
        .await
        .assert_forbidden();
    let own = w
        .post(&w.manager, &approve, &[("cash_treatment", "")])
        .await;
    assert_ne!(own.status.as_u16(), 200);
    assert!(returns_of(&w, &order).await.is_empty());
    assert!(refunds_of(&w, &order).await.is_empty());
}

/// The order page offers the review and the reversal to the manager only.
#[renox::test]
async fn the_order_page_offers_the_review_and_the_reversal_to_the_kitchen_manager() {
    let w = boot_own_riders().await;
    let (order, _delivery, _run) = delivered(&w, "online", &PREPAID, "0").await;
    let path = format!("/orders/{}", order.id);

    let manager = w.get(&w.manager, &path).await;
    manager.assert_ok();
    assert!(manager.text().contains("Customer says it never arrived"));
    assert!(manager.text().contains("Reverse the delivery"));
    let cashier = w.get(&w.cashier, &path).await;
    cashier.assert_ok();
    assert!(!cashier.text().contains("Reverse the delivery"));
    assert!(!cashier.text().contains("Customer says it never arrived"));
}
