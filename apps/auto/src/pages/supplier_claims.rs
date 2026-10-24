//! The supplier warranty claim pages (08 §4.6 step 4, `AUT.WAR.02`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/supplier-claims` | `auto.supplier_claims` | `supplier_claims.manage` |
//! | `POST /auto/supplier-claims` | `auto.supplier_claim_create` | `supplier_claims.manage` |
//! | `GET /auto/supplier-claim-job-options` | `auto.supplier_job_options` | `supplier_claims.manage` |
//! | `GET /auto/supplier-claims/{id}` | `auto.supplier_claim` | `supplier_claims.manage` |
//! | `POST /auto/supplier-claims/{id}/document` | `auto.supplier_claim_document` | `supplier_claims.manage` |
//! | `POST /auto/supplier-claims/{id}/answer` | `auto.supplier_claim_answer` | `supplier_claims.manage` |
//! | `POST /auto/supplier-claims/{id}/return` | `auto.supplier_claim_return` | `supplier_claims.manage` |
//! | `POST /auto/supplier-claims/{id}/recover` | `auto.supplier_claim_recover` | `supplier_claims.manage` |
//! | `POST /auto/supplier-claims/{id}/write-off` | `auto.supplier_claim_write_off` | `supplier_claims.manage` |
//!
//! The supplier is picked from the parties and the job cards from a list; the
//! warranty ticket is named by its number among the warranty tickets of the
//! job card.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use super::{
    Texts, invalid, parse_milli, parse_money, parse_number, parse_optional_date, parse_optional_id,
    party_name, party_of, require, text, ticked,
};
use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard};
use crate::money_text;
use crate::permissions;
use crate::supplier_claim::{SupplierAnswer, SupplierClaim, SupplierClaimStatus};
use crate::supplier_claims::{self, NewSupplierClaim, SUPPLIER_RETURN_SUBJECT_TYPE};
use crate::texts;
use crate::vehicle::Vehicle;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/supplier-claims", index)
        .name("auto.supplier_claims")
        .post("/auto/supplier-claims", create)
        .name("auto.supplier_claim_create")
        .get("/auto/supplier-claim-job-options", job_options)
        .name("auto.supplier_job_options")
        .get("/auto/supplier-claims/{id}", show)
        .name("auto.supplier_claim")
        .post("/auto/supplier-claims/{id}/document", document)
        .name("auto.supplier_claim_document")
        .post("/auto/supplier-claims/{id}/answer", answer)
        .name("auto.supplier_claim_answer")
        .post("/auto/supplier-claims/{id}/return", link_return)
        .name("auto.supplier_claim_return")
        .post("/auto/supplier-claims/{id}/recover", recover)
        .name("auto.supplier_claim_recover")
        .post("/auto/supplier-claims/{id}/write-off", write_off)
        .name("auto.supplier_claim_write_off")
        .require_auth()
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: SupplierClaimStatus) -> String {
    text(lang, &format!("supplier.status.{}", status.as_str()))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: SupplierClaimStatus) -> &'static str {
    match status {
        SupplierClaimStatus::Raised => "warning",
        SupplierClaimStatus::Recovered => "success",
        SupplierClaimStatus::WrittenOff => "info",
    }
}

/// Success toast and a refresh of the page.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// What the list can be narrowed by.
#[derive(Deserialize, Default, Debug, Clone)]
pub struct Filter {
    /// Text in the number, the ticket number or the part.
    #[serde(default)]
    q: String,
    /// `raised` or `ended`; blank for every claim.
    #[serde(default)]
    tab: String,
}

impl Filter {
    fn href(&self) -> String {
        if self.tab.is_empty() {
            "/auto/supplier-claims".to_owned()
        } else {
            format!("/auto/supplier-claims?tab={}", self.tab)
        }
    }

    fn with_tab(&self, tab: &str) -> Filter {
        Filter {
            tab: tab.to_owned(),
            ..self.clone()
        }
    }

    /// Whether a claim in `status` belongs to the tab.
    fn takes(&self, status: SupplierClaimStatus) -> bool {
        match self.tab.as_str() {
            "raised" => status == SupplierClaimStatus::Raised,
            "ended" => status != SupplierClaimStatus::Raised,
            _ => true,
        }
    }
}

/// One claim of the list.
#[derive(Serialize, Debug, Clone)]
pub struct ClaimRow {
    /// The primary key.
    pub id: i64,
    /// `SWC-0007`.
    pub number: String,
    /// The warranty ticket.
    pub ticket: String,
    /// The supplier.
    pub supplier: String,
    /// The failed part.
    pub part: String,
    /// What is claimed.
    pub amount: String,
    /// What the supplier said, in words.
    pub answer: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// The answer as staff read it.
fn answer_text(lang: &Lang, answer: SupplierAnswer) -> String {
    text(lang, &format!("supplier.answer.{}", answer.as_str()))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/supplier_claims.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<ClaimRow>,
    status_tabs: Vec<(String, String)>,
    tab_current: String,
    tab: String,
    q: String,
}

/// `GET /auto/supplier-claims`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    require(&user, permissions::SUPPLIER_CLAIMS_MANAGE)?;
    let found = supplier_claims::search(&state.db, &filter.q, None).await?;
    let mut rows = Vec::new();
    for claim in found.into_iter().filter(|c| filter.takes(c.status)) {
        rows.push(ClaimRow {
            id: claim.id,
            number: claim.number(),
            ticket: claim.ticket_number.clone(),
            supplier: party_name(&state, &lang.locale, claim.supplier_party_id).await,
            part: claim.part_description.clone(),
            amount: money_text(claim.claim_amount),
            answer: answer_text(&lang, claim.supplier_answer),
            status: status_text(&lang, claim.status),
            status_kind: status_kind(claim.status),
        });
    }
    let status_tabs = vec![
        (filter.with_tab("").href(), text(&lang, "supplier.tab.all")),
        (
            filter.with_tab("raised").href(),
            text(&lang, "supplier.tab.raised"),
        ),
        (
            filter.with_tab("ended").href(),
            text(&lang, "supplier.tab.ended"),
        ),
    ];
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        status_tabs,
        tab_current: filter.href(),
        tab: filter.tab.clone(),
        q: filter.q.clone(),
    }))
}

/// `GET /auto/supplier-claim-job-options`: the job cards the picker offers.
async fn job_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    require(&user, permissions::SUPPLIER_CLAIMS_MANAGE)?;
    let found = if query.is_lookup() {
        let mut found = Vec::new();
        for id in query.values_as::<i64>() {
            found.extend(JobCard::find(&state.db, id).await?);
        }
        found
    } else {
        let wanted = query.q.trim().to_lowercase();
        JobCard::query()
            .order_by_desc(JobCard::ID)
            .get(&state.db)
            .await?
            .into_iter()
            .filter(|card| card.number().to_lowercase().contains(&wanted))
            .take(20)
            .collect()
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

/// A day typed in a form that is required.
fn required_date(value: &str, key: &str) -> Result<renox::chrono::NaiveDate> {
    parse_optional_date(value)?.ok_or_else(|| invalid(&texts::english(key, &[])))
}

/// The form that raises a claim.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ClaimForm {
    /// The job card the warranty ticket is about, picked from the list.
    #[serde(default)]
    pub job_card: String,
    /// The number of the warranty ticket.
    #[serde(default)]
    pub ticket: String,
    /// The ticket's decision is accepted.
    #[serde(default)]
    pub accepted: String,
    /// The supplier, picked from the parties.
    #[serde(default)]
    pub supplier: String,
    /// The failed part.
    #[serde(default)]
    pub part: String,
    /// The item code.
    #[serde(default)]
    pub item_code: String,
    /// The brand.
    #[serde(default)]
    pub brand: String,
    /// The quantity.
    #[serde(default)]
    pub quantity: String,
    /// What the part cost, in dirhams.
    #[serde(default)]
    pub cost: String,
    /// The day it was bought from the supplier.
    #[serde(default)]
    pub purchase_date: String,
    /// The warranty period in months.
    #[serde(default)]
    pub warranty_months: String,
    /// The warranty mileage in kilometres.
    #[serde(default)]
    pub warranty_km: String,
    /// The terms include the labour of the replacement.
    #[serde(default)]
    pub labour_included: String,
    /// The labour of the replacement, in dirhams.
    #[serde(default)]
    pub labour: String,
    /// The job card the part was fitted on, picked from the list.
    #[serde(default)]
    pub fitted_job_card: String,
    /// The job card of the replacement, picked from the list.
    #[serde(default)]
    pub replacement_job_card: String,
    /// The mileage when the part was fitted.
    #[serde(default)]
    pub fitted_odometer: String,
    /// The day it failed.
    #[serde(default)]
    pub failure_date: String,
    /// The mileage at the failure.
    #[serde(default)]
    pub failure_odometer: String,
    /// The failed part was kept.
    #[serde(default)]
    pub kept: String,
}

/// `POST /auto/supplier-claims`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<ClaimForm>,
) -> Result<Response> {
    require(&user, permissions::SUPPLIER_CLAIMS_MANAGE)?;
    let job = parse_number(&form.job_card)?;
    let about = Subject::new(JOB_CARD_SUBJECT_TYPE, job);
    let ticket_id = supplier_claims::ticket_by_number(&state, &about, &form.ticket).await?;
    let supplier = party_of(&state, &form.supplier).await?;
    let mut new = NewSupplierClaim::new(
        ticket_id,
        about,
        supplier,
        &form.part,
        parse_money(&form.cost)?,
        required_date(&form.purchase_date, "auto.supplier.problem.dates")?,
        parse_number(&form.warranty_months)?,
        required_date(&form.failure_date, "auto.supplier.problem.dates")?,
    );
    new.decision_accepted = ticked(&form.accepted);
    new.part_kept = ticked(&form.kept);
    if !form.quantity.trim().is_empty() {
        new.quantity_milli = parse_milli(&form.quantity)?;
    }
    if !form.item_code.trim().is_empty() || !form.brand.trim().is_empty() {
        new = new.item(&form.item_code, &form.brand);
    }
    if !form.warranty_km.trim().is_empty() {
        new = new.mileage(super::parse_km(&form.warranty_km)?);
    }
    new.fitted_job_card_id = parse_optional_id(&form.fitted_job_card)?;
    new.replacement_job_card_id = parse_optional_id(&form.replacement_job_card)?;
    if !form.fitted_odometer.trim().is_empty() {
        new.fitted_odometer = Some(super::parse_km(&form.fitted_odometer)?);
    }
    if !form.failure_odometer.trim().is_empty() {
        new.failure_odometer = Some(super::parse_km(&form.failure_odometer)?);
    }
    if ticked(&form.labour_included) {
        new.labour_included = true;
        let labour = parse_money(&form.labour)?;
        new.labour_amount = (labour > 0).then_some(labour);
    }
    let claim = supplier_claims::raise(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "supplier.raised")),
        renox::HxRedirect(format!("/auto/supplier-claims/{}", claim.id)),
    )
        .into_response())
}

/// What the page of one claim needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/supplier_claim.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    status: String,
    status_kind: &'static str,
    ticket: String,
    about: String,
    supplier: String,
    part: String,
    item: String,
    quantity: String,
    part_cost: String,
    labour: String,
    amount: String,
    purchased: String,
    terms: String,
    fitted: String,
    failure: String,
    kept: String,
    document: String,
    reference: String,
    answer: String,
    answer_note: String,
    return_ref: String,
    recovery: String,
    write_off: String,
    is_open: bool,
}

/// `GET /auto/supplier-claims/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::SUPPLIER_CLAIMS_MANAGE)?;
    let claim = SupplierClaim::find_or_404(&state.db, id).await?;
    let none = text(&lang, "none");
    let terms = match claim.warranty_km {
        Some(km) => texts::t_with(
            &lang,
            "auto.supplier.terms_km",
            &[
                ("months", &claim.warranty_months),
                ("km", &crate::number_text(km)),
            ],
        ),
        None => texts::t_with(
            &lang,
            "auto.supplier.terms",
            &[("months", &claim.warranty_months)],
        ),
    };
    let about = match claim.about() {
        Some(subject) if subject.subject_type == JOB_CARD_SUBJECT_TYPE => {
            JobCard::find(&state.db, subject.subject_id)
                .await?
                .map_or_else(|| subject.to_string(), |card| card.number())
        }
        Some(subject) => subject.to_string(),
        None => none.clone(),
    };
    let fitted = match (claim.fitted_job_card_id, claim.fitted_odometer) {
        (Some(card), Some(km)) => format!("JOB-{card:05} · {} km", crate::number_text(km)),
        (None, Some(km)) => format!("{} km", crate::number_text(km)),
        _ => text(&lang, "supplier.over_the_counter"),
    };
    let failure = match claim.failure_odometer {
        Some(km) => format!("{} · {} km", claim.failure_date, crate::number_text(km)),
        None => claim.failure_date.to_string(),
    };
    let recovery = match (&claim.recovery_basis, &claim.recovery_reference) {
        (Some(basis), Some(reference)) => format!(
            "{} · {reference}",
            text(&lang, &format!("supplier.basis.{basis}"))
        ),
        (Some(basis), None) => text(&lang, &format!("supplier.basis.{basis}")),
        _ => none.clone(),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: claim.id,
        number: claim.number(),
        status: status_text(&lang, claim.status),
        status_kind: status_kind(claim.status),
        ticket: claim.ticket_number.clone(),
        about,
        supplier: party_name(&state, &lang.locale, claim.supplier_party_id).await,
        part: claim.part_description.clone(),
        item: [claim.item_code.clone(), claim.brand.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
        quantity: crate::labour_line::hours_text(claim.quantity_milli),
        part_cost: money_text(claim.part_cost),
        labour: if claim.labour_included {
            money_text(claim.labour_amount)
        } else {
            text(&lang, "supplier.labour_not_included")
        },
        amount: money_text(claim.claim_amount),
        purchased: claim.purchase_date.to_string(),
        terms,
        fitted,
        failure,
        kept: text(
            &lang,
            if claim.part_kept {
                "supplier.kept"
            } else {
                "supplier.not_kept"
            },
        ),
        document: claim.claim_document.clone().unwrap_or_else(|| none.clone()),
        reference: claim
            .supplier_reference
            .clone()
            .unwrap_or_else(|| none.clone()),
        answer: answer_text(&lang, claim.supplier_answer),
        answer_note: claim.supplier_answer_note.clone().unwrap_or_default(),
        return_ref: claim
            .supplier_return_id
            .map_or_else(|| none.clone(), |id| format!("#{id}")),
        recovery,
        write_off: claim.write_off_reason.clone().unwrap_or_default(),
        is_open: claim.is_open(),
    }))
}

/// The form of the claim document.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct DocumentForm {
    /// The claim document sent to the supplier.
    #[serde(default)]
    pub document: String,
    /// The supplier's reference.
    #[serde(default)]
    pub reference: String,
}

/// `POST /auto/supplier-claims/{id}/document`.
async fn document(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<DocumentForm>,
) -> Result<Response> {
    let reference = Some(form.reference.as_str()).filter(|r| !r.trim().is_empty());
    supplier_claims::record_document(&state, &user, id, &form.document, reference).await?;
    done(&lang, "supplier.document_saved")
}

/// The form of the supplier's answer.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct AnswerForm {
    /// `accepted` or `rejected`.
    #[serde(default)]
    pub answer: String,
    /// The supplier's reference.
    #[serde(default)]
    pub reference: String,
    /// A note with the answer.
    #[serde(default)]
    pub note: String,
}

/// `POST /auto/supplier-claims/{id}/answer`.
async fn answer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<AnswerForm>,
) -> Result<Response> {
    let answer = match form.answer.as_str() {
        "accepted" => SupplierAnswer::Accepted,
        "rejected" => SupplierAnswer::Rejected,
        _ => SupplierAnswer::Pending,
    };
    let reference = Some(form.reference.as_str()).filter(|r| !r.trim().is_empty());
    let note = Some(form.note.as_str()).filter(|n| !n.trim().is_empty());
    supplier_claims::record_answer(&state, &user, id, answer, reference, note).await?;
    done(&lang, "supplier.answer_saved")
}

/// The form of a Return to supplier.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReturnForm {
    /// The number of the Return to supplier.
    #[serde(default)]
    pub supplier_return: String,
}

/// `POST /auto/supplier-claims/{id}/return`.
async fn link_return(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReturnForm>,
) -> Result<Response> {
    let number = parse_number(&form.supplier_return)?;
    supplier_claims::link_return(
        &state,
        &user,
        id,
        &Subject::new(SUPPLIER_RETURN_SUBJECT_TYPE, number),
    )
    .await?;
    done(&lang, "supplier.return_linked")
}

/// The form of a recovery.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct RecoverForm {
    /// The supplier's credit note, or the receipt of the replacement.
    #[serde(default)]
    pub reference: String,
    /// A replacement part was received.
    #[serde(default)]
    pub replacement: String,
}

/// `POST /auto/supplier-claims/{id}/recover`.
async fn recover(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<RecoverForm>,
) -> Result<Response> {
    let reference = Some(form.reference.as_str()).filter(|r| !r.trim().is_empty());
    supplier_claims::recover(&state, &user, id, reference, ticked(&form.replacement)).await?;
    done(&lang, "supplier.recovered")
}

/// The form of a reason.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReasonForm {
    /// Why.
    #[serde(default)]
    pub reason: String,
}

/// `POST /auto/supplier-claims/{id}/write-off`.
async fn write_off(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    let asked = supplier_claims::write_off(&state, &user, id, &form.reason).await?;
    if asked.approval_request.is_some() {
        done(&lang, "supplier.writeoff_waits")
    } else {
        done(&lang, "supplier.written_off")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tabs_split_the_claims_into_raised_and_ended() {
        let raised = Filter {
            tab: "raised".to_owned(),
            ..Filter::default()
        };
        assert!(raised.takes(SupplierClaimStatus::Raised));
        assert!(!raised.takes(SupplierClaimStatus::Recovered));
        let ended = raised.with_tab("ended");
        assert!(ended.takes(SupplierClaimStatus::WrittenOff));
        assert!(!ended.takes(SupplierClaimStatus::Raised));
        assert!(Filter::default().takes(SupplierClaimStatus::Recovered));
    }

    #[test]
    fn every_status_has_a_badge_kind_the_kit_knows() {
        for status in SupplierClaimStatus::ALL {
            assert!(
                ["info", "success", "warning", "error"].contains(&status_kind(*status)),
                "{status:?}"
            );
        }
    }
}
