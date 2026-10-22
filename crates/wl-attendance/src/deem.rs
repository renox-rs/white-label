//! Deemed approval of an external timesheet (wl-attendance §4 `deem_approve`,
//! §5.6 point 3, §6 *Automations* `HRM.RUL.15` at N, §7, §10; BL-ATT-11).
//!
//! If the client's contract has a deemed-approval clause (N days, from the
//! app's [`ApproverResolver`](wl_kernel::timesheets::ApproverResolver)), the
//! system approves an external timesheet that is still `submitted` N calendar
//! days (Asia/Dubai) after the date of its latest `submit`. It runs in the
//! daily run of `HRM.RUL.15` ([`crate::escalation`]), before the escalation,
//! so with N at or below E the escalation finds nothing `submitted` and sends
//! nothing.
//!
//! [`run`] reads N as it was when the timesheet was submitted (the resolver's
//! `deemed_approval_days_at`; a resolver without history answers with the
//! contract as it is now, so a contract changed while the timesheet waits
//! applies). A context without a clause (the resolver answers none, or a
//! number below 1) is left as it is: the timesheet stays `submitted`. An
//! internal approver's timesheet is never deemed approved, by the rule and by
//! the machine's guard. The transition does what `approve` does
//! ([`crate::approval`]): the basis stored is `deemed` (and no approver),
//! attendance is locked, the labour cost is posted where the context posts
//! it, the task to collect a signed sheet closes, `TimesheetApproved` carries
//! the basis `deemed`, and a timesheet that payroll paid on provisional hours
//! is locked and reconciled ([`crate::reconcile`]). Then the client's
//! approvers and the account manager are told the hours, that they were
//! deemed approved, and the dispute route: a later dispute is handled as an
//! adjustment.
//!
//! Decisions:
//!
//! - **The race with the client.** The transition runs on a timesheet in
//!   `submitted`; when the client approved (or rejected) in the meantime the
//!   engine refuses the second one and the rule does nothing for it.
//! - **After the commit.** The cost, the event and the notice follow the
//!   approval as for `approve`; a failure there is counted and leaves the
//!   timesheet approved.
//! - **Whether a deemed approval is evidence in a billing dispute** is
//!   `⚠ open: lawyer (L-29)`. The basis is stored and passed on in the event
//!   so that wl-invoicing can show it on the invoice's support.

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::dubai_date;
use wl_kernel::timesheets::approver_resolver;

use crate::approval::{self, DEEM_APPROVE, DeemInput};
use crate::notices;
use crate::timesheet::{ApproverKind, Timesheet, TimesheetStatus};

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Timesheets deemed approved.
    pub deemed: usize,
    /// Timesheets that failed; the run moves on and the next run tries again
    /// (a timesheet that is approved already is not retried).
    pub failed: usize,
}

/// Deems `sheet` approved if its contract says so and N days have passed.
/// Returns whether it did.
async fn deem_one(state: &AppState, sheet: &Timesheet, on: NaiveDate) -> Result<bool> {
    let context = sheet.context();
    let Some(submitted) = sheet.submitted_at else {
        return Ok(false);
    };
    let Some(days) = approver_resolver(state)
        .deemed_approval_days_at(state, context.as_ref(), submitted)
        .await?
        .filter(|days| *days >= 1)
    else {
        return Ok(false);
    };
    if dubai_date(submitted) + Duration::days(days) > on {
        return Ok(false);
    }
    let ran = sheet
        .transition(DEEM_APPROVE)
        .by_system()
        .input(&DeemInput { days, on })
        .run(state)
        .await;
    match ran {
        Ok(_outcome) => {}
        // The client approved or rejected at the same moment: only one runs.
        Err(
            TransitionError::NotFrom(_) | TransitionError::Conflict | TransitionError::Guard(_),
        ) => return Ok(false),
        Err(other) => return Err(other.into_error()),
    }
    let approved = approval::approved(state, sheet.id).await?;
    notices::timesheet_deemed(state, &approved, days).await?;
    Ok(true)
}

/// Deems approved every external timesheet still `submitted` whose contract
/// has a clause and whose N days have passed on `on`.
///
/// # Errors
///
/// A database error while listing the timesheets.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<Report> {
    let waiting = Timesheet::query()
        .where_(Timesheet::STATUS.eq(TimesheetStatus::Submitted))
        .where_(Timesheet::APPROVER_KIND.eq(ApproverKind::External))
        .order_by(Timesheet::ID)
        .get(&state.db)
        .await?;
    let mut report = Report::default();
    for sheet in waiting {
        match deem_one(state, &sheet, on).await {
            Ok(true) => report.deemed += 1,
            Ok(false) => {}
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
}
