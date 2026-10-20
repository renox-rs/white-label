//! The module's settings (wl-inventory §7), as far as receiving needs them:
//! the over-receipt tolerance and the rejection reasons. They are client
//! defaults kept in code, set on [`WlInventory`](crate::WlInventory) with
//! builder calls.

use serde::{Deserialize, Serialize};

/// The over-receipt tolerance by default (§7, `STK.ACL.04`): 5%, in basis
/// points.
pub const DEFAULT_TOLERANCE_BPS: i64 = 500;

/// The value of a Return to supplier above which a second person approves it
/// by default (§7, `STK.ACL.06`): AED 5,000, in fils.
pub const DEFAULT_RETURN_APPROVAL_FILS: i64 = 500_000;

/// The value of wastage or a write-off above which a second person approves
/// it by default (`STK.ACL.02`): AED 1,000, in fils.
pub const DEFAULT_WASTAGE_APPROVAL_FILS: i64 = 100_000;

/// The days a Return to supplier may stay `dispatched` before it is chased
/// by default (§7, `STK.RUL.08`).
pub const DEFAULT_RETURN_CHASE_DAYS: i64 = 30;

/// The rejection reasons by default (§7).
pub const DEFAULT_REASONS: &[&str] = &[
    "temperature",
    "quality",
    "damaged",
    "wrong_item",
    "short_shelf_life",
    "excess",
    "other",
];

/// One reason a quantity may be rejected at receiving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectionReason {
    /// The code, e.g. `temperature`.
    pub code: String,
    /// Whether a rejection for this reason needs a reading or a photo (06:
    /// a temperature reading from the receiving checklist).
    pub evidence_required: bool,
}

/// The settings of the inventory module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryConfig {
    /// How far the accepted quantity may go above the ordered one without
    /// approval, in basis points (500 = 5%).
    pub over_receipt_tolerance_bps: i64,
    /// The configured rejection reasons.
    pub rejection_reasons: Vec<RejectionReason>,
    /// The value, in fils, above which dispatching a Return to supplier needs
    /// a second person (`STK.ACL.06`).
    pub return_approval_fils: i64,
    /// The value, in fils, above which wastage (a credit the supplier
    /// refused) needs a second person (`STK.ACL.02`).
    pub wastage_approval_fils: i64,
    /// The days a Return to supplier stays `dispatched` before procurement
    /// and finance are told, and the days between two notices
    /// (`STK.RUL.08`).
    pub return_chase_days: i64,
}

impl InventoryConfig {
    /// The reason with `code`, if it is configured.
    pub fn reason(&self, code: &str) -> Option<&RejectionReason> {
        self.rejection_reasons.iter().find(|r| r.code == code)
    }
}

impl Default for InventoryConfig {
    fn default() -> Self {
        InventoryConfig {
            over_receipt_tolerance_bps: DEFAULT_TOLERANCE_BPS,
            return_approval_fils: DEFAULT_RETURN_APPROVAL_FILS,
            wastage_approval_fils: DEFAULT_WASTAGE_APPROVAL_FILS,
            return_chase_days: DEFAULT_RETURN_CHASE_DAYS,
            rejection_reasons: DEFAULT_REASONS
                .iter()
                .map(|code| RejectionReason {
                    code: (*code).to_owned(),
                    evidence_required: false,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_spec_values() {
        let config = InventoryConfig::default();
        assert_eq!(config.over_receipt_tolerance_bps, 500);
        assert_eq!(config.rejection_reasons.len(), 7);
        assert_eq!(config.return_approval_fils, 500_000);
        assert_eq!(config.wastage_approval_fils, 100_000);
        assert_eq!(config.return_chase_days, 30);
        assert!(
            config
                .reason("temperature")
                .is_some_and(|r| !r.evidence_required)
        );
        assert!(config.reason("spoiled").is_none());
    }
}
