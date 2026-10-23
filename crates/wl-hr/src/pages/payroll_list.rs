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

use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::employee::Employee;
use crate::pages::payroll::{aed_text, run_kind};
use crate::payroll::{PayrollLine, PayrollRun};
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

/// One run of the list.
#[derive(Serialize, Debug, Clone)]
pub struct RunRow {
    /// The primary key.
    pub id: i64,
    /// The page of the run.
    pub url: String,
    /// The month, e.g. `2026-10`.
    pub period: String,
    /// The kind in words.
    pub kind: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// How many lines.
    pub lines: usize,
    /// The total net, `AED 12,500.00`.
    pub total: String,
}

/// The list page.
#[derive(Serialize, ViewData)]
#[view(template = "wl-hr/payroll_runs.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    runs: Vec<RunRow>,
    can_create: bool,
    month: String,
}

/// `GET /hr/payroll`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let runs = PayrollRun::query()
        .order_by_desc(PayrollRun::PERIOD)
        .order_by_desc(PayrollRun::ID)
        .get(&state.db)
        .await?;
    let mut rows = Vec::new();
    for run in runs {
        let lines = run.lines(&state.db).await?;
        rows.push(row_of(&lang, &run, &lines));
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        runs: rows,
        can_create: user.has_permission(permissions::PAYROLL_PREPARE),
        month: wl_kernel::documents::today().format("%Y-%m").to_string(),
    }))
}

fn row_of(lang: &Lang, run: &PayrollRun, lines: &[PayrollLine]) -> RunRow {
    RunRow {
        id: run.id,
        url: format!("/hr/payroll/{}", run.id),
        period: run.period.clone(),
        kind: text(lang, &format!("payroll.kind.{}", run.run_type)),
        status: text(lang, &format!("payroll.status.{}", run.status.as_str())),
        status_kind: run_kind(run.status),
        lines: lines.len(),
        total: aed_text(lines.iter().map(|l| l.net_amount).sum()),
    }
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
