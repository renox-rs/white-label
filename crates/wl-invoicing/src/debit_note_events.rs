//! What follows a supplier debit note, from listeners that run only after a
//! commit (wl-invoicing §4, §6):
//!
//! - `settle` / `settle_system` emit `SupplierDebitNoteSettled`;
//! - `cancel` / `cancel_system` put the cost back of an issued note, tell the
//!   supplier and emit `SupplierDebitNoteCancelled`;
//! - `issue` / `issue_system` reverse the actual cost on the cost objects of
//!   the lines through the kernel's `CostPoster` (the net of each line in AED
//!   fils, one request per note under the key `invoicing.supplier_debit_note:<id>`
//!   so a repeat changes nothing), tell the supplier, and emit
//!   `SupplierDebitNoteIssued`;
//! - a `SupplierBillApproved` bill links the drafts that wait for it
//!   ([`crate::debit_note_contract::link_waiting`]);
//! - `SupplierReturnDispatched` drafts the note of a return (`FIN.RUL.17`).

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transitioned;
use wl_kernel::invoicing::{
    ReverseCost, SupplierBillId, SupplierDebitNoteCancelled, SupplierDebitNoteIssued,
    SupplierDebitNoteSettled, SupplierReturnDispatched, cost_poster,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::bill_machine::{APPROVE, APPROVE_SYSTEM};
use crate::debit_note_machine::{CANCEL, CANCEL_SYSTEM, ISSUE, ISSUE_SYSTEM, SETTLE, SETTLE_SYSTEM};
use crate::debit_notes::lines_of;
use crate::supplier_bill::SupplierBill;
use crate::supplier_debit_note::SupplierDebitNote;
use crate::texts::english;

/// The notification type that tells the supplier of an issued note.
pub const ISSUED: &str = "invoicing.supplier_debit_note_issued";

/// The notification type that tells the supplier of a withdrawn note.
pub const CANCELLED: &str = "invoicing.supplier_debit_note_cancelled";

/// Registers [`ISSUED`], [`CANCELLED`] and the accountant's draft notice.
pub(crate) fn register_types() {
    notify::register_type(
        NotificationTypeDef::new(ISSUED, "invoicing")
            .group("supplier_bills")
            .category(Category::Transactional)
            .template(
                NotifyChannel::Email,
                "en",
                Some(english("wl_invoicing.debit_note_issued.subject")),
                english("wl_invoicing.debit_note_issued.body"),
            ),
    );
    notify::register_type(
        NotificationTypeDef::new(CANCELLED, "invoicing")
            .group("supplier_bills")
            .category(Category::Transactional)
            .template(
                NotifyChannel::Email,
                "en",
                Some(english("wl_invoicing.debit_note_cancelled.subject")),
                english("wl_invoicing.debit_note_cancelled.body"),
            ),
    );
    crate::debit_note_contract::register_types();
}

/// The idempotency key of the cost reversal of `note`.
pub fn reversal_key(note: &SupplierDebitNote) -> String {
    note.subject().to_string()
}

/// The cost the lines of `note` take back, per cost object (AED fils).
pub async fn cost_reversals(db: &Db, note: &SupplierDebitNote) -> Result<ReverseCost> {
    let mut by_object: BTreeMap<String, (wl_kernel::Subject, i64)> = BTreeMap::new();
    for line in lines_of(db, note.id).await? {
        let (Some(cost_object), Some(net_aed)) = (line.cost_object(), line.net_amount_aed) else {
            continue;
        };
        by_object
            .entry(cost_object.to_string())
            .or_insert_with(|| (cost_object, 0))
            .1 += net_aed;
    }
    let mut request = ReverseCost::new(note.subject(), reversal_key(note));
    for (_, (cost_object, amount)) in by_object {
        if amount != 0 {
            request = request.line(cost_object, amount);
        }
    }
    Ok(request)
}

/// After the commit of `issue`: the cost reversal, the supplier notice and
/// the event.
async fn issued(state: &AppState, note_id: i64) -> Result {
    let note = SupplierDebitNote::find_or_404(&state.db, note_id).await?;
    let reversal = cost_reversals(&state.db, &note).await?;
    if !reversal.lines.is_empty() {
        cost_poster(state).reverse(state, reversal).await?;
    }
    let Some(bill_id) = note.bill_id else {
        return Ok(());
    };
    // The accountant records the supplier's tax credit note when it comes.
    crate::debit_note_settle::open_tax_credit_task(state, &note).await?;
    let bill = SupplierBill::find_or_404(&state.db, bill_id).await?;
    let request = NotifyRequest::new(ISSUED, note.subject())
        .subject_label(note.number.as_deref().unwrap_or_default())
        .data(json!({
            "number": note.number,
            "amount": amount_text(&note.currency, note.gross_total),
            "bill": bill.supplier_invoice_number,
        }))
        .to_party(note.supplier());
    notify::notifier(state).notify(state, request).await?;
    state
        .emit(SupplierDebitNoteIssued {
            debit_note: note.debit_note_id(),
            supplier: note.supplier(),
            bill: SupplierBillId(bill_id),
            return_to_supplier: note.source(),
            amount: note.gross_total,
            vat: note.vat_total,
        })
        .await
}

/// After the commit of `settle`: the event.
async fn settled(state: &AppState, note_id: i64) -> Result {
    let note = SupplierDebitNote::find_or_404(&state.db, note_id).await?;
    let Some(bill_id) = note.bill_id else {
        return Ok(());
    };
    state
        .emit(SupplierDebitNoteSettled {
            debit_note: note.debit_note_id(),
            supplier: note.supplier(),
            bill: SupplierBillId(bill_id),
            amount: note.gross_total,
            difference: note.settle_difference,
        })
        .await
}

/// After the commit of `cancel`: an issued note's cost is put back and the
/// supplier is told; then the event.
async fn cancelled(state: &AppState, note_id: i64) -> Result {
    let note = SupplierDebitNote::find_or_404(&state.db, note_id).await?;
    let reason = note.cancel_reason.clone().unwrap_or_default();
    if let (Some(number), Some(bill_id)) = (note.number.clone(), note.bill_id) {
        let mut restore = cost_reversals(&state.db, &note).await?;
        restore.key = format!("{}:cancel", restore.key);
        if !restore.lines.is_empty() {
            cost_poster(state).restore(state, restore).await?;
        }
        let bill = SupplierBill::find_or_404(&state.db, bill_id).await?;
        let request = NotifyRequest::new(CANCELLED, note.subject())
            .subject_label(&number)
            .data(json!({
                "number": number,
                "amount": amount_text(&note.currency, note.gross_total),
                "bill": bill.supplier_invoice_number,
                "reason": reason,
            }))
            .to_party(note.supplier());
        notify::notifier(state).notify(state, request).await?;
    }
    state
        .emit(SupplierDebitNoteCancelled {
            debit_note: note.debit_note_id(),
            supplier: note.supplier(),
            bill: note.bill(),
            return_to_supplier: note.source(),
            amount: note.gross_total,
            reason,
        })
        .await
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<SupplierDebitNote>, state: AppState| async move {
            match event.transition.as_str() {
                ISSUE | ISSUE_SYSTEM => issued(&state, event.key).await,
                SETTLE | SETTLE_SYSTEM => settled(&state, event.key).await,
                CANCEL | CANCEL_SYSTEM => cancelled(&state, event.key).await,
                _ => Ok(()),
            }
        },
    );
    app.listen(
        |event: Transitioned<SupplierBill>, state: AppState| async move {
            if matches!(event.transition.as_str(), APPROVE | APPROVE_SYSTEM) {
                let bill = SupplierBill::find_or_404(&state.db, event.key).await?;
                crate::debit_note_contract::link_waiting(&state.db, &bill).await?;
            }
            Ok(())
        },
    );
    app.listen(|event: SupplierReturnDispatched, state: AppState| async move {
        crate::debit_note_contract::draft_for_return(&state, event)
            .await
            .map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cost_reversal_of_a_note_is_keyed_by_the_note() {
        let note = SupplierDebitNote {
            id: 7,
            ..Default::default()
        };
        assert_eq!(reversal_key(&note), "invoicing.supplier_debit_note:7");
        assert_eq!(ISSUED, "invoicing.supplier_debit_note_issued");
    }
}
