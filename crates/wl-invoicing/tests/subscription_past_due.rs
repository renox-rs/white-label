//! A subscription goes past due on a reopened cycle invoice, not on a
//! disputed one (wl-invoicing §4 `mark_past_due`, `suspend`, §5.10 step 2,
//! §5.11 step 2, `FIN.RUL.06`; #82, BL-INV-19): the four Done-when points.

mod support;

use wl_invoicing::dispute;
use wl_invoicing::dispute_resolve::{self, ResolveInput};
use wl_invoicing::subscriptions::{PAST_DUE_NOTICE, RETRY_NOTICE};
use wl_invoicing::{Invoice, WlInvoicing};
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    DisputeBy, DisputeRequest, InvoiceId, InvoiceStatus, ReopenReason, Unallocate, invoicer,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::subscriptions::SubscriptionStatus;

use support::subscriptions::{
    GROSS, bill, boot_subscriptions, d, engagement, evaluate, invoices, now, start,
    with_overdue_cycle,
};
use support::{BLUE_DUNES, Booted};

/// Flags `invoice` as disputed by a receivables clerk.
async fn disputed(t: &Booted, invoice: &Invoice) {
    let clerk = t.clerk("Dana").await;
    dispute::raise(
        t.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "the hours are wrong",
            DisputeBy::User { id: clerk.id },
        )
        .amount(invoice.balance),
    )
    .await
    .expect("the dispute is raised");
    assert!(t.reload(invoice).await.disputed);
}

/// Done when 1: an `active` subscription whose overdue cycle invoice is
/// disputed stays `active`, and the party is not told.
#[renox::test]
async fn a_disputed_overdue_cycle_invoice_leaves_it_active() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = with_overdue_cycle(&t, &engagement(1), t.day(-40)).await;
    disputed(&t, &invoice).await;
    evaluate(&t, t.day(0)).await;
    evaluate(&t, t.day(10)).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Active);
    assert!(t.sub_past_due.all().is_empty());
    assert!(t.notices.of_type(PAST_DUE_NOTICE).is_empty());
}

/// Done when 2: a paid cycle invoice reopened by a chargeback makes an
/// `active` subscription `past_due` at once, though the invoice is not due
/// yet, and the party is told.
#[renox::test]
async fn a_chargeback_on_a_paid_cycle_makes_it_past_due() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let sub = start(&t, &engagement(2), t.day(0)).await;
    assert_eq!(bill(&t, t.day(0)).await, 1);
    let invoice = invoices(&t, &sub).await.remove(0);
    t.pay(&invoice, 1, GROSS).await;
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Active);

    let reopened = invoicer(t.state())
        .unallocate(
            t.state(),
            Unallocate::new(
                InvoiceId(invoice.id),
                Subject::new("payments.payment", 1),
                GROSS,
                "payments.allocation:1",
                ReopenReason::PaymentReversed,
            )
            .reversal_reason("chargeback"),
        )
        .await
        .expect("the payment is reversed");
    assert_eq!(reopened.new_status, InvoiceStatus::Issued, "not due yet");

    let sub = now(&t, &sub).await;
    assert_eq!(sub.status, SubscriptionStatus::PastDue);
    let events = t.sub_past_due.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].subscription.0, sub.id);
    let told = t.notices.of_type(PAST_DUE_NOTICE);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients, [NotifyRecipient::Party { id: BLUE_DUNES }]);

    // Paid again: settled.
    t.pay(&invoice, 2, GROSS).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Active);
    assert_eq!(t.sub_settled.all().len(), 1);
}

/// Done when 3: `past_due` with only disputed overdue cycle invoices is not
/// suspended by `FIN.RUL.06` (and nobody is chased); the same run suspends a
/// subscription whose overdue invoice is not disputed.
#[renox::test]
async fn fin_rul_06_does_not_suspend_on_disputed_invoices_only() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (held, invoice) = with_overdue_cycle(&t, &engagement(3), t.day(-40)).await;
    let (other, _) = with_overdue_cycle(&t, &engagement(4), t.day(-40)).await;
    evaluate(&t, t.day(0)).await;
    assert_eq!(now(&t, &held).await.status, SubscriptionStatus::PastDue);
    assert_eq!(now(&t, &other).await.status, SubscriptionStatus::PastDue);

    // Already past due, then disputed: it stays past due.
    disputed(&t, &invoice).await;
    for day in [1, 3, 7, 10] {
        evaluate(&t, t.day(day)).await;
    }
    let held = now(&t, &held).await;
    assert_eq!(held.status, SubscriptionStatus::PastDue);
    assert_eq!(held.retries_done, 0, "no retry chases a disputed invoice");
    assert_eq!(now(&t, &other).await.status, SubscriptionStatus::Suspended);
    let suspended = t.sub_suspended.all();
    assert_eq!(suspended.len(), 1);
    assert_eq!(suspended[0].subscription.0, other.id);
    assert_eq!(t.notices.of_type(RETRY_NOTICE).len(), 3, "the other one only");
}

/// Done when 4: a dispute on the overdue cycle invoice resolved `upheld` on
/// 2027-03-01 (grace 7 days, to 2027-03-08) leaves it `active` through
/// 2027-03-07; the first evaluation on 2027-03-08 makes it `past_due`.
#[renox::test]
async fn after_an_upheld_dispute_it_goes_past_due_when_the_grace_ends() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = with_overdue_cycle(&t, &engagement(5), t.day(-40)).await;
    disputed(&t, &invoice).await;
    let clerk = t.clerk("Dana").await;
    // 06:00 UTC is 10:00 in Dubai on 1 March.
    t.travel_to("2027-03-01", 6, 0);
    t.app
        .at_travelled_time(dispute_resolve::resolve(
            t.state(),
            invoice.id,
            ResolveInput::upheld("the hours were right"),
            DisputeBy::User { id: clerk.id },
        ))
        .await
        .expect("the dispute is upheld");
    t.app.travel_back();
    let invoice = t.reload(&invoice).await;
    assert!(!invoice.disputed);
    assert_eq!(invoice.dispute_grace_until, Some(d("2027-03-08")));

    for day in ["2027-03-01", "2027-03-04", "2027-03-07"] {
        evaluate(&t, d(day)).await;
        assert_eq!(
            now(&t, &sub).await.status,
            SubscriptionStatus::Active,
            "{day}"
        );
    }
    evaluate(&t, d("2027-03-08")).await;
    let sub = now(&t, &sub).await;
    assert_eq!(sub.status, SubscriptionStatus::PastDue);
    assert_eq!(sub.past_due_on, Some(d("2027-03-08")));
    assert_eq!(t.sub_past_due.all().len(), 1);
}
