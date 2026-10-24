//! Items and lots in words: the name and code of an item and the number of a
//! lot, asked of the stock service (the screens never read the stock tables).

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE, stock_service};

use crate::texts::tf;

/// An item in words: `Chicken fillet (CHK-FRZ)`; `Item 6` when the stock
/// module does not know it.
pub(crate) async fn item_text(state: &AppState, lang: &Lang, item_id: i64) -> String {
    let subject = Subject::new(ITEM_SUBJECT_TYPE, item_id);
    match stock_service(state).item_label(state, &subject).await {
        Ok(Some(label)) => tf(
            lang,
            "manufacturing.screen.item_name",
            &[("name", &label.name), ("code", &label.code)],
        ),
        _ => tf(
            lang,
            "manufacturing.screen.item_unknown",
            &[("id", &item_id)],
        ),
    }
}

/// A lot in words: `Lot L-2026-001`; `Lot 10` when the stock module does not
/// know it.
pub(crate) async fn lot_text(state: &AppState, lang: &Lang, lot_id: i64) -> String {
    let subject = Subject::new(LOT_SUBJECT_TYPE, lot_id);
    let number = match stock_service(state).lot_number(state, &subject).await {
        Ok(Some(number)) => number,
        _ => lot_id.to_string(),
    };
    tf(
        lang,
        "manufacturing.screen.ncr.lot_name",
        &[("number", &number)],
    )
}
