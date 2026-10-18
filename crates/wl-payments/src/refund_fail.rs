//! Failing a refund (wl-payments §4 `fail`, A5).
//!
//! `fail` (a user with `refunds.pay`, when the bank returns a transfer or
//! the card cannot take the refund) and `fail_by_system` (the gateway
//! declined) move an `approved` or `paid` refund to `failed`. The input is a
//! kind and a reason; the reason may not be blank, and a user cannot choose
//! `gateway_declined`.
//!
//! The effects, in the transition's transaction: the open attempt is closed
//! as `failed` with the kind, reason, day (today in Asia/Dubai) and user
//! (never deleted or rewritten later), and the kind and reason are kept on
//! the refund. From `paid`, [`crate::refund::fail`] also reverses the
//! outgoing payment through `reverse_by_system` in the same transaction.
//!
//! After the commit the listener emits `RefundFailed` (the owner of a
//! source outside this module restores its side) and sends the notices
//! ([`crate::notices::refund_failed`]).

use renox::prelude::*;
use renox::state_machine::{Transition, TransitionCtx, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::payments::{
    PaymentId, RefundAttemptRef, RefundFailed, RefundFailedFrom, RefundFailureKind,
};

use crate::notices;
use crate::permissions;
use crate::refund::{Refund, RefundStatus};
use crate::refund_attempt::{AttemptOutcome, RefundAttempt};

/// The `fail` transition's name.
pub const FAIL: &str = "fail";

/// The `fail_by_system` transition's name.
pub const FAIL_BY_SYSTEM: &str = "fail_by_system";

/// What a `fail` run carries to its guards and effect, gathered by
/// [`crate::refund::fail`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct FailInput {
    /// Why it failed; `None` is refused.
    pub kind: Option<RefundFailureKind>,
    /// The reason given; may not be blank.
    pub reason: String,
    /// The attempt that failed; `None` is refused (nothing was tried).
    pub attempt_id: Option<i64>,
}

fn fail_input(ctx: &TransitionCtx) -> Option<FailInput> {
    ctx.input_as::<FailInput>().ok()
}

/// Why `input` cannot fail a refund, if it cannot. A user (`manual`) may not
/// choose `gateway_declined`.
pub(crate) fn input_problem(input: Option<&FailInput>, manual: bool) -> Option<&'static str> {
    let Some(input) = input else {
        return Some("A kind and a reason are required.");
    };
    let Some(kind) = input.kind else {
        return Some("A failure kind is required.");
    };
    if manual && !kind.is_manual() {
        return Some("Only the gateway declines a refund.");
    }
    if input.reason.trim().is_empty() {
        return Some("A reason is required.");
    }
    if input.attempt_id.is_none() {
        return Some("The refund has no attempt to fail.");
    }
    None
}

/// `transition` with the guards and effect `fail` and `fail_by_system`
/// share.
fn with_rules(transition: Transition<Refund>, manual: bool) -> Transition<Refund> {
    transition
        .from([RefundStatus::Approved, RefundStatus::Paid])
        .to(RefundStatus::Failed)
        .reason_required()
        .guard("kind_and_reason_given", move |_refund: &Refund, ctx| {
            let problem = input_problem(fail_input(ctx).as_ref(), manual);
            Box::pin(async move { problem.map_or(Ok(()), |text| Err(text.to_owned())) })
        })
        .effect(|refund: &Refund, ctx, tx| {
            let id = refund.id;
            let input = fail_input(ctx);
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a refund failure without its input"))
                })?;
                let kind = input.kind.unwrap_or_default();
                let reason = input.reason.trim().to_owned();
                let now = renox::db::now();
                if let Some(attempt_id) = input.attempt_id {
                    // The attempt is closed, never deleted.
                    RefundAttempt::query()
                        .where_(RefundAttempt::ID.eq(attempt_id))
                        .where_(
                            RefundAttempt::OUTCOME
                                .is_in([AttemptOutcome::Pending, AttemptOutcome::Paid]),
                        )
                        .update(
                            &mut *tx,
                            &[
                                ("outcome", &AttemptOutcome::Failed.as_str().to_owned()),
                                ("failure_kind", &Some(kind.as_str().to_owned())),
                                ("failure_reason", &Some(reason.clone())),
                                ("failed_on", &Some(today())),
                                ("failed_by", &by),
                                ("updated_at", &now),
                            ],
                        )
                        .await?;
                }
                // Not the status column, so outside the machine's guard.
                Refund::query()
                    .where_(Refund::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("failure_kind", &Some(kind.as_str().to_owned())),
                            ("failure_reason", &Some(reason)),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `fail` and `fail_by_system`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Refund>> {
    vec![
        with_rules(
            Transition::new(FAIL)
                .label("Mark as failed")
                .tone("danger")
                .permission(permissions::REFUNDS_PAY),
            true,
        ),
        with_rules(
            Transition::new(FAIL_BY_SYSTEM)
                .label("Refund declined")
                .tone("danger")
                .system(),
            false,
        ),
    ]
}

/// The state a refund failed from, as the event names it.
pub(crate) fn failed_from(state: &str) -> Option<RefundFailedFrom> {
    match state {
        "approved" => Some(RefundFailedFrom::Approved),
        "paid" => Some(RefundFailedFrom::Paid),
        _ => None,
    }
}

/// The event of a committed failure of `refund` (as it is now), closed on
/// `attempt`, from `from`. `None` for a source kind not known (a corrupt
/// row) or an attempt that did not fail.
pub(crate) fn failed_event(
    refund: &Refund,
    attempt: &RefundAttempt,
    from: RefundFailedFrom,
) -> Option<RefundFailed> {
    Some(RefundFailed {
        refund: refund.refund_id(),
        party: refund.party(),
        amount: refund.amount,
        currency: refund.currency.clone(),
        source: refund.source()?,
        method: attempt.method,
        kind: attempt.failure_kind?,
        reason: attempt.failure_reason.clone()?,
        attempt: RefundAttemptRef {
            id: attempt.id,
            number: attempt.number,
        },
        from,
        outgoing_payment: match from {
            RefundFailedFrom::Paid => attempt.payment_id.map(PaymentId),
            RefundFailedFrom::Approved => None,
        },
    })
}

/// Registers the listener: after a `fail` or `fail_by_system` commits,
/// `RefundFailed` is emitted, then the notices are sent.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Refund>, state: AppState| async move {
        if event.transition != FAIL && event.transition != FAIL_BY_SYSTEM {
            return Ok(());
        }
        // The committed rows.
        let refund = Refund::find_or_404(&state.db, event.key).await?;
        let attempt = RefundAttempt::latest(&state.db, refund.id)
            .await?
            .filter(|attempt| attempt.outcome == AttemptOutcome::Failed)
            .ok_or_else(|| {
                Error::Internal(renox::anyhow::anyhow!(
                    "refund {} failed without a failed attempt",
                    refund.id
                ))
            })?;
        let from = failed_from(&event.from).ok_or_else(|| {
            Error::Internal(renox::anyhow::anyhow!(
                "refund {} failed from {}",
                refund.id,
                event.from
            ))
        })?;
        let failed = failed_event(&refund, &attempt, from).ok_or_else(|| {
            Error::Internal(renox::anyhow::anyhow!(
                "refund {} has an unknown source {} ({})",
                refund.id,
                refund.source_subject(),
                refund.source_kind
            ))
        })?;
        let (kind, reason) = (failed.kind, failed.reason.clone());
        state.emit(failed).await?;
        notices::refund_failed(&state, &refund, kind, &reason).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::{StateMachine, TransitionError};
    use wl_kernel::payments::{PaymentMethod, RefundSource};
    use wl_kernel::{PartyId, Subject};

    fn ctx(kind: Option<RefundFailureKind>, reason: &str, attempt: Option<i64>) -> TransitionCtx {
        let input = FailInput {
            kind,
            reason: reason.to_owned(),
            attempt_id: attempt,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    #[test]
    fn fail_leaves_approved_and_paid_only() {
        let machine = Refund::machine();
        for name in [FAIL, FAIL_BY_SYSTEM] {
            let found = machine.find(name).expect("transition");
            assert!(found.check_from(RefundStatus::Approved).is_ok(), "{name}");
            assert!(found.check_from(RefundStatus::Paid).is_ok(), "{name}");
            for other in [
                RefundStatus::Requested,
                RefundStatus::Failed,
                RefundStatus::Rejected,
                RefundStatus::Cancelled,
            ] {
                assert!(found.check_from(other).is_err(), "{name} from {other:?}");
            }
        }
    }

    #[test]
    fn a_kind_a_reason_and_an_attempt_are_required() {
        let ok = FailInput {
            kind: Some(RefundFailureKind::BankReturn),
            reason: "account closed".into(),
            attempt_id: Some(1),
        };
        assert!(input_problem(Some(&ok), true).is_none());
        assert!(input_problem(None, true).is_some());
        let no_kind = FailInput {
            kind: None,
            ..ok.clone()
        };
        assert!(input_problem(Some(&no_kind), true).is_some());
        for reason in ["", "   "] {
            let blank = FailInput {
                reason: reason.into(),
                ..ok.clone()
            };
            assert!(input_problem(Some(&blank), true).is_some(), "{reason:?}");
        }
        let none = FailInput {
            attempt_id: None,
            ..ok
        };
        assert!(input_problem(Some(&none), true).is_some());
    }

    #[test]
    fn only_the_system_gives_gateway_declined() {
        let declined = FailInput {
            kind: Some(RefundFailureKind::GatewayDeclined),
            reason: "card expired".into(),
            attempt_id: Some(1),
        };
        assert!(input_problem(Some(&declined), true).is_some());
        assert!(input_problem(Some(&declined), false).is_none());
        let card = FailInput {
            kind: Some(RefundFailureKind::CardUnavailable),
            ..declined
        };
        assert!(input_problem(Some(&card), true).is_none());
    }

    #[renox::test]
    async fn the_guards_refuse_a_blank_reason_and_a_chosen_gateway_decline() {
        let machine = Refund::machine();
        let refund = Refund::default();
        let fail = machine.find(FAIL).expect("transition");
        let good = ctx(
            Some(RefundFailureKind::BankReturn),
            "account closed",
            Some(1),
        );
        assert!(fail.check_guards(&refund, &good).await.is_ok());
        for bad in [
            ctx(Some(RefundFailureKind::BankReturn), "  ", Some(1)),
            ctx(
                Some(RefundFailureKind::GatewayDeclined),
                "declined",
                Some(1),
            ),
            ctx(None, "account closed", Some(1)),
            TransitionCtx::new(None),
        ] {
            assert!(matches!(
                fail.check_guards(&refund, &bad).await,
                Err(TransitionError::Guard(_))
            ));
        }
        let system = machine.find(FAIL_BY_SYSTEM).expect("transition");
        let declined = ctx(
            Some(RefundFailureKind::GatewayDeclined),
            "card expired",
            Some(1),
        );
        assert!(system.check_guards(&refund, &declined).await.is_ok());
    }

    #[test]
    fn the_state_it_failed_from_is_approved_or_paid() {
        assert_eq!(failed_from("approved"), Some(RefundFailedFrom::Approved));
        assert_eq!(failed_from("paid"), Some(RefundFailedFrom::Paid));
        assert_eq!(failed_from("requested"), None);
    }

    fn refund() -> Refund {
        Refund {
            id: 5,
            party_id: 2,
            amount: 50_000,
            currency: "AED".into(),
            source_type: "invoicing.credit_note".into(),
            source_id: 7,
            source_kind: "credit_note".into(),
            status: RefundStatus::Failed,
            ..Default::default()
        }
    }

    fn attempt(payment: Option<i64>) -> RefundAttempt {
        RefundAttempt {
            id: 11,
            refund_id: 5,
            number: 1,
            method: PaymentMethod::Gateway,
            payment_id: payment,
            outcome: AttemptOutcome::Failed,
            failure_kind: Some(RefundFailureKind::GatewayDeclined),
            failure_reason: Some("card expired".into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_event_names_the_source_the_attempt_and_the_reason() {
        let event =
            failed_event(&refund(), &attempt(None), RefundFailedFrom::Approved).expect("event");
        assert_eq!(event.party, PartyId(2));
        assert_eq!(event.amount, 50_000);
        assert_eq!(event.method, PaymentMethod::Gateway);
        assert_eq!(event.kind, RefundFailureKind::GatewayDeclined);
        assert_eq!(event.reason, "card expired");
        assert_eq!(event.attempt, RefundAttemptRef { id: 11, number: 1 });
        assert_eq!(event.outgoing_payment, None);
        assert_eq!(
            event.source,
            RefundSource::CreditNote(Subject::new("invoicing.credit_note", 7))
        );
    }

    #[test]
    fn the_outgoing_payment_is_named_only_when_the_refund_was_paid() {
        let paid = failed_event(&refund(), &attempt(Some(9)), RefundFailedFrom::Paid);
        assert_eq!(paid.and_then(|e| e.outgoing_payment), Some(PaymentId(9)));
        let approved = failed_event(&refund(), &attempt(Some(9)), RefundFailedFrom::Approved);
        assert_eq!(approved.and_then(|e| e.outgoing_payment), None);
    }

    #[test]
    fn no_event_for_a_corrupt_source_or_an_attempt_that_did_not_fail() {
        let corrupt = Refund {
            source_kind: "nothing".into(),
            ..refund()
        };
        assert!(failed_event(&corrupt, &attempt(None), RefundFailedFrom::Paid).is_none());
        let open = RefundAttempt {
            failure_kind: None,
            ..attempt(None)
        };
        assert!(failed_event(&refund(), &open, RefundFailedFrom::Paid).is_none());
    }
}
