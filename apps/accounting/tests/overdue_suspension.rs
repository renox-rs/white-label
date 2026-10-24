//! Acceptance checks of #346 (BL-ACC-22, `ACC.RUL.07`): with the rule on, an
//! invoice due 2026-09-01 and unpaid suspends the engagement on 2026-10-31
//! with *non-payment* and tells the partner; the same invoice disputed does
//! not; with the rule off (the default) nothing happens.

mod support;

use accounting::overdue_suspension::{self, OVERDUE_SUSPENDED, overdue_invoice};
use accounting::reasons::NON_PAYMENT;
use accounting::{Engagement, EngagementStatus};
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot, d};

/// An active engagement of a client with invoice `INV-1` of AED 2,000 due
/// 2026-09-01 and unpaid; the partner exists.
async fn overdue_client(t: &Booted) -> (Engagement, User) {
    let partner = t.partner().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    t.invoices
        .overdue(client.company, "INV-1", 200_000, d(9, 1));
    (engagement, partner)
}

/// The partners' notices about `engagement` addressed to `user`.
fn told(t: &Booted, engagement: &Engagement, user: &User) -> usize {
    t.notices
        .of_type(OVERDUE_SUSPENDED)
        .into_iter()
        .filter(|r| r.subject == engagement.subject())
        .filter(|r| {
            r.recipients
                .iter()
                .any(|to| matches!(to, NotifyRecipient::User { id } if *id == user.id))
        })
        .count()
}

/// DW1: with the rule on, the invoice due 2026-09-01 is 59 days overdue on
/// 2026-10-30 (nothing) and 60 on 2026-10-31: the engagement is suspended by
/// the system with *non-payment* and the partner is told, once.
#[renox::test]
async fn bl_acc_22_an_invoice_60_days_overdue_suspends_the_engagement_and_tells_the_partner() {
    let t = boot().await;
    let (engagement, partner) = overdue_client(&t).await;
    overdue_suspension::switch(t.db(), true, None, None)
        .await
        .unwrap();
    assert_eq!(overdue_suspension::setting(t.db()).await.unwrap(), Some(60));

    let early = t
        .run(overdue_suspension::run(t.state(), d(10, 30)))
        .await
        .unwrap();
    assert!(early.is_empty(), "59 days overdue is not yet 60");
    assert_eq!(t.reload(&engagement).await.status, EngagementStatus::Active);

    let suspended = t
        .run(overdue_suspension::run(t.state(), d(10, 31)))
        .await
        .unwrap();
    assert_eq!(suspended, vec![engagement.id]);
    let now = t.reload(&engagement).await;
    assert_eq!(now.status, EngagementStatus::Suspended);
    assert_eq!(now.reason_code.as_deref(), Some(NON_PAYMENT));
    assert_eq!(told(&t, &engagement, &partner), 1);

    // Already suspended: a later run does nothing more.
    let again = t
        .run(overdue_suspension::run(t.state(), d(11, 1)))
        .await
        .unwrap();
    assert!(again.is_empty());
    assert_eq!(told(&t, &engagement, &partner), 1);
}

/// DW2: the same invoice with `disputed` set does not suspend the
/// engagement; once the dispute is resolved *upheld* the count goes on from
/// the original due date, so the next run suspends it.
#[renox::test]
async fn bl_acc_22_a_disputed_invoice_never_suspends_the_engagement() {
    let t = boot().await;
    let (engagement, partner) = overdue_client(&t).await;
    overdue_suspension::switch(t.db(), true, None, None)
        .await
        .unwrap();
    t.invoices.set_disputed("INV-1", true);

    let suspended = t
        .run(overdue_suspension::run(t.state(), d(10, 31)))
        .await
        .unwrap();
    assert!(suspended.is_empty());
    assert_eq!(t.reload(&engagement).await.status, EngagementStatus::Active);
    assert_eq!(told(&t, &engagement, &partner), 0);

    // Resolved upheld on 2026-11-10: the days in dispute are not excluded.
    t.invoices.set_disputed("INV-1", false);
    let suspended = t
        .run(overdue_suspension::run(t.state(), d(11, 11)))
        .await
        .unwrap();
    assert_eq!(suspended, vec![engagement.id]);
    assert_eq!(
        t.reload(&engagement).await.status,
        EngagementStatus::Suspended
    );
}

/// DW3: with the rule off (the default, and after turning it off), nothing
/// happens: no suspension, no notice.
#[renox::test]
async fn bl_acc_22_with_the_rule_off_nothing_happens() {
    let t = boot().await;
    let (engagement, partner) = overdue_client(&t).await;
    assert_eq!(overdue_suspension::setting(t.db()).await.unwrap(), None);

    let suspended = t
        .run(overdue_suspension::run(t.state(), d(10, 31)))
        .await
        .unwrap();
    assert!(suspended.is_empty());

    overdue_suspension::switch(t.db(), true, None, None)
        .await
        .unwrap();
    overdue_suspension::switch(t.db(), false, None, None)
        .await
        .unwrap();
    let suspended = t
        .run(overdue_suspension::run(t.state(), d(11, 30)))
        .await
        .unwrap();
    assert!(suspended.is_empty());
    assert_eq!(t.reload(&engagement).await.status, EngagementStatus::Active);
    assert_eq!(told(&t, &engagement, &partner), 0);
}

/// The firm may set its own N; the invoice chosen is the oldest one that is
/// open, not disputed and due N days back or more.
#[renox::test]
async fn bl_acc_22_the_days_are_the_firms_and_the_oldest_undisputed_invoice_counts() {
    let t = boot().await;
    let (engagement, _partner) = overdue_client(&t).await;
    overdue_suspension::switch(t.db(), true, Some(90), None)
        .await
        .unwrap();
    assert_eq!(overdue_suspension::setting(t.db()).await.unwrap(), Some(90));
    let suspended = t
        .run(overdue_suspension::run(t.state(), d(10, 31)))
        .await
        .unwrap();
    assert!(suspended.is_empty(), "60 days is not 90");

    let invoices = t.invoices.0.lock().unwrap().clone();
    assert_eq!(
        overdue_invoice(&invoices, d(10, 31), 60).and_then(|i| i.number.clone()),
        Some("INV-1".to_owned())
    );
    assert!(overdue_invoice(&invoices, d(10, 31), 61).is_none());
    assert_eq!(t.reload(&engagement).await.status, EngagementStatus::Active);
}
