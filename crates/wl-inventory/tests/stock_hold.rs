//! Placing a stock hold and blocking held quantity (wl-inventory §3 *Holds*,
//! §4 *Stock hold* `place` and `add_lines`, §4 *Lot* `hold`, issue #230):
//! every point of "Done when" through the module's routes and transitions.

mod support;

use renox::prelude::*;
use wl_inventory::stock_hold::{self, ADD_LINES_BY_SYSTEM, LINES_NOTICE, PLACED_NOTICE};
use wl_inventory::transfer::NewTransferLine;
use wl_inventory::{HoldScope, HoldStatus, Location, LotStatus, StockHold, TransferStatus};
use wl_kernel::Subject;
use wl_kernel::inventory::{PlaceHold, stock_hold_service};
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot};

/// The holds placed, oldest first.
async fn holds(t: &Booted) -> Vec<StockHold> {
    StockHold::query()
        .order_by(StockHold::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// The `(location, item, quantity)` of the lines of `hold`, oldest first.
async fn lines(t: &Booted, hold: &StockHold) -> Vec<(i64, i64, i64)> {
    stock_hold::lines_of(t.db(), hold.id)
        .await
        .unwrap()
        .iter()
        .map(|line| (line.location_id, line.item_id, line.quantity_milli))
        .collect()
}

/// The users a notice of `kind` was sent to.
fn told(t: &Booted, kind: &str) -> Vec<i64> {
    t.notices
        .all()
        .iter()
        .filter(|request| request.type_key == kind)
        .flat_map(|request| request.recipients.iter())
        .filter_map(|recipient| match recipient {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// Done when 1: lot L1 has 40 at Store A and 10 in transit; a whole-lot hold
/// *quality* makes lines of 40 and 10, puts L1 `on_hold`, leaves nothing
/// available, and emits `StockHoldPlaced` and `LotHeld`.
#[renox::test]
async fn a_whole_lot_hold_covers_every_location_of_the_lot() {
    let t = boot().await;
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let store_a = t.location("STORE-A", None).await;
    let store_b = t.location("STORE-B", None).await;
    t.stock_in(&item, &store_a, Some(&lot), 50_000, 1_000).await;
    let mover = t.mover("Mo").await;
    let releaser = t.releaser("Rae").await;
    let holder = t.holder("Hal").await;
    // 10 of the lot are on their way to Store B.
    let transfer = t
        .transfer(
            &store_a,
            &store_b,
            vec![NewTransferLine::of(item.id, 10_000).lot(lot.id)],
        )
        .await;
    t.send_as(&mover, &transfer).await.assert_ok();
    let transit = t
        .transfer_now(&transfer)
        .await
        .transit_location_id
        .expect("the transfer is in transit");
    assert_eq!(t.available(&item, &store_a, Some(&lot)).await, 40_000);

    let lot_id = lot.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "quality"),
            ("reference", "ticketing.ticket:9"),
            ("lots", lot_id.as_str()),
        ],
    )
    .await
    .assert_ok();

    let placed = holds(&t).await;
    assert_eq!(placed.len(), 1);
    let hold = &placed[0];
    assert_eq!(hold.status, HoldStatus::Open);
    assert_eq!(hold.scope_kind, HoldScope::Lots);
    assert_eq!(hold.reason, "quality");
    assert_eq!(hold.number.as_deref().map(|n| &n[..4]), Some("HLD-"));
    assert_eq!(hold.placed_by, Some(holder.id));
    assert_eq!(
        lines(&t, hold).await,
        vec![
            (store_a.id, item.id, 40_000),
            (transit, item.id, 10_000)
        ]
    );
    let held = t.lot_now(&lot).await;
    assert_eq!(held.status, LotStatus::OnHold);
    assert_eq!(held.status_before_hold.as_deref(), Some("released"));
    assert_eq!(t.available(&item, &store_a, Some(&lot)).await, 0);
    assert_eq!(t.available(&item, &store_a, None).await, 0);
    assert_eq!(t.available(&item, &Location::find_or_404(t.db(), transit).await.unwrap(), Some(&lot)).await, 0);

    let placed_events = t.holds.all();
    assert_eq!(placed_events.len(), 1);
    assert_eq!(placed_events[0].reason, "quality");
    assert_eq!(placed_events[0].lines.len(), 2);
    assert_eq!(
        placed_events[0].reference,
        Some(Subject::new("ticketing.ticket", 9))
    );
    let lot_events = t.lots_held.all();
    assert_eq!(lot_events.len(), 1);
    assert_eq!(lot_events[0].lot, Subject::new("inventory.lot", lot.id));
    assert_eq!(lot_events[0].reason, "quality");
    assert_eq!(lot_events[0].quantity_milli, 50_000);
    // The holders of `lots.release` are told.
    assert_eq!(told(&t, PLACED_NOTICE), vec![releaser.id]);
}

/// Done when 2: a hold *temperature* on Fridge 2 (items X 5, Y 3) leaves the
/// lot statuses alone and gives X and Y at Fridge 2 an available quantity of
/// 0, while the same lot elsewhere stays available.
#[renox::test]
async fn a_hold_on_a_location_blocks_only_that_balance() {
    let t = boot().await;
    let x = t.tracked_item("X").await;
    let lot_x = t.lot(&x, "LX", LotStatus::Released).await;
    let y = t.item("Y").await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    let shelf = t.location("SHELF", None).await;
    t.stock_in(&x, &fridge, Some(&lot_x), 5_000, 100).await;
    t.stock_in(&x, &shelf, Some(&lot_x), 2_000, 100).await;
    t.stock_in(&y, &fridge, None, 3_000, 100).await;
    let holder = t.holder("Hal").await;
    let fridge_id = fridge.id.to_string();

    t.hold_as(
        &holder,
        &[
            ("reason", "temperature"),
            ("reference", "clinics.checklist_reading:3"),
            ("location", fridge_id.as_str()),
        ],
    )
    .await
    .assert_ok();

    let hold = holds(&t).await.remove(0);
    assert_eq!(hold.scope_kind, HoldScope::Location);
    assert_eq!(hold.location_id, Some(fridge.id));
    assert_eq!(
        lines(&t, &hold).await,
        vec![(fridge.id, x.id, 5_000), (fridge.id, y.id, 3_000)]
    );
    let lot = t.lot_now(&lot_x).await;
    assert_eq!(lot.status, LotStatus::Released);
    assert_eq!(lot.status_before_hold, None);
    assert_eq!(t.available(&x, &fridge, Some(&lot_x)).await, 0);
    assert_eq!(t.available(&x, &fridge, None).await, 0);
    assert_eq!(t.available(&y, &fridge, None).await, 0);
    // The same lot at another location is untouched.
    assert_eq!(t.available(&x, &shelf, Some(&lot_x)).await, 2_000);
    assert!(t.lots_held.all().is_empty());
    assert_eq!(t.holds.all().len(), 1);
}

/// A hold limited to named items leaves the other items of the location
/// available.
#[renox::test]
async fn a_hold_on_named_items_leaves_the_others_available() {
    let t = boot().await;
    let x = t.item("X").await;
    let y = t.item("Y").await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    t.stock_in(&x, &fridge, None, 5_000, 100).await;
    t.stock_in(&y, &fridge, None, 3_000, 100).await;
    let holder = t.holder("Hal").await;
    let (fridge_id, x_id) = (fridge.id.to_string(), x.id.to_string());

    t.hold_as(
        &holder,
        &[
            ("reason", "temperature"),
            ("reference", "clinics.checklist_reading:3"),
            ("location", fridge_id.as_str()),
            ("items", x_id.as_str()),
        ],
    )
    .await
    .assert_ok();

    let hold = holds(&t).await.remove(0);
    assert_eq!(lines(&t, &hold).await, vec![(fridge.id, x.id, 5_000)]);
    assert_eq!(t.available(&x, &fridge, None).await, 0);
    assert_eq!(t.available(&y, &fridge, None).await, 3_000);
}

/// Done when 3: a transfer and a manual issue of held quantity are refused,
/// whether the hold is on the lot or on the location; the part that is not
/// held can still be issued.
#[renox::test]
async fn held_quantity_cannot_be_transferred_or_issued() {
    let t = boot().await;
    let mover = t.mover("Mo").await;
    let holder = t.holder("Hal").await;
    let store_a = t.location("STORE-A", None).await;
    let store_b = t.location("STORE-B", None).await;

    // A whole-lot hold.
    let item = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    t.stock_in(&item, &store_a, Some(&lot), 40_000, 1_000).await;
    let lot_id = lot.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "quality"),
            ("reference", "ticketing.ticket:9"),
            ("lots", lot_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    let transfer = t
        .transfer(
            &store_a,
            &store_b,
            vec![NewTransferLine::of(item.id, 4_000).lot(lot.id)],
        )
        .await;
    let refused = t.send_as(&mover, &transfer).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("not released"), "{body}");
    assert_eq!(
        t.transfer_now(&transfer).await.status,
        TransferStatus::Draft
    );
    let refused = t.issue_as_user(&mover, &item, &store_a, "4").await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("on hold"), "{body}");

    // A hold on a location: 3 held, 4 more arrived by a count.
    let y = t.item("Y").await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    t.stock_in(&y, &fridge, None, 3_000, 100).await;
    let fridge_id = fridge.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "temperature"),
            ("reference", "clinics.checklist_reading:3"),
            ("location", fridge_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    t.stock_in(&y, &fridge, None, 4_000, 100).await;
    assert_eq!(t.available(&y, &fridge, None).await, 4_000);
    let transfer = t
        .transfer(
            &fridge,
            &store_b,
            vec![NewTransferLine::of(y.id, 5_000)],
        )
        .await;
    let refused = t.send_as(&mover, &transfer).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("on hold"), "{body}");
    let refused = t.issue_as_user(&mover, &y, &fridge, "5").await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("on hold"), "{body}");
    // What is not held is issued.
    t.issue_as_user(&mover, &y, &fridge, "4").await.assert_ok();
    assert_eq!(t.available(&y, &fridge, None).await, 0);
    assert_eq!(
        wl_inventory::balance::balance(t.db(), y.id, fridge.id, None)
            .await
            .unwrap()
            .on_hand_milli,
        3_000
    );
}

/// Done when 4: a hold with no balance in scope is refused, as are a reason
/// that is not on the list, a missing reference and a user without
/// `stock.hold`.
#[renox::test]
async fn a_hold_needs_stock_a_reason_a_reference_and_the_permission() {
    let t = boot().await;
    let holder = t.holder("Hal").await;
    let empty = t.named_location("EMPTY", "Empty store").await;
    let empty_id = empty.id.to_string();

    let refused = t
        .hold_as(
            &holder,
            &[
                ("reason", "quality"),
                ("reference", "ticketing.ticket:9"),
                ("location", empty_id.as_str()),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("no stock"), "{}", refused.text());

    let item = t.item("X").await;
    t.stock_in(&item, &empty, None, 1_000, 100).await;
    t.hold_as(
        &holder,
        &[
            ("reason", "boredom"),
            ("reference", "ticketing.ticket:9"),
            ("location", empty_id.as_str()),
        ],
    )
    .await
    .assert_status(422);
    t.hold_as(
        &holder,
        &[("reason", "quality"), ("location", empty_id.as_str())],
    )
    .await
    .assert_status(422);
    let clerk = t.clerk("Cass").await;
    t.hold_as(
        &clerk,
        &[
            ("reason", "quality"),
            ("reference", "ticketing.ticket:9"),
            ("location", empty_id.as_str()),
        ],
    )
    .await
    .assert_forbidden();

    assert!(holds(&t).await.is_empty());
    assert!(t.holds.all().is_empty());
    assert_eq!(t.available(&item, &empty, None).await, 1_000);
}

/// Done when 5: with the *temperature* hold open on Fridge 2, receiving a
/// transfer of 4 kg of X warns "Fridge 2 is on temperature hold"; on going on
/// the hold gets a new line X 4, X at Fridge 2 still has nothing available
/// and the deciders are notified.
#[renox::test]
async fn stock_moved_into_a_held_location_warns_and_joins_the_hold() {
    let t = boot().await;
    let x = t.item("X").await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    let store_a = t.location("STORE-A", None).await;
    t.stock_in(&x, &fridge, None, 5_000, 100).await;
    t.stock_in(&x, &store_a, None, 20_000, 100).await;
    let mover = t.mover("Mo").await;
    let holder = t.holder("Hal").await;
    let releaser = t.releaser("Rae").await;
    let fridge_id = fridge.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "temperature"),
            ("reference", "clinics.checklist_reading:3"),
            ("location", fridge_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    let hold = holds(&t).await.remove(0);
    let transfer = t
        .transfer(&store_a, &fridge, vec![NewTransferLine::of(x.id, 4_000)])
        .await;
    t.send_as(&mover, &transfer).await.assert_ok();

    // The screen warns first and nothing moves.
    let warned = t.receive_as(&mover, &transfer, false).await;
    warned.assert_status(422);
    let body = warned.text();
    assert!(body.contains("Fridge 2 is on temperature hold"), "{body}");
    assert_eq!(
        t.transfer_now(&transfer).await.status,
        TransferStatus::InTransit
    );
    assert_eq!(lines(&t, &hold).await.len(), 1);

    // Going on receives it and the quantity joins the hold.
    t.receive_as(&mover, &transfer, true).await.assert_ok();
    assert_eq!(
        t.transfer_now(&transfer).await.status,
        TransferStatus::Received
    );
    assert_eq!(
        lines(&t, &hold).await,
        vec![(fridge.id, x.id, 5_000), (fridge.id, x.id, 4_000)]
    );
    assert_eq!(
        wl_inventory::balance::balance(t.db(), x.id, fridge.id, None)
            .await
            .unwrap()
            .on_hand_milli,
        9_000
    );
    assert_eq!(t.available(&x, &fridge, None).await, 0);
    assert_eq!(told(&t, LINES_NOTICE), vec![releaser.id]);
    // The hold is the same one, still open, with the reason of the first.
    let hold = StockHold::find_or_404(t.db(), hold.id).await.unwrap();
    assert_eq!(hold.status, HoldStatus::Open);
    assert_eq!(hold.reason, "temperature");
}

/// A second hold on a lot that is `on_hold` already adds lines and leaves
/// the lot as it is; the balance of an `expired` lot is a line too and the
/// lot stays `expired`.
#[renox::test]
async fn a_lot_already_held_or_expired_keeps_its_status_and_still_gets_lines() {
    let t = boot().await;
    let item = t.tracked_item("VACCINE").await;
    let store = t.location("STORE-A", None).await;
    let holder = t.holder("Hal").await;
    let l2 = t.lot(&item, "L2", LotStatus::Released).await;
    let l3 = t.lot(&item, "L3", LotStatus::Expired).await;
    t.stock_in(&item, &store, Some(&l2), 10_000, 1_000).await;
    t.stock_in(&item, &store, Some(&l3), 6_000, 1_000).await;
    let (l2_id, l3_id) = (l2.id.to_string(), l3.id.to_string());

    for reason in ["quality", "complaint"] {
        t.hold_as(
            &holder,
            &[
                ("reason", reason),
                ("reference", "ticketing.ticket:9"),
                ("lots", l2_id.as_str()),
            ],
        )
        .await
        .assert_ok();
    }
    let placed = holds(&t).await;
    assert_eq!(placed.len(), 2);
    assert_eq!(lines(&t, &placed[1]).await, vec![(store.id, item.id, 10_000)]);
    assert_eq!(t.lot_now(&l2).await.status, LotStatus::OnHold);
    // `hold` ran once, for the first hold.
    assert_eq!(t.lots_held.all().len(), 1);
    // The status before the first hold is still the one stored.
    assert_eq!(
        t.lot_now(&l2).await.status_before_hold.as_deref(),
        Some("released")
    );

    t.hold_as(
        &holder,
        &[
            ("reason", "other"),
            ("reference", "ticketing.ticket:10"),
            ("lots", l3_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    let placed = holds(&t).await;
    assert_eq!(lines(&t, &placed[2]).await, vec![(store.id, item.id, 6_000)]);
    assert_eq!(t.lot_now(&l3).await.status, LotStatus::Expired);
    assert_eq!(t.lots_held.all().len(), 1);
    assert_eq!(t.available(&item, &store, None).await, 0);
}

/// A vertical automation places the hold through the kernel's
/// `StockHoldService` (06: an out-of-range fridge reading), as the system.
#[renox::test]
async fn a_vertical_automation_places_a_hold_as_the_system() {
    let t = boot().await;
    let x = t.item("X").await;
    let fridge = t.named_location("FRIDGE-2", "Fridge 2").await;
    t.stock_in(&x, &fridge, None, 5_000, 100).await;

    let placed = stock_hold_service(t.state())
        .place(
            t.state(),
            PlaceHold {
                reason: "temperature".into(),
                reference: Subject::new("clinics.checklist_reading", 3),
                location: Subject::new("inventory.location", fridge.id),
                items: Vec::new(),
            },
        )
        .await
        .unwrap()
        .expect("inventory is on");

    let hold = holds(&t).await.remove(0);
    assert_eq!(placed, hold.subject());
    assert_eq!(hold.placed_by, None);
    assert_eq!(hold.reason, "temperature");
    assert_eq!(t.available(&x, &fridge, None).await, 0);
    assert_eq!(t.holds.all().len(), 1);
    // The system's `add_lines` is a transition of its own.
    assert!(
        <StockHold as renox::state_machine::StateMachine>::machine()
            .find(ADD_LINES_BY_SYSTEM)
            .is_some_and(|transition| transition.is_system())
    );
}
