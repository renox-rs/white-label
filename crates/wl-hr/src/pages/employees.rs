//! The employee pages and the onboarding actions (wl-hr §3, §4, issues #216
//! and #217).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/employees` | `wl_hr.employees` | `employees.view` |
//! | `GET /hr/employees/{id}` | `wl_hr.employee` | `employees.view` (HR, PRO, MD), or a deployment of the worker in the user's client company |
//! | `POST /hr/employees/{id}/transitions/{name}` | `wl_hr.employee_transitions.*` | `employees.view` and the transition's permission |
//! | `POST /hr/employees/{id}/exits` | `wl_hr.exit_request` | `employees.manage` in the employee's scope |
//! | `POST /hr/employees/{id}/contact-attempts` | `wl_hr.contact_attempt` | `employees.manage` in the employee's scope |
//! | `POST /hr/employees/{id}/sick-leave` | `wl_hr.sick_leave` | `leave.approve` in the employee's scope, or their manager |
//! | `POST /hr/leave/{id}/sick-note` | `wl_hr.sick_note` | the employee, `leave.approve`, or their manager |
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

use crate::absence::{self, AbsenceReport, ContactAttempt, ContactOutcome};
use crate::employee::{Employee, EmployeeStatus};
use crate::exit;
use crate::exits::{
    self, ExitDocument, ExitInput, KIND_AGREEMENT, KIND_AUTHORITY_NOTICE, KIND_DEATH_CERTIFICATE,
};
use crate::leave::{LeaveRequest, LeaveStatus};
use crate::onboarding::{
    Onboarding, OnboardingStatus, OnboardingStep, StartRefused, StartRequest, StartTrigger,
    StepStatus,
};
use crate::pages::templates::{kind_label, owner_label};
use crate::steps::{self, DoneInput, InPerson};
use crate::template::{OWNER_EMPLOYEE, OnboardingTemplate, StepKind};
use crate::{leave_service, onboarding, permissions, sick_leave, texts};

/// The texts a template reads as `tx['employees.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/hr/employees", index)
                .name("wl_hr.employees")
                .group(
                    "/hr/employees",
                    "wl_hr.employee_transitions.",
                    wl_kernel::machines::routes::<Employee>(),
                )
                .require_permission(permissions::EMPLOYEES_VIEW),
        )
        // The page of one employee: the account manager, whose role is assigned
        // in the client company, opens the workers of those clients (BL-MPW-31);
        // the handler checks who may see which tab.
        .merge(wl_kernel::scopes::anywhere(
            Routes::new()
                .get("/hr/employees/{id}", show)
                .name("wl_hr.employee"),
            permissions::EMPLOYEES_VIEW,
        ))
        .merge(
            Routes::new()
                .post("/hr/employees/{id}/onboardings", start_manual)
                .name("wl_hr.onboarding_start")
                .post("/hr/employees/{id}/exits", request_exit)
                .name("wl_hr.exit_request")
                .post(
                    "/hr/employees/{id}/cancellation-reference",
                    cancellation_reference,
                )
                .name("wl_hr.cancellation_reference")
                .post("/hr/employees/{id}/contact-attempts", record_attempt)
                .name("wl_hr.contact_attempt")
                .post("/hr/employees/{id}/sick-leave", record_sick)
                .name("wl_hr.sick_leave")
                .post("/hr/leave/{id}/sick-note", attach_sick_note)
                .name("wl_hr.sick_note")
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
pub(crate) async fn employee_name(state: &AppState, lang: &Lang, employee: &Employee) -> String {
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
    for employee in Employee::query()
        .order_by_desc(Employee::ID)
        .get(&state.db)
        .await?
    {
        employees.push(EmployeeRow {
            id: employee.id,
            name: employee_name(&state, &lang, &employee).await,
            number: employee.employee_number.clone(),
            job_role: employee.job_role.clone(),
            hire_date: employee.hire_date.to_string(),
            status: text(
                &lang,
                &format!("employees.status.{}", employee.status.as_str()),
            ),
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

/// The non-standard exit request waiting for approval, as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct PendingExitView {
    /// The action asked for, in the page's language.
    pub action: String,
    /// Who asked.
    pub requested_by: String,
    /// Why.
    pub reason: String,
}

/// One attempt to reach the worker, as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct AttemptView {
    /// The day.
    pub date: String,
    /// How.
    pub method: String,
    /// How it ended, in the page's language.
    pub outcome: String,
    /// Who tried.
    pub by: String,
    /// A note.
    pub note: String,
}

/// One leave request on the employee page.
#[derive(Serialize)]
pub struct LeaveView {
    /// The request number.
    pub id: i64,
    /// The leave type.
    pub leave_type: String,
    /// The first and last day.
    pub period: String,
    /// Where it stands.
    pub status: String,
    /// Unpaid working days.
    pub unpaid: i64,
    /// The sick note: attached, or the day it is due by.
    pub note: String,
}

/// The employee page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/employee.html")]
pub(crate) struct ShowPage {
    attempts: Vec<AttemptView>,
    report: String,
    can_record_attempt: bool,
    outcomes: Vec<(String, String)>,
    outcome_selected: String,
    pending_exit: Option<PendingExitView>,
    exit_actions: Vec<(String, String)>,
    exit_selected: String,
    can_request_exit: bool,
    leaves: Vec<LeaveView>,
    can_record_sick: bool,
    note_requests: Vec<(String, String)>,
    note_selected: String,
    tx: Texts,
    employee: EmployeeRow,
    job_title: String,
    employment_type: String,
    scopes: String,
    onboardings: Vec<OnboardingView>,
    templates: Vec<(String, String)>,
    can_manage: bool,
    can_activate: bool,
    can_exit: bool,
    cancellation_reference: String,
    today: String,
    pay_accounts: Vec<crate::pages::pay_accounts::AccountView>,
    settlements: Vec<crate::pages::settlements::RowView>,
    can_request_pay_account: bool,
    pay_channels: Vec<(String, String)>,
    pay_channel_selected: String,
    tab_list: Vec<(String, String)>,
    selected_tab: String,
    staff: bool,
    show_documents: bool,
    documents: Vec<DocumentView>,
    documents_url: String,
    show_deployments: bool,
    deployments: Vec<DeploymentView>,
    show_timesheets: bool,
    timesheets: Vec<SheetView>,
    show_salary: bool,
    salary: Option<SalaryView>,
    show_pay_accounts: bool,
}

/// A document of the worker with its validity.
#[derive(Serialize, Debug, Clone)]
pub struct DocumentView {
    /// The file.
    pub url: String,
    /// The kind.
    pub kind: String,
    /// The number, or empty.
    pub number: String,
    /// The last valid day in words, or empty.
    pub valid_to: String,
    /// The validity in words.
    pub validity: String,
    /// The badge kind.
    pub validity_kind: &'static str,
    /// The role answerable, in words.
    pub role: String,
}

/// A deployment of the worker.
#[derive(Serialize, Debug, Clone)]
pub struct DeploymentView {
    /// Its page.
    pub url: String,
    /// The client.
    pub client: String,
    /// The site.
    pub site: String,
    /// The role.
    pub role: String,
    /// The dates in words.
    pub period: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// A timesheet of the worker.
#[derive(Serialize, Debug, Clone)]
pub struct SheetView {
    /// Its page.
    pub url: String,
    /// The month, `October 2026`, or the dates of a shorter period.
    pub month: String,
    /// The hours, `160 h`.
    pub hours: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Regular or adjustment, in words.
    pub kind: String,
}

/// The salary structure in force, as the *Salary* tab shows it.
#[derive(Serialize, Debug, Clone)]
pub struct SalaryView {
    /// Whether the amounts are shown (after *Reveal*).
    pub revealed: bool,
    /// Revealed, and no structure is recorded.
    pub none: bool,
    /// The day it applies from.
    pub since: String,
    /// The basic wage.
    pub basic: String,
    /// The housing allowance.
    pub housing: String,
    /// The transport allowance.
    pub transport: String,
    /// Other allowances.
    pub other: String,
    /// The fixed wage.
    pub fixed: String,
    /// The pay of an hour.
    pub hourly: String,
}

/// `minutes` as hours: `160 h`, `7 h 30 min`.
fn pages_hours(minutes: i64) -> String {
    crate::pages::payroll::hours_text(minutes)
}

/// The query of the employee page.
#[derive(Deserialize, Debug, Default)]
pub struct ShowQuery {
    /// `salary`: show the amounts of the salary tab (audited).
    #[serde(default)]
    pub reveal: Option<String>,
}

/// The audit action of a viewed salary (`HRM.ACL.03`).
pub const AUDIT_SALARY_VIEWED: &str = "hr.employee.salary_viewed";

fn validity_kind(validity: &str) -> &'static str {
    match validity {
        "valid" => "success",
        "expiring" | "pending" => "warning",
        "expired" | "rejected" => "error",
        _ => "info",
    }
}

fn role_label(role: &str) -> String {
    if role.len() <= 3 {
        return role.to_uppercase();
    }
    let spaced = role.replace('_', " ");
    let mut chars = spaced.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// `GET /hr/employees/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Query(query): Query<ShowQuery>,
) -> Result<View> {
    let employee = Employee::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    // Staff (HR, PRO, MD) see the record; a user whose role is assigned in a
    // client company sees the worker only through the deployments there.
    let staff = user.has_permission(permissions::EMPLOYEES_VIEW);
    let placements = wl_kernel::placements::placements(&state)
        .of_employee(&state, &user, wl_kernel::EmployeeId(id), &lang.locale)
        .await?;
    if !staff && !placements.as_ref().is_some_and(|rows| !rows.is_empty()) {
        return Err(Error::Forbidden);
    }
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
                can_checklist: open && step.kind == StepKind::Checklist && (is_assignee || manages),
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
            trigger: text(
                &lang,
                &format!("onboarding.trigger.{}", onboarding.start_trigger),
            ),
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
        status: text(
            &lang,
            &format!("employees.status.{}", employee.status.as_str()),
        ),
        status_kind: employee_kind(employee.status),
        progress: progress_of(&state.db, employee.id).await?,
    };
    let pending_exit = match exits::pending_for(&state.db, &employee).await? {
        Some(request) => Some(PendingExitView {
            action: text(&lang, &format!("exits.transition.{}", request.transition)),
            requested_by: user_name(&state.db, request.requester_id).await,
            reason: request.payload.0["reason"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        }),
        None => None,
    };
    let exit_actions: Vec<(String, String)> = exits::available_actions(employee.status)
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                text(&lang, &format!("exits.transition.{name}")),
            )
        })
        .collect();
    let can_request_exit = pending_exit.is_none()
        && !exit_actions.is_empty()
        && exits::holds_manage(&state, user.id, &employee).await?;
    let mut attempts = Vec::new();
    for attempt in ContactAttempt::of_employee(&state.db, id).await? {
        attempts.push(AttemptView {
            date: attempt.attempted_on.to_string(),
            method: attempt.method.clone(),
            outcome: text(
                &lang,
                &format!("absence.outcome.{}", attempt.outcome.as_str()),
            ),
            by: match attempt.by_user_id {
                Some(by) => user_name(&state.db, by).await,
                None => String::new(),
            },
            note: attempt.note.clone().unwrap_or_default(),
        });
    }
    let report = match AbsenceReport::valid_for(&state.db, id).await? {
        Some(report) => format!("{} · {}", report.report_number, report.report_date),
        None => String::new(),
    };
    let outcomes: Vec<(String, String)> = [
        ContactOutcome::NoAnswer,
        ContactOutcome::Unreachable,
        ContactOutcome::Reached,
    ]
    .into_iter()
    .map(|o| {
        (
            o.as_str().to_owned(),
            text(&lang, &format!("absence.outcome.{}", o.as_str())),
        )
    })
    .collect();
    let can_record_attempt = matches!(
        employee.status,
        EmployeeStatus::Active
            | EmployeeStatus::OnLeave
            | EmployeeStatus::Notice
            | EmployeeStatus::Absconded
    ) && exits::holds_manage(&state, user.id, &employee).await?;
    let acts_on_leave = leave_service::holds_approve(&state, user.id, &employee).await?
        || leave_service::is_manager_of(&state, user.id, &employee).await?;
    let mut leaves = Vec::new();
    let mut note_requests = Vec::new();
    if acts_on_leave {
        for request in LeaveRequest::query()
            .where_(LeaveRequest::EMPLOYEE_ID.eq(id))
            .order_by_desc(LeaveRequest::FROM_DATE)
            .get(&state.db)
            .await?
        {
            let note = match (request.sick_note_attached_on, request.sick_note_due_on) {
                (Some(_), _) => text(&lang, "sick.note_attached"),
                (None, Some(due)) => texts::t_with(&lang, "wl_hr.sick.note_due", &[("date", &due)]),
                (None, None) => String::new(),
            };
            if request.leave_type == sick_leave::SICK
                && request.sick_note_document_id.is_none()
                && !matches!(
                    request.status,
                    LeaveStatus::Rejected | LeaveStatus::Cancelled
                )
            {
                note_requests.push((
                    request.id.to_string(),
                    format!("{} - {}", request.from_date, request.to_date),
                ));
            }
            leaves.push(LeaveView {
                id: request.id,
                leave_type: request.leave_type.clone(),
                period: format!("{} - {}", request.from_date, request.to_date),
                status: request.status.as_str().to_owned(),
                unpaid: request.unpaid_days,
                note,
            });
        }
    }
    let can_record_sick = acts_on_leave && EmployeeStatus::WORKING.contains(&employee.status);
    let sensitive = staff && user.has_permission(permissions::EMPLOYEES_SENSITIVE_VIEW);
    let show_pay_accounts = staff
        && (can_manage || sensitive || user.has_permission(permissions::PAY_ACCOUNTS_APPROVE));
    let mut pay_accounts = Vec::new();
    if show_pay_accounts {
        for account in crate::pay_account::PayAccount::of_employee(&state.db, id).await? {
            pay_accounts.push(crate::pages::pay_accounts::view_of(&state, &lang, &account).await?);
        }
    }
    let mut settlements = Vec::new();
    if staff
        && [
            permissions::SETTLEMENTS_PREPARE,
            permissions::SETTLEMENTS_APPROVE,
            permissions::SETTLEMENTS_PAY,
        ]
        .iter()
        .any(|permission| user.has_permission(permission))
    {
        for settlement in crate::settlement::FinalSettlement::of_employee(&state.db, id).await? {
            settlements.push(crate::pages::settlements::row_of(&lang, &settlement));
        }
    }
    // The documents with their validity: HR, the PRO and the MD, not the
    // account manager (who has `documents.view` for the client's papers).
    let subject = wl_kernel::workforce::EmployeeId(id).subject();
    let show_documents = staff
        && user.has_permission_anywhere("documents.view")
        && (can_manage
            || sensitive
            || user.has_permission(permissions::VISAS_MANAGE)
            || user.has_permission(permissions::EMPLOYEES_EXIT_APPROVE));
    let mut documents = Vec::new();
    if show_documents {
        for line in wl_kernel::documents::guard(&state)
            .documents_of(&state.db, &subject, day, &lang.locale)
            .await?
        {
            documents.push(DocumentView {
                url: format!("/documents/{}/file", line.id),
                kind: line.kind,
                number: line.number.unwrap_or_default(),
                valid_to: line
                    .valid_to
                    .map(crate::pages::payroll::date_text)
                    .unwrap_or_default(),
                validity: text(&lang, &format!("documents.validity.{}", line.validity)),
                validity_kind: validity_kind(&line.validity),
                role: line
                    .responsible_role
                    .as_deref()
                    .map(role_label)
                    .unwrap_or_default(),
            });
        }
    }
    let show_deployments = placements.is_some();
    let deployments: Vec<DeploymentView> = placements
        .unwrap_or_default()
        .into_iter()
        .map(|p| DeploymentView {
            url: p.url,
            client: p.client,
            site: p.site,
            role: p.role,
            period: match p.to {
                Some(to) => format!(
                    "{} - {}",
                    crate::pages::payroll::date_text(p.from),
                    crate::pages::payroll::date_text(to)
                ),
                None => format!(
                    "{} - {}",
                    crate::pages::payroll::date_text(p.from),
                    text(&lang, "deployments.open_ended")
                ),
            },
            status_kind: if p.on_site { "success" } else { "info" },
            status: p.status,
        })
        .collect();
    let show_timesheets = user.has_permission_anywhere("attendance.view");
    let mut timesheets = Vec::new();
    if show_timesheets {
        for sheet in wl_kernel::timesheets::timesheets(&state)
            .of_employee(&state, wl_kernel::EmployeeId(id))
            .await?
        {
            timesheets.push(SheetView {
                url: format!("/timesheets/{}", sheet.subject.subject_id),
                month: crate::hours::month_text(sheet.from, sheet.to),
                hours: pages_hours(sheet.minutes),
                status_kind: match sheet.status.as_str() {
                    "approved" => "success",
                    "submitted" => "warning",
                    _ => "info",
                },
                status: text(&lang, &format!("sheets.status.{}", sheet.status)),
                kind: text(&lang, &format!("sheets.kind.{}", sheet.kind)),
            });
        }
    }
    // The salary: shown after *Reveal*, and each reveal is audited.
    let revealed = sensitive && query.reveal.as_deref() == Some("salary");
    let mut salary = None;
    if sensitive {
        let found = crate::salary::SalaryStructure::effective(&state.db, id, day).await?;
        if revealed {
            renox::audit::record(
                &state.db,
                renox::audit::Entry::new(AUDIT_SALARY_VIEWED)
                    .user(user.id)
                    .subject(Employee::TABLE, id),
            )
            .await?;
        }
        salary = Some(match found {
            Some(structure) if revealed => SalaryView {
                revealed: true,
                none: false,
                since: crate::pages::payroll::date_text(structure.effective_from),
                basic: crate::pages::payroll::aed_text(structure.basic),
                housing: crate::pages::payroll::aed_text(structure.housing),
                transport: crate::pages::payroll::aed_text(structure.transport),
                other: crate::pages::payroll::aed_text(structure.other_allowances),
                fixed: crate::pages::payroll::aed_text(structure.fixed_wage()),
                hourly: crate::pages::payroll::aed_text(structure.hourly()),
            },
            None if revealed => SalaryView {
                revealed: true,
                none: true,
                since: String::new(),
                basic: String::new(),
                housing: String::new(),
                transport: String::new(),
                other: String::new(),
                fixed: String::new(),
                hourly: String::new(),
            },
            _ => SalaryView {
                revealed: false,
                none: false,
                since: String::new(),
                basic: String::new(),
                housing: String::new(),
                transport: String::new(),
                other: String::new(),
                fixed: String::new(),
                hourly: String::new(),
            },
        });
    }
    let mut tabs = vec![("overview".to_owned(), text(&lang, "tabs.overview"))];
    for (key, shown) in [
        ("documents", show_documents),
        ("deployments", show_deployments),
        ("timesheets", show_timesheets),
        ("salary", sensitive),
        ("pay_accounts", show_pay_accounts),
    ] {
        if shown {
            tabs.push((key.to_owned(), text(&lang, &format!("tabs.{key}"))));
        }
    }
    let selected_tab = if revealed { "salary" } else { "overview" }.to_owned();
    Ok(View::typed(ShowPage {
        leaves,
        can_record_sick,
        note_selected: note_requests
            .first()
            .map(|(key, _)| key.clone())
            .unwrap_or_default(),
        note_requests,
        attempts: if staff { attempts } else { Vec::new() },
        report: if staff { report } else { String::new() },
        can_record_attempt: staff && can_record_attempt,
        outcome_selected: outcomes
            .first()
            .map(|(key, _)| key.clone())
            .unwrap_or_default(),
        outcomes,
        pending_exit: if staff { pending_exit } else { None },
        exit_selected: exit_actions
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_default(),
        exit_actions,
        can_request_exit: staff && can_request_exit,
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
        onboardings: if staff { onboardings } else { Vec::new() },
        templates,
        can_manage,
        can_activate: employee.status == EmployeeStatus::Onboarding
            && user.has_permission(permissions::EMPLOYEES_MANAGE),
        can_exit: exit::route_of(employee.status).is_some() && can_manage,
        cancellation_reference: employee.cancellation_reference.clone().unwrap_or_default(),
        today: day.to_string(),
        pay_accounts,
        settlements,
        can_request_pay_account: staff && can_manage,
        pay_channels: [
            crate::pay_account::BANK_ACCOUNT,
            crate::pay_account::SALARY_CARD,
        ]
        .iter()
        .map(|c| {
            (
                (*c).to_owned(),
                text(&lang, &format!("pay_account.channel.{c}")),
            )
        })
        .collect(),
        pay_channel_selected: crate::pay_account::BANK_ACCOUNT.to_owned(),
        tab_list: tabs,
        selected_tab,
        staff,
        show_documents,
        documents,
        documents_url: format!("/documents/of/{}/{}", subject.subject_type, id),
        show_deployments,
        deployments,
        show_timesheets,
        timesheets,
        show_salary: sensitive,
        salary,
        show_pay_accounts,
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
    let employee = Employee::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
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
    Ok((
        Toast::success(text(&lang, "onboarding.started")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that requests a non-standard exit (`HRM.ACL.07`).
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ExitForm {
    /// The action, one of the five.
    #[validate(max = 40)]
    #[serde(default)]
    pub transition: String,
    /// Why.
    #[validate(max = 1000)]
    #[serde(default)]
    pub reason: String,
    /// The MoHRE absence report number.
    #[validate(max = 100)]
    #[serde(default)]
    pub report_number: String,
    /// The MoHRE absence report date.
    #[validate(max = 20)]
    #[serde(default)]
    pub report_date: String,
    /// The report file, a document number.
    #[validate(max = 20)]
    #[serde(default)]
    pub report_file: String,
    /// The other party's written agreement: a document number or reference.
    #[validate(max = 100)]
    #[serde(default)]
    pub agreement: String,
    /// The death certificate: a document number or reference.
    #[validate(max = 100)]
    #[serde(default)]
    pub death_certificate: String,
    /// The authority's notice: a document number or reference.
    #[validate(max = 100)]
    #[serde(default)]
    pub authority_notice: String,
    /// The reference of the withdrawal or cancellation of the report.
    #[validate(max = 100)]
    #[serde(default)]
    pub reference: String,
    /// The date of death.
    #[validate(max = 20)]
    #[serde(default)]
    pub death_date: String,
    /// The date the worker returned.
    #[validate(max = 20)]
    #[serde(default)]
    pub return_date: String,
}

/// A date field: `None` when blank, a 400 when it is not a date.
fn date_field(name: &str, value: &str) -> Result<Option<NaiveDate>> {
    match value.trim() {
        "" => Ok(None),
        text => text
            .parse::<NaiveDate>()
            .map(Some)
            .map_err(|_| Error::BadRequest(name.to_owned())),
    }
}

/// A text that may be a stored document's number or just a reference.
fn document_field(kind: &str, value: &str) -> Option<ExitDocument> {
    let value = value.trim();
    (!value.is_empty()).then(|| ExitDocument {
        kind: kind.to_owned(),
        document: value.parse().ok(),
        reference: Some(value.to_owned()),
    })
}

/// `POST /hr/employees/{id}/exits`: HR requests a non-standard exit; the
/// employee keeps its state until another user approves it.
async fn request_exit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ExitForm>,
) -> Result<Response> {
    let input = ExitInput {
        reason: form.reason.clone(),
        documents: [
            document_field(KIND_AGREEMENT, &form.agreement),
            document_field(KIND_DEATH_CERTIFICATE, &form.death_certificate),
            document_field(KIND_AUTHORITY_NOTICE, &form.authority_notice),
        ]
        .into_iter()
        .flatten()
        .collect(),
        report_number: Some(form.report_number.trim().to_owned()).filter(|n| !n.is_empty()),
        report_date: date_field("report_date", &form.report_date)?,
        report_file: form.report_file.trim().parse().ok(),
        absence_days: None,
        has_attendance: None,
        death_date: date_field("death_date", &form.death_date)?,
        return_date: date_field("return_date", &form.return_date)?,
        withdrawal_reference: Some(form.reference.trim().to_owned()).filter(|r| !r.is_empty()),
    };
    exits::request(&state, &user, id, form.transition.trim(), input)
        .await?
        .map_err(exits::ExitRefused::into_error)?;
    Ok((Toast::success(text(&lang, "exits.sent")), renox::HxRefresh).into_response())
}

/// The form that records same-day sick leave.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct SickForm {
    /// The first day; today when blank. Only today is accepted.
    #[validate(max = 20)]
    #[serde(default)]
    pub from_date: String,
    /// The last day, the first day when blank.
    #[validate(max = 20)]
    #[serde(default)]
    pub to_date: String,
}

/// `POST /hr/employees/{id}/sick-leave`: records the employee's sick leave
/// from today as approved at once (`record_sick`).
async fn record_sick(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<SickForm>,
) -> Result<Response> {
    let from = date_field("from_date", &form.from_date)?.unwrap_or_else(today);
    let to = date_field("to_date", &form.to_date)?.unwrap_or(from);
    sick_leave::record(&state, user.id, id, from, to).await?;
    Ok((
        Toast::success(text(&lang, "sick.recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that attaches a sick note.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct SickNoteForm {
    /// The number of the stored sick note.
    #[validate(required, max = 20)]
    pub document: String,
}

/// `POST /hr/leave/{id}/sick-note`: attaches the sick note to a sick leave;
/// the HR task for the missing note closes.
async fn attach_sick_note(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<SickNoteForm>,
) -> Result<Response> {
    let document: i64 = form
        .document
        .trim()
        .parse()
        .map_err(|_| Error::BadRequest("document".to_owned()))?;
    sick_leave::attach_note(&state, user.id, id, document).await?;
    Ok((
        Toast::success(text(&lang, "sick.attached")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that records the authority's cancellation reference.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReferenceForm {
    /// The authority's reference.
    #[validate(max = 100)]
    #[serde(default)]
    pub reference: String,
}

/// `POST /hr/employees/{id}/cancellation-reference`: HR records the
/// authority's reference of the cancellation of the residence visa, which lets
/// an absconded or deceased employee exit without a completed case.
async fn cancellation_reference(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReferenceForm>,
) -> Result<Response> {
    exit::record_cancellation_reference(&state, &user, id, &form.reference).await?;
    Ok((
        Toast::success(text(&lang, "exit.reference_saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that records an attempt to reach an absent worker.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct AttemptForm {
    /// The day, today when blank.
    #[validate(max = 20)]
    #[serde(default)]
    pub attempted_on: String,
    /// How: phone, SMS, visit.
    #[validate(max = 100)]
    #[serde(default)]
    pub method: String,
    /// `no_answer`, `unreachable` or `reached`.
    #[validate(max = 20)]
    #[serde(default)]
    pub outcome: String,
    /// A note.
    #[validate(max = 1000)]
    #[serde(default)]
    pub note: String,
}

/// `POST /hr/employees/{id}/contact-attempts`: HR records an attempt to reach
/// the worker.
async fn record_attempt(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<AttemptForm>,
) -> Result<Response> {
    let employee = Employee::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !exits::holds_manage(&state, user.id, &employee).await? {
        return Err(Error::Forbidden);
    }
    let day = date_field("attempted_on", &form.attempted_on)?.unwrap_or_else(today);
    let outcome = match form.outcome.trim() {
        "" | "no_answer" => ContactOutcome::NoAnswer,
        "unreachable" => ContactOutcome::Unreachable,
        "reached" => ContactOutcome::Reached,
        _ => return Err(Error::BadRequest("outcome".to_owned())),
    };
    absence::record_contact_attempt(
        &state,
        user.id,
        id,
        day,
        &form.method,
        outcome,
        Some(&form.note),
    )
    .await?
    .map_err(absence::AttemptRefused::into_error)?;
    Ok((
        Toast::success(text(&lang, "absence.recorded")),
        renox::HxRefresh,
    )
        .into_response())
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
    let onboarding = Onboarding::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
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
    Ok((
        Toast::success(text(&lang, "onboarding.cancelled")),
        renox::HxRefresh,
    )
        .into_response())
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
    answered(
        &lang,
        steps::mark_done(&state, id, user.id, input).await,
        "steps.done_ok",
    )
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
    answered(
        &lang,
        steps::mark_done(&state, id, user.id, input).await,
        "steps.done_ok",
    )
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
