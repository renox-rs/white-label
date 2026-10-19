//! The "not delivered" query (wl-notify §5.4, #60 A7): the messages about a
//! record that ended `abandoned` through `give_up`, so a module can show a
//! note next to it (the booking page: "reminder not delivered").

use renox::prelude::*;
use wl_kernel::Subject;

use crate::message::{Message, MessageStatus};

/// A message that gave up, as the module showing the record sees it.
#[derive(Debug, Clone)]
pub struct UndeliveredMessage {
    /// The message that gave up (the last one of its chain).
    pub message: Message,
    /// The notification type's key.
    pub type_key: String,
    /// When it gave up.
    pub gave_up_at: Option<DateTime>,
}

/// The messages about `subject` that ended `abandoned` through `give_up`,
/// newest first. An `abandoned` message that fell back (another message
/// took over from it) is not listed: that chain was still tried.
pub async fn undelivered_for(db: &Db, subject: &Subject) -> Result<Vec<UndeliveredMessage>> {
    let abandoned = Message::query()
        .where_(Message::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(Message::SUBJECT_ID.eq(subject.subject_id))
        .where_(Message::STATUS.eq(MessageStatus::Abandoned))
        .order_by_desc(Message::ID)
        .get(db)
        .await?;
    if abandoned.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = abandoned.iter().map(|m| m.id).collect();
    let fell_back: Vec<i64> = Message::query()
        .where_(Message::FALLBACK_OF.is_in(ids))
        .get(db)
        .await?
        .into_iter()
        .filter_map(|next| next.fallback_of)
        .collect();
    Ok(abandoned
        .into_iter()
        .filter(|m| !fell_back.contains(&m.id))
        .map(|message| UndeliveredMessage {
            type_key: message.type_key.clone(),
            gave_up_at: message.updated_at.or(message.created_at),
            message,
        })
        .collect())
}
