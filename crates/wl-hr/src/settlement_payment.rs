//! The payments made against a final settlement (wl-hr §4 *Final settlement*:
//! `pay`, `dispute`; `FIN.PAY.01`; issues #212, #213).
//!
//! A payment is recorded through WPS or a bank transfer to the payee. The
//! undisputed amount of a disputed settlement is paid the same way
//! ([`record`]); `pay` takes the rest of the net in the same step
//! ([`crate::settlement_pay`]).
//!
//! Decision: the payment itself is the bank's; wl-hr records what was sent
//! (amount, method, reference, day) and checks the sums. A kernel payment
//! contract for outgoing payments to an employee does not exist, and the SIF
//! file for a settlement (WPS) is built outside this module.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::permissions;
use crate::settlement::{FinalSettlement, SettlementStatus, refuse};

/// Paid through the Wages Protection System.
pub const WPS: &str = "wps";
/// Paid by a bank transfer.
pub const TRANSFER: &str = "transfer";

/// The ways a settlement is paid.
pub const METHODS: [&str; 2] = [WPS, TRANSFER];

/// One payment against a settlement.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_settlement_payments", index(settlement_id))]
pub struct SettlementPayment {
    /// The primary key.
    pub id: i64,
    /// The settlement.
    #[model(references = "wl_hr_final_settlements")]
    pub settlement_id: i64,
    /// The amount, AED fils, positive.
    pub amount: i64,
    /// [`WPS`] or [`TRANSFER`].
    pub method: String,
    /// The bank's reference.
    pub reference: String,
    /// The day it was paid.
    pub paid_on: NaiveDate,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SettlementPayment {
    /// The payments of `settlement`, oldest first.
    pub async fn of_settlement(db: &Db, settlement: i64) -> Result<Vec<SettlementPayment>> {
        SettlementPayment::query()
            .where_(SettlementPayment::SETTLEMENT_ID.eq(settlement))
            .order_by(SettlementPayment::ID)
            .get(db)
            .await
    }
}

/// What a payment carries.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct PaymentInput {
    /// The amount, AED fils.
    pub amount: i64,
    /// [`WPS`] or [`TRANSFER`].
    pub method: String,
    /// The bank's reference.
    pub reference: String,
}

/// The sum paid against `settlement`, inside `tx`.
pub async fn paid_total_in(tx: &mut Transaction, settlement: i64) -> Result<i64> {
    let rows = SettlementPayment::query()
        .where_(SettlementPayment::SETTLEMENT_ID.eq(settlement))
        .get(&mut *tx)
        .await?;
    Ok(rows.iter().map(|p| p.amount).sum())
}

/// The sum paid against `settlement`.
pub async fn paid_total(db: &Db, settlement: i64) -> Result<i64> {
    Ok(SettlementPayment::of_settlement(db, settlement)
        .await?
        .iter()
        .map(|p| p.amount)
        .sum())
}

/// Records `input` against `settlement` inside `tx`: a positive amount by a
/// known method that keeps the sum paid within the net.
pub(crate) async fn insert_in(
    tx: &mut Transaction,
    settlement: &FinalSettlement,
    input: &PaymentInput,
    by: Option<i64>,
) -> Result<SettlementPayment> {
    if input.amount <= 0 {
        return Err(refuse("wl_hr.settlement.payment_amount_required"));
    }
    if !METHODS.contains(&input.method.trim()) {
        return Err(refuse("wl_hr.settlement.payment_method_required"));
    }
    let total = paid_total_in(&mut *tx, settlement.id).await?;
    if total + input.amount > settlement.net {
        return Err(refuse("wl_hr.settlement.payment_above_net"));
    }
    let mut payment = SettlementPayment {
        settlement_id: settlement.id,
        amount: input.amount,
        method: input.method.trim().to_owned(),
        reference: input.reference.trim().to_owned(),
        paid_on: today(),
        recorded_by: by,
        ..SettlementPayment::default()
    };
    payment.insert(&mut *tx).await?;
    Ok(payment)
}

/// `by` (a holder of `settlements.pay`) records a payment against settlement
/// `id`, which is `approved` or `disputed` (the undisputed amount is paid on
/// time while the dispute stands). The state does not change; audited.
///
/// # Errors
///
/// `Forbidden` without `settlements.pay`; 422 in any other state, for an
/// amount that is not positive, an unknown method, or a sum above the net.
pub async fn record(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    input: PaymentInput,
) -> Result<SettlementPayment> {
    if !by.has_permission(permissions::SETTLEMENTS_PAY) {
        return Err(Error::Forbidden);
    }
    let mut tx = state.db.begin().await?;
    let settlement = FinalSettlement::load_in(&mut tx, id).await?;
    if !matches!(
        settlement.status,
        SettlementStatus::Approved | SettlementStatus::Disputed
    ) {
        return Err(refuse("wl_hr.settlement.payment_state"));
    }
    let payment = insert_in(&mut tx, &settlement, &input, Some(by.id)).await?;
    tx.commit().await?;
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new("hr.settlement_payment_recorded")
            .user(by.id)
            .subject(FinalSettlement::TABLE, id)
            .data(json!({
                "amount": payment.amount,
                "method": payment.method,
                "reference": payment.reference,
            })),
    )
    .await?;
    Ok(payment)
}
