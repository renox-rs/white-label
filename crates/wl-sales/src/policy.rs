//! The default approval policy of a cost entry over the budget (#138 A13):
//! always one step by a holder of `campaigns.costs.manage`, never the
//! requester (segregation of duties on, so not even as admin), with the
//! kernel's default expiry.

use wl_kernel::approvals::PolicyDefault;

use crate::permissions;

/// The approval policies' action key of a cost entry over the budget.
pub const ACTION: &str = "campaigns.cost_over_budget";

/// The default policy.
pub fn policy() -> PolicyDefault {
    PolicyDefault::new(ACTION).step_permission(permissions::CAMPAIGNS_COSTS_MANAGE)
}

/// The approval policies' action key of a points adjustment above the limit,
/// or of closing an account that still has points (`SAL.ACL.07`).
pub const ADJUSTMENT_ACTION: &str = "loyalty.adjustment_over_limit";

/// The default policy of an adjustment: one step by a holder of
/// `loyalty.manage`, never the requester.
pub fn adjustment_policy() -> PolicyDefault {
    PolicyDefault::new(ADJUSTMENT_ACTION).step_permission(permissions::LOYALTY_MANAGE)
}

/// The approval policies' action key of releasing an order from a credit
/// hold (`SAL.ACL.03`, `PLT.APR.01`).
pub const RELEASE_ACTION: &str = "sales.order_release";

/// The default policy of a release: one step by a holder of `credit.release`
/// other than the requester; the order's creator is excluded too (see
/// [`crate::sales_orders`]).
pub fn release_policy() -> PolicyDefault {
    PolicyDefault::new(RELEASE_ACTION).step_permission(permissions::CREDIT_RELEASE)
}

/// The approval policies' action key of clearing a customer account's hold
/// (`SAL.ACL.03`, `PLT.APR.01`).
pub const CLEAR_HOLD_ACTION: &str = "sales.account_hold_clear";

/// The default policy of *Clear account hold*: one step by a holder of
/// `credit.release`, never the requester.
pub fn clear_hold_policy() -> PolicyDefault {
    PolicyDefault::new(CLEAR_HOLD_ACTION).step_permission(permissions::CREDIT_RELEASE)
}

/// The default policies of the changes to a placed order that need a manager
/// (`SAL.ACL.01`, `SAL.ACL.02`, `SAL.ACL.05`): an amendment, a cancelled line,
/// the cancelled remaining lines and a cancelled order each wait for one step by a holder of
/// `sales_orders.approve`, never the requester.
pub fn order_change_policies() -> Vec<PolicyDefault> {
    [
        crate::amend::ACTION,
        crate::cancel_lines::ACTION_LINE,
        crate::cancel_lines::ACTION_REMAINING,
        crate::cancel_lines::ACTION_ORDER,
    ]
    .into_iter()
    .map(|action| PolicyDefault::new(action).step_permission(permissions::SALES_ORDERS_APPROVE))
    .collect()
}

/// The default policy of a refund or credit after completion with nothing
/// returned above the limit (`SAL.ACL.06`): one step by a holder of
/// `returns.approve`, never the recorder.
pub fn refund_policy() -> PolicyDefault {
    PolicyDefault::new(crate::sales_returns::ACTION_REFUND)
        .step_permission(permissions::RETURNS_APPROVE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_adjustment_over_the_limit_is_decided_by_a_loyalty_manager() {
        let expected =
            PolicyDefault::new("loyalty.adjustment_over_limit").step_permission("loyalty.manage");
        assert_eq!(adjustment_policy(), expected);
    }

    #[test]
    fn a_release_is_decided_by_another_credit_releaser() {
        let expected = PolicyDefault::new("sales.order_release").step_permission("credit.release");
        assert_eq!(release_policy(), expected);
    }

    #[test]
    fn clearing_an_account_hold_is_decided_by_another_credit_releaser() {
        let expected =
            PolicyDefault::new("sales.account_hold_clear").step_permission("credit.release");
        assert_eq!(clear_hold_policy(), expected);
    }

    #[test]
    fn a_change_after_fulfilment_started_is_decided_by_a_holder_of_the_approve_permission() {
        let policies = order_change_policies();
        assert_eq!(policies.len(), 4);
        assert_eq!(
            policies[0],
            PolicyDefault::new("sales.order_amend").step_permission("sales_orders.approve")
        );
    }

    #[test]
    fn a_refund_after_completion_is_decided_by_a_holder_of_the_returns_approve_permission() {
        assert_eq!(
            refund_policy(),
            PolicyDefault::new("sales.return_refund").step_permission("returns.approve")
        );
    }

    #[test]
    fn the_policy_is_one_step_by_the_holders_of_the_permission() {
        let expected = PolicyDefault::new("campaigns.cost_over_budget")
            .step_permission("campaigns.costs.manage");
        assert_eq!(policy(), expected);
    }
}
