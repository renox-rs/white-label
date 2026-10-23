//! The cancellation reason codes 10 adds to wl-scheduling's list (10 §7.1
//! *Booking*). The host gives them to the module with
//! `WlScheduling::reason_code` (`host/src/app/edtech_seed.rs`).

use wl_kernel::scheduling::Initiator;

/// The tutor is sick or has left: a one-to-one or trial booking is cancelled
/// by the centre (§4.2). Business-initiated, so any deposit is refunded in
/// full.
pub const TUTOR_UNAVAILABLE: &str = "tutor_unavailable";

/// The tutor is no longer eligible (police clearance, permit or visa
/// expired) on the booking's date (§4.2, `EDU.ACL.14`). Business-initiated.
pub const TUTOR_INELIGIBLE: &str = "tutor_ineligible";

/// The codes 10 adds, each with its default *initiated by*.
pub const SCHEDULING_REASON_CODES: &[(&str, Initiator)] = &[
    (TUTOR_UNAVAILABLE, Initiator::Business),
    (TUTOR_INELIGIBLE, Initiator::Business),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tutor_who_cannot_teach_is_the_centres_side() {
        assert!(
            SCHEDULING_REASON_CODES
                .iter()
                .all(|(_, by)| *by == Initiator::Business)
        );
        assert_eq!(SCHEDULING_REASON_CODES.len(), 2);
    }
}
