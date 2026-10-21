//! Opening a recall and holding every lot in scope (wl-inventory §3 *Recall*,
//! §4 *Recall* `open`, §5.7 step 3, issue #233): every point of "Done when"
//! through the module's route, the real transfer and the recorded events.

mod support;

use renox::auth::permissions;
use renox::prelude::*;
use wl_inventory::permissions as perm;
use wl_inventory::recall::{self, OPENED_NOTICE};
use wl_inventory::{HoldLineStatus, HoldScope, HoldStatus, Item, Lot, LotStatus, RecallSource, RecallStatus};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot};

/// The users each notice of `kind` went to, sorted.
fn told(t: &Booted, kind: &str) -> Vec<Vec<i64>> {
    t.notices
        .all()
        .iter()
        .filter(|request| request.type_key == kind)
        .map(|request| {
            let mut users: Vec<i64> = request
                .recipients
                .iter()
                .filter_map(|recipient| match recipient {
                    NotifyRecipient::User { id } => Some(*id),
                    _ => None,
                })
                .collect();
            users.sort_unstable();
            users
        })
        .collect()
}

/// Done when 1: lot L9 at Branch 1 (12), Branch 2 (4) and in transit (6): a
/// recall *supplier* on L9 gives one hold *recall* with three lines, puts L9
/// `on_hold`, tells the owner, the storekeepers and procurement, and emits
/// `RecallOpened`.
#[renox::test]
async fn a_recall_holds_the_lot_at_every_location_in_transit_too() {
    let t = boot().await;
    permissions::define_role(t.db(), "admin", &[perm::STOCK_VIEW])
        .await
        .unwrap();
    let owner = t.user_with("Olga", "admin").await;
    let keeper = t.storekeeper("Amal").await;
    let buyer = t.procurement("Pat").await;
    let manager = t.recaller("Rita").await;
    let _bystander = t.holder("Hal").await;
    let item = t.tracked_item("SERUM").await;
    let lot = t.lot(&item, "L9", LotStatus::Released).await;
    let branch_1 = t.named_location("BRANCH-1", "Branch 1").await;
    let branch_2 = t.named_location("BRANCH-2", "Branch 2").await;
    t.stock_in(&item, &branch_1, Some(&lot), 18_000, 1_000)
        .await;
    t.stock_in(&item, &branch_2, Some(&lot), 4_000, 1_000).await;
    let mover = t.mover("Mo").await;
    let transfer = t
        .transfer(
            &branch_1,
            &branch_2,
            vec![wl_inventory::transfer::NewTransferLine::of(item.id, 6_000).lot(lot.id)],
        )
        .await;
    t.send_as(&mover, &transfer).await.assert_ok();
    let transit = t
        .transfer_now(&transfer)
        .await
        .transit_location_id
        .expect("in transit");
    let item_id = item.id.to_string();

    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Contamination found by the supplier"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_ok();

    let recalls = t.all_recalls().await;
    assert_eq!(recalls.len(), 1);
    let opened = &recalls[0];
    assert_eq!(opened.status, RecallStatus::Traced);
    assert_eq!(opened.source, RecallSource::Supplier);
    assert_eq!(opened.opened_by, Some(manager.id));
    assert!(
        opened
            .number
            .as_deref()
            .is_some_and(|number| number.starts_with("RCL-")),
        "{:?}",
        opened.number
    );
    assert_eq!(
        recall::lots_in_scope(t.db(), opened.id).await.unwrap(),
        vec![lot.id]
    );
    let holds = t.all_holds().await;
    assert_eq!(holds.len(), 1);
    let hold = &holds[0];
    assert_eq!(opened.stock_hold_id, Some(hold.id));
    assert_eq!(hold.reason, "recall");
    assert_eq!(hold.scope_kind, HoldScope::Lots);
    assert_eq!(hold.status, HoldStatus::Open);
    assert_eq!(hold.reference(), Some(opened.subject()));
    let lines: Vec<(i64, i64)> = t
        .hold_lines(hold)
        .await
        .iter()
        .map(|line| (line.location_id, line.quantity_milli))
        .collect();
    assert_eq!(
        lines,
        vec![
            (branch_1.id, 12_000),
            (branch_2.id, 4_000),
            (transit, 6_000)
        ]
    );
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    assert_eq!(t.available(&item, &branch_1, Some(&lot)).await, 0);
    assert_eq!(t.available(&item, &branch_2, Some(&lot)).await, 0);

    // The owner, the storekeeper and procurement, once.
    let mut expected = vec![owner.id, keeper.id, buyer.id];
    expected.sort_unstable();
    assert_eq!(told(&t, OPENED_NOTICE), vec![expected]);

    let events = t.recalls.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].recall, opened.subject());
    assert_eq!(events[0].source, "supplier");
    assert_eq!(events[0].lots, vec![Subject::new("inventory.lot", lot.id)]);
    assert_eq!(events[0].stock_hold, hold.subject());
    let placed = t.holds.all();
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].reason, "recall");
    assert_eq!(placed[0].reference, Some(opened.subject()));
    assert_eq!(placed[0].lines.len(), 3);
    assert_eq!(t.lots_held.all().len(), 1);
}

/// Done when 2, the part that exists today: a picked dispatch holding L9 is
/// flagged by BL-STK-21, which reads the open hold lines and the lot's
/// `on_hold` status. After a recall that lookup returns L9 at every location,
/// the in-transit one included, and says how much of it is held there.
#[renox::test]
async fn the_lookup_a_dispatch_reads_returns_the_recalled_lot_at_every_location() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let item = t.tracked_item("SERUM").await;
    let lot = t.lot(&item, "L9", LotStatus::Released).await;
    let other = t.lot(&item, "L10", LotStatus::Released).await;
    let branch_1 = t.named_location("BRANCH-1", "Branch 1").await;
    let branch_2 = t.named_location("BRANCH-2", "Branch 2").await;
    t.stock_in(&item, &branch_1, Some(&lot), 18_000, 1_000)
        .await;
    t.stock_in(&item, &branch_2, Some(&lot), 4_000, 1_000).await;
    t.stock_in(&item, &branch_1, Some(&other), 9_000, 1_000)
        .await;
    let mover = t.mover("Mo").await;
    let transfer = t
        .transfer(
            &branch_1,
            &branch_2,
            vec![wl_inventory::transfer::NewTransferLine::of(item.id, 6_000).lot(lot.id)],
        )
        .await;
    t.send_as(&mover, &transfer).await.assert_ok();
    let transit = t
        .transfer_now(&transfer)
        .await
        .transit_location_id
        .expect("in transit");
    let item_id = item.id.to_string();
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Contamination found by the supplier"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_ok();

    // The lookup: the lot's undecided lines on open holds, and its status.
    let open_holds: Vec<i64> = t
        .all_holds()
        .await
        .iter()
        .filter(|hold| hold.status == HoldStatus::Open)
        .map(|hold| hold.id)
        .collect();
    let mut held_at: Vec<(i64, i64)> = Vec::new();
    for hold_id in open_holds {
        for line in wl_inventory::stock_hold::lines_of(t.db(), hold_id)
            .await
            .unwrap()
        {
            if line.lot_id == Some(lot.id) && line.status == HoldLineStatus::Undecided {
                held_at.push((line.location_id, line.quantity_milli));
            }
        }
    }
    held_at.sort_unstable();
    let mut expected = vec![
        (branch_1.id, 12_000),
        (branch_2.id, 4_000),
        (transit, 6_000),
    ];
    expected.sort_unstable();
    assert_eq!(held_at, expected);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    // The lot not recalled is not flagged.
    assert_eq!(t.lot_now(&other).await.status, LotStatus::Released);
    assert_eq!(t.available(&item, &branch_1, Some(&other)).await, 9_000);
}

/// Done when 3: a scope with no matching lot is refused, and nothing is made:
/// no recall, no hold, no lot held.
#[renox::test]
async fn a_scope_that_matches_no_lot_is_refused() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let item = t.tracked_item("SERUM").await;
    let other = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&item, &branch, Some(&lot), 5_000, 1_000).await;
    let item_id = item.id.to_string();
    let other_id = other.id.to_string();

    let unknown = t
        .recall_as(
            &manager,
            &[
                ("source", "authority"),
                ("reason", "Ordered"),
                ("items", item_id.as_str()),
                ("lot_numbers", "L10"),
            ],
        )
        .await;
    unknown.assert_status(422);
    assert!(unknown.text().contains("lot"), "{}", unknown.text());
    // The lot number of another item is not this item's.
    t.recall_as(
        &manager,
        &[
            ("source", "authority"),
            ("reason", "Ordered"),
            ("items", other_id.as_str()),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_status(422);

    assert!(t.all_recalls().await.is_empty());
    assert!(t.all_holds().await.is_empty());
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Released);
    assert_eq!(t.available(&item, &branch, Some(&lot)).await, 5_000);
    assert!(t.recalls.all().is_empty());
}

/// A recall needs a source, a reason and a scope with a number in it, and the
/// permission.
#[renox::test]
async fn a_recall_needs_a_source_a_reason_a_scope_and_the_permission() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let holder = t.holder("Hal").await;
    let item = t.tracked_item("SERUM").await;
    let lot = t.lot(&item, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&item, &branch, Some(&lot), 5_000, 1_000).await;
    let item_id = item.id.to_string();
    let good = [
        ("source", "internal"),
        ("reason", "Cold chain broken"),
        ("items", item_id.as_str()),
        ("lot_numbers", "L9"),
    ];

    t.recall_as(&holder, &good).await.assert_forbidden();
    t.recall_as(
        &manager,
        &[
            ("source", "rumour"),
            ("reason", "x"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_status(422);
    t.recall_as(
        &manager,
        &[
            ("source", "internal"),
            ("reason", "  "),
            ("items", item_id.as_str()),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_status(422);
    // Items without a number, and a number without items, are no scope.
    t.recall_as(
        &manager,
        &[
            ("source", "internal"),
            ("reason", "x"),
            ("items", item_id.as_str()),
        ],
    )
    .await
    .assert_status(422);
    t.recall_as(
        &manager,
        &[
            ("source", "internal"),
            ("reason", "x"),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_status(422);
    assert!(t.all_recalls().await.is_empty());
    assert!(t.all_holds().await.is_empty());

    t.recall_as(&manager, &good).await.assert_ok();
    assert_eq!(t.all_recalls().await.len(), 1);
}

/// Supplier lot numbers are matched to the lot's *supplier lot* field, exactly
/// after trimming, and only for the item named.
#[renox::test]
async fn supplier_lot_numbers_are_matched_exactly() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let item = t.tracked_item("SERUM").await;
    let other = t.tracked_item("VACCINE").await;
    let lot = t.lot(&item, "L9", LotStatus::Released).await;
    let same_number = t.lot(&other, "L-OTHER", LotStatus::Released).await;
    for (lot, supplier_lot) in [(&lot, "SUP-77"), (&same_number, "SUP-77")] {
        Lot::query()
            .where_(Lot::ID.eq(lot.id))
            .update(t.db(), &[("supplier_lot", &Some(supplier_lot.to_owned()))])
            .await
            .unwrap();
    }
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&item, &branch, Some(&lot), 5_000, 1_000).await;
    t.stock_in(&other, &branch, Some(&same_number), 5_000, 1_000)
        .await;
    let item_id = item.id.to_string();

    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Batch recalled"),
            ("items", item_id.as_str()),
            ("supplier_lots", "SUP-7"),
        ],
    )
    .await
    .assert_status(422);
    assert!(t.all_recalls().await.is_empty());

    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Batch recalled"),
            ("items", item_id.as_str()),
            ("supplier_lots", " SUP-77 "),
        ],
    )
    .await
    .assert_ok();

    let opened = t.all_recalls().await.remove(0);
    assert_eq!(
        recall::lots_in_scope(t.db(), opened.id).await.unwrap(),
        vec![lot.id]
    );
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    assert_eq!(t.lot_now(&same_number).await.status, LotStatus::Released);
}

/// One recall over several items holds each lot found in one hold; a number
/// is looked for in every item named.
#[renox::test]
async fn one_hold_covers_the_lots_of_several_items() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let serum = t.tracked_item("SERUM").await;
    let vaccine = t.tracked_item("VACCINE").await;
    let serum_lot = t.lot(&serum, "A1", LotStatus::Released).await;
    let vaccine_lot = t.lot(&vaccine, "B1", LotStatus::Quarantine).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&serum, &branch, Some(&serum_lot), 5_000, 1_000)
        .await;
    t.stock_in(&vaccine, &branch, Some(&vaccine_lot), 2_000, 1_000)
        .await;
    let items = format!("{}, {}", serum.id, vaccine.id);

    t.recall_as(
        &manager,
        &[
            ("source", "complaint"),
            ("reason", "Two complaints"),
            ("items", items.as_str()),
            ("lot_numbers", "A1, B1"),
        ],
    )
    .await
    .assert_ok();

    let holds = t.all_holds().await;
    assert_eq!(holds.len(), 1);
    assert_eq!(t.hold_lines(&holds[0]).await.len(), 2);
    assert_eq!(t.lot_now(&serum_lot).await.status, LotStatus::OnHold);
    assert_eq!(t.lot_now(&vaccine_lot).await.status, LotStatus::OnHold);
    let opened = t.all_recalls().await.remove(0);
    assert_eq!(
        recall::lots_in_scope(t.db(), opened.id)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        t.lot_now(&vaccine_lot).await.status_before_hold.as_deref(),
        Some("quarantine")
    );
}

/// A lot with no balance anywhere stays in the scope and gets no line; an
/// `expired` lot is in scope too, keeps its status and gets its line.
#[renox::test]
async fn a_lot_with_no_balance_is_in_scope_without_a_line() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let item = t.tracked_item("SERUM").await;
    let used_up = t.lot(&item, "L1", LotStatus::Released).await;
    let stocked = t.lot(&item, "L2", LotStatus::Released).await;
    let expired = t.lot(&item, "L3", LotStatus::Expired).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    t.stock_in(&item, &branch, Some(&stocked), 5_000, 1_000)
        .await;
    t.stock_in(&item, &branch, Some(&expired), 1_000, 1_000)
        .await;
    let item_id = item.id.to_string();

    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Whole batch"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L1, L2, L3"),
        ],
    )
    .await
    .assert_ok();

    let opened = t.all_recalls().await.remove(0);
    assert_eq!(
        recall::lots_in_scope(t.db(), opened.id).await.unwrap(),
        vec![used_up.id, stocked.id, expired.id]
    );
    let hold = t.all_holds().await.remove(0);
    let held: Vec<Option<i64>> = t
        .hold_lines(&hold)
        .await
        .iter()
        .map(|line| line.lot_id)
        .collect();
    assert_eq!(held, vec![Some(stocked.id), Some(expired.id)]);
    assert_eq!(t.lot_now(&expired).await.status, LotStatus::Expired);
    assert_eq!(t.lot_now(&stocked).await.status, LotStatus::OnHold);
}

/// A recall of lots that are all used up still opens (the trace is what it is
/// for); its hold has nothing to decide and closes at once.
#[renox::test]
async fn a_recall_of_lots_that_are_used_up_still_opens() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let item = t.tracked_item("SERUM").await;
    let lot = t.lot(&item, "L1", LotStatus::Released).await;
    let item_id = item.id.to_string();

    t.recall_as(
        &manager,
        &[
            ("source", "authority"),
            ("reason", "Seen in patients"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L1"),
        ],
    )
    .await
    .assert_ok();

    let opened = t.all_recalls().await.remove(0);
    assert_eq!(opened.status, RecallStatus::Traced);
    let hold = t.all_holds().await.remove(0);
    assert!(t.hold_lines(&hold).await.is_empty());
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(
        recall::lots_in_scope(t.db(), opened.id).await.unwrap(),
        vec![lot.id]
    );
    assert_eq!(t.recalls.all().len(), 1);
}

/// The lots of a recall cannot be picked, issued or transferred, and the
/// recall's hold cannot be cancelled.
#[renox::test]
async fn recalled_stock_cannot_move_and_the_hold_cannot_be_cancelled() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let releaser = t.releaser("Rae").await;
    let mover = t.mover("Mo").await;
    let item = t.tracked_item("SERUM").await;
    let lot = t.lot(&item, "L9", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    let other = t.named_location("BRANCH-2", "Branch 2").await;
    t.stock_in(&item, &branch, Some(&lot), 5_000, 1_000).await;
    let item_id = item.id.to_string();
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Contamination"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L9"),
        ],
    )
    .await
    .assert_ok();

    t.issue_as_user(&mover, &item, &branch, "1")
        .await
        .assert_status(422);
    let transfer = t
        .transfer(
            &branch,
            &other,
            vec![wl_inventory::transfer::NewTransferLine::of(item.id, 1_000).lot(lot.id)],
        )
        .await;
    t.send_as(&mover, &transfer).await.assert_status(422);
    let hold = t.all_holds().await.remove(0);
    let refused = t.cancel_hold_as(&releaser, &hold, "oops").await;
    refused.assert_status(422);
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    assert_eq!(
        wl_inventory::balance::on_hand_at(t.db(), item.id, branch.id, Some(lot.id))
            .await
            .unwrap(),
        5_000
    );
}

/// The lines of a recall's hold are decided like any other: a discard posts
/// the wastage to the recall as cost object, and a release waits for a second
/// holder of `lots.release` (`STK.ACL.05`).
#[renox::test]
async fn the_lines_of_a_recalls_hold_are_decided_with_a_second_person_for_a_release() {
    let t = boot().await;
    let manager = t.recaller("Rita").await;
    let ann = t.releaser("Ann").await;
    let ben = t.releaser("Ben").await;
    let item = t.tracked_item("SERUM").await;
    let lot = t.lot(&item, "L9", LotStatus::Released).await;
    let other = t.lot(&item, "L10", LotStatus::Released).await;
    let branch = t.named_location("BRANCH-1", "Branch 1").await;
    let mut priced = Item::find_or_404(t.db(), item.id).await.unwrap();
    priced.average_cost = 2_000;
    priced.save(t.db()).await.unwrap();
    t.stock_in(&item, &branch, Some(&lot), 5_000, 2_000).await;
    t.stock_in(&item, &branch, Some(&other), 3_000, 2_000).await;
    let item_id = item.id.to_string();
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Contamination"),
            ("items", item_id.as_str()),
            ("lot_numbers", "L9, L10"),
        ],
    )
    .await
    .assert_ok();
    let opened = t.all_recalls().await.remove(0);
    let hold = t.all_holds().await.remove(0);
    let lines = t.hold_lines(&hold).await;
    assert_eq!(lines.len(), 2);

    // L9 is discarded: the wastage is the recall's cost.
    t.decide_as(&ann, &lines[0], "discard").await.assert_ok();
    let costs = t.costs.posted();
    assert_eq!(costs.len(), 1);
    assert_eq!(costs[0].cost_object, opened.subject());
    assert_eq!(costs[0].amount_aed, 10_000);

    // L10 is released: Ann asks, Ben approves, Ann cannot.
    t.decide_as(&ann, &lines[1], "release").await.assert_ok();
    let request = ApprovalRequest::pending_about(t.db(), &lines[1].subject())
        .await
        .unwrap()
        .remove(0);
    let refused = approvals::approve(t.state(), request.id, &ann, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Open);
    approvals::approve(t.state(), request.id, &ben, "checked")
        .await
        .unwrap();

    assert_eq!(t.hold_now(&hold).await.status, HoldStatus::Closed);
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Rejected);
    assert_eq!(t.lot_now(&other).await.status, LotStatus::Released);
    assert_eq!(t.available(&item, &branch, Some(&other)).await, 3_000);
}

/// The package keeps its schema in step with the models.
#[renox::test]
async fn the_models_match_the_schema() {
    let t = boot().await;
    t.app.assert_models_match_schema().await;
}
