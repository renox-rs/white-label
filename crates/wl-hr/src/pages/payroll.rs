//! The payroll run page and its actions (wl-hr §4 *Payroll run*, §5.3,
//! issues #201 to #205).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/payroll/{id}` | `wl_hr.payroll_run` | `payroll.view` |
//! | `POST /hr/payroll/{id}/transitions/{name}` | `wl_hr.payroll_transitions.*` | `payroll.view` and the transition's permission |
//!
//! The page lists the lines, the lines above the Art. 25 caps (the
//! exceptions of `HRM.ACL.06`) and the actions the signed-in user may run.
//! **Approve** is disabled for every user while a line is above the caps.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::Serialize;

use crate::caps;
use crate::employee::Employee;
use crate::payroll::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
use crate::{permissions, texts};

/// The texts a template reads as `tx['payroll.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the payroll pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/hr/payroll/{id}", show)
        .name("wl_hr.payroll_run")
        .group(
            "/hr/payroll",
            "wl_hr.payroll_transitions.",
            wl_kernel::machines::routes::<PayrollRun>(),
        )
        .require_permission(permissions::PAYROLL_VIEW)
}

/// The package text `key` (without the `wl_hr.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_hr.{key}"))
}

/// `fils` as dirhams: `3000.00`.
pub(crate) fn aed(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let fils = fils.abs();
    format!("{sign}{}.{:02}", fils / 100, fils % 100)
}

/// The badge kind of a run status.
fn run_kind(status: PayrollRunStatus) -> &'static str {
    match status {
        PayrollRunStatus::Draft => "info",
        PayrollRunStatus::Calculated | PayrollRunStatus::Approved => "info",
        PayrollRunStatus::Submitted => "info",
        PayrollRunStatus::PartiallyRejected => "warning",
        PayrollRunStatus::Paid => "success",
    }
}

/// The badge kind of a line status.
fn line_kind(status: LineStatus) -> &'static str {
    match status {
        LineStatus::Included => "info",
        LineStatus::Held => "warning",
        LineStatus::Paid => "success",
        LineStatus::Failed => "error",
    }
}

/// One line of the run.
#[derive(Serialize, Debug, Clone)]
pub struct LineView {
    /// The primary key.
    pub id: i64,
    /// The employee's name.
    pub employee: String,
    /// The gross fixed wage, dirhams.
    pub wage: String,
    /// The net pay, dirhams.
    pub net: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Why it is held or failed.
    pub reason: String,
}

/// A line above a cap (`HRM.ACL.06`).
#[derive(Serialize, Debug, Clone)]
pub struct ExceptionView {
    /// The employee's name.
    pub employee: String,
    /// Which cap, in the page's language.
    pub cap: String,
    /// The deductions counted against it, dirhams.
    pub amount: String,
    /// The cap, dirhams.
    pub limit: String,
}

/// One action the user may run.
#[derive(Serialize, Debug, Clone)]
pub struct ActionView {
    /// The transition's name.
    pub name: String,
    /// Its label.
    pub label: String,
    /// Its tone.
    pub tone: String,
    /// Whether it asks for a reason.
    pub needs_reason: bool,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/payroll_run.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    run: RunView,
    lines: Vec<LineView>,
    exceptions: Vec<ExceptionView>,
    actions: Vec<ActionView>,
    approve_blocked: bool,
    can_approve: bool,
}

/// The run's header.
#[derive(Serialize, Debug, Clone)]
pub struct RunView {
    /// The primary key.
    pub id: i64,
    /// The period.
    pub period: String,
    /// `regular`, `off_cycle` or `supplementary`.
    pub run_type: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// `GET /hr/payroll/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let run = PayrollRun::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let mut names = BTreeMap::new();
    let mut lines = Vec::new();
    for line in run.lines(&state.db).await? {
        let name = match Employee::find(&state.db, line.employee_id).await? {
            Some(employee) => {
                crate::pages::employees::employee_name(&state, &lang, &employee).await
            }
            None => format!("#{}", line.employee_id),
        };
        names.insert(line.employee_id, name.clone());
        lines.push(line_view(&lang, &line, name));
    }
    let mut exceptions = Vec::new();
    for problem in caps::problems(&state.db, id).await? {
        exceptions.push(ExceptionView {
            employee: names.get(&problem.employee_id).cloned().unwrap_or_default(),
            cap: text(&lang, &format!("payroll.cap.{}", problem.scope.key())),
            amount: aed(problem.amount),
            limit: aed(problem.cap),
        });
    }
    let approve_blocked = !exceptions.is_empty();
    let mut actions = Vec::new();
    for action in run.available(&user, &state.db).await? {
        if !GENERIC.contains(&action.name.as_str()) {
            continue;
        }
        if approve_blocked && action.name == crate::payroll::APPROVE {
            continue;
        }
        actions.push(ActionView {
            name: action.name.clone(),
            label: action.label.clone(),
            tone: action.tone.clone().unwrap_or_default(),
            needs_reason: action.needs_reason,
        });
    }
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        run: RunView {
            id: run.id,
            period: run.period.clone(),
            run_type: run.run_type.clone(),
            status: text(&lang, &format!("payroll.status.{}", run.status.as_str())),
            status_kind: run_kind(run.status),
        },
        lines,
        exceptions,
        actions,
        approve_blocked: approve_blocked && run.status == PayrollRunStatus::Calculated,
        can_approve: user.has_permission(permissions::PAYROLL_APPROVE),
    }))
}

/// The transitions the generic button row runs: they need no input beyond a
/// reason. The others have a form of their own.
const GENERIC: [&str; 2] = [crate::payroll::CALCULATE, crate::payroll::APPROVE];

fn line_view(lang: &Lang, line: &PayrollLine, employee: String) -> LineView {
    LineView {
        id: line.id,
        employee,
        wage: aed(line.fixed_wage),
        net: aed(line.net_amount),
        status: text(lang, &format!("payroll.line.{}", line.status.as_str())),
        status_kind: line_kind(line.status),
        reason: line
            .held_reason
            .clone()
            .or_else(|| line.failure_reason.clone())
            .unwrap_or_default(),
    }
}
