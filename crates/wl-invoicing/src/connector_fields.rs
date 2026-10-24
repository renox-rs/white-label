//! An invoice as an AI assistant reads it with the connector's `fetch` tool
//! (BL-AGT-07): number, status, dates and the amounts in the invoice's
//! currency, each in the minor unit with the currency stated (fils for
//! AED), so "what does Al Ali owe us?" is answered from the balance.
//!
//! The connector calls it only for an invoice the user may open in the
//! search (`invoices.view` in the invoice's scope).

use renox::prelude::*;
use wl_kernel::connector::RecordFields;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;

use crate::invoice::Invoice;

/// The fields of `invoice` for an assistant.
pub fn fields(invoice: &Invoice) -> RecordFields {
    let date = |d: Option<renox::chrono::NaiveDate>| {
        d.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
    };
    let currency = invoice.currency.as_str();
    let mut fields = RecordFields::new()
        .text("Number", invoice.number.clone().unwrap_or_default())
        .text("Status", invoice.status.as_str())
        .text("Issue date", date(invoice.issue_date))
        .text("Due date", date(invoice.due_date))
        .money("Net", invoice.net_total, currency)
        .money("VAT", invoice.vat_total, currency)
        .money("Total", invoice.gross_total, currency)
        .money("Paid", invoice.allocated_total, currency)
        .money("Balance", invoice.balance, currency);
    if invoice.disputed {
        fields = fields.text("Disputed", "yes");
    }
    fields
}

/// Registers the fields of `invoicing.invoice` for the connector.
pub(crate) fn register() {
    wl_kernel::connector::register_fields(INVOICE_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(Invoice::query()
                .where_eq("id", id)
                .first(db)
                .await?
                .map(|invoice| fields(&invoice)))
        })
    });
}
