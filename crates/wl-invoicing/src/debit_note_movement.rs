//! What left a supplier debit note's balance (wl-invoicing §5.13 step 4): a
//! set off on a later bill of the same supplier, a refund the supplier paid
//! back, or the difference accepted at `settle`. Rows are never deleted. A
//! refund carries the idempotency key of the payment's allocation, so a repeat
//! of the same delivery changes nothing.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// How the debit balance was reduced.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DebitMovementKind {
    /// Applied to a later bill of the same supplier through the bill's `pay`.
    #[default]
    SetOff,
    /// The supplier paid it back (an incoming payment allocated to the note).
    Refund,
    /// The part the supplier paid short, accepted at `settle` with a reason.
    Difference,
}

/// One reduction of the debit balance. Money is `i64` fils of the note's
/// currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_supplier_debit_note_movements",
    unique(key),
    index(debit_note_id)
)]
pub struct DebitNoteMovement {
    /// The primary key.
    pub id: i64,
    /// The debit note.
    pub debit_note_id: i64,
    /// What kind of reduction it is.
    pub kind: DebitMovementKind,
    /// The type of the bill (set off) or payment (refund) it went to.
    pub target_type: Option<String>,
    /// The id of the bill or payment.
    pub target_id: Option<i64>,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// The idempotency key of a refund.
    pub key: Option<String>,
    /// The user who did it; `None` for the system.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DebitNoteMovement {
    /// A movement of `amount` fils of `kind` from note `debit_note_id` to
    /// `target`.
    pub fn new(
        debit_note_id: i64,
        kind: DebitMovementKind,
        target: Option<&Subject>,
        amount: i64,
        created_by: Option<i64>,
    ) -> Self {
        DebitNoteMovement {
            debit_note_id,
            kind,
            target_type: target.map(|t| t.subject_type.clone()),
            target_id: target.map(|t| t.subject_id),
            amount,
            created_by,
            ..Default::default()
        }
    }

    /// The bill or payment it went to.
    pub fn target(&self) -> Option<Subject> {
        match (&self.target_type, self.target_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The movements of note `debit_note_id`, oldest first.
    pub async fn of_note(db: &Db, debit_note_id: i64) -> Result<Vec<DebitNoteMovement>> {
        DebitNoteMovement::query()
            .where_(DebitNoteMovement::DEBIT_NOTE_ID.eq(debit_note_id))
            .order_by(DebitNoteMovement::ID)
            .get(db)
            .await
    }

    /// The movement written under `key`, if the key was used.
    pub async fn by_key(db: &Db, key: &str) -> Result<Option<DebitNoteMovement>> {
        DebitNoteMovement::query()
            .where_(DebitNoteMovement::KEY.eq(key.to_owned()))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_stored_in_snake_case() {
        let kinds: Vec<_> = DebitMovementKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, ["set_off", "refund", "difference"]);
    }

    #[test]
    fn a_movement_keeps_its_target() {
        let bill = Subject::new("invoicing.supplier_bill", 4);
        let row = DebitNoteMovement::new(2, DebitMovementKind::SetOff, Some(&bill), 31_500, Some(5));
        assert_eq!(row.target(), Some(bill));
        assert_eq!((row.debit_note_id, row.amount, row.created_by), (2, 31_500, Some(5)));
        let none = DebitNoteMovement::new(2, DebitMovementKind::Difference, None, 1_500, None);
        assert_eq!(none.target(), None);
    }
}
