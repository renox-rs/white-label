//! The approval policies of the payer bill (wl-invoicing §6, ACCESS-CONTROL
//! §5): the default policies the kernel stores at `wl:install`, which the
//! client changes on the stored policy, never in code.
//!
//! - **`FIN.ACL.16`** ([`money_policy`]): moving a payer's overpayment to the
//!   payer credit balance, or recording a payer's recoupment, needs the
//!   approval of a second person: any amount above 0 (the input's `amount`
//!   is the overpayment or the recoupment, and 0 for a reversal from
//!   wl-payments, which is exempt), decided by an `admin` or a holder of
//!   `payer_bills.reconcile`, never the requester (SoD), within 168 hours.
//! - **`FIN.ACL.14`** ([`write_off_policy`]): writing off a payer bill's open
//!   amount above the payer agreement's limit needs the approval of a second
//!   person. The limit is the agreement's (05: AED 1,000), so the input's
//!   `amount` is the open amount when it is above that limit and 0
//!   otherwise, and the policy is any amount above 0. Decided by an `admin`
//!   or a holder of `payer_bills.write_off`, never the requester.

use wl_kernel::approvals::PolicyDefault;

use crate::config::InvoicingConfig;
use crate::payer_closing::WRITE_OFF_ACTION;
use crate::payer_remittance::ACTION;
use crate::permissions;

/// Hours a request may wait before it can expire.
pub const EXPIRY_HOURS: i64 = 168;

/// The role besides the holders of `payer_bills.reconcile` that approves.
pub const APPROVER_ROLE: &str = "admin";

/// The `FIN.ACL.16` default policy.
pub fn money_policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .amount_above(0)
        .step_or_permission([APPROVER_ROLE], permissions::PAYER_BILLS_RECONCILE)
        .expiry_hours(Some(EXPIRY_HOURS))
}

/// The `FIN.ACL.14` default policy.
///
/// With `payer_write_off_approvers` set on the module (an app that keeps the
/// decision to its owner) the one step is decided by those roles alone;
/// otherwise by an `admin` or a holder of `payer_bills.write_off`.
pub fn write_off_policy(config: &InvoicingConfig) -> PolicyDefault {
    let policy = PolicyDefault::new(WRITE_OFF_ACTION).amount_above(0);
    let policy = if config.payer_write_off_approvers.is_empty() {
        policy.step_or_permission([APPROVER_ROLE], permissions::PAYER_BILLS_WRITE_OFF)
    } else {
        policy.step(config.payer_write_off_approvers.iter().cloned())
    };
    policy.expiry_hours(Some(EXPIRY_HOURS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_off_above_the_limit_needs_a_second_person() {
        let expected = PolicyDefault::new("invoicing.payer_write_off")
            .amount_above(0)
            .step_or_permission(["admin"], "payer_bills.write_off")
            .expiry_hours(Some(168));
        assert_eq!(write_off_policy(&InvoicingConfig::default()), expected);
    }

    #[test]
    fn an_app_may_keep_the_decision_to_its_own_roles() {
        let config = InvoicingConfig {
            payer_write_off_approvers: vec!["owner".to_owned(), "admin".to_owned()],
            ..InvoicingConfig::default()
        };
        let expected = PolicyDefault::new("invoicing.payer_write_off")
            .amount_above(0)
            .step(["owner", "admin"])
            .expiry_hours(Some(168));
        assert_eq!(write_off_policy(&config), expected);
    }

    #[test]
    fn every_overpayment_and_recoupment_needs_a_second_person() {
        let expected = PolicyDefault::new("invoicing.payer_money")
            .amount_above(0)
            .step_or_permission(["admin"], "payer_bills.reconcile")
            .expiry_hours(Some(168));
        assert_eq!(money_policy(), expected);
        assert_eq!(money_policy().action(), ACTION);
    }
}
