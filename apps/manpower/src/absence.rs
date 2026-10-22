//! A worker who has not turned up: the absence case (09 Manpower §4.7 row
//! *The worker absconds*, §7.2 `MPW.RUL.09`, §13 question 8; BL-MPW-17).
//!
//! `MPW.RUL.09` runs daily at 02:00 Asia/Dubai ([`daily`]). An employee `active`
//! in HR with an `active` deployment (a worker on the bench, or whose
//! deployments are all suspended, has no attendance expected) who has had no
//! attendance and no approved leave for more than N consecutive **calendar**
//! days (default 7, [`DEFAULT_ABSENCE_DAYS`]) gets an **absence case**:
//!
//! - Day 0 is the last day with attendance (the kernel's `AttendanceHistory`) or
//!   approved leave (`LeaveCalendar`), or the day of the deployment's `activate`
//!   or `resume` if later; the case opens at the first run after day N (day 0
//!   2026-10-31: day 7 is 2026-11-07, the case opens on the run of 2026-11-08).
//! - One open case per employee; a second trigger does nothing. The case is a
//!   ticket of the category `absence` for HR (`crate::seed::ABSENCE_CATEGORY`,
//!   linked to the employee, the deployment as its source) and a row
//!   ([`AbsenceCase`]). [`AbsenceCaseOpened`] tells wl-hr, which holds the
//!   worker's next payroll line while the case is open (`HRM.RUL.12`) and needs
//!   the case for `report_absconding`.
//! - The deployment is **not** stopped. The contract's cover obligation opens a
//!   replacement request from the day the case opens, open-ended
//!   ([`crate::cover::open_cover`], cause `absence`), and the client is told
//!   *<worker> has not turned up at <site> since <day>*, no reason.
//! - HR is reminded **daily** until two failed attempts to reach the worker, on
//!   different days after day 0, are recorded on the employee (wl-hr; read
//!   through the kernel's `Workforce::failed_contact_days`) and the MoHRE absence
//!   report number is recorded on the case ([`record_report`]). Then
//!   `report_absconding` can pass wl-hr's own guard (HR still requests it; this
//!   rule never reports a worker as absconded).
//! - When the worker returns HR closes the case with a reason ([`close`],
//!   `absence.manage`): the ticket is completed, [`AbsenceCaseClosed`] makes wl-hr
//!   release the held lines (`HRM.RUL.12` proposes an off-cycle run, prepared and
//!   approved like any run), and the cover request of the case is cancelled
//!   while still unfilled (*worker returned*); a cover deployment runs to its
//!   own end date. A worker reinstated after an absence report
//!   (`EmployeeReinstated`) closes the case the same way.
//! - **A rule run** is one compliance-critical `run_rule` per employee and day 0
//!   (`MPW.RUL.09:<employee>:<day 0>`): a failed run is tried again by the next
//!   daily run, and after five attempts the MD and the company admin get the
//!   kernel's alert (`PLT.RUL.04`). The follow-up after the case row exists
//!   (the event, the cover, the client) is claimed once (`announced_at`), so a
//!   failed one is finished by the retry and a done one never repeats.

use std::collections::BTreeMap;

use renox::chrono::{Days, NaiveDate};
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::attendance::attendance_history;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::documents::today;
use wl_kernel::role_tasks;
use wl_kernel::tickets::{Assignee, OpenTicket, TaskComment, TicketId, task_creator};
use wl_kernel::workforce::{
    AbsenceCaseClosed, AbsenceCaseOpened, EMPLOYEE_SUBJECT_TYPE, EmployeeId, EmployeeReinstated,
    leave_calendar, workforce,
};

use crate::config::DEFAULT_ABSENCE_DAYS;
use crate::contract::ClientContract;
use crate::cover::{self, CoverAsk, causes};
use crate::db::{answer, refused};
use crate::deployment::{Deployment, DeploymentStatus, SUBJECT_TYPE as DEPLOYMENT};
use crate::deployments::{announce_all_cancelled, cancel_planned_of_request, worker_name};
use crate::notices;
use crate::permissions;
use crate::request::{self, ManpowerRequest, RequestStatus};
use crate::seed::{ABSENCE_CATEGORY, ABSENCE_ROLE};
use crate::site::Site;
use crate::texts::english;

/// The rule id.
pub const RULE: &str = "MPW.RUL.09";

/// The subject type of an absence case without a ticket.
pub const SUBJECT_TYPE: &str = "manpower.absence_case";

/// The reason a cover request of an absence is cancelled with.
pub const WORKER_RETURNED: &str = "worker returned";

/// Failed attempts to reach the worker, on different days, that `report_absconding`
/// needs (ER Art. 28).
pub const FAILED_ATTEMPTS: usize = 2;

/// How far back attendance and leave are looked at, in calendar days.
const LOOKBACK_DAYS: u64 = 60;

/// The absence case of a worker.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manpower_absence_cases", index(employee_id))]
pub struct AbsenceCase {
    /// The primary key.
    pub id: i64,
    /// The wl-hr employee.
    pub employee_id: i64,
    /// The deployment the case was opened for.
    pub deployment_id: i64,
    /// The ticket of category `absence`; none when ticketing is off.
    pub ticket_id: Option<i64>,
    /// Day 0: the last day with attendance or approved leave, or the day of
    /// `activate` or `resume`.
    pub last_present_on: NaiveDate,
    /// The day the case was opened.
    pub opened_on: NaiveDate,
    /// The MoHRE absence report number HR recorded.
    pub report_number: Option<String>,
    /// The MoHRE absence report date.
    pub report_date: Option<NaiveDate>,
    /// The day of the last reminder to HR.
    pub reminded_on: Option<NaiveDate>,
    /// When the event, the cover and the client's notice were done; none while
    /// still to do.
    pub announced_at: Option<DateTime>,
    /// The day the case was closed; none while open.
    pub closed_on: Option<NaiveDate>,
    /// Why it was closed.
    pub close_reason: Option<String>,
    /// The user who closed it; none when the system did.
    pub closed_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AbsenceCase {
    /// The case as a [`Subject`]: the ticket when there is one, else
    /// `manpower.absence_case:3`. It is what wl-hr keeps in its own case.
    pub fn subject(&self) -> Subject {
        match self.ticket_id {
            Some(ticket) => TicketId(ticket).subject(),
            None => Subject::new(SUBJECT_TYPE, self.id),
        }
    }

    /// The employee as an [`EmployeeId`].
    pub fn employee(&self) -> EmployeeId {
        EmployeeId(self.employee_id)
    }

    /// Whether the case is still open.
    pub fn is_open(&self) -> bool {
        self.closed_on.is_none()
    }

    /// The open case of `employee`, if any.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_of(db: &Db, employee: i64) -> Result<Option<AbsenceCase>> {
        AbsenceCase::query()
            .where_(AbsenceCase::EMPLOYEE_ID.eq(employee))
            .where_(AbsenceCase::CLOSED_ON.is_null())
            .order_by(AbsenceCase::ID)
            .first(db)
            .await
    }
}

/// What a daily run did.
#[derive(Debug, Default)]
pub struct Run {
    /// The cases opened (or finished) now.
    pub opened: Vec<AbsenceCase>,
    /// The cases HR was reminded about now.
    pub reminded: Vec<AbsenceCase>,
}

/// Day 0 of the absence of `employee` on `day`: the latest of the last day
/// with attendance, the last day inside an approved leave and the day the
/// deployments were activated or resumed, counting back from yesterday.
async fn day_zero(
    state: &AppState,
    employee: EmployeeId,
    deployments: &[Deployment],
    day: NaiveDate,
) -> Result<Option<NaiveDate>> {
    let Some(yesterday) = day.checked_sub_days(Days::new(1)) else {
        return Ok(None);
    };
    let from = day
        .checked_sub_days(Days::new(LOOKBACK_DAYS))
        .unwrap_or(yesterday);
    let mut base = deployments
        .iter()
        .map(|deployment| {
            deployment
                .resumed_on
                .into_iter()
                .chain(deployment.activated_on)
                .chain(Some(deployment.start_date))
                .max()
                .unwrap_or(deployment.start_date)
        })
        .max();
    let attended = attendance_history(state)
        .attended_days(state, employee, from, yesterday)
        .await?;
    base = base.max(attended.last().copied());
    let leave = leave_calendar(state)
        .approved_leave(&state.db, employee, from, yesterday)
        .await?;
    for (start, end) in leave {
        if start <= yesterday {
            base = base.max(Some(end.min(yesterday)));
        }
    }
    Ok(base)
}

/// The users who look after absence cases: the HR role.
async fn hr_users(state: &AppState) -> Result<Vec<i64>> {
    Ok(role_tasks::assignee(&state.db, ABSENCE_ROLE)
        .await?
        .into_iter()
        .collect())
}

/// The case of `employee` for the spell that began after `last_present`, made
/// when there is none: the ticket, then the row.
async fn ensure_case(
    state: &AppState,
    employee: EmployeeId,
    deployment: &Deployment,
    last_present: NaiveDate,
    day: NaiveDate,
) -> Result<AbsenceCase> {
    if let Some(open) = AbsenceCase::open_of(&state.db, employee.0).await? {
        return Ok(open);
    }
    let worker = worker_name(state, "en", employee).await;
    let site = Site::find(&state.db, deployment.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default();
    let days = (day - last_present).num_days();
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 5] = [
        ("worker", &worker),
        ("site", &site),
        ("days", &days),
        ("last", &last_present),
        ("deployment", &deployment.id),
    ];
    let mut ticket = OpenTicket::new(
        Subject::new(EMPLOYEE_SUBJECT_TYPE, employee.0),
        ABSENCE_CATEGORY,
        &english("manpower.absence.ticket_title", &params),
    )
    .description(&english("manpower.absence.ticket_body", &params))
    .source(deployment.subject())
    .source_key(&format!("{RULE}:{}:{last_present}", employee.0));
    if let Some(user) = role_tasks::assignee(&state.db, ABSENCE_ROLE).await? {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    let creator = task_creator(state);
    let opened = match creator
        .find_open(state, &format!("{RULE}:{}:{last_present}", employee.0))
        .await?
    {
        Some(found) => Some(found),
        None => creator.open(state, ticket).await?,
    };
    AbsenceCase::create(
        &state.db,
        AbsenceCase {
            employee_id: employee.0,
            deployment_id: deployment.id,
            ticket_id: opened.map(|ticket| ticket.id.0),
            last_present_on: last_present,
            opened_on: day,
            ..AbsenceCase::default()
        },
    )
    .await
}

/// The follow-up of a new case, once: wl-hr is told, the cover the contract
/// requires opens for each active deployment, the client is told the worker
/// is absent, and HR gets the first reminder. A failure gives the claim back.
async fn announce(state: &AppState, id: i64, day: NaiveDate) -> Result {
    let claimed = AbsenceCase::query()
        .where_(AbsenceCase::ID.eq(id))
        .where_(AbsenceCase::ANNOUNCED_AT.is_null())
        .update(&state.db, &[("announced_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        return Ok(());
    }
    match follow_up(state, id, day).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let none: Option<DateTime> = None;
            AbsenceCase::query()
                .where_(AbsenceCase::ID.eq(id))
                .update(&state.db, &[("announced_at", &none)])
                .await?;
            Err(error)
        }
    }
}

/// The steps of [`announce`].
async fn follow_up(state: &AppState, id: i64, day: NaiveDate) -> Result {
    let case = AbsenceCase::find_or_404(&state.db, id).await?;
    state
        .emit(AbsenceCaseOpened {
            employee: case.employee(),
            case: case.subject(),
            opened_on: case.opened_on,
            last_present_on: case.last_present_on,
        })
        .await?;
    let worker = worker_name(state, "en", case.employee()).await;
    let deployments = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(case.employee_id))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Active))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    for deployment in &deployments {
        cover::open_cover(
            state,
            CoverAsk {
                deployment,
                from: case.opened_on,
                to: None,
                cause: causes::ABSENCE,
                cause_ref: Some(cover_ref(deployment.id, case.last_present_on)),
            },
        )
        .await?;
        let site = Site::find(&state.db, deployment.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        let since = case.last_present_on;
        let params: [(&str, &(dyn std::fmt::Display + Sync)); 3] =
            [("worker", &worker), ("site", &site), ("since", &since)];
        let contract =
            ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
        notices::tell_client(
            state,
            deployment.subject(),
            &contract.reference,
            deployment.client(),
            &english("manpower.absence.client_title", &params),
            &english("manpower.absence.client_body", &params),
        )
        .await?;
    }
    remind(state, &case, day).await?;
    Ok(())
}

/// The `cause_ref` of the cover request of an absence.
fn cover_ref(deployment: i64, last_present: NaiveDate) -> String {
    format!("{DEPLOYMENT}:{deployment}:absence:{last_present}")
}

/// What HR still has to record for `case`: the failed attempts (days
/// recorded, of [`FAILED_ATTEMPTS`]) and the MoHRE report number.
async fn missing(state: &AppState, case: &AbsenceCase) -> Result<(usize, bool)> {
    let tried = workforce(state)
        .failed_contact_days(&state.db, case.employee(), case.last_present_on)
        .await?
        .len();
    Ok((tried, case.report_number.is_none()))
}

/// Reminds HR about `case` on `day` unless it did today or nothing is missing.
/// Returns whether a reminder went out.
async fn remind(state: &AppState, case: &AbsenceCase, day: NaiveDate) -> Result<bool> {
    if case.reminded_on == Some(day) {
        return Ok(false);
    }
    let (tried, no_number) = missing(state, case).await?;
    if tried >= FAILED_ATTEMPTS && !no_number {
        return Ok(false);
    }
    let claimed = {
        let query = AbsenceCase::query().where_(AbsenceCase::ID.eq(case.id));
        let query = match case.reminded_on {
            Some(was) => query.where_(AbsenceCase::REMINDED_ON.eq(was)),
            None => query.where_(AbsenceCase::REMINDED_ON.is_null()),
        };
        query
            .update(&state.db, &[("reminded_on", &Some(day))])
            .await?
    };
    if claimed == 0 {
        return Ok(false);
    }
    let worker = worker_name(state, "en", case.employee()).await;
    let days = (day - case.last_present_on).num_days();
    let done = tried.min(FAILED_ATTEMPTS);
    let need = FAILED_ATTEMPTS;
    let number = if no_number {
        english("manpower.absence.number_missing", &[])
    } else {
        english("manpower.absence.number_recorded", &[])
    };
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 5] = [
        ("worker", &worker),
        ("days", &days),
        ("done", &done),
        ("need", &need),
        ("number", &number),
    ];
    let users = hr_users(state).await?;
    notices::tell(
        state,
        case.subject(),
        &worker,
        &users,
        &english("manpower.absence.reminder_title", &params),
        &english("manpower.absence.reminder_body", &params),
    )
    .await?;
    Ok(true)
}

/// One run of the rule for `employee`: opens the case for the spell whose day
/// 0 is `last_present`.
async fn open_one(
    state: &AppState,
    employee: EmployeeId,
    deployments: &[Deployment],
    last_present: NaiveDate,
    day: NaiveDate,
) -> Result {
    let spec = RuleRunSpec::new(
        RULE,
        Subject::new(EMPLOYEE_SUBJECT_TYPE, employee.0),
        format!("{RULE}:{}:{last_present}", employee.0),
    )
    .compliance_critical();
    let first = deployments.first().cloned();
    run_rule(state, spec, move |state: AppState| async move {
        let Some(deployment) = first else {
            return Ok(());
        };
        let case = ensure_case(&state, employee, &deployment, last_present, day).await?;
        announce(&state, case.id, day).await
    })
    .await?;
    Ok(())
}

/// The rule on `day`: opens the cases that are due, finishes the follow-up of
/// a case a failed run left, and reminds HR about the open cases.
///
/// # Errors
///
/// The database's, ticketing's and notifier's errors; the rest of the run goes
/// on and the first error is returned at the end.
pub async fn run_on(state: &AppState, day: NaiveDate) -> Result<Run> {
    let mut run = Run::default();
    let mut failure = None;
    let active = Deployment::query()
        .where_(Deployment::STATUS.eq(DeploymentStatus::Active))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    let mut by_employee: BTreeMap<i64, Vec<Deployment>> = BTreeMap::new();
    for deployment in active {
        by_employee
            .entry(deployment.employee_id)
            .or_default()
            .push(deployment);
    }
    for (employee, deployments) in by_employee {
        let employee = EmployeeId(employee);
        if !workforce(state).is_active(state, employee).await? {
            continue;
        }
        if let Some(open) = AbsenceCase::open_of(&state.db, employee.0).await? {
            // A case a failed run left without its follow-up: the same rule
            // run, tried again (and counted towards the alert).
            if open.announced_at.is_none()
                && let Err(error) =
                    open_one(state, employee, &deployments, open.last_present_on, day).await
            {
                failure.get_or_insert(error);
            }
            continue;
        }
        let Some(last_present) = day_zero(state, employee, &deployments, day).await? else {
            continue;
        };
        if (day - last_present).num_days() <= DEFAULT_ABSENCE_DAYS {
            continue;
        }
        if let Err(error) = open_one(state, employee, &deployments, last_present, day).await {
            eprintln!("{RULE}: employee {}: {error:?}", employee.0);
            failure.get_or_insert(error);
            continue;
        }
        if let Some(case) = AbsenceCase::open_of(&state.db, employee.0).await? {
            run.opened.push(case);
        }
    }
    let open = AbsenceCase::query()
        .where_(AbsenceCase::CLOSED_ON.is_null())
        .order_by(AbsenceCase::ID)
        .get(&state.db)
        .await?;
    for case in open {
        if !workforce(state).is_active(state, case.employee()).await? {
            continue;
        }
        match remind(state, &case, day).await {
            Ok(true) => run.reminded.push(case),
            Ok(false) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(run), Err)
}

/// The daily run at 02:00 Asia/Dubai.
///
/// # Errors
///
/// As [`run_on`].
pub async fn daily(state: &AppState) -> Result {
    run_on(state, today()).await?;
    Ok(())
}

/// `Forbidden` unless `by` holds `absence.manage`.
pub(crate) fn require_manage(by: &AuthUser) -> Result {
    if by.has_permission(permissions::ABSENCE_MANAGE) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// HR (`by`) records the MoHRE absence report of case `id`: its number and
/// date (not in the future). A later call replaces them. The ticket gets an
/// internal comment.
///
/// # Errors
///
/// `Forbidden` without `absence.manage`; a 422 for a blank number, a date in
/// the future or a case that is closed.
pub async fn record_report(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    number: &str,
    date: NaiveDate,
) -> Result<AbsenceCase> {
    require_manage(by)?;
    let case = AbsenceCase::find_or_404(&state.db, id).await?;
    if !case.is_open() {
        return Err(refused(english("manpower.absence.closed", &[])));
    }
    let number = number.trim();
    if number.is_empty() {
        return Err(refused(english("manpower.absence.report_number", &[])));
    }
    if date > today() {
        return Err(refused(english("manpower.absence.report_date", &[])));
    }
    AbsenceCase::query()
        .where_(AbsenceCase::ID.eq(id))
        .update(
            &state.db,
            &[
                ("report_number", &Some(number.to_owned())),
                ("report_date", &Some(date)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    if let Some(ticket) = case.ticket_id {
        task_creator(state)
            .comment(
                state,
                TicketId(ticket),
                TaskComment::internal(&english(
                    "manpower.absence.report_comment",
                    &[("number", &number), ("date", &date)],
                )),
            )
            .await?;
    }
    AbsenceCase::find_or_404(&state.db, id).await
}

/// Closes case `id` on `day` with `reason`: the row, the ticket, wl-hr
/// (`AbsenceCaseClosed`: the held lines are released) and the cover request of
/// the case while unfilled. Returns the case; closing a closed case changes
/// nothing.
async fn close_case(
    state: &AppState,
    id: i64,
    reason: &str,
    by: Option<i64>,
    day: NaiveDate,
) -> Result<AbsenceCase> {
    let claimed = AbsenceCase::query()
        .where_(AbsenceCase::ID.eq(id))
        .where_(AbsenceCase::CLOSED_ON.is_null())
        .update(
            &state.db,
            &[
                ("closed_on", &Some(day)),
                ("close_reason", &Some(reason.to_owned())),
                ("closed_by", &by),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let case = AbsenceCase::find_or_404(&state.db, id).await?;
    if claimed == 0 {
        return Ok(case);
    }
    if let Some(ticket) = case.ticket_id {
        task_creator(state)
            .complete(state, TicketId(ticket), reason)
            .await?;
    }
    state
        .emit(AbsenceCaseClosed {
            employee: case.employee(),
            case: case.subject(),
            closed_on: day,
            reason: reason.to_owned(),
        })
        .await?;
    cancel_cover(state, &case).await?;
    Ok(case)
}

/// Cancels the cover requests the case opened while they are `open` or
/// `partially_filled`, with the planned deployments filling them. A cover
/// deployment already `active` runs to its own end date.
async fn cancel_cover(state: &AppState, case: &AbsenceCase) -> Result {
    let cause_ref = cover_ref(case.deployment_id, case.last_present_on);
    let mut tx = state.db.begin().await?;
    let requests = ManpowerRequest::query()
        .where_(ManpowerRequest::CAUSE_REF.eq(cause_ref))
        .where_(
            ManpowerRequest::STATUS.is_in([RequestStatus::Open, RequestStatus::PartiallyFilled]),
        )
        .order_by(ManpowerRequest::ID)
        .get(&mut tx)
        .await?;
    let mut outcomes = Vec::new();
    let mut planned = Vec::new();
    let mut rows = Vec::new();
    for request in requests {
        outcomes.push(
            request
                .transition(request::AUTO_CANCEL)
                .by_system()
                .reason(WORKER_RETURNED)
                .run_in(&mut tx)
                .await
                .map_err(answer)?,
        );
        let (more, cancelled) =
            cancel_planned_of_request(&mut tx, request.id, WORKER_RETURNED).await?;
        planned.extend(more);
        rows.extend(cancelled);
    }
    tx.commit().await?;
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    for outcome in planned {
        outcome.emit(state).await?;
    }
    announce_all_cancelled(state, &rows, WORKER_RETURNED).await
}

/// HR (`by`) closes case `id` with `reason` (*hospitalised*, *returned*...):
/// the worker is back. wl-hr releases the held pay and proposes the off-cycle
/// run, which HR prepares and a second user approves.
///
/// # Errors
///
/// `Forbidden` without `absence.manage`; a 422 without a reason or for a case
/// that is closed.
pub async fn close(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<AbsenceCase> {
    require_manage(by)?;
    let case = AbsenceCase::find_or_404(&state.db, id).await?;
    if !case.is_open() {
        return Err(refused(english("manpower.absence.closed", &[])));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(english("manpower.absence.reason", &[])));
    }
    close_case(state, id, reason, Some(by.id), today()).await
}

/// The open cases, oldest first, for HR.
///
/// # Errors
///
/// `Forbidden` without `absence.manage`.
pub async fn open_cases(state: &AppState, by: &AuthUser) -> Result<Vec<AbsenceCase>> {
    require_manage(by)?;
    AbsenceCase::query()
        .where_(AbsenceCase::CLOSED_ON.is_null())
        .order_by(AbsenceCase::ID)
        .get(&state.db)
        .await
}

/// Every case, open and closed, newest first, for HR.
///
/// # Errors
///
/// `Forbidden` without `absence.manage`.
pub async fn all_cases(state: &AppState, by: &AuthUser) -> Result<Vec<AbsenceCase>> {
    require_manage(by)?;
    AbsenceCase::query()
        .order_by_desc(AbsenceCase::ID)
        .get(&state.db)
        .await
}

/// Marks `MPW.RUL.09` compliance-critical, owned by the MD (`PLT.RUL.04`).
pub(crate) fn register() {
    wl_kernel::compliance::CriticalRule::new(RULE)
        .owner_role(permissions::MD_ROLE)
        .register();
}

/// Listens for a worker reinstated after an absence report: the case closes.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: EmployeeReinstated, state: AppState| async move {
        if let Some(case) = AbsenceCase::open_of(&state.db, event.employee.0).await? {
            close_case(
                &state,
                case.id,
                &english("manpower.absence.reinstated", &[]),
                None,
                today(),
            )
            .await?;
        }
        Ok(())
    });
}
