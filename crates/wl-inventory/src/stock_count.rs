//! The Stock count (wl-inventory §3, §4 *Stock count*, `STK.CNT.01`): a
//! counting session at a location. Only what opening and abandoning one needs
//! is built: the model, the machine's states, `cancel` (`stock.approve`, with
//! a reason) and [`open_for`], which a vertical asks for through the kernel's
//! [`ScopeStock`](wl_kernel::inventory::ScopeStock) when a site stops. The
//! lines, `submit`, `request_recount` and `approve` come with the issues that
//! build them.
//!
//! ```text
//! open --cancel--> cancelled
//! ```

use renox::prelude::*;
use renox::state_machine::{Machine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::STOCK_COUNT_SUBJECT_TYPE;

use crate::permissions;

/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// Where a stock count stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StockCountStatus {
    /// Counting.
    #[default]
    Open,
    /// Counts entered; locked.
    Submitted,
    /// Some lines sent back for recounting.
    Recount,
    /// Variances accepted; adjustments posted (terminal).
    Approved,
    /// Abandoned; no adjustment (terminal).
    Cancelled,
}

impl StockCountStatus {
    /// The states in which the count is not finished.
    pub const UNFINISHED: [StockCountStatus; 3] = [
        StockCountStatus::Open,
        StockCountStatus::Submitted,
        StockCountStatus::Recount,
    ];
}

/// A counting session at a location.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_stock_counts", state_machine, index(location_id))]
pub struct StockCount {
    /// The primary key.
    pub id: i64,
    /// The location counted.
    pub location_id: i64,
    /// Why it was opened.
    pub reason: Option<String>,
    /// The record that asked for it, as a subject text.
    pub source: Option<String>,
    /// Whether the expected quantities are hidden from the counters.
    pub blind: bool,
    /// The user who opened it.
    pub opened_by: Option<i64>,
    /// When it was opened.
    pub opened_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: StockCountStatus,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StockCount {
    /// The count as a [`Subject`]: `inventory.stock_count:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(STOCK_COUNT_SUBJECT_TYPE, self.id)
    }
}

impl renox::state_machine::StateMachine for StockCount {
    type State = StockCountStatus;
    const COLUMN: renox::db::Col<StockCount, StockCountStatus> = StockCount::STATUS;

    fn machine() -> Machine<Self> {
        use StockCountStatus::{Approved, Cancelled, Open, Recount, Submitted};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Recount)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .tone("danger")
                    .permission(permissions::STOCK_APPROVE)
                    .reason_required()
                    .from([Open])
                    .to(Cancelled)
                    .effect(|count: &StockCount, ctx, tx| {
                        let id = count.id;
                        let reason = ctx.reason().map(|reason| reason.trim().to_owned());
                        Box::pin(async move {
                            StockCount::query()
                                .where_(StockCount::ID.eq(id))
                                .update(&mut *tx, &[("cancel_reason", &reason)])
                                .await?;
                            Ok(())
                        })
                    }),
            )
    }
}

/// The counts of `location_id` that are not finished.
///
/// # Errors
///
/// A database error.
pub async fn unfinished_at(db: &Db, location_id: i64) -> Result<Vec<StockCount>> {
    StockCount::query()
        .where_(StockCount::LOCATION_ID.eq(location_id))
        .where_(StockCount::STATUS.is_in(StockCountStatus::UNFINISHED))
        .order_by(StockCount::ID)
        .get(db)
        .await
}

/// Opens a count of `location_id` for `reason`, asked by `source`, unless the
/// location has one that is not finished: that one is returned and nothing is
/// opened. The boolean says whether this call opened it.
///
/// # Errors
///
/// A database error.
pub async fn open_for(
    db: &Db,
    location_id: i64,
    reason: &str,
    source: &Subject,
) -> Result<(StockCount, bool)> {
    if let Some(found) = unfinished_at(db, location_id).await?.into_iter().next() {
        return Ok((found, false));
    }
    let mut count = StockCount {
        location_id,
        reason: Some(reason.trim().to_owned()),
        source: Some(source.to_string()),
        opened_at: Some(renox::db::now()),
        status: StockCountStatus::Open,
        ..StockCount::default()
    };
    count.save(db).await?;
    Ok((count, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::StateMachine;

    #[test]
    fn a_count_opens_and_only_an_open_one_is_cancelled() {
        let machine = StockCount::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        let cancel = machine.find(CANCEL).expect("transition");
        assert_eq!(cancel.from_states(), ["open"]);
        assert_eq!(cancel.to_state(), Some("cancelled"));
        assert!(cancel.needs_reason());
    }
}
