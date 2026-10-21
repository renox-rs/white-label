//! `HRM.RUL.11` (wl-hr §6 *Automations*, §7 *WPS rejection escalation*,
//! issues #202 to #204): a run that is `rejected` or `partially_rejected` gives
//! the preparer a task per failed line and tells the approver; one that is
//! still not resubmitted (`rejected`) or not regenerated or confirmed paid
//! (`partially_rejected`) N days before the WPS due date is escalated to the MD
//! and the accountant, once.
//!
//! - **Tasks.** One per failed line, assigned to the preparer, opened through
//!   the kernel `TaskCreator` (a ticket of category *task*) about the run. The
//!   source key names the run, the line and the SIF version the line failed in,
//!   so a task is opened once per failure: a second call finds it open, and a
//!   line that fails again in a later version gets a new one.
//! - **The approver** is told by the notice of the transition
//!   ([`crate::payroll_events`]).
//! - **Escalation.** The WPS due date is the 1st of the month after the
//!   period (MR 340/2026, [`wps_due_date`]). The daily job [`TASK`] at 00:00
//!   Asia/Dubai ([`run`]) escalates every run still `rejected` or
//!   `partially_rejected` on the day N days before it (N =
//!   [`HrConfig::wps_escalation_days`](crate::HrConfig), default 1), or later
//!   if a day was missed: the MD and the accountant (the holders of the
//!   `md_role` and `accountant_role` roles) get one notice. A rejection
//!   recorded on or after that day escalates at once ([`escalate_if_due`]). The
//!   run is escalated once: [`PayrollRun::escalated_on`] is the day.
//!   A run back in `submitted`, or `paid`, is never escalated.
//! - `HRM.RUL.03`, the WPS deadline alerts, is a separate rule that looks at
//!   every run that is not `paid`; nothing here stops or replaces it.

use renox::auth::permissions::users_with_role;
use renox::chrono::{Duration, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};

use crate::config::config_of;
use crate::notices;
use crate::payroll::{LineStatus, PayrollRun, PayrollRunStatus};
use crate::payroll_events::line_employee;
use crate::texts::english;

/// The daily scheduled task, and the command, that run [`run`].
pub const TASK: &str = "hr:wps-escalation";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "00:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The source key of the task for `line_id` that failed in SIF version
/// `version` of run `run_id`.
pub fn task_key(run_id: i64, line_id: i64, version: i64) -> String {
    format!("hr.payroll_line_failed:{run_id}:{line_id}:{version}")
}

/// Opens a task for the preparer of `run` for each line that is `failed`, once
/// per failure. Returns how many were opened now. Without the ticketing module
/// no ticket is opened.
pub async fn open_tasks(state: &AppState, run: &PayrollRun) -> Result<usize> {
    let creator = tickets::task_creator(state);
    let mut opened = 0;
    for line in run.lines(&state.db).await? {
        if line.status != LineStatus::Failed {
            continue;
        }
        let key = task_key(run.id, line.id, run.sif_version);
        if creator.find_open(state, &key).await?.is_some() {
            continue;
        }
        let employee = line_employee(state, &line).await;
        let reason = line.failure_reason.clone().unwrap_or_default();
        let title = english(
            "wl_hr.payroll.task_title",
            &[("employee", &employee), ("reason", &reason)],
        );
        let description = english(
            "wl_hr.payroll.task_description",
            &[("period", &run.period), ("version", &run.sif_version)],
        );
        let mut ticket = OpenTicket::new(run.subject(), TASK_CATEGORY, &title)
            .description(&description)
            .source_key(&key);
        if let Some(preparer) = run.prepared_by {
            ticket = ticket.assign_to(Assignee::user(preparer));
        }
        if creator.open(state, ticket).await?.is_some() {
            opened += 1;
        }
    }
    Ok(opened)
}

/// The WPS due date of `period` (`2026-11`): the 1st of the following month
/// (MR 340/2026). `None` when `period` is not a month.
pub fn wps_due_date(period: &str) -> Option<NaiveDate> {
    let (year, month) = period.split_once('-')?;
    let (year, month): (i32, u32) = (year.parse().ok()?, month.parse().ok()?);
    if !(1..=12).contains(&month) {
        return None;
    }
    if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
}

/// The day from which a run of `period` that is still rejected is escalated:
/// `days` before the WPS due date.
pub fn escalation_day(period: &str, days: i64) -> Option<NaiveDate> {
    wps_due_date(period).map(|due| due - Duration::days(days))
}

/// What a run of the job did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Runs escalated to the MD and the accountant.
    pub escalated: usize,
    /// Runs that failed; the job moves on.
    pub failed: usize,
}

/// Runs `HRM.RUL.11` for `day` (Asia/Dubai): escalates each run that is
/// `rejected` or `partially_rejected`, was not escalated yet and whose
/// escalation day has come.
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    let open = PayrollRun::query()
        .where_(PayrollRun::STATUS.is_in([
            PayrollRunStatus::Rejected,
            PayrollRunStatus::PartiallyRejected,
        ]))
        .where_(PayrollRun::ESCALATED_ON.is_null())
        .order_by(PayrollRun::ID)
        .get(&state.db)
        .await?;
    for run in open {
        match escalate_if_due(state, &run, day).await {
            Ok(true) => report.escalated += 1,
            Ok(false) => {}
            Err(error) => {
                report.failed += 1;
                eprintln!("{TASK}: run {}: {error:?}", run.id);
            }
        }
    }
    Ok(report)
}

/// Escalates `run` when it is `rejected` or `partially_rejected`, has not been
/// escalated and `day` is its escalation day or later. Returns whether it did.
pub async fn escalate_if_due(state: &AppState, run: &PayrollRun, day: NaiveDate) -> Result<bool> {
    // Read it again: the caller's copy may be older than a resubmit.
    let Some(run) = PayrollRun::find(&state.db, run.id).await? else {
        return Ok(false);
    };
    if !matches!(
        run.status,
        PayrollRunStatus::Rejected | PayrollRunStatus::PartiallyRejected
    ) || run.escalated_on.is_some()
    {
        return Ok(false);
    }
    let config = config_of(state);
    let Some(from) = escalation_day(&run.period, config.wps_escalation_days) else {
        return Ok(false);
    };
    if day < from {
        return Ok(false);
    }
    let due = from + Duration::days(config.wps_escalation_days);
    let mut told = Vec::new();
    for role in [&config.md_role, &config.accountant_role] {
        told.extend(users_with_role(&state.db, role).await?.iter().map(|u| u.id));
    }
    told.sort_unstable();
    told.dedup();
    let failed = run
        .lines(&state.db)
        .await?
        .iter()
        .filter(|l| l.status == LineStatus::Failed)
        .count();
    PayrollRun::query()
        .where_(PayrollRun::ID.eq(run.id))
        .update(
            &state.db,
            &[("escalated_on", &Some(day)), ("updated_at", &now())],
        )
        .await?;
    notices::payroll_escalated(state, &run, due, failed, &told).await?;
    Ok(true)
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
        "Tell the MD and the accountant of WPS rejections still open before the due date \
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
                "{} runs escalated, {} failed.",
                report.escalated, report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_due_date_is_the_first_of_the_following_month() {
        assert_eq!(wps_due_date("2026-11"), Some(day(2026, 12, 1)));
        assert_eq!(wps_due_date("2026-12"), Some(day(2027, 1, 1)));
        assert_eq!(wps_due_date("2026-13"), None);
        assert_eq!(wps_due_date("november"), None);
    }

    #[test]
    fn the_escalation_day_is_n_days_before_the_due_date() {
        assert_eq!(escalation_day("2026-11", 1), Some(day(2026, 11, 30)));
        assert_eq!(escalation_day("2026-11", 3), Some(day(2026, 11, 28)));
        assert_eq!(escalation_day("2027-01", 1), Some(day(2027, 1, 31)));
    }
}
