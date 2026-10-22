//! The sales order (wl-sales §3 *Sales order*, §4.2, §5.2, §5.3; base for
//! BL-SAL-22): what a customer buys on account, online or by phone, from the
//! draft to `closed`. Its status moves only through the machine in
//! [`crate::sales_order_machine`]; its lines are
//! [`SaleLine`](crate::sale_line::SaleLine)s of the subject
//! `sales.sales_order:<id>`.
//!
//! Decisions of this base, which BL-SAL-22 needs and the rest of the order
//! lifecycle builds on:
//! - All ten states of §4.2 exist, so the later transitions (`hold`,
//!   `release`, `amend`, `cancel_line`, `cancel_remaining`, BL-SAL-05 to
//!   BL-SAL-11) only add transitions. This base has `submit`,
//!   `confirm_payment`, `payment_failed`, `accept`, `reject`, `cancel`,
//!   `fulfil`, `reverse_fulfilment` and `close`.
//! - A transition has one target, so the spec's multi-target transitions are
//!   a family: `submit` (→ `confirmed`), `submit_online` (→
//!   `awaiting_payment`), `submit_for_acceptance` (→ `pending_acceptance`),
//!   `confirm_payment` (→ `confirmed`) and `confirm_payment_for_acceptance`
//!   (→ `pending_acceptance`), `fulfil` (→ `partially_fulfilled`) and
//!   `fulfil_last` (→ `fulfilled`), `reverse_fulfilment` (→
//!   `partially_fulfilled`) and `reverse_fulfilment_all` (→ `confirmed`).
//!   The service picks the one that fits.
//! - The credit check (`SAL.RUL.01`, BL-SAL-10) is not built: an order that
//!   passes `submit` or `accept` is `confirmed`.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::sales::{SALES_ORDER_SUBJECT_TYPE, SalesOrderId};
use wl_kernel::{PartyId, Subject};

/// The numbering of sales orders: `SO-2026-0001` (§7 *Numbering*).
pub const ORDER_SERIES: Series = Series {
    key: "sales_order",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "SO",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// The payment mode of an order that is paid online before it is placed.
pub const PAYMENT_ONLINE: &str = "online";

/// The payment mode of an order on account (the default).
pub const PAYMENT_ON_ACCOUNT: &str = "on_account";

/// Where a sales order is (§4.2).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SalesOrderStatus {
    /// Being entered; editable.
    #[default]
    Draft,
    /// Placed with online prepayment; waiting for the gateway.
    AwaitingPayment,
    /// Placed through a channel that needs the seller to accept it.
    PendingAcceptance,
    /// Blocked by the credit check (BL-SAL-10).
    CreditHold,
    /// Accepted; stock reserved; fulfilment can start.
    Confirmed,
    /// Some quantities delivered or collected.
    PartiallyFulfilled,
    /// All quantities delivered or collected.
    Fulfilled,
    /// Everything fulfilled is invoiced or receipted.
    Closed,
    /// Not accepted by the seller.
    Rejected,
    /// Cancelled before anything was fulfilled, or the online payment failed.
    Cancelled,
}

/// Where the online payment of an order is (§3 *Payment status*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OrderPaymentStatus {
    /// Not paid online (on account, on delivery, at the counter).
    #[default]
    NotRequired,
    /// Waiting for the gateway.
    Pending,
    /// The gateway confirmed the full total.
    Received,
    /// The payment failed or the window ended.
    Failed,
}

/// A sales order.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_sales_orders",
    state_machine,
    unique(number),
    index(party_id),
    index(invoice_id),
    index(status)
)]
pub struct SalesOrder {
    /// The primary key.
    pub id: i64,
    /// The order number, e.g. `SO-2026-0001`.
    pub number: String,
    /// The customer.
    pub party_id: i64,
    /// The legal entity whose order it is.
    pub legal_entity_id: Option<i64>,
    /// The channel it came through: `phone`, `website`, `portal`...
    pub channel: String,
    /// `on_account`, `online`, `cash_on_delivery`, `cash` or `card`.
    pub payment_mode: String,
    /// The ISO currency code.
    pub currency: String,
    /// The sum of the lines before VAT, in fils.
    pub net_total: i64,
    /// The VAT, in fils.
    pub vat_total: i64,
    /// Net plus VAT, in fils.
    pub gross_total: i64,
    /// The revision; 1 until the first `amend` (BL-SAL-05).
    pub revision: i64,
    /// The user who entered it.
    pub created_by: Option<i64>,
    /// The loyalty account attached before the order is fulfilled.
    pub loyalty_account_id: Option<i64>,
    /// The user who attached it.
    pub loyalty_attached_by: Option<i64>,
    /// When it was attached.
    pub loyalty_attached_at: Option<DateTime>,
    /// The invoice drafted when the order was fulfilled.
    pub invoice_id: Option<i64>,
    /// Why it was cancelled or rejected.
    pub reason: Option<String>,
    /// When it was confirmed.
    pub confirmed_at: Option<DateTime>,
    /// When its last open quantity was fulfilled.
    pub fulfilled_at: Option<DateTime>,
    /// The order number in the channel's own system (an aggregator's),
    /// as sent (BL-SAL-01).
    pub external_number: Option<String>,
    /// [`external_number`](Self::external_number) trimmed and lower-cased; the
    /// unique key with the channel. Empty when the number is not checked.
    pub external_key: Option<String>,
    /// Where the online payment is.
    pub payment_status: OrderPaymentStatus,
    /// When the payment window ends (set when the order waits for the gateway).
    pub payment_due_at: Option<DateTime>,
    /// The payment link of wl-payments the customer pays through.
    pub payment_link_id: Option<i64>,
    /// The path of the public page of that link.
    pub payment_link: Option<String>,
    /// The promo code a use of which the order holds (BL-SAL-02, BL-SAL-03).
    pub promo_code_id: Option<i64>,
    /// Where it is; changed only by the machine.
    pub status: SalesOrderStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The `outcome` of an [`OrderPayment`] while it is handled.
pub const PAYMENT_CLAIMED: &str = "claimed";
/// The payment confirmed the order (`confirm_payment`).
pub const PAYMENT_CONFIRMED: &str = "confirmed";
/// The payment was refunded as late or unusable (`SAL.RUL.06`).
pub const PAYMENT_REFUND_REQUESTED: &str = "refund_requested";
/// The refund of the payment was refused by wl-payments.
pub const PAYMENT_REFUND_REFUSED: &str = "refund_refused";

/// A payment the gateway confirmed for an order, handled once (BL-SAL-02,
/// BL-SAL-04): the row is the claim that makes a second `PaymentReceived`
/// for the same payment change nothing.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_order_payments", unique(payment_id), index(order_id))]
pub struct OrderPayment {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "wl_sales_sales_orders")]
    pub order_id: i64,
    /// The payment (`wl-payments`).
    pub payment_id: i64,
    /// The amount, in fils.
    pub amount: i64,
    /// What was done with it: `claimed`, `confirmed`, `refund_requested` or
    /// `refund_refused`.
    pub outcome: String,
    /// The refund `wl-payments` was asked for, if any.
    pub refund_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl OrderPayment {
    /// The payments handled for `order`, oldest first.
    pub async fn of_order(db: &Db, order: i64) -> Result<Vec<OrderPayment>> {
        OrderPayment::query()
            .where_(OrderPayment::ORDER_ID.eq(order))
            .order_by(OrderPayment::ID)
            .get(db)
            .await
    }
}

impl SalesOrderStatus {
    /// Whether the outlet, kitchen, dispatch and acceptance lists show an
    /// order in this state: not a draft, and not one still waiting for the
    /// gateway (BL-SAL-02).
    pub fn reaches_fulfilment(self) -> bool {
        !matches!(
            self,
            SalesOrderStatus::Draft | SalesOrderStatus::AwaitingPayment
        )
    }
}

impl SalesOrder {
    /// The orders the outlet, kitchen, dispatch and acceptance lists show,
    /// newest first: an order waiting for its online payment is not among
    /// them until the payment is confirmed (BL-SAL-02).
    pub async fn visible_to_fulfilment(db: &Db, limit: usize) -> Result<Vec<SalesOrder>> {
        Ok(SalesOrder::query()
            .where_(SalesOrder::STATUS.ne(SalesOrderStatus::Draft))
            .where_(SalesOrder::STATUS.ne(SalesOrderStatus::AwaitingPayment))
            .order_by_desc(SalesOrder::ID)
            .get(db)
            .await?
            .into_iter()
            .take(limit)
            .collect())
    }

    /// The order of `channel` that already has the external number `key`
    /// (trimmed, lower case).
    pub async fn of_external(db: &Db, channel: &str, key: &str) -> Result<Option<SalesOrder>> {
        SalesOrder::query()
            .where_(SalesOrder::CHANNEL.eq(channel))
            .where_(SalesOrder::EXTERNAL_KEY.eq(key))
            .first(db)
            .await
    }
}

impl SalesOrder {
    /// The order as an id.
    pub fn order_id(&self) -> SalesOrderId {
        SalesOrderId(self.id)
    }

    /// The order as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(SALES_ORDER_SUBJECT_TYPE, self.id)
    }

    /// The customer.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// Whether the order's lines can still change.
    pub fn is_draft(&self) -> bool {
        self.status == SalesOrderStatus::Draft
    }

    /// Whether the order was handed over, in whole or in part, and not yet
    /// closed or ended.
    pub fn is_fulfilled_or_partly(&self) -> bool {
        matches!(
            self.status,
            SalesOrderStatus::PartiallyFulfilled | SalesOrderStatus::Fulfilled
        )
    }

    /// Whether the order ended without being fulfilled.
    pub fn is_ended_unfulfilled(&self) -> bool {
        matches!(
            self.status,
            SalesOrderStatus::Cancelled | SalesOrderStatus::Rejected
        )
    }

    /// The order whose invoice is `invoice`, if any.
    pub async fn of_invoice(db: &Db, invoice: i64) -> Result<Option<SalesOrder>> {
        SalesOrder::query()
            .where_(SalesOrder::INVOICE_ID.eq(invoice))
            .first(db)
            .await
    }
}

/// One hand-over of an order's goods: what a dispatch note, a delivery or a
/// collection fulfilled. A source fulfils an order once.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_order_fulfilments",
    unique(order_id, source_type, source_id)
)]
pub struct FulfilmentRecord {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "wl_sales_sales_orders")]
    pub order_id: i64,
    /// The type of the hand-over.
    pub source_type: String,
    /// The id of the hand-over.
    pub source_id: i64,
    /// JSON `[{"line":1,"quantity_milli":2000}]`: what the source still holds
    /// (a reversal takes quantities off).
    pub lines: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl FulfilmentRecord {
    /// The record of the hand-over `source` on order `order`, if any.
    pub async fn of_source(
        db: &Db,
        order: i64,
        source: &Subject,
    ) -> Result<Option<FulfilmentRecord>> {
        FulfilmentRecord::query()
            .where_(FulfilmentRecord::ORDER_ID.eq(order))
            .where_(FulfilmentRecord::SOURCE_TYPE.eq(source.subject_type.clone()))
            .where_(FulfilmentRecord::SOURCE_ID.eq(source.subject_id))
            .first(db)
            .await
    }

    /// What the source still holds, as (line, quantity) pairs.
    pub fn held(&self) -> Vec<wl_kernel::sales::FulfilledLine> {
        renox::serde_json::from_str(&self.lines).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = SalesOrderStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "draft",
                "awaiting_payment",
                "pending_acceptance",
                "credit_hold",
                "confirmed",
                "partially_fulfilled",
                "fulfilled",
                "closed",
                "rejected",
                "cancelled"
            ]
        );
    }

    #[test]
    fn an_order_is_a_subject_with_its_customer() {
        let order = SalesOrder {
            id: 5,
            party_id: 3,
            ..SalesOrder::default()
        };
        assert_eq!(order.subject(), Subject::new("sales.sales_order", 5));
        assert_eq!(order.party(), PartyId(3));
        assert!(order.is_draft());
    }

    #[test]
    fn a_record_reads_its_lines_back() {
        let record = FulfilmentRecord {
            lines: r#"[{"line":4,"quantity_milli":2000}]"#.into(),
            ..FulfilmentRecord::default()
        };
        let held = record.held();
        assert_eq!((held[0].line, held[0].quantity_milli), (4, 2000));
        assert!(FulfilmentRecord::default().held().is_empty());
    }
}
