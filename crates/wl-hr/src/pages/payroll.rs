//! The payroll run page and its actions (wl-hr §4 *Payroll run*, §5.3,
//! issues #201 to #205).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/payroll/{id}` | `wl_hr.payroll_run` | `payroll.view` |
//! | `POST /hr/payroll/{id}/transitions/calculate` | `wl_hr.payroll_calculate` | `payroll.view` and `payroll.prepare`; asks the timesheets for the hours first |
//! | `POST /hr/payroll/{id}/transitions/{name}` | `wl_hr.payroll_transitions.*` | `payroll.view` and the transition's permission |
//! | `POST /hr/payroll/{id}/supplementary` | `wl_hr.payroll_supplementary` | `payroll.prepare` |
//! | `POST /hr/payroll/{id}/carry/{line}` | `wl_hr.payroll_carry` | `payroll.prepare` |
//! | `POST /hr/payroll/differences/{id}/convert` | `wl_hr.payroll_difference_convert` | `payroll.prepare` |
//! | `POST /hr/payroll/{id}/rejection` | `wl_hr.payroll_rejection` | `payroll.prepare` |
//! | `POST /hr/payroll/{id}/resubmit` | `wl_hr.payroll_resubmit` | `payroll.prepare` |
//! | `POST /hr/payroll/{id}/partial` | `wl_hr.payroll_partial` | `payroll.prepare` |
//! | `POST /hr/payroll/{id}/confirm` | `wl_hr.payroll_confirm` | `payroll.approve` |
//! | `POST /hr/payroll/lines/{id}/hold` | `wl_hr.payroll_line_hold` | `payroll.prepare` |
//! | `POST /hr/payroll/lines/{id}/pay-other` | `wl_hr.payroll_line_pay_other` | `payroll.prepare` |
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
use crate::differences::{self, HourDifference};
use crate::employee::Employee;
use crate::payroll::{
    ConfirmInput, FailedLine, LineStatus, PartialInput, PayrollLine, PayrollRun, PayrollRunStatus,
};
use crate::release;
use crate::sif::SifFile;
use crate::supplementary::{self, Correction};
use crate::wps_partial::{self, OTHER_METHODS};
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
                .post("/hr/payroll/{id}/transitions/calculate", calculate)
                .name("wl_hr.payroll_calculate")
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
                .post("/hr/payroll/{id}/partial", record_partial)
                .name("wl_hr.payroll_partial")
                .post("/hr/payroll/{id}/confirm", confirm_paid)
                .name("wl_hr.payroll_confirm")
                .post("/hr/payroll/{id}/supplementary", create_supplementary)
                .name("wl_hr.payroll_supplementary")
                .post("/hr/payroll/{id}/carry/{line}", carry_line)
                .name("wl_hr.payroll_carry")
                .post("/hr/payroll/differences/{id}/convert", convert_difference)
                .name("wl_hr.payroll_difference_convert")
                .post("/hr/payroll/lines/{id}/hold", hold_line)
                .name("wl_hr.payroll_line_hold")
                .post("/hr/payroll/lines/{id}/pay-other", pay_other)
                .name("wl_hr.payroll_line_pay_other")
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
    /// Whether the line failed (it can be held or paid another way).
    pub failed: bool,
    /// Whether its payslip is published.
    pub payslip: bool,
    /// The timesheet hours the line was calculated from, e.g. `160 h`.
    pub hours: String,
    /// Whether the hours are provisional (the timesheet is not approved yet).
    pub provisional: bool,
    /// The exception of the line in the page's language, e.g. *missing
    /// timesheet*.
    pub exception: String,
    /// The pay of overtime and the like, dirhams.
    pub variable: String,
    /// The hours added or taken back by adjustments and reconciliations,
    /// dirhams, signed.
    pub difference: String,
    /// The deductions, dirhams.
    pub deductions: String,
    /// On a carried line: the warning that it pays after the WPS due date.
    pub warning: String,
    /// On a carried line: why it was carried.
    pub carried: String,
    /// On a line of a supplementary run: the component of the difference.
    pub component: String,
}

/// A held line of a paid run that the regular run can carry.
#[derive(Serialize, Debug, Clone)]
pub struct CarryView {
    /// The held line.
    pub line_id: i64,
    /// The employee's name.
    pub employee: String,
    /// The amount it pays, dirhams.
    pub amount: String,
    /// The period of the run it was held in.
    pub held_in: String,
    /// Why it was held.
    pub reason: String,
    /// The warning that carrying it pays after the WPS due date, if it does.
    pub warning: String,
}

/// One difference line of the run: hours an adjustment timesheet or a
/// reconciliation adds or takes back.
#[derive(Serialize, Debug, Clone)]
pub struct DifferenceView {
    /// The primary key.
    pub id: i64,
    /// The employee's name.
    pub employee: String,
    /// Where it comes from, in the page's language.
    pub source: String,
    /// The category of the hours.
    pub category: String,
    /// The hours, signed.
    pub hours: String,
    /// The amount, dirhams, signed.
    pub amount: String,
    /// Whether the preparer can turn it into a recovery plan: it is negative
    /// and the run is still being prepared.
    pub can_convert: bool,
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
    differences: Vec<DifferenceView>,
    carryable: Vec<CarryView>,
    corrections: Vec<CorrectionView>,
    can_correct: bool,
    actions: Vec<ActionView>,
    approve_blocked: bool,
    submit_blocked: String,

    can_approve: bool,
    can_comment: bool,
    sif_files: Vec<SifView>,
    can_record_rejection: bool,
    can_resubmit: bool,
    can_record_partial: bool,
    can_confirm: bool,
    can_move_out: bool,
    other_methods: Vec<(String, String)>,
    other_selected: String,
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
    /// On a supplementary run: the paid run it corrects.
    pub corrects: Option<i64>,
    /// On a supplementary run: why it was made.
    pub reason: String,
}

/// A supplementary run that corrects the run shown.
#[derive(Serialize, Debug, Clone)]
pub struct CorrectionView {
    /// The primary key.
    pub id: i64,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Why it was made.
    pub reason: String,
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
    let mut difference_lines = Vec::new();
    for difference in HourDifference::of_run(&state.db, id).await? {
        difference_lines.push(DifferenceView {
            id: difference.id,
            employee: names
                .get(&difference.employee_id)
                .cloned()
                .unwrap_or_default(),
            source: text(&lang, &format!("payroll.difference.{}", difference.source)),
            category: difference.category.clone(),
            hours: hours_text(difference.minutes),
            amount: aed(difference.amount),
            can_convert: difference.status == differences::OPEN
                && difference.amount < 0
                && matches!(
                    run.status,
                    PayrollRunStatus::Draft | PayrollRunStatus::Calculated
                ),
        });
    }
    let mut carry_views = Vec::new();
    if user.has_permission(permissions::PAYROLL_PREPARE) {
        for held in release::carryable(&state, &run).await? {
            let employee = match Employee::find(&state.db, held.employee_id).await? {
                Some(employee) => {
                    crate::pages::employees::employee_name(&state, &lang, &employee).await
                }
                None => format!("#{}", held.employee_id),
            };
            let held_in = PayrollRun::find(&state.db, held.run_id)
                .await?
                .map(|r| r.period)
                .unwrap_or_default();
            carry_views.push(CarryView {
                line_id: held.id,
                employee,
                amount: aed(held.net_amount),
                held_in,
                reason: held.held_reason.clone().unwrap_or_default(),
                warning: release::warning_for(&state, &held, &run)
                    .await?
                    .unwrap_or_default(),
            });
        }
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
    // `submit` is refused while a line to be paid has no WPS account: the
    // button is shown disabled with the employees, as `approve` is.
    let mut submit_blocked = String::new();
    if run.status == PayrollRunStatus::Approved && prepares {
        let mut tx = state.db.begin().await?;
        let missing = crate::pay_account_use::without_wps_account_in(&mut tx, id).await?;
        if !missing.is_empty() {
            submit_blocked = texts::t_with(
                &lang,
                "wl_hr.payroll.submit_blocked",
                &[("employees", &missing.join(", "))],
            );
        }
    }
    let mut actions = Vec::new();
    for action in run.available(&user, &state.db).await? {
        if !GENERIC.contains(&action.name.as_str()) {
            continue;
        }
        if approve_blocked && action.name == crate::payroll::APPROVE {
            continue;
        }
        if !submit_blocked.is_empty() && action.name == crate::payroll::SUBMIT {
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
            corrects: run.corrects,
            reason: run.reason.clone().unwrap_or_default(),
        },
        lines,
        exceptions,
        differences: difference_lines,
        carryable: carry_views,
        corrections: supplementary::of_original(&state.db, id)
            .await?
            .into_iter()
            .map(|r| CorrectionView {
                id: r.id,
                status: text(&lang, &format!("payroll.status.{}", r.status.as_str())),
                status_kind: run_kind(r.status),
                reason: r.reason.unwrap_or_default(),
            })
            .collect(),
        can_correct: prepares && run.status == PayrollRunStatus::Paid,
        actions,
        approve_blocked: approve_blocked && run.status == PayrollRunStatus::Calculated,
        submit_blocked,
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
        can_record_partial: prepares && run.status == PayrollRunStatus::Submitted,
        can_confirm: user.has_permission(permissions::PAYROLL_APPROVE)
            && matches!(
                run.status,
                PayrollRunStatus::Submitted | PayrollRunStatus::PartiallyRejected
            ),
        can_move_out: prepares && run.status == PayrollRunStatus::PartiallyRejected,
        other_methods: OTHER_METHODS
            .iter()
            .map(|m| ((*m).to_owned(), text(&lang, &format!("payroll.method.{m}"))))
            .collect(),
        other_selected: OTHER_METHODS[0].to_owned(),
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
const GENERIC: [&str; 5] = [
    crate::payroll::CALCULATE,
    crate::payroll::APPROVE,
    crate::payroll::SUBMIT,
    crate::payroll::RETURN,
    crate::payroll::REGENERATE,
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
        failed: line.status == LineStatus::Failed,
        payslip: line.payslip_published_at.is_some(),
        hours: if line.hours_minutes == 0 {
            String::new()
        } else {
            hours_text(line.hours_minutes)
        },
        provisional: line.provisional_minutes > 0,
        exception: line
            .exception
            .as_deref()
            .map(|e| text(lang, &format!("payroll.exception.{e}")))
            .unwrap_or_default(),
        variable: aed(line.variable_amount),
        difference: aed(line.difference_amount),
        deductions: aed(line.deductions_amount),
        warning: line.late_warning.clone().unwrap_or_default(),
        carried: line.carry_reason.clone().unwrap_or_default(),
        component: line.component.clone().unwrap_or_default(),
    }
}

/// `minutes` as hours: `160 h`, `7 h 30 min`.
pub(crate) fn hours_text(minutes: i64) -> String {
    let sign = if minutes < 0 { "-" } else { "" };
    let minutes = minutes.abs();
    if minutes % 60 == 0 {
        format!("{sign}{} h", minutes / 60)
    } else {
        format!("{sign}{} h {} min", minutes / 60, minutes % 60)
    }
}

/// `POST /hr/payroll/{id}/transitions/calculate`: the timesheets are asked for
/// the hours of the period, then the run is calculated.
async fn calculate(
    State(state): State<AppState>,
    user: AuthUser,
    back: renox::Back,
    Path(id): Path<i64>,
) -> Result<Response> {
    let _run = crate::calculation::calculate(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(back.into_response())
}

/// The form that makes a supplementary run.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct SupplementaryForm {
    /// Why the paid run is corrected.
    #[validate(max = 500)]
    #[serde(default)]
    pub reason: String,
    /// The affected employees, one per row:
    /// `<employee id>=<corrected net in dirhams>[=<component>]`.
    #[validate(max = 5000)]
    #[serde(default)]
    pub corrections: String,
}

/// `3000`, `3000.5` or `3000.50` dirhams as fils.
pub(crate) fn parse_aed(text: &str) -> Option<i64> {
    let (whole, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    if fraction.len() > 2 || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    whole.checked_mul(100)?.checked_add(fraction)
}

/// The corrections of a form: `12=3000.00=overtime`, one per row.
fn parse_corrections(text: &str) -> Result<Vec<Correction>> {
    let bad = || Error::BadRequest("corrections".to_owned());
    let mut corrections = Vec::new();
    for row in text.lines().map(str::trim).filter(|r| !r.is_empty()) {
        let mut parts = row.splitn(3, '=');
        let employee_id = parts.next().and_then(|p| p.trim().parse().ok());
        let correct_net = parts.next().and_then(parse_aed);
        let (Some(employee_id), Some(correct_net)) = (employee_id, correct_net) else {
            return Err(bad());
        };
        corrections.push(Correction {
            employee_id,
            correct_net,
            component: parts.next().unwrap_or_default().trim().to_owned(),
        });
    }
    Ok(corrections)
}

/// `POST /hr/payroll/{id}/supplementary`: the preparer makes a supplementary
/// run that corrects the paid run.
async fn create_supplementary(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<SupplementaryForm>,
) -> Result<Response> {
    let corrections = parse_corrections(&form.corrections)?;
    supplementary::create(&state, &user, id, &form.reason, &corrections).await?;
    Ok((
        Toast::success(text(&lang, "payroll.supplementary_made")),
        back,
    )
        .into_response())
}

/// The form that carries a released held line into a regular run.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CarryForm {
    /// Why the line is carried into the regular run instead of an off-cycle
    /// run.
    #[validate(max = 500)]
    #[serde(default)]
    pub reason: String,
}

/// `POST /hr/payroll/{id}/carry/{line}`: the preparer carries the released
/// held line `line` into the regular run `id`, with a reason. The warning
/// that it pays after the WPS due date comes back with the toast.
async fn carry_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path((id, line)): Path<(i64, i64)>,
    Valid(form): Valid<CarryForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::PAYROLL_PREPARE) {
        return Err(Error::Forbidden);
    }
    let carried = release::carry(&state, line, id, &form.reason).await?;
    let toast = match carried.warning {
        Some(warning) => Toast::warning(format!("{} {warning}", text(&lang, "payroll.carried"))),
        None => Toast::success(text(&lang, "payroll.carried")),
    };
    Ok((toast, back).into_response())
}

/// The form that turns a difference into a recovery plan.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ConvertForm {
    /// A note for the adjustments.
    #[validate(max = 500)]
    #[serde(default)]
    pub note: String,
}

/// `POST /hr/payroll/differences/{id}/convert`: the preparer turns a negative
/// difference into an overpayment recovery with an instalment plan.
async fn convert_difference(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<ConvertForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::PAYROLL_PREPARE) {
        return Err(Error::Forbidden);
    }
    differences::convert_to_recovery(&state, id, &form.note).await?;
    Ok((
        Toast::success(text(&lang, "payroll.difference_converted")),
        back,
    )
        .into_response())
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

/// The form that records the bank's result file.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct PartialForm {
    /// The bank's result file.
    #[validate(max = 500)]
    #[serde(default)]
    pub result_file: String,
    /// The failed lines, one per row: `<line id>=<reason from the file>`.
    #[validate(max = 5000)]
    #[serde(default)]
    pub failed: String,
}

/// The failed lines of a form: `12=invalid IBAN`, one per row.
fn parse_failed(text: &str) -> Result<Vec<FailedLine>> {
    let mut failed = Vec::new();
    for row in text.lines().map(str::trim).filter(|r| !r.is_empty()) {
        let (line, reason) = row
            .split_once('=')
            .ok_or_else(|| Error::BadRequest("failed".to_owned()))?;
        failed.push(FailedLine {
            line_id: line
                .trim()
                .parse()
                .map_err(|_| Error::BadRequest("failed".to_owned()))?,
            reason: reason.trim().to_owned(),
        });
    }
    Ok(failed)
}

/// `POST /hr/payroll/{id}/partial`: the preparer records the bank's result.
async fn record_partial(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<PartialForm>,
) -> Result<Response> {
    let input = PartialInput {
        failed: parse_failed(&form.failed)?,
        result_file: form.result_file,
    };
    run_as(&state, &user, id, crate::payroll::RECORD_PARTIAL, &input).await?;
    Ok((
        Toast::success(text(&lang, "payroll.partial_recorded")),
        back,
    )
        .into_response())
}

/// The form that confirms payment.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ConfirmForm {
    /// The bank's confirmation of the latest SIF version.
    #[validate(max = 500)]
    #[serde(default)]
    pub confirmation: String,
}

/// `POST /hr/payroll/{id}/confirm`: the approver confirms payment.
async fn confirm_paid(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<ConfirmForm>,
) -> Result<Response> {
    let input = ConfirmInput {
        confirmation: form.confirmation,
    };
    run_as(&state, &user, id, crate::payroll::CONFIRM_PAID, &input).await?;
    Ok((Toast::success(text(&lang, "payroll.paid_confirmed")), back).into_response())
}

/// The form that holds a failed line.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct HoldForm {
    /// Why the line is held.
    #[validate(max = 500)]
    #[serde(default)]
    pub reason: String,
}

/// Loads line `id` and its run, and checks that `user` may prepare it.
async fn failed_line_of(state: &AppState, user: &AuthUser, id: i64) -> Result<PayrollRun> {
    if !user.has_permission(permissions::PAYROLL_PREPARE) {
        return Err(Error::Forbidden);
    }
    let line = PayrollLine::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    PayrollRun::find_or_404(&state.db, line.run_id).await
}

/// `POST /hr/payroll/lines/{id}/hold`: a failed line is held, to be paid later
/// off-cycle.
async fn hold_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<HoldForm>,
) -> Result<Response> {
    failed_line_of(&state, &user, id).await?;
    wps_partial::hold_line(&state.db, id, &form.reason).await?;
    Ok((Toast::success(text(&lang, "payroll.held")), back).into_response())
}

/// The form that records a failed line paid another approved way.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct PayOtherForm {
    /// `transfer`, `cheque` or `cash`.
    #[validate(max = 20)]
    #[serde(default)]
    pub method: String,
    /// The payment's reference.
    #[validate(max = 200)]
    #[serde(default)]
    pub reference: String,
}

/// `POST /hr/payroll/lines/{id}/pay-other`: a failed line is paid another
/// approved way and needs no bank confirmation.
async fn pay_other(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: renox::Back,
    Path(id): Path<i64>,
    Valid(form): Valid<PayOtherForm>,
) -> Result<Response> {
    failed_line_of(&state, &user, id).await?;
    wps_partial::pay_line_other_way(&state, id, &form.method, &form.reference, Some(user.id))
        .await?;
    Ok((Toast::success(text(&lang, "payroll.other_paid")), back).into_response())
}
