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

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use super::lists;
use super::{
    Texts, invalid, parse_milli, parse_money, parse_number, parse_optional_date, parse_optional_id,
    party_name, party_of, require, text, ticked,
};
use crate::job_card::{JOB_CARD_SUBJECT_TYPE, JobCard};
use crate::money_text;
use crate::permissions;
use crate::supplier_claim::{SupplierAnswer, SupplierClaim, SupplierClaimStatus};
use crate::supplier_claims::{self, NewSupplierClaim, SUPPLIER_RETURN_SUBJECT_TYPE};
use crate::supplier_rules::SUPPLIER_CLAIM_TYPE;
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

/// The answer as staff read it.
fn answer_text(lang: &Lang, answer: SupplierAnswer) -> String {
    text(lang, &format!("supplier.answer.{}", answer.as_str()))
}

/// Whether `user` may see the claims: who raises them, and who decides their
/// write-off (the branch manager opens the record the approvals inbox names).
fn may_view(user: &AuthUser) -> bool {
    user.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE)
        || user.has_permission(permissions::SUPPLIER_CLAIMS_APPROVE)
}

/// The badge tone of a claim's status in the grid.
fn grid_tone(status: SupplierClaimStatus) -> &'static str {
    match status {
        SupplierClaimStatus::Raised => "warning",
        SupplierClaimStatus::Recovered => "success",
        SupplierClaimStatus::WrittenOff => "neutral",
    }
}

/// The supplier claims grid: the number first and frozen, the warranty
/// ticket, the supplier, the failed part, what is claimed in dirhams, what
/// the supplier said and the status in words; searched by number, ticket or
/// part.
pub(crate) fn grid(t: lists::Text<'_>) -> Grid {
    let statuses = lists::options(
        t,
        "supplier.status",
        SupplierClaimStatus::ALL
            .iter()
            .map(|status| status.as_str()),
    );
    let tones: Vec<(&str, &str)> = SupplierClaimStatus::ALL
        .iter()
        .map(|status| (status.as_str(), grid_tone(*status)))
        .collect();
    let answers = lists::options(
        t,
        "supplier.answer",
        SupplierAnswer::ALL.iter().map(|answer| answer.as_str()),
    );
    lists::baseline(Grid::new("auto-supplier-claims"))
        .title(&t("supplier.title"))
        .column(
            Column::text("number", &t("supplier.number"))
                .computed(&lists::numbered_sql("SWC-", 4))
                .frozen()
                .mobile()
                .searchable()
                .link("/auto/supplier-claims/{id}"),
        )
        .column(
            Column::select("status", &t("fields.status"), statuses)
                .badges(&tones)
                .width("11rem")
                .mobile(),
        )
        .column(Column::text("ticket_number", &t("supplier.ticket")).searchable())
        .column(Column::custom("supplier", &t("supplier.supplier")))
        .column(
            Column::text("part_description", &t("supplier.part"))
                .searchable()
                .mobile(),
        )
        .column(
            lists::money("claim_amount", &t("supplier.claimed"))
                .mobile()
                .summary(Summary::Sum),
        )
        .column(Column::select(
            "supplier_answer",
            &t("supplier.answer"),
            answers,
        ))
        .sort_by("-id")
        .row_url("/auto/supplier-claims/{id}")
        .cards_on_mobile()
        .empty_state(&t("supplier.empty"), Some(&t("supplier.empty_hint")))
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/supplier_claims.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    claims: GridPage<SupplierClaim>,
}

/// `GET /auto/supplier-claims`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !may_view(&user) {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE);
    let grid = lists::exporting(grid(&lists::texts_in(&lang)), can_export);
    if let Some(file) = lists::export_file(&grid, can_export, SupplierClaim::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(SupplierClaim::query(), &request).await?;
    let ids: Vec<i64> = page
        .items()
        .iter()
        .map(|claim| claim.supplier_party_id)
        .collect();
    let names = lists::party_names(&state, &lang, ids).await;
    let claims = lists::with_currency(
        page,
        |claim| json!({ "supplier": names.get(&claim.supplier_party_id).cloned().unwrap_or_default() }),
    );
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        claims,
    })
    .into_response())
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
    if !may_view(&user) {
        return Err(Error::Forbidden);
    }
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
        document: match claim.claim_document.as_deref() {
            // The letter the rule emailed is kept as the claim's own record.
            Some(sent) if sent.starts_with(&format!("{SUPPLIER_CLAIM_TYPE}:")) => texts::t_with(
                &lang,
                "auto.supplier.claim_letter",
                &[("number", &claim.number())],
            ),
            Some(named) => named.to_owned(),
            None => none.clone(),
        },
        reference: claim
            .supplier_reference
            .clone()
            .unwrap_or_else(|| none.clone()),
        answer: answer_text(&lang, claim.supplier_answer),
        answer_note: claim.supplier_answer_note.clone().unwrap_or_default(),
        return_ref: match claim.supplier_return_id {
            Some(id) => wl_kernel::documents::subject_label(
                &state.db,
                &Subject::new(SUPPLIER_RETURN_SUBJECT_TYPE, id),
            )
            .await?
            .unwrap_or_else(|| none.clone()),
            None => none.clone(),
        },
        recovery,
        write_off: claim.write_off_reason.clone().unwrap_or_default(),
        is_open: claim.is_open() && user.has_permission(permissions::SUPPLIER_CLAIMS_MANAGE),
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
    fn the_grid_has_the_claim_columns_and_every_status_and_answer_in_words() {
        let grid = grid(&lists::english_text);
        let keys: Vec<&str> = grid.columns().iter().map(|c| c.key()).collect();
        assert_eq!(keys[..2], ["number", "status"]);
        assert!(keys.contains(&"claim_amount") && keys.contains(&"supplier_answer"));
        let status = grid.columns().iter().find(|c| c.key() == "status").unwrap();
        assert_eq!(status.options().len(), SupplierClaimStatus::ALL.len());
        let answer = grid
            .columns()
            .iter()
            .find(|c| c.key() == "supplier_answer")
            .unwrap();
        assert_eq!(answer.options().len(), SupplierAnswer::ALL.len());
        assert!(
            answer
                .options()
                .iter()
                .all(|(_, label)| !label.starts_with("auto."))
        );
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
