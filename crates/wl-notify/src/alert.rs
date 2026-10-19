//! The in-app alert for urgent give-ups (#56 A9): a Renox database
//! notification, kind [`GIVE_UP_ALERT_KIND`], sent to the holders chosen for
//! the staff task when the type's urgency is `urgent`. It links to the
//! ticket and never carries the message's subject or body.

use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::prelude::*;
use wl_kernel::notify::Urgency;
use wl_kernel::tickets::{TicketRef, ticket_path};

use crate::texts::english_with;

/// The kind of the database notification.
pub const GIVE_UP_ALERT_KIND: &str = "notify-give-up-alert";

/// "Urgent: could not reach {recipient} about {subject}", linking to the
/// ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GiveUpAlert {
    /// Who the message was for, as staff read it.
    pub recipient: String,
    /// The subject as staff read it.
    pub subject: String,
    /// The notification type key.
    pub type_key: String,
    /// The staff task the alert is about.
    pub ticket: TicketRef,
}

impl GiveUpAlert {
    /// The alert's title.
    pub fn title(&self) -> String {
        english_with(
            "wl_notify.alert.title",
            &[("recipient", &self.recipient), ("subject", &self.subject)],
        )
    }

    /// The alert's body: the type and the ticket number.
    pub fn body(&self) -> String {
        english_with(
            "wl_notify.alert.body",
            &[("type", &self.type_key), ("ticket", &self.ticket.number)],
        )
    }

    /// Where opening the alert goes.
    pub fn url(&self) -> String {
        ticket_path(self.ticket.id)
    }
}

impl Notification for GiveUpAlert {
    fn kind(&self) -> &'static str {
        GIVE_UP_ALERT_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, _state: &AppState) -> Result<renox::serde_json::Value> {
        Ok(DatabaseMessage::warning(self.title())
            .body(self.body())
            .url(self.url())
            .with("ticket_id", self.ticket.id.0)
            .with("ticket_number", &self.ticket.number)
            .with("type_key", &self.type_key)
            .into())
    }
}

/// Whether a type with `urgency` sends the alert; the field alone decides.
pub fn is_urgent(urgency: Urgency) -> bool {
    urgency == Urgency::Urgent
}

/// Sends `alert` to each of `holders`; returns how many got it (a deleted
/// user is skipped).
pub async fn send(state: &AppState, holders: &[i64], alert: &GiveUpAlert) -> Result<usize> {
    let mut sent = 0;
    for id in holders {
        if let Some(user) = User::find(&state.db, *id).await? {
            state.notify(&user, alert).await?;
            sent += 1;
        }
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert() -> GiveUpAlert {
        GiveUpAlert {
            recipient: "Sara M.".into(),
            subject: "TKT-2026-0042".into(),
            type_key: "ticket.emergency".into(),
            ticket: TicketRef {
                id: 7.into(),
                number: "TKT-2026-0007".into(),
            },
        }
    }

    #[test]
    fn the_alert_reads_urgent_and_names_recipient_and_subject() {
        assert_eq!(
            alert().title(),
            "Urgent: could not reach Sara M. about TKT-2026-0042"
        );
        assert_eq!(
            alert().body(),
            "Notification type: ticket.emergency\nTask: TKT-2026-0007"
        );
    }

    #[test]
    fn the_alert_links_to_the_ticket_and_is_in_app_only() {
        let a = alert();
        assert_eq!(a.kind(), "notify-give-up-alert");
        assert_eq!(a.url(), "/tickets/7");
    }

    #[test]
    fn only_the_urgency_field_decides() {
        assert!(is_urgent(Urgency::Urgent));
        assert!(!is_urgent(Urgency::Normal));
    }
}
