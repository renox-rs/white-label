//! The neutral reason codes of an engagement (04 §6 *Engagement*: "suspension
//! or end reason (neutral code shown to staff)", `ACC.ACL.14`).
//!
//! An engagement stores only one of these codes, never a restricted reason:
//! a sanctions or AML concern lives on the party's restricted Block record or
//! on the CDD file's exit reason, which only the MLRO and deputy see. A code
//! outside these lists is refused, so `sanctions` or `aml_concern` can never
//! be typed in as a suspension or end reason.

use renox::prelude::*;

use crate::texts;

/// A suspension because the client is blocked, whatever the block's reason,
/// shown as **On hold** (the kernel's neutral block reason).
pub const ON_HOLD: &str = wl_kernel::NEUTRAL_BLOCK_REASON;
/// The end after a CDD exit or a rejected review, shown as **Ended**.
pub const RELATIONSHIP_ENDED: &str = "relationship_ended";
/// Non-payment (a user's suspension or end, or `ACC.RUL.07`).
pub const NON_PAYMENT: &str = "non_payment";
/// At the client's request.
pub const CLIENT_REQUEST: &str = "client_request";
/// Another reason.
pub const OTHER: &str = "other";
/// The engagement letter was not signed (`ACC.RUL.11`).
pub const LETTER_NOT_SIGNED: &str = "letter_not_signed";
/// The client withdrew before the engagement started.
pub const CLIENT_WITHDREW: &str = "client_withdrew";
/// The client was not accepted (the CDD file was rejected); the CDD reason
/// stays on the file, for the MLRO only.
pub const NOT_ACCEPTED: &str = "not_accepted";

/// The reasons a user gives for `suspend`.
pub const SUSPEND_REASONS: [&str; 3] = [NON_PAYMENT, CLIENT_REQUEST, OTHER];
/// The reasons the system gives for its suspension (`PartyBlocked`,
/// `ACC.RUL.07`).
pub const SYSTEM_SUSPEND_REASONS: [&str; 2] = [ON_HOLD, NON_PAYMENT];
/// The reasons for `end`.
pub const END_REASONS: [&str; 4] = [CLIENT_REQUEST, NON_PAYMENT, RELATIONSHIP_ENDED, OTHER];
/// The reasons for `decline`.
pub const DECLINE_REASONS: [&str; 3] = [LETTER_NOT_SIGNED, CLIENT_WITHDREW, NOT_ACCEPTED];

/// Whether `code` is one of `allowed`.
pub fn is_one_of(code: Option<&str>, allowed: &[&str]) -> bool {
    code.is_some_and(|code| allowed.contains(&code.trim()))
}

/// The label of `code` in `lang`: "On hold", "Ended", "Non-payment"… A code
/// the app does not know shows as **On hold**, so nothing unknown is ever
/// shown.
pub fn label(lang: &Lang, code: &str) -> String {
    let key = format!("accounting.reasons.{code}");
    if texts::has(&key) {
        texts::t(lang, &key)
    } else {
        texts::t(lang, "accounting.reasons.on_hold")
    }
}

/// [`label`] in English.
pub fn label_en(code: &str) -> String {
    let key = format!("accounting.reasons.{code}");
    if texts::has(&key) {
        texts::english(&key, &[])
    } else {
        texts::english("accounting.reasons.on_hold", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_codes_are_not_engagement_reasons() {
        for code in ["sanctions", "aml_concern"] {
            assert!(!is_one_of(Some(code), &SUSPEND_REASONS));
            assert!(!is_one_of(Some(code), &SYSTEM_SUSPEND_REASONS));
            assert!(!is_one_of(Some(code), &END_REASONS));
            assert!(!is_one_of(Some(code), &DECLINE_REASONS));
        }
        assert!(is_one_of(Some(" client_request "), &SUSPEND_REASONS));
        assert!(!is_one_of(None, &SUSPEND_REASONS));
    }

    #[test]
    fn the_neutral_codes_read_on_hold_and_ended() {
        assert_eq!(ON_HOLD, "on_hold");
        assert_eq!(label_en(ON_HOLD), "On hold");
        assert_eq!(label_en(RELATIONSHIP_ENDED), "Ended");
        assert_eq!(label_en("sanctions"), "On hold");
    }
}
