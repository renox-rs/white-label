//! The CostEntry state machine (wl-sales §4, #138 A11-A13).
//!
//! ```text
//! [*] draft --submit--------------------------> approved
//!       |---submit_over_budget--> pending_approval --submit_over_budget_approved--> approved
//!       |                              |--back_to_draft--> draft --reject--> rejected
//! ```
//!
//! Every transition is `.system()`: [`crate::costs`] runs exactly one of
//! `submit` and `submit_over_budget` after reading the approved total under
//! the campaign lock, and gives the machine that [`BudgetCheck`] as input so
//! the guards agree. `submit_over_budget` asks the kernel approval engine
//! (`approvals::require_in`, action `campaigns.cost_over_budget`, requester =
//! `requested_by`); the kernel runs `submit_over_budget_approved` on the last
//! yes and `back_to_draft` when the request ends without one. A listener on
//! `ApprovalDecided` then runs `reject` after a rejection.
//!
//! Part 6 adds the bill links and voids:
//!
//! ```text
//! draft --await_bill--> awaiting_bill --bill_approved--> draft (then submit)
//! draft | awaiting_bill --void_with_bill--> voided
//! approved | pending_approval --void / void_approved--> voided   (kernel Voidable)
//! ```

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Machine, StateMachine, Transition, Transitioned};
use wl_kernel::approvals::{self, ApprovalDecided, ApprovalEnd, Require, Required};
use wl_kernel::machines::SubjectMachine;
use wl_kernel::sales::CAMPAIGN_COST_ENTRY_SUBJECT_TYPE;
use wl_kernel::voids;

use crate::budget::{self, BudgetCheck};
use crate::cost_entry::{CostEntry, CostStatus};
use crate::{notices, policy};

/// The subject type of a cost entry in the kernel registry.
pub const SUBJECT_TYPE: &str = CAMPAIGN_COST_ENTRY_SUBJECT_TYPE;

/// `draft → approved`: within the budget.
pub const SUBMIT: &str = "submit";
/// `draft → pending_approval`: over the budget, an approval request.
pub const SUBMIT_OVER_BUDGET: &str = "submit_over_budget";
/// `pending_approval → approved`: the kernel runs it on the last yes.
pub const SUBMIT_OVER_BUDGET_APPROVED: &str = "submit_over_budget_approved";
/// `pending_approval → draft`: the kernel runs it when the request ends
/// without a yes (reject, return, withdraw, expiry, failed final check).
pub const BACK_TO_DRAFT: &str = "back_to_draft";
/// `draft → rejected`: an approver rejected the request.
pub const REJECT: &str = "reject";

/// `draft → awaiting_bill`: linked to a supplier bill that is not approved.
pub const AWAIT_BILL: &str = "await_bill";
/// `awaiting_bill → draft`: the bill was approved; the entry is submitted next.
pub const BILL_APPROVED: &str = "bill_approved";
/// `draft | awaiting_bill → voided`: the supplier bill was voided.
pub const VOID_WITH_BILL: &str = "void_with_bill";

/// The stored reason of a rejection with no comment.
const REJECTED: &str = "rejected";

/// Sets non-status columns of entry `id` outside the machine's guard and
/// stamps `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let now = renox::db::now();
    all.push(("updated_at", &now));
    CostEntry::query()
        .where_(CostEntry::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for CostEntry {
    type State = CostStatus;
    const COLUMN: renox::db::Col<CostEntry, CostStatus> = CostEntry::STATUS;

    fn machine() -> Machine<Self> {
        use CostStatus::{Approved, AwaitingBill, Draft, PendingApproval, Rejected, Voided};
        let mut machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(AwaitingBill)
            .tone("neutral")
            .state(PendingApproval)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .state(Rejected)
            .tone("danger")
            .terminal()
            .state(Voided)
            .tone("neutral")
            .terminal()
            .transition(submit())
            .transition(submit_over_budget())
            .transition(
                Transition::new(SUBMIT_OVER_BUDGET_APPROVED)
                    .label("Approve")
                    .from([PendingApproval])
                    .to(Approved)
                    .system()
                    .effect(|e: &CostEntry, ctx, tx| {
                        // The user who gave the last yes.
                        let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
                        let id = e.id;
                        Box::pin(async move { set(tx, id, &[("approved_by", &approver)]).await })
                    }),
            )
            .transition(
                Transition::new(BACK_TO_DRAFT)
                    .label("Approval ended")
                    .from([PendingApproval])
                    .to(Draft)
                    .system()
                    .effect(|e: &CostEntry, ctx, tx| {
                        // The rejection comment, or the kernel's text.
                        let reason = ctx.reason().map(|r| r.trim().to_owned());
                        let id = e.id;
                        Box::pin(async move { set(tx, id, &[("reason", &reason)]).await })
                    }),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .from([Draft])
                    .to(Rejected)
                    .system()
                    .effect(|e: &CostEntry, ctx, tx| {
                        let reason = ctx
                            .reason()
                            .map(str::trim)
                            .filter(|r| !r.is_empty())
                            .map(str::to_owned)
                            .or_else(|| e.reason.clone())
                            .unwrap_or_else(|| REJECTED.to_owned());
                        let id = e.id;
                        Box::pin(async move { set(tx, id, &[("reason", &Some(reason))]).await })
                    }),
            )
            .transition(await_bill())
            .transition(bill_approved())
            .transition(void_with_bill());
        // The kernel's `void` and `void_approved` (#27); nothing is posted,
        // so a void needs no approval and has no reverse handlers.
        for transition in
            voids::transitions::<CostEntry, _>(SUBJECT_TYPE, [Approved, PendingApproval])
        {
            machine = machine.transition(crate::voids::with_request_ended(transition));
        }
        machine
    }
}

/// `await_bill`: a linked entry whose bill is not approved yet is left out of
/// every check.
fn await_bill() -> Transition<CostEntry> {
    Transition::new(AWAIT_BILL)
        .label("Wait for the bill")
        .from([CostStatus::Draft])
        .to(CostStatus::AwaitingBill)
        .system()
        .guard("linked_to_a_bill", |e: &CostEntry, _ctx| {
            let verdict = if e.supplier_bill_id.is_some() && e.supplier_bill_line_id.is_some() {
                Ok(())
            } else {
                Err(crate::texts::english("wl_sales.costs.not_linked", &[]))
            };
            Box::pin(async move { verdict })
        })
}

/// `bill_approved`: the bill is approved; the one who linked the line becomes
/// the requester and the entry is submitted again with a budget check.
fn bill_approved() -> Transition<CostEntry> {
    Transition::new(BILL_APPROVED)
        .label("Bill approved")
        .from([CostStatus::AwaitingBill])
        .to(CostStatus::Draft)
        .system()
        .effect(|e: &CostEntry, _ctx, tx| {
            let id = e.id;
            let linked_by = e.linked_by.unwrap_or(e.requested_by);
            Box::pin(async move { set(tx, id, &[("requested_by", &linked_by)]).await })
        })
}

/// `void_with_bill`: the supplier bill was voided before the entry counted.
fn void_with_bill() -> Transition<CostEntry> {
    Transition::new(VOID_WITH_BILL)
        .label("Bill voided")
        .from([CostStatus::AwaitingBill, CostStatus::Draft])
        .to(CostStatus::Voided)
        .system()
        .effect(|e: &CostEntry, ctx, tx| {
            let id = e.id;
            let reason = ctx
                .reason()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map_or_else(
                    || crate::texts::english("wl_sales.costs.bill_voided_reason", &[]),
                    str::to_owned,
                );
            Box::pin(async move { set(tx, id, &[("reason", &Some(reason))]).await })
        })
}

/// The budget check the run was given.
fn check_of(ctx: &renox::state_machine::TransitionCtx) -> Option<BudgetCheck> {
    ctx.input_as::<BudgetCheck>().ok()
}

fn refused(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// `submit`: within the budget (or nothing to check), approved on save.
fn submit() -> Transition<CostEntry> {
    Transition::new(SUBMIT)
        .label("Submit")
        .from([CostStatus::Draft])
        .to(CostStatus::Approved)
        .system()
        .guard("within_budget", |e: &CostEntry, ctx| {
            let verdict = match check_of(ctx) {
                Some(check) if !check.over(e.amount) => Ok(()),
                Some(_) => Err(crate::texts::english("wl_sales.costs.over_budget", &[])),
                None => Err(crate::texts::english("wl_sales.costs.no_check", &[])),
            };
            Box::pin(async move { verdict })
        })
        .effect(|e: &CostEntry, _ctx, tx| {
            let id = e.id;
            Box::pin(async move { set(tx, id, &[("reason", &None::<String>)]).await })
        })
}

/// `submit_over_budget`: over the budget, an approval request.
fn submit_over_budget() -> Transition<CostEntry> {
    Transition::new(SUBMIT_OVER_BUDGET)
        .label("Request approval")
        .from([CostStatus::Draft])
        .to(CostStatus::PendingApproval)
        .system()
        .guard("over_budget", |e: &CostEntry, ctx| {
            let verdict = match check_of(ctx) {
                Some(check) if check.over(e.amount) => Ok(()),
                Some(_) => Err(crate::texts::english("wl_sales.costs.within_budget", &[])),
                None => Err(crate::texts::english("wl_sales.costs.no_check", &[])),
            };
            Box::pin(async move { verdict })
        })
        .effect(|e: &CostEntry, ctx, tx| {
            let e = e.clone();
            let check = check_of(ctx);
            Box::pin(async move {
                let require = Require::new(
                    policy::ACTION,
                    e.subject(),
                    SUBMIT_OVER_BUDGET,
                    e.requested_by,
                    "draft",
                )
                .payload(json!({
                    "campaign_id": e.campaign_id,
                    "amount": e.amount,
                    "approved_total": check.map(|c| c.approved),
                    "budget": check.and_then(|c| c.budget),
                }));
                match approvals::require_in(&mut *tx, &require).await? {
                    Required::Pending(request) => {
                        set(
                            &mut *tx,
                            e.id,
                            &[
                                ("approval_request_id", &Some(request.id)),
                                ("reason", &None::<String>),
                            ],
                        )
                        .await
                    }
                    // Without a policy an entry over the budget would be
                    // counted unapproved; the transaction rolls back instead.
                    Required::Allowed => Err(refused(&crate::texts::english(
                        "wl_sales.costs.no_policy",
                        &[],
                    ))),
                }
            })
        })
}

/// The machine's registry entry: the waiting state `pending_approval`, the
/// action `campaigns.cost_over_budget` and the campaign's branch as scope.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<CostEntry>(SUBJECT_TYPE)
        .waiting(CostStatus::PendingApproval)
        .action(SUBMIT_OVER_BUDGET, policy::ACTION)
        .scope(|e: &CostEntry| e.scope())
        .build()
}

/// Registers the machine, the default policy and the listeners.
pub(crate) fn register(app: &mut Registry) {
    subject_machine().register();
    renox::state_machine::register::<CostEntry>();
    policy::policy().register();
    // After the commit of an entry entering `approved`: the budget alerts.
    app.listen(
        |event: Transitioned<CostEntry>, state: AppState| async move {
            let name = event.transition.as_str();
            if ![SUBMIT, SUBMIT_OVER_BUDGET_APPROVED].contains(&name) {
                return Ok(());
            }
            let Some(entry) = CostEntry::find(&state.db, event.key).await? else {
                return Ok(());
            };
            budget::check_alerts(&state, entry.campaign_id).await
        },
    );
    // A rejected request: the entry is back in draft; reject it.
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        if event.subject.subject_type != SUBJECT_TYPE
            || event.decision != ApprovalEnd::Rejected
            || event.state_returned_to.is_none()
        {
            return Ok(());
        }
        rejected(&state, event.subject.subject_id).await
    });
}

/// Runs `reject` on entry `id`, which the kernel sent back to `draft`, and
/// tells the campaign's owner.
async fn rejected(state: &AppState, id: i64) -> Result {
    let Some(entry) = CostEntry::find(&state.db, id).await? else {
        return Ok(());
    };
    if entry.status != CostStatus::Draft {
        return Ok(());
    }
    let mut run = entry.transition(REJECT).by_system();
    if let Some(reason) = entry.reason.as_deref() {
        run = run.reason(reason);
    }
    let _outcome = run.run(state).await.map_err(|e| e.into_error())?;
    let Some(entry) = CostEntry::find(&state.db, id).await? else {
        return Ok(());
    };
    notices::cost_rejected(state, &entry).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared_and_system() {
        let machine = CostEntry::machine();
        let table = [
            (SUBMIT, "draft", "approved"),
            (SUBMIT_OVER_BUDGET, "draft", "pending_approval"),
            (SUBMIT_OVER_BUDGET_APPROVED, "pending_approval", "approved"),
            (BACK_TO_DRAFT, "pending_approval", "draft"),
            (REJECT, "draft", "rejected"),
        ];
        assert_eq!(machine.transitions().len(), table.len() + 5);
        for (name, from, to) in table {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), [from], "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["draft"]);
    }

    #[test]
    fn the_bill_and_void_transitions_are_declared() {
        let machine = CostEntry::machine();
        let table = [
            (AWAIT_BILL, vec!["draft"], "awaiting_bill", true),
            (BILL_APPROVED, vec!["awaiting_bill"], "draft", true),
            (
                VOID_WITH_BILL,
                vec!["awaiting_bill", "draft"],
                "voided",
                true,
            ),
            (
                "void",
                vec!["approved", "pending_approval"],
                "voided",
                false,
            ),
            (
                "void_approved",
                vec!["approved", "pending_approval"],
                "voided",
                true,
            ),
        ];
        for (name, from, to, system) in table {
            let t = machine.find(name).expect(name);
            assert_eq!(t.is_system(), system, "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
    }

    #[test]
    fn the_registry_entry_waits_in_pending_approval() {
        let m = subject_machine();
        assert_eq!(m.waiting_state(), Some("pending_approval"));
        assert_eq!(
            m.action_key(SUBMIT_OVER_BUDGET),
            "campaigns.cost_over_budget"
        );
    }
}
