//! Acceptance checks for the credit note numbers, the status guard and the
//! invoices a credit note refuses (#71 AC2 and AC3).

mod support;

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::credit::credit_in_full;
use wl_invoicing::credit_notes::{self, By};
use wl_invoicing::{CreditNote, CreditNoteKind};
use wl_kernel::invoicing::{CreditLine, CreditNoteStatus, InvoiceStatus, VatCategory};

use support::boot;

const LINES: [(i64, VatCategory); 2] = [
    (100_000, VatCategory::Standard),
    (50_000, VatCategory::ZeroRated),
];

fn is_refused<T>(result: std::result::Result<T, TransitionError>) -> bool {
    match result {
        Err(error) => matches!(
            error.into_error(),
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)
        ),
        Ok(_) => false,
    }
}

/// AC2: the numbers of entity PGP run `CN-PGP-<year>-0001`, `-0002`, apart
/// from the invoice numbers, and an issue that rolls back leaves no gap.
#[renox::test]
async fn the_numbers_are_gapless_per_entity_and_year() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let year = renox::db::now().format("%Y").to_string();

    // The issue fails after the number is taken (the ledger refuses the
    // allocation's movement), so the whole transaction rolls back.
    renox::db::sql(
        "CREATE TRIGGER refuse_movements BEFORE INSERT ON wl_invoicing_credit_note_movements \
         BEGIN SELECT RAISE(ABORT, 'refused for the test'); END",
    )
    .execute(t.db())
    .await
    .unwrap();
    let draft = credit_notes::draft(t.db(), &invoice, &[CreditLine::new(1, 20_000)], "a", None)
        .await
        .unwrap();
    assert!(
        credit_notes::issue(t.state(), By::System(None), &draft)
            .await
            .is_err()
    );
    let after = CreditNote::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(after.status, CreditNoteStatus::Draft);
    assert_eq!(after.number, None);
    renox::db::sql("DROP TRIGGER refuse_movements")
        .execute(t.db())
        .await
        .unwrap();

    let first = credit_notes::issue(t.state(), By::System(None), &draft)
        .await
        .unwrap();
    assert_eq!(first.number, Some(format!("CN-PGP-{year}-0001")));
    let second = credit_notes::credit_partly(
        t.state(),
        invoice.id,
        &[CreditLine::new(2, 5_000)],
        "b",
        By::System(None),
    )
    .await
    .unwrap();
    assert_eq!(second.number, Some(format!("CN-PGP-{year}-0002")));
    // The invoice's own number is not touched.
    let invoice_after = t.reload(&invoice).await;
    assert_eq!(invoice_after.number, invoice.number);
    assert!(
        invoice_after
            .number
            .as_deref()
            .is_some_and(|n| !n.starts_with("CN-"))
    );
}

/// AC2: a credit note cannot be inserted outside `draft`, and saving a
/// changed status outside a transition is refused.
#[renox::test]
async fn the_status_changes_only_through_a_transition() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    for status in [CreditNoteStatus::Issued, CreditNoteStatus::Applied] {
        let mut note = CreditNote {
            invoice_id: invoice.id,
            party_id: invoice.party_id,
            currency: "AED".into(),
            reason: "x".into(),
            kind: CreditNoteKind::Partial,
            status,
            ..Default::default()
        };
        assert!(note.insert(t.db()).await.is_err(), "{status}");
    }
    assert_eq!(CreditNote::query().count(t.db()).await.unwrap(), 0);

    let mut draft = credit_notes::draft(t.db(), &invoice, &[CreditLine::new(1, 1_000)], "x", None)
        .await
        .unwrap();
    draft.status = CreditNoteStatus::Applied;
    assert!(draft.save(t.db()).await.is_err());
    let reloaded = CreditNote::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(reloaded.status, CreditNoteStatus::Draft);
}

/// AC3: a blank reason, no lines, a line above its remaining credit, and an
/// invoice in `draft`, `credited` or `written_off` are refused; the
/// invoice's lines, totals and VAT stay as they were.
#[renox::test]
async fn bad_credits_are_refused_and_the_invoice_is_untouched() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let ok = [CreditLine::new(1, 20_000)];

    // 800.00 net is left on the first line after the first credit note.
    let first = credit_notes::credit_partly(t.state(), invoice.id, &ok, "a", By::System(None))
        .await
        .unwrap();
    assert_eq!((first.vat_total, first.gross_total), (1_000, 21_000));
    let over = [CreditLine::new(1, 80_001)];
    assert!(is_refused(
        credit_notes::credit_partly(t.state(), invoice.id, &over, "b", By::System(None)).await
    ));
    assert!(is_refused(
        credit_notes::credit_partly(t.state(), invoice.id, &ok, " ", By::System(None)).await
    ));
    assert!(is_refused(
        credit_notes::credit_partly(t.state(), invoice.id, &[], "c", By::System(None)).await
    ));
    let unissued = t.draft(&LINES, None).await;
    assert!(is_refused(
        credit_notes::credit_partly(t.state(), unissued.id, &ok, "d", By::System(None)).await
    ));

    // A credited invoice and a written-off one.
    let other = t.issued(&LINES, None, &clerk).await;
    credit_in_full(t.state(), &other, "Billed twice", false, By::System(None))
        .await
        .unwrap();
    assert_eq!(t.reload(&other).await.status, InvoiceStatus::Credited);
    assert!(is_refused(
        credit_notes::credit_partly(t.state(), other.id, &ok, "e", By::System(None)).await
    ));
    let (written_off, _) = t.written_off(None, t.day(-60), None).await;
    assert_eq!(
        t.reload(&written_off).await.status,
        InvoiceStatus::WrittenOff
    );
    assert!(is_refused(
        credit_notes::credit_partly(t.state(), written_off.id, &ok, "f", By::System(None)).await
    ));

    // Only the first credit note and the full one exist, and the invoice's
    // document is as it was.
    assert_eq!(CreditNote::query().count(t.db()).await.unwrap(), 2);
    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.net_total, after.vat_total, after.gross_total),
        (invoice.net_total, invoice.vat_total, invoice.gross_total)
    );
    assert_eq!(after.number, invoice.number);
}
