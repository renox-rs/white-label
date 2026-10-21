//! What the onboarding listens to (issue #216, #217): the exit side of the
//! employee lifecycle cancels it, an assignment change re-resolves its
//! assignees, and the evidence events do the steps of the system.
//!
//! - `EmployeeOnboardingCancelled` (the reason of the event), `EmployeeExited`
//!   and `EmployeeDeceased`: the employee's `in_progress` onboardings are
//!   cancelled ([`crate::onboarding::cancel_for`]).
//! - `EmployeeAssignmentChanged`: the open steps get their current assignees
//!   ([`crate::onboarding::re_resolve`]).
//! - `AbsenceCaseOpened` and `AbsenceCaseClosed`: the absence cases the
//!   verticals keep, which `report_absconding` needs ([`crate::absence`]); a
//!   closed case releases the lines held for it ([`crate::release`]).
//! - `DocumentExpired` and `DocumentExpiring` of an employee: the qualification
//!   events of `HRM.RUL.10` ([`crate::qualification_expiry`]).
//! - `Transitioned<PayrollRun>`: the notices and events that follow a payroll
//!   run's `return`, rejections and escalation ([`crate::payroll_events`]).
//! - `TimesheetApproved` (adjustments that affect payroll) and
//!   `TimesheetReconciliationNeeded`: the hours that enter the next regular run
//!   ([`crate::differences`]).
//! - `ChecklistCompleted`, `DocumentAccepted`, `SopPublished`,
//!   `SopAcknowledged`, `SopWithdrawn`, `SopReadersChanged`: the steps of
//!   [`crate::steps`].

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::attendance::TimesheetApproved;
use wl_kernel::documents::{
    DocumentAccepted, DocumentExpired, DocumentExpiring, SopAcknowledged, SopPublished,
    SopReadersChanged, SopWithdrawn,
};
use wl_kernel::tickets::ChecklistCompleted;
use wl_kernel::timesheets::TimesheetReconciliationNeeded;
use wl_kernel::workforce::{
    AbsenceCaseClosed, AbsenceCaseOpened, EmployeeAssignmentChanged, EmployeeDeceased,
    EmployeeExited, EmployeeOnboardingCancelled,
};

use crate::payroll::PayrollRun;
use crate::{
    absence, differences, onboarding, payroll_events, qualification_expiry, release, steps,
};

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: EmployeeOnboardingCancelled, state: AppState| async move {
            onboarding::cancel_for(&state, event.employee.0, &event.reason)
                .await
                .map(|_| ())
        },
    );
    app.listen(|event: EmployeeExited, state: AppState| async move {
        let reason = crate::texts::english("wl_hr.onboarding.cancelled_exit", &[]);
        onboarding::cancel_for(&state, event.employee.0, &reason)
            .await
            .map(|_| ())
    });
    app.listen(|event: EmployeeDeceased, state: AppState| async move {
        let reason = crate::texts::english("wl_hr.onboarding.cancelled_death", &[]);
        onboarding::cancel_for(&state, event.employee.0, &reason)
            .await
            .map(|_| ())
    });
    app.listen(
        |event: EmployeeAssignmentChanged, state: AppState| async move {
            onboarding::re_resolve(&state, event.employee.0).await
        },
    );
    app.listen(
        |event: Transitioned<PayrollRun>, state: AppState| async move {
            payroll_events::on_transitioned(&state, &event).await
        },
    );
    app.listen(|event: TimesheetApproved, state: AppState| async move {
        differences::on_approved(&state, &event).await
    });
    app.listen(
        |event: TimesheetReconciliationNeeded, state: AppState| async move {
            differences::on_reconciliation(&state, &event).await
        },
    );
    app.listen(|event: ChecklistCompleted, state: AppState| async move {
        steps::on_checklist_completed(&state, &event).await
    });
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        steps::on_document_accepted(&state, &event).await
    });
    app.listen(|event: AbsenceCaseOpened, state: AppState| async move {
        absence::on_case_opened(&state, &event).await
    });
    app.listen(|event: AbsenceCaseClosed, state: AppState| async move {
        absence::on_case_closed(&state, &event).await?;
        // `HRM.RUL.12`: the lines held for the case are released.
        release::on_case_closed(&state, &event).await
    });
    app.listen(|event: DocumentExpired, state: AppState| async move {
        qualification_expiry::on_expired(&state, &event).await
    });
    app.listen(|event: DocumentExpiring, state: AppState| async move {
        qualification_expiry::on_expiring(&state, &event).await
    });
    app.listen(
        |_event: SopPublished, state: AppState| async move { steps::on_sop_event(&state).await },
    );
    app.listen(|_event: SopAcknowledged, state: AppState| async move {
        steps::on_sop_event(&state).await
    });
    app.listen(
        |_event: SopWithdrawn, state: AppState| async move { steps::on_sop_event(&state).await },
    );
    app.listen(|_event: SopReadersChanged, state: AppState| async move {
        steps::on_sop_event(&state).await
    });
}
