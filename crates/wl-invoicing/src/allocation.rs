//! An allocation (wl-invoicing §3): money from a payment or a credit note
//! set against an invoice. The `allocate` service writes it in the same
//! transaction as the invoice's balance.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// An amount allocated to an [`Invoice`](crate::invoice::Invoice), in fils
/// of the invoice's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_allocations",
    index(invoice_id),
    index(source_type, source_id)
)]
pub struct Allocation {
    /// The primary key.
    pub id: i64,
    /// The invoice.
    pub invoice_id: i64,
    /// The type of the money's source: a payment or a credit note.
    pub source_type: String,
    /// The id of the money's source.
    pub source_id: i64,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// When it was allocated.
    pub allocated_at: DateTime,
    /// The user who allocated it; `None` for the system.
    pub created_by: Option<i64>,
    /// When it was undone (`unallocate`); `None` while it is active. A
    /// removed row is never deleted.
    pub removed_at: Option<DateTime>,
    /// Why it was removed (`payment_reversed`, ...).
    pub removed_reason: Option<String>,
    /// The idempotency key of the unallocation that removed it.
    pub removed_key: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Allocation {
    /// An allocation of `amount` fils from `source` to invoice `invoice_id`,
    /// now.
    pub fn new(invoice_id: i64, source: &Subject, amount: i64, created_by: Option<i64>) -> Self {
        Allocation {
            invoice_id,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_allocation_keeps_its_source() {
        let payment = Subject::new("payments.payment", 12);
        let allocation = Allocation::new(3, &payment, 42_000, Some(5));
        assert_eq!(allocation.source(), payment);
        assert_eq!(
            (
                allocation.invoice_id,
                allocation.amount,
                allocation.created_by
            ),
            (3, 42_000, Some(5))
        );
    }
}
