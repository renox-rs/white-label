//! The session machine (10 §7.1 *Session*; `EDU.BAT.01`, `EDU.ATT.01`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`RESCHEDULE`] | scheduled | scheduled | `batches.manage` | `new_time_given` |
//! | [`CHANGE_TUTOR`] | scheduled | scheduled | `batches.manage` | `new_tutor_given` |
//! | [`SUBMIT_ATTENDANCE`] | scheduled | held | `sessions.teach` | `start_passed`, `every_student_marked` |
//! | [`CORRECT_ATTENDANCE`] | held | held | `sessions.teach` | `within_window` (slot, BL-EDU-14) |
//! | [`CANCEL`] / [`CANCEL_SYSTEM`] | scheduled | cancelled | `batches.manage` / system (a batch `cancel` or `terminate`) | a reason |
//!
//! Decisions:
//!
//! - As for the batch ([`crate::batch_machine`]), the guards see only the
//!   record and the run's input; [`crate::sessions`] checks that the tutor and
//!   the room are free at the new time before it runs `reschedule` and
//!   `change_tutor`, and BL-EDU-08 adds the document and qualification checks
//!   (`EDU.ACL.14`) there.
//! - A change of tutor of the whole batch ([`crate::batch_machine`]) moves the
//!   sessions of the old tutor in its own effect: the sessions keep their
//!   state, so no session run is needed. `change_tutor` here is the change of
//!   one session.
//! - `submit_attendance` waits for the session to have started (the clock of
//!   the app, so a test can travel). *Every student marked* is the service's
//!   answer ([`crate::roster::prepare`]) carried in the run's input: the roster is
//!   read from the database, which a guard cannot do. The effect stores the marks
//!   of the whole roster, with the credit each costs by the absence policy and
//!   the make-up right of each excused absence ([`crate::attendance`]); the
//!   credits are taken from the packages after the commit ([`crate::credits`]) and
//!   the absences evaluated (`EDU.RUL.02`, [`crate::absences`]). That a tutor marks
//!   only the attendance of the batches assigned to them (`EDU.ACL.03`) needs the
//!   tutor-to-user link of wl-hr and is not checked here.
//! - `correct_attendance` is declared with its reason and the slot of
//!   `EDU.ACL.04` and `EDU.ACL.16` (the tutor within 24 hours, then the
//!   coordinator); BL-EDU-14 gives it its body.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_date;

use crate::attendance::{self, MarkInput};
use crate::flags;
use crate::permissions;
use crate::session::{Session, SessionStatus};
use crate::sessions::Except;
use crate::texts::english;
use crate::tutors::{self, Tutor};

/// `scheduled → scheduled`: another time.
pub const RESCHEDULE: &str = "reschedule";
/// `scheduled → scheduled`: another tutor.
pub const CHANGE_TUTOR: &str = "change_tutor";
/// `scheduled → held`: the tutor submitted the attendance.
pub const SUBMIT_ATTENDANCE: &str = "submit_attendance";
/// `held → held`: the attendance is corrected.
pub const CORRECT_ATTENDANCE: &str = "correct_attendance";
/// `scheduled → cancelled`: the session did not take place.
pub const CANCEL: &str = "cancel";
/// `scheduled → cancelled`: the batch was cancelled or terminated.
pub const CANCEL_SYSTEM: &str = "cancel_system";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInput {
    /// The new start ([`RESCHEDULE`]).
    #[serde(default)]
    pub starts_at: Option<DateTime>,
    /// The new end ([`RESCHEDULE`]).
    #[serde(default)]
    pub ends_at: Option<DateTime>,
    /// The new tutor, a wl-scheduling resource ([`CHANGE_TUTOR`]).
    #[serde(default)]
    pub tutor_resource_id: Option<i64>,
    /// The employee behind the new tutor ([`CHANGE_TUTOR`]).
    #[serde(default)]
    pub tutor_employee_id: Option<i64>,
    /// The user who acted.
    #[serde(default)]
    pub by: Option<i64>,
    /// The day of the run (Asia/Dubai).
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// The marks of every student of the roster ([`SUBMIT_ATTENDANCE`]).
    #[serde(default)]
    pub marks: Vec<MarkInput>,
    /// The service worked out the marks of the whole roster
    /// ([`SUBMIT_ATTENDANCE`]): the guard `every_student_marked`.
    #[serde(default)]
    pub marks_complete: Option<bool>,
}

fn input_of(ctx: &TransitionCtx) -> SessionInput {
    ctx.input_as::<SessionInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// Sets the columns of session `id` other than the status.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Session::query()
        .where_(Session::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for Session {
    type State = SessionStatus;
    const COLUMN: renox::db::Col<Session, SessionStatus> = Session::STATUS;

    fn machine() -> Machine<Self> {
        use SessionStatus::{Cancelled, Held, Scheduled};
        Machine::new()
            .initial([Scheduled])
            .state(Scheduled)
            .tone("info")
            .state(Held)
            .tone("success")
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(reschedule())
            .transition(change_tutor())
            .transition(submit_attendance())
            .transition(correct_attendance())
            .transition(cancel(CANCEL, false))
            .transition(cancel(CANCEL_SYSTEM, true))
    }
}

/// `reschedule`: the session moves to the start and end in the input.
fn reschedule() -> Transition<Session> {
    Transition::new(RESCHEDULE)
        .label("Reschedule")
        .permission(permissions::BATCHES_MANAGE)
        .from([SessionStatus::Scheduled])
        .to(SessionStatus::Scheduled)
        .guard("new_time_given", |_s: &Session, ctx| {
            let input = input_of(ctx);
            let ok = matches!((input.starts_at, input.ends_at), (Some(a), Some(b)) if b > a);
            Box::pin(async move { check(ok, "edtech.sessions.problem.new_time") })
        })
        .effect(|s: &Session, ctx, tx| {
            let id = s.id;
            let input = input_of(ctx);
            Box::pin(async move {
                let Some(start) = input.starts_at else {
                    return Ok(());
                };
                set(
                    tx,
                    id,
                    &[
                        ("starts_at", &start),
                        ("ends_at", &input.ends_at),
                        ("session_date", &dubai_date(start)),
                    ],
                )
                .await?;
                // Moved to a date the service found the tutor can teach: the
                // flag of `EDU.RUL.12` is resolved (the rule flags it again if
                // the new date is still affected).
                flags::clear(tx, &[id]).await
            })
        })
}

/// `change_tutor`: the session is taught by the tutor in the input.
fn change_tutor() -> Transition<Session> {
    Transition::new(CHANGE_TUTOR)
        .label("Change tutor")
        .permission(permissions::BATCHES_MANAGE)
        .from([SessionStatus::Scheduled])
        .to(SessionStatus::Scheduled)
        .guard("new_tutor_given", |_s: &Session, ctx| {
            let ok = input_of(ctx).tutor_resource_id.is_some();
            Box::pin(async move { check(ok, "edtech.sessions.problem.new_tutor") })
        })
        .effect(|s: &Session, ctx, tx| {
            let session = s.clone();
            let input = input_of(ctx);
            Box::pin(async move {
                // Under the lock of the new tutor's calendar the slot is
                // checked once more, so two changes cannot give them the same
                // time (BL-EDU-09).
                if let Some(resource) = input.tutor_resource_id {
                    tutors::reserve(
                        tx,
                        resource,
                        &[(session.starts_at, session.ends_at)],
                        Except {
                            batch: None,
                            session: Some(session.id),
                        },
                    )
                    .await?;
                }
                set(
                    tx,
                    session.id,
                    &[
                        ("tutor_resource_id", &input.tutor_resource_id),
                        ("tutor_employee_id", &input.tutor_employee_id),
                    ],
                )
                .await?;
                if let Some(resource) = input.tutor_resource_id {
                    let tutor = Tutor {
                        resource_id: resource,
                        employee_id: input.tutor_employee_id,
                    };
                    tutors::record_session(tx, &session, &tutor, input.by).await?;
                }
                // A new tutor: the flag of `EDU.RUL.12` is resolved.
                flags::clear(tx, &[session.id]).await
            })
        })
}

/// `submit_attendance`: the session took place.
fn submit_attendance() -> Transition<Session> {
    Transition::new(SUBMIT_ATTENDANCE)
        .label("Submit attendance")
        .tone("primary")
        .permission(permissions::SESSIONS_TEACH)
        .from([SessionStatus::Scheduled])
        .to(SessionStatus::Held)
        .guard("start_passed", |s: &Session, _ctx| {
            let ok = s.starts_at <= now();
            Box::pin(async move { check(ok, "edtech.sessions.problem.not_started") })
        })
        .guard("every_student_marked", |_s: &Session, ctx| {
            let ok = input_of(ctx).marks_complete == Some(true);
            Box::pin(async move { check(ok, "edtech.attendance.problem.not_all_marked") })
        })
        .effect(|s: &Session, ctx, tx| {
            let session = s.clone();
            let input = input_of(ctx);
            Box::pin(async move {
                set(
                    tx,
                    session.id,
                    &[("held_at", &Some(now())), ("attendance_by", &input.by)],
                )
                .await?;
                // The marks of the whole roster, with the credit each costs and
                // the make-up right of each excused absence, in the same
                // transaction: a `held` session always has its marks.
                attendance::store_marks(tx, &session, &input.marks, input.by, false).await
            })
        })
}

/// `correct_attendance`: declared with its reason; BL-EDU-14 fills the slot.
fn correct_attendance() -> Transition<Session> {
    Transition::new(CORRECT_ATTENDANCE)
        .label("Correct attendance")
        .permission(permissions::SESSIONS_TEACH)
        .from([SessionStatus::Held])
        .to(SessionStatus::Held)
        .reason_required()
        .guard("within_window", |_s: &Session, _ctx| {
            Box::pin(async { Ok::<(), String>(()) })
        })
}

/// `cancel`: the session did not take place. The system's twin is run by the
/// batch's `cancel` and `terminate` for every session still `scheduled`.
fn cancel(name: &'static str, system: bool) -> Transition<Session> {
    let transition = Transition::new(name)
        .label("Cancel")
        .from([SessionStatus::Scheduled])
        .to(SessionStatus::Cancelled)
        .reason_required();
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::BATCHES_MANAGE)
    };
    transition
        .effect(|s: &Session, ctx, tx| {
            let id = s.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                set(tx, id, &[("cancel_reason", &reason)]).await?;
                flags::clear(tx, &[id]).await
            })
        })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<Session>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition_of_the_spec() {
        let machine = Session::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("scheduled", Some("info"), false),
                ("held", Some("success"), false),
                ("cancelled", Some("neutral"), true),
            ]
        );
        let expected = [
            (RESCHEDULE, vec!["scheduled"], "scheduled"),
            (CHANGE_TUTOR, vec!["scheduled"], "scheduled"),
            (SUBMIT_ATTENDANCE, vec!["scheduled"], "held"),
            (CORRECT_ATTENDANCE, vec!["held"], "held"),
            (CANCEL, vec!["scheduled"], "cancelled"),
            (CANCEL_SYSTEM, vec!["scheduled"], "cancelled"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), name == CANCEL_SYSTEM, "{name}");
        }
    }

    #[test]
    fn the_guards_and_the_guard_slots_are_declared() {
        let machine = Session::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .unwrap()
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(guards(RESCHEDULE), ["new_time_given"]);
        assert_eq!(guards(CHANGE_TUTOR), ["new_tutor_given"]);
        assert_eq!(guards(SUBMIT_ATTENDANCE), ["start_passed", "every_student_marked"]);
        assert_eq!(guards(CORRECT_ATTENDANCE), ["within_window"]);
        assert!(guards(CANCEL).is_empty());
        for name in [CANCEL, CANCEL_SYSTEM, CORRECT_ATTENDANCE] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
    }
}
