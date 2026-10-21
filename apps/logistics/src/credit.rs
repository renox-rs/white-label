//! The credit due on an invoiced shipment (03 Logistics §4.4 step 4, §7.1
//! rows `reverse_delivery` and `mark_lost`; BL-LOG-10, BL-LOG-11).
//!
//! A shipment that is reversed or lost after its charge went on an issued
//! monthly invoice leaves billing: the invoice stays as it is and the invoice
//! owner (the holders of the invoice owner role) gets a task to credit the
//! charge of this shipment, with the amount. The shipment keeps the amount
//! ([`Shipment::credit_due`]) and is no longer marked invoiced, so the month
//! it is delivered again bills it again (BL-LOG-20). A shipment that was not
//! invoiced needs nothing: it is simply left out of the next draft
//! ([`Shipment::billable`]). The credit note itself is wl-invoicing's.

use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::role_tasks::{self, RoleTask};

use crate::permissions;
use crate::rates::aed;
use crate::shipment::{Shipment, track};
use crate::texts::english;

/// Marks the credit due on `shipment` and opens the task of the invoice
/// owner, when the shipment was invoiced. `why` is the text key of the cause
/// (`reversed` or `lost`) and `cause` a number that makes the task unique
/// (the reversal's id, or the shipment's for a loss). Returns the amount, 0
/// when it was not invoiced.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn due(
    state: &AppState,
    shipment: &Shipment,
    why: &str,
    cause: i64,
) -> Result<i64> {
    if shipment.invoiced_on.is_none() {
        return Ok(0);
    }
    let amount = shipment.charged_price();
    let not_invoiced: Option<DateTime> = None;
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(
            &state.db,
            &[("credit_due", &amount), ("invoiced_on", &not_invoiced)],
        )
        .await?;
    track(
        &state.db,
        shipment.id,
        "credit_due",
        &aed(amount),
        None,
    )
    .await?;
    let title = english(
        "logistics.credit.title",
        &[("number", &shipment.number), ("amount", &aed(amount))],
    );
    let description = english(
        &format!("logistics.credit.description.{why}"),
        &[("number", &shipment.number), ("amount", &aed(amount))],
    );
    let task = RoleTask::new(
        shipment.subject(),
        permissions::INVOICE_OWNER_ROLE,
        &title,
        &format!("logistics.credit:{}:{cause}", shipment.id),
    )
    .description(&description);
    role_tasks::open(state, task).await?;
    Ok(amount)
}
