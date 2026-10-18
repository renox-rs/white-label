//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_parties.title", "Parties"),
    ("wl_parties.new", "New party"),
    ("wl_parties.create", "Create party"),
    ("wl_parties.cancel", "Cancel"),
    ("wl_parties.created", "Party created."),
    ("wl_parties.empty", "No parties yet"),
    (
        "wl_parties.empty_hint",
        "Parties show up here once they are created.",
    ),
    ("wl_parties.sections.card", "Party"),
    ("wl_parties.sections.names", "Names"),
    ("wl_parties.sections.contact", "Primary contacts"),
    ("wl_parties.sections.contacts", "Contact points"),
    ("wl_parties.sections.roles", "Roles"),
    ("wl_parties.sections.history", "History"),
    ("wl_parties.no_contacts", "No contact points."),
    ("wl_parties.no_roles", "No roles."),
    ("wl_parties.no_history", "No changes yet."),
    ("wl_parties.history.change", "Change"),
    ("wl_parties.history.at", "When"),
    ("wl_parties.history.reason", "Reason"),
    ("wl_parties.fields.code", "Code"),
    ("wl_parties.fields.kind", "Kind"),
    ("wl_parties.fields.display_name", "Display name"),
    (
        "wl_parties.fields.display_name_hint",
        "Shown in lists; the English legal name when left blank.",
    ),
    ("wl_parties.fields.legal_name_en", "Legal name (English)"),
    ("wl_parties.fields.legal_name_ar", "Legal name (Arabic)"),
    ("wl_parties.fields.status", "Status"),
    ("wl_parties.fields.hold_reason", "Hold reason"),
    ("wl_parties.fields.default_language", "Language"),
    ("wl_parties.fields.preferred_channel", "Preferred channel"),
    ("wl_parties.fields.email", "Email"),
    ("wl_parties.fields.mobile", "Mobile"),
    ("wl_parties.fields.trn", "TRN"),
    (
        "wl_parties.fields.trn_hint",
        "15 digits, for a VAT-registered company.",
    ),
    ("wl_parties.fields.legal_form", "Legal form"),
    ("wl_parties.fields.trade_licence_no", "Trade licence no."),
    ("wl_parties.fields.licence_authority", "Licensing authority"),
    ("wl_parties.fields.vat_registered", "VAT registered"),
    ("wl_parties.fields.nationality", "Nationality"),
    ("wl_parties.fields.date_of_birth", "Date of birth"),
    ("wl_parties.fields.created_at", "Created"),
    ("wl_parties.fields.contact_type", "Type"),
    ("wl_parties.fields.value", "Address or number"),
    ("wl_parties.fields.label", "Label"),
    ("wl_parties.fields.is_primary", "Primary"),
    ("wl_parties.fields.verified_at", "Verified"),
    ("wl_parties.fields.role", "Role"),
    ("wl_parties.fields.since", "Since"),
    ("wl_parties.fields.until", "Until"),
    ("wl_parties.fields.external_reference", "External reference"),
    ("wl_parties.kind.person", "Person"),
    ("wl_parties.kind.company", "Company"),
    ("wl_parties.status.active", "Active"),
    ("wl_parties.status.blocked", "On hold"),
    ("wl_parties.status.archived", "Archived"),
    ("wl_parties.status.merged", "Merged"),
    ("wl_parties.language.en", "English"),
    ("wl_parties.language.ar", "Arabic"),
    ("wl_parties.channel.email", "Email"),
    ("wl_parties.channel.sms", "SMS"),
    ("wl_parties.channel.whatsapp", "WhatsApp"),
    ("wl_parties.contact_type.email", "Email"),
    ("wl_parties.contact_type.mobile", "Mobile"),
    ("wl_parties.contact_type.landline", "Landline"),
    ("wl_parties.contact_type.address", "Address"),
    ("wl_parties.reason.on_hold", "On hold"),
    ("wl_parties.reason.non_payment", "Non-payment"),
    ("wl_parties.reason.legal_dispute", "Legal dispute"),
    ("wl_parties.reason.fraud_suspected", "Fraud suspected"),
    ("wl_parties.reason.other", "Other"),
    ("wl_parties.reason.sanctions", "Sanctions"),
    ("wl_parties.reason.aml_concern", "AML concern"),
    ("wl_parties.block.action", "Block"),
    ("wl_parties.block.title", "Put the party on hold"),
    (
        "wl_parties.block.description",
        "New business with the party stops. Nothing is sent to the party.",
    ),
    ("wl_parties.block.submit", "Block party"),
    ("wl_parties.block.done", "The party is on hold."),
    ("wl_parties.block.system", "System"),
    ("wl_parties.block.open", "Open"),
    ("wl_parties.block.closed", "Lifted"),
    ("wl_parties.sections.block_records", "Block records"),
    ("wl_parties.fields.reason_code", "Reason"),
    ("wl_parties.fields.note", "Note"),
    ("wl_parties.fields.restricted", "Restricted"),
    ("wl_parties.fields.blocked_by", "Blocked by"),
    ("wl_parties.fields.blocked_at", "Blocked"),
    ("wl_parties.fields.unblocked_at", "Lifted"),
    ("wl_parties.fields.block_state", "State"),
    ("wl_parties.unblock.action", "Unblock"),
    ("wl_parties.unblock.title", "Lift the hold"),
    (
        "wl_parties.unblock.description",
        "New business with the party can start again. Another authorised user must lift a hold.",
    ),
    ("wl_parties.unblock.submit", "Unblock party"),
    ("wl_parties.unblock.done", "The hold is lifted."),
    ("wl_parties.unblock_basis.delisted", "No longer listed"),
    ("wl_parties.unblock_basis.false_match", "Match found false"),
    ("wl_parties.fields.unblock_basis", "Basis"),
    ("wl_parties.fields.unblock_files", "Evidence files"),
    (
        "wl_parties.fields.unblock_files_hint",
        "PDF, JPG, PNG, HEIC, DOCX or XLSX, up to 20 MB each.",
    ),
    ("wl_parties.fields.unblocked_by", "Lifted by"),
    ("wl_parties.fields.unblock_note", "Lifted because"),
    ("wl_parties.fields.evidence", "Evidence"),
    ("wl_parties.review.on_hold", "On hold"),
    (
        "wl_parties.review.on_hold_reason",
        "On hold \u{2014} :reason",
    ),
    (
        "wl_parties.review.notice_title",
        "A party is on hold: :count agreements to review",
    ),
    (
        "wl_parties.review.notice_title_one",
        "A party is on hold: 1 agreement to review",
    ),
    (
        "wl_parties.review.notice_body",
        "Open the review task to pause, continue or end each agreement.",
    ),
    (
        "wl_parties.errors.legal_name_required",
        "Enter the legal name in English.",
    ),
    (
        "wl_parties.errors.details_kind",
        "Person details are for persons and company details for companies.",
    ),
    ("wl_parties.errors.trn_invalid", "A TRN has 15 digits."),
    ("wl_parties.errors.role_required", "Choose a role."),
    (
        "wl_parties.errors.contact_required",
        "Enter the contact's address or number.",
    ),
    (
        "wl_parties.errors.one_primary",
        "Only one contact of each type can be primary.",
    ),
    ("wl_parties.errors.reason_unknown", "Choose a reason."),
    ("wl_parties.errors.note_required", "Write a note."),
    (
        "wl_parties.errors.reason_restricted",
        "You cannot use this reason.",
    ),
    (
        "wl_parties.errors.unblock_other_user",
        "This hold must be lifted by another authorised user",
    ),
    (
        "wl_parties.errors.unblock_basis_required",
        "Choose why the hold is lifted.",
    ),
    (
        "wl_parties.errors.unblock_evidence_required",
        "Attach at least one evidence file.",
    ),
    (
        "wl_parties.errors.unblock_file_empty",
        "An evidence file is empty.",
    ),
    (
        "wl_parties.errors.unblock_file_large",
        "An evidence file is larger than 20 MB.",
    ),
    (
        "wl_parties.errors.unblock_file_type",
        "Evidence files must be PDF, JPG, PNG, HEIC, DOCX or XLSX.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key)
}

/// Like [`t`], filling the `:name` placeholders with `params`.
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn std::fmt::Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    let text = english(key);
    let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
    renox::i18n::format(&text, &params, None)
}

/// The package's English text for `key`, or the key when it has none.
pub(crate) fn english(key: &str) -> String {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or_else(|| key.to_owned(), |(_, text)| (*text).to_owned())
}

/// Every text of the package in `lang`, keyed without the `wl_parties.`
/// prefix (`fields.code`), for the templates: `tx['fields.code']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_parties."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TEXTS;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_parties.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }
}
