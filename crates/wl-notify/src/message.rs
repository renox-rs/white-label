//! Messages (wl-notify §3 *Message*): one message to one recipient on one
//! channel. Its status moves only through the machine in [`crate::machine`].

use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};
use wl_kernel::notify::{
    ContactPurpose, ContactRelation, MessageId, NotifyChannel, NotifyRecipient,
};
use wl_kernel::{PartyId, Subject};

/// Where a message is in its lifecycle (wl-notify §4.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MessageStatus {
    /// Rendered and waiting to be sent.
    #[default]
    Queued,
    /// Held until quiet hours end or a scheduled time.
    Deferred,
    /// Accepted by the provider.
    Sent,
    /// Delivered to the device or mailbox.
    Delivered,
    /// Read receipt or open tracked.
    Read,
    /// The provider rejected it or delivery failed; may be retried or fall back.
    Failed,
    /// Not sent: no consent, opted out, suppressed address, or no contact point.
    Suppressed,
    /// Failed after all retries and fallbacks.
    Abandoned,
}

/// Who a message is for, as stored.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecipientKind {
    /// A staff (or portal) user.
    User,
    /// A party, through its contact points.
    #[default]
    Party,
    /// A bare address.
    Address,
    /// A recipient rule: the contacts of a party (#57).
    Contact,
}

/// Why a message was `suppressed` because nothing can reach the recipient.
pub const NO_CONTACT_POINT: &str = "no_contact_point";
/// Why a message was `suppressed` because its contact point became invalid
/// after it was queued (#58 A2).
pub const INVALID_CONTACT_POINT: &str = "invalid_contact_point";
/// Why a message was `suppressed` because its recipient rule found nobody.
pub const NO_CONTACT: &str = "no_contact";
/// Why a message was `suppressed` because its address is on the list.
pub const SUPPRESSED_ADDRESS: &str = "suppressed";

/// Why a marketing message was `suppressed` because its party already got the
/// most marketing messages allowed in 7 days (`NTF.RUL.02`, #62 A7).
pub const FREQUENCY_CAP: &str = "frequency_cap";

/// The quiet hours, in minutes after midnight Asia/Dubai: a normal urgency
/// message to an external recipient waits from 21:00 until 08:00 (#62 A4).
pub const QUIET_HOURS: (u32, u32) = (21 * 60, 8 * 60);

/// Why a message was `suppressed` because it announces a pause and the party
/// is on hold (#36 A11).
pub const PARTY_ON_HOLD: &str = "party_on_hold";

/// One message to one recipient on one channel.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_notify_messages",
    state_machine,
    index(status),
    index(chain_id),
    index(subject_type, subject_id),
    unique(broadcast_id, party_id)
)]
pub struct Message {
    /// The primary key.
    pub id: i64,
    /// The notification type's key.
    pub type_key: String,
    /// The type of the record it is about.
    pub subject_type: String,
    /// The id of the record it is about.
    pub subject_id: i64,
    /// The request's short label for the subject (e.g. `TKT-2026-0042`).
    pub subject_label: Option<String>,
    /// The first message of its fallback chain (its own id for the first).
    pub chain_id: Option<i64>,
    /// The message this one falls back from.
    pub fallback_of: Option<i64>,
    /// User, party or address.
    pub recipient_kind: RecipientKind,
    /// The party, for a party recipient; the chosen person for a rule.
    pub party_id: Option<i64>,
    /// The party whose contacts are meant, for a recipient rule.
    pub contact_of_party_id: Option<i64>,
    /// The rule's relation (`contact_of` or `guardian_of`).
    pub contact_relation: Option<String>,
    /// The rule's position, e.g. `finance`; none for any.
    pub contact_position: Option<String>,
    /// The first message of the chain this chain is the alternative
    /// recipient of (every message of the alternative's chain carries it).
    pub alternative_of: Option<i64>,
    /// The user, for a user recipient.
    pub user_id: Option<i64>,
    /// The party contact point the address came from.
    pub contact_point_id: Option<i64>,
    /// The channel.
    pub channel: NotifyChannel,
    /// The e-mail address or number; `None` when there was none.
    pub address: Option<String>,
    /// The language it is written in, `en` or `ar`.
    pub language: String,
    /// The rendered subject (e-mail) or title.
    pub rendered_subject: Option<String>,
    /// The rendered body.
    pub rendered_body: String,
    /// The request's template variables.
    pub data: Json<Value>,
    /// The provider that sent it.
    pub provider: Option<String>,
    /// The provider's id for it.
    pub provider_message_id: Option<String>,
    /// The broadcast it belongs to: a party gets one message per broadcast
    /// (#61 A3).
    pub broadcast_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: MessageStatus,
    /// How many sends were tried.
    pub attempts: i64,
    /// The provider's last error code, e.g. `550`.
    pub last_error_code: Option<String>,
    /// The last error text.
    pub last_error: Option<String>,
    /// The kind of the last error (`transient`, `permanent`, a hard kind).
    pub error_kind: Option<String>,
    /// Why it was suppressed, e.g. `no_contact_point`.
    pub suppress_reason: Option<String>,
    /// When it is to be sent, if held.
    pub scheduled_for: Option<DateTime>,
    /// When the provider accepted it.
    pub sent_at: Option<DateTime>,
    /// When it was delivered.
    pub delivered_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Message {
    /// The message's id in the kernel contract.
    pub fn message_id(&self) -> MessageId {
        MessageId(self.id)
    }

    /// The record it is about.
    pub fn subject(&self) -> Subject {
        Subject {
            subject_type: self.subject_type.clone(),
            subject_id: self.subject_id,
        }
    }

    /// The id of its fallback chain: the first message's.
    pub fn chain(&self) -> i64 {
        self.chain_id.unwrap_or(self.id)
    }

    /// Who it is for, in the kernel's shape.
    pub fn recipient(&self) -> NotifyRecipient {
        match self.recipient_kind {
            RecipientKind::User => NotifyRecipient::User {
                id: self.user_id.unwrap_or_default(),
            },
            RecipientKind::Party => NotifyRecipient::Party {
                id: PartyId(self.party_id.unwrap_or_default()),
            },
            RecipientKind::Contact => NotifyRecipient::Contact {
                of: PartyId(self.contact_of_party_id.unwrap_or_default()),
                purpose: ContactPurpose {
                    relation: match self.contact_relation.as_deref() {
                        Some("guardian_of") => ContactRelation::GuardianOf,
                        _ => ContactRelation::ContactOf,
                    },
                    position: self.contact_position.clone(),
                },
            },
            RecipientKind::Address => NotifyRecipient::Address {
                channel: self.channel,
                address: self.address.clone().unwrap_or_default(),
            },
        }
    }

    /// Who the message really goes to: the chosen person for a rule, else
    /// [`Message::recipient`]. A fallback stays with that person.
    pub fn person(&self) -> NotifyRecipient {
        match (self.recipient_kind, self.party_id) {
            (RecipientKind::Contact, Some(id)) => NotifyRecipient::Party { id: PartyId(id) },
            _ => self.recipient(),
        }
    }

    /// Sets the subject columns for `subject`.
    pub fn set_subject(&mut self, subject: &Subject) {
        self.subject_type = subject.subject_type.clone();
        self.subject_id = subject.subject_id;
    }

    /// Sets the recipient columns for `recipient`.
    pub fn set_recipient(&mut self, recipient: &NotifyRecipient) {
        match recipient {
            NotifyRecipient::User { id } => {
                self.recipient_kind = RecipientKind::User;
                self.user_id = Some(*id);
            }
            NotifyRecipient::Party { id } => {
                self.recipient_kind = RecipientKind::Party;
                self.party_id = Some(id.0);
            }
            NotifyRecipient::Contact { of, purpose } => {
                self.recipient_kind = RecipientKind::Contact;
                self.contact_of_party_id = Some(of.0);
                self.contact_relation = Some(purpose.relation.as_str().to_owned());
                self.contact_position.clone_from(&purpose.position);
            }
            NotifyRecipient::Address { channel, address } => {
                self.recipient_kind = RecipientKind::Address;
                self.channel = *channel;
                self.address = Some(address.clone());
            }
        }
    }

    /// The messages of fallback chain `chain`, oldest first.
    pub async fn of_chain(db: &Db, chain: i64) -> Result<Vec<Message>> {
        Message::query()
            .where_(Message::CHAIN_ID.eq(chain))
            .order_by(Message::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let names: Vec<_> = MessageStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "queued",
                "deferred",
                "sent",
                "delivered",
                "read",
                "failed",
                "suppressed",
                "abandoned"
            ]
        );
    }

    #[test]
    fn the_recipient_round_trips_through_the_columns() {
        let recipients = [
            NotifyRecipient::User { id: 7 },
            NotifyRecipient::Party { id: PartyId(3) },
            NotifyRecipient::Contact {
                of: PartyId(5),
                purpose: ContactPurpose::contact_of("finance"),
            },
            NotifyRecipient::Contact {
                of: PartyId(6),
                purpose: ContactPurpose::guardian_of(),
            },
            NotifyRecipient::Address {
                channel: NotifyChannel::Sms,
                address: "+971501234567".into(),
            },
        ];
        for recipient in recipients {
            let mut message = Message::default();
            message.set_recipient(&recipient);
            assert_eq!(message.recipient(), recipient);
        }
    }

    #[test]
    fn a_rule_message_goes_to_the_chosen_person() {
        let mut message = Message::default();
        message.set_recipient(&NotifyRecipient::Contact {
            of: PartyId(5),
            purpose: ContactPurpose::contact_of("finance"),
        });
        assert_eq!(message.person(), message.recipient());
        message.party_id = Some(8);
        assert_eq!(message.person(), NotifyRecipient::Party { id: PartyId(8) });
        assert_eq!(message.contact_relation.as_deref(), Some("contact_of"));
        assert_eq!(message.contact_position.as_deref(), Some("finance"));
    }

    #[test]
    fn the_first_message_is_its_own_chain() {
        let first = Message {
            id: 4,
            ..Default::default()
        };
        assert_eq!(first.chain(), 4);
        let fallback = Message {
            id: 5,
            chain_id: Some(4),
            fallback_of: Some(4),
            ..Default::default()
        };
        assert_eq!(fallback.chain(), 4);
        assert_eq!(fallback.message_id(), MessageId(5));
    }
}
