//! Block reason codes (wl-parties §7): why a party is blocked. Each code has
//! a *restricted* flag; a restricted code is seen only by holders of
//! `parties.block_reason.view` (`PTY.ACL.02`), everyone else sees the
//! neutral `on_hold`.
//!
//! The codes live in code, not in a table: [`BlockReasons::default`] holds
//! the §7 defaults, and a vertical adds its own with
//! [`crate::WlParties::block_reason`]. `on_hold` is always present and never
//! restricted; redefining it panics at boot.

use std::sync::Arc;

use renox::prelude::*;
use wl_kernel::NEUTRAL_BLOCK_REASON;

/// One block reason code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockReason {
    /// The code, e.g. `non_payment`.
    pub code: String,
    /// Whether only holders of `parties.block_reason.view` may see and use it.
    pub restricted: bool,
}

/// The §7 defaults, `(code, restricted)`, in the order the block dialog
/// offers them.
pub const DEFAULT_REASONS: [(&str, bool); 7] = [
    (NEUTRAL_BLOCK_REASON, false),
    ("non_payment", false),
    ("legal_dispute", false),
    ("fraud_suspected", false),
    ("other", false),
    ("sanctions", true),
    ("aml_concern", true),
];

/// The configured block reason codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockReasons {
    reasons: Vec<BlockReason>,
}

impl Default for BlockReasons {
    /// The §7 defaults: `on_hold`, `non_payment`, `legal_dispute`,
    /// `fraud_suspected`, `other`, and the restricted `sanctions` and
    /// `aml_concern`.
    fn default() -> Self {
        BlockReasons {
            reasons: DEFAULT_REASONS
                .iter()
                .map(|(code, restricted)| BlockReason {
                    code: (*code).to_owned(),
                    restricted: *restricted,
                })
                .collect(),
        }
    }
}

impl BlockReasons {
    /// Adds the code `code`, restricted or not.
    ///
    /// A code that is already known can only be made restricted, never
    /// made visible again (a vertical may restrict `fraud_suspected`, but
    /// not open up `sanctions`).
    ///
    /// # Panics
    ///
    /// When `code` is `on_hold` (always present, never restricted), is
    /// blank or not a lowercase `snake_case` key, or would un-restrict a
    /// restricted code. This runs while the app is built, so a bad setup
    /// stops the boot.
    pub fn add(&mut self, code: &str, restricted: bool) -> &mut Self {
        assert!(
            code != NEUTRAL_BLOCK_REASON,
            "wl-parties: the block reason `{NEUTRAL_BLOCK_REASON}` is always present and \
             never restricted; it cannot be redefined"
        );
        assert!(
            is_code(code),
            "wl-parties: `{code}` is not a block reason code (lowercase letters, digits and `_`)"
        );
        match self.reasons.iter_mut().find(|r| r.code == code) {
            Some(known) => {
                assert!(
                    restricted || !known.restricted,
                    "wl-parties: the block reason `{code}` is restricted and cannot be made \
                     visible to everyone"
                );
                known.restricted = restricted;
            }
            None => self.reasons.push(BlockReason {
                code: code.to_owned(),
                restricted,
            }),
        }
        self
    }

    /// Checks the invariant: `on_hold` is present and not restricted.
    ///
    /// # Panics
    ///
    /// When it is not. The module calls it in `register`.
    pub fn check(&self) {
        let neutral = self.get(NEUTRAL_BLOCK_REASON).unwrap_or_else(|| {
            panic!("wl-parties: the block reason `{NEUTRAL_BLOCK_REASON}` is missing")
        });
        assert!(
            !neutral.restricted,
            "wl-parties: the block reason `{NEUTRAL_BLOCK_REASON}` cannot be restricted"
        );
    }

    /// The reason with this code.
    pub fn get(&self, code: &str) -> Option<&BlockReason> {
        self.reasons.iter().find(|r| r.code == code)
    }

    /// Whether `code` is a configured reason.
    pub fn contains(&self, code: &str) -> bool {
        self.get(code).is_some()
    }

    /// Whether `code` is restricted. An unknown code counts as restricted,
    /// so it is never shown by mistake.
    pub fn is_restricted(&self, code: &str) -> bool {
        self.get(code).is_none_or(|r| r.restricted)
    }

    /// Every reason, in order.
    pub fn all(&self) -> &[BlockReason] {
        &self.reasons
    }

    /// The reasons everyone may see and use: the non-restricted ones.
    pub fn unrestricted(&self) -> impl Iterator<Item = &BlockReason> {
        self.reasons.iter().filter(|r| !r.restricted)
    }

    /// The reasons a user may choose: all of them with
    /// `parties.block_reason.view` (`sees_restricted`), else the
    /// non-restricted ones.
    pub fn offered(&self, sees_restricted: bool) -> impl Iterator<Item = &BlockReason> {
        self.reasons
            .iter()
            .filter(move |r| sees_restricted || !r.restricted)
    }

    /// The app's reasons: those the module provided, or the defaults.
    pub fn of(state: &AppState) -> Arc<BlockReasons> {
        state
            .provided::<BlockReasons>()
            .unwrap_or_else(|| Arc::new(BlockReasons::default()))
    }
}

/// A code is a non-empty lowercase `snake_case` key.
fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_seven_codes_of_section_7() {
        let reasons = BlockReasons::default();
        let codes: Vec<_> = reasons.all().iter().map(|r| r.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "on_hold",
                "non_payment",
                "legal_dispute",
                "fraud_suspected",
                "other",
                "sanctions",
                "aml_concern"
            ]
        );
        let restricted: Vec<_> = reasons
            .all()
            .iter()
            .filter(|r| r.restricted)
            .map(|r| r.code.as_str())
            .collect();
        assert_eq!(restricted, ["sanctions", "aml_concern"]);
        reasons.check();
    }

    #[test]
    fn a_vertical_adds_a_restricted_code_and_it_is_hidden_like_the_defaults() {
        let mut reasons = BlockReasons::default();
        reasons
            .add("tax_evasion", true)
            .add("unpaid_deposit", false);
        assert!(reasons.is_restricted("tax_evasion"));
        assert!(!reasons.is_restricted("unpaid_deposit"));
        let offered: Vec<_> = reasons.offered(false).map(|r| r.code.as_str()).collect();
        assert!(!offered.contains(&"tax_evasion"));
        assert!(!offered.contains(&"sanctions"));
        assert!(offered.contains(&"unpaid_deposit"));
        assert!(offered.contains(&"on_hold"));
        let all: Vec<_> = reasons.offered(true).map(|r| r.code.as_str()).collect();
        assert!(all.contains(&"tax_evasion"));
        assert!(all.contains(&"aml_concern"));
    }

    #[test]
    fn a_known_code_can_be_restricted_but_never_opened_up() {
        let mut reasons = BlockReasons::default();
        reasons.add("fraud_suspected", true);
        assert!(reasons.is_restricted("fraud_suspected"));
        assert_eq!(reasons.all().len(), 7);
        let opened = std::panic::catch_unwind(|| {
            BlockReasons::default().add("sanctions", false);
        });
        assert!(opened.is_err());
    }

    #[test]
    #[should_panic(expected = "on_hold")]
    fn on_hold_cannot_be_made_restricted() {
        BlockReasons::default().add("on_hold", true);
    }

    #[test]
    #[should_panic(expected = "on_hold")]
    fn on_hold_cannot_be_redefined() {
        BlockReasons::default().add("on_hold", false);
    }

    #[test]
    #[should_panic(expected = "missing")]
    fn reasons_without_on_hold_fail_the_check() {
        let mut reasons = BlockReasons::default();
        reasons.reasons.retain(|r| r.code != NEUTRAL_BLOCK_REASON);
        reasons.check();
    }

    #[test]
    #[should_panic(expected = "not a block reason code")]
    fn a_code_is_a_snake_case_key() {
        BlockReasons::default().add("Bad Code", false);
    }

    #[test]
    fn an_unknown_code_counts_as_restricted() {
        let reasons = BlockReasons::default();
        assert!(!reasons.contains("nope"));
        assert!(reasons.is_restricted("nope"));
    }
}
