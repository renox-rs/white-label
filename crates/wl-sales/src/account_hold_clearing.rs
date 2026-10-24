//! The clearing of one source of a customer account's hold (wl-sales §4.2
//! *Account hold*, `SAL.ACL.03`, `PLT.APR.01`; BL-SAL-12): *Clear account
//! hold*. It is an action on the customer account, not a machine of the
//! account; the request that carries it is a small record so the kernel's
//! approvals have a subject to run `request_approved` on.
//!
//! ```text
//! [*] draft --request--> pending_approval --request_approved--> cleared
//!                              |--back_to_draft--> draft --reject--> rejected
//! ```
//!
//! Decisions:
//! - A holder of `credit.release` asks, with a reason (*cheque replaced*,
//!   *balance paid* or *other*, which needs a note); another holder approves
//!   (segregation of duties is on, so not even as admin).
//! - A request is about one source. Approving it clears that source on every
//!   account that got it: the whole credit group when it was held as a group.
//!   The flag stays while another source is open.
//! - Clearing does not release held orders; each goes through `release`.
//! - A request that ends without a yes (rejected, returned, withdrawn,
//!   expired) is `rejected`; the requester asks again if the hold should go.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, Require, Required};
use wl_kernel::machines::SubjectMachine;

use crate::customer_account::AccountHoldSource;
use crate::policy;
use crate::texts::english;

/// The subject type of a clearing request in the kernel registry.
pub const CLEARING_SUBJECT_TYPE: &str = "sales.account_hold_clearing";

/// `draft → pending_approval`: asks the kernel for another releaser.
pub const REQUEST: &str = "request";
/// `pending_approval → cleared`: the kernel runs it on the last yes.
pub const REQUEST_APPROVED: &str = "request_approved";
/// `pending_approval → draft`: the kernel runs it when the request ends
/// without a yes.
pub const BACK_TO_DRAFT: &str = "back_to_draft";
/// `draft → rejected`: the request ended without a yes.
pub const REJECT: &str = "reject";

/// *Cheque replaced*.
pub const KIND_CHEQUE_REPLACED: &str = "cheque_replaced";
/// *Balance paid*.
pub const KIND_BALANCE_PAID: &str = "balance_paid";
/// *Other*; needs a note.
pub const KIND_OTHER: &str = "other";
/// The reasons a clearing can give.
pub const KINDS: &[&str] = &[KIND_CHEQUE_REPLACED, KIND_BALANCE_PAID, KIND_OTHER];

/// Where a clearing request is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClearingStatus {
    /// Being requested, or back after the approval ended without a yes.
    #[default]
    Draft,
    /// Waiting for another credit releaser.
    PendingApproval,
    /// The source is cleared.
    Cleared,
    /// The approval ended without a yes.
    Rejected,
}

/// A request to clear one source of an account's hold.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_account_hold_clearings",
    state_machine,
    index(account_id),
    index(status)
)]
pub struct AccountHoldClearing {
    /// The primary key.
    pub id: i64,
    /// The account it was asked on.
    #[model(references = "wl_sales_customer_accounts")]
    pub account_id: i64,
    /// The source it clears.
    #[model(references = "wl_sales_account_hold_sources")]
    pub hold_source_id: i64,
    /// `cheque_replaced`, `balance_paid` or `other`.
    pub reason_kind: String,
    /// What the requester wrote.
    pub note: Option<String>,
    /// The user who asked.
    pub requested_by: i64,
    /// The kernel's approval request.
    pub approval_request_id: Option<i64>,
    /// The releaser who gave the last yes.
    pub approved_by: Option<i64>,
    /// Why it ended without a yes.
    pub outcome_reason: Option<String>,
    /// Where it is; changed only by the machine.
    pub status: ClearingStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AccountHoldClearing {
    /// The request as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(CLEARING_SUBJECT_TYPE, self.id)
    }
}

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// Sets non-status columns of clearing `id` and stamps `updated_at`.
async fn set(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    AccountHoldClearing::query()
        .where_(AccountHoldClearing::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Clears the source of `clearing` on every account that got it.
async fn clear_sources(
    tx: &mut Transaction,
    clearing: &AccountHoldClearing,
    approver: Option<i64>,
) -> Result {
    let Some(source) = AccountHoldSource::find(&mut *tx, clearing.hold_source_id).await? else {
        return Err(refused("wl_sales.credit.problem.no_source"));
    };
    let stamp = Some(now());
    AccountHoldSource::query()
        .where_(AccountHoldSource::SOURCE_TYPE.eq(source.source_type.clone()))
        .where_(AccountHoldSource::SOURCE_ID.eq(source.source_id))
        .where_(AccountHoldSource::CLEARED_AT.is_null())
        .update(
            &mut *tx,
            &[
                ("cleared_at", &stamp),
                ("cleared_by", &approver),
                ("clearing_id", &Some(clearing.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    set(tx, clearing.id, &[("approved_by", &approver)]).await
}

impl StateMachine for AccountHoldClearing {
    type State = ClearingStatus;
    const COLUMN: renox::db::Col<AccountHoldClearing, ClearingStatus> = AccountHoldClearing::STATUS;

    fn machine() -> Machine<Self> {
        use ClearingStatus::{Cleared, Draft, PendingApproval, Rejected};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(PendingApproval)
            .tone("warning")
            .state(Cleared)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("danger")
            .terminal()
            .transition(request())
            .transition(
                Transition::new(REQUEST_APPROVED)
                    .label("Approve")
                    .from([PendingApproval])
                    .to(Cleared)
                    .system()
                    .effect(|c: &AccountHoldClearing, ctx, tx| {
                        let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
                        let c = c.clone();
                        Box::pin(async move { clear_sources(tx, &c, approver).await })
                    }),
            )
            .transition(
                Transition::new(BACK_TO_DRAFT)
                    .label("Approval ended")
                    .from([PendingApproval])
                    .to(Draft)
                    .system()
                    .effect(|c: &AccountHoldClearing, ctx, tx| {
                        let reason = ctx.reason().map(|r| r.trim().to_owned());
                        let id = c.id;
                        Box::pin(async move { set(tx, id, &[("outcome_reason", &reason)]).await })
                    }),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .from([Draft])
                    .to(Rejected)
                    .system(),
            )
    }
}

/// `request`: an approval request for another credit releaser.
fn request() -> Transition<AccountHoldClearing> {
    Transition::new(REQUEST)
        .label("Request approval")
        .from([ClearingStatus::Draft])
        .to(ClearingStatus::PendingApproval)
        .system()
        .effect(|c: &AccountHoldClearing, _ctx, tx| {
            let c = c.clone();
            Box::pin(async move {
                let require = Require::new(
                    policy::CLEAR_HOLD_ACTION,
                    c.subject(),
                    REQUEST,
                    c.requested_by,
                    "draft",
                )
                .payload(json!({
                    "account_id": c.account_id,
                    "hold_source_id": c.hold_source_id,
                    "reason_kind": c.reason_kind,
                }));
                match approvals::require_in(&mut *tx, &require).await? {
                    Required::Pending(request) => {
                        set(tx, c.id, &[("approval_request_id", &Some(request.id))]).await
                    }
                    // Without a policy the hold would be cleared unchecked;
                    // the transaction rolls back instead.
                    Required::Allowed => Err(refused("wl_sales.credit.problem.no_policy")),
                }
            })
        })
}

/// The machine's registry entry: the waiting state `pending_approval`, the
/// action `sales.account_hold_clear` and the global scope.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<AccountHoldClearing>(CLEARING_SUBJECT_TYPE)
        .waiting(ClearingStatus::PendingApproval)
        .action(REQUEST, policy::CLEAR_HOLD_ACTION)
        .scope(|_c: &AccountHoldClearing| renox::auth::permissions::Scope::global())
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_is_declared_and_system() {
        let machine = AccountHoldClearing::machine();
        let table = [
            (REQUEST, "draft", "pending_approval"),
            (REQUEST_APPROVED, "pending_approval", "cleared"),
            (BACK_TO_DRAFT, "pending_approval", "draft"),
            (REJECT, "draft", "rejected"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in table {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), [from], "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
    }

    #[test]
    fn the_registry_entry_waits_in_pending_approval_under_its_action() {
        let m = subject_machine();
        assert_eq!(m.waiting_state(), Some("pending_approval"));
        assert_eq!(m.action_key(REQUEST), "sales.account_hold_clear");
    }

    #[test]
    fn the_reasons_are_the_three_of_the_spec() {
        assert_eq!(KINDS, ["cheque_replaced", "balance_paid", "other"]);
    }
}
