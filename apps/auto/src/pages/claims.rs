//! The insurance claim pages (08 §4.7, `AUT.INS.01` .. `AUT.INS.04`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/claims` | `auto.claims` | `insurance.manage` or `invoices.issue` |
//! | `POST /auto/claims` | `auto.claim_create` | `insurance.manage` |
//! | `GET /auto/claims/{id}` | `auto.claim` | `insurance.manage` or `invoices.issue` |
//! | `POST /auto/claims/{id}/police-report` | `auto.claim_police` | `insurance.manage` |
//! | `POST /auto/claims/{id}/survey` | `auto.claim_survey` | `insurance.manage` |
//! | `POST /auto/claims/{id}/survey-request` | `auto.claim_survey_request` | `insurance.manage` |
//! | `POST /auto/claims/{id}/submit` | `auto.claim_submit` | `insurance.manage` |
//! | `POST /auto/claims/{id}/approve` | `auto.claim_approve` | `insurance.manage` |
//! | `POST /auto/claims/{id}/reject` | `auto.claim_reject` | `insurance.manage` |
//! | `POST /auto/claims/{id}/supplementary` | `auto.claim_supplementary` | `job_cards.advise` |
//! | `POST /auto/claims/{id}/supplementary/approve` | `auto.claim_supp_approve` | `insurance.manage` |
//! | `POST /auto/claims/{id}/supplementary/decline` | `auto.claim_supp_decline` | `insurance.manage` |
//! | `POST /auto/claims/{id}/invoice` | `auto.claim_invoice` | `invoices.issue` |
//! | `POST /auto/claims/{id}/total-loss` | `auto.claim_total_loss` | `insurance.manage` |
//! | `POST /auto/claims/{id}/charges` | `auto.claim_charges` | `insurance.manage` |
//! | `POST /auto/claims/{id}/cancel` | `auto.claim_cancel` | `insurance.manage` |
//! | `POST /auto/claims/{id}/void` | `auto.claim_void` | `insurance.manage` |
//! | `GET /auto/claim-vehicle-options` | `auto.claim_vehicle_options` | `insurance.manage` |
//! | `GET /auto/claim-job-options` | `auto.claim_job_options` | `insurance.manage` |
//!
//! The insurer, the vehicle and the job card are picked from lists, never
//! typed as ids; the insurer's answer is recorded with an amount per line of
//! the estimate that was sent.

use std::collections::{BTreeMap, HashMap};

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use super::lists;
use super::{
    Texts, invalid, parse_money, parse_number, parse_optional_date, parse_optional_id, party_name,
    require, text,
};
use crate::claim::{ClaimStatus, InsuranceClaim, RepairType};
use crate::claim_approval::{ApprovalKind, ApprovalOutcome, ClaimApproval};
use crate::claims::{self, Answer, NewClaim, Withdrawal};
use crate::insurer::Insurer;
use crate::insurers;
use crate::job_card::JobCard;
use crate::money_text;
use crate::permissions;
use crate::texts;
use crate::vehicle::Vehicle;
use crate::vehicles;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/claims", index)
        .name("auto.claims")
        .post("/auto/claims", create)
        .name("auto.claim_create")
        .get("/auto/claim-vehicle-options", vehicle_options)
        .name("auto.claim_vehicle_options")
        .get("/auto/claim-job-options", job_options)
        .name("auto.claim_job_options")
        .get("/auto/claims/{id}", show)
        .name("auto.claim")
        .post("/auto/claims/{id}/police-report", police_report)
        .name("auto.claim_police")
        .post("/auto/claims/{id}/survey", survey)
        .name("auto.claim_survey")
        .post("/auto/claims/{id}/survey-request", survey_request)
        .name("auto.claim_survey_request")
        .post("/auto/claims/{id}/submit", submit)
        .name("auto.claim_submit")
        .post("/auto/claims/{id}/approve", approve)
        .name("auto.claim_approve")
        .post("/auto/claims/{id}/reject", reject)
        .name("auto.claim_reject")
        .post("/auto/claims/{id}/supplementary", supplementary)
        .name("auto.claim_supplementary")
        .post("/auto/claims/{id}/supplementary/approve", supp_approve)
        .name("auto.claim_supp_approve")
        .post("/auto/claims/{id}/supplementary/decline", supp_decline)
        .name("auto.claim_supp_decline")
        .post("/auto/claims/{id}/invoice", invoice)
        .name("auto.claim_invoice")
        .post("/auto/claims/{id}/total-loss", total_loss)
        .name("auto.claim_total_loss")
        .post("/auto/claims/{id}/charges", charges)
        .name("auto.claim_charges")
        .post("/auto/claims/{id}/cancel", cancel)
        .name("auto.claim_cancel")
        .post("/auto/claims/{id}/void", void)
        .name("auto.claim_void")
        .require_auth()
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: ClaimStatus) -> String {
    text(lang, &format!("claims.status.{}", status.as_str()))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: ClaimStatus) -> &'static str {
    match status {
        ClaimStatus::Approved | ClaimStatus::RepairComplete | ClaimStatus::Settled => "success",
        ClaimStatus::SupplementaryPending | ClaimStatus::TotalLoss => "warning",
        ClaimStatus::Rejected => "error",
        ClaimStatus::Registered
        | ClaimStatus::AwaitingSurvey
        | ClaimStatus::EstimateSubmitted
        | ClaimStatus::Invoiced
        | ClaimStatus::Cancelled
        | ClaimStatus::Voided => "info",
    }
}

/// Whether `user` may see the claims.
fn may_view(user: &AuthUser) -> bool {
    user.has_permission(permissions::INSURANCE_MANAGE)
        || user.has_permission(permissions::INVOICES_ISSUE)
}

/// Success toast and a refresh of the page.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// The repair types as options, `[stored name, name]`.
fn repair_options(lang: &Lang) -> Vec<[String; 2]> {
    [RepairType::NonAgency, RepairType::Agency]
        .into_iter()
        .map(|kind| {
            [
                kind.as_str().to_owned(),
                text(lang, &format!("claims.repair.{}", kind.as_str())),
            ]
        })
        .collect()
}

/// The insurers that take new claims as options, `[id, name]`.
async fn insurer_options(state: &AppState, lang: &Lang) -> Result<Vec<[String; 2]>> {
    let mut options = Vec::new();
    for insurer in insurers::all(&state.db).await? {
        if insurer.active {
            options.push([
                insurer.id.to_string(),
                party_name(state, &lang.locale, insurer.party_id).await,
            ]);
        }
    }
    Ok(options)
}

/// The badge tone of a claim's status in the grid.
fn grid_tone(status: ClaimStatus) -> &'static str {
    match status_kind(status) {
        "error" => "danger",
        other => other,
    }
}

/// The claims grid: the number first and frozen, the insurer, the insurer's
/// claim number, the policy (searched, hidden), the plate, the accident day,
/// what the insurer approved in dirhams and the status in words.
pub(crate) fn grid(t: lists::Text<'_>) -> Grid {
    let statuses = lists::options(
        t,
        "claims.status",
        ClaimStatus::ALL.iter().map(|status| status.as_str()),
    );
    let tones: Vec<(&str, &str)> = ClaimStatus::ALL
        .iter()
        .map(|status| (status.as_str(), grid_tone(*status)))
        .collect();
    lists::baseline(Grid::new("auto-claims"))
        .title(&t("claims.title"))
        .column(
            Column::text("number", &t("claims.number"))
                .computed(&lists::numbered_sql("CLM-", 5))
                .frozen()
                .mobile()
                .searchable()
                .link("/auto/claims/{id}"),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&tones)
                .width("11rem")
                .mobile(),
        )
        .column(Column::custom("insurer", &t("claims.insurer")).mobile())
        .column(
            Column::text("claim_no", &t("claims.claim_no"))
                .searchable()
                .copyable(),
        )
        .column(
            Column::text("policy_no", &t("claims.policy_no"))
                .searchable()
                .hidden(),
        )
        .column(
            Column::text("plate", &t("vehicles.plate"))
                .computed(&lists::plate_sql("vehicle_id"))
                .searchable(),
        )
        .column(Column::date("accident_date", &t("claims.accident")))
        .column(
            lists::money("approved_amount", &t("claims.approved"))
                .mobile()
                .summary(Summary::Sum),
        )
        .sort_by("-id")
        .row_url("/auto/claims/{id}")
        .cards_on_mobile()
        .empty_state(&t("claims.empty"), Some(&t("claims.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/claims.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    claims: GridPage<InsuranceClaim>,
    can_register: bool,
    insurer_options: Vec<[String; 2]>,
    repair_options: Vec<[String; 2]>,
}

/// `GET /auto/claims`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !may_view(&user) {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission(permissions::INSURANCE_MANAGE);
    let grid = lists::exporting(grid(&lists::texts_in(&lang)), can_export);
    if let Some(file) = lists::export_file(&grid, can_export, InsuranceClaim::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(InsuranceClaim::query(), &request).await?;
    let insurer_ids: Vec<i64> = page.items().iter().map(|claim| claim.insurer_id).collect();
    let mut party_of_insurer = HashMap::new();
    for insurer_id in insurer_ids {
        if party_of_insurer.contains_key(&insurer_id) {
            continue;
        }
        if let Some(insurer) = Insurer::find(&state.db, insurer_id).await? {
            party_of_insurer.insert(insurer_id, insurer.party_id);
        }
    }
    let names = lists::party_names(&state, &lang, party_of_insurer.values().copied()).await;
    let claims = lists::with_currency(page, |claim| {
        let name = party_of_insurer
            .get(&claim.insurer_id)
            .and_then(|party| names.get(party))
            .cloned()
            .unwrap_or_default();
        json!({ "insurer": name })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        claims,
        can_register: user.has_permission(permissions::INSURANCE_MANAGE),
        insurer_options: insurer_options(&state, &lang).await?,
        repair_options: repair_options(&lang),
    })
    .into_response())
}

/// `GET /auto/claim-vehicle-options`: the vehicles the picker offers.
async fn vehicle_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let found = if query.is_lookup() {
        let mut found = Vec::new();
        for id in query.values_as::<i64>() {
            found.extend(Vehicle::find(&state.db, id).await?);
        }
        found
    } else {
        let mut all = vehicles::search(&state.db, &query.q).await?;
        all.truncate(20);
        all
    };
    Ok(Json(
        found
            .into_iter()
            .map(|vehicle| renox::select::SelectOption::new(vehicle.id, vehicle.title()))
            .collect(),
    ))
}

/// `GET /auto/claim-job-options`: the open job cards no claim funds.
async fn job_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let found = if query.is_lookup() {
        let mut found = Vec::new();
        for id in query.values_as::<i64>() {
            found.extend(JobCard::find(&state.db, id).await?);
        }
        found
    } else {
        claims::linkable_job_cards(&state.db, None, &query.q).await?
    };
    let mut options = Vec::new();
    for card in found {
        let plate = Vehicle::find(&state.db, card.vehicle_id)
            .await?
            .map_or_else(String::new, |v| v.plate());
        options.push(renox::select::SelectOption::new(
            card.id,
            format!("{} · {plate}", card.number()),
        ));
    }
    Ok(Json(options))
}

/// The form that registers a claim.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ClaimForm {
    /// The insurer, picked from the list.
    #[serde(default)]
    pub insurer: String,
    /// The vehicle, picked from the register.
    #[serde(default)]
    pub vehicle: String,
    /// The job card the claim funds, picked from the open ones.
    #[serde(default)]
    pub job_card: String,
    /// The policy number.
    #[serde(default)]
    pub policy_no: String,
    /// The insurer's claim number.
    #[serde(default)]
    pub claim_no: String,
    /// The day of the accident.
    #[serde(default)]
    pub accident_date: String,
    /// The police report number.
    #[serde(default)]
    pub police_report_no: String,
    /// The date of the police report.
    #[serde(default)]
    pub police_report_date: String,
    /// `agency` or `non_agency`.
    #[serde(default)]
    pub repair_type: String,
    /// The excess in dirhams.
    #[serde(default)]
    pub excess: String,
}

/// `POST /auto/claims`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<ClaimForm>,
) -> Result<Response> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let accident = parse_optional_date(&form.accident_date)?
        .ok_or_else(|| invalid(&texts::english("auto.claims.problem.accident_date", &[])))?;
    let mut new = NewClaim::new(
        parse_number(&form.insurer)?,
        parse_number(&form.vehicle)?,
        &form.policy_no,
        &form.claim_no,
        accident,
    )
    .excess(parse_money(&form.excess)?);
    if let Some(job) = parse_optional_id(&form.job_card)? {
        new = new.job_card(job);
    }
    if !form.police_report_no.trim().is_empty() {
        new = new.police_report(
            &form.police_report_no,
            parse_optional_date(&form.police_report_date)?,
        );
    }
    if form.repair_type == RepairType::Agency.as_str() {
        new = new.agency();
    }
    let claim = claims::register(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "claims.registered")),
        renox::HxRedirect(format!("/auto/claims/{}", claim.id)),
    )
        .into_response())
}

/// One request to the insurer and its answer.
#[derive(Serialize, Debug, Clone)]
pub struct ApprovalRow {
    /// Estimate, supplementary or charges.
    pub kind: String,
    /// 1, 2 ...
    pub round: i64,
    /// What was asked, with the findings of a supplementary.
    pub asked: String,
    /// The answer in words.
    pub outcome: String,
    /// The badge kind.
    pub outcome_kind: &'static str,
    /// The insurer's reference.
    pub reference: String,
    /// What the insurer approved.
    pub approved: String,
    /// The written approval.
    pub document: String,
    /// The insurer's reason when it declined.
    pub reason: String,
}

/// A line of the request waiting for an answer, with the amount to approve.
#[derive(Serialize, Debug, Clone)]
pub struct LineChoice {
    /// The form field.
    pub field: String,
    /// What the line is.
    pub description: String,
    /// Its price.
    pub price: String,
    /// The amount to start from, as typed: its price.
    pub value: String,
}

/// Dirhams as typed in a form: `250.00`.
fn typed_money(fils: i64) -> String {
    format!("{}.{:02}", fils / 100, fils % 100)
}

fn choices(lines: &[claims::RequestLine]) -> Vec<LineChoice> {
    lines
        .iter()
        .map(|line| LineChoice {
            field: format!("line_{}_{}", line.kind, line.line_id),
            description: line.description.clone(),
            price: money_text(line.price),
            value: typed_money(line.price),
        })
        .collect()
}

/// What the page of one claim needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/claim.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    status: String,
    status_kind: &'static str,
    insurer: String,
    customer: String,
    plate: String,
    vehicle_id: i64,
    job_card_id: Option<i64>,
    job_card: String,
    policy_no: String,
    claim_no: String,
    accident: String,
    police_report: String,
    police_document: String,
    documents_link: String,
    repair: String,
    excess: String,
    estimate: String,
    approved: String,
    rate_card: String,
    rate_card_url: String,
    closed_reason: String,
    total_loss: String,
    withdrawal: String,
    withdrawal_choice: String,
    unlinked_job_card: String,
    payer_bill: String,
    payer_bill_url: String,
    customer_invoice: String,
    customer_invoice_url: String,
    approvals: Vec<ApprovalRow>,
    estimate_lines: Vec<LineChoice>,
    supplementary_lines: Vec<LineChoice>,
    charge_lines: Vec<LineChoice>,
    can_police: bool,
    can_survey: bool,
    can_survey_request: bool,
    can_charges: bool,
    can_submit: bool,
    can_approve: bool,
    can_reject: bool,
    can_supplementary: bool,
    can_supp_answer: bool,
    can_invoice: bool,
    can_total_loss: bool,
    can_cancel: bool,
    can_void: bool,
    raw_police_report: String,
    raw_police_date: String,
    raw_excess: String,
}

/// The requests of a claim as rows.
fn approval_rows(lang: &Lang, approvals: &[ClaimApproval]) -> Vec<ApprovalRow> {
    approvals
        .iter()
        .map(|a| {
            let kind = text(lang, &format!("claims.kind.{}", a.kind.as_str()));
            let asked = match &a.findings {
                Some(findings) => format!("{} · {findings}", money_text(a.estimate_amount)),
                None => money_text(a.estimate_amount),
            };
            ApprovalRow {
                kind,
                round: a.round_no,
                asked,
                outcome: text(lang, &format!("claims.outcome.{}", a.outcome.as_str())),
                outcome_kind: match a.outcome {
                    ApprovalOutcome::Pending => "warning",
                    ApprovalOutcome::Approved => "success",
                    ApprovalOutcome::Declined => "error",
                },
                reference: a.reference.clone().unwrap_or_else(|| text(lang, "none")),
                approved: money_text(a.approved_amount),
                document: a.document.clone().unwrap_or_else(|| text(lang, "none")),
                reason: a.reason.clone().unwrap_or_else(|| text(lang, "none")),
            }
        })
        .collect()
}

/// `GET /auto/claims/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    if !may_view(&user) {
        return Err(Error::Forbidden);
    }
    let claim = InsuranceClaim::find_or_404(&state.db, id).await?;
    let insurer = Insurer::find_or_404(&state.db, claim.insurer_id).await?;
    let vehicle = Vehicle::find_or_404(&state.db, claim.vehicle_id).await?;
    let none = text(&lang, "none");
    let approvals = ClaimApproval::of_claim(&state.db, claim.id).await?;
    let manage = user.has_permission(permissions::INSURANCE_MANAGE);
    let status = claim.status;
    let open = matches!(
        status,
        ClaimStatus::Registered
            | ClaimStatus::AwaitingSurvey
            | ClaimStatus::EstimateSubmitted
            | ClaimStatus::Approved
            | ClaimStatus::SupplementaryPending
    );
    let job_card = match claim.job_card_id {
        Some(job_id) => JobCard::find(&state.db, job_id)
            .await?
            .map_or_else(|| none.clone(), |card| card.number()),
        None => none.clone(),
    };
    let police_report = match (&claim.police_report_no, claim.police_report_date) {
        (Some(no), Some(date)) => format!("{no} · {date}"),
        (Some(no), None) => no.clone(),
        _ => text(&lang, "claims.police_missing"),
    };
    let police_document = match crate::documents::police_verdict(
        &state,
        crate::documents::GUARDED_CLAIM_APPROVE,
        claim.id,
        None,
    )
    .await?
    {
        crate::documents::PoliceVerdict::Valid => text(&lang, "police.attached"),
        crate::documents::PoliceVerdict::Refused(words) => words,
    };
    let subject = claim.subject();
    let documents_link = format!(
        "/documents/of/{}/{}",
        subject.subject_type, subject.subject_id
    );
    let rate_card = match (claim.rate_card_id, claim.rate_card_version) {
        (Some(_), Some(version)) => texts::t_with(
            &lang,
            "auto.claims.rate_card_version",
            &[("version", &version)],
        ),
        _ => none.clone(),
    };
    let rate_card_url = match claim.rate_card_id {
        Some(card) if user.has_permission("rate_cards.view") => format!("/rate-cards/{card}"),
        _ => String::new(),
    };
    let payer_bill = match claim.payer_bill_id {
        Some(id) => super::payer_bill_ref(&state, &user, &lang, id).await,
        None => super::RecordRef::default(),
    };
    let customer_invoice = match claim.customer_invoice_id {
        Some(id) => super::invoice_ref(&state, &user, &lang, id).await,
        None => super::RecordRef::default(),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: claim.id,
        number: claim.number(),
        status: status_text(&lang, status),
        status_kind: status_kind(status),
        insurer: party_name(&state, &lang.locale, insurer.party_id).await,
        customer: party_name(&state, &lang.locale, claim.customer_party_id).await,
        plate: vehicle.plate(),
        vehicle_id: vehicle.id,
        job_card_id: claim.job_card_id,
        job_card,
        policy_no: claim.policy_no.clone(),
        claim_no: claim.claim_no.clone(),
        accident: claim.accident_date.to_string(),
        police_report,
        police_document,
        documents_link,
        repair: text(
            &lang,
            &format!("claims.repair.{}", claim.repair_type.as_str()),
        ),
        excess: money_text(claim.excess),
        estimate: money_text(claim.estimate_amount),
        approved: money_text(claim.approved_amount),
        rate_card,
        rate_card_url,
        closed_reason: claim.closed_reason.clone().unwrap_or_default(),
        total_loss: claim
            .total_loss_notice
            .clone()
            .unwrap_or_else(|| none.clone()),
        withdrawal: claim
            .withdrawal_document
            .clone()
            .unwrap_or_else(|| none.clone()),
        withdrawal_choice: match claim.withdrawal_choice.as_deref() {
            Some(choice) => text(&lang, &format!("claims.choice.{choice}")),
            None => none.clone(),
        },
        unlinked_job_card: match claim.unlinked_job_card_id {
            Some(job_id) => JobCard::find(&state.db, job_id)
                .await?
                .map_or_else(|| none.clone(), |card| card.number()),
            None => none.clone(),
        },
        payer_bill: if payer_bill.label.is_empty() {
            none.clone()
        } else {
            payer_bill.label
        },
        payer_bill_url: payer_bill.url,
        customer_invoice: if customer_invoice.label.is_empty() {
            none.clone()
        } else {
            customer_invoice.label
        },
        customer_invoice_url: customer_invoice.url,
        approvals: approval_rows(&lang, &approvals),
        estimate_lines: choices(
            &claims::request_lines(&state.db, claim.id, ApprovalKind::Estimate).await?,
        ),
        supplementary_lines: choices(
            &claims::request_lines(&state.db, claim.id, ApprovalKind::Supplementary).await?,
        ),
        charge_lines: choices(
            &claims::request_lines(&state.db, claim.id, ApprovalKind::Charges).await?,
        ),
        can_police: manage && !status.is_closed(),
        can_survey: manage && status == ClaimStatus::Registered,
        can_survey_request: manage && !status.is_closed(),
        can_charges: manage
            && status == ClaimStatus::TotalLoss
            && ClaimApproval::pending(&state.db, claim.id, ApprovalKind::Charges)
                .await?
                .is_some(),
        can_submit: manage
            && matches!(
                status,
                ClaimStatus::Registered | ClaimStatus::AwaitingSurvey
            ),
        can_approve: manage && status == ClaimStatus::EstimateSubmitted,
        can_reject: manage && status == ClaimStatus::EstimateSubmitted,
        can_supplementary: user.has_permission(permissions::JOB_CARDS_ADVISE)
            && status == ClaimStatus::Approved,
        can_supp_answer: manage && status == ClaimStatus::SupplementaryPending,
        can_invoice: user.has_permission(permissions::INVOICES_ISSUE)
            && matches!(status, ClaimStatus::RepairComplete | ClaimStatus::TotalLoss),
        can_total_loss: manage
            && matches!(
                status,
                ClaimStatus::AwaitingSurvey
                    | ClaimStatus::EstimateSubmitted
                    | ClaimStatus::Approved
                    | ClaimStatus::SupplementaryPending
            ),
        can_cancel: manage && open,
        can_void: manage && status == ClaimStatus::Registered,
        raw_police_report: claim.police_report_no.clone().unwrap_or_default(),
        raw_police_date: claim
            .police_report_date
            .map(|d| d.to_string())
            .unwrap_or_default(),
        raw_excess: typed_money(claim.excess),
    }))
}

/// The form of a reason.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReasonForm {
    /// Why.
    #[serde(default)]
    pub reason: String,
    /// The document that goes with it: the customer's written withdrawal.
    #[serde(default)]
    pub document: String,
    /// What the customer chose when work had started: `continue_privately` or
    /// `stop`.
    #[serde(default)]
    pub choice: String,
}

/// The form of the police report.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct PoliceForm {
    /// The report number.
    #[serde(default)]
    pub number: String,
    /// The report date.
    #[serde(default)]
    pub date: String,
}

/// `POST /auto/claims/{id}/police-report`.
async fn police_report(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<PoliceForm>,
) -> Result<Response> {
    claims::record_police_report(
        &state,
        &user,
        id,
        &form.number,
        parse_optional_date(&form.date)?,
    )
    .await?;
    done(&lang, "claims.police_saved")
}

/// `POST /auto/claims/{id}/survey-request`.
async fn survey_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    claims::send_survey_request(&state, &user, id).await?;
    done(&lang, "claims.survey_requested")
}

/// `POST /auto/claims/{id}/survey`.
async fn survey(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    claims::book_survey(&state, &user, id).await?;
    done(&lang, "claims.survey_booked")
}

/// `POST /auto/claims/{id}/submit`.
async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    claims::submit_estimate(&state, &user, id).await?;
    done(&lang, "claims.submitted")
}

/// The insurer's answer from a form: the `reference`, the `document`, the
/// `excess` and, per line of the request, `line_<kind>_<id>` with the amount
/// approved (blank: not approved).
async fn answer_of(
    state: &AppState,
    claim_id: i64,
    kind: ApprovalKind,
    form: &BTreeMap<String, String>,
) -> Result<Answer> {
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let mut answer = Answer::new(get("reference"), get("document"));
    if !get("excess").is_empty() {
        answer = answer.excess(parse_money(get("excess"))?);
    }
    for line in claims::request_lines(&state.db, claim_id, kind).await? {
        let amount = parse_money(get(&format!("line_{}_{}", line.kind, line.line_id)))?;
        if amount == 0 {
            continue;
        }
        answer = match line.kind {
            "labour" => answer.labour(line.line_id, amount),
            "charge" => answer.charge(amount),
            _ => answer.part(line.line_id, amount),
        };
    }
    Ok(answer)
}

/// `POST /auto/claims/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let answer = answer_of(&state, id, ApprovalKind::Estimate, &form).await?;
    claims::approve(&state, &user, id, answer).await?;
    done(&lang, "claims.approved_done")
}

/// `POST /auto/claims/{id}/reject`.
async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    claims::reject(&state, &user, id, &form.reason).await?;
    done(&lang, "claims.rejected")
}

/// The form of the findings of a supplementary.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct FindingsForm {
    /// What the technician found.
    #[serde(default)]
    pub findings: String,
}

/// `POST /auto/claims/{id}/supplementary`.
async fn supplementary(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<FindingsForm>,
) -> Result<Response> {
    claims::request_supplementary(&state, &user, id, &form.findings).await?;
    done(&lang, "claims.supplementary_requested")
}

/// `POST /auto/claims/{id}/supplementary/approve`.
async fn supp_approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let answer = answer_of(&state, id, ApprovalKind::Supplementary, &form).await?;
    claims::approve_supplementary(&state, &user, id, answer).await?;
    done(&lang, "claims.supplementary_approved")
}

/// `POST /auto/claims/{id}/supplementary/decline`.
async fn supp_decline(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    claims::decline_supplementary(&state, &user, id, &form.reason).await?;
    done(&lang, "claims.supplementary_declined")
}

/// `POST /auto/claims/{id}/invoice`.
async fn invoice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    claims::invoice(&state, &user, id).await?;
    done(&lang, "claims.invoiced")
}

/// `POST /auto/claims/{id}/total-loss`.
async fn total_loss(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    claims::declare_total_loss(&state, &user, id, &form.document).await?;
    done(&lang, "claims.total_loss_declared")
}

/// `POST /auto/claims/{id}/charges`: the insurer's approval of the charges of
/// a total loss, with the amount approved for each.
async fn charges(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    require(&user, permissions::INSURANCE_MANAGE)?;
    let answer = answer_of(&state, id, ApprovalKind::Charges, &form).await?;
    claims::approve_charges(&state, &user, id, answer).await?;
    done(&lang, "claims.charges_approved")
}

/// `POST /auto/claims/{id}/cancel`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    let mut request = Withdrawal::new(&form.reason);
    request.document = Some(form.document.clone()).filter(|d| !d.trim().is_empty());
    request.choice = Some(form.choice.clone()).filter(|c| !c.trim().is_empty());
    claims::withdraw(&state, &user, id, request).await?;
    done(&lang, "claims.cancelled")
}

/// `POST /auto/claims/{id}/void`.
async fn void(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    claims::void(&state, &user, id, &form.reason).await?;
    done(&lang, "claims.voided")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_has_the_claim_columns_and_every_status_in_words() {
        let grid = grid(&lists::english_text);
        let keys: Vec<&str> = grid.columns().iter().map(|c| c.key()).collect();
        assert_eq!(keys[..2], ["number", "status"]);
        assert!(keys.contains(&"approved_amount"));
        let status = grid.columns().iter().find(|c| c.key() == "status").unwrap();
        assert_eq!(status.options().len(), ClaimStatus::ALL.len());
        assert!(
            status
                .options()
                .iter()
                .any(|(value, label)| value == "supplementary_pending"
                    && label == "Supplementary pending")
        );
    }

    #[test]
    fn every_status_has_a_badge_kind_the_kit_knows() {
        for status in ClaimStatus::ALL {
            assert!(
                ["info", "success", "warning", "error"].contains(&status_kind(*status)),
                "{status:?}"
            );
        }
    }

    #[test]
    fn dirhams_are_typed_with_two_decimals() {
        assert_eq!(typed_money(125_050), "1250.50");
        assert_eq!(typed_money(5), "0.05");
    }
}
