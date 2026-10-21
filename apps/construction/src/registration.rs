//! A purchase order to a subcontractor without a valid registration
//! (`CON.ACL.13`, 02 Construction §2 "Contractor registration", §3.3, §7.1
//! *Purchase order* `issue`; BL-CON-16).
//!
//! Under Dubai Law 7/2025 a contractor checks that its subcontractors are
//! registered with Dubai Municipality. A purchase order to a **subcontractor**
//! (a supplier party with the role [`SUBCONTRACTOR_ROLE`]) delivering to a
//! **Dubai project** (the scope of the delivery location is a project whose
//! [`emirate`](Project::emirate) is `dubai`) needs approval when the
//! subcontractor has no valid document of kind [`REGISTRATION_KIND`] on file
//! on the order's date. The check runs on `issue` and again on each `revise`
//! (the date is the day it is issued or revised).
//!
//! The rule is wl-inventory's hook ([`PurchaseOrderRules`]): after the block
//! guard passed it asks for an approval request of the action
//! [`APPROVAL_ACTION`], one step decided by the project manager role; the
//! order stays a `draft` (a revision stays as it was) until the last step
//! approves, the requester never approves (segregation of duties), and a
//! rejection or a return leaves the order as it was with the reason. The order
//! is never refused outright, and orders to other suppliers, on other projects
//! or with a valid registration skip the check.
//!
//! Decisions where the spec is silent:
//!
//! - wl-documents does not block here: the validity is asked through the
//!   kernel's `document_valid` ([`DocumentGuard::is_valid`]) on the supplier
//!   party, and the role is asked through the kernel's party directory.
//! - The approver is any holder of the project manager role; wl-inventory
//!   resolves the order's scope as global today, so narrowing it to the
//!   project's own manager needs the order's scope to follow its location.
//! - A supplier with the role but whose project is not in Dubai, or a location
//!   without a project, skips the check.

use renox::prelude::*;
use wl_kernel::approvals::{PolicyDefault, Step};
use wl_kernel::documents;
use wl_kernel::inventory::{MatchFuture, OrderApproval, OrderFacts, PurchaseOrderRules};
use wl_kernel::parties;

use crate::permissions;
use crate::project::{self, Project};

/// The party role of a subcontractor (not a material supplier).
pub const SUBCONTRACTOR_ROLE: &str = "subcontractor";

/// The document kind of the Dubai Municipality contractor registration.
pub const REGISTRATION_KIND: &str = "dubai_municipality_contractor_registration";

/// The approval policy's action key of the check (`CON.ACL.13`).
pub const APPROVAL_ACTION: &str = "construction.po_subcontractor_registration";

/// The approval policy: every request of the action needs the project manager.
pub fn policy() -> PolicyDefault {
    PolicyDefault::new(APPROVAL_ACTION).step([permissions::PROJECT_MANAGER_ROLE])
}

/// The rule of the purchase order.
#[derive(Debug, Clone, Copy, Default)]
pub struct RegistrationRule;

impl RegistrationRule {
    /// Whether the order of `facts` needs the approval: a subcontractor
    /// without a valid registration on a Dubai project.
    async fn needed(state: &AppState, facts: &OrderFacts) -> Result<bool> {
        let Some(scope) = &facts.scope else {
            return Ok(false);
        };
        if scope.subject_type != project::SUBJECT_TYPE {
            return Ok(false);
        }
        let Some(project) = Project::find(&state.db, scope.subject_id).await? else {
            return Ok(false);
        };
        if !project.is_dubai() {
            return Ok(false);
        }
        let subcontractor = parties::directory(state)
            .has_role(&state.db, facts.supplier, SUBCONTRACTOR_ROLE, facts.on_date)
            .await?;
        if !subcontractor {
            return Ok(false);
        }
        let validity = documents::guard(state)
            .is_valid(
                &state.db,
                &facts.supplier.subject(),
                REGISTRATION_KIND,
                facts.on_date,
            )
            .await?;
        Ok(!validity.is_valid())
    }
}

impl PurchaseOrderRules for RegistrationRule {
    fn approval_for<'a>(
        &'a self,
        state: &'a AppState,
        facts: &'a OrderFacts,
    ) -> MatchFuture<'a, Option<OrderApproval>> {
        Box::pin(async move {
            if !Self::needed(state, facts).await? {
                return Ok(None);
            }
            Ok(Some(OrderApproval {
                action: APPROVAL_ACTION.to_owned(),
                steps: vec![Step::one_of([permissions::PROJECT_MANAGER_ROLE])],
            }))
        })
    }
}

/// Registers the policy.
pub(crate) fn register() {
    policy().register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_approval_goes_to_the_project_manager() {
        assert_eq!(policy().action(), APPROVAL_ACTION);
    }
}
