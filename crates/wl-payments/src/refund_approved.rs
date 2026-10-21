//! `RefundApproved` (wl-payments §4 Refund, §5.5a): a refund that now claims
//! its source. A listener on `Transitioned<Refund>` emits it after the commit
//! of `approve`, `approve_by_system`, `retry`, `retry_by_system` and
//! `change_method` (the last three put a failed refund back to `approved`, so
//! the owner of the source takes the amount again).
//!
//! It is for the owner of a source outside this module: wl-invoicing records
//! the *refund* movement of a payer credit balance with the approval, which is
//! the refund's own (`FIN.ACL.16`, `FIN.ACL.04`): there is no second approval
//! request.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::payments::RefundApproved;

use crate::refund::Refund;
use crate::refund_machine::{APPROVE, APPROVE_BY_SYSTEM};
use crate::refund_recover::{CHANGE_METHOD, RETRY, RETRY_BY_SYSTEM};

/// The event of the approved `refund`, approved by `approved_by`.
pub(crate) fn event_of(refund: &Refund, approved_by: Option<i64>) -> Option<RefundApproved> {
    Some(RefundApproved {
        refund: refund.refund_id(),
        party: refund.party(),
        legal_entity_id: refund.legal_entity_id,
        amount: refund.amount,
        currency: refund.currency.clone(),
        source: refund.source()?,
        approved_by,
    })
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Refund>, state: AppState| async move {
        if ![APPROVE, APPROVE_BY_SYSTEM, RETRY, RETRY_BY_SYSTEM, CHANGE_METHOD]
            .contains(&event.transition.as_str())
        {
            return Ok(());
        }
        // The committed row.
        let refund = Refund::find_or_404(&state.db, event.key).await?;
        // Whoever approved it first stays its approver after a retry.
        let approved_by = match event.transition.as_str() {
            APPROVE => event.actor_id,
            _ => refund.approved_by,
        };
        if let Some(approved) = event_of(&refund, approved_by) {
            state.emit(approved).await?;
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::payments::{RefundId, RefundSource};
    use wl_kernel::{PartyId, Subject};

    #[test]
    fn the_event_names_the_source_the_amount_and_the_approver() {
        let refund = Refund {
            id: 9,
            party_id: 4,
            legal_entity_id: 2,
            amount: 120_000,
            currency: "AED".into(),
            source_type: "invoicing.payer_credit".into(),
            source_id: 4,
            source_kind: "payer_credit".into(),
            ..Default::default()
        };
        let event = event_of(&refund, Some(7)).expect("a known source");
        assert_eq!(event.refund, RefundId(9));
        assert_eq!(event.party, PartyId(4));
        assert_eq!(event.legal_entity_id, 2);
        assert_eq!(event.amount, 120_000);
        assert_eq!(event.approved_by, Some(7));
        assert_eq!(
            event.source,
            RefundSource::PayerCredit(Subject::new("invoicing.payer_credit", 4))
        );
    }

    #[test]
    fn a_refund_with_an_unknown_source_gives_no_event() {
        let refund = Refund {
            source_kind: "nothing".into(),
            ..Default::default()
        };
        assert!(event_of(&refund, None).is_none());
    }
}
