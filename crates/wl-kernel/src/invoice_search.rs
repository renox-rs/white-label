//! Finding open invoices across parties (wl-payments §5.7 point 2,
//! BL-PAY-29): the bank reconciliation compares each incoming statement line
//! with the open invoices of the bank account's legal entity, by amount,
//! by the invoice number in the reference and by the payer's name, before it
//! knows the party. The invoicing contract ([`crate::invoicing::Invoices`])
//! reads one party's open invoices; this one reads them for an entity.
//!
//! `wl-invoicing` provides [`OpenInvoiceSearch`] with
//! [`provide_invoice_search`]; without it, [`invoice_search`] gives
//! [`NoInvoiceSearch`], which finds nothing, so no line is suggested against
//! an invoice.
//!
//! The same search reads a supplier's payable bills (BL-PAY-28): *Pay
//! supplier* lists the `approved` and `partially_paid` bills of a supplier,
//! oldest first ([`OpenInvoiceSearch::payable_bills_of`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;

use crate::invoicing::{InvoiceSummary, SupplierBillSummary};
use crate::parties::PartyId;

/// The future an [`OpenInvoiceSearch`] method returns.
pub type SearchFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Reads open invoices across parties.
pub trait OpenInvoiceSearch: Send + Sync + 'static {
    /// The open invoices (`issued`, `partially_paid`, `overdue`, with a
    /// balance) issued by the legal entity `legal_entity_id` in `currency`,
    /// oldest due first. The default finds none.
    fn open_of_entity<'a>(
        &'a self,
        _db: &'a Db,
        _legal_entity_id: i64,
        _currency: &'a str,
    ) -> SearchFuture<'a, Vec<InvoiceSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The bills of `supplier` that take a payment (`approved` or
    /// `partially_paid`, with a balance; a bill held as a possible duplicate
    /// is not approved and is not listed), oldest first (bill date, then
    /// id). The default finds none.
    fn payable_bills_of<'a>(
        &'a self,
        _db: &'a Db,
        _supplier: PartyId,
    ) -> SearchFuture<'a, Vec<SupplierBillSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The search when invoicing is off: no invoice is found.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoInvoiceSearch;

impl OpenInvoiceSearch for NoInvoiceSearch {}

/// Makes `search` the app's [`OpenInvoiceSearch`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_invoice_search(app: &mut Registry, search: impl OpenInvoiceSearch) {
    let search: Arc<dyn OpenInvoiceSearch> = Arc::new(search);
    app.provide(search);
}

/// The app's [`OpenInvoiceSearch`], or [`NoInvoiceSearch`] when none is
/// provided.
pub fn invoice_search(state: &AppState) -> Arc<dyn OpenInvoiceSearch> {
    match state.provided::<Arc<dyn OpenInvoiceSearch>>() {
        Some(search) => Arc::clone(&search),
        None => Arc::new(NoInvoiceSearch),
    }
}
