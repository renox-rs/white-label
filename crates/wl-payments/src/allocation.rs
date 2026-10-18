//! An allocation (wl-payments §3): money of a payment set against a target
//! (an invoice today). Rows are never deleted: a reversal fills the
//! `removed_*` columns and the row stays, so the history is complete.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::payments::{AllocationTarget, RemovedAllocation};

/// An amount of a payment allocated to a target, in fils of the payment's
/// currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_allocations",
    index(payment_id),
    index(target_type, target_id)
)]
pub struct Allocation {
    /// The primary key.
    pub id: i64,
    /// The payment.
    pub payment_id: i64,
    /// The type of the target, e.g. `invoicing.invoice`.
    pub target_type: String,
    /// The id of the target.
    pub target_id: i64,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// When it was allocated.
    pub allocated_at: DateTime,
    /// The user who allocated it; `None` for the system or when the caller
    /// named no user.
    pub allocated_by: Option<i64>,
    /// When it was removed (by a reversal); `None` while active.
    pub removed_at: Option<DateTime>,
    /// Why it was removed.
    pub removed_reason: Option<String>,
    /// The user who removed it; `None` for the system or while active.
    pub removed_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Allocation {
    /// An allocation of `amount` fils of payment `payment_id` to `target`,
    /// now.
    pub fn new(payment_id: i64, target: &Subject, amount: i64, allocated_by: Option<i64>) -> Self {
        Allocation {
            payment_id,
            target_type: target.subject_type.clone(),
            target_id: target.subject_id,
            amount,
            allocated_at: renox::db::now(),
            allocated_by,
            ..Default::default()
        }
    }

    /// What the money is allocated to.
    pub fn target(&self) -> Subject {
        Subject {
            subject_type: self.target_type.clone(),
            subject_id: self.target_id,
        }
    }

    /// The target as the contract's [`AllocationTarget`]; `None` for a type
    /// this crate does not know.
    pub fn allocation_target(&self) -> Option<AllocationTarget> {
        AllocationTarget::from_subject(&self.target())
    }

    /// Whether it still counts: not removed by a reversal.
    pub fn is_active(&self) -> bool {
        self.removed_at.is_none()
    }

    /// The allocation as a reversal's event lists it.
    pub fn removed(&self) -> RemovedAllocation {
        RemovedAllocation {
            allocation: self.id,
            target: self.target(),
            amount: self.amount,
        }
    }

    /// The active allocations of payment `payment_id`, oldest first.
    pub async fn active_of(db: &Db, payment_id: i64) -> Result<Vec<Allocation>> {
        Allocation::query()
            .where_(Allocation::PAYMENT_ID.eq(payment_id))
            .where_(Allocation::REMOVED_AT.is_null())
            .order_by(Allocation::ID)
            .get(db)
            .await
    }

    /// All allocations of payment `payment_id`, removed ones too, oldest
    /// first.
    pub async fn all_of(db: &Db, payment_id: i64) -> Result<Vec<Allocation>> {
        Allocation::query()
            .where_(Allocation::PAYMENT_ID.eq(payment_id))
            .order_by(Allocation::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::InvoiceId;

    #[test]
    fn an_allocation_keeps_its_target() {
        let invoice = InvoiceId(8).subject();
        let allocation = Allocation::new(3, &invoice, 200_000, Some(5));
        assert_eq!(allocation.target(), invoice);
        assert_eq!(
            allocation.allocation_target(),
            Some(AllocationTarget::Invoice(InvoiceId(8)))
        );
        assert_eq!(
            (
                allocation.payment_id,
                allocation.amount,
                allocation.allocated_by
            ),
            (3, 200_000, Some(5))
        );
        assert!(allocation.is_active());
    }

    #[test]
    fn an_unknown_target_type_has_no_contract_target() {
        let allocation = Allocation::new(1, &Subject::new("property.deposit", 2), 10, None);
        assert_eq!(allocation.allocation_target(), None);
    }

    #[test]
    fn a_removed_allocation_is_listed_with_its_id() {
        let mut allocation = Allocation::new(3, &InvoiceId(8).subject(), 100_000, None);
        allocation.id = 11;
        allocation.removed_at = Some(renox::db::now());
        assert!(!allocation.is_active());
        let removed = allocation.removed();
        assert_eq!(removed.allocation, 11);
        assert_eq!(removed.target, InvoiceId(8).subject());
        assert_eq!(removed.amount, 100_000);
    }
}
