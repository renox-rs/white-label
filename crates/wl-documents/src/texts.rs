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
    (
        "wl_documents.guard.no_provider",
        "No module maps :guarded to its document subjects",
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
