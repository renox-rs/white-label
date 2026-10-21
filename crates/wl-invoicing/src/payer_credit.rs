//! The payer credit balance (wl-invoicing §3 *Payer credit balance*,
//! §5.8 step 7): money a payer paid beyond what it owes. The money itself is
//! the payer's unallocated credit in wl-payments; this module records the
//! movements, so a recoupment can be set off against it first and a refund
//! of it (`RefundSource::PayerCredit`, [`crate::payer_credit_refund`]) knows
//! what is left.
//!
//! A movement is signed: an approved overpayment adds, a recoupment that
//! takes from the balance and a refund subtract, a failed refund adds its
//! amount back. The balance is the sum of the movements of one payer, legal
//! entity and currency.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::payer_bills::PAYER_CREDIT_SUBJECT_TYPE;

/// The kind of an overpayment moved to the balance (adds).
pub const OVERPAYMENT: &str = "overpayment";
/// The kind of a recoupment set off from the balance (subtracts).
pub const SET_OFF: &str = "set_off";
/// The kind of a refund of the balance to the payer (subtracts).
pub const REFUND: &str = "refund";
/// The kind of a refund that failed (adds back what [`REFUND`] took).
pub const REFUND_RESTORED: &str = "refund_restored";

/// One movement of a payer's credit balance.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_payer_credit_movements",
    index(payer_party_id, legal_entity_id, currency)
)]
pub struct PayerCreditMovement {
    /// The primary key.
    pub id: i64,
    /// The payer (a party).
    pub payer_party_id: i64,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The ISO currency code.
    pub currency: String,
    /// [`OVERPAYMENT`], [`SET_OFF`] or [`REFUND`].
    pub kind: String,
    /// Signed fils: above 0 adds to the balance.
    pub amount: i64,
    /// The payer bill it came from, if any.
    pub payer_bill_id: Option<i64>,
    /// The remittance that carried the overpayment, if any.
    pub remittance_id: Option<i64>,
    /// The recoupment that took it, if any.
    pub recoupment_id: Option<i64>,
    /// The wl-payments refund it belongs to ([`REFUND`], [`REFUND_RESTORED`]).
    pub refund_id: Option<i64>,
    /// The user whose approval of the refund is the movement's approval
    /// (`FIN.ACL.16`); `None` for the other kinds.
    pub approved_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayerCreditMovement {
    /// The payer.
    pub fn payer(&self) -> PartyId {
        PartyId(self.payer_party_id)
    }
}

/// The subject type of a payer's credit balance:
/// `invoicing.payer_credit`.
pub const SUBJECT_TYPE: &str = PAYER_CREDIT_SUBJECT_TYPE;

/// What `payer` has as credit balance with `entity` in `currency`, in fils:
/// the sum of its movements. Read in `db` or inside a transaction.
///
/// # Errors
///
/// The database's errors.
pub async fn balance<'c>(
    db: impl renox::db::Executor<'c>,
    payer: PartyId,
    entity: i64,
    currency: &str,
) -> Result<i64> {
    Ok(PayerCreditMovement::query()
        .where_(PayerCreditMovement::PAYER_PARTY_ID.eq(payer.0))
        .where_(PayerCreditMovement::LEGAL_ENTITY_ID.eq(entity))
        .where_(PayerCreditMovement::CURRENCY.eq(currency.to_owned()))
        .get(db)
        .await?
        .iter()
        .map(|movement| movement.amount)
        .sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_credit_is_a_subject_of_its_own() {
        assert_eq!(SUBJECT_TYPE, "invoicing.payer_credit");
        assert_eq!(
            [OVERPAYMENT, SET_OFF, REFUND, REFUND_RESTORED],
            ["overpayment", "set_off", "refund", "refund_restored"]
        );
    }
}
