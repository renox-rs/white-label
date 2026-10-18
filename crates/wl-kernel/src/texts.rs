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
        "wl_kernel.approvals.task_closed",
        "This task is already closed.",
    ),
    (
        "wl_kernel.approvals.comment_required",
        "A comment is required to reject or return.",
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
        "Returned for changes: fix and resubmit (:action)",
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
    ("wl_kernel.approvals.card.title", "Approval: :action"),
    ("wl_kernel.approvals.card.status", "Status"),
    ("wl_kernel.approvals.card.requester", "Requested by"),
    ("wl_kernel.approvals.card.requested_at", "Requested"),
    ("wl_kernel.approvals.card.expires_at", "Expires"),
    ("wl_kernel.approvals.card.current_step", "Current step"),
    ("wl_kernel.approvals.card.step_of", "Step :step of :steps"),
    ("wl_kernel.approvals.card.steps", "Steps"),
    ("wl_kernel.approvals.card.step", "Step"),
    ("wl_kernel.approvals.card.approvers", "Approvers"),
    (
        "wl_kernel.approvals.card.holders_of",
        "holders of :permission",
    ),
    ("wl_kernel.approvals.card.decisions", "Decisions"),
    ("wl_kernel.approvals.card.decision", "Decision"),
    ("wl_kernel.approvals.card.decided_by", "By"),
    ("wl_kernel.approvals.card.decided_at", "When"),
    ("wl_kernel.approvals.card.comment", "Comment"),
    (
        "wl_kernel.approvals.card.comment_optional",
        "Optional. The requester sees it.",
    ),
    (
        "wl_kernel.approvals.card.comment_reject",
        "Say why. The requester sees it.",
    ),
    ("wl_kernel.approvals.card.approve", "Approve"),
    (
        "wl_kernel.approvals.card.approve_title",
        "Approve this request",
    ),
    ("wl_kernel.approvals.card.reject", "Reject"),
    (
        "wl_kernel.approvals.card.reject_title",
        "Reject this request",
    ),
    ("wl_kernel.approvals.card.withdraw", "Withdraw"),
    (
        "wl_kernel.approvals.card.withdraw_title",
        "Withdraw this request?",
    ),
    (
        "wl_kernel.approvals.card.withdraw_message",
        "The record goes back to the state it had before the request.",
    ),
    ("wl_kernel.approvals.card.cancel", "Cancel"),
    ("wl_kernel.approvals.card.unknown_user", "User #:id"),
    ("wl_kernel.approvals.status.pending", "Pending approval"),
    ("wl_kernel.approvals.status.approved", "Approved"),
    ("wl_kernel.approvals.status.rejected", "Rejected"),
    ("wl_kernel.approvals.status.returned", "Returned"),
    ("wl_kernel.approvals.status.withdrawn", "Withdrawn"),
    ("wl_kernel.approvals.status.expired", "Expired"),
    ("wl_kernel.approvals.status.failed", "Failed"),
    ("wl_kernel.approvals.decision.approve", "Approved"),
    ("wl_kernel.approvals.decision.reject", "Rejected"),
    ("wl_kernel.approvals.decision.return", "Returned"),
    ("wl_kernel.approvals.step_state.done", "Approved"),
    (
        "wl_kernel.approvals.step_state.current",
        "Waiting for a decision",
    ),
    ("wl_kernel.approvals.step_state.waiting", "Not reached yet"),
    ("wl_kernel.approvals.step_state.closed", "Closed"),
    (
        "wl_kernel.approvals.timeline.requested",
        "Approval requested: :action",
    ),
    (
        "wl_kernel.approvals.timeline.requested_by",
        "Requested by :name",
    ),
    (
        "wl_kernel.approvals.timeline.decision",
        ":decision at step :step by :name",
    ),
    ("wl_kernel.approvals.toast.approved", "Request approved."),
    (
        "wl_kernel.approvals.toast.step_approved",
        "Step approved. The request now waits for the next step.",
    ),
    (
        "wl_kernel.approvals.toast.failed",
        "The request failed: the action could no longer run.",
    ),
    ("wl_kernel.approvals.toast.rejected", "Request rejected."),
    ("wl_kernel.approvals.toast.withdrawn", "Request withdrawn."),
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
