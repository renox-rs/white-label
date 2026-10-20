//! The CDD file machine (04 §7.1 *CDD file*).
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | [`SUBMIT`] | in_progress | pending_approval | `cdd.prepare` |
//! | [`APPROVE`] | pending_approval | approved | `cdd.approve` (MLRO) |
//! | [`REJECT`] | pending_approval | rejected | `cdd.approve` |
//! | [`FLAG_REVIEW`] | approved | review_due | system |
//! | [`REOPEN`] | review_due, pending_approval | in_progress | `cdd.prepare` |
//! | [`EXIT`] | approved, review_due, in_progress | exited | `cdd.approve` (MLRO) |
//!
//! The services in [`crate::cdd_files`] gather the checks into a
//! [`CddInput`]; `submit` and `approve` refuse while any problem is left.
//! The reason of `reject` and `exit` travels in the input and is kept on the
//! file only ([`crate::cdd::CddFile::decision_reason`], MLRO only), never in
//! the history, so no history view can show it.
//!
//! Decision: `ACC.ACL.06` (a high-risk client also needs a partner's
//! approval) and the effects of `exit` (the partner's task, BL-ACC-09) are
//! not built here.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::cdd::{CddFile, CddStatus};
use crate::db::set_cdd_file;
use crate::permissions;
use crate::texts::english;

/// The `submit` transition's name.
pub const SUBMIT: &str = "submit";
/// The `approve` transition's name.
pub const APPROVE: &str = "approve";
/// The `reject` transition's name.
pub const REJECT: &str = "reject";
/// The system's `flag_review` transition's name.
pub const FLAG_REVIEW: &str = "flag_review";
/// The `reopen` transition's name.
pub const REOPEN: &str = "reopen";
/// The `exit` transition's name.
pub const EXIT: &str = "exit";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CddInput {
    /// Every check that fails, one line each, gathered by the service.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The business day of the run (Asia/Dubai); today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// The reason of `reject` or `exit` (MLRO only).
    #[serde(default)]
    pub reason: Option<String>,
}

/// The input of a run; the default without one (`available`).
fn input_of(ctx: &TransitionCtx) -> CddInput {
    ctx.input_as::<CddInput>().unwrap_or_default()
}

impl StateMachine for CddFile {
    type State = CddStatus;
    const COLUMN: renox::db::Col<CddFile, CddStatus> = CddFile::STATUS;

    fn machine() -> Machine<Self> {
        use CddStatus::{Approved, Exited, InProgress, PendingApproval, Rejected, ReviewDue};
        Machine::new()
            .initial([InProgress])
            .state(InProgress)
            .tone("neutral")
            .state(PendingApproval)
            .tone("info")
            .state(Approved)
            .tone("success")
            .state(ReviewDue)
            .tone("warning")
            .state(Rejected)
            .tone("error")
            .terminal()
            .state(Exited)
            .tone("error")
            .terminal()
            .transition(submit())
            .transition(approve())
            .transition(reject())
            .transition(flag_review())
            .transition(reopen())
            .transition(exit())
    }
}

/// A guard that refuses while the input lists a problem.
fn no_problem(transition: Transition<CddFile>, name: &'static str) -> Transition<CddFile> {
    transition.guard(name, |_f: &CddFile, ctx| {
        let problems = input_of(ctx).problems;
        Box::pin(async move {
            if problems.is_empty() {
                Ok(())
            } else {
                Err(problems.join("\n"))
            }
        })
    })
}

/// A guard that the input carries a reason, and the effect that keeps it on
/// the file.
fn with_decision_reason(transition: Transition<CddFile>) -> Transition<CddFile> {
    transition
        .guard("reason_given", |_f: &CddFile, ctx| {
            let given = input_of(ctx)
                .reason
                .is_some_and(|r| !r.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("accounting.problems.reason_required", &[]))
                }
            })
        })
        .effect(|f: &CddFile, ctx, tx| {
            let id = f.id;
            let reason = input_of(ctx).reason.map(|r| r.trim().to_owned());
            Box::pin(async move { set_cdd_file(&mut *tx, id, &[("decision_reason", &reason)]).await })
        })
}

/// `submit`: UBOs identified, identity documents attached, the risk rated
/// and the screening done and not on a stale list (BL-ACC-08).
fn submit() -> Transition<CddFile> {
    let t = Transition::new(SUBMIT)
        .label("Submit for approval")
        .tone("primary")
        .permission(permissions::CDD_PREPARE)
        .from([CddStatus::InProgress])
        .to(CddStatus::PendingApproval);
    no_problem(t, "ready")
}

/// `approve`: no list stale, no screening on a stale list and no open
/// possible match (BL-ACC-08); sets the next review date from the rating.
fn approve() -> Transition<CddFile> {
    let t = Transition::new(APPROVE)
        .label("Approve")
        .tone("success")
        .permission(permissions::CDD_APPROVE)
        .from([CddStatus::PendingApproval])
        .to(CddStatus::Approved);
    no_problem(t, "screening_settled").effect(|f: &CddFile, ctx, tx| {
        let id = f.id;
        let on = input_of(ctx).on.unwrap_or_else(today);
        let review = f.risk_rating.next_review(on);
        let by = ctx.actor().map(|user| user.id);
        Box::pin(async move {
            set_cdd_file(
                &mut *tx,
                id,
                &[
                    ("next_review_date", &Some(review)),
                    ("approved_by", &by),
                    ("approved_at", &Some(now())),
                ],
            )
            .await
        })
    })
}

/// `reject`: with a reason; the onboarding engagements of the client are
/// declined after the commit ([`crate::events`]).
fn reject() -> Transition<CddFile> {
    with_decision_reason(
        Transition::new(REJECT)
            .label("Reject")
            .tone("danger")
            .permission(permissions::CDD_APPROVE)
            .from([CddStatus::PendingApproval])
            .to(CddStatus::Rejected),
    )
}

/// `flag_review`: the system, when the review date is reached or an AML
/// case is reported.
fn flag_review() -> Transition<CddFile> {
    Transition::new(FLAG_REVIEW)
        .label("Flag for review")
        .system()
        .from([CddStatus::Approved])
        .to(CddStatus::ReviewDue)
}

/// `reopen`: back to `in_progress` to collect more.
fn reopen() -> Transition<CddFile> {
    Transition::new(REOPEN)
        .label("Reopen")
        .permission(permissions::CDD_PREPARE)
        .from([CddStatus::ReviewDue, CddStatus::PendingApproval])
        .to(CddStatus::InProgress)
}

/// `exit`: the MLRO ends the relationship, with a reason (MLRO only).
fn exit() -> Transition<CddFile> {
    with_decision_reason(
        Transition::new(EXIT)
            .label("Exit client")
            .tone("danger")
            .permission(permissions::CDD_APPROVE)
            .from([
                CddStatus::Approved,
                CddStatus::ReviewDue,
                CddStatus::InProgress,
            ])
            .to(CddStatus::Exited),
    )
}

/// Registers the machine for `rnx state:diagram` and the admin.
pub(crate) fn register() {
    renox::state_machine::register::<CddFile>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = CddFile::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("in_progress", Some("neutral"), false),
                ("pending_approval", Some("info"), false),
                ("approved", Some("success"), false),
                ("review_due", Some("warning"), false),
                ("rejected", Some("error"), true),
                ("exited", Some("error"), true),
            ]
        );
        let expected = [
            (SUBMIT, vec!["in_progress"], "pending_approval", false),
            (APPROVE, vec!["pending_approval"], "approved", false),
            (REJECT, vec!["pending_approval"], "rejected", false),
            (FLAG_REVIEW, vec!["approved"], "review_due", true),
            (
                REOPEN,
                vec!["review_due", "pending_approval"],
                "in_progress",
                false,
            ),
            (
                EXIT,
                vec!["approved", "review_due", "in_progress"],
                "exited",
                false,
            ),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to, system) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
        assert_eq!(machine.find(SUBMIT).unwrap().guard_names(), ["ready"]);
        assert_eq!(
            machine.find(APPROVE).unwrap().guard_names(),
            ["screening_settled"]
        );
        assert_eq!(machine.find(EXIT).unwrap().guard_names(), ["reason_given"]);
    }
}
