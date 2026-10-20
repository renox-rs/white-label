//! The review of attendance (wl-attendance §4 `accept`, `correct`, `reject`
//! and `lock`; `HRM.ACL.09`, BL-ATT-05).
//!
//! A record flagged `overtime_cap`, `permit_invalid` or `overlap` is held for
//! a reviewer. `accept` and `correct` on such a record carry the rule of
//! `HRM.ACL.09`, kept in one place ([`held_rules_in`]) and run inside the
//! transition, so nothing that moves the status can skip it:
//!
//! - **A reason.** The reviewer gives one reason; it covers every flag of the
//!   record and is kept in the history of the transition.
//! - **Not the marker.** The user who recorded the record, and the employee
//!   the record is about, cannot accept or correct it. Admins are not
//!   exempt.
//! - **No overlap left.** A record with an open `overlap` flag is accepted
//!   or corrected only when its hours (as corrected) no longer overlap any
//!   other record of the employee that is not `rejected`; the refusal names
//!   the other record's location and times.
//!
//! Flags that hold nothing (`midday_break`, `outside_geofence`...) need no
//! reason and no other person: the marker may accept them when they hold
//! `attendance.review`.
//!
//! `accept` and `correct` settle the open flags ([`crate::flags::settle_in`]);
//! the flag rows stay as history. The system accepts a record only when it
//! has no open flag at all (`accept_by_system`), never one held for review.
//!
//! Decisions:
//!
//! - The permissions are checked in the scope of the work location (a
//!   reviewer of one project), so the transitions carry no global permission
//!   of their own; the services here check it and answer 403.
//! - `correct` needs `attendance.correct`; on a record with a held flag it
//!   needs `attendance.review` as well, because it then does what `accept`
//!   does.
//! - `accept_by_system` is available for the check-out and the period-end
//!   collection to call ([`accept_by_system`]); the check-out does not call
//!   it by itself yet: a closed record stays `recorded` until the timesheet's
//!   collection (BL-ATT-06) or a reviewer takes it.

use renox::db::{DateTime, Json, Transaction, now};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::serde_json::json;
use renox::state_machine::TransitionCtx;
use serde::{Deserialize, Serialize};
use wl_kernel::BusinessCalendar;
use wl_kernel::documents::dubai_date;
use wl_kernel::workforce::workforce;

use crate::access;
use crate::checks;
use crate::entry;
use crate::flags::{self, AttendanceFlag, FlagType, Pending};
use crate::location::WorkLocation;
use crate::lock;
use crate::overlap;
use crate::permissions;
use crate::record::{
    ACCEPT, ACCEPT_BY_SYSTEM, AttendanceRecord, BreakSpan, CORRECT, LOCK, REJECT,
};
use crate::service::{self, refuse};
use crate::texts::english;

/// What the service tells the transition about the person who acts: the
/// employee behind the user, which only the workforce knows.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct ReviewInput {
    /// The employee whose account is the acting user, if any.
    pub actor_employee: Option<i64>,
}

/// The input of `correct`: who acts and the corrected values.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CorrectInput {
    /// The employee whose account is the acting user, if any.
    pub actor_employee: Option<i64>,
    /// The corrected check-in.
    pub check_in: DateTime,
    /// The corrected check-out.
    pub check_out: Option<DateTime>,
    /// The corrected breaks; the breaks as they are when none.
    pub breaks: Option<Vec<BreakSpan>>,
}

/// The new values of a correction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    /// The corrected check-in.
    pub check_in: DateTime,
    /// The corrected check-out.
    pub check_out: Option<DateTime>,
    /// The corrected breaks; the breaks as they are when none.
    pub breaks: Option<Vec<BreakSpan>>,
}

/// Who acts, read from the transition.
#[derive(Debug, Clone, Default)]
pub(crate) struct Who {
    /// The acting user.
    pub actor: Option<i64>,
    /// The employee behind the acting user.
    pub actor_employee: Option<i64>,
    /// The reason given.
    pub reason: Option<String>,
}

impl Who {
    /// Reads the actor, the reason and the input of a transition.
    pub(crate) fn of(ctx: &TransitionCtx) -> Who {
        let input = ctx.input_as::<ReviewInput>().unwrap_or_default();
        let corrected = ctx.input_as::<CorrectInput>().ok();
        Who {
            actor: ctx.actor().map(|user| user.id),
            actor_employee: corrected
                .as_ref()
                .map_or(input.actor_employee, |c| c.actor_employee),
            reason: ctx
                .reason()
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty()),
        }
    }
}

/// The record with the values of a correction, to compare spans.
fn as_corrected(record: &AttendanceRecord, input: &CorrectInput) -> AttendanceRecord {
    AttendanceRecord {
        check_in: input.check_in,
        check_out: input.check_out,
        date: dubai_date(input.check_in),
        ..record.clone()
    }
}

/// `HRM.ACL.09` for `record` inside `tx`, for an `accept` or a `correct`
/// with the corrected values `corrected`: a reason, not the marker, and an
/// `overlap` only when the hours no longer overlap.
///
/// # Errors
///
/// `422` naming what is missing.
pub(crate) async fn held_rules_in(
    tx: &mut Transaction,
    record: &AttendanceRecord,
    who: &Who,
    corrected: Option<&AttendanceRecord>,
) -> Result {
    let open = AttendanceFlag::query()
        .where_(AttendanceFlag::RECORD_ID.eq(record.id))
        .where_(AttendanceFlag::RESOLVED_AT.is_null())
        .get(&mut *tx)
        .await?;
    let held: Vec<&AttendanceFlag> = open
        .iter()
        .filter(|f| f.flag_type.holds_for_review())
        .collect();
    if held.is_empty() {
        return Ok(());
    }
    let marker = who.actor.is_some() && record.recorded_by == who.actor;
    let own = who.actor_employee == Some(record.employee_id);
    if marker || own {
        return Err(refuse(english("wl_attendance.review.not_the_marker", &[])));
    }
    if who.reason.is_none() {
        return Err(refuse(english("wl_attendance.review.reason_required", &[])));
    }
    if held.iter().any(|f| f.flag_type == FlagType::Overlap) {
        let view = corrected.unwrap_or(record);
        let span = view.span(&BusinessCalendar::default(), now());
        if let Some(note) = overlap::first_clash_in(tx, view, span).await? {
            return Err(refuse(english(
                "wl_attendance.review.overlap_remains",
                &[("note", &note)],
            )));
        }
    }
    Ok(())
}

/// The effect of `accept`: the rule, then the flags are settled.
pub(crate) async fn accept_in(
    tx: &mut Transaction,
    record: &AttendanceRecord,
    who: &Who,
) -> Result {
    held_rules_in(tx, record, who, None).await?;
    let reason = who.reason.clone().unwrap_or_default();
    flags::settle_in(tx, record.id, who.actor, &reason).await?;
    // The hours are proposed on the open timesheet (non-billable for a
    // `permit_invalid` record, which the settled flag still marks).
    entry::propose_in(tx, record).await
}

/// The effect of `accept_by_system`: refused while the record has an open
/// flag of any kind.
pub(crate) async fn accept_by_system_in(tx: &mut Transaction, record: &AttendanceRecord) -> Result {
    let open = AttendanceFlag::query()
        .where_(AttendanceFlag::RECORD_ID.eq(record.id))
        .where_(AttendanceFlag::RESOLVED_AT.is_null())
        .count(&mut *tx)
        .await?;
    if open > 0 {
        return Err(refuse(english("wl_attendance.review.system_flagged", &[])));
    }
    entry::propose_in(tx, record).await
}

/// The effect of `correct`: the rule on the corrected values, the values
/// written (the originals stay in the history of the transition), the flags
/// settled and `manual_correction` noted.
pub(crate) async fn correct_in(
    tx: &mut Transaction,
    record: &AttendanceRecord,
    who: &Who,
    input: &CorrectInput,
) -> Result {
    let view = as_corrected(record, input);
    held_rules_in(tx, record, who, Some(&view)).await?;
    let breaks = Json(
        input
            .breaks
            .clone()
            .unwrap_or_else(|| record.breaks.to_vec()),
    );
    AttendanceRecord::query()
        .where_(AttendanceRecord::ID.eq(record.id))
        .update(
            &mut *tx,
            &[
                ("check_in", &input.check_in),
                ("check_out", &input.check_out),
                ("date", &view.date),
                ("breaks", &breaks),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let reason = who.reason.clone().unwrap_or_default();
    flags::settle_in(tx, record.id, who.actor, &reason).await?;
    let note = english("wl_attendance.review.corrected_note", &[("reason", &reason)]);
    flags::note_in(tx, record.id, FlagType::ManualCorrection, &note, who.actor).await?;
    // The hours proposed follow the corrected times.
    entry::propose_in(tx, &AttendanceRecord { breaks, ..view }).await
}

/// The effect of `reject`: the flags are settled with the reason.
pub(crate) async fn reject_in(
    tx: &mut Transaction,
    record: &AttendanceRecord,
    who: &Who,
) -> Result {
    let reason = who.reason.clone().unwrap_or_default();
    flags::settle_in(tx, record.id, who.actor, &reason).await?;
    // The hours proposed from it are taken off the open timesheet.
    entry::unpropose_in(tx, record.id).await
}

/// The location of `record`, or `NotFound`.
async fn location_of(state: &AppState, record: &AttendanceRecord) -> Result<WorkLocation> {
    WorkLocation::find(&state.db, record.location_id)
        .await?
        .ok_or(Error::NotFound)
}

/// The employee whose account is `user`.
async fn employee_behind(state: &AppState, user: &AuthUser) -> Result<Option<i64>> {
    Ok(workforce(state)
        .employee_of_user(&state.db, user.id)
        .await?
        .map(|employee| employee.0))
}

/// `actor` accepts record `record_id` (`attendance.review` at its location),
/// with `reason`. See the module docs for `HRM.ACL.09`.
///
/// # Errors
///
/// `NotFound`; `Forbidden` without the permission; `422` for a record that
/// is not `recorded` or `flagged`, or when `HRM.ACL.09` refuses.
pub async fn accept(
    state: &AppState,
    actor: &AuthUser,
    record_id: i64,
    reason: &str,
) -> Result<AttendanceRecord> {
    let found = AttendanceRecord::find_or_404(&state.db, record_id).await?;
    let location = location_of(state, &found).await?;
    if !access::holds(state, actor.id, permissions::REVIEW, &location).await? {
        return Err(Error::Forbidden);
    }
    let input = ReviewInput {
        actor_employee: employee_behind(state, actor).await?,
    };
    let _lock = lock::take(found.employee_id, found.date).await;
    let mut tx = state.db.begin().await?;
    let record = AttendanceRecord::find(&mut tx, record_id)
        .await?
        .ok_or(Error::NotFound)?;
    let mut run = record.transition(ACCEPT).by(actor).input(&input);
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let outcome = run
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error())?;
    tx.commit().await?;
    outcome.emit(state).await?;
    AttendanceRecord::find_or_404(&state.db, record_id).await
}

/// The system accepts record `record_id`, which the check-out found without
/// a flag.
///
/// # Errors
///
/// `NotFound`; `422` for a record that is not `recorded`, or that has an open
/// flag, whichever it is: a record held for review is never accepted by the
/// system.
pub async fn accept_by_system(state: &AppState, record_id: i64) -> Result<AttendanceRecord> {
    let found = AttendanceRecord::find_or_404(&state.db, record_id).await?;
    let _lock = lock::take(found.employee_id, found.date).await;
    let mut tx = state.db.begin().await?;
    let record = AttendanceRecord::find(&mut tx, record_id)
        .await?
        .ok_or(Error::NotFound)?;
    let outcome = record
        .transition(ACCEPT_BY_SYSTEM)
        .by_system()
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error())?;
    tx.commit().await?;
    outcome.emit(state).await?;
    AttendanceRecord::find_or_404(&state.db, record_id).await
}

/// `actor` rejects record `record_id` (`attendance.review` at its location),
/// with a `reason`.
///
/// # Errors
///
/// `NotFound`; `Forbidden` without the permission; `422` without a reason or
/// for a record that is not `recorded` or `flagged`.
pub async fn reject(
    state: &AppState,
    actor: &AuthUser,
    record_id: i64,
    reason: &str,
) -> Result<AttendanceRecord> {
    let found = AttendanceRecord::find_or_404(&state.db, record_id).await?;
    let location = location_of(state, &found).await?;
    if !access::holds(state, actor.id, permissions::REVIEW, &location).await? {
        return Err(Error::Forbidden);
    }
    let _lock = lock::take(found.employee_id, found.date).await;
    let mut tx = state.db.begin().await?;
    let record = AttendanceRecord::find(&mut tx, record_id)
        .await?
        .ok_or(Error::NotFound)?;
    let mut run = record.transition(REJECT).by(actor);
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let outcome = run
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error())?;
    tx.commit().await?;
    outcome.emit(state).await?;
    AttendanceRecord::find_or_404(&state.db, record_id).await
}

/// `actor` corrects record `record_id` to `change` (`attendance.correct` at
/// its location, and `attendance.review` when a flag holds the record), with
/// a `reason`, within the correction window. The overlap check, the overtime
/// cap, the midday break and the permit check run again on the corrected
/// values: a correction that creates an overlap flags both records.
///
/// # Errors
///
/// `NotFound`; `Forbidden` without the permissions; `422` without a reason,
/// outside the window, for a check-out that is not after the check-in, for a
/// record that is `rejected` or `locked`, or when `HRM.ACL.09` refuses.
pub async fn correct(
    state: &AppState,
    actor: &AuthUser,
    record_id: i64,
    change: Correction,
    reason: &str,
) -> Result<AttendanceRecord> {
    let found = AttendanceRecord::find_or_404(&state.db, record_id).await?;
    let location = location_of(state, &found).await?;
    if !access::holds(state, actor.id, permissions::CORRECT, &location).await? {
        return Err(Error::Forbidden);
    }
    let held = AttendanceFlag::open_of_record(&state.db, record_id)
        .await?
        .iter()
        .any(|f| f.flag_type.holds_for_review());
    if held && !access::holds(state, actor.id, permissions::REVIEW, &location).await? {
        return Err(Error::Forbidden);
    }
    if change.check_out.is_some_and(|out| out <= change.check_in) {
        return Err(refuse(english("wl_attendance.record.times", &[])));
    }
    service::ensure_in_window(state, found.date)?;
    service::ensure_in_window(state, dubai_date(change.check_in))?;
    let input = CorrectInput {
        actor_employee: employee_behind(state, actor).await?,
        check_in: change.check_in,
        check_out: change.check_out,
        breaks: change.breaks.clone(),
    };
    let new_date = dubai_date(change.check_in);
    let prepared = checks::prepare(state, found.employee(), &location, new_date).await?;
    let _lock = lock::take(found.employee_id, found.date).await;
    let _new_day = if new_date == found.date {
        None
    } else {
        Some(lock::take(found.employee_id, new_date).await)
    };
    let mut tx = state.db.begin().await?;
    let record = AttendanceRecord::find(&mut tx, record_id)
        .await?
        .ok_or(Error::NotFound)?;
    let original = json!({
        "check_in": record.check_in,
        "check_out": record.check_out,
        "breaks": record.breaks.to_vec(),
    });
    let corrected = json!({
        "check_in": change.check_in,
        "check_out": change.check_out,
        "breaks": change.breaks,
    });
    let mut run = record
        .transition(CORRECT)
        .by(actor)
        .input(&input)
        .meta(json!({ "original": original, "corrected": corrected }));
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let outcome = run
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error())?;
    let stored = AttendanceRecord::find(&mut tx, record_id)
        .await?
        .ok_or(Error::NotFound)?;
    let mut pending = Pending::default();
    checks::run_in(state, &mut tx, &stored, &location, &prepared, &mut pending).await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    pending.finish(state).await?;
    let stored = AttendanceRecord::find_or_404(&state.db, record_id).await?;
    service::settle(state, &stored).await?;
    AttendanceRecord::find_or_404(&state.db, record_id).await
}

/// The system locks the `accepted` record `record_id` inside `tx`: the
/// timesheet that used it was approved. The caller emits the outcome after
/// the commit.
///
/// # Errors
///
/// `NotFound`; `422` for a record that is not `accepted`.
pub async fn lock_in(
    tx: &mut Transaction,
    record_id: i64,
) -> Result<renox::state_machine::Outcome<AttendanceRecord>> {
    let record = AttendanceRecord::find(&mut *tx, record_id)
        .await?
        .ok_or(Error::NotFound)?;
    record
        .transition(LOCK)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(|e| e.into_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_of_a_review_defaults_to_nobody() {
        assert_eq!(ReviewInput::default().actor_employee, None);
    }
}
