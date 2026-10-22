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

/// The default policies of the changes to a placed order that need a manager
/// (`SAL.ACL.02`, `SAL.ACL.05`): an amendment, a cancelled line and the
/// cancelled remaining lines each wait for one step by a holder of
/// `sales_orders.approve`, never the requester.
pub fn order_change_policies() -> Vec<PolicyDefault> {
    [
        crate::amend::ACTION,
        crate::cancel_lines::ACTION_LINE,
        crate::cancel_lines::ACTION_REMAINING,
    ]
    .into_iter()
    .map(|action| PolicyDefault::new(action).step_permission(permissions::SALES_ORDERS_APPROVE))
    .collect()
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
    fn a_change_after_fulfilment_started_is_decided_by_a_holder_of_the_approve_permission() {
        let policies = order_change_policies();
        assert_eq!(policies.len(), 3);
        assert_eq!(
            policies[0],
            PolicyDefault::new("sales.order_amend").step_permission("sales_orders.approve")
        );
    }

    #[test]
    fn the_policy_is_one_step_by_the_holders_of_the_permission() {
        let expected = PolicyDefault::new("campaigns.cost_over_budget")
            .step_permission("campaigns.costs.manage");
        assert_eq!(policy(), expected);
    }
}
