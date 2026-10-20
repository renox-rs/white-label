//! What `DOC.RUL.04` listens to (wl-documents §4.3, §5.7, issue #53):
//!
//! - `SopPublished`: the pending acknowledgements of older versions are
//!   `replaced`, and every required reader is asked for the new one.
//! - `OnboardingStarted`, `EmployeeAssignmentChanged`: the employee becomes a
//!   required reader of the SOPs for their role and scope, and stops being one
//!   of those they left.
//! - `SopReadersChanged`: the new readers are asked, the removed ones cancelled.
//! - `EmployeeExited`, `EmployeeDeceased`, `EmployeeOnboardingCancelled`: the
//!   employee's pending acknowledgements are `cancelled`.
//!
//! `SopWithdrawn` is handled by the withdrawal itself
//! ([`Sops::withdraw`](crate::sops::Sops::withdraw)), which cancels the pending
//! acknowledgements before it emits the event with their ids.

use renox::prelude::*;
use wl_kernel::documents::{SopPublished, SopReadersChanged, today};
use wl_kernel::workforce::{
    EmployeeAssignmentChanged, EmployeeDeceased, EmployeeExited, EmployeeOnboardingCancelled,
    OnboardingStarted,
};

use super::service::Acknowledgements;

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: SopPublished, state: AppState| async move {
        Acknowledgements::replace_pending(&state, event.sop.0, event.document.0).await?;
        Acknowledgements::reconcile_sop(&state, event.sop.0, today())
            .await
            .map(|_| ())
    });
    app.listen(|event: SopReadersChanged, state: AppState| async move {
        Acknowledgements::reconcile_sop(&state, event.sop.0, today())
            .await
            .map(|_| ())
    });
    app.listen(|event: OnboardingStarted, state: AppState| async move {
        Acknowledgements::reconcile_employee(&state, event.employee.0, today())
            .await
            .map(|_| ())
    });
    app.listen(
        |event: EmployeeAssignmentChanged, state: AppState| async move {
            Acknowledgements::reconcile_employee(&state, event.employee.0, today())
                .await
                .map(|_| ())
        },
    );
    app.listen(|event: EmployeeExited, state: AppState| async move {
        let reason = crate::texts::english("wl_documents.acks.cancelled_exit", &[]);
        Acknowledgements::cancel_for_employee(&state, event.employee.0, &reason)
            .await
            .map(|_| ())
    });
    app.listen(|event: EmployeeDeceased, state: AppState| async move {
        let reason = crate::texts::english("wl_documents.acks.cancelled_death", &[]);
        Acknowledgements::cancel_for_employee(&state, event.employee.0, &reason)
            .await
            .map(|_| ())
    });
    app.listen(
        |event: EmployeeOnboardingCancelled, state: AppState| async move {
            Acknowledgements::cancel_for_employee(&state, event.employee.0, &event.reason)
                .await
                .map(|_| ())
        },
    );
}
