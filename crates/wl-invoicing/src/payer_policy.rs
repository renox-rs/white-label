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

use wl_kernel::approvals::PolicyDefault;

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

#[cfg(test)]
mod tests {
    use super::*;

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
