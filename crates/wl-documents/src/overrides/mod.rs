//! Block overrides (wl-documents §3 *Block override*, §4.3, #50).
//!
//! A [`BlockOverride`] is the one-use permission, approved by a second
//! person, to run a guarded transition although a `with_approval` kind
//! blocks it. Its status moves only through its machine ([`machine`]); the
//! approval is the `documents.block_override` default policy ([`policy`]).

pub(crate) mod consume;
pub mod machine;
pub mod model;
mod request;

use renox::prelude::*;
use wl_kernel::approvals::{ApprovalDecided, ApprovalEnd, PolicyDefault};
use wl_kernel::machines::SubjectMachine;

pub use model::{BlockOverride, BlockOverrideStatus};

use crate::permissions;

/// The override's subject type in the kernel registry and approval requests.
pub const SUBJECT_TYPE: &str = "documents.block_override";

/// The approval policies' action key of an override request.
pub const ACTION: &str = "documents.block_override";

/// Hours an approved override stays valid.
pub const OVERRIDE_VALIDITY_HOURS: i64 = 72;

/// Hours an override request may wait before it can expire.
pub const OVERRIDE_REQUEST_EXPIRY_HOURS: i64 = 72;

/// The default policy: always one step by a holder of
/// `documents.block.override` in the override's scope, never the requester
/// (segregation of duties stays on).
pub fn policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .step_permission(permissions::BLOCK_OVERRIDE)
        .expiry_hours(Some(OVERRIDE_REQUEST_EXPIRY_HOURS))
}

/// The machine's registry entry: no waiting state, the stored scope.
fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<BlockOverride>(SUBJECT_TYPE)
        .scope(|o: &BlockOverride| o.scope())
        .build()
}

/// Whether an approval that ended this way leaves its override refused: a
/// reject, a withdraw, an expiry, or an approval whose grant failed. A
/// request sent back for changes may still be resubmitted.
pub fn refuses(end: ApprovalEnd) -> bool {
    matches!(
        end,
        ApprovalEnd::Rejected | ApprovalEnd::Withdrawn | ApprovalEnd::Expired | ApprovalEnd::Failed
    )
}

/// Registers the machine with the kernel, the default policy and the
/// `ApprovalDecided` listener.
pub(crate) fn register(app: &mut Registry) {
    subject_machine().register();
    policy().register();
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        if event.subject.subject_type != SUBJECT_TYPE || !refuses(event.decision) {
            return Ok(());
        }
        machine::refuse_override(&state, event.subject.subject_id, event.decision).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_is_one_step_by_the_permission_holders_with_sod() {
        let expected = PolicyDefault::new("documents.block_override")
            .step_permission("documents.block.override")
            .expiry_hours(Some(72));
        assert_eq!(policy(), expected);
    }

    #[test]
    fn only_an_end_without_a_grant_refuses_the_override() {
        for end in [
            ApprovalEnd::Rejected,
            ApprovalEnd::Withdrawn,
            ApprovalEnd::Expired,
            ApprovalEnd::Failed,
        ] {
            assert!(refuses(end), "{end:?}");
        }
        assert!(!refuses(ApprovalEnd::Approved));
        assert!(!refuses(ApprovalEnd::Returned));
    }

    #[test]
    fn the_machine_has_no_waiting_state_and_the_stored_scope() {
        let machine = subject_machine();
        assert_eq!(machine.subject_type(), "documents.block_override");
        assert_eq!(machine.waiting_state(), None);
        assert!(machine.has_scope());
        assert!(machine.transition(machine::GRANT_APPROVED).is_some());
    }
}
