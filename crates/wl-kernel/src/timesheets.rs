//! The timesheet contracts (wl-attendance §6, §8): what other modules and the
//! verticals may ask of timesheets without depending on the `wl-attendance`
//! crate.
//!
//! - [`ApproverResolver`] (§8): who stands behind an externally approved
//!   timesheet. The vertical provides it from the deployment's contract
//!   (09 Manpower: the client approver, the escalation contact, the account
//!   manager and the deemed-approval days). `wl-attendance` asks it for the
//!   escalation of `HRM.RUL.15` and for deemed approval. Without one,
//!   [`approver_resolver`] gives [`NoApprovers`]: nobody is known and no
//!   contract has a deemed-approval clause.

use std::sync::Arc;

use renox::prelude::*;

use crate::attendance::LabourFuture;
use crate::notify::NotifyRecipient;
use crate::subject::Subject;

/// Who stands behind the timesheets of a context (a deployment, a project...)
/// that an external approver approves, as the context's contract says
/// (wl-attendance §8 `ApproverResolver`). The app provides it with
/// [`provide_approver_resolver`]. Every answer is for the context as it is
/// now, so a change of the contract applies to a timesheet that is waiting.
pub trait ApproverResolver: Send + Sync + 'static {
    /// The user who is the account manager of the context's client: the one
    /// who collects a signed sheet and is told when the hours are deemed
    /// approved. The default has none.
    fn account_manager<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async { Ok(None) })
    }

    /// The client contacts to tell when an approval is overdue (`HRM.RUL.15`
    /// at E days): the escalation contact of the contract. The default has
    /// none.
    fn escalation_contacts<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The client's approvers, to tell when their silence is taken as
    /// approval. The default has none.
    fn client_approvers<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The days after `submit` at which the contract deems a timesheet
    /// approved (`HRM.RUL.15` at N); `None` when the contract has no
    /// deemed-approval clause (the default). A value below 1 is no clause.
    fn deemed_approval_days<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async { Ok(None) })
    }
}

/// The resolver when none is provided: nobody is known and no contract has a
/// deemed-approval clause.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoApprovers;

impl ApproverResolver for NoApprovers {}

/// Makes `resolver` the app's [`ApproverResolver`].
pub fn provide_approver_resolver(app: &mut Registry, resolver: impl ApproverResolver) {
    let resolver: Arc<dyn ApproverResolver> = Arc::new(resolver);
    app.provide(resolver);
}

/// The app's [`ApproverResolver`], or [`NoApprovers`] when none is provided.
pub fn approver_resolver(state: &AppState) -> Arc<dyn ApproverResolver> {
    match state.provided::<Arc<dyn ApproverResolver>>() {
        Some(resolver) => Arc::clone(&resolver),
        None => Arc::new(NoApprovers),
    }
}
