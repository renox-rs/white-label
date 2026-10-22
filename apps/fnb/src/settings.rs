//! The settings of the vertical (06 F&B §7.2, §9, §13). A client sets them in
//! code on [`Fnb`](crate::Fnb) (`Fnb::new().cod_failures(2, 90)`); the screens
//! and the rules read them with [`settings`].

use std::sync::Arc;

use renox::prelude::*;

/// Failures of cash orders that turn cash on delivery off (`FNB.RUL.14`, N).
pub const DEFAULT_COD_FAILURES: i64 = 2;

/// The days those failures are counted over (`FNB.RUL.14`, D).
pub const DEFAULT_COD_WINDOW_DAYS: i64 = 90;

/// The days after a failed delivery was closed in which a cashier may ask for
/// a manager-approved refund (`FNB.ACL.17`).
pub const DEFAULT_REFUND_REQUEST_DAYS: i64 = 7;

/// Minutes after an aggregator order is `ready` when the kitchen manager is
/// told to chase the aggregator (`FNB.RUL.16`, N).
pub const DEFAULT_PICKUP_CHASE_MINUTES: i64 = 15;

/// Minutes after it is `ready` when the kitchen manager gets the task to
/// record what happened (`FNB.RUL.16`, M).
pub const DEFAULT_PICKUP_TASK_MINUTES: i64 = 45;

/// The settings. `Default` is the spec's defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnbSettings {
    /// Whether a prepaid order is refunded in full after a failure on the
    /// customer's side too. Off: the spec's default, until the lawyer answers
    /// open item L-33 (consumer protection).
    pub customer_side_refund: bool,
    /// The days a refund may be requested after the close (`FNB.ACL.17`).
    pub refund_request_days: i64,
    /// Customer-side failures of cash orders that turn cash on delivery off.
    pub cod_failures: i64,
    /// The days they are counted over.
    pub cod_window_days: i64,
    /// Whether `FNB.RUL.14` is on.
    pub cod_rule: bool,
    /// Minutes after `ready` when the kitchen manager is told to chase the
    /// aggregator (`FNB.RUL.16`, N).
    pub pickup_chase_minutes: i64,
    /// Minutes after `ready` when the task to record the outcome opens
    /// (`FNB.RUL.16`, M).
    pub pickup_task_minutes: i64,
    /// Whether `FNB.RUL.16` is on.
    pub pickup_rule: bool,
}

impl Default for FnbSettings {
    fn default() -> Self {
        FnbSettings {
            customer_side_refund: false,
            refund_request_days: DEFAULT_REFUND_REQUEST_DAYS,
            cod_failures: DEFAULT_COD_FAILURES,
            cod_window_days: DEFAULT_COD_WINDOW_DAYS,
            cod_rule: true,
            pickup_chase_minutes: DEFAULT_PICKUP_CHASE_MINUTES,
            pickup_task_minutes: DEFAULT_PICKUP_TASK_MINUTES,
            pickup_rule: true,
        }
    }
}

/// The app's settings, or the defaults when none were provided.
pub fn settings(state: &AppState) -> Arc<FnbSettings> {
    state
        .provided::<FnbSettings>()
        .unwrap_or_else(|| Arc::new(FnbSettings::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_specs() {
        let settings = FnbSettings::default();
        assert!(!settings.customer_side_refund);
        assert_eq!(settings.refund_request_days, 7);
        assert_eq!((settings.cod_failures, settings.cod_window_days), (2, 90));
        assert!(settings.cod_rule);
        assert_eq!(
            (settings.pickup_chase_minutes, settings.pickup_task_minutes),
            (15, 45)
        );
        assert!(settings.pickup_rule);
    }
}
