//! The kernel's supplier bill contract (wl-invoicing §6): [`Payables`]
//! implements [`SupplierBills`], so another module can read a bill line
//! without knowing this module. `register` provides it with
//! `provide_supplier_bills`.

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::invoicing::{
    BillBalance, BillRequest, PayBill, SupplierBillFuture, SupplierBillId, SupplierBillLine,
    SupplierBillLineId, SupplierBillStatus as KernelStatus, SupplierBillSummary, SupplierBills,
    UnallocateBill,
};

use crate::bill_allocation::BillAllocation;
use crate::bill_line::{BillLine, BillLineDraft};
use crate::bill_pay::pay_keyed_in;
use crate::bill_submit;
use crate::credit_notes::By;
use crate::supplier_bill::{self, BillDraft, SupplierBill, SupplierBillStatus};

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

/// `bill` as the kernel shows it: its supplier, entity, currency, state and
/// open balance.
pub fn kernel_summary(bill: &SupplierBill) -> SupplierBillSummary {
    SupplierBillSummary {
        id: SupplierBillId(bill.id),
        number: bill.supplier_invoice_number.clone(),
        supplier: bill.supplier(),
        legal_entity_id: bill.legal_entity_id,
        currency: bill.currency.clone(),
        status: kernel_status(bill.status),
        balance: bill.balance,
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

    fn find<'a>(
        &'a self,
        db: &'a Db,
        id: SupplierBillId,
    ) -> SupplierBillFuture<'a, Option<SupplierBillSummary>> {
        Box::pin(async move {
            Ok(SupplierBill::find(db, id.0)
                .await?
                .map(|bill| kernel_summary(&bill)))
        })
    }

    fn create_submitted<'a>(
        &'a self,
        state: &'a AppState,
        request: BillRequest,
    ) -> SupplierBillFuture<'a, Option<SupplierBillId>> {
        Box::pin(async move { create_submitted(state, request).await })
    }

    fn pay<'a>(
        &'a self,
        state: &'a AppState,
        request: PayBill,
    ) -> SupplierBillFuture<'a, BillBalance> {
        Box::pin(async move { pay_keyed(state, request).await })
    }

    fn unallocate<'a>(
        &'a self,
        state: &'a AppState,
        request: UnallocateBill,
    ) -> SupplierBillFuture<'a, BillBalance> {
        Box::pin(async move { crate::bill_unallocate::run(state, request).await })
    }
}

/// [`SupplierBills::create_submitted`]: a `draft` bill with the request's
/// lines, in AED, then `submit` as the system. The bill has no purchase order
/// to match, so it lands in `matched` (or `exception` when the duplicate
/// check holds it) and waits for the approval of the accountant.
async fn create_submitted(
    state: &AppState,
    request: BillRequest,
) -> Result<Option<SupplierBillId>> {
    if request.lines.is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "a supplier bill needs at least one line".to_owned(),
        ));
    }
    let mut draft = BillDraft::new(request.supplier, &request.number, request.bill_date);
    draft.legal_entity_id = request.legal_entity_id;
    for line in &request.lines {
        if line.discount != 0 {
            return Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                "a line of a supplier bill takes no discount".to_owned(),
            ));
        }
        draft = draft.line(
            BillLineDraft::new(&line.description, line.unit_price, line.vat_category)
                .quantity_milli(line.quantity_milli),
        );
    }
    let bill = supplier_bill::create(&state.db, &draft, None).await?;
    let bill = bill_submit::submit(state, By::System(None), bill.id)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(Some(SupplierBillId(bill.id)))
}

/// The bill as the kernel answers a call: its state and balance.
fn balance_of(bill: &SupplierBill, skipped: bool) -> BillBalance {
    BillBalance {
        new_status: kernel_status(bill.status),
        balance: bill.balance,
        skipped,
    }
}

/// [`SupplierBills::pay`]: applies the money in one transaction, under the
/// request's idempotency key. A key that was used gives `skipped`; a
/// refusal is a 422 and writes nothing.
async fn pay_keyed(state: &AppState, request: PayBill) -> Result<BillBalance> {
    if let Some(done) = BillAllocation::by_key(&state.db, &request.key).await? {
        let bill = SupplierBill::find_or_404(&state.db, done.bill_id).await?;
        return Ok(balance_of(&bill, true));
    }
    let bill = SupplierBill::find_or_404(&state.db, request.bill.0).await?;
    let mut tx = state.db.begin().await?;
    let outcome = match pay_keyed_in(
        &mut tx,
        &bill,
        &request.source,
        request.amount,
        None,
        Some(request.key.clone()),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(TransitionError::Failed(error)) if error.is_unique_violation() => {
            drop(tx);
            let bill = SupplierBill::find_or_404(&state.db, bill.id).await?;
            return Ok(balance_of(&bill, true));
        }
        Err(error) => return Err(error.into_error()),
    };
    tx.commit().await?;
    outcome.emit(state).await?;
    let bill = SupplierBill::find_or_404(&state.db, bill.id).await?;
    Ok(balance_of(&bill, false))
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
    fn a_bill_summary_shows_its_supplier_entity_state_and_balance() {
        let bill = SupplierBill {
            id: 3,
            party_id: 4,
            legal_entity_id: Some(1),
            supplier_invoice_number: "SB-77".into(),
            currency: "AED".into(),
            balance: 525_000,
            status: SupplierBillStatus::Approved,
            ..Default::default()
        };
        let summary = kernel_summary(&bill);
        assert_eq!(summary.id, SupplierBillId(3));
        assert_eq!(summary.number, "SB-77");
        assert_eq!(summary.supplier, wl_kernel::PartyId(4));
        assert_eq!(summary.legal_entity_id, Some(1));
        assert_eq!(summary.status, KernelStatus::Approved);
        assert_eq!(
            (summary.currency.as_str(), summary.balance),
            ("AED", 525_000)
        );
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
