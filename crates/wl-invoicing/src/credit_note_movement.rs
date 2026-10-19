//! The credit note ledger (wl-invoicing §3): where the amount of a credit
//! note went, and what came back. A row is written in the same transaction
//! as the change of the credit note's remaining balance.
//!
//! - `invoice_allocated`: used on an invoice (the target is the allocation);
//! - `invoice_credited`: used on cancelling the invoice (the target is the
//!   invoice);
//! - `refund_taken`: taken by a refund (the target is the refund, once it
//!   exists);
//! - `refund_restored`: given back when that refund failed;
//! - `allocation_restored`: given back when the allocation was removed.
//!
//! A unique index on (credit note, kind, target, attempt) makes every take
//! and restore happen once. What a target consumed is its taken total minus
//! its restored total, so a restore never gives back more than that.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// What a movement is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MovementKind {
    /// Used on an invoice.
    #[default]
    InvoiceAllocated,
    /// Used on cancelling the invoice.
    InvoiceCredited,
    /// Taken by a refund.
    RefundTaken,
    /// Given back after a failed refund.
    RefundRestored,
    /// Given back after an allocation was removed.
    AllocationRestored,
}

impl MovementKind {
    /// Whether the movement lowers the remaining balance.
    pub fn takes(self) -> bool {
        matches!(
            self,
            MovementKind::InvoiceAllocated
                | MovementKind::InvoiceCredited
                | MovementKind::RefundTaken
        )
    }

    /// The kind that gives back what `self` took, if any.
    pub fn restored_by(self) -> Option<MovementKind> {
        match self {
            MovementKind::InvoiceAllocated => Some(MovementKind::AllocationRestored),
            MovementKind::RefundTaken => Some(MovementKind::RefundRestored),
            _ => None,
        }
    }

    /// The kind of take that `self` gives back, if it is a restore.
    pub fn restores(self) -> Option<MovementKind> {
        match self {
            MovementKind::AllocationRestored => Some(MovementKind::InvoiceAllocated),
            MovementKind::RefundRestored => Some(MovementKind::RefundTaken),
            _ => None,
        }
    }
}

/// One row of a credit note's ledger. Money is `i64` fils of the credit
/// note's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_credit_note_movements",
    index(credit_note_id),
    index(target_type, target_id)
)]
pub struct CreditNoteMovement {
    /// The primary key.
    pub id: i64,
    /// The credit note.
    pub credit_note_id: i64,
    /// What it is.
    pub kind: MovementKind,
    /// The type of the target; `None` for a refund not yet created.
    pub target_type: Option<String>,
    /// The id of the target.
    pub target_id: Option<i64>,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// The refund attempt number; `None` for the first take.
    pub attempt: Option<i64>,
    /// The user whose action caused it; `None` for the system.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CreditNoteMovement {
    /// A movement of `amount` fils of credit note `credit_note_id`.
    pub fn new(
        credit_note_id: i64,
        kind: MovementKind,
        target: Option<&Subject>,
        amount: i64,
        attempt: Option<i64>,
        created_by: Option<i64>,
    ) -> Self {
        CreditNoteMovement {
            credit_note_id,
            kind,
            target_type: target.map(|t| t.subject_type.clone()),
            target_id: target.map(|t| t.subject_id),
            amount,
            attempt,
            created_by,
            ..Default::default()
        }
    }

    /// The target, when it is set.
    pub fn target(&self) -> Option<Subject> {
        match (&self.target_type, self.target_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }
}

/// The movements of `kind` of credit note `credit_note_id` on `target`.
fn on_target(
    credit_note_id: i64,
    kind: MovementKind,
    target: &Subject,
) -> renox::db::Query<CreditNoteMovement> {
    CreditNoteMovement::query()
        .where_(CreditNoteMovement::CREDIT_NOTE_ID.eq(credit_note_id))
        .where_(CreditNoteMovement::KIND.eq(kind))
        .where_(CreditNoteMovement::TARGET_TYPE.eq(target.subject_type.clone()))
        .where_(CreditNoteMovement::TARGET_ID.eq(target.subject_id))
}

/// The sum of the movements of `kind` on `target`.
pub async fn total_in(
    tx: &mut renox::db::Transaction,
    credit_note_id: i64,
    kind: MovementKind,
    target: &Subject,
) -> Result<i64> {
    on_target(credit_note_id, kind, target)
        .sum::<i64, _>(&mut *tx, "amount")
        .await
}

/// What `target` consumed of credit note `credit_note_id`: what `taken`
/// moved onto it minus what was given back.
pub async fn consumed_in(
    tx: &mut renox::db::Transaction,
    credit_note_id: i64,
    taken: MovementKind,
    target: &Subject,
) -> Result<i64> {
    let took = total_in(tx, credit_note_id, taken, target).await?;
    let gave = match taken.restored_by() {
        Some(restored) => total_in(tx, credit_note_id, restored, target).await?,
        None => 0,
    };
    Ok(took - gave)
}

/// Who asked for the first `kind` movement on `target`.
pub async fn requester_in(
    tx: &mut renox::db::Transaction,
    credit_note_id: i64,
    kind: MovementKind,
    target: &Subject,
) -> Result<Option<i64>> {
    Ok(on_target(credit_note_id, kind, target)
        .order_by(CreditNoteMovement::ID)
        .first(&mut *tx)
        .await?
        .and_then(|m| m.created_by))
}

/// Whether the movement of `kind` on `target` for `attempt` already exists.
pub async fn exists_in(
    tx: &mut renox::db::Transaction,
    credit_note_id: i64,
    kind: MovementKind,
    target: &Subject,
    attempt: Option<i64>,
) -> Result<bool> {
    let rows = on_target(credit_note_id, kind, target)
        .get(&mut *tx)
        .await?;
    Ok(rows
        .iter()
        .any(|m| m.attempt.unwrap_or(0) == attempt.unwrap_or(0)))
}

/// The ledger of credit note `credit_note_id`, oldest first.
pub async fn for_credit_note(db: &Db, credit_note_id: i64) -> Result<Vec<CreditNoteMovement>> {
    CreditNoteMovement::query()
        .where_(CreditNoteMovement::CREDIT_NOTE_ID.eq(credit_note_id))
        .order_by(CreditNoteMovement::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_stored_in_snake_case() {
        let kinds: Vec<_> = MovementKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "invoice_allocated",
                "invoice_credited",
                "refund_taken",
                "refund_restored",
                "allocation_restored"
            ]
        );
    }

    #[test]
    fn takes_and_restores_pair_up() {
        for kind in MovementKind::ALL {
            assert_eq!(kind.takes(), kind.restores().is_none());
            if let Some(restored) = kind.restored_by() {
                assert_eq!(restored.restores(), Some(*kind));
            }
        }
        assert_eq!(MovementKind::InvoiceCredited.restored_by(), None);
    }

    #[test]
    fn a_movement_keeps_its_target() {
        let refund = Subject::new("payments.refund", 9);
        let movement = CreditNoteMovement::new(
            7,
            MovementKind::RefundTaken,
            Some(&refund),
            50_000,
            None,
            Some(3),
        );
        assert_eq!(movement.target(), Some(refund));
        let pending = CreditNoteMovement::new(7, MovementKind::RefundTaken, None, 1, None, None);
        assert_eq!(pending.target(), None);
    }
}
