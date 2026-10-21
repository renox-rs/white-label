//! Pending issues where negative stock is blocked (wl-inventory §3
//! *Negative stock*, §5.8 step 3, issue #237): automatic consumption never
//! blocks the business event, the short part becomes a pending issue that
//! posts when stock arrives or is cleared by the storekeeper.

mod support;

use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::{DraftLine, Item, Location, LotStatus, PendingIssue, PendingStatus};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::stock_service;

use support::{Booted, boot};

fn day(y: i32, m: u32, d: u32) -> renox::chrono::NaiveDate {
    renox::chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn visit(n: i64) -> Subject {
    Subject::new("clinics.visit", n)
}

/// A receipt of `lines` of `item` at `branch` (delivered on one order of
/// 100 units), posted by a storekeeper.
async fn receive(
    t: &Booted,
    branch: &Location,
    item: &Item,
    lines: Vec<(i64, Option<(&str, u32)>)>,
) {
    let (order, po_line) = t.order(branch, item, 100_000, 500).await;
    let drafts = lines
        .into_iter()
        .map(|(quantity, lot)| {
            let line = DraftLine::on(po_line.id, quantity, quantity);
            match lot {
                Some((number, month)) => line.lot(number, day(2027, month, 28)),
                None => line,
            }
        })
        .collect();
    let receipt = t.receipt(&order, branch, drafts).await;
    let keeper = t.storekeeper("Amal").await;
    t.post_as(&keeper, &receipt).await.assert_ok();
}

/// Done when 1: 2 units at Branch 1 and a visit consuming 5: 2 post now and
/// a pending issue for 3 is `open`; `StockNegative` is emitted. The visit is
/// not refused.
#[renox::test]
async fn short_consumption_posts_what_is_there_and_records_the_rest() {
    let t = boot().await;
    let item = t.tracked_item("GLOVES").await;
    let branch = t.location("BRANCH-1", None).await;
    let lot = t.lot(&item, "L0", LotStatus::Released).await;
    t.stock_in(&item, &branch, Some(&lot), 2_000, 500).await;

    let done = t.consume(&item, &branch, 5_000, &visit(1)).await;

    assert_eq!((done.posted_milli, done.short_milli), (2_000, 3_000));
    let pending = t.open_pending().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].quantity_milli, 3_000);
    assert_eq!(pending[0].status, PendingStatus::Open);
    assert_eq!(pending[0].source(), visit(1));
    assert_eq!(pending[0].cost_object(), Some(visit(1)));
    assert_eq!(done.pending_issue, Some(pending[0].subject()));
    let moved = t.movements_of(&item).await;
    let consumed: Vec<_> = moved
        .iter()
        .filter(|m| m.movement_type == MovementType::Consume)
        .collect();
    assert_eq!(consumed.len(), 1);
    assert_eq!(
        (consumed[0].quantity_milli, consumed[0].lot_id),
        (2_000, Some(lot.id))
    );
    assert_eq!(consumed[0].source(), visit(1));
    assert_eq!(consumed[0].cost_object_id, Some(1));
    // A lot balance is never negative.
    assert_eq!(t.available(&item, &branch, Some(&lot)).await, 0);
    let events = t.negative.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].quantity_milli, 3_000);
    assert_eq!(events[0].source, Some(visit(1)));
    assert_eq!(events[0].pending_issue, Some(pending[0].subject()));
}

/// Done when 2: a receipt of 10 later posts the 3 from the first-expiry lot,
/// marked *assigned on arrival*.
#[renox::test]
async fn a_receipt_posts_the_pending_issue_from_the_first_expiry_lot() {
    let t = boot().await;
    let item = t.tracked_item("GLOVES").await;
    let branch = t.location("BRANCH-1", None).await;
    t.consume(&item, &branch, 3_000, &visit(1)).await;
    assert_eq!(t.open_pending().await.len(), 1);

    // Two lots arrive: the later expiry first on the receipt.
    receive(
        &t,
        &branch,
        &item,
        vec![(4_000, Some(("L-LATE", 9))), (6_000, Some(("L-EARLY", 3)))],
    )
    .await;

    assert!(t.open_pending().await.is_empty());
    let posted = PendingIssue::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(posted.status, PendingStatus::Posted);
    assert!(posted.assigned_on_arrival);
    assert_eq!(posted.resolved_by.as_deref(), Some("receipt"));
    let early = wl_inventory::Lot::query()
        .where_(wl_inventory::Lot::NUMBER.eq("L-EARLY"))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(posted.lot_id, Some(early.id));
    let consumed: Vec<_> = t
        .movements_of(&item)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::Consume)
        .collect();
    assert_eq!(consumed.len(), 1);
    assert_eq!(
        (consumed[0].quantity_milli, consumed[0].lot_id),
        (3_000, Some(early.id))
    );
    assert_eq!(consumed[0].source(), visit(1));
    // Of the 10 received, 7 are left, all of them available.
    assert_eq!(t.available(&item, &branch, None).await, 7_000);
    assert_eq!(t.available(&item, &branch, Some(&early)).await, 3_000);
}

/// A partial arrival posts the oldest pending issue first, and a pending
/// issue it covers in part is split: the posted part and the open rest.
#[renox::test]
async fn a_partial_arrival_posts_oldest_first_and_splits_the_last() {
    let t = boot().await;
    let item = t.item("GAUZE").await;
    let branch = t.location("BRANCH-1", None).await;
    t.consume(&item, &branch, 3_000, &visit(1)).await;
    t.consume(&item, &branch, 3_000, &visit(2)).await;
    assert_eq!(t.open_pending().await.len(), 2);

    receive(&t, &branch, &item, vec![(4_000, None)]).await;

    let all = PendingIssue::query()
        .order_by(PendingIssue::ID)
        .get(t.db())
        .await
        .unwrap();
    // visit 1 posted whole; visit 2 split into 1 posted and 2 open.
    let first = &all[0];
    assert_eq!(
        (first.source(), first.status, first.quantity_milli),
        (visit(1), PendingStatus::Posted, 3_000)
    );
    let open = t.open_pending().await;
    assert_eq!(open.len(), 1);
    assert_eq!(
        (open[0].source(), open[0].quantity_milli),
        (visit(2), 2_000)
    );
    let part = all
        .iter()
        .find(|p| p.split_from_id == Some(open[0].id))
        .expect("the covered part is a row of its own");
    assert_eq!(
        (part.status, part.quantity_milli),
        (PendingStatus::Posted, 1_000)
    );
    assert_eq!(t.available(&item, &branch, None).await, 0);
}

/// Done when 3: a manual issue of 5 with 2 available is refused.
#[renox::test]
async fn a_manual_issue_stops_at_zero() {
    let t = boot().await;
    let item = t.item("GAUZE").await;
    let branch = t.location("BRANCH-1", None).await;
    t.stock_in(&item, &branch, None, 2_000, 500).await;
    let keeper = t.mover("Mo").await;

    let refused = t.issue_as_user(&keeper, &item, &branch, "5").await;

    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("Only 2 of GAUZE is available"), "{body}");
    assert!(t.open_pending().await.is_empty());
    assert_eq!(t.available(&item, &branch, None).await, 2_000);
}

/// Done when 4: 4 kg of a component at a production store and a work order
/// material issue of 10 kg: 4 kg post to the work order and a pending issue
/// for 6 kg is `open`, not refused like a manual issue.
#[renox::test]
async fn a_work_order_material_issue_posts_what_is_there_and_never_blocks() {
    let t = boot().await;
    let item = t.item("RESIN").await;
    let store = t.location("PROD-STORE", None).await;
    t.stock_in(&item, &store, None, 4_000, 500).await;
    let work_order = Subject::new("manufacturing.work_order", 7);

    let done = t.consume(&item, &store, 10_000, &work_order).await;

    assert_eq!((done.posted_milli, done.short_milli), (4_000, 6_000));
    let pending = t.open_pending().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].quantity_milli, 6_000);
    assert_eq!(pending[0].source(), work_order);
    assert_eq!(pending[0].cost_object(), Some(work_order.clone()));
    let consumed: i64 = t
        .movements_of(&item)
        .await
        .iter()
        .filter(|m| m.source() == work_order)
        .map(|m| m.quantity_milli)
        .sum();
    assert_eq!(consumed, 4_000);
}

/// Held quantity is never consumed, and only released lots are: what the hold
/// or the lot status keeps back becomes a pending issue.
#[renox::test]
async fn held_stock_and_lots_not_released_are_not_consumed() {
    let t = boot().await;
    let branch = t.location("BRANCH-1", None).await;
    let holder = t.holder("Hal").await;
    let item = t.item("GAUZE").await;
    t.stock_in(&item, &branch, None, 7_000, 500).await;
    let branch_id = branch.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "quality"),
            ("reference", "ticketing.ticket:9"),
            ("location", branch_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    // 7 are held; 4 more arrive outside the hold, so 4 are available.
    t.stock_in(&item, &branch, None, 4_000, 500).await;

    let done = t.consume(&item, &branch, 5_000, &visit(1)).await;
    assert_eq!((done.posted_milli, done.short_milli), (4_000, 1_000));

    let tracked = t.tracked_item("SERUM").await;
    let waiting = t.lot(&tracked, "Q-1", LotStatus::Quarantine).await;
    t.stock_in(&tracked, &branch, Some(&waiting), 5_000, 500)
        .await;
    let done = t.consume(&tracked, &branch, 2_000, &visit(2)).await;
    assert_eq!((done.posted_milli, done.short_milli), (0, 2_000));
}

/// Done when 5: a pending issue of 3 units at an average cost of AED 40, at a
/// location with balance 0 (limit AED 1,000), cleared with a reason: `adjust`
/// +3 (AED 120) and `consume` -3 to the visit's cost object, the balance
/// stays 0, the pending issue is `cleared`, and no approval is needed.
#[renox::test]
async fn clearing_within_the_limit_enters_the_stock_and_posts_the_consumption() {
    let t = boot().await;
    let mut item = Item::new("SUTURE", "SUTURE", "pcs");
    item.average_cost = 4_000;
    item.save(t.db()).await.unwrap();
    let branch = t.location("BRANCH-1", None).await;
    t.consume(&item, &branch, 3_000, &visit(1)).await;
    let pending = t.open_pending().await.remove(0);
    let clerk = t.clerk("Cass").await;

    t.clear_as(&clerk, &pending, "counted, used from sample stock", None)
        .await
        .assert_ok();

    let now = t.pending_now(&pending).await;
    assert_eq!(now.status, PendingStatus::Cleared);
    assert_eq!(now.resolved_by.as_deref(), Some("storekeeper"));
    assert_eq!(now.cleared_by, Some(clerk.id));
    assert_eq!(
        now.clear_reason.as_deref(),
        Some("counted, used from sample stock")
    );
    let moved = t.movements_of(&item).await;
    assert_eq!(moved.len(), 2);
    assert_eq!(moved[0].movement_type, MovementType::Adjust);
    assert_eq!(
        (moved[0].quantity_milli, moved[0].unit_cost, moved[0].value),
        (3_000, 4_000, 12_000)
    );
    assert_eq!(moved[0].to_location_id, Some(branch.id));
    assert_eq!(moved[1].movement_type, MovementType::Consume);
    assert_eq!(moved[1].quantity_milli, 3_000);
    assert_eq!(moved[1].source(), visit(1));
    assert_eq!(moved[1].cost_object_type.as_deref(), Some("clinics.visit"));
    assert_eq!(moved[1].cost_object_id, Some(1));
    let on_hand = wl_inventory::balance::on_hand_at(t.db(), item.id, branch.id, None)
        .await
        .unwrap();
    assert_eq!(on_hand, 0);
    assert!(
        ApprovalRequest::pending_about(t.db(), &pending.subject())
            .await
            .unwrap()
            .is_empty()
    );
}

/// Surface: the storekeeper's page lists the open pending issues, oldest
/// first, at the locations where the user holds `stock.adjust`; a posted or
/// cleared one is gone from it, and a user without the permission is refused.
#[renox::test]
async fn the_storekeepers_page_lists_the_open_pending_issues_in_their_scope() {
    let t = boot().await;
    let item = t.item("SUTURE").await;
    let site_1 = Subject::new("construction.site", 1);
    let site_2 = Subject::new("construction.site", 2);
    let store_1 = t.location("STORE-1", Some(&site_1)).await;
    let store_2 = t.location("STORE-2", Some(&site_2)).await;
    t.consume(&item, &store_1, 3_000, &visit(1)).await;
    t.consume(&item, &store_2, 2_000, &visit(2)).await;
    t.consume(&item, &store_1, 1_000, &visit(3)).await;

    let list = |by: &User| {
        t.app.acting_as(by);
        t.app.get("/pending-issues")
    };
    // Everywhere: all three, oldest first, with what the page shows.
    let clerk = t.clerk("Cass").await;
    let all: Vec<wl_inventory::PendingRow> = list(&clerk).await.assert_ok().json();
    assert_eq!(
        all.iter()
            .map(|row| (
                row.location.as_str(),
                row.quantity_milli,
                row.source.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("STORE-1", 3_000, "clinics.visit:1"),
            ("STORE-2", 2_000, "clinics.visit:2"),
            ("STORE-1", 1_000, "clinics.visit:3"),
        ]
    );
    assert!(all.iter().all(|row| row.item == "SUTURE"));

    // A storekeeper of site 1 sees the two pending issues of its store.
    let user = User::register(t.db(), "Sam", "sam@example.com", "password123")
        .await
        .unwrap();
    user.assign_role_in(
        t.db(),
        "stock_clerk",
        &renox::auth::permissions::Scope::new(&site_1.subject_type, site_1.subject_id),
    )
    .await
    .unwrap();
    let mine: Vec<wl_inventory::PendingRow> = list(&user).await.assert_ok().json();
    assert_eq!(
        mine.iter()
            .map(|row| row.location.as_str())
            .collect::<Vec<_>>(),
        vec!["STORE-1", "STORE-1"]
    );

    // Without `stock.adjust` the page is refused.
    let holder = t.holder("Hal").await;
    list(&holder).await.assert_status(403);

    // A cleared pending issue leaves the list.
    let first = t.open_pending().await.remove(0);
    t.clear_as(&clerk, &first, "used from sample stock", None)
        .await
        .assert_ok();
    let rest: Vec<wl_inventory::PendingRow> = list(&clerk).await.assert_ok().json();
    assert_eq!(rest.len(), 2);
    assert!(rest.iter().all(|row| row.id != first.id));
}

/// Done when 6: a pending issue of 30 units (AED 1,200) asks for approval;
/// the storekeeper cannot approve; nothing posts until it is approved.
#[renox::test]
async fn clearing_above_the_limit_waits_for_a_second_person() {
    let t = boot().await;
    let mut item = Item::new("SUTURE", "SUTURE", "pcs");
    item.average_cost = 4_000;
    item.save(t.db()).await.unwrap();
    let branch = t.location("BRANCH-1", None).await;
    t.consume(&item, &branch, 30_000, &visit(1)).await;
    let pending = t.open_pending().await.remove(0);
    let clerk = t.clerk("Cass").await;
    let manager = t.user_with("Mia", "stock_manager").await;

    t.clear_as(&clerk, &pending, "counted, used from sample stock", None)
        .await
        .assert_ok();

    let requests = ApprovalRequest::pending_about(t.db(), &pending.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(t.pending_now(&pending).await.status, PendingStatus::Open);
    assert!(t.movements_of(&item).await.is_empty());

    let refused = approvals::approve(t.state(), requests[0].id, &clerk, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.pending_now(&pending).await.status, PendingStatus::Open);
    assert!(t.movements_of(&item).await.is_empty());

    approvals::approve(t.state(), requests[0].id, &manager, "approved")
        .await
        .unwrap();
    let now = t.pending_now(&pending).await;
    assert_eq!(now.status, PendingStatus::Cleared);
    assert_eq!(now.cleared_by, Some(clerk.id));
    let moved = t.movements_of(&item).await;
    assert_eq!(moved.len(), 2);
    assert_eq!(moved[0].value, 120_000);
}

/// A lot-tracked item is cleared into a lot of its own, named by the
/// storekeeper.
#[renox::test]
async fn clearing_a_lot_tracked_item_needs_its_lot() {
    let t = boot().await;
    let item = t.tracked_item("SERUM").await;
    let other = t.tracked_item("VACCINE").await;
    let branch = t.location("BRANCH-1", None).await;
    let lot = t.lot(&item, "S-1", LotStatus::Released).await;
    let wrong = t.lot(&other, "V-1", LotStatus::Released).await;
    t.consume(&item, &branch, 2_000, &visit(1)).await;
    let pending = t.open_pending().await.remove(0);
    let clerk = t.clerk("Cass").await;

    let missing = t.clear_as(&clerk, &pending, "used", None).await;
    missing.assert_status(422);
    assert!(missing.text().contains("lot"), "{}", missing.text());
    t.clear_as(&clerk, &pending, "used", Some(&wrong))
        .await
        .assert_status(422);
    assert_eq!(t.pending_now(&pending).await.status, PendingStatus::Open);

    t.clear_as(&clerk, &pending, "used", Some(&lot))
        .await
        .assert_ok();
    let now = t.pending_now(&pending).await;
    assert_eq!(
        (now.status, now.lot_id),
        (PendingStatus::Cleared, Some(lot.id))
    );
    let moved = t.movements_of(&item).await;
    assert!(moved.iter().all(|m| m.lot_id == Some(lot.id)));
}

/// A reason is needed, and only a holder of `stock.adjust` clears.
#[renox::test]
async fn clearing_needs_a_reason_and_the_permission() {
    let t = boot().await;
    let item = t.item("GAUZE").await;
    let branch = t.location("BRANCH-1", None).await;
    t.consume(&item, &branch, 1_000, &visit(1)).await;
    let pending = t.open_pending().await.remove(0);
    let clerk = t.clerk("Cass").await;
    let mover = t.mover("Mo").await;

    t.clear_as(&clerk, &pending, "  ", None)
        .await
        .assert_status(422);
    t.clear_as(&mover, &pending, "used", None)
        .await
        .assert_forbidden();
    assert_eq!(t.pending_now(&pending).await.status, PendingStatus::Open);
    assert!(t.movements_of(&item).await.is_empty());
}

/// A visit voided before its pending issue posted clears it with the reason
/// *source cancelled* and no movement.
#[renox::test]
async fn a_cancelled_source_clears_its_pending_issues_without_a_movement() {
    let t = boot().await;
    let item = t.item("GAUZE").await;
    let branch = t.location("BRANCH-1", None).await;
    t.consume(&item, &branch, 3_000, &visit(1)).await;
    t.consume(&item, &branch, 2_000, &visit(2)).await;

    let cleared = stock_service(t.state())
        .source_cancelled(t.state(), &visit(1))
        .await
        .unwrap();

    assert_eq!(cleared, 1);
    let open = t.open_pending().await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].source(), visit(2));
    let gone = PendingIssue::query()
        .where_(PendingIssue::SOURCE_ID.eq(1))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(gone.status, PendingStatus::Cleared);
    assert_eq!(gone.clear_reason.as_deref(), Some("source cancelled"));
    assert_eq!(gone.resolved_by.as_deref(), Some("source_cancelled"));
    assert!(t.movements_of(&item).await.is_empty());
    // Nothing is left to post for it when stock arrives.
    receive(&t, &branch, &item, vec![(10_000, None)]).await;
    assert_eq!(
        t.movements_of(&item)
            .await
            .iter()
            .filter(|m| m.movement_type == MovementType::Consume)
            .map(|m| m.quantity_milli)
            .sum::<i64>(),
        2_000
    );
}

/// A transfer received at the location posts its pending issues the same
/// way a receipt does.
#[renox::test]
async fn a_received_transfer_posts_the_pending_issue() {
    let t = boot().await;
    let item = t.item("GAUZE").await;
    let branch = t.location("BRANCH-1", None).await;
    let store = t.location("STORE-A", None).await;
    t.stock_in(&item, &store, None, 10_000, 500).await;
    t.consume(&item, &branch, 3_000, &visit(1)).await;
    let mover = t.mover("Mo").await;
    let transfer = t
        .transfer(
            &store,
            &branch,
            vec![wl_inventory::transfer::NewTransferLine::of(item.id, 4_000)],
        )
        .await;
    t.send_as(&mover, &transfer).await.assert_ok();
    // On its way the stock does not cover the pending issue.
    assert_eq!(t.open_pending().await.len(), 1);

    t.receive_as(&mover, &transfer, false).await.assert_ok();

    let posted = PendingIssue::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(posted.status, PendingStatus::Posted);
    assert_eq!(posted.resolved_by.as_deref(), Some("transfer"));
    assert_eq!(t.available(&item, &branch, None).await, 1_000);
    assert_eq!(t.available(&item, &store, None).await, 6_000);
}

/// Held quantity is not consumed: stock arriving at a held location joins the
/// hold, so it does not cover a pending issue either.
#[renox::test]
async fn stock_that_joins_a_hold_does_not_cover_a_pending_issue() {
    let t = boot().await;
    let item = t.item("VIAL").await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    let store = t.location("STORE-A", None).await;
    t.stock_in(&item, &branch, None, 2_000, 500).await;
    t.stock_in(&item, &store, None, 10_000, 500).await;
    let holder = t.holder("Hal").await;
    let mover = t.mover("Mo").await;
    let branch_id = branch.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "temperature"),
            ("reference", "clinics.checklist_reading:3"),
            ("location", branch_id.as_str()),
        ],
    )
    .await
    .assert_ok();
    // The 2 on hand are held: the visit gets nothing and the rest is pending.
    let done = t.consume(&item, &branch, 3_000, &visit(1)).await;
    assert_eq!((done.posted_milli, done.short_milli), (0, 3_000));
    let transfer = t
        .transfer(
            &store,
            &branch,
            vec![wl_inventory::transfer::NewTransferLine::of(item.id, 4_000)],
        )
        .await;
    t.send_as(&mover, &transfer).await.assert_ok();

    t.receive_as(&mover, &transfer, true).await.assert_ok();

    assert_eq!(t.open_pending().await.len(), 1);
    assert_eq!(t.available(&item, &branch, None).await, 0);
    let hold = wl_inventory::StockHold::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    let held = wl_inventory::stock_hold::lines_of(t.db(), hold.id)
        .await
        .unwrap();
    assert_eq!(
        held.iter().map(|l| l.quantity_milli).collect::<Vec<_>>(),
        vec![2_000, 4_000]
    );
}
