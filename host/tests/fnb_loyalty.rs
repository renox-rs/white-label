//! BL-FNB-17 (#381), 06 §7.2, `SAL.LOY.01`, `SAL.RUL.07`, `SAL.ACL.07`: the
//! loyalty programme of a fresh 06 deployment through the real host with real
//! counter sales and the real wl-notify: a counter sale of AED 200 earns 200
//! points, a redemption may take at most half of the order, and an adjustment
//! of 501 points waits for another manager while 500 does not. Only the SMS
//! and e-mail providers and the clock are stand-ins.

mod support;

use std::sync::{Arc, Mutex};

use host::roles::ADMIN;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use support::{Wire, fnb_booted_rigged, person, tenant};
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyChannel;
use wl_kernel::{BusinessCalendar, PartyId};
use wl_notify::{Clock, WlNotify};
use wl_sales::{
    AdjustmentStatus, CounterSale, LoyaltyAccount, PointsAdjustment, PointsKind, PointsMovement,
};

/// The clock of wl-notify: Dubai noon of today.
#[derive(Clone)]
struct Moving(Arc<Mutex<DateTime>>);

struct Rig(Moving);

impl Module for Rig {
    fn name(&self) -> &'static str {
        "fnb_loyalty_rig"
    }

    fn register(&self, app: &mut Registry) {
        let at = (self.0).0.clone();
        app.provide(Clock::from_fn(move || *at.lock().unwrap()));
    }
}

fn noon(day: NaiveDate) -> DateTime {
    BusinessCalendar::default().from_local(day.and_hms_opt(12, 0, 0).expect("a time"))
}

async fn world() -> (TestApp, Wire) {
    let sms = Wire::new(NotifyChannel::Sms);
    let mail = Wire::new(NotifyChannel::Email);
    let clock = Moving(Arc::new(Mutex::new(noon(today()))));
    let (s, m) = (sms.clone(), mail.clone());
    let app = fnb_booted_rigged(
        move |n: WlNotify| n.provider(s).provider(m),
        Rig(clock),
    )
    .await;
    (app, sms)
}

async fn account(app: &TestApp, party: PartyId) -> LoyaltyAccount {
    LoyaltyAccount::of_party(app.db(), party)
        .await
        .unwrap()
        .expect("the member is enrolled")
}

/// AED text of `fils`, as a form field.
fn aed(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

/// A counter sale of `price` fils before VAT (one line), the member attached;
/// it stays open.
async fn open_cart(app: &TestApp, member: &LoyaltyAccount, item: &str, price: i64) -> CounterSale {
    app.htmx()
        .post("/counter-sales", &[("x", "")])
        .await
        .assert_status(303);
    let sale = CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the cart was opened");
    app.htmx()
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[
                ("item_code", item),
                ("unit_price", aed(price).as_str()),
                ("quantity", "1"),
            ],
        )
        .await
        .assert_status(303);
    app.htmx()
        .post(
            &format!("/counter-sales/{}/member", sale.id),
            &[("code", member.code.as_str())],
        )
        .await
        .assert_status(303);
    CounterSale::find_or_404(app.db(), sale.id).await.unwrap()
}

/// Rings up and completes a counter sale paid in cash.
async fn ring_up(app: &TestApp, member: &LoyaltyAccount, item: &str, price: i64) -> CounterSale {
    let sale = open_cart(app, member, item, price).await;
    app.htmx()
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", aed(sale.gross_total).as_str())],
        )
        .await
        .assert_status(303);
    CounterSale::find_or_404(app.db(), sale.id).await.unwrap()
}

#[renox::test]
async fn the_programme_earns_a_point_per_dirham_limits_a_redemption_and_asks_a_manager_above_500() {
    let (app, sms) = world().await;
    let ada = person(&app, "Ada", ADMIN).await;
    let alia = person(&app, "Alia", ADMIN).await;
    let mariam = tenant(&app, "Mariam Al Hosani").await;

    // The cashier enrols the customer with her mobile.
    app.acting_as(&ada);
    app.htmx()
        .post(
            "/loyalty/accounts",
            &[
                ("party_id", mariam.0.to_string().as_str()),
                ("mobile", "0501234567"),
                ("terms", "1"),
            ],
        )
        .await
        .assert_status(303);
    let member = account(&app, mariam).await;

    // A counter sale of AED 200 before VAT earns 200 points.
    let sale = ring_up(&app, &member, "shawarma-plate", 20_000).await;
    assert_eq!(sale.net_total, 20_000);
    assert_eq!(account(&app, mariam).await.points_balance, 200);

    // 100 points are AED 5; an order of AED 8 may take half: AED 4. Refused.
    let small = open_cart(&app, &member, "falafel-wrap", 800).await;
    let refused = app
        .htmx()
        .post(
            &format!("/counter-sales/{}/redeem", small.id),
            &[("points", "100")],
        )
        .await;
    refused.assert_status(422);
    assert_eq!(account(&app, mariam).await.points_balance, 200);
    // On an order of AED 20 the same 100 points are within half of it: the
    // code is sent to the member's mobile.
    let big = open_cart(&app, &member, "shawarma-plate", 2_000).await;
    app.htmx()
        .post(
            &format!("/counter-sales/{}/redeem", big.id),
            &[("points", "100")],
        )
        .await
        .assert_status(303);
    app.run_all_jobs().await;
    let code = sms.sent().pop().expect("the code was sent");
    assert_eq!(code.address, "+971501234567");

    // 500 points added by hand are applied; 501 wait for another manager.
    app.htmx()
        .post(
            &format!("/loyalty/accounts/{}/adjust", member.id),
            &[("points", "500"), ("reason", "Goodwill")],
        )
        .await
        .assert_status(303);
    assert_eq!(account(&app, mariam).await.points_balance, 700);
    app.htmx()
        .post(
            &format!("/loyalty/accounts/{}/adjust", member.id),
            &[("points", "501"), ("reason", "More goodwill")],
        )
        .await
        .assert_status(303);
    let waiting = PointsAdjustment::of_account(app.db(), member.id)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.status == AdjustmentStatus::PendingApproval)
        .expect("501 points wait");
    assert_eq!(account(&app, mariam).await.points_balance, 700);
    let request = waiting.approval_request_id.expect("an approval request");
    assert!(
        approvals::approve(app.state(), request, &ada, "").await.is_err(),
        "the requester cannot approve"
    );
    approvals::approve(app.state(), request, &alia, "agreed")
        .await
        .unwrap();
    assert_eq!(account(&app, mariam).await.points_balance, 1_201);
    let added: i64 = PointsMovement::of_account(app.db(), member.id)
        .await
        .unwrap()
        .iter()
        .filter(|m| m.kind == PointsKind::Adjust)
        .map(|m| m.points)
        .sum();
    assert_eq!(added, 1_001);
}
