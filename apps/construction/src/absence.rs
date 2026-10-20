//! A worker who stops coming (`CON.RUL.08`, 02 Construction §4.2 row "A worker
//! stops coming", §7.2; BL-CON-15), run daily at 02:00 Asia/Dubai for the
//! days before.
//!
//! A worker assigned to a project (the employee has the project among the
//! scopes wl-hr lists for them, [`StaffProfile::scopes`]; crews as entities
//! come later) with no attendance and no approved leave for [`OPEN_DAYS`]
//! consecutive working days of the project's site gets an **absence case**: a
//! ticket of category [`ABSENCE_CATEGORY`] for HR, one per absence spell, and
//! the supervisor is told. [`AbsenceCaseOpened`] tells wl-hr, which
//! `report_absconding` needs. At [`REMIND_DAYS`] working days HR is reminded
//! that `report_absconding` may be requested; this rule never reports a
//! worker as absconded.
//!
//! - **Last attended day**: the latest day before the spell with an
//!   attendance record that counts (wl-attendance's
//!   [`AttendanceHistory`](wl_kernel::attendance::AttendanceHistory)) or
//!   inside an approved leave (wl-hr's
//!   [`LeaveCalendar`](wl_kernel::workforce::LeaveCalendar)). A worker who
//!   never attended in the last [`LOOKBACK_DAYS`] has not "stopped coming" and
//!   gets no case.
//! - **Working day of the site**: Monday to Friday (Saturday too when the
//!   project [`works_saturdays`](Project::works_saturdays)), never a public
//!   holiday of the client's calendar. A day on which the project was
//!   `on_hold` or `terminated` (its state at the end of that Dubai day, read
//!   from its history) does not count and does not break the run either.
//! - **Counting**: backwards from yesterday; attendance or leave on a counted
//!   day ends it. Attendance recorded again ends the spell: no reminder follows
//!   and the next absence is a new spell with its own case. The ticket stays
//!   open for HR to close with a note.
//! - The ticket's category `absence` is seeded by the vertical's starter data;
//!   the people are named by their employee number here (the names are wl-hr's).

use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine;
use serde::{Deserialize, Serialize};
use wl_kernel::attendance::attendance_history;
use wl_kernel::calendar::calendar;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::role_tasks;
use wl_kernel::tickets::{Assignee, OpenTicket, TicketId, task_creator};
use wl_kernel::workforce::{AbsenceCaseOpened, EmployeeId, leave_calendar, workforce};

use crate::notices::{holders, tell};
use crate::permissions;
use crate::project::{Project, ProjectStatus};
use crate::texts::english;

/// The category key of an absence case ticket.
pub const ABSENCE_CATEGORY: &str = "absence";

/// The consecutive working days without attendance or leave that open a case
/// (`CON.RUL.08`, N).
pub const OPEN_DAYS: usize = 3;

/// The consecutive working days after which HR is reminded that
/// `report_absconding` may be requested (`CON.RUL.08`, M).
pub const REMIND_DAYS: usize = 7;

/// How far back the days are looked at, in calendar days.
pub const LOOKBACK_DAYS: u64 = 120;

/// An absence case the rule opened for a worker of a project.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "construction_absence_cases",
    index(employee_id),
    index(project_id)
)]
pub struct AbsenceCase {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// The wl-hr employee.
    pub employee_id: i64,
    /// The ticket of category `absence`.
    pub ticket_id: i64,
    /// The last day with attendance or approved leave before the spell.
    pub last_present_on: NaiveDate,
    /// The day the case was opened.
    pub opened_on: NaiveDate,
    /// The day HR was reminded about `report_absconding`.
    pub reminded_on: Option<NaiveDate>,
    /// The day the worker was seen again; the ticket stays open for HR.
    pub ended_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a daily run did.
#[derive(Debug, Default)]
pub struct Run {
    /// The cases opened.
    pub opened: Vec<AbsenceCase>,
    /// The cases whose HR reminder was sent.
    pub reminded: Vec<AbsenceCase>,
}

/// How long a worker has been absent, counted back from a day.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Streak {
    /// The counted working days without attendance or leave.
    days: usize,
    /// The day with attendance or leave that ended the count; `None` when
    /// none was found in the window.
    last_present: Option<NaiveDate>,
}

/// Whether `day` is a working day of a site that works Saturdays or not:
/// never Sunday or a public holiday.
fn site_working_day(holidays: &[NaiveDate], works_saturdays: bool, day: NaiveDate) -> bool {
    let weekday = day.weekday();
    let open = match weekday {
        Weekday::Sun => false,
        Weekday::Sat => works_saturdays,
        _ => true,
    };
    open && !holidays.contains(&day)
}

/// Counts back from the day before `day`: `days` is the run of working days
/// of the site, not blocked, without attendance and leave; a day with
/// attendance or inside a leave ends it. Pure, so it is tested alone.
fn count_back(
    day: NaiveDate,
    attended: &[NaiveDate],
    leave: &[(NaiveDate, NaiveDate)],
    skipped: impl Fn(NaiveDate) -> bool,
) -> Streak {
    let mut days = 0;
    let mut at = day;
    for _ in 0..LOOKBACK_DAYS {
        let Some(previous) = at.checked_sub_days(Days::new(1)) else {
            break;
        };
        at = previous;
        let present =
            attended.contains(&at) || leave.iter().any(|(from, to)| *from <= at && at <= *to);
        if present {
            return Streak {
                days,
                last_present: Some(at),
            };
        }
        if skipped(at) {
            continue;
        }
        days += 1;
    }
    Streak {
        days,
        last_present: None,
    }
}

/// The state of `project` at the end of the Dubai day `day`, from its
/// history; a project without history in the window is taken as running.
fn blocked_on(history: &[(NaiveDate, String)], day: NaiveDate) -> bool {
    let state = history
        .iter()
        .take_while(|(on, _)| *on <= day)
        .next_back()
        .map(|(_, state)| state.as_str());
    matches!(
        state,
        Some(s) if s == ProjectStatus::OnHold.as_str() || s == ProjectStatus::Terminated.as_str()
    )
}

/// The Dubai days and states a project went through, oldest first.
async fn state_history(state: &AppState, project: &Project) -> Result<Vec<(NaiveDate, String)>> {
    Ok(
        state_machine::history(&state.db, Project::TABLE, project.id)
            .await?
            .into_iter()
            .map(|change| (dubai_date(change.at), change.to_state))
            .collect(),
    )
}

/// The employees assigned to `project`: listed in the workforce with the
/// project among their scopes.
async fn crew_of(state: &AppState, project: &Project) -> Result<Vec<EmployeeId>> {
    let workforce = workforce(state);
    let scope = project.subject();
    let mut crew = Vec::new();
    for employee in workforce.all_staff(&state.db).await? {
        if let Some(profile) = workforce.staff(state, employee).await?
            && profile.listed
            && profile.scopes.contains(&scope)
        {
            crew.push(employee);
        }
    }
    Ok(crew)
}

/// The users who get the absence case: the HR role, else whom the kernel
/// picks for it.
async fn hr_of(state: &AppState, project: &Project) -> Result<Vec<i64>> {
    holders(state, project, permissions::HR_ROLE).await
}

/// Opens the case of `employee` and tells the supervisor.
async fn open_case(
    state: &AppState,
    project: &Project,
    employee: EmployeeId,
    streak: &Streak,
    last_present: NaiveDate,
    day: NaiveDate,
) -> Result<Option<AbsenceCase>> {
    let employee_text = employee.0;
    let days = streak.days;
    let reminder = REMIND_DAYS;
    let title = english(
        "construction.absence.ticket_title",
        &[
            ("employee", &employee_text),
            ("days", &days),
            ("project", &project.name),
        ],
    );
    let body = english(
        "construction.absence.ticket_body",
        &[
            ("employee", &employee_text),
            ("project", &project.name),
            ("last", &last_present),
            ("reminder", &reminder),
        ],
    );
    let mut ticket = OpenTicket::new(employee.subject(), ABSENCE_CATEGORY, &title)
        .description(&body)
        .source(project.subject())
        .source_key(&format!(
            "construction.absence:{}:{last_present}",
            employee.0
        ));
    if let Some(user) = role_tasks::assignee(&state.db, permissions::HR_ROLE).await? {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    let Some(opened) = task_creator(state).open(state, ticket).await? else {
        return Ok(None);
    };
    let mut case = AbsenceCase {
        project_id: project.id,
        employee_id: employee.0,
        ticket_id: opened.id.0,
        last_present_on: last_present,
        opened_on: day,
        ..AbsenceCase::default()
    };
    case.save(&state.db).await?;
    state
        .emit(AbsenceCaseOpened {
            employee,
            case: opened.id.subject(),
            opened_on: day,
            last_present_on: last_present,
        })
        .await?;
    let supervisor_title = english(
        "construction.absence.supervisor_title",
        &[
            ("employee", &employee_text),
            ("project", &project.name),
            ("days", &days),
        ],
    );
    let supervisor_body = english(
        "construction.absence.supervisor_body",
        &[("ticket", &opened.number)],
    );
    let supervisors = holders(state, project, permissions::SUPERVISOR_ROLE).await?;
    tell(
        state,
        employee.subject(),
        &project.code,
        &supervisors,
        &supervisor_title,
        &supervisor_body,
    )
    .await?;
    Ok(Some(case))
}

/// Reminds HR that `report_absconding` may be requested for `case`; the claim
/// on `reminded_on` makes it once.
async fn remind(
    state: &AppState,
    project: &Project,
    case: &AbsenceCase,
    days: usize,
    day: NaiveDate,
) -> Result<bool> {
    let claim = Some(day);
    let claimed = AbsenceCase::query()
        .where_(AbsenceCase::ID.eq(case.id))
        .where_(AbsenceCase::REMINDED_ON.is_null())
        .update(&state.db, &[("reminded_on", &claim)])
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    let employee = case.employee_id;
    let ticket = TicketId(case.ticket_id).to_string();
    let title = english(
        "construction.absence.reminder_title",
        &[("employee", &employee), ("days", &days)],
    );
    let body = english(
        "construction.absence.reminder_body",
        &[("ticket", &ticket), ("project", &project.name)],
    );
    let users = hr_of(state, project).await?;
    tell(
        state,
        EmployeeId(employee).subject(),
        &project.code,
        &users,
        &title,
        &body,
    )
    .await?;
    Ok(true)
}

/// Runs the rule for `day`, counting the working days before it.
///
/// # Errors
///
/// The database's, ticketing's and notifier's errors.
pub async fn run_on(state: &AppState, day: NaiveDate) -> Result<Run> {
    let mut run = Run::default();
    let holidays: Vec<NaiveDate> = calendar(state).holidays().collect();
    let Some(from) = day.checked_sub_days(Days::new(LOOKBACK_DAYS)) else {
        return Ok(run);
    };
    let Some(yesterday) = day.checked_sub_days(Days::new(1)) else {
        return Ok(run);
    };
    let projects = Project::query()
        .where_(Project::STATUS.is_in([ProjectStatus::InProgress, ProjectStatus::OnHold]))
        .order_by(Project::ID)
        .get(&state.db)
        .await?;
    for project in projects {
        let history = state_history(state, &project).await?;
        let skipped = |at: NaiveDate| {
            !site_working_day(&holidays, project.works_saturdays, at) || blocked_on(&history, at)
        };
        for employee in crew_of(state, &project).await? {
            let attended = attendance_history(state)
                .attended_days(state, employee, from, yesterday)
                .await?;
            let leave = leave_calendar(state)
                .approved_leave(&state.db, employee, from, yesterday)
                .await?;
            let streak = count_back(day, &attended, &leave, skipped);
            let open = AbsenceCase::query()
                .where_(AbsenceCase::EMPLOYEE_ID.eq(employee.0))
                .where_(AbsenceCase::ENDED_ON.is_null())
                .order_by(AbsenceCase::ID)
                .first(&state.db)
                .await?;
            // The same spell, or the worker was seen again.
            let open = match open {
                Some(case)
                    if streak.days > 0
                        && streak
                            .last_present
                            .is_none_or(|last| last == case.last_present_on) =>
                {
                    Some(case)
                }
                Some(case) => {
                    AbsenceCase::query()
                        .where_(AbsenceCase::ID.eq(case.id))
                        .update(&state.db, &[("ended_on", &Some(day))])
                        .await?;
                    None
                }
                None => None,
            };
            match open {
                Some(case) => {
                    if streak.days >= REMIND_DAYS
                        && remind(state, &project, &case, streak.days, day).await?
                    {
                        run.reminded
                            .push(AbsenceCase::find_or_404(&state.db, case.id).await?);
                    }
                }
                None => {
                    let Some(last) = streak.last_present else {
                        continue;
                    };
                    if streak.days >= OPEN_DAYS
                        && let Some(case) =
                            open_case(state, &project, employee, &streak, last, day).await?
                    {
                        if streak.days >= REMIND_DAYS
                            && remind(state, &project, &case, streak.days, day).await?
                        {
                            run.reminded
                                .push(AbsenceCase::find_or_404(&state.db, case.id).await?);
                        }
                        run.opened.push(case);
                    }
                }
            }
        }
    }
    Ok(run)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn sunday_and_holidays_never_count_and_saturday_only_on_a_saturday_site() {
        let holiday = day(2027, 3, 9);
        let holidays = [holiday];
        assert!(site_working_day(&holidays, false, day(2027, 3, 5)));
        assert!(!site_working_day(&holidays, false, day(2027, 3, 6)));
        assert!(site_working_day(&holidays, true, day(2027, 3, 6)));
        assert!(!site_working_day(&holidays, true, day(2027, 3, 7)));
        assert!(!site_working_day(&holidays, true, holiday));
    }

    #[test]
    fn the_count_goes_back_from_yesterday_and_stops_at_the_last_present_day() {
        let attended = [day(2027, 2, 26)];
        let weekdays = |at: NaiveDate| !site_working_day(&[], false, at);
        // Monday 1 to Wednesday 3 March are absent: three days on Thursday.
        let streak = count_back(day(2027, 3, 4), &attended, &[], weekdays);
        assert_eq!(
            streak,
            Streak {
                days: 3,
                last_present: Some(day(2027, 2, 26))
            }
        );
        // Attendance on Friday 5 March ends it.
        let seen = [day(2027, 2, 26), day(2027, 3, 5)];
        let streak = count_back(day(2027, 3, 8), &seen, &[], weekdays);
        assert_eq!(streak.days, 0);
        assert_eq!(streak.last_present, Some(day(2027, 3, 5)));
        // A leave on those days counts as present.
        let leave = [(day(2027, 3, 1), day(2027, 3, 3))];
        let streak = count_back(day(2027, 3, 4), &attended, &leave, weekdays);
        assert_eq!(streak.days, 0);
    }

    #[test]
    fn a_blocked_day_neither_counts_nor_breaks_the_run() {
        let history = vec![
            (day(2027, 1, 1), "in_progress".to_owned()),
            (day(2027, 3, 2), "on_hold".to_owned()),
            (day(2027, 3, 3), "in_progress".to_owned()),
        ];
        assert!(!blocked_on(&history, day(2027, 3, 1)));
        assert!(blocked_on(&history, day(2027, 3, 2)));
        assert!(!blocked_on(&history, day(2027, 3, 3)));
        assert!(!blocked_on(&[], day(2027, 3, 3)));
    }
}
