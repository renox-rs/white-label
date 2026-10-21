//! Changing and ending the loyalty programme (BL-SAL-26, #145): a change
//! needs 30 days' notice and members are told when it is saved; a sale before
//! its date earns on the old terms and one from its date on the new ones,
//! while the lots already earned keep their value and expiry. Turning the
//! programme off stops earning at once, sets the closing date 90 days ahead,
//! lets members redeem through it, and expires every open lot in the run after
//! it.

mod support;

use std::time::Duration as StdDuration;

use renox::chrono::Duration;
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_sales::expiry::TASK;
use wl_sales::notices::{LOYALTY_PROGRAMME_OFF, LOYALTY_TERMS_CHANGED};
use wl_sales::{LoyaltyProgramme, LoyaltyTermsChange, PointsKind, Redemption};

use support::floor::{Floor, account, movements};
use support::shop::Shop;
use support::{Booted, boot_shop};

async fn world(extra: &[(&str, &str)]) -> (Booted, Shop, Floor) {
    let (t, shop) = boot_shop().await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, extra).await;
    (t, shop, floor)
}

#[renox::test]
async fn a_change_with_less_than_30_days_notice_is_refused_and_one_with_30_tells_the_members() {
    let (t, shop, floor) = world(&[]).await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.enrol(&t, 8, "0508888888").await;
    floor.earn(&t, &m, 100).await;
    let first_lot = movements(&t, &m, PointsKind::Earn).await.remove(0);

    t.app.acting_as(&floor.mia);
    let soon = (today() + Duration::days(29)).to_string();
    t.app
        .htmx()
        .post(
            "/loyalty/programme/changes",
            &[("effective_from", soon.as_str()), ("earn_rate", "2")],
        )
        .await
        .assert_status(422);
    assert_eq!(LoyaltyTermsChange::query().count(t.db()).await.unwrap(), 0);
    assert!(shop.mail.of_type(LOYALTY_TERMS_CHANGED).is_empty());
    t.app
        .htmx()
        .post(
            "/loyalty/programme/changes",
            &[("effective_from", soon.as_str())],
        )
        .await
        .assert_status(422);

    let on_time = (today() + Duration::days(30)).to_string();
    t.app
        .post(
            "/loyalty/programme/changes",
            &[("effective_from", on_time.as_str()), ("earn_rate", "2")],
        )
        .await
        .assert_status(303);
    let saved = LoyaltyTermsChange::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.earn_rate, Some(2));
    assert_eq!(saved.created_by, Some(floor.mia.id));
    assert_eq!(saved.notified_members, 2);
    // Both members are told, on save.
    assert_eq!(shop.mail.of_type(LOYALTY_TERMS_CHANGED).len(), 2);

    // A sale before the date still earns at the old rate (1 point per AED).
    floor.earn(&t, &m, 50).await;
    assert_eq!(account(&t, &m).await.points_balance, 150);
    // The lot earned before keeps its points and its last day.
    let kept = movements(&t, &m, PointsKind::Earn).await.remove(0);
    assert_eq!(
        (kept.points, kept.lot_expires_on),
        (first_lot.points, first_lot.lot_expires_on)
    );
}

#[renox::test]
async fn a_sale_the_day_before_the_effective_date_earns_the_old_rate_and_one_on_it_the_new_rate() {
    let (t, _shop, floor) = world(&[]).await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.earn(&t, &m, 100).await;
    let first_lot = movements(&t, &m, PointsKind::Earn).await.remove(0);

    // Two changes saved through the page: 3 points per AED in 30 days and
    // 9 in 60 days (the notice is counted from the day they are saved).
    t.app.acting_as(&floor.mia);
    for (days, rate) in [(30, "3"), (60, "9")] {
        let from = (today() + Duration::days(days)).to_string();
        t.app
            .post(
                "/loyalty/programme/changes",
                &[("effective_from", from.as_str()), ("earn_rate", rate)],
            )
            .await
            .assert_status(303);
    }
    assert_eq!(LoyaltyTermsChange::query().count(t.db()).await.unwrap(), 2);

    // The day before the first date (29 days on): still one point per AED.
    t.app.travel(StdDuration::from_secs(29 * 24 * 60 * 60));
    floor.earn(&t, &m, 10).await;
    assert_eq!(account(&t, &m).await.points_balance, 110);

    // On the date: 3 points per AED, not the 9 of the change still ahead.
    t.app.travel(StdDuration::from_secs(24 * 60 * 60));
    floor.earn(&t, &m, 10).await;
    assert_eq!(account(&t, &m).await.points_balance, 140);
    let lots = movements(&t, &m, PointsKind::Earn).await;
    assert_eq!(lots[0].points, first_lot.points);
    assert_eq!(lots[0].lot_expires_on, first_lot.lot_expires_on);
    assert_eq!((lots[1].points, lots[2].points), (10, 30));
}

#[renox::test]
async fn turning_the_programme_off_sets_the_closing_date_and_expires_every_lot_after_it() {
    let (t, shop, floor) = world(&[]).await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.earn(&t, &m, 600).await;

    t.app.acting_as(&floor.mia);
    t.app
        .htmx()
        .post("/loyalty/programme/off", &[("x", "")])
        .await
        .assert_status(422);
    assert!(
        LoyaltyProgramme::first_active(t.db())
            .await
            .unwrap()
            .unwrap()
            .closing_date
            .is_none()
    );
    t.app
        .post("/loyalty/programme/off", &[("confirm", "1")])
        .await
        .assert_status(303);
    let closing = today() + Duration::days(90);
    let programme = LoyaltyProgramme::first_active(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(programme.closing_date, Some(closing));
    assert_eq!(shop.mail.of_type(LOYALTY_PROGRAMME_OFF).len(), 1);
    // It cannot be turned off twice.
    t.app
        .htmx()
        .post("/loyalty/programme/off", &[("confirm", "1")])
        .await
        .assert_status(422);

    // Earning stopped at once; new members are not enrolled.
    floor.earn(&t, &m, 100).await;
    assert_eq!(account(&t, &m).await.points_balance, 600);
    t.app.acting_as(&floor.cora);
    t.app
        .htmx()
        .post(
            "/loyalty/accounts",
            &[("party_id", "9"), ("mobile", "0509999999"), ("terms", "1")],
        )
        .await
        .assert_status(422);

    // Redeeming still works on the closing day itself: the clock moves
    // there and the redemption is asked, confirmed and the sale completed.
    t.app.travel(StdDuration::from_secs(90 * 24 * 60 * 60));
    assert_eq!(t.app.at_travelled_time(async { today() }).await, closing);
    t.app.acting_as(&floor.cora);
    let cart = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", cart.id),
            &[("points", "500")],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/redeem/confirm", cart.id),
            &[("code", &shop.mail.code_for(&cart.subject()))],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/complete", cart.id),
            &[("cash", "16.00")],
        )
        .await
        .assert_status(303);
    assert_eq!(account(&t, &m).await.points_balance, 100);
    // The job of the closing day itself expires nothing.
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 100);
    assert!(shop.points.expired.lock().unwrap().is_empty());

    // A cart left open over midnight: the day after the closing day
    // redeeming on it is refused (the member could not be attached then).
    let late = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    t.app.travel(StdDuration::from_secs(24 * 60 * 60));
    t.app.acting_as(&floor.cora);
    t.app
        .htmx()
        .post(
            &format!("/counter-sales/{}/redeem", late.id),
            &[("points", "100")],
        )
        .await
        .assert_status(422);
    assert!(Redemption::of_sale(t.db(), &late.subject())
        .await
        .unwrap()
        .is_empty());

    // The expiry job of 02:00 that morning expires what is left. (The closing
    // day's own run, before, expired nothing.)
    t.app.run_scheduled(TASK).await.unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 0);
    let expired = shop.points.expired.lock().unwrap().clone();
    assert_eq!(expired.iter().map(|e| e.points).sum::<i64>(), 100);
    // The programme is over once its lots are.
    let ended = LoyaltyProgramme::find_or_404(t.db(), programme.id)
        .await
        .unwrap();
    assert!(!ended.active);
}
