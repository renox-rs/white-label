//! Deciding the lines of a stock hold, closing it and cancelling one placed in
//! error (wl-inventory §4 *Stock hold*, §4 *Lot*, `STK.ACL.02`, `STK.ACL.05`,
//! issue #231): every point of "Done when" through the module's routes, the
//! kernel's approvals and the listeners that settle the lots and the hold.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::movement::{MovementType, StockMovement};
use wl_inventory::{
    DraftLine, HoldLineStatus, HoldStatus, Item, Location, LocationType, Lot, LotStatus,
    ReturnStatus, StockHold, SupplierReturn, WlInventory,
};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::StockHoldClosed;

use support::{Booted, SUPPLIER, boot_with};

/// A 06 outlet: wastage above AED 200 needs a second person.
async fn boot_outlet() -> Booted {
    boot_with(WlInventory::new().wastage_approval_fils(20_000)).await
}

/// Chicken, costing AED 30 a kg.
async fn chicken(t: &Booted) -> Item {
    let mut item = Item::new("CHICKEN", "CHICKEN", "kg");
    item.average_cost = 3_000;
    item.save(t.db()).await.unwrap();
    item
}

/// Places a hold on the stock at `location` as `by`; the hold placed.
async fn hold_location(t: &Booted, by: &User, location: &Location, reason: &str) -> StockHold {
    let id = location.id.to_string();
    t.hold_as(
        by,
        &[
            ("reason", reason),
            ("reference", "ticketing.ticket:9"),
            ("location", id.as_str()),
        ],
    )
    .await
    .assert_ok();
    t.all_holds().await.pop().expect("a hold")
}

/// Places a hold on whole `lots` as `by`; the hold placed.
async fn hold_lots(t: &Booted, by: &User, lots: &[&Lot], reason: &str) -> StockHold {
    let ids = lots
        .iter()
        .map(|lot| lot.id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    t.hold_as(
        by,
        &[
            ("reason", reason),
            ("reference", "ticketing.ticket:9"),
            ("lots", ids.as_str()),
        ],
    )
    .await
    .assert_ok();
    t.all_holds().await.pop().expect("a hold")
}

/// The wastage movements of `item`.
async fn wastage_of(t: &Booted, item: &Item) -> Vec<StockMovement> {
    t.movements_of(item)
        .await
        .into_iter()
        .filter(|moved| moved.movement_type == MovementType::Wastage)
        .collect()
}

/// The approval requests waiting about `line`.
async fn waiting_on(t: &Booted, line: &wl_inventory::StockHoldLine) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &line.subject())
        .await
        .unwrap()
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Done when 1: a *temperature* hold at a 06 outlet with a line of chicken
/// 5 kg (AED 150): the chef's `discard` posts the wastage movement at once
/// (below AED 200) and, being the last line, the hold is `closed` with
/// `StockHoldClosed`; the cost goes to the hold.
#[renox::test]
async fn a_discard_below_the_value_posts_at_once_and_closes_the_hold() {
    let t = boot_outlet().await;
    let item = chicken(&t).await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    t.stock_in(&item, &fridge, None, 5_000, 3_000).await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let hold = hold_location(&t, &holder, &fridge, "temperature").await;
    let line = t.hold_lines(&hold).await.remove(0);
    assert_eq!(line.status, HoldLineStatus::Undecided);

    t.decide_as(&chef, &line, "discard").await.assert_ok();

    let wasted = wastage_of(&t, &item).await;
    assert_eq!(wasted.len(), 1);
    assert_eq!(wasted[0].quantity_milli, 5_000);
    assert_eq!(wasted[0].value, 15_000);
    assert_eq!(wasted[0].from_location_id, Some(fridge.id));
    assert_eq!(wasted[0].reason.as_deref(), Some("temperature"));
    assert_eq!(wasted[0].user_id, Some(chef.id));
    assert_eq!(
        wl_inventory::balance::on_hand_at(t.db(), item.id, fridge.id, None)
            .await
            .unwrap(),
        0
    );
    let decided = t.hold_lines(&hold).await.remove(0);
    assert_eq!(decided.status, HoldLineStatus::Discarded);
    assert_eq!(decided.outcome.as_deref(), Some("discard"));
    assert_eq!(decided.decided_by, Some(chef.id));
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    let closed: Vec<StockHoldClosed> = t.closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].hold, hold.subject());
    assert_eq!(closed[0].outcomes.len(), 1);
    assert_eq!(closed[0].outcomes[0].outcome, "discard");
    assert_eq!(closed[0].outcomes[0].quantity_milli, 5_000);
    let costs = t.costs.posted();
    assert_eq!(costs.len(), 1);
    assert_eq!(costs[0].amount_aed, 15_000);
    assert_eq!(costs[0].cost_object, hold.subject());
    assert_eq!(costs[0].cost_code.as_deref(), Some("wastage"));
}

/// Done when 2: a line worth AED 300 waits for approval by someone other than
/// the chef: no wastage movement is posted and the hold stays `open` until it
/// is approved.
#[renox::test]
async fn a_discard_above_the_value_waits_for_a_second_person() {
    let t = boot_outlet().await;
    let item = chicken(&t).await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    t.stock_in(&item, &fridge, None, 10_000, 3_000).await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let manager = t.releaser("Mia").await;
    let hold = hold_location(&t, &holder, &fridge, "temperature").await;
    let line = t.hold_lines(&hold).await.remove(0);

    t.decide_as(&chef, &line, "discard").await.assert_ok();

    let requests = waiting_on(&t, &line).await;
    assert_eq!(requests.len(), 1);
    assert!(wastage_of(&t, &item).await.is_empty());
    assert_eq!(
        t.hold_lines(&hold).await[0].status,
        HoldLineStatus::Undecided
    );
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);
    // The quantity stays held while it waits.
    assert_eq!(t.available(&item, &fridge, None).await, 0);

    let refused = approvals::approve(t.state(), requests[0].id, &chef, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert!(wastage_of(&t, &item).await.is_empty());

    approvals::approve(t.state(), requests[0].id, &manager, "ok")
        .await
        .unwrap();
    let wasted = wastage_of(&t, &item).await;
    assert_eq!(wasted.len(), 1);
    assert_eq!(wasted[0].quantity_milli, 10_000);
    // The decider is the chef, who asked, not the one who approved.
    assert_eq!(wasted[0].user_id, Some(chef.id));
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.closed.all().len(), 1);
    assert_eq!(t.costs.posted()[0].amount_aed, 30_000);
}

/// A rejected approval leaves the line undecided and the hold open.
#[renox::test]
async fn a_rejected_approval_leaves_the_line_undecided() {
    let t = boot_outlet().await;
    let item = chicken(&t).await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    t.stock_in(&item, &fridge, None, 10_000, 3_000).await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let manager = t.releaser("Mia").await;
    let hold = hold_location(&t, &holder, &fridge, "temperature").await;
    let line = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&chef, &line, "discard").await.assert_ok();
    let request = waiting_on(&t, &line).await.remove(0);

    approvals::reject(t.state(), request.id, &manager, "keep it for the vet")
        .await
        .unwrap();

    assert!(waiting_on(&t, &line).await.is_empty());
    assert_eq!(
        t.hold_lines(&hold).await[0].status,
        HoldLineStatus::Undecided
    );
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);
    assert!(wastage_of(&t, &item).await.is_empty());
    assert_eq!(t.available(&item, &fridge, None).await, 0);
    // It can be decided again, now by `release` (not a listed reason).
    t.decide_as(&chef, &t.hold_lines(&hold).await[0], "release")
        .await
        .assert_ok();
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.available(&item, &fridge, None).await, 10_000);
}

/// Done when 3: a `recall` line decided `release` by user A waits for user
/// B; A cannot approve. The lot is released only after B.
#[renox::test]
async fn releasing_a_recall_line_needs_a_second_person() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&item, &branch, Some(&lot), 12_000, 1_000).await;
    let holder = t.holder("Hal").await;
    let ann = t.releaser("Ann").await;
    let ben = t.releaser("Ben").await;
    let hold = hold_lots(&t, &holder, &[&lot], "recall").await;
    let line = t.hold_lines(&hold).await.remove(0);

    t.decide_as(&ann, &line, "release").await.assert_ok();

    let requests = waiting_on(&t, &line).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        t.hold_lines(&hold).await[0].status,
        HoldLineStatus::Undecided
    );
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);
    let refused = approvals::approve(t.state(), requests[0].id, &ann, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);

    approvals::approve(t.state(), requests[0].id, &ben, "checked")
        .await
        .unwrap();
    assert_eq!(
        t.hold_lines(&hold).await[0].status,
        HoldLineStatus::Released
    );
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Released);
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.available(&item, &branch, Some(&lot)).await, 12_000);
}

/// The reasons that need a second person are a setting.
#[renox::test]
async fn the_reasons_that_need_a_second_person_are_a_setting() {
    let t = boot_with(
        WlInventory::new()
            .wastage_approval_fils(20_000)
            .hold_second_person_reasons(&["quality"]),
    )
    .await;
    let item = t.item("GAUZE").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&item, &branch, None, 4_000, 100).await;
    let holder = t.holder("Hal").await;
    let ann = t.releaser("Ann").await;
    let hold = hold_location(&t, &holder, &branch, "quality").await;
    let line = t.hold_lines(&hold).await.remove(0);

    t.decide_as(&ann, &line, "release").await.assert_ok();
    assert_eq!(waiting_on(&t, &line).await.len(), 1);
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);
}

/// Done when 4: a whole-lot *quality* hold decided `release` on all lines,
/// with no other hold on the lot, releases the lot; one holder is enough.
#[renox::test]
async fn the_lot_is_released_when_all_its_lines_are_released() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let store_a = t.location("STORE-A", None).await;
    let store_b = t.location("STORE-B", None).await;
    t.stock_in(&item, &store_a, Some(&lot), 40_000, 1_000).await;
    t.stock_in(&item, &store_b, Some(&lot), 10_000, 1_000).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_lots(&t, &holder, &[&lot], "quality").await;
    let lines = t.hold_lines(&hold).await;
    assert_eq!(lines.len(), 2);

    t.decide_as(&rae, &lines[0], "release").await.assert_ok();
    assert!(waiting_on(&t, &lines[0]).await.is_empty());
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);
    assert!(t.released.all().is_empty());

    t.decide_as(&rae, &lines[1], "release").await.assert_ok();
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Released);
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.available(&item, &store_a, Some(&lot)).await, 40_000);
    assert_eq!(t.available(&item, &store_b, Some(&lot)).await, 10_000);
    let released = t.released.all();
    assert_eq!(released.len(), 1);
    assert_eq!(
        released[0].lot,
        wl_kernel::Subject::new("inventory.lot", lot.id)
    );
    let closed = t.closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].outcomes.len(), 2);
    assert!(closed[0].outcomes.iter().all(|o| o.outcome == "release"));
}

/// A lot under a second open hold stays `on_hold` until that one is decided
/// too.
#[renox::test]
async fn a_lot_under_another_open_hold_stays_on_hold() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let store = t.location("STORE-A", None).await;
    t.stock_in(&item, &store, Some(&lot), 40_000, 1_000).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let first = hold_lots(&t, &holder, &[&lot], "quality").await;
    let second = hold_lots(&t, &holder, &[&lot], "complaint").await;
    assert_ne!(first.id, second.id);

    t.decide_as(&rae, &t.hold_lines(&first).await[0], "release")
        .await
        .assert_ok();
    assert_eq!(t.hold_now(&first).await.status, HoldStatus::Closed);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 0);

    t.decide_as(&rae, &t.hold_lines(&second).await[0], "release")
        .await
        .assert_ok();
    assert_eq!(t.hold_now(&second).await.status, HoldStatus::Closed);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Released);
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 40_000);
}

/// A lot whose lines are all discarded has nothing left: the lot is
/// discarded (`rejected`) and `LotDiscarded` follows.
#[renox::test]
async fn a_lot_with_nothing_left_is_discarded() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let store = t.location("STORE-A", None).await;
    t.stock_in(&item, &store, Some(&lot), 3_000, 1_000).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_lots(&t, &holder, &[&lot], "quality").await;

    t.decide_as(&rae, &t.hold_lines(&hold).await[0], "discard")
        .await
        .assert_ok();

    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Rejected);
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    let discarded = t.discarded.all();
    assert_eq!(discarded.len(), 1);
    assert_eq!(discarded[0].reason, "quality");
    // The line's wastage took all 3; the lot's own discard found nothing, and
    // the event still reports what was discarded.
    assert_eq!(discarded[0].quantity_milli, 3_000);
    assert_eq!(wastage_of(&t, &item).await.len(), 1);
}

/// Regression: `LotDiscarded` reports everything of the lot that went to
/// wastage, also when it left through the lines of the hold at two
/// locations (it was 0 before, as the lot's own discard found nothing).
#[renox::test]
async fn the_lot_discarded_event_reports_what_every_line_discarded() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let store_a = t.location("STORE-A", None).await;
    let store_b = t.location("STORE-B", None).await;
    t.stock_in(&item, &store_a, Some(&lot), 3_000, 1_000).await;
    t.stock_in(&item, &store_b, Some(&lot), 2_000, 1_000).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_lots(&t, &holder, &[&lot], "quality").await;
    for line in t.hold_lines(&hold).await {
        t.decide_as(&rae, &line, "discard").await.assert_ok();
    }

    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Rejected);
    let discarded = t.discarded.all();
    assert_eq!(discarded.len(), 1);
    assert_eq!(discarded[0].quantity_milli, 5_000);
}

/// Done when 5: `return_to_supplier` moves the quantity to quarantine and a
/// draft return for the lot's supplier lists it; a second line of the same
/// supplier is added to the same draft.
#[renox::test]
async fn returning_to_the_supplier_moves_the_stock_to_quarantine_and_onto_a_draft() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let branch_a = t.named_location("BRANCH-A", "Branch A").await;
    let branch_b = t.named_location("BRANCH-B", "Branch B").await;
    let (order, po_line) = t.order(&branch_a, &item, 20_000, 500).await;
    let receipt = t
        .receipt(
            &order,
            &branch_a,
            vec![DraftLine::on(po_line.id, 20_000, 20_000).lot("V-1", day(2027, 6, 30))],
        )
        .await;
    let keeper = t.storekeeper("Amal").await;
    t.post_as(&keeper, &receipt).await.assert_ok();
    let lot = Lot::query()
        .where_(Lot::ITEM_ID.eq(item.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("the receipt made a lot");
    t.stock_in(&item, &branch_b, Some(&lot), 5_000, 500).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_lots(&t, &holder, &[&lot], "quality").await;
    let lines = t.hold_lines(&hold).await;
    assert_eq!(lines.len(), 2);

    t.decide_as(&rae, &lines[0], "return-to-supplier")
        .await
        .assert_ok();
    t.decide_as(&rae, &lines[1], "return-to-supplier")
        .await
        .assert_ok();

    let quarantine = Location::query()
        .where_(Location::LOCATION_TYPE.eq(LocationType::Quarantine))
        .first(t.db())
        .await
        .unwrap()
        .expect("the quarantine location");
    let moved: Vec<StockMovement> = t
        .movements_of(&item)
        .await
        .into_iter()
        .filter(|m| m.to_location_id == Some(quarantine.id))
        .collect();
    assert_eq!(moved.len(), 2);
    assert_eq!(moved[0].from_location_id, Some(branch_a.id));
    assert_eq!(moved[0].quantity_milli, 20_000);
    assert_eq!(moved[1].from_location_id, Some(branch_b.id));
    assert_eq!(moved[1].quantity_milli, 5_000);
    let drafts = SupplierReturn::query().get(t.db()).await.unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].status, ReturnStatus::Draft);
    assert_eq!(drafts[0].supplier_party_id, SUPPLIER.0);
    assert_eq!(drafts[0].location_id, Some(quarantine.id));
    let listed = wl_inventory::supplier_return::lines_of(t.db(), drafts[0].id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].lot_id, Some(lot.id));
    assert_eq!(listed[0].quantity_milli, 20_000);
    assert_eq!(listed[1].quantity_milli, 5_000);
    let decided = t.hold_lines(&hold).await;
    assert!(
        decided
            .iter()
            .all(|l| l.status == HoldLineStatus::ReturnedToSupplier
                && l.supplier_return_id == Some(drafts[0].id))
    );
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    // The stock waits in quarantine, set aside for the return; the lot stays
    // held until the return is dispatched.
    assert_eq!(t.available(&item, &quarantine, Some(&lot)).await, 0);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    assert_eq!(wastage_of(&t, &item).await.len(), 0);
}

/// A return to the supplier that is cancelled before it is dispatched gives
/// the lot back: it was left `on_hold` by the decision, with no hold open.
/// It goes to `quarantine` (the goods sit there, nobody released them).
#[renox::test]
async fn cancelling_the_return_gives_the_held_lot_back() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let branch = t.named_location("BRANCH-A", "Branch A").await;
    let (order, po_line) = t.order(&branch, &item, 20_000, 500).await;
    let receipt = t
        .receipt(
            &order,
            &branch,
            vec![DraftLine::on(po_line.id, 20_000, 20_000).lot("V-1", day(2027, 6, 30))],
        )
        .await;
    let keeper = t.storekeeper("Amal").await;
    t.post_as(&keeper, &receipt).await.assert_ok();
    let lot = Lot::query()
        .where_(Lot::ITEM_ID.eq(item.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("the receipt made a lot");
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_lots(&t, &holder, &[&lot], "quality").await;
    let line = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&rae, &line, "return-to-supplier")
        .await
        .assert_ok();
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    let draft = SupplierReturn::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();

    let buyer = t.procurement("Pat").await;
    t.cancel_as(
        &buyer,
        &draft,
        "The supplier took the goods back on site",
        None,
    )
    .await
    .assert_ok();

    let cancelled = SupplierReturn::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(cancelled.status, ReturnStatus::Cancelled);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Quarantine);
}

/// A lot with no supplier (a produced lot) cannot go back to one.
#[renox::test]
async fn a_lot_without_a_supplier_is_not_returned() {
    let t = boot_outlet().await;
    let item = t.tracked_item("SAUCE").await;
    let lot = t.lot(&item, "P-1", LotStatus::Released).await;
    let kitchen = t.named_location("KITCHEN", "Kitchen").await;
    t.stock_in(&item, &kitchen, Some(&lot), 4_000, 500).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_lots(&t, &holder, &[&lot], "quality").await;
    let line = t.hold_lines(&hold).await.remove(0);

    let refused = t.decide_as(&rae, &line, "return-to-supplier").await;

    refused.assert_status(422);
    assert!(refused.text().contains("supplier"), "{}", refused.text());
    assert_eq!(
        t.hold_lines(&hold).await[0].status,
        HoldLineStatus::Undecided
    );
    assert!(
        SupplierReturn::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
}

/// Releasing the stock of an `expired` lot is not offered: only a discard.
/// The lot keeps its status.
#[renox::test]
async fn an_expired_lot_can_only_be_discarded_or_returned() {
    let t = boot_outlet().await;
    let item = t.tracked_item("YOGHURT").await;
    let lot = t.lot(&item, "Y-1", LotStatus::Expired).await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    t.stock_in(&item, &fridge, Some(&lot), 4_000, 500).await;
    let holder = t.holder("Hal").await;
    let chef = t.releaser("Chef").await;
    let hold = hold_location(&t, &holder, &fridge, "temperature").await;
    let line = t.hold_lines(&hold).await.remove(0);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Expired);

    let refused = t.decide_as(&chef, &line, "release").await;
    refused.assert_status(422);
    assert!(refused.text().contains("expired"), "{}", refused.text());
    assert_eq!(
        t.hold_lines(&hold).await[0].status,
        HoldLineStatus::Undecided
    );

    t.decide_as(&chef, &line, "discard").await.assert_ok();
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Expired);
}

/// Only a holder of `lots.release` decides a line; a line is decided once.
#[renox::test]
async fn deciding_needs_the_permission_and_happens_once() {
    let t = boot_outlet().await;
    let item = t.item("GAUZE").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&item, &branch, None, 4_000, 100).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_location(&t, &holder, &branch, "other").await;
    let line = t.hold_lines(&hold).await.remove(0);

    t.decide_as(&holder, &line, "release")
        .await
        .assert_forbidden();
    assert_eq!(
        t.hold_lines(&hold).await[0].status,
        HoldLineStatus::Undecided
    );

    t.decide_as(&rae, &line, "release").await.assert_ok();
    t.decide_as(&rae, &line, "discard").await.assert_status(422);
    assert!(wastage_of(&t, &item).await.is_empty());
    assert_eq!(t.available(&item, &branch, None).await, 4_000);
}

/// A line of a whole-lot hold that was in transit follows the lot to the
/// destination of the transfer and is decided there.
#[renox::test]
async fn a_line_in_transit_follows_the_lot_to_its_destination() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let store_a = t.location("STORE-A", None).await;
    let store_b = t.location("STORE-B", None).await;
    t.stock_in(&item, &store_a, Some(&lot), 50_000, 1_000).await;
    let mover = t.mover("Mo").await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let transfer = t
        .transfer(
            &store_a,
            &store_b,
            vec![wl_inventory::transfer::NewTransferLine::of(item.id, 10_000).lot(lot.id)],
        )
        .await;
    t.send_as(&mover, &transfer).await.assert_ok();
    let hold = hold_lots(&t, &holder, &[&lot], "quality").await;
    let transit = t.transfer_now(&transfer).await.transit_location_id;
    let before: Vec<_> = t
        .hold_lines(&hold)
        .await
        .iter()
        .map(|l| (l.location_id, l.quantity_milli))
        .collect();
    assert_eq!(
        before,
        vec![(store_a.id, 40_000), (transit.unwrap(), 10_000)]
    );

    t.receive_as(&mover, &transfer, true).await.assert_ok();

    let after: Vec<_> = t
        .hold_lines(&hold)
        .await
        .iter()
        .map(|l| (l.location_id, l.quantity_milli))
        .collect();
    assert_eq!(after, vec![(store_a.id, 40_000), (store_b.id, 10_000)]);
    for line in t.hold_lines(&hold).await {
        t.decide_as(&rae, &line, "discard").await.assert_ok();
    }
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Rejected);
    assert_eq!(wastage_of(&t, &item).await.len(), 2);
}

/// Done when 6: a whole-lot hold on a `released` lot with no decided lines is
/// cancelled with *placed in error*: the stock is available and the lot is
/// `released` again (`unhold`).
#[renox::test]
async fn cancelling_a_hold_gives_a_released_lot_back() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let store = t.location("STORE-A", None).await;
    t.stock_in(&item, &store, Some(&lot), 40_000, 1_000).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_lots(&t, &holder, &[&lot], "quality").await;
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 0);

    t.cancel_hold_as(&rae, &hold, "").await.assert_ok();

    let cancelled = t.hold_now(&hold).await;
    assert_eq!(cancelled.status, HoldStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("placed in error"));
    assert_eq!(cancelled.cancelled_by, Some(rae.id));
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Released);
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 40_000);
    // The lines of a cancelled hold cannot be decided any more.
    t.decide_as(&rae, &t.hold_lines(&hold).await[0], "discard")
        .await
        .assert_status(422);
    assert!(wastage_of(&t, &item).await.is_empty());
}

/// Done when 7: the same for a lot that was in `quarantine` before the hold.
#[renox::test]
async fn cancelling_a_hold_gives_a_quarantined_lot_back_to_quarantine() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "Q-1", LotStatus::Quarantine).await;
    let store = t.location("STORE-A", None).await;
    t.stock_in(&item, &store, Some(&lot), 40_000, 1_000).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let hold = hold_lots(&t, &holder, &[&lot], "complaint").await;
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);

    t.cancel_hold_as(&rae, &hold, "wrong lot").await.assert_ok();

    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Quarantine);
    assert_eq!(
        t.hold_now(&hold).await.cancel_reason.as_deref(),
        Some("wrong lot")
    );
}

/// A lot also under another open hold stays `on_hold` when one hold is
/// cancelled.
#[renox::test]
async fn cancelling_one_hold_keeps_a_lot_that_another_hold_has() {
    let t = boot_outlet().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let store = t.location("STORE-A", None).await;
    t.stock_in(&item, &store, Some(&lot), 40_000, 1_000).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;
    let first = hold_lots(&t, &holder, &[&lot], "quality").await;
    let _second = hold_lots(&t, &holder, &[&lot], "complaint").await;

    t.cancel_hold_as(&rae, &first, "").await.assert_ok();

    assert_eq!(t.hold_now(&first).await.status, HoldStatus::Cancelled);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 0);
}

/// A hold with a decided line, a recall's hold, and a user without
/// `lots.release` cannot cancel.
#[renox::test]
async fn a_hold_is_not_cancelled_after_a_decision_nor_for_a_recall() {
    let t = boot_outlet().await;
    let item = t.item("GAUZE").await;
    let other = t.item("MASK").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&item, &branch, None, 4_000, 100).await;
    t.stock_in(&other, &branch, None, 2_000, 100).await;
    let holder = t.holder("Hal").await;
    let rae = t.releaser("Rae").await;

    let hold = hold_location(&t, &holder, &branch, "quality").await;
    t.cancel_hold_as(&holder, &hold, "oops")
        .await
        .assert_forbidden();
    let lines = t.hold_lines(&hold).await;
    assert_eq!(lines.len(), 2);
    t.decide_as(&rae, &lines[0], "release").await.assert_ok();
    let refused = t.cancel_hold_as(&rae, &hold, "oops").await;
    refused.assert_status(422);
    assert!(refused.text().contains("decided"), "{}", refused.text());
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);

    // Another location, another hold: a recall's.
    let lot_item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&lot_item, "R-1", LotStatus::Released).await;
    t.stock_in(&lot_item, &branch, Some(&lot), 4_000, 100).await;
    let recall = hold_lots(&t, &holder, &[&lot], "recall").await;
    let refused = t.cancel_hold_as(&rae, &recall, "oops").await;
    refused.assert_status(422);
    assert!(refused.text().contains("recall"), "{}", refused.text());
    assert_eq!(t.hold_now(&recall).await.status, HoldStatus::Open);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
}

/// The package keeps its schema and views in step with the models.
#[renox::test]
async fn the_models_match_the_schema() {
    let t = boot_outlet().await;
    t.app.assert_models_match_schema().await;
}
