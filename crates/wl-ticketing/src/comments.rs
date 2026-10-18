//! Ticket comments (wl-ticketing §3; #56 A6): notes on a ticket, by a user
//! or by the system. [`Tickets::comment`](crate::Tickets::comment) adds one
//! for other modules through the kernel's `TaskCreator`.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// Who sees a comment. Stored as `internal` or `requester`.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CommentVisibility {
    /// Staff only.
    #[default]
    Internal,
    /// Staff and the requester.
    Requester,
}

/// A comment on a ticket.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_ticketing_comments", index(ticket_id))]
pub struct Comment {
    /// The primary key.
    pub id: i64,
    /// The ticket.
    #[model(references = "wl_ticketing_tickets")]
    pub ticket_id: i64,
    /// The user who wrote it; `None` when the system did.
    pub author_user_id: Option<i64>,
    /// Who sees it.
    pub visibility: CommentVisibility,
    /// The text.
    pub text: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Comment {
    /// The comments of `ticket_id`, oldest first.
    pub async fn of_ticket(db: &Db, ticket_id: i64) -> Result<Vec<Comment>> {
        Comment::query()
            .where_(Comment::TICKET_ID.eq(ticket_id))
            .order_by(Comment::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_is_stored_in_snake_case() {
        let keys: Vec<_> = CommentVisibility::ALL.iter().map(|v| v.as_str()).collect();
        assert_eq!(keys, ["internal", "requester"]);
        assert_eq!(CommentVisibility::default(), CommentVisibility::Internal);
    }
}
