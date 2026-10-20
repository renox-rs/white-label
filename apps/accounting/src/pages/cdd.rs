//! The CDD file page and its actions (04 §7.1 *CDD file*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /accounting/cdd` | `accounting.cdd_open` | `cdd.prepare` |
//! | `GET /accounting/cdd/{id}` | `accounting.cdd_file` | `cdd.view` |
//! | `POST /accounting/cdd/{id}/parties` | `accounting.cdd_party_add` | `cdd.prepare` |
//! | `POST /accounting/cdd/{id}/assessment` | `accounting.cdd_assess` | `cdd.prepare` |
//! | `POST /accounting/cdd/{id}/submit` | `accounting.cdd_submit` | `cdd.prepare` |
//! | `POST /accounting/cdd/{id}/approve` | `accounting.cdd_approve` | `cdd.approve` |
//! | `POST /accounting/cdd/{id}/reject` | `accounting.cdd_reject` | `cdd.approve` |
//! | `POST /accounting/cdd/{id}/reopen` | `accounting.cdd_reopen` | `cdd.prepare` |
//! | `POST /accounting/cdd/{id}/exit` | `accounting.cdd_exit` | `cdd.approve` |
//!
//! The exit or rejection reason, and which party has an open possible match,
//! show only to holders of `parties.block_reason.view` (the MLRO and
//! deputy); everyone else sees "Screening pending" (BL-ACC-08).

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use super::{Texts, invalid, parse_id, party_name, require, text};
use crate::cdd::{CddFile, CddStatus, RiskRating, ScreenedRole};
use crate::cdd_files::{self, Assessment};
use crate::permissions;
use crate::texts;

/// The routes of the page.
pub fn routes() -> Routes {
    Routes::new()
        .post("/accounting/cdd", open)
        .name("accounting.cdd_open")
        .get("/accounting/cdd/{id}", show)
        .name("accounting.cdd_file")
        .post("/accounting/cdd/{id}/parties", add_party)
        .name("accounting.cdd_party_add")
        .post("/accounting/cdd/{id}/assessment", assess)
        .name("accounting.cdd_assess")
        .post("/accounting/cdd/{id}/submit", submit)
        .name("accounting.cdd_submit")
        .post("/accounting/cdd/{id}/approve", approve)
        .name("accounting.cdd_approve")
        .post("/accounting/cdd/{id}/reject", reject)
        .name("accounting.cdd_reject")
        .post("/accounting/cdd/{id}/reopen", reopen)
        .name("accounting.cdd_reopen")
        .post("/accounting/cdd/{id}/exit", exit)
        .name("accounting.cdd_exit")
        .require_auth()
}

/// The badge kind of a CDD status.
fn status_kind(status: CddStatus) -> &'static str {
    match status {
        CddStatus::Approved => "success",
        CddStatus::ReviewDue => "warning",
        CddStatus::Rejected | CddStatus::Exited => "error",
        CddStatus::InProgress | CddStatus::PendingApproval => "info",
    }
}

/// A screened party of the file.
#[derive(Serialize, Debug, Clone)]
pub struct PartyRow {
    /// The party's name.
    pub name: String,
    /// What it is to the client.
    pub role: String,
}

/// The file as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct FileView {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The client's name.
    pub client: String,
    /// The risk rating.
    pub rating: String,
    /// The risk factors.
    pub risk_factors: String,
    /// Whether the identity documents are attached.
    pub documents: String,
    /// The next review date.
    pub next_review: String,
    /// The exit or rejection reason: empty unless the viewer may see it.
    pub decision_reason: String,
}

/// What the CDD page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/cdd_file.html")]
pub(crate) struct CddPage {
    tx: Texts,
    file: FileView,
    status: String,
    status_kind: &'static str,
    parties: Vec<PartyRow>,
    screening: Vec<String>,
    roles: Vec<[String; 2]>,
    ratings: Vec<[String; 2]>,
    can_prepare: bool,
    can_submit: bool,
    can_approve: bool,
    can_reopen: bool,
    can_exit: bool,
}

/// `GET /accounting/cdd/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::CDD_VIEW)?;
    let found = CddFile::find_or_404(&state.db, id).await?;
    let mut parties = Vec::new();
    for row in found.parties(&state.db).await? {
        parties.push(PartyRow {
            name: party_name(&state, &lang, row.party()).await,
            role: text(&lang, &format!("cdd.role.{}", row.role.as_str())),
        });
    }
    let sees_reason = user.has_permission(permissions::BLOCK_REASON_VIEW);
    let view = FileView {
        id: found.id,
        number: found.number(),
        client: party_name(&state, &lang, found.client()).await,
        rating: text(&lang, &format!("cdd.rating.{}", found.risk_rating.as_str())),
        risk_factors: found.risk_factors.clone(),
        documents: text(
            &lang,
            if found.identity_documents_attached {
                "cdd.documents_attached"
            } else {
                "cdd.documents_missing"
            },
        ),
        next_review: found
            .next_review_date
            .map(|d| d.to_string())
            .unwrap_or_default(),
        decision_reason: if sees_reason {
            found.decision_reason.clone().unwrap_or_default()
        } else {
            String::new()
        },
    };
    let prepare = user.has_permission(permissions::CDD_PREPARE);
    let decide = user.has_permission(permissions::CDD_APPROVE);
    Ok(View::typed(CddPage {
        tx: texts::all(&lang),
        status: text(&lang, &format!("cdd.status.{}", found.status.as_str())),
        status_kind: status_kind(found.status),
        parties,
        screening: cdd_files::screening_lines(&state, &user, &found).await?,
        roles: [
            ScreenedRole::Ubo,
            ScreenedRole::Director,
            ScreenedRole::Signatory,
        ]
        .iter()
        .map(|r| {
            [
                r.as_str().to_owned(),
                text(&lang, &format!("cdd.role.{}", r.as_str())),
            ]
        })
        .collect(),
        ratings: RiskRating::ALL
            .iter()
            .map(|r| {
                [
                    r.as_str().to_owned(),
                    text(&lang, &format!("cdd.rating.{}", r.as_str())),
                ]
            })
            .collect(),
        can_prepare: prepare && found.status == CddStatus::InProgress,
        can_submit: prepare && found.status == CddStatus::InProgress,
        can_approve: decide && found.status == CddStatus::PendingApproval,
        can_reopen: prepare
            && matches!(
                found.status,
                CddStatus::ReviewDue | CddStatus::PendingApproval
            ),
        can_exit: decide
            && matches!(
                found.status,
                CddStatus::Approved | CddStatus::ReviewDue | CddStatus::InProgress
            ),
        file: view,
    }))
}

/// The form that opens a file.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct OpenForm {
    /// The client, a party's id.
    #[serde(default)]
    pub client: String,
}

/// The answer of an action: a toast and a refresh.
fn done(lang: &Lang, key: &str) -> Response {
    (Toast::success(text(lang, key)), renox::HxRefresh).into_response()
}

/// `POST /accounting/cdd`.
async fn open(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<OpenForm>,
) -> Result<Response> {
    require(&user, permissions::CDD_PREPARE)?;
    let client = parse_id(&form.client).ok_or_else(|| invalid("client: 1, 2, 3 ..."))?;
    cdd_files::open(&state.db, Some(user.id), PartyId(client)).await?;
    Ok(done(&lang, "cdd.opened"))
}

/// The form that adds a screened party.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct PartyForm {
    /// The party's id.
    #[serde(default)]
    pub party: String,
    /// `ubo`, `director` or `signatory`.
    #[serde(default)]
    pub role: String,
}

/// `POST /accounting/cdd/{id}/parties`.
async fn add_party(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<PartyForm>,
) -> Result<Response> {
    require(&user, permissions::CDD_PREPARE)?;
    let party = parse_id(&form.party).ok_or_else(|| invalid("party: 1, 2, 3 ..."))?;
    let role = [
        ScreenedRole::Ubo,
        ScreenedRole::Director,
        ScreenedRole::Signatory,
    ]
    .into_iter()
    .find(|r| r.as_str() == form.role.trim())
    .ok_or_else(|| invalid("role: ubo, director, signatory"))?;
    cdd_files::add_party(&state, id, PartyId(party), role).await?;
    Ok(done(&lang, "cdd.party_added"))
}

/// The form of the assessment.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct AssessForm {
    /// `low`, `medium` or `high`.
    #[serde(default)]
    pub rating: String,
    /// The risk factors.
    #[serde(default)]
    pub risk_factors: String,
    /// The EDD notes.
    #[serde(default)]
    pub edd_notes: String,
    /// Ticked (`on`, `1`, `true`) when the identity documents are attached.
    #[serde(default)]
    pub documents: String,
}

/// `POST /accounting/cdd/{id}/assessment`.
async fn assess(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<AssessForm>,
) -> Result<Response> {
    require(&user, permissions::CDD_PREPARE)?;
    let rating = RiskRating::ALL
        .iter()
        .copied()
        .find(|r| r.as_str() == form.rating.trim())
        .ok_or_else(|| invalid("rating: low, medium, high"))?;
    let mut assessment = Assessment::rated(rating);
    assessment.risk_factors = form.risk_factors.trim().to_owned();
    assessment.edd_notes = form.edd_notes.trim().to_owned();
    assessment.identity_documents_attached = matches!(form.documents.trim(), "on" | "1" | "true");
    cdd_files::assess(&state, id, assessment).await?;
    Ok(done(&lang, "cdd.assessed"))
}

/// `POST /accounting/cdd/{id}/submit`.
async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    cdd_files::submit(&state, &user, id).await?;
    Ok(done(&lang, "cdd.submitted"))
}

/// `POST /accounting/cdd/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    cdd_files::approve(&state, &user, id).await?;
    Ok(done(&lang, "cdd.approved"))
}

/// The form of `reject` and `exit`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct DecisionForm {
    /// The reason (MLRO only).
    #[serde(default)]
    pub reason: String,
}

/// `POST /accounting/cdd/{id}/reject`.
async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<DecisionForm>,
) -> Result<Response> {
    cdd_files::reject(&state, &user, id, &form.reason).await?;
    Ok(done(&lang, "cdd.rejected"))
}

/// `POST /accounting/cdd/{id}/reopen`.
async fn reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    cdd_files::reopen(&state, &user, id).await?;
    Ok(done(&lang, "cdd.reopened"))
}

/// `POST /accounting/cdd/{id}/exit`.
async fn exit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<DecisionForm>,
) -> Result<Response> {
    cdd_files::exit(&state, &user, id, &form.reason).await?;
    Ok(done(&lang, "cdd.exited"))
}
