//! The Lot (wl-inventory §3, §4 *Lot*): a batch of one item, with its expiry
//! date and status. A goods receipt creates a lot for an accepted line of a
//! lot-tracked item, in `quarantine` or `released` as the item says. The
//! lifecycle transitions (`release`, `expire`...) come with the issues that
//! build them; the machine declares the states, and the system's `reject`
//! and `hold` are here.
//!
//! `hold` (§4 *Lot*): a stock hold on whole lots puts each lot that is
//! `quarantine` or `released` on hold, with the reason, and keeps the status
//! it had so a hold cancelled in error can give it back (`unhold`, BL-STK-12).
//! A lot that is `on_hold` or `expired` already is not held again.
//!
//! The rest of the hold's outcomes (BL-STK-12), all run by the system once
//! the lines of a hold are decided ([`crate::hold_decide::settle`]):
//! `release` (back to `released`), `discard` (what is left goes to wastage,
//! the lot is `rejected`) and `unhold` (a hold placed in error: back to the
//! status stored when `hold` ran; a transition has one end state, so
//! [`UNHOLD`] gives `released` and [`UNHOLD_TO_QUARANTINE`] `quarantine`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::LOT_SUBJECT_TYPE;

use crate::balance;
use crate::item::{Item, value_of};
use crate::movement::{MovementType, StockMovement};

/// The `reject` transition's name.
pub const REJECT: &str = "reject";

/// The `hold` transition's name.
pub const HOLD: &str = "hold";

/// The system's `release` transition's name: the lines of the hold were
/// decided and nothing else holds the lot.
pub const RELEASE: &str = "release";

/// The system's `discard` transition's name.
pub const DISCARD: &str = "discard";

/// The system's `unhold` transition's name (a cancelled hold; the lot goes
/// back to `released`).
pub const UNHOLD: &str = "unhold";

/// The system's `unhold` of a lot that was in quarantine before the hold.
pub const UNHOLD_TO_QUARANTINE: &str = "unhold_to_quarantine";

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
    /// The status the lot had when `hold` ran (`quarantine` or `released`):
    /// the one `unhold` gives back.
    pub status_before_hold: Option<String>,
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
            // The user's `reject` (`lots.release`) comes with the lot issues;
            // so far the system rejects a held lot when a Return to supplier
            // takes its last quantity.
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .system()
                    .from([Quarantine, OnHold])
                    .to(Rejected),
            )
            // Decision: `release` is the system's here; the user's run
            // (`lots.release`, passed inspection, `SOD`) is the lot issue's.
            .transition(
                Transition::new(RELEASE)
                    .label("Release")
                    .tone("success")
                    .system()
                    .from([Quarantine, OnHold])
                    .to(Released),
            )
            .transition(
                Transition::new(DISCARD)
                    .label("Discard")
                    .tone("danger")
                    .system()
                    .reason_required()
                    .from([OnHold, Expired])
                    .to(Rejected)
                    .effect(|lot: &Lot, ctx, tx| {
                        let id = lot.id;
                        let item_id = lot.item_id;
                        let reason = ctx.reason().map(str::to_owned);
                        let by = ctx.actor().map(|user| user.id);
                        Box::pin(async move {
                            // What the lines of the hold left is wasted now.
                            let item = Item::find_or_404(&mut *tx, item_id).await?;
                            let source = Subject::new(LOT_SUBJECT_TYPE, id);
                            for (place, quantity) in balance::lot_locations(&mut *tx, id).await? {
                                let mut moved = StockMovement::leaving(
                                    MovementType::Wastage,
                                    item_id,
                                    Some(id),
                                    place,
                                    quantity,
                                    item.average_cost,
                                    value_of(quantity, item.average_cost),
                                    &source,
                                    reason.clone(),
                                    by,
                                );
                                moved.insert(&mut *tx).await?;
                            }
                            Ok(())
                        })
                    }),
            )
            .transition(
                Transition::new(UNHOLD)
                    .label("Unhold")
                    .system()
                    .from([OnHold])
                    .to(Released),
            )
            .transition(
                Transition::new(UNHOLD_TO_QUARANTINE)
                    .label("Unhold (back to quarantine)")
                    .system()
                    .from([OnHold])
                    .to(Quarantine),
            )
            .transition(
                Transition::new(HOLD)
                    .label("Hold")
                    .tone("warning")
                    .system()
                    .reason_required()
                    .from([Quarantine, Released])
                    .to(OnHold)
                    .effect(|lot: &Lot, _ctx, tx| {
                        let id = lot.id;
                        let before = lot.status.as_str().to_owned();
                        Box::pin(async move {
                            // Not the status column, so outside the machine's guard.
                            Lot::query()
                                .where_(Lot::ID.eq(id))
                                .update(&mut *tx, &[("status_before_hold", &Some(before))])
                                .await?;
                            Ok(())
                        })
                    }),
            )
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
        for state in [
            "quarantine",
            "released",
            "on_hold",
            "expired",
            "rejected",
            "depleted",
        ] {
            assert!(names.contains(&state), "{state}");
        }
        let rejected = machine.states().iter().find(|s| s.name == "rejected");
        assert!(rejected.is_some_and(|s| s.terminal));
    }

    #[test]
    fn the_system_releases_discards_and_unholds_a_held_lot() {
        let machine = Lot::machine();
        let release = machine.find(RELEASE).expect("release");
        assert!(release.is_system());
        assert_eq!(release.from_states(), ["quarantine", "on_hold"]);
        assert_eq!(release.to_state(), Some("released"));
        let discard = machine.find(DISCARD).expect("discard");
        assert!(discard.is_system());
        assert_eq!(discard.from_states(), ["on_hold", "expired"]);
        assert_eq!(discard.to_state(), Some("rejected"));
        let unhold = machine.find(UNHOLD).expect("unhold");
        assert_eq!(unhold.to_state(), Some("released"));
        let back = machine
            .find(UNHOLD_TO_QUARANTINE)
            .expect("unhold to quarantine");
        assert_eq!(back.from_states(), ["on_hold"]);
        assert_eq!(back.to_state(), Some("quarantine"));
        assert!(unhold.is_system() && back.is_system());
    }

    #[test]
    fn the_system_holds_a_lot_that_is_quarantined_or_released() {
        let machine = Lot::machine();
        let hold = machine.find(HOLD).expect("hold");
        assert!(hold.is_system());
        assert_eq!(hold.from_states(), ["quarantine", "released"]);
        assert_eq!(hold.to_state(), Some("on_hold"));
    }
}
