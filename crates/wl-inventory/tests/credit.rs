//! Crediting a dispatched Return to supplier (wl-inventory §4 *Return to
//! supplier* `credit`, `STK.ACL.02`, issue #224): the supplier's debit notes
//! cover its lines one by one, procurement settles it by hand, a refused
//! credit is wastage, a cancelled debit note gives procurement a task.

mod support;

use renox::prelude::*;
use wl_inventory::supplier_return::{self, ReturnStatus};
use wl_inventory::{Item, Location, PurchaseOrder, PurchaseOrderLine};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::invoicing::SupplierDebitNoteId;

use support::{Booted, SUPPLIER, boot};

/// Two items on one order, both credited at AED 100.00 a unit.
async fn two_lines(
    t: &Booted,
    qty_a: i64,
    qty_b: i64,
) -> (
    Location,
    (Item, PurchaseOrderLine),
    (Item, PurchaseOrderLine),
    PurchaseOrder,
) {
    let a = t.item("PIPE").await;
    let b = t.item("VALVE").await;
    let store = t.location("STORE-1", None).await;
    let (order, line_a) = t.order(&store, &a, qty_a, 10_000).await;
    let mut line_b = PurchaseOrderLine::new(order.id, 2, b.id, qty_b, 10_000);
    line_b.save(t.db()).await.unwrap();
    (store, (a, line_a), (b, line_b), order)
}

/// Done when 1: credit lines A (AED 400) and B (AED 200); a debit note that
/// covers A only leaves the return `dispatched`; a second covering B makes it
/// `credited` and emits `SupplierReturnCredited`.
#[renox::test]
async fn debit_notes_cover_the_lines_and_the_last_one_credits_the_return() {
    let t = boot().await;
    let (store, (a, line_a), (b, line_b), _order) = two_lines(&t, 4_000, 2_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&a, 4_000, &line_a), (&b, 2_000, &line_b)])
        .await;
    assert_eq!(ret.status, ReturnStatus::Dispatched);

    t.debit_note_issued(&ret, 11, &[(line_a.id, 4_000)]).await;

    let now = t.return_now(&ret).await;
    assert_eq!(now.status, ReturnStatus::Dispatched);
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!((lines[0].covered_milli, lines[1].covered_milli), (4_000, 0));
    assert!(t.credited.all().is_empty());

    t.debit_note_issued(&ret, 12, &[(line_b.id, 2_000)]).await;

    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Credited);
    assert_eq!(done.credit_basis.as_deref(), Some("debit_note"));
    assert_eq!(done.debit_note_id, Some(12));
    assert_eq!(done.wastage_fils, 0);
    let events = t.credited.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].supplier_return, ret.subject());
    assert_eq!(events[0].supplier, SUPPLIER);
    assert_eq!(events[0].debit_note, Some(SupplierDebitNoteId(12)));
    assert_eq!(events[0].basis, "debit_note");
    assert!(t.costs.posted().is_empty());
}

/// A note that covers part of a line leaves the return `dispatched`; the
/// rest of the line is covered by the next one.
#[renox::test]
async fn a_debit_note_covering_part_of_a_line_leaves_the_return_dispatched() {
    let t = boot().await;
    let (store, (a, line_a), _b, _order) = two_lines(&t, 4_000, 2_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&a, 4_000, &line_a)])
        .await;

    t.debit_note_issued(&ret, 11, &[(line_a.id, 1_000)]).await;
    t.debit_note_issued(&ret, 12, &[(line_a.id, 2_000)]).await;
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].covered_milli, 3_000);

    // A note of another order line, or an unrelated one, covers nothing.
    t.debit_note_issued(&ret, 13, &[(line_a.id + 100, 1_000)])
        .await;
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);

    t.debit_note_issued(&ret, 14, &[(line_a.id, 5_000)]).await;
    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Credited);
    // Never more than the line.
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].covered_milli, 4_000);
    assert_eq!(t.credited.all().len(), 1);
}

/// Done when 2: *supplier refused* on AED 1,500 at a limit of AED 1,000
/// needs an approval and, when approved, AED 1,500 is posted as wastage.
#[renox::test]
async fn a_refused_credit_above_the_limit_is_approved_then_posted_as_wastage() {
    let t = boot().await;
    let (store, (a, line_a), _b, _order) = two_lines(&t, 15_000, 2_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&a, 15_000, &line_a)])
        .await;
    let amal = t.procurement("Amal").await;
    let bilal = t.procurement("Bilal").await;

    t.credit_as(&amal, &ret, "supplier_refused", "supplier will not credit")
        .await
        .assert_ok();

    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    assert!(t.costs.posted().is_empty());
    let requests = ApprovalRequest::pending_about(t.db(), &ret.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "inventory.supplier_return_credit");
    assert_eq!(requests[0].requester_id, amal.id);

    // The requester cannot approve.
    assert!(
        approvals::approve(t.state(), requests[0].id, &amal, "")
            .await
            .is_err()
    );
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);

    approvals::approve(t.state(), requests[0].id, &bilal, "agreed")
        .await
        .unwrap();

    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Credited);
    assert_eq!(done.credit_basis.as_deref(), Some("supplier_refused"));
    assert_eq!(
        done.credit_reason.as_deref(),
        Some("supplier will not credit")
    );
    assert_eq!(done.credited_by, Some(amal.id));
    assert_eq!(done.wastage_fils, 150_000);
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].amount_aed, 150_000);
    assert_eq!(posted[0].source, ret.subject());
    let events = t.credited.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].wastage, 150_000);
    assert_eq!(
        events[0].reason.as_deref(),
        Some("supplier will not credit")
    );
}

/// A refused credit within the limit needs no approval, and only the value
/// no note covered is wastage.
#[renox::test]
async fn a_refused_credit_within_the_limit_posts_only_the_uncovered_value() {
    let t = boot().await;
    let (store, (a, line_a), (b, line_b), _order) = two_lines(&t, 4_000, 2_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&a, 4_000, &line_a), (&b, 2_000, &line_b)])
        .await;
    let amal = t.procurement("Amal").await;
    t.debit_note_issued(&ret, 11, &[(line_a.id, 4_000)]).await;

    t.credit_as(&amal, &ret, "supplier_refused", "refused B")
        .await
        .assert_ok();

    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Credited);
    // B only: 2 units at AED 100.
    assert_eq!(done.wastage_fils, 20_000);
    assert_eq!(t.costs.posted().len(), 1);
    assert_eq!(t.costs.posted()[0].amount_aed, 20_000);
    assert!(
        ApprovalRequest::pending_about(t.db(), &ret.subject())
            .await
            .unwrap()
            .is_empty()
    );
}

/// A refund received credits the return with no wastage.
#[renox::test]
async fn a_received_refund_credits_the_return_with_no_wastage() {
    let t = boot().await;
    let (store, (a, line_a), _b, _order) = two_lines(&t, 4_000, 2_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&a, 4_000, &line_a)])
        .await;
    let amal = t.procurement("Amal").await;

    t.credit_as(&amal, &ret, "refund_received", "refund in wl-payments")
        .await
        .assert_ok();

    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Credited);
    assert_eq!(done.credit_basis.as_deref(), Some("refund_received"));
    assert_eq!(done.wastage_fils, 0);
    assert!(t.costs.posted().is_empty());
    assert_eq!(t.credited.all().len(), 1);
    assert_eq!(t.credited.all()[0].wastage, 0);
}

/// A manual credit needs a reason, a dispatched return and the permission.
#[renox::test]
async fn a_manual_credit_needs_a_reason_a_dispatched_return_and_the_permission() {
    let t = boot().await;
    let (store, (a, line_a), _b, _order) = two_lines(&t, 4_000, 2_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&a, 4_000, &line_a)])
        .await;
    let amal = t.procurement("Amal").await;
    let keeper = t.storekeeper("Sami").await;

    t.credit_as(&amal, &ret, "refund_received", " ")
        .await
        .assert_status(422);
    t.credit_as(&amal, &ret, "gift", "a reason")
        .await
        .assert_status(422);
    t.credit_as(&keeper, &ret, "refund_received", "refund")
        .await
        .assert_status(403);
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);

    // A draft is not credited.
    let item = t.item("BOLT").await;
    t.stock_in(&item, &store, None, 1_000, 100).await;
    let draft = t
        .stock_return(
            &store,
            vec![wl_inventory::supplier_return::NewReturnLine::of(
                item.id, 1_000, "bad",
            )],
        )
        .await;
    t.credit_as(&amal, &draft, "refund_received", "refund")
        .await
        .assert_status(422);
}

/// Done when 3: `SupplierDebitNoteCancelled` for a dispatched return gives
/// procurement one task and the return stays `dispatched`, what the note
/// covered being open again.
#[renox::test]
async fn a_cancelled_debit_note_gives_procurement_a_task_and_the_return_stays_dispatched() {
    let t = boot().await;
    let (store, (a, line_a), (b, line_b), _order) = two_lines(&t, 4_000, 2_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&a, 4_000, &line_a), (&b, 2_000, &line_b)])
        .await;
    t.debit_note_issued(&ret, 11, &[(line_a.id, 4_000)]).await;

    t.debit_note_cancelled(&ret, 11, &[(line_a.id, 4_000)])
        .await;

    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].covered_milli, 0);
    let tickets = t.tickets.opened();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].subject, ret.subject());
    assert_eq!(
        tickets[0].source_key.as_deref(),
        Some("inventory.debit_note_cancelled:11")
    );
    assert!(tickets[0].title.contains(ret.number.as_deref().unwrap()));
    // The same note cancelled twice is still one task.
    t.debit_note_cancelled(&ret, 11, &[(line_a.id, 4_000)])
        .await;
    assert_eq!(t.tickets.opened().len(), 1);
}

/// A debit note cancelled after `credited`: the return does not reopen,
/// procurement still gets the task.
#[renox::test]
async fn a_credited_return_does_not_reopen_when_its_debit_note_is_cancelled() {
    let t = boot().await;
    let (store, (a, line_a), _b, _order) = two_lines(&t, 4_000, 2_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&a, 4_000, &line_a)])
        .await;
    t.debit_note_issued(&ret, 11, &[(line_a.id, 4_000)]).await;
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Credited);

    t.debit_note_cancelled(&ret, 11, &[(line_a.id, 4_000)])
        .await;

    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Credited);
    assert_eq!(t.tickets.opened().len(), 1);
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].covered_milli, 4_000);
}
