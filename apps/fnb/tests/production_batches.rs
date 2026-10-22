//! BL-FNB-11 (#375), 06 F&B §4.4 *recall* row, §7.1 *Production batch*,
//! `STK.ACL.02`, `STK.ACL.05`: a central kitchen batch records the ingredient
//! lots it used when it starts and cannot start with a lot on hold; its lot
//! being held puts it `on_hold`, releasing the lot makes it `produced` again,
//! and discarding the lot discards it with exactly one wastage movement; a
//! temperature hold on a location leaves it `produced`. The modules are the
//! real wl-inventory (lots, holds, wastage), wl-sales, wl-payments and
//! wl-delivery.

mod support;

use fnb::BatchStatus;
use renox::prelude::*;
use wl_inventory::{LotStatus, StockHold};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};

use support::stock::{
    finish, hold, kitchen, lot, lot_now, made_lot, plan, reload, start, wastage_of,
};
use support::{World, boot};

/// The hold lines of the hold `hold`, as stored now.
async fn lines_of(w: &World, hold: &StockHold) -> Vec<wl_inventory::StockHoldLine> {
    wl_inventory::stock_hold::lines_of(w.db(), hold.id)
        .await
        .unwrap()
}

/// AC1: given tomato lot T7 on hold, `start` of batch B1 picking T7 is
/// refused, naming the lot, and the batch stays planned.
#[renox::test]
async fn starting_a_batch_with_a_lot_on_hold_is_refused() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t7 = lot(&w, &k.tomato, "T7", &k.central, 30_000, 3).await;
    hold(&w, &t7, "recall").await;
    assert_eq!(lot_now(&w, &t7).await.status, LotStatus::OnHold);
    let b1 = plan(&w, &k).await;

    let refused = start(&w, &k.chef, &k, &b1, &t7).await;
    refused.assert_status(422);
    assert!(refused.text().contains("T7"), "{}", refused.text());
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert_eq!(reload(&w, &b1).await.status, BatchStatus::Planned);
    assert!(b1.inputs(w.db()).await.unwrap().is_empty());
}

/// AC1: the head chef holds the permission; a cook and a stranger do not.
#[renox::test]
async fn only_a_holder_of_production_manage_plans_and_starts_a_batch() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let b = plan(&w, &k).await;

    start(&w, &w.cook, &k, &b, &t5).await.assert_status(403);
    start(&w, &w.nobody, &k, &b, &t5).await.assert_status(403);
    w.post(&w.cook, "/batches", &[("recipe", "1")])
        .await
        .assert_status(403);
    w.get(&w.nobody, "/batches").await.assert_status(403);
    w.get(&k.manager, "/batches").await.assert_ok();
    assert_eq!(reload(&w, &b).await.status, BatchStatus::Planned);
}

/// AC2: batch B2 started with T5 records T5 in its genealogy; the earliest
/// expiry is proposed when the chef picks nothing.
#[renox::test]
async fn a_started_batch_records_the_ingredient_lot_it_used() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let _later = lot(&w, &k.tomato, "T9", &k.central, 40_000, 9).await;
    let b2 = plan(&w, &k).await;

    start(&w, &k.chef, &k, &b2, &t5).await.assert_status(303);
    let started = reload(&w, &b2).await;
    assert_eq!(started.status, BatchStatus::InProduction);
    let inputs = started.inputs(w.db()).await.unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].lot_id, t5.id);
    assert_eq!(inputs[0].quantity_milli, 12_000);

    // Nothing picked: the earliest expiry (T5) is taken.
    let b3 = plan(&w, &k).await;
    w.post(&k.chef, &format!("/batches/{}/start", b3.id), &[("x", "")])
        .await
        .assert_status(303);
    let proposed = reload(&w, &b3).await.inputs(w.db()).await.unwrap();
    assert_eq!(proposed[0].lot_id, t5.id);
}

/// AC4: given B2 made with T5 (expiry in 6 days), `finish` with an expiry
/// later than that is refused; with an earlier one it creates the lot with the
/// expiry entered, and the ingredient lot is consumed.
#[renox::test]
async fn finishing_a_batch_refuses_an_expiry_later_than_its_ingredients() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let b2 = plan(&w, &k).await;
    start(&w, &k.chef, &k, &b2, &t5).await.assert_status(303);

    finish(&w, &k.chef, &b2, "10", 8).await.assert_status(422);
    finish(&w, &k.chef, &b2, "0", 4).await.assert_status(422);
    assert_eq!(reload(&w, &b2).await.status, BatchStatus::InProduction);
    assert!(reload(&w, &b2).await.lot_id.is_none());

    finish(&w, &k.chef, &b2, "10", 4).await.assert_status(303);
    let done = reload(&w, &b2).await;
    assert_eq!(done.status, BatchStatus::Produced);
    assert_eq!(done.actual_yield_milli, Some(10_000));
    let l20 = made_lot(&w, &done).await;
    assert_eq!(l20.expiry_date, done.expiry_date);
    assert_eq!(l20.item_id, k.sauce.id);
    assert_eq!(l20.status, LotStatus::Released);
    let on_hand = wl_inventory::movement::on_hand(w.db(), k.sauce.id, k.central.id)
        .await
        .unwrap();
    assert_eq!(on_hand, 10_000);
    let tomatoes = wl_inventory::movement::on_hand(w.db(), k.tomato.id, k.central.id)
        .await
        .unwrap();
    assert_eq!(tomatoes, 28_000, "12 kg of T5 were consumed");
}

/// AC3: when B2's lot L20 is held by a recall, B2 is `on_hold`, its stock is
/// blocked and the chef is notified; once the lot is released it is
/// `produced`.
#[renox::test]
async fn a_held_lot_puts_its_batch_on_hold_and_releasing_it_brings_it_back() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let b2 = plan(&w, &k).await;
    start(&w, &k.chef, &k, &b2, &t5).await.assert_status(303);
    finish(&w, &k.chef, &b2, "10", 4).await.assert_status(303);
    let l20 = made_lot(&w, &b2).await;

    let placed = hold(&w, &l20, "quality").await;
    let held = reload(&w, &b2).await;
    assert_eq!(held.status, BatchStatus::OnHold);
    assert_eq!(held.hold_reason.as_deref(), Some("quality"));
    assert_eq!(lot_now(&w, &l20).await.status, LotStatus::OnHold);
    assert!(
        w.notices
            .users_of_type("fnb.staff_notice")
            .contains(&k.chef.id),
        "the chef is told"
    );

    // The chef releases the line of the quality hold alone (not a listed
    // reason), the lot is released and the batch is produced again.
    let line = lines_of(&w, &placed).await.remove(0);
    w.post(&k.chef, &format!("/stock-hold-lines/{}/release", line.id), &[])
        .await
        .assert_ok();
    assert_eq!(lot_now(&w, &l20).await.status, LotStatus::Released);
    let back = reload(&w, &b2).await;
    assert_eq!(back.status, BatchStatus::Produced);
    assert!(back.hold_reason.is_none());
}

/// AC5: on `LotDiscarded` for L20 (8 kg left) B2 is `discarded` and exactly
/// one wastage movement of 8 kg exists; the batch's entry links the lot's
/// discard and posts nothing itself.
#[renox::test]
async fn discarding_the_lot_discards_the_batch_with_one_wastage_movement() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let b2 = plan(&w, &k).await;
    start(&w, &k.chef, &k, &b2, &t5).await.assert_status(303);
    finish(&w, &k.chef, &b2, "8", 4).await.assert_status(303);
    let l20 = made_lot(&w, &b2).await;
    let placed = hold(&w, &l20, "recall").await;
    assert_eq!(reload(&w, &b2).await.status, BatchStatus::OnHold);

    // A hold line is the only way out of a hold: the chef discards it.
    let line = lines_of(&w, &placed).await.remove(0);
    w.post(&k.chef, &format!("/stock-hold-lines/{}/discard", line.id), &[])
        .await
        .assert_ok();

    let gone = reload(&w, &b2).await;
    assert_eq!(gone.status, BatchStatus::Discarded);
    let wasted = wastage_of(&w, &l20).await;
    assert_eq!(wasted.len(), 1, "exactly one wastage movement");
    assert_eq!(wasted[0].quantity_milli, 8_000);
    let entry = fnb::BatchWastage::of_batch(w.db(), b2.id)
        .await
        .unwrap()
        .expect("the batch has its wastage entry");
    assert_eq!(entry.posted_by, "lot");
    assert_eq!(entry.quantity_milli, 8_000);
    assert_eq!(entry.value, 12_000, "8 kg at AED 15.00 the kilogram");
}

/// A batch on hold cannot be discarded from its own page: its hold line is
/// decided, so the hold's approvals apply.
#[renox::test]
async fn a_batch_on_hold_is_not_discarded_from_its_page() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let b2 = plan(&w, &k).await;
    start(&w, &k.chef, &k, &b2, &t5).await.assert_status(303);
    finish(&w, &k.chef, &b2, "8", 4).await.assert_status(303);
    hold(&w, &made_lot(&w, &b2).await, "recall").await;

    let refused = w
        .post(
            &k.chef,
            &format!("/batches/{}/discard", b2.id),
            &[("reason", "spoiled")],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("hold line"), "{}", refused.text());
    assert_eq!(reload(&w, &b2).await.status, BatchStatus::OnHold);
}

/// AC6: a temperature hold on the stock at the kitchen's location blocks that
/// stock only: the batch stays `produced`, its lot is not on hold.
#[renox::test]
async fn a_hold_on_the_location_leaves_the_batch_produced() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let b2 = plan(&w, &k).await;
    start(&w, &k.chef, &k, &b2, &t5).await.assert_status(303);
    finish(&w, &k.chef, &b2, "10", 4).await.assert_status(303);
    let l20 = made_lot(&w, &b2).await;

    wl_inventory::hold_service::place(
        w.state(),
        wl_inventory::hold_service::NewHold {
            reason: "temperature".to_owned(),
            reference: Subject::new("test.reading", 1),
            scope: wl_inventory::hold_service::NewScope::Location {
                location_id: k.central.id,
                item_ids: vec![k.sauce.id],
            },
            by: None,
        },
    )
    .await
    .expect("the temperature hold is placed");

    assert_eq!(reload(&w, &b2).await.status, BatchStatus::Produced);
    assert_eq!(lot_now(&w, &l20).await.status, LotStatus::Released);
}

/// `finish` with an ingredient lot on hold at that moment makes the output lot
/// on hold with the same reason and the batch runs `hold` at once.
#[renox::test]
async fn finishing_with_a_held_ingredient_holds_the_output_at_once() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let b2 = plan(&w, &k).await;
    start(&w, &k.chef, &k, &b2, &t5).await.assert_status(303);
    hold(&w, &t5, "recall").await;

    finish(&w, &k.chef, &b2, "10", 4).await.assert_status(303);
    let done = reload(&w, &b2).await;
    assert_eq!(done.status, BatchStatus::OnHold);
    assert_eq!(done.hold_reason.as_deref(), Some("recall"));
    assert_eq!(made_lot(&w, &done).await.status, LotStatus::OnHold);
}

/// A discard from `produced` posts the wastage movement once; above AED 200 a
/// second holder of `stock.approve` approves first, never the chef.
#[renox::test]
async fn a_discard_above_the_limit_waits_for_the_kitchen_manager_and_posts_once() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 80_000, 6).await;
    // Two batches: 10 kg (AED 150) and 20 kg (AED 300) of sauce.
    let small = plan(&w, &k).await;
    start(&w, &k.chef, &k, &small, &t5).await.assert_status(303);
    finish(&w, &k.chef, &small, "10", 4).await.assert_status(303);
    let big = plan(&w, &k).await;
    w.post(
        &k.chef,
        &format!("/batches/{}/start", big.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    finish(&w, &k.chef, &big, "20", 4).await.assert_status(303);

    // AED 150: the chef discards alone; one wastage movement.
    w.post(
        &k.chef,
        &format!("/batches/{}/discard", small.id),
        &[("reason", "Spoiled in the blast chiller")],
    )
    .await
    .assert_status(303);
    assert_eq!(reload(&w, &small).await.status, BatchStatus::Discarded);
    let small_lot = made_lot(&w, &small).await;
    assert_eq!(wastage_of(&w, &small_lot).await.len(), 1);
    let entry = fnb::BatchWastage::of_batch(w.db(), small.id)
        .await
        .unwrap()
        .expect("the entry");
    assert_eq!(entry.posted_by, "batch");
    assert_eq!(entry.quantity_milli, 10_000);

    // AED 300: waits; the chef cannot approve it, the kitchen manager can.
    w.post(
        &k.chef,
        &format!("/batches/{}/discard", big.id),
        &[("reason", "Failed the quality check")],
    )
    .await
    .assert_status(303);
    assert_eq!(reload(&w, &big).await.status, BatchStatus::Produced);
    let big_lot = made_lot(&w, &big).await;
    assert!(wastage_of(&w, &big_lot).await.is_empty(), "nothing posted yet");
    let request = ApprovalRequest::pending_about(w.db(), &big.subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits");
    assert!(approvals::approve(w.state(), request.id, &k.chef, "").await.is_err());
    approvals::approve(w.state(), request.id, &k.manager, "")
        .await
        .expect("the manager approves");

    assert_eq!(reload(&w, &big).await.status, BatchStatus::Discarded);
    let wasted = wastage_of(&w, &big_lot).await;
    assert_eq!(wasted.len(), 1);
    assert_eq!(wasted[0].quantity_milli, 20_000);
}

/// The planned batch can be cancelled; one that started cannot.
#[renox::test]
async fn a_planned_batch_is_cancelled_and_a_started_one_is_not() {
    let w = boot().await;
    let k = kitchen(&w).await;
    let t5 = lot(&w, &k.tomato, "T5", &k.central, 40_000, 6).await;
    let planned = plan(&w, &k).await;
    w.post(&k.chef, &format!("/batches/{}/cancel", planned.id), &[("x", "")])
        .await
        .assert_status(303);
    assert!(fnb::ProductionBatch::find(w.db(), planned.id)
        .await
        .unwrap()
        .is_none());

    let started = plan(&w, &k).await;
    start(&w, &k.chef, &k, &started, &t5).await.assert_status(303);
    w.post(&k.chef, &format!("/batches/{}/cancel", started.id), &[("x", "")])
        .await
        .assert_status(422);
}
