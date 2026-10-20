//! A dispatched Return to supplier reaches wl-invoicing (`FIN.RUL.17`, issue
//! #222): the event wl-inventory emits is the one wl-invoicing listens for,
//! so the debit note is drafted without any glue in between. Both real
//! modules run in one app.

mod support;

use renox::prelude::*;
use wl_invoicing::{DebitNoteOrigin, SupplierDebitNote};
use wl_inventory::PurchaseOrderLine;
use wl_inventory::supplier_return::{self, ReturnStatus};
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::debit_notes;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::BillLineDraft;
use wl_kernel::documents::today;
use wl_kernel::inventory::PURCHASE_ORDER_LINE_SUBJECT_TYPE;
use wl_kernel::Subject;
use wl_kernel::invoicing::{SupplierDebitNoteId, SupplierDebitNoteStatus, VatCategory};

use support::{SUPPLIER, boot_with_invoicing, boot_with_parties};

/// Dispatching a credit return of 3 units bought at AED 120.00 drafts one
/// debit note for the return, priced at the order, with the order line.
#[renox::test]
async fn dispatching_a_credit_return_drafts_a_debit_note_in_invoicing() {
    let t = boot_with_invoicing().await;
    let item = t.item("PIPE").await;
    let store = t.location("MAIN", None).await;
    let (_order, po_line) = t.order(&store, &item, 10_000, 12_000).await;

    let ret = t
        .dispatched_credit_return(&store, &[(&item, 3_000, &po_line)])
        .await;

    let notes = SupplierDebitNote::query().get(t.db()).await.unwrap();
    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert_eq!(note.origin, DebitNoteOrigin::Return);
    assert_eq!(note.party_id, ret.supplier_party_id);
    let source = ret.subject();
    assert_eq!(
        note.source_type.as_deref(),
        Some(source.subject_type.as_str())
    );
    assert_eq!(note.source_id, Some(source.subject_id));
    assert_eq!(note.bill_id, None);
    let lines = wl_invoicing::debit_notes::lines_of(t.db(), note.id)
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].quantity_milli, 3_000);
    assert_eq!(lines[0].unit_price, 12_000);
    assert_eq!(
        lines[0].po_line().map(|s| (s.subject_type, s.subject_id)),
        Some((PURCHASE_ORDER_LINE_SUBJECT_TYPE.to_owned(), po_line.id))
    );
}

/// An approved supplier bill of `quantity_milli` of the order line at AED
/// 120.00 (`SUPPLIER`, which the real wl-parties knows), so the draft note a
/// dispatched return waits with is linked to it.
async fn approved_bill_on(
    t: &support::Booted,
    line: &PurchaseOrderLine,
    quantity_milli: i64,
) -> wl_invoicing::SupplierBill {
    let bill_line = BillLineDraft::new("Pipe", 12_000, VatCategory::Standard)
        .quantity_milli(quantity_milli)
        .po_line(Subject::new(PURCHASE_ORDER_LINE_SUBJECT_TYPE, line.id));
    let draft = BillDraft::new(SUPPLIER, "SUP-1", today()).line(bill_line);
    let bill = supplier_bill::create(t.db(), &draft, None).await.unwrap();
    submit(t.state(), By::System(None), bill.id).await.unwrap();
    approve(t.state(), By::System(None), bill.id, "")
        .await
        .unwrap()
}

/// Issuing the debit note the real module drafted for a dispatched return
/// emits the event wl-inventory listens for: the return is `credited` by the
/// note, with the number of the real note, and `SupplierReturnCredited` is
/// emitted once.
#[renox::test]
async fn an_issued_debit_note_credits_the_return() {
    let t = boot_with_parties().await;
    let item = t.item("PIPE").await;
    let store = t.location("MAIN", None).await;
    let (_order, po_line) = t.order(&store, &item, 10_000, 12_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&item, 3_000, &po_line)])
        .await;
    approved_bill_on(&t, &po_line, 10_000).await;
    let note = SupplierDebitNote::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(note.status, SupplierDebitNoteStatus::Draft);
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);

    debit_notes::issue(t.state(), By::System(None), note.id)
        .await
        .unwrap();

    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Credited);
    assert_eq!(done.credit_basis.as_deref(), Some("debit_note"));
    assert_eq!(done.debit_note_id, Some(note.id));
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].covered_milli, 3_000);
    let events = t.credited.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].debit_note, Some(SupplierDebitNoteId(note.id)));
    assert!(t.tickets.opened().is_empty());
}

/// A debit note the real module cancels (a draft here, with a reason) gives
/// procurement one task and leaves the return `dispatched` and uncovered; the
/// one issued and cancelled afterwards is the same task.
#[renox::test]
async fn a_cancelled_debit_note_opens_the_task_and_the_return_stays_dispatched() {
    let t = boot_with_parties().await;
    let item = t.item("PIPE").await;
    let store = t.location("MAIN", None).await;
    let (_order, po_line) = t.order(&store, &item, 10_000, 12_000).await;
    let ret = t
        .dispatched_credit_return(&store, &[(&item, 3_000, &po_line)])
        .await;
    approved_bill_on(&t, &po_line, 10_000).await;
    let note = SupplierDebitNote::query().first(t.db()).await.unwrap().unwrap();

    wl_invoicing::debit_note_settle::cancel(
        t.state(),
        By::System(None),
        note.id,
        "The return was cancelled",
    )
    .await
    .unwrap();

    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].covered_milli, 0);
    let tickets = t.tickets.opened();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].subject, ret.subject());
    assert_eq!(
        tickets[0].source_key,
        Some(format!("inventory.debit_note_cancelled:{}", note.id))
    );
    assert!(t.credited.all().is_empty());
}
