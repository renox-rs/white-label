//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("clinics.button_cancel", "Cancel"),
    ("clinics.none", "—"),
    ("clinics.fields.status", "Status"),
    // The appointments.
    ("clinics.appointments.title", "Appointment"),
    ("clinics.appointments.patient", "Patient"),
    ("clinics.appointments.service", "Service"),
    ("clinics.appointments.visit_date", "Visit date"),
    (
        "clinics.appointments.problem.cancelled",
        "This appointment is cancelled.",
    ),
    (
        "clinics.appointments.problem.checked_in",
        "The patient is already checked in; the cover can no longer change here.",
    ),
    (
        "clinics.appointments.problem.other_patient",
        "This policy belongs to another patient.",
    ),
    ("clinics.appointments.cancelled", "This appointment is cancelled."),
    // The coverage outcome at check-in (CLN.ACL.15).
    ("clinics.history.system", "System"),
    ("clinics.appointments.list_title", "Appointments"),
    (
        "clinics.appointments.list_subtitle",
        "From today, soonest first, with where the cover of each insured visit stands.",
    ),
    ("clinics.appointments.when", "When"),
    ("clinics.appointments.cover", "Cover"),
    ("clinics.appointments.empty", "No appointments from today"),
    ("clinics.coverage.waiting", "Waiting for the coverage outcome"),
    ("clinics.coverage.not_insured", "Not insured"),
    ("clinics.coverage.policy", "Insurance policy"),
    ("clinics.coverage.attach_policy", "Book with a policy"),
    (
        "clinics.coverage.attach_policy_description",
        "An appointment booked with an insurance policy is an insured visit: it is checked in only with its coverage outcome.",
    ),
    ("clinics.coverage.policy_attached", "Policy attached."),
    ("clinics.coverage.eligibility", "Eligibility result"),
    ("clinics.coverage.eligibility_positive", "Positive"),
    ("clinics.coverage.eligibility_negative", "Negative"),
    ("clinics.coverage.eligibility_missing", "Not recorded"),
    ("clinics.coverage.record_eligibility", "Record eligibility"),
    (
        "clinics.coverage.record_eligibility_description",
        "What the payer answered, through its portal or by phone, about this policy today. For a re-split, also record the other policy.",
    ),
    ("clinics.coverage.eligibility_recorded", "Eligibility recorded."),
    ("clinics.coverage.result", "Result"),
    ("clinics.coverage.reference", "Payer reference"),
    ("clinics.coverage.note", "Note"),
    ("clinics.coverage.outcome", "Coverage outcome"),
    ("clinics.coverage.outcome_missing", "Not set"),
    ("clinics.coverage.outcome_none", "Not an insured visit"),
    ("clinics.coverage.outcome.insured", "Insured"),
    ("clinics.coverage.outcome.re_split", "Re-split to another policy"),
    ("clinics.coverage.outcome.self_pay", "Self-pay"),
    ("clinics.coverage.record_outcome", "Set the coverage outcome"),
    (
        "clinics.coverage.record_outcome_description",
        "Insured, re-split to another valid policy, or self-pay with the patient's signed consent. Otherwise reschedule or cancel with the reason coverage not confirmed.",
    ),
    ("clinics.coverage.outcome_recorded", "Coverage outcome recorded."),
    ("clinics.coverage.resplit_policy", "Policy for a re-split"),
    ("clinics.coverage.split_policy", "Policy of the split"),
    ("clinics.coverage.check_in", "Check in"),
    (
        "clinics.coverage.check_in_message",
        "The patient is checked in and the visit is opened.",
    ),
    ("clinics.coverage.checked_in", "Checked in"),
    ("clinics.coverage.open_visit", "Open the visit"),
    ("clinics.coverage.change_to_insured", "Change to insured"),
    (
        "clinics.coverage.change_to_insured_description",
        "The payer now confirms the cover. The signed self-pay consent stays on file, marked superseded. After the service starts the outcome is fixed.",
    ),
    ("clinics.coverage.changed_to_insured", "The visit is insured."),
    (
        "clinics.coverage.problem.checked_in",
        "The patient is already checked in; the coverage outcome can no longer be recorded here.",
    ),
    (
        "clinics.coverage.problem.not_insured",
        "This appointment was not booked with an insurance policy.",
    ),
    (
        "clinics.coverage.problem.outcome_required",
        "Coverage outcome required: record the eligibility result and one coverage outcome before checking the patient in.",
    ),
    (
        "clinics.coverage.problem.insured",
        "Insured needs a positive eligibility result today and a policy and card that are valid on the visit day.",
    ),
    (
        "clinics.coverage.problem.re_split",
        "The policy must be the patient's own, valid on the visit day, with a positive eligibility result recorded today.",
    ),
    (
        "clinics.coverage.problem.fixed",
        "After the service starts, the coverage outcome is fixed.",
    ),
    ("clinics.coverage.problem.already", "The visit is already insured."),
    // The self-pay consent.
    ("clinics.consent.title", "Self-pay consent"),
    ("clinics.consent.estimate", "Estimated price"),
    ("clinics.consent.sign", "Self-pay consent"),
    (
        "clinics.consent.sign_description",
        "The patient signs on the tablet, seeing the services and their estimated self-pay price. The bill uses the price list on the visit day.",
    ),
    ("clinics.consent.signer", "Patient's name (signature)"),
    (
        "clinics.consent.more_services",
        "More service codes, comma separated",
    ),
    ("clinics.consent.signed", "Consent signed; the visit is self-pay."),
    ("clinics.consent.superseded", "superseded"),
    (
        "clinics.consent.problem.no_price",
        "There is no self-pay price for :service on the visit day; the clinic manager sets one in the self-pay price list.",
    ),
    (
        "clinics.consent.problem.required",
        "A signed self-pay consent showing the estimated price is required.",
    ),
    (
        "clinics.consent.problem.signer",
        "The patient's name is required to sign.",
    ),
    (
        "clinics.consent.problem.not_on_consent",
        ":service is not on the self-pay consent; ask the patient to sign an updated consent first.",
    ),
    (
        "clinics.consent.problem.not_self_pay",
        "Only a self-pay visit in progress takes an updated consent.",
    ),
    // The visit.
    ("clinics.visits.details", "Visit"),
    ("clinics.visits.history", "History"),
    ("clinics.visits.history_at", "When"),
    ("clinics.visits.history_action", "What"),
    ("clinics.visits.history_by", "By"),
    ("clinics.visits.history_empty", "No history yet"),
    ("clinics.visits.status.checked_in", "Checked in"),
    ("clinics.visits.status.in_progress", "In progress"),
    ("clinics.visits.status.completed", "Completed"),
    ("clinics.visits.status.cancelled", "Cancelled"),
    ("clinics.visits.transition.check_in", "Checked in"),
    ("clinics.visits.transition.start", "Started"),
    ("clinics.visits.transition.complete", "Completed"),
    ("clinics.visits.transition.cancel", "Cancelled"),
    // The licence expiry (CLN.RUL.01).
    (
        "clinics.licences.subject",
        "{{ kind }} expired: {{ practitioner }}",
    ),
    (
        "clinics.licences.body",
        "The {{ kind }} of {{ practitioner }} expired on {{ expires_on }}. New bookings for them at {{ branches }} are refused and cannot be overridden. Appointments affected: {{ count }}; they are in conflict for the front desk to reassign or reschedule. Nothing was cancelled.",
    ),
    // The insurance policies.
    ("clinics.policies.title", "Insurance policies"),
    (
        "clinics.policies.subtitle",
        "The patients' cover with their payers.",
    ),
    ("clinics.policies.new", "New policy"),
    (
        "clinics.policies.new_description",
        "The patient and the payer (parties), the plan, the member id and the dates on the card.",
    ),
    ("clinics.policies.patient_party", "Patient (party number)"),
    ("clinics.policies.payer_party", "Payer (party number)"),
    ("clinics.policies.patient", "Patient"),
    ("clinics.policies.payer", "Payer"),
    ("clinics.policies.plan", "Plan"),
    ("clinics.policies.member_id", "Member id"),
    ("clinics.policies.valid_from", "Valid from"),
    ("clinics.policies.valid_to", "Valid to"),
    ("clinics.policies.card_valid_to", "Card valid to"),
    ("clinics.policies.valid_today", "Valid"),
    ("clinics.policies.expired", "Expired"),
    ("clinics.policies.empty", "No insurance policies yet"),
    ("clinics.policies.created", "Policy recorded."),
    ("clinics.policies.card_updated", "Card updated."),
    ("clinics.policies.problem.plan", "The plan is required."),
    (
        "clinics.policies.problem.member_id",
        "The member id is required.",
    ),
    (
        "clinics.policies.problem.dates",
        "The policy cannot end before it starts.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the
/// English one.
#[allow(dead_code)]
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], replacing `:name` placeholders with `params`.
#[allow(dead_code)]
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    english(key, params)
}

/// Every text of the app in `lang`, keyed without the `clinics.` prefix
/// (`appointments.title`), for the templates: `tx['appointments.title']`.
#[allow(dead_code)]
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("clinics."), t(lang, key)))
        .collect()
}

/// The English text for `key` with `params` filled in; the key itself when
/// it has no text.
#[allow(dead_code)]
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
    use super::{TEXTS, english};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("clinics.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(english("clinics.none", &[]), "—");
        assert_eq!(english("clinics.unknown", &[]), "clinics.unknown");
    }
}
