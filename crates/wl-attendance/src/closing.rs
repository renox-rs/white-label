//! Closing the open timesheet of a context when its work ends (09 Manpower
//! §7.1 *Deployment* `end` and *Client contract* `end`; BL-MPW-06).
//!
//! The module provides [`Closer`] as the app's
//! [`TimesheetClosing`](wl_kernel::timesheets::TimesheetClosing). For the
//! regular timesheets of one owner in one context that are still `open`:
//!
//! - a period that runs past the last day is cut to it, so the timesheet
//!   covers the days the worker was on site and nothing after;
//! - it is submitted as the system at once, with the checks of any submit
//!   ([`crate::submit::submit_by_system`]): one a check refuses stays `open`
//!   and its submitter gets the task listing the days;
//! - a timesheet that starts after the last day is left as it is.
//!
//! A timesheet that is already `submitted`, `approved` or `locked` is not
//! touched, so a second call finds nothing to do.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::attendance::LabourFuture;
use wl_kernel::timesheets::{TimesheetClosing, TimesheetClosure};
use wl_kernel::workforce::EmployeeId;

use crate::submit::{self, SystemSubmit};
use crate::timesheet::{Timesheet, TimesheetKind, TimesheetStatus};

/// Closes the open timesheets of `owner` in `context` at `last_day`.
///
/// # Errors
///
/// A database error.
pub async fn close_open(
    state: &AppState,
    context: &Subject,
    owner: EmployeeId,
    last_day: NaiveDate,
) -> Result<Vec<TimesheetClosure>> {
    let open = Timesheet::query()
        .where_(Timesheet::OWNER_EMPLOYEE_ID.eq(owner.0))
        .where_(Timesheet::CONTEXT_TYPE.eq(context.subject_type.clone()))
        .where_(Timesheet::CONTEXT_ID.eq(context.subject_id))
        .where_(Timesheet::KIND.eq(TimesheetKind::Regular))
        .where_(Timesheet::STATUS.eq(TimesheetStatus::Open))
        .where_(Timesheet::PERIOD_FROM.lte(last_day))
        .order_by(Timesheet::ID)
        .get(&state.db)
        .await?;
    let mut closed = Vec::new();
    for sheet in open {
        let period_to = sheet.period_to.min(last_day);
        if sheet.period_to > last_day {
            Timesheet::query()
                .where_(Timesheet::ID.eq(sheet.id))
                .update(
                    &state.db,
                    &[("period_to", &period_to), ("updated_at", &now())],
                )
                .await?;
        }
        let (submitted, message) = match submit::submit_by_system(state, sheet.id).await {
            Ok(SystemSubmit::Submitted(_)) => (true, None),
            Ok(SystemSubmit::Refused { message, .. }) => (false, Some(message)),
            // Another run submitted it first.
            Err(_) => continue,
        };
        closed.push(TimesheetClosure {
            timesheet: sheet.subject(),
            period_to,
            submitted,
            message,
        });
    }
    Ok(closed)
}

/// The module's [`TimesheetClosing`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Closer;

impl TimesheetClosing for Closer {
    fn close_open<'a>(
        &'a self,
        state: &'a AppState,
        context: &'a Subject,
        owner: EmployeeId,
        last_day: NaiveDate,
    ) -> LabourFuture<'a, Vec<TimesheetClosure>> {
        Box::pin(close_open(state, context, owner, last_day))
    }
}
