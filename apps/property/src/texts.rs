//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("property.button_cancel", "Cancel"),
    ("property.none", "—"),
    ("property.pending_approval", "Waiting for approval"),
    ("property.fields.status", "Status"),
    // The portfolio.
    ("property.portfolio.title", "Property"),
    ("property.portfolio.subtitle", "Buildings, units and their availability."),
    ("property.portfolio.empty", "No buildings yet"),
    ("property.emirates.dubai", "Dubai"),
    ("property.emirates.abu_dhabi", "Abu Dhabi"),
    ("property.emirates.sharjah", "Sharjah"),
    ("property.emirates.ajman", "Ajman"),
    ("property.emirates.umm_al_quwain", "Umm Al Quwain"),
    ("property.emirates.ras_al_khaimah", "Ras Al Khaimah"),
    ("property.emirates.fujairah", "Fujairah"),
    ("property.buildings.add", "Add building"),
    ("property.buildings.created", "Building added."),
    ("property.buildings.name", "Building"),
    ("property.buildings.code", "Code"),
    ("property.buildings.name_en", "Name (English)"),
    ("property.buildings.name_ar", "Name (Arabic)"),
    ("property.buildings.emirate", "Emirate"),
    ("property.buildings.community", "Community"),
    ("property.buildings.plot_number", "Plot number"),
    ("property.buildings.premise_number", "Premise number (DEWA)"),
    ("property.buildings.units", "Units"),
    ("property.buildings.available", "Available"),
    ("property.buildings.occupancy", "Occupancy"),
    ("property.buildings.details", "Building"),
    ("property.buildings.code_required", "Give the building a code."),
    ("property.buildings.name_required", "Give the building a name."),
    ("property.buildings.code_taken", "A building has this code already."),
    // Units.
    ("property.units.title", "Units"),
    ("property.units.empty", "No units yet"),
    ("property.units.add", "Add unit"),
    ("property.units.created", "Unit added."),
    ("property.units.number", "Unit"),
    ("property.units.type_label", "Type"),
    ("property.units.usage_label", "Use"),
    ("property.units.area", "Size (sq ft)"),
    ("property.units.bedrooms", "Bedrooms"),
    ("property.units.asking_rent", "Asking rent (AED a year)"),
    ("property.units.notes", "Notes"),
    ("property.units.tenant", "Tenant"),
    ("property.units.number_required", "Give the unit a number."),
    ("property.units.no_building", "The building was not found."),
    ("property.units.negative", "The size and the rent cannot be negative."),
    ("property.units.number_taken", "The building has a unit with this number already."),
    (
        "property.units.reserve_later",
        "A unit is reserved with a booking deposit, which comes with reservations.",
    ),
    ("property.units.type.apartment", "Apartment"),
    ("property.units.type.villa", "Villa"),
    ("property.units.type.office", "Office"),
    ("property.units.type.shop", "Shop"),
    ("property.units.type.warehouse", "Warehouse"),
    ("property.units.usage.residential", "Residential"),
    ("property.units.usage.commercial", "Commercial"),
    ("property.units.status.available", "Available"),
    ("property.units.status.reserved", "Reserved"),
    ("property.units.status.occupied", "Occupied"),
    ("property.units.status.notice_given", "Notice given"),
    ("property.units.status.under_maintenance", "Under maintenance"),
    ("property.units.status.off_market", "Off the market"),
    // Leases.
    ("property.leases.title", "Leases"),
    ("property.leases.subtitle", "Tenancy contracts and their versions."),
    ("property.leases.empty", "No leases yet"),
    ("property.leases.new", "New lease"),
    ("property.leases.created", "Lease drafted."),
    ("property.leases.number", "Lease"),
    ("property.leases.unit", "Unit"),
    ("property.leases.tenant", "Tenant"),
    ("property.leases.tenant_number", "Tenant (party number)"),
    ("property.leases.term", "Term"),
    ("property.leases.start_date", "Start date"),
    ("property.leases.end_date", "End date"),
    ("property.leases.annual_rent", "Annual rent (AED)"),
    ("property.leases.deposit", "Security deposit (AED)"),
    ("property.leases.deposit_waived", "Waived"),
    ("property.leases.deposit_waived_done", "Deposit waived."),
    ("property.leases.instalments", "Rent instalments"),
    ("property.leases.first_due", "First instalment due"),
    ("property.leases.notice_days", "Notice period (days)"),
    ("property.leases.notice_days_hint", "Leave empty for the 90 days of the law."),
    ("property.leases.notice_days_label", "Notice period (days)"),
    ("property.leases.terms", "Clauses"),
    ("property.leases.terms_title", "Terms"),
    ("property.leases.vat", "VAT"),
    ("property.leases.effective_date", "Effective date"),
    ("property.leases.effective_date_hint", "Required for an amendment."),
    ("property.leases.follow_up_until", "Follow-up date"),
    ("property.leases.kind_label", "Version"),
    ("property.leases.kind.new", "First version"),
    ("property.leases.kind.renewal", "Renewal"),
    ("property.leases.kind.amendment", "Amendment"),
    ("property.leases.versions", "Versions"),
    ("property.leases.version", "Version"),
    ("property.leases.version_kind", "Kind"),
    ("property.leases.new_version", "Renew or amend"),
    (
        "property.leases.new_version_description",
        "A new version of this lease for the same tenant and unit.",
    ),
    ("property.leases.version_created", "Version drafted."),
    ("property.leases.submit", "Submit for signature"),
    (
        "property.leases.submit_message",
        "The contract goes out for signature and the lease can no longer be edited.",
    ),
    ("property.leases.submitted", "Lease submitted for signature."),
    ("property.leases.activate", "Activate"),
    (
        "property.leases.activate_message",
        "The signed lease starts: its charges are scheduled.",
    ),
    ("property.leases.activated", "Lease activated."),
    ("property.leases.waive_deposit", "Waive the deposit"),
    ("property.leases.waive_message", "Activation will not wait for the deposit."),
    ("property.leases.status.draft", "Draft"),
    ("property.leases.status.pending_signature", "Pending signature"),
    ("property.leases.status.active", "Active"),
    ("property.leases.status.renewal_due", "Renewal due"),
    ("property.leases.status.expired", "Holding over"),
    ("property.leases.status.notice_given", "Notice given"),
    ("property.leases.status.renewed", "Renewed"),
    ("property.leases.status.amended", "Amended"),
    ("property.leases.status.terminated", "Terminated"),
    ("property.leases.status.cancelled", "Cancelled"),
    ("property.leases.no_unit", "The unit was not found."),
    ("property.leases.no_tenant", "The tenant was not found among the parties."),
    ("property.leases.no_previous", "The version this one replaces was not found."),
    ("property.leases.dates", "The end date must be after the start date."),
    ("property.leases.rent_required", "The annual rent must be above 0."),
    ("property.leases.deposit_negative", "The deposit cannot be negative."),
    ("property.leases.instalments_range", "Pay the rent in 1 to :max instalments."),
    ("property.leases.version_kind_bad", "A new version is a renewal or an amendment."),
    (
        "property.leases.one_in_progress",
        "Another renewal or amendment of this lease is still being prepared or signed.",
    ),
    ("property.leases.edit_draft_only", "A lease can be edited only while it is a draft."),
    ("property.leases.unit_not_available", "The unit is not available."),
    (
        "property.leases.previous_not_renewable",
        "The previous version is :status: a renewal replaces a lease that is due for renewal or holding over.",
    ),
    (
        "property.leases.previous_not_amendable",
        "The previous version is :status: an amendment replaces a lease that is active, due for renewal or holding over.",
    ),
    (
        "property.leases.effective_required",
        "An amendment needs the date it applies from.",
    ),
    (
        "property.leases.effective_past",
        "An amendment cannot apply from before the first day of this month.",
    ),
    (
        "property.leases.effective_outside",
        "An amendment applies from a date within the term of the version it changes.",
    ),
    (
        "property.leases.plan_mismatch",
        "The payment plan adds up to :plan, not the annual rent of :rent.",
    ),
    ("property.leases.cancel_pending", "A cancellation of this lease is waiting for approval."),
    (
        "property.leases.deposit_missing",
        "The deposit has not been received: :needed needed, :held held. A property manager can waive it.",
    ),
    ("property.leases.deposit_zero", "A deposit cannot be adjusted to nothing."),
    ("property.leases.waive_status", "Only a lease that is not active yet can have its deposit waived."),
    // The payment plan and the cheques.
    ("property.charges.title", "Payment plan"),
    ("property.charges.empty", "No charges"),
    ("property.charges.number", "No."),
    ("property.charges.description", "Charge"),
    ("property.charges.due_date", "Due"),
    ("property.charges.amount", "Amount"),
    ("property.charges.invoice", "Invoice"),
    ("property.charges.status.planned", "Planned"),
    ("property.charges.status.scheduled", "Scheduled"),
    ("property.charges.status.invoiced", "Invoiced"),
    ("property.charges.status.cancelled", "Cancelled"),
    ("property.vat.standard", "5% VAT"),
    ("property.vat.zero_rated", "Zero-rated"),
    ("property.vat.exempt", "Exempt"),
    ("property.vat.out_of_scope", "Out of scope"),
    ("property.vat.reverse_charge", "Reverse charge"),
    ("property.cheques.title", "Post-dated cheques"),
    ("property.cheques.empty", "No cheques received"),
    ("property.cheques.record", "Record cheques"),
    (
        "property.cheques.record_description",
        "The post-dated cheques the tenant handed over.",
    ),
    ("property.cheques.recorded", "Cheques recorded."),
    ("property.cheques.bank", "Bank"),
    ("property.cheques.number", "Cheque"),
    ("property.cheques.date", "Date"),
    ("property.cheques.lines", "Cheques"),
    (
        "property.cheques.lines_hint",
        "One cheque a line: number | YYYY-MM-DD | amount | instalment (optional)",
    ),
    ("property.cheques.none", "Give at least one cheque."),
    ("property.cheques.lease_over", "This lease has ended: no cheque can be recorded for it."),
    ("property.cheques.bad_charge", "A cheque can only pay a live charge of this lease."),
    ("property.cheques.status.received", "Received"),
    ("property.cheques.status.due_for_deposit", "Due for deposit"),
    ("property.cheques.status.deposited", "Deposited"),
    ("property.cheques.status.cleared", "Cleared"),
    ("property.cheques.status.bounced", "Bounced"),
    ("property.cheques.status.legal_case", "Legal case"),
    ("property.cheques.status.replaced", "Replaced"),
    ("property.cheques.status.written_off", "Written off"),
    ("property.cheques.status.returned", "Returned"),
    // Ejari.
    ("property.ejari.record", "Record Ejari"),
    (
        "property.ejari.description",
        "The Ejari or Tawtheeq registration number, typed in; attach the certificate as a document of the lease.",
    ),
    ("property.ejari.number", "Ejari / Tawtheeq number"),
    ("property.ejari.registered_on", "Registered on"),
    ("property.ejari.expires_on", "Expires on"),
    ("property.ejari.missing", "Not registered"),
    ("property.ejari.recorded", "The Ejari registration was recorded."),
    ("property.ejari.recorded_done", "Ejari recorded."),
    ("property.ejari.number_required", "Give the registration number."),
    ("property.ejari.not_running", "Only a lease in force has an Ejari registration."),
    // Tasks.
    ("property.tasks.ejari_new", "Register the Ejari contract of :lease"),
    ("property.tasks.ejari_renewal", "Renew the Ejari contract of :lease"),
    ("property.tasks.ejari_amendment", "Update the Ejari contract of :lease"),
    (
        "property.tasks.ejari_description",
        "Register the tenancy contract with Ejari or Tawtheeq and record the number on the lease.",
    ),
    ("property.tasks.deposit_adjust", "Adjust the deposit of :lease to :amount"),
    // History.
    ("property.history.title", "History"),
    ("property.history.change", "Change"),
    ("property.history.at", "When"),
    ("property.history.reason", "Reason"),
    ("property.history.empty", "Nothing yet"),
    // Cancellation.
    ("property.cancel_reason", "Cancelled because"),
    ("property.cancel_forfeit", "Deposit forfeited"),
    ("property.cancel_forfeit_clause", "Clause of the forfeit"),
    ("property.cancel.forfeit_negative", "The forfeited amount cannot be negative."),
    (
        "property.cancel.clause_required",
        "A forfeit needs the clause of the signed reservation or contract terms that allows it.",
    ),
    ("property.cancel.forfeit_too_high", "The forfeited amount cannot exceed the deposit."),
    ("property.cancel.action", "Cancel the lease"),
    (
        "property.cancel.description",
        "The prospect backs out. Money or cheques received need a property manager's approval.",
    ),
    ("property.cancel.submit", "Cancel the lease"),
    ("property.cancel.reason", "Reason"),
    ("property.cancel.forfeit", "Deposit forfeited (AED)"),
    (
        "property.cancel.forfeit_hint",
        "Only where the signed reservation or contract terms allow it.",
    ),
    ("property.cancel.clause", "Clause of the terms"),
    ("property.cancel.done", "Lease cancelled."),
    ("property.cancel.sent", "Cancellation sent to a property manager for approval."),
    ("property.cancel.default_reason", "Lease cancelled"),
    ("property.cancel.refund_reason", "Lease :lease cancelled: :reason"),
    (
        "property.cancel.task_deposited",
        "Cheques deposited before :lease was cancelled",
    ),
    (
        "property.cancel.task_problems",
        "Finish the cancellation of :lease by hand",
    ),
    // The chase of a lease waiting for its signature.
    (
        "property.notices.lease_unsigned_subject",
        "Lease {{ lease }} still waits for its signature",
    ),
    (
        "property.notices.lease_unsigned_body",
        "The lease {{ lease }} has been waiting for its signature and deposit for {{ days }} days, since {{ since }}.",
    ),
    ("property.chase.extend", "Extend the follow-up"),
    (
        "property.chase.extend_description",
        "Give the date the signature is expected by: at most 30 days from today.",
    ),
    ("property.chase.until", "Follow up on"),
    ("property.chase.extended_done", "Follow-up extended."),
    ("property.chase.extended", "The follow-up was extended to :until."),
    ("property.chase.task_title", "Extend or cancel the lease :lease, still waiting for its signature"),
    (
        "property.chase.task_description",
        "The lease has waited :days days since :since. Extend the follow-up to a date at most 30 days ahead, or cancel it.",
    ),
    ("property.chase.not_waiting", "Only a lease waiting for its signature has a follow-up."),
    (
        "property.chase.date_range",
        "The follow-up date is after today and at most :max days ahead.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
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

/// Every text of the app in `lang`, keyed without the `property.` prefix
/// (`portfolio.title`), for the templates: `tx['portfolio.title']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("property."), t(lang, key)))
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
    use super::{TEXTS, english};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("property.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(
            english("property.tasks.ejari_new", &[("lease", &"L-000012")]),
            "Register the Ejari contract of L-000012"
        );
    }
}
