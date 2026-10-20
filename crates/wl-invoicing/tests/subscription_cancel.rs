//! Cancelling a subscription when its agreement ends (wl-invoicing §4
//! `cancel`, §7 *pro rata on cancel*; wl-parties `PTY.ACL.03`; #84,
//! BL-INV-21): the five Done-when points. The vertical's cancel goes through
//! the kernel's `Subscriptions`; the user's through the back-office route.
//! The March cycle is AED 3,100.00 (AED 2,952.38 plus 5% VAT); 21 of its 31
//! days unused credit AED 2,100.00 (AED 2,000.00 plus AED 100.00 VAT).

mod support;

use renox::prelude::*;
use wl_invoicing::{Invoice, Subscription, WlInvoicing, permissions as perm};
use wl_kernel::Subject;
use wl_kernel::invoicing::{CreditNoteStatus, InvoiceStatus, VatCategory};
use wl_kernel::subscriptions::{
    CancelSubscription, PauseCause, PauseSubscription, StartSubscription, SubscriptionStatus,
    subscriptions,
};

use support::subscriptions::{
    GROSS, NET, bill, boot_subscriptions, credit_notes, d, enrolment, invoices, now, subscription,
};
use support::{BLUE_DUNES, Booted};

/// The credit of 21 unused days of 31.
const CREDIT: i64 = 210_000;

/// A monthly membership for `agreement` from 1 March 2027 with 30 days'
/// notice, its March cycle billed and paid in full (when `paid`).
async fn march(t: &Booted, agreement: &Subject, paid: bool) -> (Subscription, Invoice) {
    let id = subscriptions(t.state())
        .start(
            t.state(),
            StartSubscription::new(BLUE_DUNES, agreement.clone(), "Membership", NET, d("2027-03-01"))
                .vat_category(VatCategory::Standard)
                .notice_days(30),
        )
        .await
        .unwrap()
        .expect("invoicing is on");
    let sub = subscription(t, id).await;
    assert_eq!(bill(t, d("2027-03-01")).await, 1);
    let invoice = invoices(t, &sub).await.remove(0);
    assert_eq!(invoice.gross_total, GROSS);
    if paid {
        t.pay(&invoice, 1, GROSS).await;
    }
    (sub, t.reload(&invoice).await)
}

/// The vertical withdraws `agreement` on `on` (10 enrolment `withdraw`).
async fn withdraw(t: &Booted, agreement: &Subject, cancel: CancelSubscription) {
    subscriptions(t.state())
        .cancel(t.state(), agreement, cancel)
        .await
        .expect("the cancel runs");
}

/// Done when 1: withdrawn on 2027-03-10, the subscription is `cancelled` at
/// once (its 30 days' notice do not apply), a credit note of AED 2,100.00 is
/// issued, and `SubscriptionCancelled` is emitted.
#[renox::test]
async fn a_withdrawal_cancels_at_once_with_a_pro_rata_credit() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = march(&t, &enrolment(1), true).await;
    withdraw(&t, &enrolment(1), CancelSubscription::new(d("2027-03-10"))).await;

    let sub = now(&t, &sub).await;
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);
    assert_eq!(sub.cancelled_on, Some(d("2027-03-10")));
    assert_eq!(sub.cancel_on, None, "no notice period");
    let notes = credit_notes(&t, &invoice).await;
    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert_eq!(note.gross_total, CREDIT);
    assert_eq!((note.net_total, note.vat_total), (200_000, 10_000));
    assert_ne!(note.status, CreditNoteStatus::Draft);
    assert_eq!(sub.final_credit_note_id, Some(note.id));
    let cancelled = t.sub_cancelled.all();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].subscription.0, sub.id);
    assert_eq!(cancelled[0].source, enrolment(1));
}

/// Done when 2: with the vertical's flag "settled", no credit note.
#[renox::test]
async fn a_settled_withdrawal_credits_nothing() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = march(&t, &enrolment(2), true).await;
    withdraw(
        &t,
        &enrolment(2),
        CancelSubscription::new(d("2027-03-10")).settled(),
    )
    .await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Cancelled);
    assert!(credit_notes(&t, &invoice).await.is_empty());
    assert_eq!(t.sub_cancelled.all().len(), 1);
}

/// Done when 3: for a blocked party the credit note is issued and no refund
/// is paid (`PTY.ACL.03`).
#[renox::test]
async fn a_blocked_party_gets_the_credit_note_and_no_refund() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = march(&t, &enrolment(3), true).await;
    t.directory.set_blocked(BLUE_DUNES, true);
    withdraw(&t, &enrolment(3), CancelSubscription::new(d("2027-03-10"))).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Cancelled);
    let notes = credit_notes(&t, &invoice).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].gross_total, CREDIT);
    assert_eq!(notes[0].remaining, CREDIT, "kept as the party's credit");
    assert!(t.refunds.asked().is_empty(), "no refund is paid");
}

/// The edge case of an unpaid cycle: the credit note reduces the cycle
/// invoice (the unused part is not owed) and nothing is refunded.
#[renox::test]
async fn an_unpaid_cycle_is_reduced_by_the_credit() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (_, invoice) = march(&t, &enrolment(4), false).await;
    withdraw(&t, &enrolment(4), CancelSubscription::new(d("2027-03-10"))).await;
    let invoice = t.reload(&invoice).await;
    assert_eq!(invoice.balance, GROSS - CREDIT);
    assert_eq!(invoice.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(credit_notes(&t, &invoice).await[0].remaining, 0);
    assert!(t.refunds.asked().is_empty());
}

/// Done when 4: the March cycle paid, paused from 2027-03-11 and withdrawn
/// on 2027-04-15, a credit note of AED 2,100.00 (11 to 31 March) is issued.
#[renox::test]
async fn a_withdrawal_while_paused_credits_from_the_pause_start() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = march(&t, &enrolment(5), true).await;
    subscriptions(t.state())
        .pause(
            t.state(),
            &enrolment(5),
            PauseSubscription::new(PauseCause::Agreement, d("2027-03-11")),
        )
        .await
        .unwrap();
    assert_eq!(bill(&t, d("2027-04-15")).await, 0, "April is not billed");
    withdraw(&t, &enrolment(5), CancelSubscription::new(d("2027-04-15"))).await;
    let sub = now(&t, &sub).await;
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);
    let notes = credit_notes(&t, &invoice).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].gross_total, CREDIT);
    assert_eq!(invoices(&t, &sub).await.len(), 1, "no final invoice");
}

/// `POST /subscriptions/{id}/cancel` as a holder of `subscriptions.manage`
/// on 2027-03-10.
async fn user_cancel(t: &Booted, sub: &Subscription) {
    let user = t
        .helper(
            "Mona",
            &[perm::SUBSCRIPTIONS_VIEW, perm::SUBSCRIPTIONS_MANAGE],
        )
        .await;
    t.post_as(
        &format!("/subscriptions/{}/cancel", sub.id),
        &user,
        &[("on", "2027-03-10"), ("reason", "moving abroad")],
    )
    .await
    .assert_redirect(&format!("/subscriptions/{}", sub.id));
}

/// Done when 5, default: a user cancel with *pro rata on cancel* off makes no
/// credit note and no final invoice.
#[renox::test]
async fn a_user_cancel_is_not_prorated_by_default() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = march(&t, &enrolment(6), true).await;
    // No notice for this one: the user cancel ends it at once.
    Subscription::query()
        .where_(Subscription::ID.eq(sub.id))
        .update(t.db(), &[("notice_days", &0_i64)])
        .await
        .unwrap();
    user_cancel(&t, &sub).await;
    let sub = now(&t, &sub).await;
    assert_eq!(sub.status, SubscriptionStatus::Cancelled);
    assert_eq!(sub.cancel_reason.as_deref(), Some("moving abroad"));
    assert!(credit_notes(&t, &invoice).await.is_empty());
    assert_eq!(invoices(&t, &sub).await.len(), 1, "no final invoice");
    assert_eq!(sub.final_invoice_id, None);
}

/// Done when 5, on: with *pro rata on cancel* on and the same dates,
/// AED 2,100.00 is credited.
#[renox::test]
async fn a_user_cancel_is_prorated_when_the_setting_is_on() {
    let t = boot_subscriptions(WlInvoicing::new().subscription_pro_rata_on_cancel(true)).await;
    let (sub, invoice) = march(&t, &enrolment(7), true).await;
    Subscription::query()
        .where_(Subscription::ID.eq(sub.id))
        .update(t.db(), &[("notice_days", &0_i64)])
        .await
        .unwrap();
    user_cancel(&t, &sub).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Cancelled);
    let notes = credit_notes(&t, &invoice).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].gross_total, CREDIT);
}

/// A user cancel with notice is scheduled, and carried out (without
/// proration) when the notice ends.
#[renox::test]
async fn a_user_cancel_with_notice_ends_when_the_notice_does() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = march(&t, &enrolment(8), true).await;
    user_cancel(&t, &sub).await;
    let scheduled = now(&t, &sub).await;
    assert_eq!(scheduled.status, SubscriptionStatus::Active);
    assert_eq!(scheduled.cancel_on, Some(d("2027-04-09")));
    support::subscriptions::evaluate(&t, d("2027-04-08")).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Active);
    support::subscriptions::evaluate(&t, d("2027-04-09")).await;
    let ended = now(&t, &sub).await;
    assert_eq!(ended.status, SubscriptionStatus::Cancelled);
    assert_eq!(ended.cancelled_on, Some(d("2027-04-09")));
    assert_eq!(ended.cancel_reason.as_deref(), Some("moving abroad"));
    assert!(credit_notes(&t, &invoice).await.is_empty());
}
