//! The page of one rule run (wl-kernel §4.2), which the daily digest of
//! failed runs (`PLT.RUL.03`) links to.
//!
//! - `GET /rule-runs/{id}`: the rule, the record, the occurrence, the state,
//!   the attempts, the last error and each time the run failed or stopped.
//!   Company admins and holders of `automations.retry` see it (403
//!   otherwise).
//! - `POST /rule-runs/{id}/rerun`: the `rerun` transition of a `failed` run;
//!   the machine checks `automations.retry`.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::Serialize;

use super::failures::{FailureKind, RuleRunFailure};
use super::run::{RuleRun, RuleRunStatus, transitions};
use crate::digest::{info_of, record_name, rule_title};
use crate::documents::dubai_text;
use crate::permissions::AUTOMATIONS_RETRY;
use crate::scopes::COMPANY_ADMIN_ROLE;
use crate::texts;

/// The name of the page's route.
pub const SHOW_ROUTE: &str = "wl_kernel.rule_run";

/// The name of the rerun route.
pub const RERUN_ROUTE: &str = "wl_kernel.rule_run.rerun";

/// The page's template.
pub const TEMPLATE: &str = "wl-kernel/rule_run.html";

/// The texts a template reads as `tx['key']`.
type Texts = BTreeMap<&'static str, String>;

/// One failure of the run, as the page lists it.
#[derive(Debug, Serialize)]
pub struct FailureRow {
    id: i64,
    at: String,
    kind: String,
    error: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/rule_run.html")]
pub struct RuleRunPage {
    tx: Texts,
    heading: String,
    run_id: i64,
    rule: String,
    record: String,
    occurrence: String,
    status_label: String,
    status_kind: String,
    attempts: i64,
    last_error: String,
    started_at: String,
    finished_at: String,
    failures: Vec<FailureRow>,
    can_rerun: bool,
}

/// The routes, for logged-in users; who may see and rerun is checked per
/// request.
pub fn routes() -> Routes {
    Routes::new()
        .get("/rule-runs/{id}", show)
        .name(SHOW_ROUTE)
        .post("/rule-runs/{id}/rerun", rerun)
        .name(RERUN_ROUTE)
        .require_auth()
}

fn page_texts(lang: &Lang) -> Texts {
    [
        "rule",
        "subject",
        "occurrence",
        "status",
        "attempts",
        "error",
        "started_at",
        "finished_at",
        "failures",
        "failure_at",
        "failure_kind",
        "failures_empty",
        "rerun",
        "rerun_title",
        "rerun_message",
        "cancel",
    ]
    .into_iter()
    .map(|key| (key, texts::t(lang, &format!("wl_kernel.rule_run.{key}"))))
    .collect()
}

/// The state's label and badge kind.
fn status_of(lang: &Lang, status: RuleRunStatus) -> (String, String) {
    let kind = match status {
        RuleRunStatus::Succeeded => "success",
        RuleRunStatus::Retrying => "warning",
        RuleRunStatus::Failed | RuleRunStatus::Stopped => "error",
        RuleRunStatus::Queued | RuleRunStatus::Skipped => "info",
    };
    (
        texts::t(lang, &format!("wl_kernel.rule_run.status.{}", status.as_str())),
        kind.to_owned(),
    )
}

fn when(at: Option<renox::db::DateTime>) -> String {
    at.map_or_else(|| "—".to_owned(), dubai_text)
}

async fn show(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    if !(user.has_role(COMPANY_ADMIN_ROLE) || user.has_permission(AUTOMATIONS_RETRY)) {
        return Err(Error::Forbidden);
    }
    let run = RuleRun::find_or_404(&db, id).await?;
    let subject = run.subject();
    let mut tx = db.begin().await?;
    let info = info_of(&mut tx, &subject).await?;
    drop(tx);
    let failures = RuleRunFailure::query()
        .where_(RuleRunFailure::RUN_ID.eq(run.id))
        .order_by(RuleRunFailure::AT)
        .order_by(RuleRunFailure::ID)
        .get(&db)
        .await?
        .into_iter()
        .map(|failure| FailureRow {
            id: failure.id,
            at: dubai_text(failure.at),
            kind: texts::t(
                &lang,
                match failure.kind {
                    FailureKind::Failed => "wl_kernel.rule_run.status.failed",
                    FailureKind::Stopped => "wl_kernel.rule_run.status.stopped",
                },
            ),
            error: failure.error.unwrap_or_else(|| "—".to_owned()),
        })
        .collect();
    let (status_label, status_kind) = status_of(&lang, run.status);
    Ok(View::typed(RuleRunPage {
        tx: page_texts(&lang),
        heading: texts::t_with(&lang, "wl_kernel.rule_run.title", &[("id", &run.id)]),
        run_id: run.id,
        rule: format!("{} ({})", rule_title(&run.rule), run.rule),
        record: record_name(&subject, info.number.as_deref()),
        occurrence: run.occurrence_key.clone(),
        status_label,
        status_kind,
        attempts: run.attempts,
        last_error: run.error.clone().unwrap_or_else(|| "—".to_owned()),
        started_at: when(run.started_at),
        finished_at: when(run.finished_at),
        failures,
        can_rerun: run.status == RuleRunStatus::Failed && user.has_permission(AUTOMATIONS_RETRY),
    }))
}

async fn rerun(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: Back,
    Path(id): Path<i64>,
) -> Result<Response> {
    let run = RuleRun::find_or_404(&state.db, id).await?;
    let _done = run
        .transition(transitions::RERUN)
        .by(&user)
        .run(&state)
        .await
        .map_err(TransitionError::into_error)?;
    let toast = Toast::success(texts::t(&lang, "wl_kernel.rule_run.toast.queued"));
    Ok((toast, back).into_response())
}
