//! What follows a payroll run's transitions, after the commit: the notices
//! and the kernel events of `return` (issue #201), the rejection of a SIF
//! file and the tasks of `HRM.RUL.11` (issues #202 to #204).
//!
//! These are listeners of `Transitioned<PayrollRun>`, not effects: a notice
//! that cannot be sent does not undo the change of the run.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{PostCost, post_cost};
use wl_kernel::{
    EmployeeId, FailedPayrollLine, PayrollLinesFailed, PayrollRunId, PayrollRunPaid,
    PayrollRunReturned,
};

use crate::payroll::{
    CONFIRM_PAID, LineStatus, PayrollLine, PayrollRun, RECORD_PARTIAL, RECORD_PARTIAL_BY_SYSTEM,
    RECORD_REJECTION, RETURN, SUBMIT,
};
use crate::{notices, wps_alerts};

/// The listener registered by [`crate::events`].
pub(crate) async fn on_transitioned(state: &AppState, event: &Transitioned<PayrollRun>) -> Result {
    match event.transition.as_str() {
        RETURN => returned(state, event.key).await,
        RECORD_REJECTION | RECORD_PARTIAL | RECORD_PARTIAL_BY_SYSTEM => {
            failed(state, event.key).await
        }
        SUBMIT => crate::calculation::record_use(state, event.key).await,
        CONFIRM_PAID => paid(state, event.key).await,
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

/// The preparer and the approver of `run`, once each.
pub(crate) fn staff_of(run: &PayrollRun) -> Vec<i64> {
    let mut users: Vec<i64> = run.prepared_by.into_iter().chain(run.approved_by).collect();
    users.sort_unstable();
    users.dedup();
    users
}

/// The lines of `run` that are `failed`, with their reasons, for the event.
async fn failed_lines(state: &AppState, run: &PayrollRun) -> Result<Vec<FailedPayrollLine>> {
    Ok(run
        .lines(&state.db)
        .await?
        .into_iter()
        .filter(|l| l.status == LineStatus::Failed)
        .map(|l| FailedPayrollLine {
            line: l.id,
            employee: EmployeeId(l.employee_id),
            reason: l.failure_reason.clone().unwrap_or_default(),
        })
        .collect())
}

/// `record_rejection` and `record_partial`: the preparer and the approver are
/// told (`HRM.RUL.11`) and `PayrollLinesFailed` follows with the failed lines
/// and their reasons.
pub(crate) async fn failed(state: &AppState, run_id: i64) -> Result {
    let Some(run) = PayrollRun::find(&state.db, run_id).await? else {
        return Ok(());
    };
    let lines = failed_lines(state, &run).await?;
    let mut reasons: Vec<String> = lines.iter().map(|l| l.reason.clone()).collect();
    reasons.sort();
    reasons.dedup();
    notices::payroll_failed(
        state,
        &run,
        lines.len(),
        &reasons.join(", "),
        &staff_of(&run),
    )
    .await?;
    wps_alerts::open_tasks(state, &run).await?;
    // A rejection recorded after the escalation day is escalated at once.
    wps_alerts::escalate_if_due(state, &run, today()).await?;
    state
        .emit(PayrollLinesFailed {
            run: PayrollRunId(run.id),
            period: run.period.clone(),
            lines,
        })
        .await
}

/// `confirm_paid`: `PayrollRunPaid` with the employees paid and the total.
async fn paid(state: &AppState, run_id: i64) -> Result {
    let Some(run) = PayrollRun::find(&state.db, run_id).await? else {
        return Ok(());
    };
    let paid: Vec<PayrollLine> = run
        .lines(&state.db)
        .await?
        .into_iter()
        .filter(|l| l.status == LineStatus::Paid)
        .collect();
    state
        .emit(PayrollRunPaid {
            run: PayrollRunId(run.id),
            period: run.period.clone(),
            total: paid.iter().map(|l| l.net_amount).sum(),
            employees: paid.iter().map(|l| EmployeeId(l.employee_id)).collect(),
        })
        .await?;
    post_costs(state, &run).await?;
    // `HRM.RUL.12`: lines whose hold cleared while this run was being paid are
    // released now that it is `paid`.
    crate::release::propose(state).await.map(|_| ())
}

/// `confirm_paid`: the cost of each paid line is posted to its cost object
/// (`FIN.CST.01`): the gross pay of a regular or off-cycle line, the signed
/// difference of a supplementary line, so the cost of a correction lands on
/// the same cost objects as the paid run. The key names the line, so a second
/// call posts nothing more.
async fn post_costs(state: &AppState, run: &PayrollRun) -> Result {
    let supplementary = run.run_type == crate::supplementary::SUPPLEMENTARY;
    for line in run.lines(&state.db).await? {
        let (Some(kind), Some(id)) = (line.cost_object_type.as_deref(), line.cost_object_id) else {
            continue;
        };
        if line.status != LineStatus::Paid {
            continue;
        }
        let amount = if supplementary {
            line.net_amount
        } else {
            line.net_amount + line.deductions_amount
        };
        if amount == 0 {
            continue;
        }
        let request = PostCost::new(
            run.subject(),
            format!("hr.payroll_line:{}", line.id),
            wl_kernel::Subject::new(kind, id),
            amount,
        );
        post_cost(state, request).await?;
    }
    Ok(())
}
