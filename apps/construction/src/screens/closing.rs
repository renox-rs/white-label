//! `GET /construction/projects/{id}/closing` as a page: the termination, the
//! final account, what is still open and the claims the final account has to
//! settle.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;

use super::ipa::DeductionRow;
use super::{
    PROJECTS_URL, Texts, deduction_kind, ipa_url, project_kind, tx, variation_kind, variations_url,
    word,
};
use crate::call::GuaranteeCall;
use crate::deduction::IpaDeduction;
use crate::ipa::Ipa;
use crate::project::Project;
use crate::retention_deduction::RetentionDeduction;
use crate::variation::Variation;

/// The final account.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct FinalAccount {
    status: String,
    status_kind: &'static str,
    url: String,
    valuation_on: Option<NaiveDate>,
    certified_amount: Option<i64>,
    decided_amount: Option<i64>,
    decision_reference: Option<String>,
    invoice_url: Option<String>,
}

/// A deduction from the retention.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct RetentionRow {
    reference: String,
    reason: String,
    amount: i64,
    status: String,
    status_kind: &'static str,
}

/// A disputed call on a guarantee.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct CallRow {
    demand_notice: String,
    amount_paid: i64,
}

/// A variation of the final account or an unrecovered cost.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct VariationRow {
    label: String,
    status: String,
    status_kind: &'static str,
    approved_amount: Option<i64>,
    unrecovered_amount: i64,
}

impl VariationRow {
    fn of(lang: &Lang, variation: &Variation) -> Self {
        VariationRow {
            label: format!("{} rev. {}", variation.code, variation.revision),
            status: word(lang, "status", variation.status.as_str()),
            status_kind: variation_kind(variation.status),
            approved_amount: variation.approved_amount,
            unrecovered_amount: variation.unrecovered_amount,
        }
    }
}

/// A contested deduction with the IPA it is on.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ContestedRow {
    #[serde(flatten)]
    line: DeductionRow,
    ipa_url: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "construction/closing.html")]
pub(crate) struct ClosingPage {
    tx: Texts,
    title: String,
    subtitle: String,
    back_url: String,
    variations_url: String,
    status: String,
    status_kind: &'static str,
    termination_reason: Option<String>,
    effective_on: Option<NaiveDate>,
    final_account: Option<FinalAccount>,
    warnings: Vec<String>,
    deductions: Vec<ContestedRow>,
    retention_deductions: Vec<RetentionRow>,
    calls: Vec<CallRow>,
    claims: Vec<VariationRow>,
    unrecovered: Vec<VariationRow>,
}

/// Everything the closing screen reads of a project.
pub(crate) struct Closing<'a> {
    /// The project.
    pub project: &'a Project,
    /// The final account, once there is one.
    pub account: Option<&'a Ipa>,
    /// The warnings of what is still open, as texts.
    pub warnings: Vec<String>,
    /// The contested deductions of the certified IPAs.
    pub deductions: &'a [IpaDeduction],
    /// The contested deductions from the retention.
    pub retention_deductions: &'a [RetentionDeduction],
    /// The disputed guarantee calls.
    pub calls: &'a [GuaranteeCall],
    /// The variations pursued in the final account.
    pub claims: &'a [Variation],
    /// The variations whose cost nothing recovers.
    pub unrecovered: &'a [Variation],
}

/// The page of a project's closing screen.
pub(crate) fn page(lang: &Lang, closing: &Closing<'_>) -> Response {
    let project = closing.project;
    View::typed(ClosingPage {
        tx: tx(
            lang,
            &[
                "closing.back",
                "closing.termination",
                "closing.reason",
                "closing.effective_on",
                "closing.final_account",
                "closing.no_final_account",
                "closing.open_final",
                "closing.valuation_on",
                "closing.certified",
                "closing.decided",
                "closing.decision",
                "closing.invoice",
                "closing.warnings",
                "closing.nothing_open",
                "closing.deductions",
                "closing.retention_deductions",
                "closing.calls",
                "closing.demand",
                "closing.paid",
                "closing.claims",
                "closing.unrecovered",
                "closing.variation",
                "closing.unrecovered_cost",
                "closing.approved",
                "closing.ipa",
                "closing.reason_column",
                "variations.title",
                "ipa.no_value",
                "kind",
                "reference",
                "amount",
                "status",
                "none",
            ],
        ),
        title: format!("{} · {}", word(lang, "projects", "closing"), project.code),
        subtitle: project.name.clone(),
        back_url: PROJECTS_URL.to_owned(),
        variations_url: variations_url(project.id),
        status: word(lang, "status", project.status.as_str()),
        status_kind: project_kind(project.status),
        termination_reason: project
            .termination_reason
            .as_deref()
            .map(|reason| word(lang, "reason", reason)),
        effective_on: project.termination_effective_on,
        final_account: closing.account.map(|account| FinalAccount {
            status: word(lang, "status", account.status.as_str()),
            status_kind: super::ipa_kind(account.status),
            url: ipa_url(account.id),
            valuation_on: account.valuation_on,
            certified_amount: account.certified_amount,
            decided_amount: account.decided_amount,
            decision_reference: account.decision_reference.clone(),
            invoice_url: account.invoice_id.map(|id| format!("/invoices/{id}")),
        }),
        warnings: closing.warnings.clone(),
        deductions: closing
            .deductions
            .iter()
            .map(|line| ContestedRow {
                line: DeductionRow::of(lang, line),
                ipa_url: ipa_url(line.ipa_id),
            })
            .collect(),
        retention_deductions: closing
            .retention_deductions
            .iter()
            .map(|line| RetentionRow {
                reference: line.reference.clone(),
                reason: line.reason.clone(),
                amount: line.amount,
                status: word(lang, "status", line.status.as_str()),
                status_kind: deduction_kind(line.status.as_str()),
            })
            .collect(),
        calls: closing
            .calls
            .iter()
            .map(|call| CallRow {
                demand_notice: call.demand_notice.clone(),
                amount_paid: call.amount_paid.unwrap_or_default(),
            })
            .collect(),
        claims: closing
            .claims
            .iter()
            .map(|variation| VariationRow::of(lang, variation))
            .collect(),
        unrecovered: closing
            .unrecovered
            .iter()
            .map(|variation| VariationRow::of(lang, variation))
            .collect(),
    })
    .into_response()
}
