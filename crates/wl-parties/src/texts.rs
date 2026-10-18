//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_parties.title", "Parties"),
    (
        "wl_parties.errors.legal_name_required",
        "Enter the legal name in English.",
    ),
    (
        "wl_parties.errors.details_kind",
        "Person details are for persons and company details for companies.",
    ),
    ("wl_parties.errors.trn_invalid", "A TRN has 15 digits."),
    ("wl_parties.errors.role_required", "Choose a role."),
    (
        "wl_parties.errors.contact_required",
        "Enter the contact's address or number.",
    ),
    (
        "wl_parties.errors.one_primary",
        "Only one contact of each type can be primary.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or_else(|| key.to_owned(), |(_, text)| (*text).to_owned())
}
