//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_documents.title", "Documents"),
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
];

/// The text for `key`: the app's translation when it has one, else the English one.
#[expect(
    dead_code,
    reason = "the guard (#45 part 5) and pages (#45 part 6) use it"
)]
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
