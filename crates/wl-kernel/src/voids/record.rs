//! The Void record and the effects a void reverses (wl-kernel §5.12).

use std::fmt;

use renox::db::Json;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// A kind of effect a record may have had on money, stock or the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    /// A ledger posting.
    Posting,
    /// A payment allocated to an invoice or deposit.
    Allocation,
    /// A stock movement.
    StockMove,
    /// A reserved slot (a booking).
    ReservedSlot,
    /// A task opened for the record.
    Task,
}

impl EffectKind {
    /// The kind as stored and shown: `stock_move`.
    pub fn as_str(self) -> &'static str {
        match self {
            EffectKind::Posting => "posting",
            EffectKind::Allocation => "allocation",
            EffectKind::StockMove => "stock_move",
            EffectKind::ReservedSlot => "reserved_slot",
            EffectKind::Task => "task",
        }
    }
}

impl fmt::Display for EffectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One thing a record did that a void undoes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    /// The kind of effect.
    pub kind: EffectKind,
    /// What it touches, e.g. `invoice INV-ACME-2026-0101`.
    pub reference: String,
    /// The amount in fils.
    pub amount: i64,
    /// The quantity of stock.
    pub quantity: i64,
}

impl Effect {
    /// An effect of `kind` on `reference`, with no amount and no quantity.
    pub fn new(kind: EffectKind, reference: impl Into<String>) -> Self {
        Effect {
            kind,
            reference: reference.into(),
            amount: 0,
            quantity: 0,
        }
    }

    /// The amount in fils.
    pub fn amount(mut self, fils: i64) -> Self {
        self.amount = fils;
        self
    }

    /// The quantity of stock.
    pub fn quantity(mut self, quantity: i64) -> Self {
        self.quantity = quantity;
        self
    }

    /// Whether this effect means money or stock was posted: a posting or an
    /// allocation above 0 fils, or a stock move of any non-zero quantity.
    /// Reserved slots and tasks never count.
    pub fn is_posted(&self) -> bool {
        match self.kind {
            EffectKind::Posting | EffectKind::Allocation => self.amount > 0,
            EffectKind::StockMove => self.quantity != 0,
            EffectKind::ReservedSlot | EffectKind::Task => false,
        }
    }
}

/// Why a reverse handler refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReverseError {
    /// It cannot be undone (e.g. "stock already issued onward"); the
    /// message goes back to the user unchanged.
    Failed(String),
    /// The accounting period of the effect is locked.
    LockedPeriod(String),
}

impl ReverseError {
    /// The message for the user.
    pub fn message(&self) -> &str {
        match self {
            ReverseError::Failed(message) | ReverseError::LockedPeriod(message) => message,
        }
    }
}

impl fmt::Display for ReverseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ReverseError {}

impl From<renox::Error> for ReverseError {
    fn from(error: renox::Error) -> Self {
        ReverseError::Failed(error.to_string())
    }
}

impl From<renox::db::DbError> for ReverseError {
    fn from(error: renox::db::DbError) -> Self {
        ReverseError::Failed(error.to_string())
    }
}

/// The proof that a record was voided: one per subject.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_voids",
    unique(subject_type, subject_id),
    index(series_key)
)]
pub struct VoidRecord {
    /// The primary key.
    pub id: i64,
    /// The subject's type, e.g. `payments.receipt`.
    pub subject_type: String,
    /// The subject's id.
    pub subject_id: i64,
    /// The number the record keeps, if it has one.
    pub number: Option<String>,
    /// The series the number came from.
    pub series_key: Option<String>,
    /// The state the record was in before.
    pub state_before: String,
    /// Why it was voided.
    pub reason: String,
    /// The user who voided it (the requester when it was approved).
    pub voided_by: i64,
    /// The approval request, when money or stock was posted.
    pub approval_request_id: Option<i64>,
    /// What the void reversed.
    pub effects: Json<Vec<Effect>>,
    /// When it was voided (an instant, shown in Asia/Dubai).
    pub at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl VoidRecord {
    /// The record that was voided.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }

    /// The Void record of `subject`, if it was voided.
    pub async fn of_subject<'c>(
        db: impl renox::db::Executor<'c>,
        subject: &Subject,
    ) -> Result<Option<VoidRecord>> {
        VoidRecord::query()
            .where_(VoidRecord::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(VoidRecord::SUBJECT_ID.eq(subject.subject_id))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_stock_count_as_posted() {
        let posting = Effect::new(EffectKind::Posting, "journal 1").amount(1);
        let allocation = Effect::new(EffectKind::Allocation, "invoice INV-1").amount(300_000);
        let stock = Effect::new(EffectKind::StockMove, "cement").quantity(-4);
        assert!(posting.is_posted() && allocation.is_posted() && stock.is_posted());
    }

    #[test]
    fn zero_amounts_slots_and_tasks_do_not() {
        assert!(!Effect::new(EffectKind::Posting, "j").is_posted());
        assert!(
            !Effect::new(EffectKind::Allocation, "i")
                .amount(0)
                .is_posted()
        );
        assert!(!Effect::new(EffectKind::StockMove, "s").is_posted());
        // An amount on a slot or a task is not money.
        assert!(
            !Effect::new(EffectKind::ReservedSlot, "slot")
                .amount(500)
                .is_posted()
        );
        assert!(
            !Effect::new(EffectKind::Task, "task")
                .amount(500)
                .quantity(2)
                .is_posted()
        );
    }

    #[test]
    fn effects_serialize_with_a_snake_case_kind() {
        let effect = Effect::new(EffectKind::ReservedSlot, "slot A").quantity(1);
        let json = renox::serde_json::to_value(&effect).unwrap();
        assert_eq!(json["kind"], "reserved_slot");
        assert_eq!(effect.kind.to_string(), "reserved_slot");
        let back: Effect = renox::serde_json::from_value(json).unwrap();
        assert_eq!(back, effect);
    }

    #[test]
    fn a_reverse_error_shows_its_message_unchanged() {
        let failed = ReverseError::Failed("stock already issued onward".into());
        let locked = ReverseError::LockedPeriod("the period is locked".into());
        assert_eq!(failed.to_string(), "stock already issued onward");
        assert_eq!(locked.message(), "the period is locked");
    }
}
