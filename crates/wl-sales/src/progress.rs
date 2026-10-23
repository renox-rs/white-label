//! The fulfilment progress of an order's lines (wl-sales §4.2 *Fulfilment
//! from deliveries*; BL-SAL-13): *not dispatched*, *picked*, *out for
//! delivery*, *partly delivered*, *delivered* or *lost*. It is a derived view,
//! worked out each time the order is shown from the dispatches made for it
//! (the kernel's `DispatchDirectory`) and the quantities fulfilled; it is not
//! a state of the order.
//!
//! - A line with goods **lost** (a dispatch in `lost`) shows *lost* first,
//!   then goods **out for delivery** (`dispatched`), then **picked**.
//! - With no dispatch in motion: nothing open and something fulfilled is
//!   *delivered*; something fulfilled and something open (a refused quantity,
//!   a part still to send) is *partly delivered*; otherwise *not dispatched*,
//!   which is also what a line whose dispatch came back (`returned`) or was
//!   `discarded` shows: its reservation request is kept and the line is
//!   delivered again or cancelled.
//! - A line cancelled in full shows *not dispatched*; an order without a
//!   dispatch directory (a kitchen or a counter) reads the quantities only.

use std::collections::BTreeMap;

use renox::prelude::*;
use wl_kernel::inventory::dispatch_directory;

use crate::deliveries::SALE_LINE_SUBJECT_TYPE;
use crate::sale_line::{KIND_ITEM, SaleLine};
use crate::sales_order::SalesOrder;

/// The keys of the progress words, `wl_sales.progress.<key>`.
pub const PROGRESS_KEYS: [&str; 6] = [
    "not_dispatched",
    "picked",
    "out_for_delivery",
    "partly_delivered",
    "delivered",
    "lost",
];

/// The progress key of each item line of `order`, by sale line id.
///
/// # Errors
///
/// A database error.
pub async fn of_lines(
    state: &AppState,
    order: &SalesOrder,
    lines: &[SaleLine],
) -> Result<BTreeMap<i64, &'static str>> {
    let dispatches = dispatch_directory(state)
        .of_source(state, &order.subject())
        .await?;
    let mut progress = BTreeMap::new();
    for line in lines.iter().filter(|line| line.kind == KIND_ITEM) {
        let in_motion = |status: &str| {
            dispatches.iter().any(|dispatch| {
                dispatch.status == status
                    && dispatch.lines.iter().any(|dispatched| {
                        dispatched.source_line.as_ref().is_some_and(|source| {
                            source.subject_type == SALE_LINE_SUBJECT_TYPE
                                && source.subject_id == line.id
                        })
                    })
            })
        };
        let key = if in_motion("lost") {
            "lost"
        } else if in_motion("dispatched") {
            "out_for_delivery"
        } else if in_motion("picked") {
            "picked"
        } else if line.fulfilled_milli > 0 && line.open_milli() == 0 {
            "delivered"
        } else if line.fulfilled_milli > 0 {
            "partly_delivered"
        } else {
            "not_dispatched"
        };
        progress.insert(line.id, key);
    }
    Ok(progress)
}
