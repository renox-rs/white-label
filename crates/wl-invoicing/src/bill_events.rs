//! The kernel events the Supplier bill machine emits (wl-invoicing §6), from
//! a listener on [`Transitioned<SupplierBill>`], so only after a commit and
//! never on a rollback:
//!
//! - `approve` / `approve_system` -> [`SupplierBillApproved`];
//! - `void` / `void_system` -> [`SupplierBillVoided`] (with the reason);
//! - `submit_held` / `submit_held_system` -> the accountant notice
//!   ([`crate::bill_notices`]).

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::invoicing::{SupplierBillApproved, SupplierBillId, SupplierBillVoided};

use crate::bill_machine::{APPROVE, APPROVE_SYSTEM, VOID, VOID_SYSTEM};
use crate::bill_submit::{SUBMIT_HELD, SUBMIT_HELD_SYSTEM};
use crate::supplier_bill::SupplierBill;

/// `SupplierBillApproved` for `bill`, as it is after the approval.
pub(crate) fn approved(bill: &SupplierBill) -> SupplierBillApproved {
    SupplierBillApproved {
        bill: SupplierBillId(bill.id),
        supplier: bill.supplier(),
        amount: bill.gross_total,
    }
}

/// `SupplierBillVoided` for `bill` with the reason of the run (the stored
/// void reason when the run carries none).
pub(crate) fn voided(bill: &SupplierBill, reason: Option<&str>) -> SupplierBillVoided {
    SupplierBillVoided {
        bill: SupplierBillId(bill.id),
        reason: reason
            .map(str::to_owned)
            .or_else(|| bill.void_reason.clone())
            .unwrap_or_default(),
    }
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<SupplierBill>, state: AppState| async move {
            let name = event.transition.as_str();
            if ![
                APPROVE,
                APPROVE_SYSTEM,
                VOID,
                VOID_SYSTEM,
                SUBMIT_HELD,
                SUBMIT_HELD_SYSTEM,
            ]
            .contains(&name)
            {
                return Ok(());
            }
            // The committed row: the totals and the stored reasons.
            let bill = SupplierBill::find_or_404(&state.db, event.key).await?;
            match name {
                APPROVE | APPROVE_SYSTEM => state.emit(approved(&bill)).await,
                VOID | VOID_SYSTEM => state.emit(voided(&bill, event.reason.as_deref())).await,
                _ => crate::bill_notices::notify_held(&state, bill.id).await,
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::PartyId;

    fn bill() -> SupplierBill {
        SupplierBill {
            id: 3,
            party_id: 4,
            gross_total: 300_000,
            void_reason: Some("duplicate".into()),
            ..Default::default()
        }
    }

    #[test]
    fn an_approval_carries_the_supplier_and_the_gross() {
        let event = approved(&bill());
        assert_eq!(event.bill, SupplierBillId(3));
        assert_eq!(event.supplier, PartyId(4));
        assert_eq!(event.amount, 300_000);
    }

    #[test]
    fn a_void_carries_the_reason() {
        assert_eq!(voided(&bill(), Some("by mistake")).reason, "by mistake");
        assert_eq!(voided(&bill(), None).reason, "duplicate");
        assert_eq!(voided(&bill(), None).bill, SupplierBillId(3));
    }
}
