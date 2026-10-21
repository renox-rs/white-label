//! Recovering or writing off a cash shortage (wl-payments §4 *Cash handover*
//! `recover`, `write_off`, §5.5 step 3, `FIN.ACL.06`, `FIN.ACL.19`, issue
//! #110).
//!
//! - **`recover`** (short → recovered, `cash.count`) runs when the recoveries
//!   recorded on the handover ([`crate::cash_recovery`]) cover the whole
//!   difference. [`recover`] may record a last cash recovery first, so
//!   "recover with cash AED 50.00" is one call.
//! - **`write_off`** (short → written_off, `cash.write_off`) writes off only
//!   what is left after the recoveries (difference − recoveries); the
//!   recoveries stay shown. The rest is posted as a loss of the holder's
//!   outlet or branch ([`CashPosting`]). Above the limit (`FIN.ACL.06`,
//!   setting `cash_write_off_limit`, AED 0 unless set: 03 sets AED 50) it is an
//!   approval request (`PLT.APR.01`, policy `payments.cash_write_off`, any other
//!   holder of `cash.write_off`); the kernel then runs `write_off_approved`.
//!   At the final approval the rest is checked again: when a recovery or a
//!   recount changed it, the request fails and nothing is written off.
//!
//! A missing cheque is not a cash shortage and is not handled here
//! (BL-PAY-18). After the commit `CashShortageSettled` is emitted.

use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{
    Outcome, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{
    ApprovalRequest, PolicyDefault, register_final_check, submit::REQUEST_KEY,
};
use wl_kernel::custody::{CASH_HANDOVER_SUBJECT_TYPE, CashShortageSettled, HandoverStatus};
use wl_kernel::machines::SubjectMachine;

use crate::cash_handover::{CashHandover, CashPosting, PostingKind};
use crate::cash_recovery::{self, CashRecovery};
use crate::config_of;
use crate::permissions;
use crate::texts::english;

/// `recover`: short → recovered.
pub const RECOVER: &str = "recover";
/// `write_off`: short → written_off, by a user.
pub const WRITE_OFF: &str = "write_off";
/// `write_off`, run by the kernel once the request is approved.
pub const WRITE_OFF_APPROVED: &str = "write_off_approved";

/// The approval policy's action key of a write-off (`FIN.ACL.06`). Not the
/// default `payments.write_off`, which is the cheque's.
pub const ACTION: &str = "payments.cash_write_off";

/// What a write-off runs on; it is also the approval request's payload,
/// which the kernel hands to `write_off_approved`. `amount` is the key the
/// policy's limit reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct WriteOffInput {
    /// What is left of the difference after the recoveries, in fils.
    pub amount: i64,
    /// Why it is written off.
    pub reason: String,
}

fn write_off_input(ctx: &TransitionCtx) -> Option<WriteOffInput> {
    ctx.input_as::<WriteOffInput>().ok()
}

/// Why the recoveries do not cover the shortage of `handover`, if they do
/// not.
pub(crate) fn recover_problem(handover: &CashHandover) -> Option<String> {
    if handover.difference <= 0 {
        return Some("There is no shortage to recover.".to_owned());
    }
    if handover.recovered < handover.difference {
        return Some(format!(
            "The recoveries recorded ({}) do not cover the whole difference ({}): record the rest or write it off.",
            crate::followup::money_text(&handover.currency, handover.recovered),
            crate::followup::money_text(&handover.currency, handover.difference)
        ));
    }
    None
}

/// Why `input` cannot write off `handover`, if it cannot.
pub(crate) fn write_off_problem(input: Option<&WriteOffInput>) -> Option<String> {
    let Some(input) = input else {
        return Some("The write-off was not prepared by the cash service.".to_owned());
    };
    if input.reason.trim().is_empty() {
        return Some("A reason is required.".to_owned());
    }
    if input.amount <= 0 {
        return Some(
            "Nothing is left to write off: record the recovery with `recover`.".to_owned(),
        );
    }
    None
}

/// What both write-off transitions share.
fn writing_off(transition: Transition<CashHandover>) -> Transition<CashHandover> {
    transition
        .from([HandoverStatus::Short])
        .to(HandoverStatus::WrittenOff)
        .guard("reason_and_rest", |_handover: &CashHandover, ctx| {
            let problem = write_off_problem(write_off_input(ctx).as_ref());
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|handover: &CashHandover, ctx, tx| {
            let id = handover.id;
            let location = handover.location.clone();
            let legal_entity_id = handover.legal_entity_id;
            let amount = write_off_input(ctx).map_or(0, |input| input.amount);
            Box::pin(async move {
                CashHandover::query()
                    .where_(CashHandover::ID.eq(id))
                    .update(&mut *tx, &[("written_off", &amount)])
                    .await?;
                // A loss of the holder's outlet or branch.
                let mut posting = CashPosting {
                    handover_id: id,
                    kind: PostingKind::ShortageLoss,
                    amount,
                    location,
                    legal_entity_id,
                    ..Default::default()
                };
                posting.insert(&mut *tx).await?;
                Ok(())
            })
        })
}

/// `recover`, `write_off` and `write_off_approved`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<CashHandover>> {
    vec![
        Transition::new(RECOVER)
            .label("Recover")
            .tone("success")
            .from([HandoverStatus::Short])
            .to(HandoverStatus::Recovered)
            .permission(permissions::CASH_COUNT)
            .guard(
                "recoveries_cover_the_difference",
                |handover: &CashHandover, _ctx| {
                    let problem = recover_problem(handover);
                    Box::pin(async move { problem.map_or(Ok(()), Err) })
                },
            ),
        writing_off(
            Transition::new(WRITE_OFF)
                .label("Write off")
                .tone("danger")
                .permission(permissions::CASH_WRITE_OFF),
        ),
        writing_off(
            Transition::new(WRITE_OFF_APPROVED)
                .label("Write off (approved)")
                .system(),
        ),
    ]
}

async fn reload(
    state: &AppState,
    handover: &CashHandover,
) -> std::result::Result<CashHandover, TransitionError> {
    CashHandover::find_or_404(&state.db, handover.id)
        .await
        .map_err(TransitionError::Failed)
}

/// `user` (`cash.count`) recovers the `short` `handover`: with `cash` fils of
/// cash from the responsible person recorded first, when given. The recoveries
/// must then cover the whole difference.
///
/// # Errors
///
/// `Forbidden` without `cash.count`; `NotFrom` unless `short`; `Guard` (422)
/// when the recoveries do not cover the difference (a recovery recorded here
/// stays recorded), or as [`cash_recovery::record_cash`].
pub async fn recover(
    state: &AppState,
    user: &AuthUser,
    handover: &CashHandover,
    cash: Option<i64>,
) -> std::result::Result<CashHandover, TransitionError> {
    if !user.has_permission(permissions::CASH_COUNT) {
        return Err(TransitionError::Forbidden);
    }
    if user.id == handover.holder_user_id || user.id == handover.responsible_user_id {
        return Err(TransitionError::Guard(
            "A shortage is recovered by someone other than its holder (FIN.ACL.05).".to_owned(),
        ));
    }
    if let Some(amount) = cash {
        cash_recovery::record_cash(state, user, handover, amount, None).await?;
    }
    let handover = reload(state, handover).await?;
    let _outcome = handover.transition(RECOVER).by(user).run(state).await?;
    reload(state, &handover).await
}

/// What a write-off request came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOffRequest {
    /// No approval was needed: the rest is written off.
    WrittenOff,
    /// An approval request (its id) waits for a second user (`FIN.ACL.06`).
    Requested(i64),
}

/// `user` (`cash.write_off`) writes off what is left of the `short`
/// `handover` after its recoveries, for `reason`. Above the limit it waits for
/// the approval of another holder of `cash.write_off`; the request is
/// announced after the commit.
///
/// # Errors
///
/// `Forbidden` without `cash.write_off`; `NotFrom` unless `short`; `Guard`
/// (422) without a reason, with nothing left to write off, or when a request
/// for the handover is already pending.
pub async fn write_off(
    state: &AppState,
    user: &AuthUser,
    handover: &CashHandover,
    reason: &str,
) -> std::result::Result<WriteOffRequest, TransitionError> {
    let handover = reload(state, handover).await?;
    let input = WriteOffInput {
        amount: handover.rest(),
        reason: reason.trim().to_owned(),
    };
    let outcome = handover
        .transition(WRITE_OFF)
        .by(user)
        .reason(reason.trim())
        .input(&input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(match &outcome {
        Outcome::Pending(pending) => match pending.data.get(REQUEST_KEY).and_then(Value::as_i64) {
            Some(id) => WriteOffRequest::Requested(id),
            None => WriteOffRequest::WrittenOff,
        },
        _ => WriteOffRequest::WrittenOff,
    })
}

/// The final re-check of a write-off (`FIN.ACL.06`): what is left of the
/// difference must still be the amount asked for; otherwise the request
/// fails and nothing is written off.
async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    if request.action != ACTION {
        return Ok(None);
    }
    let Some(handover) = CashHandover::find(&mut *tx, request.subject_id).await? else {
        return Ok(None);
    };
    let asked = request
        .payload
        .get("amount")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if handover.rest() != asked {
        return Ok(Some(english("wl_payments.cash.changed").to_owned()));
    }
    Ok(None)
}

/// After the commit of a recovery or a write-off: `CashShortageSettled`.
async fn after_settled(state: &AppState, event: &Transitioned<CashHandover>) -> Result {
    if ![RECOVER, WRITE_OFF, WRITE_OFF_APPROVED].contains(&event.transition.as_str()) {
        return Ok(());
    }
    let handover = CashHandover::find_or_404(&state.db, event.key).await?;
    let recovered = CashRecovery::query()
        .where_(CashRecovery::HANDOVER_ID.eq(handover.id))
        .where_(CashRecovery::STATUS.eq(cash_recovery::RecoveryStatus::Recorded))
        .sum::<i64, _>(&state.db, "amount")
        .await?;
    state
        .emit(CashShortageSettled {
            handover: handover.handover_id(),
            holder_user: handover.holder_user_id,
            status: handover.status,
            recovered,
            written_off: handover.written_off,
        })
        .await
}

/// Registers the machine's approval set-up and the listener.
pub(crate) fn register(app: &mut Registry, write_off_limit: i64) {
    // The kernel runs `write_off_approved` by name on this subject; the
    // policy key is not the cheque's `payments.write_off`.
    SubjectMachine::of::<CashHandover>(CASH_HANDOVER_SUBJECT_TYPE)
        .action(WRITE_OFF, ACTION)
        .register();
    // `FIN.ACL.06`: above the limit another holder of `cash.write_off`
    // approves, never the requester.
    PolicyDefault::new(ACTION)
        .amount_above(write_off_limit)
        .step_permission(permissions::CASH_WRITE_OFF)
        .register();
    register_final_check(CASH_HANDOVER_SUBJECT_TYPE, |tx, request| {
        Box::pin(recheck(tx, request))
    });
    app.listen(
        |event: Transitioned<CashHandover>, state: AppState| async move {
            after_settled(&state, &event).await
        },
    );
}

/// The write-off limit now in force, in fils.
pub fn limit(state: &AppState) -> i64 {
    config_of(state).cash_write_off_limit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short() -> CashHandover {
        CashHandover {
            difference: 5_000,
            recovered: 0,
            status: HandoverStatus::Short,
            currency: "AED".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn recover_needs_the_recoveries_to_cover_the_whole_difference() {
        let mut h = short();
        assert!(recover_problem(&h).is_some());
        h.recovered = 2_000;
        assert!(recover_problem(&h).unwrap().contains("AED 20.00"));
        h.recovered = 5_000;
        assert_eq!(recover_problem(&h), None);
        h.difference = 0;
        assert!(recover_problem(&h).is_some());
    }

    #[test]
    fn a_write_off_needs_a_reason_and_something_left() {
        let ok = WriteOffInput {
            amount: 3_000,
            reason: "Till error".into(),
        };
        assert_eq!(write_off_problem(Some(&ok)), None);
        let blank = WriteOffInput {
            reason: "  ".into(),
            ..ok.clone()
        };
        assert!(write_off_problem(Some(&blank)).is_some());
        let nothing = WriteOffInput { amount: 0, ..ok };
        assert!(write_off_problem(Some(&nothing)).is_some());
        assert!(write_off_problem(None).is_some());
    }

    #[test]
    fn the_write_off_policy_is_not_the_cheques() {
        assert_eq!(ACTION, "payments.cash_write_off");
        assert_ne!(ACTION, "payments.write_off");
    }
}
