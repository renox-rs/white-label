//! The claim pages (05 §4.3, §7.1 *Claim*; `CLN.CLM.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/claims` | `clinics.claims` | `claims.manage` |
//! | `GET /clinics/claims/{id}` | `clinics.claim` | `claims.manage` |
//! | `POST /clinics/claims/{id}/validate` | `clinics.claim_validate` | `claims.manage` |
//! | `POST /clinics/claims/{id}/submit` | `clinics.claim_submit` | `claims.manage` |
//! | `POST /clinics/claims/{id}/resubmit` | `clinics.claim_resubmit` | `claims.manage` |
//! | `POST /clinics/claims/{id}/remittance` | `clinics.claim_remittance` | `claims.manage` |
//!
//! The pages only read the form and call the services of
//! [`crate::claims`] and [`crate::claim_money`]; the transitions are the
//! machine's. A claim offers an action only from the states the machine
//! leaves with it.
//!
//! Amounts typed in a form are dirhams (`450` or `450.00`).

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::payer_bills::payer_bills;

use super::{Texts, invalid, parse_date, party_name, require, text, text_with};
use crate::claim::{
    AnswerKind, AttemptKind, Claim, ClaimAttempt, ClaimLine, ClaimRemittance, ClaimStatus,
};
use crate::claim_money::{self, Advice, AdviceLine, Recorded};
use crate::claims::{self, attempt_deadline, resubmission_deadline};
use crate::contract::PayerContract;
use crate::money_text;
use crate::permissions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/claims", index)
        .name("clinics.claims")
        .get("/clinics/claims/{id}", show)
        .name("clinics.claim")
        .post("/clinics/claims/{id}/validate", validate)
        .name("clinics.claim_validate")
        .post("/clinics/claims/{id}/submit", submit)
        .name("clinics.claim_submit")
        .post("/clinics/claims/{id}/resubmit", resubmit)
        .name("clinics.claim_resubmit")
        .post("/clinics/claims/{id}/remittance", remittance)
        .name("clinics.claim_remittance")
        .require_auth()
}

/// The stored name of a status.
fn status_key(status: ClaimStatus) -> &'static str {
    status.as_str()
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: ClaimStatus) -> String {
    text(lang, &format!("claim.status.{}", status_key(status)))
}

/// The badge kind of a status.
fn status_kind(status: ClaimStatus) -> &'static str {
    match status {
        ClaimStatus::Draft
        | ClaimStatus::Ready
        | ClaimStatus::Submitted
        | ClaimStatus::Resubmitted
        | ClaimStatus::WrittenOff
        | ClaimStatus::Voided => "info",
        ClaimStatus::Paid => "success",
        ClaimStatus::PartiallyPaid => "warning",
        ClaimStatus::Denied => "error",
    }
}

/// Dirhams typed in a form (`450`, `450.5`, `1,450.00`) as fils; blank is 0.
pub(crate) fn parse_aed(text: &str) -> std::result::Result<i64, Error> {
    let cleaned = text.trim().replace(',', "");
    if cleaned.is_empty() {
        return Ok(0);
    }
    let (whole, fraction) = cleaned.split_once('.').unwrap_or((cleaned.as_str(), ""));
    let bad = || invalid("amount: 450 or 450.00");
    if fraction.len() > 2 || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let whole: i64 = whole.parse().map_err(|_| bad())?;
    let fraction: i64 = format!("{fraction:0<2}").parse().map_err(|_| bad())?;
    if whole < 0 {
        return Err(bad());
    }
    whole
        .checked_mul(100)
        .and_then(|fils| fils.checked_add(fraction))
        .ok_or_else(bad)
}

/// One claim of the list.
#[derive(Serialize, Debug, Clone)]
pub struct ClaimRow {
    /// The primary key.
    pub id: i64,
    /// `CLM-0101`.
    pub number: String,
    /// The patient's name.
    pub patient: String,
    /// The payer's name.
    pub payer: String,
    /// The day of the visit.
    pub visit_date: String,
    /// What is claimed.
    pub claimed: String,
    /// What the payer paid.
    pub paid: String,
    /// What is denied and open.
    pub denied: String,
    /// The last day to send it, or a dash.
    pub deadline: String,
    /// The status.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/claims.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    claims: Vec<ClaimRow>,
}

/// `GET /clinics/claims`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::CLAIMS_MANAGE)?;
    let all = Claim::query()
        .order_by_desc(Claim::ID)
        .get(&state.db)
        .await?;
    let dash = text(&lang, "none");
    let mut rows = Vec::with_capacity(all.len());
    for claim in &all {
        let contract = match claim.contract_id {
            Some(id) => PayerContract::find(&state.db, id).await?,
            None => None,
        };
        let deadline = if claim.status.is_editable() {
            attempt_deadline(claim, contract.as_ref())
        } else {
            None
        };
        rows.push(ClaimRow {
            id: claim.id,
            number: claim.number(),
            patient: party_name(&state, &lang, claim.patient()).await,
            payer: party_name(&state, &lang, claim.payer()).await,
            visit_date: claim.visit_date.to_string(),
            claimed: money_text(claim.claimed),
            paid: money_text(claim.paid),
            denied: money_text(claim.denied),
            deadline: deadline.map_or_else(|| dash.clone(), |d| d.to_string()),
            status: status_text(&lang, claim.status),
            status_kind: status_kind(claim.status),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        claims: rows,
    }))
}

/// One line of the claim.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    /// The claim line's id (what the sheets name their fields by).
    pub id: i64,
    /// What it claims.
    pub description: String,
    /// The service code.
    pub code: String,
    /// What is claimed.
    pub claimed: String,
    /// What the payer paid.
    pub paid: String,
    /// What is denied and open.
    pub denied: String,
    /// What was taken back.
    pub recouped: String,
    /// What is still open on the line (claimed less paid), for the hint.
    pub open: String,
    /// The payer's code, or a dash.
    pub payer_code: String,
}

/// One submission attempt.
#[derive(Serialize, Debug, Clone)]
pub struct AttemptRow {
    /// The order of the attempt.
    pub number: i64,
    /// Submission or resubmission.
    pub kind: String,
    /// The channel's reference, or a dash.
    pub reference: String,
    /// The day it was sent.
    pub submitted_on: String,
    /// The channel's errors, or a dash.
    pub errors: String,
}

/// One remittance or recoupment recorded.
#[derive(Serialize, Debug, Clone)]
pub struct AnswerRow {
    /// The primary key.
    pub id: i64,
    /// A remittance or a recoupment.
    pub kind: String,
    /// The payer's reference, or a dash.
    pub reference: String,
    /// The date of the advice.
    pub date: String,
    /// What the claim had been paid after it.
    pub paid: String,
    /// What was denied and open after it.
    pub denied: String,
    /// What went to the payer credit balance, or what was taken back.
    pub amount: String,
    /// The payer's recoupment code, or a dash.
    pub code: String,
}

/// What the page of one claim needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/claim.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    patient: String,
    payer: String,
    visit_number: String,
    visit_date: String,
    status: String,
    status_kind: &'static str,
    payer_bill: String,
    payer_reference: String,
    claimed: String,
    paid: String,
    denied: String,
    resubmissions: i64,
    submission_reference: String,
    submitted_on: String,
    late_reason: String,
    deadline: String,
    held: bool,
    held_text: String,
    held_result: String,
    lines: Vec<LineRow>,
    attempts: Vec<AttemptRow>,
    answers: Vec<AnswerRow>,
    today: String,
    can_validate: bool,
    can_submit: bool,
    can_resubmit: bool,
    can_remit: bool,
}

/// `GET /clinics/claims/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::CLAIMS_MANAGE)?;
    let claim = Claim::find_or_404(&state.db, id).await?;
    let dash = text(&lang, "none");
    let contract = match claim.contract_id {
        Some(contract) => PayerContract::find(&state.db, contract).await?,
        None => None,
    };
    let deadline = match claim.status {
        ClaimStatus::Draft | ClaimStatus::Ready => attempt_deadline(&claim, contract.as_ref()),
        ClaimStatus::PartiallyPaid | ClaimStatus::Denied => {
            resubmission_deadline(&claim, contract.as_ref())
        }
        _ => None,
    };
    let lines: Vec<LineRow> = ClaimLine::of_claim(&state.db, id)
        .await?
        .iter()
        .map(|l| LineRow {
            id: l.id,
            description: l.description.clone(),
            code: l.code.clone(),
            claimed: money_text(l.claimed),
            paid: money_text(l.paid),
            denied: money_text(l.denied),
            recouped: money_text(l.recouped),
            open: money_text((l.claimed - l.paid).max(0)),
            payer_code: l.denial_code.clone().unwrap_or_else(|| dash.clone()),
        })
        .collect();
    let attempts: Vec<AttemptRow> = ClaimAttempt::of_claim(&state.db, id)
        .await?
        .iter()
        .map(|a| AttemptRow {
            number: a.number,
            kind: text(
                &lang,
                match a.kind {
                    AttemptKind::Submission => "claim.attempt.submission",
                    AttemptKind::Resubmission => "claim.attempt.resubmission",
                },
            ),
            reference: a.reference.clone().unwrap_or_else(|| dash.clone()),
            submitted_on: a.submitted_on.to_string(),
            errors: if a.errors.0.is_empty() {
                dash.clone()
            } else {
                a.errors
                    .0
                    .iter()
                    .map(|e| format!("{} {}", e.code, e.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            },
        })
        .collect();
    let answers: Vec<AnswerRow> = ClaimRemittance::of_claim(&state.db, id)
        .await?
        .iter()
        .map(|r| AnswerRow {
            id: r.id,
            kind: text(
                &lang,
                match r.kind {
                    AnswerKind::Remittance => "claim.answer.remittance",
                    AnswerKind::Recoupment => "claim.answer.recoupment",
                },
            ),
            reference: r.reference.clone().unwrap_or_else(|| dash.clone()),
            date: r.advice_date.to_string(),
            paid: money_text(r.paid),
            denied: money_text(r.denied),
            amount: money_text(match r.kind {
                AnswerKind::Remittance => r.overpayment,
                AnswerKind::Recoupment => r.recouped,
            }),
            code: r.code.clone().unwrap_or_else(|| dash.clone()),
        })
        .collect();
    let bill = payer_bills(&state)
        .find(&state.db, claim.payer_bill())
        .await?;
    let held = claim.is_held();
    let held_text = match (&claim.held_kind, claim.held_reference.as_deref()) {
        (Some(kind), reference) if held => text_with(
            &lang,
            &format!("claim.held.{kind}"),
            &[("reference", &reference.unwrap_or(""))],
        ),
        _ => String::new(),
    };
    let held_result = claim.held_result.as_deref().map_or_else(String::new, |r| {
        text_with(&lang, "claim.held.ended", &[("result", &r)])
    });
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: claim.id,
        number: claim.number(),
        patient: party_name(&state, &lang, claim.patient()).await,
        payer: party_name(&state, &lang, PartyId(claim.payer_party_id)).await,
        visit_number: format!("VIS-{:04}", claim.visit_id),
        visit_date: claim.visit_date.to_string(),
        status: status_text(&lang, claim.status),
        status_kind: status_kind(claim.status),
        payer_bill: bill
            .and_then(|b| b.number)
            .unwrap_or_else(|| dash.clone()),
        payer_reference: claim
            .payer_reference
            .clone()
            .unwrap_or_else(|| dash.clone()),
        claimed: money_text(claim.claimed),
        paid: money_text(claim.paid),
        denied: money_text(claim.denied),
        resubmissions: claim.resubmission_count,
        submission_reference: claim
            .submission_reference
            .clone()
            .unwrap_or_else(|| dash.clone()),
        submitted_on: claim
            .submitted_on
            .map_or_else(|| dash.clone(), |d| d.to_string()),
        late_reason: claim.late_reason.clone().unwrap_or_else(|| dash.clone()),
        deadline: deadline.map_or_else(|| dash.clone(), |d| d.to_string()),
        held,
        held_text,
        held_result,
        lines,
        attempts,
        answers,
        today: today().to_string(),
        can_validate: claim.status == ClaimStatus::Draft,
        can_submit: claim.status == ClaimStatus::Ready,
        can_resubmit: matches!(claim.status, ClaimStatus::PartiallyPaid | ClaimStatus::Denied),
        can_remit: claim.status.is_with_payer() && !held,
    }))
}

/// The toast and refresh that answer an action.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// The refusal of a transition as the error of the page.
fn refusal(error: TransitionError) -> Error {
    error.into_error()
}

/// The form of `validate`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ValidateForm {
    /// Why the claim is sent late, where the payer accepts late claims.
    #[serde(default)]
    pub late_reason: String,
}

/// `POST /clinics/claims/{id}/validate`.
async fn validate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ValidateForm>,
) -> Result<Response> {
    let reason = Some(form.late_reason.as_str()).filter(|r| !r.trim().is_empty());
    claims::validate(&state, &user, id, reason, now())
        .await
        .map_err(refusal)?;
    done(&lang, "claim.validated")
}

/// The form of `submit` and of the reference of `resubmit`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct SubmitForm {
    /// The reference the channel gave the submission.
    #[serde(default)]
    pub reference: String,
}

/// `POST /clinics/claims/{id}/submit`.
async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<SubmitForm>,
) -> Result<Response> {
    let reference = Some(form.reference.as_str()).filter(|r| !r.trim().is_empty());
    claims::submit(&state, &user, id, reference, now())
        .await
        .map_err(refusal)?;
    done(&lang, "claim.submitted")
}

/// The form of `resubmit`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ResubmitForm {
    /// The corrections made.
    #[serde(default)]
    pub corrections: String,
    /// The reference the channel gave the resubmission.
    #[serde(default)]
    pub reference: String,
}

/// `POST /clinics/claims/{id}/resubmit`.
async fn resubmit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ResubmitForm>,
) -> Result<Response> {
    let reference = Some(form.reference.as_str()).filter(|r| !r.trim().is_empty());
    claims::resubmit(&state, &user, id, &form.corrections, reference, now())
        .await
        .map_err(refusal)?;
    done(&lang, "claim.resubmitted")
}

/// The advice date typed in a form, today when blank.
fn advice_date(text: &str) -> std::result::Result<NaiveDate, Error> {
    Ok(parse_date(text)?.unwrap_or_else(today))
}

/// What a money action answers: recorded, or held for a second person.
fn answered(lang: &Lang, recorded: Recorded, done_key: &str, held_key: &str) -> Result<Response> {
    let key = match recorded {
        Recorded::Done(_) => done_key,
        Recorded::Held(_) => held_key,
    };
    done(lang, key)
}

/// `POST /clinics/claims/{id}/remittance`. The form has the `reference`, the
/// `date` and the `payment` of the advice and, per claim line,
/// `line_<id>_paid`, `line_<id>_denied`, `line_<id>_code` and
/// `line_<id>_reason`.
async fn remittance(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    require(&user, permissions::CLAIMS_MANAGE)?;
    let get = |key: &str| form.get(key).map_or("", |v| v.trim());
    let mut advice = Advice::new(get("reference"), advice_date(get("date"))?);
    if !get("payment").is_empty() {
        advice = advice.payment(
            get("payment")
                .parse::<i64>()
                .map_err(|_| invalid("payment: 1, 2, 3 ..."))?,
        );
    }
    for line in ClaimLine::of_claim(&state.db, id).await? {
        let paid = parse_aed(get(&format!("line_{}_paid", line.id)))?;
        let denied = parse_aed(get(&format!("line_{}_denied", line.id)))?;
        if paid == 0 && denied == 0 {
            continue;
        }
        advice = advice.line(AdviceLine::paid(line.id, paid).denied(
            denied,
            get(&format!("line_{}_code", line.id)),
            get(&format!("line_{}_reason", line.id)),
        ));
    }
    let recorded = claim_money::record_remittance(&state, &user, id, advice)
        .await
        .map_err(refusal)?;
    answered(&lang, recorded, "claim.remittance_recorded", "claim.remittance_held")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirhams_are_read_as_fils() {
        assert_eq!(parse_aed("").unwrap(), 0);
        assert_eq!(parse_aed("450").unwrap(), 45_000);
        assert_eq!(parse_aed("450.5").unwrap(), 45_050);
        assert_eq!(parse_aed("1,450.00").unwrap(), 145_000);
        assert!(parse_aed("4.505").is_err());
        assert!(parse_aed("abc").is_err());
        assert!(parse_aed("-5").is_err());
    }

    #[test]
    fn a_paid_claim_offers_no_other_action_than_the_recoupment() {
        assert!(!ClaimStatus::Paid.is_with_payer());
        assert_eq!(status_kind(ClaimStatus::Paid), "success");
        assert_eq!(status_kind(ClaimStatus::Denied), "error");
    }
}
