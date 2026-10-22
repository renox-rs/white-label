//! The leave request state machine (wl-hr §4 *Leave request*, base of
//! BL-HRM-02, issue #192).
//!
//! ```text
//! [*] requested | approved          (approved: `record_sick`, see leave_service)
//! requested --approve--> approved
//! requested --reject--> rejected
//! requested | approved --cancel--> cancelled
//! approved --complete--> completed
//! ```
//!
//! Decision: the four transitions are system transitions that the services of
//! [`crate::leave_service`] run after they checked who may act. The rules
//! ("`leave.approve`, or the employee's manager", "the employee or
//! `leave.approve`") depend on the employee's scope, which the engine's
//! permission check does not know (it counts global roles only); the same
//! choice as the onboarding steps. The user is kept in the run's input and in
//! the row (`decided_by`).
//!
//! `record_sick` has no source state: the service inserts the request as
//! `approved` (an initial state) with a history row, and does what `approve`
//! does.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::leave::{LeaveRequest, LeaveStatus};
use crate::texts::english;

/// A requested leave is approved.
pub const APPROVE: &str = "approve";
/// A requested leave is rejected.
pub const REJECT: &str = "reject";
/// A requested or approved leave is withdrawn before it starts.
pub const CANCEL: &str = "cancel";
/// The leave period is over.
pub const COMPLETE: &str = "complete";

/// What a run of a leave transition needs, gathered by the service (a guard
/// cannot read the database and an effect gets no `AppState`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LeaveInput {
    /// The user who acts; `None` for the system.
    pub by: Option<i64>,
    /// The day to judge by (Asia/Dubai); today when absent.
    pub today: Option<NaiveDate>,
    /// Whether the document the leave type needs (the sick note) is attached.
    pub has_document: bool,
    /// The working days that are unpaid (`approve`).
    pub unpaid_days: i64,
}

impl LeaveInput {
    /// The day the run is judged by.
    pub fn day(&self) -> NaiveDate {
        self.today.unwrap_or_else(today)
    }
}

fn input_of(ctx: &TransitionCtx) -> LeaveInput {
    ctx.input_as::<LeaveInput>().unwrap_or_default()
}

/// Sets the columns of request `id` outside the machine's guard and stamps
/// `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    LeaveRequest::query()
        .where_(LeaveRequest::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for LeaveRequest {
    type State = LeaveStatus;
    const COLUMN: renox::db::Col<LeaveRequest, LeaveStatus> = LeaveRequest::STATUS;

    fn machine() -> Machine<Self> {
        use LeaveStatus::{Approved, Cancelled, Completed, Rejected, Requested};
        Machine::new()
            .initial([Requested, Approved])
            .state(Requested)
            .tone("info")
            .state(Approved)
            .tone("success")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(APPROVE)
                    .label("Approve")
                    .tone("success")
                    .system()
                    .from([Requested])
                    .to(Approved)
                    .guard("document_attached", |leave: &LeaveRequest, ctx| {
                        let needs_note = leave.leave_type == "sick";
                        let has = input_of(ctx).has_document;
                        Box::pin(async move {
                            if !needs_note || has {
                                Ok(())
                            } else {
                                Err(english("wl_hr.leave.problem.sick_note", &[]))
                            }
                        })
                    })
                    .effect(|leave: &LeaveRequest, ctx, tx| {
                        let id = leave.id;
                        let input = input_of(ctx);
                        Box::pin(async move {
                            let at = Some(now());
                            set(
                                tx,
                                id,
                                &[
                                    ("unpaid_days", &input.unpaid_days),
                                    ("decided_by", &input.by),
                                    ("decided_at", &at),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .tone("danger")
                    .system()
                    .reason_required()
                    .from([Requested])
                    .to(Rejected)
                    .effect(|leave: &LeaveRequest, ctx, tx| {
                        let id = leave.id;
                        let by = input_of(ctx).by;
                        let reason = ctx.reason().map(|r| r.trim().to_owned());
                        Box::pin(async move {
                            let at = Some(now());
                            set(
                                tx,
                                id,
                                &[
                                    ("decided_by", &by),
                                    ("decided_at", &at),
                                    ("decision_reason", &reason),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .tone("neutral")
                    .system()
                    .from([Requested, Approved])
                    .to(Cancelled)
                    .guard("not_started", |leave: &LeaveRequest, ctx| {
                        let started = leave.from_date <= input_of(ctx).day();
                        Box::pin(async move {
                            if started {
                                Err(english("wl_hr.leave.problem.started", &[]))
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|leave: &LeaveRequest, ctx, tx| {
                        let id = leave.id;
                        let by = input_of(ctx).by;
                        let reason = ctx
                            .reason()
                            .map(|r| r.trim().to_owned())
                            .filter(|r| !r.is_empty());
                        Box::pin(async move {
                            let at = Some(now());
                            set(
                                tx,
                                id,
                                &[
                                    ("decided_by", &by),
                                    ("decided_at", &at),
                                    ("decision_reason", &reason),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(COMPLETE)
                    .label("Complete")
                    .tone("success")
                    .system()
                    .from([Approved])
                    .to(Completed)
                    .guard("ended", |leave: &LeaveRequest, ctx| {
                        let over = leave.to_date < input_of(ctx).day();
                        Box::pin(async move {
                            if over {
                                Ok(())
                            } else {
                                Err(english("wl_hr.leave.problem.not_over", &[]))
                            }
                        })
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_starts_requested_or_approved_and_every_transition_is_the_systems() {
        let machine = LeaveRequest::machine();
        assert_eq!(machine.initial_states(), ["requested", "approved"]);
        let table: [(&str, &[&str], &str); 4] = [
            (APPROVE, &["requested"], "approved"),
            (REJECT, &["requested"], "rejected"),
            (CANCEL, &["requested", "approved"], "cancelled"),
            (COMPLETE, &["approved"], "completed"),
        ];
        for (name, from, to) in table {
            let transition = machine.find(name).unwrap();
            assert!(transition.is_system(), "{name}");
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.transitions().len(), 4);
    }
}
