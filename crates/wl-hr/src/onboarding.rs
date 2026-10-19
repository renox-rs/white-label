//! Onboarding (wl-hr §3 *Onboarding*, §4 *Onboarding*, §5.8, issue #216): one
//! employee's run through the steps of the templates that apply to their role.
//!
//! [`start`] is the `start` of the spec: from every active template that
//! applies (`hire`, `HRM.RUL.16`, run by [`start_on_hire`] when an employee is
//! created in `onboarding`) or from the templates HR chooses (`manual`, e.g.
//! for a role change; a manual onboarding blocks nothing). The templates are
//! frozen: each step is copied with its template's version, one step per
//! template step, assignees resolved ([`crate::assignees`]), the due date the
//! start date plus the template step's due days (calendar days in
//! Asia/Dubai; a due date on a holiday is kept). A second onboarding is not
//! started from a template that is already `in_progress` for the employee.
//!
//! Assignees follow the people: [`re_resolve`] gives the open steps their
//! current holders (called on `EmployeeAssignmentChanged`, and by the host
//! when a role assignment of a manager or an HR user changes); with nobody
//! left the step goes to HR. [`recompute_due_dates`] follows a corrected hire
//! date. The onboarding cancels with the exit of the employee
//! ([`cancel_for`]).
//!
//! The dependency BL-HRM-05 (`cancel_onboarding`) is not built yet: the
//! onboarding listens to the kernel's `EmployeeOnboardingCancelled`, which
//! that transition will emit, and the check that a cancelled employee's
//! onboarding is the one the transition names is added with it (F2).

use renox::chrono::{Days, NaiveDate};
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::workforce::{OnboardingId, OnboardingStarted};
use wl_kernel::{Subject, checklists};

use crate::assignees;
use crate::employee::Employee;
use crate::onboarding_machine::{CANCEL, CANCEL_STEP};
use crate::template::{OnboardingTemplate, StepKind, TemplateStep};

/// Where an onboarding stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OnboardingStatus {
    /// Steps open.
    #[default]
    InProgress,
    /// Every required step `done` or `waived`.
    Completed,
    /// The employee never started or left before the end.
    Cancelled,
}

/// Where a step stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StepStatus {
    /// To do.
    #[default]
    Open,
    /// Done, with its evidence.
    Done,
    /// Does not apply to this employee.
    Waived,
    /// The onboarding was cancelled.
    Cancelled,
}

/// One employee's onboarding.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_onboardings", state_machine, index(employee_id, status))]
pub struct Onboarding {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// `hire` or `manual`.
    pub start_trigger: String,
    /// The date the due days count from.
    pub started_on: NaiveDate,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// Where it stands; changed only by the machine.
    pub status: OnboardingStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A template an onboarding was started from, at its frozen version.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_onboarding_templates_applied", unique(onboarding_id, template_id))]
pub struct AppliedTemplate {
    /// The primary key.
    pub id: i64,
    /// The onboarding.
    #[model(references = "wl_hr_onboardings")]
    pub onboarding_id: i64,
    /// The template.
    pub template_id: i64,
    /// Its version when the onboarding started.
    pub template_version: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One step for one employee.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_onboarding_steps", state_machine, index(onboarding_id, status))]
pub struct OnboardingStep {
    /// The primary key.
    pub id: i64,
    /// The onboarding.
    #[model(references = "wl_hr_onboardings")]
    pub onboarding_id: i64,
    /// The template the step comes from.
    pub template_id: i64,
    /// The template step it was copied from.
    pub template_step_id: i64,
    /// The order, from 1 across the templates.
    pub position: i64,
    /// The title in English.
    pub title_en: String,
    /// The title in Arabic.
    pub title_ar: String,
    /// `hr`, `manager`, `employee` or `role:<name>`.
    pub owner: String,
    /// What the step asks for.
    pub kind: StepKind,
    /// Whether the onboarding needs it to complete.
    pub required: bool,
    /// Whether a `task` step needs an evidence file or note.
    pub evidence_required: bool,
    /// The checklist template key.
    pub checklist_template: Option<String>,
    /// The document kind.
    pub document_kind: Option<String>,
    /// Help for whoever does the step.
    pub help_text: String,
    /// Days after the start date.
    pub due_days: i64,
    /// The due date.
    pub due_date: NaiveDate,
    /// Where it stands; changed only by the machine.
    pub status: StepStatus,
    /// Who marked it done.
    pub done_by: Option<i64>,
    /// When it was done.
    pub done_at: Option<DateTime>,
    /// The evidence note.
    pub evidence_note: Option<String>,
    /// The evidence document.
    pub evidence_document_id: Option<i64>,
    /// The checklist run started on the step.
    pub checklist_run_id: Option<i64>,
    /// The failed items of the last checklist run.
    pub failed_items: Option<String>,
    /// The staff user who recorded it in person.
    pub witness_id: Option<i64>,
    /// The drawn signature of an in-person record.
    pub signature: Option<String>,
    /// Who waived it.
    pub waived_by: Option<i64>,
    /// Why it was waived.
    pub waive_reason: Option<String>,
    /// A note, e.g. *no SOPs*.
    pub note: Option<String>,
    /// The day the assignees were last reminded (`HRM.RUL.17`).
    pub last_reminded_on: Option<NaiveDate>,
    /// The day HR and the manager were told it is overdue; once.
    pub escalated_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A user a step is assigned to.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_onboarding_step_assignees", unique(step_id, user_id))]
pub struct StepAssignee {
    /// The primary key.
    pub id: i64,
    /// The step.
    #[model(references = "wl_hr_onboarding_steps")]
    pub step_id: i64,
    /// The user.
    pub user_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Onboarding {
    /// The onboarding as an id.
    pub fn onboarding_id(&self) -> OnboardingId {
        OnboardingId(self.id)
    }

    /// The onboarding as a kernel subject.
    pub fn subject(&self) -> Subject {
        self.onboarding_id().subject()
    }

    /// The steps, in order.
    pub async fn steps(&self, db: &Db) -> Result<Vec<OnboardingStep>> {
        OnboardingStep::query()
            .where_(OnboardingStep::ONBOARDING_ID.eq(self.id))
            .order_by(OnboardingStep::POSITION)
            .get(db)
            .await
    }

    /// The templates it was started from.
    pub async fn templates(&self, db: &Db) -> Result<Vec<AppliedTemplate>> {
        AppliedTemplate::query()
            .where_(AppliedTemplate::ONBOARDING_ID.eq(self.id))
            .order_by(AppliedTemplate::ID)
            .get(db)
            .await
    }

    /// The onboardings of `employee`, newest first.
    pub async fn of_employee(db: &Db, employee: i64) -> Result<Vec<Onboarding>> {
        Onboarding::query()
            .where_(Onboarding::EMPLOYEE_ID.eq(employee))
            .order_by_desc(Onboarding::ID)
            .get(db)
            .await
    }
}

impl OnboardingStep {
    /// The step as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(wl_kernel::workforce::ONBOARDING_STEP_SUBJECT_TYPE, self.id)
    }

    /// The users it is assigned to, by id.
    pub async fn assignees(&self, db: &Db) -> Result<Vec<i64>> {
        Ok(StepAssignee::query()
            .where_(StepAssignee::STEP_ID.eq(self.id))
            .order_by(StepAssignee::USER_ID)
            .get(db)
            .await?
            .into_iter()
            .map(|a| a.user_id)
            .collect())
    }

    /// Whether the step is open and past its due date on `day`: *overdue*
    /// is shown from the day after the due date.
    pub fn overdue_on(&self, day: NaiveDate) -> bool {
        self.status == StepStatus::Open && day > self.due_date
    }

    /// Whether the step is `done` or `waived`.
    pub fn is_settled(&self) -> bool {
        matches!(self.status, StepStatus::Done | StepStatus::Waived)
    }
}

/// `start` plus `days` calendar days.
pub fn due_date(start: NaiveDate, days: i64) -> NaiveDate {
    start
        .checked_add_days(Days::new(u64::try_from(days.max(0)).unwrap_or(0)))
        .unwrap_or(start)
}

/// How an onboarding starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartTrigger {
    /// `HRM.RUL.16`: the employee was created in `onboarding`.
    Hire,
    /// HR started it (`onboarding.manage`).
    Manual,
}

impl StartTrigger {
    /// The stored name.
    pub fn name(self) -> &'static str {
        match self {
            StartTrigger::Hire => "hire",
            StartTrigger::Manual => "manual",
        }
    }
}

/// What starts an onboarding.
#[derive(Debug, Clone)]
pub struct StartRequest {
    /// The employee.
    pub employee_id: i64,
    /// `hire` or `manual`.
    pub trigger: StartTrigger,
    /// The templates to start; empty for every applicable one.
    pub templates: Vec<i64>,
    /// The date the due days count from: the hire date for `hire`, today
    /// for `manual` when not given.
    pub start_on: Option<NaiveDate>,
}

/// Why an onboarding was not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefused {
    /// There is no such employee.
    EmployeeNotFound,
    /// No active template applies, or a chosen one is unknown or switched off.
    NoTemplate,
    /// Every template is already `in_progress` for the employee.
    AlreadyRunning,
}

impl StartRefused {
    /// The English explanation.
    pub fn message(self) -> String {
        let key = match self {
            StartRefused::EmployeeNotFound => "wl_hr.onboarding.employee_not_found",
            StartRefused::NoTemplate => "wl_hr.onboarding.no_template",
            StartRefused::AlreadyRunning => "wl_hr.onboarding.already_running",
        };
        crate::texts::english(key, &[])
    }

    /// The refusal as an HTTP 422 (404 for an unknown employee).
    pub fn into_error(self) -> Error {
        match self {
            StartRefused::EmployeeNotFound => Error::NotFound,
            _ => Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message()),
        }
    }
}

/// The ids of the templates `in_progress` onboardings of `employee` were
/// started from.
async fn running_templates(db: &Db, employee: i64) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    let running = Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee))
        .where_(Onboarding::STATUS.eq(OnboardingStatus::InProgress))
        .get(db)
        .await?;
    for onboarding in running {
        ids.extend(onboarding.templates(db).await?.into_iter().map(|t| t.template_id));
    }
    Ok(ids)
}

/// Starts an onboarding for the employee in `request` (the `start` of §4).
/// The first steps' assignees are told and `OnboardingStarted` follows the
/// commit; steps that are already settled by what exists (an accepted
/// document, no SOP to acknowledge) are done at once.
pub async fn start(
    state: &AppState,
    request: StartRequest,
) -> Result<std::result::Result<Onboarding, StartRefused>> {
    let Some(employee) = Employee::find(&state.db, request.employee_id).await? else {
        return Ok(Err(StartRefused::EmployeeNotFound));
    };
    let mut templates: Vec<OnboardingTemplate> = if request.templates.is_empty() {
        OnboardingTemplate::applicable(&state.db, &employee.job_role, employee.employment_type)
            .await?
    } else {
        let mut chosen = Vec::new();
        for id in &request.templates {
            match OnboardingTemplate::find(&state.db, *id).await? {
                Some(t) if t.active => chosen.push(t),
                _ => return Ok(Err(StartRefused::NoTemplate)),
            }
        }
        chosen
    };
    if templates.is_empty() {
        return Ok(Err(StartRefused::NoTemplate));
    }
    let running = running_templates(&state.db, employee.id).await?;
    templates.retain(|t| !running.contains(&t.id));
    if templates.is_empty() {
        return Ok(Err(StartRefused::AlreadyRunning));
    }
    let started_on = match (request.start_on, request.trigger) {
        (Some(day), _) => day,
        (None, StartTrigger::Hire) => employee.hire_date,
        (None, StartTrigger::Manual) => today(),
    };
    // Everything that reads before the transaction holds the connection.
    let scopes = employee.scopes(&state.db).await?;
    let mut owners: Vec<(String, Vec<i64>)> = Vec::new();
    let mut plan: Vec<(OnboardingTemplate, TemplateStep)> = Vec::new();
    for template in &templates {
        for step in template.steps(&state.db).await? {
            if !owners.iter().any(|(o, _)| *o == step.owner) {
                let ids = assignees::resolve(state, &employee, &scopes, &step.owner).await?;
                owners.push((step.owner.clone(), ids));
            }
            plan.push((template.clone(), step));
        }
    }
    let mut tx = state.db.begin().await?;
    let onboarding = Onboarding::create(
        &mut tx,
        Onboarding {
            employee_id: employee.id,
            start_trigger: request.trigger.name().to_owned(),
            started_on,
            ..Onboarding::default()
        },
    )
    .await?;
    for template in &templates {
        AppliedTemplate::create(
            &mut tx,
            AppliedTemplate {
                onboarding_id: onboarding.id,
                template_id: template.id,
                template_version: template.version,
                ..AppliedTemplate::default()
            },
        )
        .await?;
    }
    let mut created = Vec::new();
    for (position, (template, step)) in plan.iter().enumerate() {
        let row = OnboardingStep::create(
            &mut tx,
            OnboardingStep {
                onboarding_id: onboarding.id,
                template_id: template.id,
                template_step_id: step.id,
                position: i64::try_from(position).unwrap_or(0) + 1,
                title_en: step.title_en.clone(),
                title_ar: step.title_ar.clone(),
                owner: step.owner.clone(),
                kind: step.kind,
                required: step.required,
                evidence_required: step.evidence_required,
                checklist_template: step.checklist_template.clone(),
                document_kind: step.document_kind.clone(),
                help_text: step.help_text.clone(),
                due_days: step.due_days,
                due_date: due_date(started_on, step.due_days),
                ..OnboardingStep::default()
            },
        )
        .await?;
        let users = owners
            .iter()
            .find(|(o, _)| *o == step.owner)
            .map(|(_, ids)| ids.clone())
            .unwrap_or_default();
        for user in &users {
            StepAssignee::create(
                &mut tx,
                StepAssignee {
                    step_id: row.id,
                    user_id: *user,
                    ..StepAssignee::default()
                },
            )
            .await?;
        }
        created.push((row, users));
    }
    tx.commit().await?;
    state
        .emit(OnboardingStarted {
            onboarding: onboarding.onboarding_id(),
            employee: employee.employee_id(),
            job_role: employee.job_role.clone(),
            scope: scopes.first().cloned(),
            trigger: request.trigger.name().to_owned(),
        })
        .await?;
    // The assignees of the first steps are told.
    let first = created.iter().map(|(s, _)| s.due_days).min();
    for (step, users) in created.iter().filter(|(s, _)| Some(s.due_days) == first) {
        crate::notices::step_assigned(state, &employee, step, users).await?;
    }
    crate::steps::evaluate_system_steps(state, onboarding.id).await?;
    Ok(Ok(Onboarding::find_or_404(&state.db, onboarding.id).await?))
}

/// `HRM.RUL.16`: starts the onboarding of `employee`, who was created in
/// `onboarding`, from the templates that apply (trigger `hire`, due dates
/// from the hire date). `None` when no template applies.
pub async fn start_on_hire(state: &AppState, employee: &Employee) -> Result<Option<Onboarding>> {
    match start(
        state,
        StartRequest {
            employee_id: employee.id,
            trigger: StartTrigger::Hire,
            templates: Vec::new(),
            start_on: None,
        },
    )
    .await?
    {
        Ok(onboarding) => Ok(Some(onboarding)),
        Err(StartRefused::NoTemplate | StartRefused::AlreadyRunning) => Ok(None),
        Err(refused) => Err(refused.into_error()),
    }
}

/// The open steps of the `in_progress` onboardings of `employee`.
async fn open_steps(db: &Db, employee: i64) -> Result<Vec<OnboardingStep>> {
    let mut steps = Vec::new();
    let running = Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee))
        .where_(Onboarding::STATUS.eq(OnboardingStatus::InProgress))
        .get(db)
        .await?;
    for onboarding in running {
        steps.extend(
            OnboardingStep::query()
                .where_(OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
                .where_(OnboardingStep::STATUS.eq(StepStatus::Open))
                .order_by(OnboardingStep::POSITION)
                .get(db)
                .await?,
        );
    }
    Ok(steps)
}

/// Gives the open steps of `employee` their current assignees: a new manager,
/// a departed assignee, a changed scope. With nobody left a step goes to HR.
/// Done steps keep who did them. The users who are new on a step are told.
pub async fn re_resolve(state: &AppState, employee_id: i64) -> Result {
    let Some(employee) = Employee::find(&state.db, employee_id).await? else {
        return Ok(());
    };
    let scopes = employee.scopes(&state.db).await?;
    for step in open_steps(&state.db, employee_id).await? {
        let wanted = assignees::resolve(state, &employee, &scopes, &step.owner).await?;
        let current = step.assignees(&state.db).await?;
        if wanted == current {
            continue;
        }
        let mut tx = state.db.begin().await?;
        for user in current.iter().filter(|u| !wanted.contains(u)) {
            StepAssignee::query()
                .where_(StepAssignee::STEP_ID.eq(step.id))
                .where_(StepAssignee::USER_ID.eq(*user))
                .delete(&mut tx)
                .await?;
        }
        let added: Vec<i64> = wanted
            .iter()
            .copied()
            .filter(|u| !current.contains(u))
            .collect();
        for user in &added {
            StepAssignee::create(
                &mut tx,
                StepAssignee {
                    step_id: step.id,
                    user_id: *user,
                    ..StepAssignee::default()
                },
            )
            .await?;
        }
        tx.commit().await?;
        crate::notices::step_assigned(state, &employee, &step, &added).await?;
    }
    Ok(())
}

/// A corrected hire date moves the due dates of the open steps of the
/// `hire` onboardings of `employee` to the new date plus their due days, plus
/// the days of the approved leave that has started (`HRM.RUL.17`).
pub async fn recompute_due_dates(state: &AppState, employee_id: i64, hire_date: NaiveDate) -> Result {
    let leave = crate::leave::started_leave(&state.db, employee_id, today()).await?;
    let hired = Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee_id))
        .where_(Onboarding::START_TRIGGER.eq("hire"))
        .where_(Onboarding::STATUS.eq(OnboardingStatus::InProgress))
        .get(&state.db)
        .await?;
    for onboarding in hired {
        Onboarding::query()
            .where_(Onboarding::ID.eq(onboarding.id))
            .update(&state.db, &[("started_on", &hire_date), ("updated_at", &now())])
            .await?;
        for step in OnboardingStep::query()
            .where_(OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
            .where_(OnboardingStep::STATUS.eq(StepStatus::Open))
            .get(&state.db)
            .await?
        {
            OnboardingStep::query()
                .where_(OnboardingStep::ID.eq(step.id))
                .update(
                    &state.db,
                    &[
                        (
                            "due_date",
                            &crate::leave::shifted_due(
                                due_date(hire_date, step.due_days),
                                hire_date,
                                &leave,
                            ),
                        ),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
    }
    Ok(())
}

/// Cancels the `in_progress` onboardings of `employee_id` for `reason`: the
/// employee left, died or never started. Open steps become `cancelled`,
/// checklist runs in progress are voided with the reason, no further
/// reminders. Returns how many onboardings it cancelled.
pub async fn cancel_for(state: &AppState, employee_id: i64, reason: &str) -> Result<usize> {
    let running = Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee_id))
        .where_(Onboarding::STATUS.eq(OnboardingStatus::InProgress))
        .order_by(Onboarding::ID)
        .get(&state.db)
        .await?;
    let count = running.len();
    for onboarding in running {
        cancel(state, onboarding, reason).await?;
    }
    Ok(count)
}

/// Cancels one onboarding with its open steps in one transaction, then voids
/// the checklist runs started on them.
pub(crate) async fn cancel(state: &AppState, onboarding: Onboarding, reason: &str) -> Result {
    let steps = OnboardingStep::query()
        .where_(OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
        .where_(OnboardingStep::STATUS.eq(StepStatus::Open))
        .get(&state.db)
        .await?;
    let mut tx = state.db.begin().await?;
    let outcome = onboarding
        .transition(CANCEL)
        .by_system()
        .reason(reason)
        .meta(json!({ "reason": reason }))
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error())?;
    Onboarding::query()
        .where_(Onboarding::ID.eq(onboarding.id))
        .update(&mut tx, &[("cancel_reason", &Some(reason.to_owned()))])
        .await?;
    let mut outcomes = Vec::new();
    for step in &steps {
        outcomes.push(
            step.transition(CANCEL_STEP)
                .by_system()
                .run_in(&mut tx)
                .await
                .map_err(|e| e.into_error())?,
        );
    }
    tx.commit().await?;
    outcome.emit(state).await?;
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    let runner = checklists(state);
    for run in steps.iter().filter_map(|s| s.checklist_run_id) {
        runner
            .void(state, wl_kernel::tickets::ChecklistRunId(run), reason)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    #[test]
    fn due_dates_count_calendar_days_from_the_start() {
        assert_eq!(due_date(day(11, 2), 7), day(11, 9));
        assert_eq!(due_date(day(11, 5), 7), day(11, 12));
        assert_eq!(due_date(day(12, 28), 7), NaiveDate::from_ymd_opt(2027, 1, 4).unwrap());
        assert_eq!(due_date(day(11, 2), 0), day(11, 2));
    }

    #[test]
    fn a_step_is_overdue_from_the_day_after_its_due_date() {
        let step = OnboardingStep {
            due_date: day(11, 9),
            ..OnboardingStep::default()
        };
        assert!(!step.overdue_on(day(11, 9)));
        assert!(step.overdue_on(day(11, 10)));
        let done = OnboardingStep {
            status: StepStatus::Done,
            ..step
        };
        assert!(!done.overdue_on(day(12, 1)) && done.is_settled());
    }

    #[test]
    fn a_refusal_has_a_message() {
        assert!(!StartRefused::NoTemplate.message().is_empty());
        assert!(matches!(StartRefused::EmployeeNotFound.into_error(), Error::NotFound));
    }
}
