//! The back-office actions on a project (02 Construction §7.1 *Project*):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /construction/projects/award` | `construction.project_award` | `projects.award` |
//! | `POST /construction/ipas/{id}/submit` | `construction.ipa_submit` | `ipas.submit` in the project |
//! | `POST /construction/ipas/{id}/certificate` | `construction.ipa_certificate` | `ipas.certify` in the project |
//! | `GET /construction/ipas/{id}` | `construction.ipa_show` | `projects.view`, `projects.manage` or `ipas.certify` in the project |
//! | `POST /construction/ipas/{id}/invoice` | `construction.ipa_invoice` | `invoices.issue` in the project |
//! | `POST /construction/ipas/{id}/deductions` | `construction.deduction_record` | `ipas.deductions.record` in the project |
//! | `POST /construction/deductions/{id}/accept` | `construction.deduction_accept` | `ipas.deductions.decide` in the project |
//! | `POST /construction/deductions/{id}/contest` | `construction.deduction_contest` | `ipas.deductions.decide` in the project |
//! | `POST /construction/projects/{id}/terminate` | `construction.project_terminate` | `projects.terminate` in the project |
//! | `GET /construction/projects/{id}/closing` | `construction.project_closing` | `projects.view` or `projects.manage` in the project |
//! | `POST /construction/projects/{id}/settle` | `construction.project_settle` | `projects.manage` in the project |
//! | `POST /construction/projects/{id}/cancel` | `construction.project_cancel` | `projects.cancel` |
//!
//! The approval of a termination is the kernel's: the MD finds the request
//! under "Needs your action". The closing screen answers in JSON with the
//! final account and the warnings of what is still open.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::PartyId;

use crate::award::{self, Award, Awarded};
use crate::cancellation;
use crate::certificate::{self, CertificateRequest, Recorded};
use crate::deduction::{self, Decided, DeductionKind, NewDeduction};
use crate::final_account::{self, ClosingWarnings};
use crate::invoice;
use crate::ipa::{self, Ipa};
use crate::permissions;
use crate::project::Project;
use crate::submission;
use crate::termination::{self, TerminateRequest, TerminationReason};
use crate::texts::t;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .post("/construction/projects/award", award)
        .name("construction.project_award")
        .post("/construction/ipas/{id}/submit", submit_ipa)
        .name("construction.ipa_submit")
        .post("/construction/ipas/{id}/certificate", certificate)
        .name("construction.ipa_certificate")
        .get("/construction/ipas/{id}", show_ipa)
        .name("construction.ipa_show")
        .post("/construction/ipas/{id}/invoice", invoice_ipa)
        .name("construction.ipa_invoice")
        .post("/construction/ipas/{id}/deductions", record_deduction)
        .name("construction.deduction_record")
        .post("/construction/deductions/{id}/accept", accept_deduction)
        .name("construction.deduction_accept")
        .post("/construction/deductions/{id}/contest", contest_deduction)
        .name("construction.deduction_contest")
        .post("/construction/projects/{id}/terminate", terminate)
        .name("construction.project_terminate")
        .get("/construction/projects/{id}/closing", closing)
        .name("construction.project_closing")
        .post("/construction/projects/{id}/settle", settle)
        .name("construction.project_settle")
        .post("/construction/projects/{id}/cancel", cancel)
        .name("construction.project_cancel")
        .require_auth()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A transition's refusal as an answer.
fn answer(error: TransitionError) -> Error {
    match error {
        TransitionError::Guard(message) => refused(message),
        other => other.into_error(),
    }
}

/// `POST /construction/projects/award`: `client_party_id`, `code`, `name`,
/// `contract_value` (fils) and `award_document`. A blocked client is refused
/// with the text the viewer may see.
async fn award(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let text = |name: &str| form.get(name).cloned().unwrap_or_default();
    let client = text("client_party_id")
        .trim()
        .parse::<i64>()
        .map_err(|_| refused(t(&lang, "construction.bad_input")))?;
    let contract_value = match text("contract_value").trim() {
        "" => 0,
        value => value
            .parse::<i64>()
            .map_err(|_| refused(t(&lang, "construction.bad_input")))?,
    };
    let given = Award {
        client: PartyId(client),
        code: text("code"),
        name: text("name"),
        contract_value,
        award_document: text("award_document"),
        manager_user_id: None,
    };
    match award::accept(&state, &user, &given).await.map_err(answer)? {
        Awarded::Started(_project) => Ok((
            Toast::success(t(&lang, "construction.award.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Awarded::Refused(refusal) => Err(refused(refusal.message(&lang))),
    }
}

/// `POST /construction/projects/{id}/terminate`: `reason`, `notice` and
/// `effective_on` (`2027-05-31`).
async fn terminate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form
        .get("reason")
        .and_then(|reason| TerminationReason::parse(reason))
        .ok_or_else(|| refused(t(&lang, "construction.terminate.reason_required")))?;
    let effective_on = form
        .get("effective_on")
        .and_then(|date| date.trim().parse::<NaiveDate>().ok())
        .ok_or_else(|| refused(t(&lang, "construction.terminate.date_required")))?;
    let notice = form.get("notice").cloned().unwrap_or_default();
    let ask = TerminateRequest {
        reason,
        notice,
        effective_on,
    };
    let outcome = termination::request(&state, &user, id, ask)
        .await
        .map_err(answer)?;
    let key = if outcome.is_pending() {
        "construction.terminate.requested"
    } else {
        "construction.terminate.done"
    };
    Ok((Toast::success(t(&lang, key)), renox::HxRefresh).into_response())
}

/// `POST /construction/ipas/{id}/submit`: `replaces`, the id of the pending
/// IPA this one replaces (empty when none is pending).
async fn submit_ipa(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let replaces = match form.get("replaces").map(|value| value.trim()) {
        None | Some("") => None,
        Some(value) => Some(
            value
                .parse::<i64>()
                .map_err(|_| refused(t(&lang, "construction.bad_input")))?,
        ),
    };
    submission::submit(&state, &user, id, replaces)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.ipa.submitted")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/ipas/{id}/certificate`: `certified_amount` (fils, the
/// cumulative work before deductions), `certificate` and `certified_on`
/// (`2027-03-05`, today when empty). For a superseded IPA the certificate is
/// recorded on the IPA that replaced it.
async fn certificate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let certified_amount = form
        .get("certified_amount")
        .and_then(|amount| amount.trim().parse::<i64>().ok())
        .ok_or_else(|| refused(t(&lang, "construction.ipa.certified_amount_needed")))?;
    let certified_on = match form.get("certified_on").map(|value| value.trim()) {
        None | Some("") => None,
        Some(value) => Some(
            value
                .parse::<NaiveDate>()
                .map_err(|_| refused(t(&lang, "construction.bad_input")))?,
        ),
    };
    let request = CertificateRequest {
        certified_amount,
        certificate: form.get("certificate").cloned().unwrap_or_default(),
        certified_on,
    };
    let key = match certificate::record(&state, &user, id, &request)
        .await
        .map_err(answer)?
    {
        Recorded::Certified(_) => "construction.ipa.certified",
        Recorded::Attached(_) => "construction.ipa.certificate_attached",
    };
    Ok((Toast::success(t(&lang, key)), renox::HxRefresh).into_response())
}

/// `POST /construction/ipas/{id}/invoice`: the tax invoice, the credit note,
/// the dispute and the supplier bill from the client (see [`crate::invoice`]).
async fn invoice_ipa(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    invoice::invoice(&state, &user, id).await.map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.invoice.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// A deduction line as the screen shows it.
#[derive(Debug, Serialize)]
struct DeductionView {
    id: i64,
    ipa_id: i64,
    kind: &'static str,
    reference: String,
    description: String,
    amount: i64,
    status: &'static str,
}

impl DeductionView {
    fn of(line: &deduction::IpaDeduction) -> Self {
        DeductionView {
            id: line.id,
            ipa_id: line.ipa_id,
            kind: line.kind.as_str(),
            reference: line.reference.clone(),
            description: line.description.clone(),
            amount: line.amount,
            status: line.status.as_str(),
        }
    }
}

/// `POST /construction/ipas/{id}/deductions`: `kind`, `reference`, `amount`
/// (fils, excluding VAT) and `description` (needed for the kind `other`).
async fn record_deduction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let text = |name: &str| form.get(name).cloned().unwrap_or_default();
    let kind = [
        DeductionKind::BackCharge,
        DeductionKind::ContraCharge,
        DeductionKind::LiquidatedDamages,
        DeductionKind::Other,
    ]
    .into_iter()
    .find(|kind| kind.as_str() == text("kind").trim())
    .ok_or_else(|| refused(t(&lang, "construction.deduction.kind_unknown")))?;
    let amount = text("amount")
        .trim()
        .parse::<i64>()
        .map_err(|_| refused(t(&lang, "construction.deduction.amount_needed")))?;
    let new = NewDeduction {
        kind,
        reference: text("reference"),
        description: text("description"),
        amount,
    };
    deduction::record(&state, &user, id, &new)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.deduction.recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/deductions/{id}/accept`: above AED 25,000 the MD is
/// asked and the line stays undecided until then.
async fn accept_deduction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let key = match deduction::accept(&state, &user, id).await.map_err(answer)? {
        Decided::Done(_) => "construction.deduction.accepted",
        Decided::AwaitingMd(_) => "construction.deduction.awaiting_md",
    };
    Ok((Toast::success(t(&lang, key)), renox::HxRefresh).into_response())
}

/// `POST /construction/deductions/{id}/contest`.
async fn contest_deduction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    deduction::contest(&state, &user, id)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.deduction.contested")),
        renox::HxRefresh,
    )
        .into_response())
}

/// An IPA as its screen shows it.
#[derive(Debug, Serialize)]
struct IpaView {
    id: i64,
    project_id: i64,
    number: i64,
    status: &'static str,
    previous_pending: bool,
    previous_certified: i64,
    replaces: Option<i64>,
    claimed_amount: i64,
    certified_amount: Option<i64>,
    claim_gap: Option<i64>,
    certificate: Option<String>,
    late_certificate: Option<String>,
    invoice_id: Option<i64>,
    credit_note_id: Option<i64>,
    dispute_id: Option<i64>,
    supplier_bill_id: Option<i64>,
    deductions: Vec<DeductionView>,
}

/// `GET /construction/ipas/{id}`.
async fn show_ipa(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    let found = Ipa::find_or_404(&state.db, id).await?;
    let project = Project::find_or_404(&state.db, found.project_id).await?;
    let scope = project.scope();
    if ![
        permissions::PROJECTS_VIEW,
        permissions::PROJECTS_MANAGE,
        permissions::IPAS_CERTIFY,
    ]
    .iter()
    .any(|permission| user.has_permission_in(permission, &scope))
    {
        return Err(Error::Forbidden);
    }
    let view = IpaView {
        id: found.id,
        project_id: found.project_id,
        number: found.number,
        status: found.status.as_str(),
        previous_pending: found.previous_pending,
        previous_certified: found.previous_certified,
        replaces: found.replaces_id,
        claimed_amount: found.claimed_amount,
        certified_amount: found.certified_amount,
        claim_gap: found.claim_gap,
        certificate: found.certificate_ref.clone(),
        late_certificate: found.late_certificate_ref.clone(),
        invoice_id: found.invoice_id,
        credit_note_id: found.credit_note_id,
        dispute_id: found.dispute_id,
        supplier_bill_id: found.supplier_bill_id,
        deductions: deduction::lines_of(&state.db, id)
            .await?
            .iter()
            .map(DeductionView::of)
            .collect(),
    };
    let body = renox::serde_json::to_string(&view)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// The final account as the closing screen shows it.
#[derive(Debug, Serialize)]
struct FinalAccountView {
    id: i64,
    status: &'static str,
    valuation_on: Option<NaiveDate>,
    certified_amount: Option<i64>,
    decided_amount: Option<i64>,
    decision_reference: Option<String>,
    invoice_id: Option<i64>,
}

/// The closing screen of a terminated project.
#[derive(Debug, Serialize)]
struct ClosingView {
    status: &'static str,
    termination_reason: Option<String>,
    effective_on: Option<NaiveDate>,
    final_account: Option<FinalAccountView>,
    warnings: ClosingWarnings,
    warning_texts: Vec<String>,
    contested_deductions: Vec<DeductionView>,
}

/// `GET /construction/projects/{id}/closing`.
async fn closing(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    let project = Project::find_or_404(&state.db, id).await?;
    let scope = project.scope();
    if !user.has_permission_in(permissions::PROJECTS_VIEW, &scope)
        && !user.has_permission_in(permissions::PROJECTS_MANAGE, &scope)
    {
        return Err(Error::Forbidden);
    }
    let account = ipa::final_account_of(&state.db, id).await?;
    let warnings = final_account::closing_warnings(&state, id).await?;
    let view = ClosingView {
        status: project.status.as_str(),
        termination_reason: project.termination_reason.clone(),
        effective_on: project.termination_effective_on,
        final_account: account.map(|account| FinalAccountView {
            id: account.id,
            status: account.status.as_str(),
            valuation_on: account.valuation_on,
            certified_amount: account.certified_amount,
            decided_amount: account.decided_amount,
            decision_reference: account.decision_reference.clone(),
            invoice_id: account.invoice_id,
        }),
        warning_texts: warnings.texts(),
        warnings,
        contested_deductions: deduction::final_account_items(&state.db, id)
            .await?
            .iter()
            .map(DeductionView::of)
            .collect(),
    };
    let body = renox::serde_json::to_string(&view)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// `POST /construction/projects/{id}/settle`.
async fn settle(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    final_account::settle_final_account(&state, &user, id)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.settle.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/projects/{id}/cancel`: `reason`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map(String::as_str).unwrap_or_default();
    if reason.trim().is_empty() {
        return Err(refused(t(&lang, "construction.cancel.reason_needed")));
    }
    cancellation::cancel(&state, &user, id, reason)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.cancel.done")),
        renox::HxRefresh,
    )
        .into_response())
}
