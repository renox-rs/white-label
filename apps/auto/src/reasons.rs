//! The cancellation reason codes 08 adds to wl-scheduling's list (08 §7.1
//! *Booking*). The host gives them to the module with
//! `WlScheduling::reason_code` (`host/src/app/auto_seed.rs`).

use wl_kernel::scheduling::Initiator;

/// The customer declined the estimate or left before one was sent: the
/// booking is cancelled with the job card's `decline` (`SCH.ACL.06`).
pub const ESTIMATE_DECLINED: &str = "estimate_declined";

/// A duplicate check-in: the job card is voided and its booking cancelled
/// with work done *none*. The workshop's side.
pub const DUPLICATE_CHECK_IN: &str = "duplicate_check_in";

/// The customer stopped the job after work started: the booking is cancelled
/// with the work done billed on the job card.
pub const JOB_STOPPED: &str = "job_stopped";

/// A fleet contract ended or was terminated: the bookings after its last day
/// of service are cancelled by the system (`BL-AUT-12`, `BL-AUT-13`). The
/// workshop's side.
pub const FLEET_CONTRACT_ENDED: &str = "fleet_contract_ended";

/// The advisor cancels a booking kept while the fleet contract is suspended
/// (`BL-AUT-13`). The workshop's side.
pub const FLEET_SUSPENDED: &str = "fleet_suspended";

/// The codes 08 adds, each with its default *initiated by*.
pub const SCHEDULING_REASON_CODES: &[(&str, Initiator)] = &[
    (ESTIMATE_DECLINED, Initiator::Party),
    (DUPLICATE_CHECK_IN, Initiator::Business),
    (JOB_STOPPED, Initiator::Party),
    (FLEET_CONTRACT_ENDED, Initiator::Business),
    (FLEET_SUSPENDED, Initiator::Business),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_codes_are_distinct_and_say_whose_side_they_are() {
        let mut codes: Vec<&str> = SCHEDULING_REASON_CODES.iter().map(|(c, _)| *c).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 5);
        let duplicate = SCHEDULING_REASON_CODES
            .iter()
            .find(|(code, _)| *code == DUPLICATE_CHECK_IN)
            .unwrap();
        assert_eq!(duplicate.1, Initiator::Business);
    }
}
