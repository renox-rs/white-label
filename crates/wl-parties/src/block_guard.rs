//! The module's [`BlockGuard`] (wl-parties §6): refuses new agreements with,
//! and money out to, a blocked party. [`crate::WlParties`] provides it with
//! [`wl_kernel::block_guard::provide_block_guard`].
//!
//! Only a `blocked` party is refused; an active, archived, merged or unknown
//! party is allowed, and money in never is. The refusal is neutral ("on
//! hold") for the system and for users without `parties.block_reason.view`.
//! A holder sees one line per open Block record: its reason label and the
//! link to the record on the party page.
//!
//! The exception approver comes from the module setup: `finance_manager`,
//! `compliance` when screening is on, or the role of
//! [`crate::WlParties::exception_approver`].

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::approvals::{PolicyDefault, Step};
use wl_kernel::block_guard::{
    ActionKind, BLOCK_EXCEPTION_ACTION, BlockFuture, BlockGuard, BlockReasonLine, BlockRefusal,
    ExceptionStep, GuardOutcome,
};
use wl_kernel::{PartyId, PartyStatus};

use crate::block_record::BlockRecord;
use crate::party::Party;
use crate::texts;
use crate::visibility;

/// The role that approves an exception when screening is off.
pub const DEFAULT_APPROVER: &str = "finance_manager";

/// The role that approves an exception when screening is on.
pub const SCREENING_APPROVER: &str = "compliance";

/// Reads `wl_parties_parties` and the open Block records for the kernel
/// contract.
#[derive(Debug, Clone)]
pub struct PartiesBlockGuard {
    approver: String,
}

impl PartiesBlockGuard {
    /// A guard whose exceptions are approved by `approver`.
    pub fn new(approver: impl Into<String>) -> Self {
        PartiesBlockGuard {
            approver: approver.into(),
        }
    }

    /// The role that approves an exception.
    pub fn approver(&self) -> &str {
        &self.approver
    }

    /// The default approval policy of the action `parties.block_exception`:
    /// every run needs one step by the approver role, with segregation of
    /// duties on and the kernel's default expiry.
    ///
    /// The step also needs the permission
    /// [`crate::permissions::BLOCK_EXCEPTION`] in the subject's scope.
    pub fn policy(&self) -> PolicyDefault {
        PolicyDefault::new(BLOCK_EXCEPTION_ACTION).with_step(
            Step::one_of([self.approver.clone()]).requiring(crate::permissions::BLOCK_EXCEPTION),
        )
    }
}

/// The label of the role `role` in English: the text
/// `wl_parties.approver.<role>`, else the role key with spaces.
fn approver_label(role: &str) -> String {
    let key = format!("wl_parties.approver.{role}");
    let label = texts::english(&key);
    if label == key {
        role.replace('_', " ")
    } else {
        label
    }
}

impl BlockGuard for PartiesBlockGuard {
    fn check_in<'a>(
        &'a self,
        tx: &'a mut Transaction,
        viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome> {
        Box::pin(async move {
            if !kind.is_guarded() {
                return Ok(GuardOutcome::Allowed);
            }
            let found = Party::query()
                .where_(Party::ID.eq(party.0))
                .first(&mut *tx)
                .await?;
            match found {
                Some(found) if found.status == PartyStatus::Blocked => {}
                _ => return Ok(GuardOutcome::Allowed),
            }
            let approver = approver_label(&self.approver);
            let reasons = match viewer {
                Some(user) if visibility::sees_restricted(user) => {
                    let records = BlockRecord::open_for(&mut *tx, party.0).await?;
                    records
                        .iter()
                        .map(|record| BlockReasonLine {
                            label: visibility::reason_label_en(&record.reason_code),
                            link: format!("/parties/{}/block-records/{}", party.0, record.id),
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            Ok(GuardOutcome::Blocked(BlockRefusal::detailed(
                party, kind, reasons, approver,
            )))
        })
    }

    fn exception_step(&self) -> ExceptionStep {
        ExceptionStep::new(self.approver.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_has_one_step_by_the_approver() {
        let expected = PolicyDefault::new("parties.block_exception")
            .with_step(Step::one_of(["compliance"]).requiring("parties.block_exception"));
        assert_eq!(PartiesBlockGuard::new("compliance").policy(), expected);
    }

    #[test]
    fn the_step_names_the_approver_and_the_permission() {
        let step = PartiesBlockGuard::new("compliance").exception_step();
        assert_eq!(step.role, "compliance");
        assert_eq!(step.permission, "parties.block_exception");
    }

    #[test]
    fn a_role_has_a_label_or_its_key_with_spaces() {
        assert_eq!(approver_label("finance_manager"), "Finance manager");
        assert_eq!(approver_label("compliance"), "Compliance");
        assert_eq!(approver_label("site_head"), "site head");
    }
}
