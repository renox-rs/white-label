//! The credit note ledger, restore and `reopen` (wl-invoicing §4, #71 part
//! 4): the movements, `use_on_invoice`, a restore capped at what the target
//! consumed, `reopen` of an applied credit note, a restore that leaves an
//! issued one in its state, and `restore_allocation`.

mod support;

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_invoicing::credit::credit_in_full;
use wl_invoicing::credit_note_movement::{self, CreditNoteMovement, MovementKind};
use wl_invoicing::credit_notes::{self, By};
use wl_invoicing::restore::{self, ALLOCATION_SUBJECT_TYPE, Cause, RESTORED, Restore};
use wl_invoicing::{Allocation, CreditNote};
use wl_kernel::Subject;
use wl_kernel::invoicing::{CreditLine, CreditNoteStatus, InvoiceStatus, VatCategory, invoicer};
use wl_kernel::notify::NotifyRecipient;

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
            wl_kernel::invoicing::InvoiceId(invoice.id),
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

fn refund(id: i64) -> Subject {
    Subject::new("payments.refund", id)
}

/// Takes `amount` of `note` as the refund `id` would (the sub-task that
/// requests refunds does this in its own flow).
async fn take_for_refund(t: &Booted, note: &CreditNote, id: i64, amount: i64, by: Option<i64>) {
    let mut tx = t.db().begin().await.unwrap();
    let current = CreditNote::find_or_404(&mut tx, note.id).await.unwrap();
    sqlx_like_lower(&mut tx, &current, amount).await;
    CreditNoteMovement::new(
        note.id,
        MovementKind::RefundTaken,
        Some(&refund(id)),
        amount,
        None,
        by,
    )
    .insert(&mut tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let current = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    if current.remaining == 0 {
        apply(t, &current).await;
    }
}

async fn sqlx_like_lower(tx: &mut renox::db::Transaction, note: &CreditNote, amount: i64) {
    renox::db::sql(format!(
        "UPDATE {} SET remaining = remaining - ? WHERE id = ?",
        CreditNote::TABLE
    ))
    .bind(amount)
    .bind(note.id)
    .execute(&mut *tx)
    .await
    .unwrap();
    assert!(note.remaining >= amount);
}

async fn apply(t: &Booted, note: &CreditNote) {
    let outcome = note
        .transition("apply")
        .by_system()
        .run(t.state())
        .await
        .expect("apply");
    outcome.emit(t.state()).await.expect("emit");
}

fn failure(note: &CreditNote, id: i64, attempt: i64, amount: i64) -> Restore {
    Restore {
        credit_note: note.id,
        cause: Cause::RefundFailed,
        target: refund(id),
        attempt: Some(attempt),
        amount,
        reason: Some("card expired".into()),
    }
}

#[renox::test]
async fn issuing_writes_the_invoice_allocated_movement() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let id = invoicer(t.state())
        .credit_partly(
            t.state(),
            wl_kernel::invoicing::InvoiceId(invoice.id),
            vec![CreditLine::new(1, 20_000)],
            "Damaged".into(),
            Some(clerk.id),
        )
        .await
        .unwrap();
    let allocation = Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    let moves = credit_note_movement::for_credit_note(t.db(), id.0)
        .await
        .unwrap();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].kind, MovementKind::InvoiceAllocated);
    assert_eq!(
        moves[0].target(),
        Some(Subject::new(ALLOCATION_SUBJECT_TYPE, allocation.id))
    );
    assert_eq!(
        (moves[0].amount, moves[0].created_by),
        (21_000, Some(clerk.id))
    );
}

#[renox::test]
async fn crediting_in_full_writes_the_invoice_credited_movement() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let note = credit_in_full(t.state(), &invoice, "Billed twice", false, By::System(None))
        .await
        .unwrap();
    let moves = credit_note_movement::for_credit_note(t.db(), note.id)
        .await
        .unwrap();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].kind, MovementKind::InvoiceCredited);
    assert_eq!(
        moves[0].target(),
        Some(Subject::new("invoicing.invoice", invoice.id))
    );
    assert_eq!(moves[0].amount, invoice.gross_total);
}

#[renox::test]
async fn a_credit_note_is_used_on_another_open_invoice() {
    let t = boot().await;
    let (_, note, clerk) = open_note(&t).await;
    let other = t.issued(&LINES, None, &clerk).await;

    // Above the remaining, above the balance and nothing are refused.
    for amount in [0, 50_001] {
        let result =
            credit_notes::use_on_invoice(t.state(), By::System(None), &note, &other, amount)
                .await
                .map(|_| ());
        assert!(refused(result));
    }
    let part =
        credit_notes::use_on_invoice(t.state(), By::System(Some(clerk.id)), &note, &other, 20_000)
            .await
            .unwrap();
    assert_eq!(
        (part.remaining, part.status),
        (30_000, CreditNoteStatus::Issued)
    );
    let after = t.reload(&other).await;
    assert_eq!(after.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(after.balance, other.gross_total - 20_000);

    let rest = credit_notes::use_on_invoice(t.state(), By::System(None), &part, &after, 30_000)
        .await
        .unwrap();
    assert_eq!(
        (rest.remaining, rest.status),
        (0, CreditNoteStatus::Applied)
    );
    let moves = credit_note_movement::for_credit_note(t.db(), note.id)
        .await
        .unwrap();
    let used: Vec<i64> = moves
        .iter()
        .filter(|m| m.kind == MovementKind::InvoiceAllocated)
        .map(|m| m.amount)
        .collect();
    assert_eq!(used, [20_000, 30_000]);

    // An applied credit note, or a paid invoice, cannot be used.
    let again = credit_notes::use_on_invoice(t.state(), By::System(None), &rest, &after, 1)
        .await
        .map(|_| ());
    assert!(refused(again));
}

#[renox::test]
async fn a_failed_refund_reopens_an_applied_credit_note() {
    let t = boot().await;
    let (_, note, _) = open_note(&t).await;
    let requester = t.helper("Cleo", &[]).await;
    let accountant = t.accountant("Zaid").await;
    take_for_refund(&t, &note, 9, 50_000, Some(requester.id)).await;
    let applied = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    assert_eq!(
        (applied.status, applied.remaining),
        (CreditNoteStatus::Applied, 0)
    );

    let done = restore::restore(t.state(), failure(&note, 9, 1, 50_000))
        .await
        .unwrap()
        .expect("restored");
    assert_eq!(
        (done.amount, done.remaining, done.reopened),
        (50_000, 50_000, true)
    );

    let after = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    assert_eq!(after.status, CreditNoteStatus::Issued);
    assert_eq!(after.remaining, 50_000);
    // The document is untouched.
    assert_eq!(
        (
            after.number,
            after.net_total,
            after.vat_total,
            after.gross_total
        ),
        (
            note.number,
            note.net_total,
            note.vat_total,
            note.gross_total
        )
    );

    let reopened = t.reopened.all();
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0].amount_restored, 50_000);
    assert_eq!(reopened[0].cause, refund(9));
    assert_eq!(reopened[0].credit_note.0, note.id);

    let notices = t.notices.of_type(RESTORED);
    assert_eq!(notices.len(), 2);
    assert!(matches!(
        notices[0].recipients.as_slice(),
        [NotifyRecipient::User { id }] if *id == requester.id
    ));
    assert!(matches!(
        notices[1].recipients.as_slice(),
        [NotifyRecipient::User { id }] if *id == accountant.id
    ));
    for notice in &notices {
        assert_eq!(notice.data["reason"], "card expired");
        assert_eq!(notice.data["amount"], "AED 500.00");
        assert_eq!(notice.data["remaining"], "AED 500.00");
    }
}

#[renox::test]
async fn a_restore_on_an_issued_credit_note_changes_no_state() {
    let t = boot().await;
    let (_, note, clerk) = open_note(&t).await;
    let other = t.issued(&LINES, None, &clerk).await;
    credit_notes::use_on_invoice(t.state(), By::System(None), &note, &other, 20_000)
        .await
        .unwrap();
    take_for_refund(&t, &note, 9, 30_000, Some(clerk.id)).await;
    // Still `applied`; the refund fails.
    restore::restore(t.state(), failure(&note, 9, 1, 30_000))
        .await
        .unwrap();
    let after = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    assert_eq!(
        (after.status, after.remaining),
        (CreditNoteStatus::Issued, 30_000)
    );
    t.reopened.all();

    // Now a refund of part of what is left, while `issued`.
    take_for_refund(&t, &note, 10, 10_000, Some(clerk.id)).await;
    let before = t.reopened.all().len();
    let done = restore::restore(t.state(), failure(&note, 10, 1, 10_000))
        .await
        .unwrap()
        .unwrap();
    assert!(!done.reopened);
    let after = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    assert_eq!(
        (after.status, after.remaining),
        (CreditNoteStatus::Issued, 30_000)
    );
    assert_eq!(t.reopened.all().len(), before);
    let history = note_history(&t, &after).await;
    assert_eq!(history.iter().filter(|h| *h == "reopen").count(), 1);
}

async fn note_history(t: &Booted, note: &CreditNote) -> Vec<String> {
    renox::state_machine::history(t.db(), CreditNote::TABLE, note.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.transition)
        .collect()
}

#[renox::test]
async fn the_same_failure_twice_changes_nothing_and_a_retry_restores_again() {
    let t = boot().await;
    let (_, note, _) = open_note(&t).await;
    take_for_refund(&t, &note, 9, 50_000, None).await;

    let first = restore::restore(t.state(), failure(&note, 9, 1, 50_000))
        .await
        .unwrap();
    assert!(first.is_some());
    let notices = t.notices.of_type(RESTORED).len();
    let moves = credit_note_movement::for_credit_note(t.db(), note.id)
        .await
        .unwrap()
        .len();

    // The same attempt again: nothing.
    let twice = restore::restore(t.state(), failure(&note, 9, 1, 50_000))
        .await
        .unwrap();
    assert_eq!(twice, None);
    // Attempt 2 that the retry never took again: nothing.
    let untaken = restore::restore(t.state(), failure(&note, 9, 2, 50_000))
        .await
        .unwrap();
    assert_eq!(untaken, None);
    assert_eq!(t.notices.of_type(RESTORED).len(), notices);
    assert_eq!(t.reopened.all().len(), 1);
    let after = credit_note_movement::for_credit_note(t.db(), note.id)
        .await
        .unwrap();
    assert_eq!(after.len(), moves);

    // The retry takes the amount again (attempt 2), then fails again.
    take_for_refund_attempt(&t, &note, 9, 2, 50_000).await;
    let again = restore::restore(t.state(), failure(&note, 9, 2, 50_000))
        .await
        .unwrap()
        .expect("restored again");
    assert_eq!(again.amount, 50_000);
    let latest = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    assert_eq!(latest.remaining, 50_000);
    assert!(latest.remaining <= latest.gross_total);
}

async fn take_for_refund_attempt(
    t: &Booted,
    note: &CreditNote,
    id: i64,
    attempt: i64,
    amount: i64,
) {
    let mut tx = t.db().begin().await.unwrap();
    let current = CreditNote::find_or_404(&mut tx, note.id).await.unwrap();
    sqlx_like_lower(&mut tx, &current, amount).await;
    CreditNoteMovement::new(
        note.id,
        MovementKind::RefundTaken,
        Some(&refund(id)),
        amount,
        Some(attempt),
        None,
    )
    .insert(&mut tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let current = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    if current.remaining == 0 {
        apply(t, &current).await;
    }
}

#[renox::test]
async fn a_restore_is_capped_at_what_the_refund_consumed() {
    let t = boot().await;
    let (_, note, _) = open_note(&t).await;
    take_for_refund(&t, &note, 9, 30_000, None).await;
    let done = restore::restore(t.state(), failure(&note, 9, 1, 50_000))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.amount, 30_000);
    let after = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    assert_eq!(after.remaining, 50_000);
    // A refund that took nothing restores nothing.
    let none = restore::restore(t.state(), failure(&note, 77, 1, 10_000))
        .await
        .unwrap();
    assert_eq!(none, None);
}

#[renox::test]
async fn restoring_an_allocation_reopens_the_credit_note_once() {
    let t = boot().await;
    let clerk = t.user_a().await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let id = invoicer(t.state())
        .credit_partly(
            t.state(),
            wl_kernel::invoicing::InvoiceId(invoice.id),
            vec![CreditLine::new(1, 20_000)],
            "Damaged".into(),
            None,
        )
        .await
        .unwrap();
    let note = CreditNote::find_or_404(t.db(), id.0).await.unwrap();
    assert_eq!(note.status, CreditNoteStatus::Applied);
    let allocation = Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();

    let done = restore::restore_allocation(t.state(), allocation.id, allocation.amount, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (done.amount, done.remaining, done.reopened),
        (21_000, 21_000, true)
    );
    let after = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    assert_eq!(
        (after.status, after.remaining),
        (CreditNoteStatus::Issued, 21_000)
    );
    let reopened = t.reopened.all();
    assert_eq!(reopened.len(), 1);
    assert_eq!(
        reopened[0].cause,
        Subject::new(ALLOCATION_SUBJECT_TYPE, allocation.id)
    );

    let again = restore::restore_allocation(t.state(), allocation.id, allocation.amount, None)
        .await
        .unwrap();
    assert_eq!(again, None);
    let same = CreditNote::find_or_404(t.db(), note.id).await.unwrap();
    assert_eq!(same.remaining, 21_000);

    // A payment's allocation is not a credit note's.
    t.pay(&invoice, 5, 1_000).await;
    let payment = Allocation::query()
        .where_(Allocation::SOURCE_TYPE.eq("payments.payment"))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restore::restore_allocation(t.state(), payment.id, 1_000, None)
            .await
            .unwrap(),
        None
    );
}
