//! The back-office actions on a project (02 Construction §7.1 *Project*):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /construction/projects` | `construction.projects` | `projects.view` or `projects.manage` in a project |
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
//! | `POST /construction/projects/{id}/complete` | `construction.project_complete` | `projects.manage` in the project |
//! | `POST /construction/projects/{id}/close` | `construction.project_close` | `projects.manage` in the project |
//! | `POST /construction/retentions/{id}/bond` | `construction.retention_bond` | `retentions.manage` in the project |
//! | `POST /construction/retentions/{id}/make-due` | `construction.retention_make_due` | `retentions.manage` in the project |
//! | `POST /construction/retentions/{id}/invoice` | `construction.retention_invoice` | `invoices.issue` in the project |
//! | `POST /construction/retentions/{id}/deductions` | `construction.retention_deduction_record` | `retentions.manage` in the project |
//! | `POST /construction/retention-deductions/{id}/accept` | `construction.retention_deduction_accept` | `ipas.deductions.decide` in the project |
//! | `POST /construction/retention-deductions/{id}/contest` | `construction.retention_deduction_contest` | `ipas.deductions.decide` in the project |
//! | `POST /construction/retentions/{id}/dispute` | `construction.retention_dispute` | `retentions.manage` in the project |
//! | `POST /construction/projects/{id}/guarantees` | `construction.guarantee_register` | `guarantees.manage` in the project |
//! | `POST /construction/guarantees/{id}/issue` | `construction.guarantee_issue` | `guarantees.manage` in the project |
//! | `POST /construction/guarantees/{id}/reduce` | `construction.guarantee_reduce` | `guarantees.manage` in the project |
//! | `POST /construction/guarantees/{id}/extend` | `construction.guarantee_extend` | `guarantees.manage` in the project |
//! | `POST /construction/guarantees/{id}/release` | `construction.guarantee_release` | `guarantees.manage` in the project |
//! | `POST /construction/guarantees/{id}/call` | `construction.guarantee_call` | `guarantees.manage` in the project |
//! | `POST /construction/guarantees/{id}/settle-call` | `construction.guarantee_settle_call` | `guarantees.manage` in the project |
//! | `POST /construction/guarantee-calls/{id}/dispute` | `construction.guarantee_call_dispute` | `projects.manage` in the project |
//! | `POST /construction/projects/{id}/retention-notice/dismiss` | `construction.retention_notice_dismiss` | `retentions.manage` in the project |
//!
//! The approval of a termination is the kernel's: the MD finds the request
//! under "Needs your action". The IPA, the closing screen (the final account
//! and the warnings of what is still open) and, in `variation_pages`, the
//! variation report answer a page when the request names `text/html` and
//! JSON otherwise ([`wl_kernel::negotiate`]); the projects list is a page.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::Serialize;
use wl_kernel::PartyId;

use crate::award::{self, Award, Awarded};
use crate::call;
use crate::cancellation;
use crate::certificate::{self, CertificateRequest, Recorded};
use crate::completion::{self, CompleteRequest};
use crate::deduction::{self, Decided, DeductionKind, NewDeduction};
use crate::final_account::{self, ClosingWarnings};
use crate::guarantee::{self, GuaranteeKind, NewGuarantee};
use crate::invoice;
use crate::ipa::{self, Ipa};
use crate::permissions;
use crate::project::Project;
use crate::release;
use crate::retention_deduction::{self, NewRetentionDeduction};
use crate::screens::{self, closing::Closing};
use crate::submission;
use crate::termination::{self, TerminateRequest, TerminationReason};
use crate::texts::t;
use crate::variation;
use crate::vat_notice;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/construction/projects", projects_index)
        .name("construction.projects")
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
        .post("/construction/projects/{id}/complete", complete_project)
        .name("construction.project_complete")
        .post("/construction/projects/{id}/close", close_project)
        .name("construction.project_close")
        .post("/construction/retentions/{id}/bond", link_bond)
        .name("construction.retention_bond")
        .post("/construction/retentions/{id}/make-due", make_due_with_bond)
        .name("construction.retention_make_due")
        .post("/construction/retentions/{id}/invoice", invoice_retention)
        .name("construction.retention_invoice")
        .post(
            "/construction/projects/{id}/retention-notice/dismiss",
            dismiss_retention_notice,
        )
        .name("construction.retention_notice_dismiss")
        .post(
            "/construction/retentions/{id}/deductions",
            record_retention_deduction,
        )
        .name("construction.retention_deduction_record")
        .post(
            "/construction/retention-deductions/{id}/accept",
            accept_retention_deduction,
        )
        .name("construction.retention_deduction_accept")
        .post(
            "/construction/retention-deductions/{id}/contest",
            contest_retention_deduction,
        )
        .name("construction.retention_deduction_contest")
        .post("/construction/retentions/{id}/dispute", dispute_retention)
        .name("construction.retention_dispute")
        .post("/construction/projects/{id}/guarantees", register_guarantee)
        .name("construction.guarantee_register")
        .post("/construction/guarantees/{id}/issue", issue_guarantee)
        .name("construction.guarantee_issue")
        .post("/construction/guarantees/{id}/reduce", reduce_guarantee)
        .name("construction.guarantee_reduce")
        .post("/construction/guarantees/{id}/extend", extend_guarantee)
        .name("construction.guarantee_extend")
        .post("/construction/guarantees/{id}/release", release_guarantee)
        .name("construction.guarantee_release")
        .post("/construction/guarantees/{id}/call", call_guarantee)
        .name("construction.guarantee_call")
        .post(
            "/construction/guarantees/{id}/settle-call",
            settle_guarantee_call,
        )
        .name("construction.guarantee_settle_call")
        .post(
            "/construction/guarantee-calls/{id}/dispute",
            dispute_guarantee_call,
        )
        .name("construction.guarantee_call_dispute")
        .require_auth()
}

/// A 422 with `message`.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A transition's refusal as an answer.
pub(crate) fn answer(error: TransitionError) -> Error {
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
    lang: Lang,
    headers: HeaderMap,
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
    if wl_kernel::negotiate::wants_html(&headers) {
        let replaced = match found.replaces_id {
            Some(earlier) => Ipa::find(&state.db, earlier).await?,
            None => None,
        };
        let lines = deduction::lines_of(&state.db, id).await?;
        return Ok(screens::ipa::page(
            &lang,
            &found,
            &project,
            replaced.as_ref(),
            &lines,
        ));
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
    contested_retention_deductions: Vec<RetentionDeductionView>,
    guarantee_call_claims: Vec<GuaranteeCallView>,
    variation_claims: Vec<VariationView>,
    unrecovered_variations: Vec<VariationView>,
}

/// A variation as the closing screen shows it: a claim of the final account
/// or an unrecovered cost.
#[derive(Debug, Serialize)]
struct VariationView {
    id: i64,
    code: String,
    revision: i64,
    status: &'static str,
    unrecovered_amount: i64,
    approved_amount: Option<i64>,
}

impl VariationView {
    fn of(variation: &variation::Variation) -> Self {
        VariationView {
            id: variation.id,
            code: variation.code.clone(),
            revision: variation.revision,
            status: variation.status.as_str(),
            unrecovered_amount: variation.unrecovered_amount,
            approved_amount: variation.approved_amount,
        }
    }
}

/// A disputed call on a guarantee as the closing screen shows it: a claim of
/// the final account.
#[derive(Debug, Serialize)]
struct GuaranteeCallView {
    id: i64,
    guarantee_id: i64,
    demand_notice: String,
    amount_paid: i64,
}

/// A deduction from the retention as the closing screen shows it.
#[derive(Debug, Serialize)]
struct RetentionDeductionView {
    id: i64,
    retention_id: i64,
    amount: i64,
    reference: String,
    reason: String,
    status: &'static str,
}

/// `GET /construction/projects/{id}/closing`.
async fn closing(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
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
    let deductions = deduction::final_account_items(&state.db, id).await?;
    let retention_deductions = retention_deduction::contested_of(&state.db, id).await?;
    let calls = call::claims_of(&state.db, id).await?;
    let claims = variation::claims_of(&state.db, id).await?;
    let unrecovered = variation::unrecovered_of(&state.db, id).await?;
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(screens::closing::page(
            &lang,
            &Closing {
                project: &project,
                account: account.as_ref(),
                warnings: warnings.texts(),
                deductions: &deductions,
                retention_deductions: &retention_deductions,
                calls: &calls,
                claims: &claims,
                unrecovered: &unrecovered,
            },
        ));
    }
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
        contested_deductions: deductions.iter().map(DeductionView::of).collect(),
        contested_retention_deductions: retention_deductions
            .iter()
            .map(|line| RetentionDeductionView {
                id: line.id,
                retention_id: line.retention_id,
                amount: line.amount,
                reference: line.reference.clone(),
                reason: line.reason.clone(),
                status: line.status.as_str(),
            })
            .collect(),
        guarantee_call_claims: calls
            .iter()
            .map(|claim| GuaranteeCallView {
                id: claim.id,
                guarantee_id: claim.guarantee_id,
                demand_notice: claim.demand_notice.clone(),
                amount_paid: claim.amount_paid.unwrap_or_default(),
            })
            .collect(),
        variation_claims: claims.iter().map(VariationView::of).collect(),
        unrecovered_variations: unrecovered.iter().map(VariationView::of).collect(),
    };
    let body = renox::serde_json::to_string(&view)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// `GET /construction/projects`: the projects the person may see, as a
/// page. 403 for a person who holds neither `projects.view` nor
/// `projects.manage` in any project.
async fn projects_index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    let may = [permissions::PROJECTS_VIEW, permissions::PROJECTS_MANAGE]
        .iter()
        .any(|permission| user.allows(permission) || user.has_permission_anywhere(permission));
    if !may {
        return Err(Error::Forbidden);
    }
    screens::projects::page(&state, &user, &lang, request).await
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

/// The toast of a bond step: the warning when the bond is below the held
/// amount, else `key`.
fn bond_toast(lang: &Lang, key: &str, warning: Option<String>) -> Toast {
    match warning {
        Some(text) => Toast::warning(text),
        None => Toast::success(t(lang, key)),
    }
}

/// `POST /construction/projects/{id}/complete`: `certificate` (the handover
/// certificate) and `completed_on` (`2027-06-15`).
async fn complete_project(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let completed_on = form
        .get("completed_on")
        .and_then(|date| date.trim().parse::<NaiveDate>().ok())
        .ok_or_else(|| refused(t(&lang, "construction.complete.date_needed")))?;
    let request = CompleteRequest {
        certificate: form.get("certificate").cloned().unwrap_or_default(),
        completed_on,
    };
    completion::complete(&state, &user, id, request)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.complete.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/projects/{id}/close`.
async fn close_project(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let closed = completion::close(&state, &user, id).await.map_err(answer)?;
    let toast = if closed.warnings.is_empty() {
        Toast::success(t(&lang, "construction.close.done"))
    } else {
        Toast::warning(closed.warnings.texts().join(" "))
    };
    Ok((toast, renox::HxRefresh).into_response())
}

/// `POST /construction/retentions/{id}/bond`: `guarantee_id`, the retention
/// bond that replaces the part. A bond below the held amount is a warning.
async fn link_bond(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let guarantee = form
        .get("guarantee_id")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .ok_or_else(|| refused(t(&lang, "construction.bad_input")))?;
    let linked = release::link_bond(&state, &user, id, guarantee)
        .await
        .map_err(answer)?;
    Ok((
        bond_toast(&lang, "construction.retention.bond_linked", linked.warning),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/retentions/{id}/make-due`: the `held` part is made due
/// because its retention bond is `active`.
async fn make_due_with_bond(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let made = release::make_due_with_bond(&state, &user, id)
        .await
        .map_err(answer)?;
    Ok((
        bond_toast(&lang, "construction.retention.made_due", made.warning),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/retentions/{id}/invoice`: the retention tax invoice of
/// a `due` part.
async fn invoice_retention(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    release::invoice(&state, &user, id).await.map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.retention.invoiced")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/projects/{id}/retention-notice/dismiss`: `reason`.
async fn dismiss_retention_notice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map(String::as_str).unwrap_or_default();
    vat_notice::dismiss(&state, &user, id, reason)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.vat_notice.dismissed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/retentions/{id}/deductions`: `amount` (fils),
/// `reference` (the client's notice or contra-charge reference) and `reason`.
async fn record_retention_deduction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let amount = form
        .get("amount")
        .and_then(|amount| amount.trim().parse::<i64>().ok())
        .ok_or_else(|| refused(t(&lang, "construction.retention.deduction_amount")))?;
    let new = NewRetentionDeduction {
        retention_id: id,
        amount,
        reference: form.get("reference").cloned().unwrap_or_default(),
        reason: form.get("reason").cloned().unwrap_or_default(),
    };
    retention_deduction::record(&state, &user, &new)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.retention.deduction_recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/retention-deductions/{id}/accept`: above AED 25,000
/// the MD is asked and the deduction stays undecided until then.
async fn accept_retention_deduction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let key = match retention_deduction::accept(&state, &user, id)
        .await
        .map_err(answer)?
    {
        retention_deduction::Decided::Done(_) => "construction.retention.deduction_accepted",
        retention_deduction::Decided::AwaitingMd(_) => {
            "construction.retention.deduction_awaiting_md"
        }
    };
    Ok((Toast::success(t(&lang, key)), renox::HxRefresh).into_response())
}

/// `POST /construction/retention-deductions/{id}/contest`.
async fn contest_retention_deduction(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    retention_deduction::contest(&state, &user, id)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.retention.deduction_contested")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /construction/retentions/{id}/dispute`: `amount` (fils, excluding
/// VAT) and `reason`; opens the dispute on the retention invoice.
async fn dispute_retention(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let amount = form
        .get("amount")
        .and_then(|amount| amount.trim().parse::<i64>().ok())
        .ok_or_else(|| refused(t(&lang, "construction.retention.deduction_amount")))?;
    let reason = form.get("reason").map(String::as_str).unwrap_or_default();
    retention_deduction::dispute_invoiced(&state, &user, id, amount, reason)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(t(&lang, "construction.retention.disputed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The text field `name` of a form, empty when absent.
pub(crate) fn text(form: &BTreeMap<String, String>, name: &str) -> String {
    form.get(name).cloned().unwrap_or_default()
}

/// The date field `name` of a form; `None` when empty, a 422 when it is not a
/// date.
pub(crate) fn date_field(
    form: &BTreeMap<String, String>,
    name: &str,
    lang: &Lang,
) -> Result<Option<NaiveDate>> {
    match form.get(name).map(|value| value.trim()) {
        None | Some("") => Ok(None),
        Some(value) => value
            .parse::<NaiveDate>()
            .map(Some)
            .map_err(|_| refused(t(lang, "construction.bad_input"))),
    }
}

/// The amount field `name` of a form, in fils.
fn amount_field(form: &BTreeMap<String, String>, name: &str, lang: &Lang) -> Result<i64> {
    text(form, name)
        .trim()
        .parse::<i64>()
        .map_err(|_| refused(t(lang, "construction.guarantee.amount_needed")))
}

/// A toast for a step of a guarantee, refreshing the page.
fn guarantee_done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(t(lang, key)), renox::HxRefresh).into_response())
}

/// `POST /construction/projects/{id}/guarantees`: `kind` (`advance`,
/// `performance` or `retention_bond`), `bank`, `beneficiary`, `amount` (fils)
/// and `expires_on`.
async fn register_guarantee(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let kind = [
        GuaranteeKind::Advance,
        GuaranteeKind::Performance,
        GuaranteeKind::RetentionBond,
    ]
    .into_iter()
    .find(|kind| kind.as_str() == text(&form, "kind").trim())
    .ok_or_else(|| refused(t(&lang, "construction.guarantee.kind_unknown")))?;
    let new = NewGuarantee {
        project_id: id,
        kind,
        bank: text(&form, "bank"),
        beneficiary: text(&form, "beneficiary"),
        amount: amount_field(&form, "amount", &lang)?,
        expires_on: date_field(&form, "expires_on", &lang)?,
    };
    guarantee::register(&state, &user, &new)
        .await
        .map_err(answer)?;
    guarantee_done(&lang, "construction.guarantee.registered")
}

/// `POST /construction/guarantees/{id}/issue`: `document`, the bank's
/// guarantee.
async fn issue_guarantee(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    guarantee::issue(&state, &user, id, &text(&form, "document"))
        .await
        .map_err(answer)?;
    guarantee_done(&lang, "construction.guarantee.issued")
}

/// `POST /construction/guarantees/{id}/reduce`: the new `amount` (fils) and
/// `document`, the bank's amendment or the beneficiary's consent.
async fn reduce_guarantee(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let amount = amount_field(&form, "amount", &lang)?;
    guarantee::reduce(&state, &user, id, amount, &text(&form, "document"))
        .await
        .map_err(answer)?;
    guarantee_done(&lang, "construction.guarantee.reduced")
}

/// `POST /construction/guarantees/{id}/extend`: the new `expires_on` and
/// `document`, the bank's amendment.
async fn extend_guarantee(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let expires_on = date_field(&form, "expires_on", &lang)?
        .ok_or_else(|| refused(t(&lang, "construction.guarantee.expiry_needed")))?;
    guarantee::extend(&state, &user, id, expires_on, &text(&form, "document"))
        .await
        .map_err(answer)?;
    guarantee_done(&lang, "construction.guarantee.extended")
}

/// `POST /construction/guarantees/{id}/release`: `document`, the returned
/// original or the bank's cancellation letter.
async fn release_guarantee(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    guarantee::release(&state, &user, id, &text(&form, "document"))
        .await
        .map_err(answer)?;
    guarantee_done(&lang, "construction.guarantee.released")
}

/// `POST /construction/guarantees/{id}/call`: the `amount` demanded (fils) and
/// `document`, the beneficiary's demand notice.
async fn call_guarantee(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let amount = amount_field(&form, "amount", &lang)?;
    call::record(&state, &user, id, amount, &text(&form, "document"))
        .await
        .map_err(answer)?;
    guarantee_done(&lang, "construction.guarantee.called")
}

/// `POST /construction/guarantees/{id}/settle-call`: the `amount` the bank
/// paid (fils; 0 for a call withdrawn before payment) and `document`, the
/// bank's debit advice or the beneficiary's withdrawal letter.
async fn settle_guarantee_call(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let paid = amount_field(&form, "amount", &lang)?;
    call::settle(&state, &user, id, paid, &text(&form, "document"))
        .await
        .map_err(answer)?;
    guarantee_done(&lang, "construction.guarantee.call_settled")
}

/// `POST /construction/guarantee-calls/{id}/dispute`: the project manager
/// marks the call disputed, so its paid amount is a claim in the final
/// account.
async fn dispute_guarantee_call(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    call::dispute(&state, &user, id).await.map_err(answer)?;
    guarantee_done(&lang, "construction.guarantee.call_disputed")
}
