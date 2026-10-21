//! A refund of the payer credit balance (wl-invoicing §3 *Payer credit
//! balance*, `FIN.ACL.16`; wl-payments §5.5a, issue #101).
//!
//! Money an insurer or payer paid in excess and that was not set off against
//! recoupments is paid back with a wl-payments refund whose source is the
//! payer credit balance (`invoicing.payer_credit:<payer>`). The refund's own
//! approval, by a second person (`FIN.ACL.04`), is the movement's approval:
//! there is no second approval request. When wl-payments announces
//! [`RefundApproved`] for such a refund, [`take`] records a *refund* movement
//! of the negative amount, naming the refund and the approver. When the refund
//! fails ([`RefundFailed`]), [`restore`] gives the amount back with a
//! *refund_restored* movement; a retry takes it again. Both are idempotent per
//! refund: they look at the refund's net movements, so a repeated event
//! changes nothing.
//!
//! Decision: the amount is taken even when a recoupment set off the balance
//! between the request and the approval; the balance then shows below 0 until
//! the payer's next overpayment, instead of failing an approval that was
//! already committed.
//!
//! [`Credits`] is the kernel's [`PayerCredits`]: what wl-payments reads for
//! its refundable balance guard.

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::payer_bills::{PAYER_CREDIT_SUBJECT_TYPE, PayerBillFuture, PayerCredits};
use wl_kernel::payments::{RefundApproved, RefundFailed, RefundSource};

use crate::payer_credit::{self, PayerCreditMovement, REFUND, REFUND_RESTORED};

/// The payer a refund source names, when it is a payer credit balance.
fn payer_of(source: &RefundSource) -> Option<PartyId> {
    match source {
        RefundSource::PayerCredit(subject) if subject.subject_type == PAYER_CREDIT_SUBJECT_TYPE => {
            Some(PartyId(subject.subject_id))
        }
        _ => None,
    }
}

/// What the movements of refund `refund` add up to: below 0 while it holds
/// the amount, 0 before it took it and after it gave it back.
async fn net_of(tx: &mut renox::db::Transaction, refund: i64) -> Result<i64> {
    Ok(PayerCreditMovement::query()
        .where_(PayerCreditMovement::REFUND_ID.eq(refund))
        .get(&mut *tx)
        .await?
        .iter()
        .map(|movement| movement.amount)
        .sum())
}

/// Takes the amount of the approved refund from the payer credit balance:
/// one *refund* movement, with the approver. Nothing for another source, or
/// when the refund holds the amount already.
///
/// # Errors
///
/// The database's errors.
pub async fn take(state: &AppState, event: &RefundApproved) -> Result {
    let Some(payer) = payer_of(&event.source) else {
        return Ok(());
    };
    let mut tx = state.db.begin().await?;
    if net_of(&mut tx, event.refund.0).await? != 0 {
        return Ok(());
    }
    PayerCreditMovement {
        payer_party_id: payer.0,
        legal_entity_id: event.legal_entity_id,
        currency: event.currency.clone(),
        kind: REFUND.to_owned(),
        amount: -event.amount,
        refund_id: Some(event.refund.0),
        approved_by: event.approved_by,
        ..Default::default()
    }
    .insert(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Gives the amount of the failed refund back to the payer credit balance:
/// one *refund_restored* movement. Nothing for another source, or when the
/// refund holds nothing.
///
/// # Errors
///
/// The database's errors.
pub async fn restore(state: &AppState, event: &RefundFailed) -> Result {
    if payer_of(&event.source).is_none() {
        return Ok(());
    }
    let mut tx = state.db.begin().await?;
    let net = net_of(&mut tx, event.refund.0).await?;
    if net >= 0 {
        return Ok(());
    }
    let Some(taken) = PayerCreditMovement::query()
        .where_(PayerCreditMovement::REFUND_ID.eq(event.refund.0))
        .where_(PayerCreditMovement::KIND.eq(REFUND))
        .order_by_desc(PayerCreditMovement::ID)
        .first(&mut tx)
        .await?
    else {
        return Ok(());
    };
    PayerCreditMovement {
        payer_party_id: taken.payer_party_id,
        legal_entity_id: taken.legal_entity_id,
        currency: taken.currency.clone(),
        kind: REFUND_RESTORED.to_owned(),
        amount: -net,
        refund_id: Some(event.refund.0),
        ..Default::default()
    }
    .insert(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The balances wl-payments reads for its refundable balance guard.
#[derive(Debug, Clone, Copy, Default)]
pub struct Credits;

impl PayerCredits for Credits {
    fn balance<'a>(
        &'a self,
        db: &'a Db,
        payer: PartyId,
        legal_entity_id: i64,
        currency: &'a str,
    ) -> PayerBillFuture<'a, i64> {
        Box::pin(payer_credit::balance(db, payer, legal_entity_id, currency))
    }
}

/// Registers the listeners of `RefundApproved` and `RefundFailed`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: RefundApproved, state: AppState| async move {
        take(&state, &event).await
    });
    app.listen(|event: RefundFailed, state: AppState| async move {
        restore(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::Subject;

    #[test]
    fn only_a_payer_credit_source_names_a_payer() {
        let credit = RefundSource::PayerCredit(Subject::new(PAYER_CREDIT_SUBJECT_TYPE, 4));
        assert_eq!(payer_of(&credit), Some(PartyId(4)));
        let other = RefundSource::PayerCredit(Subject::new("sales.order", 4));
        assert_eq!(payer_of(&other), None);
        assert_eq!(payer_of(&RefundSource::PartyCredit(PartyId(4))), None);
    }
}
