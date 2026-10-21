//! The period-end submit (wl-attendance §4 `submit_by_system`, `HRM.RUL.08`,
//! issue #183).
//!
//! Once the period of an `open` timesheet is over, the system submits it with
//! the checks of a user's submit ([`crate::submit::submit_by_system`]). One
//! that fails them stays `open` and its submitter gets a task listing the
//! days; the run moves on to the next timesheet. A repeat of the run opens no
//! second task and submits nothing twice. Adjustment timesheets are left to
//! the people who open them.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::submit::{self, SystemSubmit};
use crate::timesheet::{Timesheet, TimesheetKind, TimesheetStatus};

/// The daily scheduled task, and the command, that run [`run`].
pub const TASK: &str = "attendance:period-end-submit";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "01:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Timesheets the system submitted.
    pub submitted: usize,
    /// Timesheets a check refused: they stay `open` with a task.
    pub refused: usize,
    /// Timesheets that failed for another reason; the run moves on.
    pub failed: usize,
}

/// Submits, as the system, every `open` timesheet whose period ended before
/// `on`.
///
/// # Errors
///
/// A database error while listing the timesheets.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<Report> {
    let due = Timesheet::query()
        .where_(Timesheet::STATUS.eq(TimesheetStatus::Open))
        .where_(Timesheet::PERIOD_TO.lt(on))
        .order_by(Timesheet::ID)
        .get(&state.db)
        .await?;
    let mut report = Report::default();
    for sheet in due
        .into_iter()
        .filter(|sheet| sheet.kind != TimesheetKind::Adjustment)
    {
        match submit::submit_by_system(state, sheet.id).await {
            Ok(SystemSubmit::Submitted(_)) => report.submitted += 1,
            Ok(SystemSubmit::Refused { .. }) => report.refused += 1,
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
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
        "Submit the open timesheets whose period is over (--date YYYY-MM-DD checks another day)",
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
                "{} submitted, {} refused, {} failed.",
                report.submitted, report.refused, report.failed
            );
            Ok(())
        },
    );
}
