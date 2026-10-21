//! Changing and ending the loyalty programme (BL-SAL-26, #145): a change
//! needs 30 days' notice and members are told when it is saved; a sale before
//! its date earns on the old terms and one from its date on the new ones,
//! while the lots already earned keep their value and expiry. Turning the
//! programme off stops earning at once, sets the closing date 90 days ahead,
//! lets members redeem through it, and expires every open lot in the run after
//! it.

mod support;

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_sales::expiry::TASK;
use wl_sales::notices::{LOYALTY_PROGRAMME_OFF, LOYALTY_TERMS_CHANGED};
use wl_sales::{LoyaltyProgramme, LoyaltyTermsChange, PointsKind};

use support::floor::{Floor, account, movements};
use support::shop::Shop;
use support::{Booted, boot_shop};

async fn world(extra: &[(&str, &str)]) -> (Booted, Shop, Floor) {
    let (t, shop) = boot_shop().await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, extra).await;
    (t, shop, floor)
}

/// Runs the real expiry command for the day `on`.
async fn run(t: &Booted, on: NaiveDate) {
    t.app
        .kernel()
        .call(TASK, vec!["--date".to_owned(), on.to_string()])
        .await
        .expect("the expiry task runs");
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
    let saved = LoyaltyTermsChange::query().first(t.db()).await.unwrap().unwrap();
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
async fn a_sale_from_the_effective_date_earns_on_the_new_rate_and_the_old_lots_stay() {
    let (t, _shop, floor) = world(&[]).await;
    let m = floor.enrol(&t, 7, "0501234567").await;
    floor.earn(&t, &m, 100).await;
    let first_lot = movements(&t, &m, PointsKind::Earn).await.remove(0);

    // One change took effect yesterday, another is still ahead.
    for (from, rate) in [(today() - Duration::days(1), 3), (today() + Duration::days(30), 9)] {
        LoyaltyTermsChange::create(
            t.db(),
            LoyaltyTermsChange {
                programme_id: first_lot_programme(&t).await,
                effective_from: from,
                earn_rate: Some(rate),
                ..LoyaltyTermsChange::default()
            },
        )
        .await
        .unwrap();
    }
    floor.earn(&t, &m, 10).await;
    // 3 points per AED from the first change, not 9 from the one ahead.
    assert_eq!(account(&t, &m).await.points_balance, 130);
    let lots = movements(&t, &m, PointsKind::Earn).await;
    assert_eq!(lots[0].points, first_lot.points);
    assert_eq!(lots[0].lot_expires_on, first_lot.lot_expires_on);
    assert_eq!(lots[1].points, 30);
}

async fn first_lot_programme(t: &Booted) -> i64 {
    LoyaltyProgramme::first_active(t.db()).await.unwrap().unwrap().id
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
    assert!(LoyaltyProgramme::first_active(t.db()).await.unwrap().unwrap().closing_date.is_none());
    t.app
        .post("/loyalty/programme/off", &[("confirm", "1")])
        .await
        .assert_status(303);
    let closing = today() + Duration::days(90);
    let programme = LoyaltyProgramme::first_active(t.db()).await.unwrap().unwrap();
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

    // Redeeming still works through the closing date.
    let cart = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    t.app
        .post(&format!("/counter-sales/{}/redeem", cart.id), &[("points", "500")])
        .await
        .assert_status(303);

    // The run on the closing day expires nothing; the one the morning after
    // expires every open lot.
    run(&t, closing).await;
    assert_eq!(account(&t, &m).await.points_balance, 600);
    assert!(shop.points.expired.lock().unwrap().is_empty());
    run(&t, closing + Duration::days(1)).await;
    assert_eq!(account(&t, &m).await.points_balance, 0);
    let expired = shop.points.expired.lock().unwrap().clone();
    assert_eq!(expired.iter().map(|e| e.points).sum::<i64>(), 600);
    // The programme is over once its lots are.
    let ended = LoyaltyProgramme::find_or_404(t.db(), programme.id).await.unwrap();
    assert!(!ended.active);
}
