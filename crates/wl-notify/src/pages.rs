//! The back-office pages (wl-notify §6, DESIGN-SYSTEM):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /notification-types` | `wl_notify.types` | `notifications.templates.manage` |
//! | `GET /notification-log` | `wl_notify.log` | `notifications.log.view` |
//!
//! (`/notifications` is the Renox bell's page.) The log never shows a
//! message that gave up or fell back as sent or delivered: an `abandoned`
//! message reads "Not delivered (gave up)" or "Not delivered (fell back to
//! SMS)", and the "Sent" counter counts only `sent`, `delivered` and `read`.
//! Every template is compiled in and can be replaced by the app
//! (`resources/views/wl-notify/…`).

use std::collections::{BTreeMap, HashMap};

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::notify::{GiveUpReaction, NotifyChannel};

use crate::message::{Message, MessageStatus, RecipientKind};
use crate::permissions;
use crate::texts;
use crate::types::NotificationType;

/// The texts a template reads as `tx['fields.key']`.
type Texts = BTreeMap<&'static str, String>;

/// The package text `key` (without the `wl_notify.` prefix) in a language.
type Text<'a> = &'a dyn Fn(&str) -> String;

/// How many messages the log shows, newest first.
pub const LOG_LIMIT: u64 = 200;

/// The module's routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/notification-types", types_index)
        .name("wl_notify.types")
        .require_permission(permissions::TEMPLATES_MANAGE)
        .merge(
            Routes::new()
                .get("/notification-log", log_index)
                .name("wl_notify.log")
                .require_permission(permissions::LOG_VIEW),
        )
}

/// The package texts in `lang`.
fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| texts::t(lang, &format!("wl_notify.{key}"))
}

/// A channel's label: "Email", "SMS".
fn channel_label(t: Text<'_>, channel: NotifyChannel) -> String {
    t(&format!("channel.{}", channel.as_str()))
}

/// A staff role's label: the text `wl_notify.role.<key>` when the app has
/// one, else the key in words (`collector` → "Collector").
fn role_label(t: Text<'_>, role: &str) -> String {
    let key = format!("role.{role}");
    let found = t(&key);
    if found != format!("wl_notify.{key}") {
        return found;
    }
    let words = role.replace(['_', '-', '.'], " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// One notification type as the types page shows it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TypeRow {
    /// The type's key.
    pub key: String,
    /// The sending module.
    pub module: String,
    /// "Reminder".
    pub category: String,
    /// "Normal" or "Urgent".
    pub urgency: String,
    /// "Email → SMS".
    pub channels: String,
    /// Whether the type is statutory.
    pub statutory: bool,
    /// "Yes" or "No".
    pub statutory_label: String,
    /// The reactions in run order: "Mark contact invalid, Staff task", or
    /// "None".
    pub give_up: String,
    /// "Collector", or "—".
    pub staff_role: String,
}

/// The row of `kind` with the texts of `t`.
pub(crate) fn type_row(t: Text<'_>, kind: &NotificationType) -> TypeRow {
    let reactions = kind.reactions();
    let give_up = if reactions.is_empty() {
        t(&format!("reaction.{}", GiveUpReaction::None.as_str()))
    } else {
        reactions
            .iter()
            .map(|r| t(&format!("reaction.{}", r.as_str())))
            .collect::<Vec<_>>()
            .join(", ")
    };
    TypeRow {
        key: kind.key.clone(),
        module: kind.module.clone(),
        category: t(&format!("category.{}", kind.category.as_str())),
        urgency: t(&format!("urgency.{}", kind.urgency.as_str())),
        channels: kind
            .channels
            .0
            .iter()
            .map(|c| channel_label(t, *c))
            .collect::<Vec<_>>()
            .join(" → "),
        statutory: kind.statutory,
        statutory_label: t(if kind.statutory { "yes" } else { "no" }),
        give_up,
        staff_role: kind
            .staff_role
            .as_deref()
            .filter(|role| !role.trim().is_empty())
            .map_or_else(|| t("none"), |role| role_label(t, role)),
    }
}

/// What the types page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/types.html")]
pub(crate) struct TypesPage {
    tx: Texts,
    types: Vec<TypeRow>,
}

/// `GET /notification-types`: every type with its statutory flag, give-up
/// reaction and staff role told.
async fn types_index(State(db): State<Db>, lang: Lang) -> Result<View> {
    let t = texts_in(&lang);
    let types = NotificationType::query()
        .order_by(NotificationType::MODULE)
        .order_by(NotificationType::KEY)
        .get(&db)
        .await?
        .iter()
        .map(|kind| type_row(&t, kind))
        .collect();
    Ok(View::typed(TypesPage {
        tx: texts::all(&lang),
        types,
    }))
}

/// Whether a message in `status` counts as sent: `sent`, `delivered` or
/// `read`, never `abandoned`, `failed` or `suppressed`.
pub fn counts_as_sent(status: MessageStatus) -> bool {
    matches!(
        status,
        MessageStatus::Sent | MessageStatus::Delivered | MessageStatus::Read
    )
}

/// A message's status label and badge tone. `fell_back_to` is the channel
/// of the message that took over from this one, if any: an `abandoned`
/// message reads "Not delivered (fell back to SMS)" then, else "Not
/// delivered (gave up)".
pub(crate) fn status_label(
    t: Text<'_>,
    status: MessageStatus,
    fell_back_to: Option<NotifyChannel>,
) -> (String, &'static str) {
    match status {
        MessageStatus::Abandoned => match fell_back_to {
            Some(channel) => (
                t("status.fell_back").replace(":channel", &channel_label(t, channel)),
                "warning",
            ),
            None => (t("status.gave_up"), "error"),
        },
        MessageStatus::Sent => (t("status.sent"), "info"),
        MessageStatus::Delivered | MessageStatus::Read => {
            (t(&format!("status.{}", status.as_str())), "success")
        }
        MessageStatus::Failed => (t("status.failed"), "error"),
        MessageStatus::Queued | MessageStatus::Deferred | MessageStatus::Suppressed => {
            (t(&format!("status.{}", status.as_str())), "")
        }
    }
}

/// One message as the log shows it.
#[derive(Serialize, Debug, Clone)]
pub struct LogRow {
    /// The message's id.
    pub id: i64,
    /// When it was queued.
    pub created_at: Option<DateTime>,
    /// The notification type's key.
    pub type_key: String,
    /// The record it is about, e.g. `invoice #12`.
    pub subject: String,
    /// The address, or who it is for.
    pub recipient: String,
    /// "Email", "SMS".
    pub channel: String,
    /// The status as people read it.
    pub status: String,
    /// The status badge's kind (`info`, `success`, `warning`, `error`), or
    /// empty for a plain one.
    pub tone: &'static str,
    /// How many sends were tried.
    pub attempts: i64,
    /// The last error, e.g. `550 mailbox unknown`, or empty.
    pub last_error: String,
}

/// The log row of `message` with the texts of `t`.
pub(crate) fn log_row(
    t: Text<'_>,
    message: &Message,
    fell_back_to: Option<NotifyChannel>,
) -> LogRow {
    let (status, tone) = status_label(t, message.status, fell_back_to);
    let recipient = match (&message.address, message.recipient_kind) {
        (Some(address), _) => address.clone(),
        (None, RecipientKind::Party) => format!("party #{}", message.party_id.unwrap_or_default()),
        (None, RecipientKind::User) => format!("user #{}", message.user_id.unwrap_or_default()),
        (None, RecipientKind::Address) => t("none"),
    };
    let last_error = [
        message.last_error_code.as_deref(),
        message.last_error.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    LogRow {
        id: message.id,
        created_at: message.created_at,
        type_key: message.type_key.clone(),
        subject: format!("{} #{}", message.subject_type, message.subject_id),
        recipient,
        channel: channel_label(t, message.channel),
        status,
        tone,
        attempts: message.attempts,
        last_error,
    }
}

/// The log's counters, over every message.
#[derive(Serialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogCounts {
    /// `sent`, `delivered` and `read` only.
    pub sent: u64,
    /// `queued`, `deferred` and `failed` (waiting for a retry).
    pub pending: u64,
    /// `abandoned`: gave up or fell back.
    pub not_delivered: u64,
    /// `suppressed`.
    pub suppressed: u64,
}

impl LogCounts {
    /// The counter `status` adds to.
    fn bucket(&mut self, status: MessageStatus) -> &mut u64 {
        match status {
            s if counts_as_sent(s) => &mut self.sent,
            MessageStatus::Abandoned => &mut self.not_delivered,
            MessageStatus::Suppressed => &mut self.suppressed,
            _ => &mut self.pending,
        }
    }

    /// Counts the messages of the database.
    async fn of(db: &Db) -> Result<LogCounts> {
        let mut counts = LogCounts::default();
        for status in MessageStatus::ALL.iter().copied() {
            let n = Message::query()
                .where_(Message::STATUS.eq(status))
                .count(db)
                .await?;
            *counts.bucket(status) += n;
        }
        Ok(counts)
    }
}

/// What the log page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-notify/log.html")]
pub(crate) struct LogPage {
    tx: Texts,
    counts: LogCounts,
    messages: Vec<LogRow>,
}

/// `GET /notification-log`: the latest messages, newest first, with their
/// status labels, and the counters.
async fn log_index(State(db): State<Db>, lang: Lang) -> Result<View> {
    let t = texts_in(&lang);
    let messages = Message::query()
        .order_by_desc(Message::ID)
        .limit(LOG_LIMIT)
        .get(&db)
        .await?;
    let abandoned: Vec<i64> = messages
        .iter()
        .filter(|m| m.status == MessageStatus::Abandoned)
        .map(|m| m.id)
        .collect();
    let mut fell_back: HashMap<i64, NotifyChannel> = HashMap::new();
    if !abandoned.is_empty() {
        for next in Message::query()
            .where_(Message::FALLBACK_OF.is_in(abandoned))
            .get(&db)
            .await?
        {
            if let Some(of) = next.fallback_of {
                fell_back.insert(of, next.channel);
            }
        }
    }
    let rows = messages
        .iter()
        .map(|m| log_row(&t, m, fell_back.get(&m.id).copied()))
        .collect();
    Ok(View::typed(LogPage {
        tx: texts::all(&lang),
        counts: LogCounts::of(&db).await?,
        messages: rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::db::Json;
    use wl_kernel::notify::Category;
    use wl_kernel::notify::GiveUpReaction::{
        AlternativeRecipient, MarkContactInvalid, StaffTask, StatutoryNotice,
    };

    fn english(key: &str) -> String {
        texts::english(&format!("wl_notify.{key}"))
    }

    fn reminder() -> NotificationType {
        NotificationType {
            key: "invoice.reminder".into(),
            module: "invoicing".into(),
            category: Category::Reminder,
            give_up: Json(vec![AlternativeRecipient, MarkContactInvalid, StaffTask]),
            staff_role: Some("collector".into()),
            channels: Json(vec![NotifyChannel::Email, NotifyChannel::Sms]),
            ..Default::default()
        }
    }

    #[test]
    fn a_type_shows_statutory_reactions_in_run_order_and_the_staff_role() {
        let row = type_row(&english, &reminder());
        assert_eq!(row.statutory_label, "No");
        assert_eq!(
            row.give_up,
            "Mark contact invalid, Alternative recipient, Staff task"
        );
        assert_eq!(row.staff_role, "Collector");
        assert_eq!(row.channels, "Email → SMS");
        assert_eq!(row.category, "Reminder");

        let notice = NotificationType {
            statutory: true,
            give_up: Json(vec![StatutoryNotice, MarkContactInvalid]),
            staff_role: None,
            ..reminder()
        };
        let row = type_row(&english, &notice);
        assert_eq!(row.statutory_label, "Yes");
        assert_eq!(row.give_up, "Mark contact invalid, Statutory notice");
        assert_eq!(row.staff_role, "—");

        let quiet = NotificationType {
            give_up: Json(vec![GiveUpReaction::None]),
            ..reminder()
        };
        assert_eq!(type_row(&english, &quiet).give_up, "None");
    }

    #[test]
    fn an_abandoned_message_never_reads_as_sent() {
        let (gave_up, _) = status_label(&english, MessageStatus::Abandoned, None);
        assert_eq!(gave_up, "Not delivered (gave up)");
        let (fell_back, _) =
            status_label(&english, MessageStatus::Abandoned, Some(NotifyChannel::Sms));
        assert_eq!(fell_back, "Not delivered (fell back to SMS)");
        for label in [gave_up, fell_back] {
            assert!(!label.contains("Sent") && !label.contains("Delivered"));
        }
        assert!(!counts_as_sent(MessageStatus::Abandoned));
        assert!(!counts_as_sent(MessageStatus::Failed));
        assert!(!counts_as_sent(MessageStatus::Suppressed));
        assert!(counts_as_sent(MessageStatus::Delivered));
    }

    #[test]
    fn the_counters_put_abandoned_messages_under_not_delivered() {
        let mut counts = LogCounts::default();
        for status in MessageStatus::ALL.iter().copied() {
            *counts.bucket(status) += 1;
        }
        assert_eq!(
            counts,
            LogCounts {
                sent: 3,
                pending: 3,
                not_delivered: 1,
                suppressed: 1,
            }
        );
    }

    #[test]
    fn a_log_row_shows_the_address_and_the_last_error() {
        let message = Message {
            id: 9,
            type_key: "invoice.reminder".into(),
            subject_type: "invoice".into(),
            subject_id: 12,
            channel: NotifyChannel::Email,
            address: Some("accounts@bluedunes.example".into()),
            status: MessageStatus::Abandoned,
            attempts: 1,
            last_error_code: Some("550".into()),
            last_error: Some("mailbox unknown".into()),
            ..Default::default()
        };
        let row = log_row(&english, &message, Some(NotifyChannel::Sms));
        assert_eq!(row.recipient, "accounts@bluedunes.example");
        assert_eq!(row.subject, "invoice #12");
        assert_eq!(row.channel, "Email");
        assert_eq!(row.status, "Not delivered (fell back to SMS)");
        assert_eq!(row.last_error, "550 mailbox unknown");
    }
}
