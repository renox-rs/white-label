//! The invoice of an order that is delivered in parts (wl-sales §5.2 step 4:
//! "the invoice is drafted on delivery or periodically, depending on
//! configuration"; BL-SAL-13).
//!
//! - Setting `invoice_on_delivery` **off** (the default): the invoice is
//!   drafted once, when the order becomes `fulfilled`, for what is not
//!   cancelled. A reversal of a delivery brings a draft back to the quantities
//!   that stay fulfilled; with none, the draft is discarded.
//! - Setting **on** (07): the draft is created at the first delivery and kept
//!   at the quantities the customer *accepted* (the item lines' fulfilled
//!   quantity, the discount in proportion); every later delivery, reversal
//!   and loss from review brings it up to date. The loyalty redemption line
//!   joins once nothing is open.
//! - **An issued invoice is never changed** (the invoicer refuses): the
//!   accountant gets a task naming the invoice and the order. A delivery
//!   accepted after the invoice was issued is not billed by this module; the
//!   periodic invoicing of the client picks it up (decision: the order keeps
//!   one invoice; a second one for a later delivery is a client billing
//!   choice, not made here).

use renox::prelude::*;
use wl_kernel::invoicing::{InvoiceDraft, InvoiceId, InvoiceStatus, invoicer};

use crate::config::config_of;
use crate::sale_line::{KIND_ITEM, SaleLine};
use crate::sales_order::SalesOrder;

/// What [`sync`] found and did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Synced {
    /// The order has no invoice and none was due.
    Nothing,
    /// A draft was created, or the order's draft brought up to date.
    Drafted,
    /// The draft was discarded: nothing stays delivered.
    Discarded,
    /// The invoice was issued already: it is not changed.
    Issued(i64),
}

/// The invoice draft of the order's lines as they stand.
pub(crate) fn invoice_draft(
    order: &SalesOrder,
    lines: &[SaleLine],
    on_delivery: bool,
) -> InvoiceDraft {
    let mut draft = InvoiceDraft::new(order.party(), order.subject()).currency(&order.currency);
    if let Some(entity) = order.legal_entity_id {
        draft = draft.legal_entity(entity);
    }
    let nothing_open = lines
        .iter()
        .filter(|line| line.kind == KIND_ITEM)
        .all(|line| line.open_milli() == 0);
    // With the invoice on delivery, or once an invoice was drafted and a
    // reversal opened quantity again: only what stays delivered is billed.
    let anything_delivered = lines
        .iter()
        .any(|line| line.kind == KIND_ITEM && line.fulfilled_milli > 0);
    let delivered_only =
        !nothing_open && (on_delivery || (order.invoice_id.is_some() && anything_delivered));
    for line in lines {
        let billed = if delivered_only {
            line.delivered_draft_line()
        } else {
            line.billable_draft_line()
        };
        if let Some(billed) = billed {
            draft = draft.line(billed.origin(order.subject()));
        }
    }
    draft
}

/// The invoice of `order` when it was issued: it is not changed by a
/// reversal, so the accountant is told.
///
/// # Errors
///
/// A database error.
pub async fn issued_invoice(state: &AppState, order: &SalesOrder) -> Result<Option<i64>> {
    let Some(invoice) = order.invoice_id else {
        return Ok(None);
    };
    let summary = invoicer(state).find(&state.db, InvoiceId(invoice)).await?;
    Ok(summary
        .filter(|summary| summary.status != InvoiceStatus::Draft)
        .map(|_| invoice))
}

/// Whether anything of the order is billed by `invoice_draft` now.
fn has_lines(order: &SalesOrder, lines: &[SaleLine], on_delivery: bool) -> bool {
    !invoice_draft(order, lines, on_delivery).lines.is_empty()
}

/// Brings the invoice of order `id` in line with what is delivered now: see
/// the module header.
///
/// # Errors
///
/// A database error.
pub async fn sync(state: &AppState, id: i64) -> Result<Synced> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let on_delivery = config_of(state).invoice_on_delivery;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let invoices = invoicer(state);
    let Some(invoice) = order.invoice_id else {
        if !on_delivery || !has_lines(&order, &lines, true) {
            return Ok(Synced::Nothing);
        }
        let draft = invoice_draft(&order, &lines, true);
        if let Some(created) = invoices.draft(&state.db, draft).await? {
            SalesOrder::query()
                .where_(SalesOrder::ID.eq(id))
                .update(&state.db, &[("invoice_id", &Some(created.0))])
                .await?;
            return Ok(Synced::Drafted);
        }
        return Ok(Synced::Nothing);
    };
    let invoice_id = InvoiceId(invoice);
    let Some(summary) = invoices.find(&state.db, invoice_id).await? else {
        return Ok(Synced::Nothing);
    };
    if summary.status != InvoiceStatus::Draft {
        return Ok(Synced::Issued(invoice));
    }
    if has_lines(&order, &lines, on_delivery) {
        let done = invoices
            .update_draft(
                &state.db,
                invoice_id,
                invoice_draft(&order, &lines, on_delivery),
            )
            .await?;
        return Ok(if done {
            Synced::Drafted
        } else {
            Synced::Nothing
        });
    }
    if invoices.discard_draft(&state.db, invoice_id).await? {
        let cleared: Option<i64> = None;
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(id))
            .update(&state.db, &[("invoice_id", &cleared)])
            .await?;
        return Ok(Synced::Discarded);
    }
    Ok(Synced::Nothing)
}
