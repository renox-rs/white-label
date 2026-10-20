//! The Lot (wl-inventory §3, §4 *Lot*): a batch of one item, with its expiry
//! date and status. A goods receipt creates a lot for an accepted line of a
//! lot-tracked item, in `quarantine` or `released` as the item says. The
//! lifecycle transitions (`release`, `hold`, `expire`...) come with the
//! issues that build them; the machine declares the states.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// Where a lot stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LotStatus {
    /// Received or produced; not usable until inspected.
    Quarantine,
    /// Usable, pickable and sellable.
    #[default]
    Released,
    /// The whole lot is blocked by an open stock hold.
    OnHold,
    /// Past its expiry date.
    Expired,
    /// Failed inspection, discarded or returned (terminal).
    Rejected,
    /// Nothing left (terminal).
    Depleted,
}

/// A batch of one item.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_lots", state_machine, unique(item_id, number))]
pub struct Lot {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The lot number.
    pub number: String,
    /// The supplier's lot number.
    pub supplier_lot: Option<String>,
    /// The type of the record the lot came from.
    pub source_type: Option<String>,
    /// The id of the record the lot came from.
    pub source_id: Option<i64>,
    /// The day the lot expires.
    pub expiry_date: Option<NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: LotStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Lot {
    /// The record the lot came from.
    pub fn source(&self) -> Option<Subject> {
        match (&self.source_type, self.source_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }
}

impl StateMachine for Lot {
    type State = LotStatus;
    const COLUMN: renox::db::Col<Lot, LotStatus> = Lot::STATUS;

    fn machine() -> Machine<Self> {
        use LotStatus::{Depleted, Expired, OnHold, Quarantine, Rejected, Released};
        Machine::new()
            .initial([Quarantine, Released])
            .state(Quarantine)
            .tone("warning")
            .state(Released)
            .tone("success")
            .state(OnHold)
            .tone("warning")
            .state(Expired)
            .tone("error")
            .state(Rejected)
            .tone("error")
            .terminal()
            .state(Depleted)
            .tone("neutral")
            .terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lot_starts_quarantined_or_released() {
        let machine = Lot::machine();
        assert_eq!(machine.initial_states(), ["quarantine", "released"]);
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in ["quarantine", "released", "on_hold", "expired", "rejected", "depleted"] {
            assert!(names.contains(&state), "{state}");
        }
        let rejected = machine.states().iter().find(|s| s.name == "rejected");
        assert!(rejected.is_some_and(|s| s.terminal));
    }
}
