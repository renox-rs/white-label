//! The COD remittance, its shipment lines and its adjustment lines (03
//! Logistics §6 *COD remittance*, §7.1 *COD remittance*; BL-LOG-14 and
//! BL-LOG-15). A remittance's status moves only through the machine in
//! [`crate::remittance_machine`]; [`crate::remit`] builds the draft and
//! [`crate::adjustment`] holds the adjustment lines.
//!
//! The lifecycle is `draft`, `approve`, `reopen` and `pay` with the payout
//! through the kernel's payments contract, the offsets of open invoices
//! ([`crate::netting`]), `return_payment` and `repay` for a transfer the bank
//! returned (BL-LOG-17) and the `disputed` flag (BL-LOG-18).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::{PartyId, Subject};

/// The subject type of a remittance.
pub const SUBJECT_TYPE: &str = "logistics.remittance";

/// `REM-{0000}`: the number of a remittance, also the number of its
/// statement.
pub const REMITTANCE_SERIES: Series = Series {
    key: "cod_remittance",
    pattern: "{prefix}-{0000}",
    prefix: "REM",
    reset: Reset::Never,
    gapless: false,
    per_entity: false,
};

/// Where a remittance is in its life (§7.1 *COD remittance*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RemittanceStatus {
    /// Collected COD for the period, minus fees, adjustments and offsets.
    #[default]
    Draft,
    /// Checked by a second user (`LOG.ACL.07`).
    Approved,
    /// Paid to the client; the statement is in the portal.
    Paid,
    /// The bank returned the transfer; the amount is owed to the client
    /// again.
    PaymentReturned,
}

/// A payout of collected cash to a client.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "logistics_remittances",
    state_machine,
    unique(number),
    index(client_party_id),
    index(status)
)]
pub struct Remittance {
    /// The primary key.
    pub id: i64,
    /// The number from the `cod_remittance` series.
    pub number: String,
    /// The client (a party).
    pub client_party_id: i64,
    /// The end of the period it was drafted for: shipments delivered before
    /// this instant are on it.
    pub period_end: DateTime,
    /// The COD of the shipment lines, less the differences the client
    /// absorbed, in fils.
    pub cod_total: i64,
    /// The client's COD fees, in fils.
    pub fee_total: i64,
    /// The approved adjustment lines on it, signed, in fils.
    pub adjustment_total: i64,
    /// The open invoices offset against the COD (COD netting), in fils
    /// (BL-LOG-16).
    pub offset_total: i64,
    /// What is paid out: never below 0, in fils.
    pub net: i64,
    /// What a negative net was set to 0 for and carried to the next draft,
    /// in fils (a positive number).
    pub carried_forward: i64,
    /// The user who prepared it (the one who built or rebuilt the draft; none
    /// for the system's weekly run).
    pub prepared_by: Option<i64>,
    /// The user who approved it.
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// The user who paid it.
    pub paid_by: Option<i64>,
    /// When it was paid.
    pub paid_at: Option<DateTime>,
    /// The bank reference of the payment.
    pub payment_reference: Option<String>,
    /// The voucher of the payout in wl-payments.
    pub payment_id: Option<i64>,
    /// The number of that voucher.
    pub payment_number: Option<String>,
    /// The account it was paid to.
    pub bank_account: Option<String>,
    /// Why `pay` sent it back to a draft, for the accountant (BL-LOG-16).
    pub message: Option<String>,
    /// When the client's confirmation that the account the transfer was
    /// returned from is correct was recorded (BL-LOG-17).
    pub account_confirmed_at: Option<DateTime>,
    /// The user who recorded it.
    pub account_confirmed_by: Option<i64>,
    /// How the client confirmed it.
    pub account_confirmed_note: Option<String>,
    /// The client contests the statement (BL-LOG-18).
    pub disputed: bool,
    /// The ticket of the dispute.
    pub dispute_ticket_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: RemittanceStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Remittance {
    /// The remittance as a [`Subject`]: `logistics.remittance:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The client.
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The shipment lines, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn lines(&self, db: &Db) -> Result<Vec<RemittanceLine>> {
        RemittanceLine::query()
            .where_(RemittanceLine::REMITTANCE_ID.eq(self.id))
            .order_by(RemittanceLine::ID)
            .get(db)
            .await
    }

    /// The offsets of open invoices on it, oldest due date first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn offsets(&self, db: &Db) -> Result<Vec<crate::netting::Offset>> {
        crate::netting::offsets_of(db, self.id).await
    }

    /// The adjustment lines on it, oldest first (every status).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn adjustments(&self, db: &Db) -> Result<Vec<crate::adjustment::AdjustmentLine>> {
        crate::adjustment::AdjustmentLine::query()
            .where_(crate::adjustment::AdjustmentLine::REMITTANCE_ID.eq(self.id))
            .order_by(crate::adjustment::AdjustmentLine::ID)
            .get(db)
            .await
    }

    /// The returns of its transfer, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn returns(&self, db: &Db) -> Result<Vec<PaymentReturn>> {
        PaymentReturn::query()
            .where_(PaymentReturn::REMITTANCE_ID.eq(self.id))
            .order_by(PaymentReturn::ID)
            .get(db)
            .await
    }

    /// The remittance of `client` that is still a draft, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn draft_of(db: &Db, client: PartyId) -> Result<Option<Remittance>> {
        Remittance::query()
            .where_(Remittance::CLIENT_PARTY_ID.eq(client.0))
            .where_(Remittance::STATUS.eq(RemittanceStatus::Draft))
            .order_by(Remittance::ID)
            .first(db)
            .await
    }
}

/// A transfer of a remittance that the bank returned: each return keeps its
/// advice (BL-LOG-17).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_payment_returns", index(remittance_id))]
pub struct PaymentReturn {
    /// The primary key.
    pub id: i64,
    /// The remittance.
    #[model(references = "logistics_remittances")]
    pub remittance_id: i64,
    /// The bank's return advice: its reference or document.
    pub advice: String,
    /// The reason the bank gave.
    pub reason: String,
    /// The account the transfer went to.
    pub bank_account: Option<String>,
    /// The reference of the returned transfer.
    pub payment_reference: Option<String>,
    /// The voucher of the returned transfer.
    pub payment_number: Option<String>,
    /// The user who recorded the return.
    pub returned_by: Option<i64>,
    /// When.
    pub returned_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One shipment on a remittance.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "logistics_remittance_lines",
    unique(shipment_id),
    index(remittance_id)
)]
pub struct RemittanceLine {
    /// The primary key.
    pub id: i64,
    /// The remittance.
    #[model(references = "logistics_remittances")]
    pub remittance_id: i64,
    /// The shipment; on one remittance only.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// The COD recorded on the shipment, in fils.
    pub cod_amount: i64,
    /// The difference the client agreed to absorb, in fils.
    pub absorbed: i64,
    /// Its reason.
    pub absorbed_reason: String,
    /// The client's COD fee on what is paid out, in fils.
    pub fee: i64,
    /// COD less the absorbed difference less the fee, in fils.
    pub amount: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RemittanceLine {
    /// What the client is owed for the shipment before the fee: the COD less
    /// the difference it absorbed.
    pub fn collected_for_client(&self) -> i64 {
        self.cod_amount - self.absorbed
    }
}
