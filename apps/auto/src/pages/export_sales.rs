//! The export sales page (`AUT.EXP.01`, `AUT.RUL.15`, BL-AUT-14).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/export-sales` | `auto.export_sales` | `export_sales.view` |
//! | `POST /auto/export-sales` | `auto.export_sale_create` | `export_sales.manage` |
//! | `POST /auto/export-sales/{id}/evidence` | `auto.export_evidence` | `export_sales.manage` |
//!
//! The list shows each sale with the days since its supply date and where it
//! stands in words: waiting for exit evidence, missing (the accountant was
//! told at 60 days), *5% VAT due* (flagged at 90 days), or attached.

use std::collections::HashMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use super::lists;
use super::{Texts, invalid, parse_number, parse_optional_date, require, text};
use crate::export_sale::{EVIDENCE_KINDS, ExportSale};
use crate::export_sales;
use crate::permissions;
use crate::texts;

/// The routes of the page.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/export-sales", index)
        .name("auto.export_sales")
        .post("/auto/export-sales", create)
        .name("auto.export_sale_create")
        .post("/auto/export-sales/{id}/evidence", evidence)
        .name("auto.export_evidence")
        .require_auth()
}

/// Where a sale stands, as SQL: *5% VAT due* once flagged at 90 days (late
/// when evidence came after), *attached* with evidence, *missing* once the
/// accountant was told at 60 days, else *waiting*.
const STANDING_SQL: &str = "CASE WHEN {T}.\"vat_flagged_at\" IS NOT NULL THEN \
    CASE WHEN EXISTS (SELECT 1 FROM \"auto_export_evidence\" e WHERE e.\"export_sale_id\" = {T}.\"id\") \
    THEN 'vat_due_late' ELSE 'vat_due' END \
    WHEN EXISTS (SELECT 1 FROM \"auto_export_evidence\" e WHERE e.\"export_sale_id\" = {T}.\"id\") \
    THEN 'attached' \
    WHEN {T}.\"notified_at\" IS NOT NULL THEN 'missing' ELSE 'waiting' END";

/// The standings, in the order a sale goes through them.
const STANDINGS: [&str; 5] = ["waiting", "missing", "vat_due", "vat_due_late", "attached"];

/// The export sales grid: the invoice first and frozen (a link to the sale's
/// record in invoicing), the customer, the supply date, the days since, the
/// invoice total in its currency, where it stands in words and the evidence
/// attached; searched by invoice number.
pub(crate) fn grid(t: lists::Text<'_>) -> Grid {
    let standings = lists::options(t, "export.status", STANDINGS);
    lists::baseline(Grid::new("auto-export-sales"))
        .title(&t("export.title"))
        .column(
            Column::text("invoice_number", &t("export.invoice"))
                .frozen()
                .mobile()
                .searchable()
                .link("/invoices/{invoice_id}"),
        )
        .column(Column::custom("customer", &t("export.customer")).mobile())
        .column(Column::date("supply_date", &t("export.supply_date")))
        .column(Column::custom("days", &t("export.days")))
        .column(
            Column::money("gross_total", &t("export.total"))
                .currency_from("currency")
                .mobile()
                .summary(Summary::Sum),
        )
        .column(
            Column::select("standing", &t("fields.status"), standings)
                .computed(STANDING_SQL)
                .width("14rem")
                .badges(&[
                    ("waiting", "info"),
                    ("missing", "warning"),
                    ("vat_due", "danger"),
                    ("vat_due_late", "danger"),
                    ("attached", "success"),
                ])
                .mobile(),
        )
        .column(Column::custom("evidence", &t("export.evidence")))
        .column(Column::custom("attach", "").frozen_right())
        .sort_by("-supply_date")
        .cards_on_mobile()
        .empty_state(&t("export.empty"), Some(&t("export.empty_hint")))
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/export_sales.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    sales: GridPage<ExportSale>,
    /// The sales of the page, `[id, invoice number]`, for the sheets that
    /// attach their evidence.
    sheets: Vec<(i64, String)>,
    can_manage: bool,
    kind_options: Vec<[String; 2]>,
    today: String,
}

/// `GET /auto/export-sales`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    require(&user, permissions::EXPORT_SALES_VIEW)?;
    let today = wl_kernel::documents::today();
    let can_export = user.has_permission(permissions::EXPORT_SALES_MANAGE);
    let grid = lists::exporting(grid(&lists::texts_in(&lang)), can_export);
    if let Some(file) = lists::export_file(&grid, can_export, ExportSale::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(ExportSale::query(), &request).await?;
    let items: Vec<ExportSale> = page.items().to_vec();
    let mut evidence = HashMap::new();
    for sale in &items {
        let attached = sale.evidence(&state.db).await?;
        evidence.insert(
            sale.id,
            if attached.is_empty() {
                text(&lang, "none")
            } else {
                attached
                    .iter()
                    .map(|piece| {
                        format!(
                            "{} {}",
                            text(&lang, &format!("export.kind.{}", piece.kind)),
                            piece.reference
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            },
        );
    }
    let party_ids: Vec<i64> = items.iter().map(|sale| sale.party_id).collect();
    let names = lists::party_names(&state, &lang, party_ids).await;
    let sheets = items.iter().map(|sale| (sale.id, sale.number())).collect();
    let sales = page.extend(|sale| {
        json!({
            "customer": names.get(&sale.party_id).cloned().unwrap_or_default(),
            "days": sale.days_since_supply(today),
            "evidence": evidence.get(&sale.id).cloned().unwrap_or_default(),
        })
    });
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        sales,
        sheets,
        can_manage: user.has_permission(permissions::EXPORT_SALES_MANAGE),
        kind_options: EVIDENCE_KINDS
            .iter()
            .map(|kind| {
                [
                    (*kind).to_owned(),
                    text(&lang, &format!("export.kind.{kind}")),
                ]
            })
            .collect(),
        today: today.to_string(),
    })
    .into_response())
}

/// The form that registers an export sale.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct SaleForm {
    /// The invoice.
    #[serde(default)]
    pub invoice: String,
    /// The supply date.
    #[serde(default)]
    pub supply_date: String,
}

/// `POST /auto/export-sales`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<SaleForm>,
) -> Result<Response> {
    require(&user, permissions::EXPORT_SALES_MANAGE)?;
    let invoice = parse_number(&form.invoice)?;
    let supply: NaiveDate = parse_optional_date(&form.supply_date)?
        .ok_or_else(|| invalid(&texts::english("auto.export.supply_hint", &[])))?;
    export_sales::record(&state, &user, invoice, supply).await?;
    Ok((
        Toast::success(text(&lang, "export.registered")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that attaches exit evidence.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct EvidenceForm {
    /// One of the three kinds.
    #[serde(default)]
    pub kind: String,
    /// The document's number or file name.
    #[serde(default)]
    pub reference: String,
}

/// `POST /auto/export-sales/{id}/evidence`.
async fn evidence(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<EvidenceForm>,
) -> Result<Response> {
    export_sales::attach_evidence(&state, &user, id, &form.kind, &form.reference).await?;
    Ok((
        Toast::success(text(&lang, "export.attached")),
        renox::HxRefresh,
    )
        .into_response())
}
