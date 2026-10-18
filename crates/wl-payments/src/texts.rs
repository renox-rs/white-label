//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[("wl_payments.title", "Payments")];

/// The text for `key`: the app's translation when it has one, else the English one.
#[expect(dead_code, reason = "the back-office pages (#94 part 7) use it")]
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
