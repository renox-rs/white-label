//! Refunds from a credit note and `RefundFailed` (wl-invoicing §4, #71 part
//! 5): `request_refund` takes first and undoes on a refusal, a failed refund
//! gives its amount back once per attempt, and a retry takes again through
//! the kernel's `take_credit_for_refund`.

mod support;

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::CreditNote;
use wl_invoicing::credit_note_movement::{self, MovementKind};
use wl_invoicing::credit_notes::{self, By};
use wl_invoicing::refund::request_refund;
use wl_invoicing::restore::RESTORED;
use wl_kernel::invoicing::{
    CREDIT_NOTE_SUBJECT_TYPE, CreditLine, CreditNoteId, CreditNoteStatus, InvoiceId, VatCategory,
    invoicer,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{
    PaymentMethod, RefundAttemptRef, RefundFailed, RefundFailedFrom, RefundFailureKind, RefundId,
    RefundSource,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

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

/// A credit note of AED 500.00 on a paid invoice, so all of it is open.
async fn open_note(t: &Booted) -> (wl_invoicing::Invoice, CreditNote, renox::auth::User) {
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    t.pay(&invoice, 1, invoice.gross_total).await;
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
    assert_eq!(note.remaining, 50_000);
    (invoice, note, clerk)
}

async fn reload(t: &Booted, note: &CreditNote) -> CreditNote {
    CreditNote::find_or_404(t.db(), note.id).await.unwrap()
}

fn failed(note: &CreditNote, refund: RefundId, attempt: i32, amount: i64) -> RefundFailed {
    RefundFailed {
        refund,
        party: PartyId(note.party_id),
        amount,
        currency: "AED".into(),
        source: RefundSource::CreditNote(note.subject()),
        method: PaymentMethod::Transfer,
        kind: RefundFailureKind::CardUnavailable,
        reason: "card expired".into(),
        attempt: RefundAttemptRef {
            id: i64::from(attempt),
            number: attempt,
        },
        from: RefundFailedFrom::Approved,
        outgoing_payment: None,
    }
}

#[renox::test]
async fn a_refund_takes_the_credit_note_and_applies_it() {
    let t = boot().await;
    let (_, note, clerk) = open_note(&t).await;
    let refund = request_refund(
        t.state(),
        By::System(Some(clerk.id)),
        &note,
        50_000,
        Some(PaymentMethod::Transfer),
        "Customer left",
    )
    .await
    .unwrap();

    let asked = t.refunds.asked();
    assert_eq!(asked.len(), 1);
    let (request, by) = &asked[0];
    assert_eq!(*by, Some(clerk.id));
    assert_eq!(request.source, RefundSource::CreditNote(note.subject()));
    assert_eq!(
        request.source.subject().subject_type,
        CREDIT_NOTE_SUBJECT_TYPE
    );
    assert_eq!(
        (request.party, request.amount, request.currency.as_str()),
        (PartyId(note.party_id), 50_000, "AED")
    );
    assert_eq!(request.legal_entity_id, note.legal_entity_id.unwrap());
    assert_eq!(request.method, Some(PaymentMethod::Transfer));
    assert_eq!(request.reason, "Customer left");

    let after = reload(&t, &note).await;
    assert_eq!(
        (after.status, after.remaining),
        (CreditNoteStatus::Applied, 0)
    );
    let moves = credit_note_movement::for_credit_note(t.db(), note.id)
        .await
        .unwrap();
    let taken: Vec<_> = moves
        .iter()
        .filter(|m| m.kind == MovementKind::RefundTaken)
        .collect();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].target(), Some(refund.subject()));
    assert_eq!(
        (taken[0].amount, taken[0].attempt, taken[0].created_by),
        (50_000, None, Some(clerk.id))
    );
}

#[renox::test]
async fn a_refund_above_the_remaining_or_from_another_state_is_refused() {
    let t = boot().await;
    let (_, note, clerk) = open_note(&t).await;
    let by = || By::System(Some(clerk.id));
    for amount in [0, -1, 50_001] {
        let result = request_refund(t.state(), by(), &note, amount, None, "Why").await;
        assert!(refused(result), "amount {amount}");
    }
    assert!(refused(
        request_refund(t.state(), by(), &note, 100, None, "  ").await
    ));
    request_refund(t.state(), by(), &note, 50_000, None, "All")
        .await
        .unwrap();
    // Now `applied`.
    let applied = reload(&t, &note).await;
    assert!(refused(
        request_refund(t.state(), by(), &applied, 1, None, "More").await
    ));
    // A draft is not refundable either.
    let invoice = t.issued(&LINES, None, &clerk).await;
    let draft = credit_notes::draft(
        t.db(),
        &invoice,
        &[CreditLine::new(1, 10_000)],
        "Draft",
        Some(clerk.id),
    )
    .await
    .unwrap();
    assert!(refused(
        request_refund(t.state(), by(), &draft, 1, None, "Draft").await
    ));
    assert_eq!(t.refunds.asked().len(), 1);
}

#[renox::test]
async fn a_refusal_by_payments_loses_nothing() {
    let t = boot().await;
    let (_, note, clerk) = open_note(&t).await;
    let before = credit_note_movement::for_credit_note(t.db(), note.id)
        .await
        .unwrap()
        .len();
    t.refunds.refuse(true);
    let result = request_refund(
        t.state(),
        By::System(Some(clerk.id)),
        &note,
        50_000,
        None,
        "Why",
    )
    .await;
    assert!(refused(result));
    let after = reload(&t, &note).await;
    assert_eq!(
        (after.status, after.remaining),
        (CreditNoteStatus::Issued, 50_000)
    );
    let moves = credit_note_movement::for_credit_note(t.db(), note.id)
        .await
        .unwrap();
    assert_eq!(moves.len(), before);
    assert!(t.refunds.asked().is_empty());
}

#[renox::test]
async fn a_failed_refund_reopens_the_credit_note_once() {
    let t = boot().await;
    let (_, note, clerk) = open_note(&t).await;
    let accountant = t.accountant("Zaid").await;
    let refund = request_refund(
        t.state(),
        By::System(Some(clerk.id)),
        &note,
        50_000,
        None,
        "Why",
    )
    .await
    .unwrap();

    let event = failed(&note, refund, 1, 50_000);
    t.state().emit(event.clone()).await.unwrap();
    let after = reload(&t, &note).await;
    assert_eq!(
        (after.status, after.remaining),
        (CreditNoteStatus::Issued, 50_000)
    );
    assert_eq!(
        (after.number.clone(), after.gross_total),
        (note.number.clone(), note.gross_total)
    );
    let reopened = t.reopened.all();
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0].amount_restored, 50_000);
    assert_eq!(reopened[0].cause, refund.subject());
    let notices = t.notices.of_type(RESTORED);
    assert_eq!(notices.len(), 2);
    assert!(matches!(
        notices[0].recipients.as_slice(),
        [NotifyRecipient::User { id }] if *id == clerk.id
    ));
    assert!(matches!(
        notices[1].recipients.as_slice(),
        [NotifyRecipient::User { id }] if *id == accountant.id
    ));
    assert_eq!(notices[0].data["reason"], "card expired");

    // Delivered again: nothing changes.
    t.state().emit(event).await.unwrap();
    assert_eq!(reload(&t, &note).await.remaining, 50_000);
    assert_eq!(t.reopened.all().len(), 1);
    assert_eq!(t.notices.of_type(RESTORED).len(), 2);
    let restored = credit_note_movement::for_credit_note(t.db(), note.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == MovementKind::RefundRestored)
        .count();
    assert_eq!(restored, 1);
}

#[renox::test]
async fn a_retry_takes_again_and_its_failure_restores_again() {
    let t = boot().await;
    let (_, note, clerk) = open_note(&t).await;
    let refund = request_refund(
        t.state(),
        By::System(Some(clerk.id)),
        &note,
        50_000,
        None,
        "Why",
    )
    .await
    .unwrap();
    t.state()
        .emit(failed(&note, refund, 1, 50_000))
        .await
        .unwrap();

    // Attempt 2 failed, but the retry never took the amount again.
    t.state()
        .emit(failed(&note, refund, 2, 50_000))
        .await
        .unwrap();
    assert_eq!(reload(&t, &note).await.remaining, 50_000);
    assert_eq!(t.reopened.all().len(), 1);

    // The retry takes it again; a second call for the same attempt is a no-op.
    let invoices = invoicer(t.state());
    let id = CreditNoteId(note.id);
    for _ in 0..2 {
        invoices
            .take_credit_for_refund(t.state(), id, refund.subject(), 2, 50_000)
            .await
            .unwrap();
    }
    let taken = reload(&t, &note).await;
    assert_eq!(
        (taken.status, taken.remaining),
        (CreditNoteStatus::Applied, 0)
    );
    // Nothing is left to take for another attempt.
    let again = invoices
        .take_credit_for_refund(t.state(), id, refund.subject(), 3, 1)
        .await;
    assert!(again.is_err());

    t.state()
        .emit(failed(&note, refund, 2, 50_000))
        .await
        .unwrap();
    let back = reload(&t, &note).await;
    assert_eq!(
        (back.status, back.remaining),
        (CreditNoteStatus::Issued, 50_000)
    );
    assert_eq!(t.reopened.all().len(), 2);
    // The requester of the first take is told again.
    assert_eq!(t.notices.of_type(RESTORED).len(), 2);
}

#[renox::test]
async fn only_a_credit_note_source_is_restored() {
    let t = boot().await;
    let (_, note, clerk) = open_note(&t).await;
    let refund = request_refund(
        t.state(),
        By::System(Some(clerk.id)),
        &note,
        20_000,
        None,
        "Why",
    )
    .await
    .unwrap();
    for source in [
        RefundSource::CreditNote(Subject::new("sales.voucher", note.id)),
        RefundSource::PartyCredit(PartyId(note.party_id)),
    ] {
        let mut event = failed(&note, refund, 1, 20_000);
        event.source = source;
        t.state().emit(event).await.unwrap();
    }
    assert_eq!(reload(&t, &note).await.remaining, 30_000);
    assert!(t.reopened.all().is_empty());
    assert!(t.notices.of_type(RESTORED).is_empty());

    // On an `issued` credit note the restore changes no state.
    t.state()
        .emit(failed(&note, refund, 1, 20_000))
        .await
        .unwrap();
    let after = reload(&t, &note).await;
    assert_eq!(
        (after.status, after.remaining),
        (CreditNoteStatus::Issued, 50_000)
    );
    assert!(t.reopened.all().is_empty());
    assert_eq!(t.notices.of_type(RESTORED).len(), 1);
}
