//! What becomes of the enrolments when a batch ends early: `cancel` before it
//! runs (10 §7.1 *Batch* `cancel`, BL-EDU-13) and `terminate` while it runs
//! (`terminate`, `EDU.ACL.15`, BL-EDU-12).
//!
//! The coordinator names, per enrolment, *transfer to batch X* or *withdraw*
//! (default withdraw). The batch transition checks the choices ([`validate`]),
//! and its effect does everything that is the database's, in the transaction of
//! the transition ([`run_in`]):
//!
//! - the sessions still `scheduled` are cancelled (`cancel_system`), which frees
//!   the tutor and the room and leaves nothing for the tutor's pay: only the
//!   sessions `held` are taught ([`crate::sessions::taught_in`]);
//! - an `active` or `frozen` enrolment with a transfer choice is moved
//!   (`transfer_system`) when the seat in the target is still there at that
//!   moment, taken atomically (`EDU.ACL.17`); when it is gone the enrolment is
//!   withdrawn instead ([`ExitAction::FellBack`]) and the coordinator is told;
//! - every other enrolment is withdrawn (`withdraw_system`), with the refund of
//!   a `cancel` (everything paid, the registration fee included, no deduction,
//!   [`crate::refund::for_cancellation`]) or of a `terminate` (the unused
//!   sessions pro rata, no admin fee, no no-refund tier, the registration fee
//!   not refunded, [`crate::refund::for_termination`]); a `pending_payment`
//!   enrolment is withdrawn with nothing to refund.
//!
//! What it did is written to the batch as its log. After the commit [`finish`]
//! reads it once: the credit notes and refund requests are made
//! ([`crate::settlement::settle`]; each refund waits for its own approval,
//! `FIN.ACL.04`, recorded by the user who asked), students and guardians are
//! told, the coordinator is told of the fallbacks, and for a regulatory
//! termination the centre manager and the owner are told as well.
//!
//! Decisions: what was paid is looked up before the transition (the contracts
//! cannot be asked inside it) and carried in the input; the sessions held are
//! counted inside the transaction at the moment of the approval, so the
//! sessions held while a termination waited count as used.

use renox::chrono::NaiveDate;
use renox::db::{Json, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::batch::{Batch, BatchStatus};
use crate::batch_machine::BatchInput;
use crate::db::{refused, set_batch};
use crate::enrolment::{Enrolment, EnrolmentStatus};
use crate::enrolment_machine::{EnrolmentInput, TRANSFER_SYSTEM, WITHDRAW_SYSTEM};
use crate::enrolments::paid_of;
use crate::notices;
use crate::permissions;
use crate::refund::{self, Refund};
use crate::seats;
use crate::session::{Session, SessionStatus};
use crate::session_machine::{CANCEL_SYSTEM, SessionInput};
use crate::sessions;
use crate::settlement;
use crate::texts::english;

/// How a batch ends early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    /// Before its first session (`cancel`): a full refund.
    Cancel,
    /// While it runs (`terminate`): the unused sessions pro rata.
    Terminate,
}

/// What the coordinator chose for one enrolment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitChoice {
    /// The enrolment.
    pub enrolment_id: i64,
    /// The batch to move the student to; `None` withdraws.
    #[serde(default)]
    pub transfer_to: Option<i64>,
}

impl ExitChoice {
    /// Withdraw enrolment `enrolment_id`.
    pub fn withdraw(enrolment_id: i64) -> Self {
        ExitChoice {
            enrolment_id,
            transfer_to: None,
        }
    }

    /// Move enrolment `enrolment_id` to batch `batch_id`.
    pub fn transfer(enrolment_id: i64, batch_id: i64) -> Self {
        ExitChoice {
            enrolment_id,
            transfer_to: Some(batch_id),
        }
    }
}

/// What an enrolment had paid when the exit was asked, in fils.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitPaid {
    /// The enrolment.
    pub enrolment_id: i64,
    /// What its invoices had received.
    pub paid: i64,
}

/// What happened to one enrolment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitAction {
    /// Withdrawn, as chosen (or by default).
    Withdrawn,
    /// Moved to the batch chosen.
    Transferred,
    /// A transfer was chosen but the seat was gone: withdrawn instead.
    FellBack,
    /// Not touched: it no longer held a seat.
    Skipped,
}

/// The log line of one enrolment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitOutcome {
    /// The enrolment.
    pub enrolment_id: i64,
    /// What happened to it.
    pub action: ExitAction,
    /// The batch it was to move to, when a transfer was chosen.
    #[serde(default)]
    pub target_batch_id: Option<i64>,
    /// The enrolment that replaced it, after a transfer.
    #[serde(default)]
    pub new_enrolment_id: Option<i64>,
    /// The refund of a withdrawal, in fils.
    #[serde(default)]
    pub refund: i64,
}

/// The choices of `choices` made whole: every enrolment of `batch` that holds
/// a seat, with the choice named for it or the default (withdraw), and what each
/// had paid. Refuses a choice for an enrolment that is not in the batch or
/// holds no seat, named twice, or a transfer of a student who has not paid yet
/// or to a batch that cannot take the student.
///
/// # Errors
///
/// A 422 in words; the database's errors and the contracts'.
pub async fn validate(
    state: &AppState,
    batch: &Batch,
    choices: &[ExitChoice],
) -> Result<(Vec<ExitChoice>, Vec<ExitPaid>)> {
    let seated = Enrolment::seated_in(&state.db, batch.id).await?;
    let mut seen: Vec<i64> = Vec::new();
    for choice in choices {
        let Some(found) = seated.iter().find(|e| e.id == choice.enrolment_id) else {
            return Err(refused(english(
                "edtech.exit.problem.not_in_batch",
                &[("enrolment", &choice.enrolment_id)],
            )));
        };
        if seen.contains(&choice.enrolment_id) {
            return Err(refused(english(
                "edtech.exit.problem.twice",
                &[("enrolment", &found.number())],
            )));
        }
        seen.push(choice.enrolment_id);
        if let Some(target) = choice.transfer_to {
            if found.status == EnrolmentStatus::PendingPayment {
                return Err(refused(english(
                    "edtech.exit.problem.unpaid_transfer",
                    &[("enrolment", &found.number())],
                )));
            }
            let other = Batch::find(&state.db, target).await?;
            let ok = other.as_ref().is_some_and(|o| {
                o.id != batch.id
                    && o.course_id == batch.course_id
                    && matches!(o.status, BatchStatus::Open | BatchStatus::Running)
            });
            if !ok {
                return Err(refused(english(
                    "edtech.exit.problem.target",
                    &[("enrolment", &found.number()), ("batch", &target)],
                )));
            }
        }
    }
    let mut plan = Vec::new();
    let mut paid = Vec::new();
    for e in &seated {
        let choice = choices
            .iter()
            .find(|c| c.enrolment_id == e.id)
            .cloned()
            .unwrap_or_else(|| ExitChoice::withdraw(e.id));
        plan.push(choice);
        let amount = if e.status == EnrolmentStatus::PendingPayment {
            0
        } else {
            paid_of(state, e).await?
        };
        paid.push(ExitPaid {
            enrolment_id: e.id,
            paid: amount,
        });
    }
    Ok((plan, paid))
}

/// Cancels the sessions of batch `batch_id` that are still `scheduled`, for
/// `reason`. Returns how many.
async fn cancel_sessions(tx: &mut Transaction, batch_id: i64, reason: &str) -> Result<i64> {
    let open = Session::query()
        .where_(Session::BATCH_ID.eq(batch_id))
        .where_(Session::STATUS.eq(SessionStatus::Scheduled))
        .order_by(Session::ID)
        .get(&mut *tx)
        .await?;
    let mut cancelled = 0;
    for session in open {
        let _outcome = session
            .transition(CANCEL_SYSTEM)
            .by_system()
            .reason(reason)
            .input(&SessionInput::default())
            .run_in(&mut *tx)
            .await
            .map_err(TransitionError::into_error)?;
        cancelled += 1;
    }
    Ok(cancelled)
}

/// The refund of a withdrawal of `e` from batch `batch` as of `on`.
async fn refund_of(
    tx: &mut Transaction,
    e: &Enrolment,
    paid: i64,
    kind: ExitKind,
    on: NaiveDate,
) -> Result<Refund> {
    if e.status == EnrolmentStatus::PendingPayment {
        return Ok(refund::nothing_paid());
    }
    Ok(match kind {
        ExitKind::Cancel => refund::for_cancellation(paid),
        ExitKind::Terminate => {
            let held = match e.batch_id {
                Some(batch) => sessions::held_until(&mut *tx, batch, on).await?,
                None => 0,
            };
            refund::for_termination(paid, e.registration_fee, e.sessions_total, held)
        }
    })
}

/// Withdraws `e` as the system with the refund of `kind`.
async fn withdraw_in(
    tx: &mut Transaction,
    e: &Enrolment,
    paid: i64,
    kind: ExitKind,
    input: &BatchInput,
    reason: &str,
) -> Result<i64> {
    let on = today();
    let refund = refund_of(tx, e, paid, kind, on).await?;
    let amount = refund.refund;
    let run = EnrolmentInput {
        on: Some(on),
        by: input.by,
        amount,
        reason: Some(reason.to_owned()),
        refund: Some(refund),
        ..EnrolmentInput::default()
    };
    let _outcome = e
        .transition(WITHDRAW_SYSTEM)
        .by_system()
        .reason(reason)
        .input(&run)
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(amount)
}

/// Moves `e` to batch `target` as the system; the seat there is taken in the
/// transition's effect, atomically.
async fn transfer_in(
    tx: &mut Transaction,
    e: &Enrolment,
    target: i64,
    paid: i64,
    kind: ExitKind,
    input: &BatchInput,
) -> Result<Option<i64>> {
    let on = today();
    let held = match (kind, e.batch_id) {
        (ExitKind::Terminate, Some(batch)) => sessions::held_until(&mut *tx, batch, on).await?,
        _ => 0,
    };
    // A cancel takes nothing off: all that was paid moves, the registration fee
    // included; a terminate moves what was paid for the sessions not yet used.
    let carried = match kind {
        ExitKind::Cancel => paid,
        ExitKind::Terminate => {
            refund::carried_value(paid, e.registration_fee, e.sessions_total, held)
        }
    };
    let run = EnrolmentInput {
        on: Some(on),
        by: input.by,
        target_batch_id: Some(target),
        target_has_seat: Some(true),
        target_room: false,
        target_sessions: sessions::planned_count(&mut *tx, target).await?,
        carried,
        ..EnrolmentInput::default()
    };
    let _outcome = e
        .transition(TRANSFER_SYSTEM)
        .by_system()
        .input(&run)
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    let moved = Enrolment::find_or_404(&mut *tx, e.id).await?;
    Ok(moved.transferred_to_id)
}

/// Does the exit of `kind` for every enrolment of the input, inside `tx`:
/// returns what happened to each.
async fn apply_in(
    tx: &mut Transaction,
    batch: &Batch,
    input: &BatchInput,
    kind: ExitKind,
    reason: &str,
) -> Result<Vec<ExitOutcome>> {
    let mut log = Vec::new();
    for choice in &input.choices {
        let Some(e) = Enrolment::find(&mut *tx, choice.enrolment_id).await? else {
            continue;
        };
        if e.batch_id != Some(batch.id) || !e.status.takes_a_seat() {
            log.push(ExitOutcome {
                enrolment_id: e.id,
                action: ExitAction::Skipped,
                target_batch_id: choice.transfer_to,
                new_enrolment_id: None,
                refund: 0,
            });
            continue;
        }
        let paid = input
            .paid
            .iter()
            .find(|p| p.enrolment_id == e.id)
            .map_or(0, |p| p.paid);
        let mut action = ExitAction::Withdrawn;
        let mut new_enrolment_id = None;
        let mut refunded = 0;
        if let Some(target) = choice.transfer_to
            && e.status != EnrolmentStatus::PendingPayment
        {
            let attempt = {
                let e = e.clone();
                let input = input.clone();
                tx.savepoint(move |tx| {
                    Box::pin(async move { transfer_in(tx, &e, target, paid, kind, &input).await })
                })
                .await
            };
            match attempt {
                Ok(next) => {
                    action = ExitAction::Transferred;
                    new_enrolment_id = next;
                }
                Err(error) if seats::is_full(&error) => action = ExitAction::FellBack,
                Err(other) => return Err(other),
            }
        }
        if action != ExitAction::Transferred {
            let again = Enrolment::find_or_404(&mut *tx, e.id).await?;
            refunded = withdraw_in(tx, &again, paid, kind, input, reason).await?;
        }
        log.push(ExitOutcome {
            enrolment_id: e.id,
            action,
            target_batch_id: choice.transfer_to,
            new_enrolment_id,
            refund: refunded,
        });
    }
    Ok(log)
}

/// The effect of a batch `cancel` or `terminate`: stores the reason and who
/// asked, cancels the sessions, deals with the enrolments and writes the log.
///
/// # Errors
///
/// The database's errors and the machines'.
pub(crate) async fn run_in(
    tx: &mut Transaction,
    batch: &Batch,
    input: &BatchInput,
    reason: Option<String>,
    kind: ExitKind,
) -> Result {
    let why = reason.clone().unwrap_or_default();
    let column = match kind {
        ExitKind::Cancel => "cancel_reason",
        ExitKind::Terminate => "termination_reason",
    };
    set_batch(
        tx,
        batch.id,
        &[
            (column, &reason),
            ("exit_requested_by", &input.by),
            ("exit_regulatory", &input.regulatory),
        ],
    )
    .await?;
    cancel_sessions(tx, batch.id, &why).await?;
    let log = apply_in(tx, batch, input, kind, &why).await?;
    set_batch(tx, batch.id, &[("exit_log", &Json(log))]).await
}

/// The words for what an enrolment's exit means for the family.
fn family_text(
    kind: ExitKind,
    outcome: &ExitOutcome,
    batch: &str,
    reason: &str,
) -> (String, String) {
    let ended = english(
        match kind {
            ExitKind::Cancel => "edtech.exit.ended_cancel",
            ExitKind::Terminate => "edtech.exit.ended_terminate",
        },
        &[],
    );
    let title = english(
        "edtech.exit.family_title",
        &[("batch", &batch), ("ended", &ended)],
    );
    let what = match outcome.action {
        ExitAction::Transferred => english(
            "edtech.exit.family_moved",
            &[(
                "target",
                &format!("BAT-{:04}", outcome.target_batch_id.unwrap_or_default()),
            )],
        ),
        _ if outcome.refund > 0 => english(
            "edtech.exit.family_refund",
            &[("amount", &crate::money_text(outcome.refund))],
        ),
        _ => english("edtech.exit.family_nothing", &[]),
    };
    let body = english(
        "edtech.exit.family_body",
        &[
            ("batch", &batch),
            ("ended", &ended),
            ("reason", &reason),
            ("what", &what),
        ],
    );
    (title, body)
}

/// After the commit of a batch `cancel` or `terminate`, once: the money
/// documents of the withdrawals are made, the family of each enrolment is told,
/// the coordinator who asked is told of the transfers that fell back to a
/// withdrawal, and for a regulatory termination the centre manager and the owner
/// are told. Answers whether this run did it.
///
/// # Errors
///
/// The database's errors and the notifier's.
pub async fn finish(state: &AppState, batch_id: i64, kind: ExitKind) -> Result<bool> {
    let claimed = Batch::query()
        .where_(Batch::ID.eq(batch_id))
        .where_(Batch::EXIT_FINISHED_AT.is_null())
        .update(&state.db, &[("exit_finished_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let batch = Batch::find_or_404(&state.db, batch_id).await?;
    let number = batch.number();
    let reason = match kind {
        ExitKind::Cancel => batch.cancel_reason.clone(),
        ExitKind::Terminate => batch.termination_reason.clone(),
    }
    .unwrap_or_default();
    let mut fell_back = Vec::new();
    for outcome in &batch.exit_log.0 {
        if outcome.action == ExitAction::Skipped {
            continue;
        }
        let Some(e) = Enrolment::find(&state.db, outcome.enrolment_id).await? else {
            continue;
        };
        if matches!(outcome.action, ExitAction::Withdrawn | ExitAction::FellBack) {
            settlement::settle_quietly(state, e.id).await;
        }
        if outcome.action == ExitAction::FellBack {
            fell_back.push(e.number());
        }
        let (title, body) = family_text(kind, outcome, &number, &reason);
        notices::tell_family(state, &e, &title, &body).await?;
    }
    if !fell_back.is_empty() {
        let who: Vec<i64> = batch.exit_requested_by.into_iter().collect();
        let title = english("edtech.exit.fallback_title", &[("batch", &number)]);
        let list = fell_back.join(", ");
        let body = english("edtech.exit.fallback_body", &[("enrolments", &list)]);
        notices::tell_users(state, batch.subject(), &number, &who, &title, &body).await?;
    }
    if kind == ExitKind::Terminate && batch.exit_regulatory {
        let mut people = notices::holders(state, permissions::CENTRE_MANAGER_ROLE).await?;
        people.extend(notices::holders(state, permissions::OWNER_ROLE).await?);
        let title = english("edtech.exit.regulatory_title", &[("batch", &number)]);
        let body = english(
            "edtech.exit.regulatory_body",
            &[("batch", &number), ("reason", &reason)],
        );
        notices::tell_users(state, batch.subject(), &number, &people, &title, &body).await?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_withdraws_by_default_and_names_its_target_when_it_transfers() {
        assert_eq!(ExitChoice::withdraw(3).transfer_to, None);
        assert_eq!(ExitChoice::transfer(3, 9).transfer_to, Some(9));
    }

    #[test]
    fn the_log_round_trips_through_json() {
        let log = vec![ExitOutcome {
            enrolment_id: 1,
            action: ExitAction::FellBack,
            target_batch_id: Some(2),
            new_enrolment_id: None,
            refund: 147_000,
        }];
        let text = renox::serde_json::to_string(&log).unwrap();
        assert!(text.contains("fell_back"));
        let back: Vec<ExitOutcome> = renox::serde_json::from_str(&text).unwrap();
        assert_eq!(back, log);
    }
}
