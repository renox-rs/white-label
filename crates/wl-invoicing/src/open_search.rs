//! The open invoices of a legal entity across parties, for the bank
//! reconciliation's match suggestions (BL-PAY-29), and a supplier's payable
//! bills, for *Pay supplier* (BL-PAY-28): the kernel's
//! [`OpenInvoiceSearch`](wl_kernel::invoice_search::OpenInvoiceSearch).

use renox::prelude::*;
use wl_kernel::invoice_search::{OpenInvoiceSearch, SearchFuture};
use wl_kernel::PartyId;
use wl_kernel::invoicing::{InvoiceStatus, InvoiceSummary, SupplierBillSummary};

use crate::invoice::Invoice;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};

/// The search wl-invoicing provides.
pub struct OpenInvoices;

impl OpenInvoiceSearch for OpenInvoices {
    fn open_of_entity<'a>(
        &'a self,
        db: &'a Db,
        legal_entity_id: i64,
        currency: &'a str,
    ) -> SearchFuture<'a, Vec<InvoiceSummary>> {
        Box::pin(async move {
            let open = [
                InvoiceStatus::Issued,
                InvoiceStatus::PartiallyPaid,
                InvoiceStatus::Overdue,
            ];
            let rows = Invoice::query()
                .where_(Invoice::STATUS.is_in(open))
                .where_(Invoice::LEGAL_ENTITY_ID.eq(legal_entity_id))
                .where_(Invoice::CURRENCY.eq(currency))
                .where_(Invoice::BALANCE.gt(0))
                .order_by(Invoice::ID)
                .get(db)
                .await?;
            let mut found: Vec<InvoiceSummary> = rows.iter().map(Invoice::summary).collect();
            found.sort_by_key(|invoice| (invoice.due_date, invoice.id.0));
            Ok(found)
        })
    }

    fn payable_bills_of<'a>(
        &'a self,
        db: &'a Db,
        supplier: PartyId,
    ) -> SearchFuture<'a, Vec<SupplierBillSummary>> {
        Box::pin(async move {
            let rows = SupplierBill::query()
                .where_(SupplierBill::PARTY_ID.eq(supplier.0))
                .where_(SupplierBill::STATUS.is_in([
                    SupplierBillStatus::Approved,
                    SupplierBillStatus::PartiallyPaid,
                ]))
                .where_(SupplierBill::BALANCE.gt(0))
                .order_by(SupplierBill::BILL_DATE)
                .order_by(SupplierBill::ID)
                .get(db)
                .await?;
            Ok(rows.iter().map(crate::payables::kernel_summary).collect())
        })
    }
}
