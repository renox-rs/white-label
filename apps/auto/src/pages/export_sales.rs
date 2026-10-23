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

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Texts, invalid, parse_number, parse_optional_date, party_name, require, text};
use crate::export_sale::{EVIDENCE_KINDS, ExportSale};
use crate::export_sales;
use crate::money_text;
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

/// One export sale of the list.
#[derive(Serialize, Debug, Clone)]
pub struct SaleRow {
    /// The primary key.
    pub id: i64,
    /// The invoice's number.
    pub invoice: String,
    /// The customer's name.
    pub customer: String,
    /// The supply date.
    pub supply: String,
    /// The days since the supply date.
    pub days: String,
    /// The invoice's total, `AED 25,000.00`.
    pub total: String,
    /// Where it stands, in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// The evidence attached, `Bill of lading BL-77 · Airway bill AW-9`, or a
    /// dash.
    pub evidence: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/export_sales.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<SaleRow>,
    can_manage: bool,
    kind_options: Vec<[String; 2]>,
    today: String,
}

/// Where a sale stands, in words and as a badge kind.
fn standing(lang: &Lang, sale: &ExportSale, has_evidence: bool) -> (String, &'static str) {
    let (key, kind) = if sale.vat_flagged_at.is_some() {
        if has_evidence {
            ("export.status.vat_due_late", "error")
        } else {
            ("export.status.vat_due", "error")
        }
    } else if has_evidence {
        ("export.status.attached", "success")
    } else if sale.notified_at.is_some() {
        ("export.status.missing", "warning")
    } else {
        ("export.status.waiting", "info")
    };
    (text(lang, key), kind)
}

/// `GET /auto/export-sales`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::EXPORT_SALES_VIEW)?;
    let today = wl_kernel::documents::today();
    let mut rows = Vec::new();
    for sale in export_sales::all(&state.db).await? {
        let attached = sale.evidence(&state.db).await?;
        let (status, status_kind) = standing(&lang, &sale, !attached.is_empty());
        rows.push(SaleRow {
            id: sale.id,
            invoice: sale.number(),
            customer: party_name(&state, &lang.locale, sale.party_id).await,
            supply: sale.supply_date.to_string(),
            days: sale.days_since_supply(today).to_string(),
            total: money_text(sale.gross_total),
            status,
            status_kind,
            evidence: if attached.is_empty() {
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
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        can_manage: user.has_permission(permissions::EXPORT_SALES_MANAGE),
        kind_options: EVIDENCE_KINDS
            .iter()
            .map(|kind| [(*kind).to_owned(), text(&lang, &format!("export.kind.{kind}"))])
            .collect(),
        today: today.to_string(),
    }))
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
    Ok((Toast::success(text(&lang, "export.registered")), renox::HxRefresh).into_response())
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
    Ok((Toast::success(text(&lang, "export.attached")), renox::HxRefresh).into_response())
}
