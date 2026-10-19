//! A supplier bill allocation (wl-invoicing §3): money from an outgoing
//! payment or a debit note applied to a supplier bill. The `pay` service
//! writes it in the same transaction as the bill's balance.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// An amount applied to a [`SupplierBill`](crate::supplier_bill::SupplierBill),
/// in fils of the bill's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_supplier_bill_allocations",
    unique(key),
    index(bill_id),
    index(source_type, source_id)
)]
pub struct BillAllocation {
    /// The primary key.
    pub id: i64,
    /// The bill.
    pub bill_id: i64,
    /// The type of the money's source: an outgoing payment or a debit note.
    pub source_type: String,
    /// The id of the money's source.
    pub source_id: i64,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// The idempotency key of the payment that wrote it, when it had one.
    pub key: Option<String>,
    /// When it was applied.
    pub allocated_at: DateTime,
    /// The user who applied it; `None` for the system.
    pub created_by: Option<i64>,
    /// When it was undone; `None` while it is active. A removed row is never
    /// deleted.
    pub removed_at: Option<DateTime>,
    /// Why it was removed (`payment_reversed`, `debit_note_cancelled`).
    pub removed_reason: Option<String>,
    /// The idempotency key of the unallocation that removed it.
    pub removed_key: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BillAllocation {
    /// An allocation of `amount` fils from `source` to bill `bill_id`, now.
    pub fn new(bill_id: i64, source: &Subject, amount: i64, created_by: Option<i64>) -> Self {
        BillAllocation {
            bill_id,
            source_type: source.subject_type.clone(),
            source_id: source.subject_id,
            amount,
            allocated_at: renox::db::now(),
            created_by,
            ..Default::default()
        }
    }

    /// Whether it still counts (it was not removed).
    pub fn is_active(&self) -> bool {
        self.removed_at.is_none()
    }

    /// Where the money came from.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The allocation written under `key`, if the key was used.
    pub async fn by_key(db: &Db, key: &str) -> Result<Option<BillAllocation>> {
        BillAllocation::query()
            .where_(BillAllocation::KEY.eq(key.to_owned()))
            .first(db)
            .await
    }

    /// The active allocations of bill `bill_id` from `source`, oldest first.
    pub async fn active_from(
        tx: &mut renox::db::Transaction,
        bill_id: i64,
        source: &Subject,
    ) -> Result<Vec<BillAllocation>> {
        BillAllocation::query()
            .where_(BillAllocation::BILL_ID.eq(bill_id))
            .where_(BillAllocation::SOURCE_TYPE.eq(source.subject_type.clone()))
            .where_(BillAllocation::SOURCE_ID.eq(source.subject_id))
            .where_(BillAllocation::REMOVED_AT.is_null())
            .order_by(BillAllocation::ID)
            .get(&mut *tx)
            .await
    }

    /// The allocations of bill `bill_id`, oldest first, removed ones too.
    pub async fn of_bill(db: &Db, bill_id: i64) -> Result<Vec<BillAllocation>> {
        BillAllocation::query()
            .where_(BillAllocation::BILL_ID.eq(bill_id))
            .order_by(BillAllocation::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_allocation_keeps_its_source_and_starts_active() {
        let payment = Subject::new("payments.payment", 12);
        let allocation = BillAllocation::new(3, &payment, 1_050_000, Some(5));
        assert_eq!(allocation.source(), payment);
        assert_eq!(
            (allocation.bill_id, allocation.amount, allocation.created_by),
            (3, 1_050_000, Some(5))
        );
        assert!(allocation.is_active());
        let removed = BillAllocation {
            removed_at: Some(renox::db::now()),
            ..allocation
        };
        assert!(!removed.is_active());
    }
}
