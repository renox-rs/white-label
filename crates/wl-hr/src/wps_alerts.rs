//! `HRM.RUL.11` (wl-hr §6 *Automations*, issues #202 to #204): a run that is
//! `rejected` or `partially_rejected` gives the preparer a task per failed line
//! and tells the approver; one that is still not resubmitted (`rejected`) or
//! not regenerated or confirmed paid (`partially_rejected`) N days before the
//! WPS due date is escalated to the MD and the accountant, once.
//!
//! This file holds the tasks; the escalation is added to it with BL-HRM-14.
//!
//! - **Tasks.** One per failed line, assigned to the preparer, opened through
//!   the kernel `TaskCreator` (a ticket of category *task*) about the run. The
//!   source key names the run, the line and the SIF version the line failed in,
//!   so a task is opened once per failure: a second call finds it open, and a
//!   line that fails again in a later version gets a new one.
//! - The approver is told by the notice of the transition
//!   ([`crate::payroll_events`]).

use renox::prelude::*;
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};

use crate::payroll::{LineStatus, PayrollRun};
use crate::payroll_events::line_employee;
use crate::texts::english;

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
