//! `GET /construction/ipas/{id}` as a page: the application, the amounts
//! claimed and certified, the deductions and the records that follow from it
//! (the project's closing screen and variation report, the tax invoice, the
//! credit note, the dispute and the supplier bill).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;

use super::{
    PROJECTS_URL, Texts, closing_url, deduction_kind, ipa_kind, ipa_url, tx, variations_url, word,
};
use crate::deduction::IpaDeduction;
use crate::ipa::Ipa;
use crate::project::Project;
use crate::texts::english;

/// A related record to open.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Link {
    label: String,
    url: String,
    icon: &'static str,
}

/// One deduction line.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct DeductionRow {
    kind: String,
    reference: String,
    description: String,
    amount: i64,
    status: String,
    status_kind: &'static str,
}

impl DeductionRow {
    /// The row of a line.
    pub(crate) fn of(lang: &Lang, line: &IpaDeduction) -> Self {
        DeductionRow {
            kind: word(lang, "kind", line.kind.as_str()),
            reference: line.reference.clone(),
            description: line.description.clone(),
            amount: line.amount,
            status: word(lang, "status", line.status.as_str()),
            status_kind: deduction_kind(line.status.as_str()),
        }
    }
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "construction/ipa.html")]
pub(crate) struct IpaPage {
    tx: Texts,
    title: String,
    subtitle: String,
    back_url: String,
    status: String,
    status_kind: &'static str,
    kind: String,
    valuation_on: Option<NaiveDate>,
    claimed_amount: i64,
    certified_amount: Option<i64>,
    claim_gap: Option<i64>,
    previous_certified: i64,
    previous_pending: bool,
    submitted_on: Option<NaiveDate>,
    certified_on: Option<NaiveDate>,
    certificate: Option<String>,
    late_certificate: Option<String>,
    decision_reference: Option<String>,
    decided_amount: Option<i64>,
    replaces: Option<Link>,
    links: Vec<Link>,
    deductions: Vec<DeductionRow>,
}

/// The page of `ipa` of `project`, with the IPA it `replaced` and its
/// `deductions`.
pub(crate) fn page(
    lang: &Lang,
    ipa: &Ipa,
    project: &Project,
    replaced: Option<&Ipa>,
    deductions: &[IpaDeduction],
) -> Response {
    let link = |key: &str, url: String, icon: &'static str| Link {
        label: english(key, &[]),
        url,
        icon,
    };
    let mut links = vec![
        link(
            "construction.screen.ipa.closing",
            closing_url(project.id),
            "clipboard-list",
        ),
        link(
            "construction.screen.ipa.variations",
            variations_url(project.id),
            "file-text",
        ),
    ];
    if let Some(id) = ipa.invoice_id {
        links.push(link(
            "construction.screen.ipa.invoice",
            format!("/invoices/{id}"),
            "receipt",
        ));
    }
    if let Some(id) = ipa.credit_note_id {
        links.push(link(
            "construction.screen.ipa.credit_note",
            format!("/credit-notes/{id}"),
            "file-text",
        ));
    }
    if ipa.dispute_id.is_some() {
        // A dispute is opened from its invoice's page.
        let url = ipa
            .invoice_id
            .map_or_else(|| ipa_url(ipa.id), |id| format!("/invoices/{id}"));
        links.push(link(
            "construction.screen.ipa.dispute",
            url,
            "triangle-alert",
        ));
    }
    if let Some(id) = ipa.supplier_bill_id {
        links.push(link(
            "construction.screen.ipa.supplier_bill",
            format!("/supplier-bills/{id}"),
            "file-text",
        ));
    }
    let title = if ipa.revision > 1 {
        english(
            "construction.screen.ipa.title_revision",
            &[("number", &ipa.number), ("revision", &ipa.revision)],
        )
    } else {
        english("construction.screen.ipa.title", &[("number", &ipa.number)])
    };
    View::typed(IpaPage {
        tx: tx(lang),
        title,
        subtitle: format!("{} · {}", project.code, project.name),
        back_url: PROJECTS_URL.to_owned(),
        status: word(lang, "status", ipa.status.as_str()),
        status_kind: ipa_kind(ipa.status),
        kind: word(lang, "kind", ipa.kind.as_str()),
        valuation_on: ipa.valuation_on,
        claimed_amount: ipa.claimed_amount,
        certified_amount: ipa.certified_amount,
        claim_gap: ipa.claim_gap,
        previous_certified: ipa.previous_certified,
        previous_pending: ipa.previous_pending,
        submitted_on: ipa.submitted_on,
        certified_on: ipa.certified_on,
        certificate: ipa.certificate_ref.clone(),
        late_certificate: ipa.late_certificate_ref.clone(),
        decision_reference: ipa.decision_reference.clone(),
        decided_amount: ipa.decided_amount,
        replaces: replaced.map(|earlier| Link {
            label: english(
                "construction.screen.ipa.replaces",
                &[("number", &earlier.number)],
            ),
            url: ipa_url(earlier.id),
            icon: "history",
        }),
        links,
        deductions: deductions
            .iter()
            .map(|line| DeductionRow::of(lang, line))
            .collect(),
    })
    .into_response()
}
