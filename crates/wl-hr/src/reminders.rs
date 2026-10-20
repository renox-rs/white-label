//! `HRM.RUL.17` (wl-hr §6, issue #218): the daily run that reminds and
//! escalates overdue onboarding steps.
//!
//! Every day at 08:00 Asia/Dubai [`run`] looks at the open steps of the
//! `in_progress` onboardings:
//!
//! - **Remind:** a step whose due date has passed is reminded to its
//!   assignees on the first run after the due date, then every
//!   [`REMIND_EVERY`] (R) days. At most one reminder per step per day. A step
//!   nobody is assigned to (the employee has no account and records it in
//!   person) is reminded to HR, who record it.
//! - **Escalate:** once, [`ESCALATE_AFTER`] (E) calendar days after the due
//!   date, HR and the employee's manager are told (the manager's holders, or
//!   HR when the scope has none). An optional step is reminded like a
//!   required one, but HR and the manager are not told.
//! - **Pause:** an employee who is `on_leave` or `absconded` is skipped. The
//!   due dates of open steps move by the days of approved leave first
//!   ([`crate::leave::sync_due_dates`]; not while `absconded`). After
//!   `reinstate` the first run reminds an overdue step and tells HR and the
//!   manager when E days have passed and they were not told before; the next
//!   reminder follows R days later.
//! - A step that is `done`, `waived` or `cancelled`, and any step of a
//!   `completed` or `cancelled` onboarding, is never reminded.
//!
//! The days are counted in calendar days in Asia/Dubai; [`run`] takes the
//! day as a parameter, so the command can check another day.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::employee::{Employee, EmployeeStatus};
use crate::onboarding::{Onboarding, OnboardingStatus, OnboardingStep, StepStatus};
use crate::{assignees, leave, notices};

/// The daily scheduled task, and the command, that run [`run`].
pub const TASK: &str = "hr:step-reminders";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "08:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// R: days between two reminders of the same step.
pub const REMIND_EVERY: i64 = 2;

/// E: days overdue after which HR and the manager are told, once.
pub const ESCALATE_AFTER: i64 = 5;

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Steps whose due date moved with the leave of the employee.
    pub moved: usize,
    /// Reminders sent to assignees.
    pub reminded: usize,
    /// Steps HR and the manager were told about.
    pub escalated: usize,
    /// Onboardings that failed; the run moves on.
    pub failed: usize,
}

/// What a step needs on a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// Remind the assignees.
    pub remind: bool,
    /// Tell HR and the manager.
    pub escalate: bool,
    /// Calendar days past the due date.
    pub days: i64,
}

/// What `step` needs on `day`: nothing unless it is open and past its due
/// date.
pub fn plan(step: &OnboardingStep, day: NaiveDate) -> Plan {
    let days = (day - step.due_date).num_days();
    if step.status != StepStatus::Open || days < 1 {
        return Plan {
            remind: false,
            escalate: false,
            days,
        };
    }
    Plan {
        remind: step
            .last_reminded_on
            .is_none_or(|last| (day - last).num_days() >= REMIND_EVERY),
        escalate: step.required && step.escalated_on.is_none() && days >= ESCALATE_AFTER,
        days,
    }
}

/// Runs `HRM.RUL.17` for `day` (Asia/Dubai).
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    let running = Onboarding::query()
        .where_(Onboarding::STATUS.eq(OnboardingStatus::InProgress))
        .order_by(Onboarding::ID)
        .get(&state.db)
        .await?;
    // The leave first, so a step is judged by its due date as it is now.
    let mut employees: Vec<i64> = running.iter().map(|o| o.employee_id).collect();
    employees.sort_unstable();
    employees.dedup();
    for employee in employees {
        match leave::sync_due_dates(state, employee, day).await {
            Ok(moved) => report.moved += moved,
            Err(error) => {
                report.failed += 1;
                eprintln!("{TASK}: employee {employee}: {error:?}");
            }
        }
    }
    for onboarding in running {
        if let Err(error) = handle(state, &onboarding, day, &mut report).await {
            report.failed += 1;
            eprintln!("{TASK}: onboarding {}: {error:?}", onboarding.id);
        }
    }
    Ok(report)
}

/// Reminds and escalates the open steps of one onboarding.
async fn handle(
    state: &AppState,
    onboarding: &Onboarding,
    day: NaiveDate,
    report: &mut Report,
) -> Result {
    let employee = Employee::find_or_404(&state.db, onboarding.employee_id).await?;
    if matches!(
        employee.status,
        EmployeeStatus::OnLeave | EmployeeStatus::Absconded
    ) {
        return Ok(());
    }
    let scopes = employee.scopes(&state.db).await?;
    let steps = OnboardingStep::query()
        .where_(OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
        .where_(OnboardingStep::STATUS.eq(StepStatus::Open))
        .order_by(OnboardingStep::POSITION)
        .get(&state.db)
        .await?;
    for step in steps {
        let needs = plan(&step, day);
        if needs.remind {
            let mut users = step.assignees(&state.db).await?;
            if users.is_empty() {
                users = assignees::hr_holders(state, &scopes).await?;
            }
            OnboardingStep::query()
                .where_(OnboardingStep::ID.eq(step.id))
                .update(
                    &state.db,
                    &[("last_reminded_on", &Some(day)), ("updated_at", &now())],
                )
                .await?;
            notices::step_reminder(state, &employee, &step, &users, needs.days).await?;
            report.reminded += 1;
        }
        if needs.escalate {
            let mut told = assignees::hr_holders(state, &scopes).await?;
            told.extend(assignees::resolve(state, &employee, &scopes, "manager").await?);
            told.sort_unstable();
            told.dedup();
            OnboardingStep::query()
                .where_(OnboardingStep::ID.eq(step.id))
                .update(
                    &state.db,
                    &[("escalated_on", &Some(day)), ("updated_at", &now())],
                )
                .await?;
            notices::step_escalated(state, &employee, &step, &told, needs.days).await?;
            report.escalated += 1;
        }
    }
    Ok(())
}

/// Registers the daily [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Remind the assignees of overdue onboarding steps and tell HR and the manager \
         (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = run(&state, on).await?;
            println!(
                "{} steps moved, {} reminded, {} escalated, {} failed.",
                report.moved, report.reminded, report.escalated, report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn open(due: NaiveDate) -> OnboardingStep {
        OnboardingStep {
            due_date: due,
            required: true,
            ..OnboardingStep::default()
        }
    }

    #[test]
    fn nothing_is_due_on_the_due_date_itself() {
        let step = open(day(11, 9));
        let p = plan(&step, day(11, 9));
        assert!(!p.remind && !p.escalate);
    }

    #[test]
    fn the_first_run_after_the_due_date_reminds_then_every_second_day() {
        let mut step = open(day(11, 9));
        assert!(plan(&step, day(11, 10)).remind);
        step.last_reminded_on = Some(day(11, 10));
        assert!(!plan(&step, day(11, 10)).remind);
        assert!(!plan(&step, day(11, 11)).remind);
        assert!(plan(&step, day(11, 12)).remind);
    }

    #[test]
    fn hr_is_told_once_after_five_days_and_not_for_an_optional_step() {
        let mut step = open(day(11, 9));
        assert!(!plan(&step, day(11, 13)).escalate);
        assert!(plan(&step, day(11, 14)).escalate);
        assert_eq!(plan(&step, day(11, 14)).days, 5);
        step.escalated_on = Some(day(11, 14));
        assert!(!plan(&step, day(11, 16)).escalate);
        let optional = OnboardingStep {
            required: false,
            ..open(day(11, 9))
        };
        assert!(!plan(&optional, day(11, 20)).escalate);
        assert!(plan(&optional, day(11, 20)).remind);
    }

    #[test]
    fn a_settled_step_needs_nothing() {
        let done = OnboardingStep {
            status: StepStatus::Done,
            ..open(day(11, 9))
        };
        let p = plan(&done, day(11, 30));
        assert!(!p.remind && !p.escalate);
    }
}
