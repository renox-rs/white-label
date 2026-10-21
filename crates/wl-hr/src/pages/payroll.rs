//! The payroll run page and its actions (wl-hr §4 *Payroll run*, §5.3,
//! issues #201 to #205).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/payroll/{id}` | `wl_hr.payroll_run` | `payroll.view` |
//! | `POST /hr/payroll/{id}/transitions/{name}` | `wl_hr.payroll_transitions.*` | `payroll.view` and the transition's permission |
//! | `POST /hr/payroll/{id}/rejection` | `wl_hr.payroll_rejection` | `payroll.prepare` |
//! | `POST /hr/payroll/{id}/resubmit` | `wl_hr.payroll_resubmit` | `payroll.prepare` |
//! | `POST /hr/payroll/lines/{id}/comment` | `wl_hr.payroll_line_comment` | `payroll.approve`, not the preparer |
//!
//! The page lists the lines, the lines above the Art. 25 caps (the
//! exceptions of `HRM.ACL.06`) and the actions the signed-in user may run.
//! **Approve** is disabled for every user while a line is above the caps.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};

use crate::caps;
use crate::employee::Employee;
use crate::payroll::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
use crate::sif::SifFile;
use crate::wps_rejection::{REASON_FILE_FORMAT, REASONS, RejectionInput, ResubmitInput};
use crate::{permissions, texts};

/// The texts a template reads as `tx['payroll.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the payroll pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(
            Routes::new()
                .get("/hr/payroll/{id}", show)
                .name("wl_hr.payroll_run")
                .group(
                    "/hr/payroll",
                    "wl_hr.payroll_transitions.",
                    wl_kernel::machines::routes::<PayrollRun>(),
                )
                .require_permission(permissions::PAYROLL_VIEW),
        )
        .merge(
            Routes::new()
                .post("/hr/payroll/lines/{id}/comment", comment_line)
                .name("wl_hr.payroll_line_comment")
                .post("/hr/payroll/{id}/rejection", record_rejection)
                .name("wl_hr.payroll_rejection")
                .post("/hr/payroll/{id}/resubmit", resubmit)
                .name("wl_hr.payroll_resubmit")
                .require_auth(),
        )
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
        PayrollRunStatus::Rejected => "error",
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
    /// The approver's comment on the line.
    pub comment: String,
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
    can_comment: bool,
    sif_files: Vec<SifView>,
    can_record_rejection: bool,
    can_resubmit: bool,
    rejection_reasons: Vec<(String, String)>,
    rejection_selected: String,
}

/// One SIF version.
#[derive(Serialize, Debug, Clone)]
pub struct SifView {
    /// 1, 2, ...
    pub version: i64,
    /// The file's name.
    pub name: String,
    /// How many lines it carries.
    pub lines: i64,
    /// The sum of the nets, dirhams.
    pub total: String,
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
    let prepares = user.has_permission(permissions::PAYROLL_PREPARE);
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
        can_comment: can_comment(&user, &run),
        sif_files: SifFile::of_run(&state.db, id)
            .await?
            .into_iter()
            .map(|f| SifView {
                version: f.version,
                name: f.file_name,
                lines: f.line_count,
                total: aed(f.total),
            })
            .collect(),
        can_record_rejection: prepares && run.status == PayrollRunStatus::Submitted,
        can_resubmit: prepares && run.status == PayrollRunStatus::Rejected,
        rejection_reasons: REASONS
            .iter()
            .map(|r| {
                (
                    (*r).to_owned(),
                    text(&lang, &format!("payroll.rejection.{r}")),
                )
            })
            .collect(),
        rejection_selected: REASON_FILE_FORMAT.to_owned(),
    }))
}

/// The transitions the generic button row runs: they need no input beyond a
/// reason. The others have a form of their own.
const GENERIC: [&str; 3] = [
    crate::payroll::CALCULATE,
    crate::payroll::APPROVE,
    crate::payroll::RETURN,
];

/// Whether `user` may comment on the lines of `run`: an approver who is not
/// the preparer, while the run is `calculated` or `approved`.
fn can_comment(user: &AuthUser, run: &PayrollRun) -> bool {
    user.has_permission(permissions::PAYROLL_APPROVE)
        && matches!(
            run.status,
            PayrollRunStatus::Calculated | PayrollRunStatus::Approved
        )
        && run.prepared_by != Some(user.id)
}

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
        comment: line.approver_comment.clone().unwrap_or_default(),
    }
}

/// The form that comments on a line.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CommentForm {
    /// The comment; blank takes it away.
    #[validate(max = 1000)]
    #[serde(default)]
    pub comment: String,
}

/// `POST /hr/payroll/lines/{id}/comment`: the approver comments on a line to
/// fix before `return`.
async fn comment_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<CommentForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::PAYROLL_APPROVE) {
        return Err(Error::Forbidden);
    }
    let line = PayrollLine::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let run = PayrollRun::find_or_404(&state.db, line.run_id).await?;
    if !can_comment(&user, &run) {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            text(&lang, "payroll.comment_closed"),
        ));
    }
    crate::payroll::comment_line(&state.db, id, &form.comment).await?;
    Ok((Toast::success(text(&lang, "payroll.comment_saved")), back).into_response())
}

/// Runs the user transition `name` of run `id` as `user` with `input`.
async fn run_as<I: Serialize>(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    name: &str,
    input: &I,
) -> Result {
    let run = PayrollRun::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let _outcome = run
        .transition(name)
        .by(user)
        .input(input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// The form that records the bank's rejection of the file.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RejectionForm {
    /// `file_format`, `establishment_id`, `funding` or `other`.
    #[validate(max = 40)]
    #[serde(default)]
    pub reason: String,
    /// The bank's rejection notice.
    #[validate(max = 500)]
    #[serde(default)]
    pub notice: String,
}

/// `POST /hr/payroll/{id}/rejection`: the preparer records the rejection.
async fn record_rejection(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<RejectionForm>,
) -> Result<Response> {
    let input = RejectionInput {
        reason: form.reason,
        notice: form.notice,
    };
    run_as(&state, &user, id, crate::payroll::RECORD_REJECTION, &input).await?;
    Ok((Toast::success(text(&lang, "payroll.rejected")), back).into_response())
}

/// The form that resubmits a rejected file.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ResubmitForm {
    /// What was done to fix the cause.
    #[validate(max = 1000)]
    #[serde(default)]
    pub note: String,
}

/// `POST /hr/payroll/{id}/resubmit`: the preparer resubmits the rejected
/// file as a new version.
async fn resubmit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<ResubmitForm>,
) -> Result<Response> {
    let input = ResubmitInput { note: form.note };
    run_as(&state, &user, id, crate::payroll::RESUBMIT, &input).await?;
    Ok((Toast::success(text(&lang, "payroll.resubmitted")), back).into_response())
}
