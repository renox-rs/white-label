//! Who sees a ticket of a confidential category (`WRK.ACL.03`, wl-ticketing
//! §10): holders of `tickets.confidential.view` and the requester. Everyone
//! else gets a 404 for the ticket and does not find it in the list. Each
//! view of a confidential ticket is audited (`ticket.confidential_viewed`).

use renox::db::Query;
use renox::prelude::*;

use crate::permissions;
use crate::ticket::Ticket;

/// The audit action recorded when a confidential ticket is viewed.
pub const VIEWED: &str = "ticket.confidential_viewed";

/// Whether `user` may see `ticket`: any ticket of an ordinary category, a
/// confidential one only for a holder of `tickets.confidential.view` or its
/// requester.
pub fn can_see(ticket: &Ticket, user: &AuthUser) -> bool {
    !ticket.confidential
        || user.has_permission(permissions::CONFIDENTIAL_VIEW)
        || ticket.requester_user_id == Some(user.id)
}

/// `NotFound` unless `user` may see `ticket`: a confidential ticket does not
/// reveal that it exists.
///
/// # Errors
///
/// `NotFound`.
pub fn ensure_visible(ticket: &Ticket, user: &AuthUser) -> Result {
    if can_see(ticket, user) {
        Ok(())
    } else {
        Err(Error::NotFound)
    }
}

/// `query` limited to the tickets `user` may see.
pub fn visible_to(query: Query<Ticket>, user: &AuthUser) -> Query<Ticket> {
    if user.has_permission(permissions::CONFIDENTIAL_VIEW) {
        return query;
    }
    let id = user.id;
    query.where_any(|q| {
        q.where_(Ticket::CONFIDENTIAL.eq(false))
            .where_(Ticket::REQUESTER_USER_ID.eq(id))
    })
}

/// Records that `user` viewed `ticket`, when it is confidential.
///
/// # Errors
///
/// The database's errors.
pub async fn audit_view(db: &Db, user: &AuthUser, ticket: &Ticket) -> Result {
    if !ticket.confidential {
        return Ok(());
    }
    renox::audit::record(
        db,
        renox::audit::Entry::new(VIEWED)
            .user(user.id)
            .subject(Ticket::TABLE, ticket.id),
    )
    .await
}
