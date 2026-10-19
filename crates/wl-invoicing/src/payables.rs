//! The kernel's supplier bill contract (wl-invoicing §6): [`Payables`]
//! implements [`SupplierBills`], so another module can read a bill line
//! without knowing this module. `register` provides it with
//! `provide_supplier_bills`.

use renox::prelude::*;
use wl_kernel::invoicing::{
    SupplierBillFuture, SupplierBillId, SupplierBillLine, SupplierBillLineId,
    SupplierBillStatus as KernelStatus, SupplierBills,
};

use crate::bill_line::BillLine;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};

/// The kernel's name for the module's bill state; the same seven states.
pub fn kernel_status(status: SupplierBillStatus) -> KernelStatus {
    match status {
        SupplierBillStatus::Draft => KernelStatus::Draft,
        SupplierBillStatus::Matched => KernelStatus::Matched,
        SupplierBillStatus::Exception => KernelStatus::Exception,
        SupplierBillStatus::Approved => KernelStatus::Approved,
        SupplierBillStatus::PartiallyPaid => KernelStatus::PartiallyPaid,
        SupplierBillStatus::Paid => KernelStatus::Paid,
        SupplierBillStatus::Void => KernelStatus::Void,
    }
}

/// The module's state for the kernel's.
pub fn local_status(status: KernelStatus) -> SupplierBillStatus {
    match status {
        KernelStatus::Draft => SupplierBillStatus::Draft,
        KernelStatus::Matched => SupplierBillStatus::Matched,
        KernelStatus::Exception => SupplierBillStatus::Exception,
        KernelStatus::Approved => SupplierBillStatus::Approved,
        KernelStatus::PartiallyPaid => SupplierBillStatus::PartiallyPaid,
        KernelStatus::Paid => SupplierBillStatus::Paid,
        KernelStatus::Void => SupplierBillStatus::Void,
    }
}

/// `line` of `bill` as the kernel shows it. All VAT is treated as
/// recoverable until input-VAT rules exist, so the amount is the line's net
/// in AED (0 while the exchange rate is unknown).
pub fn kernel_line(bill: &SupplierBill, line: &BillLine) -> SupplierBillLine {
    SupplierBillLine {
        bill: SupplierBillId(bill.id),
        line: SupplierBillLineId(line.id),
        bill_number: bill.supplier_invoice_number.clone(),
        supplier: bill.supplier(),
        legal_entity_id: bill.legal_entity_id,
        description: line.description.clone(),
        amount_aed: line.net_amount_aed.unwrap_or(0),
        bill_status: kernel_status(bill.status),
    }
}

/// The module's [`SupplierBills`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Payables;

impl SupplierBills for Payables {
    fn line<'a>(
        &'a self,
        db: &'a Db,
        id: SupplierBillLineId,
    ) -> SupplierBillFuture<'a, Option<SupplierBillLine>> {
        Box::pin(async move {
            let Some(line) = BillLine::find(db, id.0).await? else {
                return Ok(None);
            };
            let Some(bill) = SupplierBill::find(db, line.bill_id).await? else {
                return Ok(None);
            };
            Ok(Some(kernel_line(&bill, &line)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_states_map_both_ways() {
        use SupplierBillStatus::*;
        for s in [
            Draft,
            Matched,
            Exception,
            Approved,
            PartiallyPaid,
            Paid,
            Void,
        ] {
            assert_eq!(local_status(kernel_status(s)), s);
        }
        assert_eq!(kernel_status(Exception), KernelStatus::Exception);
    }

    #[test]
    fn a_line_shows_its_bill_and_net_in_aed() {
        let bill = SupplierBill {
            id: 3,
            party_id: 4,
            legal_entity_id: Some(1),
            supplier_invoice_number: "INV-9".into(),
            status: SupplierBillStatus::Approved,
            ..Default::default()
        };
        let line = BillLine {
            id: 8,
            bill_id: 3,
            description: "Pump".into(),
            net_amount: 1_000,
            net_amount_aed: Some(3_670),
            ..Default::default()
        };
        let shown = kernel_line(&bill, &line);
        assert_eq!(
            (shown.bill, shown.line),
            (SupplierBillId(3), SupplierBillLineId(8))
        );
        assert_eq!(shown.bill_number, "INV-9");
        assert_eq!(shown.supplier.0, 4);
        assert_eq!(shown.legal_entity_id, Some(1));
        assert_eq!(shown.amount_aed, 3_670);
        assert_eq!(shown.bill_status, KernelStatus::Approved);
    }
}
