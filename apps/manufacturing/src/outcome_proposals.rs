//! The outcome proposed for the started goods of a cancelled sales line
//! (BL-MFG-14, 07 §7.1): an item flagged *customer specific* (printed labels
//! and packaging made for one buyer) is proposed *scrap*; any other item
//! *finish to stock*. wl-sales preselects it in the cancel sheets and the
//! person may choose another.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{LOCATION_SUBJECT_TYPE, stock_service};
use wl_kernel::sales::{LineOutcomeKind, OutcomeProposals, SalesFuture, SalesOrderId};

use crate::conformity::ItemFlag;

/// The proposals of the plant: scrap what is customer specific, else finish
/// to stock.
pub struct ItemOutcomeProposals;

impl OutcomeProposals for ItemOutcomeProposals {
    fn proposed<'a>(
        &'a self,
        state: &'a AppState,
        _order: SalesOrderId,
        _line: i64,
        item_code: &'a str,
    ) -> SalesFuture<'a, Option<LineOutcomeKind>> {
        Box::pin(async move {
            let Some(found) = stock_service(state)
                .choices(state, item_code, &Subject::new(LOCATION_SUBJECT_TYPE, 0))
                .await?
            else {
                return Ok(None);
            };
            let specific = ItemFlag::of(&state.db, found.item.subject_id)
                .await?
                .is_some_and(|flag| flag.customer_specific);
            Ok(Some(if specific {
                LineOutcomeKind::Scrap
            } else {
                LineOutcomeKind::FinishToStock
            }))
        })
    }
}
