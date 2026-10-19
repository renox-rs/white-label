//! The Deposit (wl-payments §3, §4): money held as security for a source
//! agreement, e.g. a lease.
//!
//! The Deposit base has the model, all five states ([`DepositStatus`]) and
//! three of the transitions ([`crate::deposit_machine`]): [`hold`] (a user
//! with `deposits.manage`), [`start_settlement`] (`deposits.manage`, or the
//! system when the source agreement ended) and [`refund`]
//! (`deposits.refund`), which requests a Refund with the deposit as source
//! for the held amount. [`forfeit`] keeps a deposit being settled and
//! [`transfer`] hands a held one to a new holder; both and `refund` wait for the
//! deductions published to the party and disputed by it
//! ([`crate::deposit_deductions`]). A held deposit's amount is changed by
//! [`crate::deposit_adjust`].
//!
//! The money of a received payment allocated to a deposit
//! ([`crate::allocate`]) leaves the party's credit. The deposit stays
//! `refunded` (terminal) whatever happens to its refund: "refund failed" is
//! read from the linked refund ([`Deposit::refund_failed`]), so nothing has
//! to be cleared when a failed refund is tried again.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::payments::{DepositId, PaymentId, PaymentMethod, RecordPayment};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::allocate::{allocate_to_deposit_in, deposit_problem};
use crate::credit;
use crate::deposit_adjust::DepositAdjustment;
use crate::deposit_deductions::gate_of;
use crate::deposit_machine::{
    FORFEIT, ForfeitInput, REFUND, RefundDepositInput, START_SETTLEMENT,
    START_SETTLEMENT_BY_SYSTEM, TRANSFER, TransferInput,
};
use crate::payment::Payment;
use crate::permissions;
use crate::refund::{Refund, RefundStatus};

/// The lifecycle state of a deposit (wl-payments §4); the state of the
/// Deposit machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DepositStatus {
    /// Received and held for the source agreement.
    #[default]
    Held,
    /// The agreement ended; the deductions are being decided.
    SettlementPending,
    /// The balance is paid back (terminal).
    Refunded,
    /// Kept in full, with a reason (terminal).
    Forfeited,
    /// Handed over with the source agreement to another holder (terminal).
    Transferred,
}

impl DepositStatus {
    /// Whether a refund of the deposit can still be asked: the deposit is
    /// not forfeited or transferred.
    pub fn is_refundable(self) -> bool {
        matches!(
            self,
            DepositStatus::Held | DepositStatus::SettlementPending | DepositStatus::Refunded
        )
    }
}

/// What a user asks to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldDeposit {
    /// Whose deposit it is.
    pub party: PartyId,
    /// The legal entity holding it.
    pub legal_entity_id: i64,
    /// What it is held for, e.g. a lease.
    pub source: Subject,
    /// The amount the source requires, in fils; above 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The received payment to allocate to the deposit, if the money is in:
    /// its unallocated amount, up to the deposit amount.
    pub received_payment: Option<PaymentId>,
}

/// A deposit. Money is `i64` fils of [`Deposit::currency`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_deposits",
    state_machine,
    index(status),
    index(party_id),
    index(source_type, source_id)
)]
pub struct Deposit {
    /// The primary key.
    pub id: i64,
    /// Whose deposit it is.
    pub party_id: i64,
    /// The legal entity holding it.
    pub legal_entity_id: i64,
    /// The type of what it is held for, e.g. `property.lease`.
    pub source_type: String,
    /// The id of what it is held for.
    pub source_id: i64,
    /// The amount the source requires, in fils; always > 0.
    pub amount: i64,
    /// What is held: the money allocated to the deposit, in fils.
    pub held_amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// The payment the deposit was received with.
    pub received_payment_id: Option<i64>,
    /// How it is paid back: `transfer` or `cheque`; set by `refund`.
    pub refund_method: Option<PaymentMethod>,
    /// The refund `refund` requested.
    pub refund_id: Option<i64>,
    /// Whether a legal case on the source agreement keeps the deductions
    /// dispute flag until a staff member records its decision.
    pub dispute_legal_case: bool,
    /// Why `forfeit` kept the deposit.
    pub forfeit_reason: Option<String>,
    /// The second user who approved the forfeiture.
    pub forfeit_approved_by: Option<i64>,
    /// The new holder a transferred deposit was handed to.
    pub transferred_to: Option<i64>,
    /// The reference of the new holder's receipt.
    pub transfer_receipt: Option<String>,
    /// The outgoing payment to the new holder.
    pub transfer_payment_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: DepositStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Deposit {
    /// The deposit's id in the kernel contract.
    pub fn deposit_id(&self) -> DepositId {
        DepositId(self.id)
    }

    /// The deposit as a [`Subject`]: `payments.deposit:7`.
    pub fn subject(&self) -> Subject {
        self.deposit_id().subject()
    }

    /// Whose deposit it is.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// What it is held for, as a [`Subject`].
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.source_type.clone(),
            subject_id: self.source_id,
        }
    }

    /// The refund `refund` requested, if any.
    pub async fn linked_refund(&self, db: &Db) -> Result<Option<Refund>> {
        match self.refund_id {
            Some(id) => Refund::find(db, id).await,
            None => Ok(None),
        }
    }

    /// Whether the linked refund `failed`. Read from the refund, so the
    /// deposit stays `refunded` and nothing is cleared when it is tried
    /// again.
    pub async fn refund_failed(&self, db: &Db) -> Result<bool> {
        Ok(self
            .linked_refund(db)
            .await?
            .is_some_and(|refund| refund.status == RefundStatus::Failed))
    }

    /// What can still be refunded out of the deposit, in fils: what it
    /// holds less the refunds that claim it or took it; never below 0.
    pub async fn refundable(&self, db: &Db) -> Result<i64> {
        let source = self.subject();
        let claimed = Refund::query()
            .where_(Refund::SOURCE_TYPE.eq(source.subject_type.as_str()))
            .where_(Refund::SOURCE_ID.eq(source.subject_id))
            .where_(Refund::STATUS.is_in(RefundStatus::ACTIVE))
            .sum::<i64, _>(db, "amount")
            .await?;
        Ok((self.held_amount - claimed).max(0))
    }

    /// The deposits of `party`, newest first.
    pub async fn of_party(db: &Db, party: PartyId) -> Result<Vec<Deposit>> {
        Deposit::query()
            .where_(Deposit::PARTY_ID.eq(party.0))
            .order_by_desc(Deposit::ID)
            .get(db)
            .await
    }
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// Why a deposit of `amount` fils cannot be held, if it cannot.
pub(crate) fn amount_problem(amount: i64) -> Option<&'static str> {
    (amount <= 0).then_some("The deposit amount must be above 0.")
}

/// Why `method` cannot pay a deposit back, if it cannot: only a transfer or
/// a cheque.
pub(crate) fn method_problem(method: Option<PaymentMethod>) -> Option<&'static str> {
    match method {
        Some(PaymentMethod::Transfer | PaymentMethod::Cheque) => None,
        Some(_) => Some("A deposit is paid back by transfer or cheque."),
        None => Some("A refund method (transfer or cheque) is required."),
    }
}

/// `user` holds a deposit of `request.amount` fils for `request.source`; it
/// starts `held`. When a received payment is given, its unallocated amount
/// up to the deposit amount is allocated to the deposit, in the same
/// transaction, and leaves the party's credit.
///
/// # Errors
///
/// `Forbidden` without `deposits.manage`. `Guard` (422) for an amount of 0
/// or less, an unknown party, an inactive legal entity, or a payment that
/// is not found, incoming, `received`, of the same party and currency, with
/// something left to allocate. Nothing is written then.
pub async fn hold(
    state: &AppState,
    user: &AuthUser,
    request: &HoldDeposit,
) -> std::result::Result<Deposit, TransitionError> {
    if !user.has_permission(permissions::DEPOSITS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    if let Some(problem) = amount_problem(request.amount) {
        return Err(guard(problem));
    }
    let found = wl_kernel::parties::directory(state)
        .find(&state.db, request.party)
        .await?;
    if found.is_none() {
        return Err(guard("The party was not found."));
    }
    let entity = LegalEntity::find(&state.db, request.legal_entity_id).await?;
    if !entity.as_ref().is_some_and(LegalEntity::is_active) {
        return Err(guard("The legal entity is not active."));
    }
    let mut deposit = Deposit {
        party_id: request.party.0,
        legal_entity_id: request.legal_entity_id,
        source_type: request.source.subject_type.clone(),
        source_id: request.source.subject_id,
        amount: request.amount,
        currency: request.currency.trim().to_ascii_uppercase(),
        received_payment_id: request.received_payment.map(|id| id.0),
        status: DepositStatus::Held,
        ..Default::default()
    };
    // The money that is in, read before the transaction opens.
    let payment = match request.received_payment {
        Some(id) => {
            let payment = Payment::find(&state.db, id.0)
                .await?
                .ok_or_else(|| guard("The payment was not found."))?;
            let free = credit::unallocated(&state.db, &payment).await?;
            if free <= 0 {
                return Err(guard("The payment has nothing left to allocate."));
            }
            let amount = free.min(request.amount);
            if let Some(problem) = deposit_problem(&payment, &deposit, free, amount) {
                return Err(guard(problem));
            }
            Some((payment, amount))
        }
        None => None,
    };
    let mut tx = state.db.begin().await?;
    deposit
        .insert_in_state(&mut tx, DepositStatus::Held, "Deposit held")
        .await?;
    if let Some((payment, amount)) = payment {
        allocate_to_deposit_in(&mut tx, &payment, &mut deposit, amount, Some(user.id)).await?;
    }
    tx.commit().await?;
    Ok(deposit)
}

/// `actor` starts the settlement of `deposit` (`held` →
/// `settlement_pending`); `None` is the system, when the source agreement
/// ended.
///
/// # Errors
///
/// `Forbidden` without `deposits.manage`; `NotFrom` unless `held`.
pub async fn start_settlement(
    state: &AppState,
    actor: Option<&AuthUser>,
    deposit: &Deposit,
) -> std::result::Result<Deposit, TransitionError> {
    let run = match actor {
        Some(user) => deposit.transition(START_SETTLEMENT).by(user),
        None => deposit.transition(START_SETTLEMENT_BY_SYSTEM).by_system(),
    };
    let _outcome = run.run(state).await?;
    reload(state, deposit).await
}

/// `user` refunds `deposit` by `method` (`transfer` or `cheque`): the
/// deposit becomes `refunded` and, in the same transaction, a Refund is
/// requested with the deposit as source, for the held amount. The refund
/// then follows the Refund flow (approval `FIN.ACL.04`, pay, fail).
///
/// # Errors
///
/// `Forbidden` without `deposits.refund`; `NotFrom` unless
/// `settlement_pending`; `Guard` (422) without a method of transfer or
/// cheque, for a deposit that holds nothing or already has a refund, while a
/// deductions dispute is open, and (with the dispute setting on) before the
/// deductions are published and their dispute window has passed.
pub async fn refund(
    state: &AppState,
    user: &AuthUser,
    deposit: &Deposit,
    method: Option<PaymentMethod>,
) -> std::result::Result<Deposit, TransitionError> {
    let deposit = reload(state, deposit).await?;
    let input = RefundDepositInput {
        method,
        gate: Some(gate_of(state, &deposit).await?),
        part_refund_open: DepositAdjustment::part_refund_open(&state.db, deposit.id).await?,
    };
    let _outcome = deposit
        .transition(REFUND)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    reload(state, &deposit).await
}

/// What a user gives to forfeit a deposit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForfeitRequest {
    /// Why the deposit is kept; required.
    pub reason: String,
    /// The second user who approves the forfeiture: another user who may
    /// refund deposits (`deposits.refund`).
    pub approver: Option<i64>,
}

/// `user` forfeits `deposit`: the party keeps no claim on it (`forfeited`,
/// terminal). Like `refund`, it waits for the deductions: no deductions
/// dispute may be open, and with the dispute setting on the forfeiture must
/// have been published to the party ([`crate::deposit_deductions::publish`]
/// with a reason) and the dispute window of that version must have passed.
///
/// The approval is that of a second user who holds `deposits.refund`
/// (`request.approver`); the vertical's limits (e.g. `PRP.ACL.05`) are its own.
///
/// # Errors
///
/// `Forbidden` without `deposits.refund`; `NotFrom` unless
/// `settlement_pending`; `Guard` (422) without a reason, without an approver
/// who is another user holding `deposits.refund`, while a dispute is open, and
/// before the forfeiture is published and its window has passed.
pub async fn forfeit(
    state: &AppState,
    user: &AuthUser,
    deposit: &Deposit,
    request: &ForfeitRequest,
) -> std::result::Result<Deposit, TransitionError> {
    let deposit = reload(state, deposit).await?;
    let approver_ok = match request.approver {
        Some(id) if id != user.id => {
            renox::auth::permissions::users_with_permission(&state.db, permissions::DEPOSITS_REFUND)
                .await?
                .iter()
                .any(|u| u.id == id)
        }
        _ => false,
    };
    let input = ForfeitInput {
        reason: request.reason.trim().to_owned(),
        approver: request.approver,
        approver_ok,
        gate: Some(gate_of(state, &deposit).await?),
    };
    let _outcome = deposit
        .transition(FORFEIT)
        .by(user)
        .reason(input.reason.clone())
        .input(&input)
        .run(state)
        .await?;
    reload(state, &deposit).await
}

/// What a user gives to hand a deposit to another holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    /// The new holder, a party (the new manager or owner).
    pub new_holder: PartyId,
    /// The reference of the new holder's receipt (the document attached in
    /// wl-documents); required.
    pub receipt: String,
}

/// `user` hands the `held` `deposit` to another holder, against the new
/// holder's receipt, when the source agreement passes to it before it ends
/// (e.g. 01 management agreement `end` or `transfer_unit`). The deposit
/// becomes `transferred` (terminal) and the liability leaves the client; in the
/// same transaction the money passed on, the held amount, is recorded as an
/// outgoing payment to the new holder (by transfer, with the receipt as its
/// reference); it is not a refund. After the commit the party is told who now
/// holds the deposit (not while it is blocked: that is a message, the transfer
/// itself is not money out to the party) and `DepositSettled` (transferred) is
/// emitted.
///
/// # Errors
///
/// `Forbidden` without `deposits.manage`; `NotFrom` unless `held` (a deposit
/// in `settlement_pending` completes its settlement first); `Guard` (422)
/// without a receipt, for a new holder that does not exist or is the
/// deposit's own party, for a deposit that holds nothing, or when the payment
/// is refused (the transfer method is not enabled).
pub async fn transfer(
    state: &AppState,
    user: &AuthUser,
    deposit: &Deposit,
    request: &TransferRequest,
) -> std::result::Result<Deposit, TransitionError> {
    let deposit = reload(state, deposit).await?;
    let holder_found = wl_kernel::parties::directory(state)
        .find(&state.db, request.new_holder)
        .await?
        .is_some();
    let receipt = request.receipt.trim().to_owned();
    let input = TransferInput {
        receipt: receipt.clone(),
        new_holder: request.new_holder.0,
        holder_ok: holder_found && request.new_holder != deposit.party(),
        part_refund_open: DepositAdjustment::part_refund_open(&state.db, deposit.id).await?,
    };
    let mut payment_request = RecordPayment::new(
        request.new_holder,
        deposit.legal_entity_id,
        PaymentMethod::Transfer,
        deposit.held_amount,
    )
    .outgoing()
    .currency(&deposit.currency)
    .date(today())
    .method_details(renox::serde_json::json!({
        "deposit": deposit.id,
        "receipt": receipt,
    }));
    if !receipt.is_empty() {
        payment_request = payment_request.reference(&receipt);
    }
    let (payment, record_input) =
        crate::record::prepare(state, &payment_request, Some(user.id)).await?;
    let mut tx = state.db.begin().await?;
    let outcome = deposit
        .transition(TRANSFER)
        .by(user)
        .input(&input)
        .run_in(&mut tx)
        .await?;
    let (payment, recorded) = crate::record::record_in(&mut tx, None, payment, &record_input).await?;
    // Not the status column, so outside the machine's guard.
    Deposit::query()
        .where_(Deposit::ID.eq(deposit.id))
        .update(
            &mut tx,
            &[
                ("transferred_to", &Some(request.new_holder.0)),
                ("transfer_receipt", &Some(receipt)),
                ("transfer_payment_id", &Some(payment.id)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    recorded.emit(state).await.map_err(TransitionError::Failed)?;
    reload(state, &deposit).await
}

async fn reload(
    state: &AppState,
    deposit: &Deposit,
) -> std::result::Result<Deposit, TransitionError> {
    Deposit::find_or_404(&state.db, deposit.id)
        .await
        .map_err(TransitionError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deposit_has_five_states_and_starts_held() {
        assert_eq!(DepositStatus::default(), DepositStatus::Held);
        assert_eq!(
            DepositStatus::SettlementPending.as_str(),
            "settlement_pending"
        );
        assert_eq!(DepositStatus::Transferred.as_str(), "transferred");
    }

    #[test]
    fn only_a_forfeited_or_transferred_deposit_cannot_be_refunded() {
        assert!(DepositStatus::Held.is_refundable());
        assert!(DepositStatus::SettlementPending.is_refundable());
        assert!(DepositStatus::Refunded.is_refundable());
        assert!(!DepositStatus::Forfeited.is_refundable());
        assert!(!DepositStatus::Transferred.is_refundable());
    }

    #[test]
    fn a_deposit_is_a_subject_of_its_own_type() {
        let deposit = Deposit {
            id: 7,
            party_id: 3,
            source_type: "property.lease".into(),
            source_id: 9,
            ..Default::default()
        };
        assert_eq!(deposit.subject().to_string(), "payments.deposit:7");
        assert_eq!(deposit.source().to_string(), "property.lease:9");
        assert_eq!(deposit.party(), PartyId(3));
        assert_eq!(deposit.deposit_id(), DepositId(7));
    }

    #[test]
    fn the_amount_must_be_above_zero() {
        assert!(amount_problem(0).is_some());
        assert!(amount_problem(-1).is_some());
        assert_eq!(amount_problem(500_000), None);
    }

    #[test]
    fn a_deposit_is_paid_back_by_transfer_or_cheque_only() {
        assert_eq!(method_problem(Some(PaymentMethod::Transfer)), None);
        assert_eq!(method_problem(Some(PaymentMethod::Cheque)), None);
        assert!(method_problem(Some(PaymentMethod::Cash)).is_some());
        assert!(method_problem(None).is_some());
    }
}
