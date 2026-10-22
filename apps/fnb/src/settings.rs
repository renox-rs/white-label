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

/// An unmatched amount on an imported payout statement above which the
/// accountant gets a reconciliation task (`FNB.RUL.10`, X: AED 100), in fils.
pub const DEFAULT_PAYOUT_TASK_FILS: i64 = 10_000;

/// A wastage above this value needs the kitchen manager's approval, never the
/// chef who recorded it (`STK.ACL.02`, AED 200), in fils.
pub const DEFAULT_WASTAGE_APPROVAL_FILS: i64 = 20_000;

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
    /// The unmatched amount of an imported payout statement above which the
    /// accountant gets a task, in fils (`FNB.RUL.10`, X).
    pub payout_task_fils: i64,
    /// Whether `FNB.RUL.10` is on.
    pub payout_rule: bool,
    /// The wastage value above which a discard of a batch needs a second
    /// holder of `stock.approve` (`STK.ACL.02`), in fils.
    pub wastage_approval_fils: i64,
    /// The channels whose sales have no known customer besides the
    /// aggregators' (the counter): a recall counts them per outlet and day.
    pub walk_in_channels: Vec<String>,
    /// Whether a completed order consumes its recipe from the outlet's stock
    /// (theoretical consumption: it keeps the stock balances right and lets a
    /// recall trace list the order as *possible*). Off in the library; the
    /// 06 client turns it on.
    pub recipe_consumption: bool,
    /// Whether the owner is told that the authority is to be notified of a
    /// recall (open item AU-20, authority notification). Off until the lawyer
    /// answers: the recall report is exported and sent by hand.
    pub authority_notice_task: bool,
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
            payout_task_fils: DEFAULT_PAYOUT_TASK_FILS,
            payout_rule: true,
            wastage_approval_fils: DEFAULT_WASTAGE_APPROVAL_FILS,
            walk_in_channels: vec!["pos".to_owned(), "counter".to_owned()],
            recipe_consumption: false,
            authority_notice_task: false,
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
        assert_eq!(settings.payout_task_fils, 10_000);
        assert!(settings.payout_rule);
    }
}
