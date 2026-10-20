//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("accounting.button_cancel", "Cancel"),
    ("accounting.none", "—"),
    ("accounting.fields.status", "Status"),
    // The engagements.
    ("accounting.engagements.title", "Engagements"),
    (
        "accounting.engagements.subtitle",
        "The firm's clients and the services it runs for them.",
    ),
    ("accounting.engagements.new", "New engagement"),
    (
        "accounting.engagements.new_description",
        "The client company (a party), the services and the fee. It starts in onboarding.",
    ),
    ("accounting.engagements.client_party", "Client (party number)"),
    ("accounting.engagements.client", "Client"),
    ("accounting.engagements.services", "Services"),
    ("accounting.engagements.fee_basis", "Fee basis"),
    ("accounting.engagements.fee_basis.retainer", "Retainer"),
    ("accounting.engagements.fee_basis.fixed", "Fixed fee"),
    ("accounting.engagements.fee_basis.time", "Time and materials"),
    ("accounting.engagements.retainer_fee", "Monthly retainer (AED)"),
    ("accounting.engagements.number", "Engagement"),
    ("accounting.engagements.reason", "Reason"),
    ("accounting.engagements.empty", "No engagements yet"),
    ("accounting.engagements.created", "Engagement created."),
    ("accounting.engagements.details", "Engagement"),
    ("accounting.engagements.start", "Start"),
    ("accounting.engagements.letter", "Engagement letter"),
    ("accounting.engagements.letter_signed", "Signed"),
    ("accounting.engagements.letter_signed_on", "Signed on :date"),
    ("accounting.engagements.letter_not_signed", "Not signed"),
    ("accounting.engagements.record_letter", "Record signed letter"),
    (
        "accounting.engagements.record_letter_description",
        "The day the client signed the engagement letter.",
    ),
    ("accounting.engagements.signed_on", "Signed on"),
    ("accounting.engagements.letter_recorded", "Signed letter recorded."),
    ("accounting.engagements.tax_profile", "Tax profile"),
    ("accounting.engagements.tax_profile_none", "None recorded"),
    ("accounting.engagements.tax_profile_complete", "Complete"),
    ("accounting.engagements.activated_on", "Activated on"),
    ("accounting.engagements.ended_on", "Ended on"),
    ("accounting.engagements.cdd", "CDD file"),
    ("accounting.engagements.open_cdd", "Open the CDD file"),
    ("accounting.engagements.activate", "Activate"),
    (
        "accounting.engagements.activate_message",
        "The compliance calendar is generated and the retainer starts.",
    ),
    ("accounting.engagements.activated", "Engagement activated."),
    ("accounting.engagements.decline", "Decline"),
    ("accounting.engagements.declined", "Engagement declined."),
    ("accounting.engagements.suspend", "Suspend"),
    ("accounting.engagements.suspended", "Engagement suspended."),
    ("accounting.engagements.resume", "Resume"),
    (
        "accounting.engagements.resume_message",
        "The engagement runs again.",
    ),
    ("accounting.engagements.resumed", "Engagement resumed."),
    ("accounting.engagements.end", "End"),
    ("accounting.engagements.ended", "Engagement ended."),
    (
        "accounting.engagements.reason_description",
        "Choose the reason. It is shown to staff as it reads here.",
    ),
    ("accounting.engagements.status.onboarding", "Onboarding"),
    ("accounting.engagements.status.active", "Active"),
    ("accounting.engagements.status.suspended", "Suspended"),
    ("accounting.engagements.status.ended", "Ended"),
    ("accounting.engagements.status.declined", "Declined"),
    ("accounting.engagements.transition.activate", "Activated"),
    ("accounting.engagements.transition.decline", "Declined"),
    ("accounting.engagements.transition.auto_decline", "Declined"),
    ("accounting.engagements.transition.suspend", "Suspended"),
    ("accounting.engagements.transition.auto_suspend", "Suspended"),
    ("accounting.engagements.transition.resume", "Resumed"),
    ("accounting.engagements.transition.end", "Ended"),
    // The history.
    ("accounting.history.title", "History"),
    ("accounting.history.at", "When"),
    ("accounting.history.action", "What"),
    ("accounting.history.by", "By"),
    ("accounting.history.system", "System"),
    ("accounting.history.empty", "Nothing happened yet"),
    // The neutral reasons (`ACC.ACL.14`).
    ("accounting.reasons.on_hold", "On hold"),
    ("accounting.reasons.relationship_ended", "Ended"),
    ("accounting.reasons.non_payment", "Non-payment"),
    ("accounting.reasons.client_request", "Client request"),
    ("accounting.reasons.other", "Other"),
    ("accounting.reasons.letter_not_signed", "Letter not signed"),
    ("accounting.reasons.client_withdrew", "Client withdrew"),
    ("accounting.reasons.not_accepted", "Not accepted"),
    // Activation.
    (
        "accounting.activation.refused",
        "This engagement cannot be activated:",
    ),
    (
        "accounting.activation.cdd_not_approved",
        "The CDD file is not approved: the MLRO approves it first",
    ),
    (
        "accounting.activation.letter_not_signed",
        "The engagement letter is not signed: record the signed letter",
    ),
    (
        "accounting.activation.no_tax_profile",
        "No tax profile: record the client's tax profile",
    ),
    (
        "accounting.activation.tax_profile_incomplete",
        "The tax profile is incomplete: :missing",
    ),
    // The tax profile.
    ("accounting.tax_profile.financial_year_end", "financial year end"),
    (
        "accounting.tax_profile.ct_registration_no",
        "corporate tax registration number",
    ),
    ("accounting.tax_profile.trn", "VAT TRN (15 digits)"),
    (
        "accounting.tax_profile.vat_period",
        "VAT period and its first period end",
    ),
    // The calendar and the retainer.
    (
        "accounting.calendar.vat_return",
        "VAT return, period ending :end",
    ),
    (
        "accounting.calendar.ct_return",
        "Corporate tax return, year ending :end",
    ),
    ("accounting.retainer.plan", ":title retainer"),
    // Problems.
    (
        "accounting.problems.reason_unknown",
        "Choose one of the reasons offered.",
    ),
    (
        "accounting.problems.reason_required",
        "Give the reason.",
    ),
    (
        "accounting.problems.client_on_hold",
        "The client is on hold.",
    ),
    ("accounting.problems.title_required", "Name the services."),
    (
        "accounting.problems.fee_negative",
        "The retainer cannot be negative.",
    ),
    (
        "accounting.problems.client_unknown",
        "The client must be a known party.",
    ),
    (
        "accounting.problems.not_onboarding",
        "Only an engagement in onboarding takes this.",
    ),
    (
        "accounting.problems.cdd_locked",
        "The CDD file is no longer in progress.",
    ),
    // The CDD file.
    ("accounting.cdd.title", "CDD file"),
    ("accounting.cdd.status.in_progress", "In progress"),
    ("accounting.cdd.status.pending_approval", "Pending approval"),
    ("accounting.cdd.status.approved", "Approved"),
    ("accounting.cdd.status.review_due", "Review due"),
    ("accounting.cdd.status.rejected", "Rejected"),
    ("accounting.cdd.status.exited", "Exited"),
    ("accounting.cdd.role.client", "Client company"),
    ("accounting.cdd.role.ubo", "UBO"),
    ("accounting.cdd.role.director", "Director"),
    ("accounting.cdd.role.signatory", "Signatory"),
    ("accounting.cdd.rating", "Risk rating"),
    ("accounting.cdd.rating.unrated", "Not rated"),
    ("accounting.cdd.rating.low", "Low"),
    ("accounting.cdd.rating.medium", "Medium"),
    ("accounting.cdd.rating.high", "High"),
    ("accounting.cdd.risk_factors", "Risk factors"),
    ("accounting.cdd.edd_notes", "EDD notes"),
    ("accounting.cdd.documents", "Identity documents"),
    ("accounting.cdd.documents_attached", "Attached"),
    ("accounting.cdd.documents_missing", "Not attached"),
    ("accounting.cdd.next_review", "Next review"),
    ("accounting.cdd.parties", "Screened parties"),
    ("accounting.cdd.parties_empty", "No party yet"),
    ("accounting.cdd.party", "Party"),
    ("accounting.cdd.party_role", "Role"),
    ("accounting.cdd.add_party", "Add party"),
    (
        "accounting.cdd.add_party_description",
        "A UBO, director or signatory of the client (a party number).",
    ),
    ("accounting.cdd.assess", "Assess"),
    (
        "accounting.cdd.assess_description",
        "Rate the risk and note the factors.",
    ),
    ("accounting.cdd.submit", "Submit for approval"),
    (
        "accounting.cdd.submit_message",
        "The MLRO is asked to approve the file.",
    ),
    ("accounting.cdd.approve", "Approve"),
    (
        "accounting.cdd.approve_message",
        "The client may be served; the next review is set from the rating.",
    ),
    ("accounting.cdd.reject", "Reject"),
    ("accounting.cdd.reopen", "Reopen"),
    (
        "accounting.cdd.reopen_message",
        "The file goes back to in progress.",
    ),
    ("accounting.cdd.exit", "Exit client"),
    ("accounting.cdd.reason", "Reason (MLRO only)"),
    (
        "accounting.cdd.reason_description",
        "Only the MLRO and deputy see this reason.",
    ),
    ("accounting.cdd.opened", "CDD file opened."),
    ("accounting.cdd.party_added", "Party added."),
    ("accounting.cdd.assessed", "Assessment recorded."),
    ("accounting.cdd.submitted", "CDD file submitted."),
    ("accounting.cdd.approved", "CDD file approved."),
    ("accounting.cdd.rejected", "CDD file rejected."),
    ("accounting.cdd.reopened", "CDD file reopened."),
    ("accounting.cdd.exited", "Client exited."),
    (
        "accounting.cdd.submit_refused",
        "This CDD file cannot be submitted:",
    ),
    (
        "accounting.cdd.no_ubos",
        "No UBO is identified: add the client's UBOs",
    ),
    (
        "accounting.cdd.no_documents",
        "The identity documents are not attached",
    ),
    ("accounting.cdd.not_rated", "The risk is not rated"),
];

/// Whether the app has a text for `key`.
pub(crate) fn has(key: &str) -> bool {
    TEXTS.iter().any(|(k, _)| *k == key)
}

/// The text for `key`: the app's translation when it has one, else the
/// English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], replacing `:name` placeholders with `params`.
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    english(key, params)
}

/// Every text of the app in `lang`, keyed without the `accounting.` prefix
/// (`engagements.title`), for the templates: `tx['engagements.title']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("accounting."), t(lang, key)))
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
    use super::{TEXTS, english, has};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("accounting.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(
            english("accounting.calendar.vat_return", &[("end", &"2026-12-31")]),
            "VAT return, period ending 2026-12-31"
        );
        assert!(has("accounting.reasons.on_hold"));
        assert!(!has("accounting.reasons.sanctions"));
    }
}
