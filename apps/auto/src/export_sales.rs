//! Registering an export sale and attaching its exit evidence (08 `AUT.EXP.01`,
//! BL-AUT-14).
//!
//! Decisions:
//!
//! - **Registered by the accountant** (`export_sales.manage`) from the
//!   zero-rated invoice, with the supply date (the delivery or collection date,
//!   not in the future). The invoice's total, currency, customer and number are
//!   read from wl-invoicing; a credited invoice is not an export sale any more.
//!   Whether the invoice's lines are zero-rated is the accountant's word: the
//!   kernel's invoice summary does not carry the VAT categories (see
//!   `DEFERRED.md`).
//! - **Any one** of a customs exit certificate, a bill of lading or an airway
//!   bill stops the rule for the invoice; more may be attached. The evidence is
//!   a document reference (its number or file name): attaching the file itself
//!   is wl-documents' (`DOC.FIL.01`, deferred for the app).
//! - **Evidence after 90 days**: the flag *5% VAT due* stays, the accountant is
//!   told once, nothing is reversed (open item A-12: whether the zero rate can
//!   be restored).

use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

use crate::db::refused;
use crate::export_rule;
use crate::export_sale::{EVIDENCE_KINDS, ExportEvidence, ExportSale};
use crate::permissions;
use crate::texts::english;

/// The export sales, newest supply first.
///
/// # Errors
///
/// The database's errors.
pub async fn all(db: &Db) -> Result<Vec<ExportSale>> {
    ExportSale::query()
        .order_by_desc(ExportSale::SUPPLY_DATE)
        .order_by_desc(ExportSale::ID)
        .get(db)
        .await
}

/// Registers the zero-rated invoice `invoice_id` as an export sale supplied on
/// `supply_date`.
///
/// # Errors
///
/// A 403 without `export_sales.manage`; a 422 for an invoice that does not
/// exist, was credited or is registered already, or a supply date in the
/// future.
pub async fn record(
    state: &AppState,
    by: &AuthUser,
    invoice_id: i64,
    supply_date: renox::chrono::NaiveDate,
) -> Result<ExportSale> {
    if !by.has_permission(permissions::EXPORT_SALES_MANAGE) {
        return Err(Error::Forbidden);
    }
    if supply_date > today() {
        return Err(refused(english("auto.export.problem.future", &[])));
    }
    let invoice = invoicer(state)
        .find(&state.db, InvoiceId(invoice_id))
        .await?
        .ok_or_else(|| refused(english("auto.export.problem.invoice", &[])))?;
    if invoice.status == InvoiceStatus::Credited {
        return Err(refused(english("auto.export.problem.credited", &[])));
    }
    let known = ExportSale::query()
        .where_(ExportSale::INVOICE_ID.eq(invoice_id))
        .exists(&state.db)
        .await?;
    if known {
        return Err(refused(english("auto.export.problem.known", &[])));
    }
    let PartyId(party) = invoice.party;
    let mut sale = ExportSale {
        invoice_id,
        invoice_number: invoice.number.clone(),
        party_id: party,
        currency: invoice.currency.clone(),
        gross_total: invoice.gross_total,
        supply_date,
        created_by: Some(by.id),
        ..ExportSale::default()
    };
    sale.save(&state.db).await?;
    ExportSale::find_or_404(&state.db, sale.id).await
}

/// Attaches evidence of `kind` (an exit certificate, a bill of lading or an
/// airway bill) with its `reference` to export sale `id`. From then on the rule
/// leaves the sale alone; evidence after the flag keeps the flag and tells the
/// accountant once.
///
/// # Errors
///
/// A 403 without `export_sales.manage`; a 422 for a kind that is none of the
/// three or a blank reference.
pub async fn attach_evidence(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    kind: &str,
    reference: &str,
) -> Result<ExportSale> {
    if !by.has_permission(permissions::EXPORT_SALES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let sale = ExportSale::find_or_404(&state.db, id).await?;
    let kind = kind.trim();
    if !EVIDENCE_KINDS.contains(&kind) {
        return Err(refused(english("auto.export.problem.kind", &[])));
    }
    let reference = reference.trim();
    if reference.is_empty() {
        return Err(refused(english("auto.export.problem.reference", &[])));
    }
    let mut evidence = ExportEvidence {
        export_sale_id: sale.id,
        kind: kind.to_owned(),
        reference: reference.to_owned(),
        attached_by: Some(by.id),
        ..ExportEvidence::default()
    };
    evidence.save(&state.db).await?;
    ExportSale::query()
        .where_(ExportSale::ID.eq(sale.id))
        .update(&state.db, &[("updated_at", &now())])
        .await?;
    if sale.vat_flagged_at.is_some() && sale.late_evidence_notified_at.is_none() {
        export_rule::tell_late_evidence(state, &sale, kind).await?;
    }
    ExportSale::find_or_404(&state.db, id).await
}
