//! The Refund state machine (wl-payments §4, A11).
//!
//! ```text
//! [*] --> requested --approve--> approved --pay--> paid
//! requested --reject--> rejected (terminal)
//! approved --pay_by_system--> paid   (the gateway confirmed a refund)
//! approved, paid --fail, fail_by_system--> failed --retry--> approved
//! failed --cancel--> cancelled (terminal)
//! ```
//!
//! All six states are declared with their tones. This part builds
//! `approve`, `reject`, `pay`, `pay_by_system`, and `fail` and `fail_by_system`
//! ([`crate::refund_fail`]); `retry`, `change_method` and
//! `cancel` come with the refund issues that follow, into this `machine()`.
//!
//! `approve` is the approval of `FIN.ACL.04`: it needs `refunds.approve`
//! and never the user who requested the refund (segregation of duties,
//! which this module never turns off). It is the one decision, so it is not
//! deferred as a kernel approval request. A listener on
//! `Transitioned<Refund>` emits `RefundPaid` after a `pay` commits.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::payments::{PaymentMethod, RefundPaid};

use crate::permissions;
use crate::refund::{Refund, RefundStatus};
use crate::refund_attempt::{AttemptOutcome, RefundAttempt};
use crate::refund_fail;

/// The `approve` transition's name.
pub const APPROVE: &str = "approve";
/// The `reject` transition's name.
pub const REJECT: &str = "reject";
/// The `pay` transition's name.
pub const PAY: &str = "pay";

/// The `pay_by_system` transition's name: the gateway confirmed a refund.
pub const PAY_BY_SYSTEM: &str = "pay_by_system";

/// What a `pay_by_system` run carries: the `pending` attempt it closes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PayBySystemInput {
    /// The attempt the gateway answered for.
    pub attempt_id: i64,
}

/// What a `pay` run carries to its guards and effect, gathered by
/// [`crate::refund::pay`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PayInput {
    /// How the refund is paid.
    pub method: PaymentMethod,
    /// The payment reference.
    pub reference: String,
    /// The outgoing payment recorded in the same transaction.
    pub payment_id: i64,
    /// Whether the source still covers the amount.
    pub covered: bool,
}

fn pay_input(ctx: &TransitionCtx) -> Option<PayInput> {
    ctx.input_as::<PayInput>().ok()
}

/// Why `approver` may not approve a refund `requested_by` asked for
/// (`FIN.ACL.04`), if so. A system request has no requester.
pub(crate) fn sod_problem(requested_by: Option<i64>, approver: Option<i64>) -> Option<String> {
    match (requested_by, approver) {
        (Some(requester), Some(approver)) if requester == approver => Some(
            "FIN.ACL.04: whoever requested a refund cannot approve it; another user must."
                .to_owned(),
        ),
        (_, None) => Some("A refund is approved by a user.".to_owned()),
        _ => None,
    }
}

impl StateMachine for Refund {
    type State = RefundStatus;
    const COLUMN: renox::db::Col<Refund, RefundStatus> = Refund::STATUS;

    fn machine() -> Machine<Self> {
        use RefundStatus::{Approved, Cancelled, Failed, Paid, Rejected, Requested};
        let machine = Machine::new()
            .initial([Requested])
            .state(Requested)
            .tone("warning")
            .state(Approved)
            .tone("info")
            .state(Paid)
            .tone("success")
            .state(Failed)
            .tone("error")
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal();
        transitions().into_iter().fold(machine, Machine::transition)
    }
}

fn transitions() -> Vec<Transition<Refund>> {
    let mut all = vec![approve(), reject(), pay(), pay_by_system()];
    all.extend(refund_fail::transitions());
    all
}

fn approve() -> Transition<Refund> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("primary")
        .permission(permissions::REFUNDS_APPROVE)
        .from([RefundStatus::Requested])
        .to(RefundStatus::Approved)
        .guard("not_the_requester", |refund: &Refund, ctx| {
            let problem = sod_problem(refund.requested_by, ctx.actor().map(|user| user.id));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|refund: &Refund, ctx, tx| {
            let id = refund.id;
            let approver = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                // Not the status column, so outside the machine's guard.
                Refund::query()
                    .where_(Refund::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("approved_by", &approver),
                            ("approved_at", &Some(renox::db::now())),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

fn reject() -> Transition<Refund> {
    Transition::new(REJECT)
        .label("Reject")
        .tone("danger")
        .permission(permissions::REFUNDS_APPROVE)
        .from([RefundStatus::Requested])
        .to(RefundStatus::Rejected)
        .reason_required()
        .effect(|refund: &Refund, ctx, tx| {
            let id = refund.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                Refund::query()
                    .where_(Refund::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("decision_reason", &reason),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

fn pay() -> Transition<Refund> {
    Transition::new(PAY)
        .label("Pay")
        .tone("primary")
        .permission(permissions::REFUNDS_PAY)
        .from([RefundStatus::Approved])
        .to(RefundStatus::Paid)
        .guard("reference_given", |_refund: &Refund, ctx| {
            let given = pay_input(ctx).is_some_and(|input| !input.reference.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err("The method and a payment reference are required.".to_owned())
                }
            })
        })
        .guard("source_covers", |_refund: &Refund, ctx| {
            let covered = pay_input(ctx).is_some_and(|input| input.covered);
            Box::pin(async move {
                if covered {
                    Ok(())
                } else {
                    Err("The source no longer covers the refund.".to_owned())
                }
            })
        })
        .effect(|refund: &Refund, ctx, tx| {
            let id = refund.id;
            let input = pay_input(ctx);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a refund payment without its input"))
                })?;
                Refund::query()
                    .where_(Refund::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("method", &Some(input.method.as_str().to_owned())),
                            ("reference", &Some(input.reference.trim().to_owned())),
                            ("payment_id", &Some(input.payment_id)),
                            ("paid_at", &Some(renox::db::now())),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

fn pay_by_system() -> Transition<Refund> {
    Transition::new(PAY_BY_SYSTEM)
        .label("Gateway refund confirmed")
        .tone("primary")
        .system()
        .from([RefundStatus::Approved])
        .to(RefundStatus::Paid)
        .guard("attempt_given", |_refund: &Refund, ctx| {
            let given = ctx.input_as::<PayBySystemInput>().is_ok();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err("The gateway refund attempt is required.".to_owned())
                }
            })
        })
        .effect(|refund: &Refund, ctx, tx| {
            let id = refund.id;
            let input = ctx.input_as::<PayBySystemInput>().ok();
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!(
                        "a gateway payment without its attempt"
                    ))
                })?;
                // No outgoing payment: the gateway moved the money.
                Refund::query()
                    .where_(Refund::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("paid_at", &Some(renox::db::now())),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                RefundAttempt::query()
                    .where_(RefundAttempt::ID.eq(input.attempt_id))
                    .where_(RefundAttempt::OUTCOME.eq(AttemptOutcome::Pending))
                    .update(
                        &mut *tx,
                        &[
                            ("outcome", &AttemptOutcome::Paid.as_str().to_owned()),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// Registers the listener: after a `pay` or `pay_by_system` commits,
/// `RefundPaid` is emitted.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: renox::state_machine::Transitioned<Refund>, state: AppState| async move {
            if event.transition != PAY && event.transition != PAY_BY_SYSTEM {
                return Ok(());
            }
            // The committed row.
            let refund = Refund::find_or_404(&state.db, event.key).await?;
            state
                .emit(RefundPaid {
                    refund: refund.refund_id(),
                    party: refund.party(),
                    amount: refund.amount,
                    source: refund.source().ok_or_else(|| {
                        Error::Internal(renox::anyhow::anyhow!(
                            "refund {} has an unknown source {} ({})",
                            refund.id,
                            refund.source_subject(),
                            refund.source_kind
                        ))
                    })?,
                })
                .await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::TransitionError;

    fn input(reference: &str, covered: bool) -> TransitionCtx {
        let input = PayInput {
            method: PaymentMethod::Transfer,
            reference: reference.to_owned(),
            payment_id: 3,
            covered,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    #[test]
    fn all_six_states_are_declared_and_a_refund_starts_requested() {
        let machine = Refund::machine();
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in [
            "requested",
            "approved",
            "paid",
            "failed",
            "rejected",
            "cancelled",
        ] {
            assert!(names.contains(&state), "{state}");
        }
        assert_eq!(machine.initial_states(), ["requested"]);
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
        for state in ["rejected", "cancelled"] {
            let found = machine.states().iter().find(|s| s.name == state);
            assert!(found.is_some_and(|s| s.terminal), "{state}");
        }
        let paid = machine.states().iter().find(|s| s.name == "paid");
        assert!(paid.is_some_and(|s| !s.terminal));
    }

    #[test]
    fn the_requester_cannot_approve_but_another_user_or_the_system_request_can() {
        assert!(sod_problem(Some(3), Some(3)).is_some_and(|m| m.contains("FIN.ACL.04")));
        assert!(sod_problem(Some(3), Some(4)).is_none());
        assert!(sod_problem(None, Some(4)).is_none());
        assert!(sod_problem(Some(3), None).is_some());
    }

    #[test]
    fn each_transition_leaves_only_its_states() {
        let machine = Refund::machine();
        let from = |name: &str, state: RefundStatus| {
            machine
                .find(name)
                .expect("transition")
                .check_from(state)
                .is_ok()
        };
        for name in [APPROVE, REJECT] {
            assert!(from(name, RefundStatus::Requested), "{name}");
            assert!(!from(name, RefundStatus::Approved), "{name}");
        }
        assert!(from(PAY, RefundStatus::Approved));
        assert!(!from(PAY, RefundStatus::Requested));
        assert!(!from(PAY, RefundStatus::Paid));
    }

    #[test]
    fn pay_by_system_leaves_only_approved() {
        let machine = Refund::machine();
        let found = machine.find(PAY_BY_SYSTEM).expect("transition");
        assert!(found.check_from(RefundStatus::Approved).is_ok());
        assert!(found.check_from(RefundStatus::Requested).is_err());
        assert!(found.check_from(RefundStatus::Paid).is_err());
        assert!(found.check_from(RefundStatus::Failed).is_err());
    }

    #[renox::test]
    async fn pay_by_system_needs_its_attempt() {
        let machine = Refund::machine();
        let found = machine.find(PAY_BY_SYSTEM).expect("transition");
        let refund = Refund::default();
        let with = TransitionCtx::new(None).with_input(
            renox::serde_json::to_value(PayBySystemInput { attempt_id: 1 }).expect("json"),
        );
        assert!(found.check_guards(&refund, &with).await.is_ok());
        assert!(matches!(
            found.check_guards(&refund, &TransitionCtx::new(None)).await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[renox::test]
    async fn pay_needs_a_reference_and_a_covering_source() {
        let machine = Refund::machine();
        let pay = machine.find(PAY).expect("transition");
        let refund = Refund::default();
        assert!(
            pay.check_guards(&refund, &input("TRX-1", true))
                .await
                .is_ok()
        );
        for ctx in [
            input("  ", true),
            input("TRX-1", false),
            TransitionCtx::new(None),
        ] {
            assert!(matches!(
                pay.check_guards(&refund, &ctx).await,
                Err(TransitionError::Guard(_))
            ));
        }
    }

    #[renox::test]
    async fn approve_is_refused_to_the_requester_without_a_user() {
        let machine = Refund::machine();
        let approve = machine.find(APPROVE).expect("transition");
        let refund = Refund {
            requested_by: Some(3),
            ..Default::default()
        };
        // No user in the run: refused.
        assert!(matches!(
            approve
                .check_guards(&refund, &TransitionCtx::new(None))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }
}
