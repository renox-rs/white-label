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
