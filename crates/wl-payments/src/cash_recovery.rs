//! The recoveries of a cash shortage (wl-payments §4 *Cash handover*, "Part
//! recoveries", `FIN.ACL.19`, `FIN.ACL.05`, issue #110; open item L-20).
//!
//! While a handover is `short`, each part recovered is recorded on it with no
//! change of its state ([`CashRecovery`]); `recover` runs when the recoveries
//! cover the whole difference ([`crate::cash_settle`]).
//!
//! - **Cash** from the responsible person: [`record_cash`] (`cash.count`,
//!   never the holder or the responsible person). It counts at once.
//! - **Salary deduction**, passed to wl-hr as an adjustment of type *cash
//!   shortage recovery* (`FIN.ACL.19`). It is **off by default**: the
//!   `cash_salary_deduction` setting of the client decides, never this code
//!   (`⚠ open: lawyer L-20`). While it is off the option is not offered and
//!   [`request_deduction`] is refused. When on, a deduction is
//!   `requested` by a counter and always goes to an approval by someone
//!   other than the requester (`authorise_deduction`, policy
//!   `payments.authorise_deduction`, any other holder of `cash.write_off`;
//!   `SOD`), then `authorised`, and the responsible person acknowledges it
//!   ([`acknowledge`]): only then is it passed to wl-hr
//!   (`wl_kernel::ShortageRecovery`, idempotent by the recovery's id; the
//!   wl-hr deduction caps are its own) and counted as a recovery. An approval
//!   that is rejected, expires or fails marks the recovery `rejected`.
//!
//! The approval re-checks at the final step that the handover is still
//! `short` and the amount still fits what is left.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{
    ApprovalDecided, ApprovalEnd, ApprovalRequest, PolicyDefault, register_final_check,
    submit::REQUEST_KEY,
};
use wl_kernel::custody::{HandoverStatus, ShortageDeduction, shortage_recovery};
use wl_kernel::machines::SubjectMachine;
use wl_kernel::Subject;

use crate::cash_handover::CashHandover;
use crate::cash_notices;
use crate::config_of;
use crate::followup::money_text;
use crate::permissions;
use crate::texts::english;

/// The subject type of a recovery.
pub const RECOVERY_SUBJECT_TYPE: &str = "payments.cash_recovery";

/// `authorise_deduction`: requested → authorised, by a counter (approval).
pub const AUTHORISE: &str = "authorise_deduction";
/// `authorise_deduction`, run by the kernel once the request is approved.
pub const AUTHORISE_APPROVED: &str = "authorise_deduction_approved";
/// `acknowledge`: authorised → recorded, by the responsible person.
pub const ACKNOWLEDGE: &str = "acknowledge";
/// `reject_deduction`: requested → rejected, by the system.
pub const REJECT: &str = "reject_deduction";

/// The approval policy's action key of a deduction (`FIN.ACL.19`).
pub const ACTION: &str = "payments.authorise_deduction";

/// What a recovery is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecoveryKind {
    /// Cash from the responsible person.
    #[default]
    Cash,
    /// A deduction from wages, passed to wl-hr.
    SalaryDeduction,
}

/// Where a recovery stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecoveryStatus {
    /// Counts as a recovery of the handover.
    #[default]
    Recorded,
    /// A deduction waiting for its approval.
    Requested,
    /// A deduction approved; waiting for the holder's acknowledgement.
    Authorised,
    /// A deduction whose approval was rejected, expired or failed.
    Rejected,
}

/// A recovery of a shortage. Money is `i64` fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cash_recoveries",
    state_machine,
    index(handover_id, status)
)]
pub struct CashRecovery {
    /// The primary key.
    pub id: i64,
    /// The handover.
    pub handover_id: i64,
    /// The person it is taken from.
    pub responsible_user_id: i64,
    /// Cash or salary deduction.
    pub kind: RecoveryKind,
    /// The amount in fils.
    pub amount: i64,
    /// A note.
    pub note: Option<String>,
    /// The user who recorded or requested it.
    pub recorded_by: Option<i64>,
    /// The approval of a deduction.
    pub approval_request_id: Option<i64>,
    /// Who gave the last approval.
    pub approved_by: Option<i64>,
    /// Who acknowledged a deduction.
    pub acknowledged_by: Option<i64>,
    /// When it was acknowledged.
    pub acknowledged_at: Option<DateTime>,
    /// The adjustment wl-hr made.
    pub hr_reference: Option<String>,
    /// Why the approval did not go through.
    pub failure: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: RecoveryStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CashRecovery {
    /// The recovery as a subject.
    pub fn subject(&self) -> Subject {
        Subject::new(RECOVERY_SUBJECT_TYPE, self.id)
    }

    /// The recoveries of `handover_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_handover(db: &Db, handover_id: i64) -> Result<Vec<CashRecovery>> {
        CashRecovery::query()
            .where_(CashRecovery::HANDOVER_ID.eq(handover_id))
            .order_by(CashRecovery::ID)
            .get(db)
            .await
    }

    /// The fils of deductions still waiting (requested or authorised) on
    /// `handover_id`.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn waiting_total(db: &Db, handover_id: i64) -> Result<i64> {
        CashRecovery::query()
            .where_(CashRecovery::HANDOVER_ID.eq(handover_id))
            .where_(CashRecovery::STATUS.is_in([RecoveryStatus::Requested, RecoveryStatus::Authorised]))
            .sum::<i64, _>(db, "amount")
            .await
    }
}

/// What `authorise_deduction` carries: the amount, which the policy reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AuthoriseInput {
    /// The amount in fils.
    pub amount: i64,
}

/// What `acknowledge` carries: the adjustment wl-hr made.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AcknowledgeInput {
    /// The reference of the adjustment.
    pub hr_reference: String,
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

impl StateMachine for CashRecovery {
    type State = RecoveryStatus;
    const COLUMN: renox::db::Col<CashRecovery, RecoveryStatus> = CashRecovery::STATUS;

    fn machine() -> Machine<Self> {
        use RecoveryStatus::{Authorised, Recorded, Rejected, Requested};
        Machine::new()
            .initial([Recorded, Requested])
            .state(Recorded)
            .tone("success")
            .terminal()
            .state(Requested)
            .tone("warning")
            .state(Authorised)
            .tone("info")
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(AUTHORISE)
                    .label("Ask for approval")
                    .from([Requested])
                    .to(Authorised)
                    .permission(permissions::CASH_COUNT),
            )
            .transition(
                Transition::new(AUTHORISE_APPROVED)
                    .label("Approved")
                    .from([Requested])
                    .to(Authorised)
                    .system(),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Rejected")
                    .from([Requested, Authorised])
                    .to(Rejected)
                    .system(),
            )
            .transition(
                Transition::new(ACKNOWLEDGE)
                    .label("Acknowledge")
                    .from([Authorised])
                    .to(Recorded)
                    .guard("responsible_person", |recovery: &CashRecovery, ctx| {
                        let actor = ctx.actor().map(|user| user.id);
                        let mine = actor == Some(recovery.responsible_user_id);
                        let given = ctx
                            .input_as::<AcknowledgeInput>()
                            .is_ok_and(|input| !input.hr_reference.is_empty());
                        Box::pin(async move {
                            if !mine {
                                Err("Only the person the deduction is taken from acknowledges it."
                                    .to_owned())
                            } else if !given {
                                Err("The deduction was not passed to HR.".to_owned())
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|recovery: &CashRecovery, ctx, tx| {
                        let id = recovery.id;
                        let handover_id = recovery.handover_id;
                        let amount = recovery.amount;
                        let actor = ctx.actor().map(|user| user.id);
                        let reference = ctx
                            .input_as::<AcknowledgeInput>()
                            .ok()
                            .map(|input| input.hr_reference);
                        Box::pin(async move {
                            let at = Some(renox::db::now());
                            CashRecovery::query()
                                .where_(CashRecovery::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("acknowledged_by", &actor),
                                        ("acknowledged_at", &at),
                                        ("hr_reference", &reference),
                                    ],
                                )
                                .await?;
                            CashHandover::query()
                                .where_(CashHandover::ID.eq(handover_id))
                                .increment(&mut *tx, "recovered", amount)
                                .await?;
                            Ok(())
                        })
                    }),
            )
    }
}

async fn reload_handover(
    state: &AppState,
    handover: &CashHandover,
) -> std::result::Result<CashHandover, TransitionError> {
    CashHandover::find_or_404(&state.db, handover.id)
        .await
        .map_err(TransitionError::Failed)
}

/// Why `user` cannot record a recovery of `amount` on `handover`, if they
/// cannot: the handover is not `short`, they are the holder or the person it
/// is taken from (`FIN.ACL.05`), or the amount is not above 0 and within what
/// is left of the difference (`room`).
pub(crate) fn recovery_problem(
    handover: &CashHandover,
    user_id: i64,
    amount: i64,
    room: i64,
) -> Option<String> {
    if handover.status != HandoverStatus::Short {
        return Some("Recoveries are recorded on a short handover.".to_owned());
    }
    if user_id == handover.holder_user_id || user_id == handover.responsible_user_id {
        return Some("Cash is handled by someone other than its holder (FIN.ACL.05).".to_owned());
    }
    if amount <= 0 {
        return Some("A recovery is above 0.".to_owned());
    }
    if amount > room {
        return Some(format!(
            "Only {} of the shortage is left to recover.",
            money_text("AED", room.max(0))
        ));
    }
    None
}

/// `user` (`cash.count`) records `amount` fils recovered in cash from the
/// responsible person of the `short` `handover`, with no change of state.
///
/// # Errors
///
/// `Forbidden` without `cash.count`; `Guard` (422) unless the handover is
/// `short`, for the holder or the responsible person, or an amount that is not
/// above 0 and within what is left to recover.
pub async fn record_cash(
    state: &AppState,
    user: &AuthUser,
    handover: &CashHandover,
    amount: i64,
    note: Option<&str>,
) -> std::result::Result<CashRecovery, TransitionError> {
    if !user.has_permission(permissions::CASH_COUNT) {
        return Err(TransitionError::Forbidden);
    }
    let handover = reload_handover(state, handover).await?;
    let waiting = CashRecovery::waiting_total(&state.db, handover.id).await?;
    if let Some(problem) = recovery_problem(&handover, user.id, amount, handover.rest() - waiting) {
        return Err(guard(problem));
    }
    let mut recovery = CashRecovery {
        handover_id: handover.id,
        responsible_user_id: handover.responsible_user_id,
        kind: RecoveryKind::Cash,
        amount,
        note: note.map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned),
        recorded_by: Some(user.id),
        status: RecoveryStatus::Recorded,
        ..Default::default()
    };
    let mut tx = state.db.begin().await?;
    recovery
        .insert_in_state(&mut tx, RecoveryStatus::Recorded, "Cash recovered")
        .await?;
    add_recovered(&mut tx, handover.id, amount).await?;
    tx.commit().await?;
    Ok(recovery)
}

/// Adds `amount` to the recovered total of the `short` handover `id`; 422
/// when it moved on meanwhile.
async fn add_recovered(
    tx: &mut Transaction,
    id: i64,
    amount: i64,
) -> std::result::Result<(), TransitionError> {
    let rows = CashHandover::query()
        .where_(CashHandover::ID.eq(id))
        .where_(CashHandover::STATUS.eq(HandoverStatus::Short))
        .increment(&mut *tx, "recovered", amount)
        .await?;
    if rows == 0 {
        return Err(guard("The handover is no longer short."));
    }
    Ok(())
}

/// What [`request_deduction`] did.
#[derive(Debug, Clone)]
pub struct DeductionRequested {
    /// The recovery, `requested`.
    pub recovery: CashRecovery,
    /// The approval request that waits for another user (`FIN.ACL.19`).
    pub request: Option<i64>,
}

/// `user` (`cash.count`) asks to recover `amount` fils of the `short`
/// `handover` by a salary deduction (`FIN.ACL.19`). Refused while the
/// client's `cash_salary_deduction` setting is off (the default). The
/// approval by another holder of `cash.write_off` and the holder's
/// acknowledgement follow.
///
/// # Errors
///
/// `Forbidden` without `cash.count`; `Guard` (422) when salary deduction is
/// off, and as [`record_cash`].
pub async fn request_deduction(
    state: &AppState,
    user: &AuthUser,
    handover: &CashHandover,
    amount: i64,
    note: Option<&str>,
) -> std::result::Result<DeductionRequested, TransitionError> {
    if !user.has_permission(permissions::CASH_COUNT) {
        return Err(TransitionError::Forbidden);
    }
    if !config_of(state).cash_salary_deduction {
        return Err(guard(english("wl_payments.cash.deduction_off")));
    }
    let handover = reload_handover(state, handover).await?;
    let waiting = CashRecovery::waiting_total(&state.db, handover.id).await?;
    if let Some(problem) = recovery_problem(&handover, user.id, amount, handover.rest() - waiting) {
        return Err(guard(problem));
    }
    let mut recovery = CashRecovery {
        handover_id: handover.id,
        responsible_user_id: handover.responsible_user_id,
        kind: RecoveryKind::SalaryDeduction,
        amount,
        note: note.map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned),
        recorded_by: Some(user.id),
        status: RecoveryStatus::Requested,
        ..Default::default()
    };
    let mut tx = state.db.begin().await?;
    recovery
        .insert_in_state(&mut tx, RecoveryStatus::Requested, "Salary deduction requested")
        .await?;
    tx.commit().await?;
    let outcome = recovery
        .transition(AUTHORISE)
        .by(user)
        .input(&AuthoriseInput { amount })
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    let request = match &outcome {
        renox::state_machine::Outcome::Pending(pending) => pending
            .data
            .get(REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        _ => None,
    };
    if let Some(id) = request {
        CashRecovery::query()
            .where_(CashRecovery::ID.eq(recovery.id))
            .update(&state.db, &[("approval_request_id", &Some(id))])
            .await?;
    }
    let recovery = CashRecovery::find_or_404(&state.db, recovery.id).await?;
    Ok(DeductionRequested { recovery, request })
}

/// The responsible person `user` acknowledges the `authorised` deduction
/// `recovery`: it is passed to wl-hr as an adjustment of type *cash shortage
/// recovery* and counts as a recovery of the handover.
///
/// # Errors
///
/// `NotFrom` unless `authorised`; `Guard` (422) for anyone but the person it is
/// taken from, or when wl-hr cannot carry the deduction out.
pub async fn acknowledge(
    state: &AppState,
    user: &AuthUser,
    recovery: &CashRecovery,
) -> std::result::Result<CashRecovery, TransitionError> {
    let recovery = CashRecovery::find_or_404(&state.db, recovery.id).await?;
    if recovery.status != RecoveryStatus::Authorised {
        return Err(TransitionError::NotFrom(recovery.status.as_str().to_owned()));
    }
    if user.id != recovery.responsible_user_id {
        return Err(guard(
            "Only the person the deduction is taken from acknowledges it.",
        ));
    }
    let handover = CashHandover::find_or_404(&state.db, recovery.handover_id).await?;
    if handover.status != HandoverStatus::Short {
        return Err(guard("The handover is no longer short."));
    }
    let deduction = ShortageDeduction {
        handover: handover.handover_id(),
        recovery: recovery.id,
        responsible_user: recovery.responsible_user_id,
        amount: recovery.amount,
        approved_by: recovery.approved_by,
        acknowledged: true,
        reason: english("wl_payments.cash.deduction_reason").replace(
            "{period}",
            &cash_notices::period_text(&handover),
        ),
    };
    let reference = shortage_recovery(state)
        .deduct(state, deduction)
        .await
        .map_err(|error| match error {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => guard(message),
            other => TransitionError::Failed(other),
        })?;
    let _outcome = recovery
        .transition(ACKNOWLEDGE)
        .by(user)
        .input(&AcknowledgeInput {
            hr_reference: reference,
        })
        .run(state)
        .await?;
    Ok(CashRecovery::find_or_404(&state.db, recovery.id).await?)
}

/// The final re-check of a deduction's approval: the handover must still be
/// `short` with room for the amount.
async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    if request.action != ACTION {
        return Ok(None);
    }
    let Some(recovery) = CashRecovery::find(&mut *tx, request.subject_id).await? else {
        return Ok(None);
    };
    let Some(handover) = CashHandover::find(&mut *tx, recovery.handover_id).await? else {
        return Ok(None);
    };
    if handover.status != HandoverStatus::Short {
        return Ok(Some(english("wl_payments.cash.no_longer_short").to_owned()));
    }
    if recovery.amount > handover.rest() {
        return Ok(Some(english("wl_payments.cash.changed").to_owned()));
    }
    Ok(None)
}

/// An approval about a deduction ended: records who approved, or marks the
/// recovery rejected.
async fn approval_decided(state: &AppState, event: &ApprovalDecided) -> Result {
    if event.subject.subject_type != RECOVERY_SUBJECT_TYPE {
        return Ok(());
    }
    match event.decision {
        ApprovalEnd::Approved => {
            let approver = event.approvers.last().copied();
            CashRecovery::query()
                .where_(CashRecovery::ID.eq(event.subject.subject_id))
                .update(&state.db, &[("approved_by", &approver)])
                .await?;
        }
        ApprovalEnd::Returned => {}
        other => {
            let recovery = CashRecovery::find_or_404(&state.db, event.subject.subject_id).await?;
            if recovery.status != RecoveryStatus::Requested {
                return Ok(());
            }
            let why = format!("{other:?}");
            CashRecovery::query()
                .where_(CashRecovery::ID.eq(recovery.id))
                .update(&state.db, &[("failure", &Some(why.clone()))])
                .await?;
            match recovery
                .transition(REJECT)
                .by_system()
                .reason(&why)
                .run(state)
                .await
            {
                Ok(_) | Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
                Err(error) => return Err(error.into_error()),
            }
            let handover = CashHandover::find_or_404(&state.db, recovery.handover_id).await?;
            if let Some(manager) = cash_notices::manager_of(state, &handover).await? {
                let text = english("wl_payments.cash.deduction_rejected")
                    .replace("{amount}", &money_text(&handover.currency, recovery.amount));
                cash_notices::task(state, &handover, &[manager], &text).await?;
            }
        }
    }
    Ok(())
}

/// Registers the recovery machine, its approval policy and its listener.
pub(crate) fn register(app: &mut Registry) {
    SubjectMachine::of::<CashRecovery>(RECOVERY_SUBJECT_TYPE).register();
    // `FIN.ACL.19`: when salary deduction is on, a deduction always needs the
    // approval of another holder of `cash.write_off`.
    PolicyDefault::new(ACTION)
        .step_permission(permissions::CASH_WRITE_OFF)
        .register();
    register_final_check(RECOVERY_SUBJECT_TYPE, |tx, request| {
        Box::pin(recheck(tx, request))
    });
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        approval_decided(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short() -> CashHandover {
        CashHandover {
            id: 1,
            holder_user_id: 5,
            responsible_user_id: 5,
            expected: 230_000,
            difference: 5_000,
            status: HandoverStatus::Short,
            ..Default::default()
        }
    }

    #[test]
    fn a_recovery_is_recorded_on_a_short_handover_by_someone_else() {
        let h = short();
        assert_eq!(recovery_problem(&h, 6, 2_000, 5_000), None);
        assert!(recovery_problem(&h, 5, 2_000, 5_000).is_some());
        let mut counted = short();
        counted.status = HandoverStatus::Counted;
        assert!(recovery_problem(&counted, 6, 2_000, 5_000).is_some());
    }

    #[test]
    fn a_recovery_is_above_zero_and_within_what_is_left() {
        let h = short();
        assert!(recovery_problem(&h, 6, 0, 5_000).is_some());
        assert!(recovery_problem(&h, 6, 5_001, 5_000).is_some());
        assert_eq!(recovery_problem(&h, 6, 5_000, 5_000), None);
    }

    #[test]
    fn the_machine_has_the_deduction_path() {
        let machine = CashRecovery::machine();
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in ["recorded", "requested", "authorised", "rejected"] {
            assert!(names.contains(&state), "{state}");
        }
        assert_eq!(machine.initial_states(), ["recorded", "requested"]);
    }
}
