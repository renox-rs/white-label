//! The records of a change to a placed sales order (wl-sales §3 *revision*,
//! §4.2, §10 *Amended and cancelled orders*; BL-SAL-05, BL-SAL-06,
//! BL-SAL-08, BL-SAL-09, shared with BL-SAL-07):
//!
//! - [`OrderRevision`]: an earlier revision of the order, kept read-only;
//! - [`OrderChange`]: one change that was made, written in the transaction
//!   of the transition and read after the commit to emit the events and settle
//!   the money, so a change the kernel ran at the last approval is announced
//!   exactly like one a user ran;
//! - [`OrderSettlement`]: a money step a change led to (a payment link, a
//!   refund, a credit note, an extra or a fee invoice).

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The `kind` of an amendment.
pub const KIND_AMEND: &str = "amend";
/// The `kind` of one cancelled line.
pub const KIND_CANCEL_LINE: &str = "cancel_line";
/// The `kind` of the remaining lines cancelled.
pub const KIND_CANCEL_REMAINING: &str = "cancel_remaining";
/// The `kind` of a whole order cancelled after fulfilment started (BL-SAL-07).
pub const KIND_CANCEL: &str = "cancel";

/// An earlier revision of an order, kept read-only: what the order was
/// before the amendment that replaced it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_order_revisions", unique(order_id, revision))]
pub struct OrderRevision {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "wl_sales_sales_orders")]
    pub order_id: i64,
    /// The revision number this row keeps.
    pub revision: i64,
    /// Why the revision was replaced.
    pub reason: String,
    /// The sum of the lines before VAT, in fils.
    pub net_total: i64,
    /// The VAT, in fils.
    pub vat_total: i64,
    /// Net plus VAT, in fils.
    pub gross_total: i64,
    /// JSON: the lines of the revision ([`RevisionLine`]).
    pub lines: String,
    /// The user who replaced it.
    pub amended_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One line of a kept revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionLine {
    /// The sale line (its id).
    pub line: i64,
    /// Its position.
    pub line_no: i64,
    /// The item code.
    pub item_code: String,
    /// The description.
    pub description: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The price of one unit before VAT, in fils.
    pub unit_price: i64,
    /// The discount before VAT, in fils.
    pub discount: i64,
    /// The line with VAT, in fils.
    pub gross: i64,
}

impl OrderRevision {
    /// The lines the revision had.
    pub fn lines(&self) -> Vec<RevisionLine> {
        renox::serde_json::from_str(&self.lines).unwrap_or_default()
    }

    /// The kept revisions of `order`, oldest first.
    pub async fn of_order(db: &Db, order: i64) -> Result<Vec<OrderRevision>> {
        OrderRevision::query()
            .where_(OrderRevision::ORDER_ID.eq(order))
            .order_by(OrderRevision::REVISION)
            .get(db)
            .await
    }

    /// The kept revision `revision` of `order`, if any.
    pub async fn find_of(db: &Db, order: i64, revision: i64) -> Result<Option<OrderRevision>> {
        OrderRevision::query()
            .where_(OrderRevision::ORDER_ID.eq(order))
            .where_(OrderRevision::REVISION.eq(revision))
            .first(db)
            .await
    }
}

/// One change of an order.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_sales_order_changes", index(order_id, announced_at))]
pub struct OrderChange {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "wl_sales_sales_orders")]
    pub order_id: i64,
    /// `amend`, `cancel_line`, `cancel_remaining` or `cancel`.
    pub kind: String,
    /// Why.
    pub reason: String,
    /// The user who asked.
    pub changed_by: Option<i64>,
    /// JSON: what the kind needs to announce and settle the change.
    pub detail: String,
    /// When the events were emitted and the money settled.
    pub announced_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl OrderChange {
    /// Writes the change in `tx`, to be announced after the commit.
    pub async fn record<T: Serialize>(
        tx: &mut Transaction,
        order: i64,
        kind: &str,
        reason: &str,
        by: Option<i64>,
        detail: &T,
    ) -> Result<OrderChange> {
        let mut change = OrderChange {
            order_id: order,
            kind: kind.to_owned(),
            reason: reason.trim().to_owned(),
            changed_by: by,
            detail: renox::serde_json::to_string(detail)
                .map_err(|e| Error::Internal(renox::anyhow::Error::new(e)))?,
            ..OrderChange::default()
        };
        change.insert(&mut *tx).await?;
        Ok(change)
    }

    /// The detail, read as `T`.
    pub fn detail_as<T: DeserializeOwned>(&self) -> Option<T> {
        renox::serde_json::from_str(&self.detail).ok()
    }

    /// The changes of `order` not announced yet, oldest first.
    pub async fn unannounced(db: &Db, order: i64) -> Result<Vec<OrderChange>> {
        OrderChange::query()
            .where_(OrderChange::ORDER_ID.eq(order))
            .where_(OrderChange::ANNOUNCED_AT.is_null())
            .order_by(OrderChange::ID)
            .get(db)
            .await
    }

    /// Claims the change for announcing: `true` for the one caller that sets
    /// `announced_at`, so a change is announced once.
    pub async fn claim(db: &Db, id: i64) -> Result<bool> {
        let changed = renox::db::sql(format!(
            "UPDATE {} SET announced_at = ?, updated_at = ? WHERE id = ? AND announced_at IS NULL",
            OrderChange::TABLE
        ))
        .bind(now())
        .bind(now())
        .bind(id)
        .execute(db)
        .await?;
        Ok(changed > 0)
    }

    /// The changes of `order` of `kind`, newest first.
    pub async fn of_kind(db: &Db, order: i64, kind: &str) -> Result<Vec<OrderChange>> {
        OrderChange::query()
            .where_(OrderChange::ORDER_ID.eq(order))
            .where_(OrderChange::KIND.eq(kind))
            .order_by_desc(OrderChange::ID)
            .get(db)
            .await
    }
}

/// A payment link for an increase of a prepaid order.
pub const SETTLE_LINK: &str = "link";
/// A refund requested of `wl-payments`.
pub const SETTLE_REFUND: &str = "refund";
/// A credit note requested of `wl-invoicing` for an issued invoice.
pub const SETTLE_CREDIT_NOTE: &str = "credit_note";
/// An extra invoice for added value on an issued invoice.
pub const SETTLE_EXTRA_INVOICE: &str = "extra_invoice";
/// A cancellation fee invoiced or kept from the prepayment with a tax
/// document.
pub const SETTLE_FEE_INVOICE: &str = "fee_invoice";
/// A draft invoice updated to the new lines.
pub const SETTLE_DRAFT_UPDATED: &str = "draft_updated";

/// A link that waits for the customer to pay.
pub const STATUS_OPEN: &str = "open";
/// A link that was paid.
pub const STATUS_PAID: &str = "paid";
/// A link that expired unpaid.
pub const STATUS_EXPIRED: &str = "expired";
/// A link that was replaced by a new one.
pub const STATUS_RESENT: &str = "resent";
/// A link whose increase was amended back.
pub const STATUS_REVERTED: &str = "reverted";
/// A refund or document that was requested.
pub const STATUS_REQUESTED: &str = "requested";
/// A step that was done.
pub const STATUS_DONE: &str = "done";
/// A step the other module refused or could not do; a person must act.
pub const STATUS_FAILED: &str = "failed";

/// A money step of a change.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[allow(clippy::duplicated_attributes)] // each `index(..)` is a separate index; `status` repeats by design
#[model(
    table = "wl_sales_order_settlements",
    index(order_id, kind, status),
    index(status, due_at)
)]
pub struct OrderSettlement {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "wl_sales_sales_orders")]
    pub order_id: i64,
    /// The change that led to it.
    pub change_id: Option<i64>,
    /// The revision in force when it was made.
    pub revision: i64,
    /// `link`, `refund`, `credit_note`, `extra_invoice`, `fee_invoice` or
    /// `draft_updated`.
    pub kind: String,
    /// The amount, in fils.
    pub amount: i64,
    /// Where it stands.
    pub status: String,
    /// The link, refund, credit note or invoice in the module that owns it.
    pub reference_id: Option<i64>,
    /// The public path of a payment link.
    pub link_path: Option<String>,
    /// When a link expires.
    pub due_at: Option<DateTime>,
    /// A note for the person reading the order.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl OrderSettlement {
    /// The money steps of `order`, oldest first.
    pub async fn of_order(db: &Db, order: i64) -> Result<Vec<OrderSettlement>> {
        OrderSettlement::query()
            .where_(OrderSettlement::ORDER_ID.eq(order))
            .order_by(OrderSettlement::ID)
            .get(db)
            .await
    }

    /// The payment links of `order` in `status`, newest first.
    pub async fn links(db: &Db, order: i64, status: &str) -> Result<Vec<OrderSettlement>> {
        OrderSettlement::query()
            .where_(OrderSettlement::ORDER_ID.eq(order))
            .where_(OrderSettlement::KIND.eq(SETTLE_LINK))
            .where_(OrderSettlement::STATUS.eq(status))
            .order_by_desc(OrderSettlement::ID)
            .get(db)
            .await
    }

    /// Sets the status (and stamps the row) of settlement `id`.
    pub async fn set_status(db: &Db, id: i64, status: &str) -> Result {
        OrderSettlement::query()
            .where_(OrderSettlement::ID.eq(id))
            .update(
                db,
                &[("status", &status.to_owned()), ("updated_at", &now())],
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_revision_reads_its_lines_back() {
        let line = RevisionLine {
            line: 4,
            line_no: 1,
            item_code: "MEAL".into(),
            description: "Meal".into(),
            quantity_milli: 10_000,
            unit_price: 40_000,
            discount: 0,
            gross: 420_000,
        };
        let revision = OrderRevision {
            lines: renox::serde_json::to_string(&[line.clone()]).unwrap(),
            ..OrderRevision::default()
        };
        assert_eq!(revision.lines(), vec![line]);
        assert!(OrderRevision::default().lines().is_empty());
    }

    #[test]
    fn a_change_reads_its_detail_back() {
        let change = OrderChange {
            detail: r#"{"a":3}"#.into(),
            ..OrderChange::default()
        };
        let read: renox::serde_json::Value = change.detail_as().unwrap();
        assert_eq!(read["a"], 3);
    }
}
