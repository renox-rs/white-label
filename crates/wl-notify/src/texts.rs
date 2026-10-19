//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_notify.title", "Notifications"),
    ("wl_notify.types.title", "Notification types"),
    (
        "wl_notify.types.subtitle",
        "What each module sends, and what happens when a message cannot be delivered.",
    ),
    ("wl_notify.types.empty", "No notification types yet"),
    (
        "wl_notify.types.empty_hint",
        "Types show up here once the modules that send them are installed.",
    ),
    ("wl_notify.types.saved", "Give-up reactions saved"),
    ("wl_notify.types.edit", "Edit"),
    ("wl_notify.types.edit_title", "Give-up reactions"),
    (
        "wl_notify.types.edit_subtitle",
        "What happens when a message of this type cannot be delivered. Reactions run in the order shown.",
    ),
    ("wl_notify.types.reactions", "Reactions"),
    (
        "wl_notify.types.reactions_hint",
        "Mark contact invalid runs first; combine it with Staff task so staff hear about it.",
    ),
    (
        "wl_notify.types.staff_role_hint",
        "The role that gets the staff task, e.g. collector. Leave blank for none.",
    ),
    ("wl_notify.types.save", "Save"),
    ("wl_notify.types.cancel", "Cancel"),
    (
        "wl_notify.proof.method_not_offered",
        "This way of delivering is not accepted for this notice.",
    ),
    (
        "wl_notify.proof.date_missing",
        "Enter the date the notice was delivered.",
    ),
    (
        "wl_notify.proof.date_in_future",
        "The delivery date cannot be in the future.",
    ),
    (
        "wl_notify.proof.date_before_give_up",
        "The notice was not delivered before it gave up.",
    ),
    (
        "wl_notify.proof.attachment_missing",
        "Attach the proof of delivery.",
    ),
    (
        "wl_notify.proof.note_missing",
        "Describe the call: who was told, and when.",
    ),
    (
        "wl_notify.proof.attachment_type",
        "The proof must be a PDF, an image or a text file.",
    ),
    ("wl_notify.log.title", "Message log"),
    (
        "wl_notify.log.subtitle",
        "The latest messages, newest first, and what became of each.",
    ),
    ("wl_notify.log.empty", "No messages yet"),
    (
        "wl_notify.log.empty_hint",
        "Messages show up here once a module sends one.",
    ),
    ("wl_notify.counters.sent", "Sent"),
    ("wl_notify.counters.pending", "In progress"),
    ("wl_notify.counters.not_delivered", "Not delivered"),
    ("wl_notify.counters.suppressed", "Not sent"),
    ("wl_notify.fields.key", "Type"),
    ("wl_notify.fields.module", "Module"),
    ("wl_notify.fields.category", "Category"),
    ("wl_notify.fields.urgency", "Urgency"),
    ("wl_notify.fields.channels", "Channels"),
    ("wl_notify.fields.statutory", "Statutory"),
    ("wl_notify.fields.give_up", "Give-up reaction"),
    ("wl_notify.fields.staff_role", "Staff role told"),
    ("wl_notify.fields.created_at", "Created"),
    ("wl_notify.fields.subject", "About"),
    ("wl_notify.fields.recipient", "Recipient"),
    ("wl_notify.fields.channel", "Channel"),
    ("wl_notify.fields.status", "Status"),
    ("wl_notify.fields.attempts", "Attempts"),
    ("wl_notify.fields.last_error", "Last error"),
    ("wl_notify.yes", "Yes"),
    ("wl_notify.no", "No"),
    ("wl_notify.none", "—"),
    ("wl_notify.category.transactional", "Transactional"),
    ("wl_notify.category.reminder", "Reminder"),
    ("wl_notify.category.security", "Security"),
    ("wl_notify.category.marketing", "Marketing"),
    ("wl_notify.urgency.normal", "Normal"),
    ("wl_notify.urgency.urgent", "Urgent"),
    ("wl_notify.channel.email", "Email"),
    ("wl_notify.channel.sms", "SMS"),
    ("wl_notify.channel.whatsapp", "WhatsApp"),
    ("wl_notify.reaction.staff_task", "Staff task"),
    (
        "wl_notify.reaction.alternative_recipient",
        "Alternative recipient",
    ),
    (
        "wl_notify.reaction.mark_contact_invalid",
        "Mark contact invalid",
    ),
    ("wl_notify.reaction.statutory_notice", "Statutory notice"),
    ("wl_notify.reaction.none", "None"),
    (
        "wl_notify.task.title",
        "Could not reach :recipient about :subject",
    ),
    ("wl_notify.task.recipient", "Recipient: :recipient"),
    ("wl_notify.task.type", "Notification type: :type"),
    ("wl_notify.task.channels", "Channels tried:"),
    ("wl_notify.task.channel_line", "- :channel :address: :error"),
    (
        "wl_notify.task.channel_line_for",
        "- :who, :channel :address: :error",
    ),
    ("wl_notify.task.no_address", "no address"),
    ("wl_notify.task.no_error", "no error recorded"),
    ("wl_notify.task.gave_up_at", "Gave up at :time (Asia/Dubai)"),
    ("wl_notify.task.party", "Party :id"),
    ("wl_notify.task.user", "User :id"),
    (
        "wl_notify.statutory.title",
        "Deliver :subject notice to :recipient by another means",
    ),
    (
        "wl_notify.statutory.methods",
        "Accepted proof of delivery: :methods",
    ),
    ("wl_notify.statutory.proof_link", "Record the proof: :path"),
    ("wl_notify.proof_method.courier", "courier"),
    ("wl_notify.proof_method.registered_mail", "registered mail"),
    ("wl_notify.proof_method.hand_delivery", "hand delivery"),
    ("wl_notify.proof_method.call", "call"),
    (
        "wl_notify.alert.title",
        "Urgent: could not reach :recipient about :subject",
    ),
    (
        "wl_notify.alert.body",
        "Notification type: :type\nTask: :ticket",
    ),
    ("wl_notify.status.queued", "Queued"),
    ("wl_notify.status.deferred", "Held"),
    ("wl_notify.status.sent", "Sent"),
    ("wl_notify.status.delivered", "Delivered"),
    ("wl_notify.status.read", "Read"),
    ("wl_notify.status.failed", "Failed"),
    ("wl_notify.status.suppressed", "Not sent (suppressed)"),
    ("wl_notify.status.gave_up", "Not delivered (gave up)"),
    (
        "wl_notify.status.fell_back",
        "Not delivered (fell back to :channel)",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key)
}

/// The package's English text for `key`, or the key when it has none.
pub(crate) fn english(key: &str) -> String {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or_else(|| key.to_owned(), |(_, text)| (*text).to_owned())
}

/// The English text for `key` with each `:name` placeholder replaced by its
/// value in one pass, so a value is never read as a placeholder.
pub(crate) fn english_with(key: &str, params: &[(&str, &str)]) -> String {
    let text = english(key);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find(':') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        match params.iter().find(|(name, _)| after.starts_with(name)) {
            Some((name, value)) => {
                out.push_str(value);
                rest = &after[name.len()..];
            }
            None => {
                out.push(':');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Every text of the package in `lang`, keyed without the `wl_notify.`
/// prefix (`fields.key`), for the templates: `tx['fields.key']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_notify."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{TEXTS, english_with};

    #[test]
    fn a_value_is_never_read_as_a_placeholder() {
        let text = english_with(
            "wl_notify.task.title",
            &[("recipient", "A :subject"), ("subject", "TKT-1")],
        );
        assert_eq!(text, "Could not reach A :subject about TKT-1");
    }

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_notify.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }
}
