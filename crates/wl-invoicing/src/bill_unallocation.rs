//! The bill unallocations log (wl-invoicing §3, #75 A4 and A6): one row per
//! supplier bill `unallocate` run, by idempotency key. A key that is in the
//! log, `done` or `failed`, is never run again.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::BillReopenReason;

use crate::unallocation::UnallocationOutcome;

/// A bill unallocation, done or refused. Amounts are fils of the bill's
/// currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_bill_unallocations", unique(key), index(bill_id))]
pub struct BillUnallocation {
    /// The primary key.
    pub id: i64,
    /// The bill (it may not exist, for a refused run).
    pub bill_id: i64,
    /// The type of the money's source.
    pub source_type: String,
    /// The id of the money's source.
    pub source_id: i64,
    /// The amount asked, in fils.
    pub amount: i64,
    /// The idempotency key, e.g. `payments.allocation:<id>`.
    pub key: String,
    /// Why the bill is reopened.
    pub reason: BillReopenReason,
    /// The payment's reversal reason key, when there is one.
    pub reversal_reason: Option<String>,
    /// `done` or `failed`.
    pub outcome: UnallocationOutcome,
    /// The refusal's message; `None` when done.
    pub message: Option<String>,
    /// The transition that ran; `None` when failed.
    pub transition: Option<String>,
    /// The bill's balance after, in fils; `None` when failed.
    pub balance_after: Option<i64>,
    /// The user who ran it; `None` for the system.
    pub ran_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BillUnallocation {
    /// Where the money came from.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The log row of `key`, if the key was used.
    pub async fn by_key(db: &Db, key: &str) -> Result<Option<BillUnallocation>> {
        BillUnallocation::query()
            .where_(BillUnallocation::KEY.eq(key.to_owned()))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_keeps_its_source_and_starts_done() {
        let row = BillUnallocation {
            source_type: "payments.payment".into(),
            source_id: 9,
            ..Default::default()
        };
        assert_eq!(row.source(), Subject::new("payments.payment", 9));
        assert_eq!(row.outcome, UnallocationOutcome::Done);
        assert_eq!(row.reason, BillReopenReason::PaymentReversed);
    }
}
