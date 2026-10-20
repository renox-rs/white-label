//! What the onboarding listens to (issue #216, #217): the exit side of the
//! employee lifecycle cancels it, an assignment change re-resolves its
//! assignees, and the evidence events do the steps of the system.
//!
//! - `EmployeeOnboardingCancelled` (the reason of the event), `EmployeeExited`
//!   and `EmployeeDeceased`: the employee's `in_progress` onboardings are
//!   cancelled ([`crate::onboarding::cancel_for`]).
//! - `EmployeeAssignmentChanged`: the open steps get their current assignees
//!   ([`crate::onboarding::re_resolve`]).
//! - `ChecklistCompleted`, `DocumentAccepted`, `SopPublished`,
//!   `SopAcknowledged`, `SopWithdrawn`, `SopReadersChanged`: the steps of
//!   [`crate::steps`].

use renox::prelude::*;
use wl_kernel::documents::{
    DocumentAccepted, SopAcknowledged, SopPublished, SopReadersChanged, SopWithdrawn,
};
use wl_kernel::tickets::ChecklistCompleted;
use wl_kernel::workforce::{
    EmployeeAssignmentChanged, EmployeeDeceased, EmployeeExited, EmployeeOnboardingCancelled,
};

use crate::{onboarding, steps};

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
    app.listen(|event: ChecklistCompleted, state: AppState| async move {
        steps::on_checklist_completed(&state, &event).await
    });
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        steps::on_document_accepted(&state, &event).await
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
