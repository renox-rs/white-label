//! Drafting supplier debit notes (wl-invoicing §3, #76 A4 and A6): the manual
//! draft against a bill, and changing or discarding a draft. The amounts of
//! every line go through [`crate::vat`]; the bill's own lines, totals and VAT
//! are never written. The status changes only through
//! [`crate::debit_note_machine`]; a note is inserted only as `draft`.
//!
//! A line may not take more than what is left on its bill line: the quantity
//! and the net of the bill line less the lines of the bill's `issued` and
//! `settled` debit notes (draft notes do not count). [`caps_problem`] is the
//! check, run at draft here and again at `issue`.

use std::collections::HashMap;

use renox::prelude::*;
use wl_kernel::invoicing::SupplierDebitNoteStatus;

use crate::bill_line::BillLine;
use crate::debit_note_line::DebitNoteLine;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};
use crate::supplier_debit_note::{DebitNoteOrigin, DebitNoteReason, SupplierDebitNote};
use crate::texts::english;
use crate::vat::{self, VatTotals};

/// The bill states a debit note can be raised against.
pub const DEBITABLE: [SupplierBillStatus; 3] = [
    SupplierBillStatus::Approved,
    SupplierBillStatus::PartiallyPaid,
    SupplierBillStatus::Paid,
];

/// What a user enters for one line of a manual debit note: a bill line, a
/// quantity and a unit price (the difference for a price difference, the
/// missing quantity for a short delivery).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebitLine {
    /// The bill line it reduces.
    pub bill_line_id: i64,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit, before VAT, in fils.
    pub unit_price: i64,
}

impl DebitLine {
    /// A line taking `quantity_milli` at `unit_price` off `bill_line_id`.
    pub fn new(bill_line_id: i64, quantity_milli: i64, unit_price: i64) -> Self {
        Self {
            bill_line_id,
            quantity_milli,
            unit_price,
        }
    }
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The quantity and the net already taken off each bill line of `bill_id`
/// by its `issued` and `settled` debit notes, other than `except` (a note
/// being checked again at `issue`).
pub async fn debited_by_line(
    db: &Db,
    bill_id: i64,
    except: Option<i64>,
) -> Result<HashMap<i64, (i64, i64)>> {
    let counted: Vec<i64> = SupplierDebitNote::query()
        .where_(SupplierDebitNote::BILL_ID.eq(bill_id))
        .where_(SupplierDebitNote::STATUS.is_in([
            SupplierDebitNoteStatus::Issued,
            SupplierDebitNoteStatus::Settled,
        ]))
        .get(db)
        .await?
        .into_iter()
        .map(|n| n.id)
        .filter(|id| Some(*id) != except)
        .collect();
    if counted.is_empty() {
        return Ok(HashMap::new());
    }
    let lines = DebitNoteLine::query()
        .where_(DebitNoteLine::DEBIT_NOTE_ID.is_in(counted))
        .get(db)
        .await?;
    let mut by_line: HashMap<i64, (i64, i64)> = HashMap::new();
    for line in lines {
        if let Some(bill_line_id) = line.bill_line_id {
            let entry = by_line.entry(bill_line_id).or_insert((0, 0));
            entry.0 += line.quantity_milli;
            entry.1 += line.net_amount;
        }
    }
    Ok(by_line)
}

/// Why `lines` (those of one note) take more than is left on the bill
/// lines in `bill_lines`, if they do; `except` is the note's own id when it
/// is checked again at `issue`. A line with no bill line is skipped.
pub async fn caps_problem(
    db: &Db,
    bill_id: i64,
    bill_lines: &[BillLine],
    lines: &[DebitNoteLine],
    except: Option<i64>,
) -> Result<Option<String>> {
    let debited = debited_by_line(db, bill_id, except).await?;
    let mut wanted: HashMap<i64, (i64, i64)> = HashMap::new();
    for line in lines {
        if let Some(bill_line_id) = line.bill_line_id {
            let entry = wanted.entry(bill_line_id).or_insert((0, 0));
            entry.0 += line.quantity_milli;
            entry.1 += line.net_amount;
        }
    }
    for bill_line in bill_lines {
        let Some((quantity, net)) = wanted.get(&bill_line.id) else {
            continue;
        };
        let (done_quantity, done_net) = debited.get(&bill_line.id).copied().unwrap_or((0, 0));
        if *quantity > vat::remaining_debit(bill_line.quantity_milli, done_quantity)
            || *net > vat::remaining_debit(bill_line.net_amount, done_net)
        {
            return Ok(Some(format!(
                "{} (line {})",
                english("wl_invoicing.debit_note.errors.over_bill_line"),
                bill_line.position
            )));
        }
    }
    Ok(None)
}

/// The checked reason code and text of a manual note.
fn reason_of(code: &str, text: &str) -> Result<(DebitNoteReason, String)> {
    let reason = DebitNoteReason::parse(code.trim())
        .ok_or_else(|| refused(format!("`{code}` is not a debit note reason")))?;
    let text = text.trim().to_owned();
    if reason == DebitNoteReason::Other && text.is_empty() {
        return Err(refused(english("wl_invoicing.debit_note.errors.reason")));
    }
    Ok((reason, text))
}

/// The lines of a manual note on `bill` for `drafts` (not yet saved, ids and
/// note id unset), or why they cannot be saved.
async fn build_lines(
    db: &Db,
    bill: &SupplierBill,
    drafts: &[DebitLine],
    except: Option<i64>,
) -> Result<Vec<DebitNoteLine>> {
    if drafts.is_empty() {
        return Err(refused(english("wl_invoicing.debit_note.errors.lines")));
    }
    let bill_lines = crate::supplier_bill::lines_of(db, bill.id).await?;
    let rate = bill.aed_rate();
    let mut lines = Vec::with_capacity(drafts.len());
    for (draft, position) in drafts.iter().zip(1..) {
        let Some(bill_line) = bill_lines.iter().find(|l| l.id == draft.bill_line_id) else {
            return Err(refused(format!(
                "line {position} is not a line of the supplier bill"
            )));
        };
        if draft.quantity_milli <= 0 {
            return Err(refused(format!(
                "line {position} has a quantity of 0 or less"
            )));
        }
        if draft.unit_price <= 0 {
            return Err(refused(format!("line {position} has a price of 0 or less")));
        }
        lines.push(DebitNoteLine::of_bill_line(
            0,
            position,
            bill_line,
            draft.quantity_milli,
            draft.unit_price,
            rate,
        ));
    }
    if let Some(message) = caps_problem(db, bill.id, &bill_lines, &lines, except).await? {
        return Err(refused(message));
    }
    Ok(lines)
}

/// Creates a `draft` manual debit note against `bill` with its lines and
/// totals in one transaction. The receiving entity, currency and rate are
/// the bill's; each line takes the VAT category and cost object of its bill
/// line.
///
/// Refused (422) for a bill that is not `approved`, `partially_paid` or
/// `paid`, an unknown reason code, a blank reason with code `other`, no
/// lines, a line that is not on the bill, a quantity or price of 0 or less,
/// or a line above what is left on its bill line.
pub async fn draft_manual(
    db: &Db,
    bill: &SupplierBill,
    reason_code: &str,
    reason: &str,
    lines: &[DebitLine],
    created_by: Option<i64>,
) -> Result<SupplierDebitNote> {
    let bill = SupplierBill::find_or_404(db, bill.id).await?;
    if !DEBITABLE.contains(&bill.status) {
        return Err(refused(english(
            "wl_invoicing.debit_note.errors.bill_state",
        )));
    }
    let (code, text) = reason_of(reason_code, reason)?;
    let legal_entity_id = bill
        .legal_entity_id
        .ok_or_else(|| refused("the supplier bill has no receiving entity"))?;
    let mut built = build_lines(db, &bill, lines, None).await?;
    let mut note = SupplierDebitNote {
        legal_entity_id,
        party_id: bill.party_id,
        bill_id: Some(bill.id),
        origin: DebitNoteOrigin::Manual,
        reason_code: Some(code.as_str().to_owned()),
        reason: text,
        currency: bill.currency.clone(),
        exchange_rate_micro: bill.exchange_rate_micro,
        created_by,
        status: SupplierDebitNoteStatus::Draft,
        ..Default::default()
    };
    note.set_totals(&totals_of_lines(&built));
    let mut tx = db.begin().await?;
    note.insert(&mut tx).await?;
    for line in &mut built {
        line.debit_note_id = note.id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(note)
}

/// The totals of `lines`.
pub fn totals_of_lines(lines: &[DebitNoteLine]) -> VatTotals {
    let amounts: Vec<_> = lines.iter().map(DebitNoteLine::amounts).collect();
    VatTotals::of(&amounts)
}

/// The draft manual note `id`, or 422.
async fn manual_draft(db: &Db, id: i64) -> Result<SupplierDebitNote> {
    let note = SupplierDebitNote::find_or_404(db, id).await?;
    if note.status != SupplierDebitNoteStatus::Draft || note.origin != DebitNoteOrigin::Manual {
        return Err(refused(english("wl_invoicing.debit_note.errors.not_draft")));
    }
    Ok(note)
}

/// Replaces the reason and the lines of manual draft `id`, with totals, in
/// one transaction. Same refusals as [`draft_manual`], and 422 when it is
/// no longer a `draft` or did not come from a user.
pub async fn change(
    db: &Db,
    id: i64,
    reason_code: &str,
    reason: &str,
    lines: &[DebitLine],
) -> Result<SupplierDebitNote> {
    let mut note = manual_draft(db, id).await?;
    let bill_id = note
        .bill_id
        .ok_or_else(|| refused("the debit note has no supplier bill"))?;
    let bill = SupplierBill::find_or_404(db, bill_id).await?;
    if !DEBITABLE.contains(&bill.status) {
        return Err(refused(english(
            "wl_invoicing.debit_note.errors.bill_state",
        )));
    }
    let (code, text) = reason_of(reason_code, reason)?;
    let mut built = build_lines(db, &bill, lines, Some(id)).await?;
    note.reason_code = Some(code.as_str().to_owned());
    note.reason = text;
    note.set_totals(&totals_of_lines(&built));
    let mut tx = db.begin().await?;
    note.save(&mut tx).await?;
    delete_lines(&mut tx, id).await?;
    for line in &mut built {
        line.debit_note_id = id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(note)
}

/// Deletes manual draft `id` and its lines: nothing else changes, as a
/// draft has no effect on the bill, the cost or the VAT. Refused (422) when
/// it is no longer a `draft`: an issued note is never deleted.
pub async fn discard(db: &Db, id: i64) -> Result<()> {
    let mut note = manual_draft(db, id).await?;
    let mut tx = db.begin().await?;
    delete_lines(&mut tx, id).await?;
    note.delete(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Deletes the lines of note `id` in `tx`.
async fn delete_lines(tx: &mut renox::db::Transaction, id: i64) -> Result<()> {
    let old = DebitNoteLine::query()
        .where_(DebitNoteLine::DEBIT_NOTE_ID.eq(id))
        .get(&mut *tx)
        .await?;
    for mut line in old {
        line.delete(&mut *tx).await?;
    }
    Ok(())
}

/// The lines of debit note `debit_note_id`, in order.
pub async fn lines_of(db: &Db, debit_note_id: i64) -> Result<Vec<DebitNoteLine>> {
    DebitNoteLine::query()
        .where_(DebitNoteLine::DEBIT_NOTE_ID.eq(debit_note_id))
        .order_by(DebitNoteLine::POSITION)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_checked() {
        let (reason, text) = reason_of("price_difference", "  ").unwrap();
        assert_eq!(
            (reason, text.as_str()),
            (DebitNoteReason::PriceDifference, "")
        );
        assert!(reason_of("other", "   ").is_err());
        assert!(reason_of("bogus", "x").is_err());
        let (reason, text) = reason_of(" other ", " lost in transit ").unwrap();
        assert_eq!(
            (reason, text.as_str()),
            (DebitNoteReason::Other, "lost in transit")
        );
    }

    #[test]
    fn the_totals_are_the_sum_of_the_lines() {
        let line = |net: i64| {
            let mut l = DebitNoteLine {
                quantity_milli: 1_000,
                unit_price: net,
                ..Default::default()
            };
            l.vat_category = wl_kernel::invoicing::VatCategory::Standard;
            l.price(Some(1_000_000));
            l
        };
        let totals = totals_of_lines(&[line(30_000), line(10_000)]);
        assert_eq!(
            (totals.net, totals.vat, totals.gross),
            (40_000, 2_000, 42_000)
        );
    }
}
