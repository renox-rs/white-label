//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_kernel.legal_entities", "Legal entities"),
    (
        "wl_kernel.approvals.already_pending",
        "This is already waiting for approval.",
    ),
    ("wl_kernel.approvals.sent", "Sent for approval"),
    (
        "wl_kernel.approvals.requested_title",
        "Approval requested: :action",
    ),
    (
        "wl_kernel.approvals.requested_body",
        "Request #:request about :subject waits for your decision.",
    ),
    (
        "wl_kernel.approvals.already_decided",
        "This request was already decided.",
    ),
    (
        "wl_kernel.approvals.comment_required",
        "A comment is required to reject.",
    ),
    (
        "wl_kernel.approvals.reason_withdrawn",
        "approval withdrawn by requester",
    ),
    ("wl_kernel.approvals.reason_expired", "approval expired"),
    (
        "wl_kernel.approvals.reason_failed",
        "approval failed: :message",
    ),
    (
        "wl_kernel.approvals.subject_moved",
        "Subject already moved to `:state`.",
    ),
    (
        "wl_kernel.approvals.approved_title",
        "Approval granted: :action",
    ),
    (
        "wl_kernel.approvals.rejected_title",
        "Approval rejected: :action",
    ),
    (
        "wl_kernel.approvals.returned_title",
        "Approval returned: :action",
    ),
    (
        "wl_kernel.approvals.withdrawn_title",
        "Approval withdrawn: :action",
    ),
    (
        "wl_kernel.approvals.expired_title",
        "Approval expired: :action",
    ),
    (
        "wl_kernel.approvals.failed_title",
        "Approval failed: :action",
    ),
    (
        "wl_kernel.approvals.decided_body",
        "Request #:request about :subject: :reason",
    ),
    (
        "wl_kernel.approvals.approved_body",
        "Request #:request about :subject was approved.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], filling the `:name` placeholders with `params`.
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    en_with(key, params)
}

/// The English text for `key`: for texts stored in the app's default
/// language (history reasons) and messages written without a request.
pub(crate) fn en(key: &str) -> String {
    en_with(key, &[])
}

/// Like [`en`], filling the `:name` placeholders with `params`.
pub(crate) fn en_with(key: &str, params: &[(&str, &dyn Display)]) -> String {
    let text = TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
    renox::i18n::format(text, &params, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_texts_fill_their_placeholders() {
        assert_eq!(en("wl_kernel.approvals.sent"), "Sent for approval");
        assert_eq!(
            en_with(
                "wl_kernel.approvals.requested_title",
                &[("action", &"payments.refund")]
            ),
            "Approval requested: payments.refund"
        );
        assert_eq!(en("wl_kernel.unknown"), "wl_kernel.unknown");
    }
}
