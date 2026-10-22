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
    (
        "clinics.appointments.cancelled",
        "This appointment is cancelled.",
    ),
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
    (
        "clinics.coverage.waiting",
        "Waiting for the coverage outcome",
    ),
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
    (
        "clinics.coverage.eligibility_recorded",
        "Eligibility recorded.",
    ),
    ("clinics.coverage.result", "Result"),
    ("clinics.coverage.reference", "Payer reference"),
    ("clinics.coverage.note", "Note"),
    ("clinics.coverage.outcome", "Coverage outcome"),
    ("clinics.coverage.outcome_missing", "Not set"),
    ("clinics.coverage.outcome_none", "Not an insured visit"),
    ("clinics.coverage.outcome.insured", "Insured"),
    (
        "clinics.coverage.outcome.re_split",
        "Re-split to another policy",
    ),
    ("clinics.coverage.outcome.self_pay", "Self-pay"),
    (
        "clinics.coverage.record_outcome",
        "Set the coverage outcome",
    ),
    (
        "clinics.coverage.record_outcome_description",
        "Insured, re-split to another valid policy, or self-pay with the patient's signed consent. Otherwise reschedule or cancel with the reason coverage not confirmed.",
    ),
    (
        "clinics.coverage.outcome_recorded",
        "Coverage outcome recorded.",
    ),
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
    (
        "clinics.coverage.changed_to_insured",
        "The visit is insured.",
    ),
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
    (
        "clinics.coverage.problem.services",
        "Services are written as code:units, for example laser:2, consultation:1; each at least one unit, none twice.",
    ),
    (
        "clinics.coverage.problem.already",
        "The visit is already insured.",
    ),
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
    (
        "clinics.consent.signed",
        "Consent signed; the visit is self-pay.",
    ),
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
        "clinics.consent.problem.no_services",
        "Name at least one service for the consent.",
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
    // The prior authorisations (CLN.INS.03).
    ("clinics.authorisation.title", "Prior authorisations"),
    (
        "clinics.authorisation.subtitle",
        "What the payers approved for named services, with the units left.",
    ),
    ("clinics.authorisation.new", "New authorisation"),
    (
        "clinics.authorisation.new_description",
        "Prepare the request for the patient's policy: the service codes it should cover and the units asked for.",
    ),
    ("clinics.authorisation.number", "Number"),
    ("clinics.authorisation.details", "Authorisation"),
    (
        "clinics.authorisation.services",
        "Service codes, comma separated",
    ),
    ("clinics.authorisation.units", "Units left"),
    ("clinics.authorisation.of", "of"),
    ("clinics.authorisation.units_asked", "Units asked for"),
    ("clinics.authorisation.units_approved", "Units approved"),
    ("clinics.authorisation.units_consumed", "Units consumed"),
    ("clinics.authorisation.units_left", "Units left"),
    ("clinics.authorisation.reference", "Payer's reference"),
    ("clinics.authorisation.valid_until", "Valid until"),
    (
        "clinics.authorisation.denial_reason",
        "Reason the payer gave",
    ),
    ("clinics.authorisation.empty", "No prior authorisations yet"),
    ("clinics.authorisation.created", "Authorisation prepared."),
    ("clinics.authorisation.status.draft", "Draft"),
    ("clinics.authorisation.status.requested", "Requested"),
    ("clinics.authorisation.status.approved", "Approved"),
    (
        "clinics.authorisation.status.partially_approved",
        "Partially approved",
    ),
    ("clinics.authorisation.status.denied", "Denied"),
    ("clinics.authorisation.status.used", "Used"),
    ("clinics.authorisation.status.expired", "Expired"),
    ("clinics.authorisation.status.cancelled", "Cancelled"),
    ("clinics.authorisation.request", "Send to the payer"),
    (
        "clinics.authorisation.request_description",
        "Records that the request was sent, with the payer's reference if it gave one.",
    ),
    ("clinics.authorisation.requested", "Request recorded."),
    ("clinics.authorisation.approve", "Approve"),
    (
        "clinics.authorisation.approve_description",
        "The payer approved the units asked for: its reference and the last day it is valid.",
    ),
    ("clinics.authorisation.approved", "Authorisation approved."),
    (
        "clinics.authorisation.approve_partially",
        "Approve partially",
    ),
    (
        "clinics.authorisation.approve_partially_description",
        "The payer approved fewer units than were asked for.",
    ),
    (
        "clinics.authorisation.approved_partially",
        "Authorisation partially approved.",
    ),
    ("clinics.authorisation.deny", "Deny"),
    (
        "clinics.authorisation.deny_description",
        "The payer refused. The patient is told, without the reason, and may choose to pay for the service themselves.",
    ),
    ("clinics.authorisation.denied", "Authorisation denied."),
    ("clinics.authorisation.cancel", "Cancel"),
    (
        "clinics.authorisation.cancel_message",
        "The authorisation is no longer needed.",
    ),
    (
        "clinics.authorisation.cancelled",
        "Authorisation cancelled.",
    ),
    (
        "clinics.authorisation.required_title",
        "Services that need a prior authorisation",
    ),
    (
        "clinics.authorisation.required_new",
        "Service that needs one",
    ),
    (
        "clinics.authorisation.required_description",
        "A service on the payer's list is checked in only with a valid authorisation that has units left, or as self-pay with consent.",
    ),
    ("clinics.authorisation.required_code", "Service code"),
    ("clinics.authorisation.required_remove", "Remove"),
    (
        "clinics.authorisation.required_remove_message",
        "The payer no longer asks for an authorisation for this service.",
    ),
    (
        "clinics.authorisation.required_empty",
        "No service needs an authorisation",
    ),
    (
        "clinics.authorisation.required_added",
        "Service added to the list.",
    ),
    (
        "clinics.authorisation.required_removed",
        "Service removed from the list.",
    ),
    (
        "clinics.authorisation.problem.services",
        "Name the services and the units before the request is sent.",
    ),
    (
        "clinics.authorisation.problem.approval",
        "The payer's reference and a last valid day that is not in the past are required.",
    ),
    (
        "clinics.authorisation.problem.units",
        "Partial approval needs at least one unit and fewer than were asked for.",
    ),
    (
        "clinics.authorisation.problem.reason",
        "The reason the payer gave is required.",
    ),
    (
        "clinics.authorisation.problem.units_left",
        "There are not that many units left on the authorisation.",
    ),
    (
        "clinics.authorisation.problem.not_last",
        "These are not the last units: the authorisation stays open.",
    ),
    (
        "clinics.authorisation.problem.is_last",
        "These are the last units: the authorisation is used.",
    ),
    (
        "clinics.authorisation.problem.not_passed",
        "The last valid day has not passed yet.",
    ),
    (
        "clinics.authorisation.problem.units_asked",
        "Ask for at least one unit.",
    ),
    (
        "clinics.authorisation.problem.service_code",
        "The service code is required.",
    ),
    (
        "clinics.authorisation.problem.required",
        "Prior authorisation required: self-pay with consent or reschedule.",
    ),
    (
        "clinics.authorisation.problem.nothing_to_sign",
        "Every service is covered by an authorisation; no self-pay consent is needed.",
    ),
    (
        "clinics.authorisation.line_covered",
        "Prior authorisation: :service, :units units covered by :numbers.",
    ),
    (
        "clinics.authorisation.line_short",
        "Prior authorisation: :service, :covered of :units units covered; :self_pay go self-pay with the patient's consent, or reschedule while a new authorisation is requested.",
    ),
    (
        "clinics.authorisation.services_action",
        "Services and units",
    ),
    (
        "clinics.authorisation.services_description",
        "The services the visit will use and the units of each, for example laser:2, consultation:1. A service that needs a prior authorisation is checked against the units left.",
    ),
    (
        "clinics.authorisation.services_field",
        "Services (code:units)",
    ),
    (
        "clinics.authorisation.services_recorded",
        "Services recorded.",
    ),
    (
        "clinics.authorisation.consent_action",
        "Self-pay consent for the uncovered services",
    ),
    (
        "clinics.authorisation.consent_description",
        "The patient signs for the services a prior authorisation does not cover, with their estimated price. The services it covers stay in the payer split.",
    ),
    (
        "clinics.authorisation.consent_signed",
        "Consent signed for the uncovered services.",
    ),
    ("clinics.authorisation.holds", "Units held"),
    (
        "clinics.authorisation.denied_subject",
        "Your insurer did not approve the authorisation",
    ),
    (
        "clinics.authorisation.denied_body",
        "Your insurer did not approve authorisation {{ number }}. You may choose to pay for the service yourself, or we can help you with another date. Please contact the clinic.",
    ),
    // The flags on an appointment (CLN.RUL.07, CLN.RUL.09, CLN.RUL.10).
    ("clinics.flags.title", "Flags"),
    ("clinics.flags.none", "No flags"),
    ("clinics.flags.needs_authorisation", "Needs authorisation"),
    ("clinics.flags.insurance_card", "Insurance card to renew"),
    (
        "clinics.flags.needed_detail",
        "A prior authorisation is needed for :service on :date and none covers it.",
    ),
    (
        "clinics.flags.needed_task",
        "Request a prior authorisation for :service on :date",
    ),
    ("clinics.flags.cause_denied", "was denied"),
    ("clinics.flags.cause_used", "is used up"),
    ("clinics.flags.cause_expired", "has ended"),
    ("clinics.flags.cause_look_ahead", "ends in three days"),
    (
        "clinics.flags.stopped_detail",
        "Prior authorisation :number :cause and no other covers this appointment.",
    ),
    (
        "clinics.flags.stopped_task",
        "Request a new prior authorisation: :number :cause",
    ),
    (
        "clinics.cards.subject",
        "Please send us your new insurance card",
    ),
    (
        "clinics.cards.body",
        "Your insurance policy or card will not be valid on the day of your appointment at {{ clinic }} on {{ date }} at {{ time }}. Please upload your new insurance card here: {{ link }}",
    ),
    (
        "clinics.flags.card_detail",
        "The policy ends or the card expires before the visit on :date: a new card was asked for.",
    ),
    (
        "clinics.policies.problem.card_expired",
        "The card has already expired.",
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
    (
        "clinics.policies.problem.duplicate",
        "This policy is on file already for the patient: same payer and member id, and the dates overlap.",
    ),
    (
        "clinics.policies.problem.patient",
        "Give the patient's party number, as the Parties list shows it (23 or P-00023).",
    ),
    (
        "clinics.policies.problem.payer",
        "Give the payer's party number, as the Parties list shows it (25 or P-00025).",
    ),
    (
        "clinics.policies.problem.patient_unknown",
        "There is no party with the patient's number.",
    ),
    (
        "clinics.policies.problem.payer_unknown",
        "There is no party with the payer's number.",
    ),
    (
        "clinics.policies.problem.payer_is_patient",
        "The payer is the patient. Give the number of the insurer or the TPA.",
    ),
    // Charge capture (CLN.ENC.01, CLN.ENC.02; BL-CLN-05, BL-CLN-06).
    ("clinics.charges.title", "Charge capture"),
    ("clinics.charges.empty", "Nothing recorded yet"),
    ("clinics.charges.code", "Code"),
    ("clinics.charges.units", "Units"),
    ("clinics.charges.kind", "Kind"),
    ("clinics.charges.price", "Price"),
    ("clinics.charges.patient_share", "Patient share"),
    ("clinics.charges.payer_share", "Payer share"),
    ("clinics.charges.note", "Note"),
    ("clinics.charges.kind.service", "Service"),
    ("clinics.charges.kind.consumable", "Consumable"),
    ("clinics.charges.vat", "VAT"),
    ("clinics.charges.vat.zero", "Zero-rated"),
    ("clinics.charges.vat.standard", "Standard rated (5%)"),
    ("clinics.charges.add", "Record a charge"),
    (
        "clinics.charges.add_description",
        "A service the practitioner delivered, or a consumable issued to the patient. It is priced on the visit date.",
    ),
    ("clinics.charges.recorded", "Recorded."),
    ("clinics.charges.none", "Nothing done"),
    (
        "clinics.charges.none_description",
        "No service was delivered and no consumable issued. The visit can then be cancelled.",
    ),
    ("clinics.charges.none_done", "Work done set to none."),
    ("clinics.charges.accept", "Patient accepts the work done"),
    (
        "clinics.charges.accept_description",
        "The clinic stopped the visit. Without the patient's acceptance nothing is charged and no claim is made.",
    ),
    ("clinics.charges.accepted", "Acceptance recorded."),
    ("clinics.charges.not_suitable", "Procedure not suitable"),
    (
        "clinics.charges.not_suitable_description",
        "The planned procedure is not performed and not charged. Pick the consultation given instead.",
    ),
    ("clinics.charges.consultation", "Consultation service given"),
    (
        "clinics.charges.not_suitable_done",
        "Recorded: procedure not performed.",
    ),
    (
        "clinics.charges.refund_deposit",
        "Refund the deposit in full",
    ),
    (
        "clinics.charges.refund_deposit_description",
        "The deposit is paid back instead of being applied to the patient's share.",
    ),
    (
        "clinics.charges.refund_deposit_done",
        "The deposit will be refunded in full.",
    ),
    ("clinics.charges.bill", "Bill the visit"),
    ("clinics.charges.billed", "The visit is billed."),
    ("clinics.charges.work", "Work done"),
    ("clinics.charges.work.pending", "Not recorded yet"),
    ("clinics.charges.work.none", "None"),
    ("clinics.charges.work.captured", "Recorded"),
    ("clinics.charges.work.accepted", "Accepted by the patient"),
    (
        "clinics.charges.work.not_suitable",
        "Procedure not performed",
    ),
    ("clinics.charges.patient_total", "Patient share"),
    ("clinics.charges.payer_total", "Payer share"),
    ("clinics.charges.invoice", "Invoice"),
    ("clinics.charges.payer_bill", "Draft claim"),
    ("clinics.charges.deposit_applied", "Deposit applied"),
    ("clinics.charges.deposit_refunded", "Deposit refunded"),
    (
        "clinics.charges.reason.clinic_stop",
        "Clinic stop: not charged",
    ),
    ("clinics.charges.reason.package", "Paid by package"),
    (
        "clinics.charges.reason.not_suitable",
        "Not performed: not suitable",
    ),
    (
        "clinics.charges.problem.closed",
        "The visit is over or already billed.",
    ),
    (
        "clinics.charges.problem.line",
        "Give a code and at least one unit.",
    ),
    (
        "clinics.charges.problem.none_with_work",
        "A service or a consumable was recorded: the work done cannot be none.",
    ),
    (
        "clinics.charges.problem.consultation_needed",
        "Pick the consultation service that was given (not the planned procedure).",
    ),
    (
        "clinics.charges.problem.no_contract",
        "No payer contract for this policy on the visit date; ask the billing coordinator.",
    ),
    (
        "clinics.billing.problem.no_patient",
        "The visit has no patient to bill.",
    ),
    (
        "clinics.billing.problem.invoicing_off",
        "Invoicing is off: the visit cannot be billed.",
    ),
    (
        "clinics.billing.problem.no_agreement",
        "The payer contract has no payer agreement: the claim cannot be made.",
    ),
    ("clinics.billing.share_line", ":line: patient share"),
    (
        "clinics.billing.deposit_refund",
        "Deposit not needed by visit :number",
    ),
    ("clinics.consent.update", "Sign an updated consent"),
    (
        "clinics.consent.update_description",
        "The patient signs again with the services added during the visit.",
    ),
    ("clinics.consent.updated", "Updated consent signed."),
    // Payer contracts (CLN.INS.01).
    ("clinics.contract.title", "Payer contracts"),
    (
        "clinics.contract.subtitle",
        "What each payer pays: the copay, the deductible and the tariff of its services.",
    ),
    ("clinics.contract.new", "New contract"),
    (
        "clinics.contract.new_description",
        "The terms of a payer for a plan (or every plan), from a date.",
    ),
    ("clinics.contract.plan", "Plan"),
    ("clinics.contract.agreement", "Payer agreement"),
    ("clinics.contract.copay", "Copay (%)"),
    (
        "clinics.contract.problem.copay",
        "The copay is a percentage from 0 to 100, for example 20 or 12.5.",
    ),
    ("clinics.contract.problem.contract", "Choose the contract."),
    ("clinics.contract.deductible", "Deductible a visit (AED)"),
    ("clinics.contract.valid_from", "From"),
    ("clinics.contract.valid_to", "To"),
    ("clinics.contract.created", "Contract recorded."),
    ("clinics.contract.empty", "No payer contract yet"),
    ("clinics.contract.tariff", "Set a tariff"),
    (
        "clinics.contract.tariff_description",
        "The payer's price of a service, and whether it covers it.",
    ),
    ("clinics.contract.contract", "Contract"),
    ("clinics.contract.service", "Service code"),
    (
        "clinics.contract.price",
        "Payer's price (AED, blank = price list)",
    ),
    ("clinics.contract.covered", "Covered"),
    ("clinics.contract.covered_yes", "Covered"),
    ("clinics.contract.covered_no", "Excluded"),
    ("clinics.contract.tariff_set", "Tariff recorded."),
    ("clinics.contract.tariffs", "Tariffs"),
    (
        "clinics.contract.problem.terms",
        "The copay is 0 to 10,000 basis points, the deductible is not negative and the contract does not end before it starts.",
    ),
    (
        "clinics.contract.problem.tariff",
        "Give a service code and a price that is not negative.",
    ),
    // Claims to the payers (CLN.CLM.01).
    (
        "clinics.contract.first_submission_days",
        "Days to send a claim",
    ),
    ("clinics.contract.resubmission_days", "Days to resubmit"),
    (
        "clinics.contract.resubmission_limit",
        "Resubmissions accepted",
    ),
    (
        "clinics.contract.accepts_late_claims",
        "Accepts late claims",
    ),
    ("clinics.contract.late_no", "No"),
    ("clinics.contract.late_yes", "Yes, with a late reason"),
    ("clinics.contract.windows", "Windows"),
    (
        "clinics.contract.missed_billing",
        "Missed-window share may be billed to the patient",
    ),
    (
        "clinics.contract.missed_billing_description",
        "Whether the patient may be billed for a payer share the clinic did not submit in time. Off unless the owner allows it; whether it is lawful depends on the payer contract and the emirate's rules.",
    ),
    ("clinics.contract.missed_billing_no", "No"),
    ("clinics.contract.missed_billing_yes", "Yes"),
    ("clinics.contract.missed_billing_set", "Setting saved."),
    (
        "clinics.contract.windows_text",
        "Send within :first days (coordinator told after :reminder), resubmit within :again days, :limit times at most",
    ),
    (
        "clinics.contract.reminder_days",
        "Days after the visit to remind the coordinator",
    ),
    (
        "clinics.contract.problem.reminder",
        "N must be below the submission window: the days after the visit to remind the coordinator are 0 or more and fewer than the days to send a claim.",
    ),
    // The first-submission window about to pass (CLN.RUL.03).
    (
        "clinics.window.subject",
        "Claim {{ number }} is still not sent: {{ days_left }} days left",
    ),
    (
        "clinics.window.body",
        "Claim {{ number }} for the visit of {{ visit_date }} is still {{ status }}. The payer accepts it until {{ deadline }}. Validate and submit it in time; after that it can only be written off.",
    ),
    (
        "clinics.window.task_title",
        "Claim :number must be sent by :deadline",
    ),
    (
        "clinics.window.task_description",
        "Claim :number for the visit of :visit_date is still :status and the payer's first-submission window ends on :deadline. Send it in time, or it is written off as not submitted in time.",
    ),
    ("clinics.claim.title", "Claims"),
    (
        "clinics.claim.subtitle",
        "The claims to the payers: prepare, validate and submit them, record the channel's rejection and the payer's answers.",
    ),
    ("clinics.claim.number", "Claim"),
    ("clinics.claim.empty", "No claim yet"),
    ("clinics.claim.details", "Claim"),
    ("clinics.claim.visit", "Visit"),
    ("clinics.claim.visit_date", "Visit date"),
    ("clinics.claim.payer_bill", "Payer bill"),
    (
        "clinics.claim.payer_reference",
        "Prior authorisation reference",
    ),
    ("clinics.claim.claimed", "Claimed"),
    ("clinics.claim.paid", "Paid"),
    ("clinics.claim.denied", "Denied"),
    ("clinics.claim.recouped", "Taken back"),
    (
        "clinics.claim.written_off",
        "Written off or moved to the patient",
    ),
    ("clinics.claim.deadline", "Last day to send"),
    ("clinics.claim.closed", "Closed as"),
    ("clinics.claim.closed.written_off", "Written off :amount"),
    (
        "clinics.claim.closed.transferred",
        "Moved to the patient :amount",
    ),
    (
        "clinics.claim.not_submitted_in_time",
        "not submitted in time",
    ),
    (
        "clinics.claim.kpi",
        "Not submitted in time: :amount in :claims claims closed after the payer's first-submission window.",
    ),
    ("clinics.claim.write_off", "Write off"),
    (
        "clinics.claim.write_off_description",
        "Close the claim: the open amount is written off, or moved to the patient where the payer contract allows it. Above the limit the owner approves first.",
    ),
    ("clinics.claim.close_reason", "Reason"),
    ("clinics.claim.void", "Void"),
    (
        "clinics.claim.void_description",
        "Void a claim made in error, before it was sent. The number is kept and shown as void; the payer bill is voided and the visit's payer split is removed.",
    ),
    ("clinics.claim.void_code", "Reason"),
    ("clinics.claim.void_code.duplicate", "Duplicate"),
    ("clinics.claim.void_code.wrong_payer", "Wrong payer"),
    ("clinics.claim.void_code.wrong_policy", "Wrong policy"),
    ("clinics.claim.void_note", "Note"),
    (
        "clinics.claim.void_duplicate_of",
        "Duplicate of (a duplicate needs the claim)",
    ),
    (
        "clinics.claim.void_policy",
        "Right policy (blank: coverage to resolve)",
    ),
    ("clinics.claim.void_none", "—"),
    ("clinics.claim.voided", "Claim voided."),
    (
        "clinics.claim.voided_resplit",
        "Claim voided and the visit split again: :number.",
    ),
    (
        "clinics.claim.voided_to_resolve",
        "Claim voided. The visit has no payer split: coverage to resolve.",
    ),
    ("clinics.claim.void_info", "Voided on :date (:reason)"),
    ("clinics.claim.duplicate_of", "Duplicate of"),
    ("clinics.claim.duplicated_by", "Duplicated by"),
    ("clinics.claim.replaces", "Replaces"),
    ("clinics.claim.replaced_by", "Replaced by"),
    (
        "clinics.claim.coverage_to_resolve",
        "The visit has no payer split: coverage to resolve. Choose the right policy to split it again.",
    ),
    ("clinics.claim.resolve", "Resolve coverage"),
    (
        "clinics.claim.resolve_description",
        "Split the visit again under the right policy of the patient: a new claim is made.",
    ),
    ("clinics.claim.resolved", "The visit is split again."),
    ("clinics.claim.closed_done", "Claim closed."),
    (
        "clinics.claim.close_held",
        "The write-off waits for the owner's approval.",
    ),
    ("clinics.claim.resubmissions", "Resubmissions"),
    ("clinics.claim.submission_reference", "Submission reference"),
    ("clinics.claim.submitted_on", "Submitted on"),
    ("clinics.claim.late_reason", "Late reason"),
    (
        "clinics.claim.late_reason_hint",
        "Only for a payer that accepts late claims, after its window.",
    ),
    ("clinics.claim.lines", "Lines"),
    ("clinics.claim.lines_empty", "No line"),
    ("clinics.claim.line_description", "Line"),
    ("clinics.claim.line_code", "Code"),
    ("clinics.claim.line_open", "Open"),
    ("clinics.claim.line_paid", "Paid"),
    ("clinics.claim.payer_code", "Payer's code"),
    ("clinics.claim.attempts", "Submission attempts"),
    ("clinics.claim.attempts_empty", "Not sent yet"),
    ("clinics.claim.attempt_number", "Attempt"),
    ("clinics.claim.attempt_kind", "Kind"),
    ("clinics.claim.attempt_errors", "Channel errors"),
    ("clinics.claim.attempt.submission", "Submission"),
    ("clinics.claim.attempt.resubmission", "Resubmission"),
    ("clinics.claim.answers", "The payer's answers"),
    ("clinics.claim.answers_empty", "The payer has not answered"),
    ("clinics.claim.answer_kind", "Answer"),
    ("clinics.claim.answer_amount", "To credit / taken back"),
    ("clinics.claim.answer.remittance", "Remittance"),
    ("clinics.claim.answer.recoupment", "Recoupment"),
    ("clinics.claim.status.draft", "Draft"),
    ("clinics.claim.status.ready", "Ready"),
    ("clinics.claim.status.submitted", "Submitted"),
    ("clinics.claim.status.paid", "Paid"),
    ("clinics.claim.status.partially_paid", "Partially paid"),
    ("clinics.claim.status.denied", "Denied"),
    ("clinics.claim.status.resubmitted", "Resubmitted"),
    ("clinics.claim.status.written_off", "Written off"),
    ("clinics.claim.status.voided", "Void"),
    ("clinics.claim.validate", "Validate"),
    (
        "clinics.claim.validate_description",
        "Check the codes, the prior authorisation and the payer's window. A validated claim waits for submission.",
    ),
    ("clinics.claim.validated", "Claim validated."),
    ("clinics.claim.submit", "Submit"),
    (
        "clinics.claim.submit_description",
        "Record that the claim was sent through the claims channel. The payer bill is issued with its number.",
    ),
    ("clinics.claim.submitted", "Claim submitted."),
    ("clinics.claim.reject", "Record rejection"),
    (
        "clinics.claim.reject_description",
        "The channel refused the claim before the payer looked at it. It goes back to draft to be fixed, validated and sent again with the same number.",
    ),
    (
        "clinics.claim.reject_errors",
        "Channel errors, one per line as CODE: message",
    ),
    (
        "clinics.claim.rejected",
        "Rejection recorded: the claim is a draft again.",
    ),
    ("clinics.claim.resubmit", "Resubmit"),
    (
        "clinics.claim.resubmit_description",
        "Send the corrected claim to the payer again, within its resubmission window and limit.",
    ),
    ("clinics.claim.corrections", "Corrections"),
    ("clinics.claim.resubmitted", "Claim resubmitted."),
    ("clinics.claim.remit", "Record remittance"),
    (
        "clinics.claim.remit_description",
        "The payer's remittance advice. An amount above what is open on a line waits for a second person's approval.",
    ),
    ("clinics.claim.remittance_recorded", "Remittance recorded."),
    (
        "clinics.claim.remittance_held",
        "The advice pays more than is open on a line: it waits for a second person's approval and nothing is recorded yet.",
    ),
    ("clinics.claim.advice_reference", "Advice reference"),
    ("clinics.claim.advice_date", "Advice date"),
    ("clinics.claim.advice_paid", "paid"),
    ("clinics.claim.advice_denied", "Denied"),
    ("clinics.claim.advice_code", "Denial code"),
    ("clinics.claim.advice_reason", "Reason"),
    ("clinics.claim.recoup", "Record recoupment"),
    (
        "clinics.claim.recoup_description",
        "Money the payer took back. A second person approves it; the recouped lines are open again as denied with the payer's code.",
    ),
    ("clinics.claim.recoup_code", "Recoupment code"),
    ("clinics.claim.recoup_amount", "taken back"),
    ("clinics.claim.recoupment_recorded", "Recoupment recorded."),
    (
        "clinics.claim.recoupment_held",
        "The recoupment waits for a second person's approval and nothing is recorded yet.",
    ),
    (
        "clinics.claim.held.remittance",
        "The remittance :reference waits for a second person's approval. Nothing is recorded until it is given.",
    ),
    (
        "clinics.claim.held.recoupment",
        "The recoupment :reference waits for a second person's approval. Nothing is recorded until it is given.",
    ),
    (
        "clinics.claim.held.write_off",
        "The write-off waits for the owner's approval. The claim stays as it is until it is given.",
    ),
    (
        "clinics.claim.held.ended",
        "The advice that waited for approval ended without a yes (:result). Nothing was recorded.",
    ),
    (
        "clinics.claim.problem.no_patient",
        "The visit has no patient: the claim cannot be made.",
    ),
    (
        "clinics.claim.problem.no_payer",
        "The payer of the claim cannot be found.",
    ),
    (
        "clinics.claim.problem.codes",
        "Every line of the claim needs its service code.",
    ),
    (
        "clinics.claim.problem.authorisation",
        "The payer asks for a prior authorisation for :codes: link one before validating.",
    ),
    (
        "clinics.claim.problem.window",
        "The payer accepted this claim until :date. It cannot be validated now.",
    ),
    (
        "clinics.claim.problem.late_reason",
        "The payer's window ended on :date. The payer accepts late claims: give the late reason.",
    ),
    (
        "clinics.claim.problem.errors",
        "Give at least one channel error with its code and message.",
    ),
    (
        "clinics.claim.problem.remitted",
        "The payer has answered this claim: it cannot be sent back as rejected.",
    ),
    (
        "clinics.claim.problem.corrections",
        "Write down the corrections.",
    ),
    (
        "clinics.claim.problem.resubmit_window",
        "The payer's resubmission window ended on :date.",
    ),
    (
        "clinics.claim.problem.resubmit_limit",
        "The payer accepts :limit resubmissions of a claim and this one has had them.",
    ),
    (
        "clinics.claim.problem.answer",
        "The payer's answer is not recorded on the payer bill.",
    ),
    (
        "clinics.claim.problem.recoup_above",
        "The payer cannot take back more than it paid on a line, and not nothing.",
    ),
    (
        "clinics.claim.problem.recoup_code",
        "Give the payer's recoupment code.",
    ),
    (
        "clinics.claim.problem.held",
        "An advice already waits for a second person's approval on this claim.",
    ),
    (
        "clinics.claim.problem.no_line",
        "A line of the advice is not a line of this claim.",
    ),
    (
        "clinics.claim.problem.close_reason",
        "Give the reason for closing the claim.",
    ),
    (
        "clinics.claim.problem.close_nothing",
        "Nothing is open on this claim.",
    ),
    (
        "clinics.claim.problem.window_open",
        "The payer still accepts this claim through :date. Send it in time, or write it off after that day.",
    ),
    (
        "clinics.claim.problem.window_unknown",
        "The payer contract gives no first-submission window to count, so the claim cannot be shown to have missed it.",
    ),
    (
        "clinics.claim.problem.accepts_late",
        "This payer accepts late claims: validate the claim with a late reason instead of writing it off.",
    ),
    (
        "clinics.claim.problem.void_submitted",
        "This claim was sent to the payer (an attempt is recorded), so it is not voided: resubmit it or write it off.",
    ),
    (
        "clinics.claim.problem.void_code",
        "Give the reason: duplicate, wrong payer or wrong policy.",
    ),
    (
        "clinics.claim.problem.void_duplicate_needed",
        "A duplicate needs the claim it duplicates: pick it from the patient's other claims.",
    ),
    (
        "clinics.claim.problem.void_duplicate_other",
        "Pick a claim of the same patient that is not void.",
    ),
    (
        "clinics.claim.problem.void_waits",
        "The void waits for an approval and cannot go on from here.",
    ),
    (
        "clinics.claim.problem.resplit_other_patient",
        "This policy belongs to another patient.",
    ),
    (
        "clinics.claim.problem.resplit_not_valid",
        "This policy is not valid on the day of the visit.",
    ),
    (
        "clinics.claim.problem.resplit_no_share",
        "This policy gives no payer share for the visit: leave the policy out and resolve the coverage.",
    ),
    (
        "clinics.claim.problem.resplit_patient_share",
        "With this policy the patient's share would change, and it was invoiced already: resolve the coverage with the billing team.",
    ),
    (
        "clinics.claim.problem.resolve_nothing",
        "The visit of this claim has no coverage to resolve.",
    ),
    (
        "clinics.claim.task.coverage_title",
        "Visit :visit has no payer split: coverage to resolve",
    ),
    (
        "clinics.claim.task.coverage_description",
        "The claim :number of visit :visit was voided (:reason) and no right policy was named. Nothing moved to the patient. Find the right policy and split the visit again from the voided claim.",
    ),
    (
        "clinics.claim.task.coverage_resolved",
        "The visit was split again.",
    ),
    (
        "clinics.claim.task.rejected_title",
        "Claim :number was rejected: fix it and send it again",
    ),
    (
        "clinics.claim.task.rejected_description",
        "The claims channel rejected claim :number: :errors. Fix it, validate it and submit it again by :deadline.",
    ),
    (
        "clinics.claim.task.denied_title",
        "Claim :number has denied lines: resubmit, move to the patient or write off",
    ),
    (
        "clinics.claim.task.denied_description",
        "Denied: :lines. Resubmit by :deadline.",
    ),
    (
        "clinics.claim.task.no_deadline",
        "the payer's deadline (none is set on its contract)",
    ),
    // The lot of every consumable issued to a visit (STK.MOV.01, STK.LOT.01).
    ("clinics.lots.title", "Consumables and lots"),
    ("clinics.lots.lot", "Lot"),
    ("clinics.lots.lot_field", "Lot (scanned or picked)"),
    ("clinics.lots.expiry", "Expires"),
    ("clinics.lots.quantity", "Quantity"),
    ("clinics.lots.proposed", "Proposed lot"),
    (
        "clinics.lots.proposed_description",
        "The lot with the earliest expiry at the branch store that is not expired and not on hold. Leave the lot blank to choose it later.",
    ),
    (
        "clinics.lots.proposed_none",
        "No lot is available at the branch store.",
    ),
    ("clinics.lots.code", "Consumable code"),
    ("clinics.lots.propose", "Show the proposed lot"),
    ("clinics.lots.assign", "Set the lot"),
    (
        "clinics.lots.assign_description",
        "Pick or scan the lot of this consumable. A wrong lot is corrected here, before the visit is checked out. Leave blank to take the proposed lot.",
    ),
    ("clinics.lots.line", "Line"),
    ("clinics.lots.assigned", "The lot is set."),
    ("clinics.lots.uncertain", "Lot uncertain"),
    ("clinics.lots.required", "Lot required"),
    ("clinics.lots.issued", "Issued"),
    ("clinics.lots.waiting", "Waiting for stock"),
    (
        "clinics.lots.none_yet",
        "Issued from the store when the visit is checked out.",
    ),
    (
        "clinics.lots.problem.not_available",
        "Lot :lot cannot be issued: it is unknown at this store, on hold or expired.",
    ),
    (
        "clinics.lots.problem.required",
        "Lot required: set the lot of :codes before the visit is checked out.",
    ),
    (
        "clinics.lots.problem.not_offered",
        "Lot no longer available: the lot of :codes is on hold, expired or used up; pick another lot before the visit is checked out.",
    ),
    (
        "clinics.lots.problem.open",
        "The visit is not over yet: the stock is issued when it is checked out.",
    ),
    ("clinics.lots.issue_pending", "Stock issue"),
    ("clinics.lots.issue_done", "Issued"),
    (
        "clinics.lots.issue_waiting",
        "Pending: the stock was not issued",
    ),
    (
        "clinics.lots.issue_unknown_store",
        "Pending: the branch store or the item is not known to the stock",
    ),
    ("clinics.lots.issue_retry", "Issue the stock again"),
    (
        "clinics.lots.issue_retried",
        "The stock issue was tried again.",
    ),
    (
        "clinics.lots.problem.no_line",
        "This is not a consumable of the visit that needs a lot.",
    ),
    (
        "clinics.lots.problem.issued",
        "This consumable was issued already; the lot cannot be edited here.",
    ),
    (
        "clinics.lots.problem.none_available",
        "No lot is available at the branch store.",
    ),
    // A recalled lot traced to patients (CLN.ACL.01, STK.ACL.05).
    ("clinics.recall.title", "Product recall"),
    (
        "clinics.recall.subtitle",
        "The patients who got a recalled product: name, visit day and branch only. Each view is recorded.",
    ),
    ("clinics.recall.patient", "Patient"),
    ("clinics.recall.visit_date", "Visit date"),
    ("clinics.recall.branch", "Branch"),
    ("clinics.recall.notice", "Notice"),
    ("clinics.recall.told", "Told"),
    ("clinics.recall.told_party", "Told through"),
    ("clinics.recall.lot_uncertain", "Lot uncertain"),
    ("clinics.recall.empty", "No visit used this lot."),
    ("clinics.recall.send", "Send the notices"),
    (
        "clinics.recall.send_description",
        "Each patient (or guardian) gets one message listing the visit days. It names the clinic and asks them to call; it never names the product.",
    ),
    ("clinics.recall.sent", "Notices sent."),
    ("clinics.recall.notice.pending", "Not told"),
    ("clinics.recall.notice.sent", "Sent"),
    ("clinics.recall.notice.waived", "Waived"),
    ("clinics.recall.label", "Notice from the clinic"),
    ("clinics.recall.subject", "A message from {{ clinic }}"),
    (
        "clinics.recall.body",
        "{{ clinic }}: please {{ contact }} about a product used in your visit on {{ dates }}.",
    ),
    ("clinics.recall.call_us", "call us"),
    ("clinics.recall.call_us_on", "call us on :phone"),
    (
        "clinics.recall.problem.nothing",
        "Nobody is left to tell: every patient of this recall was told or waived.",
    ),
    (
        "clinics.recall.problem.outcome",
        "Record what came of the call.",
    ),
    ("clinics.recall.calls_title", "Recall calls"),
    (
        "clinics.recall.calls_subtitle",
        "Patients who could not be reached by message. Call them and record the outcome.",
    ),
    ("clinics.recall.calls_empty", "No call is waiting."),
    ("clinics.recall.call_outcome", "What came of the call"),
    ("clinics.recall.call_record", "Record the call"),
    ("clinics.recall.call_recorded", "The call is recorded."),
    ("clinics.recall.call_days", "Visit days"),
    ("clinics.recall.call_gave_up", "Gave up"),
    // The advertising permit of a campaign (CLN.ACL.13).
    ("clinics.permit.title", "Advertising permits"),
    (
        "clinics.permit.subtitle",
        "A campaign that promotes services is activated and sends only with the regulator's permit number and validity recorded and approved by the medical director.",
    ),
    ("clinics.permit.record", "Record a permit"),
    (
        "clinics.permit.record_description",
        "Record the regulator's permit for a campaign, or that the campaign promotes no service. Recording again renews it.",
    ),
    ("clinics.permit.campaign", "Campaign"),
    ("clinics.permit.promotes", "Promotes services"),
    ("clinics.permit.promotes_yes", "Yes"),
    ("clinics.permit.promotes_no", "No"),
    ("clinics.permit.number", "Permit number"),
    ("clinics.permit.valid_to", "Valid to"),
    ("clinics.permit.approved_by", "Approved by"),
    ("clinics.permit.valid", "Valid"),
    ("clinics.permit.empty", "No permit is recorded yet."),
    ("clinics.permit.recorded", "The permit is recorded."),
    (
        "clinics.permit.problem.campaign",
        "Give the number of the campaign.",
    ),
    (
        "clinics.permit.problem.missing",
        "The campaign promotes services and has no advertising permit recorded.",
    ),
    (
        "clinics.permit.problem.no_number",
        "The advertising permit needs its number.",
    ),
    (
        "clinics.permit.problem.no_validity",
        "The advertising permit needs the last day it is valid.",
    ),
    (
        "clinics.permit.problem.lapsed",
        "The advertising permit is no longer valid.",
    ),
    // Late entry after an outage (PLT.ACL.10).
    ("clinics.late.title", "Times from the paper sheet"),
    ("clinics.late.enter", "Enter a time"),
    (
        "clinics.late.enter_description",
        "A check-in, a check-out or a consumable written on paper during an outage: give the time it really happened. A time more than 30 minutes behind needs a reason.",
    ),
    ("clinics.late.kind", "What"),
    ("clinics.late.kind.check_in", "Check-in"),
    ("clinics.late.kind.check_out", "Check-out"),
    ("clinics.late.kind.consumable", "Consumable"),
    ("clinics.late.time", "Time"),
    ("clinics.late.entered_at", "Entered"),
    ("clinics.late.flag", "Late entry"),
    ("clinics.late.reason", "Reason"),
    (
        "clinics.late.reason_hint",
        "Why it is entered late, for example the outage.",
    ),
    ("clinics.late.line", "Consumable line (for a consumable)"),
    ("clinics.late.entered", "The time is entered."),
    (
        "clinics.late.sent_for_approval",
        "The time is sent for approval.",
    ),
    (
        "clinics.late.problem.time",
        "Give the time as date and hour.",
    ),
    (
        "clinics.late.problem.kind",
        "Choose a check-in, a check-out or a consumable.",
    ),
    (
        "clinics.late.problem.future",
        "A time in the future cannot be entered.",
    ),
    (
        "clinics.late.problem.not_completed",
        "A check-out time is entered for a visit that is completed.",
    ),
    (
        "clinics.late.problem.line",
        "Choose a consumable line of this visit.",
    ),
    // The call list while the SMS or WhatsApp connection is down (05 section 8).
    ("clinics.call_list.title", "Appointments to call"),
    (
        "clinics.call_list.subtitle",
        "While SMS or WhatsApp is down: the appointments of the next 24 hours, per branch. Phone each patient and tick the row.",
    ),
    ("clinics.call_list.phone", "Mobile"),
    ("clinics.call_list.starts", "Starts"),
    ("clinics.call_list.called", "Called"),
    ("clinics.call_list.tick", "Called"),
    (
        "clinics.call_list.tick_message",
        "Tick this appointment as called?",
    ),
    (
        "clinics.call_list.ticked",
        "The appointment is ticked as called.",
    ),
    (
        "clinics.call_list.empty",
        "Nobody to call: SMS and WhatsApp are working, or nothing starts in the next 24 hours.",
    ),
    ("clinics.call_list.since", "The connection went down at"),
    (
        "clinics.call_list.problem.none_down",
        "SMS and WhatsApp are working: there is no list to tick.",
    ),
    (
        "clinics.call_list.problem.not_listed",
        "This appointment is not on the call list.",
    ),
    // The recall invitation (CLN.RUL.02).
    ("clinics.recall_invite.subject", "Time for your next visit"),
    (
        "clinics.recall_invite.body",
        "It is time to book your next visit. Call us or use the link to choose a time.",
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
