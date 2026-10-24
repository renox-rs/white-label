//! The claim ticket of a shipment (03 Logistics §4.4 step 3, §6 *Claim*): a
//! ticket of the category *claim* about the shipment, linked on it. Opened for
//! a reversal or a disposal where the company is liable (BL-LOG-10) and for
//! every lost shipment (BL-LOG-11). One ticket per cause: the source key keeps
//! a second call from opening a second ticket.

use renox::prelude::*;
use wl_kernel::tickets::{self, OpenTicket, TicketRef};

use crate::hold::CLAIM_CATEGORY;
use crate::shipment::{Shipment, track};

/// Links ticket `ticket` (with its `number`, when known) as the claim ticket
/// of shipment `id`.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn link(state: &AppState, id: i64, ticket: i64, number: Option<&str>) -> Result {
    let ticket_id = Some(ticket);
    let number = number.map(str::to_owned);
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(
            &state.db,
            &[
                ("claim_ticket_id", &ticket_id),
                ("claim_ticket_number", &number),
            ],
        )
        .await?;
    // The claim's number finds the shipment (BL-AGT-12).
    crate::search::index(
        &state.db,
        &wl_kernel::Subject::new(crate::shipment::SUBJECT_TYPE, id),
    )
    .await?;
    Ok(())
}

/// Opens a claim ticket for `shipment` and links it. A ticket already open
/// under `source_key` is the one linked (nothing is opened twice). A refused
/// ticket, or ticketing being off, is written to the tracking events and the
/// shipment stays as it is: `None`.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn open(
    state: &AppState,
    shipment: &Shipment,
    source_key: &str,
    title: &str,
    description: &str,
) -> Result<Option<TicketRef>> {
    let request = OpenTicket::new(shipment.subject(), CLAIM_CATEGORY, title)
        .description(description)
        .requester_party(shipment.client())
        .source_key(source_key);
    match tickets::task_creator(state).open(state, request).await {
        Ok(Some(ticket)) => {
            link(state, shipment.id, ticket.id.0, Some(&ticket.number)).await?;
            track(&state.db, shipment.id, "claim_ticket", &ticket.number, None).await?;
            Ok(Some(ticket))
        }
        Ok(None) => {
            track(
                &state.db,
                shipment.id,
                "claim_ticket_skipped",
                "ticketing is off",
                None,
            )
            .await?;
            Ok(None)
        }
        Err(error) => {
            let note = format!("{error:?}");
            track(&state.db, shipment.id, "claim_ticket_failed", &note, None).await?;
            Ok(None)
        }
    }
}
