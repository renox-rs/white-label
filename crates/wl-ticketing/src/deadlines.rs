//! The deadline task services (wl-ticketing §4.4 *Task*, §5.4, §6 `Tasks`):
//! creating tasks, the user actions, holding and releasing the tasks of a
//! paused agreement, waiving them when it ends, the nightly `mark_overdue`
//! (`WRK.RUL.05`) and the start-date and due-date reminders (`WRK.RUL.06`),
//! and the module's [`DeadlineTasks`] for the kernel contract.
//!
//! **Next occurrence.** `complete`, `complete_reviewed`, `close_missed`,
//! `waive` and `auto_waive` generate the next occurrence of a recurring task
//! (its due date one recurrence later), after the commit, once per task;
//! never once the source agreement ended, nor for a system waive asked
//! without it ([`WaiveTask::without_next`]). A next occurrence of a held
//! source is created held.
//!
//! **Held tasks** (BL-TKT-10). [`hold`] sets the held flag on every open task
//! of the source and keeps state and due date; tasks created while held are
//! created held. Client reminders stop (the staff ones go on), and
//! `mark_overdue` still runs. [`release`] makes the past-due tasks `overdue`,
//! clears the flag and lists, once, for the manager the tasks that went
//! overdue while held. Decision: no hold reason is stored, so the flag shows
//! no reason to anyone (a restricted block reason cannot leak).
//!
//! **Waive** (BL-TKT-09). A `tasks.manage` holder waives with a reason; from
//! a started task (`in_progress`, `waiting`) only when *waive a started task*
//! is on ([`WlTicketing::waive_started_tasks`](crate::WlTicketing::waive_started_tasks);
//! on unless set, 04: off). When the source agreement ends ([`end_source`]),
//! its `upcoming` tasks are waived with "source agreement ended" (started
//! ones too when the setting is on: the work recorded is kept and the
//! assignee and the counterpart are told) and the open tasks left are listed
//! for the manager on one task. Decision: tasks `in_review` or `overdue`,
//! which cannot be waived, are listed too.

use std::sync::{Arc, Mutex, PoisonError};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::{occurrence_key, schedule_subject};
use wl_kernel::deadlines::{
    DeadlineFuture, DeadlineTaskId, DeadlineTaskStatus, DeadlineTaskSummary, DeadlineTasks,
    EndedTasks, NewDeadlineTask, ReleasedTasks, SOURCE_ENDED_REASON, TaskCompleted, TaskOverdue,
    WaiveTask,
};
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::TicketRef;
use wl_kernel::{COMPANY_ADMIN_ROLE, Subject};

use crate::deadline::{DeadlineTask, TaskSource, start_date};
use crate::deadline_machine::{
    AUTO_UNBLOCK, AUTO_WAIVE, CLOSE_MISSED, COMPLETE, COMPLETE_REVIEWED, MARK_OVERDUE, SEND_BACK,
    START, SUBMIT_REVIEW, TaskInput, UNBLOCK, WAIT, WAIVE,
};
use crate::texts::english;
use crate::visit_check::holders_in;

/// The rule that marks past-due tasks `overdue` (§6; 04 `ACC.RUL.04`).
/// A vertical runs it under its own rule id with
/// [`WlTicketing::overdue_rule_id`](crate::WlTicketing::overdue_rule_id) and
/// makes that id compliance-critical with
/// `wl_kernel::compliance::CriticalRule::new(..)`.
pub const OVERDUE_RULE: &str = "WRK.RUL.05";
/// The nightly task and command of [`OVERDUE_RULE`].
pub const OVERDUE_TASK: &str = "ticketing:task_overdue";
/// When [`OVERDUE_TASK`] runs, Asia/Dubai.
pub const OVERDUE_AT: &str = "00:30";

/// The rule that reminds of a task's start date and of its due date (§6;
/// 04 `ACC.RUL.01`).
pub const REMINDER_RULE: &str = "WRK.RUL.06";
/// The daily task and command of [`REMINDER_RULE`].
pub const REMINDER_TASK: &str = "ticketing:task_reminders";
/// When [`REMINDER_TASK`] runs, Asia/Dubai.
pub const REMINDER_AT: &str = "07:00";

/// The owner and the manager are told a task went overdue.
pub const OVERDUE_NOTICE: &str = "ticketing.task_overdue";
/// The assignee and the counterpart are told a started task was waived.
pub const WAIVED_NOTICE: &str = "ticketing.task_waived";
/// The owner is reminded of the start date or the due date.
pub const REMINDER_NOTICE: &str = "ticketing.task_reminder";
/// The client is reminded of a task that needs their input.
pub const CLIENT_REMINDER_NOTICE: &str = "ticketing.task_client_reminder";
/// The counterpart is told the task waits on them.
pub const WAITING_NOTICE: &str = "ticketing.task_waiting";
/// The owner is told the wait is over.
pub const UNBLOCKED_NOTICE: &str = "ticketing.task_unblocked";
/// The reviewers are told a task waits for their review.
pub const REVIEW_NOTICE: &str = "ticketing.task_review";

/// The states a task is open in.
const OPEN: [DeadlineTaskStatus; 5] = [
    DeadlineTaskStatus::Upcoming,
    DeadlineTaskStatus::InProgress,
    DeadlineTaskStatus::Waiting,
    DeadlineTaskStatus::InReview,
    DeadlineTaskStatus::Overdue,
];

/// The states `mark_overdue` leaves.
const NOT_YET_OVERDUE: [DeadlineTaskStatus; 4] = [
    DeadlineTaskStatus::Upcoming,
    DeadlineTaskStatus::InProgress,
    DeadlineTaskStatus::Waiting,
    DeadlineTaskStatus::InReview,
];

/// The deadline task settings of the client (§7), provided by the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    /// *Waive a started task* allowed (on; 04: off).
    pub waive_started: bool,
    /// The role told of overdue tasks and given the lists on release and on
    /// end (the company administrators unless set; 04: the manager).
    pub manager_role: String,
    /// The days before the due date the owner (and, for a task that needs
    /// client input, the client) is reminded (none unless set; 04: 30, 14,
    /// 7, 3).
    pub reminder_days: Vec<i64>,
    /// The rule id the nightly `mark_overdue` runs under ([`OVERDUE_RULE`]
    /// unless set; 04: `ACC.RUL.04`).
    pub overdue_rule: String,
    /// The roles told of an overdue task besides its owners and the manager
    /// role, in the source's scope (none unless set; 04: the partner).
    pub overdue_roles: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            waive_started: true,
            manager_role: COMPANY_ADMIN_ROLE.to_owned(),
            reminder_days: Vec::new(),
            overdue_rule: OVERDUE_RULE.to_owned(),
            overdue_roles: Vec::new(),
        }
    }
}

/// The settings of this app.
pub(crate) fn settings(state: &AppState) -> Settings {
    state
        .provided::<Settings>()
        .map_or_else(Settings::default, |s| (*s).clone())
}

/// The notification types of deadline tasks.
pub fn notification_types() -> [NotificationTypeDef; 7] {
    let of = |key: &str, texts: &str| {
        let subject = english(&format!("wl_ticketing.notice.{texts}.subject")).to_owned();
        let body = english(&format!("wl_ticketing.notice.{texts}.body")).to_owned();
        NotificationTypeDef::new(key, "ticketing")
            .group("tasks")
            .template(NotifyChannel::Email, "en", Some(&subject), &body)
    };
    [
        of(OVERDUE_NOTICE, "task_overdue"),
        of(WAIVED_NOTICE, "task_waived"),
        of(REMINDER_NOTICE, "task_reminder"),
        of(CLIENT_REMINDER_NOTICE, "task_client_reminder"),
        of(WAITING_NOTICE, "task_waiting"),
        of(UNBLOCKED_NOTICE, "task_unblocked"),
        of(REVIEW_NOTICE, "task_review"),
    ]
}

/// A 422 with the package text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key).to_owned())
}

/// The input of a run on `today`.
fn input(state: &AppState, today: NaiveDate) -> TaskInput {
    TaskInput {
        today,
        waive_started: settings(state).waive_started,
        evidence: None,
    }
}

/// Creates an `upcoming` task from `new`; created held when its source is
/// held.
///
/// # Errors
///
/// 422 for a blank title or type or a negative lead time; the database's
/// errors.
pub async fn create(state: &AppState, new: NewDeadlineTask) -> Result<DeadlineTask> {
    if new.title.trim().is_empty() || new.task_type.trim().is_empty() {
        return Err(refused("wl_ticketing.task.problem.blank"));
    }
    if new.lead_days < 0 {
        return Err(refused("wl_ticketing.task.problem.lead_days"));
    }
    let held = TaskSource::is_held(&state.db, &new.source).await?;
    let mut task = DeadlineTask {
        task_type: new.task_type.trim().to_owned(),
        title: new.title.trim().to_owned(),
        description: new.description,
        subject_type: new.source.subject_type.clone(),
        subject_id: new.source.subject_id,
        party_id: new.party.map(|p| p.0),
        due_date: new.due_date,
        lead_days: new.lead_days,
        start_date: start_date(new.due_date, new.lead_days),
        owner_user_id: new.owner_user,
        owner_role: new.owner_role,
        reviewer_role: new.reviewer_role,
        evidence_required: new.evidence_required,
        needs_client_input: new.needs_client_input,
        recurrence: new.recurrence,
        period_end: new.period_end,
        held,
        status: DeadlineTaskStatus::Upcoming,
        ..DeadlineTask::default()
    };
    task.insert(&state.db).await?;
    Ok(task)
}

/// The next occurrence of `task`, generated once when it recurs and its
/// source has not ended; `None` otherwise.
///
/// # Errors
///
/// The database's errors.
pub async fn next_occurrence(
    state: &AppState,
    task: &DeadlineTask,
) -> Result<Option<DeadlineTask>> {
    if task.next_task_id.is_some() {
        return Ok(None);
    }
    let source = task.source();
    if TaskSource::has_ended(&state.db, &source).await? {
        return Ok(None);
    }
    let Some(due) = task.recurrence.next(task.due_date) else {
        return Ok(None);
    };
    let held = TaskSource::is_held(&state.db, &source).await?;
    let mut next = DeadlineTask {
        task_type: task.task_type.clone(),
        title: task.title.clone(),
        description: task.description.clone(),
        subject_type: task.subject_type.clone(),
        subject_id: task.subject_id,
        party_id: task.party_id,
        due_date: due,
        lead_days: task.lead_days,
        start_date: start_date(due, task.lead_days),
        owner_user_id: task.owner_user_id,
        owner_role: task.owner_role.clone(),
        reviewer_role: task.reviewer_role.clone(),
        evidence_required: task.evidence_required,
        needs_client_input: task.needs_client_input,
        recurrence: task.recurrence,
        period_end: task.period_end.and_then(|p| task.recurrence.next(p)),
        held,
        previous_task_id: Some(task.id),
        status: DeadlineTaskStatus::Upcoming,
        ..DeadlineTask::default()
    };
    next.insert(&state.db).await?;
    DeadlineTask::query()
        .where_(DeadlineTask::ID.eq(task.id))
        .update(&state.db, &[("next_task_id", &Some(next.id))])
        .await?;
    Ok(Some(next))
}

/// `start` by `user`.
///
/// # Errors
///
/// `Forbidden` without `tasks.work`; `NotFrom` unless `upcoming` or
/// `overdue`.
pub async fn start(
    state: &AppState,
    user: &AuthUser,
    task: &DeadlineTask,
) -> std::result::Result<(), TransitionError> {
    let _outcome = task.transition(START).by(user).run(state).await?;
    Ok(())
}

/// `wait` by `user`, with `reason` (needed unless the task has a linked
/// open item).
///
/// # Errors
///
/// `Forbidden` without `tasks.work`; `Guard` without a reason or item.
pub async fn wait(
    state: &AppState,
    user: &AuthUser,
    task: &DeadlineTask,
    reason: Option<&str>,
) -> std::result::Result<(), TransitionError> {
    let mut run = task.transition(WAIT).by(user);
    if let Some(reason) = reason {
        run = run.reason(reason);
    }
    let _outcome = run.run(state).await?;
    Ok(())
}

/// `unblock` by `user`.
///
/// # Errors
///
/// `Forbidden` without `tasks.work`; `NotFrom` unless `waiting`.
pub async fn unblock(
    state: &AppState,
    user: &AuthUser,
    task: &DeadlineTask,
) -> std::result::Result<(), TransitionError> {
    let _outcome = task.transition(UNBLOCK).by(user).run(state).await?;
    Ok(())
}

/// `unblock` as the system: the linked items were answered.
///
/// # Errors
///
/// `NotFrom` unless `waiting`.
pub async fn auto_unblock(
    state: &AppState,
    task: &DeadlineTask,
) -> std::result::Result<(), TransitionError> {
    let _outcome = task.transition(AUTO_UNBLOCK).by_system().run(state).await?;
    Ok(())
}

/// `submit_review` by `user`.
///
/// # Errors
///
/// `Forbidden` without `tasks.work`; `Guard` without a reviewer role.
pub async fn submit_review(
    state: &AppState,
    user: &AuthUser,
    task: &DeadlineTask,
) -> std::result::Result<(), TransitionError> {
    let _outcome = task.transition(SUBMIT_REVIEW).by(user).run(state).await?;
    Ok(())
}

/// `send_back` by `user`, with `comment`.
///
/// # Errors
///
/// `Forbidden` without `tasks.review`; `ReasonRequired` without a comment.
pub async fn send_back(
    state: &AppState,
    user: &AuthUser,
    task: &DeadlineTask,
    comment: Option<&str>,
) -> std::result::Result<(), TransitionError> {
    let mut run = task.transition(SEND_BACK).by(user);
    if let Some(comment) = comment {
        run = run.reason(comment);
    }
    let _outcome = run.run(state).await?;
    Ok(())
}

/// `complete` by `user` (`complete_reviewed` for a task in review), with
/// `evidence`.
///
/// # Errors
///
/// `Forbidden` without the permission; `Guard` without required evidence,
/// or for a task with a reviewer that is not in review.
pub async fn complete(
    state: &AppState,
    user: &AuthUser,
    task: &DeadlineTask,
    evidence: Option<&str>,
    today: NaiveDate,
) -> std::result::Result<(), TransitionError> {
    let name = if task.status == DeadlineTaskStatus::InReview {
        COMPLETE_REVIEWED
    } else {
        COMPLETE
    };
    let mut gathered = input(state, today);
    gathered.evidence = evidence.map(str::to_owned);
    let _outcome = task
        .transition(name)
        .by(user)
        .input(&gathered)
        .run(state)
        .await?;
    Ok(())
}

/// `close_missed` by `user`, with `reason`.
///
/// # Errors
///
/// `Forbidden` without `tasks.manage`; `ReasonRequired` without a reason;
/// `NotFrom` unless `overdue`.
pub async fn close_missed(
    state: &AppState,
    user: &AuthUser,
    task: &DeadlineTask,
    reason: Option<&str>,
) -> std::result::Result<(), TransitionError> {
    let mut run = task.transition(CLOSE_MISSED).by(user);
    if let Some(reason) = reason {
        run = run.reason(reason);
    }
    let _outcome = run.run(state).await?;
    Ok(())
}

/// `waive` by `user`, with `reason`, on `today`.
///
/// # Errors
///
/// `Forbidden` without `tasks.manage`; `ReasonRequired` without a reason;
/// `Guard` for a started task while *waive a started task* is off;
/// `NotFrom` for a task in review, overdue or closed.
pub async fn waive(
    state: &AppState,
    user: &AuthUser,
    task: &DeadlineTask,
    reason: Option<&str>,
    today: NaiveDate,
) -> std::result::Result<(), TransitionError> {
    let mut run = task.transition(WAIVE).by(user).input(&input(state, today));
    if let Some(reason) = reason {
        run = run.reason(reason);
    }
    let _outcome = run.run(state).await?;
    Ok(())
}

/// Waives `task` as the system with `waive`; `Ok(false)` when it cannot be
/// waived (its state, or a started task while the setting is off).
///
/// # Errors
///
/// 422 for a blank reason; the database's errors.
pub async fn waive_as_system(
    state: &AppState,
    task: &DeadlineTask,
    waive: &WaiveTask,
    today: NaiveDate,
) -> Result<bool> {
    if waive.reason.trim().is_empty() {
        return Err(refused("wl_ticketing.task.problem.reason"));
    }
    let run = task
        .transition(AUTO_WAIVE)
        .by_system()
        .reason(waive.reason.trim())
        .input(&input(state, today))
        .meta(json!({ "no_next": !waive.next_occurrence }))
        .run(state)
        .await;
    match run {
        Ok(_) => Ok(true),
        Err(
            TransitionError::NotFrom(_) | TransitionError::Guard(_) | TransitionError::Conflict,
        ) => Ok(false),
        Err(other) => Err(other.into_error()),
    }
}

/// Marks `overdue`, as the system, every open task due before `today`
/// (only those of `only`, when given); held tasks too. Returns the ids.
async fn overdue_now(
    state: &AppState,
    today: NaiveDate,
    only: Option<&Subject>,
) -> Result<Vec<i64>> {
    let mut query = DeadlineTask::query().where_(DeadlineTask::STATUS.is_in(NOT_YET_OVERDUE));
    if let Some(source) = only {
        query = query
            .where_(DeadlineTask::SUBJECT_TYPE.eq(source.subject_type.as_str()))
            .where_(DeadlineTask::SUBJECT_ID.eq(source.subject_id));
    }
    let tasks = query.order_by(DeadlineTask::ID).get(&state.db).await?;
    let gathered = input(state, today);
    let mut marked = Vec::new();
    for task in tasks.iter().filter(|t| t.due_date < today) {
        match task
            .transition(MARK_OVERDUE)
            .by_system()
            .input(&gathered)
            .run(state)
            .await
        {
            Ok(_) => marked.push(task.id),
            // Moved meanwhile.
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    Ok(marked)
}

/// Runs `WRK.RUL.05` for `today` (Asia/Dubai): one rule run per date, so a
/// second run for the same date does nothing and a failed one is retried.
/// The run is recorded under the client's rule id
/// ([`WlTicketing::overdue_rule_id`](crate::WlTicketing::overdue_rule_id);
/// 04: `ACC.RUL.04`), so a vertical that marks that id compliance-critical
/// gets the owner alert when it fails or is missed. Returns how many tasks
/// went overdue.
///
/// # Errors
///
/// The database's errors.
pub async fn mark_overdue(state: &AppState, today: NaiveDate) -> Result<usize> {
    let rule = settings(state).overdue_rule;
    let spec = RuleRunSpec::new(&rule, schedule_subject(), occurrence_key(&rule, today));
    let seen = Arc::new(Mutex::new(0usize));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let marked = overdue_now(&state, today, None).await?;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = marked.len();
        Ok(())
    })
    .await?;
    let marked = *seen.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(marked)
}

/// What a reminder run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReminderReport {
    /// Reminders sent to staff (owners).
    pub staff: usize,
    /// Reminders sent to clients.
    pub client: usize,
    /// Client reminders not sent because the task is held.
    pub held: usize,
}

/// The staff users who own `task`: its owner, else the holders of its owner
/// role in the source's scope.
async fn owners(state: &AppState, task: &DeadlineTask) -> Result<Vec<i64>> {
    if let Some(user) = task.owner_user_id {
        return Ok(vec![user]);
    }
    match &task.owner_role {
        Some(role) => holders_in(state, &task.source(), role).await,
        None => Ok(Vec::new()),
    }
}

/// The data of a task's notices.
fn notice_data(task: &DeadlineTask) -> Value {
    json!({
        "title": task.title,
        "task_type": task.task_type,
        "due_date": task.due_date.to_string(),
        "start_date": task.start_date.to_string(),
        "source": task.source().to_string(),
    })
}

/// Sends `type_key` about `task` to `users`, when there are any.
async fn tell_users(
    state: &AppState,
    type_key: &str,
    task: &DeadlineTask,
    users: &[i64],
) -> Result<usize> {
    if users.is_empty() {
        return Ok(0);
    }
    let mut request = NotifyRequest::new(type_key, task.subject()).data(notice_data(task));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(users.len())
}

/// Reminds of the tasks whose start date is reached (`upcoming`, once) or
/// that are due in one of the reminder days: the owner always, the client
/// when the task needs their input or waits on them, unless it is held.
async fn remind_now(state: &AppState, today: NaiveDate, report: &mut ReminderReport) -> Result<()> {
    let settings = settings(state);
    let tasks = DeadlineTask::query()
        .where_(DeadlineTask::STATUS.is_in(NOT_YET_OVERDUE))
        .order_by(DeadlineTask::ID)
        .get(&state.db)
        .await?;
    for task in &tasks {
        let start_reached = task.status == DeadlineTaskStatus::Upcoming
            && task.start_date <= today
            && task.start_notified_on.is_none();
        let days_left = (task.due_date - today).num_days();
        let due_soon = settings.reminder_days.contains(&days_left);
        if !start_reached && !due_soon {
            continue;
        }
        let staff = owners(state, task).await?;
        report.staff += tell_users(state, REMINDER_NOTICE, task, &staff).await?;
        let wants_client = task.needs_client_input || task.status == DeadlineTaskStatus::Waiting;
        if let Some(party) = task.party()
            && wants_client
        {
            if task.held {
                report.held += 1;
            } else {
                let request = NotifyRequest::new(CLIENT_REMINDER_NOTICE, task.subject())
                    .to_party(party)
                    .data(notice_data(task));
                notify::notifier(state).notify(state, request).await?;
                report.client += 1;
            }
        }
        if start_reached {
            DeadlineTask::query()
                .where_(DeadlineTask::ID.eq(task.id))
                .update(&state.db, &[("start_notified_on", &Some(today))])
                .await?;
        }
    }
    Ok(())
}

/// Runs `WRK.RUL.06` for `today` (Asia/Dubai): one rule run per date.
///
/// # Errors
///
/// The database's errors.
pub async fn remind(state: &AppState, today: NaiveDate) -> Result<ReminderReport> {
    let spec = RuleRunSpec::new(
        REMINDER_RULE,
        schedule_subject(),
        occurrence_key(REMINDER_RULE, today),
    );
    let seen = Arc::new(Mutex::new(ReminderReport::default()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let mut report = ReminderReport::default();
        let result = remind_now(&state, today, &mut report).await;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = report;
        result
    })
    .await?;
    let report = *seen.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(report)
}

/// Sets the held flag on every open task of `source`.
async fn set_held(db: &Db, source: &Subject, held: bool) -> Result<()> {
    DeadlineTask::query()
        .where_(DeadlineTask::SUBJECT_TYPE.eq(source.subject_type.as_str()))
        .where_(DeadlineTask::SUBJECT_ID.eq(source.subject_id))
        .where_(DeadlineTask::STATUS.is_in(OPEN))
        .update(db, &[("held", &held)])
        .await?;
    Ok(())
}

/// The summaries of the tasks of `source`.
async fn summaries(db: &Db, source: &Subject) -> Result<Vec<DeadlineTaskSummary>> {
    Ok(DeadlineTask::of_source(db, source)
        .await?
        .iter()
        .map(DeadlineTask::summary)
        .collect())
}

/// Holds the tasks of `source` from `on` (§4.4 "Held tasks"): the flag on
/// every open task, state and due date unchanged; tasks created later are
/// created held. Nothing for a source held already or ended.
///
/// # Errors
///
/// The database's errors.
pub async fn hold(
    state: &AppState,
    source: &Subject,
    on: NaiveDate,
) -> Result<Vec<DeadlineTaskSummary>> {
    let row = TaskSource::of_or_new(&state.db, source).await?;
    if row.ended_on.is_none() {
        if row.held_on.is_none() {
            TaskSource::query()
                .where_(TaskSource::ID.eq(row.id))
                .update(&state.db, &[("held_on", &Some(on))])
                .await?;
        }
        set_held(&state.db, source, true).await?;
    }
    summaries(&state.db, source).await
}

/// `VAT Q3 (overdue since 2026-11-29)`.
fn overdue_line(task: &DeadlineTaskSummary) -> String {
    let since = task
        .overdue_on
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    english("wl_ticketing.task.overdue_line")
        .replace(":title", &task.title)
        .replace(":since", &since)
}

/// Opens (or finds open) the manager's task about `source` keyed `key`.
async fn manager_task(
    state: &AppState,
    source: &Subject,
    key: &str,
    title: &str,
    lines: &[String],
) -> Result<Option<TicketRef>> {
    let role = settings(state).manager_role;
    let task = RoleTask::new(source.clone(), &role, title, key).description(&lines.join("\n"));
    role_tasks::open(state, task).await
}

/// Releases the tasks of `source` on `on` (§4.4 "Held tasks"): past-due
/// open tasks go `overdue`, the flag clears, and the tasks that went overdue
/// while held are listed once for the manager. Nothing for a source that is
/// not held.
///
/// # Errors
///
/// The database's errors.
pub async fn release(state: &AppState, source: &Subject, on: NaiveDate) -> Result<ReleasedTasks> {
    let Some(row) = TaskSource::of(&state.db, source).await? else {
        return Ok(ReleasedTasks::default());
    };
    let Some(held_on) = row.held_on else {
        return Ok(ReleasedTasks::default());
    };
    if row.ended_on.is_some() {
        return Ok(ReleasedTasks::default());
    }
    overdue_now(state, on, Some(source)).await?;
    let released: Vec<DeadlineTaskId> = DeadlineTask::of_source(&state.db, source)
        .await?
        .iter()
        .filter(|t| t.held)
        .map(DeadlineTask::task_id)
        .collect();
    set_held(&state.db, source, false).await?;
    TaskSource::query()
        .where_(TaskSource::ID.eq(row.id))
        .update(&state.db, &[("held_on", &None::<NaiveDate>)])
        .await?;
    let overdue: Vec<DeadlineTaskSummary> = summaries(&state.db, source)
        .await?
        .into_iter()
        .filter(|t| t.status == DeadlineTaskStatus::Overdue)
        .filter(|t| t.overdue_on.is_some_and(|d| d >= held_on))
        .collect();
    let manager_task = if overdue.is_empty() {
        None
    } else {
        let lines: Vec<String> = overdue.iter().map(overdue_line).collect();
        let title = english("wl_ticketing.task.released_title").to_owned();
        let key = format!("ticketing.tasks_released:{source}:{held_on}");
        manager_task(state, source, &key, &title, &lines).await?
    };
    Ok(ReleasedTasks {
        released,
        overdue,
        manager_task,
    })
}

/// `CT return (in_progress)`.
fn open_line(task: &DeadlineTask) -> String {
    english("wl_ticketing.task.open_line")
        .replace(":title", &task.title)
        .replace(":status", task.status.as_str())
}

/// The source agreement ended on `on` (BL-TKT-09): no next occurrence from
/// now on, `upcoming` tasks (and started ones when the setting is on) waived
/// with "source agreement ended", the held flag cleared without a release
/// list, and the other open tasks listed for the manager on one task.
/// Nothing for a source that ended already.
///
/// # Errors
///
/// The database's errors.
pub async fn end_source(state: &AppState, source: &Subject, on: NaiveDate) -> Result<EndedTasks> {
    let row = TaskSource::of_or_new(&state.db, source).await?;
    if row.ended_on.is_some() {
        return Ok(EndedTasks::default());
    }
    TaskSource::query()
        .where_(TaskSource::ID.eq(row.id))
        .update(
            &state.db,
            &[("ended_on", &Some(on)), ("held_on", &None::<NaiveDate>)],
        )
        .await?;
    set_held(&state.db, source, false).await?;
    let waive_started = settings(state).waive_started;
    let waive = WaiveTask::new(SOURCE_ENDED_REASON).without_next();
    let mut ended = EndedTasks::default();
    let mut left = Vec::new();
    for task in DeadlineTask::of_source(&state.db, source).await? {
        if task.status.is_terminal() {
            continue;
        }
        let waivable = task.status == DeadlineTaskStatus::Upcoming
            || (waive_started && task.status.is_started());
        if waivable && waive_as_system(state, &task, &waive, on).await? {
            ended.waived.push(task.task_id());
            continue;
        }
        ended.listed.push(task.task_id());
        left.push(open_line(&task));
    }
    if !left.is_empty() {
        let title = english("wl_ticketing.task.ended_title").to_owned();
        let key = format!("ticketing.source_ended:{source}");
        ended.manager_task = manager_task(state, source, &key, &title, &left).await?;
    }
    Ok(ended)
}

/// After a transition commits: the next occurrence, the events and the
/// notices.
async fn after(state: &AppState, event: &Transitioned<DeadlineTask>) -> Result<()> {
    let Some(task) = DeadlineTask::find(&state.db, event.key).await? else {
        return Ok(());
    };
    let name = event.transition.as_str();
    match name {
        MARK_OVERDUE => {
            // The owners, the manager role and the other roles told (04:
            // the assignee, the manager and the partner), each once.
            let mut users = owners(state, &task).await?;
            let settings = settings(state);
            users.extend(holders_in(state, &task.source(), &settings.manager_role).await?);
            for role in &settings.overdue_roles {
                users.extend(holders_in(state, &task.source(), role).await?);
            }
            users.sort_unstable();
            users.dedup();
            tell_users(state, OVERDUE_NOTICE, &task, &users).await?;
            state
                .emit(TaskOverdue {
                    task: task.task_id(),
                    source: task.source(),
                    owner: task.owner_user_id,
                    due_date: task.due_date,
                    held: task.held,
                })
                .await?;
        }
        COMPLETE | COMPLETE_REVIEWED => {
            state
                .emit(TaskCompleted {
                    task: task.task_id(),
                    source: task.source(),
                    period_end: task.period_end,
                    late: task.late,
                })
                .await?;
            next_occurrence(state, &task).await?;
        }
        CLOSE_MISSED => {
            next_occurrence(state, &task).await?;
        }
        WAIVE | AUTO_WAIVE => {
            let started = matches!(event.from.as_str(), "in_progress" | "waiting");
            if started {
                // The work recorded so far stays on the task; the assignee
                // and the counterpart are told.
                let staff = owners(state, &task).await?;
                let mut request =
                    NotifyRequest::new(WAIVED_NOTICE, task.subject()).data(notice_data(&task));
                for user in &staff {
                    request = request.to_user(*user);
                }
                if let Some(party) = task.party() {
                    request = request.to_party(party);
                }
                if !request.recipients.is_empty() {
                    notify::notifier(state).notify(state, request).await?;
                }
            }
            let no_next = event
                .meta
                .get("no_next")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !no_next {
                next_occurrence(state, &task).await?;
            }
        }
        WAIT => {
            if let Some(party) = task.party()
                && !task.held
            {
                let request = NotifyRequest::new(WAITING_NOTICE, task.subject())
                    .to_party(party)
                    .data(notice_data(&task));
                notify::notifier(state).notify(state, request).await?;
            }
        }
        UNBLOCK | AUTO_UNBLOCK => {
            let staff = owners(state, &task).await?;
            tell_users(state, UNBLOCKED_NOTICE, &task, &staff).await?;
        }
        SUBMIT_REVIEW => {
            if let Some(role) = &task.reviewer_role {
                let reviewers = holders_in(state, &task.source(), role).await?;
                tell_users(state, REVIEW_NOTICE, &task, &reviewers).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Registers the machine's listener, the notices, the nightly
/// `mark_overdue`, the daily reminders and their commands.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    app.listen(
        |event: Transitioned<DeadlineTask>, state: AppState| async move {
            after(&state, &event).await
        },
    );
    app.schedule()
        .daily_at(OVERDUE_AT, OVERDUE_TASK, |state: AppState| async move {
            mark_overdue(&state, wl_kernel::documents::today())
                .await
                .map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        OVERDUE_TASK,
        "Mark overdue the deadline tasks whose due date passed (WRK.RUL.05)",
        |_args, state: AppState| async move {
            let marked = mark_overdue(&state, wl_kernel::documents::today()).await?;
            println!("{marked} tasks marked overdue.");
            Ok(())
        },
    );
    app.schedule()
        .daily_at(REMINDER_AT, REMINDER_TASK, |state: AppState| async move {
            remind(&state, wl_kernel::documents::today())
                .await
                .map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        REMINDER_TASK,
        "Remind owners and clients of the deadline tasks starting or falling due (WRK.RUL.06)",
        |_args, state: AppState| async move {
            let report = remind(&state, wl_kernel::documents::today()).await?;
            println!(
                "{} staff and {} client reminders sent; {} held.",
                report.staff, report.client, report.held
            );
            Ok(())
        },
    );
}

/// The module's [`DeadlineTasks`] for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct Deadlines;

impl DeadlineTasks for Deadlines {
    fn create<'a>(
        &'a self,
        state: &'a AppState,
        task: NewDeadlineTask,
    ) -> DeadlineFuture<'a, Option<DeadlineTaskId>> {
        Box::pin(async move { Ok(Some(create(state, task).await?.task_id())) })
    }

    fn find<'a>(
        &'a self,
        db: &'a Db,
        id: DeadlineTaskId,
    ) -> DeadlineFuture<'a, Option<DeadlineTaskSummary>> {
        Box::pin(async move {
            Ok(DeadlineTask::find(db, id.0)
                .await?
                .map(|task| task.summary()))
        })
    }

    fn of_subject<'a>(
        &'a self,
        db: &'a Db,
        source: &'a Subject,
    ) -> DeadlineFuture<'a, Vec<DeadlineTaskSummary>> {
        Box::pin(async move { summaries(db, source).await })
    }

    fn hold<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> DeadlineFuture<'a, Vec<DeadlineTaskSummary>> {
        Box::pin(async move { hold(state, source, on).await })
    }

    fn release<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> DeadlineFuture<'a, ReleasedTasks> {
        Box::pin(async move { release(state, source, on).await })
    }

    fn waive<'a>(
        &'a self,
        state: &'a AppState,
        id: DeadlineTaskId,
        waive: WaiveTask,
    ) -> DeadlineFuture<'a, bool> {
        Box::pin(async move {
            let Some(task) = DeadlineTask::find(&state.db, id.0).await? else {
                return Ok(false);
            };
            waive_as_system(state, &task, &waive, wl_kernel::documents::today()).await
        })
    }

    fn end_source<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
        on: NaiveDate,
    ) -> DeadlineFuture<'a, EndedTasks> {
        Box::pin(async move { end_source(state, source, on).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_default_to_the_module_values() {
        let settings = Settings::default();
        assert!(settings.waive_started);
        assert_eq!(settings.manager_role, COMPANY_ADMIN_ROLE);
        assert!(settings.reminder_days.is_empty());
        assert_eq!(settings.overdue_rule, OVERDUE_RULE);
        assert!(settings.overdue_roles.is_empty());
    }

    fn overdue_task() -> DeadlineTask {
        DeadlineTask {
            title: "VAT Q3".into(),
            subject_type: "accounting.engagement".into(),
            subject_id: 12,
            overdue_on: NaiveDate::from_ymd_opt(2026, 11, 29),
            status: DeadlineTaskStatus::Overdue,
            ..DeadlineTask::default()
        }
    }

    #[test]
    fn the_open_line_names_the_task_and_its_state() {
        assert_eq!(open_line(&overdue_task()), "VAT Q3 (overdue)");
    }

    #[test]
    fn the_overdue_line_names_the_day_it_went_overdue() {
        assert_eq!(
            overdue_line(&overdue_task().summary()),
            "VAT Q3 (overdue since 2026-11-29)"
        );
    }
}
