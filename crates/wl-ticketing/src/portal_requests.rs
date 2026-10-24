//! Requests raised from the portal (BL-TKT-14, `WRK.TKT.01`, `WRK.ACL.01`,
//! `POR.SLF.01`): the ticket service a portal section calls, without this
//! crate depending on the portal.
//!
//! - [`raise`]: a ticket with channel *portal* and the portal user as
//!   requester, billed to the party the user acts for; the category (one
//!   marked *portal*, never a platform one) sets priority, SLA and assignee
//!   as for a staff ticket. Photos (images up to 20 MB) are stored through
//!   the kernel's `DocumentStore` on the ticket, kind [`PHOTO_KIND`].
//! - [`requests_for`]: the tickets the party's users raised and the tickets
//!   on the subjects the party holds, newest first: never another tenant's.
//! - [`visible_comments`]: only the comments marked visible to the
//!   requester; internal ones never show. The assignee is not shown at all
//!   (no phone).
//! - [`charge_text`]: the amount with VAT of a chargeable ticket, shown
//!   before the requester confirms.
//!
//! Whether the user may raise a request (an active grant; a `read_only` one
//! may not; a blocked party may) is the portal's decision, made by the
//! caller.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::document_store::document_store;
use wl_kernel::tickets::{OpenTicket, TICKET_SUBJECT_TYPE, TicketChannel, TicketRef};
use wl_kernel::PartyId;

use crate::charge;
use crate::comments::{Comment, CommentVisibility};
use crate::config::Category;
use crate::open::Tickets;
use crate::ticket::{Ticket, TicketStatus};

/// The document kind of a request's photos.
pub const PHOTO_KIND: &str = "ticket_photo";

/// The largest photo the portal takes: 20 MB.
pub const MAX_PHOTO_BYTES: usize = 20 * 1024 * 1024;

/// What the portal's *New request* form gives.
#[derive(Debug, Clone)]
pub struct PortalRequest {
    /// The record it is about: one of the party's units.
    pub subject: Subject,
    /// The category's key.
    pub category: String,
    /// The description, in the requester's words.
    pub description: String,
    /// The photos.
    pub photos: Vec<Upload>,
}

/// The categories open to the portal, by name: active, marked *portal*, not
/// a platform category.
///
/// # Errors
///
/// The database's errors.
pub async fn portal_categories(db: &Db) -> Result<Vec<Category>> {
    Category::query()
        .where_(Category::ACTIVE.eq(true))
        .where_(Category::PORTAL.eq(true))
        .where_(Category::PLATFORM.eq(false))
        .order_by(Category::NAME_EN)
        .get(db)
        .await
}

fn unprocessable(message: &str) -> Error {
    abort(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

/// Whether `file` is an image the portal takes.
fn is_image(file: &Upload) -> bool {
    let declared = file.content_type().trim().to_ascii_lowercase();
    let sniffed = file.sniffed_type().unwrap_or_default().to_ascii_lowercase();
    sniffed.starts_with("image/") || (sniffed.is_empty() && declared.starts_with("image/"))
}

/// Portal user `user_id`, acting for `party`, raises `request`.
///
/// # Errors
///
/// 422 for a category not open to the portal, a blank description, a photo
/// that is not an image or larger than 20 MB (nothing is stored then), or a
/// photo the document kind refuses.
pub async fn raise(
    state: &AppState,
    user_id: i64,
    party: PartyId,
    request: PortalRequest,
) -> Result<TicketRef> {
    let category = portal_categories(&state.db)
        .await?
        .into_iter()
        .find(|c| c.key == request.category.trim())
        .ok_or_else(|| unprocessable("Choose one of the categories offered."))?;
    let description = request.description.trim();
    if description.is_empty() {
        return Err(unprocessable("Describe the problem."));
    }
    for photo in &request.photos {
        if !is_image(photo) {
            return Err(unprocessable(&format!(
                "{} is not a photo.",
                photo.file_name()
            )));
        }
        if photo.size() > MAX_PHOTO_BYTES {
            return Err(unprocessable(&format!(
                "{} is larger than 20 MB.",
                photo.file_name()
            )));
        }
    }
    let title = category.name("en").to_owned();
    let open = OpenTicket::new(request.subject.clone(), &category.key, &title)
        .description(description)
        .channel(TicketChannel::Portal)
        .requester_user(user_id)
        .bill_to(party);
    let ticket = Tickets::open(state, open).await?;
    let store = document_store(state);
    for photo in request.photos {
        store
            .attach(
                state,
                Subject::new(TICKET_SUBJECT_TYPE, ticket.id.0),
                PHOTO_KIND,
                photo,
                "portal",
            )
            .await?;
    }
    Ok(ticket)
}

/// The requests of a party (`WRK.ACL.01`): the tickets raised by `users`
/// (the party's portal users) or by the party, and the tickets on
/// `subjects` (what the party holds), newest first.
///
/// # Errors
///
/// The database's errors.
pub async fn requests_for(
    db: &Db,
    party: PartyId,
    users: &[i64],
    subjects: &[Subject],
) -> Result<Vec<Ticket>> {
    let mut found: Vec<Ticket> = Ticket::query()
        .where_any(|q| {
            q.where_(Ticket::REQUESTER_PARTY_ID.eq(party.0))
                .where_(Ticket::REQUESTER_USER_ID.is_in(users.to_vec()))
        })
        .get(db)
        .await?;
    for subject in subjects {
        let on = Ticket::query()
            .where_(Ticket::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(Ticket::SUBJECT_ID.eq(subject.subject_id))
            .get(db)
            .await?;
        for ticket in on {
            if !found.iter().any(|t| t.id == ticket.id) {
                found.push(ticket);
            }
        }
    }
    // A confidential ticket (a staff task) never shows in the portal.
    found.retain(|t| !t.confidential);
    found.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(found)
}

/// The status a requester reads.
pub fn status_text(status: TicketStatus) -> &'static str {
    match status {
        TicketStatus::Open => "Open",
        TicketStatus::Assigned => "Assigned",
        TicketStatus::InProgress => "In progress",
        TicketStatus::OnHold => "On hold",
        TicketStatus::Resolved => "Resolved",
        TicketStatus::Closed => "Closed",
        TicketStatus::Cancelled => "Cancelled",
    }
}

/// The comments of `ticket` the requester sees, oldest first: only those
/// marked visible.
///
/// # Errors
///
/// The database's errors.
pub async fn visible_comments(db: &Db, ticket: &Ticket) -> Result<Vec<Comment>> {
    Ok(Comment::of_ticket(db, ticket.id)
        .await?
        .into_iter()
        .filter(|c| c.visibility == CommentVisibility::Requester)
        .collect())
}

/// The amount with VAT of a chargeable `ticket` (`AED 262.50`), or `None`.
///
/// # Errors
///
/// The database's errors.
pub async fn charge_text(db: &Db, ticket: &Ticket) -> Result<Option<String>> {
    if !ticket.chargeable {
        return Ok(None);
    }
    let lines = charge::lines_of(db, ticket.id).await?;
    if lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(charge::money(charge::total(&lines))))
}
