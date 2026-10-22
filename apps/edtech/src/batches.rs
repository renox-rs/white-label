//! The batch services (10 §4.2, §7.1 *Batch*, `EDU.BAT.01`): planning one,
//! opening it, starting it when its first session date comes and completing it
//! when its last session is held.
//!
//! The machine ([`crate::batch_machine`]) holds the rules a record and its
//! input can answer; this file gathers what it cannot know (the public
//! holidays of the centre's calendar, the plan the weekly pattern gives, the
//! sessions still open) and runs the transition. `cancel` (BL-EDU-13) and
//! `terminate` (BL-EDU-12) are here; the service of `change_tutor` grows with
//! BL-EDU-09.

use std::collections::BTreeSet;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError, Transitioned};
use wl_kernel::approvals::{ApprovalPolicy, announce, submit::REQUEST_KEY};
use wl_kernel::Subject;
use wl_kernel::calendar::calendar;
use wl_kernel::documents::today;

use crate::batch::{Batch, BatchStatus};
use crate::batch_machine::{
    BatchInput, CANCEL, COMPLETE, OPEN, START, TERMINATE, TERMINATE_ACTION, TERMINATE_APPROVED,
};
use crate::course::Course;
use crate::db::refused;
use crate::exit::{self, ExitChoice, ExitKind};
use crate::permissions;
use crate::plan::{self, PlanProblem, PlannedSession};
use crate::sessions;
use crate::texts::english;
use crate::tutors::{self, Tutor};

/// The scheduled task and command that start the batches whose first session
/// date came.
pub const TASK: &str = "edtech:batches";
/// The time of day it runs.
pub const TASK_AT: &str = "00:30";
/// The time zone of [`TASK_AT`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// A batch to plan.
#[derive(Debug, Clone)]
pub struct NewBatch {
    /// The course it runs.
    pub course_id: i64,
    /// The campus, a wl-scheduling location.
    pub campus: Option<Subject>,
    /// The seats.
    pub capacity: i64,
    /// The first day of the pattern.
    pub start_date: NaiveDate,
    /// The last day of the pattern.
    pub end_date: NaiveDate,
    /// The weekdays, 1 = Monday ... 7 = Sunday.
    pub weekdays: Vec<i64>,
    /// The start time of every session, `HH:MM`.
    pub start_time: String,
    /// The length of every session in minutes.
    pub duration_minutes: i64,
    /// The tutor, a wl-scheduling resource.
    pub tutor_resource_id: Option<i64>,
    /// The employee behind the tutor.
    pub tutor_employee_id: Option<i64>,
    /// The room, a wl-scheduling resource.
    pub room_resource_id: Option<i64>,
    /// The persons the room holds.
    pub room_limit: Option<i64>,
}

impl NewBatch {
    /// A batch of `course_id` with `capacity` seats from `start_date` to
    /// `end_date`, on `weekdays` at `start_time` for `duration_minutes`.
    pub fn new(
        course_id: i64,
        capacity: i64,
        (start_date, end_date): (NaiveDate, NaiveDate),
        weekdays: &[i64],
        start_time: &str,
        duration_minutes: i64,
    ) -> Self {
        NewBatch {
            course_id,
            campus: None,
            capacity,
            start_date,
            end_date,
            weekdays: weekdays.to_vec(),
            start_time: start_time.to_owned(),
            duration_minutes,
            tutor_resource_id: None,
            tutor_employee_id: None,
            room_resource_id: None,
            room_limit: None,
        }
    }

    /// With the tutor (a resource) and the employee behind it.
    pub fn tutor(mut self, resource: i64, employee: Option<i64>) -> Self {
        self.tutor_resource_id = Some(resource);
        self.tutor_employee_id = employee;
        self
    }

    /// With the room (a resource) and the persons it holds, when it says.
    pub fn room(mut self, resource: i64, limit: Option<i64>) -> Self {
        self.room_resource_id = Some(resource);
        self.room_limit = limit;
        self
    }

    /// At the campus.
    pub fn campus(mut self, campus: Subject) -> Self {
        self.campus = Some(campus);
        self
    }
}

/// The public holidays of the centre's calendar.
pub fn holidays(state: &AppState) -> BTreeSet<NaiveDate> {
    calendar(state).holidays().collect()
}

/// The sessions the weekly pattern of `batch` gives around the holidays.
///
/// # Errors
///
/// A 422 naming what is wrong with the pattern.
pub fn plan_for(state: &AppState, batch: &Batch) -> Result<Vec<PlannedSession>> {
    plan::sessions(batch, &holidays(state)).map_err(|problem| {
        let key = match problem {
            PlanProblem::StartTime => "edtech.batches.problem.start_time",
            PlanProblem::Duration => "edtech.batches.problem.duration",
            PlanProblem::Weekdays => "edtech.batches.problem.weekdays",
            PlanProblem::Dates => "edtech.batches.problem.dates",
        };
        refused(english(key, &[]))
    })
}

/// Plans a batch: it starts `planned`, not visible to students.
///
/// # Errors
///
/// A 403 without `batches.manage`; a 404 for an unknown course; a 422 when the
/// course is not active, the seats are fewer than one, or the pattern gives no
/// session.
pub async fn create(state: &AppState, by: &AuthUser, new: NewBatch) -> Result<Batch> {
    if !by.has_permission(permissions::BATCHES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let course = Course::find_or_404(&state.db, new.course_id).await?;
    if !course.active {
        return Err(refused(english("edtech.batches.problem.course_inactive", &[])));
    }
    if new.capacity < 1 {
        return Err(refused(english("edtech.batches.problem.capacity", &[])));
    }
    let mut batch = Batch {
        course_id: course.id,
        campus_type: new.campus.as_ref().map(|c| c.subject_type.clone()),
        campus_id: new.campus.as_ref().map(|c| c.subject_id),
        capacity: new.capacity,
        start_date: new.start_date,
        end_date: new.end_date,
        weekdays: renox::db::Json(new.weekdays.clone()),
        start_time: new.start_time.trim().to_owned(),
        duration_minutes: new.duration_minutes,
        tutor_resource_id: new.tutor_resource_id,
        tutor_employee_id: new.tutor_employee_id,
        room_resource_id: new.room_resource_id,
        room_limit: new.room_limit,
        created_by: Some(by.id),
        ..Batch::default()
    };
    batch.weekdays = renox::db::Json(batch.weekday_list());
    // A pattern that gives no session is refused when it is planned, not when
    // it is opened.
    if plan_for(state, &batch)?.is_empty() {
        return Err(refused(english("edtech.batches.problem.no_sessions", &[])));
    }
    let mut tx = state.db.begin().await?;
    batch.insert(&mut tx).await?;
    if let Some(resource) = batch.tutor_resource_id {
        let tutor = Tutor {
            resource_id: resource,
            employee_id: batch.tutor_employee_id,
        };
        tutors::record_batch(&mut tx, batch.id, &tutor, batch.start_date, Some(by.id)).await?;
    }
    tx.commit().await?;
    Ok(batch)
}

/// Runs the transition `name` on batch `id`, as the user `by` or the system.
pub(crate) async fn run(
    state: &AppState,
    by: Option<&AuthUser>,
    id: i64,
    name: &str,
    input: BatchInput,
) -> Result<Batch> {
    let batch = Batch::find_or_404(&state.db, id).await?;
    let run = batch.transition(name);
    let run = match by {
        Some(user) => run.by(user),
        None => run.by_system(),
    };
    let _outcome = run
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Batch::find_or_404(&state.db, id).await
}

/// `open`: the batch is open for enrolment. The plan is made from the weekly
/// pattern around the holidays and checked before the run.
///
/// The centre permit and the course approval must be valid on the batch's last
/// session date (BL-EDU-07, [`crate::documents`]); BL-EDU-08 adds the tutor's
/// `EDU.ACL.14` on every session date. Both are checked in [`before_open`].
///
/// # Errors
///
/// The machine's: a 403 without `batches.manage`, a 422 unless the batch is
/// `planned` and its plan is complete; a 422 from [`before_open`].
pub async fn open(state: &AppState, by: &AuthUser, id: i64) -> Result<Batch> {
    let batch = Batch::find_or_404(&state.db, id).await?;
    let sessions = if batch.status == BatchStatus::Planned {
        plan_for(state, &batch)?
    } else {
        Vec::new()
    };
    before_open(state, &batch, &sessions, by.id).await?;
    let input = BatchInput {
        sessions,
        by: Some(by.id),
        ..BatchInput::default()
    };
    run(state, Some(by), id, OPEN, input).await
}

/// What must hold before a batch opens and the machine cannot see: the tutor
/// and the room are free for every session of the plan (all or nothing: the
/// refusal names the days that fail), and the centre permit and the course
/// approval, where the course needs one, are valid on the **last session date**
/// (BL-EDU-07: they must cover the whole batch; no override). BL-EDU-08 adds the
/// tutor's eligibility on every session date.
async fn before_open(
    state: &AppState,
    batch: &Batch,
    plan: &[PlannedSession],
    actor: i64,
) -> Result {
    sessions::check_free(&state.db, batch, plan).await?;
    if let Some(last) = plan.last() {
        crate::documents::check_batch_open(state, batch, last.date, Some(actor)).await?;
    }
    Ok(())
}

/// `start` batch `id`, as the system; a batch that is no longer `open` is left
/// as it is.
///
/// # Errors
///
/// The database's errors and the machine's.
pub async fn start(state: &AppState, id: i64) -> Result {
    let batch = Batch::find_or_404(&state.db, id).await?;
    let run = batch
        .transition(START)
        .by_system()
        .input(&BatchInput::default())
        .run(state)
        .await;
    match run {
        Ok(_) | Err(TransitionError::NotFrom(_)) => Ok(()),
        Err(other) => Err(other.into_error()),
    }
}

/// `complete` batch `id`: every session is held or cancelled. A user must hold
/// `batches.manage`; the system (`by` is `None`) is the run after the last
/// session.
///
/// # Errors
///
/// A 403 without `batches.manage`; a 422 unless the batch is `running` and
/// has no session still waiting for its attendance.
pub async fn complete(state: &AppState, by: Option<&AuthUser>, id: i64) -> Result<Batch> {
    if let Some(user) = by
        && !user.has_permission(permissions::BATCHES_MANAGE)
    {
        return Err(Error::Forbidden);
    }
    let unsubmitted = sessions::count_scheduled(&state.db, id).await?;
    let input = BatchInput {
        unsubmitted: Some(unsubmitted),
        by: by.map(|user| user.id),
        ..BatchInput::default()
    };
    run(state, None, id, COMPLETE, input).await
}

/// Completes batch `id` when it is `running` and its last open session has
/// been held or cancelled. Returns whether it did.
///
/// # Errors
///
/// The database's errors and the machine's.
pub async fn complete_if_done(state: &AppState, id: i64) -> Result<bool> {
    let batch = Batch::find_or_404(&state.db, id).await?;
    if batch.status != BatchStatus::Running {
        return Ok(false);
    }
    if sessions::count_scheduled(&state.db, id).await? > 0 {
        return Ok(false);
    }
    complete(state, None, id).await?;
    Ok(true)
}

/// `start`: the open batches whose first session date has come are running.
/// Returns the ids started.
///
/// # Errors
///
/// The database's errors and the machine's.
pub async fn start_due(state: &AppState, on: NaiveDate) -> Result<Vec<i64>> {
    let due = Batch::query()
        .where_(Batch::STATUS.eq(BatchStatus::Open))
        .where_(Batch::START_DATE.lte(on))
        .order_by(Batch::ID)
        .get(&state.db)
        .await?;
    let mut started = Vec::new();
    for batch in due {
        let run = batch
            .transition(START)
            .by_system()
            .input(&BatchInput::default())
            .run(state)
            .await;
        match run {
            Ok(_) => started.push(batch.id),
            // Another run got there first.
            Err(TransitionError::NotFrom(_)) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(started)
}

/// The daily run: the batches whose first session date came start, and the
/// running batches without an open session complete. Returns the ids started.
///
/// # Errors
///
/// The database's errors and the machine's.
pub async fn run_daily(state: &AppState) -> Result<Vec<i64>> {
    let started = start_due(state, today()).await?;
    let running = Batch::query()
        .where_(Batch::STATUS.eq(BatchStatus::Running))
        .order_by(Batch::ID)
        .get(&state.db)
        .await?;
    for batch in running {
        complete_if_done(state, batch.id).await?;
    }
    Ok(started)
}

/// `cancel`: the batch will not run. The coordinator gives a reason and,
/// per enrolment, *transfer to batch X* or *withdraw* (the default). The
/// sessions are cancelled, a transfer takes its seat in the target at that
/// moment (a seat that is gone makes it a withdrawal and the coordinator is
/// told), every withdrawal is refunded in full, registration fee included, and
/// the family is told ([`crate::exit`]); each refund waits for its own approval
/// (`FIN.ACL.04`).
///
/// # Errors
///
/// A 403 without `batches.manage`; a 422 in words for a batch that is not
/// `planned` or `open`, a missing reason, or a choice that cannot be done.
pub async fn cancel(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    choices: Vec<ExitChoice>,
) -> Result<Batch> {
    if !by.has_permission(permissions::BATCHES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let batch = Batch::find_or_404(&state.db, id).await?;
    if !matches!(batch.status, BatchStatus::Planned | BatchStatus::Open) {
        return Err(refused(english("edtech.batches.problem.cancel_status", &[])));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(english("edtech.batches.problem.cancel_reason", &[])));
    }
    let (plan, paid) = exit::validate(state, &batch, &choices).await?;
    let input = BatchInput {
        by: Some(by.id),
        reason: Some(reason.to_owned()),
        choices: plan,
        paid,
        choices_checked: true,
        ..BatchInput::default()
    };
    let _outcome = batch
        .transition(CANCEL)
        .by(by)
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Batch::find_or_404(&state.db, id).await
}

/// The reasons a running batch is terminated for, and whether each is
/// regulatory (the centre manager and the owner are told).
pub const TERMINATION_REASONS: &[(&str, bool)] = &[
    ("tutor_unavailable", false),
    ("approval_expired", true),
    ("approval_revoked", true),
    ("venue", false),
    ("too_few_students", false),
];

/// Whether the termination reason `code` is regulatory: the approval expired or
/// was revoked.
pub fn is_regulatory(code: &str) -> bool {
    TERMINATION_REASONS
        .iter()
        .any(|(known, regulatory)| *known == code && *regulatory)
}

/// What a request to terminate a batch came to.
#[derive(Debug, Clone)]
pub struct Termination {
    /// The batch: `running` while the centre manager decides.
    pub batch: Batch,
    /// The approval request that waits for the centre manager.
    pub approval_request: Option<i64>,
}

/// `terminate`: the coordinator asks to stop a running batch for the reason
/// `code` (one of [`TERMINATION_REASONS`], with an optional `note`) and names,
/// per enrolment, *transfer to batch X* or *withdraw* (the default). The kernel
/// defers the run as a request that a holder of `batches.approve` other than the
/// coordinator decides (`EDU.ACL.15`); sessions go on until then. At the approval
/// the remaining sessions are cancelled, the enrolments are moved or withdrawn
/// with the unused sessions refunded pro rata and no admin fee, and the people
/// are told ([`crate::exit`]).
///
/// # Errors
///
/// A 403 without `batches.manage`; a 422 in words for a batch that is not
/// `running`, an unknown reason, a choice that cannot be done, or no approval
/// policy in force.
pub async fn terminate(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    code: &str,
    note: Option<&str>,
    choices: Vec<ExitChoice>,
) -> Result<Termination> {
    if !by.has_permission(permissions::BATCHES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let batch = Batch::find_or_404(&state.db, id).await?;
    if batch.status != BatchStatus::Running {
        return Err(refused(english("edtech.batches.problem.terminate_status", &[])));
    }
    let code = code.trim();
    if !TERMINATION_REASONS.iter().any(|(known, _)| *known == code) {
        return Err(refused(english("edtech.batches.problem.terminate_reason", &[])));
    }
    let label = english(&format!("edtech.batches.termination.{code}"), &[]);
    let reason = match note.map(str::trim).filter(|n| !n.is_empty()) {
        Some(note) => format!("{label}: {note}"),
        None => label,
    };
    let (plan, paid) = exit::validate(state, &batch, &choices).await?;
    let mut input = BatchInput {
        by: Some(by.id),
        reason: Some(reason.clone()),
        choices: plan,
        paid,
        choices_checked: true,
        regulatory: is_regulatory(code),
        ..BatchInput::default()
    };
    // A policy taken away must not let a termination through unapproved.
    let value = renox::serde_json::to_value(&input).unwrap_or(renox::serde_json::Value::Null);
    input.approval_ready = ApprovalPolicy::for_action(&state.db, TERMINATE_ACTION, &value)
        .await?
        .is_some();
    let outcome = batch
        .transition(TERMINATE)
        .by(by)
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending
            .data
            .get(REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        Outcome::Done(_) => None,
    };
    Ok(Termination {
        batch: Batch::find_or_404(&state.db, id).await?,
        approval_request,
    })
}

/// Registers the machines and the daily run.
pub(crate) fn register(app: &mut Registry) {
    crate::batch_machine::register();
    sessions::register(app);
    // A batch that ended early: the money documents, the notices and the log
    // of what happened to each enrolment, once, after the commit.
    app.listen(|event: Transitioned<Batch>, state: AppState| async move {
        let kind = match event.transition.as_str() {
            CANCEL => Some(ExitKind::Cancel),
            TERMINATE | TERMINATE_APPROVED => Some(ExitKind::Terminate),
            _ => None,
        };
        if let Some(kind) = kind {
            exit::finish(&state, event.key, kind).await?;
        }
        Ok(())
    });
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Start the open batches whose first session date has come and complete the running ones whose sessions are all closed",
        |_args, state: AppState| async move {
            let started = run_daily(&state).await?;
            println!("{} batches started.", started.len());
            Ok(())
        },
    );
}
