//! Refunds from a credit note (wl-invoicing §4, #71 A9): the remaining
//! balance of an `issued` credit note is paid back to the party through
//! `wl-payments`.
//!
//! [`request_refund`] takes first and asks second, so two requests at once
//! never both pass:
//!
//! 1. in one transaction a `refund_taken` movement (no refund yet) is
//!    written and the remaining is lowered;
//! 2. the kernel [`PaymentRecorder::request_refund`] asks the refund, with
//!    the credit note as its source;
//! 3. the refund id is stored on the movement, and the credit note is
//!    `applied` when nothing is left.
//!
//! When payments refuses, the movement is removed and the remaining is
//! given back in one transaction, so nothing is lost and the ledger is as it
//! was. The approval follows the Refund flow (`FIN.ACL.04`) in
//! `wl-payments`.
//!
//! [`take_for_refund`] is the same take for a refund that exists already:
//! #100's retry calls it for a new attempt through
//! [`Invoicer::take_credit_for_refund`](wl_kernel::invoicing::Invoicer).
//! A failure of the refund gives the amount back ([`crate::restore`]).

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::Subject;
use wl_kernel::invoicing::CreditNoteStatus;
use wl_kernel::payments::{PaymentMethod, RefundId, RefundRequest, RefundSource, payment_recorder};

use crate::credit_note::CreditNote;
use crate::credit_note_movement::{CreditNoteMovement, MovementKind, exists_in, requester_in};
use crate::credit_notes::{By, Outcomes, apply_in, lower_remaining_in};

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The refund attempt as it is stored: the first attempt is the take with
/// no attempt number, so a retry asking for attempt 1 never takes twice.
fn stored_attempt(attempt: i64) -> Option<i64> {
    (attempt > 1).then_some(attempt)
}

/// Why `amount` cannot be refunded from `note`, if it cannot.
fn problem(note: &CreditNote, amount: i64) -> Option<&'static str> {
    if note.status != CreditNoteStatus::Issued {
        return Some("only an issued credit note can be refunded");
    }
    if amount <= 0 || amount > note.remaining {
        return Some("the amount must be above 0 and within the credit note's remaining balance");
    }
    None
}

/// Refunds `amount` fils of the remaining balance of issued credit note
/// `note` to its party, as `by` (a user needs `invoices.credit`): see the
/// module. Returns the refund, which starts `requested` in `wl-payments`.
///
/// # Errors
///
/// `Forbidden` without `invoices.credit`; a 422 for a blank reason, a credit
/// note that is not `issued`, an amount that is not above 0 or is above the
/// remaining balance, and whatever payments refuses (it is off, an inactive
/// legal entity, ...). Nothing is lost then: the remaining and the movements
/// are as before.
pub async fn request_refund(
    state: &AppState,
    by: By<'_>,
    note: &CreditNote,
    amount: i64,
    method: Option<PaymentMethod>,
    reason: &str,
) -> std::result::Result<RefundId, TransitionError> {
    if let By::User(user) = &by
        && !user.has_permission_anywhere(crate::permissions::CREDIT)
    {
        return Err(Error::Forbidden.into());
    }
    if reason.trim().is_empty() {
        return Err(refused("a reason is required").into());
    }
    let note = CreditNote::find_or_404(&state.db, note.id).await?;
    if let Some(problem) = problem(&note, amount) {
        return Err(refused(problem).into());
    }
    let Some(legal_entity_id) = note.legal_entity_id else {
        return Err(refused("the credit note has no issuing entity").into());
    };

    // 1. Take first.
    let mut tx = state.db.begin().await?;
    lower_remaining_in(&mut tx, &note, amount).await?;
    let mut taken = CreditNoteMovement::new(
        note.id,
        MovementKind::RefundTaken,
        None,
        amount,
        None,
        by.id(),
    );
    taken.insert(&mut tx).await?;
    tx.commit().await?;

    // 2. Ask payments.
    let request = RefundRequest {
        source: RefundSource::CreditNote(note.subject()),
        party: note.party(),
        legal_entity_id,
        amount,
        currency: note.currency.clone(),
        method,
        gateway_txn: None,
        reason: reason.trim().to_owned(),
    };
    let asked = payment_recorder(state)
        .request_refund(state, request, by.id())
        .await;
    let refund = match asked {
        Ok(refund) => refund,
        Err(error) => {
            // Undo: the row goes and the remaining comes back.
            give_back(state, &taken).await?;
            return Err(error.into());
        }
    };

    // 3. Link the refund and apply when nothing is left.
    let mut tx = state.db.begin().await?;
    let target = refund.subject();
    renox::db::sql(format!(
        "UPDATE {} SET target_type = ?, target_id = ?, updated_at = ? WHERE id = ?",
        CreditNoteMovement::TABLE
    ))
    .bind(target.subject_type)
    .bind(target.subject_id)
    .bind(renox::db::now())
    .bind(taken.id)
    .execute(&mut tx)
    .await?;
    let after = CreditNote::find_or_404(&mut tx, note.id).await?;
    let mut outcomes = Outcomes::default();
    if after.remaining == 0 && after.status == CreditNoteStatus::Issued {
        apply_in(&mut tx, &after, &by, &mut outcomes).await?;
    }
    tx.commit().await?;
    outcomes.emit(state).await?;
    Ok(refund)
}

/// Removes the pending `taken` movement and raises the remaining by its
/// amount, in one transaction.
async fn give_back(state: &AppState, taken: &CreditNoteMovement) -> Result {
    let mut tx = state.db.begin().await?;
    renox::db::sql(format!(
        "DELETE FROM {} WHERE id = ?",
        CreditNoteMovement::TABLE
    ))
    .bind(taken.id)
    .execute(&mut tx)
    .await?;
    renox::db::sql(format!(
        "UPDATE {} SET remaining = remaining + ?, updated_at = ? WHERE id = ?",
        CreditNote::TABLE
    ))
    .bind(taken.amount)
    .bind(renox::db::now())
    .bind(taken.credit_note_id)
    .execute(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Takes `amount` fils of the remaining balance of credit note `note_id` for
/// `refund` on its numbered `attempt`, without a user: #100's retry takes
/// the amount again after a failure gave it back. Idempotent per refund and
/// attempt (attempt 1 is the take [`request_refund`] made). The credit note
/// is `applied` when nothing is left.
///
/// # Errors
///
/// A 404 for an unknown credit note; a 422 when it is not `issued`, or the
/// amount is not above 0 or is above the remaining balance. Nothing is
/// written then.
pub async fn take_for_refund(
    state: &AppState,
    note_id: i64,
    refund: &Subject,
    attempt: i64,
    amount: i64,
) -> std::result::Result<(), TransitionError> {
    let attempt = stored_attempt(attempt);
    let mut tx = state.db.begin().await?;
    let note = CreditNote::find_or_404(&mut tx, note_id).await?;
    if exists_in(&mut tx, note.id, MovementKind::RefundTaken, refund, attempt).await? {
        return Ok(());
    }
    if let Some(problem) = problem(&note, amount) {
        return Err(refused(problem).into());
    }
    let requester = requester_in(&mut tx, note.id, MovementKind::RefundTaken, refund).await?;
    lower_remaining_in(&mut tx, &note, amount).await?;
    CreditNoteMovement::new(
        note.id,
        MovementKind::RefundTaken,
        Some(refund),
        amount,
        attempt,
        requester,
    )
    .insert(&mut tx)
    .await?;
    let mut outcomes = Outcomes::default();
    if note.remaining == amount {
        let after = CreditNote::find_or_404(&mut tx, note.id).await?;
        apply_in(&mut tx, &after, &By::System(requester), &mut outcomes).await?;
    }
    tx.commit().await?;
    outcomes.emit(state).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issued(remaining: i64) -> CreditNote {
        CreditNote {
            status: CreditNoteStatus::Issued,
            remaining,
            gross_total: 50_000,
            ..Default::default()
        }
    }

    #[test]
    fn only_an_issued_credit_note_within_its_remaining_is_refunded() {
        assert_eq!(problem(&issued(50_000), 50_000), None);
        assert_eq!(problem(&issued(50_000), 1), None);
        assert!(problem(&issued(30_000), 30_001).is_some());
        assert!(problem(&issued(30_000), 0).is_some());
        assert!(problem(&issued(30_000), -5).is_some());
        for status in [CreditNoteStatus::Draft, CreditNoteStatus::Applied] {
            let note = CreditNote {
                status,
                ..issued(50_000)
            };
            assert!(problem(&note, 100).is_some());
        }
    }

    #[test]
    fn the_first_attempt_is_stored_without_a_number() {
        assert_eq!(stored_attempt(1), None);
        assert_eq!(stored_attempt(2), Some(2));
        assert_eq!(stored_attempt(0), None);
    }
}
