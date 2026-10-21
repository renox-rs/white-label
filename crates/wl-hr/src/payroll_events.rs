//! What follows a payroll run's transitions, after the commit: the notices
//! and the kernel events of `return` (issue #201), the rejection of a SIF
//! file and the tasks of `HRM.RUL.11` (issues #202 to #204).
//!
//! These are listeners of `Transitioned<PayrollRun>`, not effects: a notice
//! that cannot be sent does not undo the change of the run.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::{PayrollRunId, PayrollRunReturned};

use crate::notices;
use crate::payroll::{PayrollLine, PayrollRun, RETURN};

/// The listener registered by [`crate::events`].
pub(crate) async fn on_transitioned(state: &AppState, event: &Transitioned<PayrollRun>) -> Result {
    match event.transition.as_str() {
        RETURN => returned(state, event.key).await,
        _ => Ok(()),
    }
}

/// `return`: the preparer is told the reason and the comments on the lines
/// to fix; `PayrollRunReturned` follows.
async fn returned(state: &AppState, run_id: i64) -> Result {
    let Some(run) = PayrollRun::find(&state.db, run_id).await? else {
        return Ok(());
    };
    let reason = run.return_reason.clone().unwrap_or_default();
    let comments = comments_of(state, &run).await?;
    let preparer: Vec<i64> = run.prepared_by.into_iter().collect();
    notices::payroll_returned(state, &run, &reason, &comments, &preparer).await?;
    state
        .emit(PayrollRunReturned {
            run: PayrollRunId(run.id),
            period: run.period.clone(),
            reason,
        })
        .await
}

/// The approver's comments on the lines of `run`: `Employee: comment`, one
/// after the other.
async fn comments_of(state: &AppState, run: &PayrollRun) -> Result<String> {
    let mut parts = Vec::new();
    for line in run.lines(&state.db).await? {
        let Some(comment) = line.approver_comment.as_deref() else {
            continue;
        };
        parts.push(format!("{}: {comment}", line_employee(state, &line).await));
    }
    Ok(parts.join("; "))
}

/// The name of the employee of `line` for staff-facing text.
pub(crate) async fn line_employee(state: &AppState, line: &PayrollLine) -> String {
    match crate::employee::Employee::find(&state.db, line.employee_id).await {
        Ok(Some(employee)) => notices::name_of(state, &employee).await,
        _ => format!("#{}", line.employee_id),
    }
}
