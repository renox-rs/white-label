//! Recording the supplier's tax credit note, setting the debit balance off,
//! settling and cancelling a supplier debit note (wl-invoicing §4 `settle`
//! and `cancel`, §5.13 steps 4 and 5, #77).
//!
//! - [`record_tax_credit_note`] stores the number, date and file of the
//!   supplier's own VAT document, closes the accountant's task, and settles
//!   the note when its whole amount is already applied: the user who records
//!   it settles it when that comes last.
//! - [`set_off`] applies the debit balance to an `approved` or
//!   `partially_paid` bill of the same supplier, receiving entity and
//!   currency; that bill runs `pay` for the amount, with the debit note as the
//!   source. Set off moves money only, so it may come before the tax credit
//!   note.
//! - [`refund`] is what wl-payments calls (through the kernel's
//!   `settle_from_payment`) when the supplier's payment back is allocated to
//!   the note. It is a refund, not a payment of a bill. A key that was used
//!   changes nothing.
//! - [`settle`] settles by hand, accepting what the supplier paid short as a
//!   difference with a reason. The system settles by itself when the last set
//!   off or refund leaves nothing open and the tax credit note is recorded (or
//!   the client's setting accepts the debit note alone): with the default
//!   setting a note applied in full that still lacks its tax credit note stays
//!   `issued`, and the accountant's task stays open ([`TaskKind::RecordTaxCreditNote`],
//!   opened at `issue`).
//! - [`cancel`] withdraws a draft or issued note with a reason. An issued one
//!   gives back everything it did: every bill it reduced runs `unallocate`
//!   for what was applied from the note (also from `paid`, reason *debit note
//!   cancelled*), its debit balance is closed, the cost is put back and the
//!   supplier is told ([`crate::debit_note_events`]); the input VAT it took
//!   back stops counting by itself, as a cancelled note counts nowhere
//!   ([`crate::debit_note_vat`]). A note the supplier has already paid back
//!   is not cancelled.
//!
//! A user needs `supplier_bills.approve`; the system acts for whoever caused
//! it. Every service runs in one transaction and emits the events after the
//! commit.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Outcome, TransitionError};
use wl_kernel::Subject;
use wl_kernel::invoicing::{BillReopenReason, SupplierBillId, SupplierDebitNoteStatus, UnallocateBill};
use wl_kernel::notify::fill;

use crate::allocate::amount_text;
use crate::bill_allocation::BillAllocation;
use crate::bill_pay::pay_in;
use crate::bill_unallocate::unallocate_in;
use crate::credit_notes::{By, run_of};
use crate::debit_note_machine::{
    CANCEL, CANCEL_SYSTEM, CancelInput, SETTLE, SETTLE_SYSTEM, SettleInput,
};
use crate::debit_note_movement::{DebitMovementKind, DebitNoteMovement};
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};
use crate::supplier_debit_note::SupplierDebitNote;
use crate::task::{ACCOUNTANT, InvoicingTask, TaskKind, TaskStatus};
use crate::texts::english;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A user needs `supplier_bills.approve`; the system needs nothing.
fn check_permission(by: &By<'_>) -> Result {
    if let By::User(user) = by
        && !user.has_permission_anywhere(crate::permissions::SUPPLIER_BILLS_APPROVE)
    {
        return Err(Error::Forbidden);
    }
    Ok(())
}

/// The key of the task that asks for the tax credit note of `note`.
pub fn task_key(note: &SupplierDebitNote) -> String {
    format!("{}:tax_credit_note", note.subject())
}

/// Opens (once) the accountant's task to record the supplier's tax credit
/// note of the issued `note`, when the setting requires it. Returns whether a
/// task was opened.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn open_tax_credit_task(state: &AppState, note: &SupplierDebitNote) -> Result<bool> {
    if !crate::config_of(state).tax_credit_note_required {
        return Ok(false);
    }
    let message = fill(
        english("wl_invoicing.task.record_tax_credit_note"),
        &json!({
            "number": note.number.clone().unwrap_or_default(),
            "amount": amount_text(&note.currency, note.gross_total),
        }),
    );
    let mut task = InvoicingTask {
        kind: TaskKind::RecordTaxCreditNote,
        bill_id: note.bill_id,
        source_type: note.subject().subject_type,
        source_id: note.id,
        amount: note.gross_total,
        key: task_key(note),
        message,
        role: ACCOUNTANT.to_owned(),
        status: TaskStatus::Open,
        ..Default::default()
    };
    match task.insert(&state.db).await {
        Ok(()) => Ok(true),
        Err(error) if error.is_unique_violation() => Ok(false),
        Err(error) => Err(error),
    }
}

/// Closes the task to record the tax credit note of `note`, if it is open.
async fn close_tax_credit_task(db: &Db, note: &SupplierDebitNote, by: Option<i64>) -> Result {
    let found = InvoicingTask::query()
        .where_(InvoicingTask::KIND.eq(TaskKind::RecordTaxCreditNote))
        .where_(InvoicingTask::KEY.eq(task_key(note)))
        .first(db)
        .await?;
    if let Some(mut task) = found.filter(|t| t.status == TaskStatus::Open) {
        task.status = TaskStatus::Done;
        task.done_by = by;
        task.done_at = Some(renox::db::now());
        task.save_only(db, &["status", "done_by", "done_at"]).await?;
    }
    Ok(())
}

/// The open tasks of debit note `note`, oldest first.
pub async fn open_tasks(db: &Db, note: &SupplierDebitNote) -> Result<Vec<InvoicingTask>> {
    InvoicingTask::query()
        .where_(InvoicingTask::KIND.eq(TaskKind::RecordTaxCreditNote))
        .where_(InvoicingTask::KEY.eq(task_key(note)))
        .where_(InvoicingTask::STATUS.eq(TaskStatus::Open))
        .order_by(InvoicingTask::ID)
        .get(db)
        .await
}

/// Runs `settle` on issued `note` in `tx`. `difference` is the part the
/// supplier paid short and the reason it is accepted.
pub(crate) async fn settle_in(
    tx: &mut renox::db::Transaction,
    note: &SupplierDebitNote,
    required: bool,
    difference: Option<(i64, &str)>,
    by: &By<'_>,
) -> std::result::Result<Outcome<SupplierDebitNote>, TransitionError> {
    let input = SettleInput {
        tax_credit_note_required: required,
        difference: difference.map_or(0, |(amount, _)| amount),
        difference_reason: difference.map(|(_, reason)| reason.to_owned()),
        by: by.id(),
    };
    run_of(note, by, SETTLE, SETTLE_SYSTEM, json!({}))
        .input(&input)
        .run_in(&mut *tx)
        .await
}

/// Whether issued `note` is applied in full and, when the setting needs it,
/// has its tax credit note: the system settles it then.
pub fn ready_to_settle(note: &SupplierDebitNote, required: bool) -> bool {
    note.status == SupplierDebitNoteStatus::Issued
        && note.debit_balance == 0
        && (!required || note.has_tax_credit_note())
}

/// Settles `note` in `tx` when [`ready_to_settle`].
pub(crate) async fn settle_if_ready(
    tx: &mut renox::db::Transaction,
    note: &SupplierDebitNote,
    required: bool,
    by: &By<'_>,
) -> std::result::Result<Option<Outcome<SupplierDebitNote>>, TransitionError> {
    if ready_to_settle(note, required) {
        Ok(Some(settle_in(tx, note, required, None, by).await?))
    } else {
        Ok(None)
    }
}

/// Emits `outcomes` after the commit.
async fn emit_all(
    state: &AppState,
    notes: Vec<Outcome<SupplierDebitNote>>,
    bills: Vec<Outcome<SupplierBill>>,
) -> std::result::Result<(), TransitionError> {
    for outcome in notes {
        outcome.emit(state).await.map_err(TransitionError::Failed)?;
    }
    for outcome in bills {
        outcome.emit(state).await.map_err(TransitionError::Failed)?;
    }
    Ok(())
}

/// The issued note `id`, or 422.
async fn issued_note(db: &Db, id: i64) -> Result<SupplierDebitNote> {
    let note = SupplierDebitNote::find_or_404(db, id).await?;
    if note.status != SupplierDebitNoteStatus::Issued {
        return Err(refused(english("wl_invoicing.debit_note.errors.settle_state")));
    }
    Ok(note)
}

/// Takes `amount` off the debit balance of `note` (as loaded), only from the
/// balance that was loaded, so two reductions at once never both pass.
async fn lower_balance_in(
    tx: &mut renox::db::Transaction,
    note: &SupplierDebitNote,
    amount: i64,
) -> Result {
    let moved = renox::db::sql(format!(
        "UPDATE {} SET debit_balance = debit_balance - ?, updated_at = ? \
         WHERE id = ? AND debit_balance = ?",
        SupplierDebitNote::TABLE
    ))
    .bind(amount)
    .bind(renox::db::now())
    .bind(note.id)
    .bind(note.debit_balance)
    .execute(&mut *tx)
    .await?;
    if moved == 0 {
        return Err(refused("The debit note changed; try again."));
    }
    Ok(())
}

/// Records the supplier's tax credit note `number` dated `date` (and its file,
/// a document id) on issued debit note `id`, and settles the note when its
/// whole amount is already applied.
///
/// # Errors
///
/// `Forbidden` for a user without `supplier_bills.approve`; 422 for a note
/// that is not `issued`, a blank number, or a tax credit note recorded
/// already.
pub async fn record_tax_credit_note(
    state: &AppState,
    by: By<'_>,
    id: i64,
    number: &str,
    date: NaiveDate,
    document: Option<i64>,
) -> std::result::Result<SupplierDebitNote, TransitionError> {
    check_permission(&by)?;
    let note = issued_note(&state.db, id).await?;
    if number.trim().is_empty() {
        return Err(refused(english("wl_invoicing.debit_note.errors.tcn_number")).into());
    }
    if note.has_tax_credit_note() {
        return Err(refused(english("wl_invoicing.debit_note.errors.tcn_twice")).into());
    }
    let required = crate::config_of(state).tax_credit_note_required;
    let mut tx = state.db.begin().await?;
    SupplierDebitNote::query()
        .where_(SupplierDebitNote::ID.eq(id))
        .update(
            &mut tx,
            &[
                ("tax_credit_note_number", &Some(number.trim().to_owned())),
                ("tax_credit_note_date", &Some(date)),
                ("tax_credit_note_document", &document),
                ("tax_credit_note_by", &by.id()),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    let stored = SupplierDebitNote::find_or_404(&mut tx, id).await?;
    let settled = settle_if_ready(&mut tx, &stored, required, &by).await?;
    tx.commit().await?;
    emit_all(state, settled.into_iter().collect(), Vec::new()).await?;
    close_tax_credit_task(&state.db, &stored, by.id()).await?;
    Ok(SupplierDebitNote::find_or_404(&state.db, id).await?)
}

/// Settles issued debit note `id` by hand, accepting `difference` (the part
/// the supplier paid short) with a reason when the whole amount is not
/// applied.
///
/// # Errors
///
/// `Forbidden` for a user without `supplier_bills.approve`; 422 for a note
/// that is not `issued`, a missing tax credit note (with the default setting),
/// a debit balance still open that the difference does not cover, or a
/// difference without a reason.
pub async fn settle(
    state: &AppState,
    by: By<'_>,
    id: i64,
    difference: Option<(i64, &str)>,
) -> std::result::Result<SupplierDebitNote, TransitionError> {
    let note = SupplierDebitNote::find_or_404(&state.db, id).await?;
    let required = crate::config_of(state).tax_credit_note_required;
    let mut tx = state.db.begin().await?;
    let outcome = settle_in(&mut tx, &note, required, difference, &by).await?;
    tx.commit().await?;
    emit_all(state, vec![outcome], Vec::new()).await?;
    Ok(SupplierDebitNote::find_or_404(&state.db, id).await?)
}

/// Sets `amount` fils of the debit balance of issued note `id` off on bill
/// `bill_id`, which runs `pay` for it. The note settles when nothing is left
/// and the tax credit note is recorded (or not required).
///
/// # Errors
///
/// `Forbidden` for a user without `supplier_bills.approve`; 422 for a note
/// that is not `issued`, a bill of another supplier, receiving entity or
/// currency, a bill that is not `approved` or `partially_paid`, or an amount
/// that is not above 0, within the debit balance and within the bill's
/// balance.
pub async fn set_off(
    state: &AppState,
    by: By<'_>,
    id: i64,
    bill_id: i64,
    amount: i64,
) -> std::result::Result<SupplierDebitNote, TransitionError> {
    check_permission(&by)?;
    let note = issued_note(&state.db, id).await?;
    let bill = SupplierBill::find_or_404(&state.db, bill_id).await?;
    let same_party = bill.party_id == note.party_id
        && bill.legal_entity_id == Some(note.legal_entity_id)
        && bill.currency == note.currency;
    let open = matches!(
        bill.status,
        SupplierBillStatus::Approved | SupplierBillStatus::PartiallyPaid
    );
    if !same_party || !open {
        return Err(refused(english("wl_invoicing.debit_note.errors.set_off_bill")).into());
    }
    if amount <= 0 || amount > note.debit_balance || amount > bill.balance {
        return Err(refused(english("wl_invoicing.debit_note.errors.set_off_amount")).into());
    }
    let required = crate::config_of(state).tax_credit_note_required;
    let mut tx = state.db.begin().await?;
    lower_balance_in(&mut tx, &note, amount).await?;
    let paid = pay_in(&mut tx, &bill, &note.subject(), amount, by.id()).await?;
    DebitNoteMovement::new(
        note.id,
        DebitMovementKind::SetOff,
        Some(&bill.subject()),
        amount,
        by.id(),
    )
    .insert(&mut tx)
    .await?;
    let stored = SupplierDebitNote::find_or_404(&mut tx, id).await?;
    let settled = settle_if_ready(&mut tx, &stored, required, &by).await?;
    tx.commit().await?;
    emit_all(state, settled.into_iter().collect(), vec![paid]).await?;
    Ok(SupplierDebitNote::find_or_404(&state.db, id).await?)
}

/// Applies `amount` fils the supplier paid back by `payment` (an incoming
/// payment allocated to the note) to issued note `id`, under the idempotency
/// `key`. The note settles when nothing is left and the tax credit note is
/// recorded (or not required); a smaller payment leaves the rest open, to be
/// settled with a difference.
///
/// # Errors
///
/// 422 for a note that is not `issued` or an amount that is not above 0 and
/// within the debit balance.
pub async fn refund(
    state: &AppState,
    id: i64,
    payment: &Subject,
    amount: i64,
    key: &str,
) -> Result {
    if DebitNoteMovement::by_key(&state.db, key).await?.is_some() {
        return Ok(());
    }
    let note = issued_note(&state.db, id).await?;
    if amount <= 0 || amount > note.debit_balance {
        return Err(refused(english("wl_invoicing.debit_note.errors.set_off_amount")));
    }
    let required = crate::config_of(state).tax_credit_note_required;
    let by = By::System(None);
    let mut tx = state.db.begin().await?;
    lower_balance_in(&mut tx, &note, amount).await?;
    let mut row = DebitNoteMovement::new(
        note.id,
        DebitMovementKind::Refund,
        Some(payment),
        amount,
        None,
    );
    row.key = Some(key.to_owned());
    match row.insert(&mut tx).await {
        Ok(()) => {}
        // The same delivery again: nothing changes.
        Err(error) if error.is_unique_violation() => return Ok(()),
        Err(error) => return Err(error),
    }
    let stored = SupplierDebitNote::find_or_404(&mut tx, id).await?;
    let settled = settle_if_ready(&mut tx, &stored, required, &by)
        .await
        .map_err(|e| e.into_error())?;
    tx.commit().await?;
    emit_all(state, settled.into_iter().collect(), Vec::new())
        .await
        .map_err(|e| e.into_error())
}

/// Cancels draft or issued debit note `id` with a `reason`. An issued note's
/// bills reopen (`unallocate` for what was applied from it), its debit
/// balance is closed, the cost is put back and the supplier is told.
///
/// # Errors
///
/// `Forbidden` for a user without `supplier_bills.approve`; 422 without a
/// reason, for a note that is already settled or cancelled, or one the
/// supplier has paid something back on.
pub async fn cancel(
    state: &AppState,
    by: By<'_>,
    id: i64,
    reason: &str,
) -> std::result::Result<SupplierDebitNote, TransitionError> {
    let note = SupplierDebitNote::find_or_404(&state.db, id).await?;
    let movements = DebitNoteMovement::of_note(&state.db, id).await?;
    let refunded: i64 = movements
        .iter()
        .filter(|m| m.kind == DebitMovementKind::Refund)
        .map(|m| m.amount)
        .sum();
    let input = CancelInput {
        refunded,
        by: by.id(),
    };
    let mut tx = state.db.begin().await?;
    let mut run = run_of(&note, &by, CANCEL, CANCEL_SYSTEM, json!({})).input(&input);
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let outcome = run.run_in(&mut tx).await?;
    let mut reopened = Vec::new();
    if note.status == SupplierDebitNoteStatus::Issued {
        let source = note.subject();
        let rows = BillAllocation::query()
            .where_(BillAllocation::SOURCE_TYPE.eq(source.subject_type.clone()))
            .where_(BillAllocation::SOURCE_ID.eq(source.subject_id))
            .where_(BillAllocation::REMOVED_AT.is_null())
            .order_by(BillAllocation::ID)
            .get(&mut tx)
            .await?;
        let mut per_bill: BTreeMap<i64, i64> = BTreeMap::new();
        for row in rows {
            *per_bill.entry(row.bill_id).or_default() += row.amount;
        }
        for (bill_id, amount) in per_bill {
            let bill = SupplierBill::find_or_404(&mut tx, bill_id).await?;
            let request = UnallocateBill::new(
                SupplierBillId(bill_id),
                source.clone(),
                amount,
                format!("{source}:cancel:{bill_id}"),
                BillReopenReason::DebitNoteCancelled,
            );
            reopened.push(unallocate_in(&mut tx, &bill, &request, by.id()).await?);
        }
    }
    tx.commit().await?;
    emit_all(state, vec![outcome], reopened).await?;
    close_tax_credit_task(&state.db, &note, by.id()).await?;
    Ok(SupplierDebitNote::find_or_404(&state.db, id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(status: SupplierDebitNoteStatus, balance: i64, tcn: bool) -> SupplierDebitNote {
        SupplierDebitNote {
            id: 5,
            status,
            debit_balance: balance,
            tax_credit_note_number: tcn.then(|| "TCN-1".to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn a_note_is_ready_when_applied_in_full_and_its_tax_credit_note_is_recorded() {
        let issued = SupplierDebitNoteStatus::Issued;
        assert!(ready_to_settle(&note(issued, 0, true), true));
        assert!(!ready_to_settle(&note(issued, 0, false), true));
        assert!(ready_to_settle(&note(issued, 0, false), false));
        assert!(!ready_to_settle(&note(issued, 100, true), true));
        assert!(!ready_to_settle(
            &note(SupplierDebitNoteStatus::Draft, 0, true),
            true
        ));
        assert!(!ready_to_settle(
            &note(SupplierDebitNoteStatus::Settled, 0, true),
            true
        ));
    }

    #[test]
    fn the_task_of_a_note_has_its_own_key() {
        assert_eq!(
            task_key(&note(SupplierDebitNoteStatus::Issued, 0, false)),
            "invoicing.supplier_debit_note:5:tax_credit_note"
        );
    }
}
