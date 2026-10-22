//! The purchase price variance of a receipt that covers a negative balance
//! (wl-inventory §3 *Negative stock*, issue #238 / BL-STK-19).
//!
//! At a location set to `allow_consumption` automatic consumption may take an
//! item that is not lot-tracked below zero at the current average cost (see
//! [`crate::stock_service`]). The next receipt at the location covers the
//! negative quantity first: that part is valued at the receipt's unit cost,
//! and `(receipt cost - cost used) x quantity` is the price variance. It is
//! kept as a [`PriceVariance`] row in the receipt's transaction
//! ([`record_in`]) and posted as cost coded `price_variance` after the
//! commit ([`announce`], once per row: the key makes a repeat harmless).
//!
//! Decisions: the cost used is the item's average cost when the receipt posts
//! (a negative consumption does not change it); a part that is not negative
//! (the receipt is larger than the shortage) is received as usual and has no
//! variance; a variance of 0 leaves no row.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::GOODS_RECEIPT_SUBJECT_TYPE;
use wl_kernel::invoicing::{PostCost, post_cost};

use crate::balance;
use crate::item::{Item, value_of};
use crate::location::NegativeStockPolicy;
use crate::location::Location;

/// The cost code of the price variance.
pub const COST_CODE: &str = "price_variance";

/// The difference between a receipt's cost and the cost already used for a
/// negative quantity it covered.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_price_variances",
    index(receipt_id),
    index(item_id, location_id)
)]
pub struct PriceVariance {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The location that was negative.
    pub location_id: i64,
    /// The goods receipt that covered it.
    pub receipt_id: i64,
    /// The receipt line.
    pub receipt_line_id: i64,
    /// The negative quantity covered, in thousandths.
    pub quantity_milli: i64,
    /// The unit cost the consumption used, in fils.
    pub cost_used: i64,
    /// The unit cost of the receipt, in fils.
    pub receipt_cost: i64,
    /// `(receipt cost - cost used) x quantity`, in fils.
    pub amount: i64,
    /// Whether the cost was posted.
    pub posted: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PriceVariance {
    /// The idempotency key of the cost posted for the row.
    pub fn key(&self) -> String {
        format!("inventory.price_variance:{}", self.id)
    }

    /// The variances of receipt `receipt_id`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_receipt(db: &Db, receipt_id: i64) -> Result<Vec<PriceVariance>> {
        PriceVariance::query()
            .where_(PriceVariance::RECEIPT_ID.eq(receipt_id))
            .order_by(PriceVariance::ID)
            .get(db)
            .await
    }
}

/// The negative quantity of `item` at `location_id` that `accepted_milli`
/// received covers, in thousandths (0 when the balance is not negative).
pub fn covered_milli(on_hand_milli: i64, accepted_milli: i64) -> i64 {
    if on_hand_milli >= 0 {
        return 0;
    }
    (-on_hand_milli).min(accepted_milli)
}

/// The variance of `covered_milli` thousandths valued at `receipt_cost`
/// instead of `cost_used` (fils a unit), in fils.
pub fn variance_of(covered_milli: i64, cost_used: i64, receipt_cost: i64) -> i64 {
    value_of(covered_milli, receipt_cost) - value_of(covered_milli, cost_used)
}

/// In the receipt's transaction, before its movement for the line is
/// inserted: records the variance of the negative quantity `accepted_milli`
/// received at `location_id` covers. Nothing for a lot-tracked item or a
/// balance that is not negative.
///
/// # Errors
///
/// A database error.
pub async fn record_in(
    tx: &mut Transaction,
    item: &Item,
    location_id: i64,
    receipt_id: i64,
    receipt_line_id: i64,
    accepted_milli: i64,
    receipt_cost: i64,
) -> Result<Option<PriceVariance>> {
    if item.lot_tracked {
        return Ok(None);
    }
    let location = Location::find_or_404(&mut *tx, location_id).await?;
    if location.negative_stock_policy != NegativeStockPolicy::AllowConsumption {
        return Ok(None);
    }
    let on_hand = balance::on_hand_at(&mut *tx, item.id, location_id, None).await?;
    let covered = covered_milli(on_hand, accepted_milli);
    if covered == 0 {
        return Ok(None);
    }
    let amount = variance_of(covered, item.average_cost, receipt_cost);
    if amount == 0 {
        return Ok(None);
    }
    let mut row = PriceVariance {
        item_id: item.id,
        location_id,
        receipt_id,
        receipt_line_id,
        quantity_milli: covered,
        cost_used: item.average_cost,
        receipt_cost,
        amount,
        posted: false,
        ..PriceVariance::default()
    };
    row.insert(&mut *tx).await?;
    Ok(Some(row))
}

/// After the commit of a receipt: posts the variances not posted yet as cost
/// coded `price_variance` on the receipt.
///
/// # Errors
///
/// A database error, or the cost poster's.
pub async fn announce(state: &AppState, receipt_id: i64) -> Result {
    let source = Subject::new(GOODS_RECEIPT_SUBJECT_TYPE, receipt_id);
    for row in PriceVariance::of_receipt(&state.db, receipt_id)
        .await?
        .iter()
        .filter(|row| !row.posted)
    {
        let request = PostCost::new(source.clone(), row.key(), source.clone(), row.amount)
            .cost_code(COST_CODE);
        post_cost(state, request).await?;
        PriceVariance::query()
            .where_(PriceVariance::ID.eq(row.id))
            .update(
                &state.db,
                &[("posted", &true), ("updated_at", &renox::db::now())],
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_receipt_covers_the_negative_quantity_first() {
        assert_eq!(covered_milli(-3_000, 10_000), 3_000);
        assert_eq!(covered_milli(-3_000, 2_000), 2_000);
        assert_eq!(covered_milli(0, 10_000), 0);
        assert_eq!(covered_milli(4_000, 10_000), 0);
    }

    #[test]
    fn the_variance_is_the_cost_difference_on_the_covered_quantity() {
        // 3 kg used at AED 3, received at AED 4: AED 3.
        assert_eq!(variance_of(3_000, 300, 400), 300);
        assert_eq!(variance_of(3_000, 400, 300), -300);
        assert_eq!(variance_of(3_000, 300, 300), 0);
    }
}
