//! The events the vertical emits (09 Manpower §7.1). Other modules and the
//! issues that build on the base listen to them; each is emitted once, after
//! the commit of the transition it names.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

/// A client contract became `active` (`activate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractActivated {
    /// The contract, `manpower.client_contract:4`.
    pub contract: Subject,
    /// The client company.
    pub client: PartyId,
    /// The last day of the contract.
    pub end_date: NaiveDate,
}

impl Event for ContractActivated {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_events_round_trip_through_json() {
        let event = ContractActivated {
            contract: Subject::new("manpower.client_contract", 4),
            client: PartyId(9),
            end_date: NaiveDate::from_ymd_opt(2027, 5, 31).unwrap(),
        };
        let json = renox::serde_json::to_string(&event).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<ContractActivated>(&json).unwrap(),
            event
        );
    }
}
