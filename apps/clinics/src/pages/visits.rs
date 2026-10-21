//! The visit page (05 `CLN.ENC.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/visits/{id}` | `clinics.visit` | `visits.view` |
//! | `POST /clinics/visits/{id}/insured` | `clinics.visit_insured` | `coverage.record` |
//!
//! The page shows the cover the check-in recorded. When the eligibility turns
//! positive before the service starts, the receptionist changes the outcome
//! to *insured*; after `start` the outcome is fixed (`CLN.ACL.15`).

use renox::prelude::*;
use renox::state_machine::history;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_text;

use super::appointments::{outcome_label, policy_label};
use super::{Texts, party_name, require, text, user_name};
use crate::authorisation::AuthorisationHold;
use crate::consent::SelfPayConsent;
use crate::coverage::{self, CoverageOutcome};
use crate::money_text;
use crate::permissions;
use crate::policy::InsurancePolicy;
use crate::texts;
use crate::visit::{Visit, VisitStatus};

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/visits/{id}", show)
        .name("clinics.visit")
        .post("/clinics/visits/{id}/insured", insured)
        .name("clinics.visit_insured")
        .require_auth()
}

/// The badge kind of a visit status.
fn status_kind(status: VisitStatus) -> &'static str {
    match status {
        VisitStatus::CheckedIn | VisitStatus::InProgress => "info",
        VisitStatus::Completed => "success",
        VisitStatus::Cancelled => "warning",
    }
}

/// One row of the history.
#[derive(Serialize, Debug, Clone)]
pub struct HistoryRow {
    /// When (Asia/Dubai).
    pub at: String,
    /// What happened.
    pub action: String,
    /// Who ran it, or the system.
    pub by: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/visit.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    booking: i64,
    patient: String,
    service: String,
    visit_date: String,
    status: String,
    status_kind: &'static str,
    policy: String,
    outcome: String,
    outcome_kind: &'static str,
    split_policy: String,
    consent: String,
    estimate: String,
    holds: String,
    can_insure: bool,
    history: Vec<HistoryRow>,
}

/// `GET /clinics/visits/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::VISITS_VIEW)?;
    let visit = Visit::find_or_404(&state.db, id).await?;
    let none = text(&lang, "none");
    let patient = match visit.patient_party_id {
        Some(party) => party_name(&state, &lang, wl_kernel::PartyId(party)).await,
        None => none.clone(),
    };
    let policy_text = |policy: Option<InsurancePolicy>| {
        policy.map_or_else(|| none.clone(), |p| policy_label(&lang, &p))
    };
    let booked_with = match visit.policy_id {
        Some(policy) => InsurancePolicy::find(&state.db, policy).await?,
        None => None,
    };
    let split_with = match visit.coverage_policy_id {
        Some(policy) => InsurancePolicy::find(&state.db, policy).await?,
        None => None,
    };
    let consent = match visit.consent_id {
        Some(consent) => SelfPayConsent::find(&state.db, consent).await?,
        None => None,
    };
    let held = AuthorisationHold::of_visit(&state.db, visit.id).await?;
    let holds = if held.is_empty() {
        none.clone()
    } else {
        held.iter()
            .map(|hold| {
                format!(
                    "{}: PA-{:04} ({})",
                    hold.service_code, hold.authorisation_id, hold.units
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut rows = Vec::new();
    for row in history(&state.db, Visit::TABLE, visit.id).await? {
        let by = match row.actor_id {
            Some(actor) => user_name(&state.db, actor).await,
            None => text(&lang, "history.system"),
        };
        rows.push(HistoryRow {
            at: dubai_text(row.at),
            action: text(&lang, &format!("visits.transition.{}", row.transition)),
            by,
        });
    }
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: visit.id,
        number: visit.number(),
        booking: visit.booking_id,
        patient,
        service: visit.service_code.clone(),
        visit_date: visit.visit_date.to_string(),
        status: text(&lang, &format!("visits.status.{}", visit.status.as_str())),
        status_kind: status_kind(visit.status),
        policy: policy_text(booked_with),
        outcome: visit.coverage_outcome.map_or_else(
            || text(&lang, "coverage.outcome_none"),
            |o| outcome_label(&lang, o),
        ),
        outcome_kind: match visit.coverage_outcome {
            Some(CoverageOutcome::Insured) => "success",
            Some(_) => "warning",
            None => "info",
        },
        split_policy: policy_text(split_with),
        consent: consent.as_ref().map_or_else(
            || none.clone(),
            |c| format!("{} — {}", c.signer_name, c.signed_on),
        ),
        estimate: consent.as_ref().map_or_else(
            || none.clone(),
            |c| format!("{} ({})", money_text(c.estimated_fils), c.lines_text()),
        ),
        holds,
        can_insure: user.has_permission(permissions::COVERAGE_RECORD)
            && visit.is_insured()
            && visit.status == VisitStatus::CheckedIn
            && visit.coverage_outcome != Some(CoverageOutcome::Insured),
        history: rows,
    }))
}

/// The form that changes the outcome to insured.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct InsuredForm {
    /// The payer's reference of the positive result.
    #[serde(default)]
    pub reference: String,
}

/// `POST /clinics/visits/{id}/insured`.
async fn insured(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<InsuredForm>,
) -> Result<Response> {
    require(&user, permissions::COVERAGE_RECORD)?;
    let reference = Some(form.reference.trim()).filter(|r| !r.is_empty());
    coverage::change_to_insured(&state, Some(user.id), id, reference, renox::db::now()).await?;
    Ok((
        Toast::success(text(&lang, "coverage.changed_to_insured")),
        renox::HxRefresh,
    )
        .into_response())
}
