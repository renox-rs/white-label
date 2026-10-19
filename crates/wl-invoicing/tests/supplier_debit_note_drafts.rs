//! Supplier debit note entities and drafts (wl-invoicing §3, #76 part 2): the
//! manual draft, its per-line VAT, the remaining-quantity check, change and
//! discard, and the four-state machine's guard rails. The issue effects come
//! with the issue part.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::debit_notes::{self, DebitLine};
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{
    BillLine, BillLineDraft, DebitNoteLine, DebitNoteOrigin, PricedAt, SupplierBill,
    SupplierBillStatus, SupplierDebitNote,
};
use wl_kernel::Subject;
use wl_kernel::invoicing::{SupplierDebitNoteStatus, VatCategory};

use support::{BLUE_DUNES, Booted, boot};

const A: i64 = 7;
const B: i64 = 8;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

/// An approved bill of 10 units at AED 100.00 + 5% VAT (one line, on a PO
/// line, coded to a cost object).
async fn approved_bill(t: &Booted) -> (SupplierBill, BillLine) {
    let line = BillLineDraft::new("Pump", 10_000, VatCategory::Standard)
        .quantity_milli(10_000)
        .po_line(Subject::new("procurement.po_line", 31))
        .cost_object(Subject::new("property.unit", 4));
    let draft = BillDraft::new(BLUE_DUNES, "INV-9", day(5)).line(line);
    let bill = supplier_bill::create(t.db(), &draft, Some(A))
        .await
        .unwrap();
    submit(t.state(), By::System(Some(A)), bill.id)
        .await
        .unwrap();
    let bill = approve(t.state(), By::System(Some(B)), bill.id, "")
        .await
        .unwrap();
    let line = supplier_bill::lines_of(t.db(), bill.id)
        .await
        .unwrap()
        .remove(0);
    (bill, line)
}

fn is_422<T: std::fmt::Debug>(result: Result<T>) -> bool {
    matches!(
        result,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    )
}

async fn force_status(t: &Booted, note: &SupplierDebitNote, status: &str) {
    renox::db::sql(format!(
        "UPDATE {} SET status = ? WHERE id = ?",
        SupplierDebitNote::TABLE
    ))
    .bind(status)
    .bind(note.id)
    .execute(t.db())
    .await
    .unwrap();
}

#[renox::test]
async fn a_bill_line_keeps_its_po_line() {
    let t = boot().await;
    let (_, line) = approved_bill(&t).await;
    assert_eq!(
        line.po_line(),
        Some(Subject::new("procurement.po_line", 31))
    );
}

#[renox::test]
async fn a_manual_draft_has_per_line_vat_and_leaves_the_bill_alone() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let note = debit_notes::draft_manual(
        t.db(),
        &bill,
        "short_delivery",
        "",
        &[DebitLine::new(line.id, 3_000, 10_000)],
        Some(A),
    )
    .await
    .unwrap();
    assert_eq!(note.status, SupplierDebitNoteStatus::Draft);
    assert_eq!(note.number, None);
    assert_eq!(note.origin, DebitNoteOrigin::Manual);
    assert_eq!(note.bill_id, Some(bill.id));
    assert_eq!(note.party_id, BLUE_DUNES.0);
    assert_eq!(note.legal_entity_id, bill.legal_entity_id.unwrap());
    assert_eq!(note.reason_code.as_deref(), Some("short_delivery"));
    assert_eq!(note.created_by, Some(A));
    assert_eq!(
        (note.net_total, note.vat_total, note.gross_total),
        (30_000, 1_500, 31_500)
    );
    assert_eq!(note.vat_total_aed, Some(1_500));
    assert_eq!((note.applied_to_bill, note.debit_balance), (0, 0));
    let lines = debit_notes::lines_of(t.db(), note.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].bill_line_id, Some(line.id));
    assert_eq!(lines[0].priced_at, PricedAt::Bill);
    assert_eq!(lines[0].net_amount_aed, Some(30_000));
    assert_eq!(
        lines[0].cost_object(),
        Some(Subject::new("property.unit", 4))
    );
    assert_eq!(
        lines[0].po_line(),
        Some(Subject::new("procurement.po_line", 31))
    );
    // The bill is untouched.
    let after = SupplierBill::find_or_404(t.db(), bill.id).await.unwrap();
    assert_eq!(after.status, SupplierBillStatus::Approved);
    assert_eq!(
        (after.gross_total, after.balance, after.vat_total),
        (bill.gross_total, bill.balance, bill.vat_total)
    );
    let bill_line = supplier_bill::lines_of(t.db(), bill.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(bill_line.net_amount, line.net_amount);
}

#[renox::test]
async fn a_manual_draft_is_refused_for_the_wrong_bill_or_reason_or_lines() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let one = [DebitLine::new(line.id, 1_000, 10_000)];
    // A bill that is not approved.
    let other = BillDraft::new(BLUE_DUNES, "INV-10", day(6)).line(BillLineDraft::new(
        "Fee",
        1_000,
        VatCategory::Standard,
    ));
    let draft_bill = supplier_bill::create(t.db(), &other, Some(A))
        .await
        .unwrap();
    assert!(is_422(
        debit_notes::draft_manual(t.db(), &draft_bill, "quality", "", &one, Some(A)).await
    ));
    // Reasons.
    assert!(is_422(
        debit_notes::draft_manual(t.db(), &bill, "other", "  ", &one, Some(A)).await
    ));
    assert!(is_422(
        debit_notes::draft_manual(t.db(), &bill, "bogus", "x", &one, Some(A)).await
    ));
    assert!(
        debit_notes::draft_manual(t.db(), &bill, "other", "Lost", &one, Some(A))
            .await
            .is_ok()
    );
    // Lines.
    assert!(is_422(
        debit_notes::draft_manual(t.db(), &bill, "quality", "", &[], Some(A)).await
    ));
    for bad in [
        DebitLine::new(line.id + 99, 1_000, 10_000),
        DebitLine::new(line.id, 0, 10_000),
        DebitLine::new(line.id, 1_000, 0),
    ] {
        assert!(is_422(
            debit_notes::draft_manual(t.db(), &bill, "quality", "", &[bad], Some(A)).await
        ));
    }
}

#[renox::test]
async fn a_line_may_not_exceed_what_issued_notes_left_on_the_bill_line() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let take = |qty: i64| [DebitLine::new(line.id, qty, 10_000)];
    let first = debit_notes::draft_manual(t.db(), &bill, "quality", "", &take(3_000), Some(A))
        .await
        .unwrap();
    // A draft does not count: 8 of 10 is fine while the 3 is only a draft.
    let eight = debit_notes::draft_manual(t.db(), &bill, "quality", "", &take(8_000), Some(A))
        .await
        .unwrap();
    discard_ok(&t, eight.id).await;
    force_status(&t, &first, "issued").await;
    // 3 issued: 8 is refused, 7 accepted; two drafts of 7 can exist.
    assert!(is_422(
        debit_notes::draft_manual(t.db(), &bill, "quality", "", &take(8_000), Some(A)).await
    ));
    let seven = debit_notes::draft_manual(t.db(), &bill, "quality", "", &take(7_000), Some(A))
        .await
        .unwrap();
    let again = debit_notes::draft_manual(t.db(), &bill, "quality", "", &take(7_000), Some(A))
        .await
        .unwrap();
    // The check is repeatable at issue: once `seven` is issued, `again` fails.
    let bill_lines = supplier_bill::lines_of(t.db(), bill.id).await.unwrap();
    let again_lines = debit_notes::lines_of(t.db(), again.id).await.unwrap();
    let before =
        debit_notes::caps_problem(t.db(), bill.id, &bill_lines, &again_lines, Some(again.id))
            .await
            .unwrap();
    assert_eq!(before, None);
    force_status(&t, &seven, "issued").await;
    let after =
        debit_notes::caps_problem(t.db(), bill.id, &bill_lines, &again_lines, Some(again.id))
            .await
            .unwrap();
    assert!(after.unwrap().contains("line 1"));
    // A cancelled note frees its quantity again.
    force_status(&t, &seven, "cancelled").await;
    let freed =
        debit_notes::caps_problem(t.db(), bill.id, &bill_lines, &again_lines, Some(again.id))
            .await
            .unwrap();
    assert_eq!(freed, None);
}

#[renox::test]
async fn the_net_may_not_exceed_what_is_left_either() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    // One unit at AED 150.00 is above the line's AED 100.00 unit price: the
    // quantity fits, but 10 x 150.00 is above the net of AED 1,000.00.
    let result = debit_notes::draft_manual(
        t.db(),
        &bill,
        "price_difference",
        "",
        &[DebitLine::new(line.id, 10_000, 15_000)],
        Some(A),
    )
    .await;
    assert!(is_422(result));
    let ok = debit_notes::draft_manual(
        t.db(),
        &bill,
        "price_difference",
        "",
        &[DebitLine::new(line.id, 10_000, 500)],
        Some(A),
    )
    .await
    .unwrap();
    assert_eq!(ok.net_total, 5_000);
}

async fn discard_ok(t: &Booted, id: i64) {
    debit_notes::discard(t.db(), id).await.unwrap();
}

#[renox::test]
async fn a_draft_is_changed_and_discarded_without_any_effect() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let note = debit_notes::draft_manual(
        t.db(),
        &bill,
        "quality",
        "",
        &[DebitLine::new(line.id, 3_000, 10_000)],
        Some(A),
    )
    .await
    .unwrap();
    let changed = debit_notes::change(
        t.db(),
        note.id,
        "other",
        " Cracked ",
        &[DebitLine::new(line.id, 2_000, 10_000)],
    )
    .await
    .unwrap();
    assert_eq!(
        (changed.reason_code.as_deref(), changed.reason.as_str()),
        (Some("other"), "Cracked")
    );
    assert_eq!(changed.gross_total, 21_000);
    let lines = debit_notes::lines_of(t.db(), note.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].quantity_milli, 2_000);
    // A refused change keeps the draft as it was.
    assert!(is_422(
        debit_notes::change(
            t.db(),
            note.id,
            "other",
            "",
            &[DebitLine::new(line.id, 2_000, 10_000)]
        )
        .await
    ));
    assert!(is_422(
        debit_notes::change(
            t.db(),
            note.id,
            "quality",
            "",
            &[DebitLine::new(line.id, 11_000, 10_000)]
        )
        .await
    ));
    let kept = SupplierDebitNote::find_or_404(t.db(), note.id)
        .await
        .unwrap();
    assert_eq!(kept.gross_total, 21_000);
    // Discarding removes the note and its lines, nothing else.
    debit_notes::discard(t.db(), note.id).await.unwrap();
    assert!(
        SupplierDebitNote::find(t.db(), note.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        debit_notes::lines_of(t.db(), note.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(DebitNoteLine::query().get(t.db()).await.unwrap().len(), 0);
    let after = SupplierBill::find_or_404(t.db(), bill.id).await.unwrap();
    assert_eq!(
        (after.status, after.balance),
        (SupplierBillStatus::Approved, bill.balance)
    );
}

#[renox::test]
async fn only_a_manual_draft_is_changed_or_discarded() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let note = debit_notes::draft_manual(
        t.db(),
        &bill,
        "quality",
        "",
        &[DebitLine::new(line.id, 1_000, 10_000)],
        Some(A),
    )
    .await
    .unwrap();
    force_status(&t, &note, "issued").await;
    assert!(is_422(debit_notes::discard(t.db(), note.id).await));
    assert!(is_422(
        debit_notes::change(
            t.db(),
            note.id,
            "quality",
            "",
            &[DebitLine::new(line.id, 1_000, 1)]
        )
        .await
    ));
}

#[renox::test]
async fn a_note_is_never_inserted_outside_draft_and_its_status_is_not_saved_directly() {
    let t = boot().await;
    let (bill, line) = approved_bill(&t).await;
    let mut issued = SupplierDebitNote {
        legal_entity_id: bill.legal_entity_id.unwrap(),
        party_id: bill.party_id,
        currency: "AED".into(),
        status: SupplierDebitNoteStatus::Issued,
        ..Default::default()
    };
    assert!(issued.insert(t.db()).await.is_err());
    let mut note = debit_notes::draft_manual(
        t.db(),
        &bill,
        "quality",
        "",
        &[DebitLine::new(line.id, 1_000, 10_000)],
        Some(A),
    )
    .await
    .unwrap();
    note.status = SupplierDebitNoteStatus::Issued;
    assert!(note.save(t.db()).await.is_err());
    let kept = SupplierDebitNote::find_or_404(t.db(), note.id)
        .await
        .unwrap();
    assert_eq!(kept.status, SupplierDebitNoteStatus::Draft);
}
