//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_notify.title", "Notifications"),
    (
        "wl_notify.optout.email",
        "You are receiving this because you agreed to hear from :brand. Unsubscribe: :link",
    ),
    (
        "wl_notify.optout.email_ar",
        "وصلتك هذه الرسالة لأنك وافقت على تلقي رسائل من :brand. لإلغاء الاشتراك: :link",
    ),
    ("wl_notify.optout.sms", ":brand. Reply STOP to opt out"),
    ("wl_notify.optout.sms_ar", ":brand. للإلغاء أرسل STOP"),
    (
        "wl_notify.unsubscribe.title",
        "Unsubscribe from marketing emails",
    ),
    (
        "wl_notify.unsubscribe.intro",
        "Confirm to stop receiving marketing emails from this sender. Messages about your account and invoices are not affected.",
    ),
    ("wl_notify.unsubscribe.confirm", "Unsubscribe"),
    ("wl_notify.unsubscribe.done_title", "You are unsubscribed"),
    (
        "wl_notify.unsubscribe.done",
        "You will not receive marketing emails from this sender any more.",
    ),
    ("wl_notify.unsubscribe.invalid_title", "Link not valid"),
    (
        "wl_notify.unsubscribe.invalid",
        "This unsubscribe link is not valid. Use the link in a recent email from the sender.",
    ),
    (
        "wl_notify.broadcast.excluded.blocked_party",
        "On hold or no longer active",
    ),
    (
        "wl_notify.broadcast.excluded.frequency_cap",
        "Over the marketing frequency cap",
    ),
    ("wl_notify.broadcast.excluded.no_consent", "No consent"),
    (
        "wl_notify.broadcast.excluded.no_contact_point",
        "No contact point",
    ),
    ("wl_notify.broadcast.excluded.opted_out", "Opted out"),
    (
        "wl_notify.broadcast.excluded.suppressed",
        "Address suppressed",
    ),
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
    (
        "wl_notify.notice.not_delivered",
        "Notice not delivered: :label",
    ),
    ("wl_notify.notice.since", "Since :date"),
    ("wl_notify.notice.record_proof", "Record proof"),
    ("wl_notify.notice.banner_title", "Notice not delivered"),
    ("wl_notify.proof.title", "Record proof of delivery"),
    (
        "wl_notify.proof.subtitle",
        "The notice could not be delivered by message; record how it was delivered.",
    ),
    ("wl_notify.proof.notice", "Notice"),
    ("wl_notify.proof.about", "About"),
    ("wl_notify.proof.recipient", "For"),
    ("wl_notify.proof.gave_up_on", "Not delivered since"),
    ("wl_notify.proof.method", "Delivered by"),
    ("wl_notify.proof.delivered_on", "Delivery date"),
    ("wl_notify.proof.note", "Note"),
    (
        "wl_notify.proof.attachment",
        "Signed note, receipt or photo",
    ),
    (
        "wl_notify.proof.attachment_hint",
        "PDF, image or text file, up to 20 MB. A call needs a note instead.",
    ),
    ("wl_notify.proof.submit", "Record proof"),
    ("wl_notify.proof.cancel", "Cancel"),
    ("wl_notify.proof.recorded", "Proof recorded"),
    ("wl_notify.proof.recorded_title", "Proof already recorded"),
    ("wl_notify.proof.recorded_by", "Recorded by user :user"),
    ("wl_notify.proof.download", "Download the attachment"),
    ("wl_notify.proof.task_note", "Proof recorded"),
    (
        "wl_notify.proof.already_recorded",
        "A proof is already recorded for this notice, or it was delivered.",
    ),
    (
        "wl_notify.proof.date_invalid",
        "Enter the date as YYYY-MM-DD.",
    ),
    (
        "wl_notify.proof.attachment_size",
        "The file is larger than 20 MB.",
    ),
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
    (
        "wl_notify.otp.try_other",
        "Code could not be sent; try :channel",
    ),
    ("wl_notify.otp.failed", "Code could not be sent"),
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
    ("wl_notify.broadcasts.title", "Broadcasts"),
    (
        "wl_notify.broadcasts.subtitle",
        "Marketing messages to a segment, asked for by a campaign. Above the size limit a second approver must agree.",
    ),
    ("wl_notify.broadcasts.empty", "No broadcasts yet"),
    (
        "wl_notify.broadcasts.empty_hint",
        "A campaign asks for a broadcast; it shows here.",
    ),
    ("wl_notify.broadcasts.detail_title", "Broadcast"),
    ("wl_notify.broadcasts.estimate", "Estimated recipients"),
    ("wl_notify.broadcasts.send_at", "Send at"),
    ("wl_notify.broadcasts.template", "Message"),
    ("wl_notify.broadcasts.rule_label", "Audience"),
    (
        "wl_notify.broadcasts.rule",
        "Segment :segment of :requester",
    ),
    ("wl_notify.broadcasts.requested_by", "Asked by (user)"),
    ("wl_notify.broadcasts.approved_by", "Approved by (user)"),
    ("wl_notify.broadcasts.cancel_reason", "Cancel reason"),
    ("wl_notify.broadcasts.recipients", "Recipients queued"),
    ("wl_notify.broadcasts.excluded", "Left out"),
    ("wl_notify.broadcasts.reason", "Reason"),
    ("wl_notify.broadcasts.excluded_none", "Nobody was left out."),
    ("wl_notify.broadcasts.results", "Results"),
    ("wl_notify.broadcasts.results.sent", "Sent"),
    ("wl_notify.broadcasts.results.delivered", "Delivered"),
    ("wl_notify.broadcasts.results.failed", "Failed"),
    ("wl_notify.broadcasts.results.opted_out", "Opted out"),
    ("wl_notify.broadcasts.cancel", "Cancel broadcast"),
    (
        "wl_notify.broadcasts.cancel_reason_label",
        "Reason (optional)",
    ),
    ("wl_notify.broadcasts.cancelled", "Broadcast cancelled"),
    ("wl_notify.broadcasts.back", "All broadcasts"),
    ("wl_notify.broadcasts.count", "Count"),
    ("wl_notify.send_windows.title", "Marketing send windows"),
    (
        "wl_notify.send_windows.subtitle",
        "Marketing is sent only inside the window of its channel (Asia/Dubai time). A message outside it waits for the next opening.",
    ),
    ("wl_notify.send_windows.start", "Opens"),
    ("wl_notify.send_windows.end", "Closes"),
    ("wl_notify.send_windows.save", "Save"),
    ("wl_notify.send_windows.saved", "Send window saved"),
    (
        "wl_notify.send_windows.sms_hint",
        "SMS marketing may only be sent 07:00-21:00.",
    ),
    (
        "wl_notify.send_windows.hint",
        "Any window within the day; it must open before it closes.",
    ),
    (
        "wl_notify.send_windows.bad_time",
        "Enter a time as hours and minutes, e.g. 07:00.",
    ),
    ("wl_notify.broadcast.status.draft", "Draft"),
    (
        "wl_notify.broadcast.status.pending_approval",
        "Waiting for approval",
    ),
    ("wl_notify.broadcast.status.scheduled", "Scheduled"),
    ("wl_notify.broadcast.status.sending", "Sending"),
    ("wl_notify.broadcast.status.completed", "Completed"),
    ("wl_notify.broadcast.status.cancelled", "Cancelled"),
    (
        "wl_notify.broadcast.problem.within_limit",
        "This broadcast is within the size that needs no approval.",
    ),
    (
        "wl_notify.broadcast.problem.over_limit",
        "This broadcast is above the size that needs no approval.",
    ),
    (
        "wl_notify.broadcast.problem.no_limit",
        "The broadcast limit was not given.",
    ),
    (
        "wl_notify.broadcast.problem.cancel_not_allowed",
        "Only the requester or a holder of notifications.marketing.send may cancel a broadcast.",
    ),
    (
        "wl_notify.broadcast.problem.no_approval_policy",
        "A broadcast above the limit needs an approval policy.",
    ),
    (
        "wl_notify.broadcast.problem.no_user",
        "A broadcast needs the user who asks for it.",
    ),
    (
        "wl_notify.broadcast.problem.requester_ended",
        "A broadcast can only be asked for by an active campaign.",
    ),
    (
        "wl_notify.broadcast.problem.requester_refused",
        "The campaign may not send this broadcast: :reason.",
    ),
    (
        "wl_notify.broadcast.problem.unknown_type",
        "The notification type of this broadcast does not exist.",
    ),
    (
        "wl_notify.broadcast.problem.not_marketing",
        "Only a notification type of category marketing can be broadcast.",
    ),
    (
        "wl_notify.broadcast.problem.channel_unused",
        "The notification type is not sent on this channel.",
    ),
    (
        "wl_notify.broadcast.problem.no_template",
        "The notification type has no text for this channel.",
    ),
    (
        "wl_notify.broadcast.problem.no_sender_id",
        "A marketing SMS needs a registered AD- sender ID.",
    ),
    (
        "wl_notify.broadcast.problem.not_found",
        "The broadcast no longer exists.",
    ),
    (
        "wl_notify.broadcast.cancelled.subject",
        "Your broadcast was cancelled",
    ),
    (
        "wl_notify.broadcast.cancelled.body",
        "Broadcast {{ broadcast }} on {{ channel }} was cancelled: {{ reason }}.",
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
