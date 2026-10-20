//! Posting a goods receipt with accepted and rejected quantity per line
//! (wl-inventory §4 *Goods receipt* `post`, §5.6, `STK.ACL.04`, issue #220).

mod support;

use renox::prelude::*;
use wl_inventory::goods_receipt::lines_of;
use wl_inventory::movement::{self, MovementType};
use wl_inventory::supplier_return::{self, RejectedHandling, ReturnOrigin, ReturnStatus};
use wl_inventory::{
    DraftLine, GoodsReceiptStatus, LotStatus, PurchaseOrderLine, PurchaseOrderStatus,
};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::purchase_match;

use support::{Booted, boot};

/// 100 kg of chicken at AED 12.00 a kg, ordered for the store.
async fn chicken(t: &Booted) -> (wl_inventory::Item, wl_inventory::Location, wl_inventory::PurchaseOrder, PurchaseOrderLine) {
    let item = t.item("CHICKEN").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = t.order(&store, &item, 100_000, 1_200).await;
    (item, store, order, line)
}

fn day(y: i32, m: u32, d: u32) -> renox::chrono::NaiveDate {
    renox::chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Done when 1: received 100, accepted 80, rejected 20 for *temperature*
/// (reading 9 C) refused at the door: stock +80 kg, the order
/// `partially_received` with 20 outstanding, a Return to supplier for 20 kg
/// refused at the door and rejected at receipt, and `GoodsReceived` 80/20.
#[renox::test]
async fn accepted_goods_enter_stock_and_rejected_ones_go_back_to_the_supplier() {
    let t = boot().await;
    let (item, store, order, po_line) = chicken(&t).await;
    let amal = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![
                DraftLine::on(po_line.id, 100_000, 80_000)
                    .rejected_for("temperature", RejectedHandling::RefusedAtDoor)
                    .evidence("9 C"),
            ],
        )
        .await;
    assert_eq!(receipt.status, GoodsReceiptStatus::Draft);
    assert_eq!(receipt.number, None);

    t.post_as(&amal, &receipt).await.assert_ok();

    let posted = t.reload(&receipt).await;
    assert_eq!(posted.status, GoodsReceiptStatus::Posted);
    assert!(posted.number.as_deref().is_some_and(|n| n.starts_with("GRN-")));
    assert_eq!(posted.posted_by, Some(amal.id));

    // Only the accepted quantity is stock; nothing moved for the rest.
    let moves = movement::of_source(t.db(), &posted.subject()).await.unwrap();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].movement_type, MovementType::Receive);
    assert_eq!((moves[0].quantity_milli, moves[0].value), (80_000, 96_000));
    assert_eq!(moves[0].to_location_id, Some(store.id));
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 80_000);
    let costed = wl_inventory::Item::find_or_404(t.db(), item.id).await.unwrap();
    assert_eq!(costed.average_cost, 1_200);

    // The order counts the accepted quantity; 20 kg stay outstanding.
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::PartiallyReceived);
    let line = t.line_now(&po_line).await;
    assert_eq!(line.accepted_qty_milli, 80_000);
    assert_eq!(line.outstanding_milli(), 20_000);

    // A Return to supplier for the 20 kg refused at the door.
    let returns = supplier_return::of_source(t.db(), &posted.subject()).await.unwrap();
    assert_eq!(returns.len(), 1);
    let ret = &returns[0];
    assert_eq!(ret.handling, RejectedHandling::RefusedAtDoor);
    // The system dispatched it as the receipt posted (BL-STK-03).
    assert_eq!(ret.status, ReturnStatus::Dispatched);
    assert_eq!(ret.supplier_party_id, support::SUPPLIER.0);
    assert!(ret.number.as_deref().is_some_and(|n| n.starts_with("RTS-")));
    assert_eq!(
        ret.collection_proof.as_deref(),
        Some(posted.subject().to_string().as_str())
    );
    let ret_lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(ret_lines.len(), 1);
    assert_eq!(ret_lines[0].quantity_milli, 20_000);
    assert_eq!(ret_lines[0].reason, "temperature");
    assert_eq!(ret_lines[0].origin, ReturnOrigin::RejectedAtReceipt);
    assert_eq!(ret_lines[0].po_line_id, Some(po_line.id));
    assert_eq!(ret_lines[0].unit_cost, 1_200);
    let stored = lines_of(t.db(), receipt.id).await.unwrap();
    assert_eq!(ret_lines[0].receipt_line_id, Some(stored[0].id));

    // GoodsReceived shows 80 accepted and 20 rejected.
    let events = t.received.all();
    assert_eq!(events.len(), 1);
    let line = &events[0].lines[0];
    assert_eq!(line.received_qty_milli, 100_000);
    assert_eq!((line.accepted_qty_milli, line.rejected_qty_milli), (80_000, 20_000));
    assert_eq!(line.reason.as_deref(), Some("temperature"));
    assert_eq!(events[0].value, 96_000);
    assert_eq!(events[0].purchase_order, Some(order.subject()));
    assert_eq!(events[0].location, store.subject());
}

/// Done when 6: the supplier bill match sees 80 kg, not 100.
#[renox::test]
async fn the_bill_match_sees_the_accepted_quantity_only() {
    let t = boot().await;
    let (_, store, order, po_line) = chicken(&t).await;
    let amal = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(po_line.id, 100_000, 80_000)
                .rejected_for("quality", RejectedHandling::KeptForCollection)],
        )
        .await;
    t.post_as(&amal, &receipt).await.assert_ok();
    let lines = purchase_match(t.state())
        .lines(t.db(), &order.subject())
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].po_line, po_line.id);
    assert_eq!(lines[0].ordered_qty_milli, 100_000);
    assert_eq!(lines[0].accepted_qty_milli, 80_000);
    assert_eq!(lines[0].unit_price, 1_200);
    // An order of another type or an unknown one has no lines.
    let none = purchase_match(t.state())
        .lines(t.db(), &Subject::new("inventory.purchase_order", 999))
        .await
        .unwrap();
    assert!(none.is_empty());
}

/// Done when 2: rejected 20 without a reason is refused; nothing is written.
#[renox::test]
async fn a_rejected_quantity_without_a_reason_is_refused() {
    let t = boot().await;
    let (item, store, order, po_line) = chicken(&t).await;
    let amal = t.storekeeper("Amal").await;
    let mut line = DraftLine::on(po_line.id, 100_000, 80_000);
    line.rejected_handling = Some(RejectedHandling::RefusedAtDoor);
    let receipt = t.receipt(&order, &store, vec![line]).await;
    t.post_as(&amal, &receipt)
        .await
        .assert_status(422)
        .assert_see("give a reason");

    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Draft);
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 0);
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::Issued);
    assert!(supplier_return::of_source(t.db(), &receipt.subject()).await.unwrap().is_empty());
    assert!(t.received.all().is_empty());

    // Counted again with a reason, the same receipt posts.
    let stored = lines_of(t.db(), receipt.id).await.unwrap();
    let again = DraftLine::on(po_line.id, 100_000, 80_000)
        .rejected_for("damaged", RejectedHandling::RefusedAtDoor);
    wl_inventory::goods_receipt::update_line(t.db(), receipt.id, stored[0].id, &again)
        .await
        .unwrap();
    t.post_as(&amal, &receipt).await.assert_ok();
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
}

/// Accepted plus rejected must equal received, to the thousandth.
#[renox::test]
async fn accepted_plus_rejected_must_equal_received() {
    let t = boot().await;
    let (_, store, order, po_line) = chicken(&t).await;
    let amal = t.storekeeper("Amal").await;
    let mut line = DraftLine::on(po_line.id, 100_000, 80_000)
        .rejected_for("damaged", RejectedHandling::RefusedAtDoor);
    line.rejected_qty_milli = 19_999;
    let receipt = t.receipt(&order, &store, vec![line]).await;
    t.post_as(&amal, &receipt)
        .await
        .assert_status(422)
        .assert_see("accepted plus rejected");
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Draft);
}

/// Done when 3: accepted 106 of 100 (tolerance 5%): storekeeper A's `post`
/// creates an approval request and the receipt stays `draft`; A cannot
/// approve; procurement user B approves, the receipt is `posted` and stock is
/// +106.
#[renox::test]
async fn an_over_receipt_above_the_tolerance_waits_for_a_second_person() {
    let t = boot().await;
    let (item, store, order, po_line) = chicken(&t).await;
    let a = t.storekeeper("Amal").await;
    let b = t.procurement("Bilal").await;
    let receipt = t
        .receipt(&order, &store, vec![DraftLine::on(po_line.id, 106_000, 106_000)])
        .await;

    t.post_as(&a, &receipt).await.assert_ok();

    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Draft);
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 0);
    assert!(t.received.all().is_empty());
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::Issued);
    let requests = ApprovalRequest::pending_about(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "inventory.goods_receipt_post");
    assert_eq!(requests[0].transition, "post");
    assert_eq!(requests[0].requester_id, a.id);

    // A holds no `purchase_orders.issue`: A cannot approve.
    let refused = approvals::approve(t.state(), requests[0].id, &a, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Draft);

    let decided = approvals::approve(t.state(), requests[0].id, &b, "within reason")
        .await
        .unwrap();
    assert!(decided.is_some());

    let posted = t.reload(&receipt).await;
    assert_eq!(posted.status, GoodsReceiptStatus::Posted);
    assert_eq!(posted.posted_by, Some(a.id));
    assert!(posted.number.is_some());
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 106_000);
    let order_now = t.order_now(&order).await;
    assert_eq!(order_now.status, PurchaseOrderStatus::Received);
    assert_eq!(t.line_now(&po_line).await.accepted_qty_milli, 106_000);
    assert_eq!(t.line_now(&po_line).await.outstanding_milli(), 0);
    let events = t.received.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].lines[0].accepted_qty_milli, 106_000);
}

/// The poster never approves their own over-receipt, even holding both
/// roles.
#[renox::test]
async fn the_poster_cannot_approve_their_own_over_receipt() {
    let t = boot().await;
    let (_, store, order, po_line) = chicken(&t).await;
    let pia = t.storekeeper("Pia").await;
    pia.assign_role(t.db(), "procurement").await.unwrap();
    let receipt = t
        .receipt(&order, &store, vec![DraftLine::on(po_line.id, 110_000, 110_000)])
        .await;
    t.post_as(&pia, &receipt).await.assert_ok();
    let requests = ApprovalRequest::pending_about(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    let refused = approvals::approve(t.state(), requests[0].id, &pia, "").await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Draft);
}

/// A rejection returns the receipt for correction: it stays a draft.
#[renox::test]
async fn a_rejected_over_receipt_stays_a_draft() {
    let t = boot().await;
    let (item, store, order, po_line) = chicken(&t).await;
    let a = t.storekeeper("Amal").await;
    let b = t.procurement("Bilal").await;
    let receipt = t
        .receipt(&order, &store, vec![DraftLine::on(po_line.id, 120_000, 120_000)])
        .await;
    t.post_as(&a, &receipt).await.assert_ok();
    let requests = ApprovalRequest::pending_about(t.db(), &receipt.subject())
        .await
        .unwrap();
    approvals::reject(t.state(), requests[0].id, &b, "too much")
        .await
        .unwrap();
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Draft);
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 0);
    // Corrected to the order, it posts without approval.
    let stored = lines_of(t.db(), receipt.id).await.unwrap();
    wl_inventory::goods_receipt::update_line(
        t.db(),
        receipt.id,
        stored[0].id,
        &DraftLine::on(po_line.id, 100_000, 100_000),
    )
    .await
    .unwrap();
    t.post_as(&a, &receipt).await.assert_ok();
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
}

/// Done when 4: accepted 105 of 100 is the tolerance itself: no approval.
#[renox::test]
async fn an_over_receipt_within_the_tolerance_needs_no_approval() {
    let t = boot().await;
    let (item, store, order, po_line) = chicken(&t).await;
    let a = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(&order, &store, vec![DraftLine::on(po_line.id, 105_000, 105_000)])
        .await;
    t.post_as(&a, &receipt).await.assert_ok();
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
    assert!(
        ApprovalRequest::pending_about(t.db(), &receipt.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 105_000);
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::Received);
}

/// Rejected quantity does not count towards the tolerance: 120 received, 100
/// accepted needs no approval.
#[renox::test]
async fn rejected_quantity_does_not_count_towards_the_tolerance() {
    let t = boot().await;
    let (_, store, order, po_line) = chicken(&t).await;
    let a = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(po_line.id, 120_000, 100_000)
                .rejected_for("excess", RejectedHandling::KeptForCollection)],
        )
        .await;
    t.post_as(&a, &receipt).await.assert_ok();
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
    // Rejected excess does not change what is outstanding.
    let line = t.line_now(&po_line).await;
    assert_eq!(line.accepted_qty_milli, 100_000);
    assert_eq!(line.outstanding_milli(), 0);
}

/// Done when 5: rejected goods kept for collection make a `draft` Return to
/// supplier with no collection proof yet, and nothing of them enters stock.
#[renox::test]
async fn goods_kept_for_collection_wait_on_a_draft_return() {
    let t = boot().await;
    let (item, store, order, po_line) = chicken(&t).await;
    let a = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(po_line.id, 100_000, 80_000)
                .rejected_for("quality", RejectedHandling::KeptForCollection)
                .credit()],
        )
        .await;
    t.post_as(&a, &receipt).await.assert_ok();
    let returns = supplier_return::of_source(t.db(), &receipt.subject()).await.unwrap();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].handling, RejectedHandling::KeptForCollection);
    assert_eq!(returns[0].status, ReturnStatus::Draft);
    assert_eq!(returns[0].collection_proof, None);
    let lines = supplier_return::lines_of(t.db(), returns[0].id).await.unwrap();
    assert_eq!(lines[0].quantity_milli, 20_000);
    assert_eq!(lines[0].wanted_outcome, wl_inventory::WantedOutcome::Credit);
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 80_000);
    let moves = movement::of_source(t.db(), &receipt.subject()).await.unwrap();
    assert_eq!(moves.iter().map(|m| m.quantity_milli).sum::<i64>(), 80_000);
}

/// Mixed lines on one receipt: one return for the door-refused lines and a
/// separate draft one for the kept lines.
#[renox::test]
async fn mixed_rejected_lines_make_two_returns() {
    let t = boot().await;
    let (item, store, order, first) = chicken(&t).await;
    let rice = t.item("RICE").await;
    let mut second = PurchaseOrderLine::new(order.id, 2, rice.id, 50_000, 400);
    second.save(t.db()).await.unwrap();
    let a = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![
                DraftLine::on(first.id, 100_000, 90_000)
                    .rejected_for("temperature", RejectedHandling::RefusedAtDoor),
                DraftLine::on(second.id, 50_000, 45_000)
                    .rejected_for("damaged", RejectedHandling::KeptForCollection),
            ],
        )
        .await;
    t.post_as(&a, &receipt).await.assert_ok();
    let returns = supplier_return::of_source(t.db(), &receipt.subject()).await.unwrap();
    assert_eq!(returns.len(), 2);
    let door = returns.iter().find(|r| r.handling == RejectedHandling::RefusedAtDoor).unwrap();
    let kept = returns.iter().find(|r| r.handling == RejectedHandling::KeptForCollection).unwrap();
    assert!(door.collection_proof.is_some() && kept.collection_proof.is_none());
    assert_eq!(supplier_return::lines_of(t.db(), door.id).await.unwrap()[0].quantity_milli, 10_000);
    assert_eq!(supplier_return::lines_of(t.db(), kept.id).await.unwrap()[0].quantity_milli, 5_000);
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 90_000);
    assert_eq!(movement::on_hand(t.db(), rice.id, store.id).await.unwrap(), 45_000);
    // Each line adds to its own order line: the order is partly received.
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::PartiallyReceived);
}

/// All rejected is allowed: no movement, the order is unchanged, the return
/// is made.
#[renox::test]
async fn a_receipt_with_everything_rejected_moves_no_stock() {
    let t = boot().await;
    let (item, store, order, po_line) = chicken(&t).await;
    let a = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(po_line.id, 100_000, 0)
                .rejected_for("wrong_item", RejectedHandling::RefusedAtDoor)],
        )
        .await;
    t.post_as(&a, &receipt).await.assert_ok();
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
    assert!(movement::of_source(t.db(), &receipt.subject()).await.unwrap().is_empty());
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 0);
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::Issued);
    assert_eq!(t.line_now(&po_line).await.accepted_qty_milli, 0);
    assert_eq!(
        supplier_return::of_source(t.db(), &receipt.subject()).await.unwrap().len(),
        1
    );
    let events = t.received.all();
    assert_eq!(events[0].lines[0].accepted_qty_milli, 0);
    assert_eq!(events[0].lines[0].rejected_qty_milli, 100_000);
    assert_eq!(events[0].value, 0);
}

/// Accepted quantity of a lot-tracked item needs its lot and expiry; the lot
/// is made at `post` in the status the item says.
#[renox::test]
async fn a_lot_tracked_line_needs_its_lot_and_makes_it() {
    let t = boot().await;
    let store = t.location("STORE-1", None).await;
    let milk = t.tracked_item("MILK").await;
    let (order, po_line) = t.order(&store, &milk, 100_000, 500).await;
    let a = t.storekeeper("Amal").await;
    let bare = t
        .receipt(&order, &store, vec![DraftLine::on(po_line.id, 100_000, 100_000)])
        .await;
    t.post_as(&a, &bare)
        .await
        .assert_status(422)
        .assert_see("lot number");
    assert_eq!(t.reload(&bare).await.status, GoodsReceiptStatus::Draft);

    let stored = lines_of(t.db(), bare.id).await.unwrap();
    wl_inventory::goods_receipt::update_line(
        t.db(),
        bare.id,
        stored[0].id,
        &DraftLine::on(po_line.id, 100_000, 100_000).lot("L-2611", day(2027, 1, 31)),
    )
    .await
    .unwrap();
    t.post_as(&a, &bare).await.assert_ok();
    let lines = lines_of(t.db(), bare.id).await.unwrap();
    let lot = wl_inventory::Lot::find_or_404(t.db(), lines[0].lot_id.unwrap())
        .await
        .unwrap();
    assert_eq!((lot.number.as_str(), lot.item_id), ("L-2611", milk.id));
    assert_eq!(lot.expiry_date, Some(day(2027, 1, 31)));
    assert_eq!(lot.status, LotStatus::Released);
    let moves = movement::of_source(t.db(), &bare.subject()).await.unwrap();
    assert_eq!(moves[0].lot_id, Some(lot.id));
}

/// A lot of an item whose new lots start in quarantine is made there.
#[renox::test]
async fn new_lots_of_such_an_item_start_in_quarantine() {
    let t = boot().await;
    let store = t.location("STORE-1", None).await;
    let mut serum = t.tracked_item("SERUM").await;
    serum.quarantine_new_lots = true;
    serum.save(t.db()).await.unwrap();
    let (order, po_line) = t.order(&store, &serum, 10_000, 9_000).await;
    let a = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(po_line.id, 10_000, 10_000).lot("S-1", day(2027, 6, 30))],
        )
        .await;
    t.post_as(&a, &receipt).await.assert_ok();
    let lines = lines_of(t.db(), receipt.id).await.unwrap();
    let lot = wl_inventory::Lot::find_or_404(t.db(), lines[0].lot_id.unwrap())
        .await
        .unwrap();
    assert_eq!(lot.status, LotStatus::Quarantine);
}

/// The moving weighted average cost across receipts.
#[renox::test]
async fn the_average_cost_follows_the_receipts() {
    let t = boot().await;
    let store = t.location("STORE-1", None).await;
    let item = t.item("CEMENT").await;
    let (first_order, first_line) = t.order(&store, &item, 100_000, 1_000).await;
    let a = t.storekeeper("Amal").await;
    let first = t
        .receipt(&first_order, &store, vec![DraftLine::on(first_line.id, 100_000, 100_000)])
        .await;
    t.post_as(&a, &first).await.assert_ok();
    let (second_order, second_line) = t.order(&store, &item, 100_000, 1_200).await;
    let second = t
        .receipt(&second_order, &store, vec![DraftLine::on(second_line.id, 100_000, 100_000)])
        .await;
    t.post_as(&a, &second).await.assert_ok();
    let after = wl_inventory::Item::find_or_404(t.db(), item.id).await.unwrap();
    assert_eq!(after.average_cost, 1_100);
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 200_000);
}

/// A receipt is posted by a holder of `goods_receipts.post` at the receiving
/// location (`STK.ACL.04`); a second post is refused.
#[renox::test]
async fn only_a_storekeeper_of_the_location_posts_and_only_once() {
    let t = boot().await;
    let site = Subject::new("construction.project", 4);
    let other = Subject::new("construction.project", 5);
    let store = t.location("SITE-4", Some(&site)).await;
    let item = t.item("STEEL").await;
    let (order, po_line) = t.order(&store, &item, 10_000, 3_000).await;
    let receipt = t
        .receipt(&order, &store, vec![DraftLine::on(po_line.id, 10_000, 10_000)])
        .await;
    let outsider = t.storekeeper_in("Omar", &other).await;
    t.post_as(&outsider, &receipt).await.assert_status(403);
    let nobody = t.user_with("Nadia", "procurement").await;
    t.post_as(&nobody, &receipt).await.assert_status(403);
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Draft);

    let local = t.storekeeper_in("Sami", &site).await;
    t.post_as(&local, &receipt).await.assert_ok();
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
    t.post_as(&local, &receipt).await.assert_status(422);
    assert_eq!(movement::on_hand(t.db(), item.id, store.id).await.unwrap(), 10_000);
}

/// An order that is not open for receiving refuses the receipt.
#[renox::test]
async fn a_receipt_against_a_draft_order_is_refused() {
    let t = boot().await;
    let store = t.location("STORE-1", None).await;
    let item = t.item("PIPE").await;
    let mut order = wl_inventory::PurchaseOrder::new(support::SUPPLIER, store.id);
    order.save(t.db()).await.unwrap();
    let mut po_line = PurchaseOrderLine::new(order.id, 1, item.id, 10_000, 100);
    po_line.save(t.db()).await.unwrap();
    let a = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(&order, &store, vec![DraftLine::on(po_line.id, 10_000, 10_000)])
        .await;
    t.post_as(&a, &receipt)
        .await
        .assert_status(422)
        .assert_see("not open");
}

/// The module's checks on the draft itself: an unknown order line is refused
/// when the draft is recorded.
#[renox::test]
async fn a_draft_names_lines_of_its_order() {
    let t = boot().await;
    let (_, store, order, _) = chicken(&t).await;
    let refused = wl_inventory::goods_receipt::create_draft(
        t.db(),
        None,
        wl_inventory::DraftReceipt {
            purchase_order_id: order.id,
            location_id: store.id,
            delivery_note: None,
            lines: vec![DraftLine::on(9_999, 1_000, 1_000)],
        },
    )
    .await;
    assert!(matches!(refused, Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))));
}
