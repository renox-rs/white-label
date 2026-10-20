//! BL-PAY-17 (#110), `FIN.ACL.05`, X-07, D-12, 03-N9-a, 05-N11-a: another user
//! counts a holder's cash; a short count is a shortage that tells the holder
//! and the manager, and a third user recounts it; an overage is kept as a
//! liability. The tests act through the real cash pages.

mod support;

use renox::auth::permissions;
use renox::prelude::*;
use wl_kernel::custody::{HandoverStatus, PeriodKind};
use wl_kernel::notify::NotifyRecipient;
use wl_payments::cash_handover::{CountKind, PostingKind};
use wl_payments::cash_notices::CASH_SHORTAGE;

use support::{Booted, boot};

/// The manager role cash shortages are told to by default.
async fn finance_manager(t: &Booted, name: &str) -> User {
    let user = User::register(
        t.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "password123",
    )
    .await
    .unwrap();
    permissions::define_role(t.db(), "finance_manager", &[wl_payments::permissions::CASH_VIEW])
        .await
        .unwrap();
    user.assign_role(t.db(), "finance_manager").await.unwrap();
    user
}

fn notified(recipients: &[NotifyRecipient], user: &User) -> bool {
    recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user.id))
}

/// AC1: receptionist R with AED 2,300.00 expected on 2027-04-01, counter C
/// counts AED 2,250.00: the handover is `short`, `CashShortage` carries AED
/// 50.00, and R and R's manager are notified.
#[renox::test]
async fn a_short_count_is_a_shortage_that_tells_the_holder_and_the_manager() {
    let t = boot().await;
    let manager = finance_manager(&t, "Mona").await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let handover = t.collect(&r, "2027-04-01", 230_000, 1).await;

    t.count_cash(&c, &handover, "2250.00").await.assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!((after.counted, after.difference), (225_000, 5_000));
    assert_eq!(after.counted_by, Some(c.id));
    let events = t.cash_shortage.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].difference, 5_000);
    assert_eq!(events[0].holder_user, r.id);
    assert_eq!(events[0].handover, after.handover_id());
    assert_eq!(events[0].period_kind, PeriodKind::Day);
    assert_eq!(events[0].source, None);
    let counted = t.cash_counted.all();
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0].status, HandoverStatus::Short);
    assert_eq!((counted[0].expected, counted[0].counted), (230_000, 225_000));
    let recipients = t.notices.recipients(CASH_SHORTAGE);
    assert!(notified(&recipients, &r), "the holder is told");
    assert!(notified(&recipients, &manager), "the manager is told");
    let request = &t.notices.of_type(CASH_SHORTAGE)[0];
    assert_eq!(request.data["amount"], "AED 50.00");
    let counts = t.counts(&after).await;
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0].kind, CountKind::Count);
}

/// AC2: R cannot count; C cannot recount; counter D recounts AED 2,300.00 and
/// the handover is `counted`. A recount says whether the holder was present;
/// a user without `cash.count` is refused at the route.
#[renox::test]
async fn the_holder_cannot_count_nor_the_first_counter_recount_and_a_third_user_does() {
    let t = boot().await;
    // The holder holds the permission too: the rule, not the missing
    // permission, refuses them.
    let r = t.counter("Rana").await;
    let c = t.counter("Chandra").await;
    let d = t.counter("Dev").await;
    let plain = t.holder("Pia").await;
    let handover = t.collect(&r, "2027-04-01", 230_000, 1).await;

    t.count_cash(&r, &handover, "2300.00").await.assert_status(422);
    t.count_cash(&plain, &handover, "2300.00").await.assert_forbidden();
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Open);
    t.count_cash(&c, &handover, "2250.00").await.assert_ok();

    t.recount_cash(&c, &handover, "2300.00").await.assert_status(422);
    t.recount_cash(&r, &handover, "2300.00").await.assert_status(422);
    // A recount that does not say whether the holder was there is refused.
    t.cash_post(
        &d,
        &format!("/cash/{}/recount", handover.id),
        &[("counted", "2300.00")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reloaded(&handover).await.status, HandoverStatus::Short);

    t.recount_cash(&d, &handover, "2300.00").await.assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Counted);
    assert_eq!((after.counted, after.difference), (230_000, 0));
    assert_eq!(after.recounted_by, Some(d.id));
    assert_eq!(after.counted_by, Some(c.id), "the first counter stays");
    assert_eq!(after.holder_present, Some(true));
    let counts = t.counts(&after).await;
    assert_eq!(counts.len(), 2);
    assert_eq!(counts[1].kind, CountKind::Recount);
    assert_eq!(t.cash_counted.all().len(), 2);
    assert_eq!(t.cash_counted.all()[1].status, HandoverStatus::Counted);
}

/// A recount that is still short updates the difference and tells nobody
/// twice by mistake: the handover stays `short`, and the count made without
/// the holder is recorded as such.
#[renox::test]
async fn a_recount_that_is_still_short_updates_the_difference() {
    let t = boot().await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let d = t.counter("Dev").await;
    let handover = t.collect(&r, "2027-04-01", 230_000, 1).await;
    t.count_cash(&c, &handover, "2250.00").await.assert_ok();

    t.cash_post(
        &d,
        &format!("/cash/{}/recount", handover.id),
        &[("counted", "2270.00"), ("absent", "on")],
    )
    .await
    .assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!((after.counted, after.difference), (227_000, 3_000));
    assert!(after.holder_absent);
    assert_eq!(after.holder_present, None);
}

/// Counted AED 2,320.00 against AED 2,300.00 expected: the handover is
/// `counted`, with an overage line of AED 20.00 held as a liability, not as
/// income, and no shortage.
#[renox::test]
async fn an_overage_is_counted_and_held_as_a_liability() {
    let t = boot().await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let handover = t.collect(&r, "2027-04-01", 230_000, 1).await;

    t.count_cash(&c, &handover, "2320.00").await.assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Counted);
    assert_eq!((after.difference, after.overage), (0, 2_000));
    let postings = t.postings(&after).await;
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].kind, PostingKind::OverageLiability);
    assert_eq!(postings[0].amount, 2_000);
    assert_eq!(postings[0].location.as_deref(), Some("Dubai branch"));
    assert!(t.cash_shortage.all().is_empty());
    assert!(t.notices.of_type(CASH_SHORTAGE).is_empty());
    assert_eq!(t.cash_counted.all()[0].status, HandoverStatus::Counted);
}

/// An exact count is `counted` with nothing posted; counting AED 0 is a
/// shortage of the whole amount; a handover counted is not counted again.
#[renox::test]
async fn an_exact_count_posts_nothing_and_a_count_of_nothing_is_a_full_shortage() {
    let t = boot().await;
    let r = t.holder("Rana").await;
    let c = t.counter("Chandra").await;
    let exact = t.collect(&r, "2027-04-01", 230_000, 1).await;
    t.count_cash(&c, &exact, "2300.00").await.assert_ok();
    let after = t.reloaded(&exact).await;
    assert_eq!(after.status, HandoverStatus::Counted);
    assert!(t.postings(&after).await.is_empty());
    t.count_cash(&c, &after, "2300.00").await.assert_status(422);

    let nothing = t.collect(&r, "2027-04-02", 90_000, 2).await;
    t.count_cash(&c, &nothing, "0").await.assert_ok();
    let short = t.reloaded(&nothing).await;
    assert_eq!(short.status, HandoverStatus::Short);
    assert_eq!((short.counted, short.difference), (0, 90_000));
    // A typo is asked again, not counted.
    let other = t.collect(&r, "2027-04-03", 90_000, 3).await;
    t.count_cash(&c, &other, "abc").await.assert_status(422);
    assert_eq!(t.reloaded(&other).await.status, HandoverStatus::Open);
}
