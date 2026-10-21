//! A late gateway confirmation (wl-payments §4 `late_confirm`, `request`
//! of a refund, §5.2 step 4, §7 *Late payment refunds*).
//!
//! The gateway may confirm a payment after it was marked `failed`. The money
//! is then `received`, with a receipt number and `PaymentReceived`, and:
//!
//! - when the prepaid record is still active and the documents of the link
//!   are still open, it is allocated as `confirm` does, each document up to
//!   its open balance;
//! - what is not allocated stays as the party's credit and a refund of it is
//!   requested with the reason [`REASON`], back to the same card. The refund
//!   waits for another person's approval (`FIN.ACL.04`), except when the
//!   strict conditions hold and the setting is on: it equals the payment, it
//!   goes back to the same card and the prepaid record is no longer active.
//!   It is then `approved` at once, with no second person, and sent to the
//!   gateway;
//! - the accountants are told what happened.
//!
//! A prepaid record is **no longer active** when its owner says so (cancelled,
//! rejected, its slot gone: [`PrepaidSubjects`](wl_kernel::PrepaidSubjects)),
//! when another payment of it is `received`, or when every document of the
//! link is closed already.
//!
//! A confirmation whose amount does not match is not confirmed (the
//! accountant gets a task, [`crate::online::gateway_succeeded`]). A replayed
//! webhook finds the payment `received` and does nothing.

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::invoicing::invoicer;
use wl_kernel::payments::{
    AllocationTarget, Direction, PaymentMethod, PaymentStatus, RefundSource as KernelRefundSource,
    prepaid_subjects,
};

use crate::config_of;
use crate::followup::money_text;
use crate::notices;
use crate::online;
use crate::payment::Payment;
use crate::payment_link::{self, PaymentLink};
use crate::refund::{self, Refund, RefundRequest};

/// The reason of the refund a late payment asks for.
pub const REASON: &str = "late confirmation";

/// Whether the refund of a late payment is approved at once, with no second
/// person (wl-payments §4 `request`): the setting is on, the refund is the
/// whole payment, it goes back to the same card, and the prepaid record is no
/// longer active.
pub fn auto_approves(
    setting: bool,
    refund: i64,
    payment: i64,
    has_prepaid: bool,
    subject_active: bool,
    same_card: bool,
) -> bool {
    setting && same_card && has_prepaid && !subject_active && refund == payment
}

/// Whether the prepaid record of `payment` still waits for its money: its
/// owner says it is active, no other payment of it is `received`, and the
/// link still has an open document (or names none). A payment with no prepaid
/// record counts as active.
async fn subject_active(
    state: &AppState,
    payment: &Payment,
    link: Option<&PaymentLink>,
) -> Result<bool> {
    let Some(prepaid) = payment.prepaid() else {
        return Ok(true);
    };
    if !prepaid_subjects(state).is_active(state, &prepaid).await? {
        return Ok(false);
    }
    let elsewhere = Payment::query()
        .where_(Payment::PREPAID_TYPE.eq(prepaid.subject_type.as_str()))
        .where_(Payment::PREPAID_ID.eq(prepaid.subject_id))
        .where_(Payment::STATUS.eq(PaymentStatus::Received))
        .where_(Payment::DIRECTION.eq(Direction::In))
        .where_(Payment::ID.ne(payment.id))
        .count(&state.db)
        .await?;
    if elsewhere > 0 {
        return Ok(false);
    }
    if let Some(link) = link
        && !link.targets.0.is_empty()
    {
        let invoices = invoicer(state);
        let mut any_open = false;
        for target in link.targets.0.iter() {
            let Some(AllocationTarget::Invoice(id)) =
                AllocationTarget::from_subject(&target.subject())
            else {
                continue;
            };
            if let Some(invoice) = invoices.find(&state.db, id).await?
                && invoice.status.is_open()
                && invoice.balance > 0
            {
                any_open = true;
            }
        }
        if !any_open {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Asks the refund of `amount` fils of the late `payment`, back to the same
/// card, as the system. `None` when the refund cannot be asked (nothing
/// refundable is left).
async fn request_refund(
    state: &AppState,
    payment: &Payment,
    amount: i64,
) -> Result<Option<Refund>> {
    let request = RefundRequest {
        source: KernelRefundSource::Payment(payment.payment_id()),
        party: payment.party(),
        legal_entity_id: payment.legal_entity_id,
        amount,
        currency: payment.currency.clone(),
        method: Some(PaymentMethod::Gateway),
        gateway_txn: payment.gateway_transaction.clone(),
        reason: REASON.to_owned(),
    };
    match refund::request_for_source_as(state, &request, None).await {
        Ok(refund) => Ok(Some(refund)),
        Err(TransitionError::Guard(_)) => Ok(None),
        Err(other) => Err(other.into_error()),
    }
}

/// After a committed `late_confirm` of payment `id`: allocate or keep as
/// credit with a refund, emit `PaymentReceived`, tell the accountants.
pub(crate) async fn after_late_confirm(state: &AppState, id: i64) -> Result {
    let payment = Payment::find_or_404(&state.db, id).await?;
    let link = match payment.payment_link_id {
        Some(link_id) => PaymentLink::find(&state.db, link_id).await?,
        None => None,
    };
    let active = subject_active(state, &payment, link.as_ref()).await?;
    let allocations = if active {
        online::allocate_to_link(state, &payment).await?
    } else {
        Vec::new()
    };
    let allocated: i64 = allocations.iter().map(|a| a.amount).sum();
    if allocated > 0
        && let Some(link) = &link
    {
        payment_link::mark_paid(state, link.id, payment.id).await?;
    }
    state
        .emit(online::received_event(&payment, allocations))
        .await?;
    let rest = payment.amount - allocated;
    let refund = if rest > 0 {
        request_refund(state, &payment, rest).await?
    } else {
        None
    };
    let same_card = payment.method == PaymentMethod::Gateway
        && payment
            .gateway_transaction
            .as_deref()
            .is_some_and(|txn| !txn.trim().is_empty());
    let auto = auto_approves(
        config_of(state).late_refund_auto,
        rest,
        payment.amount,
        payment.prepaid().is_some(),
        active,
        same_card,
    );
    let mut approved = false;
    if let Some(refund) = &refund
        && auto
    {
        match refund::approve_late(state, refund).await {
            Ok(refund) => {
                approved = true;
                // Back to the card at once. A blocked party, or a gateway
                // that refuses, leaves it `approved` for a person to send.
                let _sent = refund::send_to_gateway_as_system(state, &refund).await;
            }
            Err(TransitionError::Guard(_) | TransitionError::NotFrom(_)) => {}
            Err(other) => return Err(other.into_error()),
        }
    }
    let money = |fils: i64| money_text(&payment.currency, fils);
    let mut text = format!(
        "Late payment {} of {} arrived after the online payment had failed",
        payment.number.clone().unwrap_or_default(),
        money(payment.amount)
    );
    if allocated > 0 {
        text.push_str(&format!("; {} allocated", money(allocated)));
    }
    match (&refund, approved) {
        (Some(_), true) => text.push_str(&format!(
            "; {} kept as credit and refunded to the same card at once",
            money(rest)
        )),
        (Some(_), false) => text.push_str(&format!(
            "; {} kept as credit, its refund waits for approval",
            money(rest)
        )),
        (None, _) if rest > 0 => text.push_str(&format!("; {} kept as credit", money(rest))),
        _ => {}
    }
    notices::accountants_told(state, &payment, &text).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refund_is_approved_at_once_only_when_every_condition_holds() {
        assert!(auto_approves(true, 25_000, 25_000, true, false, true));
        // The setting is off.
        assert!(!auto_approves(false, 25_000, 25_000, true, false, true));
        // Only part of the payment.
        assert!(!auto_approves(true, 10_000, 25_000, true, false, true));
        // The record is still active.
        assert!(!auto_approves(true, 25_000, 25_000, true, true, true));
        // Not back to the same card.
        assert!(!auto_approves(true, 25_000, 25_000, true, false, false));
        // No prepaid record: an ordinary invoice link is never auto-approved.
        assert!(!auto_approves(true, 25_000, 25_000, false, false, true));
    }
}
