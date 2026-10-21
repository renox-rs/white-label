//! Expiring points per lot with a reminder, and the points liability report
//! (BL-SAL-24, #143, `SAL.RUL.08`; also the "lot expiring while held" check of
//! #142): the daily run reminds a member once, expires the rest of a lot after
//! its last day and never twice, leaves what an open sale holds, and the
//! account shows a reminder that gave up. Every check runs the real command
//! `sales:points_expiry` (or its function) against the real tables.

mod support;

use std::time::Duration;

use renox::chrono::{Datelike, Duration as Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::notify::{
    ChannelTried, GiveUpReaction, MessageGaveUp, MessageId, NotifyChannel, NotifyRecipient,
};
use wl_sales::expiry::{self, TASK};
use wl_sales::liability;
use wl_sales::notices::LOYALTY_EXPIRY;
use wl_sales::{LoyaltyAccount, PointsKind, PointsMovement, Redemption, RedemptionStatus};

use support::floor::{Floor, account, movements};
use support::shop::Shop;
use support::{Booted, boot_shop};

/// The party of the member M in every test.
const M: i64 = 7;

async fn world(points: i64, extra: &[(&str, &str)]) -> (Booted, Shop, Floor, LoyaltyAccount) {
    let (t, shop) = boot_shop().await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, extra).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    if points > 0 {
        floor.earn(&t, &m, points).await;
    }
    let m = account(&t, &m).await;
    (t, shop, floor, m)
}

/// Runs the real command for the day `on`.
async fn run(t: &Booted, on: NaiveDate) {
    t.app
        .kernel()
        .call(TASK, vec!["--date".to_owned(), on.to_string()])
        .await
        .expect("the expiry task runs");
}

/// Sets the last day of the lot `lot`.
async fn end_on(t: &Booted, lot: &PointsMovement, last: NaiveDate) {
    let last = Some(last);
    PointsMovement::query()
        .where_(PointsMovement::ID.eq(lot.id))
        .update(t.db(), &[("lot_expires_on", &last)])
        .await
        .unwrap();
}

#[renox::test]
async fn a_lot_of_300_with_100_used_gets_one_reminder_and_200_expire_the_morning_after_its_last_day()
 {
    let (t, shop, floor, m) = world(300, &[("min_redeem_points", "100")]).await;
    let lot = movements(&t, &m, PointsKind::Earn).await.remove(0);
    // The lot ends 40 days from now (the plan's 30 Nov): the reminder is 30
    // days before, the expiry the morning after.
    let last = today() + Days::days(40);
    end_on(&t, &lot, last).await;

    // 100 of the 300 are used by a redemption on a sale.
    let sale = floor
        .open_cart(&t, &[("tee", "5.00", "1")], Some(&m.code))
        .await;
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", sale.id),
            &[("points", "100")],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/redeem/confirm", sale.id),
            &[("code", &shop.mail.code_for(&sale.subject()))],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "4.25")],
        )
        .await
        .assert_status(303);
    let used = PointsMovement::find_or_404(t.db(), lot.id).await.unwrap();
    assert_eq!(used.lot_remaining(), 200);
    // The sale earned 4 points on what was left, in a lot ending a year on.
    assert_eq!(account(&t, &m).await.points_balance, 204);

    // 31 days before the last day: not yet.
    run(&t, last - Days::days(31)).await;
    assert!(shop.mail.of_type(LOYALTY_EXPIRY).is_empty());
    // 30 days before: one reminder, for the 200 points that are left.
    run(&t, last - Days::days(30)).await;
    let told = shop.mail.of_type(LOYALTY_EXPIRY);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].recipients,
        vec![NotifyRecipient::Party { id: PartyId(M) }]
    );
    assert_eq!(told[0].data["points"], 200);
    assert_eq!(told[0].data["value"], "2.00");
    assert_eq!(
        told[0].data["expires_on"],
        last.format("%-d %b %Y").to_string()
    );
    // Another run, a day later, does not remind again.
    run(&t, last - Days::days(29)).await;
    assert_eq!(shop.mail.of_type(LOYALTY_EXPIRY).len(), 1);

    // On the last day the lot can still be redeemed: nothing expires.
    run(&t, last).await;
    assert!(movements(&t, &m, PointsKind::Expire).await.is_empty());
    // The morning after, the rest expires with `PointsExpired`.
    run(&t, last + Days::days(1)).await;
    let gone = movements(&t, &m, PointsKind::Expire).await;
    assert_eq!(gone.len(), 1);
    assert_eq!((gone[0].points, gone[0].value), (-200, 200));
    assert_eq!(gone[0].lot_id, Some(lot.id));
    assert_eq!(gone[0].cause, "expire");
    assert_eq!(account(&t, &m).await.points_balance, 4);
    let events = shop.points.expired.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (
            events[0].party,
            events[0].points,
            events[0].value,
            events[0].lot_expired_on
        ),
        (PartyId(M), 200, 200, last)
    );

    // Running the job again expires nothing more.
    run(&t, last + Days::days(1)).await;
    run(&t, last + Days::days(2)).await;
    assert_eq!(movements(&t, &m, PointsKind::Expire).await.len(), 1);
    assert_eq!(shop.points.expired.lock().unwrap().len(), 1);
    assert_eq!(account(&t, &m).await.points_balance, 4);
}

#[renox::test]
async fn a_reminder_that_gave_up_does_not_stop_the_lot_expiring_and_the_account_shows_it() {
    let (t, shop, floor, m) = world(300, &[]).await;
    let lot = movements(&t, &m, PointsKind::Earn).await.remove(0);
    let last = today() + Days::days(20);
    end_on(&t, &lot, last).await;
    run(&t, today()).await;
    assert_eq!(shop.mail.of_type(LOYALTY_EXPIRY).len(), 1);
    assert!(account(&t, &m).await.expiry_reminder_failed_at.is_none());
    t.app
        .get("/loyalty")
        .await
        .assert_ok()
        .assert_dont_see("Reminder not delivered");

    // The message gives up (wl-notify emits this when every channel failed).
    let gave_up = MessageGaveUp {
        message: MessageId(1),
        type_key: LOYALTY_EXPIRY.to_owned(),
        subject: m.subject(),
        subject_label: Some(m.code.clone()),
        recipient: NotifyRecipient::Party { id: PartyId(M) },
        gave_up_at: renox::db::now(),
        contact_point: None,
        channels_tried: vec![ChannelTried {
            message: MessageId(1),
            channel: NotifyChannel::Email,
            last_error_code: Some("550".to_owned()),
            last_error: Some("mailbox unknown".to_owned()),
        }],
        statutory: false,
        reactions: vec![GiveUpReaction::None],
        alternative_of: None,
        contact_of: None,
    };
    t.state().emit(gave_up).await.unwrap();
    assert!(account(&t, &m).await.expiry_reminder_failed_at.is_some());
    t.app.acting_as(&floor.cora);
    t.app
        .get("/loyalty")
        .await
        .assert_ok()
        .assert_see("Reminder not delivered");

    // The lot expires on its date all the same.
    run(&t, last + Days::days(1)).await;
    let gone = movements(&t, &m, PointsKind::Expire).await;
    assert_eq!(gone.iter().map(|x| x.points).sum::<i64>(), -300);
    assert_eq!(account(&t, &m).await.points_balance, 0);
}

#[renox::test]
async fn a_lot_ending_at_midnight_is_not_expired_while_a_sale_holds_its_points() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let lot = movements(&t, &m, PointsKind::Earn).await.remove(0);
    // The lot's last day is today; it can still be redeemed.
    end_on(&t, &lot, today()).await;
    let sale = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", sale.id),
            &[("points", "800")],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/redeem/confirm", sale.id),
            &[("code", &shop.mail.code_for(&sale.subject()))],
        )
        .await
        .assert_status(303);
    assert_eq!(account(&t, &m).await.points_held, 800);

    // The run of the next morning finds the lot past its day, but held.
    let tomorrow = today() + Days::days(1);
    let report = expiry::run_daily(t.state(), tomorrow, renox::db::now())
        .await
        .unwrap();
    assert_eq!((report.expired_lots, report.expired_points), (0, 0));
    assert!(movements(&t, &m, PointsKind::Expire).await.is_empty());
    assert_eq!(account(&t, &m).await.points_balance, 800);

    // The sale is abandoned: the hold ends, the next run expires the points.
    t.app.travel(Duration::from_secs(31 * 60));
    let later = t.app.at_travelled_time(async { renox::db::now() }).await;
    let report = expiry::run_daily(t.state(), tomorrow, later).await.unwrap();
    assert_eq!(report.released, 1);
    assert_eq!((report.expired_lots, report.expired_points), (1, 800));
    let held = &Redemption::of_sale(t.db(), &sale.subject()).await.unwrap()[0];
    assert_eq!(held.status, RedemptionStatus::Released);
    let m = account(&t, &m).await;
    assert_eq!((m.points_balance, m.points_held), (0, 0));
}

#[renox::test]
async fn a_negative_balance_has_nothing_to_expire() {
    let (t, shop, _floor, m) = world(100, &[]).await;
    let lot = movements(&t, &m, PointsKind::Earn).await.remove(0);
    end_on(&t, &lot, today() - Days::days(2)).await;
    // The member's balance went below zero through reversals elsewhere.
    LoyaltyAccount::query()
        .where_(LoyaltyAccount::ID.eq(m.id))
        .update(t.db(), &[("points_balance", &-5_i64)])
        .await
        .unwrap();
    run(&t, today()).await;
    assert!(movements(&t, &m, PointsKind::Expire).await.is_empty());
    assert_eq!(account(&t, &m).await.points_balance, -5);
    assert!(shop.points.expired.lock().unwrap().is_empty());
}

/// Gives the account `account` a lot of `points` ending on `last`, as a
/// fixture for the report.
async fn lot_of(
    t: &Booted,
    account: &LoyaltyAccount,
    points: i64,
    last: NaiveDate,
) -> PointsMovement {
    PointsMovement::create(
        t.db(),
        PointsMovement {
            account_id: account.id,
            kind: PointsKind::Earn,
            points,
            value: points,
            source_type: "sales.counter_sale".to_owned(),
            source_id: 99,
            cause: "earn".to_owned(),
            lot_expires_on: Some(last),
            ..PointsMovement::default()
        },
    )
    .await
    .unwrap()
}

#[renox::test]
async fn the_liability_report_values_120000_points_at_aed_1200_and_counts_negative_balances_as_zero()
 {
    let (t, _shop, floor, m) = world(0, &[]).await;
    let year = today().year();
    let as_at = NaiveDate::from_ymd_opt(year, 12, 31).unwrap();
    let ends = NaiveDate::from_ymd_opt(year + 1, 6, 30).unwrap();
    lot_of(&t, &m, 120_000, ends).await;
    // Another member whose balance is below zero counts as zero.
    let other = floor.enrol(&t, 8, "0509999999").await;
    let small = lot_of(&t, &other, 100, ends).await;
    PointsMovement::create(
        t.db(),
        PointsMovement {
            account_id: other.id,
            kind: PointsKind::ReverseEarn,
            points: -150,
            source_type: "sales.sales_return".to_owned(),
            source_id: 5,
            cause: "return".to_owned(),
            lot_id: Some(small.id),
            ..PointsMovement::default()
        },
    )
    .await
    .unwrap();
    // A lot already past its last day on that date is not outstanding.
    lot_of(&t, &m, 7, NaiveDate::from_ymd_opt(year, 12, 30).unwrap()).await;

    let report = liability::report(t.db(), as_at).await.unwrap();
    assert_eq!((report.points, report.value), (120_000, 120_000));
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].expiry_month, format!("{}-06", year + 1));
    // The lot was created today, so its age is the days to the date.
    assert_eq!(
        report.rows[0].age,
        liability::age_bucket((as_at - today()).num_days())
    );
    assert_eq!(report.rows[0].brand, "");

    // The accountant's page shows AED 1,200.00.
    t.app.acting_as(&floor.cora);
    t.app
        .get(&format!("/loyalty/liability?as_at={as_at}"))
        .await
        .assert_ok()
        .assert_see("AED 1,200.00")
        .assert_see("120000");
    t.app
        .htmx()
        .get("/loyalty/liability?as_at=31-12")
        .await
        .assert_status(422);
}
