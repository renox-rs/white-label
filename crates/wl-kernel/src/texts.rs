//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_kernel.legal_entities", "Legal entities"),
    ("wl_kernel.block_guard.on_hold", "This party is on hold"),
    ("wl_kernel.block_guard.blocked", "Blocked: :reason"),
    (
        "wl_kernel.block_guard.not_blocked",
        "This party is not on hold, so no exception is needed.",
    ),
    (
        "wl_kernel.block_guard.request_exception",
        "Request exception (approved by :approver)",
    ),
    (
        "wl_kernel.screening_guard.refused",
        "Screening list is out of date: :lists",
    ),
    (
        "wl_kernel.screening_guard.notice",
        "screened on a stale list (:lists)",
    ),
    ("wl_kernel.screening_guard.list", ":name of :date"),
    (
        "wl_kernel.screening_guard.never_updated",
        ":name (no update yet)",
    ),
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
    ("wl_kernel.approvals.card.earlier", "Earlier decisions"),
    (
        "wl_kernel.approvals.card.earlier_rejected",
        "rejected on :date by :name",
    ),
    (
        "wl_kernel.approvals.card.earlier_returned",
        "returned on :date by :name",
    ),
    ("wl_kernel.approvals.card.return", "Return for changes"),
    (
        "wl_kernel.approvals.card.return_title",
        "Return this request for changes",
    ),
    (
        "wl_kernel.approvals.card.comment_return",
        "Required. Say what the requester must fix; they get a task to fix and resubmit.",
    ),
    (
        "wl_kernel.approvals.card.task_open",
        "Returned for changes: waiting for the requester to fix and resubmit.",
    ),
    (
        "wl_kernel.approvals.card.not_resubmitting",
        "Not resubmitting",
    ),
    (
        "wl_kernel.approvals.card.not_resubmitting_title",
        "Not resubmitting this request?",
    ),
    (
        "wl_kernel.approvals.card.not_resubmitting_message",
        "The task to fix and resubmit closes. You can still submit the action again later.",
    ),
    (
        "wl_kernel.approvals.needs_action.item",
        "Fix and resubmit: :action",
    ),
    ("wl_kernel.approvals.awaiting.item", "Decide: :action"),
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
    (
        "wl_kernel.approvals.toast.returned",
        "Request returned for changes.",
    ),
    (
        "wl_kernel.approvals.toast.not_resubmitting",
        "Task closed: not resubmitting.",
    ),
    (
        "wl_kernel.voids.not_voidable",
        "This record cannot be voided.",
    ),
    (
        "wl_kernel.voids.no_permission",
        "You may not void this record.",
    ),
    (
        "wl_kernel.voids.given_to_third_party",
        "This record was given to a third party, so it cannot be voided.",
    ),
    (
        "wl_kernel.voids.locked_period",
        "The accounting period is locked, so this record cannot be voided.",
    ),
    (
        "wl_kernel.voids.reverse_failed",
        "What this record did could not be reversed.",
    ),
    ("wl_kernel.voids.more", "More"),
    ("wl_kernel.voids.action", "Void"),
    ("wl_kernel.voids.title", "Void this record"),
    ("wl_kernel.voids.reason", "Reason"),
    (
        "wl_kernel.voids.reason_hint",
        "Why was it entered in error? The record keeps its number and is struck through.",
    ),
    ("wl_kernel.voids.reason_required", "Give a reason."),
    ("wl_kernel.voids.toast.voided", "Voided"),
    (
        "wl_kernel.voids.cannot_change",
        "A voided record cannot be changed.",
    ),
    (
        "wl_kernel.late_entries.not_opted_in",
        "This kind of record does not take late entries.",
    ),
    (
        "wl_kernel.late_entries.no_approval_policy",
        "No approval policy is active for late entries.",
    ),
    (
        "wl_kernel.late_entries.reason_required",
        "Give the reason this entry is late.",
    ),
    ("wl_kernel.late_entries.title", "Late entry"),
    ("wl_kernel.late_entries.late_by", "Entered :minutes minutes late"),
    (
        "wl_kernel.late_entries.notice",
        "This work happened :minutes minutes ago. Give the reason it is entered late.",
    ),
    (
        "wl_kernel.late_entries.notice_approval",
        "This work happened :minutes minutes ago, or in a closed period. A manager must approve it before it is saved.",
    ),
    ("wl_kernel.late_entries.reason", "Reason it is late"),
    (
        "wl_kernel.late_entries.reason_hint",
        "For example: router down, power cut, paper sheet from the site.",
    ),
    ("wl_kernel.late_entries.outage", "Outage reference"),
    (
        "wl_kernel.late_entries.outage_hint",
        "Optional: the connection that was down, or a note such as outage at branch 2 from 08:30 to 10:00.",
    ),
    ("wl_kernel.late_entries.occurred_at", "Business time"),
    ("wl_kernel.late_entries.entered_at", "Entered at"),
    ("wl_kernel.late_entries.approval", "Approval request"),
    ("wl_kernel.late_entries.review", "Already fired, review"),
    (
        "wl_kernel.late_entries.fired_run",
        ":rule on :subject at :at",
    ),
    ("wl_kernel.late_entries.status.requested", "Requested"),
    (
        "wl_kernel.late_entries.status.awaiting_approval",
        "Waiting for approval",
    ),
    ("wl_kernel.late_entries.status.recorded", "Recorded"),
    ("wl_kernel.late_entries.status.discarded", "Not recorded"),
    (
        "wl_kernel.integrations.unknown",
        "This connection is not registered.",
    ),
    (
        "wl_kernel.integrations.alert.subject",
        "Connection down: {{ connection }}",
    ),
    (
        "wl_kernel.integrations.alert.body",
        "{{ connection }} has had no successful call since {{ since }} (Asia/Dubai). Last error: {{ error }}. Manual fallback: {{ fallback }}",
    ),
    (
        "wl_kernel.integrations.banner",
        "The connection :connection is down. :fallback",
    ),
    (
        "wl_kernel.integrations.reason_required",
        "Give a reason.",
    ),
    ("wl_kernel.integrations.toast.disabled", "Connection disabled."),
    ("wl_kernel.integrations.toast.enabled", "Connection enabled."),
    ("wl_kernel.integrations.status.up", "Up"),
    ("wl_kernel.integrations.status.degraded", "Degraded"),
    ("wl_kernel.integrations.status.down", "Down"),
    ("wl_kernel.integrations.status.disabled", "Disabled"),
    ("wl_kernel.integrations.page.title", "Integrations"),
    (
        "wl_kernel.integrations.page.subtitle",
        "The external systems the modules call, and the calls waiting for them.",
    ),
    ("wl_kernel.integrations.page.key", "Connection"),
    ("wl_kernel.integrations.page.module", "Module"),
    ("wl_kernel.integrations.page.status", "Status"),
    ("wl_kernel.integrations.page.failed_since", "Failing since"),
    ("wl_kernel.integrations.page.last_success", "Last success"),
    ("wl_kernel.integrations.page.queued", "Queued"),
    ("wl_kernel.integrations.page.empty", "No connections are registered."),
    ("wl_kernel.integrations.page.disable", "Disable"),
    ("wl_kernel.integrations.page.disable_title", "Disable this connection"),
    ("wl_kernel.integrations.page.reason", "Reason"),
    (
        "wl_kernel.integrations.page.reason_hint",
        "Why is it switched off? Nothing is sent while it is disabled; the queue is kept.",
    ),
    ("wl_kernel.integrations.page.enable", "Enable"),
    ("wl_kernel.integrations.page.enable_title", "Enable this connection"),
    (
        "wl_kernel.integrations.page.enable_message",
        "The queued calls are sent in order.",
    ),
    ("wl_kernel.integrations.page.cancel", "Cancel"),
    ("wl_kernel.integrations.page.owner_role", "Owner role"),
    ("wl_kernel.integrations.page.down_after", "Down after"),
    ("wl_kernel.integrations.page.minutes", ":minutes minutes"),
    ("wl_kernel.integrations.page.last_error", "Last error"),
    ("wl_kernel.integrations.page.disabled_reason", "Disabled because"),
    ("wl_kernel.integrations.page.fallback", "Manual fallback"),
    ("wl_kernel.integrations.page.screens", "Screens affected"),
    ("wl_kernel.integrations.page.queue", "Queued calls"),
    ("wl_kernel.integrations.page.call", "Call"),
    ("wl_kernel.integrations.page.subject", "About"),
    ("wl_kernel.integrations.page.enqueued_at", "Queued at"),
    ("wl_kernel.integrations.page.attempts", "Failed attempts"),
    ("wl_kernel.integrations.page.next_attempt", "Next attempt"),
    ("wl_kernel.integrations.page.queue_empty", "Nothing is queued."),
    (
        "wl_kernel.integrations.reconcile.title",
        "Reconcile :connection: manual work against the late sync",
    ),
    (
        "wl_kernel.integrations.reconcile.obsolete",
        "Not sent: the record changed while the call was queued.",
    ),
    (
        "wl_kernel.integrations.no_inbound",
        "This connection takes no inbound data.",
    ),
    (
        "wl_kernel.compliance.alert.subject",
        "Compliance alert: {{ rule }}",
    ),
    (
        "wl_kernel.compliance.alert.body",
        "The compliance-critical rule {{ rule }} {{ what }} (Asia/Dubai, {{ at }}). Error: {{ error }}. A task is open for the owner until the duty is met.",
    ),
    (
        "wl_kernel.compliance.what.failed",
        "failed after its last attempt",
    ),
    (
        "wl_kernel.compliance.what.stopped",
        "was stopped because its rule chain is too deep",
    ),
    (
        "wl_kernel.compliance.what.missed",
        "did not run on schedule (planned :planned)",
    ),
    (
        "wl_kernel.compliance.task_title",
        "Compliance rule :rule :what",
    ),
    (
        "wl_kernel.compliance.task_description",
        "The rule :rule has not run for this occurrence. Run it again, or record how the duty was met by hand. Error: :error",
    ),
    (
        "wl_kernel.compliance.rerun_note",
        "Rule ran successfully at :time (Asia/Dubai).",
    ),
    (
        "wl_kernel.compliance.note_required",
        "Record how the duty was met.",
    ),
    (
        "wl_kernel.compliance.already_closed",
        "This alert is already closed.",
    ),
    ("wl_kernel.compliance.page.title", "Compliance alerts"),
    (
        "wl_kernel.compliance.page.subtitle",
        "Rules that protect a legal duty and did not run. Run them again, or record how the duty was met.",
    ),
    ("wl_kernel.compliance.page.rule", "Rule"),
    ("wl_kernel.compliance.page.what", "What happened"),
    ("wl_kernel.compliance.page.subject", "About"),
    ("wl_kernel.compliance.page.raised_at", "Raised"),
    ("wl_kernel.compliance.page.task", "Task"),
    ("wl_kernel.compliance.page.actions", "Actions"),
    ("wl_kernel.compliance.page.close", "Record how it was met"),
    (
        "wl_kernel.compliance.page.close_title",
        "How was the duty met?",
    ),
    ("wl_kernel.compliance.page.note", "Note"),
    (
        "wl_kernel.compliance.page.note_hint",
        "Required: what was done by hand, by whom and when.",
    ),
    ("wl_kernel.compliance.page.attachment", "Attachment"),
    (
        "wl_kernel.compliance.page.attachment_hint",
        "Optional: the number of an uploaded document that proves it.",
    ),
    ("wl_kernel.compliance.page.empty", "No open alerts."),
    ("wl_kernel.compliance.kind.failed", "Failed"),
    ("wl_kernel.compliance.kind.stopped", "Stopped"),
    ("wl_kernel.compliance.kind.missed", "Did not run on schedule"),
    (
        "wl_kernel.compliance.attachment_invalid",
        "Give the number of a document.",
    ),
    ("wl_kernel.compliance.toast.closed", "Alert closed."),
    (
        "wl_kernel.digest.heading",
        "Voids and late entries from :from to :until (Asia/Dubai)",
    ),
    ("wl_kernel.digest.voided", "Voided"),
    ("wl_kernel.digest.late", "Late entries"),
    (
        "wl_kernel.digest.void_line",
        "- :record: voided by :user, reason: :reason (:at)",
    ),
    (
        "wl_kernel.digest.late_line",
        "- :record: entered by :user, reason: :reason; business time :occurred, entered :entered",
    ),
    ("wl_kernel.digest.confidential_line", "- :record"),
    (
        "wl_kernel.digest.review_line",
        "  already fired, review: :rule, :record, :at",
    ),
    (
        "wl_kernel.integrations.not_yet_down",
        "The connection has not been without a successful call for long enough.",
    ),
    (
        "wl_kernel.late_entries.limits_invalid",
        "The flag limit must be above 0 minutes and not above the approval limit.",
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
