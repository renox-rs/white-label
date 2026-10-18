//! The unallocations log (wl-invoicing §3, #70 A4 and A5): one row per
//! `unallocate` run, by idempotency key. A key that is in the log, `done` or
//! `failed`, is never run again.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::ReopenReason;

/// How an unallocation ended.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UnallocationOutcome {
    /// The invoice was reopened.
    #[default]
    Done,
    /// It was refused; the message says why.
    Failed,
}

/// An unallocation, done or refused. Amounts are fils of the invoice's
/// currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_unallocations", unique(key), index(invoice_id))]
pub struct Unallocation {
    /// The primary key.
    pub id: i64,
    /// The invoice (it may not exist, for a refused run).
    pub invoice_id: i64,
    /// The type of the money's source.
    pub source_type: String,
    /// The id of the money's source.
    pub source_id: i64,
    /// The amount asked, in fils.
    pub amount: i64,
    /// The idempotency key, e.g. `payments.allocation:<id>`.
    pub key: String,
    /// Why the invoice is reopened.
    pub reason: ReopenReason,
    /// The payment's reversal reason key, when there is one.
    pub reversal_reason: Option<String>,
    /// `done` or `failed`.
    pub outcome: UnallocationOutcome,
    /// The refusal's message; `None` when done.
    pub message: Option<String>,
    /// The transition that ran; `None` when failed.
    pub transition: Option<String>,
    /// The invoice's balance after, in fils; `None` when failed.
    pub balance_after: Option<i64>,
    /// The user who ran it; `None` for the system.
    pub ran_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Unallocation {
    /// Where the money came from.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The log row of `key`, if the key was used.
    pub async fn by_key(db: &Db, key: &str) -> Result<Option<Unallocation>> {
        Unallocation::query()
            .where_(Unallocation::KEY.eq(key.to_owned()))
            .first(db)
            .await
    }
}
