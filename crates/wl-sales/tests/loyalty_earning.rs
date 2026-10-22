//! Loyalty points on qualifying sales (BL-SAL-22, #141, `SAL.RUL.07`,
//! `SAL.ACL.08`): the programme, enrolment with the terms version, attaching a
//! member before the sale completes, and earning on a counter sale, a sales
//! order reaching `fulfilled`, and a paid invoice (with the insurer's share
//! left out). Every check drives the real pages, transitions and listeners.

mod support;

use renox::chrono::{Duration, Months};
use renox::prelude::*;
use wl_kernel::consents::MARKETING;
use wl_kernel::invoicing::{InvoiceId, InvoicePaid};
use wl_kernel::notify::{NotifyChannel, NotifyRecipient};
use wl_kernel::sales::{FulfilOrder, FulfilledLine, SalesOrderId, order_fulfilment};
use wl_kernel::{PartyId, Subject};
use wl_sales::notices::LOYALTY_REWARD;
use wl_sales::{
    CounterSale, EarnKind, LoyaltyEarning, LoyaltyReview, PointsKind, SaleLine, SalesOrder,
    SalesOrderStatus, WlSales,
};

use support::floor::{Floor, account, movements};
use support::{Booted, boot_shop, boot_shop_with};

/// The party of the member M in every test.
const M: i64 = 7;

async fn member_floor() -> (Booted, support::shop::Shop, Floor) {
    let (t, shop) = boot_shop().await;
    let floor = Floor::new(&t).await;
    (t, shop, floor)
}

#[renox::test]
async fn a_counter_sale_of_aed_105_with_vat_earns_100_points_expiring_12_months_later() {
    let (t, shop, floor) = member_floor().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "050 123 4567").await;

    let sale = floor
        .ring_up(
            &t,
            &[("tee", "100.00", "1")],
            Some(&m.code),
            &[("cash", "105.00")],
        )
        .await;
    assert_eq!(sale.gross_total, 10_500);

    let m = account(&t, &m).await;
    assert_eq!(m.points_balance, 100);
    let lots = movements(&t, &m, PointsKind::Earn).await;
    assert_eq!(lots.len(), 1);
    assert_eq!(lots[0].points, 100);
    assert_eq!(lots[0].value, 100);
    assert_eq!(lots[0].source(), sale.subject());
    let on = wl_kernel::documents::dubai_date(sale.completed_at.unwrap());
    assert_eq!(
        lots[0].lot_expires_on,
        on.checked_add_months(Months::new(12))
    );
    let earned = shop.points.earned.lock().unwrap().clone();
    assert_eq!(earned.len(), 1);
    assert_eq!((earned[0].party, earned[0].points), (PartyId(M), 100));
    assert_eq!(earned[0].source, sale.subject());
    // The sale earns once: the earning is its key.
    let earning = LoyaltyEarning::of_source(t.db(), &sale.subject())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (earning.eligible_net, earning.basis_gross),
        (10_000, 10_500)
    );
}

#[renox::test]
async fn points_are_rounded_down_and_excluded_items_and_small_sales_do_not_count() {
    let (t, _, floor) = member_floor().await;
    floor
        .turn_on(&t, &[("minimum", "50.00"), ("excluded_items", "voucher")])
        .await;
    let m = floor.enrol(&t, M, "0501234567").await;

    // AED 99.90 counts for 99 points; the voucher counts for nothing.
    floor
        .ring_up(
            &t,
            &[("tee", "99.90", "1"), ("voucher", "40.00", "1")],
            Some(&m.code),
            &[("cash", "146.90")],
        )
        .await;
    assert_eq!(account(&t, &m).await.points_balance, 99);

    // Below the minimum nothing is earned.
    floor
        .ring_up(
            &t,
            &[("pen", "20.00", "1")],
            Some(&m.code),
            &[("cash", "21.00")],
        )
        .await;
    assert_eq!(account(&t, &m).await.points_balance, 99);
    assert_eq!(movements(&t, &m, PointsKind::Earn).await.len(), 1);
}

#[renox::test]
async fn a_sale_with_no_member_attached_before_completion_earns_nothing() {
    let (t, shop, floor) = member_floor().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    let sale = floor
        .ring_up(&t, &[("tee", "100.00", "1")], None, &[("cash", "105.00")])
        .await;
    assert_eq!(account(&t, &m).await.points_balance, 0);
    assert!(shop.points.earned.lock().unwrap().is_empty());
    // Attaching afterwards is refused (SAL.ACL.08): never after completion.
    t.app
        .post(
            &format!("/counter-sales/{}/member", sale.id),
            &[("code", &m.code)],
        )
        .await
        .assert_status(422);
    assert_eq!(
        CounterSale::find_or_404(t.db(), sale.id)
            .await
            .unwrap()
            .loyalty_account_id,
        None
    );
}

#[renox::test]
async fn an_invoice_with_an_insurer_share_earns_on_the_members_share_only() {
    let (t, shop, floor) = member_floor().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    // AED 1,000 before VAT (1,050 with it); the insurer pays 800 before VAT.
    let claim = Subject::new("clinics.visit", 3);
    shop.books.add(5, PartyId(M), 100_000, 105_000);
    shop.insurer.claim(claim.clone(), 80_000);

    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(5),
            party: PartyId(M),
            source: claim.clone(),
        })
        .await
        .unwrap();
    // The member's AED 200 before VAT (AED 210 with it) earns 200 points.
    let m = account(&t, &m).await;
    assert_eq!(m.points_balance, 200);
    let earning = LoyaltyEarning::of_source(t.db(), &Subject::new("invoicing.invoice", 5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (earning.eligible_net, earning.basis_gross),
        (20_000, 21_000)
    );

    // Paid again (an event delivered twice): it earns once.
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(5),
            party: PartyId(M),
            source: claim,
        })
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 200);
    assert_eq!(movements(&t, &m, PointsKind::Earn).await.len(), 1);
}

/// An on-account order of AED 300 before VAT for the party M with the member
/// attached, confirmed; the id of its only line.
async fn order(t: &Booted, member: &str) -> (SalesOrder, SaleLine) {
    t.app
        .post(
            "/sales-orders",
            &[("party_id", &M.to_string()), ("channel", "phone")],
        )
        .await
        .assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[("item_code", "desk"), ("unit_price", "300.00")],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/sales-orders/{}/member", order.id),
            &[("code", member)],
        )
        .await
        .assert_status(303);
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    let order = SalesOrder::find_or_404(t.db(), order.id).await.unwrap();
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    let line = SaleLine::of(t.db(), &order.subject())
        .await
        .unwrap()
        .remove(0);
    (order, line)
}

/// Hands the whole order over from the dispatch note `n`.
async fn hand_over(t: &Booted, order: &SalesOrder, line: &SaleLine, n: i64) {
    order_fulfilment(t.state())
        .fulfil(
            t.state(),
            FulfilOrder {
                order: SalesOrderId(order.id),
                lines: vec![FulfilledLine {
                    line: line.id,
                    quantity_milli: 1000,
                }],
                source: Subject::new("inventory.dispatch", n),
            },
        )
        .await
        .unwrap();
}

#[renox::test]
async fn an_order_on_account_earns_on_fulfilled_before_its_invoice_is_paid_and_only_once() {
    let (t, shop, floor) = member_floor().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    let (order, line) = order(&t, &m.code).await;

    // Confirmed is not enough: the order earns when it is fulfilled.
    assert_eq!(account(&t, &m).await.points_balance, 0);
    hand_over(&t, &order, &line, 1).await;
    let fulfilled = SalesOrder::find_or_404(t.db(), order.id).await.unwrap();
    assert_eq!(fulfilled.status, SalesOrderStatus::Fulfilled);
    assert_eq!(account(&t, &m).await.points_balance, 300);
    let earned = shop.points.earned.lock().unwrap().clone();
    assert_eq!(earned.len(), 1);
    assert_eq!(earned[0].source, order.subject());

    // Its invoice (drafted on fulfilment) is paid: no second earn.
    let invoice = fulfilled.invoice_id.expect("the invoice was drafted");
    t.state()
        .emit(InvoicePaid {
            invoice: InvoiceId(invoice),
            party: PartyId(M),
            source: order.subject(),
        })
        .await
        .unwrap();
    assert_eq!(account(&t, &m).await.points_balance, 300);
    assert_eq!(movements(&t, &m, PointsKind::Earn).await.len(), 1);
}

#[renox::test]
async fn a_per_visit_programme_makes_a_reward_available_on_the_tenth_sale_and_tells_the_member() {
    let (t, shop, floor) = member_floor().await;
    floor
        .turn_on(
            &t,
            &[
                ("earn_kind", "per_visit"),
                ("earn_rate", "10"),
                ("visit_reward", "a free coffee"),
            ],
        )
        .await;
    let m = floor.enrol(&t, M, "0501234567").await;
    let mut last = None;
    for n in 1..=10 {
        let sale = floor
            .ring_up(
                &t,
                &[("coffee", "10.00", "1")],
                Some(&m.code),
                &[("cash", "10.50")],
            )
            .await;
        last = Some(sale);
        let now = account(&t, &m).await;
        if n < 10 {
            assert_eq!(now.visits, n);
            assert_eq!(now.reward_available_until, None);
            assert!(shop.mail.of_type(LOYALTY_REWARD).is_empty());
        }
    }
    let m = account(&t, &m).await;
    // The 10th sale made the reward available; visits restart at 0.
    assert_eq!(m.visits, 0);
    let on = wl_kernel::documents::dubai_date(last.unwrap().completed_at.unwrap());
    assert_eq!(
        m.reward_available_until,
        on.checked_add_months(Months::new(12))
    );
    let told = shop.mail.of_type(LOYALTY_REWARD);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].recipients,
        vec![NotifyRecipient::Party { id: PartyId(M) }]
    );
    // A visit is a movement of one; the balance of points stays zero.
    assert_eq!(movements(&t, &m, PointsKind::Earn).await.len(), 10);
    assert_eq!(m.points_balance, 0);
    let earning = LoyaltyEarning::query()
        .order_by_desc(LoyaltyEarning::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(earning.earn_kind, EarnKind::PerVisit);
    assert!(earning.reward_made);
}

#[renox::test]
async fn enrolment_records_the_terms_version_and_is_not_a_marketing_consent() {
    let (t, _, floor) = member_floor().await;
    let programme = floor.turn_on(&t, &[("terms_version", "2026-1")]).await;
    assert_eq!(programme.terms_version, "2026-1");

    let m = floor.enrol(&t, M, "050 123 4567").await;
    assert_eq!(m.terms_version, "2026-1");
    assert_eq!(m.mobile, "+971501234567");
    assert_eq!(m.enrolled_via, "counter");
    assert_eq!(m.enrolled_by_user_id, Some(floor.cora.id));
    assert!(m.enrolled_at.is_some());
    assert!(m.code.starts_with("LYL-"));
    // No consent was created or implied: PTY.CNS.01 asks for it separately.
    let consents = wl_kernel::consents::consents(t.state());
    for channel in [
        NotifyChannel::Email,
        NotifyChannel::Sms,
        NotifyChannel::Whatsapp,
    ] {
        assert!(
            !consents
                .has_consent(t.db(), PartyId(M), MARKETING, channel)
                .await
                .unwrap()
        );
    }

    // The terms must be accepted.
    t.app
        .post(
            "/loyalty/accounts",
            &[("party_id", "8"), ("mobile", "0502222222")],
        )
        .await
        .assert_status(422);
    // A minor is enrolled by a guardian, who accepts the terms.
    t.app
        .post(
            "/loyalty/accounts",
            &[
                ("party_id", "9"),
                ("guardian_party_id", "10"),
                ("mobile", "0559999999"),
                ("terms", "1"),
            ],
        )
        .await
        .assert_status(303);
    let minor = wl_sales::LoyaltyAccount::of_party(t.db(), PartyId(9))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(minor.accepted_by_party_id, Some(10));
    assert_eq!(minor.notify_party(), PartyId(10));
}

#[renox::test]
async fn a_second_account_with_the_same_mobile_number_or_party_is_refused() {
    let (t, _, floor) = member_floor().await;
    floor.turn_on(&t, &[]).await;
    floor.enrol(&t, M, "050 123 4567").await;

    // The same number, written another way, for another party.
    t.app
        .post(
            "/loyalty/accounts",
            &[
                ("party_id", "8"),
                ("mobile", "+971 50 123 4567"),
                ("terms", "1"),
            ],
        )
        .await
        .assert_status(422);
    // The same party again.
    t.app
        .post(
            "/loyalty/accounts",
            &[("party_id", "7"), ("mobile", "0507777777"), ("terms", "1")],
        )
        .await
        .assert_status(422);
    // A bad number.
    t.app
        .post(
            "/loyalty/accounts",
            &[("party_id", "8"), ("mobile", "123"), ("terms", "1")],
        )
        .await
        .assert_status(422);
    assert_eq!(
        wl_sales::LoyaltyAccount::query()
            .count(t.db())
            .await
            .unwrap(),
        1
    );
    // Another party with another number is welcome.
    floor.enrol(&t, 8, "0508888888").await;
    assert_eq!(
        wl_sales::LoyaltyAccount::query()
            .count(t.db())
            .await
            .unwrap(),
        2
    );
}

#[renox::test]
async fn a_cashier_cannot_attach_a_sale_to_their_own_account_nor_a_member_to_yesterdays_sale() {
    let (t, shop, floor) = member_floor().await;
    floor.turn_on(&t, &[]).await;
    let own = floor.enrol(&t, M, "0501234567").await;
    let other = floor.enrol(&t, 8, "0508888888").await;
    // Cora is the employee whose person is party 7: the member M.
    shop.staff.set(floor.cora.id, M);

    t.app
        .post("/counter-sales", &[("x", "")])
        .await
        .assert_status(303);
    let sale = CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    let url = format!("/counter-sales/{}/member", sale.id);
    // By code, by mobile number and by portal login: refused.
    t.app
        .post(&url, &[("code", &own.code)])
        .await
        .assert_status(422);
    t.app
        .post(&url, &[("mobile", "050 123 4567")])
        .await
        .assert_status(422);
    t.app
        .post(&url, &[("party_id", "7")])
        .await
        .assert_status(422);
    assert_eq!(
        CounterSale::find_or_404(t.db(), sale.id)
            .await
            .unwrap()
            .loyalty_account_id,
        None
    );
    // Someone else's account is fine, by any of the three.
    t.app
        .post(&url, &[("mobile", "0508888888")])
        .await
        .assert_status(303);
    assert_eq!(
        CounterSale::find_or_404(t.db(), sale.id)
            .await
            .unwrap()
            .loyalty_account_id,
        Some(other.id)
    );
    // An unknown code is refused.
    t.app
        .post(&url, &[("code", "LYL-9999")])
        .await
        .assert_status(422);

    // A sale completed yesterday takes no member.
    let done = floor
        .ring_up(&t, &[("tee", "10.00", "1")], None, &[("cash", "10.50")])
        .await;
    CounterSale::query()
        .where_(CounterSale::ID.eq(done.id))
        .update(
            t.db(),
            &[("completed_at", &Some(renox::db::now() - Duration::days(1)))],
        )
        .await
        .unwrap();
    t.app
        .post(
            &format!("/counter-sales/{}/member", done.id),
            &[("code", &other.code)],
        )
        .await
        .assert_status(422);
}

#[renox::test]
async fn the_sixth_earning_sale_of_a_day_still_earns_and_is_listed_for_the_manager() {
    let (t, _, floor) = member_floor().await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    for _ in 0..5 {
        floor
            .ring_up(
                &t,
                &[("tee", "10.00", "1")],
                Some(&m.code),
                &[("cash", "10.50")],
            )
            .await;
    }
    assert!(
        LoyaltyReview::query()
            .first(t.db())
            .await
            .unwrap()
            .is_none()
    );
    floor
        .ring_up(
            &t,
            &[("tee", "10.00", "1")],
            Some(&m.code),
            &[("cash", "10.50")],
        )
        .await;
    // The 6th still earns.
    assert_eq!(account(&t, &m).await.points_balance, 60);
    let listed = LoyaltyReview::query().get(t.db()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].sales_count, 6);
    assert_eq!(listed[0].account_id, m.id);

    // The manager sees it, the cashier does not; reviewing clears it.
    t.app.acting_as(&floor.cora);
    t.app.get("/loyalty/review").await.assert_forbidden();
    t.app.acting_as(&floor.mia);
    t.app
        .get("/loyalty/review")
        .await
        .assert_ok()
        .assert_see(&m.code);
    t.app
        .post(
            &format!("/loyalty/review/{}/resolve", listed[0].id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    t.app
        .get("/loyalty/review")
        .await
        .assert_ok()
        .assert_dont_see(&m.code);
}

#[renox::test]
async fn a_higher_daily_limit_lists_nobody() {
    let (t, shop) = boot_shop_with(WlSales::new().loyalty_daily_sales_limit(8)).await;
    let _ = shop;
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    for _ in 0..6 {
        floor
            .ring_up(
                &t,
                &[("tee", "10.00", "1")],
                Some(&m.code),
                &[("cash", "10.50")],
            )
            .await;
    }
    assert!(
        LoyaltyReview::query()
            .first(t.db())
            .await
            .unwrap()
            .is_none()
    );
}
