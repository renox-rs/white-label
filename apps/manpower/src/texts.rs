//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    (
        "manpower.bad_input",
        "Check the form: a value is missing or not in the right form.",
    ),
    // The notices.
    ("manpower.notice.subject", "{{ title }}"),
    ("manpower.notice.body", "{{ body }}"),
    // What a refusal says.
    (
        "manpower.problems.client_unknown",
        "The client is not a known party.",
    ),
    ("manpower.problems.site_name", "Give the site a name."),
    (
        "manpower.problems.reference",
        "Give the contract a reference.",
    ),
    (
        "manpower.problems.dates",
        "The contract must end after it starts.",
    ),
    (
        "manpower.problems.payment_terms",
        "The payment terms, the notice period and the deemed-approval days cannot be negative or zero.",
    ),
    (
        "manpower.problems.signed_in_future",
        "A contract cannot be signed on a day that has not come.",
    ),
    (
        "manpower.problems.signed_document",
        "Attach the signed contract.",
    ),
    (
        "manpower.problems.not_draft",
        "The contract is no longer a draft.",
    ),
    // Client contract `activate` (§7.1).
    (
        "manpower.contract.not_signed",
        "The client has not signed the contract: record the signed contract first.",
    ),
    (
        "manpower.contract.no_rate_card",
        "No rate card version accepted by the client is in force on the start date of the contract.",
    ),
    (
        "manpower.contract.over",
        "The contract has already ended: its end date is in the past.",
    ),
    (
        "manpower.contract.history_cancel",
        "Contract cancelled before it was signed into force",
    ),
    // The manpower request (§7.1).
    (
        "manpower.request.positions",
        "The request has no position left for this worker.",
    ),
    (
        "manpower.request.contract_unknown",
        "The client contract does not exist.",
    ),
    (
        "manpower.request.contract_over",
        "The client contract has ended or was cancelled.",
    ),
    (
        "manpower.request.site",
        "Pick a site of the client that is in use.",
    ),
    (
        "manpower.request.fields",
        "Give the role and the shift, and ask for at least one worker.",
    ),
    (
        "manpower.request.dates",
        "The request must not end before it starts.",
    ),
    (
        "manpower.request.contract_not_active",
        "The contract does not take new requests now.",
    ),
    (
        "manpower.request.outside_contract",
        "The request runs past the end date of the contract.",
    ),
    (
        "manpower.request.not_on_rate_card",
        "The rate card does not price {{ role }} on the {{ shift }} shift.",
    ),
    (
        "manpower.request.submitted_title",
        "New request for {{ role }}",
    ),
    (
        "manpower.request.submitted_body",
        "The client asks for {{ count }} {{ role }} from {{ from }}.",
    ),
    // The screens.
    ("manpower.screen.status", "Status"),
    ("manpower.screen.client", "Client"),
    ("manpower.screen.none", "None."),
    ("manpower.screen.contracts.title", "Client contracts"),
    (
        "manpower.screen.contracts.subtitle",
        "The agreements with the client companies.",
    ),
    ("manpower.screen.contracts.reference", "Contract"),
    ("manpower.screen.contracts.period", "Period"),
    ("manpower.screen.contracts.billing", "Billing"),
    ("manpower.screen.contracts.cover", "Cover"),
    ("manpower.screen.contracts.cover_required", "Cover required"),
    ("manpower.screen.contracts.empty", "No client contracts yet"),
    (
        "manpower.screen.contracts.empty_text",
        "A contract is made for a client company before its workers are requested.",
    ),
    ("manpower.screen.requests.title", "Manpower requests"),
    (
        "manpower.screen.requests.subtitle",
        "The workers the clients ask for.",
    ),
    ("manpower.screen.requests.request", "Request"),
    ("manpower.screen.requests.workers", "Workers"),
    ("manpower.screen.requests.period", "Period"),
    ("manpower.screen.requests.cover", "Cover"),
    ("manpower.screen.requests.empty", "No requests yet"),
    (
        "manpower.screen.requests.empty_text",
        "A client asks for workers by role, headcount, site and shift.",
    ),
    ("manpower.screen.status.draft", "Draft"),
    ("manpower.screen.status.open", "Open"),
    ("manpower.screen.status.partially_filled", "Partly filled"),
    ("manpower.screen.status.filled", "Filled"),
    ("manpower.screen.status.active", "Active"),
    ("manpower.screen.status.on_hold", "On hold"),
    ("manpower.screen.status.terminating", "Terminating"),
    ("manpower.screen.status.ended", "Ended"),
    ("manpower.screen.status.cancelled", "Cancelled"),
    ("manpower.screen.billing.weekly", "Weekly"),
    ("manpower.screen.billing.fortnightly", "Every two weeks"),
    ("manpower.screen.billing.monthly", "Monthly"),
    // The nav.
    ("manpower.nav.contracts", "Client contracts"),
    ("manpower.nav.requests", "Manpower requests"),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key, &[])
}

/// The English text for `key` with `params` filled in as `{{ name }}`; the
/// key itself when it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &(dyn Display + Sync))]) -> String {
    let text = TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let mut data = renox::serde_json::Map::new();
    for (name, value) in params {
        data.insert(
            (*name).to_owned(),
            renox::serde_json::Value::String(value.to_string()),
        );
    }
    wl_kernel::notify::fill(text, &renox::serde_json::Value::Object(data))
}

/// The English text of `key`, unfilled: a notification template.
pub(crate) fn template(key: &str) -> &'static str {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, text)| *text)
}

/// Every text of the screens, by the name a template reads
/// (`tx['contracts.title']` is the key `manpower.screen.contracts.title`).
pub(crate) fn screen_texts(lang: &Lang) -> std::collections::BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix("manpower.screen.")?, t(lang, key))))
        .collect()
}

/// The word for `value` in `group` (`status`, `billing`); the value itself
/// when it has no text.
pub(crate) fn word(lang: &Lang, group: &str, value: &str) -> String {
    let key = format!("manpower.screen.{group}.{value}");
    let found = t(lang, &key);
    if found == key {
        value.replace('_', " ")
    } else {
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("manpower.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_text_is_filled_with_its_parameters() {
        assert_eq!(
            english("manpower.notice.subject", &[("title", &"Hello")]),
            "Hello"
        );
        assert_eq!(english("manpower.nothing", &[]), "manpower.nothing");
    }
}
