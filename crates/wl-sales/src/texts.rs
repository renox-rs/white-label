//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_sales.title", "Campaigns"),
    ("wl_sales.cancel", "Cancel"),
    ("wl_sales.lead_sources.title", "Lead sources"),
    (
        "wl_sales.lead_sources.subtitle",
        "Where campaigns and leads come from. A source in use can be switched off but not deleted.",
    ),
    ("wl_sales.lead_sources.empty", "No lead sources"),
    (
        "wl_sales.lead_sources.empty_hint",
        "Run the sales installer to store the defaults.",
    ),
    ("wl_sales.lead_sources.add", "Add a lead source"),
    ("wl_sales.lead_sources.add_title", "Add a lead source"),
    (
        "wl_sales.lead_sources.add_description",
        "The code cannot be changed later.",
    ),
    ("wl_sales.lead_sources.added", "Lead source added."),
    ("wl_sales.lead_sources.rename", "Rename"),
    ("wl_sales.lead_sources.rename_title", "Rename :code"),
    ("wl_sales.lead_sources.renamed", "Lead source renamed."),
    ("wl_sales.lead_sources.activate", "Activate"),
    ("wl_sales.lead_sources.deactivate", "Deactivate"),
    (
        "wl_sales.lead_sources.deactivate_title",
        "Deactivate :code?",
    ),
    (
        "wl_sales.lead_sources.deactivate_message",
        "It can no longer be chosen for new campaigns. Existing ones keep it.",
    ),
    ("wl_sales.lead_sources.activated", "Lead source activated."),
    (
        "wl_sales.lead_sources.deactivated",
        "Lead source deactivated.",
    ),
    ("wl_sales.lead_sources.in_use", "In use"),
    (
        "wl_sales.lead_sources.bad_code",
        "Use 2 to 30 lower case letters, digits or underscores.",
    ),
    (
        "wl_sales.lead_sources.code_taken",
        "A lead source has this code.",
    ),
    ("wl_sales.lead_sources.name_required", "Give the name."),
    (
        "wl_sales.lead_sources.in_use_refused",
        "This lead source is in use; deactivate it instead.",
    ),
    (
        "wl_sales.campaigns.name_required",
        "Give the campaign a name.",
    ),
    (
        "wl_sales.campaigns.bad_code",
        "Use 2 to 20 letters, digits, hyphens or underscores for the code.",
    ),
    (
        "wl_sales.campaigns.code_taken",
        "A campaign has or had this code.",
    ),
    (
        "wl_sales.campaigns.bad_promo_code",
        "Use 2 to 20 letters, digits, hyphens or underscores for the promo code.",
    ),
    (
        "wl_sales.campaigns.promo_code_taken",
        "Another campaign has this promo code.",
    ),
    (
        "wl_sales.campaigns.end_before_start",
        "The end date cannot be before the start date.",
    ),
    (
        "wl_sales.campaigns.not_draft",
        "Only a draft campaign can be deleted.",
    ),
    ("wl_sales.campaigns.not_found", "There is no such campaign."),
    (
        "wl_sales.notices.ended_subject",
        "Campaign {{ code }} has ended",
    ),
    (
        "wl_sales.notices.ended_body",
        "The campaign {{ name }} ({{ code }}) ended; its last day was {{ end_date }}.",
    ),
    ("wl_sales.fields.code", "Code"),
    ("wl_sales.fields.name_en", "Name (English)"),
    ("wl_sales.fields.name_ar", "Name (Arabic)"),
    ("wl_sales.fields.active", "Active"),
    ("wl_sales.fields.status", "Status"),
    ("wl_sales.status.active", "Active"),
    ("wl_sales.status.inactive", "Inactive"),
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

/// Every text of the package in `lang`, keyed without the `wl_sales.`
/// prefix (`fields.code`), for the templates: `tx['fields.code']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_sales."), t(lang, key)))
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
        assert!(keys.iter().all(|key| key.starts_with("wl_sales.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(
            english(
                "wl_sales.lead_sources.rename_title",
                &[("code", &"walk_in")]
            ),
            "Rename walk_in"
        );
    }
}
