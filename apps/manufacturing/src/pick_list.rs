//! The material pick list of a work order (07 Manufacturing §7.1 `release`
//! effect): one line per BOM component with the quantity the work order
//! needs, generated at release and withdrawn when the work order is
//! cancelled.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// One material a work order needs.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_pick_lines", index(work_order_id))]
pub struct PickLine {
    /// The primary key.
    pub id: i64,
    /// The work order.
    pub work_order_id: i64,
    /// The material, a wl-inventory item.
    pub item_id: i64,
    /// The quantity needed, in thousandths.
    pub required_milli: i64,
    /// Whether the list was withdrawn.
    pub withdrawn: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Writes the pick list of work order `id` from `lines` (item, quantity),
/// replacing a list withdrawn earlier.
pub(crate) async fn generate(tx: &mut Transaction, id: i64, lines: &[(i64, i64)]) -> Result {
    PickLine::query()
        .where_(PickLine::WORK_ORDER_ID.eq(id))
        .delete(&mut *tx)
        .await?;
    for (item_id, required_milli) in lines {
        PickLine::create(
            &mut *tx,
            PickLine {
                work_order_id: id,
                item_id: *item_id,
                required_milli: *required_milli,
                ..PickLine::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// Withdraws the pick list of work order `id`.
pub(crate) async fn withdraw(tx: &mut Transaction, id: i64) -> Result {
    PickLine::query()
        .where_(PickLine::WORK_ORDER_ID.eq(id))
        .update(&mut *tx, &[("withdrawn", &true), ("updated_at", &now())])
        .await?;
    Ok(())
}

/// The pick list of work order `id`, in item order.
///
/// # Errors
///
/// A database error.
pub async fn of_order<'c>(db: impl renox::db::Executor<'c>, id: i64) -> Result<Vec<PickLine>> {
    PickLine::query()
        .where_(PickLine::WORK_ORDER_ID.eq(id))
        .order_by(PickLine::ID)
        .get(db)
        .await
}
