//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    // The project machine.
    (
        "construction.project.budget_needed",
        "Set the budget before the project starts.",
    ),
    ("construction.project.hold_history", "Project put on hold"),
    // Terminating a project (`CON.ACL.15`).
    (
        "construction.terminate.reason_required",
        "Say why the project stops: terminated by the client, our default, main contractor insolvent, or agreed.",
    ),
    (
        "construction.terminate.notice_required",
        "Attach the termination notice.",
    ),
    (
        "construction.terminate.date_required",
        "Give the effective date of the termination, the last working day.",
    ),
    (
        "construction.terminate.history_terminated_by_client",
        "Terminated by the client",
    ),
    (
        "construction.terminate.history_our_default",
        "Terminated for our default",
    ),
    (
        "construction.terminate.history_main_contractor_insolvent",
        "Main contractor insolvent",
    ),
    (
        "construction.terminate.history_agreed",
        "Stopped by agreement",
    ),
    (
        "construction.terminate.attendance_note",
        "Recorded after the last working day of the terminated project, {{ date }}.",
    ),
    (
        "construction.terminate.requests_reason",
        "project terminated",
    ),
    (
        "construction.terminate.count_reason",
        "Project terminated: count the site store for the final account.",
    ),
    (
        "construction.terminate.tickets_reason",
        "project terminated",
    ),
    (
        "construction.terminate.reassign_title",
        "Reassign the crews of {{ project }}",
    ),
    (
        "construction.terminate.reassign_body",
        "The project ends on {{ date }}. The crews work until then; plan their next projects and review any attendance recorded after that day.",
    ),
    (
        "construction.terminate.orders_title",
        "Cancel or short-close the open orders of {{ project }}",
    ),
    (
        "construction.terminate.orders_body",
        "The project was terminated. Tell each supplier and cancel the order (nothing received) or short-close it: {{ orders }}.",
    ),
    (
        "construction.terminate.guarantee_title",
        "Follow up the {{ kind }} guarantee of {{ project }}",
    ),
    (
        "construction.terminate.guarantee_advance",
        "Reduce it to the unrecovered advance.",
    ),
    (
        "construction.terminate.guarantee_performance",
        "Release it, or respond to a call.",
    ),
    (
        "construction.terminate.guarantee_retention_bond",
        "Release the retention bond.",
    ),
    (
        "construction.terminate.subject",
        "Project {{ project }} has been terminated",
    ),
    (
        "construction.terminate.body",
        "Project {{ project }} stops on {{ date }}. That is the last working day on site.",
    ),
    // Settling the final account of a terminated project.
    (
        "construction.settle.no_final_account",
        "The project has no final account IPA.",
    ),
    (
        "construction.settle.not_certified",
        "The final account is not certified yet: record the client's certificate, or the decision of the court or the arbitrator.",
    ),
    (
        "construction.settle.not_invoiced",
        "The final account is not invoiced yet: invoice it, or write its balance off.",
    ),
    (
        "construction.settle.warn_retention",
        "Retention parts are still open.",
    ),
    (
        "construction.settle.warn_guarantees",
        "Guarantees are still open.",
    ),
    (
        "construction.settle.warn_stock_count",
        "A stock count of the site store is still open.",
    ),
    // Cancelling a project before work.
    (
        "construction.cancel.has_attendance",
        "Attendance was recorded on the project: it ends through termination.",
    ),
    (
        "construction.cancel.has_receipt",
        "A goods receipt was recorded on the project: it ends through termination.",
    ),
    (
        "construction.cancel.has_ipa",
        "An IPA exists for the project: it ends through termination.",
    ),
    (
        "construction.cancel.reason_needed",
        "Give the reason for cancelling the project.",
    ),
    (
        "construction.cancel.orders_title",
        "Cancel the open orders of {{ project }}",
    ),
    (
        "construction.cancel.orders_body",
        "The project was cancelled before work started. Tell each supplier and cancel the order: {{ orders }}.",
    ),
    // Starting a project from an award (BL-CON-03).
    (
        "construction.award.document_needed",
        "Attach the award document (the LPO or the subcontract).",
    ),
    (
        "construction.award.code_and_name_needed",
        "Give the code and the name of the project.",
    ),
    (
        "construction.award.code_taken",
        "A project with the code {{ code }} exists.",
    ),
    ("construction.award.history", "Started from the award"),
    // The pages.
    (
        "construction.award.done",
        "Project started from the award.",
    ),
    (
        "construction.terminate.requested",
        "Termination sent to the MD for approval.",
    ),
    ("construction.terminate.done", "Project terminated."),
    ("construction.settle.done", "Final account settled."),
    ("construction.cancel.done", "Project cancelled."),
    ("construction.bad_input", "Check the fields and try again."),
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
pub(crate) fn english(key: &str, params: &[(&str, &dyn Display)]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("construction.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_text_is_filled_and_an_unknown_key_is_itself() {
        let text = english(
            "construction.terminate.reassign_title",
            &[("project", &"Tower")],
        );
        assert_eq!(text, "Reassign the crews of Tower");
        assert_eq!(english("construction.nothing", &[]), "construction.nothing");
    }
}
