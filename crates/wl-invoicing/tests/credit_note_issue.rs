//! Issuing a credit note, applying it and crediting an invoice in full
//! (wl-invoicing §4, #71 part 3): the gapless number, the allocation to the
//! invoice, `apply`, and the invoice's `credit`.

mod support;

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::credit::credit_in_full;
use wl_invoicing::credit_notes::{self, By};
use wl_invoicing::permissions as perm;
use wl_invoicing::{Allocation, CreditNote, CreditNoteKind};
use wl_kernel::invoicing::{
    CreditLine, CreditNoteStatus, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};

use support::boot;

const LINES: [(i64, VatCategory); 2] = [
    (100_000, VatCategory::Standard),
    (50_000, VatCategory::ZeroRated),
];

fn refused<T>(result: std::result::Result<T, TransitionError>) -> bool {
    match result {
        Err(error) => matches!(
            error.into_error(),
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)
        ),
        Ok(_) => false,
    }
}

#[renox::test]
async fn issuing_allocates_to_the_invoice_and_applies_the_credit_note() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let credit = t.helper("Cleo", &[perm::VIEW, perm::CREDIT]).await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let draft = credit_notes::draft(
        t.db(),
        &invoice,
        &[CreditLine::new(1, 20_000)],
        "Damaged goods",
        Some(credit.id),
    )
    .await
    .unwrap();

    let note = credit_notes::issue(t.state(), By::System(Some(credit.id)), &draft)
        .await
        .unwrap();
    assert!(note.number.as_deref().is_some_and(|n| n.starts_with("CN-")));
    assert!(note.number.as_deref().is_some_and(|n| n.ends_with("-0001")));
    assert!(note.issuer_snapshot.is_some());
    assert!(note.party_snapshot.is_some());
    assert!(note.issue_date.is_some());
    assert_eq!(note.gross_total, 21_000);
    // All of it went to the invoice: nothing is left, so it is applied.
    assert_eq!(note.remaining, 0);
    assert_eq!(note.status, CreditNoteStatus::Applied);

    let after = t.reload(&invoice).await;
    assert_eq!(after.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(after.balance, invoice.gross_total - 21_000);
    assert_eq!(
        (after.net_total, after.vat_total, after.gross_total),
        (invoice.net_total, invoice.vat_total, invoice.gross_total)
    );
    let allocations = Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0].source(), note.subject());
    assert_eq!(allocations[0].amount, 21_000);

    // The next one takes the next number; the invoice numbers are apart.
    let second = invoicer(t.state())
        .credit_partly(
            t.state(),
            InvoiceId(invoice.id),
            vec![CreditLine::new(2, 5_000)],
            "Short delivery".into(),
            Some(credit.id),
        )
        .await
        .unwrap();
    let second = CreditNote::find_or_404(t.db(), second.0).await.unwrap();
    assert!(
        second
            .number
            .as_deref()
            .is_some_and(|n| n.ends_with("-0002"))
    );
}

#[renox::test]
async fn a_credit_note_on_a_paid_invoice_stays_as_the_partys_credit() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    t.pay(&invoice, 1, invoice.gross_total).await;
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);

    let id = invoicer(t.state())
        .credit_partly(
            t.state(),
            InvoiceId(invoice.id),
            vec![CreditLine::new(2, 50_000)],
            "Service not delivered".into(),
            None,
        )
        .await
        .unwrap();
    let note = CreditNote::find_or_404(t.db(), id.0).await.unwrap();
    assert_eq!(note.status, CreditNoteStatus::Issued);
    assert_eq!(note.remaining, 50_000);
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);

    let open = invoicer(t.state())
        .open_credit(t.db(), wl_kernel::PartyId(invoice.party_id), "AED")
        .await
        .unwrap();
    assert_eq!(open, 50_000);
    let found = invoicer(t.state())
        .find_credit_note(t.db(), id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (found.status, found.remaining),
        (CreditNoteStatus::Issued, 50_000)
    );
    let listed = invoicer(t.state())
        .credit_notes_for(t.db(), InvoiceId(invoice.id))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
}

#[renox::test]
async fn issuing_is_refused_above_the_remaining_credit() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let credit = t.helper("Cleo", &[perm::VIEW, perm::CREDIT]).await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    // Two drafts of 600.00 net on a 1,000.00 line: the first issued wins.
    let lines = [CreditLine::new(1, 60_000)];
    let first = credit_notes::draft(t.db(), &invoice, &lines, "One", None)
        .await
        .unwrap();
    let second = credit_notes::draft(t.db(), &invoice, &lines, "Two", None)
        .await
        .unwrap();

    credit_notes::issue(t.state(), By::System(Some(credit.id)), &first)
        .await
        .unwrap();
    let over = credit_notes::issue(t.state(), By::System(Some(credit.id)), &second)
        .await
        .map(|_| ());
    assert!(refused(over));
    let kept = CreditNote::find_or_404(t.db(), second.id).await.unwrap();
    assert_eq!(kept.status, CreditNoteStatus::Draft);
    assert_eq!(kept.number, None);
}

#[renox::test]
async fn crediting_in_full_cancels_the_invoice() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let credit = t.helper("Cleo", &[perm::VIEW, perm::CREDIT]).await;
    let invoice = t.issued(&LINES, None, &clerk).await;

    let note = credit_in_full(
        t.state(),
        &invoice,
        "Billed twice",
        false,
        By::System(Some(credit.id)),
    )
    .await
    .unwrap();
    assert_eq!(note.kind, CreditNoteKind::Full);
    assert_eq!(note.status, CreditNoteStatus::Applied);
    assert_eq!(note.remaining, 0);
    assert_eq!(
        (note.net_total, note.vat_total, note.gross_total),
        (invoice.net_total, invoice.vat_total, invoice.gross_total)
    );
    let after = t.reload(&invoice).await;
    assert_eq!(after.status, InvoiceStatus::Credited);
    assert_eq!(after.balance, 0);
    assert_eq!(after.allocated_total, 0);
    assert_eq!(
        t.history(&invoice).await.last().map(String::as_str),
        Some("credit_system")
    );

    // Not twice.
    let again = credit_in_full(
        t.state(),
        &after,
        "Again",
        false,
        By::System(Some(credit.id)),
    )
    .await;
    assert!(matches!(again, Err(TransitionError::NotFrom(_))));
}

#[renox::test]
async fn crediting_in_full_is_refused_with_money_allocated_or_no_reason() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let blank = credit_in_full(t.state(), &invoice, "  ", false, By::System(None)).await;
    assert!(matches!(blank, Err(TransitionError::ReasonRequired)));

    t.pay(&invoice, 1, 10_000).await;
    let paid = t.reload(&invoice).await;
    let refused_in_full =
        credit_in_full(t.state(), &paid, "Billed twice", false, By::System(None)).await;
    assert!(refused_in_full.is_err());

    // The refused run rolled back whole: no credit note, no number used.
    let notes = credit_notes::for_invoice(t.db(), invoice.id).await.unwrap();
    assert!(notes.is_empty());
    let after = t.reload(&invoice).await;
    assert_eq!(after.status, InvoiceStatus::PartiallyPaid);

    // Without invoicing's own permission a system run through the Invoicer
    // credits an untouched invoice.
    let other = t.issued(&LINES, None, &clerk).await;
    let id = invoicer(t.state())
        .credit_in_full(t.state(), InvoiceId(other.id), "Cancelled".into(), None)
        .await
        .unwrap();
    let note = CreditNote::find_or_404(t.db(), id.0).await.unwrap();
    assert_eq!(note.status, CreditNoteStatus::Applied);
    assert_eq!(t.reload(&other).await.status, InvoiceStatus::Credited);
}
