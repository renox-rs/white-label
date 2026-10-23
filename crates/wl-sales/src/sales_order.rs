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
//! - The credit check (`SAL.RUL.01`, BL-SAL-10) runs at `submit` and `accept`
//!   for an order on account whose customer has a
//!   [`CustomerAccount`](crate::customer_account::CustomerAccount): a failure
//!   is `submit_hold` / `accept_hold` (→ `credit_hold`), and `hold` puts a
//!   confirmed order there at dispatch. The state before the hold, the
//!   reason and the figures are kept on the order ([`crate::credit`]).

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
    /// The customer's own PO number, as typed.
    pub customer_po: Option<String>,
    /// The call-off number against a blanket PO, as typed.
    pub call_off: Option<String>,
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
    /// Whether `SalesOrderConfirmed` was emitted for it: a release from a
    /// hold at `submit` or `accept` emits it, one from a hold at dispatch
    /// does not.
    pub confirmed_announced: bool,
    /// When its last open quantity was fulfilled.
    pub fulfilled_at: Option<DateTime>,
    /// Whether `accept` needs the credit check of the service: the order is
    /// on account and its customer has an account (set at `submit`).
    pub credit_gate: bool,
    /// The state the order was in before `credit_hold` (`confirmed` when it
    /// was held at `submit` or `accept`); set by the hold.
    pub state_before_hold: Option<String>,
    /// Why it is on `credit_hold`: `limit`, `overdue` or `account_on_hold`.
    pub hold_reason: Option<String>,
    /// What the customer owes with this order counted, in fils, at the hold.
    pub hold_open_balance: Option<i64>,
    /// The overdue balance beyond the grace days at the hold, in fils.
    pub hold_overdue: Option<i64>,
    /// The limit checked against at the hold, in fils.
    pub hold_limit: Option<i64>,
    /// When it was put on hold.
    pub held_at: Option<DateTime>,
    /// When the last hold was released (BL-SAL-11).
    pub released_at: Option<DateTime>,
    /// What the customer owed without this order when the hold was released,
    /// in fils; the next dispatch passes while it owes no more.
    pub release_exposure: Option<i64>,
    /// The overdue balance at that release, in fils.
    pub release_overdue: Option<i64>,
    /// Where it is; changed only by the machine.
    pub status: SalesOrderStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
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

    /// Whether the order is paid on account (the credit control applies).
    pub fn is_on_account(&self) -> bool {
        self.payment_mode == PAYMENT_ON_ACCOUNT
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
