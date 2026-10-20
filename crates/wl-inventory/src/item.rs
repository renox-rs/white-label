//! The Item (wl-inventory §3): anything stocked or bought. Only what receiving
//! needs is here: the code, the name, the base unit, whether it is
//! lot-tracked, whether its new lots start in quarantine and its moving
//! weighted average cost. Quantities are kept in thousandths of the base unit.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::ITEM_SUBJECT_TYPE;

/// What kind of thing an item is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ItemType {
    /// Kept in stock.
    #[default]
    Stocked,
    /// Bought but not kept.
    NonStocked,
    /// Used up on site.
    Consumable,
    /// Made by the company.
    Finished,
    /// Part of a finished item.
    Component,
    /// Not a thing.
    Service,
}

/// An item. Money is `i64` fils of the base unit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_items", unique(code))]
pub struct Item {
    /// The primary key.
    pub id: i64,
    /// The code, e.g. `CHK-FRZ`.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// The kind of item.
    pub item_type: ItemType,
    /// The unit quantities are kept in: `kg`, `pcs`, `m`.
    pub base_unit: String,
    /// Whether every receipt and issue records a lot.
    pub lot_tracked: bool,
    /// Whether new lots start in `quarantine`.
    pub quarantine_new_lots: bool,
    /// Days from production to expiry.
    pub shelf_life_days: Option<i64>,
    /// The moving weighted average cost of one base unit.
    pub average_cost: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Item {
    /// An item with `code`, `name` and the `base_unit`.
    pub fn new(code: &str, name: &str, base_unit: &str) -> Self {
        Item {
            code: code.trim().to_owned(),
            name_en: name.trim().to_owned(),
            base_unit: base_unit.trim().to_owned(),
            ..Item::default()
        }
    }

    /// Tracked by lot.
    pub fn lot_tracked(mut self) -> Self {
        self.lot_tracked = true;
        self
    }

    /// The item as a [`Subject`]: `inventory.item:9`.
    pub fn subject(&self) -> Subject {
        Subject::new(ITEM_SUBJECT_TYPE, self.id)
    }
}

/// The moving weighted average cost of an item after `quantity_milli`
/// thousandths come in at `price` (fils of one base unit), when
/// `on_hand_milli` thousandths were on hand at `average` (§3 *Costing*),
/// rounded half up. Nothing on hand (or less) takes the price.
pub fn weighted_average(on_hand_milli: i64, average: i64, quantity_milli: i64, price: i64) -> i64 {
    if on_hand_milli <= 0 {
        return price;
    }
    let total = i128::from(on_hand_milli) + i128::from(quantity_milli);
    if total <= 0 {
        return price;
    }
    let value = i128::from(on_hand_milli) * i128::from(average)
        + i128::from(quantity_milli) * i128::from(price);
    i64::try_from((2 * value + total) / (2 * total)).unwrap_or(price)
}

/// The moving weighted average cost after `quantity_milli` thousandths that
/// were bought at `price` leave again (a reversed receipt), when
/// `on_hand_milli` were on hand at `average`: the value left over the
/// quantity left, rounded half up. When nothing (or less) is left the
/// average stays, as the next receipt takes its own price.
pub fn average_after_removal(
    on_hand_milli: i64,
    average: i64,
    quantity_milli: i64,
    price: i64,
) -> i64 {
    let left = i128::from(on_hand_milli) - i128::from(quantity_milli);
    if left <= 0 {
        return average;
    }
    let value = i128::from(on_hand_milli) * i128::from(average)
        - i128::from(quantity_milli) * i128::from(price);
    i64::try_from(((2 * value + left) / (2 * left)).max(0)).unwrap_or(average)
}

/// The moving weighted average cost after `delta` fils (a landed cost, or a
/// refund of one when negative) are put on the `on_hand_milli` thousandths
/// that were on hand at `average`: the value plus `delta`, over the quantity,
/// rounded half up. The value stops at zero. Nothing on hand keeps the
/// average.
pub fn average_after_revalue(on_hand_milli: i64, average: i64, delta: i64) -> i64 {
    if on_hand_milli <= 0 {
        return average;
    }
    let value = (i128::from(value_of(on_hand_milli, average)) + i128::from(delta)).max(0);
    let on_hand = i128::from(on_hand_milli);
    i64::try_from((2 * value * 1_000 + on_hand) / (2 * on_hand)).unwrap_or(average)
}

/// `quantity_milli` thousandths as a number of base units for a message:
/// `80`, `12.5`, `0.125`.
pub fn quantity_text(quantity_milli: i64) -> String {
    let sign = if quantity_milli < 0 { "-" } else { "" };
    let whole = quantity_milli.abs() / 1_000;
    let fraction = quantity_milli.abs() % 1_000;
    if fraction == 0 {
        return format!("{sign}{whole}");
    }
    let fraction = format!("{fraction:03}");
    format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
}

/// The value of `quantity_milli` thousandths at `unit_cost` fils a unit,
/// rounded half up, in fils.
pub fn value_of(quantity_milli: i64, unit_cost: i64) -> i64 {
    let value = i128::from(quantity_milli) * i128::from(unit_cost);
    i64::try_from((2 * value + 1_000) / 2_000).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_is_a_subject_and_starts_without_cost() {
        let item = Item::new(" CHK ", " Chicken ", "kg").lot_tracked();
        assert_eq!(
            (item.code.as_str(), item.name_en.as_str()),
            ("CHK", "Chicken")
        );
        assert!(item.lot_tracked);
        assert_eq!(item.average_cost, 0);
        assert_eq!(
            Item { id: 9, ..item }.subject().to_string(),
            "inventory.item:9"
        );
    }

    #[test]
    fn the_average_cost_moves_with_what_comes_in() {
        // 100 kg at 10.00 and 100 kg at 12.00.
        assert_eq!(weighted_average(100_000, 1_000, 100_000, 1_200), 1_100);
        // Nothing on hand: the price.
        assert_eq!(weighted_average(0, 500, 80_000, 1_200), 1_200);
        assert_eq!(weighted_average(-5_000, 500, 80_000, 1_200), 1_200);
        // Rounded half up: (1 kg at 0.01 + 1 kg at 0.02) / 2 = 0.015 -> 0.02.
        assert_eq!(weighted_average(1_000, 1, 1_000, 2), 2);
    }

    #[test]
    fn the_average_cost_comes_back_when_a_receipt_leaves() {
        // 100 kg at 10.00 and 80 kg at 12.00 (average 10.89); the 80 leave.
        assert_eq!(weighted_average(100_000, 1_000, 80_000, 1_200), 1_089);
        assert_eq!(average_after_removal(180_000, 1_089, 80_000, 1_200), 1_000);
        // Nothing left: the average stays.
        assert_eq!(average_after_removal(80_000, 1_200, 80_000, 1_200), 1_200);
    }

    #[test]
    fn a_quantity_reads_as_base_units() {
        assert_eq!(quantity_text(80_000), "80");
        assert_eq!(quantity_text(12_500), "12.5");
        assert_eq!(quantity_text(125), "0.125");
        assert_eq!(quantity_text(0), "0");
    }

    #[test]
    fn a_landed_cost_raises_the_average_by_its_share_of_the_quantity() {
        // 60 units at AED 10.00 (1,000 fils) and AED 600 more: AED 20.00.
        assert_eq!(average_after_revalue(60_000, 1_000, 60_000), 2_000);
        // A refund of AED 100: AED 8.33 (833.33 fils).
        assert_eq!(average_after_revalue(60_000, 1_000, -10_000), 833);
        // The value stops at zero.
        assert_eq!(average_after_revalue(60_000, 1_000, -90_000), 0);
        assert_eq!(average_after_revalue(0, 1_000, 5_000), 1_000);
    }

    #[test]
    fn a_value_is_rounded_half_up_to_fils() {
        assert_eq!(value_of(80_000, 1_200), 96_000);
        assert_eq!(value_of(500, 3), 2);
        assert_eq!(value_of(0, 100), 0);
    }
}
