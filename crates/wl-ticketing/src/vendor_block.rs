//! A blocked vendor's service contracts (`PTY.ACL.03`, wl-parties §5.6,
//! wl-ticketing §4.1 `assign`, issue #153): the effect of blocking a vendor
//! party on the agreements it holds here is a **review**.
//!
//! Nothing ends or is cancelled by itself. While the vendor is blocked:
//!
//! - `assign` to it is refused: assigning work to a vendor counts as starting
//!   new business with it, so [`crate::assign`] asks the kernel's
//!   `BlockGuard::check_in(vendor, Start)` ([`refusal`]). A reassignment
//!   *away* from it checks only the new assignee; a visit already
//!   `in_progress` continues and can be resolved;
//! - a new planned visit of its contracts is created **unassigned** (see
//!   [`Tickets::open`](crate::Tickets::open)), and `assign` does not propose
//!   the vendor ([`ServiceContracts::proposed_vendor`](crate::ServiceContracts::proposed_vendor));
//! - the facility manager (the decider, `facility_manager` unless the
//!   vertical's `BlockEffectOverride` says otherwise) gets the review task of
//!   `wl-parties` with the vendor's `active` and `expiring` contracts, each
//!   naming its open planned visits; on the contract page the decider
//!   reassigns the visits (reason of their choice) or terminates the contract.
//!   An unblock lets the vendor be assigned again; the planned visits left
//!   unassigned show on the contracts for the coordinator.
//!
//! The refusals and labels here are the neutral "This party is on hold": the
//! block reason is never shown to the coordinator or the vendor (the guard is
//! asked as the system).

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::block_effects::{
    AgreementFuture, AgreementKind, AgreementRef, AgreementSource, BlockEffect, Decision,
};
use wl_kernel::block_guard::{self, ActionKind, GuardOutcome};
use wl_kernel::tickets::SERVICE_CONTRACT_SUBJECT_TYPE;
use wl_kernel::{PartyId, Subject};

use crate::contract::{ServiceContract, ServiceContracts, is_open_visit};
use crate::texts::english;

/// The agreement kind of wl-parties §5.6.
pub const KIND: &str = "service_contract";

/// The role that decides, unless the vertical says otherwise.
pub const DECIDER: &str = "facility_manager";

/// Asks the kernel's block guard whether new work may start with `party`: the
/// neutral refusal text when it is blocked, `None` when it may. Asked as the
/// system, so the block reason is never in it.
///
/// # Errors
///
/// The database's errors.
pub async fn refusal(state: &AppState, party: PartyId) -> Result<Option<String>> {
    let mut tx = state.db.begin().await?;
    let outcome = block_guard::guard(state)
        .check_in(&mut tx, None, party, ActionKind::Start)
        .await?;
    // Only a question: nothing was written.
    drop(tx);
    Ok(match outcome {
        GuardOutcome::Allowed => None,
        GuardOutcome::Blocked(refusal) => Some(refusal.message_en()),
    })
}

/// Whether `party` is blocked for new work.
///
/// # Errors
///
/// The database's errors.
pub async fn is_blocked(state: &AppState, party: PartyId) -> Result<bool> {
    Ok(refusal(state, party).await?.is_some())
}

/// The service contracts of a vendor, for the review task and its decision.
struct ServiceContractAgreements;

impl AgreementSource for ServiceContractAgreements {
    fn open_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> AgreementFuture<'a, Vec<AgreementRef>> {
        Box::pin(async move {
            let mut refs = Vec::new();
            for contract in ServiceContract::in_force_of(&state.db, party).await? {
                let visits = contract.visits(&state.db).await?;
                let open = visits.iter().filter(|v| is_open_visit(v)).count();
                let scope = wl_kernel::scopes::scope_of(&state.db, &contract.subject()).await?;
                refs.push(AgreementRef {
                    subject: contract.subject(),
                    label: format!("{} - {open} planned visits", contract.number),
                    scope: (!scope.is_global()).then_some(scope),
                });
            }
            Ok(refs)
        })
    }

    fn decide<'a>(
        &'a self,
        state: &'a AppState,
        user: i64,
        subject: &'a Subject,
        decision: Decision,
    ) -> AgreementFuture<'a, ()> {
        Box::pin(async move {
            if subject.subject_type != SERVICE_CONTRACT_SUBJECT_TYPE {
                return Err(Error::NotFound);
            }
            match decision {
                // Nothing to pause: no new visit is assigned while the vendor
                // is blocked. Continuing leaves the contract as it is.
                Decision::Pause | Decision::Continue => Ok(()),
                Decision::End => {
                    let contract = ServiceContract::find(&state.db, subject.subject_id)
                        .await?
                        .ok_or(Error::NotFound)?;
                    let reason = english("wl_ticketing.vendor_block.terminate_reason");
                    ServiceContracts::terminate_by_decision(state, &contract, user, reason)
                        .await
                        .map_err(TransitionError::into_error)
                }
            }
        })
    }
}

/// Registers the agreement kind: a review, decided by the facility manager.
pub(crate) fn register() {
    AgreementKind::new(KIND)
        .owner("wl-ticketing")
        .effect(BlockEffect::Review)
        .decider(DECIDER)
        .source(ServiceContractAgreements)
        .register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_is_a_review_decided_by_the_facility_manager() {
        register();
        let rule =
            wl_kernel::block_effects::effect_for(KIND, &renox::auth::permissions::Scope::global());
        assert_eq!(rule.effect, BlockEffect::Review);
        assert_eq!(rule.decider_role.as_deref(), Some("facility_manager"));
        assert!(wl_kernel::block_effects::source_for(KIND).is_some());
    }
}
