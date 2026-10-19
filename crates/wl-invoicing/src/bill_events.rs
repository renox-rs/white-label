//! The kernel events the Supplier bill machine emits (wl-invoicing §6), from
//! a listener on [`Transitioned<SupplierBill>`], so only after a commit and
//! never on a rollback:
//!
//! - `approve` / `approve_system` -> [`SupplierBillApproved`];
//! - `void` / `void_system` -> [`SupplierBillVoided`] (with the reason);
//! - `submit_held` / `submit_held_system` -> the accountant notice
//!   ([`crate::bill_notices`]);
//! - `unallocate` / `unallocate_partially` -> [`SupplierBillReopened`] (the
//!   money source, amount and reason come from the run's `meta`), then the
//!   accountant notice ([`crate::bill_reopen_notices`]).

use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    BillReopenReason, SupplierBillApproved, SupplierBillId, SupplierBillReopened,
    SupplierBillVoided,
};

use crate::bill_machine::{APPROVE, APPROVE_SYSTEM, VOID, VOID_SYSTEM};
use crate::bill_submit::{SUBMIT_HELD, SUBMIT_HELD_SYSTEM};
use crate::bill_unallocate::REOPEN_TARGETS;
use crate::payables::kernel_status;
use crate::supplier_bill::SupplierBill;

/// `SupplierBillApproved` for `bill`, as it is after the approval.
pub(crate) fn approved(bill: &SupplierBill) -> SupplierBillApproved {
    SupplierBillApproved {
        bill: SupplierBillId(bill.id),
        supplier: bill.supplier(),
        amount: bill.gross_total,
        cost_objects: Vec::new(),
        landed_costs: Vec::new(),
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

/// `SupplierBillReopened` for `bill` after an unallocation run with `meta`:
/// the money source, amount and reason. `None` when the run's `meta` does
/// not carry them. The balance and state are the committed ones.
pub(crate) fn reopened(bill: &SupplierBill, meta: &Value) -> Option<SupplierBillReopened> {
    let source = meta
        .get("source")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<Subject>().ok())?;
    let amount = meta.get("amount").and_then(Value::as_i64)?;
    let reason = meta
        .get("reason")
        .cloned()
        .and_then(|value| renox::serde_json::from_value::<BillReopenReason>(value).ok())?;
    Some(SupplierBillReopened {
        bill: SupplierBillId(bill.id),
        supplier: bill.supplier(),
        source,
        amount,
        balance: bill.balance,
        new_status: kernel_status(bill.status),
        reason,
    })
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<SupplierBill>, state: AppState| async move {
            let name = event.transition.as_str();
            if REOPEN_TARGETS.iter().any(|(target, _)| *target == name) {
                let bill = SupplierBill::find_or_404(&state.db, event.key).await?;
                let Some(reopened) = reopened(&bill, &event.meta) else {
                    return Ok(());
                };
                state.emit(reopened.clone()).await?;
                let why = event.meta.get("reversal_reason").and_then(Value::as_str);
                return crate::bill_reopen_notices::notify(&state, &bill, &reopened, why).await;
            }
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
    fn a_reopening_is_read_from_the_run_meta_and_the_committed_row() {
        let bill = SupplierBill {
            id: 3,
            party_id: 4,
            balance: 1_050_000,
            status: crate::supplier_bill::SupplierBillStatus::Approved,
            ..Default::default()
        };
        let meta = renox::serde_json::json!({
            "source": "payments.payment:9",
            "amount": 1_050_000,
            "reason": "payment_reversed",
            "reversal_reason": "bank_return",
        });
        let event = reopened(&bill, &meta).expect("event");
        assert_eq!(event.source, Subject::new("payments.payment", 9));
        assert_eq!((event.amount, event.balance), (1_050_000, 1_050_000));
        assert_eq!(event.reason, BillReopenReason::PaymentReversed);
        assert_eq!(event.supplier, PartyId(4));
        assert!(reopened(&bill, &renox::serde_json::json!({})).is_none());
    }

    #[test]
    fn a_void_carries_the_reason() {
        assert_eq!(voided(&bill(), Some("by mistake")).reason, "by mistake");
        assert_eq!(voided(&bill(), None).reason, "duplicate");
        assert_eq!(voided(&bill(), None).bill, SupplierBillId(3));
    }
}
