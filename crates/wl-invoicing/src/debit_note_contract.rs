//! Debit notes for goods returned to a supplier (wl-invoicing §5.13 steps 1
//! and 2, `FIN.RUL.17`) and the kernel's [`SupplierDebitNotes`] contract.
//!
//! When wl-inventory dispatches a return ([`SupplierReturnDispatched`]), a
//! debit note is drafted for the returned lines, once per return (a repeat
//! of the event gives the existing draft; the source is unique):
//!
//! - **The bill exists.** The oldest `approved`, `partially_paid` or `paid`
//!   bill of the supplier that has a line for every returned PO line is the
//!   note's bill. Each returned line takes that bill line's price, VAT
//!   category and cost object (priced at the bill).
//! - **The bill has not arrived.** The draft has no bill and waits. Its lines
//!   take the PO price of the event (priced at the PO, standard VAT, in AED),
//!   and the receiving entity is the default one. When a bill of the supplier
//!   is approved ([`link_waiting`]), the draft links to it if that bill has a
//!   line for every returned PO line, and its lines are re-priced at the
//!   bill's prices.
//!
//! The accountant (the holders of the notify role, default `accountant`) is
//! told of every new draft. A draft from a return is not checked against the
//! bill lines until `issue`: the goods have left already.
//!
//! Decisions:
//!
//! - A bill that covers only some of the returned PO lines is not taken; the
//!   draft waits for one that covers them all.
//! - Without a bill the currency is AED, because the event gives the price
//!   in the PO's currency without naming it.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::LegalEntity;
use wl_kernel::PartyId;
use wl_kernel::inventory::{
    PURCHASE_ORDER_LINE_SUBJECT_TYPE, ReturnOutcome, SupplierReturnDispatched,
    SupplierReturnDispatchedLine,
};
use wl_kernel::invoicing::{
    SupplierBillFuture, SupplierDebitNoteId, SupplierDebitNoteStatus, SupplierDebitNoteSummary,
    SupplierDebitNotes, VatCategory,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::bill_line::BillLine;
use crate::config_of;
use crate::debit_note_line::{DebitNoteLine, PricedAt};
use crate::debit_notes::{DEBITABLE, lines_of, totals_of_lines};
use crate::supplier_bill::{self, SupplierBill};
use crate::supplier_debit_note::{DebitNoteOrigin, SupplierDebitNote};
use crate::texts::english;

/// The notification type of a draft made from a return.
pub const DRAFTED: &str = "invoicing.supplier_debit_note_drafted";

/// Registers [`DRAFTED`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(
        NotificationTypeDef::new(DRAFTED, "invoicing")
            .group("supplier_bills")
            .category(Category::Transactional)
            .template(
                NotifyChannel::Email,
                "en",
                Some(english("wl_invoicing.debit_note_drafted.subject")),
                english("wl_invoicing.debit_note_drafted.body"),
            ),
    );
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A returned line: the PO line it came in on and the quantity.
type Wanted = (wl_kernel::Subject, i64);

/// The lines of `event` that ask for a credit: a line that asks for a
/// replacement gets new goods, not a debit note.
pub fn credit_lines(event: &SupplierReturnDispatched) -> Vec<&SupplierReturnDispatchedLine> {
    event
        .lines
        .iter()
        .filter(|line| line.wanted_outcome == ReturnOutcome::Credit)
        .collect()
}

/// The order line of `line` as a [`wl_kernel::Subject`], if it names one.
fn po_line_of(line: &SupplierReturnDispatchedLine) -> Option<wl_kernel::Subject> {
    line.po_line
        .map(|id| wl_kernel::Subject::new(PURCHASE_ORDER_LINE_SUBJECT_TYPE, id))
}

/// The returned lines as PO line and quantity. A line without an order line
/// cannot be matched to a bill line; it is left out here, so the draft finds
/// no covering bill and is priced at the PO.
fn wanted_of(returned: &[&SupplierReturnDispatchedLine]) -> Vec<Wanted> {
    returned
        .iter()
        .filter_map(|r| Some((po_line_of(r)?, r.quantity_milli)))
        .collect()
}

/// Whether `bill_lines` has a line for every PO line in `wanted`.
fn covers(wanted: &[Wanted], bill_lines: &[BillLine]) -> bool {
    !wanted.is_empty()
        && wanted
            .iter()
            .all(|(po, _)| bill_lines.iter().any(|l| l.po_line().as_ref() == Some(po)))
}

/// The first of `bills` (oldest first) that has a line for every PO line in
/// `wanted`, with its lines.
async fn covering_bill(
    db: &Db,
    bills: Vec<SupplierBill>,
    wanted: &[Wanted],
) -> Result<Option<(SupplierBill, Vec<BillLine>)>> {
    for bill in bills {
        let lines = supplier_bill::lines_of(db, bill.id).await?;
        if covers(wanted, &lines) {
            return Ok(Some((bill, lines)));
        }
    }
    Ok(None)
}

/// The lines of the draft for `wanted` at the prices of `bill_lines`.
fn lines_at_bill(
    wanted: &[Wanted],
    bill_lines: &[BillLine],
    rate: Option<i64>,
) -> Vec<DebitNoteLine> {
    wanted
        .iter()
        .zip(1..)
        .filter_map(|((po, quantity_milli), position)| {
            let bill_line = bill_lines
                .iter()
                .find(|l| l.po_line().as_ref() == Some(po))?;
            Some(DebitNoteLine::of_bill_line(
                0,
                position,
                bill_line,
                *quantity_milli,
                bill_line.unit_price,
                rate,
            ))
        })
        .collect()
}

/// The lines of the draft for `returned` at the PO price, for a note that
/// waits for its bill.
fn lines_at_po(returned: &[&SupplierReturnDispatchedLine]) -> Vec<DebitNoteLine> {
    returned
        .iter()
        .zip(1..)
        .map(|(r, position)| {
            let po_line = po_line_of(r);
            let mut line = DebitNoteLine {
                position,
                po_line_type: po_line.as_ref().map(|p| p.subject_type.clone()),
                po_line_id: po_line.map(|p| p.subject_id),
                description: r.item.to_string(),
                quantity_milli: r.quantity_milli,
                unit_price: r.po_price,
                vat_category: VatCategory::Standard,
                priced_at: PricedAt::Po,
                ..Default::default()
            };
            line.price(Some(crate::vat::RATE_SCALE));
            line
        })
        .collect()
}

/// Drafts the debit note for `event` (`FIN.RUL.17`), once per source, and
/// tells the accountant. Returns the note and whether it was just made.
///
/// # Errors
///
/// 422 for a return with no line asking for a credit or a line of 0 or less.
pub async fn draft_for_return(
    state: &AppState,
    event: SupplierReturnDispatched,
) -> Result<(SupplierDebitNote, bool)> {
    let db = &state.db;
    let existing = SupplierDebitNote::query()
        .where_(SupplierDebitNote::SOURCE_TYPE.eq(event.source.subject_type.clone()))
        .where_(SupplierDebitNote::SOURCE_ID.eq(event.source.subject_id))
        .first(db)
        .await?;
    if let Some(note) = existing {
        return Ok((note, false));
    }
    let returned = credit_lines(&event);
    if returned.is_empty() || returned.iter().any(|l| l.quantity_milli <= 0) {
        return Err(refused(english("wl_invoicing.debit_note.errors.lines")));
    }
    let bills = SupplierBill::query()
        .where_(SupplierBill::PARTY_ID.eq(event.supplier.0))
        .where_(SupplierBill::STATUS.is_in(DEBITABLE))
        .order_by(SupplierBill::ID)
        .get(db)
        .await?;
    let wanted = wanted_of(&returned);
    let found = covering_bill(db, bills, &wanted).await?;
    let mut note = SupplierDebitNote {
        party_id: event.supplier.0,
        origin: DebitNoteOrigin::Return,
        source_type: Some(event.source.subject_type.clone()),
        source_id: Some(event.source.subject_id),
        reason: returned
            .iter()
            .map(|l| l.reason.trim())
            .filter(|r| !r.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        status: SupplierDebitNoteStatus::Draft,
        ..Default::default()
    };
    let mut lines = match found {
        Some((bill, bill_lines)) => {
            note.legal_entity_id = match bill.legal_entity_id {
                Some(id) => id,
                None => LegalEntity::default_entity(db).await?.id,
            };
            note.bill_id = Some(bill.id);
            note.currency = bill.currency.clone();
            note.exchange_rate_micro = bill.exchange_rate_micro;
            lines_at_bill(&wanted, &bill_lines, bill.aed_rate())
        }
        None => {
            note.legal_entity_id = LegalEntity::default_entity(db).await?.id;
            note.currency = "AED".to_owned();
            lines_at_po(&returned)
        }
    };
    note.set_totals(&totals_of_lines(&lines));
    let mut tx = db.begin().await?;
    note.insert(&mut tx).await?;
    for line in &mut lines {
        line.debit_note_id = note.id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    tell_accountant(state, &note).await?;
    Ok((note, true))
}

/// Tells the holders of the notify role that `note` was drafted from a
/// return.
async fn tell_accountant(state: &AppState, note: &SupplierDebitNote) -> Result {
    let config = config_of(state);
    let mut ids: Vec<i64> =
        users_with_role_in(&state.db, &config.debit_note_notify_role, &Scope::global())
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let supplier = wl_kernel::parties::directory(state)
        .billing_identity(&state.db, note.supplier())
        .await?
        .map(|identity| identity.legal_name_en)
        .unwrap_or_default();
    let waiting = if note.bill_id.is_some() {
        "priced at the bill"
    } else {
        "waiting for the bill, priced at the PO"
    };
    let mut request = NotifyRequest::new(DRAFTED, note.subject()).data(json!({
        "supplier": supplier,
        "amount": amount_text(&note.currency, note.gross_total),
        "state": waiting,
        "link": format!("/supplier-debit-notes/{}", note.id),
    }));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Links the drafts that wait for a bill of the supplier of `bill` to it
/// when it has a line for every returned PO line, and re-prices their lines
/// at the bill's prices. Run after the bill is approved. Returns how many
/// drafts were linked.
///
/// # Errors
///
/// The database's errors.
pub async fn link_waiting(db: &Db, bill: &SupplierBill) -> Result<usize> {
    if !DEBITABLE.contains(&bill.status) {
        return Ok(0);
    }
    let waiting = SupplierDebitNote::query()
        .where_(SupplierDebitNote::PARTY_ID.eq(bill.party_id))
        .where_(SupplierDebitNote::STATUS.eq(SupplierDebitNoteStatus::Draft))
        .where_(SupplierDebitNote::ORIGIN.eq(DebitNoteOrigin::Return))
        .where_(SupplierDebitNote::BILL_ID.is_null())
        .order_by(SupplierDebitNote::ID)
        .get(db)
        .await?;
    if waiting.is_empty() {
        return Ok(0);
    }
    let bill_lines = supplier_bill::lines_of(db, bill.id).await?;
    let mut linked = 0;
    for mut note in waiting {
        let old = lines_of(db, note.id).await?;
        let wanted: Vec<Wanted> = old
            .iter()
            .filter_map(|l| Some((l.po_line()?, l.quantity_milli)))
            .collect();
        // Every line must be known by its PO line to be matched.
        if wanted.len() != old.len() || !covers(&wanted, &bill_lines) {
            continue;
        }
        let mut fresh = lines_at_bill(&wanted, &bill_lines, bill.aed_rate());
        note.bill_id = Some(bill.id);
        note.legal_entity_id = bill.legal_entity_id.unwrap_or(note.legal_entity_id);
        note.currency = bill.currency.clone();
        note.exchange_rate_micro = bill.exchange_rate_micro;
        note.set_totals(&totals_of_lines(&fresh));
        let mut tx = db.begin().await?;
        note.save(&mut tx).await?;
        for mut line in old {
            line.delete(&mut tx).await?;
        }
        for line in &mut fresh {
            line.debit_note_id = note.id;
            line.insert(&mut tx).await?;
        }
        tx.commit().await?;
        linked += 1;
    }
    Ok(linked)
}

/// The module's [`SupplierDebitNotes`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DebitNotes;

impl SupplierDebitNotes for DebitNotes {
    fn draft_for_return<'a>(
        &'a self,
        state: &'a AppState,
        event: SupplierReturnDispatched,
    ) -> SupplierBillFuture<'a, SupplierDebitNoteId> {
        Box::pin(async move {
            let (note, _) = draft_for_return(state, event).await?;
            Ok(SupplierDebitNoteId(note.id))
        })
    }

    fn find<'a>(
        &'a self,
        db: &'a Db,
        id: SupplierDebitNoteId,
    ) -> SupplierBillFuture<'a, Option<SupplierDebitNoteSummary>> {
        Box::pin(async move {
            Ok(SupplierDebitNote::find(db, id.0)
                .await?
                .map(|n| n.summary()))
        })
    }

    fn debit_balance<'a>(
        &'a self,
        db: &'a Db,
        supplier: PartyId,
        entity: i64,
        currency: &'a str,
    ) -> SupplierBillFuture<'a, i64> {
        Box::pin(async move { debit_balance_of(db, supplier, entity, currency).await })
    }

    fn settle_from_payment<'a>(
        &'a self,
        state: &'a AppState,
        id: SupplierDebitNoteId,
        payment: wl_kernel::Subject,
        amount: i64,
        key: &'a str,
    ) -> SupplierBillFuture<'a, ()> {
        Box::pin(async move {
            crate::debit_note_settle::refund(state, id.0, &payment, amount, key).await
        })
    }

    fn cancel<'a>(
        &'a self,
        state: &'a AppState,
        id: SupplierDebitNoteId,
        reason: &'a str,
        by: Option<i64>,
    ) -> SupplierBillFuture<'a, ()> {
        Box::pin(async move {
            crate::debit_note_settle::cancel(
                state,
                crate::credit_notes::By::System(by),
                id.0,
                reason,
            )
            .await
            .map(|_| ())
            .map_err(|e| e.into_error())
        })
    }
}

/// What `supplier` has open as a debit balance with `entity` in `currency`:
/// the sum over its `issued` notes.
///
/// # Errors
///
/// The database's errors.
pub async fn debit_balance_of(
    db: &Db,
    supplier: PartyId,
    entity: i64,
    currency: &str,
) -> Result<i64> {
    Ok(SupplierDebitNote::query()
        .where_(SupplierDebitNote::PARTY_ID.eq(supplier.0))
        .where_(SupplierDebitNote::LEGAL_ENTITY_ID.eq(entity))
        .where_(SupplierDebitNote::CURRENCY.eq(currency.to_owned()))
        .where_(SupplierDebitNote::STATUS.eq(SupplierDebitNoteStatus::Issued))
        .get(db)
        .await?
        .iter()
        .map(|n| n.debit_balance)
        .sum())
}
