//! The counter sale (wl-sales §3, §4.3, §5.4; base for BL-SAL-22): a cart
//! rung up at a till that is paid, or charged to the customer account, and
//! never edited or voided afterwards (`SAL.ACL.04`: a completed sale is
//! reversed by a sales return with a credit note).
//!
//! It has two states, `open` and `completed`, and moves only through the
//! machine in [`crate::counter_sale_machine`]. Its lines are
//! [`SaleLine`](crate::sale_line::SaleLine)s of the subject
//! `sales.counter_sale:<id>`; what was taken is a list of
//! [`CounterSalePayment`]s.
//!
//! Decisions of this base, which BL-SAL-22 needs and the rest of the till
//! lifecycle builds on:
//! - The till session (`open → counting → closed → reviewed`, §4.3) is another
//!   issue: a sale only remembers the id of the session it was rung up on.
//! - The payments of the tenders are recorded by the till through
//!   `wl-payments`; the sale keeps the payment id a tender names, which is how
//!   a reversed or charged-back payment finds its sale (`SAL.RUL.09`).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::sales::{COUNTER_SALE_SUBJECT_TYPE, CounterSaleId};
use wl_kernel::{PartyId, Subject};

/// The numbering of counter sales and their receipts: `RCPT-2026-0001`
/// (§7 *Numbering*). A cart that is abandoned may leave a gap.
pub const RECEIPT_SERIES: Series = Series {
    key: "counter_sale",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "RCPT",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// Where a counter sale is (§4.3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CounterSaleStatus {
    /// The cart: lines can be added.
    #[default]
    Open,
    /// Paid or charged to the customer account; locked.
    Completed,
}

/// A counter sale.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_counter_sales",
    state_machine,
    unique(number),
    index(party_id),
    index(invoice_id),
    index(status)
)]
pub struct CounterSale {
    /// The primary key.
    pub id: i64,
    /// The receipt number, e.g. `RCPT-2026-0001`.
    pub number: String,
    /// The legal entity whose sale it is.
    pub legal_entity_id: Option<i64>,
    /// The customer, when the sale names one.
    pub party_id: Option<i64>,
    /// The till session it was rung up on.
    pub till_session_id: Option<i64>,
    /// The cashier who rang it up.
    pub cashier_user_id: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The sum of the lines before VAT, in fils.
    pub net_total: i64,
    /// The VAT, in fils.
    pub vat_total: i64,
    /// Net plus VAT, in fils.
    pub gross_total: i64,
    /// The loyalty account attached before completion (`SAL.ACL.08`).
    pub loyalty_account_id: Option<i64>,
    /// The user who attached it.
    pub loyalty_attached_by: Option<i64>,
    /// When it was attached.
    pub loyalty_attached_at: Option<DateTime>,
    /// The invoice drafted for the sale in `wl-invoicing`.
    pub invoice_id: Option<i64>,
    /// When the sale completed.
    pub completed_at: Option<DateTime>,
    /// Where it is; changed only by the machine.
    pub status: CounterSaleStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CounterSale {
    /// The sale as an id.
    pub fn sale_id(&self) -> CounterSaleId {
        CounterSaleId(self.id)
    }

    /// The sale as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(COUNTER_SALE_SUBJECT_TYPE, self.id)
    }

    /// The customer, if the sale names one.
    pub fn party(&self) -> Option<PartyId> {
        self.party_id.map(PartyId)
    }

    /// Whether the cart can still change.
    pub fn is_open(&self) -> bool {
        self.status == CounterSaleStatus::Open
    }

    /// The sale paid with the payment `payment`, if any.
    pub async fn of_payment(db: &Db, payment: i64) -> Result<Option<CounterSale>> {
        let tender = CounterSalePayment::query()
            .where_(CounterSalePayment::PAYMENT_ID.eq(payment))
            .order_by(CounterSalePayment::ID)
            .first(db)
            .await?;
        match tender {
            Some(tender) => CounterSale::find(db, tender.counter_sale_id).await,
            None => Ok(None),
        }
    }

    /// The sale whose invoice is `invoice`, if any.
    pub async fn of_invoice(db: &Db, invoice: i64) -> Result<Option<CounterSale>> {
        CounterSale::query()
            .where_(CounterSale::INVOICE_ID.eq(invoice))
            .first(db)
            .await
    }

    /// What was taken for the sale, in the order it was entered.
    pub async fn payments(&self, db: &Db) -> Result<Vec<CounterSalePayment>> {
        CounterSalePayment::query()
            .where_(CounterSalePayment::COUNTER_SALE_ID.eq(self.id))
            .order_by(CounterSalePayment::ID)
            .get(db)
            .await
    }
}

/// What was taken for a completed counter sale with one payment method.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_counter_sale_payments",
    index(counter_sale_id),
    index(payment_id)
)]
pub struct CounterSalePayment {
    /// The primary key.
    pub id: i64,
    /// The sale.
    #[model(references = "wl_sales_counter_sales")]
    pub counter_sale_id: i64,
    /// `cash`, `card`, `account` or `other`.
    pub method: String,
    /// The amount, in fils.
    pub amount: i64,
    /// The `wl-payments` payment the till recorded, if any.
    pub payment_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sale_is_a_subject_and_open_until_completed() {
        let sale = CounterSale {
            id: 9,
            party_id: Some(4),
            ..CounterSale::default()
        };
        assert!(sale.is_open());
        assert_eq!(sale.subject(), Subject::new("sales.counter_sale", 9));
        assert_eq!(sale.party(), Some(PartyId(4)));
        let done = CounterSale {
            status: CounterSaleStatus::Completed,
            ..sale
        };
        assert!(!done.is_open());
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = CounterSaleStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, ["open", "completed"]);
    }
}
