//! A recovery (wl-invoicing §3, #66): a payment set against a written-off
//! invoice. The `recover` service writes one row per recovery, in the same
//! transaction as the allocation, so that a reversal can undo exactly it.
//! Amounts are fils of the invoice's currency; the VAT share is AED fils.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::vat_adjustment::AdjustmentStatus;

/// What one recovery did to the invoice and to its bad-debt relief.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_recoveries",
    unique(allocation_id),
    index(invoice_id),
    index(source_type, source_id)
)]
pub struct Recovery {
    /// The primary key.
    pub id: i64,
    /// The written-off invoice.
    pub invoice_id: i64,
    /// The type of the money's source, e.g. `payments.payment`.
    pub source_type: String,
    /// The id of the money's source.
    pub source_id: i64,
    /// The allocation the recovery wrote.
    pub allocation_id: i64,
    /// The amount recovered, in fils; always > 0.
    pub amount: i64,
    /// The total recovered after this one, in fils.
    pub recovered_total: i64,
    /// The VAT share of the amount, in AED fils.
    pub vat_share: i64,
    /// Whether the relief was claimed (`included`) at the time.
    pub relief_claimed: bool,
    /// The status of the bad-debt relief it saw; `None` without a relief.
    pub relief_status: Option<AdjustmentStatus>,
    /// The bad-debt relief adjustment it saw.
    pub relief_adjustment_id: Option<i64>,
    /// The `bad_debt_recovery` adjustment it created.
    pub recovery_adjustment_id: Option<i64>,
    /// The day it was recovered, in Asia/Dubai.
    pub recovered_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Recovery {
    /// Where the money came from.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The recoveries of invoice `invoice_id`, oldest first.
    pub async fn of_invoice(db: &Db, invoice_id: i64) -> Result<Vec<Recovery>> {
        Recovery::query()
            .where_(Recovery::INVOICE_ID.eq(invoice_id))
            .order_by(Recovery::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recovery_keeps_its_source() {
        let recovery = Recovery {
            source_type: "payments.payment".into(),
            source_id: 7,
            ..Default::default()
        };
        assert_eq!(recovery.source(), Subject::new("payments.payment", 7));
    }
}
