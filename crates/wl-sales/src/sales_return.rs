//! The sales return (wl-sales §3, §4.5, §5.6; base for BL-SAL-22): goods that
//! come back from a counter sale or a sales order, or a refund after
//! completion with nothing to return. Its status moves only through the
//! machine in [`crate::sales_return_machine`]; the quantities it takes back
//! are [`SalesReturnLine`]s.
//!
//! Decisions of this base, which BL-SAL-22 needs and the rest of the return
//! lifecycle builds on:
//! - A return stores the value it takes back (before VAT, VAT, with VAT) and
//!   the part the loyalty programme counts, computed from the sale's lines
//!   when it is requested, so a later reader does not recompute them.
//! - The credit note or the refund is made by the caller (wl-invoicing's
//!   credit note flow, wl-payments' refund) and named when the return is
//!   resolved; the approval of a refund after completion with nothing
//!   returned (`SAL.ACL.06`, BL-SAL-14) and the stock movements of
//!   `wl-inventory` (BL-SAL-13, BL-SAL-15) are not built here.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::sales::{SALES_RETURN_SUBJECT_TYPE, SalesReturnId};
use wl_kernel::{PartyId, Subject};

/// The numbering of sales returns: `RET-2026-0001` (§7 *Numbering*).
pub const RETURN_SERIES: Series = Series {
    key: "sales_return",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "RET",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// Where a sales return is (§4.5).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReturnStatus {
    /// The customer asked to return goods.
    #[default]
    Requested,
    /// The goods are back, being inspected.
    Received,
    /// Credited, refunded or replaced.
    Resolved,
    /// A refund made by the channel was taken back.
    Reversed,
    /// Not accepted; goods handed back.
    Rejected,
}

/// Whether goods come back.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReturnKind {
    /// Goods come back and are inspected.
    #[default]
    Goods,
    /// Food or a service: nothing to return; resolved with a complaint.
    NoGoods,
}

/// How a return was resolved.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReturnResolution {
    /// A credit note in `wl-invoicing`.
    #[default]
    CreditNote,
    /// A refund in `wl-payments`.
    Refund,
    /// A replacement order at zero price.
    Replacement,
    /// The aggregator refunded its customer itself (no money moves).
    RefundedByChannel,
}

/// A sales return.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_sales_returns",
    state_machine,
    unique(number),
    index(sale_type, sale_id),
    index(credit_note_id),
    index(status)
)]
pub struct SalesReturn {
    /// The primary key.
    pub id: i64,
    /// The return number, e.g. `RET-2026-0001`.
    pub number: String,
    /// The type of the sale returned.
    pub sale_type: String,
    /// The id of the sale returned.
    pub sale_id: i64,
    /// The customer, when the sale names one.
    pub party_id: Option<i64>,
    /// Goods or no goods.
    pub kind: ReturnKind,
    /// How it was resolved.
    pub resolution: Option<ReturnResolution>,
    /// Why the customer returns.
    pub reason: String,
    /// The value returned before VAT, in fils.
    pub net_value: i64,
    /// The VAT of it, in fils.
    pub vat_value: i64,
    /// The value returned with VAT, in fils.
    pub gross_value: i64,
    /// The part of `net_value` the loyalty programme counts, in fils.
    pub eligible_net: i64,
    /// The credit note issued for it.
    pub credit_note_id: Option<i64>,
    /// The recall (`inventory.recall:N`) the goods joined when they were
    /// received (BL-SAL-15): the recall, not the return, decides their outcome.
    pub recall_id: Option<i64>,
    /// The complaint category (no goods).
    pub complaint_category: Option<String>,
    /// The responsible party (no goods).
    pub responsible_party: Option<String>,
    /// Why it was rejected or reversed.
    pub closing_reason: Option<String>,
    /// The user who requested it.
    pub requested_by: Option<i64>,
    /// The user who resolved it.
    pub handled_by: Option<i64>,
    /// When the goods were received.
    pub received_at: Option<DateTime>,
    /// When it was resolved.
    pub resolved_at: Option<DateTime>,
    /// Where it is; changed only by the machine.
    pub status: ReturnStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SalesReturn {
    /// The return as an id.
    pub fn return_id(&self) -> SalesReturnId {
        SalesReturnId(self.id)
    }

    /// The return as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(SALES_RETURN_SUBJECT_TYPE, self.id)
    }

    /// The sale returned as a kernel subject.
    pub fn sale(&self) -> Subject {
        Subject::new(&self.sale_type, self.sale_id)
    }

    /// The customer, when the sale names one.
    pub fn party(&self) -> Option<PartyId> {
        self.party_id.map(PartyId)
    }

    /// The recall the goods joined, as a kernel subject.
    pub fn recall(&self) -> Option<Subject> {
        self.recall_id
            .map(|id| Subject::new(wl_kernel::inventory::RECALL_SUBJECT_TYPE, id))
    }

    /// Whether resolving the return restores or writes off stock: not for
    /// goods under a recall, which the recall decides (BL-SAL-15).
    pub fn stock_follows_the_return(&self) -> bool {
        self.recall_id.is_none()
    }

    /// The returns of the sale `sale`, oldest first.
    pub async fn of_sale(db: &Db, sale: &Subject) -> Result<Vec<SalesReturn>> {
        SalesReturn::query()
            .where_(SalesReturn::SALE_TYPE.eq(sale.subject_type.clone()))
            .where_(SalesReturn::SALE_ID.eq(sale.subject_id))
            .order_by(SalesReturn::ID)
            .get(db)
            .await
    }

    /// The return resolved with the credit note `credit_note`, if any.
    pub async fn of_credit_note(db: &Db, credit_note: i64) -> Result<Option<SalesReturn>> {
        SalesReturn::query()
            .where_(SalesReturn::CREDIT_NOTE_ID.eq(credit_note))
            .first(db)
            .await
    }

    /// The quantities the return takes back.
    pub async fn lines(&self, db: &Db) -> Result<Vec<SalesReturnLine>> {
        SalesReturnLine::query()
            .where_(SalesReturnLine::RETURN_ID.eq(self.id))
            .order_by(SalesReturnLine::ID)
            .get(db)
            .await
    }
}

/// The quantity of one sale line a return takes back.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_sales_return_lines",
    index(return_id),
    index(sale_line_id)
)]
pub struct SalesReturnLine {
    /// The primary key.
    pub id: i64,
    /// The return.
    #[model(references = "wl_sales_sales_returns")]
    pub return_id: i64,
    /// The sale line.
    pub sale_line_id: i64,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The lot (`inventory.lot:N`, its id) the customer presents, for an item
    /// tracked by lot: a lot under an open recall is recalled goods.
    pub lot_id: Option<i64>,
    /// The recall (`inventory.recall:N`, its id) this line is to join, written
    /// before the goods are received: a line pending and not joined is retried.
    pub recall_pending_id: Option<i64>,
    /// When the line joined the recall's hold.
    pub recall_joined_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_kinds_and_resolutions_are_stored_in_snake_case() {
        fn names(all: Vec<&str>) -> Vec<&str> {
            all
        }
        assert_eq!(
            names(ReturnStatus::ALL.iter().map(|s| s.as_str()).collect()),
            ["requested", "received", "resolved", "reversed", "rejected"]
        );
        assert_eq!(
            names(ReturnKind::ALL.iter().map(|s| s.as_str()).collect()),
            ["goods", "no_goods"]
        );
        assert_eq!(
            names(ReturnResolution::ALL.iter().map(|s| s.as_str()).collect()),
            [
                "credit_note",
                "refund",
                "replacement",
                "refunded_by_channel"
            ]
        );
    }

    #[test]
    fn a_return_names_its_sale() {
        let r = SalesReturn {
            id: 2,
            sale_type: "sales.counter_sale".into(),
            sale_id: 9,
            ..SalesReturn::default()
        };
        assert_eq!(r.sale(), Subject::new("sales.counter_sale", 9));
        assert_eq!(r.subject(), Subject::new("sales.sales_return", 2));
    }
}
