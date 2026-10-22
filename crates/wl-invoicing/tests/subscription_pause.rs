//! Pausing and resuming a subscription with its agreement or a party block
//! (wl-invoicing §4 `pause`, `resume`, the vertical mapping; wl-parties §5.6;
//! #83, BL-INV-20): the six Done-when points. The verticals' calls go
//! through the kernel's `Subscriptions`; the block arrives as the
//! `PartyBlocked` and `PartyUnblocked` events wl-parties emits.

mod support;

use renox::prelude::*;
use wl_invoicing::WlInvoicing;
use wl_invoicing::reminders::{self, AFTER_DUE};
use wl_invoicing::subscriptions::PAUSED_NOTICE;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::parties::{NEUTRAL_BLOCK_REASON, PartyBlocked, PartyUnblocked};
use wl_kernel::subscriptions::{
    PauseCause, PauseSubscription, SUBSCRIPTION_SUBJECT_TYPE, SubscriptionStatus, subscriptions,
};

use support::subscriptions::{
    bill, boot_subscriptions, causes, d, engagement, enrolment, evaluate, invoices, now, start,
    with_overdue_cycle,
};
use support::{BLUE_DUNES, Booted};

/// A client whose user pauses have a yearly allowance (10 membership).
fn membership() -> WlInvoicing {
    WlInvoicing::new().subscription_pause_allowance(60)
}

/// wl-parties blocks Blue Dunes (display reason `court_order`).
async fn block(t: &Booted) {
    t.directory.set_blocked(BLUE_DUNES, true);
    t.state()
        .emit(PartyBlocked {
            party: BLUE_DUNES,
            display_reason: "court_order".into(),
        })
        .await
        .expect("the block is heard");
}

/// wl-parties unblocks Blue Dunes.
async fn unblock(t: &Booted) {
    t.directory.set_blocked(BLUE_DUNES, false);
    t.state()
        .emit(PartyUnblocked {
            party: BLUE_DUNES,
            display_reason: "court_order".into(),
        })
        .await
        .expect("the unblock is heard");
}

/// The vertical pauses `agreement` (10 enrolment `freeze`, 04 engagement
/// `suspend`) on `on`.
async fn freeze(t: &Booted, agreement: &Subject, on: &str) {
    subscriptions(t.state())
        .pause(
            t.state(),
            agreement,
            PauseSubscription::new(PauseCause::Agreement, d(on)),
        )
        .await
        .expect("the pause runs");
}

/// The messages about subscriptions so far.
fn subscription_messages(t: &Booted) -> usize {
    t.notices
        .all()
        .iter()
        .filter(|r| r.subject.subject_type == SUBSCRIPTION_SUBJECT_TYPE)
        .count()
}

/// Done when 1: the enrolment frozen, the membership is `paused`, the
/// allowance used is unchanged and no new cycle invoice is billed.
#[renox::test]
async fn an_agreement_pause_stops_billing_and_spares_the_allowance() {
    let t = boot_subscriptions(membership()).await;
    let sub = start(&t, &enrolment(1), d("2027-01-01")).await;
    assert_eq!(bill(&t, d("2027-01-01")).await, 1);
    freeze(&t, &enrolment(1), "2027-01-15").await;
    let paused = now(&t, &sub).await;
    assert_eq!(paused.status, SubscriptionStatus::Paused);
    assert_eq!(paused.paused_from, Some(d("2027-01-15")));
    assert_eq!(paused.pause_days_used, 0, "a system pause is not counted");
    let open = causes(&t, &sub).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].cause, PauseCause::Agreement);

    assert_eq!(bill(&t, d("2027-03-15")).await, 0);
    assert_eq!(
        invoices(&t, &sub).await.len(),
        1,
        "no cycle billed while paused"
    );
    assert_eq!(t.notices.of_type(PAUSED_NOTICE).len(), 1);
}

/// Done when 2: `PartyBlocked` (effect pause) pauses the 04 retainer with
/// the reason `on_hold` and sends the party nothing.
#[renox::test]
async fn a_block_pauses_with_on_hold_and_no_message() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let sub = start(&t, &engagement(2), d("2027-01-01")).await;
    block(&t).await;
    let paused = now(&t, &sub).await;
    assert_eq!(paused.status, SubscriptionStatus::Paused);
    assert_eq!(paused.pause_reason.as_deref(), Some(NEUTRAL_BLOCK_REASON));
    let open = causes(&t, &sub).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].cause, PauseCause::PartyBlock);
    assert_eq!(open[0].reason, "on_hold");
    assert_eq!(subscription_messages(&t), 0, "nothing is sent to the party");
    // The page shows only the neutral reason.
    let admin = t.admin_a().await;
    t.app.acting_as(&admin);
    t.app
        .get(&format!("/subscriptions/{}", sub.id))
        .await
        .assert_ok()
        .assert_see("on_hold")
        .assert_dont_see("court_order");
}

/// Done when 3: a `past_due` subscription paused keeps the reminders of its
/// overdue cycle invoice.
#[renox::test]
async fn a_paused_past_due_subscription_keeps_its_reminders() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (sub, invoice) = with_overdue_cycle(&t, &engagement(3), t.day(-40)).await;
    evaluate(&t, t.day(0)).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::PastDue);
    freeze(&t, &engagement(3), &t.day(0).to_string()).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Paused);

    reminders::run(t.state(), today())
        .await
        .expect("the reminder run");
    let sent = t.notices.of_type(AFTER_DUE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].subject, t.reload(&invoice).await.subject());
}

/// Done when 4: on `PartyUnblocked` it resumes to `past_due` when a cycle
/// invoice from before the pause is still overdue, else to `active`.
#[renox::test]
async fn an_unblock_resumes_to_past_due_or_active() {
    let t = boot_subscriptions(WlInvoicing::new()).await;
    let (owing, _) = with_overdue_cycle(&t, &engagement(4), t.day(-40)).await;
    evaluate(&t, t.day(0)).await;
    let paid_up = start(&t, &engagement(5), t.day(0)).await;
    block(&t).await;
    assert_eq!(now(&t, &owing).await.status, SubscriptionStatus::Paused);
    assert_eq!(now(&t, &paid_up).await.status, SubscriptionStatus::Paused);

    unblock(&t).await;
    assert_eq!(now(&t, &owing).await.status, SubscriptionStatus::PastDue);
    let paid_up = now(&t, &paid_up).await;
    assert_eq!(paid_up.status, SubscriptionStatus::Active);
    assert_eq!(paid_up.next_cycle_date, today());
    assert!(causes(&t, &paid_up).await.is_empty());
}

/// Done when 5: a user pause until 2027-02-01 with a block and an unblock
/// in January stays paused until 2027-02-01.
#[renox::test]
async fn a_user_pause_outlives_a_block_in_between() {
    let t = boot_subscriptions(membership()).await;
    let sub = start(&t, &enrolment(6), d("2027-01-01")).await;
    subscriptions(t.state())
        .pause(
            t.state(),
            &enrolment(6),
            PauseSubscription::new(PauseCause::User, d("2027-01-05"))
                .reason("travelling")
                .until(d("2027-02-01")),
        )
        .await
        .expect("the user pause runs");
    assert_eq!(now(&t, &sub).await.pause_days_used, 27);

    t.travel_to("2027-01-10", 6, 0);
    t.app.at_travelled_time(block(&t)).await;
    assert_eq!(causes(&t, &sub).await.len(), 2);
    t.travel_to("2027-01-20", 6, 0);
    t.app.at_travelled_time(unblock(&t)).await;
    t.app.travel_back();
    let open = causes(&t, &sub).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].cause, PauseCause::User);
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Paused);

    evaluate(&t, d("2027-01-31")).await;
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Paused);
    evaluate(&t, d("2027-02-01")).await;
    let resumed = now(&t, &sub).await;
    assert_eq!(resumed.status, SubscriptionStatus::Active);
    assert_eq!(resumed.next_cycle_date, d("2027-02-01"));
    assert_eq!(resumed.pause_days_used, 27, "the block was not counted");
}

/// Done when 6: paused by the enrolment freeze, a block runs `pause` again:
/// still `paused`, two causes stored, the allowance unchanged; a second
/// block adds nothing.
#[renox::test]
async fn a_block_on_a_frozen_membership_adds_its_cause() {
    let t = boot_subscriptions(membership()).await;
    let sub = start(&t, &enrolment(7), d("2027-01-01")).await;
    freeze(&t, &enrolment(7), "2027-01-15").await;
    block(&t).await;
    let paused = now(&t, &sub).await;
    assert_eq!(paused.status, SubscriptionStatus::Paused);
    assert_eq!(paused.pause_days_used, 0);
    assert_eq!(
        paused.paused_from,
        Some(d("2027-01-15")),
        "the pause did not restart"
    );
    let kinds: Vec<_> = causes(&t, &sub).await.iter().map(|c| c.cause).collect();
    assert_eq!(kinds, [PauseCause::Agreement, PauseCause::PartyBlock]);
    let history = renox::state_machine::history(t.db(), wl_invoicing::Subscription::TABLE, sub.id)
        .await
        .unwrap();
    let names: Vec<_> = history.iter().map(|h| h.transition.as_str()).collect();
    assert_eq!(names, ["pause_system", "pause_system"]);

    block(&t).await;
    assert_eq!(causes(&t, &sub).await.len(), 2);
    let summary = subscriptions(t.state())
        .of_agreement(t.db(), &enrolment(7))
        .await
        .unwrap();
    assert_eq!(
        summary[0].pause_causes,
        [PauseCause::Agreement, PauseCause::PartyBlock]
    );

    // The freeze lifted while the party is still blocked: it stays paused.
    subscriptions(t.state())
        .resume(
            t.state(),
            &enrolment(7),
            PauseCause::Agreement,
            d("2027-02-10"),
        )
        .await
        .unwrap();
    assert_eq!(now(&t, &sub).await.status, SubscriptionStatus::Paused);
    assert_eq!(causes(&t, &sub).await.len(), 1);
}

/// 10 `EDU.RUL.14` (#440): the vertical hears each pause cause added and
/// removed, with the cause, so it can freeze the enrolment of a membership a
/// user paused and ignore its own pause and a block. A cause removed while
/// another keeps the subscription `paused` is heard too, with `resumed` false.
#[renox::test]
async fn each_pause_cause_added_and_removed_is_told_to_the_vertical() {
    let t = boot_subscriptions(membership()).await;
    let sub = start(&t, &enrolment(8), d("2027-01-01")).await;
    subscriptions(t.state())
        .pause(
            t.state(),
            &enrolment(8),
            PauseSubscription::new(PauseCause::User, d("2027-01-05"))
                .reason("travelling")
                .until(d("2027-01-19")),
        )
        .await
        .expect("the user pause runs");
    freeze(&t, &enrolment(8), "2027-01-06").await;

    let paused = t.sub_paused.all();
    let told: Vec<_> = paused.iter().map(|e| (e.cause, e.first)).collect();
    assert_eq!(
        told,
        [(PauseCause::User, true), (PauseCause::Agreement, false)]
    );
    assert_eq!(paused[0].until, Some(d("2027-01-19")));
    assert_eq!(paused[0].source, enrolment(8));
    assert_eq!(paused[0].subscription.0, sub.id);

    // The user's pause ends while the agreement's keeps it paused.
    subscriptions(t.state())
        .resume(t.state(), &enrolment(8), PauseCause::User, d("2027-01-19"))
        .await
        .unwrap();
    // Then the agreement resumes: nothing is left.
    subscriptions(t.state())
        .resume(
            t.state(),
            &enrolment(8),
            PauseCause::Agreement,
            d("2027-01-20"),
        )
        .await
        .unwrap();
    let resumed = t.sub_resumed.all();
    let told: Vec<_> = resumed.iter().map(|e| (e.cause, e.resumed)).collect();
    assert_eq!(
        told,
        [(PauseCause::User, false), (PauseCause::Agreement, true)]
    );
    assert!(resumed.iter().all(|e| !e.to_past_due));
    // A cause that is not open tells nothing.
    subscriptions(t.state())
        .resume(
            t.state(),
            &enrolment(8),
            PauseCause::Agreement,
            d("2027-01-21"),
        )
        .await
        .unwrap();
    assert_eq!(t.sub_resumed.all().len(), 2);
}
