//! The AML case pages: the MLRO's alone (04 §7.1 *AML case*, `ACC.ACL.05`;
//! BL-ACC-10).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /accounting/aml-cases` | `accounting.aml_cases` | `aml.manage` |
//! | `GET /accounting/aml-cases/{id}` | `accounting.aml_case` | `aml.manage` |
//! | `POST /accounting/aml-cases/{id}/review` | `accounting.aml_case_review` | `aml.manage` |
//! | `POST /accounting/aml-cases/{id}/close` | `accounting.aml_case_close` | `aml.manage` |
//! | `POST /accounting/aml-cases/{id}/report` | `accounting.aml_case_report` | `aml.manage` |
//! | `POST /accounting/aml-cases/{id}/decision` | `accounting.aml_case_decision` | `aml.manage` |
//! | `POST /accounting/engagements/{id}/aml-concern` | `accounting.engagement_aml_concern` | `aml.raise` |
//!
//! A user without `aml.manage` gets a 404, never a 403: the pages do not say
//! that a case exists. The concern a member of staff raises answers with a
//! toast and nothing else.

use renox::prelude::*;
use renox::state_machine::history;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_text;

use super::{Texts, invalid, party_name, text, user_name};
use crate::aml::{AmlCase, AmlDecision, AmlStatus};
use crate::aml_cases;
use crate::engagement::Engagement;
use crate::permissions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/accounting/aml-cases", index)
        .name("accounting.aml_cases")
        .get("/accounting/aml-cases/{id}", show)
        .name("accounting.aml_case")
        .post("/accounting/aml-cases/{id}/review", review)
        .name("accounting.aml_case_review")
        .post("/accounting/aml-cases/{id}/close", close)
        .name("accounting.aml_case_close")
        .post("/accounting/aml-cases/{id}/report", report)
        .name("accounting.aml_case_report")
        .post("/accounting/aml-cases/{id}/decision", decide)
        .name("accounting.aml_case_decision")
        .post("/accounting/engagements/{id}/aml-concern", raise)
        .name("accounting.engagement_aml_concern")
        .require_auth()
}

/// `NotFound` unless `user` holds `aml.manage`.
fn mlro(user: &AuthUser) -> Result {
    if user.has_permission(permissions::AML_MANAGE) {
        Ok(())
    } else {
        Err(Error::NotFound)
    }
}

/// The badge kind of an AML status.
fn status_kind(status: AmlStatus) -> &'static str {
    match status {
        AmlStatus::Open | AmlStatus::UnderReview => "warning",
        AmlStatus::Reported => "error",
        AmlStatus::ClosedNoAction => "info",
    }
}

/// One case of the list.
#[derive(Serialize, Debug, Clone)]
pub struct CaseRow {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The client's name.
    pub client: String,
    /// Concern or sanctions match.
    pub kind: String,
    /// The status.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/aml_cases.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    cases: Vec<CaseRow>,
}

/// `GET /accounting/aml-cases`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    mlro(&user)?;
    let mut cases = Vec::new();
    let mut found = AmlCase::query()
        .order_by_desc(AmlCase::ID)
        .get(&state.db)
        .await?;
    found.truncate(200);
    for case in found {
        cases.push(CaseRow {
            id: case.id,
            number: case.number(),
            client: party_name(&state, &lang, case.client()).await,
            kind: text(&lang, &format!("aml.kind.{}", case.kind.as_str())),
            status: text(&lang, &format!("aml.status.{}", case.status.as_str())),
            status_kind: status_kind(case.status),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        cases,
    }))
}

/// A confirmed match linked to the case.
#[derive(Serialize, Debug, Clone)]
pub struct MatchRow {
    /// The matched party.
    pub party: String,
    /// What it is to the client.
    pub role: String,
    /// The list and the entry.
    pub list: String,
    /// The freeze and report tasks with their state.
    pub tasks: String,
}

/// One row of the case's history.
#[derive(Serialize, Debug, Clone)]
pub struct HistoryRow {
    /// When (Asia/Dubai).
    pub at: String,
    /// The transition.
    pub action: String,
    /// Who ran it, or the system.
    pub by: String,
}

/// What the case page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/aml_case.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    client: String,
    kind: String,
    facts: String,
    raised_by: String,
    block_note: String,
    closing_reason: String,
    goaml_reference: String,
    goaml_date: String,
    status: String,
    status_kind: &'static str,
    matches: Vec<MatchRow>,
    history: Vec<HistoryRow>,
    can_review: bool,
    can_close: bool,
    can_report: bool,
    can_decide: bool,
    decision: String,
    decision_options: Vec<[String; 2]>,
    today: String,
}

/// `GET /accounting/aml-cases/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let case = aml_cases::visible(&state, &user, id).await?;
    let guard = wl_kernel::screening_guard::screening_guard(&state);
    let mut matches = Vec::new();
    for row in case.matches(&state.db).await? {
        let confirmed = guard.confirmed_match(&state.db, row.screening_id).await?;
        let tasks = confirmed
            .map(|c| {
                c.tasks
                    .iter()
                    .map(|task| {
                        let kind = match task.kind {
                            wl_kernel::screening_guard::MatchTaskKind::Freeze => "freeze",
                            wl_kernel::screening_guard::MatchTaskKind::Report => "report",
                        };
                        let state_text = if task.done {
                            text(&lang, "aml.task_done")
                        } else {
                            format!(
                                "{}, {}",
                                text(&lang, "aml.task_open"),
                                texts::t_with(
                                    &lang,
                                    "accounting.aml.task_due",
                                    &[("due", &dubai_text(task.due_at))]
                                )
                            )
                        };
                        format!("{kind}: {state_text}")
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        matches.push(MatchRow {
            party: party_name(&state, &lang, wl_kernel::PartyId(row.matched_party_id)).await,
            role: text(&lang, &format!("cdd.role.{}", row.role.as_str())),
            list: format!(
                "{} {}",
                row.list_name.clone().unwrap_or_default(),
                row.list_entry.clone().unwrap_or_default()
            )
            .trim()
            .to_owned(),
            tasks,
        });
    }
    let mut rows = Vec::new();
    for row in history(&state.db, AmlCase::TABLE, case.id).await? {
        rows.push(HistoryRow {
            at: dubai_text(row.at),
            action: text(&lang, &format!("aml.transition.{}", row.transition)),
            by: match row.actor_id {
                Some(id) => user_name(&state.db, id).await,
                None => text(&lang, "history.system"),
            },
        });
    }
    let raised_by = match case.raised_by {
        Some(id) => user_name(&state.db, id).await,
        None => text(&lang, "aml.system"),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: case.id,
        number: case.number(),
        client: party_name(&state, &lang, case.client()).await,
        kind: text(&lang, &format!("aml.kind.{}", case.kind.as_str())),
        facts: case.facts.clone(),
        raised_by,
        block_note: case.block_note.clone().unwrap_or_default(),
        closing_reason: case.closing_reason.clone().unwrap_or_default(),
        goaml_reference: case.goaml_reference.clone().unwrap_or_default(),
        goaml_date: case.goaml_date.map(|d| d.to_string()).unwrap_or_default(),
        status: text(&lang, &format!("aml.status.{}", case.status.as_str())),
        status_kind: status_kind(case.status),
        matches,
        history: rows,
        can_review: case.status == AmlStatus::Open,
        can_close: case.status == AmlStatus::UnderReview,
        can_report: case.status == AmlStatus::UnderReview,
        can_decide: case.status == AmlStatus::Reported && case.engagement_decision.is_none(),
        decision: case
            .engagement_decision
            .map(|d| {
                let label = text(&lang, &format!("aml.decision.{}", d.as_str()));
                match (case.decided_on, case.decision_note.as_deref()) {
                    (Some(day), Some(note)) => format!("{label} ({day}): {note}"),
                    (Some(day), None) => format!("{label} ({day})"),
                    _ => label,
                }
            })
            .unwrap_or_default(),
        decision_options: AmlDecision::ALL
            .iter()
            .map(|d| {
                [
                    d.as_str().to_owned(),
                    text(&lang, &format!("aml.decision.{}", d.as_str())),
                ]
            })
            .collect(),
        today: wl_kernel::documents::today().to_string(),
    }))
}

/// `POST /accounting/aml-cases/{id}/review`.
async fn review(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    aml_cases::review(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "aml.reviewed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that closes a case.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CloseForm {
    /// Why nothing is reported.
    #[serde(default)]
    pub reason: String,
}

/// `POST /accounting/aml-cases/{id}/close`.
async fn close(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CloseForm>,
) -> Result<Response> {
    aml_cases::close(&state, &user, id, &form.reason).await?;
    Ok((Toast::success(text(&lang, "aml.closed")), renox::HxRefresh).into_response())
}

/// The form that records the report.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReportForm {
    /// The goAML reference.
    #[serde(default)]
    pub reference: String,
    /// The date reported.
    #[serde(default)]
    pub date: String,
}

/// `POST /accounting/aml-cases/{id}/report`.
async fn report(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReportForm>,
) -> Result<Response> {
    let date = if form.date.trim().is_empty() {
        None
    } else {
        Some(
            form.date
                .trim()
                .parse()
                .map_err(|_| invalid("date: YYYY-MM-DD"))?,
        )
    };
    aml_cases::report(&state, &user, id, &form.reference, date).await?;
    Ok((
        Toast::success(text(&lang, "aml.reported")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that records the engagement decision.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct DecisionForm {
    /// `continue`, `suspend` or `exit`.
    #[serde(default)]
    pub decision: String,
    /// The MLRO's note.
    #[serde(default)]
    pub note: String,
}

/// `POST /accounting/aml-cases/{id}/decision`.
async fn decide(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<DecisionForm>,
) -> Result<Response> {
    let decision = AmlDecision::ALL
        .iter()
        .copied()
        .find(|d| d.as_str() == form.decision.trim())
        .ok_or_else(|| invalid("decision: continue, suspend, exit"))?;
    aml_cases::decide(&state, &user, id, decision, &form.note).await?;
    Ok((
        Toast::success(text(&lang, "aml.decision_recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that raises a concern.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct RaiseForm {
    /// What was seen.
    #[serde(default)]
    pub facts: String,
}

/// `POST /accounting/engagements/{id}/aml-concern`: a member of staff tells
/// the MLRO. The answer says only that the MLRO is told.
async fn raise(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<RaiseForm>,
) -> Result<Response> {
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    aml_cases::raise(&state, &user, engagement.client(), &form.facts).await?;
    Ok((Toast::success(text(&lang, "aml.raised")), renox::HxRefresh).into_response())
}
