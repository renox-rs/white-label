//! BL-FNB-08 (#372), 06 §4.2 row *Complaint after delivery*, §10: a *no goods*
//! return of an order whose responsible party is the rider, and a failed
//! delivery closed with the rider responsible, write a *rider fault* entry
//! (order, category, amount, date) for the rider of the delivery's last run,
//! seen by the kitchen manager and HR and by no other rider, with no pay
//! effect; the refunds report groups refunds after completion by category and
//! responsible party, net and gross, with the aggregators' refunds taken from
//! payouts; the failed deliveries report groups the failures by reason. The
//! modules are the real wl-sales, wl-payments, wl-inventory and wl-delivery.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::order::{decision, party, reason};
use fnb::reports::{self, ReportFilter};
use fnb::{FnbOrder, FnbOrderStatus, RiderFault};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::today;
use wl_sales::sale_line::SaleLine;

use support::payouts::{complaints_clerk, completed, file, import, staff, statement, void};
use support::{World, boot_own_riders};

/// One shawarma plate at AED 95.00 (ORD-950).
const PREPAID: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "95.00")];

/// Two shawarma plates at AED 30.00: AED 60.00 (ORD-504).
const TWO_PLATES: [(&str, &str, &str); 1] = [("shawarma-plate", "2", "30.00")];

/// One shawarma plate at AED 85.00 (ORD-501).
const PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "85.00")];

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

/// A prepaid order delivered by the rider: `completed`, with its last run's
/// rider known.
async fn delivered(w: &World, lines: &[(&str, &str, &str)]) -> FnbOrder {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    let customer = w.customer(&format!("Mariam Al Hosani {n}")).await;
    let (order, delivery, _run) = w.dispatched(customer, "online", lines).await;
    w.deliver_stop(&delivery, "0").await.assert_status(303);
    let done = w.reload(&order).await;
    assert_eq!(done.status, FnbOrderStatus::Completed);
    assert_eq!(
        done.rider_user_id,
        Some(w.rider.id),
        "the rider of the last run"
    );
    done
}

/// The clerk files a complaint refund on `order`'s first line: *cold*,
/// responsible `party`, `amount` dirhams.
async fn complain(w: &World, clerk: &User, order: &FnbOrder, party: &str, amount: &str) {
    let sales = support::payouts::sales_of(w, order).await;
    let line = SaleLine::of(w.db(), &sales.subject())
        .await
        .unwrap()
        .remove(0);
    let key = format!("line_{}_refund", line.id);
    w.post(
        clerk,
        &format!("/sales-orders/{}/refund", sales.id),
        &[
            ("reason", "The food arrived cold"),
            ("category", "cold"),
            ("party", party),
            (key.as_str(), amount),
        ],
    )
    .await
    .assert_status(303);
}

/// A period around today.
fn around_today() -> ReportFilter {
    ReportFilter {
        from: Some(today() - TimeDelta::days(1)),
        to: Some(today() + TimeDelta::days(1)),
        ..ReportFilter::default()
    }
}

/// AC1: given a *no goods* return on ORD-950 (cold, AED 30.00, party *rider*,
/// rider R), R's record shows one *rider fault* entry for ORD-950; the kitchen
/// manager and HR see it, other riders do not.
#[renox::test]
async fn a_complaint_blamed_on_the_rider_writes_one_rider_fault_for_the_rider_of_the_run() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let clerk = complaints_clerk(&w).await;
    let order = delivered(&w, &PREPAID).await;

    complain(&w, &clerk, &order, "driver_or_rider", "30.00").await;

    let faults = RiderFault::newest(w.db(), None, 10).await.unwrap();
    assert_eq!(faults.len(), 1);
    let fault = &faults[0];
    assert_eq!(fault.order_id, order.id);
    assert_eq!(fault.order_number, order.number);
    assert_eq!(fault.rider_user_id, Some(w.rider.id));
    assert_eq!(fault.category, "cold");
    assert_eq!(fault.amount, 3_000);
    assert_eq!(fault.occurred_on, today());
    // A complaint blamed on the business writes none.
    let other = delivered(&w, &PREPAID).await;
    complain(&w, &clerk, &other, "business", "20.00").await;
    assert_eq!(RiderFault::newest(w.db(), None, 10).await.unwrap().len(), 1);

    // HR and the kitchen manager see the entry; riders and others do not.
    let manager = support::make_user(
        w.db(),
        "Mona",
        &[
            fnb::permissions::RIDER_FAULTS_VIEW,
            fnb::permissions::ORDERS_VIEW,
        ],
    )
    .await;
    for viewer in [&s.hr, &manager] {
        w.get(viewer, "/rider-faults")
            .await
            .assert_ok()
            .assert_see(&order.number)
            .assert_see("Rami");
    }
    for blocked in [&w.rider, &w.other_rider, &w.cook, &w.cashier, &w.nobody] {
        w.get(blocked, "/rider-faults").await.assert_status(403);
    }
    // Nothing touches pay: the entry is only a record.
    assert!(fault.sales_return_id.is_some());
}

/// AC2: the refunds report for October shows AED 30.00 under *cold* / *rider*,
/// net and gross, and the filters narrow it.
#[renox::test]
async fn the_refunds_report_shows_the_complaint_under_cold_and_rider() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let clerk = complaints_clerk(&w).await;
    let order = delivered(&w, &PREPAID).await;
    complain(&w, &clerk, &order, "driver_or_rider", "30.00").await;

    let report = reports::refunds(w.db(), &around_today()).await.unwrap();
    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(
        (group.category.as_str(), group.party.as_str()),
        ("cold", "driver_or_rider")
    );
    assert_eq!((group.count, group.gross, group.net), (1, 3_000, 3_000));
    assert_eq!((report.gross, report.net), (3_000, 3_000));

    // Filters: brand, outlet, channel and period.
    let by_brand = ReportFilter {
        brand: order.brand_id,
        ..around_today()
    };
    assert_eq!(
        reports::refunds(w.db(), &by_brand)
            .await
            .unwrap()
            .facts
            .len(),
        1
    );
    let other_brand = ReportFilter {
        brand: Some(order.brand_id.unwrap_or(0) + 1000),
        ..around_today()
    };
    assert!(
        reports::refunds(w.db(), &other_brand)
            .await
            .unwrap()
            .facts
            .is_empty()
    );
    let other_channel = ReportFilter {
        channel: Some("aggregator_a".to_owned()),
        ..around_today()
    };
    assert!(
        reports::refunds(w.db(), &other_channel)
            .await
            .unwrap()
            .facts
            .is_empty()
    );
    let last_month = ReportFilter {
        to: Some(today() - TimeDelta::days(40)),
        ..ReportFilter::default()
    };
    assert!(
        reports::refunds(w.db(), &last_month)
            .await
            .unwrap()
            .facts
            .is_empty()
    );

    // The page: the person with the permission sees it, others do not.
    let from = (today() - TimeDelta::days(1)).to_string();
    let to = (today() + TimeDelta::days(1)).to_string();
    w.get(&s.reports, &format!("/reports/refunds?from={from}&to={to}"))
        .await
        .assert_ok()
        .assert_see("Cold")
        .assert_see("Rider")
        .assert_see("30.00");
    w.get(&w.cashier, "/reports/refunds")
        .await
        .assert_status(403);
}

/// AC3: the failed deliveries report lists ORD-504 under customer
/// unreachable.
#[renox::test]
async fn the_failed_deliveries_report_lists_the_order_under_customer_unreachable() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    let customer = w.customer(&format!("Mariam Al Hosani {n}")).await;
    let (order, delivery, _run) = w.dispatched(customer, "online", &TWO_PLATES).await;
    w.fail_stop(&delivery, "not_home").await.assert_status(303);
    assert_eq!(order.total, 6_000);

    let groups = reports::failed_deliveries(w.db(), &around_today())
        .await
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].reason, reason::CUSTOMER_UNREACHABLE);
    assert_eq!(groups[0].orders.len(), 1);
    assert_eq!(groups[0].orders[0].id, order.id);
    assert_eq!(groups[0].gross, 6_000);
    let other_channel = ReportFilter {
        channel: Some("aggregator_a".to_owned()),
        ..around_today()
    };
    assert!(
        reports::failed_deliveries(w.db(), &other_channel)
            .await
            .unwrap()
            .is_empty()
    );

    w.get(&s.reports, "/reports/failed-deliveries")
        .await
        .assert_ok()
        .assert_see(&order.number)
        .assert_see("Customer unreachable");
    w.get(&w.nobody, "/reports/failed-deliveries")
        .await
        .assert_status(403);
}

/// `close_failed` with the rider responsible adds the fault; a customer-side
/// failure does not.
#[renox::test]
async fn a_failed_delivery_closed_with_the_rider_responsible_writes_a_rider_fault() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    // The rider could not go on: the party proposed is the rider.
    let customer = w.customer("Mariam Al Hosani R").await;
    let (order, delivery, run) = w.dispatched(customer, "online", &PLATE).await;
    w.fail_stop(&delivery, "rider_issue")
        .await
        .assert_status(303);
    w.bring_back(&delivery, &run).await;
    let closed = w.reload(&order).await;
    assert_eq!(closed.status, FnbOrderStatus::Cancelled);
    assert_eq!(closed.responsible_party.as_deref(), Some(party::RIDER));
    assert_eq!(
        closed.refund_decision.as_deref(),
        Some(decision::FULL_REFUND)
    );

    let faults = RiderFault::newest(w.db(), None, 10).await.unwrap();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].order_id, order.id);
    assert_eq!(faults[0].rider_user_id, Some(w.rider.id));
    assert_eq!(faults[0].category, reason::RIDER_ISSUE);
    assert_eq!(faults[0].amount, 8_500);
    assert!(faults[0].sales_return_id.is_none());
    w.get(&s.hr, "/rider-faults")
        .await
        .assert_ok()
        .assert_see(&order.number);

    // The customer was not home: no fault.
    let customer = w.customer("Mariam Al Hosani C").await;
    let (second, delivery, run) = w.dispatched(customer, "online", &TWO_PLATES).await;
    w.fail_stop(&delivery, "not_home").await.assert_status(303);
    w.bring_back(&delivery, &run).await;
    assert_eq!(w.reload(&second).await.status, FnbOrderStatus::Cancelled);
    assert_eq!(RiderFault::newest(w.db(), None, 10).await.unwrap().len(), 1);
}

/// AC4: the refunds report lists ORD-1001's aggregator refund (BL-FNB-10) AED
/// 25.00 under *cold* / *business*, the party set on `match`; a voided
/// statement takes it out again.
#[renox::test]
async fn the_refunds_report_includes_the_aggregator_refund_taken_from_a_payout() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let order = completed(&w, "ORD-1001", "60.00").await;
    import(
        &w,
        &s.accountant,
        "AGG-W41",
        &file(&["ORD-1001,60.00,12.00,0,25.00,23.00,Food was cold"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W41").await;
    w.post(
        &s.accountant,
        &format!("/payouts/{}/match", st.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);

    let report = reports::refunds(w.db(), &around_today()).await.unwrap();
    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(
        (group.category.as_str(), group.party.as_str()),
        ("cold", "business")
    );
    assert_eq!((group.gross, group.net), (2_500, 2_500));
    assert_eq!(report.facts[0].resolution, "refunded_by_channel");
    assert_eq!(report.facts[0].order_id, Some(order.id));
    let channel = ReportFilter {
        channel: Some("aggregator_a".to_owned()),
        ..around_today()
    };
    assert_eq!(
        reports::refunds(w.db(), &channel)
            .await
            .unwrap()
            .facts
            .len(),
        1
    );
    w.get(&s.reports, "/reports/refunds")
        .await
        .assert_ok()
        .assert_see("Refunded by the aggregator")
        .assert_see("25.00");

    // A voided statement takes its return back, and the report with it.
    void(&w, &s.accountant, st.id, "wrong week").await;
    let request = ApprovalRequest::pending_about(w.db(), &st.subject())
        .await
        .unwrap()
        .remove(0);
    approvals::approve(w.state(), request.id, &s.approver, "")
        .await
        .unwrap();
    assert!(
        reports::refunds(w.db(), &around_today())
            .await
            .unwrap()
            .facts
            .is_empty()
    );
}
