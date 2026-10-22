//! The cancellation reason codes 05 adds to wl-scheduling's list (05 §7.1
//! *Appointment*, wl-scheduling §4.1). The host gives them to the module with
//! `WlScheduling::reason_code` (`host/src/app/clinics_seed.rs`).

use wl_kernel::scheduling::Initiator;

/// The patient's cover failed at check-in (eligibility negative, card or
/// policy expired) and the patient declined self-pay (`CLN.ACL.15`). It
/// counts as the clinic's side for the deposit: refunded in full.
pub const COVERAGE_NOT_CONFIRMED: &str = "coverage_not_confirmed";

/// The codes 05 adds, each with its default *initiated by*.
pub const SCHEDULING_REASON_CODES: &[(&str, Initiator)] =
    &[(COVERAGE_NOT_CONFIRMED, Initiator::Business)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_not_confirmed_is_the_clinics_side() {
        assert_eq!(
            SCHEDULING_REASON_CODES,
            &[("coverage_not_confirmed", Initiator::Business)]
        );
    }
}
