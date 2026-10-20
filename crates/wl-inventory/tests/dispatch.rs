//! Dispatching a Return to supplier (wl-inventory §4 *Return to supplier*
//! `dispatch`, `STK.ACL.06`, issue #222): goods from stock leave with a
//! `return_out` movement, a return above the value waits for a second person,
//! goods refused at the door are dispatched by the receipt's `post`.

mod support;

use renox::prelude::*;
use wl_inventory::movement::{self, MovementType};
use wl_inventory::supplier_return::{self, NewReturnLine, RejectedHandling, ReturnStatus};
use wl_inventory::{DraftLine, Lot, LotStatus, PurchaseOrder, PurchaseOrderStatus, WantedOutcome};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::ReturnOutcome;

use support::{Booted, boot};

/// An item and a store holding `quantity_milli` of a lot at AED 50.00 a unit
/// (receipt cost), the item's average cost being AED 48.00.
async fn quarantined(
    t: &Booted,
    quantity_milli: i64,
) -> (wl_inventory::Item, wl_inventory::Location, Lot) {
    let mut item = t.item("PIPE").await;
    item.average_cost = 4_800;
    item.save(t.db()).await.unwrap();
    let store = t.location("QUARANTINE-1", None).await;
    let lot = t.lot(&item, "Q-1", LotStatus::Quarantine).await;
    t.stock_in(&item, &store, Some(&lot), quantity_milli, 5_000)
        .await;
    (item, store, lot)
}

/// Done when 1: a draft return of 10 units from quarantine (receipt cost AED
/// 50, average AED 48) with outcome `credit`, dispatched with a driver
/// signature: -10 `return_out` at AED 50, AED 20 to the price variance, a
/// return note PDF and `SupplierReturnDispatched`.
#[renox::test]
async fn goods_from_quarantine_leave_at_the_receipt_cost_with_a_return_note() {
    let t = boot().await;
    let (item, store, lot) = quarantined(&t, 25_000).await;
    let ret = t
        .stock_return(
            &store,
            vec![
                NewReturnLine::of(item.id, 10_000, "damaged")
                    .lot(lot.id)
                    .credit()
                    .at_cost(5_000),
            ],
        )
        .await;
    assert_eq!(ret.status, ReturnStatus::Draft);
    assert!(ret.number.as_deref().is_some_and(|n| n.starts_with("RTS-")));
    let clerk = t.procurement("Cara").await;

    t.dispatch_as(&clerk, &ret, "driver_signature", "SIG-77")
        .await
        .assert_ok();

    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Dispatched);
    assert_eq!(done.dispatched_by, Some(clerk.id));
    assert!(done.dispatched_at.is_some());
    assert_eq!(
        done.collection_proof.as_deref(),
        Some("driver_signature:SIG-77")
    );
    // -10 return_out at the receipt's cost.
    let moves = movement::of_source(t.db(), &done.subject()).await.unwrap();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].movement_type, MovementType::ReturnOut);
    assert_eq!(moves[0].quantity_milli, 10_000);
    assert_eq!((moves[0].unit_cost, moves[0].value), (5_000, 50_000));
    assert_eq!(moves[0].from_location_id, Some(store.id));
    assert_eq!(moves[0].lot_id, Some(lot.id));
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        15_000
    );
    // (50 - 48) x 10 = AED 20 to the price variance.
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].variance, 2_000);
    // The return note.
    let note = done.return_note.clone().expect("a return note");
    assert!(note.starts_with("%PDF-"));
    assert!(note.contains(done.number.as_deref().unwrap()));
    assert!(note.contains("driver_signature:SIG-77"));
    assert!(note.contains("PIPE"));
    // The event.
    let events = t.dispatched.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].supplier_return, done.subject());
    assert_eq!(events[0].supplier.0, support::SUPPLIER.0);
    assert_eq!(events[0].source.to_string(), "inventory.stock_hold:1");
    let line = &events[0].lines[0];
    assert_eq!((line.quantity_milli, line.po_price), (10_000, 5_000));
    assert_eq!(line.wanted_outcome, ReturnOutcome::Credit);
    assert_eq!(line.reason, "damaged");
    assert_eq!(
        line.lot.as_ref().map(ToString::to_string),
        Some(format!("inventory.lot:{}", lot.id))
    );
    // Dispatched is not dispatched twice.
    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_status(422);
    assert_eq!(t.dispatched.all().len(), 1);
}

/// Done when 2: a return worth AED 6,000 (limit 5,000): creator A's dispatch
/// creates an approval request and the return stays `draft`; A cannot
/// approve; B approves, the return is `dispatched`.
#[renox::test]
async fn a_return_above_the_value_waits_for_a_second_person() {
    let t = boot().await;
    let (item, store, lot) = quarantined(&t, 120_000).await;
    let ret = t
        .stock_return(
            &store,
            vec![
                NewReturnLine::of(item.id, 120_000, "wrong_item")
                    .lot(lot.id)
                    .credit()
                    .at_cost(5_000),
            ],
        )
        .await;
    let a = t.procurement("Amal").await;
    let b = t.procurement("Bilal").await;

    t.dispatch_as(&a, &ret, "courier", "C-9").await.assert_ok();

    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Draft);
    assert!(
        movement::of_source(t.db(), &ret.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.dispatched.all().is_empty());
    let requests = ApprovalRequest::pending_about(t.db(), &ret.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "inventory.supplier_return_dispatch");
    assert_eq!(requests[0].transition, "dispatch");
    assert_eq!(requests[0].requester_id, a.id);

    // The creator cannot approve their own request.
    let refused = approvals::approve(t.state(), requests[0].id, &a, "").await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Draft);

    let decided = approvals::approve(t.state(), requests[0].id, &b, "agreed")
        .await
        .unwrap();
    assert!(decided.is_some());
    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Dispatched);
    // The proof the requester gave and the requester travel with the request.
    assert_eq!(done.collection_proof.as_deref(), Some("courier:C-9"));
    assert_eq!(done.dispatched_by, Some(a.id));
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        0
    );
    assert_eq!(t.dispatched.all().len(), 1);
}

/// A return exactly at the value needs no approval.
#[renox::test]
async fn a_return_at_the_value_needs_no_approval() {
    let t = boot().await;
    let (item, store, lot) = quarantined(&t, 100_000).await;
    // 100 units at AED 50 is AED 5,000, not above it.
    let ret = t
        .stock_return(
            &store,
            vec![
                NewReturnLine::of(item.id, 100_000, "damaged")
                    .lot(lot.id)
                    .credit()
                    .at_cost(5_000),
            ],
        )
        .await;
    let a = t.procurement("Amal").await;
    t.dispatch_as(&a, &ret, "courier", "C-1").await.assert_ok();
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    assert!(
        ApprovalRequest::pending_about(t.db(), &ret.subject())
            .await
            .unwrap()
            .is_empty()
    );
}

/// A rejected approval leaves the return a draft.
#[renox::test]
async fn a_rejected_approval_leaves_the_return_a_draft() {
    let t = boot().await;
    let (item, store, lot) = quarantined(&t, 120_000).await;
    let ret = t
        .stock_return(
            &store,
            vec![
                NewReturnLine::of(item.id, 120_000, "damaged")
                    .lot(lot.id)
                    .credit()
                    .at_cost(5_000),
            ],
        )
        .await;
    let a = t.procurement("Amal").await;
    let b = t.procurement("Bilal").await;
    t.dispatch_as(&a, &ret, "courier", "C-9").await.assert_ok();
    let requests = ApprovalRequest::pending_about(t.db(), &ret.subject())
        .await
        .unwrap();
    approvals::reject(t.state(), requests[0].id, &b, "no")
        .await
        .unwrap();
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Draft);
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        120_000
    );
}

/// Done when 3 (the quantities): a `replace` line from stock on an order
/// gives its quantity back to the order line. The order's state is in
/// `reopen.rs`.
#[renox::test]
async fn a_replacement_from_stock_is_outstanding_on_the_order_again() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, mut po_line) = t.order(&store, &item, 50_000, 5_000).await;
    po_line.accepted_qty_milli = 50_000;
    po_line.save(t.db()).await.unwrap();
    t.stock_in(&item, &store, None, 50_000, 5_000).await;
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 10_000, "damaged").replacing_on(po_line.id)],
        )
        .await;
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    // The order's price is the line's cost.
    assert_eq!(lines[0].unit_cost, 5_000);
    let clerk = t.procurement("Cara").await;

    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();

    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    let line = t.line_now(&po_line).await;
    assert_eq!(line.accepted_qty_milli, 40_000);
    assert_eq!(line.outstanding_milli(), 10_000);
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].restored_milli, 10_000);
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        40_000
    );
    assert_eq!(t.dispatched.all()[0].lines[0].po_line, Some(po_line.id));
}

/// A `credit` line from stock gives nothing back to the order.
#[renox::test]
async fn a_credit_from_stock_leaves_the_order_as_it_is() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (_, mut po_line) = t.order(&store, &item, 50_000, 5_000).await;
    po_line.accepted_qty_milli = 50_000;
    po_line.save(t.db()).await.unwrap();
    t.stock_in(&item, &store, None, 50_000, 5_000).await;
    let mut line = NewReturnLine::of(item.id, 10_000, "damaged").credit();
    line.po_line_id = Some(po_line.id);
    let ret = t.stock_return(&store, vec![line]).await;
    let clerk = t.procurement("Cara").await;
    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();
    assert_eq!(t.line_now(&po_line).await.outstanding_milli(), 0);
}

/// Done when 4: a PO line of 100 with 80 accepted and 20 rejected at receipt
/// (outstanding 20); dispatching the return of the 20 with `replace` leaves
/// the outstanding at 20, not 40, and posts no movement.
#[renox::test]
async fn goods_rejected_at_receipt_restore_nothing_and_move_no_stock() {
    let t = boot().await;
    let item = t.item("CHICKEN").await;
    let store = t.location("STORE-1", None).await;
    let (order, po_line) = t.order(&store, &item, 100_000, 1_200).await;
    let amal = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![
                DraftLine::on(po_line.id, 100_000, 80_000)
                    .rejected_for("quality", RejectedHandling::KeptForCollection),
            ],
        )
        .await;
    t.post_as(&amal, &receipt).await.assert_ok();
    let returns = supplier_return::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(returns[0].status, ReturnStatus::Draft);
    assert_eq!(t.line_now(&po_line).await.outstanding_milli(), 20_000);
    let clerk = t.procurement("Cara").await;

    t.dispatch_as(&clerk, &returns[0], "courier", "C-5")
        .await
        .assert_ok();

    assert_eq!(
        t.return_now(&returns[0]).await.status,
        ReturnStatus::Dispatched
    );
    assert_eq!(t.line_now(&po_line).await.outstanding_milli(), 20_000);
    assert_eq!(t.line_now(&po_line).await.accepted_qty_milli, 80_000);
    assert!(
        movement::of_source(t.db(), &returns[0].subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        80_000
    );
    let lines = supplier_return::lines_of(t.db(), returns[0].id)
        .await
        .unwrap();
    assert_eq!(lines[0].restored_milli, 0);
    assert_eq!(lines[0].wanted_outcome, WantedOutcome::Replace);
}

/// Done when 5: a door-refused return of AED 8,000 is dispatched by the
/// receipt's `post` at once, with no movement and no approval request.
#[renox::test]
async fn goods_refused_at_the_door_are_dispatched_by_the_system_as_the_receipt_posts() {
    let t = boot().await;
    let item = t.item("CHICKEN").await;
    let store = t.location("STORE-1", None).await;
    // 100 kg at AED 80.00 a kg is AED 8,000.
    let (order, po_line) = t.order(&store, &item, 100_000, 8_000).await;
    let amal = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![
                DraftLine::on(po_line.id, 100_000, 0)
                    .rejected_for("wrong_item", RejectedHandling::RefusedAtDoor),
            ],
        )
        .await;

    t.post_as(&amal, &receipt).await.assert_ok();

    let returns = supplier_return::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(returns.len(), 1);
    let ret = &returns[0];
    assert_eq!(ret.status, ReturnStatus::Dispatched);
    assert_eq!(ret.handling, RejectedHandling::RefusedAtDoor);
    assert_eq!(
        ret.collection_proof.as_deref(),
        Some(t.reload(&receipt).await.subject().to_string().as_str())
    );
    assert_eq!(ret.dispatched_by, Some(amal.id));
    assert!(
        ret.return_note
            .as_deref()
            .is_some_and(|n| n.starts_with("%PDF-"))
    );
    assert!(
        movement::of_source(t.db(), &ret.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        movement::of_source(t.db(), &receipt.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        ApprovalRequest::pending_about(t.db(), &ret.subject())
            .await
            .unwrap()
            .is_empty()
    );
    // Nothing came in: the whole line is still outstanding.
    assert_eq!(t.line_now(&po_line).await.outstanding_milli(), 100_000);
    // The events: the receipt, then the return.
    assert_eq!(t.received.all().len(), 1);
    let events = t.dispatched.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].supplier_return, ret.subject());
    assert_eq!(events[0].source, receipt.subject());
    assert_eq!(events[0].lines[0].po_price, 8_000);
    assert_eq!(events[0].lines[0].wanted_outcome, ReturnOutcome::Replace);
    assert_eq!(events[0].lines[0].reason, "wrong_item");
}

/// Done when 6: a kept-for-collection return of 120 units rejected at
/// receipt at the order's price of AED 50 (AED 6,000): creator A's `dispatch`
/// creates an approval request.
#[renox::test]
async fn goods_kept_for_collection_above_the_value_need_a_second_person() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, po_line) = t.order(&store, &item, 120_000, 5_000).await;
    let amal = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![
                DraftLine::on(po_line.id, 120_000, 0)
                    .rejected_for("damaged", RejectedHandling::KeptForCollection),
            ],
        )
        .await;
    t.post_as(&amal, &receipt).await.assert_ok();
    let returns = supplier_return::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    let a = t.procurement("Amal-P").await;

    t.dispatch_as(&a, &returns[0], "courier", "C-8")
        .await
        .assert_ok();

    assert_eq!(t.return_now(&returns[0]).await.status, ReturnStatus::Draft);
    let requests = ApprovalRequest::pending_about(t.db(), &returns[0].subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].requester_id, a.id);
}

/// Done when 7: the last 5 units of an `on_hand` lot returned: the lot is
/// `rejected`. Part of it returned leaves the hold as it is.
#[renox::test]
async fn the_last_units_of_a_lot_on_hold_reject_the_lot() {
    let t = boot().await;
    let item = t.item("MILK").await;
    let store = t.location("STORE-1", None).await;
    let lot = t.lot(&item, "H-1", LotStatus::OnHold).await;
    t.stock_in(&item, &store, Some(&lot), 5_000, 800).await;
    let clerk = t.procurement("Cara").await;
    let first = t
        .stock_return(
            &store,
            vec![
                NewReturnLine::of(item.id, 3_000, "expired")
                    .lot(lot.id)
                    .credit(),
            ],
        )
        .await;
    t.dispatch_as(&clerk, &first, "courier", "C-1")
        .await
        .assert_ok();
    assert_eq!(
        Lot::find_or_404(t.db(), lot.id).await.unwrap().status,
        LotStatus::OnHold
    );

    let last = t
        .stock_return(
            &store,
            vec![
                NewReturnLine::of(item.id, 2_000, "expired")
                    .lot(lot.id)
                    .credit(),
            ],
        )
        .await;
    t.dispatch_as(&clerk, &last, "courier", "C-2")
        .await
        .assert_ok();

    assert_eq!(
        Lot::find_or_404(t.db(), lot.id).await.unwrap().status,
        LotStatus::Rejected
    );
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        0
    );
}

/// A lot that is not on hold is not rejected when it is emptied.
#[renox::test]
async fn an_emptied_lot_that_was_not_on_hold_keeps_its_state() {
    let t = boot().await;
    let (item, store, lot) = quarantined(&t, 5_000).await;
    let ret = t
        .stock_return(
            &store,
            vec![
                NewReturnLine::of(item.id, 5_000, "damaged")
                    .lot(lot.id)
                    .credit(),
            ],
        )
        .await;
    let clerk = t.procurement("Cara").await;
    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();
    assert_eq!(
        Lot::find_or_404(t.db(), lot.id).await.unwrap().status,
        LotStatus::Quarantine
    );
}

/// Refusals: no proof, stock that is not there, a replacement on a closed
/// order, a user without the permission. Nothing is posted.
#[renox::test]
async fn a_dispatch_without_proof_or_stock_or_an_open_order_is_refused() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, mut po_line) = t.order(&store, &item, 50_000, 5_000).await;
    po_line.accepted_qty_milli = 50_000;
    po_line.save(t.db()).await.unwrap();
    t.stock_in(&item, &store, None, 8_000, 5_000).await;
    let clerk = t.procurement("Cara").await;

    // No proof.
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 5_000, "damaged").credit()],
        )
        .await;
    t.dispatch_as(&clerk, &ret, "courier", "  ")
        .await
        .assert_status(422)
        .assert_see("collection proof");
    t.dispatch_as(&clerk, &ret, "pigeon", "P-1")
        .await
        .assert_status(422);

    // More than is there.
    let big = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 9_000, "damaged").credit()],
        )
        .await;
    t.dispatch_as(&clerk, &big, "courier", "C-1")
        .await
        .assert_status(422)
        .assert_see("has 8 at the location");

    // A replacement on a closed order.
    PurchaseOrder::query()
        .where_(PurchaseOrder::ID.eq(order.id))
        .update(t.db(), &[("status", &PurchaseOrderStatus::Closed)])
        .await
        .unwrap();
    let closed = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 1_000, "damaged").replacing_on(po_line.id)],
        )
        .await;
    t.dispatch_as(&clerk, &closed, "courier", "C-1")
        .await
        .assert_status(422)
        .assert_see("closed");

    // A storekeeper holds no `supplier_returns.manage`.
    let amal = t.storekeeper("Amal").await;
    t.dispatch_as(&amal, &ret, "courier", "C-1")
        .await
        .assert_status(403);

    for r in [&ret, &big, &closed] {
        assert_eq!(t.return_now(r).await.status, ReturnStatus::Draft);
        assert!(
            movement::of_source(t.db(), &r.subject())
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        8_000
    );
    assert_eq!(t.line_now(&po_line).await.accepted_qty_milli, 50_000);
    assert!(t.dispatched.all().is_empty());
}

/// A return without lines, or a line without a reason, is not made.
#[renox::test]
async fn a_return_needs_lines_with_a_quantity_and_a_reason() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let none = wl_inventory::supplier_return::create_from_stock(
        t.db(),
        None,
        supplier_return::NewReturn {
            supplier: support::SUPPLIER,
            source: wl_kernel::Subject::new("inventory.stock_hold", 1),
            location_id: store.id,
            lines: vec![],
        },
    )
    .await;
    assert!(none.is_err());
    let bad = wl_inventory::supplier_return::create_from_stock(
        t.db(),
        None,
        supplier_return::NewReturn {
            supplier: support::SUPPLIER,
            source: wl_kernel::Subject::new("inventory.stock_hold", 1),
            location_id: store.id,
            lines: vec![NewReturnLine::of(item.id, 1_000, "  ")],
        },
    )
    .await;
    assert!(bad.is_err());
}
