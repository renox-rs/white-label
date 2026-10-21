//! Bill only the delivered quantity and handle a cash-on-delivery difference
//! (wl-invoicing §5.2 step 4, §6 *Consumes: wl-delivery*, issue #85).
//! `wl-delivery` emits `DeliveryDelivered`; this module reacts without
//! depending on that crate.
//!
//! - **Delivered quantity.** A `draft` invoice with a line whose origin is a
//!   line of the delivery bills what the recipient accepted: the line's
//!   quantity becomes the delivered quantity (the discount follows pro
//!   rata, amounts half-up to fils), never the refused quantity. A line
//!   with nothing delivered leaves the draft; a draft left without lines is
//!   discarded, so a delivery of nothing bills nothing. A quantity is never
//!   raised above the draft's. An invoice that is already issued is left
//!   alone (a prepaid order is credited through the sales return flow). A
//!   source that drafts after the delivery reads the quantities with
//!   `DeliveryDelivered::billable_milli`.
//! - **COD difference** (cash collected below the amount due), by who
//!   absorbs it:
//!   - *customer*: the invoice is not touched; the open balance stays open
//!     and the reminders go on;
//!   - *client* (03): the invoice is not touched; the difference is the
//!     negative adjustment line of the client's COD remittance
//!     (`CodAbsorption::remittance_adjustment`, apps/logistics);
//!   - *business* or *driver*: not billed. A draft gets the negative line
//!     *COD difference absorbed (business)* with the reason; an issued or
//!     paid invoice gets a credit note for the difference with the same
//!     reason (`FIN.ACL.01`). The driver's share is recovered in the cash
//!     handover (wl-payments), never from the customer.
//!
//!   The difference is the cash, so it includes VAT: the line or credit
//!   note takes the net that makes the gross equal the difference at the
//!   rate of the invoice's largest line (a decision: the spec gives the
//!   amount only). One row of [`CodDifference`] per delivery keeps an event
//!   received twice from reducing the invoice twice.
//!
//! The invoice that bills the delivery is found through the lines whose
//! origin is a line of the delivery, else the latest invoice whose source is
//! the delivery's source. Without one the row is `no_invoice`.

use renox::db::Transaction;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{CodAbsorber, CodAbsorption, DeliveredLine, DeliveryDelivered};
use wl_kernel::invoicing::{CreditLine, InvoiceStatus, VatCategory};
use wl_kernel::{Subject, delivery::DELIVERY_SUBJECT_TYPE};

use crate::credit_notes::{self, By};
use crate::invoice::Invoice;
use crate::invoicer;
use crate::line::InvoiceLine;
use crate::vat::{self, VatTotals};

/// What became of a COD difference.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CodOutcome {
    /// The customer still owes it: the invoice keeps it open.
    #[default]
    CustomerOwes,
    /// The client bears it: a negative line on its COD remittance.
    ClientRemittance,
    /// A draft was reduced by a negative line.
    DraftReduced,
    /// An issued invoice got a credit note.
    Credited,
    /// No invoice bills the delivery (yet).
    NoInvoice,
    /// The credit note was refused; the note says why.
    Refused,
}

/// What `DeliveryDelivered` did about the COD difference of one delivery.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_cod_differences",
    unique(delivery_type, delivery_id),
    index(invoice_id)
)]
pub struct CodDifference {
    /// The primary key.
    pub id: i64,
    /// The subject type of the delivery.
    pub delivery_type: String,
    /// The delivery's id.
    pub delivery_id: i64,
    /// The type of the record the delivery was requested for.
    pub source_type: String,
    /// The id of that record.
    pub source_id: i64,
    /// The invoice that bills the delivery, if one was found.
    pub invoice_id: Option<i64>,
    /// Who bears the difference: `customer`, `client`, `business`, `driver`.
    pub absorbed_by: String,
    /// The difference in fils.
    pub amount: i64,
    /// The reason of the difference.
    pub reason: String,
    /// What became of it.
    pub outcome: CodOutcome,
    /// The credit note of an issued invoice.
    pub credit_note_id: Option<i64>,
    /// The position of the negative line on a draft.
    pub line_position: Option<i64>,
    /// Why a credit was refused.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CodDifference {
    /// The row of the delivery `delivery_id`, if its difference was handled.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_delivery(db: &Db, delivery_id: i64) -> Result<Option<CodDifference>> {
        CodDifference::query()
            .where_(CodDifference::DELIVERY_TYPE.eq(DELIVERY_SUBJECT_TYPE))
            .where_(CodDifference::DELIVERY_ID.eq(delivery_id))
            .first(db)
            .await
    }
}

/// The text of the negative line and of the credit note.
pub(crate) fn absorbed_text(absorber: CodAbsorber, reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        format!("COD difference absorbed ({})", absorber.as_str())
    } else {
        format!("COD difference absorbed ({}): {reason}", absorber.as_str())
    }
}

/// The net that makes net + VAT equal `gross` at `rate_bp`, half-up.
pub(crate) fn net_of_gross(gross: i64, rate_bp: i64) -> i64 {
    vat::div_round_half_up(i128::from(gross) * 10_000, i128::from(10_000 + rate_bp))
}

/// The quantity a draft line bills once `delivered` is all that was
/// accepted, and the discount that goes with it; `None` when the line is
/// not reduced (the draft already bills as much or less).
pub(crate) fn trimmed(quantity: i64, discount: i64, delivered: i64) -> Option<(i64, i64)> {
    if delivered < 0 || delivered >= quantity || quantity <= 0 {
        return None;
    }
    let discount = vat::div_round_half_up(
        i128::from(discount) * i128::from(delivered),
        i128::from(quantity),
    );
    Some((delivered, discount))
}

fn target_of(delivered: &DeliveredLine) -> Option<(Subject, i64)> {
    Some((delivered.line.clone()?, delivered.delivered_milli))
}

/// Writes the totals of `invoice` from `lines`, as `replace_draft` does.
async fn write_totals(
    tx: &mut Transaction,
    invoice: &Invoice,
    lines: &[InvoiceLine],
) -> Result<()> {
    let amounts: Vec<_> = lines.iter().map(InvoiceLine::amounts).collect();
    let mut totals = invoice.clone();
    totals.set_totals(&VatTotals::of(&amounts));
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .where_(Invoice::STATUS.eq(InvoiceStatus::Draft))
        .update(
            &mut *tx,
            &[
                ("net_total", &totals.net_total),
                ("vat_total", &totals.vat_total),
                ("gross_total", &totals.gross_total),
                ("vat_total_aed", &totals.vat_total_aed),
                ("balance", &totals.balance),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Brings the lines of the draft `invoice_id` that bill a delivered line to
/// the delivered quantity. Returns whether the draft is left without lines
/// (the caller discards it).
async fn trim_draft(db: &Db, invoice_id: i64, targets: &[(Subject, i64)]) -> Result<bool> {
    let Some(invoice) = Invoice::find(db, invoice_id).await? else {
        return Ok(false);
    };
    if invoice.status != InvoiceStatus::Draft {
        return Ok(false);
    }
    let rate = invoice.aed_rate();
    let mut lines = crate::invoicer::lines_of(db, invoice.id).await?;
    let mut changed = false;
    let mut removed: Vec<i64> = Vec::new();
    for line in &mut lines {
        let Some(origin) = line.origin() else {
            continue;
        };
        let Some((_, delivered)) = targets.iter().find(|(subject, _)| *subject == origin) else {
            continue;
        };
        let Some((quantity, discount)) = trimmed(line.quantity_milli, line.discount, *delivered)
        else {
            continue;
        };
        changed = true;
        if quantity == 0 {
            removed.push(line.id);
            continue;
        }
        line.quantity_milli = quantity;
        line.discount = discount;
        let amounts = line.compute(rate);
        line.set_amounts(&amounts);
    }
    if !changed {
        return Ok(false);
    }
    let kept: Vec<InvoiceLine> = lines
        .iter()
        .filter(|line| !removed.contains(&line.id))
        .cloned()
        .collect();
    let mut tx = db.begin().await?;
    for id in &removed {
        InvoiceLine::query()
            .where_(InvoiceLine::ID.eq(*id))
            .delete(&mut tx)
            .await?;
    }
    // The kept lines close up, from 1, in their order; every line moves to a
    // position at or below its own, so none meets another.
    let mut renumbered = Vec::with_capacity(kept.len());
    for (line, position) in kept.into_iter().zip(1..) {
        let mut line = line;
        line.position = position;
        line.save(&mut tx).await?;
        renumbered.push(line);
    }
    write_totals(&mut tx, &invoice, &renumbered).await?;
    tx.commit().await?;
    Ok(renumbered.is_empty())
}

/// The ids of the invoices (drafts too) with a line that bills one of
/// `origins`, oldest first.
async fn invoices_billing(db: &Db, origins: &[Subject]) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = Vec::new();
    for origin in origins {
        let rows = InvoiceLine::query()
            .where_(InvoiceLine::ORIGIN_TYPE.eq(origin.subject_type.clone()))
            .where_(InvoiceLine::ORIGIN_ID.eq(origin.subject_id))
            .order_by(InvoiceLine::INVOICE_ID)
            .get(db)
            .await?;
        ids.extend(rows.iter().map(|l| l.invoice_id));
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// The invoice that bills `event`'s delivery: through the origins of its
/// lines, else by the delivery's source; a credited invoice does not count.
async fn invoice_of(db: &Db, event: &DeliveryDelivered) -> Result<Option<Invoice>> {
    let origins: Vec<Subject> = event.lines.iter().filter_map(|l| l.line.clone()).collect();
    let mut ids = invoices_billing(db, &origins).await?;
    if ids.is_empty() {
        let by_source = Invoice::query()
            .where_(Invoice::SOURCE_TYPE.eq(event.source.subject_type.clone()))
            .where_(Invoice::SOURCE_ID.eq(event.source.subject_id))
            .order_by(Invoice::ID)
            .get(db)
            .await?;
        ids = by_source.into_iter().map(|i| i.id).collect();
    }
    for id in ids {
        if let Some(invoice) = Invoice::find(db, id).await?
            && invoice.status != InvoiceStatus::Credited
        {
            return Ok(Some(invoice));
        }
    }
    Ok(None)
}

/// The VAT category of the invoice's largest line (standard when it has
/// none).
fn dominant_category(lines: &[InvoiceLine]) -> VatCategory {
    lines
        .iter()
        .filter(|l| l.net_amount > 0)
        .max_by_key(|l| l.net_amount)
        .map_or(VatCategory::Standard, |l| l.vat_category)
}

/// Adds the negative line to the draft `invoice`; its position.
async fn reduce_draft(
    db: &Db,
    invoice: &Invoice,
    delivery: Subject,
    absorption: &CodAbsorption,
) -> Result<i64> {
    let rate = invoice.aed_rate();
    let mut lines = crate::invoicer::lines_of(db, invoice.id).await?;
    let category = dominant_category(&lines);
    let net = net_of_gross(absorption.amount, category.rate_bp());
    let position = lines.iter().map(|l| l.position).max().unwrap_or(0) + 1;
    let mut line = InvoiceLine {
        invoice_id: invoice.id,
        position,
        description: absorbed_text(absorption.absorber, &absorption.reason),
        quantity_milli: 1_000,
        unit_price: -net,
        discount: 0,
        vat_category: category,
        origin_type: Some(delivery.subject_type.clone()),
        origin_id: Some(delivery.subject_id),
        ..Default::default()
    };
    line.set_amounts(&line.compute(rate));
    let mut tx = db.begin().await?;
    line.insert(&mut tx).await?;
    lines.push(line);
    write_totals(&mut tx, invoice, &lines).await?;
    tx.commit().await?;
    Ok(position)
}

/// The credit lines that take `net` off the invoice's lines, largest
/// remaining credit first-come: each line up to what is left to credit on it.
async fn credit_lines(db: &Db, invoice: &Invoice, net: i64) -> Result<Vec<CreditLine>> {
    let lines = crate::invoicer::lines_of(db, invoice.id).await?;
    let credited = credit_notes::credited_by_line(db, invoice.id).await?;
    let mut left = net;
    let mut planned = Vec::new();
    for line in lines.iter().filter(|l| l.net_amount > 0) {
        if left <= 0 {
            break;
        }
        let remaining = vat::remaining_credit(
            line.net_amount,
            credited.get(&line.id).copied().unwrap_or(0),
        );
        let take = remaining.min(left);
        if take > 0 {
            planned.push(CreditLine::new(line.position, take));
            left -= take;
        }
    }
    Ok(planned)
}

/// Credits an issued or paid `invoice` for the difference. Returns the
/// credit note's id, or why it was refused.
async fn credit_invoice(
    state: &AppState,
    invoice: &Invoice,
    absorption: &CodAbsorption,
) -> std::result::Result<i64, String> {
    let lines = crate::invoicer::lines_of(&state.db, invoice.id)
        .await
        .map_err(|e| e.to_string())?;
    let category = dominant_category(&lines);
    let net = net_of_gross(absorption.amount, category.rate_bp());
    let planned = credit_lines(&state.db, invoice, net)
        .await
        .map_err(|e| e.to_string())?;
    let credited: i64 = planned.iter().map(|l| l.net).sum();
    if planned.is_empty() || credited != net {
        return Err("the invoice has no credit left for the difference".to_owned());
    }
    let reason = absorbed_text(absorption.absorber, &absorption.reason);
    credit_notes::credit_partly(state, invoice.id, &planned, &reason, By::System(None))
        .await
        .map(|note| note.id)
        .map_err(|e| e.into_error().to_string())
}

/// Handles the COD difference of `event`, once.
async fn settle_difference(state: &AppState, event: &DeliveryDelivered) -> Result {
    let Some(absorption) = event.cod_absorption() else {
        return Ok(());
    };
    if CodDifference::of_delivery(&state.db, event.delivery.0)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let invoice = invoice_of(&state.db, event).await?;
    let mut row = CodDifference {
        delivery_type: DELIVERY_SUBJECT_TYPE.to_owned(),
        delivery_id: event.delivery.0,
        source_type: event.source.subject_type.clone(),
        source_id: event.source.subject_id,
        invoice_id: invoice.as_ref().map(|i| i.id),
        absorbed_by: absorption.absorber.as_str().to_owned(),
        amount: absorption.amount,
        reason: absorption.reason.clone(),
        ..Default::default()
    };
    match absorption.absorber {
        CodAbsorber::Customer => row.outcome = CodOutcome::CustomerOwes,
        CodAbsorber::Client => row.outcome = CodOutcome::ClientRemittance,
        CodAbsorber::Business | CodAbsorber::Driver => match &invoice {
            None => row.outcome = CodOutcome::NoInvoice,
            Some(invoice) if invoice.status == InvoiceStatus::Draft => {
                row.line_position = Some(
                    reduce_draft(&state.db, invoice, event.delivery.subject(), &absorption).await?,
                );
                row.outcome = CodOutcome::DraftReduced;
            }
            Some(invoice) => match credit_invoice(state, invoice, &absorption).await {
                Ok(id) => {
                    row.credit_note_id = Some(id);
                    row.outcome = CodOutcome::Credited;
                }
                Err(message) => {
                    row.note = Some(message);
                    row.outcome = CodOutcome::Refused;
                }
            },
        },
    }
    row.insert(&state.db).await?;
    Ok(())
}

/// `DeliveryDelivered`: the drafts bill the delivered quantity, then the COD
/// difference is handled.
async fn on_delivered(state: &AppState, event: &DeliveryDelivered) -> Result {
    let targets: Vec<(Subject, i64)> = event.lines.iter().filter_map(target_of).collect();
    if !targets.is_empty() {
        let origins: Vec<Subject> = targets.iter().map(|(s, _)| s.clone()).collect();
        for id in invoices_billing(&state.db, &origins).await? {
            if trim_draft(&state.db, id, &targets).await? {
                invoicer::discard_draft(&state.db, wl_kernel::invoicing::InvoiceId(id)).await?;
            }
        }
    }
    settle_difference(state, event).await
}

/// Listens for `DeliveryDelivered`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: DeliveryDelivered, state: AppState| async move {
        on_delivered(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_trimmed_to_the_delivered_quantity_with_its_discount() {
        assert_eq!(trimmed(10_000, 1_000, 8_000), Some((8_000, 800)));
        assert_eq!(trimmed(10_000, 0, 0), Some((0, 0)));
        // Half-up on the discount.
        assert_eq!(trimmed(3_000, 1, 1_500), Some((1_500, 1)));
    }

    #[test]
    fn a_line_is_never_raised_or_trimmed_by_a_bad_quantity() {
        assert_eq!(trimmed(10_000, 0, 10_000), None);
        assert_eq!(trimmed(10_000, 0, 12_000), None);
        assert_eq!(trimmed(10_000, 0, -1), None);
        assert_eq!(trimmed(0, 0, 0), None);
    }

    #[test]
    fn the_net_of_a_gross_difference_follows_the_rate() {
        assert_eq!(net_of_gross(3_000, 500), 2_857);
        assert_eq!(net_of_gross(3_000, 0), 3_000);
    }

    #[test]
    fn the_text_names_the_absorber_and_the_reason() {
        assert_eq!(
            absorbed_text(CodAbsorber::Business, ""),
            "COD difference absorbed (business)"
        );
        assert_eq!(
            absorbed_text(CodAbsorber::Driver, " short of cash "),
            "COD difference absorbed (driver): short of cash"
        );
    }
}
