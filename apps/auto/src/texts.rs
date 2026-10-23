//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("auto.button_cancel", "Cancel"),
    ("auto.none", "—"),
    ("auto.fields.status", "Status"),
    ("auto.history.system", "System"),
];

/// Every text of the app: this file's and those of the later parts.
fn entries() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    TEXTS
        .iter()
        .chain(crate::texts_vehicles::TEXTS.iter())
        .chain(crate::texts_jobs::TEXTS.iter())
        .chain(crate::texts_stops::TEXTS.iter())
        .chain(crate::texts_claims::TEXTS.iter())
        .chain(crate::texts_fleet::TEXTS.iter())
        .chain(crate::texts_supplier::TEXTS.iter())
        .chain(crate::texts_police::TEXTS.iter())
        .chain(crate::texts_incidents::TEXTS.iter())
}

/// The text for `key`: the app's translation when it has one, else the
/// English one.
#[allow(dead_code)]
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], replacing `:name` placeholders with `params`.
#[allow(dead_code)]
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    english(key, params)
}

/// Every text of the app in `lang`, keyed without the `auto.` prefix
/// (`vehicles.title`), for the templates: `tx['vehicles.title']`.
#[allow(dead_code)]
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    entries()
        .map(|(key, _)| (key.trim_start_matches("auto."), t(lang, key)))
        .collect()
}

/// The English text for `key` with `params` filled in; the key itself when
/// it has no text.
#[allow(dead_code)]
pub(crate) fn english(key: &str, params: &[(&str, &dyn Display)]) -> String {
    let text = entries()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
    renox::i18n::format(text, &params, None)
}

#[cfg(test)]
mod tests {
    use super::{english, entries};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = entries().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("auto.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(english("auto.none", &[]), "—");
        assert_eq!(english("auto.unknown", &[]), "auto.unknown");
    }
}
