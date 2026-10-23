//! The outage and the daily closing in the 06 host (BL-FNB-14 #378, BL-FNB-15
//! #379, BL-PAY-19 #112): the roles hold the permissions of the stories, the
//! demo company has Marina in an outage with its brands paused and Sharjah's
//! Habibi Bowls open, a closing with a till shortage and a late entry the area
//! manager approved, a closing that refused a late entry, and the screens open
//! for the roles that hold them.

use fnb::closing::{ClosingStatus, DailyClosing};
use fnb::outage::{self, Outage, OutageStatus, OutletChannel, channel_kind};
use fnb::permissions as perm;
use fnb::{Brand, Outlet, PaperOrder};
use host::roles::{self, ACCOUNTANT, ADMIN, AREA_MANAGER, CASHIER, KITCHEN_MANAGER, LINE_COOK};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;
use wl_kernel::custody::HandoverStatus;

const DEMO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-fnb/client.toml"
);

fn holds(role: &str, permission: &str) -> bool {
    roles::grants_of(role).contains(&permission)
}

async fn demo() -> TestApp {
    let config = ClientConfig::load(DEMO).expect("the demo client");
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    // As a deployment does before the demo: the install, then the seeds.
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.kernel().seed().await.expect("the seeds run");
    app.kernel()
        .call(host::fnb_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs");
    app
}

async fn user(app: &TestApp, email: &str) -> User {
    User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .expect("the demo person is made")
}

/// AC: the kitchen manager resumes the channels, enters the paper orders and
/// submits the closing; only the area manager reopens it and approves a late
/// entry; the accountant approves but does not reopen; the cashier keeps the till.
#[renox::test]
async fn the_roles_hold_the_permissions_of_the_outage_and_the_closing() {
    let _app = demo().await;
    for permission in [
        perm::OUTLET_CHANNELS_VIEW,
        perm::OUTLET_CHANNELS_MANAGE,
        perm::PAPER_ORDERS_ENTER,
        perm::CLOSING_VIEW,
        perm::CLOSING_SUBMIT,
    ] {
        assert!(holds(KITCHEN_MANAGER, permission), "{permission}");
        assert!(holds(ADMIN, permission), "{permission}");
    }
    assert!(!holds(KITCHEN_MANAGER, perm::CLOSING_REOPEN));
    assert!(!holds(KITCHEN_MANAGER, perm::CLOSING_APPROVE));
    for permission in [
        perm::OUTLET_CHANNELS_MANAGE,
        perm::CLOSING_APPROVE,
        perm::CLOSING_REOPEN,
        perm::CLOSING_CUTOFF,
        "late_entries.approve",
    ] {
        assert!(holds(AREA_MANAGER, permission), "{permission}");
    }
    assert!(holds(ACCOUNTANT, perm::CLOSING_APPROVE));
    assert!(holds(ACCOUNTANT, "pos.review"));
    assert!(!holds(ACCOUNTANT, perm::CLOSING_REOPEN));
    assert!(holds(CASHIER, perm::PAPER_ORDERS_ENTER));
    assert!(holds(CASHIER, "pos.open_till"));
    assert!(holds(CASHIER, "pos.close_till"));
    assert!(!holds(CASHIER, perm::CLOSING_SUBMIT));
    assert!(!holds(LINE_COOK, perm::OUTLET_CHANNELS_VIEW));
}

/// AC: Marina's kitchen devices are silent in the demo: its brands are paused,
/// Habibi Bowls is open at Sharjah, the managers have the notice, and a paper
/// order of the outage carries the late-entry flag.
#[renox::test]
async fn the_demo_has_marina_in_an_outage_with_paused_brands_and_a_late_paper_order() {
    let app = demo().await;
    let db = app.db();
    let marina = Outlet::by_code(db, "marina").await.unwrap().unwrap();
    let sharjah = Outlet::by_code(db, "sharjah").await.unwrap().unwrap();
    let open = Outage::open_of(db, marina.id)
        .await
        .unwrap()
        .expect("an outage");
    assert_eq!(open.status, OutageStatus::Paused);
    let paused: Vec<OutletChannel> = OutletChannel::of_outlet(db, marina.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|c| c.kind == channel_kind::D2C)
        .collect();
    assert_eq!(paused.len(), 2);
    assert!(
        paused
            .iter()
            .all(|c| c.paused && c.outage_id == Some(open.id))
    );
    let habibi = Brand::by_code(db, "habibi").await.unwrap().unwrap();
    assert!(
        !outage::accepts_orders(db, marina.id, habibi.id)
            .await
            .unwrap()
    );
    assert!(
        outage::accepts_orders(db, sharjah.id, habibi.id)
            .await
            .unwrap()
    );
    assert!(Outage::open_of(db, sharjah.id).await.unwrap().is_none());

    let orders = PaperOrder::latest(db, marina.id, 10).await.unwrap();
    assert!(
        orders
            .iter()
            .any(|p| p.late && p.outage_id == Some(open.id)),
        "{orders:?}"
    );
    // Seeding again changes nothing.
    app.kernel()
        .call(host::fnb_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs again");
    assert_eq!(Outage::query().count(db).await.unwrap(), 1);
}

/// AC: the demo's closings: Marina's old day is approved and refused a late
/// entry (the area manager has the task); Sharjah's is reopened by an approved
/// late entry, and its till was AED 35.00 short.
#[renox::test]
async fn the_demo_has_a_closing_reopened_by_a_late_entry_and_one_that_refused_it() {
    let app = demo().await;
    let db = app.db();
    let closings = DailyClosing::query().get(db).await.unwrap();
    let approved: Vec<&DailyClosing> = closings
        .iter()
        .filter(|c| c.status == ClosingStatus::Approved)
        .collect();
    assert_eq!(approved.len(), 1, "{closings:?}");
    let reopened: Vec<&DailyClosing> = closings
        .iter()
        .filter(|c| c.status == ClosingStatus::Reopened)
        .collect();
    assert_eq!(reopened.len(), 1, "{closings:?}");
    let sharjah = Outlet::by_code(db, "sharjah").await.unwrap().unwrap();
    assert_eq!(reopened[0].outlet_id, sharjah.id);
    assert_eq!(
        reopened[0].reopen_reason.as_deref(),
        Some("Late entries after an outage")
    );
    assert_eq!(reopened[0].variance, Some(-3_500));
    let late = PaperOrder::of_day(db, sharjah.id, reopened[0].business_date.unwrap())
        .await
        .unwrap();
    assert_eq!(late.len(), 1);
    assert!(late[0].late);
    assert_eq!(late[0].reopened_closing_id, Some(reopened[0].id));
    // The shortage of the till is a short handover of the till.
    let handover = wl_payments::cash_till::of_session(db, reopened[0].till_session_id.unwrap())
        .await
        .unwrap()
        .expect("the closed till made a handover");
    assert_eq!(handover.status, HandoverStatus::Short);
    assert_eq!(handover.difference, 3_500);
    // And the approved closing refused a paper order: nothing of it was kept.
    let marina = Outlet::by_code(db, "marina").await.unwrap().unwrap();
    assert!(
        PaperOrder::of_day(db, marina.id, approved[0].business_date.unwrap())
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC: the screens open for the roles that hold them and not for the cook.
#[renox::test]
async fn the_outage_and_closing_screens_open_for_the_roles_that_hold_them() {
    let app = demo().await;
    let kitchen = user(&app, "kitchen@zaatar.example").await;
    let area = user(&app, "area@zaatar.example").await;
    let cashier = user(&app, "cashier@zaatar.example").await;
    let cook = user(&app, "cook@zaatar.example").await;

    app.acting_as(&kitchen);
    app.get("/outlet-channels")
        .await
        .assert_ok()
        .assert_see("Marina Kitchen")
        .assert_see("Temporarily closed")
        .assert_see("Resume the channels");
    app.get("/closings")
        .await
        .assert_ok()
        .assert_see("Sharjah Kitchen");
    app.get("/ordering/habibi")
        .await
        .assert_ok()
        .assert_see("Taking orders")
        .assert_see("Temporarily closed");

    app.acting_as(&area);
    app.get("/closings").await.assert_ok();
    let approved = DailyClosing::query()
        .where_(DailyClosing::STATUS.eq(ClosingStatus::Approved))
        .first(app.db())
        .await
        .unwrap()
        .expect("an approved closing");
    app.get(&format!("/closings/{}", approved.id))
        .await
        .assert_ok();

    app.acting_as(&cashier);
    app.get("/till-sessions").await.assert_ok().assert_see("T2");
    app.get("/outlet-channels").await.assert_ok();

    app.acting_as(&cook);
    app.get("/outlet-channels").await.assert_forbidden();
    app.get("/closings").await.assert_forbidden();
    app.get("/till-sessions").await.assert_forbidden();
}
