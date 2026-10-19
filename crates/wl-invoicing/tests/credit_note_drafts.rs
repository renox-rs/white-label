//! Drafting, changing and discarding a credit note (wl-invoicing §5.12,
//! #71 part 2): per-line VAT, the remaining credit of a line, and a draft
//! with no number and no effect on the invoice.

mod support;

use renox::prelude::*;
use wl_invoicing::credit_notes;
use wl_invoicing::{CreditNote, CreditNoteKind, CreditNoteLine};
use wl_kernel::invoicing::{CreditLine, CreditNoteStatus, VatCategory};

use support::boot;

const LINES: [(i64, VatCategory); 2] = [
    (100_000, VatCategory::Standard),
    (50_000, VatCategory::ZeroRated),
];

fn is_refused<T>(result: Result<T>) -> bool {
    matches!(
        result,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    )
}

#[renox::test]
async fn a_draft_credit_note_has_per_line_vat_and_no_effect() {
    let t = boot().await;
    let user = t.user_a().await;
    let invoice = t.issued(&LINES, None, &user).await;

    let note = credit_notes::draft(
        t.db(),
        &invoice,
        &[CreditLine::new(1, 20_000)],
        " Damaged goods ",
        Some(user.id),
    )
    .await
    .unwrap();
    assert_eq!(note.status, CreditNoteStatus::Draft);
    assert_eq!(note.kind, CreditNoteKind::Partial);
    assert_eq!(note.number, None);
    assert_eq!(note.reason, "Damaged goods");
    assert_eq!(
        (note.invoice_id, note.party_id),
        (invoice.id, invoice.party_id)
    );
    assert_eq!(note.legal_entity_id, invoice.legal_entity_id);
    assert_eq!(
        (note.net_total, note.vat_total, note.gross_total),
        (20_000, 1_000, 21_000)
    );
    assert_eq!(note.remaining, 0);

    let lines = credit_notes::lines_of(t.db(), note.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].vat_category, VatCategory::Standard);
    assert_eq!(lines[0].gross_amount, 21_000);

    // The invoice is untouched.
    let after = t.reload(&invoice).await;
    assert_eq!(after.status, invoice.status);
    assert_eq!(
        (
            after.net_total,
            after.vat_total,
            after.gross_total,
            after.balance
        ),
        (
            invoice.net_total,
            invoice.vat_total,
            invoice.gross_total,
            invoice.balance
        )
    );
}

#[renox::test]
async fn bad_drafts_are_refused() {
    let t = boot().await;
    let user = t.user_a().await;
    let invoice = t.issued(&LINES, None, &user).await;
    let ok = [CreditLine::new(1, 20_000)];

    assert!(is_refused(
        credit_notes::draft(t.db(), &invoice, &ok, "  ", None).await
    ));
    assert!(is_refused(
        credit_notes::draft(t.db(), &invoice, &[], "x", None).await
    ));
    let over = [CreditLine::new(1, 100_001)];
    assert!(is_refused(
        credit_notes::draft(t.db(), &invoice, &over, "x", None).await
    ));
    // An invoice that is still a draft cannot be credited.
    let unissued = t.draft(&LINES, None).await;
    assert!(is_refused(
        credit_notes::draft(t.db(), &unissued, &ok, "x", None).await
    ));
    assert_eq!(CreditNote::query().count(t.db()).await.unwrap(), 0);
}

#[renox::test]
async fn a_draft_is_changed_and_discarded() {
    let t = boot().await;
    let user = t.user_a().await;
    let invoice = t.issued(&LINES, None, &user).await;
    let note = credit_notes::draft(t.db(), &invoice, &[CreditLine::new(1, 20_000)], "a", None)
        .await
        .unwrap();

    let changed = credit_notes::change(
        t.db(),
        &note,
        &[CreditLine::new(1, 10_000), CreditLine::new(2, 50_000)],
        "b",
    )
    .await
    .unwrap();
    assert_eq!(changed.reason, "b");
    assert_eq!(
        (changed.net_total, changed.vat_total, changed.gross_total),
        (60_000, 500, 60_500)
    );
    let lines = credit_notes::lines_of(t.db(), note.id).await.unwrap();
    assert_eq!(lines.iter().map(|l| l.position).collect::<Vec<_>>(), [1, 2]);
    assert!(is_refused(
        credit_notes::change(t.db(), &note, &[CreditLine::new(1, 999_999)], "c").await
    ));
    // The failed change left the lines as they were.
    assert_eq!(
        credit_notes::lines_of(t.db(), note.id).await.unwrap().len(),
        2
    );

    credit_notes::discard(t.db(), &note).await.unwrap();
    assert!(CreditNote::find(t.db(), note.id).await.unwrap().is_none());
    assert_eq!(CreditNoteLine::query().count(t.db()).await.unwrap(), 0);
}
