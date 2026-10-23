//! The insurance claim pages (08 §4.7, `AUT.INS.01` .. `AUT.INS.04`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/claims` | `auto.claims` | `insurance.manage` or `invoices.issue` |
//! | `POST /auto/claims` | `auto.claim_create` | `insurance.manage` |
//! | `GET /auto/claims/{id}` | `auto.claim` | `insurance.manage` or `invoices.issue` |
//! | `POST /auto/claims/{id}/police-report` | `auto.claim_police` | `insurance.manage` |
//! | `POST /auto/claims/{id}/survey` | `auto.claim_survey` | `insurance.manage` |
//! | `POST /auto/claims/{id}/submit` | `auto.claim_submit` | `insurance.manage` |
//! | `POST /auto/claims/{id}/approve` | `auto.claim_approve` | `insurance.manage` |
//! | `POST /auto/claims/{id}/reject` | `auto.claim_reject` | `insurance.manage` |
//! | `POST /auto/claims/{id}/supplementary` | `auto.claim_supplementary` | `job_cards.advise` |
//! | `POST /auto/claims/{id}/supplementary/approve` | `auto.claim_supp_approve` | `insurance.manage` |
//! | `POST /auto/claims/{id}/supplementary/decline` | `auto.claim_supp_decline` | `insurance.manage` |
//! | `POST /auto/claims/{id}/invoice` | `auto.claim_invoice` | `invoices.issue` |
//! | `POST /auto/claims/{id}/total-loss` | `auto.claim_total_loss` | `insurance.manage` |
//! | `POST /auto/claims/{id}/cancel` | `auto.claim_cancel` | `insurance.manage` |
//! | `POST /auto/claims/{id}/void` | `auto.claim_void` | `insurance.manage` |
//! | `GET /auto/claim-vehicle-options` | `auto.claim_vehicle_options` | `insurance.manage` |
//! | `GET /auto/claim-job-options` | `auto.claim_job_options` | `insurance.manage` |
//!
//! The insurer, the vehicle and the job card are picked from lists, never
//! typed as ids; the insurer's answer is recorded with an amount per line of
//! the estimate that was sent.

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    Texts, invalid, parse_money, parse_number, parse_optional_date, parse_optional_id, party_name,
    require, text,
};
use crate::claim::{ClaimStatus, InsuranceClaim, RepairType};
use crate::claim_approval::{ApprovalKind, ApprovalOutcome, ClaimApproval};
use crate::claims::{self, Answer, NewClaim};
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

/// What the list can be narrowed by.
#[derive(Deserialize, Default, Debug, Clone)]
pub struct Filter {
    /// Text in the claim number, the insurer's claim number or the policy.
    #[serde(default)]
    q: String,
    /// `open`, `invoiced` or `closed`; blank for every claim.
    #[serde(default)]
    tab: String,
}

impl Filter {
    fn href(&self) -> String {
        if self.tab.is_empty() {
            "/auto/claims".to_owned()
        } else {
            format!("/auto/claims?tab={}", self.tab)
        }
    }

    fn with_tab(&self, tab: &str) -> Filter {
        Filter {
            tab: tab.to_owned(),
            ..self.clone()
        }
    }

    /// Whether a claim in `status` belongs to the tab.
    fn takes(&self, status: ClaimStatus) -> bool {
        match self.tab.as_str() {
            "open" => !status.is_closed() && status != ClaimStatus::Invoiced,
            "invoiced" => status == ClaimStatus::Invoiced,
            "closed" => status.is_closed(),
            _ => true,
        }
    }
}

/// One claim of the list.
#[derive(Serialize, Debug, Clone)]
pub struct ClaimRow {
    /// The primary key.
    pub id: i64,
    /// `CLM-00007`.
    pub number: String,
    /// The insurer's claim number.
    pub claim_no: String,
    /// The insurer's name.
    pub insurer: String,
    /// `Dubai A 12345`.
    pub plate: String,
    /// The accident date.
    pub accident: String,
    /// What the insurer approved.
    pub approved: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
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

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/claims.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<ClaimRow>,
    tabs: Vec<(String, String)>,
    tab_current: String,
    tab: String,
    q: String,
    can_register: bool,
    insurer_options: Vec<[String; 2]>,
    repair_options: Vec<[String; 2]>,
}

/// `GET /auto/claims`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    if !may_view(&user) {
        return Err(Error::Forbidden);
    }
    let found = claims::search(&state.db, &filter.q, None).await?;
    let mut rows = Vec::new();
    for claim in found.into_iter().filter(|c| filter.takes(c.status)) {
        let insurer = Insurer::find_or_404(&state.db, claim.insurer_id).await?;
        let vehicle = Vehicle::find_or_404(&state.db, claim.vehicle_id).await?;
        rows.push(ClaimRow {
            id: claim.id,
            number: claim.number(),
            claim_no: claim.claim_no.clone(),
            insurer: party_name(&state, &lang.locale, insurer.party_id).await,
            plate: vehicle.plate(),
            accident: claim.accident_date.to_string(),
            approved: money_text(claim.approved_amount),
            status: status_text(&lang, claim.status),
            status_kind: status_kind(claim.status),
        });
    }
    let tabs = vec![
        (filter.with_tab("").href(), text(&lang, "claims.tab.all")),
        (
            filter.with_tab("open").href(),
            text(&lang, "claims.tab.open"),
        ),
        (
            filter.with_tab("invoiced").href(),
            text(&lang, "claims.tab.invoiced"),
        ),
        (
            filter.with_tab("closed").href(),
            text(&lang, "claims.tab.closed"),
        ),
    ];
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        tabs,
        tab_current: filter.href(),
        tab: filter.tab.clone(),
        q: filter.q.clone(),
        can_register: user.has_permission(permissions::INSURANCE_MANAGE),
        insurer_options: insurer_options(&state, &lang).await?,
        repair_options: repair_options(&lang),
    }))
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
    closed_reason: String,
    total_loss: String,
    withdrawal: String,
    payer_bill: String,
    customer_invoice: String,
    approvals: Vec<ApprovalRow>,
    estimate_lines: Vec<LineChoice>,
    supplementary_lines: Vec<LineChoice>,
    can_police: bool,
    can_survey: bool,
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
    let police_document =
        match crate::documents::police_verdict(&state, crate::documents::GUARDED_CLAIM_APPROVE, claim.id, None)
            .await?
        {
            crate::documents::PoliceVerdict::Valid => text(&lang, "police.attached"),
            crate::documents::PoliceVerdict::Refused(words) => words,
        };
    let subject = claim.subject();
    let documents_link = format!("/documents/of/{}/{}", subject.subject_type, subject.subject_id);
    let rate_card = match (claim.rate_card_id, claim.rate_card_version) {
        (Some(card), Some(version)) => format!("#{card} v{version}"),
        _ => none.clone(),
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
        closed_reason: claim.closed_reason.clone().unwrap_or_default(),
        total_loss: claim
            .total_loss_notice
            .clone()
            .unwrap_or_else(|| none.clone()),
        withdrawal: claim
            .withdrawal_document
            .clone()
            .unwrap_or_else(|| none.clone()),
        payer_bill: claim
            .payer_bill_id
            .map_or_else(|| none.clone(), |bill| format!("#{bill}")),
        customer_invoice: claim
            .customer_invoice_id
            .map_or_else(|| none.clone(), |invoice| format!("#{invoice}")),
        approvals: approval_rows(&lang, &approvals),
        estimate_lines: choices(
            &claims::request_lines(&state.db, claim.id, ApprovalKind::Estimate).await?,
        ),
        supplementary_lines: choices(
            &claims::request_lines(&state.db, claim.id, ApprovalKind::Supplementary).await?,
        ),
        can_police: manage && !status.is_closed(),
        can_survey: manage && status == ClaimStatus::Registered,
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

/// `POST /auto/claims/{id}/cancel`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    let document = Some(form.document.as_str()).filter(|d| !d.trim().is_empty());
    claims::cancel(&state, &user, id, &form.reason, document).await?;
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
    fn the_tabs_split_the_claims_by_where_they_stand() {
        let open = Filter {
            tab: "open".to_owned(),
            ..Filter::default()
        };
        assert!(open.takes(ClaimStatus::Approved));
        assert!(!open.takes(ClaimStatus::Invoiced));
        assert!(!open.takes(ClaimStatus::Voided));
        let closed = open.with_tab("closed");
        assert!(closed.takes(ClaimStatus::Settled));
        assert!(!closed.takes(ClaimStatus::TotalLoss));
        assert!(Filter::default().takes(ClaimStatus::Registered));
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
