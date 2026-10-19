//! The Supplier bill state machine (wl-invoicing §4).
//!
//! ```text
//! [*] --> draft --submit--> matched --approve--> approved
//!         draft --submit_held--> exception --approve--> approved
//!         draft | matched | exception --void--> void (terminal)
//!         approved | partially_paid --pay--> partially_paid | paid  (system)
//!         partially_paid | paid --unallocate--> approved | partially_paid  (system)
//! ```
//!
//! All seven states are declared with their tones and a new bill is inserted
//! only as `draft`; `SupplierBill` is `#[model(state_machine)]`, so `save`
//! refuses a changed status. `submit` and `submit_held` are in
//! [`crate::bill_submit`]; `pay` and `pay_in_full` (system only) in
//! [`crate::bill_pay`]; `unallocate` and `unallocate_partially` (system only)
//! in [`crate::bill_unallocate`].
//!
//! `approve` (matched or exception to approved, `supplier_bills.approve`)
//! takes a `reason` (the run's reason, which the history row carries). From
//! `exception` the reason must not be blank, and a bill held as a possible
//! duplicate, or held for work after the contract end, cannot be approved by
//! the user who entered it, whatever their permissions. The approver and the
//! reason are stored on the bill; one approval covers every exception row.
//!
//! A bill with a line coded to owner drawings is approved only after the
//! approval of the drawing ([`crate::drawings`], `FIN.ACL.21`): the effect of
//! both approve transitions refuses (422) until one covers the same amount.
//!
//! `void` (draft, matched or exception to void, `supplier_bills.approve`)
//! takes a `reason` that must not be blank and is stored on the bill. The
//! creator may void. Each has a system twin (`approve_system`,
//! `void_system`) for callers with no signed-in user; they name the acting
//! user in the run's input.
//!
//! The machine is registered with the kernel as the subject type
//! `invoicing.supplier_bill`, with no waiting state, so `approve` and `void`
//! post through `machines::routes::<SupplierBill>()`.

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::machines::SubjectMachine;

use crate::credit_notes::{By, run_of};
use crate::permissions;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};
use crate::texts::english;

/// The subject type of a supplier bill in the kernel registry.
pub const SUBJECT_TYPE: &str = "invoicing.supplier_bill";

/// The transition to `approved`.
pub const APPROVE: &str = "approve";
/// The system twin of [`APPROVE`].
pub const APPROVE_SYSTEM: &str = "approve_system";
/// The transition to `void`.
pub const VOID: &str = "void";
/// The system twin of [`VOID`].
pub const VOID_SYSTEM: &str = "void_system";

/// The states `approve` leaves.
const APPROVABLE: [SupplierBillStatus; 2] =
    [SupplierBillStatus::Matched, SupplierBillStatus::Exception];

/// The states `void` leaves.
const VOIDABLE: [SupplierBillStatus; 3] = [
    SupplierBillStatus::Draft,
    SupplierBillStatus::Matched,
    SupplierBillStatus::Exception,
];

/// The input of a system run of `approve` or `void`: the user it acts for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ActInput {
    /// The acting user's id.
    pub by: Option<i64>,
}

impl StateMachine for SupplierBill {
    type State = SupplierBillStatus;
    const COLUMN: renox::db::Col<SupplierBill, SupplierBillStatus> = SupplierBill::STATUS;

    fn machine() -> Machine<Self> {
        use SupplierBillStatus::{Approved, Draft, Exception, Matched, Paid, PartiallyPaid, Void};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Matched)
            .tone("info")
            .state(Exception)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .state(PartiallyPaid)
            .tone("info")
            .state(Paid)
            .tone("success")
            .state(Void)
            .tone("neutral")
            .terminal();
        crate::bill_submit::transitions()
            .into_iter()
            .chain(approve_transitions())
            .chain(void_transitions())
            .chain(crate::bill_pay::transitions())
            .chain(crate::bill_unallocate::transitions())
            .fold(machine, Machine::transition)
    }
}

/// The user running the transition: the signed-in user, else the user a
/// system run acts for.
fn actor_id(ctx: &TransitionCtx) -> Option<i64> {
    ctx.actor()
        .map(|user| user.id)
        .or_else(|| ctx.input_as::<ActInput>().ok().and_then(|i| i.by))
}

/// The reason given, trimmed; `None` when blank.
fn reason_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// The `approve` and `approve_system` transitions.
fn approve_transitions() -> [Transition<SupplierBill>; 2] {
    [
        approve_rules(
            Transition::new(APPROVE)
                .label("Approve")
                .tone("success")
                .permission(permissions::SUPPLIER_BILLS_APPROVE),
        ),
        approve_rules(
            Transition::new(APPROVE_SYSTEM)
                .label("Approve (system)")
                .system(),
        ),
    ]
}

/// What both approve transitions share.
fn approve_rules(transition: Transition<SupplierBill>) -> Transition<SupplierBill> {
    transition
        .from(APPROVABLE)
        .to(SupplierBillStatus::Approved)
        .guard("reason_given", |bill: &SupplierBill, ctx| {
            let held = bill.status == SupplierBillStatus::Exception;
            let given = reason_of(ctx).is_some();
            Box::pin(async move {
                if !held || given {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.supplier_bill.problem.approve_reason").to_owned())
                }
            })
        })
        .guard("not_creator", |bill: &SupplierBill, ctx| {
            // A held possible duplicate, or work after the contract end:
            // never the user who entered it.
            let creator = bill.created_by;
            let blocked = bill.status == SupplierBillStatus::Exception
                && (bill.duplicate_of.is_some() || bill.after_end_hold)
                && creator.is_some()
                && actor_id(ctx) == creator;
            Box::pin(async move {
                if blocked {
                    Err(english("wl_invoicing.supplier_bill.problem.creator").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|bill: &SupplierBill, ctx, tx| {
            let id = bill.id;
            let approver = actor_id(ctx);
            let reason = reason_of(ctx);
            Box::pin(async move {
                // A bill with an owner drawing is approved only after the
                // approval of the drawing (`FIN.ACL.21`); a refusal rolls
                // the transition back (422).
                crate::drawings::ensure_bill_approved(&mut *tx, id).await?;
                // Not the status column, so outside the machine's guard.
                SupplierBill::query()
                    .where_(SupplierBill::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("approved_by", &approver),
                            ("approval_reason", &reason),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The `void` and `void_system` transitions.
fn void_transitions() -> [Transition<SupplierBill>; 2] {
    [
        void_rules(
            Transition::new(VOID)
                .label("Void")
                .tone("danger")
                .permission(permissions::SUPPLIER_BILLS_APPROVE),
        ),
        void_rules(Transition::new(VOID_SYSTEM).label("Void (system)").system()),
    ]
}

/// What both void transitions share.
fn void_rules(transition: Transition<SupplierBill>) -> Transition<SupplierBill> {
    transition
        .from(VOIDABLE)
        .to(SupplierBillStatus::Void)
        .guard("reason_given", |_bill: &SupplierBill, ctx| {
            let given = reason_of(ctx).is_some();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("wl_invoicing.supplier_bill.problem.void_reason").to_owned())
                }
            })
        })
        .effect(|bill: &SupplierBill, ctx, tx| {
            let id = bill.id;
            let reason = reason_of(ctx);
            Box::pin(async move {
                SupplierBill::query()
                    .where_(SupplierBill::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("void_reason", &reason), ("updated_at", &renox::db::now())],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// Runs `user_name` (or `system_name`) on bill `id` with `reason`, in its own
/// transaction; the reason is the history row's.
async fn act(
    state: &AppState,
    by: By<'_>,
    id: i64,
    reason: &str,
    user_name: &str,
    system_name: &str,
) -> std::result::Result<SupplierBill, TransitionError> {
    let bill = SupplierBill::find_or_404(&state.db, id).await?;
    let mut run =
        run_of(&bill, &by, user_name, system_name, json!({})).input(&ActInput { by: by.id() });
    let reason = reason.trim();
    if !reason.is_empty() {
        run = run.reason(reason);
    }
    let _outcome = run.run(state).await?;
    Ok(SupplierBill::find_or_404(&state.db, id).await?)
}

/// `by` approves bill `id` with `reason` (`approve`).
///
/// # Errors
///
/// `Forbidden` without `supplier_bills.approve`; `NotFrom` unless the bill
/// is `matched` or `exception`; `Guard` (422) for a blank reason on a held
/// bill, or for the creator of a held possible duplicate.
pub async fn approve(
    state: &AppState,
    by: By<'_>,
    id: i64,
    reason: &str,
) -> std::result::Result<SupplierBill, TransitionError> {
    act(state, by, id, reason, APPROVE, APPROVE_SYSTEM).await
}

/// `by` voids bill `id` with `reason` (`void`).
///
/// # Errors
///
/// `Forbidden` without `supplier_bills.approve`; `NotFrom` unless the bill
/// is `draft`, `matched` or `exception`; `Guard` (422) for a blank reason.
pub async fn void(
    state: &AppState,
    by: By<'_>,
    id: i64,
    reason: &str,
) -> std::result::Result<SupplierBill, TransitionError> {
    act(state, by, id, reason, VOID, VOID_SYSTEM).await
}

/// Registers the machine with the kernel and `rnx state:diagram`.
pub(crate) fn register() {
    SubjectMachine::of::<SupplierBill>(SUBJECT_TYPE).register();
    renox::state_machine::register::<SupplierBill>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held() -> SupplierBill {
        SupplierBill {
            id: 2,
            status: SupplierBillStatus::Exception,
            created_by: Some(7),
            duplicate_of: Some(1),
            ..Default::default()
        }
    }

    fn ctx(actor_id: i64, reason: Option<&str>) -> TransitionCtx {
        let mut ctx = TransitionCtx::new(None)
            .with_input(renox::serde_json::to_value(ActInput { by: Some(actor_id) }).unwrap());
        if let Some(reason) = reason {
            ctx = ctx.with_reason(reason);
        }
        ctx
    }

    async fn refusal(name: &str, bill: &SupplierBill, ctx: &TransitionCtx) -> Option<String> {
        match SupplierBill::machine()
            .find(name)
            .unwrap()
            .check_guards(bill, ctx)
            .await
        {
            Ok(()) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    #[test]
    fn the_seven_states_and_their_tones() {
        let machine = SupplierBill::machine();
        let states: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            states,
            [
                "draft",
                "matched",
                "exception",
                "approved",
                "partially_paid",
                "paid",
                "void"
            ]
        );
        assert_eq!(machine.initial_states(), ["draft"]);
        let void = machine.states().last().unwrap();
        assert!(void.terminal);
        assert_eq!(machine.find(APPROVE).unwrap().from_states().len(), 2);
        assert_eq!(machine.find(VOID).unwrap().from_states().len(), 3);
    }

    #[test]
    fn the_machine_is_registered_without_a_waiting_state() {
        let machine = SubjectMachine::of::<SupplierBill>(SUBJECT_TYPE).build();
        assert_eq!(machine.subject_type(), "invoicing.supplier_bill");
        assert_eq!(machine.waiting_state(), None);
        assert!(machine.transition(APPROVE).is_some());
        assert!(machine.transition(VOID).is_some());
    }

    #[renox::test]
    async fn a_held_bill_is_approved_with_a_reason_by_another_user() {
        assert_eq!(
            refusal(APPROVE, &held(), &ctx(8, Some("two deliveries"))).await,
            None
        );
        let blank = refusal(APPROVE, &held(), &ctx(8, Some("  "))).await;
        assert!(blank.unwrap().contains("reason"));
        assert!(refusal(APPROVE, &held(), &ctx(8, None)).await.is_some());
        let creator = refusal(APPROVE, &held(), &ctx(7, Some("ok"))).await;
        assert!(creator.unwrap().contains("entered it"));
    }

    #[renox::test]
    async fn a_matched_bill_needs_no_reason_and_its_creator_may_approve() {
        let matched = SupplierBill {
            status: SupplierBillStatus::Matched,
            ..held()
        };
        assert_eq!(refusal(APPROVE, &matched, &ctx(7, None)).await, None);
        // Held for another reason than a duplicate: the creator guard is the
        // duplicate's only.
        let other = SupplierBill {
            duplicate_of: None,
            ..held()
        };
        assert_eq!(refusal(APPROVE, &other, &ctx(7, Some("ok"))).await, None);
    }

    #[renox::test]
    async fn a_void_needs_a_reason_and_the_creator_may_void() {
        assert_eq!(
            refusal(VOID, &held(), &ctx(7, Some("duplicate"))).await,
            None
        );
        assert!(refusal(VOID, &held(), &ctx(7, None)).await.is_some());
        assert!(refusal(VOID, &held(), &ctx(7, Some(" "))).await.is_some());
    }
}
