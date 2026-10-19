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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_is_one_step_by_the_holders_of_the_permission() {
        let expected = PolicyDefault::new("campaigns.cost_over_budget")
            .step_permission("campaigns.costs.manage");
        assert_eq!(policy(), expected);
    }
}
