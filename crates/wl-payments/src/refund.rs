//! The Refund (wl-payments §3, §4, A11): money returned to a party out of a
//! payment's unallocated amount or out of the party's credit balance.
//!
//! The Refund base has the model, all six states ([`RefundStatus`]) and
//! four of the transitions ([`crate::refund_machine`]): [`request`] (a
//! user with `refunds.request`), [`approve`] (`refunds.approve`, never the
//! requester: `FIN.ACL.04`), [`reject`] (a reason is required) and [`pay`]
//! (`refunds.pay`): it records an outgoing payment with source `system`,
//! links it to the refund, opens an attempt ([`crate::refund_attempt`]) and
//! emits `RefundPaid`. [`request_for_source`] asks a refund of a source
//! another module owns (credit note, payer credit, order, booking);
//! [`send_to_gateway`] sends an approved `gateway` refund to the gateway
//! (attempt `pending`), and [`gateway_refund_outcome`] takes the gateway's
//! answer, once per attempt: `Confirmed` runs `pay_by_system`. `fail`
//! and `fail_by_system` close the attempt and, from `paid`, reverse the
//! outgoing payment ([`fail`]); `retry`, `change_method` and `cancel` come with the refund issues that
//! follow. The status moves only through the machine.
//!
//! A paid refund lowers the party's credit balance ([`crate::credit`]); a
//! refund is refused above the refundable balance of its source.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::payments::{
    DepositId, Direction, PAYMENT_SUBJECT_TYPE, PaymentId, PaymentMethod, PaymentSource,
    RecordPayment, RefundFailureKind, RefundId, RefundSource as KernelRefundSource, ReversalReason,
};
use wl_kernel::{LegalEntity, PartyId, Subject};

use crate::credit;
use crate::deposit::Deposit;
use crate::gateway::{RefundOutcome, gateway_of};
use crate::payment::Payment;
use crate::permissions;
use crate::record;
use crate::refund_attempt::{AttemptOutcome, RefundAttempt};
use crate::refund_fail::{FAIL, FAIL_BY_SYSTEM, FailInput};
use crate::refund_machine::{APPROVE, PAY, PAY_BY_SYSTEM, PayBySystemInput, PayInput, REJECT};
use crate::reverse;

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

/// A refund another module asks for, out of a source it owns
/// ([`request_for_source`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundRequest {
    /// What it comes from.
    pub source: KernelRefundSource,
    /// Who is paid back.
    pub party: PartyId,
    /// The legal entity paying.
    pub legal_entity_id: i64,
    /// The amount, in fils; above 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// How it is to be paid, if already known; a `gateway` refund needs
    /// [`gateway_txn`](RefundRequest::gateway_txn).
    pub method: Option<PaymentMethod>,
    /// The gateway transaction a `gateway` refund goes back to.
    pub gateway_txn: Option<String>,
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
    /// The kind of the source (`payment`, `credit_note`, `deposit`,
    /// `party_credit`, `payer_credit`, `order` or `booking`).
    pub source_kind: String,
    /// The amount, in fils; always > 0.
    pub amount: i64,
    /// The ISO currency code, e.g. `AED`.
    pub currency: String,
    /// Why it is refunded.
    pub reason: String,
    /// How it is or was paid; set by `pay`, or at the request for a
    /// gateway refund.
    pub method: Option<PaymentMethod>,
    /// The gateway transaction a `gateway` refund goes back to.
    pub gateway_txn: Option<String>,
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
    /// Why the latest attempt failed.
    pub failure_kind: Option<RefundFailureKind>,
    /// The reason given for it.
    pub failure_reason: Option<String>,
    /// The details a changed method pays to: an IBAN, or the name on a
    /// cheque that is not the party's (`change_method`).
    pub payee_details: Option<String>,
    /// The user who set them: new bank details are approved by someone else
    /// (`FIN.ACL.04`).
    pub details_by: Option<i64>,
    /// Why a failed refund was cancelled.
    pub cancel_reason: Option<String>,
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

    /// What it comes from, in the kernel contract; `None` for a kind not
    /// known (a corrupt row).
    pub fn source(&self) -> Option<KernelRefundSource> {
        KernelRefundSource::from_kind(&self.source_kind, &self.source_subject())
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
    let source = match &request.source {
        RefundSource::Payment(id) => KernelRefundSource::Payment(*id),
        RefundSource::PartyCredit { party, .. } => KernelRefundSource::PartyCredit(*party),
    };
    insert_requested(
        state,
        Some(user.id),
        &resolved,
        &source,
        request.amount,
        None,
        None,
        &request.reason,
    )
    .await
}

/// Inserts the refund `requested`, with its history row, in its own
/// transaction.
#[allow(clippy::too_many_arguments)]
async fn insert_requested(
    state: &AppState,
    requested_by: Option<i64>,
    resolved: &Resolved,
    source: &KernelRefundSource,
    amount: i64,
    method: Option<PaymentMethod>,
    gateway_txn: Option<String>,
    reason: &str,
) -> std::result::Result<Refund, TransitionError> {
    let mut tx = state.db.begin().await?;
    let refund = insert_requested_in(
        &mut tx,
        requested_by,
        resolved,
        source,
        amount,
        method,
        gateway_txn,
        reason,
    )
    .await?;
    tx.commit().await?;
    Ok(refund)
}

/// Inserts the refund `requested` in `tx`, with its history row.
#[allow(clippy::too_many_arguments)]
async fn insert_requested_in(
    tx: &mut renox::db::Transaction,
    requested_by: Option<i64>,
    resolved: &Resolved,
    source: &KernelRefundSource,
    amount: i64,
    method: Option<PaymentMethod>,
    gateway_txn: Option<String>,
    reason: &str,
) -> Result<Refund> {
    let subject = source.subject();
    let mut refund = Refund {
        party_id: resolved.party.0,
        legal_entity_id: resolved.legal_entity_id,
        source_type: subject.subject_type,
        source_id: subject.subject_id,
        source_kind: source.kind().to_owned(),
        amount,
        currency: resolved.currency.clone(),
        reason: reason.trim().to_owned(),
        method,
        gateway_txn,
        requested_by,
        status: RefundStatus::Requested,
        ..Default::default()
    };
    // Born `requested`, with a history row for it.
    let reason = refund.reason.clone();
    refund
        .insert_in_state(&mut *tx, RefundStatus::Requested, &reason)
        .await?;
    Ok(refund)
}

/// The refund of the held amount of `deposit` by `method`, requested by
/// user `by` in `tx`: the effect of the deposit's `refund`. The deposit
/// transition checked the user's permission, so none is asked here; the
/// refund still waits for an approval by another user (`FIN.ACL.04`).
pub(crate) async fn request_for_deposit_in(
    tx: &mut renox::db::Transaction,
    deposit: &Deposit,
    method: PaymentMethod,
    by: Option<i64>,
) -> Result<Refund> {
    let resolved = Resolved {
        party: deposit.party(),
        legal_entity_id: deposit.legal_entity_id,
        currency: deposit.currency.clone(),
        refundable: deposit.held_amount,
    };
    insert_requested_in(
        tx,
        by,
        &resolved,
        &KernelRefundSource::Deposit(deposit.deposit_id()),
        deposit.held_amount,
        Some(method),
        None,
        &format!("Refund of deposit {}", deposit.id),
    )
    .await
}

/// Why a gateway refund cannot be asked with `method` and `gateway_txn`,
/// if it cannot: a `gateway` refund needs the transaction it goes back to.
pub(crate) fn gateway_problem(
    method: Option<PaymentMethod>,
    gateway_txn: Option<&str>,
) -> Option<&'static str> {
    let given = gateway_txn.is_some_and(|txn| !txn.trim().is_empty());
    (method == Some(PaymentMethod::Gateway) && !given)
        .then_some("A gateway refund needs the gateway transaction.")
}

/// `user` asks to refund `request.amount` fils of a source, for the
/// module that owns it. A payment or party-credit source keeps the balance
/// check of [`request`]; a deposit is read here and refused above what it
/// holds; any other source (credit note, payer credit, order, booking) is
/// vouched for by the caller, as this module cannot read its balance. The refund still goes through [`approve`]
/// (`FIN.ACL.04`).
///
/// # Errors
///
/// `Forbidden` without `refunds.request`. `Guard` (422) for an empty
/// reason, an amount of 0 or less (or above the balance of a payment or
/// party credit), a party that does not match the source or is unknown, an
/// inactive legal entity, or a `gateway` method without its transaction.
/// Nothing is written then.
pub async fn request_for_source(
    state: &AppState,
    request: &RefundRequest,
    by: &AuthUser,
) -> std::result::Result<Refund, TransitionError> {
    if !by.has_permission(permissions::REFUNDS_REQUEST) {
        return Err(TransitionError::Forbidden);
    }
    request_for_source_as(state, request, Some(by.id)).await
}

/// [`request_for_source`] for the kernel contract
/// ([`wl_kernel::payments::PaymentRecorder::request_refund`]), which names
/// a user id (`None` = the system), not an `AuthUser`: the calling module
/// has authorized its user already, so `refunds.request` is not asked here.
/// The refund still waits for another user's approval (`FIN.ACL.04`).
///
/// # Errors
///
/// The `Guard` errors of [`request_for_source`].
pub async fn request_for_source_as(
    state: &AppState,
    request: &RefundRequest,
    by: Option<i64>,
) -> std::result::Result<Refund, TransitionError> {
    if let Some(problem) = reason_problem(&request.reason) {
        return Err(guard(problem));
    }
    if let Some(problem) = gateway_problem(request.method, request.gateway_txn.as_deref()) {
        return Err(guard(problem));
    }
    let (resolved, balance) = match &request.source {
        KernelRefundSource::Payment(id) => {
            let resolved = resolve(state, &RefundSource::Payment(*id)).await?;
            let balance = resolved.refundable;
            (resolved, balance)
        }
        KernelRefundSource::PartyCredit(party) => {
            let local = RefundSource::PartyCredit {
                party: *party,
                legal_entity_id: request.legal_entity_id,
                currency: request.currency.clone(),
            };
            let resolved = resolve(state, &local).await?;
            let balance = resolved.refundable;
            (resolved, balance)
        }
        KernelRefundSource::Deposit(id) => {
            let resolved = resolve_deposit(state, *id, request).await?;
            let balance = resolved.refundable;
            (resolved, balance)
        }
        _ => (resolve_vouched(state, request).await?, i64::MAX),
    };
    if resolved.party != request.party {
        return Err(guard("The party does not match the source."));
    }
    if let Some(problem) = amount_problem(request.amount, balance) {
        return Err(guard(problem));
    }
    let gateway_txn = request
        .gateway_txn
        .as_deref()
        .map(str::trim)
        .filter(|txn| !txn.is_empty())
        .map(str::to_owned);
    insert_requested(
        state,
        by,
        &resolved,
        &request.source,
        request.amount,
        request.method,
        gateway_txn,
        &request.reason,
    )
    .await
}

/// The deposit a refund goes back to, checked against the request: it must
/// exist, belong to the party and legal entity, be in the request's
/// currency and still be held (not forfeited or transferred); no more than
/// what it holds, less the refunds that already claim it, is refundable.
async fn resolve_deposit(
    state: &AppState,
    id: DepositId,
    request: &RefundRequest,
) -> std::result::Result<Resolved, TransitionError> {
    let deposit = Deposit::find(&state.db, id.0)
        .await?
        .ok_or_else(|| guard("The deposit was not found."))?;
    if deposit.party() != request.party {
        return Err(guard("The party does not match the source."));
    }
    if deposit.legal_entity_id != request.legal_entity_id {
        return Err(guard("The legal entity does not match the deposit."));
    }
    if !deposit
        .currency
        .eq_ignore_ascii_case(request.currency.trim())
    {
        return Err(guard("The currency does not match the deposit."));
    }
    if !deposit.status.is_refundable() {
        return Err(guard("The deposit is no longer held."));
    }
    Ok(Resolved {
        party: deposit.party(),
        legal_entity_id: deposit.legal_entity_id,
        currency: deposit.currency.clone(),
        refundable: deposit.refundable(&state.db).await?,
    })
}

/// The party and legal entity of a source another module vouches for.
async fn resolve_vouched(
    state: &AppState,
    request: &RefundRequest,
) -> std::result::Result<Resolved, TransitionError> {
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
    Ok(Resolved {
        party: request.party,
        legal_entity_id: request.legal_entity_id,
        currency: request.currency.trim().to_ascii_uppercase(),
        refundable: i64::MAX,
    })
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
    if latest_attempt_pending(state, refund).await? {
        return Err(guard("A gateway refund is waiting for the gateway."));
    }
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
    RefundAttempt::open_in(
        &mut tx,
        refund.id,
        method,
        (!reference.is_empty()).then(|| reference.to_owned()),
        None,
        Some(payment.id),
        AttemptOutcome::Paid,
    )
    .await?;
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

async fn latest_attempt_pending(
    state: &AppState,
    refund: &Refund,
) -> std::result::Result<bool, TransitionError> {
    Ok(RefundAttempt::latest(&state.db, refund.id)
        .await?
        .is_some_and(|attempt| attempt.outcome == AttemptOutcome::Pending))
}

/// `user` sends the approved gateway `refund` to the gateway, which opens
/// the next attempt `pending` with the gateway's refund reference. The
/// gateway answers later through [`gateway_refund_outcome`].
///
/// # Errors
///
/// `Forbidden` without `refunds.pay`; `NotFrom` unless `approved`; `Guard`
/// (422) unless the method is `gateway` with its transaction, or while an
/// attempt is already `pending`; `Failed` when the gateway refuses the
/// call. Nothing is written then.
pub async fn send_to_gateway(
    state: &AppState,
    user: &AuthUser,
    refund: &Refund,
) -> std::result::Result<RefundAttempt, TransitionError> {
    if !user.has_permission(permissions::REFUNDS_PAY) {
        return Err(TransitionError::Forbidden);
    }
    let refund = reload(state, refund).await?;
    if refund.status != RefundStatus::Approved {
        return Err(TransitionError::NotFrom(refund.status.as_str().to_owned()));
    }
    if refund.method != Some(PaymentMethod::Gateway) {
        return Err(guard("Only a gateway refund goes to the gateway."));
    }
    let Some(txn) = refund
        .gateway_txn
        .as_deref()
        .filter(|txn| !txn.trim().is_empty())
    else {
        return Err(guard("A gateway refund needs the gateway transaction."));
    };
    if latest_attempt_pending(state, &refund).await? {
        return Err(guard("The refund is already with the gateway."));
    }
    let sent = gateway_of(state)
        .refund(txn, refund.amount)
        .await
        .map_err(TransitionError::Failed)?;
    let mut tx = state.db.begin().await?;
    let attempt = RefundAttempt::open_in(
        &mut tx,
        refund.id,
        PaymentMethod::Gateway,
        None,
        Some(sent.reference),
        None,
        AttemptOutcome::Pending,
    )
    .await?;
    tx.commit().await?;
    Ok(attempt)
}

/// What the gateway says about the refund it holds under `gateway_ref`
/// (the webhook of a later gateway adapter; tests call it directly).
/// `Confirmed` runs the system `pay_by_system`: the refund is `paid` and
/// its attempt closed `paid`, with no outgoing payment. `RefundPaid`
/// follows after the commit.
///
/// An outcome for an attempt that is no longer `pending`, or for a
/// reference nobody holds, is ignored: nothing is changed or sent, and
/// `None` is returned (the duplicate webhook of an attempt). Otherwise the
/// refund is returned as it is now.
///
/// # Errors
///
/// `NotFrom` when the refund is not `approved` any more. `Declined` runs the
/// system `fail_by_system` with the kind `gateway_declined`; the reason is
/// the gateway's text, or [`DECLINED_BY_GATEWAY`] when it gave none.
pub async fn gateway_refund_outcome(
    state: &AppState,
    gateway_ref: &str,
    outcome: RefundOutcome,
) -> std::result::Result<Option<Refund>, TransitionError> {
    let Some(attempt) = RefundAttempt::find_by_gateway_ref(&state.db, gateway_ref).await? else {
        return Ok(None);
    };
    if attempt.outcome != AttemptOutcome::Pending {
        return Ok(None);
    }
    let refund = reload_id(state, attempt.refund_id).await?;
    match outcome {
        RefundOutcome::Confirmed => {
            let input = PayBySystemInput {
                attempt_id: attempt.id,
            };
            let _outcome = refund
                .transition(PAY_BY_SYSTEM)
                .by_system()
                .input(&input)
                .run(state)
                .await?;
            Ok(Some(reload(state, &refund).await?))
        }
        RefundOutcome::Declined { reason } => {
            let reason = reason.trim();
            let reason = if reason.is_empty() {
                DECLINED_BY_GATEWAY
            } else {
                reason
            };
            let failed = run_fail(
                state,
                &refund,
                None,
                RefundFailureKind::GatewayDeclined,
                reason,
            )
            .await?;
            Ok(Some(failed))
        }
    }
}

/// The reason stored when the gateway declines without a text.
pub const DECLINED_BY_GATEWAY: &str = "Declined by the gateway";

/// `user` fails `refund` (`approved` or `paid`): the bank returned the
/// transfer or the card cannot take it. The latest attempt is closed with
/// `kind` and `reason`; from `paid` the outgoing payment is reversed
/// (`bank_return`) in the same transaction. `RefundFailed` and the notices
/// follow after the commit.
///
/// # Errors
///
/// `Forbidden` without `refunds.pay`; `NotFrom` unless `approved` or `paid`
/// (a second `fail`); `Guard` (422) for a blank reason, for the kind
/// `gateway_declined` (only the system gives it) or when there is no attempt
/// to fail. Nothing is written then.
pub async fn fail(
    state: &AppState,
    user: &AuthUser,
    refund: &Refund,
    kind: RefundFailureKind,
    reason: &str,
) -> std::result::Result<Refund, TransitionError> {
    if !user.has_permission(permissions::REFUNDS_PAY) {
        return Err(TransitionError::Forbidden);
    }
    let refund = reload(state, refund).await?;
    run_fail(state, &refund, Some(user), kind, reason).await
}

/// Runs `fail` (by `user`) or `fail_by_system` (no user) on `refund`.
async fn run_fail(
    state: &AppState,
    refund: &Refund,
    user: Option<&AuthUser>,
    kind: RefundFailureKind,
    reason: &str,
) -> std::result::Result<Refund, TransitionError> {
    let attempt = RefundAttempt::latest(&state.db, refund.id)
        .await?
        .filter(|attempt| attempt.outcome != AttemptOutcome::Failed);
    let input = FailInput {
        kind: Some(kind),
        reason: reason.to_owned(),
        attempt_id: attempt.as_ref().map(|attempt| attempt.id),
    };
    let reason = reason.trim();
    let mut run = refund.transition(if user.is_some() { FAIL } else { FAIL_BY_SYSTEM });
    run = match user {
        Some(user) => run.by(user),
        None => run.by_system(),
    };
    if !reason.is_empty() {
        run = run.reason(reason.to_owned());
    }
    // The outgoing payment is read before the transaction opens: a second
    // connection taken while it is open can wait for one that never frees.
    let outgoing = match (refund.status, refund.payment_id) {
        (RefundStatus::Paid, Some(payment_id)) => Some(
            Payment::find(&state.db, payment_id)
                .await?
                .ok_or_else(|| guard("The outgoing payment was not found."))?,
        ),
        _ => None,
    };
    let mut tx = state.db.begin().await?;
    let failed = run.input(&input).run_in(&mut tx).await?;
    // From `paid`, the outgoing payment goes back: reversed today, its date
    // kept. An outgoing payment has no bank-return notice or deficit task.
    let mut reversed = None;
    if let Some(payment) = outgoing {
        reversed = Some(
            reverse::reverse_in(
                &mut tx,
                &payment,
                ReversalReason::BankReturn,
                reason,
                user.map(|user| user.id),
            )
            .await?,
        );
    }
    tx.commit().await?;
    if let Some(reversed) = reversed {
        reversed
            .emit(state)
            .await
            .map_err(TransitionError::Failed)?;
    }
    failed.emit(state).await.map_err(TransitionError::Failed)?;
    reload(state, refund).await
}

async fn reload_id(state: &AppState, id: i64) -> std::result::Result<Refund, TransitionError> {
    Refund::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)
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
    fn a_gateway_refund_needs_its_transaction() {
        let gateway = Some(PaymentMethod::Gateway);
        assert!(gateway_problem(gateway, None).is_some());
        assert!(gateway_problem(gateway, Some("  ")).is_some());
        assert!(gateway_problem(gateway, Some("txn_1")).is_none());
        assert!(gateway_problem(Some(PaymentMethod::Transfer), None).is_none());
        assert!(gateway_problem(None, None).is_none());
    }

    #[test]
    fn a_refund_names_its_source_by_kind() {
        let note = Subject::new("invoicing.credit_note", 7);
        let refund = Refund {
            source_type: note.subject_type.clone(),
            source_id: 7,
            source_kind: "order".into(),
            ..Default::default()
        };
        assert_eq!(
            refund.source(),
            Some(KernelRefundSource::Order(note.clone()))
        );
        let unknown = Refund {
            source_kind: "nothing".into(),
            ..refund
        };
        assert_eq!(unknown.source(), None);
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
