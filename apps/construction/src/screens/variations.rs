//! `GET /construction/projects/{id}/variation-report` as a page: every
//! variation with its price, what the client approved and the gap.

use renox::prelude::*;
use serde::Serialize;

use super::{PROJECTS_URL, Texts, closing_url, tx, variation_kind, word};
use crate::project::Project;
use crate::texts::english;
use crate::variation::{ReportLine, VariationKind, VariationStatus};

/// One line of the report.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct Row {
    label: String,
    kind: String,
    status: String,
    status_kind: &'static str,
    is_time: bool,
    priced_amount: i64,
    approved_amount: Option<i64>,
    gap: Option<i64>,
    days_asked: Option<i64>,
    days_granted: Option<i64>,
    days_gap: Option<i64>,
    unrecovered_amount: i64,
    notice_late: bool,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "construction/variation_report.html")]
pub(crate) struct VariationReportPage {
    tx: Texts,
    title: String,
    subtitle: String,
    back_url: String,
    closing_url: String,
    rows: Vec<Row>,
    total_gap: i64,
    total_approved: i64,
    total_unrecovered: i64,
}

/// The status of a report line, as the enum.
fn status_of(name: &str) -> VariationStatus {
    [
        VariationStatus::Draft,
        VariationStatus::Submitted,
        VariationStatus::Approved,
        VariationStatus::Rejected,
        VariationStatus::Claimed,
        VariationStatus::Abandoned,
        VariationStatus::Withdrawn,
    ]
    .into_iter()
    .find(|status| status.as_str() == name)
    .unwrap_or_default()
}

/// The page of the variation report of `project`.
pub(crate) fn page(lang: &Lang, project: &Project, lines: &[ReportLine]) -> Response {
    let rows: Vec<Row> = lines
        .iter()
        .map(|line| Row {
            label: english(
                "construction.screen.variations.revision",
                &[("code", &line.code), ("revision", &line.revision)],
            ),
            kind: word(lang, "kind", line.kind),
            status: word(lang, "status", line.status),
            status_kind: variation_kind(status_of(line.status)),
            is_time: line.kind == VariationKind::Time.as_str(),
            priced_amount: line.priced_amount,
            approved_amount: line.approved_amount,
            gap: line.gap,
            days_asked: line.days_asked,
            days_granted: line.days_granted,
            days_gap: line.days_gap,
            unrecovered_amount: line.unrecovered_amount,
            notice_late: line.notice_late,
        })
        .collect();
    let money = |row: &&Row| !row.is_time;
    View::typed(VariationReportPage {
        tx: tx(
            lang,
            &[
                "variations.title",
                "variations.back",
                "variations.variation",
                "variations.priced",
                "variations.approved",
                "variations.gap",
                "variations.days_asked",
                "variations.days_granted",
                "variations.days_gap",
                "variations.unrecovered",
                "variations.late",
                "variations.total_gap",
                "variations.total_approved",
                "variations.total_unrecovered",
                "variations.empty",
                "variations.empty_text",
                "variations.closing",
                "kind",
                "status",
            ],
        ),
        title: word(lang, "variations", "title"),
        subtitle: format!("{} · {}", project.code, project.name),
        back_url: PROJECTS_URL.to_owned(),
        closing_url: closing_url(project.id),
        total_gap: rows.iter().filter_map(|row| row.gap).sum(),
        total_approved: rows
            .iter()
            .filter(money)
            .filter_map(|row| row.approved_amount)
            .sum(),
        total_unrecovered: rows.iter().map(|row| row.unrecovered_amount).sum(),
        rows,
    })
    .into_response()
}
