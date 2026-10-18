//! The Refund (wl-payments §3, §4, A11): money returned to a party out of a
//! payment's unallocated amount or out of the party's credit balance.
//!
//! The Refund base has the model, all six states ([`RefundStatus`]) and
//! four of the transitions ([`crate::refund_machine`]): [`request`] (a
//! user with `refunds.request`), [`approve`] (`refunds.approve`, never the
//! requester: `FIN.ACL.04`), [`reject`] (a reason is required) and [`pay`]
//! (`refunds.pay`): it records an outgoing payment with source `system`,
//! links it to the refund, and emits `RefundPaid`. `fail`, `retry`,
//! `change_method`, `cancel` and the system request that goes straight to
//! `approved` come with the refund issues that follow. The status moves
//! only through the machine.
//!
//! A paid refund lowers the party's credit balance ([`crate::credit`]); a
//! refund is refused above the refundable balance of its source.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::payments::{
    Direction, PAYMENT_SUBJECT_TYPE, PaymentId, PaymentMethod, PaymentSource, RecordPayment,
    RefundId,
};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::credit;
use crate::payment::Payment;
use crate::permissions;
use crate::record;
use crate::refund_machine::{APPROVE, PAY, PayInput, REJECT};

/// The lifecycle state of a refund (wl-payments §4); the state of the
/// Refund machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RefundStatus {
    /// Waiting for approval.
    #[default]
    Requested,
    /// Approved; to be paid.
    Approved,
    /// Paid out; leaves only if the bank returns the transfer.
    Paid,
    /// The gateway refused it, or the bank returned the transfer.
    Failed,
    /// Not approved (terminal).
    Rejected,
    /// Given up after a failure; the amount is back on the party's credit
    /// balance (terminal).
    Cancelled,
}

impl RefundStatus {
    /// The states in which a refund still claims its source's balance.
    pub const PENDING: [RefundStatus; 2] = [RefundStatus::Requested, RefundStatus::Approved];

    /// The states in which a refund claims its source's balance or has
    /// taken it.
    pub const ACTIVE: [RefundStatus; 3] = [
        RefundStatus::Requested,
        RefundStatus::Approved,
        RefundStatus::Paid,
    ];
}

/// What a refund comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefundSource {
    /// The unallocated amount of an incoming payment.
    Payment(PaymentId),
    /// The party's credit balance in a currency, for a legal entity.
    PartyCredit {
        /// Who is paid back.
        party: PartyId,
        /// The legal entity paying.
        legal_entity_id: i64,
        /// The ISO currency code, e.g. `AED`.
        currency: String,
    },
}

/// What a user asks to refund.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRefund {
    /// What it comes from.
    pub source: RefundSource,
    /// The amount, in fils; above 0 and within the refundable balance.
    pub amount: i64,
    /// Why.
    pub reason: String,
}

/// A refund. Money is `i64` fils of [`Refund::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_refunds",
    state_machine,
    index(status),
    index(party_id),
    index(source_type, source_id)
)]
pub struct Refund {
    /// The primary key.
    pub id: i64,
    /// Who is paid back.
    pub party_id: i64,
    /// The legal entity paying.
    pub legal_entity_id: i64,
    /// The type of the source: `payments.payment` or the party's.
    pub source_type: String,
    /// The id of the source.
    pub source_id: i64,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// Why it is refunded.
    pub reason: String,
    /// How it was paid; set by `pay`.
    pub method: Option<PaymentMethod>,
    /// The payment reference; set by `pay`.
    pub reference: Option<String>,
    /// The user who requested it; `None` for the system.
    pub requested_by: Option<i64>,
    /// The user who approved it.
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// Why it was rejected.
    pub decision_reason: Option<String>,
    /// The outgoing payment `pay` recorded.
    pub payment_id: Option<i64>,
    /// When it was paid.
    pub paid_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: RefundStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Refund {
    /// The refund's id in the kernel contract.
    pub fn refund_id(&self) -> RefundId {
        RefundId(self.id)
    }

    /// Who is paid back.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// What it comes from, as a [`Subject`].
    pub fn source_subject(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// Whether it comes from a payment.
    pub fn source_payment(&self) -> Option<PaymentId> {
        (self.source_type == PAYMENT_SUBJECT_TYPE).then_some(PaymentId(self.source_id))
    }

    /// The refunds of `party` waiting to be paid or paid, newest first.
    pub async fn active_of(db: &Db, party: PartyId) -> Result<Vec<Refund>> {
        Refund::query()
            .where_(Refund::PARTY_ID.eq(party.0))
            .where_(Refund::STATUS.is_in(RefundStatus::ACTIVE))
            .order_by_desc(Refund::ID)
            .get(db)
            .await
    }
}

impl RefundSource {
    /// The source as a [`Subject`].
    pub fn subject(&self) -> Subject {
        match self {
            RefundSource::Payment(id) => id.subject(),
            RefundSource::PartyCredit { party, .. } => party.subject(),
        }
    }
}

/// Why `reason` cannot be given, if it cannot: empty or only spaces.
pub(crate) fn reason_problem(reason: &str) -> Option<&'static str> {
    reason
        .trim()
        .is_empty()
        .then_some("A reason is required for a refund.")
}

/// Why a refund of `amount` fils cannot be asked when `refundable` is
/// left, if it cannot.
pub(crate) fn amount_problem(amount: i64, refundable: i64) -> Option<String> {
    if amount <= 0 {
        return Some("The amount to refund must be above 0.".to_owned());
    }
    if amount > refundable {
        return Some(format!(
            "The amount is above the refundable balance ({refundable} fils)."
        ));
    }
    None
}

/// Where a refund of the given source goes: the party, the legal entity,
/// the currency and the balance still refundable.
struct Resolved {
    party: PartyId,
    legal_entity_id: i64,
    currency: String,
    refundable: i64,
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

async fn resolve(
    state: &AppState,
    source: &RefundSource,
) -> std::result::Result<Resolved, TransitionError> {
    match source {
        RefundSource::Payment(id) => {
            let payment = Payment::find(&state.db, id.0)
                .await?
                .ok_or_else(|| guard("The payment was not found."))?;
            if payment.direction != Direction::In || !payment.status.is_received() {
                return Err(guard("Only a received incoming payment can be refunded."));
            }
            Ok(Resolved {
                party: payment.party(),
                legal_entity_id: payment.legal_entity_id,
                currency: payment.currency.clone(),
                refundable: credit::refundable_payment(&state.db, &payment).await?,
            })
        }
        RefundSource::PartyCredit {
            party,
            legal_entity_id,
            currency,
        } => {
            let currency = currency.trim().to_ascii_uppercase();
            let found = wl_kernel::parties::directory(state)
                .find(&state.db, *party)
                .await?;
            if found.is_none() {
                return Err(guard("The party was not found."));
            }
            let entity = LegalEntity::find(&state.db, *legal_entity_id).await?;
            if !entity.as_ref().is_some_and(LegalEntity::is_active) {
                return Err(guard("The legal entity is not active."));
            }
            Ok(Resolved {
                party: *party,
                legal_entity_id: *legal_entity_id,
                refundable: credit::refundable_credit(&state.db, *party, &currency).await?,
                currency,
            })
        }
    }
}

/// `user` asks to refund `request.amount` fils of `request.source`; the
/// refund starts `requested`.
///
/// # Errors
///
/// `Forbidden` without `refunds.request`. `Guard` (422) for an empty
/// reason, an amount of 0 or less or above the refundable balance, a
/// payment that is not found, incoming and `received`, or an unknown party
/// or inactive legal entity. Nothing is written then.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    request: &RequestRefund,
) -> std::result::Result<Refund, TransitionError> {
    if !user.has_permission(permissions::REFUNDS_REQUEST) {
        return Err(TransitionError::Forbidden);
    }
    if let Some(problem) = reason_problem(&request.reason) {
        return Err(guard(problem));
    }
    let resolved = resolve(state, &request.source).await?;
    if let Some(problem) = amount_problem(request.amount, resolved.refundable) {
        return Err(guard(problem));
    }
    let source = request.source.subject();
    let mut refund = Refund {
        party_id: resolved.party.0,
        legal_entity_id: resolved.legal_entity_id,
        source_type: source.subject_type,
        source_id: source.subject_id,
        amount: request.amount,
        currency: resolved.currency,
        reason: request.reason.trim().to_owned(),
        requested_by: Some(user.id),
        status: RefundStatus::Requested,
        ..Default::default()
    };
    let mut tx = state.db.begin().await?;
    // Born `requested`, with a history row for it.
    let reason = refund.reason.clone();
    refund
        .insert_in_state(&mut tx, RefundStatus::Requested, &reason)
        .await?;
    tx.commit().await?;
    Ok(refund)
}

/// `user` approves `refund`.
///
/// # Errors
///
/// `Forbidden` without `refunds.approve`; `NotFrom` unless `requested`;
/// `Guard` (422) for the user who requested it (`FIN.ACL.04`).
pub async fn approve(
    state: &AppState,
    user: &AuthUser,
    refund: &Refund,
) -> std::result::Result<Refund, TransitionError> {
    let _outcome = refund.transition(APPROVE).by(user).run(state).await?;
    reload(state, refund).await
}

/// `user` rejects `refund`, with a reason.
///
/// # Errors
///
/// `Forbidden` without `refunds.approve`; `NotFrom` unless `requested`;
/// `Guard` (422) without a reason.
pub async fn reject(
    state: &AppState,
    user: &AuthUser,
    refund: &Refund,
    reason: &str,
) -> std::result::Result<Refund, TransitionError> {
    let mut run = refund.transition(REJECT).by(user);
    if !reason.trim().is_empty() {
        run = run.reason(reason.trim().to_owned());
    }
    let _outcome = run.run(state).await?;
    reload(state, refund).await
}

/// `user` pays the approved `refund` by `method`, with `reference`: an
/// outgoing payment (source `system`, `recorded_by` the user) is recorded
/// and linked to the refund, in one transaction with the status change.
/// `RefundPaid` follows after the commit.
///
/// # Errors
///
/// `Forbidden` without `refunds.pay`; `NotFrom` unless `approved`; `Guard`
/// (422) without a reference, when the source no longer covers the amount,
/// or when the outgoing payment is refused (method not enabled, details
/// incomplete). Nothing is written then and no voucher number is used.
pub async fn pay(
    state: &AppState,
    user: &AuthUser,
    refund: &Refund,
    method: PaymentMethod,
    reference: &str,
) -> std::result::Result<Refund, TransitionError> {
    if !user.has_permission(permissions::REFUNDS_PAY) {
        return Err(TransitionError::Forbidden);
    }
    let reference = reference.trim();
    let mut request = RecordPayment::new(
        refund.party(),
        refund.legal_entity_id,
        method,
        refund.amount,
    );
    request.direction = Direction::Out;
    request.currency = refund.currency.clone();
    request.date = Some(today());
    request.reference = (!reference.is_empty()).then(|| reference.to_owned());
    request.source = PaymentSource::System;
    // What the guards need is read before the transaction opens.
    let covered = credit::covers(&state.db, refund).await?;
    let (payment, input) = record::prepare(state, &request, Some(user.id)).await?;
    let mut tx = state.db.begin().await?;
    let (payment, recorded) = record::record_in(&mut tx, None, payment, &input).await?;
    let pay_input = PayInput {
        method,
        reference: reference.to_owned(),
        payment_id: payment.id,
        covered,
    };
    let paid = refund
        .transition(PAY)
        .by(user)
        .input(&pay_input)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    // `PaymentReceived` is for incoming money only (crate::events); the
    // listener of the refund emits `RefundPaid`.
    recorded
        .emit(state)
        .await
        .map_err(TransitionError::Failed)?;
    paid.emit(state).await.map_err(TransitionError::Failed)?;
    reload(state, refund).await
}

async fn reload(state: &AppState, refund: &Refund) -> std::result::Result<Refund, TransitionError> {
    Refund::find_or_404(&state.db, refund.id)
        .await
        .map_err(TransitionError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refund_has_six_states_and_starts_requested() {
        assert_eq!(RefundStatus::default(), RefundStatus::Requested);
        assert!(RefundStatus::PENDING.contains(&RefundStatus::Approved));
        assert!(!RefundStatus::ACTIVE.contains(&RefundStatus::Rejected));
        assert!(RefundStatus::ACTIVE.contains(&RefundStatus::Paid));
    }

    #[test]
    fn a_reason_must_not_be_blank() {
        assert!(reason_problem("").is_some());
        assert!(reason_problem("   ").is_some());
        assert!(reason_problem("Duplicate payment").is_none());
    }

    #[test]
    fn the_amount_must_be_positive_and_within_the_balance() {
        assert!(amount_problem(0, 100).is_some());
        assert!(amount_problem(-5, 100).is_some());
        assert!(amount_problem(101, 100).is_some());
        assert!(amount_problem(100, 100).is_none());
    }

    #[test]
    fn the_source_is_a_payment_or_the_party() {
        let from_payment = RefundSource::Payment(PaymentId(4));
        assert_eq!(from_payment.subject().to_string(), "payments.payment:4");
        let credit = RefundSource::PartyCredit {
            party: PartyId(7),
            legal_entity_id: 2,
            currency: "AED".into(),
        };
        assert_eq!(credit.subject(), PartyId(7).subject());
        let refund = Refund {
            id: 9,
            party_id: 7,
            source_type: PAYMENT_SUBJECT_TYPE.into(),
            source_id: 4,
            ..Default::default()
        };
        assert_eq!(refund.source_payment(), Some(PaymentId(4)));
        assert_eq!(refund.source_subject(), from_payment.subject());
        assert_eq!(refund.refund_id(), RefundId(9));
        assert_eq!(refund.party(), PartyId(7));
        let credit_refund = Refund {
            source_type: PartyId(7).subject().subject_type,
            source_id: 7,
            ..Default::default()
        };
        assert_eq!(credit_refund.source_payment(), None);
    }
}
