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
    // The IPA: submitting, replacing, certifying (BL-CON-04).
    (
        "construction.ipa.progress_needed",
        "Record the progress for the period before submitting the IPA.",
    ),
    (
        "construction.ipa.replacement_needed",
        "IPA {{ number }} is still waiting for its certificate: submit this one as its replacement.",
    ),
    (
        "construction.ipa.replace_latest_only",
        "Only the most recent IPA waiting for its certificate can be replaced.",
    ),
    (
        "construction.ipa.nothing_to_replace",
        "The IPA to replace is not waiting for its certificate.",
    ),
    (
        "construction.ipa.superseded_subject",
        "Payment application {{ old }} of {{ project }} is replaced",
    ),
    (
        "construction.ipa.superseded_body",
        "Payment application {{ old }} of {{ project }} is replaced by payment application {{ new }}. Please certify the new one.",
    ),
    (
        "construction.ipa.certified_amount_needed",
        "Give the certified amount, the work before the client's deductions.",
    ),
    (
        "construction.ipa.certificate_needed",
        "Attach the client's certificate.",
    ),
    (
        "construction.ipa.cannot_certify",
        "IPA {{ number }} cannot take a certificate in its state.",
    ),
    (
        "construction.ipa.late_certificate_title",
        "Late certificate attached to IPA {{ number }} of {{ project }}",
    ),
    (
        "construction.ipa.late_certificate_body",
        "A certificate for a replaced IPA arrived after the replacing IPA was certified. It is attached for reference; no amount changed.",
    ),
    (
        "construction.ipa.cutoff_title",
        "Draft IPA {{ number }} of {{ project }} is ready",
    ),
    (
        "construction.ipa.cutoff_body",
        "The monthly cut-off made draft IPA {{ number }}. Record the progress.",
    ),
    (
        "construction.ipa.cutoff_pending_title",
        "Draft IPA {{ number }} of {{ project }} waits for an earlier certificate",
    ),
    (
        "construction.ipa.cutoff_pending_body",
        "IPA {{ pending }} is still waiting for its certificate. Draft IPA {{ number }} is marked previous pending and can be submitted only as its replacement.",
    ),
    ("construction.ipa.submitted", "IPA submitted."),
    ("construction.ipa.certified", "Certificate recorded."),
    (
        "construction.ipa.certificate_attached",
        "The certificate was attached to the replacing IPA for reference.",
    ),
    // The pages.
    ("construction.award.done", "Project started from the award."),
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
