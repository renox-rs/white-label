//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_documents.title", "Documents"),
    ("wl_documents.none", "—"),
    ("wl_documents.cancel", "Cancel"),
    ("wl_documents.review.title", "Review queue"),
    (
        "wl_documents.review.subtitle",
        "Documents waiting for review. A renewal a refused transition waits for comes first.",
    ),
    ("wl_documents.review.empty", "Nothing to review"),
    (
        "wl_documents.review.empty_hint",
        "Uploaded documents show up here until they are accepted or rejected.",
    ),
    ("wl_documents.review.priority", "Blocking"),
    ("wl_documents.review.file", "Open file"),
    ("wl_documents.review.accept", "Accept"),
    ("wl_documents.review.accept_title", "Accept this document?"),
    (
        "wl_documents.review.accept_description",
        "It becomes the valid document of its kind for this record, and the previous one is superseded.",
    ),
    ("wl_documents.review.accepted", "Document accepted."),
    ("wl_documents.review.reject", "Reject"),
    ("wl_documents.review.reject_title", "Reject this document?"),
    (
        "wl_documents.review.reject_description",
        "The uploader is told the reason and can send a new version.",
    ),
    ("wl_documents.review.rejected", "Document rejected."),
    (
        "wl_documents.review.reason_required",
        "Say why the document is rejected.",
    ),
    ("wl_documents.fields.kind", "Kind"),
    ("wl_documents.fields.subject", "Record"),
    ("wl_documents.fields.title", "Title"),
    ("wl_documents.fields.number", "Number"),
    ("wl_documents.fields.expiry_date", "Expires"),
    ("wl_documents.fields.version", "Version"),
    ("wl_documents.fields.uploaded_by", "Uploaded by"),
    ("wl_documents.fields.submitted_at", "Submitted"),
    ("wl_documents.fields.reason", "Reason"),
    ("wl_documents.fields.key", "Key"),
    ("wl_documents.fields.name", "Name"),
    ("wl_documents.fields.subject_types", "Attaches to"),
    ("wl_documents.fields.legal_block", "Legal block"),
    ("wl_documents.fields.override_mode", "Override"),
    ("wl_documents.fields.responsible_role", "Renewed by"),
    ("wl_documents.fields.entries", "Guarded transitions"),
    ("wl_documents.fields.guarded", "Transition"),
    (
        "wl_documents.fields.guarded_hint",
        "<entity>.<transition>, e.g. deployment.activate",
    ),
    ("wl_documents.fields.mode", "Mode"),
    ("wl_documents.fields.condition", "Applies to"),
    ("wl_documents.fields.condition_field", "Condition field"),
    (
        "wl_documents.fields.condition_field_hint",
        "Optional: a field of the guarded record, e.g. category",
    ),
    ("wl_documents.fields.condition_values", "Condition values"),
    (
        "wl_documents.fields.condition_values_hint",
        "The values of that field the entry applies to",
    ),
    ("wl_documents.fields.locked", "Locked"),
    ("wl_documents.mode.block", "Block"),
    ("wl_documents.mode.flag", "Flag"),
    ("wl_documents.override.never", "Not allowed"),
    ("wl_documents.override.with_approval", "With approval"),
    ("wl_documents.kinds.title", "Document kinds"),
    (
        "wl_documents.kinds.subtitle",
        "The kinds of documents records carry, and the transitions they guard.",
    ),
    ("wl_documents.kinds.empty", "No document kinds yet"),
    (
        "wl_documents.kinds.empty_hint",
        "Kinds show up here once the modules that use them are installed.",
    ),
    ("wl_documents.kinds.every_record", "Every record"),
    ("wl_documents.kinds.condition", ":field is one of :values"),
    (
        "wl_documents.kinds.no_entries",
        "This kind guards no transition",
    ),
    ("wl_documents.kinds.add", "Add transition"),
    ("wl_documents.kinds.add_title", "Guard a transition"),
    (
        "wl_documents.kinds.add_description",
        "While the record has no valid document of this kind, the transition is refused (block) or flagged (flag).",
    ),
    ("wl_documents.kinds.added", "Transition added."),
    ("wl_documents.kinds.switch", "Change mode"),
    (
        "wl_documents.kinds.switch_title",
        "Change the mode of :guarded",
    ),
    ("wl_documents.kinds.switched", "Mode changed."),
    ("wl_documents.kinds.override_title", "Override mode"),
    (
        "wl_documents.kinds.override_description",
        "Whether a refusal for this kind can be overridden once, with a second person's approval.",
    ),
    ("wl_documents.kinds.override_change", "Change override"),
    (
        "wl_documents.kinds.override_changed",
        "Override mode changed.",
    ),
    (
        "wl_documents.kinds.override_locked",
        ":kind is a legal block: its override mode is locked to never",
    ),
    ("wl_documents.kinds.remove", "Remove"),
    ("wl_documents.kinds.remove_title", "Stop guarding :guarded?"),
    (
        "wl_documents.kinds.remove_message",
        "The transition no longer checks this kind of document.",
    ),
    ("wl_documents.kinds.removed", "Transition removed."),
    (
        "wl_documents.kinds.condition_incomplete",
        "Give both the condition field and its values, or neither",
    ),
    (
        "wl_documents.kinds.locked",
        ":kind is a legal block: :guarded cannot be removed or changed",
    ),
    (
        "wl_documents.kinds.no_provider",
        "No module guards :guarded, so :kind cannot block it",
    ),
    (
        "wl_documents.kinds.already_guarded",
        ":kind already guards :guarded",
    ),
    (
        "wl_documents.kinds.invalid_transition",
        ":guarded is not an <entity>.<transition> for :kind",
    ),
    (
        "wl_documents.attach.subject_type",
        ":kind cannot be attached to :subject_type",
    ),
    (
        "wl_documents.attach.file_type",
        ":kind does not accept .:extension files (allowed: :allowed)",
    ),
    (
        "wl_documents.attach.too_large",
        ":file is larger than :max MB, the most :kind allows",
    ),
    ("wl_documents.attach.empty", ":file is empty"),
    (
        "wl_documents.attach.needs_review",
        ":kind needs review: submit it instead of recording it",
    ),
    (
        "wl_documents.attach.number_missing",
        ":kind needs its number",
    ),
    (
        "wl_documents.attach.expiry_missing",
        ":kind needs its expiry date",
    ),
    ("wl_documents.guard.expired", ":kind expired on :date"),
    ("wl_documents.guard.missing", ":kind missing"),
    ("wl_documents.guard.rejected", ":kind rejected"),
    (
        "wl_documents.guard.pending_review",
        ":kind not accepted yet",
    ),
    ("wl_documents.guard.renew", "renew: :role"),
    ("wl_documents.guard.override_never", "override: not allowed"),
    (
        "wl_documents.guard.override_with_approval",
        "override: with approval",
    ),
    (
        "wl_documents.guard.renewal_pending",
        "renewal pending review since :date",
    ),
    ("wl_documents.flags.title", "Document flags"),
    (
        "wl_documents.flags.subtitle",
        "Records a transition went through with a document not valid. Unreviewed flags come first; a flag is reviewed with a note and never removed.",
    ),
    ("wl_documents.flags.label", "Flag"),
    ("wl_documents.flags.state", "State"),
    ("wl_documents.flags.state.open", "Unreviewed"),
    ("wl_documents.flags.state.reviewed", "Reviewed"),
    ("wl_documents.flags.record_type", "Record type"),
    ("wl_documents.flags.guarded", "Guarded transition"),
    ("wl_documents.flags.on_date", "On date"),
    ("wl_documents.flags.reviewed_by", "Reviewed by"),
    ("wl_documents.flags.reviewed_at", "Reviewed at"),
    ("wl_documents.flags.note", "Review note"),
    ("wl_documents.flags.empty", "No flags"),
    (
        "wl_documents.flags.empty_hint",
        "Records that went through a flag-mode transition with a document not valid show up here.",
    ),
    ("wl_documents.flags.review", "Mark reviewed"),
    (
        "wl_documents.flags.review_title",
        "Mark this flag reviewed?",
    ),
    (
        "wl_documents.flags.review_description",
        "The flag stays on the record with your name and the note.",
    ),
    ("wl_documents.flags.reviewed", "Flag marked reviewed."),
    ("wl_documents.flag.expired", ":kind expired :date"),
    ("wl_documents.flag.missing", ":kind missing"),
    ("wl_documents.flag.rejected", ":kind rejected"),
    (
        "wl_documents.flag.pending_review",
        ":kind renewal pending review",
    ),
    (
        "wl_documents.flag.note_required",
        "Say why the flag is reviewed.",
    ),
    (
        "wl_documents.flag.already_reviewed",
        "This flag was already reviewed.",
    ),
    ("wl_documents.planned.expires", ":kind expires :date"),
    ("wl_documents.planned.expired", ":kind expired on :date"),
    ("wl_documents.planned.missing", ":kind missing"),
    ("wl_documents.planned.rejected", ":kind rejected"),
    (
        "wl_documents.planned.pending_review",
        ":kind renewal pending review",
    ),
    (
        "wl_documents.expiring.subject",
        "{{ kind_name }} of {{ subject_label }} expires on {{ expires_on }}",
    ),
    (
        "wl_documents.expiring.body",
        "{{ summary }}\n{{ items_text }}",
    ),
    (
        "wl_documents.expiring.reminder",
        "The :kind of :subject expires on :expires (:left day(s) left).",
    ),
    (
        "wl_documents.expiring.items",
        "Planned work dated after the expiry date is not covered:",
    ),
    ("wl_documents.expiring.item", "- :label on :date (:guarded)"),
    (
        "wl_documents.expired.subject",
        "{{ kind_name }} of {{ subject_label }} expired on {{ expires_on }}",
    ),
    (
        "wl_documents.expired.body",
        "{{ summary }}\n{{ items_text }}",
    ),
    (
        "wl_documents.expired.message",
        "The :kind of :subject expired on :expires. Renew it before the work below goes on.",
    ),
    (
        "wl_documents.expired.items",
        "These records need a valid document:",
    ),
    ("wl_documents.expired.item", "- :label on :date (:guarded)"),
    ("wl_documents.renewal.title", "Renew :kind of :subject"),
    (
        "wl_documents.renewal.description",
        "The :kind of :subject expired on :expires. Upload the renewed document for review.",
    ),
    (
        "wl_documents.renewal.done",
        "Renewed: :kind, valid until :expires",
    ),
    (
        "wl_documents.renewal.done_open",
        "Renewed: :kind, no expiry date",
    ),
    (
        "wl_documents.override.blank_reason",
        "Say why the override is needed.",
    ),
    (
        "wl_documents.override.not_overridable",
        ":kind cannot be overridden",
    ),
    (
        "wl_documents.override.nothing_to_override",
        ":kind does not block :guarded for this record: there is nothing to override",
    ),
    (
        "wl_documents.override.already_open",
        "An override of :kind for this record is already open",
    ),
    (
        "wl_documents.override.no_approval",
        "Overrides need their approval policy: it is turned off, so no override can be requested",
    ),
    (
        "wl_documents.guard.no_provider",
        "No module maps :guarded to its document subjects",
    ),
    ("wl_documents.overrides.title", "Block overrides"),
    (
        "wl_documents.overrides.subtitle",
        "One-use permissions, approved by a second person, to run a blocked transition.",
    ),
    ("wl_documents.overrides.new", "Request an override"),
    (
        "wl_documents.overrides.waiting",
        "Waiting for your approval",
    ),
    (
        "wl_documents.overrides.waiting_empty",
        "Nothing waits for your approval",
    ),
    ("wl_documents.overrides.mine", "Your requests"),
    (
        "wl_documents.overrides.mine_empty",
        "You have not asked for an override",
    ),
    ("wl_documents.overrides.open", "Open"),
    ("wl_documents.overrides.details", "Override"),
    ("wl_documents.overrides.history", "History"),
    ("wl_documents.overrides.no_history", "No history yet"),
    ("wl_documents.overrides.history_change", "Change"),
    ("wl_documents.overrides.history_at", "When (Asia/Dubai)"),
    ("wl_documents.overrides.history_reason", "Note"),
    ("wl_documents.overrides.fields.record", "Record id"),
    ("wl_documents.overrides.fields.requested_by", "Requested by"),
    ("wl_documents.overrides.fields.approved_by", "Approved by"),
    (
        "wl_documents.overrides.fields.approved_at",
        "Approved (Asia/Dubai)",
    ),
    (
        "wl_documents.overrides.fields.valid_until",
        "Valid until (Asia/Dubai)",
    ),
    ("wl_documents.overrides.fields.used_at", "Used (Asia/Dubai)"),
    ("wl_documents.overrides.fields.status", "Status"),
    ("wl_documents.overrides.fields.reason", "Reason"),
    (
        "wl_documents.overrides.fields.reason_hint",
        "Why the blocked transition must run anyway.",
    ),
    (
        "wl_documents.overrides.fields.guarded_hint",
        "The guarded transition, e.g. deployment.activate.",
    ),
    (
        "wl_documents.overrides.request_title",
        "Request an override",
    ),
    (
        "wl_documents.overrides.request_description",
        "A second person with the permission must approve it. Once approved it is valid for 72 hours and you can use it once.",
    ),
    ("wl_documents.overrides.request_submit", "Send request"),
    ("wl_documents.overrides.requested", "Override requested."),
    (
        "wl_documents.overrides.status.requested",
        "Waiting for approval",
    ),
    ("wl_documents.overrides.status.approved", "Approved"),
    ("wl_documents.overrides.status.refused", "Refused"),
    ("wl_documents.overrides.status.used", "Used"),
    ("wl_documents.overrides.status.expired", "Expired"),
    (
        "wl_documents.sops.refused.blank_code",
        "Give the SOP a code.",
    ),
    (
        "wl_documents.sops.refused.blank_title",
        "Give the SOP an English title.",
    ),
    (
        "wl_documents.sops.refused.blank_owner_role",
        "Choose the role that owns the SOP.",
    ),
    (
        "wl_documents.sops.refused.bad_interval",
        "The review interval and the days to acknowledge must be at least 1.",
    ),
    (
        "wl_documents.sops.refused.bad_entity",
        "Give the legal entity as both a type and an id, or leave both empty.",
    ),
    (
        "wl_documents.sops.refused.code_taken",
        "An SOP with the code :code already exists for this legal entity.",
    ),
    (
        "wl_documents.sops.refused.withdrawn",
        ":code is withdrawn; accept a new version before changing its readers.",
    ),
    (
        "wl_documents.sops.refused.blank_reader_role",
        "Give each reader a role.",
    ),
    (
        "wl_documents.sops.refused.bad_reader_scope",
        "Give a reader's scope as both a type and an id, or leave both empty.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], replacing `:name` placeholders with `params`.
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    english(key, params)
}

/// Every text of the package in `lang`, keyed without the `wl_documents.`
/// prefix (`fields.kind`), for the templates: `tx['fields.kind']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_documents."), t(lang, key)))
        .collect()
}

/// The English text for `key` with `params` filled in; the key itself when
/// it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &dyn Display)]) -> String {
    let text = TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
    renox::i18n::format(text, &params, None)
}

#[cfg(test)]
mod tests {
    use super::{TEXTS, english};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_documents.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(
            english(
                "wl_documents.kinds.switch_title",
                &[("guarded", &"run.dispatch")]
            ),
            "Change the mode of run.dispatch"
        );
    }
}
