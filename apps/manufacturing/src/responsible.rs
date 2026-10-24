//! Who is told that a conformity certificate of an item expires (07 §7.1
//! settings *Responsible role per document kind*, BL-MFG-14): the QC
//! inspectors of each plant that holds stock of the item or has an open work
//! order for it; when no plant does, the QC inspectors of every plant (the
//! item has no scope of its own).

use renox::auth::permissions::Scope;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::{DocumentFuture, ReminderSchedule};
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, stock_service};

use crate::permissions;
use crate::plant::{Line, Plant};
use crate::work_order::WorkOrder;

/// The reminder scopes of the certificates of an item.
pub struct ItemPlants;

impl ReminderSchedule for ItemPlants {
    fn scopes<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<Vec<Scope>>> {
        Box::pin(async move {
            if subject.subject_type != ITEM_SUBJECT_TYPE {
                return Ok(None);
            }
            let mut found = Vec::new();
            for plant in Plant::query()
                .where_(Plant::ACTIVE.eq(true))
                .order_by(Plant::ID)
                .get(&state.db)
                .await?
            {
                if holds(state, &plant, subject).await? {
                    found.push(permissions::plant_scope(plant.id));
                }
            }
            Ok((!found.is_empty()).then_some(found))
        })
    }
}

/// Whether `plant` has an open work order for the item or stock of it at the
/// locations of its lines.
async fn holds(state: &AppState, plant: &Plant, item: &Subject) -> Result<bool> {
    let open = WorkOrder::query()
        .where_(WorkOrder::PLANT_ID.eq(plant.id))
        .where_(WorkOrder::ITEM_ID.eq(item.subject_id))
        .get(&state.db)
        .await?
        .iter()
        .any(|order| order.status.is_open());
    if open {
        return Ok(true);
    }
    let stock = stock_service(state);
    for line in Line::query()
        .where_(Line::PLANT_ID.eq(plant.id))
        .get(&state.db)
        .await?
    {
        for location in [line.store(), line.finished_goods()].into_iter().flatten() {
            if stock
                .on_hand(state, item, &location)
                .await?
                .is_some_and(|quantity| quantity > 0)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
