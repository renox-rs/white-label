//! Doing, waiving and reopening onboarding steps (wl-hr §3 *Onboarding*, §4
//! *Onboarding step*, §5.8 steps 3-6, `HRM.ACL.10`, issue #217).
//!
//! How a step is done follows its kind:
//!
//! - `task`: an assignee marks it done ([`mark_done`]), with an evidence note
//!   or file when the step needs it. A step owned by the employee is marked
//!   by the employee in their own session, or in person with a drawn
//!   signature on a staff device, the staff user recorded as witness
//!   ([`InPerson`]); nobody else marks it for them. After an in-person
//!   record the employee is told on their own channel.
//! - `checklist`: done when the checklist run started on the step
//!   ([`start_checklist`]) completes *passed* ([`on_checklist_completed`]); a
//!   *failed* run leaves the step open with the failed items, and a new run
//!   can be started.
//! - `document`: done when an accepted document of the kind is on the
//!   employee ([`on_document_accepted`]); one that was accepted before the
//!   onboarding started makes the step done at start.
//! - `sop_acknowledgement`: done when nothing is pending for the employee
//!   ([`evaluate_system_steps`], re-run on the SOP events). With no SOP for
//!   the role the step is done at start, noted *no SOPs*. A member of staff
//!   records the acknowledgement in person through the kernel's
//!   `SopAcknowledgements`.
//!
//! Waiving needs `onboarding.manage` in the employee's scope, a reason and a
//! user who is not an assignee of the step; reopening needs the same
//! permission, a reason and a user other than the one who marked the step. A
//! step done by the system (checklist, document, SOP) is not reopened: it
//! follows its evidence; a waived step of any kind is. The onboarding
//! completes after the last required step ([`complete_if_done`]).
//!
//! Each service checks who may act and runs exactly one transition of the
//! step machine ([`crate::onboarding_machine`]) with the user in the run's
//! metadata, so the history keeps who did what even after a `reopen`.

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::StateMachine;
use wl_kernel::documents::{
    DocumentAccepted, SopStaff, Validity, guard as document_guard, sop_acknowledgements, today,
};
use wl_kernel::tickets::{ChecklistCompleted, ChecklistOutcome, ChecklistRunId, checklists};
use wl_kernel::workforce::{EMPLOYEE_SUBJECT_TYPE, OnboardingCompleted};
use wl_kernel::Subject;

use crate::employee::Employee;
use crate::onboarding::{Onboarding, OnboardingStatus, OnboardingStep, StepStatus};
use crate::onboarding_machine::{COMPLETE, CompleteInput, MARK_DONE, REOPEN, StepRun, WAIVE};
use crate::permissions::ONBOARDING_MANAGE;
use crate::template::{OWNER_EMPLOYEE, StepKind};
use crate::{assignees, notices};

/// A drawn signature recorded in person (`HRM.ACL.10`): the staff user who
/// acts is the witness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InPerson {
    /// The signature, as drawn on the staff device.
    pub signature: String,
}

/// What a user gives when marking a step done.
#[derive(Debug, Clone, Default)]
pub struct DoneInput {
    /// An evidence note.
    pub note: Option<String>,
    /// An evidence document.
    pub document_id: Option<i64>,
    /// Recorded in person for the employee.
    pub in_person: Option<InPerson>,
}

/// Why a step was not done, waived or reopened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepRefused {
    /// There is no such step.
    NotFound,
    /// The onboarding is no longer in progress.
    OnboardingClosed,
    /// The step is not open.
    NotOpen,
    /// The step is not done or waived.
    NotSettled,
    /// The user is not an assignee of the step.
    NotAssignee,
    /// A step of the employee is marked by the employee, or in person.
    EmployeeOnly,
    /// An in-person record needs the drawn signature.
    SignatureRequired,
    /// The user does not hold `onboarding.manage` in the employee's scope.
    NotManager,
    /// The step needs an evidence file or note.
    EvidenceRequired,
    /// The step is done by the system from its evidence, not by a user.
    ByEvidence,
    /// A reason is needed.
    ReasonRequired,
    /// An assignee cannot waive their own step.
    AssigneeCannotWaive,
    /// The one who marked the step cannot reopen it.
    SameUser,
    /// A step done by the system is not reopened.
    CannotReopen,
    /// The step is not a checklist step, or ticketing started no run.
    NoChecklist,
}

impl StepRefused {
    /// The English explanation.
    pub fn message(self) -> String {
        let key = match self {
            StepRefused::NotFound => "wl_hr.steps.refused.not_found",
            StepRefused::OnboardingClosed => "wl_hr.steps.refused.closed",
            StepRefused::NotOpen => "wl_hr.steps.refused.not_open",
            StepRefused::NotSettled => "wl_hr.steps.refused.not_settled",
            StepRefused::NotAssignee => "wl_hr.steps.refused.not_assignee",
            StepRefused::EmployeeOnly => "wl_hr.steps.refused.employee_only",
            StepRefused::SignatureRequired => "wl_hr.steps.refused.signature",
            StepRefused::NotManager => "wl_hr.steps.refused.not_manager",
            StepRefused::EvidenceRequired => "wl_hr.steps.refused.evidence",
            StepRefused::ByEvidence => "wl_hr.steps.refused.by_evidence",
            StepRefused::ReasonRequired => "wl_hr.steps.refused.reason",
            StepRefused::AssigneeCannotWaive => "wl_hr.steps.refused.assignee_waive",
            StepRefused::SameUser => "wl_hr.steps.refused.same_user",
            StepRefused::CannotReopen => "wl_hr.steps.refused.cannot_reopen",
            StepRefused::NoChecklist => "wl_hr.steps.refused.no_checklist",
        };
        crate::texts::english(key, &[])
    }

    /// The refusal as an HTTP 422 (404 for an unknown step, 403 for a user
    /// who may not act).
    pub fn into_error(self) -> Error {
        match self {
            StepRefused::NotFound => Error::NotFound,
            StepRefused::NotAssignee | StepRefused::NotManager | StepRefused::EmployeeOnly => {
                Error::Forbidden
            }
            _ => Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message()),
        }
    }
}

type Refusal<T> = std::result::Result<T, StepRefused>;

/// A step with what the checks need.
struct Loaded {
    step: OnboardingStep,
    onboarding: Onboarding,
    employee: Employee,
    scopes: Vec<Subject>,
}

async fn load(state: &AppState, step_id: i64) -> Result<Refusal<Loaded>> {
    let Some(step) = OnboardingStep::find(&state.db, step_id).await? else {
        return Ok(Err(StepRefused::NotFound));
    };
    let onboarding = Onboarding::find_or_404(&state.db, step.onboarding_id).await?;
    let employee = Employee::find_or_404(&state.db, onboarding.employee_id).await?;
    let scopes = employee.scopes(&state.db).await?;
    Ok(Ok(Loaded {
        step,
        onboarding,
        employee,
        scopes,
    }))
}

/// Whether `user` holds `onboarding.manage` in one of `scopes` (globally,
/// when the employee has none).
pub async fn holds_manage(state: &AppState, user: i64, scopes: &[Subject]) -> Result<bool> {
    let scopes: Vec<Scope> = if scopes.is_empty() {
        vec![Scope::global()]
    } else {
        scopes
            .iter()
            .map(|s| Scope::new(&s.subject_type, s.subject_id))
            .collect()
    };
    for scope in &scopes {
        if users_with_permission_in(&state.db, ONBOARDING_MANAGE, scope)
            .await?
            .iter()
            .any(|u| u.id == user)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether steps of `onboarding` can still be done or waived: it is in
/// progress, or it completed and an optional step is still open. A cancelled
/// onboarding has no open step left.
fn accepts_work(onboarding: &Onboarding) -> bool {
    matches!(
        onboarding.status,
        OnboardingStatus::InProgress | OnboardingStatus::Completed
    )
}

/// The reason, trimmed; `None` when blank.
fn trimmed(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Marks step `step_id` done as user `by`: a `task` by an assignee (the
/// employee's own steps by the employee, or in person with the signature and
/// `by` as witness), a `sop_acknowledgement` step in person. A `checklist` or
/// `document` step is done by its evidence, not by a user.
pub async fn mark_done(
    state: &AppState,
    step_id: i64,
    by: i64,
    input: DoneInput,
) -> Result<Refusal<OnboardingStep>> {
    let ctx = match load(state, step_id).await? {
        Ok(ctx) => ctx,
        Err(refused) => return Ok(Err(refused)),
    };
    let Loaded {
        step,
        onboarding,
        employee,
        scopes,
    } = ctx;
    if !accepts_work(&onboarding) {
        return Ok(Err(StepRefused::OnboardingClosed));
    }
    if step.status != StepStatus::Open {
        return Ok(Err(StepRefused::NotOpen));
    }
    let note = input.note.as_deref().and_then(trimmed);
    let mut run = StepRun {
        by: Some(by),
        note: note.clone(),
        document_id: input.document_id,
        ..StepRun::default()
    };
    let mut in_person = false;
    match step.kind {
        StepKind::Checklist | StepKind::Document => return Ok(Err(StepRefused::ByEvidence)),
        StepKind::SopAcknowledgement => {
            // Recorded in person only: the employee acknowledges in their
            // own session through wl-documents.
            let Some(person) = &input.in_person else {
                return Ok(Err(StepRefused::EmployeeOnly));
            };
            let Some(signature) = trimmed(&person.signature) else {
                return Ok(Err(StepRefused::SignatureRequired));
            };
            if !holds_manage(state, by, &scopes).await? {
                return Ok(Err(StepRefused::NotManager));
            }
            let who = SopStaff {
                employee: employee.subject(),
                role: employee.job_role.clone(),
                scopes: scopes.clone(),
            };
            sop_acknowledgements(state)
                .acknowledge_in_person(state, &who, &signature, by)
                .await?;
            run.by = employee.user_id;
            run.witness_id = Some(by);
            run.signature = Some(signature);
            in_person = true;
            if !sop_acknowledged_by(state, &employee, &scopes).await? {
                // Something is still pending: the step stays open.
                return Ok(Ok(OnboardingStep::find_or_404(&state.db, step_id).await?));
            }
        }
        StepKind::Task => {
            if step.owner == OWNER_EMPLOYEE {
                match &input.in_person {
                    Some(person) => {
                        let Some(signature) = trimmed(&person.signature) else {
                            return Ok(Err(StepRefused::SignatureRequired));
                        };
                        if !holds_manage(state, by, &scopes).await? {
                            return Ok(Err(StepRefused::NotManager));
                        }
                        run.by = employee.user_id;
                        run.witness_id = Some(by);
                        run.signature = Some(signature);
                        in_person = true;
                    }
                    None => {
                        if employee.user_id != Some(by) {
                            return Ok(Err(StepRefused::EmployeeOnly));
                        }
                    }
                }
            } else if !step.assignees(&state.db).await?.contains(&by) {
                return Ok(Err(StepRefused::NotAssignee));
            }
            if step.evidence_required && note.is_none() && input.document_id.is_none() {
                return Ok(Err(StepRefused::EvidenceRequired));
            }
        }
    }
    let _outcome = step.transition(MARK_DONE)
        .by_system()
        .input(&run)
        .meta(json!({ "ran_by": by }))
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let done = OnboardingStep::find_or_404(&state.db, step_id).await?;
    if in_person {
        let witness = renox::auth::User::find(&state.db, by)
            .await?
            .map_or_else(|| format!("#{by}"), |u| u.name);
        let at = wl_kernel::documents::dubai_date(now()).to_string();
        notices::in_person_recorded(state, &employee, &done, &witness, &at).await?;
    }
    complete_if_done(state, onboarding.id).await?;
    Ok(Ok(done))
}

/// Whether nothing is pending any more for `employee`.
async fn sop_acknowledged_by(state: &AppState, employee: &Employee, scopes: &[Subject]) -> Result<bool> {
    let who = SopStaff {
        employee: employee.subject(),
        role: employee.job_role.clone(),
        scopes: scopes.to_vec(),
    };
    Ok(sop_acknowledgements(state)
        .pending_for(state, &who)
        .await?
        .is_empty())
}

/// Waives open step `step_id` as user `by`, with `reason`: `by` holds
/// `onboarding.manage` in the employee's scope and is not an assignee of the
/// step (`HRM.ACL.10`).
pub async fn waive(
    state: &AppState,
    step_id: i64,
    by: i64,
    reason: &str,
) -> Result<Refusal<OnboardingStep>> {
    let Loaded {
        step,
        onboarding,
        scopes,
        ..
    } = match load(state, step_id).await? {
        Ok(ctx) => ctx,
        Err(refused) => return Ok(Err(refused)),
    };
    if !accepts_work(&onboarding) {
        return Ok(Err(StepRefused::OnboardingClosed));
    }
    if step.status != StepStatus::Open {
        return Ok(Err(StepRefused::NotOpen));
    }
    let Some(reason) = trimmed(reason) else {
        return Ok(Err(StepRefused::ReasonRequired));
    };
    if !holds_manage(state, by, &scopes).await? {
        return Ok(Err(StepRefused::NotManager));
    }
    if step.assignees(&state.db).await?.contains(&by) {
        return Ok(Err(StepRefused::AssigneeCannotWaive));
    }
    let _outcome = step.transition(WAIVE)
        .by_system()
        .reason(reason)
        .input(&StepRun {
            by: Some(by),
            ..StepRun::default()
        })
        .meta(json!({ "ran_by": by }))
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    complete_if_done(state, onboarding.id).await?;
    Ok(Ok(OnboardingStep::find_or_404(&state.db, step_id).await?))
}

/// Reopens done or waived step `step_id` as user `by`, with `reason`: `by`
/// holds `onboarding.manage`, is not the one who marked or waived it, and the
/// onboarding is `in_progress`. A done step is reopened only when it is a
/// `task`; a `document` or `sop_acknowledgement` step that is reopened is
/// evaluated again at once and is `done` again when its evidence is there.
pub async fn reopen(
    state: &AppState,
    step_id: i64,
    by: i64,
    reason: &str,
) -> Result<Refusal<OnboardingStep>> {
    let Loaded {
        step,
        onboarding,
        employee,
        scopes,
    } = match load(state, step_id).await? {
        Ok(ctx) => ctx,
        Err(refused) => return Ok(Err(refused)),
    };
    if onboarding.status != OnboardingStatus::InProgress {
        return Ok(Err(StepRefused::OnboardingClosed));
    }
    let marked_by = match step.status {
        StepStatus::Done => {
            if step.kind != StepKind::Task {
                return Ok(Err(StepRefused::CannotReopen));
            }
            step.done_by
        }
        StepStatus::Waived => step.waived_by,
        _ => return Ok(Err(StepRefused::NotSettled)),
    };
    let Some(reason) = trimmed(reason) else {
        return Ok(Err(StepRefused::ReasonRequired));
    };
    if !holds_manage(state, by, &scopes).await? {
        return Ok(Err(StepRefused::NotManager));
    }
    if marked_by == Some(by) {
        return Ok(Err(StepRefused::SameUser));
    }
    let assignees = step.assignees(&state.db).await?;
    let _outcome = step.transition(REOPEN)
        .by_system()
        .reason(reason.clone())
        .meta(json!({ "ran_by": by }))
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let reopened = OnboardingStep::find_or_404(&state.db, step_id).await?;
    notices::step_reopened(state, &employee, &reopened, &assignees, &reason).await?;
    evaluate_system_steps(state, onboarding.id).await?;
    Ok(Ok(OnboardingStep::find_or_404(&state.db, step_id).await?))
}

/// Starts a checklist run on open `checklist` step `step_id` (`WRK.CHK.01`)
/// for an assignee or a holder of `onboarding.manage`; a new run replaces
/// the one that failed.
pub async fn start_checklist(
    state: &AppState,
    step_id: i64,
    by: i64,
) -> Result<Refusal<OnboardingStep>> {
    let Loaded {
        step,
        onboarding,
        scopes,
        ..
    } = match load(state, step_id).await? {
        Ok(ctx) => ctx,
        Err(refused) => return Ok(Err(refused)),
    };
    if !accepts_work(&onboarding) {
        return Ok(Err(StepRefused::OnboardingClosed));
    }
    if step.status != StepStatus::Open {
        return Ok(Err(StepRefused::NotOpen));
    }
    let Some(template) = step
        .checklist_template
        .clone()
        .filter(|_| step.kind == StepKind::Checklist)
    else {
        return Ok(Err(StepRefused::NoChecklist));
    };
    if !step.assignees(&state.db).await?.contains(&by) && !holds_manage(state, by, &scopes).await? {
        return Ok(Err(StepRefused::NotAssignee));
    }
    let Some(run) = checklists(state)
        .start(state, &template, step.subject())
        .await?
    else {
        return Ok(Err(StepRefused::NoChecklist));
    };
    OnboardingStep::query()
        .where_(OnboardingStep::ID.eq(step.id))
        .update(
            &state.db,
            &[
                ("checklist_run_id", &Some(run.0)),
                ("failed_items", &None::<String>),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(Ok(OnboardingStep::find_or_404(&state.db, step_id).await?))
}

/// Runs `mark_done` on `step` for the system, with `run` as its evidence.
async fn done_by_system(state: &AppState, step: &OnboardingStep, run: StepRun) -> Result {
    let _outcome = step.transition(MARK_DONE)
        .by_system()
        .input(&run)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    Ok(())
}

/// `ChecklistCompleted`: a passed run marks its step done; a failed one
/// leaves the step open with the failed items shown. A second event for a
/// step that is no longer open is ignored.
pub async fn on_checklist_completed(state: &AppState, event: &ChecklistCompleted) -> Result {
    let Some(step) = OnboardingStep::query()
        .where_(OnboardingStep::CHECKLIST_RUN_ID.eq(event.run.0))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    if step.status != StepStatus::Open {
        return Ok(());
    }
    match event.outcome {
        ChecklistOutcome::Passed => {
            done_by_system(
                state,
                &step,
                StepRun {
                    checklist_run_id: Some(event.run.0),
                    ..StepRun::default()
                },
            )
            .await?;
            complete_if_done(state, step.onboarding_id).await
        }
        ChecklistOutcome::Failed => {
            OnboardingStep::query()
                .where_(OnboardingStep::ID.eq(step.id))
                .update(
                    &state.db,
                    &[
                        ("failed_items", &Some(event.failed_items.join("; "))),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            Ok(())
        }
    }
}

/// The open steps of the `in_progress` onboardings, of kind `kind`.
async fn open_steps_of_kind(
    db: &Db,
    kind: StepKind,
    onboarding: Option<i64>,
    employee: Option<i64>,
) -> Result<Vec<OnboardingStep>> {
    let mut query = OnboardingStep::query()
        .where_(OnboardingStep::STATUS.eq(StepStatus::Open))
        .where_(OnboardingStep::KIND.eq(kind));
    if let Some(id) = onboarding {
        query = query.where_(OnboardingStep::ONBOARDING_ID.eq(id));
    }
    let mut steps = Vec::new();
    for step in query.order_by(OnboardingStep::ID).get(db).await? {
        let Some(owner) = Onboarding::find(db, step.onboarding_id).await? else {
            continue;
        };
        if owner.status != OnboardingStatus::InProgress
            || employee.is_some_and(|e| e != owner.employee_id)
        {
            continue;
        }
        steps.push(step);
    }
    Ok(steps)
}

/// `DocumentAccepted` for an employee: the open `document` steps of the kind
/// are done with that document.
pub async fn on_document_accepted(state: &AppState, event: &DocumentAccepted) -> Result {
    if event.subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
        return Ok(());
    }
    let steps = open_steps_of_kind(
        &state.db,
        StepKind::Document,
        None,
        Some(event.subject.subject_id),
    )
    .await?;
    for step in steps
        .iter()
        .filter(|s| s.document_kind.as_deref() == Some(event.kind.as_str()))
    {
        done_by_system(
            state,
            step,
            StepRun {
                document_id: Some(event.document.0),
                ..StepRun::default()
            },
        )
        .await?;
        complete_if_done(state, step.onboarding_id).await?;
    }
    Ok(())
}

/// Re-evaluates the open `document` and `sop_acknowledgement` steps of
/// `onboarding_id`: a `document` step whose kind has an accepted document on
/// the employee, and an SOP step with nothing pending for the employee, are
/// `done` (an SOP step with no SOP at all is noted *no SOPs*). Then the
/// onboarding completes if that was the last required step.
pub async fn evaluate_system_steps(state: &AppState, onboarding_id: i64) -> Result {
    let Some(onboarding) = Onboarding::find(&state.db, onboarding_id).await? else {
        return Ok(());
    };
    if onboarding.status != OnboardingStatus::InProgress {
        return Ok(());
    }
    let employee = Employee::find_or_404(&state.db, onboarding.employee_id).await?;
    let scopes = employee.scopes(&state.db).await?;
    for step in open_steps_of_kind(&state.db, StepKind::Document, Some(onboarding_id), None).await? {
        let Some(kind) = step.document_kind.clone() else {
            continue;
        };
        let validity = document_guard(state)
            .is_valid(&state.db, &employee.subject(), &kind, today())
            .await?;
        // Without the documents module nothing is accepted.
        if let Validity::Valid {
            document: Some(document),
            ..
        } = validity
        {
            done_by_system(
                state,
                &step,
                StepRun {
                    document_id: Some(document.0),
                    ..StepRun::default()
                },
            )
            .await?;
        }
    }
    for step in open_steps_of_kind(&state.db, StepKind::SopAcknowledgement, Some(onboarding_id), None).await? {
        if sop_acknowledged_by(state, &employee, &scopes).await? {
            done_by_system(state, &step, StepRun::default()).await?;
            OnboardingStep::query()
                .where_(OnboardingStep::ID.eq(step.id))
                .update(
                    &state.db,
                    &[
                        (
                            "note",
                            &Some(crate::texts::english("wl_hr.steps.no_sops", &[])),
                        ),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
    }
    complete_if_done(state, onboarding_id).await
}

/// Re-evaluates the open SOP steps of every `in_progress` onboarding: on
/// `SopPublished`, `SopAcknowledged`, `SopWithdrawn` and `SopReadersChanged`.
pub async fn on_sop_event(state: &AppState) -> Result {
    let mut seen: Vec<i64> = Vec::new();
    for step in open_steps_of_kind(&state.db, StepKind::SopAcknowledgement, None, None).await? {
        if !seen.contains(&step.onboarding_id) {
            seen.push(step.onboarding_id);
            evaluate_system_steps(state, step.onboarding_id).await?;
        }
    }
    Ok(())
}

/// Completes `onboarding_id` when every required step is `done` or `waived`:
/// optional steps still open are left open; HR and the manager are told and
/// `OnboardingCompleted` follows the commit.
pub async fn complete_if_done(state: &AppState, onboarding_id: i64) -> Result {
    let Some(onboarding) = Onboarding::find(&state.db, onboarding_id).await? else {
        return Ok(());
    };
    if onboarding.status != OnboardingStatus::InProgress {
        return Ok(());
    }
    let open_required = onboarding
        .steps(&state.db)
        .await?
        .iter()
        .filter(|s| s.required && s.status == StepStatus::Open)
        .count();
    if open_required > 0 {
        return Ok(());
    }
    let _outcome = onboarding
        .transition(COMPLETE)
        .by_system()
        .input(&CompleteInput { open_required })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    let employee = Employee::find_or_404(&state.db, onboarding.employee_id).await?;
    let scopes = employee.scopes(&state.db).await?;
    state
        .emit(OnboardingCompleted {
            onboarding: onboarding.onboarding_id(),
            employee: employee.employee_id(),
            job_role: employee.job_role.clone(),
            scope: scopes.first().cloned(),
            trigger: onboarding.start_trigger.clone(),
        })
        .await?;
    let mut told = assignees::hr_holders(state, &scopes).await?;
    told.extend(assignees::resolve(state, &employee, &scopes, "manager").await?);
    told.sort_unstable();
    told.dedup();
    notices::completed(state, &employee, &onboarding, &told).await
}

/// The checklist run `run` of an onboarding step, if there is one.
pub async fn step_of_run(db: &Db, run: ChecklistRunId) -> Result<Option<OnboardingStep>> {
    OnboardingStep::query()
        .where_(OnboardingStep::CHECKLIST_RUN_ID.eq(run.0))
        .first(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_text_is_no_text() {
        assert_eq!(trimmed("  "), None);
        assert_eq!(trimmed(" ok "), Some("ok".to_owned()));
    }

    #[test]
    fn who_may_not_act_is_forbidden_and_the_rest_is_unprocessable() {
        for refused in [
            StepRefused::NotAssignee,
            StepRefused::NotManager,
            StepRefused::EmployeeOnly,
        ] {
            assert!(matches!(refused.into_error(), Error::Forbidden));
        }
        assert!(matches!(StepRefused::NotFound.into_error(), Error::NotFound));
        assert!(matches!(
            StepRefused::EvidenceRequired.into_error(),
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)
        ));
        assert!(!StepRefused::SameUser.message().is_empty());
    }
}
