//! BL-FNB-14 (#378), 06 §4.6 row *Internet or power outage at the outlet*,
//! §7.2 `FNB.RUL.15`, `PLT.ACL.10`: an outlet whose kitchen devices stop
//! answering for 5 minutes during its opening hours has its brands paused on the
//! ordering site ("temporarily closed") and both managers told; a brand also
//! cooked by another outlet stays orderable there; silence outside the opening
//! hours triggers nothing; when the devices answer again the kitchen manager is
//! asked to resume (every 15 minutes) and the channels stay paused until the
//! manager resumes them; a paper order is entered with its business time and the
//! late-entry flag. The modules are the real wl-kernel device connections,
//! wl-sales, wl-payments, wl-inventory and wl-delivery.

mod support;

use fnb::outage::{self, Outage, OutageStatus, OutletChannel, channel_kind};
use fnb::{Brand, PaperOrder};
use renox::prelude::*;
use wl_kernel::integrations::ConnectionStatus;

use renox::testing::TestResponse;
use support::World;
use support::ops::{
    MARINA, Ops, SHARJAH, beat, boot_ops, day_after, dubai_at, go_to, now_of, status_of, tick,
};

const NOTICE: &str = "fnb.staff_notice";

/// Both of Marina's devices answered at `clock` tomorrow, Sharjah's too.
async fn all_beat(w: &World, clock_day: i64, clock: &str) {
    go_to(w, dubai_at(clock_day, clock)).await;
    for device in ["kds-1", "kds-2"] {
        beat(w, MARINA, device).await;
    }
    beat(w, SHARJAH, "kds-s1").await;
}

/// The D2C channels of `outlet_id`, paused or not.
async fn d2c(w: &World, outlet_id: i64) -> Vec<OutletChannel> {
    OutletChannel::of_outlet(w.db(), outlet_id)
        .await
        .unwrap()
        .into_iter()
        .filter(|c| c.kind == channel_kind::D2C)
        .collect()
}

/// Devices silent from 20:00; the check at 20:06 takes the connection down.
async fn marina_down(w: &World, marina: i64) -> Outage {
    all_beat(w, 1, "20:00").await;
    go_to(w, dubai_at(1, "20:06")).await;
    // Sharjah's device keeps answering.
    beat(w, SHARJAH, "kds-s1").await;
    tick(w).await;
    Outage::open_of(w.db(), marina)
        .await
        .unwrap()
        .expect("the outage opened")
}

/// A paper order P-0001 of AED 42.00 that happened at 20:20 on `day`, posted
/// by `by`.
async fn paper(w: &World, by: &User, outlet: i64, day: &str, reason: &str) -> TestResponse {
    let outlet = outlet.to_string();
    w.post(
        by,
        "/paper-orders",
        &[
            ("outlet", outlet.as_str()),
            ("channel", "counter"),
            ("reference", "P-0001"),
            ("amount", "42.00"),
            ("method", "cash"),
            ("occurred_on", day),
            ("occurred_time", "20:20"),
            ("reason", reason),
        ],
    )
    .await
}

/// AC1: Outlet 1 open 11:00 to 02:00 cooking brands A and B, devices silent from
/// 20:00: at 20:05 A and B show "temporarily closed" for Outlet 1 and both
/// managers are notified.
#[renox::test]
async fn silent_devices_pause_the_outlets_brands_and_tell_both_managers() {
    let (w, o) = boot_ops().await;
    let outage = marina_down(&w, o.marina.id).await;
    assert_eq!(status_of(&w, MARINA).await, ConnectionStatus::Down);
    assert_eq!(outage.outlet_id, o.marina.id);
    assert_eq!(outage.status, OutageStatus::Paused);
    assert_eq!(outage.source, outage::source::DEVICES);
    assert_eq!(outage.connection_key.as_deref(), Some(MARINA));

    let marina = d2c(&w, o.marina.id).await;
    assert_eq!(marina.len(), 2, "Zaatar Grill and Habibi Bowls");
    assert!(
        marina
            .iter()
            .all(|c| c.paused && c.outage_id == Some(outage.id))
    );

    // The ordering page shows Marina closed.
    let habibi = Brand::by_code(w.db(), "habibi").await.unwrap().unwrap();
    let zaatar = Brand::by_code(w.db(), "zaatar").await.unwrap().unwrap();
    assert!(
        !outage::accepts_orders(w.db(), o.marina.id, zaatar.id)
            .await
            .unwrap()
    );
    assert!(
        !outage::accepts_orders(w.db(), o.marina.id, habibi.id)
            .await
            .unwrap()
    );
    let page = w.get(&w.nobody, "/ordering/zaatar").await;
    page.assert_ok();
    assert!(
        page.text().contains("Temporarily closed"),
        "{}",
        page.text()
    );

    // Both managers are told, and the kitchen manager has the task.
    let told = w.notices.users_of_type(NOTICE);
    assert!(told.contains(&o.kitchen.id), "{told:?}");
    assert!(told.contains(&o.area.id), "{told:?}");
    assert_eq!(w.tickets.opened_with(&outage::task_key(&outage)).len(), 1);
}

/// AC2: Brand A cooked also by Outlet 2 stays orderable in Outlet 2's areas;
/// Sharjah's own channels and the aggregators are not touched.
#[renox::test]
async fn a_brand_cooked_by_another_outlet_stays_orderable_there() {
    let (w, o) = boot_ops().await;
    let outage = marina_down(&w, o.marina.id).await;
    let habibi = Brand::by_code(w.db(), "habibi").await.unwrap().unwrap();
    assert!(
        outage::accepts_orders(w.db(), o.sharjah.id, habibi.id)
            .await
            .unwrap()
    );
    let kitchens = outage::ordering_of(w.db(), &habibi).await.unwrap();
    let open: Vec<(&str, bool)> = kitchens.iter().map(|k| (k.name.as_str(), k.open)).collect();
    assert_eq!(open, [("Marina Kitchen", false), ("Sharjah Kitchen", true)]);
    assert!(d2c(&w, o.sharjah.id).await.iter().all(|c| !c.paused));
    assert!(
        Outage::open_of(w.db(), o.sharjah.id)
            .await
            .unwrap()
            .is_none()
    );
    // An aggregator's tablet is paused by hand, not by the rule.
    let tablets: Vec<OutletChannel> = OutletChannel::of_outlet(w.db(), o.marina.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|c| c.kind == channel_kind::AGGREGATOR)
        .collect();
    assert_eq!(tablets.len(), 3);
    assert!(tablets.iter().all(|c| !c.paused));
    assert!(outage.is_open());
}

/// AC3: devices silent at 03:00, outside Marina's opening hours, trigger
/// nothing.
#[renox::test]
async fn silence_outside_the_opening_hours_triggers_nothing() {
    let (w, o) = boot_ops().await;
    all_beat(&w, 1, "01:40").await;
    go_to(&w, dubai_at(1, "03:00")).await;
    tick(&w).await;
    go_to(&w, dubai_at(1, "03:10")).await;
    tick(&w).await;
    assert_ne!(status_of(&w, MARINA).await, ConnectionStatus::Down);
    assert!(
        Outage::open_of(w.db(), o.marina.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(d2c(&w, o.marina.id).await.iter().all(|c| !c.paused));
}

/// AC4: at 21:10 the devices answer; the kitchen manager is asked to resume and
/// to enter paper orders; the channels stay paused until the manager resumes;
/// the prompt repeats every 15 minutes and the system never resumes.
#[renox::test]
async fn the_manager_is_asked_to_resume_and_the_channels_stay_paused_until_then() {
    let (w, o) = boot_ops().await;
    let outage = marina_down(&w, o.marina.id).await;
    let before = w.notices.of_type(NOTICE).len();

    go_to(&w, dubai_at(1, "21:10")).await;
    beat(&w, MARINA, "kds-1").await;
    beat(&w, MARINA, "kds-2").await;
    let back = Outage::find_or_404(w.db(), outage.id).await.unwrap();
    assert_eq!(back.status, OutageStatus::DevicesBack);
    assert!(back.devices_back_at.is_some());
    assert_eq!(back.prompts, 1, "asked once when the devices answered");
    assert!(w.notices.of_type(NOTICE).len() > before);
    assert!(
        d2c(&w, o.marina.id).await.iter().all(|c| c.paused),
        "nothing resumes by itself"
    );

    // Not due again before 15 minutes; due at 15.
    let state = w.state().clone();
    let at = now_of(&w).await;
    let sent = outage::run_prompts(&state, at + renox::chrono::TimeDelta::minutes(14))
        .await
        .unwrap();
    assert_eq!(sent, 0);
    let sent = outage::run_prompts(&state, at + renox::chrono::TimeDelta::minutes(15))
        .await
        .unwrap();
    assert_eq!(sent, 1);
    assert_eq!(
        Outage::find_or_404(w.db(), outage.id)
            .await
            .unwrap()
            .prompts,
        2
    );
    assert!(d2c(&w, o.marina.id).await.iter().all(|c| c.paused));

    // A person without the permission cannot resume; the manager does.
    w.post(
        &w.nobody,
        &format!("/outages/{}/resume", outage.id),
        &[("x", "")],
    )
    .await
    .assert_forbidden();
    w.post(
        &o.kitchen,
        &format!("/outages/{}/resume", outage.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let resumed = Outage::find_or_404(w.db(), outage.id).await.unwrap();
    assert_eq!(resumed.status, OutageStatus::Resumed);
    assert_eq!(resumed.resumed_by, Some(o.kitchen.id));
    assert!(d2c(&w, o.marina.id).await.iter().all(|c| !c.paused));
    assert_eq!(
        w.tickets.closed().len(),
        1,
        "the kitchen manager's task closes"
    );
    // No more prompts once resumed.
    let sent = outage::run_prompts(&state, at + renox::chrono::TimeDelta::minutes(40))
        .await
        .unwrap();
    assert_eq!(sent, 0);
}

/// Flapping: a second `down` while the outage is open does nothing new; devices
/// that answered and went silent again put the outage back to paused.
#[renox::test]
async fn a_second_down_while_paused_does_nothing_new() {
    let (w, o) = boot_ops().await;
    let first = marina_down(&w, o.marina.id).await;
    let notified = w.notices.of_type(NOTICE).len();
    let state = w.state().clone();
    let marina = fnb::Outlet::find_or_404(w.db(), o.marina.id).await.unwrap();
    let again = outage::open_by_devices(&state, &marina, now_of(&w).await)
        .await
        .unwrap();
    assert!(again.is_none(), "an outage is open already");
    assert_eq!(Outage::query().count(w.db()).await.unwrap(), 1);
    assert_eq!(w.notices.of_type(NOTICE).len(), notified);
    assert_eq!(first.status, OutageStatus::Paused);
}

/// AC5: a paper order entered at 21:30 with business time 20:20 carries the
/// late-entry flag, needs a reason, and joins the outage.
#[renox::test]
async fn a_paper_order_entered_late_carries_the_flag_and_its_business_time() {
    let (w, o) = boot_ops().await;
    let outage = marina_down(&w, o.marina.id).await;
    go_to(&w, dubai_at(1, "21:30")).await;
    let day = day_after(1).to_string();
    // Seventy minutes late: a reason is needed.
    paper(&w, &o.till, o.marina.id, &day, "")
        .await
        .assert_status(422);
    assert_eq!(PaperOrder::query().count(w.db()).await.unwrap(), 0);
    paper(&w, &o.till, o.marina.id, &day, "Internet was down")
        .await
        .assert_status(303);

    let paper = PaperOrder::query()
        .first(w.db())
        .await
        .unwrap()
        .expect("recorded");
    assert!(paper.late, "the late-entry flag");
    assert_eq!(paper.late_reason.as_deref(), Some("Internet was down"));
    assert_eq!(paper.amount, 4_200);
    assert_eq!(paper.outage_id, Some(outage.id));
    assert_eq!(paper.entered_by, Some(o.till.id));
    assert_eq!(paper.occurred_at, Some(dubai_at(1, "20:20")));
    assert!(paper.entered_at.unwrap() >= dubai_at(1, "21:30"));
    assert_eq!(paper.business_date, Some(day_after(1)));
    assert!(paper.late_entry_id.is_some());

    // The channels page lists it, for the roles that see the channels.
    let page = w.get(&o.kitchen, "/outlet-channels").await;
    page.assert_ok();
    assert!(page.text().contains("P-0001"), "{}", page.text());
    assert!(page.text().contains("Marina Kitchen"));
}

/// A manager pauses an outlet by hand with a reason; nothing but a manager
/// resumes it; a cashier cannot pause or resume.
#[renox::test]
async fn a_manager_pauses_the_channels_by_hand_and_a_cashier_cannot() {
    let (w, o) = boot_ops().await;
    let path = format!("/outlets/{}/pause", o.sharjah.id);
    w.post(&o.till, &path, &[("reason", "Oven broken")])
        .await
        .assert_forbidden();
    w.post(&o.area, &path, &[("reason", "")])
        .await
        .assert_status(422);
    w.post(&o.area, &path, &[("reason", "Oven broken")])
        .await
        .assert_status(303);
    let outage = Outage::open_of(w.db(), o.sharjah.id)
        .await
        .unwrap()
        .expect("paused");
    assert_eq!(outage.source, outage::source::MANUAL);
    assert_eq!(outage.reason.as_deref(), Some("Oven broken"));
    assert!(d2c(&w, o.sharjah.id).await.iter().all(|c| c.paused));
    // A second pause is refused: it is paused already.
    w.post(&o.area, &path, &[("reason", "Again")])
        .await
        .assert_status(422);
    // One aggregator's tablet is marked paused and resumed on its own.
    let tablet = OutletChannel::of_outlet(w.db(), o.sharjah.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.kind == channel_kind::AGGREGATOR)
        .unwrap();
    w.post(
        &o.area,
        &format!("/outlet-channels/{}/pause", tablet.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let paused = OutletChannel::find_or_404(w.db(), tablet.id).await.unwrap();
    assert!(paused.paused);
    assert_eq!(paused.outage_id, Some(outage.id));
    w.post(
        &o.till,
        &format!("/outages/{}/resume", outage.id),
        &[("x", "")],
    )
    .await
    .assert_forbidden();
    w.post(
        &o.area,
        &format!("/outages/{}/resume", outage.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    assert!(
        OutletChannel::of_outlet(w.db(), o.sharjah.id)
            .await
            .unwrap()
            .iter()
            .all(|c| !c.paused)
    );
}

/// The brand's checkout of a paused kitchen takes no order: it says the kitchen
/// is temporarily closed and offers nothing to pay with; the kitchen that is
/// open, or a brand with another kitchen open, checks out as before.
#[renox::test]
async fn the_checkout_of_a_paused_kitchen_takes_no_order() {
    let (w, o) = boot_ops().await;
    let before = w
        .get(&o.till, "/checkout/payment?brand=zaatar&outlet=marina")
        .await;
    before.assert_ok();
    assert!(before.text().contains("pay-online"), "{}", before.text());
    marina_down(&w, o.marina.id).await;
    let closed = w
        .get(&o.till, "/checkout/payment?brand=zaatar&outlet=marina")
        .await;
    closed.assert_ok();
    assert!(
        closed.text().contains("kitchen-closed"),
        "{}",
        closed.text()
    );
    assert!(!closed.text().contains("pay-online"));
    // Habibi Bowls is still cooked at Sharjah.
    let open = w.get(&o.till, "/checkout/payment?brand=habibi").await;
    assert!(open.text().contains("pay-online"), "{}", open.text());
    let sharjah = w
        .get(&o.till, "/checkout/payment?brand=habibi&outlet=sharjah")
        .await;
    assert!(sharjah.text().contains("pay-online"));
    // The brand with no other kitchen is closed for everyone.
    let zaatar = w.get(&o.till, "/checkout/payment?brand=zaatar").await;
    assert!(
        zaatar.text().contains("kitchen-closed"),
        "{}",
        zaatar.text()
    );
}
