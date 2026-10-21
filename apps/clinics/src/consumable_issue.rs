//! What the stock issued for a consumable of a visit (05 §4.6 step 2,
//! `STK.MOV.01`, `STK.LOT.01`): one row for each lot the quantity came from,
//! and one for the short part that became a pending issue (*lot uncertain*
//! until the lot is assigned on arrival). The rows are what a recall looks up
//! from the visit's side; the movements themselves are wl-inventory's and are
//! never edited here.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A part of the quantity of a consumable line that left the store.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_consumable_issues",
    index(visit_id),
    index(charge_line_id),
    index(lot_id)
)]
pub struct ConsumableIssue {
    /// The primary key.
    pub id: i64,
    /// The visit.
    #[model(references = "clinics_visits")]
    pub visit_id: i64,
    /// The consumable line.
    #[model(references = "clinics_charge_lines")]
    pub charge_line_id: i64,
    /// The inventory item (`inventory.item:9`).
    pub item_id: i64,
    /// The branch store it left (`inventory.location:2`).
    pub location_id: i64,
    /// The lot (`inventory.lot:4`); none for the part that is a pending issue.
    pub lot_id: Option<i64>,
    /// The lot's number.
    pub lot_number: Option<String>,
    /// The lot's last day of use.
    pub expiry_date: Option<NaiveDate>,
    /// The quantity, in thousandths of the item's base unit.
    pub quantity_milli: i64,
    /// The pending issue (`inventory.pending_issue:5`) that holds the short
    /// part: its lot is assigned when stock arrives.
    pub pending_issue_id: Option<i64>,
    /// When the stock left.
    pub issued_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ConsumableIssue {
    /// Whether the part waits for stock: its lot is not known yet.
    pub fn is_uncertain(&self) -> bool {
        self.pending_issue_id.is_some() && self.lot_id.is_none()
    }

    /// The rows of visit `visit`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_visit(db: &Db, visit: i64) -> Result<Vec<ConsumableIssue>> {
        ConsumableIssue::query()
            .where_(ConsumableIssue::VISIT_ID.eq(visit))
            .order_by(ConsumableIssue::ID)
            .get(db)
            .await
    }

    /// The rows of the charge line `line`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_line(db: &Db, line: i64) -> Result<Vec<ConsumableIssue>> {
        ConsumableIssue::query()
            .where_(ConsumableIssue::CHARGE_LINE_ID.eq(line))
            .order_by(ConsumableIssue::ID)
            .get(db)
            .await
    }

    /// The rows that took lot `lot` (`inventory.lot:4`), oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_lot(db: &Db, lot: i64) -> Result<Vec<ConsumableIssue>> {
        ConsumableIssue::query()
            .where_(ConsumableIssue::LOT_ID.eq(Some(lot)))
            .order_by(ConsumableIssue::ID)
            .get(db)
            .await
    }
}
