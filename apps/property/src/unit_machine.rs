//! The Unit state machine (01 Property §7.1 *Unit*, `PRP.PRT.02`).
//!
//! ```text
//! [*] available
//! available --reserve--> reserved --release--> available        (reservations are Later)
//! available, reserved --occupy--> occupied                       (system: lease activate)
//! occupied --mark_notice--> notice_given --cancel_notice--> occupied
//! notice_given, occupied --vacate--> available                   (system: lease terminate)
//! notice_given, occupied --vacate_to_maintenance--> under_maintenance
//! under_maintenance --mark_ready--> available
//! available, under_maintenance --withdraw--> off_market --relist--> available
//! ```
//!
//! `vacate` has two targets in §7.1, so it is two transitions here, as the
//! other machines of the repository split a transition by target.
//! A renewal or amendment version of a lease never changes the unit's state.
//!
//! Decisions on what the spec leaves open: `reserve` needs the booking
//! deposit, which comes with reservations (`PRP.LEA.02`, Later), so it is
//! declared and refuses with that reason; `mark_ready` does not check open
//! tickets yet, since no kernel contract reads the tickets of a subject.

use renox::state_machine::{Machine, StateMachine, Transition};

use crate::building::Building;
use crate::permissions;
use crate::unit::{Unit, UnitStatus};

/// `reserve`: a prospect holds the unit.
pub const RESERVE: &str = "reserve";
/// `release`: a reservation ends (a user).
pub const RELEASE: &str = "release";
/// `release_by_system`: a reservation ends because the lease was cancelled.
pub const RELEASE_BY_SYSTEM: &str = "release_by_system";
/// `occupy`: a new lease is activated.
pub const OCCUPY: &str = "occupy";
/// `mark_notice`: the tenant gave notice.
pub const MARK_NOTICE: &str = "mark_notice";
/// `cancel_notice`: the notice was withdrawn.
pub const CANCEL_NOTICE: &str = "cancel_notice";
/// `vacate`: the tenant moved out; the unit is available.
pub const VACATE: &str = "vacate";
/// `vacate_to_maintenance`: the tenant moved out; the move-out inspection
/// opened tickets.
pub const VACATE_TO_MAINTENANCE: &str = "vacate_to_maintenance";
/// `mark_ready`: the work is done.
pub const MARK_READY: &str = "mark_ready";
/// `withdraw`: the unit is taken off the market.
pub const WITHDRAW: &str = "withdraw";
/// `relist`: the unit is back on the market.
pub const RELIST: &str = "relist";

/// A transition a user runs, in the scope of the unit's building.
fn user(name: &str, label: &str, permission: &'static str) -> Transition<Unit> {
    Transition::new(name)
        .label(label)
        .permission_in(permission, |unit: &Unit| {
            Building::scope_of(unit.building_id)
        })
}

/// A transition only the system runs (the lease machine, a rule).
fn system(name: &str, label: &str) -> Transition<Unit> {
    Transition::new(name).label(label).system()
}

impl StateMachine for Unit {
    type State = UnitStatus;
    const COLUMN: renox::db::Col<Unit, UnitStatus> = Unit::STATUS;

    fn machine() -> Machine<Self> {
        use UnitStatus::{Available, NoticeGiven, Occupied, OffMarket, Reserved, UnderMaintenance};
        Machine::new()
            .initial([Available])
            .state(Available)
            .tone("info")
            .state(Reserved)
            .tone("info")
            .state(Occupied)
            .tone("success")
            .state(NoticeGiven)
            .tone("warning")
            .state(UnderMaintenance)
            .tone("warning")
            .state(OffMarket)
            .tone("neutral")
            .transition(
                user(RESERVE, "Reserve", permissions::UNITS_RESERVE)
                    .from([Available])
                    .to(Reserved)
                    .guard("booking_deposit", |_unit: &Unit, _ctx| {
                        Box::pin(async {
                            Err(crate::texts::english("property.units.reserve_later", &[]))
                        })
                    }),
            )
            .transition(
                user(RELEASE, "Release", permissions::UNITS_RESERVE)
                    .from([Reserved])
                    .to(Available),
            )
            .transition(
                system(RELEASE_BY_SYSTEM, "Release")
                    .from([Reserved])
                    .to(Available),
            )
            .transition(
                system(OCCUPY, "Occupy")
                    .from([Available, Reserved])
                    .to(Occupied),
            )
            .transition(
                system(MARK_NOTICE, "Mark notice")
                    .from([Occupied])
                    .to(NoticeGiven),
            )
            .transition(
                system(CANCEL_NOTICE, "Cancel notice")
                    .from([NoticeGiven])
                    .to(Occupied),
            )
            .transition(
                system(VACATE, "Vacate")
                    .from([NoticeGiven, Occupied])
                    .to(Available),
            )
            .transition(
                system(VACATE_TO_MAINTENANCE, "Vacate to maintenance")
                    .from([NoticeGiven, Occupied])
                    .to(UnderMaintenance),
            )
            .transition(
                user(MARK_READY, "Mark ready", permissions::UNITS_MANAGE)
                    .from([UnderMaintenance])
                    .to(Available),
            )
            .transition(
                user(WITHDRAW, "Take off the market", permissions::UNITS_MANAGE)
                    .from([Available, UnderMaintenance])
                    .to(OffMarket),
            )
            .transition(
                user(RELIST, "Put back on the market", permissions::UNITS_MANAGE)
                    .from([OffMarket])
                    .to(Available),
            )
    }
}

/// Registers the machine with `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<Unit>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = Unit::machine();
        let table: [(&str, &[&str], &str, bool); 11] = [
            (RESERVE, &["available"], "reserved", false),
            (RELEASE, &["reserved"], "available", false),
            (RELEASE_BY_SYSTEM, &["reserved"], "available", true),
            (OCCUPY, &["available", "reserved"], "occupied", true),
            (MARK_NOTICE, &["occupied"], "notice_given", true),
            (CANCEL_NOTICE, &["notice_given"], "occupied", true),
            (VACATE, &["notice_given", "occupied"], "available", true),
            (
                VACATE_TO_MAINTENANCE,
                &["notice_given", "occupied"],
                "under_maintenance",
                true,
            ),
            (MARK_READY, &["under_maintenance"], "available", false),
            (
                WITHDRAW,
                &["available", "under_maintenance"],
                "off_market",
                false,
            ),
            (RELIST, &["off_market"], "available", false),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), by_system, "{name}");
        }
    }

    #[test]
    fn a_unit_starts_available_and_every_state_has_a_tone() {
        let machine = Unit::machine();
        assert_eq!(machine.initial_states(), ["available"]);
        assert_eq!(machine.states().len(), 6);
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
    }
}
