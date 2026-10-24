//! The payroll run list and the start of a run (issue #510, `BL-MPW-30`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /hr/payroll` | `wl_hr.payroll_runs` | `payroll.view` |
//! | `POST /hr/payroll` | `wl_hr.payroll_create` | `payroll.prepare` |
//!
//! The list shows period, kind, status in words, lines and the total net in
//! AED. *New run* makes a `regular` run in `draft` for a month and calculates
//! it (the `calculate` start); a month that has a regular run is refused in
//! words. Decision: only `regular` runs are started by hand, off-cycle runs
//! come from held lines (`HRM.RUL.12`) and supplementary runs from a paid
//! run. The legal entity is the one of the first employee that has one.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::employee::Employee;
use crate::payroll::{PayrollRun, PayrollRunStatus};
use crate::{calculation, hours, permissions, texts};

type Texts = BTreeMap<&'static str, String>;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/hr/payroll", index)
        .name("wl_hr.payroll_runs")
        .require_permission(permissions::PAYROLL_VIEW)
        .merge(
            Routes::new()
                .post("/hr/payroll", create)
                .name("wl_hr.payroll_create")
                .require_auth(),
        )
}

fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_hr.{key}"))
}

/// The list page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/payroll_runs.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    runs: GridPage<PayrollRun>,
    can_create: bool,
    month: String,
}

/// The payroll runs grid: filters by kind and status, the number of lines
/// and the total net of each run (sums of its lines), and a sum of the totals.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let t = |key: &str| text(lang, key);
    let kinds: Vec<(String, String)> = ["regular", "off_cycle", "supplementary"]
        .iter()
        .map(|kind| ((*kind).to_owned(), t(&format!("payroll.kind.{kind}"))))
        .collect();
    let statuses: Vec<(String, String)> = [
        PayrollRunStatus::Draft,
        PayrollRunStatus::Calculated,
        PayrollRunStatus::Approved,
        PayrollRunStatus::Submitted,
        PayrollRunStatus::Rejected,
        PayrollRunStatus::PartiallyRejected,
        PayrollRunStatus::Paid,
    ]
    .iter()
    .map(|status| {
        (
            status.as_str().to_owned(),
            t(&format!("payroll.status.{}", status.as_str())),
        )
    })
    .collect();
    let grid = Grid::new("hr-payroll-runs")
        .title(&t("payroll.list.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("period", &t("payroll.list.period"))
                .frozen()
                .mobile()
                .searchable()
                .link("/hr/payroll/{id}"),
        )
        .column(Column::select("run_type", &t("payroll.list.kind"), kinds).mobile())
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("calculated", "info"),
                    ("approved", "info"),
                    ("submitted", "info"),
                    ("rejected", "danger"),
                    ("partially_rejected", "warning"),
                    ("paid", "success"),
                ])
                .mobile(),
        )
        .column(Column::count_of(
            "lines",
            &t("payroll.lines"),
            "wl_hr_payroll_lines",
            "run_id",
        ))
        .column(
            Column::money("total", &t("payroll.list.total"))
                .computed(
                    "(SELECT COALESCE(SUM(\"wl_hr_payroll_lines\".\"net_amount\"), 0) \
                     FROM \"wl_hr_payroll_lines\" WHERE \"wl_hr_payroll_lines\".\"run_id\" = {T}.\"id\")",
                )
                .mobile()
                .summary(Summary::Sum),
        )
        .column(Column::text("reason", &t("payroll.list.reason")).hidden().searchable())
        .sort_by("-period")
        .row_url("/hr/payroll/{id}")
        .cards_on_mobile()
        .empty_state(
            &t("payroll.list.empty"),
            Some(&t("payroll.list.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

/// `GET /hr/payroll`: the grid, or its export (`?export=`, with
/// `payroll.prepare` or `payroll.approve`).
async fn index(
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    let can_export = user.has_permission(permissions::PAYROLL_PREPARE)
        || user.has_permission(permissions::PAYROLL_APPROVE);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    if let Some(file) = grid.export(PayrollRun::query(), &request).await? {
        return Ok(file);
    }
    let runs = grid.page(PayrollRun::query(), &request).await?;
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        runs,
        can_create: user.has_permission(permissions::PAYROLL_PREPARE),
        month: wl_kernel::documents::today().format("%Y-%m").to_string(),
    })
    .into_response())
}

/// The form of *New run*.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct NewRunForm {
    /// The month, `2026-10`.
    #[validate(max = 7)]
    #[serde(default)]
    pub period: String,
}

fn refused(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// `POST /hr/payroll`: makes the regular run of a month and calculates it.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<NewRunForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::PAYROLL_PREPARE) {
        return Err(Error::Forbidden);
    }
    let period = form.period.trim().to_owned();
    if hours::period_bounds(&period).is_none() || period.len() != 7 {
        return Err(refused(text(&lang, "payroll.list.bad_period")));
    }
    let taken = PayrollRun::query()
        .where_(PayrollRun::PERIOD.eq(period.as_str()))
        .where_(PayrollRun::RUN_TYPE.eq(calculation::REGULAR))
        .exists(&state.db)
        .await?;
    if taken {
        return Err(refused(texts::t_with(
            &lang,
            "wl_hr.payroll.list.period_taken",
            &[("period", &period)],
        )));
    }
    let entity = Employee::query()
        .where_(Employee::LEGAL_ENTITY_ID.is_not_null())
        .order_by(Employee::ID)
        .first(&state.db)
        .await?
        .and_then(|e| e.legal_entity_id);
    let mut run = PayrollRun {
        legal_entity_id: entity,
        period,
        run_type: calculation::REGULAR.to_owned(),
        ..PayrollRun::default()
    };
    run.insert(&state.db).await?;
    calculation::calculate(&state, &user, run.id)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(Redirect::to(&format!("/hr/payroll/{}", run.id)).into_response())
}
