//! Redeeming loyalty points as a discount line, confirmed by the member
//! (BL-SAL-23, #142, wl-sales §5.9 step 3): the code to the member's mobile,
//! the minimum and the maximum share, three wrong codes, the hold across
//! tills, the VAT of the line, and the points given back by a return, a
//! rejected order and a cancelled line. Every check drives the real pages,
//! transitions and listeners; the one-time code is the fake notifier's (the
//! real `send_otp` is tested in wl-notify and in `redemption_real.rs`).

mod support;

use std::time::Duration;

use renox::chrono::{Duration as Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::notify::{NotifyChannel, NotifyRecipient};
use wl_kernel::sales::{LoyaltyAccountId, QuoteRedemption, discount_engine};
use wl_kernel::{PartyId, Subject};
use wl_sales::notices::LOYALTY_GIVEN_BACK;
use wl_sales::permissions as perm;
use wl_sales::redeem;
use wl_sales::sale_line::KIND_LOYALTY_REDEMPTION;
use wl_sales::sales_orders;
use wl_sales::{
    CounterSale, LoyaltyAccount, PointsKind, PointsMovement, Redemption, RedemptionStatus,
    ReturnStatus, SaleLine, SalesOrder, SalesOrderStatus, SalesReturn, WlSales,
};

use support::floor::{Floor, account, movements};
use support::shop::Shop;
use support::{Booted, boot_shop_with};

/// The party of the member M in every test.
const M: i64 = 7;

/// `fils` as `105.00`.
fn aed(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

/// A programme, M enrolled and, with `points` above zero, M holding that many
/// points from one earning sale.
async fn world_with(
    sales: WlSales,
    points: i64,
    extra: &[(&str, &str)],
) -> (Booted, Shop, Floor, LoyaltyAccount) {
    let (t, shop) = boot_shop_with(sales).await;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, extra).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    if points > 0 {
        let price = format!("{points}.00");
        let paid = aed(points * 105);
        floor
            .ring_up(
                &t,
                &[("gift", price.as_str(), "1")],
                Some(&m.code),
                &[("cash", paid.as_str())],
            )
            .await;
    }
    let m = account(&t, &m).await;
    assert_eq!(m.points_balance, points);
    (t, shop, floor, m)
}

async fn world(points: i64, extra: &[(&str, &str)]) -> (Booted, Shop, Floor, LoyaltyAccount) {
    world_with(WlSales::new(), points, extra).await
}

/// An open cart of two lines of AED 19.05: AED 40.00 with VAT (VAT 1.90).
async fn forty(t: &Booted, floor: &Floor, m: &LoyaltyAccount) -> CounterSale {
    let sale = floor
        .open_cart(
            t,
            &[("tee", "19.05", "1"), ("cap", "19.05", "1")],
            Some(&m.code),
        )
        .await;
    assert_eq!(
        (sale.net_total, sale.vat_total, sale.gross_total),
        (3810, 190, 4000)
    );
    sale
}

async fn reload(t: &Booted, sale: &CounterSale) -> CounterSale {
    CounterSale::find_or_404(t.db(), sale.id).await.unwrap()
}

/// Asks for `points` on the cart through the page.
async fn ask(t: &Booted, sale: &CounterSale, points: i64, status: u16) {
    t.app
        .post(
            &format!("/counter-sales/{}/redeem", sale.id),
            &[("points", &points.to_string())],
        )
        .await
        .assert_status(status);
}

/// Enters `code` for the cart's redemption through the page.
async fn enter(t: &Booted, sale: &CounterSale, code: &str, status: u16) {
    t.app
        .post(
            &format!("/counter-sales/{}/redeem/confirm", sale.id),
            &[("code", code)],
        )
        .await
        .assert_status(status);
}

/// Enters the code the fake notifier sent for the cart.
async fn enter_right(t: &Booted, shop: &Shop, sale: &CounterSale) {
    enter(t, sale, &shop.mail.code_for(&sale.subject()), 303).await;
}

async fn complete(t: &Booted, sale: &CounterSale, cash: &str, status: u16) {
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", cash)],
        )
        .await
        .assert_status(status);
}

async fn redemptions(t: &Booted, sale: &Subject) -> Vec<Redemption> {
    Redemption::of_sale(t.db(), sale).await.unwrap()
}

async fn lines(t: &Booted, sale: &Subject) -> Vec<SaleLine> {
    SaleLine::of(t.db(), sale).await.unwrap()
}

fn redemption_lines(all: &[SaleLine]) -> Vec<&SaleLine> {
    all.iter()
        .filter(|l| l.kind == KIND_LOYALTY_REDEMPTION)
        .collect()
}

#[renox::test]
async fn m_with_800_points_redeems_them_as_an_aed_8_line_after_entering_the_code() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let sale = forty(&t, &floor, &m).await;

    ask(&t, &sale, 800, 303).await;
    // The code went to M's mobile for this sale; nothing is held and no line
    // is on the cart yet.
    let sent = shop.mail.otp_requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].purpose, "loyalty_redemption");
    assert_eq!(sent[0].subject, sale.subject());
    assert_eq!(
        sent[0].to,
        NotifyRecipient::Address {
            channel: NotifyChannel::Sms,
            address: "+971501234567".to_owned()
        }
    );
    let pending = redemptions(&t, &sale.subject()).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, RedemptionStatus::Pending);
    assert_eq!(lines(&t, &sale.subject()).await.len(), 2);
    assert_eq!(account(&t, &m).await.points_held, 0);

    // A wrong code holds nothing.
    enter(&t, &sale, &shop.mail.wrong_code(&sale.subject()), 422).await;
    assert_eq!(
        redemptions(&t, &sale.subject()).await[0].status,
        RedemptionStatus::Pending
    );

    // The right code holds the points and adds the AED 8.00 line.
    enter_right(&t, &shop, &sale).await;
    let held = &redemptions(&t, &sale.subject()).await[0];
    assert_eq!(held.status, RedemptionStatus::Held);
    assert_eq!((held.points, held.value), (800, 800));
    assert_eq!(account(&t, &m).await.points_held, 800);
    let all = lines(&t, &sale.subject()).await;
    let line = redemption_lines(&all);
    assert_eq!(line.len(), 1);
    assert_eq!(line[0].description, "Loyalty redemption");
    assert_eq!(
        (line[0].net(), line[0].vat(), line[0].gross()),
        (-762, -38, -800)
    );
    assert_eq!(held.line_id, Some(line[0].id));

    // The VAT of the sale goes from AED 1.90 to AED 1.52.
    let sale = reload(&t, &sale).await;
    assert_eq!(
        (sale.net_total, sale.vat_total, sale.gross_total),
        (3048, 152, 3200)
    );
    t.app
        .get(&format!("/counter-sales/{}", sale.id))
        .await
        .assert_ok()
        .assert_see("Loyalty redemption");

    // Completing the sale writes the `redeem` and releases the hold.
    complete(&t, &sale, "32.00", 303).await;
    let done = &redemptions(&t, &sale.subject()).await[0];
    assert_eq!(done.status, RedemptionStatus::Applied);
    let spent = movements(&t, &m, PointsKind::Redeem).await;
    assert_eq!(spent.iter().map(|x| x.points).sum::<i64>(), -800);
    assert_eq!(spent.iter().map(|x| x.value).sum::<i64>(), 800);
    assert!(spent.iter().all(|x| x.source() == sale.subject()));
    // The sale earned on what was left after the redemption: 3048 fils.
    let m = account(&t, &m).await;
    assert_eq!((m.points_balance, m.points_held), (30, 0));
    let events = shop.points.redeemed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].party, events[0].points, events[0].value),
        (PartyId(M), 800, 800)
    );
    assert_eq!(events[0].source, sale.subject());
}

#[renox::test]
async fn with_a_maximum_share_of_50_percent_a_sale_of_aed_10_accepts_500_points() {
    let (t, shop, floor, m) = world(1000, &[("max_share_percent", "50")]).await;
    let programme = wl_sales::LoyaltyProgramme::first_active(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(programme.max_share_bps, 5_000);
    let sale = floor
        .open_cart(&t, &[("tee", "9.52", "1")], Some(&m.code))
        .await;
    assert_eq!(sale.gross_total, 1_000);

    // 600 points would pay AED 6.00, more than half: no code is sent.
    ask(&t, &sale, 600, 422).await;
    assert!(shop.mail.otp_requests().is_empty());
    assert!(redemptions(&t, &sale.subject()).await.is_empty());

    ask(&t, &sale, 500, 303).await;
    enter_right(&t, &shop, &sale).await;
    let all = lines(&t, &sale.subject()).await;
    assert_eq!(redemption_lines(&all)[0].gross(), -500);
    assert_eq!(reload(&t, &sale).await.gross_total, 500);
}

#[renox::test]
async fn m_with_400_points_cannot_redeem_below_the_minimum_of_500() {
    let (t, shop, floor, m) = world(400, &[]).await;
    let sale = floor
        .open_cart(&t, &[("tee", "50.00", "1")], Some(&m.code))
        .await;
    ask(&t, &sale, 400, 422).await;
    assert!(shop.mail.otp_requests().is_empty());
    assert!(redemptions(&t, &sale.subject()).await.is_empty());
    // Nor can M spend more than M has.
    ask(&t, &sale, 600, 422).await;
    assert!(shop.mail.otp_requests().is_empty());
}

#[renox::test]
async fn three_wrong_codes_refuse_the_redemption_for_the_sale_and_it_goes_on_without_it() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let sale = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    ask(&t, &sale, 600, 303).await;
    let wrong = shop.mail.wrong_code(&sale.subject());
    for _ in 0..3 {
        enter(&t, &sale, &wrong, 422).await;
    }
    let refused = &redemptions(&t, &sale.subject()).await[0];
    assert_eq!(refused.status, RedemptionStatus::Refused);
    assert_eq!(refused.refusal.as_deref(), Some("too_many_tries"));

    // The right code now does nothing, and no new code can be sent for the sale.
    enter(&t, &sale, &shop.mail.code_for(&sale.subject()), 422).await;
    ask(&t, &sale, 600, 422).await;
    assert_eq!(shop.mail.otp_requests().len(), 1);

    // The sale completes without the redemption line.
    complete(&t, &sale, "21.00", 303).await;
    let all = lines(&t, &sale.subject()).await;
    assert!(redemption_lines(&all).is_empty());
    assert_eq!(reload(&t, &sale).await.gross_total, 2_100);
    assert!(movements(&t, &m, PointsKind::Redeem).await.is_empty());

    // The refusal is for that sale only.
    let other = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    ask(&t, &other, 600, 303).await;
}

#[renox::test]
async fn a_code_entered_six_minutes_after_sending_is_expired_and_a_new_one_can_be_sent() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let sale = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    ask(&t, &sale, 600, 303).await;
    let first = shop.mail.code_for(&sale.subject());
    t.app.travel(Duration::from_secs(6 * 60));
    enter(&t, &sale, &first, 422).await;
    assert_eq!(
        redemptions(&t, &sale.subject()).await[0].status,
        RedemptionStatus::Pending
    );
    // A new code can be sent for the same redemption and works.
    ask(&t, &sale, 600, 303).await;
    assert_eq!(shop.mail.otp_requests().len(), 2);
    assert_eq!(redemptions(&t, &sale.subject()).await.len(), 1);
    enter_right(&t, &shop, &sale).await;
    assert_eq!(
        redemptions(&t, &sale.subject()).await[0].status,
        RedemptionStatus::Held
    );
}

#[renox::test]
async fn a_code_that_cannot_be_sent_refuses_the_redemption_and_the_sale_goes_on() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let sale = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    shop.mail.cannot_send_codes(true);
    ask(&t, &sale, 600, 422).await;
    let refused = &redemptions(&t, &sale.subject()).await[0];
    assert_eq!(refused.status, RedemptionStatus::Refused);
    assert_eq!(refused.refusal.as_deref(), Some("code_not_sent"));
    complete(&t, &sale, "21.00", 303).await;
    assert!(movements(&t, &m, PointsKind::Redeem).await.is_empty());

    // Another sale may try again once codes can be sent.
    shop.mail.cannot_send_codes(false);
    let next = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    ask(&t, &next, 600, 303).await;
}

#[renox::test]
async fn without_the_code_setting_the_points_are_held_at_once() {
    let sales = WlSales::new().confirm_redemption_by_code(false);
    let (t, shop, floor, m) = world_with(sales, 800, &[]).await;
    let sale = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    ask(&t, &sale, 600, 303).await;
    assert!(shop.mail.otp_requests().is_empty());
    let held = &redemptions(&t, &sale.subject()).await[0];
    assert_eq!(held.status, RedemptionStatus::Held);
    assert_eq!(held.confirmed_via.as_deref(), Some("counter"));
    assert_eq!(account(&t, &m).await.points_held, 600);
}

/// A completed sale of AED 40.00 with M's 800 points redeemed on it.
async fn redeemed_sale(t: &Booted, shop: &Shop, floor: &Floor, m: &LoyaltyAccount) -> CounterSale {
    let sale = forty(t, floor, m).await;
    ask(t, &sale, 800, 303).await;
    enter_right(t, shop, &sale).await;
    complete(t, &sale, "32.00", 303).await;
    reload(t, &sale).await
}

/// Returns both lines of `sale` through the pages: request, receive, inspect
/// and refund.
async fn return_everything(t: &Booted, sale: &CounterSale) -> SalesReturn {
    let items: Vec<String> = lines(t, &sale.subject())
        .await
        .iter()
        .filter(|l| l.kind == "item")
        .map(|l| format!("{}:1", l.id))
        .collect();
    t.app
        .post(
            "/sales-returns",
            &[
                ("sale_type", "counter_sale"),
                ("sale_id", &sale.id.to_string()),
                ("lines", &items.join(",")),
            ],
        )
        .await
        .assert_status(303);
    let r = SalesReturn::query()
        .order_by_desc(SalesReturn::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/sales-returns/{}/resolve", r.id),
            &[("resolution", "refund"), ("inspected", "1")],
        )
        .await
        .assert_status(303);
    let r = SalesReturn::find_or_404(t.db(), r.id).await.unwrap();
    assert_eq!(r.status, ReturnStatus::Resolved);
    r
}

#[renox::test]
async fn returning_the_sale_gives_the_800_points_back_with_their_expiry_and_tells_m() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let first_lot = movements(&t, &m, PointsKind::Earn).await[0].clone();
    let sale = redeemed_sale(&t, &shop, &floor, &m).await;
    assert_eq!(account(&t, &m).await.points_balance, 30);

    let r = return_everything(&t, &sale).await;
    // The 30 points the sale earned are taken back and the 800 come back.
    let m = account(&t, &m).await;
    assert_eq!(m.points_balance, 800);
    let row = &redemptions(&t, &sale.subject()).await[0];
    assert_eq!(row.returned_points, 800);
    let back = movements(&t, &m, PointsKind::ReverseRedeem).await;
    assert_eq!(back.iter().map(|x| x.points).sum::<i64>(), 800);
    assert!(
        back.iter()
            .all(|x| x.cause == "return" && x.source() == r.subject())
    );
    // They keep the expiry of the lot they came from.
    assert!(
        back.iter()
            .all(|x| x.lot_expires_on == first_lot.lot_expires_on)
    );
    let events = shop.points.given_back.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].party, events[0].points), (PartyId(M), 800));
    let told = shop.mail.of_type(LOYALTY_GIVEN_BACK);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].recipients,
        vec![NotifyRecipient::Party { id: PartyId(M) }]
    );
}

#[renox::test]
async fn a_lot_that_expired_gets_thirty_days_when_the_points_come_back() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let first_lot = movements(&t, &m, PointsKind::Earn).await[0].clone();
    let sale = redeemed_sale(&t, &shop, &floor, &m).await;
    let yesterday: Option<NaiveDate> = Some(wl_kernel::documents::today() - Days::days(1));
    PointsMovement::query()
        .where_(PointsMovement::ID.eq(first_lot.id))
        .update(t.db(), &[("lot_expires_on", &yesterday)])
        .await
        .unwrap();
    return_everything(&t, &sale).await;
    let back = movements(&t, &m, PointsKind::ReverseRedeem).await;
    assert_eq!(back.iter().map(|x| x.points).sum::<i64>(), 800);
    let in_thirty = wl_kernel::documents::today() + Days::days(30);
    assert!(back.iter().all(|x| x.lot_expires_on == Some(in_thirty)));
}

#[renox::test]
async fn points_confirmed_on_one_till_are_refused_on_another_until_the_hold_lapses() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let till1 = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    let till2 = floor
        .open_cart(&t, &[("tee", "20.00", "1")], Some(&m.code))
        .await;
    ask(&t, &till1, 800, 303).await;
    enter_right(&t, &shop, &till1).await;
    assert_eq!(account(&t, &m).await.points_held, 800);

    // Within the 30 minutes the same points cannot be spent on till 2.
    t.app.travel(Duration::from_secs(20 * 60));
    ask(&t, &till2, 800, 422).await;
    assert!(redemptions(&t, &till2.subject()).await.is_empty());

    // Till 1 is abandoned: the hold lapses and the points can be used.
    t.app.travel(Duration::from_secs(11 * 60));
    ask(&t, &till2, 800, 303).await;
    let released = t
        .app
        .at_travelled_time(async { redeem::release_lapsed(t.state(), renox::db::now()).await })
        .await
        .unwrap();
    assert_eq!(released, 1);
    let first = &redemptions(&t, &till1.subject()).await[0];
    assert_eq!(first.status, RedemptionStatus::Released);
    // The abandoned cart lost its line, and the hold is let go.
    let all = lines(&t, &till1.subject()).await;
    assert!(redemption_lines(&all).is_empty());
    assert_eq!(reload(&t, &till1).await.gross_total, 2_100);
    assert_eq!(account(&t, &m).await.points_held, 0);
    // Nothing was written for the abandoned sale.
    assert!(movements(&t, &m, PointsKind::Redeem).await.is_empty());
}

#[renox::test]
async fn a_shortfall_at_completion_takes_the_line_off_before_the_sale_completes() {
    let (t, shop, floor, m) = world(800, &[]).await;
    let sale = forty(&t, &floor, &m).await;
    ask(&t, &sale, 800, 303).await;
    enter_right(&t, &shop, &sale).await;
    assert_eq!(reload(&t, &sale).await.gross_total, 3_200);
    // 700 of the points are taken off meanwhile by two points adjustments
    // (each within the limit of 500).
    let ana = t
        .user("Ana", &[perm::LOYALTY_VIEW, perm::LOYALTY_ADJUST])
        .await;
    t.app.acting_as(&ana);
    for _ in 0..2 {
        t.app
            .post(
                &format!("/loyalty/accounts/{}/adjust", m.id),
                &[("points", "-350"), ("reason", "Abuse of the programme")],
            )
            .await
            .assert_status(303);
    }
    t.app.acting_as(&floor.cora);
    assert_eq!(account(&t, &m).await.points_balance, 100);

    // The tenders for AED 32.00 no longer match: the line is off.
    complete(&t, &sale, "32.00", 422).await;
    let all = lines(&t, &sale.subject()).await;
    assert!(redemption_lines(&all).is_empty());
    let sale = reload(&t, &sale).await;
    assert_eq!(sale.gross_total, 4_000);
    assert!(sale.is_open());
    assert_eq!(
        redemptions(&t, &sale.subject()).await[0].status,
        RedemptionStatus::Released
    );
    assert_eq!(account(&t, &m).await.points_held, 0);
    complete(&t, &sale, "40.00", 303).await;
    assert!(movements(&t, &m, PointsKind::Redeem).await.is_empty());
}

/// A draft order of party 7 through the page with the lines
/// (code, AED before VAT) and M attached.
async fn draft_order(
    t: &Booted,
    floor: &Floor,
    m: &LoyaltyAccount,
    channel: &str,
    mode: &str,
    items: &[(&str, &str)],
) -> SalesOrder {
    t.app.acting_as(&floor.cora);
    t.app
        .post(
            "/sales-orders",
            &[
                ("party_id", "7"),
                ("channel", channel),
                ("payment_mode", mode),
            ],
        )
        .await
        .assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    for (code, price) in items {
        t.app
            .post(
                &format!("/sales-orders/{}/lines", order.id),
                &[
                    ("item_code", *code),
                    ("unit_price", *price),
                    ("quantity", "1"),
                ],
            )
            .await
            .assert_status(303);
    }
    t.app
        .post(
            &format!("/sales-orders/{}/member", order.id),
            &[("code", &m.code)],
        )
        .await
        .assert_status(303);
    SalesOrder::find_or_404(t.db(), order.id).await.unwrap()
}

/// Redeems `points` on the draft order through the pages.
async fn redeem_on_order(t: &Booted, shop: &Shop, order: &SalesOrder, points: i64) {
    t.app
        .post(
            &format!("/sales-orders/{}/redeem", order.id),
            &[("points", &points.to_string())],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/sales-orders/{}/redeem/confirm", order.id),
            &[("code", &shop.mail.code_for(&order.subject()))],
        )
        .await
        .assert_status(303);
}

async fn order_now(t: &Booted, order: &SalesOrder) -> SalesOrder {
    SalesOrder::find_or_404(t.db(), order.id).await.unwrap()
}

#[renox::test]
async fn an_online_order_paid_with_500_points_and_rejected_gives_them_back_and_a_failed_payment_releases_the_hold()
 {
    let (t, shop, floor, m) = world(1000, &[]).await;
    let order = draft_order(&t, &floor, &m, "website", "online", &[("tee", "9.52")]).await;
    assert_eq!(order.gross_total, 1_000);
    redeem_on_order(&t, &shop, &order, 500).await;
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    let order = order_now(&t, &order).await;
    assert_eq!(order.status, SalesOrderStatus::AwaitingPayment);
    assert_eq!(order.gross_total, 500);
    // Held while the order waits for its payment, however long.
    t.app.travel(Duration::from_secs(3 * 60 * 60));
    let waiting = &redemptions(&t, &order.subject()).await[0];
    assert_eq!(waiting.status, RedemptionStatus::Held);
    assert!(waiting.hold_until.is_none());
    assert!(movements(&t, &m, PointsKind::Redeem).await.is_empty());

    // The payment is confirmed: the `redeem` is written.
    let paid = sales_orders::confirm_payment(t.state(), order.id, 500)
        .await
        .unwrap();
    assert_eq!(paid.status, SalesOrderStatus::PendingAcceptance);
    assert_eq!(
        redemptions(&t, &order.subject()).await[0].status,
        RedemptionStatus::Applied
    );
    let spent = movements(&t, &m, PointsKind::Redeem).await;
    assert_eq!(spent.iter().map(|x| x.points).sum::<i64>(), -500);
    assert_eq!(account(&t, &m).await.points_balance, 500);

    // The outlet rejects the order: the 500 points come back and M is told.
    // The cashier's session lapsed during the three hours: signing in again.
    t.app.acting_as(&floor.cora);
    t.app
        .post(
            &format!("/sales-orders/{}/transitions/reject", order.id),
            &[("reason", "Out of stock")],
        )
        .await
        .assert_status(303);
    assert_eq!(
        order_now(&t, &order).await.status,
        SalesOrderStatus::Rejected
    );
    assert_eq!(account(&t, &m).await.points_balance, 1000);
    let back = movements(&t, &m, PointsKind::ReverseRedeem).await;
    assert_eq!(back.iter().map(|x| x.points).sum::<i64>(), 500);
    let told = shop.mail.of_type(LOYALTY_GIVEN_BACK);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].recipients,
        vec![NotifyRecipient::Party { id: PartyId(M) }]
    );

    // An order whose payment fails releases the hold and writes no movement.
    let failing = draft_order(&t, &floor, &m, "website", "online", &[("tee", "9.52")]).await;
    redeem_on_order(&t, &shop, &failing, 500).await;
    assert_eq!(account(&t, &m).await.points_held, 500);
    t.app
        .post(
            &format!("/sales-orders/{}/submit", failing.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    let failed = sales_orders::payment_failed(t.state(), failing.id)
        .await
        .unwrap();
    assert_eq!(failed.status, SalesOrderStatus::Cancelled);
    assert_eq!(
        redemptions(&t, &failing.subject()).await[0].status,
        RedemptionStatus::Released
    );
    let m = account(&t, &m).await;
    assert_eq!((m.points_balance, m.points_held), (1000, 0));
    assert_eq!(movements(&t, &m, PointsKind::Redeem).await.len(), 1);
    assert_eq!(shop.mail.of_type(LOYALTY_GIVEN_BACK).len(), 1);
}

#[renox::test]
async fn cancelling_one_of_two_equal_lines_of_a_confirmed_order_gives_back_250_of_500_points() {
    let (t, shop, floor, m) = world(1000, &[]).await;
    let order = draft_order(
        &t,
        &floor,
        &m,
        "phone",
        "on_account",
        &[("tee", "9.52"), ("cap", "9.52")],
    )
    .await;
    assert_eq!(order.gross_total, 2_000);
    redeem_on_order(&t, &shop, &order, 500).await;
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    let order = order_now(&t, &order).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    assert_eq!(order.gross_total, 1_500);
    // Confirmed on account: the `redeem` is written at once.
    assert_eq!(
        redemptions(&t, &order.subject()).await[0].status,
        RedemptionStatus::Applied
    );
    assert_eq!(account(&t, &m).await.points_balance, 500);

    let url = format!("/sales-orders/{}/cancel-line", order.id);
    // A reason is required, the quantity must be open, and something must stay.
    t.app
        .htmx()
        .post(&url, &[("line_no", "1"), ("quantity", "1"), ("reason", "")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &url,
            &[
                ("line_no", "1"),
                ("quantity", "2"),
                ("reason", "Changed mind"),
            ],
        )
        .await
        .assert_status(422);
    t.app
        .post(
            &url,
            &[
                ("line_no", "1"),
                ("quantity", "1"),
                ("reason", "Changed mind"),
            ],
        )
        .await
        .assert_status(303);
    assert_eq!(
        order_now(&t, &order).await.status,
        SalesOrderStatus::Confirmed
    );
    let line = lines(&t, &order.subject()).await.remove(0);
    assert_eq!(line.cancelled_milli, 1000);
    // Half of the order's value is cancelled: 250 of the 500 points.
    let row = &redemptions(&t, &order.subject()).await[0];
    assert_eq!(row.returned_points, 250);
    let back = movements(&t, &m, PointsKind::ReverseRedeem).await;
    assert_eq!(back.iter().map(|x| x.points).sum::<i64>(), 250);
    assert!(back.iter().all(|x| x.cause == "cancel_line"));
    assert_eq!(account(&t, &m).await.points_balance, 750);
    assert_eq!(shop.mail.of_type(LOYALTY_GIVEN_BACK).len(), 1);

    // The last open line cannot be cancelled this way: that is `cancel`.
    t.app
        .post(
            &url,
            &[
                ("line_no", "2"),
                ("quantity", "1"),
                ("reason", "Changed mind"),
            ],
        )
        .await
        .assert_status(422);
    assert_eq!(account(&t, &m).await.points_balance, 750);
}

#[renox::test]
async fn the_discount_engine_prices_the_redemption_line_of_a_sale() {
    let (t, _shop, floor, m) = world(800, &[]).await;
    let sale = forty(&t, &floor, &m).await;
    let engine = discount_engine(t.state());
    let quote = engine
        .quote_redemption(
            t.state(),
            QuoteRedemption {
                account: LoyaltyAccountId(m.id),
                sale: sale.subject(),
                points: 800,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (quote.points, quote.value, quote.line_net, quote.line_vat),
        (800, 800, -762, -38)
    );
    assert_eq!(quote.maximum_points, 4_000);
    // Below the minimum it is refused.
    let refused = engine
        .quote_redemption(
            t.state(),
            QuoteRedemption {
                account: LoyaltyAccountId(m.id),
                sale: sale.subject(),
                points: 400,
            },
        )
        .await;
    assert!(refused.is_err());
    // Pricing adds no line and holds nothing.
    assert_eq!(lines(&t, &sale.subject()).await.len(), 2);
    assert_eq!(account(&t, &m).await.points_held, 0);
}
