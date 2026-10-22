//! The session services (10 §4.2, §7.1 *Session*, `EDU.BAT.01`): generating
//! the sessions of a batch when it opens, keeping a tutor or a room out of
//! two places at once, moving, cancelling and holding a session, and telling
//! wl-scheduling when the tutor and the room are taken.
//!
//! The contracts used are the kernel's (`wl_kernel::scheduling`):
//! [`SessionBlocks`] is the app's [`AvailabilitySource`] (wl-scheduling §8:
//! "class sessions from `apps/edtech`"), so a trial class or a one-to-one slot
//! is never booked over a class and a session planned over a block is a
//! conflict (`SCH.RUL.05`). Decision: the kernel has no contract that asks
//! wl-scheduling whether a resource is free (its `Availability::is_free` is
//! reached through the crate), so the check that the tutor and the room are
//! free for every session of a batch ([`conflicts`]) reads the sessions of the
//! app only; the bookings of the tutor are the booking side's own check
//! (`BookingConflict`).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Executor, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::scheduling::{
    AvailabilitySource, SchedulingFuture, SourceBlock, provide_availability_source,
};

use crate::batch::{Batch, BatchStatus};
use crate::db::refused;
use crate::permissions;
use crate::plan::{self, PlannedSession};
use crate::session::{Session, SessionStatus};
use crate::session_machine::{
    CANCEL, CHANGE_TUTOR, RESCHEDULE, SUBMIT_ATTENDANCE, SessionInput,
};
use crate::texts::english;
use crate::tutors::{self, Tutor};

/// The reason of a block a session makes in wl-scheduling.
pub const BLOCK_REASON: &str = "session";
/// The source of a block a session makes in wl-scheduling.
pub const BLOCK_SOURCE: &str = "session";

/// A tutor or a room already taken by another session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    /// The resource that is taken.
    pub resource_id: i64,
    /// The session that takes it.
    pub session_id: i64,
    /// Its batch.
    pub batch_id: i64,
    /// The day of the slot that was asked for (Asia/Dubai).
    pub on: NaiveDate,
}

/// What a check leaves out: the sessions of a batch (the batch being opened
/// has none yet), or one session (the one being moved).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Except {
    /// The batch whose sessions do not count.
    pub batch: Option<i64>,
    /// The session that does not count.
    pub session: Option<i64>,
}

/// The slots of `slots` in which `tutor` or `room` is taken by a `scheduled`
/// session, one [`Clash`] per resource, session and slot.
///
/// # Errors
///
/// The database's errors.
pub async fn conflicts<'c>(
    db: impl Executor<'c>,
    tutor: Option<i64>,
    room: Option<i64>,
    slots: &[(DateTime, DateTime)],
    except: Except,
) -> Result<Vec<Clash>> {
    let (Some(first), Some(last)) = (
        slots.iter().map(|(start, _)| *start).min(),
        slots.iter().map(|(_, end)| *end).max(),
    ) else {
        return Ok(Vec::new());
    };
    if tutor.is_none() && room.is_none() {
        return Ok(Vec::new());
    }
    let taken = Session::query()
        .where_(Session::STATUS.eq(SessionStatus::Scheduled))
        .where_(Session::STARTS_AT.lt(last))
        .where_(Session::ENDS_AT.gt(first))
        .get(db)
        .await?;
    let mut found = Vec::new();
    for session in &taken {
        if except.session == Some(session.id) || except.batch == Some(session.batch_id) {
            continue;
        }
        for (start, end) in slots {
            if session.starts_at >= *end || session.ends_at <= *start {
                continue;
            }
            for (wanted, held) in [
                (tutor, session.tutor_resource_id),
                (room, session.room_resource_id),
            ] {
                if let (Some(wanted), Some(held)) = (wanted, held)
                    && wanted == held
                {
                    found.push(Clash {
                        resource_id: wanted,
                        session_id: session.id,
                        batch_id: session.batch_id,
                        on: dubai_date(*start),
                    });
                }
            }
        }
    }
    Ok(found)
}

/// The refusal for `clashes`: the days on which the tutor or the room is
/// taken.
fn clash_error(clashes: &[Clash]) -> Error {
    let mut days: Vec<NaiveDate> = clashes.iter().map(|c| c.on).collect();
    days.sort_unstable();
    days.dedup();
    let list = days
        .iter()
        .map(NaiveDate::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    refused(english("edtech.sessions.problem.clash", &[("dates", &list)]))
}

/// Refuses with the days of `clashes` when there are any.
pub(crate) fn no_clash(clashes: &[Clash]) -> Result {
    if clashes.is_empty() {
        Ok(())
    } else {
        Err(clash_error(clashes))
    }
}

/// The slots of `plan`.
pub fn slots_of(plan: &[PlannedSession]) -> Vec<(DateTime, DateTime)> {
    plan.iter().map(|p| (p.starts_at, p.ends_at)).collect()
}

/// Checks that the tutor and the room of `batch` are free for every session
/// of `plan` (all or nothing: the refusal names the days that fail).
///
/// # Errors
///
/// A 422 naming the days; the database's errors.
pub async fn check_free<'c>(
    db: impl Executor<'c>,
    batch: &Batch,
    plan: &[PlannedSession],
) -> Result {
    let clashes = conflicts(
        db,
        batch.tutor_resource_id,
        batch.room_resource_id,
        &slots_of(plan),
        Except {
            batch: Some(batch.id),
            session: None,
        },
    )
    .await?;
    no_clash(&clashes)
}

/// Inserts the sessions of `plan` for `batch` in `tx`, after checking once
/// more, inside the transaction, that the tutor and the room are free. It is
/// the effect of the batch's `open`.
///
/// # Errors
///
/// A 422 naming the days when the tutor or the room is taken; the database's
/// errors.
pub(crate) async fn insert_planned(
    tx: &mut Transaction,
    batch: &Batch,
    plan: &[PlannedSession],
) -> Result {
    check_free(&mut *tx, batch, plan).await?;
    for planned in plan {
        let mut session = Session {
            batch_id: batch.id,
            number: planned.number,
            session_date: planned.date,
            starts_at: planned.starts_at,
            ends_at: planned.ends_at,
            tutor_resource_id: batch.tutor_resource_id,
            tutor_employee_id: batch.tutor_employee_id,
            room_resource_id: batch.room_resource_id,
            ..Session::default()
        };
        session.insert(&mut *tx).await?;
    }
    Ok(())
}

/// Gives the sessions of the old tutor of batch `batch_id`, from `from` on,
/// to the new tutor. A session already moved to another tutor keeps its own;
/// a `held` or `cancelled` session is not touched. It is part of the batch's
/// `change_tutor`: under the lock of the new tutor's calendar every slot is
/// checked once more, so the change is all or nothing, and the sessions moved
/// are no longer flagged (`EDU.RUL.12`).
///
/// # Errors
///
/// A 422 naming the days the new tutor is taken; the database's errors.
pub(crate) async fn reassign_remaining(
    tx: &mut Transaction,
    batch_id: i64,
    from: NaiveDate,
    old: Option<i64>,
    resource: Option<i64>,
    employee: Option<i64>,
) -> Result {
    let remaining = Session::query()
        .where_(Session::BATCH_ID.eq(batch_id))
        .where_(Session::STATUS.eq(SessionStatus::Scheduled))
        .where_(Session::SESSION_DATE.gte(from))
        .get(&mut *tx)
        .await?;
    let moving: Vec<Session> = remaining
        .into_iter()
        .filter(|s| s.tutor_resource_id == old)
        .collect();
    if let Some(resource) = resource {
        let slots: Vec<(DateTime, DateTime)> =
            moving.iter().map(|s| (s.starts_at, s.ends_at)).collect();
        tutors::reserve(&mut *tx, resource, &slots, Except::default()).await?;
    }
    for session in &moving {
        Session::query()
            .where_(Session::ID.eq(session.id))
            .update(
                &mut *tx,
                &[
                    ("tutor_resource_id", &resource),
                    ("tutor_employee_id", &employee),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    let ids: Vec<i64> = moving.iter().map(|s| s.id).collect();
    crate::flags::clear(tx, &ids).await
}

/// The number of `scheduled` sessions of batch `batch_id`.
///
/// # Errors
///
/// The database's errors.
pub async fn count_scheduled(db: &Db, batch_id: i64) -> Result<i64> {
    let open = Session::query()
        .where_(Session::BATCH_ID.eq(batch_id))
        .where_(Session::STATUS.eq(SessionStatus::Scheduled))
        .get(db)
        .await?;
    Ok(i64::try_from(open.len()).unwrap_or(i64::MAX))
}

/// The sessions of batch `batch_id` that were `held` on or before `on`: the
/// sessions the students used (attended or not).
///
/// # Errors
///
/// The database's errors.
pub async fn held_until<'c, E: Executor<'c>>(db: E, batch_id: i64, on: NaiveDate) -> Result<i64> {
    let held = Session::query()
        .where_(Session::BATCH_ID.eq(batch_id))
        .where_(Session::STATUS.eq(SessionStatus::Held))
        .where_(Session::SESSION_DATE.lte(on))
        .count(db)
        .await?;
    Ok(i64::try_from(held).unwrap_or(i64::MAX))
}

/// The sessions a tutor taught between `from` and `until`, both days included:
/// the sessions `held` of the tutor, which the monthly timesheet collects
/// (`EDU.RUL.10`). A cancelled session, as those of a batch `cancel` or
/// `terminate`, is not taught and is not in it.
///
/// # Errors
///
/// The database's errors.
pub async fn taught_in<'c, E: Executor<'c>>(
    db: E,
    tutor_resource_id: i64,
    from: NaiveDate,
    until: NaiveDate,
) -> Result<Vec<Session>> {
    Session::query()
        .where_(Session::TUTOR_RESOURCE_ID.eq(tutor_resource_id))
        .where_(Session::STATUS.eq(SessionStatus::Held))
        .where_(Session::SESSION_DATE.gte(from))
        .where_(Session::SESSION_DATE.lte(until))
        .order_by(Session::STARTS_AT)
        .get(db)
        .await
}

/// The sessions of batch `batch_id` that still count for the students: every
/// one that is not `cancelled`.
///
/// # Errors
///
/// The database's errors.
pub async fn planned_count<'c, E: Executor<'c>>(db: E, batch_id: i64) -> Result<i64> {
    let planned = Session::query()
        .where_(Session::BATCH_ID.eq(batch_id))
        .where_(Session::STATUS.ne(SessionStatus::Cancelled))
        .count(db)
        .await?;
    Ok(i64::try_from(planned).unwrap_or(i64::MAX))
}

/// The wl-scheduling blocks the `scheduled` sessions make for `resource_id`
/// between `from` and `until`.
///
/// # Errors
///
/// The database's errors.
pub async fn blocks_of(
    db: &Db,
    resource_id: i64,
    from: DateTime,
    until: DateTime,
) -> Result<Vec<SourceBlock>> {
    let taken = Session::query()
        .where_(Session::STATUS.eq(SessionStatus::Scheduled))
        .where_(Session::STARTS_AT.lt(until))
        .where_(Session::ENDS_AT.gt(from))
        .order_by(Session::STARTS_AT)
        .get(db)
        .await?;
    Ok(taken
        .iter()
        .filter(|s| {
            s.tutor_resource_id == Some(resource_id) || s.room_resource_id == Some(resource_id)
        })
        .map(|s| SourceBlock {
            starts_at: s.starts_at,
            ends_at: s.ends_at,
            reason: BLOCK_REASON.to_owned(),
            source: BLOCK_SOURCE.to_owned(),
            reference: s.block_reference(),
        })
        .collect())
}

/// The app's [`AvailabilitySource`]: a tutor or a room is not bookable while a
/// `scheduled` session holds it.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionBlocks;

impl AvailabilitySource for SessionBlocks {
    fn blocks<'a>(
        &'a self,
        state: &'a AppState,
        resource_id: i64,
        _employee_id: Option<i64>,
        from: DateTime,
        until: DateTime,
    ) -> SchedulingFuture<'a, Vec<SourceBlock>> {
        Box::pin(async move { blocks_of(&state.db, resource_id, from, until).await })
    }
}

/// The session `id` and the user `by`, who must hold `permission`.
async fn load(state: &AppState, by: &AuthUser, id: i64, permission: &str) -> Result<Session> {
    if !by.has_permission(permission) {
        return Err(Error::Forbidden);
    }
    Session::find_or_404(&state.db, id).await
}

/// Runs the transition `name` of the user `by` on `session`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    session: &Session,
    name: &str,
    input: SessionInput,
) -> Result<Session> {
    let _outcome = session
        .transition(name)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Session::find_or_404(&state.db, session.id).await
}

/// `reschedule`: the session moves to `date` at `start_time` (`HH:MM`, Dubai
/// time) and keeps its length. The tutor and the room must be free then, and
/// the tutor's documents must be valid on the new date (`EDU.ACL.14`).
///
/// # Errors
///
/// A 403 without `batches.manage`; a 422 for a time that is not `HH:MM`, a
/// session that is not `scheduled`, a tutor or room already taken, or a tutor
/// whose documents do not cover the new date.
pub async fn reschedule(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    date: NaiveDate,
    start_time: &str,
) -> Result<Session> {
    let session = load(state, by, id, permissions::BATCHES_MANAGE).await?;
    let time = plan::parse_time(start_time)
        .ok_or_else(|| refused(english("edtech.batches.problem.start_time", &[])))?;
    let length = session.ends_at - session.starts_at;
    let starts_at = BusinessCalendar::default().from_local(date.and_time(time));
    let ends_at = starts_at + length;
    if session.status == SessionStatus::Scheduled {
        if let Some(resource) = session.tutor_resource_id {
            let batch = Batch::find_or_404(&state.db, session.batch_id).await?;
            let tutor = Tutor {
                resource_id: resource,
                employee_id: session.tutor_employee_id,
            };
            tutors::tutor_eligible(state, &tutor, &batch, &[date]).await?;
        }
        let clashes = conflicts(
            &state.db,
            session.tutor_resource_id,
            session.room_resource_id,
            &[(starts_at, ends_at)],
            Except {
                batch: None,
                session: Some(session.id),
            },
        )
        .await?;
        no_clash(&clashes)?;
    }
    let input = SessionInput {
        starts_at: Some(starts_at),
        ends_at: Some(ends_at),
        by: Some(by.id),
        on: Some(today()),
        ..SessionInput::default()
    };
    run(state, by, &session, RESCHEDULE, input).await
}

/// `change_tutor`: the session is taught by `resource` (and the employee
/// behind it). The new tutor must be free then and their documents valid on the
/// date (`EDU.ACL.14`).
///
/// # Errors
///
/// A 403 without `batches.manage`; a 422 for a session that is not
/// `scheduled`, a tutor already taken, or a tutor whose documents do not cover
/// the date.
pub async fn change_tutor(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    resource: i64,
    employee: Option<i64>,
) -> Result<Session> {
    let session = load(state, by, id, permissions::BATCHES_MANAGE).await?;
    let tutor = Tutor {
        resource_id: resource,
        employee_id: employee,
    };
    let batch = Batch::find_or_404(&state.db, session.batch_id).await?;
    tutors::tutor_eligible(state, &tutor, &batch, &[session.session_date]).await?;
    if session.status == SessionStatus::Scheduled {
        let clashes = conflicts(
            &state.db,
            Some(resource),
            None,
            &[(session.starts_at, session.ends_at)],
            Except {
                batch: None,
                session: Some(session.id),
            },
        )
        .await?;
        no_clash(&clashes)?;
    }
    let input = SessionInput {
        tutor_resource_id: Some(resource),
        tutor_employee_id: employee,
        by: Some(by.id),
        on: Some(today()),
        ..SessionInput::default()
    };
    run(state, by, &session, CHANGE_TUTOR, input).await
}

/// `submit_attendance`: the session took place. The batch completes when this
/// was its last open session ([`on_closed`]).
///
/// # Errors
///
/// A 403 without `sessions.teach`; a 422 for a session that is not
/// `scheduled` or has not started.
pub async fn submit_attendance(state: &AppState, by: &AuthUser, id: i64) -> Result<Session> {
    let session = load(state, by, id, permissions::SESSIONS_TEACH).await?;
    let input = SessionInput {
        by: Some(by.id),
        on: Some(today()),
        ..SessionInput::default()
    };
    run(state, by, &session, SUBMIT_ATTENDANCE, input).await
}

/// `cancel`: the session did not take place, for `reason`. With `make_up_on`,
/// a make-up session of the same length, tutor and room is added to the batch
/// on that day, in the same step.
///
/// # Errors
///
/// A 403 without `batches.manage`; a 422 without a reason, for a session that
/// is not `scheduled`, a make-up day in the past or a tutor or room taken
/// then.
pub async fn cancel(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    make_up_on: Option<NaiveDate>,
) -> Result<Session> {
    let session = load(state, by, id, permissions::BATCHES_MANAGE).await?;
    let calendar = BusinessCalendar::default();
    let make_up = match make_up_on {
        Some(date) => {
            if date < today() {
                return Err(refused(english("edtech.sessions.problem.make_up_past", &[])));
            }
            let starts_at = calendar.from_local(date.and_time(calendar.local(session.starts_at).time()));
            let ends_at = starts_at + (session.ends_at - session.starts_at);
            let clashes = conflicts(
                &state.db,
                session.tutor_resource_id,
                session.room_resource_id,
                &[(starts_at, ends_at)],
                Except {
                    batch: None,
                    session: Some(session.id),
                },
            )
            .await?;
            no_clash(&clashes)?;
            Some((date, starts_at, ends_at))
        }
        None => None,
    };
    let input = SessionInput {
        by: Some(by.id),
        on: Some(today()),
        ..SessionInput::default()
    };
    let mut tx = state.db.begin().await?;
    let outcome = session
        .transition(CANCEL)
        .by(by)
        .reason(reason)
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    if let Some((date, starts_at, ends_at)) = make_up {
        let last = Session::query()
            .where_(Session::BATCH_ID.eq(session.batch_id))
            .order_by_desc(Session::NUMBER)
            .first(&mut tx)
            .await?;
        let mut added = Session {
            batch_id: session.batch_id,
            number: last.map_or(1, |s| s.number + 1),
            session_date: date,
            starts_at,
            ends_at,
            tutor_resource_id: session.tutor_resource_id,
            tutor_employee_id: session.tutor_employee_id,
            room_resource_id: session.room_resource_id,
            online_link: session.online_link.clone(),
            make_up: true,
            ..Session::default()
        };
        added.insert(&mut tx).await?;
    }
    tx.commit().await?;
    outcome.emit(state).await?;
    Session::find_or_404(&state.db, id).await
}

/// The batch follows its sessions: when the last open session of a batch is
/// held or cancelled, the batch is completed (`complete`: "system, last
/// session held"); the first session held starts an open batch.
///
/// # Errors
///
/// The database's errors and the machine's.
pub async fn on_closed(state: &AppState, session_id: i64) -> Result {
    let Some(session) = Session::find(&state.db, session_id).await? else {
        return Ok(());
    };
    let Some(batch) = Batch::find(&state.db, session.batch_id).await? else {
        return Ok(());
    };
    if batch.status == BatchStatus::Open && session.status == SessionStatus::Held {
        crate::batches::start(state, batch.id).await?;
    }
    crate::batches::complete_if_done(state, batch.id).await?;
    Ok(())
}

/// Registers the machine, the availability source and the listener.
pub(crate) fn register(app: &mut Registry) {
    crate::session_machine::register();
    provide_availability_source(app, SessionBlocks);
    app.listen(
        |event: Transitioned<Session>, state: AppState| async move {
            if event.transition == SUBMIT_ATTENDANCE || event.transition == CANCEL {
                on_closed(&state, event.key).await?;
            }
            if event.transition == CHANGE_TUTOR {
                tutors::tell_session_change(&state, event.key).await?;
            }
            Ok(())
        },
    );
}

/// The time a day's session starts, for the pages: `18:00`.
pub fn time_text(at: DateTime) -> String {
    BusinessCalendar::default()
        .local(at)
        .format("%H:%M")
        .to_string()
}

/// The session's length in whole minutes.
pub fn minutes_of(session: &Session) -> i64 {
    (session.ends_at - session.starts_at).num_minutes()
}
