//! The Company budget state machine (wl-invoicing §4, `FIN.BUD.01`,
//! `PLT.APR.01`):
//!
//! ```text
//! [*] --> draft --submit--> submitted --approve--> approved --supersede--> superseded
//!         draft <--return-- submitted
//!         draft --discard--> discarded
//! ```
//!
//! The spec's `approve` is the kernel's `submit_approved` (the transition the
//! approval engine runs by name on the last yes) and its `return` is
//! `back_to_draft` (what the engine runs when the approval request ends
//! without a yes: an approver's *return*, a rejection, a withdrawal, an
//! expiry or a failed final check). `submitted` is the machine's waiting
//! state: [`SUBMIT`] creates the approval request in its effect
//! (`approvals::require_in`, action [`ACTION`]) to the holders of
//! `budgets.approve`, never the user who submitted (SoD).
//!
//! `submit_approved` moves the earlier approved version of the entity and
//! year to `superseded` in the same transaction, so a report sees either the
//! old or the new version, never both; once it commits, a listener emits
//! `CompanyBudgetApproved`. `supersede` is a system transition only that
//! approval runs. The status changes only through the machine.

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{
    Machine, StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, PolicyDefault, Require, Required};
use wl_kernel::budgets::{COMPANY_BUDGET_SUBJECT_TYPE, CompanyBudgetApproved, CompanyBudgetId};
use wl_kernel::machines::SubjectMachine;

use crate::budget::{BudgetStatus, CompanyBudget, CompanyBudgetLine, StartChoice};
use crate::budget_category::{BudgetCategory, refused};
use crate::credit_notes::By;
use crate::permissions;
use crate::texts::english;

/// The transitions' names.
pub const SUBMIT: &str = "submit";
/// The kernel runs it on the last yes (the spec's `approve`).
pub const SUBMIT_APPROVED: &str = "submit_approved";
/// The kernel runs it when the approval ends without a yes (the spec's
/// `return`).
pub const BACK_TO_DRAFT: &str = "back_to_draft";
/// A draft given up.
pub const DISCARD: &str = "discard";
/// The replaced approved version.
pub const SUPERSEDE: &str = "supersede";

/// The approval policies' action key of a budget submit.
pub const ACTION: &str = "invoicing.budget_approval";

/// Hours a budget approval may wait before it can expire.
pub const EXPIRY_HOURS: i64 = 336;

/// What a submit run carries to its guards, gathered by [`submit`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SubmitInput {
    /// How many lines the version has.
    pub lines: usize,
    /// How many lines have an inactive category or a month outside the year.
    pub bad_lines: usize,
}

/// The run's input, if it is a [`SubmitInput`].
fn input(ctx: &TransitionCtx) -> Option<SubmitInput> {
    ctx.input_as::<SubmitInput>().ok()
}

/// The id of the user running a transition.
fn actor_id(ctx: &TransitionCtx) -> Option<i64> {
    ctx.actor().map(|user| user.id)
}

/// Writes columns other than the status, in the transition's transaction.
async fn record(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    CompanyBudget::query()
        .where_(CompanyBudget::ID.eq(id))
        .update(&mut *tx, columns)
        .await?;
    Ok(())
}

/// `submit`: a draft with lines, for approval.
fn submit_transition() -> Transition<CompanyBudget> {
    Transition::new(SUBMIT)
        .label("Submit for approval")
        .tone("primary")
        .permission(permissions::BUDGETS_MANAGE)
        .from([BudgetStatus::Draft])
        .to(BudgetStatus::Submitted)
        .guard("has_lines", |_budget: &CompanyBudget, ctx| {
            let lines = input(ctx).map(|i| i.lines);
            Box::pin(async move {
                match lines {
                    Some(lines) if lines > 0 => Ok(()),
                    _ => Err(english("wl_invoicing.budget.problem.no_lines").to_owned()),
                }
            })
        })
        .guard("lines_are_valid", |_budget: &CompanyBudget, ctx| {
            let bad = input(ctx).map(|i| i.bad_lines);
            Box::pin(async move {
                match bad {
                    Some(0) => Ok(()),
                    _ => Err(english("wl_invoicing.budget.problem.bad_lines").to_owned()),
                }
            })
        })
        .guard("revision_reason_given", |budget: &CompanyBudget, _ctx| {
            let missing = budget.is_revision()
                && budget
                    .revision_reason
                    .as_deref()
                    .is_none_or(|r| r.trim().is_empty());
            Box::pin(async move {
                if missing {
                    Err(english("wl_invoicing.budget.problem.revision_reason").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|budget: &CompanyBudget, ctx, tx| {
            let id = budget.id;
            let subject = budget.subject();
            let by = actor_id(ctx);
            Box::pin(async move {
                let requester = by.ok_or_else(|| {
                    refused(english("wl_invoicing.budget.problem.submitter_unknown"))
                })?;
                let require = Require::new(ACTION, subject, SUBMIT, requester, "draft")
                    .payload(json!({ "budget": id }));
                let request = match approvals::require_in(&mut *tx, &require).await? {
                    Required::Pending(request) => request,
                    // Without the policy the budget would be in force
                    // unapproved; the transaction rolls back instead.
                    Required::Allowed => {
                        return Err(refused(english(
                            "wl_invoicing.budget.problem.no_approval_policy",
                        )));
                    }
                };
                record(
                    tx,
                    id,
                    &[
                        ("submitted_by", &Some(requester)),
                        ("submitted_at", &Some(renox::db::now())),
                        ("approval_request_id", &Some(request.id)),
                        ("return_reason", &None::<String>),
                    ],
                )
                .await
            })
        })
}

/// `submit_approved`: the last yes. The earlier approved version of the
/// entity and year is superseded in the same transaction.
fn approve_transition() -> Transition<CompanyBudget> {
    Transition::new(SUBMIT_APPROVED)
        .label("Approve")
        .from([BudgetStatus::Submitted])
        .to(BudgetStatus::Approved)
        .system()
        .effect(|budget: &CompanyBudget, ctx, tx| {
            let id = budget.id;
            let entity = budget.legal_entity_id;
            let year = budget.fiscal_year;
            let approver = ctx.meta().get("actor_id").and_then(Value::as_i64);
            Box::pin(async move {
                let earlier = CompanyBudget::query()
                    .where_(CompanyBudget::LEGAL_ENTITY_ID.eq(entity))
                    .where_(CompanyBudget::FISCAL_YEAR.eq(year))
                    .where_(CompanyBudget::STATUS.eq(BudgetStatus::Approved))
                    .where_(CompanyBudget::ID.ne(id))
                    .first(&mut *tx)
                    .await?;
                let mut supersedes = None;
                if let Some(earlier) = earlier {
                    let _superseded = earlier
                        .transition(SUPERSEDE)
                        .by_system()
                        .run_in(&mut *tx)
                        .await
                        .map_err(TransitionError::into_error)?;
                    supersedes = Some(earlier.id);
                }
                record(
                    tx,
                    id,
                    &[
                        ("approved_by", &approver),
                        ("approved_at", &Some(renox::db::now())),
                        ("supersedes_id", &supersedes),
                    ],
                )
                .await
            })
        })
}

/// `back_to_draft`: the approval ended without a yes; the reason is the
/// approver's comment or the kernel's fixed text.
fn return_transition() -> Transition<CompanyBudget> {
    Transition::new(BACK_TO_DRAFT)
        .label("Return to draft")
        .from([BudgetStatus::Submitted])
        .to(BudgetStatus::Draft)
        .system()
        .effect(|budget: &CompanyBudget, ctx, tx| {
            let id = budget.id;
            let reason = ctx
                .reason()
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty());
            Box::pin(async move { record(tx, id, &[("return_reason", &reason)]).await })
        })
}

/// `discard`: a draft nobody will finish.
fn discard_transition() -> Transition<CompanyBudget> {
    Transition::new(DISCARD)
        .label("Discard")
        .tone("danger")
        .permission(permissions::BUDGETS_MANAGE)
        // `reason_required` makes the run refuse (422) without a reason.
        .reason_required()
        .from([BudgetStatus::Draft])
        .to(BudgetStatus::Discarded)
        .effect(|budget: &CompanyBudget, ctx, tx| {
            let id = budget.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            Box::pin(async move { record(tx, id, &[("discard_reason", &reason)]).await })
        })
}

/// `supersede`: replaced by a newer approved version.
fn supersede_transition() -> Transition<CompanyBudget> {
    Transition::new(SUPERSEDE)
        .label("Supersede")
        .from([BudgetStatus::Approved])
        .to(BudgetStatus::Superseded)
        .system()
}

impl StateMachine for CompanyBudget {
    type State = BudgetStatus;
    const COLUMN: renox::db::Col<CompanyBudget, BudgetStatus> = CompanyBudget::STATUS;

    fn machine() -> Machine<Self> {
        use BudgetStatus::{Approved, Discarded, Draft, Submitted, Superseded};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Approved)
            .tone("success")
            .state(Superseded)
            .tone("neutral")
            .terminal()
            .state(Discarded)
            .tone("neutral")
            .terminal()
            .transition(submit_transition())
            .transition(approve_transition())
            .transition(return_transition())
            .transition(discard_transition())
            .transition(supersede_transition())
    }
}

/// What the guards need that they cannot read: the lines and whether each
/// has an active category and a month of the fiscal year.
async fn gather(db: &Db, budget: &CompanyBudget) -> Result<SubmitInput> {
    let lines = CompanyBudgetLine::of(db, budget.id).await?;
    let months = budget.months();
    let mut bad_lines = 0;
    for line in &lines {
        let active = BudgetCategory::find(db, line.category_id)
            .await?
            .is_some_and(|c| c.active);
        if !active || !months.contains(&line.month) {
            bad_lines += 1;
        }
    }
    Ok(SubmitInput {
        lines: lines.len(),
        bad_lines,
    })
}

/// Submits draft budget `id` for approval as `by`, in one transaction: the
/// version goes to `submitted` and an approval request goes to the holders
/// of `budgets.approve` (`PLT.APR.01`). Returns the budget as it is after
/// the commit.
///
/// # Errors
///
/// `Forbidden` without `budgets.manage`; `NotFrom` when it is not a `draft`;
/// `Guard` (422) when it has no line, a line has an inactive category or a
/// month outside the year, or a revision has no reason. Nothing is written
/// then.
pub async fn submit(
    state: &AppState,
    by: &AuthUser,
    id: i64,
) -> std::result::Result<CompanyBudget, TransitionError> {
    let budget = CompanyBudget::find_or_404(&state.db, id).await?;
    let gathered = gather(&state.db, &budget).await?;
    let mut tx = state.db.begin().await?;
    let outcome = budget
        .transition(SUBMIT)
        .by(by)
        .input(&gathered)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    Ok(CompanyBudget::find_or_404(&state.db, id).await?)
}

/// Gives up draft budget `id` with a reason, as `by`: the entity and year
/// are free for a new version; an approved version stays in force.
///
/// # Errors
///
/// `Forbidden` without `budgets.manage`; `NotFrom` when it is not a `draft`;
/// `ReasonRequired` (422) without a reason.
pub async fn discard(
    state: &AppState,
    by: By<'_>,
    id: i64,
    reason: &str,
) -> std::result::Result<CompanyBudget, TransitionError> {
    let budget = CompanyBudget::find_or_404(&state.db, id).await?;
    let mut tx = state.db.begin().await?;
    let outcome = match by {
        By::User(user) => budget.transition(DISCARD).by(user),
        By::System(_) => budget.transition(DISCARD).by_system(),
    }
    .reason(reason)
    .run_in(&mut tx)
    .await?;
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    Ok(CompanyBudget::find_or_404(&state.db, id).await?)
}

/// The default approval policy: every budget needs one approval by a holder
/// of `budgets.approve` other than the user who submitted it (SoD).
pub fn approval_policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .step_permission(permissions::BUDGETS_APPROVE)
        .expiry_hours(Some(EXPIRY_HOURS))
}

/// `CompanyBudgetApproved` for `budget`, as it is after the approval commit.
pub(crate) fn approved_event(budget: &CompanyBudget) -> CompanyBudgetApproved {
    CompanyBudgetApproved {
        budget: budget.budget_id(),
        legal_entity_id: budget.legal_entity_id,
        fiscal_year: i32::try_from(budget.fiscal_year).unwrap_or_default(),
        version: budget.version,
        superseded: budget.supersedes_id.map(CompanyBudgetId),
    }
}

/// Registers the machine with the kernel (waiting state `submitted`), the
/// default policy, the page of the subject for "Needs your action" and the
/// listener that emits `CompanyBudgetApproved`.
pub(crate) fn register(app: &mut Registry) {
    SubjectMachine::of::<CompanyBudget>(COMPANY_BUDGET_SUBJECT_TYPE)
        .waiting(BudgetStatus::Submitted)
        .register();
    renox::state_machine::register::<CompanyBudget>();
    approval_policy().register();
    approvals::register_subject_link(COMPANY_BUDGET_SUBJECT_TYPE, |id| format!("/budgets/{id}"));
    app.listen(
        |event: Transitioned<CompanyBudget>, state: AppState| async move {
            if event.transition != SUBMIT_APPROVED {
                return Ok(());
            }
            let budget = CompanyBudget::find_or_404(&state.db, event.key).await?;
            state.emit(approved_event(&budget)).await
        },
    );
}

/// The key (without the `wl_invoicing.` prefix) of the text that says how a
/// version started.
pub(crate) fn start_key(choice: StartChoice) -> &'static str {
    match choice {
        StartChoice::Empty => "budget.start.empty",
        StartChoice::LastBudget => "budget.start.last_budget",
        StartChoice::LastActual => "budget.start.last_actual",
        StartChoice::Revision => "budget.start.revision",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_ctx(input: &SubmitInput) -> TransitionCtx {
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).unwrap())
    }

    async fn refusal(budget: &CompanyBudget, ctx: &TransitionCtx) -> Option<String> {
        let machine = CompanyBudget::machine();
        match machine
            .find(SUBMIT)
            .unwrap()
            .check_guards(budget, ctx)
            .await
        {
            Ok(()) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    #[test]
    fn the_machine_has_the_five_transitions_of_the_spec() {
        let machine = CompanyBudget::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let find = |name: &str| machine.find(name).expect("transition");
        assert_eq!(find(SUBMIT).from_states(), ["draft"]);
        assert_eq!(find(SUBMIT).to_state(), Some("submitted"));
        assert_eq!(find(SUBMIT_APPROVED).from_states(), ["submitted"]);
        assert_eq!(find(SUBMIT_APPROVED).to_state(), Some("approved"));
        assert_eq!(find(BACK_TO_DRAFT).from_states(), ["submitted"]);
        assert_eq!(find(BACK_TO_DRAFT).to_state(), Some("draft"));
        assert_eq!(find(DISCARD).from_states(), ["draft"]);
        assert_eq!(find(DISCARD).to_state(), Some("discarded"));
        assert!(find(DISCARD).needs_reason());
        assert_eq!(find(SUPERSEDE).from_states(), ["approved"]);
        assert_eq!(find(SUPERSEDE).to_state(), Some("superseded"));
        let system: Vec<&str> = machine
            .transitions()
            .iter()
            .filter(|t| t.is_system())
            .map(|t| t.name())
            .collect();
        assert_eq!(system, [SUBMIT_APPROVED, BACK_TO_DRAFT, SUPERSEDE]);
    }

    #[test]
    fn the_budget_is_registered_with_its_waiting_state() {
        let machine = SubjectMachine::of::<CompanyBudget>(COMPANY_BUDGET_SUBJECT_TYPE)
            .waiting(BudgetStatus::Submitted)
            .build();
        assert_eq!(machine.subject_type(), "invoicing.company_budget");
        assert_eq!(machine.waiting_state(), Some("submitted"));
        assert!(machine.transition("submit_approved").is_some());
        assert!(machine.transition("back_to_draft").is_some());
        assert_eq!(machine.action_key(SUBMIT), "invoicing.submit");
    }

    #[test]
    fn every_budget_needs_an_approver_with_the_permission() {
        let expected = PolicyDefault::new("invoicing.budget_approval")
            .step_permission("budgets.approve")
            .expiry_hours(Some(336));
        assert_eq!(approval_policy(), expected);
    }

    #[renox::test]
    async fn the_guards_pass_a_ready_draft_and_refuse_the_rest() {
        let ready = SubmitInput {
            lines: 12,
            bad_lines: 0,
        };
        let budget = CompanyBudget::default();
        assert_eq!(refusal(&budget, &input_ctx(&ready)).await, None);
        let empty = SubmitInput {
            lines: 0,
            bad_lines: 0,
        };
        let text = refusal(&budget, &input_ctx(&empty))
            .await
            .unwrap_or_default();
        assert!(text.contains("at least one line"), "{text}");
        let bad = SubmitInput {
            lines: 3,
            bad_lines: 1,
        };
        let text = refusal(&budget, &input_ctx(&bad)).await.unwrap_or_default();
        assert!(text.contains("inactive"), "{text}");
        let bare = TransitionCtx::new(None);
        assert!(refusal(&budget, &bare).await.is_some());
    }

    #[renox::test]
    async fn a_revision_needs_its_reason() {
        let ready = SubmitInput {
            lines: 1,
            bad_lines: 0,
        };
        let revision = CompanyBudget {
            start_choice: StartChoice::Revision,
            ..Default::default()
        };
        let text = refusal(&revision, &input_ctx(&ready))
            .await
            .unwrap_or_default();
        assert!(text.contains("reason"), "{text}");
        let reasoned = CompanyBudget {
            revision_reason: Some("Rent went up".into()),
            ..revision
        };
        assert_eq!(refusal(&reasoned, &input_ctx(&ready)).await, None);
    }

    #[test]
    fn the_approved_event_names_the_version_it_replaced() {
        let budget = CompanyBudget {
            id: 5,
            legal_entity_id: 2,
            fiscal_year: 2027,
            version: 2,
            supersedes_id: Some(4),
            ..Default::default()
        };
        let event = approved_event(&budget);
        assert_eq!(event.budget, CompanyBudgetId(5));
        assert_eq!(event.fiscal_year, 2027);
        assert_eq!(event.superseded, Some(CompanyBudgetId(4)));
    }
}
