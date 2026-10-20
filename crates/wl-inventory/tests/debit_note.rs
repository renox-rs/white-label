//! A dispatched Return to supplier reaches wl-invoicing (`FIN.RUL.17`, issue
//! #222): the event wl-inventory emits is the one wl-invoicing listens for,
//! so the debit note is drafted without any glue in between. Both real
//! modules run in one app.

mod support;

use renox::prelude::*;
use wl_invoicing::{DebitNoteOrigin, SupplierDebitNote};
use wl_kernel::inventory::PURCHASE_ORDER_LINE_SUBJECT_TYPE;

use support::boot_with_invoicing;

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
    let source = ret.source();
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
