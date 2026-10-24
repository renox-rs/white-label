//! Tickets in the global search and the Related panel (BL-AGT-04,
//! `PLT.SRC.01`, `PLT.REL.01`): found by their number and their title, over
//! the record they are about.
//!
//! A ticket is in the scope of the record it is about (a unit or a
//! building: its building), read in the indexing transaction from that
//! record's kernel machine or search document.
//!
//! **Confidential tickets** (`tickets.confidential.view`): a confidential
//! ticket needs that permission on top of `tickets.view`
//! (`SearchDocument::requires`), so only its holders find it, in the
//! search, the Related panel and the agents' tools alike; it is still
//! indexed by its number only, without its title, words or requester. Its
//! requester sees it in the portal and in their own tickets, not here.
//!
//! The ticket's unit is declared by the vertical that owns units; its
//! documents come from the documents module's generic relation.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, SearchDocumentRow, Searchable, kinds, scope_in};
use wl_kernel::tickets::TICKET_SUBJECT_TYPE;

use crate::permissions;
use crate::ticket::Ticket;

/// The permission to see parties, as the parties module names it.
const PARTIES_VIEW: &str = "parties.view";

/// Declares `ticketing.ticket` for search, its party relations, and keeps
/// its document current after each transition.
pub(crate) fn register(app: &mut Registry) {
    Searchable::of::<Ticket>(TICKET_SUBJECT_TYPE)
        .after(PARTY_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .record_permission(permissions::CONFIDENTIAL_VIEW)
        .label("Tickets")
        .link("/tickets/{id}")
        .document_in(|tx, ticket| Box::pin(document_in(tx, ticket)))
        .register();
    Relation::belongs_to(
        TICKET_SUBJECT_TYPE,
        "party",
        PARTY_SUBJECT_TYPE,
        "wl_ticketing_tickets",
        "requester_party_id",
    )
    .label("Requested by")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    Relation::has_many(
        PARTY_SUBJECT_TYPE,
        "tickets",
        TICKET_SUBJECT_TYPE,
        "wl_ticketing_tickets",
        "requester_party_id",
    )
    .label("Tickets")
    .permission(permissions::VIEW)
    .register();
    app.listen(|event: Transitioned<Ticket>, state: AppState| async move {
        index(&state.db, event.key).await
    });
}

/// The search document of `ticket`.
pub async fn document_in(tx: &mut Transaction, ticket: &Ticket) -> Result<SearchDocument> {
    let about = ticket.subject();
    let about_title = stored_in(tx, &about)
        .await?
        .map(|document| document.title)
        .unwrap_or_default();
    let scope = scope_in(tx, &[about]).await?;
    let mut document = SearchDocument::new(ticket.number.clone())
        .identifier(kinds::NUMBER, ticket.number.clone())
        .status(ticket.status.as_str())
        .scope(scope);
    if ticket.confidential {
        document = document.requires(permissions::CONFIDENTIAL_VIEW);
    } else {
        let requester = match ticket.requester_party_id {
            Some(id) => stored_in(tx, &Subject::new(PARTY_SUBJECT_TYPE, id))
                .await?
                .map(|document| document.title)
                .unwrap_or_default(),
            None => String::new(),
        };
        document = document
            .secondary(ticket.title.clone())
            .keyword(about_title)
            .keyword(requester);
    }
    Ok(document)
}

/// The stored search document of `subject`, read in `tx`.
async fn stored_in(tx: &mut Transaction, subject: &Subject) -> Result<Option<SearchDocumentRow>> {
    SearchDocumentRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .first(&mut *tx)
        .await
}

/// Writes the document of ticket `id` again in its own transaction.
pub async fn index(db: &Db, id: i64) -> Result {
    let mut tx = db.begin().await?;
    wl_kernel::search::index_in(&mut tx, &Subject::new(TICKET_SUBJECT_TYPE, id)).await?;
    tx.commit().await?;
    Ok(())
}
