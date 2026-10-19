//! The package's texts. English is the base; the neutral refusal also has its
//! Arabic text here, because the portal must show it in Arabic exactly
//! (wl-portal §6, §7). An app changes or translates any text by putting the
//! key in its `resources/lang/{locale}.json`; that wins.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// The key of the neutral text for actions refused while the party is blocked
/// (wl-portal §7, the setting *Neutral text*).
pub const NEUTRAL_KEY: &str = "wl_portal.blocked.neutral";

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    (
        NEUTRAL_KEY,
        "This request cannot be completed online. Please contact us.",
    ),
    ("wl_portal.refusals.title", "Refused portal actions"),
    (
        "wl_portal.refusals.subtitle",
        "Portal actions refused because the party is blocked. Only you can see this list.",
    ),
    ("wl_portal.refusals.empty", "Nothing refused"),
    ("wl_portal.refusals.at", "When (Asia/Dubai)"),
    ("wl_portal.refusals.user", "Portal user"),
    ("wl_portal.refusals.party", "Party"),
    ("wl_portal.refusals.action", "Action"),
    ("wl_portal.action.new_booking", "New booking"),
    ("wl_portal.action.new_order", "New order"),
    ("wl_portal.action.sign_agreement", "Sign a new agreement"),
    ("wl_portal.action.refund_request", "Refund request"),
    (
        "wl_portal.action.deposit_release_request",
        "Deposit release request",
    ),
    (
        "wl_portal.action.cancel_with_refund",
        "Cancellation with a refund",
    ),
    ("wl_portal.action.pay_invoice", "Pay an invoice"),
    ("wl_portal.action.maintenance_request", "Maintenance request"),
    ("wl_portal.action.complaint", "Complaint"),
    ("wl_portal.action.message", "Message"),
    ("wl_portal.action.document_upload", "Document upload"),
    ("wl_portal.action.profile_change", "Profile change"),
    (
        "wl_portal.action.cancel_without_refund",
        "Cancellation without a refund",
    ),
    (
        "wl_portal.grants.refused.suspended_from",
        "The grant was not suspended from :from.",
    ),
];

/// The Arabic texts of the keys that must show in Arabic out of the box.
const ARABIC: &[(&str, &str)] = &[(
    NEUTRAL_KEY,
    "هذا الطلب لا يمكن إتمامه عبر الإنترنت. يرجى التواصل معنا.",
)];

/// The text for `key`: the app's translation when it has one, else the
/// package's Arabic text in an Arabic page, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], replacing `:name` placeholders with `params`.
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    if lang.locale == "ar"
        && let Some((_, text)) = ARABIC.iter().find(|(k, _)| *k == key)
    {
        let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
        return renox::i18n::format(text, &params, None);
    }
    english(key, params)
}

/// The neutral text shown for an action refused while the party is blocked,
/// in the user's language: the same whatever the reason.
pub fn neutral(lang: &Lang) -> String {
    t(lang, NEUTRAL_KEY)
}

/// Every text of the package in `lang`, keyed without the `wl_portal.`
/// prefix (`refusals.title`), for the templates: `tx['refusals.title']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_portal."), t(lang, key)))
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
    use super::{ARABIC, NEUTRAL_KEY, TEXTS, english};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_portal.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn the_neutral_text_is_the_one_of_the_spec_in_both_languages() {
        assert_eq!(
            english(NEUTRAL_KEY, &[]),
            "This request cannot be completed online. Please contact us."
        );
        assert_eq!(
            ARABIC[0].1,
            "هذا الطلب لا يمكن إتمامه عبر الإنترنت. يرجى التواصل معنا."
        );
    }

    #[test]
    fn the_neutral_text_never_names_a_block() {
        let text = english(NEUTRAL_KEY, &[]).to_lowercase();
        for word in ["hold", "block", "sanction", "frozen"] {
            assert!(!text.contains(word), "{word}");
        }
    }

    #[test]
    fn every_action_has_a_text() {
        for action in crate::guard::PortalAction::ALL {
            let key = format!("wl_portal.action.{}", action.key());
            assert!(TEXTS.iter().any(|(k, _)| *k == key), "{key}");
        }
    }
}
