//! The employee pages and the onboarding actions (wl-hr §3, §4, issues #216
//! and #217).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/employees` | `wl_hr.employees` | `employees.view` |
//! | `GET /hr/employees/{id}` | `wl_hr.employee` | `employees.view` |
//! | `POST /hr/employees/{id}/transitions/{name}` | `wl_hr.employee_transitions.*` | `employees.view` and the transition's permission |
//! | `POST /hr/employees/{id}/onboardings` | `wl_hr.onboarding_start` | `onboarding.manage` in the employee's scope |
//! | `POST /hr/onboardings/{id}/cancel` | `wl_hr.onboarding_cancel` | `onboarding.manage` in the employee's scope |
//! | `POST /hr/steps/{id}/done` | `wl_hr.step_done` | an assignee of the step |
//! | `POST /hr/steps/{id}/in-person` | `wl_hr.step_in_person` | `onboarding.manage` in the employee's scope |
//! | `POST /hr/steps/{id}/waive` | `wl_hr.step_waive` | `onboarding.manage` in the employee's scope |
//! | `POST /hr/steps/{id}/reopen` | `wl_hr.step_reopen` | `onboarding.manage` in the employee's scope |
//! | `POST /hr/steps/{id}/checklist` | `wl_hr.step_checklist` | an assignee, or `onboarding.manage` |
//!
//! The action routes need a signed-in user and carry no permission guard of
//! their own: the guard counts global roles only, while HR may hold
//! `onboarding.manage` in one branch, and an assignee may be the employee or
//! a manager with no HR role. The services in [`crate::steps`] and the
//! handlers check who may act in the employee's scope (`HRM.ACL.10`) and
//! answer 403.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::employee::{Employee, EmployeeStatus};
use crate::onboarding::{
    Onboarding, OnboardingStatus, OnboardingStep, StartRefused, StartRequest, StartTrigger,
    StepStatus,
};
use crate::pages::templates::{kind_label, owner_label};
use crate::steps::{self, DoneInput, InPerson};
use crate::template::{OWNER_EMPLOYEE, OnboardingTemplate, StepKind};
use crate::{onboarding, permissions, texts};

/// The texts a template reads as `tx['employees.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/hr/employees", index)
                .name("wl_hr.employees")
                .get("/hr/employees/{id}", show)
                .name("wl_hr.employee")
                .group(
                    "/hr/employees",
                    "wl_hr.employee_transitions.",
                    wl_kernel::machines::routes::<Employee>(),
                )
                .require_permission(permissions::EMPLOYEES_VIEW),
        )
        .merge(
            Routes::new()
                .post("/hr/employees/{id}/onboardings", start_manual)
                .name("wl_hr.onboarding_start")
                .post("/hr/onboardings/{id}/cancel", cancel_manual)
                .name("wl_hr.onboarding_cancel")
                .post("/hr/steps/{id}/in-person", in_person)
                .name("wl_hr.step_in_person")
                .post("/hr/steps/{id}/waive", waive)
                .name("wl_hr.step_waive")
                .post("/hr/steps/{id}/reopen", reopen)
                .name("wl_hr.step_reopen")
                .post("/hr/steps/{id}/done", done)
                .name("wl_hr.step_done")
                .post("/hr/steps/{id}/checklist", checklist)
                .name("wl_hr.step_checklist")
                .require_auth(),
        )
}

/// The package text `key` (without the `wl_hr.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_hr.{key}"))
}

/// The name of user `id`, else `#id`.
async fn user_name(db: &Db, id: i64) -> String {
    User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| format!("#{id}"), |u| u.name)
}

/// The employee's name, else `#id`.
async fn employee_name(state: &AppState, lang: &Lang, employee: &Employee) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, employee.party(), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", employee.id))
}

/// The badge kind of an employee status.
fn employee_kind(status: EmployeeStatus) -> &'static str {
    match status {
        EmployeeStatus::Active => "success",
        EmployeeStatus::Onboarding | EmployeeStatus::OnLeave => "info",
        EmployeeStatus::Notice => "warning",
        EmployeeStatus::Absconded => "error",
        EmployeeStatus::Deceased | EmployeeStatus::Exited | EmployeeStatus::OnboardingCancelled => {
            "info"
        }
    }
}

/// One employee of the list.
#[derive(Serialize, Debug, Clone)]
pub struct EmployeeRow {
    /// The primary key.
    pub id: i64,
    /// The name.
    pub name: String,
    /// The employee number.
    pub number: String,
    /// The job role.
    pub job_role: String,
    /// The hire date.
    pub hire_date: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Required steps done or waived of all required steps, e.g. `3/5`.
    pub progress: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/employees.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    employees: Vec<EmployeeRow>,
}

/// The progress of the employee's `in_progress` onboardings: required steps
/// settled of required steps; empty with none.
async fn progress_of(db: &Db, employee: i64) -> Result<String> {
    let (mut settled, mut total) = (0, 0);
    let mut any = false;
    for onboarding in Onboarding::of_employee(db, employee).await? {
        if onboarding.status != OnboardingStatus::InProgress {
            continue;
        }
        any = true;
        for step in onboarding.steps(db).await?.iter().filter(|s| s.required) {
            total += 1;
            if step.is_settled() {
                settled += 1;
            }
        }
    }
    Ok(if any {
        format!("{settled}/{total}")
    } else {
        String::new()
    })
}

/// `GET /hr/employees`.
async fn index(State(state): State<AppState>, lang: Lang) -> Result<View> {
    let mut employees = Vec::new();
    for employee in Employee::query().order_by_desc(Employee::ID).get(&state.db).await? {
        employees.push(EmployeeRow {
            id: employee.id,
            name: employee_name(&state, &lang, &employee).await,
            number: employee.employee_number.clone(),
            job_role: employee.job_role.clone(),
            hire_date: employee.hire_date.to_string(),
            status: text(&lang, &format!("employees.status.{}", employee.status.as_str())),
            status_kind: employee_kind(employee.status),
            progress: progress_of(&state.db, employee.id).await?,
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        employees,
    }))
}

/// One step of an onboarding as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct StepView {
    /// The primary key.
    pub id: i64,
    /// The title.
    pub title: String,
    /// What it asks for.
    pub kind: String,
    /// Who does it.
    pub owner: String,
    /// The assignees' names, or *in person*.
    pub assignees: String,
    /// Required or optional.
    pub required: bool,
    /// The due date.
    pub due: String,
    /// Whether it is open and past its due date.
    pub overdue: bool,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Who did it, and the witness of an in-person record.
    pub done: String,
    /// The kind, the note (e.g. *No SOPs*, the evidence or the waive reason)
    /// and the failed items of the last checklist run.
    pub detail: String,
    /// The user may mark it done.
    pub can_mark_done: bool,
    /// The user may record it in person.
    pub can_in_person: bool,
    /// The user may waive it.
    pub can_waive: bool,
    /// The user may reopen it.
    pub can_reopen: bool,
    /// The user may start its checklist.
    pub can_checklist: bool,
    /// The step needs evidence.
    pub evidence_required: bool,
}

/// One onboarding as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct OnboardingView {
    /// The primary key.
    pub id: i64,
    /// When it started.
    pub started_on: String,
    /// `hire` or `manual`, in the page's language.
    pub trigger: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Required steps settled of required steps.
    pub progress: String,
    /// Why it was cancelled.
    pub cancel_reason: String,
    /// The user may cancel it (a manual one in progress).
    pub can_cancel: bool,
    /// The steps.
    pub steps: Vec<StepView>,
}

/// The employee page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/employee.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    employee: EmployeeRow,
    job_title: String,
    employment_type: String,
    scopes: String,
    onboardings: Vec<OnboardingView>,
    templates: Vec<(String, String)>,
    can_manage: bool,
    can_activate: bool,
    today: String,
}

/// `GET /hr/employees/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let employee = Employee::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let scopes = employee.scopes(&state.db).await?;
    let manages = steps::holds_manage(&state, user.id, &scopes).await?;
    let can_manage = manages;
    let day = today();
    let mut onboardings = Vec::new();
    for onboarding in Onboarding::of_employee(&state.db, id).await? {
        let in_progress = onboarding.status == OnboardingStatus::InProgress;
        let mut rows = Vec::new();
        let mut settled = 0;
        let mut required = 0;
        for step in onboarding.steps(&state.db).await? {
            if step.required {
                required += 1;
                if step.is_settled() {
                    settled += 1;
                }
            }
            let assignee_ids = step.assignees(&state.db).await?;
            let mut names = Vec::new();
            for assignee in &assignee_ids {
                names.push(user_name(&state.db, *assignee).await);
            }
            let is_assignee = assignee_ids.contains(&user.id);
            let open = in_progress && step.status == StepStatus::Open;
            let own_step = step.owner == OWNER_EMPLOYEE;
            let marked_by = match step.status {
                StepStatus::Done => step.done_by,
                StepStatus::Waived => step.waived_by,
                _ => None,
            };
            let mut done = String::new();
            if let Some(by) = step.done_by {
                done = user_name(&state.db, by).await;
            }
            if let Some(witness) = step.witness_id {
                done = format!(
                    "{done} ({}: {})",
                    text(&lang, "steps.witness"),
                    user_name(&state.db, witness).await
                );
            }
            rows.push(StepView {
                id: step.id,
                title: if lang.locale == "ar" && !step.title_ar.trim().is_empty() {
                    step.title_ar.clone()
                } else {
                    step.title_en.clone()
                },
                kind: kind_label(&lang, step.kind),
                owner: owner_label(&lang, &step.owner),
                assignees: if names.is_empty() {
                    text(&lang, "steps.nobody")
                } else {
                    names.join(", ")
                },
                required: step.required,
                due: step.due_date.to_string(),
                overdue: step.overdue_on(day),
                status: if step.overdue_on(day) {
                    text(&lang, "steps.overdue")
                } else {
                    text(&lang, &format!("steps.status.{}", step.status.as_str()))
                },
                status_kind: match step.status {
                    StepStatus::Open if step.overdue_on(day) => "error",
                    StepStatus::Done => "success",
                    _ => "info",
                },
                done,
                detail: [
                    Some(kind_label(&lang, step.kind)),
                    step.note
                        .clone()
                        .or_else(|| step.evidence_note.clone())
                        .or_else(|| step.waive_reason.clone()),
                    step.failed_items
                        .as_ref()
                        .map(|items| format!("{}: {items}", text(&lang, "steps.failed_items"))),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
                can_mark_done: open
                    && step.kind == StepKind::Task
                    && if own_step {
                        employee.user_id == Some(user.id)
                    } else {
                        is_assignee
                    },
                can_in_person: open
                    && manages
                    && ((own_step && step.kind == StepKind::Task)
                        || step.kind == StepKind::SopAcknowledgement),
                can_waive: open && manages && !is_assignee,
                can_reopen: in_progress
                    && manages
                    && marked_by != Some(user.id)
                    && match step.status {
                        StepStatus::Done => step.kind == StepKind::Task,
                        StepStatus::Waived => true,
                        _ => false,
                    },
                can_checklist: open
                    && step.kind == StepKind::Checklist
                    && (is_assignee || manages),
                evidence_required: step.evidence_required,
            });
        }
        let (status, kind) = match onboarding.status {
            OnboardingStatus::InProgress => ("in_progress", "info"),
            OnboardingStatus::Completed => ("completed", "success"),
            OnboardingStatus::Cancelled => ("cancelled", "info"),
        };
        onboardings.push(OnboardingView {
            id: onboarding.id,
            started_on: onboarding.started_on.to_string(),
            trigger: text(&lang, &format!("onboarding.trigger.{}", onboarding.start_trigger)),
            status: text(&lang, &format!("onboarding.status.{status}")),
            status_kind: kind,
            progress: format!("{settled}/{required}"),
            cancel_reason: onboarding.cancel_reason.clone().unwrap_or_default(),
            can_cancel: in_progress && can_manage && onboarding.start_trigger == "manual",
            steps: rows,
        });
    }
    let templates = OnboardingTemplate::query()
        .where_(OnboardingTemplate::ACTIVE.eq(true))
        .order_by(OnboardingTemplate::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|t| (t.id.to_string(), t.name_en))
        .collect();
    let row = EmployeeRow {
        id: employee.id,
        name: employee_name(&state, &lang, &employee).await,
        number: employee.employee_number.clone(),
        job_role: employee.job_role.clone(),
        hire_date: employee.hire_date.to_string(),
        status: text(&lang, &format!("employees.status.{}", employee.status.as_str())),
        status_kind: employee_kind(employee.status),
        progress: progress_of(&state.db, employee.id).await?,
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        employee: row,
        job_title: employee.job_title.clone(),
        employment_type: text(
            &lang,
            &format!("employees.type.{}", employee.employment_type.as_str()),
        ),
        scopes: scopes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        onboardings,
        templates,
        can_manage,
        can_activate: employee.status == EmployeeStatus::Onboarding
            && user.has_permission(permissions::EMPLOYEES_MANAGE),
        today: day.to_string(),
    }))
}

/// The form that starts an onboarding by hand.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct StartForm {
    /// The template id.
    #[validate(required, max = 20)]
    pub template: String,
    /// The start date, today when blank.
    #[validate(max = 20)]
    #[serde(default)]
    pub start_date: String,
}

/// `POST /hr/employees/{id}/onboardings`: starts the chosen template by hand.
async fn start_manual(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<StartForm>,
) -> Result<Response> {
    let employee = Employee::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let scopes = employee.scopes(&state.db).await?;
    if !steps::holds_manage(&state, user.id, &scopes).await? {
        return Err(Error::Forbidden);
    }
    let template: i64 = form
        .template
        .trim()
        .parse()
        .map_err(|_| Error::BadRequest("template".to_owned()))?;
    let start_on = match form.start_date.trim() {
        "" => None,
        text => Some(
            text.parse::<NaiveDate>()
                .map_err(|_| Error::BadRequest("start_date".to_owned()))?,
        ),
    };
    onboarding::start(
        &state,
        StartRequest {
            employee_id: id,
            trigger: StartTrigger::Manual,
            templates: vec![template],
            start_on,
        },
    )
    .await?
    .map_err(StartRefused::into_error)?;
    Ok((Toast::success(text(&lang, "onboarding.started")), renox::HxRefresh).into_response())
}

/// A reason.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReasonForm {
    /// Why.
    #[validate(max = 500)]
    #[serde(default)]
    pub reason: String,
}

/// `POST /hr/onboardings/{id}/cancel`: cancels a manual onboarding by hand.
async fn cancel_manual(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    let onboarding = Onboarding::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let employee = Employee::find_or_404(&state.db, onboarding.employee_id).await?;
    let scopes = employee.scopes(&state.db).await?;
    if !steps::holds_manage(&state, user.id, &scopes).await? {
        return Err(Error::Forbidden);
    }
    if onboarding.start_trigger != "manual" || onboarding.status != OnboardingStatus::InProgress {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            text(&lang, "onboarding.manual_only"),
        ));
    }
    if form.reason.trim().is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            text(&lang, "onboarding.problem.reason"),
        ));
    }
    onboarding::cancel(&state, onboarding, form.reason.trim()).await?;
    Ok((Toast::success(text(&lang, "onboarding.cancelled")), renox::HxRefresh).into_response())
}

/// The mark-done form.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct DoneForm {
    /// An evidence note.
    #[validate(max = 1000)]
    #[serde(default)]
    pub note: String,
    /// An evidence document number.
    #[validate(max = 20)]
    #[serde(default)]
    pub document: String,
    /// The employee's drawn signature, for an in-person record.
    #[validate(max = 100000)]
    #[serde(default)]
    pub signature: String,
}

/// The answer to a step action.
fn answered(
    lang: &Lang,
    done: Result<std::result::Result<OnboardingStep, steps::StepRefused>>,
    key: &str,
) -> Result<Response> {
    match done? {
        Ok(_) => Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response()),
        Err(refused) => Err(refused.into_error()),
    }
}

/// `POST /hr/steps/{id}/done`: marks a step done as the signed-in user.
async fn done(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<DoneForm>,
) -> Result<Response> {
    let input = DoneInput {
        note: Some(form.note.clone()),
        document_id: form.document.trim().parse().ok(),
        in_person: None,
    };
    answered(&lang, steps::mark_done(&state, id, user.id, input).await, "steps.done_ok")
}

/// `POST /hr/steps/{id}/in-person`: records a step in person for the
/// employee, with the drawn signature; the signed-in user is the witness.
async fn in_person(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<DoneForm>,
) -> Result<Response> {
    let input = DoneInput {
        note: Some(form.note.clone()),
        document_id: form.document.trim().parse().ok(),
        in_person: Some(InPerson {
            signature: form.signature.clone(),
        }),
    };
    answered(&lang, steps::mark_done(&state, id, user.id, input).await, "steps.done_ok")
}

/// `POST /hr/steps/{id}/waive`.
async fn waive(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    answered(
        &lang,
        steps::waive(&state, id, user.id, &form.reason).await,
        "steps.waived_ok",
    )
}

/// `POST /hr/steps/{id}/reopen`.
async fn reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReasonForm>,
) -> Result<Response> {
    answered(
        &lang,
        steps::reopen(&state, id, user.id, &form.reason).await,
        "steps.reopened_ok",
    )
}

/// `POST /hr/steps/{id}/checklist`: starts the checklist run of the step.
async fn checklist(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    answered(
        &lang,
        steps::start_checklist(&state, id, user.id).await,
        "steps.checklist_ok",
    )
}
