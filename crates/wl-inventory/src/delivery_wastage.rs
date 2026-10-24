//! The wastage of a discarded delivery (wl-delivery §4.1 `discard`,
//! `WRK.RUL.16`; wl-inventory §5.2 step 3; issue #163).
//!
//! When a perishable delivery is destroyed after a failed attempt,
//! `wl-delivery` asks the inventory through the kernel's
//! [`DeliveryWastage`]:
//!
//! - [`estimate`](DeliveryWastage::estimate) says what the goods are worth at
//!   cost, which `wl-delivery` compares with the limit of `STK.ACL.02` to ask
//!   for a second person;
//! - [`record`](DeliveryWastage::record) posts the wastage with the source
//!   (the order) as reference and the driver's reason.
//!
//! Each delivery line names the source line it was copied from. The
//! vertical's [`SaleLineStockSource`](wl_kernel::inventory::SaleLineStockSource)
//! gives its recipe: when the consumption of the line was not yet posted, the
//! exploded components of the quantity are posted as `wastage` movements
//! (theoretical, like the consumption they replace); when it was posted, no
//! second movement is made and the value of the line is the wastage reported.
//! A line the source cannot explain (a dispatch note: BL-STK-23 records its
//! own wastage on `DeliveryDiscarded`) gives nothing, and with nothing the
//! answer is `None`.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::delivery::{DeliveryFuture, DeliveryWastage, DiscardWastage, WastageRecorded};
use wl_kernel::inventory::{SaleLineStock, sale_line_stock};

use crate::item::{Item, value_of};
use crate::location::Location;
use crate::sales_cancel::waste_components;

/// The subject type of a stock movement, for a wastage reference.
pub const MOVEMENT_SUBJECT_TYPE: &str = "inventory.stock_movement";

/// The module's [`DeliveryWastage`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryWastage;

/// The recipe of each line of `request` that has one, with the quantity.
async fn recipes(state: &AppState, request: &DiscardWastage) -> Result<Vec<(SaleLineStock, i64)>> {
    let source = sale_line_stock(state);
    let mut found = Vec::new();
    for line in &request.lines {
        let Some(sale_line) = &line.source_line else {
            continue;
        };
        if let Some(recipe) = source
            .of_line(state, &request.source, sale_line.subject_id)
            .await?
        {
            found.push((recipe, line.quantity_milli));
        }
    }
    Ok(found)
}

/// The value of `quantity_milli` of a line whose whole is `recipe.value` for
/// `recipe.quantity_milli`.
fn prorated(recipe: &SaleLineStock, quantity_milli: i64) -> i64 {
    if recipe.quantity_milli <= 0 {
        return recipe.value;
    }
    i64::try_from(
        i128::from(recipe.value) * i128::from(quantity_milli) / i128::from(recipe.quantity_milli),
    )
    .unwrap_or(recipe.value)
}

/// What the goods are worth at cost.
async fn estimate(state: &AppState, request: &DiscardWastage) -> Result<i64> {
    let mut total = 0;
    for (recipe, quantity) in recipes(state, request).await? {
        if recipe.consumed {
            total += prorated(&recipe, quantity);
            continue;
        }
        for component in &recipe.components {
            let item = Item::find_or_404(&state.db, component.item.subject_id).await?;
            let need = i64::try_from(
                (i128::from(component.per_unit_milli) * i128::from(quantity) + 500) / 1_000,
            )
            .unwrap_or(0);
            total += value_of(need, item.average_cost);
        }
    }
    Ok(total)
}

/// Posts the wastage and says what it came to.
async fn record(state: &AppState, request: &DiscardWastage) -> Result<Option<WastageRecorded>> {
    let found = recipes(state, request).await?;
    if found.is_empty() {
        return Ok(None);
    }
    let mut value = 0;
    let mut reference: Option<Subject> = None;
    let mut tx = state.db.begin().await?;
    for (recipe, quantity) in &found {
        if recipe.consumed {
            // Already consumed: no second movement, the value is reported.
            value += prorated(recipe, *quantity);
            continue;
        }
        let location = Location::find_or_404(&mut tx, recipe.location.subject_id).await?;
        let wasted = waste_components(
            &mut tx,
            &request.source,
            &location,
            &recipe.components,
            *quantity,
            &request.reason,
        )
        .await?;
        value += wasted.value;
        if reference.is_none() {
            reference = wasted
                .first_movement
                .map(|id| Subject::new(MOVEMENT_SUBJECT_TYPE, id));
        }
    }
    tx.commit().await?;
    Ok(Some(WastageRecorded { reference, value }))
}

impl DeliveryWastage for InventoryWastage {
    fn estimate<'a>(
        &'a self,
        state: &'a AppState,
        request: &'a DiscardWastage,
    ) -> DeliveryFuture<'a, i64> {
        Box::pin(async move { estimate(state, request).await })
    }

    fn record<'a>(
        &'a self,
        state: &'a AppState,
        request: &'a DiscardWastage,
    ) -> DeliveryFuture<'a, Option<WastageRecorded>> {
        Box::pin(async move { record(state, request).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_of_a_consumed_line_is_worth_its_share() {
        let recipe = SaleLineStock {
            location: Subject::new("inventory.location", 1),
            components: Vec::new(),
            quantity_milli: 3_000,
            consumed: true,
            value: 9_000,
        };
        assert_eq!(prorated(&recipe, 1_000), 3_000);
        assert_eq!(prorated(&recipe, 3_000), 9_000);
    }
}
