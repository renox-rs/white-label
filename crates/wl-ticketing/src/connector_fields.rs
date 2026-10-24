//! A ticket as an AI assistant reads it with the connector's `fetch` tool
//! (BL-AGT-07): its number, status, priority and channel, and its title and
//! description, which are marked as untrusted text when an outsider wrote
//! them (a ticket from the portal or an e-mail, or asked for by a party),
//! so an instruction in a tenant's complaint reads as data.
//!
//! The connector calls it only for a ticket the user may open in the search
//! (a confidential one needs `tickets.confidential.view`).

use renox::prelude::*;
use wl_kernel::connector::RecordFields;
use wl_kernel::tickets::{TICKET_SUBJECT_TYPE, TicketChannel};

use crate::ticket::Ticket;

/// Whether `ticket`'s words come from outside the company.
pub fn written_outside(ticket: &Ticket) -> bool {
    ticket.requester_party_id.is_some()
        || matches!(ticket.channel, TicketChannel::Portal | TicketChannel::Email)
}

/// The fields of `ticket` for an assistant.
pub fn fields(ticket: &Ticket) -> RecordFields {
    let outside = written_outside(ticket);
    RecordFields::new()
        .text("Number", ticket.number.clone())
        .text("Status", ticket.status.as_str())
        .text("Priority", ticket.priority.to_string())
        .text("Came in by", ticket.channel.to_string())
        .text_from("Title", &ticket.title, outside)
        .text_from("Description", &ticket.description, outside)
}

/// Registers the fields of `ticketing.ticket` for the connector.
pub(crate) fn register() {
    wl_kernel::connector::register_fields(TICKET_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(Ticket::query()
                .where_eq("id", id)
                .first(db)
                .await?
                .map(|ticket| fields(&ticket)))
        })
    });
}
