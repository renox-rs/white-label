//! Recovering a failed refund (wl-payments §4 `retry`, `change_method` and
//! `cancel`, §5.5a steps 2 and 3, issue #100).
//!
//! A refund that `failed` goes one of three ways:
//!
//! - **`retry`** (`refunds.pay`, or the system, `FIN.RUL.21`,
//!   [`crate::refund_retry`]): back to `approved` with the same method and
//!   details, and for a gateway refund sent to the gateway again at once (a
//!   new attempt `pending`). **At most
//!   [`PaymentsConfig::refund_max_retries`](crate::config::PaymentsConfig) retries** (1 unless set): the manual ones count too, so a refund retried
//!   by the rule is refused another retry. The source must still have room
//!   for the amount (other refunds may have claimed it meanwhile).
//! - **`change_method`** (`refunds.pay`): pay by another method: the original
//!   card through the gateway, a transfer, a cheque or cash. The target is
//!   `approved` when it is paid to the same party by cash, by a cheque in
//!   the party's name or to the original card ([`needs_approval`] is false),
//!   otherwise `requested`: new bank details, or a cheque to someone else,
//!   need a new approval (`FIN.ACL.04`), and the user who set them cannot
//!   give it (`details_by`). Two machine transitions, `change_method` and
//!   `change_method_for_approval`, one for each target; the service picks
//!   and the guards check the pick. The party is told the new method.
//! - **`cancel`** (`refunds.approve`, with a reason: the party cannot be
//!   reached, or asks to keep the credit): the refund ends `cancelled`. A
//!   failed refund does not claim its source, so the amount is already back
//!   on the party's credit balance (a deposit-sourced refund counts in it
//!   too, [`crate::credit`]); the party is told where reachable.
//!
//! Any of the three closes the open `change_method` task of the accountant
//! ([`crate::followup`]).

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{self, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::payments::PaymentMethod;

use crate::config_of;
use crate::credit;
use crate::followup::FollowupTask;
use crate::gateway::gateway_of;
use crate::notices;
use crate::permissions;
use crate::refund::{Refund, RefundStatus};
use crate::refund_attempt::{AttemptOutcome, RefundAttempt};

/// The `retry` transition's name.
pub const RETRY: &str = "retry";

/// The `retry` transition run by the system (`FIN.RUL.21`).
pub const RETRY_BY_SYSTEM: &str = "retry_by_system";

/// The `change_method` transition's name: the new method is paid to the same
/// party, so the refund goes to `approved`.
pub const CHANGE_METHOD: &str = "change_method";

/// `change_method` for a method that needs a new approval: the refund goes
/// to `requested`.
pub const CHANGE_METHOD_FOR_APPROVAL: &str = "change_method_for_approval";

/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// The methods a failed refund can be paid by instead: the original card
/// through the gateway, a transfer, a cheque or cash.
pub const CHANGE_METHODS: [PaymentMethod; 4] = [
    PaymentMethod::Gateway,
    PaymentMethod::Transfer,
    PaymentMethod::Cheque,
    PaymentMethod::Cash,
];

/// What a `retry` run carries to its guards, gathered by the service.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RetryInput {
    /// How many times the refund was retried already.
    pub retries: i64,
    /// The most retries.
    pub max: i64,
    /// Whether the source still has room for the amount.
    pub room: bool,
}

/// What a `change_method` run carries to its guards and effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ChangeInput {
    /// The new method.
    pub method: Option<PaymentMethod>,
    /// The new bank details (a transfer), or the name on a cheque that is not
    /// the party's.
    pub details: Option<String>,
    /// Whether the source still has room for the amount.
    pub room: bool,
}

fn retry_input(ctx: &TransitionCtx) -> Option<RetryInput> {
    ctx.input_as::<RetryInput>().ok()
}

fn change_input(ctx: &TransitionCtx) -> Option<ChangeInput> {
    ctx.input_as::<ChangeInput>().ok()
}

/// Whether paying by `method` with `details` needs a new approval
/// (`FIN.ACL.04`): cash and the original card go to the same party; a cheque
/// does when it is made out to someone else (`details` is that name); a
/// transfer always does, as its bank details are new.
pub fn needs_approval(method: PaymentMethod, details: Option<&str>) -> bool {
    let given = details.is_some_and(|d| !d.trim().is_empty());
    match method {
        PaymentMethod::Cash | PaymentMethod::Gateway => false,
        PaymentMethod::Cheque => given,
        _ => true,
    }
}

/// `details` as stored: trimmed, and none for a method that takes none.
pub(crate) fn details_of(method: PaymentMethod, details: Option<&str>) -> Option<String> {
    if matches!(method, PaymentMethod::Cash | PaymentMethod::Gateway) {
        return None;
    }
    details
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_owned)
}

/// Why `input` cannot change the method of `refund`, if it cannot.
/// `for_approval` says which of the two transitions is run.
pub(crate) fn change_problem(
    refund: &Refund,
    input: Option<&ChangeInput>,
    for_approval: bool,
) -> Option<String> {
    let Some(input) = input else {
        return Some("A method is required.".to_owned());
    };
    let Some(method) = input.method.filter(|m| CHANGE_METHODS.contains(m)) else {
        return Some(
            "Choose the original card through the gateway, a transfer, a cheque or cash."
                .to_owned(),
        );
    };
    let details = details_of(method, input.details.as_deref());
    if method == PaymentMethod::Transfer && details.is_none() {
        return Some("New bank details are required for a transfer.".to_owned());
    }
    if method == PaymentMethod::Gateway
        && refund
            .gateway_txn
            .as_deref()
            .is_none_or(|txn| txn.trim().is_empty())
    {
        return Some("The original card needs the gateway transaction.".to_owned());
    }
    if needs_approval(method, details.as_deref()) != for_approval {
        return Some(
            if for_approval {
                "This method is paid to the same party: it needs no new approval."
            } else {
                "New bank details or another payee need a new approval."
            }
            .to_owned(),
        );
    }
    if refund.method == Some(method) && refund.payee_details == details {
        return Some("Nothing changed: retry the refund instead.".to_owned());
    }
    if !input.room {
        return Some("The source no longer covers the refund.".to_owned());
    }
    None
}

/// Why a refund retried `retries` times cannot be retried again, if it
/// cannot (`max` is the most).
pub(crate) fn retry_problem(refund: &Refund, input: Option<&RetryInput>) -> Option<String> {
    let Some(input) = input else {
        return Some("The action was not prepared by the refund service.".to_owned());
    };
    if refund.method.is_none() {
        return Some("The refund has no method to retry.".to_owned());
    }
    if input.retries >= input.max {
        return Some(format!(
            "A refund is retried at most {} time(s), and this one was retried {} time(s).",
            input.max, input.retries
        ));
    }
    if !input.room {
        return Some("The source no longer covers the refund.".to_owned());
    }
    None
}

/// `retry` and `retry_by_system` share their rules.
fn retrying(transition: Transition<Refund>) -> Transition<Refund> {
    transition
        .from([RefundStatus::Failed])
        .to(RefundStatus::Approved)
        .guard("retry_allowed", |refund: &Refund, ctx| {
            let problem = retry_problem(refund, retry_input(ctx).as_ref());
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
}

/// `change_method` and `change_method_for_approval` share their rules; the
/// refund goes to `to`.
fn changing(
    transition: Transition<Refund>,
    to: RefundStatus,
    for_approval: bool,
) -> Transition<Refund> {
    transition
        .from([RefundStatus::Failed])
        .to(to)
        .guard("method_allowed", move |refund: &Refund, ctx| {
            let problem = change_problem(refund, change_input(ctx).as_ref(), for_approval);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(move |refund: &Refund, ctx, tx| {
            let id = refund.id;
            let input = change_input(ctx);
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a method change without its input"))
                })?;
                let method = input.method.unwrap_or_default();
                let details = details_of(method, input.details.as_deref());
                let now = renox::db::now();
                // Not the status column, so outside the machine's guard.
                Refund::query()
                    .where_(Refund::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("method", &Some(method.as_str().to_owned())),
                            ("payee_details", &details),
                            ("details_by", &by),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                if for_approval {
                    // The earlier approval does not cover the new details.
                    Refund::query()
                        .where_(Refund::ID.eq(id))
                        .update(
                            &mut *tx,
                            &[
                                ("approved_by", &None::<i64>),
                                ("approved_at", &None::<DateTime>),
                            ],
                        )
                        .await?;
                }
                Ok(())
            })
        })
}

/// `retry`, `retry_by_system`, `change_method`,
/// `change_method_for_approval` and `cancel`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Refund>> {
    vec![
        retrying(
            Transition::new(RETRY)
                .label("Retry")
                .tone("primary")
                .permission(permissions::REFUNDS_PAY),
        ),
        retrying(
            Transition::new(RETRY_BY_SYSTEM)
                .label("Retry (system)")
                .system(),
        ),
        changing(
            Transition::new(CHANGE_METHOD)
                .label("Change method")
                .tone("primary")
                .permission(permissions::REFUNDS_PAY),
            RefundStatus::Approved,
            false,
        ),
        changing(
            Transition::new(CHANGE_METHOD_FOR_APPROVAL)
                .label("Change method (for approval)")
                .tone("primary")
                .permission(permissions::REFUNDS_PAY),
            RefundStatus::Requested,
            true,
        ),
        Transition::new(CANCEL)
            .label("Cancel")
            .tone("danger")
            .permission(permissions::REFUNDS_APPROVE)
            .from([RefundStatus::Failed])
            .to(RefundStatus::Cancelled)
            .reason_required()
            .effect(|refund: &Refund, ctx, tx| {
                let id = refund.id;
                let reason = ctx.reason().map(|r| r.trim().to_owned());
                Box::pin(async move {
                    Refund::query()
                        .where_(Refund::ID.eq(id))
                        .update(
                            &mut *tx,
                            &[
                                ("cancel_reason", &reason),
                                ("updated_at", &renox::db::now()),
                            ],
                        )
                        .await?;
                    Ok(())
                })
            }),
    ]
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

async fn reload(state: &AppState, refund: &Refund) -> std::result::Result<Refund, TransitionError> {
    Refund::find_or_404(&state.db, refund.id)
        .await
        .map_err(TransitionError::Failed)
}

/// How many times `refund_id` was retried, by a user or by the system.
///
/// # Errors
///
/// The database's errors.
pub async fn retries_of(db: &Db, refund_id: i64) -> Result<i64> {
    let history = state_machine::history(db, Refund::TABLE, refund_id).await?;
    let retries = history
        .iter()
        .filter(|row| row.transition == RETRY || row.transition == RETRY_BY_SYSTEM)
        .count();
    Ok(i64::try_from(retries).unwrap_or(i64::MAX))
}

/// When the refund `refund_id` last failed, from its history.
///
/// # Errors
///
/// The database's errors.
pub async fn failed_at(db: &Db, refund_id: i64) -> Result<Option<DateTime>> {
    let history = state_machine::history(db, Refund::TABLE, refund_id).await?;
    Ok(history
        .iter()
        .rev()
        .find(|row| row.to_state == RefundStatus::Failed.as_str())
        .map(|row| row.at))
}

/// Retries the failed `refund`, run by `user` (`None`: the system,
/// `FIN.RUL.21`). A gateway refund is sent to the gateway again in the same
/// transaction (a new attempt `pending`), so a gateway that refuses leaves
/// the refund `failed`.
async fn retry_as(
    state: &AppState,
    refund: &Refund,
    user: Option<&AuthUser>,
) -> std::result::Result<Refund, TransitionError> {
    let refund = reload(state, refund).await?;
    if refund.status != RefundStatus::Failed {
        return Err(TransitionError::NotFrom(refund.status.as_str().to_owned()));
    }
    let input = RetryInput {
        retries: retries_of(&state.db, refund.id).await?,
        max: config_of(state).refund_max_retries,
        room: credit::room_for(&state.db, &refund).await?,
    };
    let run = refund.transition(if user.is_some() {
        RETRY
    } else {
        RETRY_BY_SYSTEM
    });
    let run = match user {
        Some(user) => run.by(user),
        None => run.by_system(),
    };
    let mut tx = state.db.begin().await?;
    let outcome = run.input(&input).run_in(&mut tx).await?;
    if refund.method == Some(PaymentMethod::Gateway) {
        let Some(txn) = refund
            .gateway_txn
            .as_deref()
            .map(str::trim)
            .filter(|txn| !txn.is_empty())
        else {
            return Err(guard("A gateway refund needs the gateway transaction."));
        };
        let sent = gateway_of(state)
            .refund(txn, refund.amount)
            .await
            .map_err(TransitionError::Failed)?;
        RefundAttempt::open_in(
            &mut tx,
            refund.id,
            PaymentMethod::Gateway,
            None,
            Some(sent.reference),
            None,
            AttemptOutcome::Pending,
        )
        .await?;
    }
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    FollowupTask::close_for_refund(&state.db, refund.id).await?;
    reload(state, &refund).await
}

/// `user` retries the failed `refund` with the same method and details.
///
/// # Errors
///
/// `Forbidden` without `refunds.pay`; `NotFrom` unless `failed`; `Guard`
/// (422) once the refund was retried the most times, or when the source no
/// longer has room for the amount; `Failed` when the gateway refuses the
/// call. Nothing is written then.
pub async fn retry(
    state: &AppState,
    user: &AuthUser,
    refund: &Refund,
) -> std::result::Result<Refund, TransitionError> {
    if !user.has_permission(permissions::REFUNDS_PAY) {
        return Err(TransitionError::Forbidden);
    }
    retry_as(state, refund, Some(user)).await
}

/// The system retries the failed `refund` (`FIN.RUL.21`).
///
/// # Errors
///
/// As [`retry`], without the permission check.
pub(crate) async fn retry_by_system(
    state: &AppState,
    refund: &Refund,
) -> std::result::Result<Refund, TransitionError> {
    retry_as(state, refund, None).await
}

/// `user` pays the failed `refund` another way: `method` (the original card
/// through the gateway, a transfer, a cheque or cash) with `details` (the new
/// bank details of a transfer, or the name on a cheque that is not the
/// party's). The refund goes to `approved` when [`needs_approval`] is false,
/// else to `requested`. The party is told the new method.
///
/// # Errors
///
/// `Forbidden` without `refunds.pay`; `NotFrom` unless `failed`; `Guard`
/// (422) for another method, a transfer without bank details, a gateway
/// refund without its transaction, nothing changed, or a source with no room
/// for the amount.
pub async fn change_method(
    state: &AppState,
    user: &AuthUser,
    refund: &Refund,
    method: PaymentMethod,
    details: Option<&str>,
) -> std::result::Result<Refund, TransitionError> {
    if !user.has_permission(permissions::REFUNDS_PAY) {
        return Err(TransitionError::Forbidden);
    }
    let refund = reload(state, refund).await?;
    if refund.status != RefundStatus::Failed {
        return Err(TransitionError::NotFrom(refund.status.as_str().to_owned()));
    }
    let details = details_of(method, details);
    let for_approval = needs_approval(method, details.as_deref());
    let input = ChangeInput {
        method: Some(method),
        details,
        room: credit::room_for(&state.db, &refund).await?,
    };
    let _outcome = refund
        .transition(if for_approval {
            CHANGE_METHOD_FOR_APPROVAL
        } else {
            CHANGE_METHOD
        })
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    FollowupTask::close_for_refund(&state.db, refund.id).await?;
    let changed = reload(state, &refund).await?;
    notices::method_changed(state, &changed, method)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(changed)
}

/// `user` cancels the failed `refund` for `reason`: the amount stays on the
/// party's credit balance and the party is told where reachable.
///
/// # Errors
///
/// `Forbidden` without `refunds.approve`; `NotFrom` unless `failed`;
/// `ReasonRequired` without a reason.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    refund: &Refund,
    reason: &str,
) -> std::result::Result<Refund, TransitionError> {
    let mut run = refund.transition(CANCEL).by(user);
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let _outcome = run.run(state).await?;
    FollowupTask::close_for_refund(&state.db, refund.id).await?;
    let cancelled = reload(state, refund).await?;
    notices::cancelled(state, &cancelled)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refund(method: Option<PaymentMethod>) -> Refund {
        Refund {
            id: 5,
            method,
            gateway_txn: Some("txn_1".into()),
            status: RefundStatus::Failed,
            ..Default::default()
        }
    }

    fn change(method: PaymentMethod, details: Option<&str>) -> ChangeInput {
        ChangeInput {
            method: Some(method),
            details: details.map(str::to_owned),
            room: true,
        }
    }

    #[test]
    fn cash_the_card_and_a_cheque_in_the_partys_name_need_no_new_approval() {
        assert!(!needs_approval(PaymentMethod::Cash, None));
        assert!(!needs_approval(PaymentMethod::Gateway, None));
        assert!(!needs_approval(PaymentMethod::Cheque, None));
        assert!(!needs_approval(PaymentMethod::Cheque, Some("  ")));
        assert!(needs_approval(
            PaymentMethod::Cheque,
            Some("Another Trading LLC")
        ));
        assert!(needs_approval(
            PaymentMethod::Transfer,
            Some("AE07 0331 2345 6789 0123 456")
        ));
        assert!(needs_approval(PaymentMethod::Transfer, None));
    }

    #[test]
    fn cash_and_the_card_take_no_details() {
        assert_eq!(details_of(PaymentMethod::Cash, Some("x")), None);
        assert_eq!(details_of(PaymentMethod::Gateway, Some("x")), None);
        assert_eq!(
            details_of(PaymentMethod::Transfer, Some("  AE07  ")).as_deref(),
            Some("AE07")
        );
        assert_eq!(details_of(PaymentMethod::Cheque, Some(" ")), None);
    }

    #[test]
    fn a_method_change_is_checked_against_the_transition_it_runs_in() {
        let failed = refund(Some(PaymentMethod::Gateway));
        // Cheque in the party's name: no approval.
        let cheque = change(PaymentMethod::Cheque, None);
        assert_eq!(change_problem(&failed, Some(&cheque), false), None);
        assert!(change_problem(&failed, Some(&cheque), true).is_some());
        // A new IBAN: for approval only.
        let iban = change(PaymentMethod::Transfer, Some("AE07 0331"));
        assert_eq!(change_problem(&failed, Some(&iban), true), None);
        assert!(change_problem(&failed, Some(&iban), false).is_some());
    }

    #[test]
    fn a_transfer_needs_details_and_the_card_needs_its_transaction() {
        let failed = refund(Some(PaymentMethod::Cheque));
        let bare = change(PaymentMethod::Transfer, None);
        assert!(
            change_problem(&failed, Some(&bare), true).is_some_and(|m| m.contains("bank details"))
        );
        let card = change(PaymentMethod::Gateway, None);
        assert_eq!(change_problem(&failed, Some(&card), false), None);
        let no_txn = Refund {
            gateway_txn: None,
            ..failed.clone()
        };
        assert!(
            change_problem(&no_txn, Some(&card), false)
                .is_some_and(|m| m.contains("gateway transaction"))
        );
    }

    #[test]
    fn only_the_four_methods_and_a_real_change_are_accepted() {
        let failed = refund(Some(PaymentMethod::Transfer));
        for method in [
            PaymentMethod::Card,
            PaymentMethod::SetOff,
            PaymentMethod::Wallet,
        ] {
            assert!(change_problem(&failed, Some(&change(method, None)), true).is_some());
        }
        assert!(change_problem(&failed, None, false).is_some());
        assert!(change_problem(&failed, Some(&ChangeInput::default()), false).is_some());
        // The same method and details is a retry, not a change.
        let same = Refund {
            payee_details: Some("AE07".into()),
            ..failed.clone()
        };
        assert!(
            change_problem(
                &same,
                Some(&change(PaymentMethod::Transfer, Some("AE07"))),
                true
            )
            .is_some_and(|m| m.contains("Nothing changed"))
        );
        assert_eq!(
            change_problem(
                &same,
                Some(&change(PaymentMethod::Transfer, Some("AE99"))),
                true
            ),
            None
        );
    }

    #[test]
    fn a_source_without_room_refuses_the_change() {
        let failed = refund(Some(PaymentMethod::Gateway));
        let mut cash = change(PaymentMethod::Cash, None);
        cash.room = false;
        assert!(
            change_problem(&failed, Some(&cash), false)
                .is_some_and(|m| m.contains("no longer covers"))
        );
    }

    #[test]
    fn a_refund_is_retried_below_the_maximum_only() {
        let failed = refund(Some(PaymentMethod::Gateway));
        let input = |retries| RetryInput {
            retries,
            max: 1,
            room: true,
        };
        assert_eq!(retry_problem(&failed, Some(&input(0))), None);
        assert!(retry_problem(&failed, Some(&input(1))).is_some());
        assert!(retry_problem(&failed, Some(&input(2))).is_some());
        assert!(retry_problem(&failed, None).is_some());
        assert!(retry_problem(&refund(None), Some(&input(0))).is_some());
        let no_room = RetryInput {
            room: false,
            ..input(0)
        };
        assert!(retry_problem(&failed, Some(&no_room)).is_some());
    }

    #[test]
    fn the_machine_has_the_recovery_transitions() {
        let machine = Refund::machine();
        let expected = [
            (RETRY, "approved", false),
            (RETRY_BY_SYSTEM, "approved", true),
            (CHANGE_METHOD, "approved", false),
            (CHANGE_METHOD_FOR_APPROVAL, "requested", false),
            (CANCEL, "cancelled", false),
        ];
        for (name, to, system) in expected {
            let found = machine.find(name).expect(name);
            assert_eq!(found.from_states(), ["failed"], "{name}");
            assert_eq!(found.to_state(), Some(to), "{name}");
            assert_eq!(found.is_system(), system, "{name}");
        }
        assert!(machine.find(CANCEL).expect("cancel").needs_reason());
        assert!(!machine.find(RETRY).expect("retry").needs_reason());
    }

    #[renox::test]
    async fn the_guards_refuse_a_run_the_service_did_not_prepare() {
        let machine = Refund::machine();
        let failed = refund(Some(PaymentMethod::Gateway));
        for name in [
            RETRY,
            RETRY_BY_SYSTEM,
            CHANGE_METHOD,
            CHANGE_METHOD_FOR_APPROVAL,
        ] {
            let found = machine.find(name).expect(name);
            assert!(matches!(
                found.check_guards(&failed, &TransitionCtx::new(None)).await,
                Err(TransitionError::Guard(_))
            ));
        }
    }
}
