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
    ("wl_parties.block_effects.title", "Block effects"),
    (
        "wl_parties.block_effects.subtitle",
        "What a block does to each kind of agreement. The vertical sets these values.",
    ),
    ("wl_parties.block_effects.agreement", "Agreement"),
    ("wl_parties.block_effects.owner", "Owner"),
    ("wl_parties.block_effects.effect", "Effect"),
    ("wl_parties.block_effects.decider", "Decider role"),
    ("wl_parties.block_effects.source", "Set by"),
    ("wl_parties.block_effects.has_source", "Lists agreements"),
    ("wl_parties.block_effects.no_decider", "None"),
    ("wl_parties.effect.pause", "Pause"),
    ("wl_parties.effect.continue", "Continue"),
    ("wl_parties.effect.review", "Review"),
    ("wl_parties.rule_source.default", "Default"),
    ("wl_parties.rule_source.module", "Module"),
    ("wl_parties.rule_source.vertical", "Vertical"),
    ("wl_parties.block_kind.engagement", "Engagement"),
    ("wl_parties.block_kind.subscription", "Subscription"),
    ("wl_parties.block_kind.enrolment", "Enrolment"),
    ("wl_parties.block_kind.lease", "Lease"),
    ("wl_parties.block_kind.deployment", "Deployment"),
    ("wl_parties.block_kind.project", "Project"),
    ("wl_parties.block_kind.sales_order", "Sales order"),
    ("wl_parties.block_kind.booking", "Booking"),
    ("wl_parties.block_kind.purchase_order", "Purchase order"),
    ("wl_parties.block_kind.service_contract", "Service contract"),
    ("wl_parties.block_kind.money_out", "Money out"),
    ("wl_parties.block_kind.money_in", "Money in"),
    ("wl_parties.review.title", "Review tasks"),
    (
        "wl_parties.review.subtitle",
        "Agreements of parties on hold that wait for your decision.",
    ),
    ("wl_parties.review.empty", "No review tasks"),
    (
        "wl_parties.review.empty_hint",
        "Tasks show up here when a party is put on hold.",
    ),
    ("wl_parties.review.task", "Review task"),
    ("wl_parties.review.fields.party", "Party"),
    ("wl_parties.review.fields.agreement", "Agreement"),
    ("wl_parties.review.fields.scope", "Scope"),
    ("wl_parties.review.fields.status", "Status"),
    ("wl_parties.review.fields.pending", "Waiting"),
    ("wl_parties.review.fields.opened", "Opened"),
    ("wl_parties.review.fields.closed", "Closed"),
    ("wl_parties.review.fields.decision", "Decision"),
    ("wl_parties.review.fields.decided_by", "Decided by"),
    ("wl_parties.review.fields.decided_at", "Decided"),
    ("wl_parties.review.fields.note", "Note"),
    ("wl_parties.review.fields.by", "By"),
    ("wl_parties.review.fields.at", "When"),
    ("wl_parties.review.sections.items", "Agreements"),
    ("wl_parties.review.sections.notes", "Notes"),
    ("wl_parties.review.no_notes", "No notes."),
    ("wl_parties.review.no_items", "No agreements."),
    ("wl_parties.review.global_scope", "Everywhere"),
    ("wl_parties.review.system", "System"),
    ("wl_parties.review.party_unblocked", "Party unblocked"),
    ("wl_parties.review.status.open", "Open"),
    ("wl_parties.review.status.closed", "Closed"),
    ("wl_parties.review.decision.pending", "Waiting"),
    ("wl_parties.review.decision.pause", "Pause"),
    ("wl_parties.review.decision.continue", "Continue"),
    ("wl_parties.review.decision.end", "End"),
    ("wl_parties.review.decide", "Decide"),
    ("wl_parties.review.decide_title", "Decide on the agreement"),
    (
        "wl_parties.review.decide_description",
        "The agreement's own module carries it out. Nothing is sent to the party.",
    ),
    ("wl_parties.review.decide_submit", "Confirm decision"),
    ("wl_parties.review.decided", "Decision recorded."),
    (
        "wl_parties.review.refused",
        "The agreement was not changed: :reason",
    ),
    (
        "wl_parties.review.already_decided",
        "This agreement is already decided.",
    ),
    ("wl_parties.review.task_closed", "This task is closed."),
    (
        "wl_parties.review.no_source",
        "No module handles this kind of agreement.",
    ),
    (
        "wl_parties.review.no_decider",
        "No decider role is set for this kind; users with the block permission decide.",
    ),
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
