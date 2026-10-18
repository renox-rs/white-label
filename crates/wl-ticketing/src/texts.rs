//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_ticketing.title", "Tickets"),
    (
        "wl_ticketing.notice.assigned.subject",
        "Ticket {{ number }} is assigned to you",
    ),
    (
        "wl_ticketing.notice.assigned.body",
        "{{ title }} ({{ priority }} priority). Visit: {{ visit }}. Respond by {{ response_due }}; resolve by {{ resolution_due }}.",
    ),
    (
        "wl_ticketing.notice.withdrawn.subject",
        "Ticket {{ number }} was reassigned",
    ),
    (
        "wl_ticketing.notice.withdrawn.body",
        "{{ title }} is no longer yours; it now goes to {{ assignee }}. Reason: {{ reason }}.",
    ),
    (
        "wl_ticketing.notice.visit_rescheduled.subject",
        "The visit for ticket {{ number }} has moved",
    ),
    (
        "wl_ticketing.notice.visit_rescheduled.body",
        "{{ title }}: the visit is now {{ visit }}.",
    ),
    ("wl_ticketing.notice.not_scheduled", "not scheduled"),
];

/// The English text for `key`, or `key` itself when the package has none.
/// Notification templates are registered with it: the client changes them on
/// the stored notification type, not in the language files.
pub(crate) fn english(key: &str) -> &str {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| text)
}

/// The text for `key`: the app's translation when it has one, else the English one.
#[expect(dead_code, reason = "the back-office pages (#146 part 7) use it")]
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key).to_owned()
}
