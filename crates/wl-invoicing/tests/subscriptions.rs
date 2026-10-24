//! The Subscription lifecycle (wl-invoicing §4 *Subscription*, `FIN.RUL.06`;
//! the base of #82, #83 and #84): one cycle invoice per cycle, linked to the
//! subscription; `past_due` on an overdue cycle invoice with the party told;
//! the retries and the suspension after the last one; `settle` when paid;
//! the pages.

mod support;

use wl_invoicing::WlInvoicing;
use wl_invoicing::subscriptions::{PAST_DUE_NOTICE, RETRY_NOTICE};
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::subscriptions::{SUBSCRIPTION_SUBJECT_TYPE, SubscriptionStatus, subscriptions};

use support::subscriptions::{
    GROSS, bill, boot_subscriptions, cycles, d, engagement, evaluate, invoices, now, start,
    with_overdue_cycle,
};
use support::{BLUE_DUNES, boot};

/// Each cycle up to the run day is billed once, as an issued invoice whose
/// source is the subscription; the next cycle date moves on.
#[renox::test]
async fn one_cycle_invoice_is_billed_per_cycle() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let sub = start(&t, &engagement(1), d("2027-01-01")).await;
    assert_eq!(sub.status, SubscriptionStatus::Active);
    assert_eq!(bill(&t, d("2027-03-15")).await, 3);
    assert_eq!(bill(&t, d("2027-03-15")).await, 0, "a cycle is billed once");
    let billed = invoices(&t, &sub).await;
    assert_eq!(billed.len(), 3);
    for invoice in &billed {
        assert_eq!(invoice.status, InvoiceStatus::Issued);
        assert_eq!(invoice.gross_total, GROSS);
        assert_eq!(invoice.source_type, SUBSCRIPTION_SUBJECT_TYPE);
        assert_eq!(invoice.source_id, sub.id);
    }
    let periods: Vec<_> = cycles(&t, &sub)
        .await
        .iter()
        .map(|c| (c.period_start, c.period_end))
        .collect();
    assert_eq!(
        periods,
        [
            (d("2027-01-01"), d("2027-01-31")),
            (d("2027-02-01"), d("2027-02-28")),
            (d("2027-03-01"), d("2027-03-31")),
        ]
    );
    assert_eq!(now(&t, &sub).await.next_cycle_date, d("2027-04-01"));
    let summary = subscriptions(t.state())
        .of_agreement(t.db(), &engagement(1))
        .await
        .unwrap();
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].party, BLUE_DUNES);
    assert_eq!(summary[0].next_cycle_date, d("2027-04-01"));
}

/// Without the setting, the system leaves the cycle invoice as a draft for
/// review.
#[renox::test]
async fn without_auto_issue_the_cycle_invoice_waits_as_a_draft() {
    let t = boot().await;
    let sub = start(&t, &engagement(2), d("2027-01-01")).await;
    assert_eq!(bill(&t, d("2027-01-01")).await, 1);
    assert_eq!(invoices(&t, &sub).await[0].status, InvoiceStatus::Draft);
}

/// `FIN.RUL.06`: past due tells the party; each retry day tells it again;
/// after the last retry it is suspended; paying settles it.
#[renox::test]
async fn an_overdue_cycle_goes_past_due_then_suspended_then_settles() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = with_overdue_cycle(&t, &engagement(3), t.day(-40)).await;
    evaluate(&t, t.day(0)).await;
    let sub = now(&t, &sub).await;
    assert_eq!(sub.status, SubscriptionStatus::PastDue);
    assert_eq!(sub.past_due_on, Some(t.day(0)));
    assert_eq!(t.sub_past_due.all().len(), 1);
    let told = t.notices.of_type(PAST_DUE_NOTICE);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].recipients,
        [NotifyRecipient::Party { id: BLUE_DUNES }]
    );

    for (day, retries) in [(1, 1), (2, 1), (3, 2)] {
        evaluate(&t, t.day(day)).await;
        assert_eq!(now(&t, &sub).await.retries_done, retries, "day {day}");
        assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::PastDue);
    }
    evaluate(&t, t.day(7)).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Suspended);
    assert_eq!(t.notices.of_type(RETRY_NOTICE).len(), 3);
    assert_eq!(t.sub_suspended.all().len(), 1);

    t.pay(&invoice, 1, GROSS).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Active);
    assert_eq!(t.sub_settled.all().len(), 1);
}

/// With suspension off (04), it stays `past_due` after the last retry.
#[renox::test]
async fn without_suspension_it_stays_past_due() {
    let t = boot_subscriptions(WlInvoicing::new().subscription_suspension(false)).await;
    let (sub, _) = with_overdue_cycle(&t, &engagement(4), t.day(-40)).await;
    evaluate(&t, t.day(0)).await;
    evaluate(&t, t.day(10)).await;
    let sub = now(&t, &sub).await;
    assert_eq!(sub.status, SubscriptionStatus::PastDue);
    assert_eq!(sub.retries_done, 3);
    assert!(t.sub_suspended.all().is_empty());
}

/// The list and the page show the subscription and its cycles.
#[renox::test]
async fn the_pages_show_the_subscription() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let sub = start(&t, &engagement(5), d("2027-01-01")).await;
    bill(&t, d("2027-01-01")).await;
    let admin = t.admin_a().await;
    t.app.acting_as(&admin);
    t.app
        .get("/subscriptions")
        .await
        .assert_ok()
        .assert_see("Retainer");
    t.app
        .get(&format!("/subscriptions/{}", sub.id))
        .await
        .assert_ok()
        .assert_see("Engagement #5")
        .assert_see("AED 3,100.00");
    let viewer = t.viewer().await;
    t.app.acting_as(&viewer);
    t.app.get("/subscriptions").await.assert_forbidden();
}
