//! The AML case machine (04 §7.1 *AML case*).
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | `open` | — | `open` | `aml.raise` (any staff), or the system (`ACC.RUL.12`) |
//! | [`REVIEW`] | open | under_review | `aml.manage` (MLRO) |
//! | [`REPORT`] | under_review | reported | `aml.manage` |
//! | [`CLOSE`] | under_review | closed_no_action | `aml.manage` |
//!
//! Decision: `open` is how the record comes to be, not a move between two
//! states: [`crate::aml_cases`] inserts the case in `open` with the history
//! row [`OPEN`] (the facts are required there). A system case starts `open`;
//! the MLRO takes it with `review`, then either `close` or `report`; `close`
//! is not offered from `open`.
//!
//! The services gather what the guards need into an [`AmlInput`]. `close`
//! needs a reason and, for a sanctions case, the freeze and report tasks of
//! every linked match done; `report` needs the goAML reference and date.
//! The transitions are the MLRO's alone: `aml.manage` is held by the
//! compliance role and by nobody else (`ACC.ACL.05`), not even the company
//! admin.

use renox::chrono::NaiveDate;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::aml::{AmlCase, AmlStatus};
use crate::permissions;
use crate::texts::english;

/// The history row of a case coming to be.
pub const OPEN: &str = "open";
/// The `review` transition's name.
pub const REVIEW: &str = "review";
/// The `report` transition's name.
pub const REPORT: &str = "report";
/// The `close` transition's name.
pub const CLOSE: &str = "close";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmlInput {
    /// The reason of `close`.
    #[serde(default)]
    pub reason: Option<String>,
    /// Every linked task still open, one line each, gathered by the service
    /// (`close`); the guard refuses while any is left.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The goAML reference of the report (`report`).
    #[serde(default)]
    pub goaml_reference: Option<String>,
    /// The date the report was filed on goAML (`report`).
    #[serde(default)]
    pub goaml_date: Option<NaiveDate>,
    /// The business day of the run (Asia/Dubai); today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
}

/// The input of a run; the default without one (`available`).
pub(crate) fn input_of(ctx: &TransitionCtx) -> AmlInput {
    ctx.input_as::<AmlInput>().unwrap_or_default()
}

impl StateMachine for AmlCase {
    type State = AmlStatus;
    const COLUMN: renox::db::Col<AmlCase, AmlStatus> = AmlCase::STATUS;

    fn machine() -> Machine<Self> {
        use AmlStatus::{ClosedNoAction, Open, Reported, UnderReview};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("warning")
            .state(UnderReview)
            .tone("warning")
            .state(Reported)
            .tone("error")
            .terminal()
            .state(ClosedNoAction)
            .tone("neutral")
            .terminal()
            .transition(review())
            .transition(report())
            .transition(close())
    }
}

/// `review`: the MLRO takes the case.
fn review() -> Transition<AmlCase> {
    Transition::new(REVIEW)
        .label("Review")
        .tone("primary")
        .permission(permissions::AML_MANAGE)
        .from([AmlStatus::Open])
        .to(AmlStatus::UnderReview)
}

/// `report`: the goAML reference and date are recorded; the effects on the
/// CDD file, the partner and the engagement run after the commit
/// ([`crate::aml_cases::after_report`]).
fn report() -> Transition<AmlCase> {
    Transition::new(REPORT)
        .label("Report")
        .tone("danger")
        .permission(permissions::AML_MANAGE)
        .from([AmlStatus::UnderReview])
        .to(AmlStatus::Reported)
        .guard("goaml_recorded", |_c: &AmlCase, ctx| {
            let input = input_of(ctx);
            let given = input
                .goaml_reference
                .as_deref()
                .is_some_and(|r| !r.trim().is_empty())
                && input.goaml_date.is_some();
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("accounting.aml.problem.goaml", &[]))
                }
            })
        })
        .effect(|c: &AmlCase, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            let reference = input.goaml_reference.map(|r| r.trim().to_owned());
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                crate::db::set_aml_case(
                    &mut *tx,
                    id,
                    &[
                        ("goaml_reference", &reference),
                        ("goaml_date", &input.goaml_date),
                        ("reported_by", &by),
                    ],
                )
                .await
            })
        })
}

/// `close`: reviewed, no report needed; a reason is recorded and, for a
/// sanctions case, every linked freeze and report task is done. It never
/// unblocks the client.
fn close() -> Transition<AmlCase> {
    Transition::new(CLOSE)
        .label("Close")
        .tone("neutral")
        .permission(permissions::AML_MANAGE)
        .from([AmlStatus::UnderReview])
        .to(AmlStatus::ClosedNoAction)
        .guard("reason_given", |_c: &AmlCase, ctx| {
            let given = input_of(ctx)
                .reason
                .is_some_and(|reason| !reason.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("accounting.aml.problem.reason", &[]))
                }
            })
        })
        .guard("tasks_done", |_c: &AmlCase, ctx| {
            let problems = input_of(ctx).problems;
            Box::pin(async move {
                if problems.is_empty() {
                    Ok(())
                } else {
                    Err(problems.join("\n"))
                }
            })
        })
        .effect(|c: &AmlCase, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            let reason = input.reason.map(|r| r.trim().to_owned());
            let on = input.on.unwrap_or_else(today);
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                crate::db::set_aml_case(
                    &mut *tx,
                    id,
                    &[
                        ("closing_reason", &reason),
                        ("closed_by", &by),
                        ("closed_on", &Some(on)),
                    ],
                )
                .await
            })
        })
}

/// Registers the machine for `rnx state:diagram`. Decision: no approval
/// policy and no link in the approvals' "Needs your action" page: the case
/// is not shown outside the MLRO's own pages.
pub(crate) fn register() {
    renox::state_machine::register::<AmlCase>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = AmlCase::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("open", Some("warning"), false),
                ("under_review", Some("warning"), false),
                ("reported", Some("error"), true),
                ("closed_no_action", Some("neutral"), true),
            ]
        );
        let expected = [
            (REVIEW, vec!["open"], "under_review"),
            (REPORT, vec!["under_review"], "reported"),
            (CLOSE, vec!["under_review"], "closed_no_action"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert!(!transition.is_system(), "{name}");
        }
        assert!(machine.find(REVIEW).unwrap().guard_names().is_empty());
        assert_eq!(
            machine.find(REPORT).unwrap().guard_names(),
            ["goaml_recorded"]
        );
        assert_eq!(
            machine.find(CLOSE).unwrap().guard_names(),
            ["reason_given", "tasks_done"]
        );
    }
}
