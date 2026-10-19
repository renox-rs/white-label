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
    // The renewal window and the notice deadline (PRP.RUL.01, PRP.RUL.09).
    (
        "property.notices.renewal_due_subject",
        "Lease {{ lease }} is due for renewal",
    ),
    (
        "property.notices.renewal_due_body",
        "The lease {{ lease }} ends on {{ end_date }}. Record the renewal decision now: a notice of changed terms must reach the tenant by {{ deadline }}.",
    ),
    (
        "property.notices.renewal_deadline_subject",
        "Notice deadline of lease {{ lease }} in {{ days }} days",
    ),
    (
        "property.notices.renewal_deadline_body",
        "The notice of changed terms of {{ lease }} is due by {{ deadline }}, {{ days }} days from now. {{ problem }}",
    ),
    (
        "property.notices.current_terms_only_subject",
        "Lease {{ lease }} can be renewed on the current terms only",
    ),
    (
        "property.notices.current_terms_only_body",
        "The notice deadline of {{ lease }} was {{ deadline }} and no timely notice of changed terms was delivered. The lease can be renewed on its current terms only, unless the tenant signs the new ones.",
    ),
    (
        "property.notices.renewal_window_subject",
        "Your tenancy {{ lease }} is coming up for renewal",
    ),
    (
        "property.notices.renewal_window_body",
        "Your tenancy {{ lease }} ends on {{ end_date }}. Your landlord will contact you about the renewal.",
    ),
    (
        "property.notices.changed_terms_subject",
        "Notice of changed terms for tenancy {{ lease }}",
    ),
    (
        "property.notices.changed_terms_body",
        "Your landlord gives notice that the terms of tenancy {{ lease }} will change when it is renewed after {{ end_date }}. Please contact your property manager for the new terms.",
    ),
    (
        "property.notices.holding_over_subject",
        "Lease {{ lease }} is holding over",
    ),
    (
        "property.notices.holding_over_body",
        "The lease {{ lease }} ended on {{ end_date }} and the tenant stays. It renews on the same terms and billing continues until {{ until }}. Issue the renewal version or record a notice.",
    ),
    // Holding over (PRP.RUL.03, PRP.RUL.14).
    ("property.holding.line", ":description (holding over)"),
    ("property.holding.not_over", "The end date of the lease has not passed."),
    (
        "property.holding.renewal_active",
        "A renewal version is active, so the lease is not holding over.",
    ),
    ("property.holding.until", "Holding over until"),
    (
        "property.holding.task_title",
        "Issue the renewal of :lease, held over after its end date, or record a notice",
    ),
    (
        "property.holding.task_description",
        "The tenant stays after the end date, so the lease renews on the same terms and is billed until :until. Issue the renewal version on the same terms and renew Ejari, or record a notice.",
    ),
    ("property.holding.task_done", "A renewal version was submitted."),
    ("property.renewal.title", "Renewal"),
    ("property.renewal.notice_deadline", "Notice deadline"),
    ("property.renewal.decision", "Renewal decision"),
    ("property.renewal.decision_undecided", "Undecided"),
    ("property.renewal.decision_same_terms", "Same terms"),
    ("property.renewal.decision_changed_terms", "Changed terms"),
    ("property.renewal.decision_not_renewing", "Not renewing"),
    ("property.renewal.decide", "Set the renewal decision"),
    (
        "property.renewal.decide_description",
        "Same terms, changed terms or not renewing. Changed terms need a notice to the tenant before the notice deadline.",
    ),
    ("property.renewal.decided", "Renewal decision saved."),
    ("property.renewal.decision_refused", "Only a lease in force has a renewal decision."),
    ("property.renewal.notice", "Notice of changed terms"),
    ("property.renewal.notice_not_sent", "Not sent"),
    ("property.renewal.notice_undelivered", "Sent on :sent, not delivered"),
    ("property.renewal.notice_delivered", "Delivered on :delivered"),
    ("property.renewal.send_notice", "Send the notice of changed terms"),
    (
        "property.renewal.send_notice_message",
        "The tenant gets the notice of changed terms. It counts once its delivery is proved.",
    ),
    ("property.renewal.notice_sent", "Notice of changed terms sent."),
    (
        "property.renewal.notice_needs_decision",
        "Record the decision changed terms before sending the notice.",
    ),
    ("property.renewal.notice_refused", "Only a lease in force can be sent a notice of changed terms."),
    ("property.renewal.notice_already_delivered", "The notice was delivered already."),
    ("property.renewal.current_terms_only", "Current terms only"),
    (
        "property.renewal.current_terms_only_yes",
        "Current terms only, unless the tenant signs the new ones",
    ),
    ("property.renewal.proof_receipt", "Provider receipt"),
    ("property.renewal.proof_recorded", "Proof recorded by staff"),
    // Changed terms against the tenant (PRP.ACL.17).
    (
        "property.terms.notice_late",
        "Changed terms against the tenant: notice not timely (deadline :deadline), it was delivered on :delivered. Renew on the current terms, or attach the tenant's signed acceptance of the changed terms.",
    ),
    (
        "property.terms.notice_missing",
        "Changed terms against the tenant: notice not delivered (deadline :deadline), no proof of delivery is recorded. Renew on the current terms, or attach the tenant's signed acceptance of the changed terms.",
    ),
    (
        "property.terms.amendment_needs_acceptance",
        "Changed terms against the tenant in an amendment need the tenant's signed acceptance attached to the draft, whatever notice was given.",
    ),
    ("property.terms.answer", "Answer: against the tenant?"),
    (
        "property.terms.answer_description",
        "Other terms differ from the current version. Say whether they are against the tenant; a no needs a reason, which is kept in the history.",
    ),
    ("property.terms.against_question", "Changes terms against the tenant?"),
    ("property.terms.yes", "Yes"),
    ("property.terms.no", "No"),
    ("property.terms.reason", "Reason for no"),
    ("property.terms.reason_hint", "For example a typo fix."),
    ("property.terms.answered", "Answer saved."),
    ("property.terms.answer_draft_only", "Only a draft renewal or amendment has this answer."),
    ("property.terms.reason_required", "A no needs a reason."),
    ("property.terms.not_against", "No"),
    ("property.terms.against_found", "Yes, found in the rent, the deposit or the cheques"),
    ("property.terms.against_asked", "Yes, until answered no with a reason"),
    ("property.terms.acceptance", "Tenant's signed acceptance"),
    ("property.terms.acceptance_attached", "Attached"),
    ("property.terms.acceptance_missing", "Not attached"),
    ("property.renewal.problem_undecided", "No renewal decision is recorded."),
    (
        "property.renewal.problem_undelivered",
        "The notice of changed terms was not delivered (notice not delivered).",
    ),
    // Renewing a lease without billing a period twice (BL-PRP-07).
    (
        "property.renew.no_renewal",
        "No renewal version of this lease is active.",
    ),
    (
        "property.renew.differences_title",
        "Periods already invoiced before the renewal started",
    ),
    ("property.renew.period", "Period from"),
    ("property.renew.invoiced", "Invoiced"),
    ("property.renew.renewal", "Renewal charges"),
    ("property.renew.difference", "Difference"),
    (
        "property.renew.differences_toast",
        "Activated. The renewal charges another rent for periods already invoiced: see the table on this page and add a charge or a credit by hand.",
    ),
    (
        "property.tasks.relink_cheques",
        "Relink the cheques of :lease to the charges of the renewal",
    ),
    // Recording a notice to end a lease (BL-PRP-09).
    (
        "property.notices.landlord_notice_subject",
        "Notice to end tenancy {{ lease }}",
    ),
    (
        "property.notices.landlord_notice_body",
        "Your landlord gives notice, dated {{ notice_date }}, that tenancy {{ lease }} (ending {{ end_date }}) will be ended. {{ ground }}",
    ),
    ("property.notice.record", "Record a notice"),
    (
        "property.notice.record_description",
        "The tenant's or the landlord's notice to end the lease. A tenant's notice needs the planned move-out date. A landlord's notice is sent to the tenant as a statutory notice.",
    ),
    ("property.notice.recorded", "Notice recorded."),
    ("property.notice.date", "Notice date"),
    ("property.notice.party", "Given by"),
    ("property.notice.party_tenant", "The tenant"),
    ("property.notice.party_landlord", "The landlord"),
    ("property.notice.reason", "Reason"),
    ("property.notice.reason_end_of_term", "End of term"),
    ("property.notice.reason_early_termination", "Early termination"),
    ("property.notice.reason_landlords_ground", "Landlord's ground"),
    ("property.notice.reason_non_payment", "Non-payment"),
    ("property.notice.ground", "Landlord's ground, as text"),
    ("property.notice.planned_move_out", "Planned move-out date"),
    ("property.notice.penalty", "Early termination penalty (AED)"),
    (
        "property.notice.penalty_hint",
        "From the contract; added as a charge due on the planned move-out date.",
    ),
    ("property.notice.title", "Notice to end the lease"),
    ("property.notice.not_delivered", "Notice not delivered"),
    ("property.notice.delivered_on", "Delivered on :day"),
    ("property.notice.sent_on", "Sent on :day"),
    ("property.notice.party_required", "Say who gave the notice: the tenant or the landlord."),
    (
        "property.notice.reason_required",
        "Give the reason: end of term, early termination, the landlord's ground or non-payment.",
    ),
    ("property.notice.date_required", "Give the date of the notice."),
    (
        "property.notice.planned_required",
        "A tenant's notice needs the planned move-out date.",
    ),
    (
        "property.notice.planned_before",
        "The planned move-out date cannot be before the notice date.",
    ),
    ("property.notice.penalty_negative", "The penalty cannot be negative."),
    (
        "property.notice.penalty_not_allowed",
        "A penalty belongs to a tenant's notice with the reason early termination.",
    ),
    (
        "property.notice.charge_type_missing",
        "Configure the charge type first: the early termination fee is not in the charge catalogue.",
    ),
    ("property.notice.not_running", "A notice can be recorded on a lease in force only."),
    (
        "property.notice.planned_edit_refused",
        "Only the planned move-out date of a tenant's notice can be changed.",
    ),
    ("property.notice.planned_changed", "Planned move-out date changed."),
    ("property.notice.change_planned", "Change the planned move-out date"),
    (
        "property.notice.penalty_line",
        "Early termination fee, :lease",
    ),
    (
        "property.notice.ticket_title",
        "Move-out inspection of :lease",
    ),
    (
        "property.notice.ticket_description",
        "The tenant moves out on :day. Inspect the unit, collect the keys and record the inspection on the lease.",
    ),
    (
        "property.notice.ticket_description_open",
        "Inspect the unit when the tenant moves out, collect the keys and record the inspection on the lease. The move-out date follows the proof of delivery of the landlord's notice.",
    ),
    ("property.charge_types.required", "Give the charge type a code and a name."),
    // Withdrawing a notice (BL-PRP-10, PRP.ACL.20).
    ("property.withdraw.action", "Withdraw the notice"),
    (
        "property.withdraw.message",
        "The lease goes back to the state its dates give and the unit to occupied. The landlord side's written agreement must be attached to the lease.",
    ),
    ("property.withdraw.done", "Notice withdrawn."),
    (
        "property.withdraw.agreement_missing",
        "Attach the written agreement of the landlord side to the lease first (PRP.ACL.20).",
    ),
    (
        "property.withdraw.inspection_done",
        "The move-out inspection was completed: the notice cannot be withdrawn.",
    ),
    (
        "property.withdraw.other_tenant",
        "A lease for another tenant on this unit was submitted: the notice cannot be withdrawn (PRP.ACL.20).",
    ),
    (
        "property.withdraw.wrong_target",
        "By its dates the lease goes back to :state.",
    ),
    ("property.withdraw.reason", "Notice withdrawn with the written agreement of the landlord side"),
    ("property.withdraw.ticket_note", "Cancelled: the notice was withdrawn."),
    // Terminating a lease (BL-PRP-11).
    ("property.inspection.record", "Record the move-out inspection"),
    (
        "property.inspection.description",
        "When the inspection is completed and the keys are back, the lease can be terminated. Dates you leave out keep what was recorded.",
    ),
    ("property.inspection.done_on", "Inspection completed on"),
    ("property.inspection.keys_returned_on", "All keys returned on"),
    ("property.inspection.defects", "Defects that need work"),
    ("property.inspection.recorded", "Move-out inspection recorded."),
    ("property.inspection.no_notice", "Only a lease with a notice has a move-out inspection."),
    ("property.inspection.nothing", "Give the date of the inspection or of the keys."),
    ("property.inspection.ticket_note", "The move-out inspection was recorded on the lease."),
    ("property.terminate.action", "Terminate the lease"),
    (
        "property.terminate.description",
        "The tenant has moved out. Charges after the move-out date are cancelled, the invoices of later periods are credited pro rata, later cheques are returned and the deposit settlement starts.",
    ),
    ("property.terminate.move_out", "Move-out date (keys returned)"),
    ("property.terminate.rent_due_until", "Rent due until"),
    (
        "property.terminate.rent_due_until_hint",
        "Only when the contract keeps rent due after move-out: charges and cheques up to this date stay.",
    ),
    ("property.terminate.credit", "The credit that is left"),
    ("property.terminate.credit_refund", "Refund it to the tenant"),
    ("property.terminate.credit_keep", "Keep it as credit"),
    ("property.terminate.credited", "Credited for later periods"),
    ("property.terminate.done", "Lease terminated."),
    ("property.terminate.history", "Tenant moved out"),
    (
        "property.terminate.inspection_missing",
        "Complete the move-out inspection first.",
    ),
    ("property.terminate.keys_missing", "All the keys must be returned first."),
    ("property.terminate.move_out_required", "Give the move-out date."),
    (
        "property.terminate.move_out_early",
        "The move-out date cannot be before the lease started.",
    ),
    (
        "property.terminate.due_until_early",
        "Rent cannot be due until a day before the move-out date.",
    ),
    ("property.terminate.reason", "Lease :lease ended: the tenant moved out"),
    (
        "property.terminate.draft_straddles",
        "Invoice :invoice is a draft for a period that straddles the move-out date: adjust it by hand.",
    ),
    (
        "property.terminate.task_problems",
        "Finish the termination of :lease by hand",
    ),
    ("property.ejari.cancel", "Record the Ejari cancellation"),
    (
        "property.ejari.cancel_description",
        "The registration was cancelled after move-out.",
    ),
    ("property.ejari.cancelled_on", "Ejari cancelled on"),
    ("property.ejari.cancelled_done", "Ejari cancellation recorded."),
    (
        "property.ejari.cancel_refused",
        "Only a terminated lease with a registration number has an Ejari cancellation.",
    ),
    ("property.ejari.cancelled", "The Ejari cancellation was recorded."),
    ("property.ejari.cancel_task", "Cancel the Ejari contract of :lease"),
    (
        "property.ejari.cancel_task_description",
        "The lease ended :days days ago and the registration :number is not cancelled. Cancel it and record the date on the lease.",
    ),
    (
        "property.notices.ejari_cancellation_subject",
        "Cancel the Ejari contract of {{ lease }}",
    ),
    (
        "property.notices.ejari_cancellation_body",
        "The lease {{ lease }} ended on {{ terminated_on }} and its registration {{ ejari_number }} is not cancelled yet. Cancel it and record the date on the lease.",
    ),
    // The legal case flag (BL-PRP-13).
    ("property.legal.open", "Legal case open"),
    ("property.legal.title", "Legal cases"),
    ("property.legal.set", "Set a legal case"),
    (
        "property.legal.set_description",
        "While a legal case is open, the reminders on the invoices of the lease pause, a renewal or an amendment cannot be submitted and the tenant gets no automatic message. A ticket of the category legal case is linked to it.",
    ),
    ("property.legal.set_done", "Legal case set."),
    ("property.legal.type", "Case"),
    ("property.legal.type_arrears", "Arrears"),
    ("property.legal.type_eviction", "Eviction"),
    ("property.legal.type_deposit_dispute", "Deposit dispute"),
    ("property.legal.type_other", "Other"),
    ("property.legal.reference", "Rental Dispute Centre or court reference"),
    (
        "property.legal.reference_hint",
        "Can wait while the case is prepared; needed to clear it with a judgment.",
    ),
    ("property.legal.ticket", "Ticket"),
    ("property.legal.ticket_new", "A new ticket"),
    ("property.legal.status_open", "Open"),
    ("property.legal.opened_on", "Set on"),
    ("property.legal.cleared_on", "Cleared on"),
    ("property.legal.clear", "Clear a legal case"),
    (
        "property.legal.clear_description",
        "Give the outcome. Its ticket is resolved if it is still open; reminders resume from the next scheduled step.",
    ),
    ("property.legal.clear_done", "Legal case cleared."),
    ("property.legal.case", "Legal case"),
    ("property.legal.outcome", "Outcome"),
    ("property.legal.outcome_paid", "Paid"),
    ("property.legal.outcome_settled", "Settled"),
    ("property.legal.outcome_judgment", "Judgment"),
    ("property.legal.outcome_eviction_executed", "Eviction executed"),
    ("property.legal.note", "Note"),
    ("property.legal.outcome_required", "Give the outcome: paid, settled or judgment."),
    (
        "property.legal.reference_required",
        "A judgment needs the case reference: record it first.",
    ),
    ("property.legal.cleared_already", "This legal case is cleared already."),
    (
        "property.legal.not_started",
        "A lease that never started has no legal case.",
    ),
    (
        "property.legal.ticket_unavailable",
        "Pick an open legal case ticket of this lease that has no flag yet.",
    ),
    ("property.legal.ticket_title", "Legal case (:type) on :lease"),
    (
        "property.legal.ticket_description",
        "The legal case flag of :lease. Follow the case and clear the flag with its outcome.",
    ),
    ("property.legal.ticket_resolved", "The legal case was cleared: :outcome."),
    // Amending an active lease (BL-PRP-08).
    (
        "property.amend.no_amendment",
        "No active amendment of this lease was found.",
    ),
    (
        "property.amend.line_old",
        "Rent :from to :to (previous terms) · :lease",
    ),
    (
        "property.amend.line_new",
        "Rent :from to :to (amended terms) · :lease",
    ),
    ("property.amend.credit_reason", "Lease :lease amended"),
    (
        "property.amend.cheque_reason",
        "Lease :lease amended: the amounts changed",
    ),
    (
        "property.amend.task_cheques",
        "Collect replacement cheques for :lease",
    ),
    (
        "property.amend.task_cheques_description",
        ":count cheques were handed back against receipt :receipt because the amounts changed. Collect the replacements.",
    ),
    (
        "property.amend.task_problems",
        "Finish the amendment of :lease by hand",
    ),
    (
        "property.amend.credit_left",
        ":amount of the payments taken off the credited invoices stays as the tenant's credit.",
    ),
    ("property.amend.top_up_line", "Deposit top-up · :lease"),
    // Forcing the end of a lease (BL-PRP-12).
    ("property.force.action", "Force terminate"),
    ("property.force.submit", "Request approval"),
    (
        "property.force.description",
        "The tenant abandoned the unit or was evicted. Another user with the right to approve forced terminations decides; the lease stays as it is until then.",
    ),
    ("property.force.reason", "Reason"),
    ("property.force.reason_abandoned", "The unit was abandoned"),
    ("property.force.reason_eviction_executed", "The eviction was executed"),
    ("property.force.reason_required", "Choose the reason: abandoned or eviction executed."),
    ("property.force.move_out", "Move-out date"),
    (
        "property.force.move_out_hint",
        "The abandonment inspection date, or the execution date of the eviction.",
    ),
    ("property.force.move_out_required", "Give the move-out date."),
    ("property.force.basis", "Legal basis for repossession"),
    (
        "property.force.basis_hint",
        "For an abandoned unit: the document or reference that allows repossession. Attach it to the lease too.",
    ),
    (
        "property.force.basis_required",
        "State the legal basis for repossessing the abandoned unit.",
    ),
    ("property.force.keep", "Cheques to keep as evidence"),
    (
        "property.force.keep_hint",
        "Cheque numbers of the register, separated by commas. Later cheques not named are kept while a legal case is open, else returned.",
    ),
    ("property.force.hand_back", "Cheques to return"),
    (
        "property.force.cheque_unknown",
        "A cheque named here was not received with this lease.",
    ),
    (
        "property.force.missing_judgment",
        "Judgment missing: attach the court judgment to the lease.",
    ),
    (
        "property.force.missing_execution_record",
        "Execution record missing: attach the execution report of the eviction to the lease.",
    ),
    (
        "property.force.missing_abandonment_inspection",
        "Abandonment inspection missing: attach the inspection with its photos to the lease.",
    ),
    (
        "property.force.missing_repossession_basis",
        "Repossession basis missing: attach the legal basis for repossession to the lease.",
    ),
    ("property.force.history_abandoned", "The tenant abandoned the unit"),
    ("property.force.history_eviction_executed", "The eviction was executed"),
    (
        "property.force.sent",
        "Sent for approval. The lease stays as it is until it is approved.",
    ),
    ("property.force.done", "Lease terminated."),
    (
        "property.force.evidence_note",
        "Held as evidence: lease :lease was ended by force",
    ),
    // Escalating arrears (BL-PRP-14).
    (
        "property.notices.payment_demand_subject",
        "Formal payment demand for {{ lease }}",
    ),
    (
        "property.notices.payment_demand_body",
        "Rent is overdue on tenancy {{ lease }}: {{ invoices }}. The total open is {{ total }}. Pay it within {{ days }} days of receiving this demand; otherwise the landlord may refer the matter to the Rental Dispute Centre.",
    ),
    (
        "property.arrears.demand_task",
        "Send the formal payment demand for :lease",
    ),
    (
        "property.arrears.demand_task_description",
        "Invoices overdue by :days days or more: :list. Send the statutory notice from the lease; the demand period starts when its delivery is proven.",
    ),
    ("property.arrears.demand_added", "Added to the demand: :list"),
    ("property.arrears.nothing_left", "Nothing is overdue any more: the demand is not needed."),
    ("property.arrears.demand_sent", "The formal payment demand was sent."),
    (
        "property.arrears.not_open",
        "This demand is sent or closed already.",
    ),
    (
        "property.arrears.legal_task",
        "Open a legal case or agree a payment plan for :lease",
    ),
    (
        "property.arrears.legal_task_description",
        "The demand period of :days days has passed and these invoices are still unpaid: :list. Open a legal case on the lease, or record a payment plan with the agreed dates.",
    ),
    (
        "property.arrears.plan_agreed",
        "A payment plan was agreed until :until.",
    ),
    ("property.arrears.plan_note_required", "Write the agreed dates."),
    ("property.arrears.plan_date_past", "The latest agreed date cannot be in the past."),
    ("property.arrears.title", "Arrears"),
    ("property.arrears.send", "Send the formal payment demand"),
    (
        "property.arrears.send_description",
        "Sends the statutory notice to the tenant, listing the invoices overdue by 30 days or more. The list is frozen once it is sent.",
    ),
    ("property.arrears.demand", "Demand"),
    ("property.arrears.sent_done", "Formal payment demand sent."),
    ("property.arrears.plan", "Record a payment plan"),
    (
        "property.arrears.plan_description",
        "A note with the agreed dates. The invoices are not changed; reminders and escalation stay quiet until the latest agreed date.",
    ),
    ("property.arrears.plan_note", "The agreed dates"),
    ("property.arrears.plan_latest", "Latest agreed date"),
    ("property.arrears.plan_done", "Payment plan recorded."),
    ("property.arrears.demand_open", "Waiting to be sent"),
    ("property.arrears.demand_sent_on", "Sent on :day"),
    ("property.arrears.demand_closed", "Closed"),
    ("property.arrears.delivered", "Delivered on :day"),
    ("property.arrears.not_delivered", "Delivery not proven"),
    ("property.arrears.invoices", "Invoices"),
    ("property.arrears.status", "Demand"),
    ("property.arrears.plan_until", "Payment plan until"),
    // The window to dispute the deposit deductions (BL-PRP-16).
    (
        "property.notices.deposit_ready_subject",
        "The deposit of {{ lease }} may be settled",
    ),
    (
        "property.notices.deposit_ready_body",
        "Version {{ version }} of the deductions of {{ lease }}, published on {{ published_on }}, was not disputed. The settlement may proceed.",
    ),
    (
        "property.notices.deposit_dispute_unanswered_subject",
        "Deposit dispute of {{ lease }} not answered",
    ),
    (
        "property.notices.deposit_dispute_unanswered_body",
        "The tenant of {{ lease }} disputed the deposit deductions on {{ opened_on }} and nobody has answered for {{ days }} days. Answer it or agree the lines with the tenant.",
    ),
    ("property.force.case_executed", "The eviction was executed."),
    (
        "property.force.case_abandoned",
        "The unit was abandoned and the lease ended by force.",
    ),
    // Management agreements and owner statements (BL-PRP-19).
    ("property.agreements.no_owner", "The owner is not a party."),
    (
        "property.agreements.fee_invalid",
        "The fee must be between 0% and 100%, and the fixed fee cannot be negative.",
    ),
    ("property.agreements.dates_invalid", "The agreement cannot end before it starts."),
    ("property.agreements.no_unit", "The unit was not found."),
    (
        "property.agreements.unit_taken",
        "The unit is under another management agreement on that day.",
    ),
    ("property.statements.title", "Owner statements"),
    (
        "property.statements.subtitle",
        "What was collected for each owner, the costs and the management fee.",
    ),
    ("property.statements.empty", "No owner statements yet"),
    ("property.statements.number", "Statement"),
    ("property.statements.owner", "Owner"),
    ("property.statements.agreement", "Agreement"),
    ("property.statements.period", "Period"),
    ("property.statements.brought_forward", "Brought forward"),
    ("property.statements.collected", "Collected"),
    ("property.statements.expenses", "Expenses"),
    ("property.statements.fee", "Management fee"),
    ("property.statements.adjustments", "Adjustments"),
    ("property.statements.balance", "Balance"),
    ("property.statements.prepared_by", "Prepared by"),
    ("property.statements.approved_by", "Approved by"),
    ("property.statements.published_on", "Published on"),
    ("property.statements.return_reason", "Returned because"),
    ("property.statements.revise_reason", "Revised because"),
    ("property.statements.system", "Generated by the system"),
    ("property.statements.status.draft", "Draft"),
    ("property.statements.status.approved", "Approved"),
    ("property.statements.status.published", "Published"),
    ("property.statements.status.settled", "Settled"),
    ("property.statements.status.superseded", "Superseded"),
    ("property.statements.lines", "Lines"),
    ("property.statements.lines_empty", "No lines"),
    ("property.statements.line_description", "Description"),
    ("property.statements.line_amount", "Amount"),
    ("property.statements.line_kind", "Kind"),
    ("property.statements.line_direction", "Direction (adjustments only)"),
    ("property.statements.direction_credit", "In favour of the owner"),
    ("property.statements.direction_debit", "Against the owner"),
    ("property.statements.kind.collected", "Rent collected"),
    ("property.statements.kind.expense", "Expense"),
    ("property.statements.kind.fee", "Management fee"),
    ("property.statements.kind.adjustment", "Adjustment"),
    ("property.statements.kind.written_off", "Rent written off"),
    ("property.statements.kind.recovered", "Recovered after write-off"),
    ("property.statements.add_line", "Add a line"),
    ("property.statements.add_line_description", "A cost or a correction on the draft. You become its preparer."),
    ("property.statements.line_added", "Line added."),
    ("property.statements.remove_line", "Remove a line"),
    ("property.statements.line_removed", "Line removed."),
    ("property.statements.reason", "Reason"),
    ("property.statements.approve", "Approve"),
    ("property.statements.approve_message", "Approve the statement for the owner."),
    ("property.statements.return", "Return"),
    ("property.statements.return_description", "Sends the statement back to its preparer."),
    ("property.statements.publish", "Publish"),
    ("property.statements.publish_message", "Send the statement to the owner."),
    ("property.statements.revise", "Revise"),
    ("property.statements.revise_description", "Replaces the published statement with a corrected draft. Possible until a payout is recorded."),
    ("property.statements.done", "Done."),
    ("property.statements.fee_line", "Management fee :percent%"),
    (
        "property.statements.preparer",
        "The user who prepared the statement cannot approve it.",
    ),
    (
        "property.statements.not_draft",
        "Only a draft can be edited: return the statement first.",
    ),
    (
        "property.statements.line_invalid",
        "Give the line a description, an amount other than 0 and a kind that is entered by hand.",
    ),
    (
        "property.statements.paid_out",
        "A payout was recorded: the statement cannot be revised.",
    ),
    (
        "property.notices.owner_statement_draft_subject",
        "Draft owner statement {{ statement }}",
    ),
    (
        "property.notices.owner_statement_draft_body",
        "The draft statement {{ statement }} for {{ period }} was generated; the balance is {{ balance }}. Check it and add the costs.",
    ),
    (
        "property.notices.owner_statement_returned_subject",
        "Owner statement {{ statement }} returned",
    ),
    (
        "property.notices.owner_statement_returned_body",
        "The property manager returned {{ statement }} for {{ period }}: {{ reason }}",
    ),
    (
        "property.notices.owner_statement_subject",
        "Your statement {{ statement }}",
    ),
    (
        "property.notices.owner_statement_body",
        "Statement {{ statement }} for {{ period }}. Brought forward {{ brought_forward }}; collected {{ collected }}; expenses {{ expenses }}; management fee {{ fee }}; adjustments {{ adjustments }}; balance {{ balance }}. Lines: {{ lines }}",
    ),
    (
        "property.notices.owner_statement_revised_subject",
        "Statement {{ statement }} will be corrected",
    ),
    (
        "property.notices.owner_statement_revised_body",
        "We found a correction to statement {{ statement }} for {{ period }}. A corrected statement (revision {{ revision }}) follows.",
    ),
    // Writing off rent with the owner's consent (BL-PRP-18).
    ("property.write_off.action", "Write off an invoice"),
    (
        "property.write_off.description",
        "For a managed unit the owner's consent is needed. A property manager other than you approves the write-off.",
    ),
    ("property.write_off.invoice", "Invoice"),
    ("property.write_off.reason_code", "Reason"),
    ("property.write_off.reason_text", "Explanation (needed for other)"),
    ("property.write_off.consent_note", "Owner's consent (what was agreed, or where the e-mail or letter is kept)"),
    ("property.write_off.consent_by", "Given by (the owner's contact)"),
    ("property.write_off.consent_on", "Given on"),
    ("property.write_off.submit", "Request the write-off"),
    ("property.write_off.requested", "The write-off waits for approval."),
    ("property.write_off.done", "The invoice was written off."),
    ("property.write_off.reason.insolvent", "Insolvent"),
    ("property.write_off.reason.untraceable", "Untraceable"),
    ("property.write_off.reason.uneconomic_to_pursue", "Uneconomic to pursue"),
    ("property.write_off.reason.settlement_agreed", "Settlement agreed"),
    ("property.write_off.reason.other", "Other"),
    (
        "property.write_off.consent_required",
        "The unit is managed for an owner: record the owner's consent to the write-off first.",
    ),
    (
        "property.write_off.consent_invalid",
        "The owner's consent needs a note, the owner's contact who gave it, and a date that is not in the future.",
    ),
    // Disputing a statement line (BL-PRP-21).
    ("property.disputes.title", "Disputes"),
    ("property.disputes.flag", "Disputed"),
    ("property.disputes.open", "Record a dispute"),
    (
        "property.disputes.open_description",
        "The owner disputes a line. The statement is flagged, a ticket is opened and no payout is made until it is cleared.",
    ),
    ("property.disputes.line", "Line"),
    ("property.disputes.whole", "The statement as a whole"),
    ("property.disputes.comment", "What the owner says"),
    ("property.disputes.opened", "The dispute was recorded."),
    ("property.disputes.clear", "Clear a dispute"),
    (
        "property.disputes.clear_description",
        "Revise the statement instead to clear every dispute at once. The owner is told the outcome by e-mail.",
    ),
    ("property.disputes.dispute", "Dispute"),
    ("property.disputes.outcome", "Outcome"),
    ("property.disputes.outcome_upheld", "The statement stands"),
    ("property.disputes.outcome_adjusted", "An adjustment line goes on the next statement"),
    ("property.disputes.outcome_reallocated", "The cost is moved between the owners on their next statements"),
    ("property.disputes.outcome_revised", "The statement is revised"),
    ("property.disputes.amount", "Amount of the adjustment"),
    ("property.disputes.direction", "Effect on this owner's balance"),
    ("property.disputes.credit", "In favour of this owner"),
    ("property.disputes.debit", "Against this owner"),
    ("property.disputes.other_agreement", "Other owner's agreement (number)"),
    ("property.disputes.note", "Note"),
    ("property.disputes.cleared", "The dispute was cleared."),
    ("property.disputes.status", "Status"),
    ("property.disputes.status_open", "Open"),
    ("property.disputes.status_cleared", "Cleared"),
    ("property.disputes.ticket_title", "Owner statement dispute: :statement"),
    ("property.disputes.ticket_resolved", "Dispute cleared: :outcome"),
    (
        "property.disputes.adjustment_line",
        "Adjustment of :statement: :what",
    ),
    (
        "property.disputes.not_disputable",
        "Only a published statement, or a settled one after its payout, can be disputed.",
    ),
    ("property.disputes.comment_required", "Write what the owner disputes."),
    ("property.disputes.cleared_already", "The dispute is cleared already."),
    ("property.disputes.amount_required", "Give the amount of the adjustment."),
    (
        "property.disputes.other_agreement_invalid",
        "The other agreement must be another agreement that reports to its owner.",
    ),
    (
        "property.notices.owner_statement_dispute_outcome_subject",
        "Your question about statement {{ statement }}",
    ),
    (
        "property.notices.owner_statement_dispute_outcome_body",
        "About statement {{ statement }} for {{ period }}: {{ outcome }}.",
    ),
    // Paying out, carrying forward or invoicing a balance (BL-PRP-20).
    ("property.payout.pay_out", "Pay out"),
    (
        "property.payout.pay_out_message",
        "Ask to pay the owner the whole balance. A property manager other than you approves it.",
    ),
    ("property.payout.block_exception", "Ask for an exception"),
    ("property.payout.carry_forward", "Carry forward"),
    (
        "property.payout.carry_forward_message",
        "The balance is brought forward on the next statement.",
    ),
    ("property.payout.invoice_owner", "Invoice the owner"),
    (
        "property.payout.invoice_owner_message",
        "Invoice the negative balance to the owner from the management company. A property manager other than you approves it.",
    ),
    ("property.payout.asked", "The payout waits for approval."),
    ("property.payout.paid", "The owner is paid."),
    ("property.payout.exception_asked", "The owner is on hold: an exception was requested."),
    ("property.payout.carried", "The balance is carried forward."),
    ("property.payout.invoice_asked", "The invoice to the owner waits for approval."),
    ("property.payout.invoiced", "The owner is invoiced."),
    ("property.payout.settled_how", "Settled by"),
    ("property.payout.how_payout", "Payout"),
    ("property.payout.how_carried_forward", "Carried forward"),
    ("property.payout.how_invoiced", "Invoiced to the owner"),
    (
        "property.payout.not_positive",
        "Only a positive balance is paid out; carry a balance of zero or less forward.",
    ),
    (
        "property.payout.disputed",
        "The statement is disputed: clear the dispute before the payout.",
    ),
    (
        "property.payout.positive",
        "A positive balance is paid out, not carried forward.",
    ),
    (
        "property.payout.not_negative",
        "Only a negative balance is invoiced to the owner.",
    ),
    (
        "property.payout.not_allowed",
        "The agreement does not allow invoicing a negative balance, and the last three statements did not all end negative.",
    ),
    (
        "property.payout.no_charge_type",
        "Configure the charge type recharged costs (with its VAT treatment) before the owner is invoiced.",
    ),
    ("property.payout.invoice_line", "Negative balance of statement :statement"),
    // The deposit applied to the deductions (BL-PRP-17).
    ("property.deposit_forfeit.action", "Apply the deposit to the deductions"),
    (
        "property.deposit_forfeit.description",
        "Keeps the whole deposit. It goes to the unpaid rent invoices, oldest first, then to the damage lines; the damages it does not cover are invoiced to the tenant.",
    ),
    ("property.deposit_forfeit.approver", "Approved by (property manager)"),
    ("property.deposit_forfeit.approver_hint", "The number of a property manager of the building who is not you."),
    ("property.deposit_forfeit.second", "Second approval (finance or the owner)"),
    ("property.deposit_forfeit.second_hint", "Needed when the total deductions are above AED 5,000.00."),
    ("property.deposit_forfeit.submit", "Forfeit the deposit"),
    ("property.deposit_forfeit.done", "The deposit was applied to the deductions."),
    ("property.deposit_forfeit.title", "Deposit settlement"),
    ("property.deposit_forfeit.held", "Deposit"),
    ("property.deposit_forfeit.rent_due", "Unpaid rent"),
    ("property.deposit_forfeit.damages", "Damages"),
    ("property.deposit_forfeit.to_rent", "Applied to rent"),
    ("property.deposit_forfeit.to_damages", "Kept for damages"),
    ("property.deposit_forfeit.excess", "Damages invoiced"),
    ("property.deposit_forfeit.invoice", "Damages invoice"),
    ("property.deposit_forfeit.applied_on", "Applied on"),
    (
        "property.deposit_forfeit.approver_wrong",
        "The approver must be a property manager of the building.",
    ),
    (
        "property.deposit_forfeit.second_required",
        "The deductions are above :limit: finance or the owner must approve too.",
    ),
    (
        "property.deposit_forfeit.done_already",
        "The deposit of this lease was applied to the deductions already.",
    ),
    (
        "property.deposit_forfeit.unpublished",
        "Publish the deductions to the tenant first.",
    ),
    (
        "property.deposit_forfeit.disputed",
        "A line of the deductions is disputed: answer it first.",
    ),
    (
        "property.deposit_forfeit.window",
        "The tenant may dispute the deductions until :day. Wait until it has passed.",
    ),
    (
        "property.deposit_forfeit.no_charge_type",
        "Configure the charge type damages (with its VAT treatment) before the damages are invoiced.",
    ),
    // The lifecycle of a management agreement and the handover pack (BL-PRP-22).
    ("property.agreements.no_units", "The agreement lists no unit."),
    (
        "property.agreements.fee_missing",
        "Set the fee (a percentage or a fixed fee) and the payout frequency first.",
    ),
    (
        "property.agreements.notice_missing",
        "Give the notice date, the party who gave notice and the handover date.",
    ),
    (
        "property.agreements.handover_early",
        "The handover date cannot be before the notice date.",
    ),
    (
        "property.agreements.holder_unknown",
        "The party the handover goes to was not found.",
    ),
    (
        "property.agreements.receipts_missing",
        "The new holder's receipt is missing for :count deposits or cheques of the handover pack.",
    ),
    (
        "property.agreements.settlement_pending",
        "A deposit waits for its settlement: refund or forfeit it before the agreement ends.",
    ),
    (
        "property.agreements.final_statement",
        "The final owner statement up to the handover date must be approved first.",
    ),
    ("property.handover.deposit_line", "Deposit held for :lease"),
    ("property.handover.cheque_line", "Cheque :number of :date for :lease"),
    (
        "property.handover.not_ending",
        "The agreement is not waiting for its handover.",
    ),
    ("property.handover.receipt_blank", "Give the reference of the receipt."),
    (
        "property.handover.done_already",
        "This line was handed over already.",
    ),
    (
        "property.handover.statement_covers",
        "A statement covers the handover date already: approve it.",
    ),
    ("property.handover.return_reason", "handed over"),
    (
        "property.handover.task_problems",
        "Finish the handover of :agreement by hand",
    ),
    (
        "property.notices.agreement_notice_subject",
        "Notice on management agreement {{ agreement }}",
    ),
    (
        "property.notices.agreement_notice_body",
        "Notice was given by the {{ party }} on {{ notice_on }}. The units are handed over on {{ handover }}. Prepare the handover pack and the final owner statement.",
    ),
    (
        "property.notices.payee_changed_subject",
        "Who you pay for {{ lease }}",
    ),
    (
        "property.notices.payee_changed_body",
        "From {{ handover }} the rent and other payments for {{ lease }} are paid to {{ payee }}. This does not change your lease.",
    ),
    // The management agreement pages (BL-PRP-22).
    ("property.agreements.title", "Management agreements"),
    (
        "property.agreements.subtitle",
        "The contracts that let the company manage an owner's units.",
    ),
    ("property.agreements.new", "New agreement"),
    (
        "property.agreements.new_description",
        "Draft the agreement with the owner; add the units, then activate it with the signed agreement attached.",
    ),
    ("property.agreements.owner_party", "Owner (party number)"),
    ("property.agreements.fee_percent", "Fee (% of the rent collected)"),
    ("property.agreements.fee_fixed", "Fixed fee per statement (AED)"),
    ("property.agreements.start_date", "First day"),
    ("property.agreements.empty", "No management agreements yet"),
    ("property.agreements.number", "Agreement"),
    ("property.agreements.owner", "Owner"),
    ("property.agreements.fee", "Fee"),
    ("property.agreements.start", "From"),
    ("property.agreements.created", "Agreement drafted."),
    ("property.agreements.status.draft", "Draft"),
    ("property.agreements.status.active", "Active"),
    ("property.agreements.status.ending", "Ending"),
    ("property.agreements.status.ended", "Ended"),
    (
        "property.agreements.units_locked",
        "Units are added to a draft or an active agreement only.",
    ),
    ("property.agreements.unit_added", "Unit added."),
    ("property.agreements.activate", "Activate"),
    (
        "property.agreements.activate_message",
        "The company manages the owner's units from now on. The signed agreement must be attached.",
    ),
    ("property.agreements.activated", "Agreement activated."),
    ("property.agreements.add_unit", "Add unit"),
    (
        "property.agreements.add_unit_description",
        "A unit that is under another agreement waits for the transfer to this one.",
    ),
    ("property.agreements.unit", "Unit"),
    ("property.agreements.units", "Units"),
    ("property.agreements.units_empty", "No unit under this agreement yet"),
    ("property.agreements.date_in", "In from"),
    ("property.agreements.date_out", "Out on"),
    ("property.agreements.give_notice", "Give notice"),
    (
        "property.agreements.give_notice_description",
        "Either side ends the agreement. The handover pack is prepared and the property managers and accountants are told.",
    ),
    ("property.agreements.notice_on", "Notice given on"),
    ("property.agreements.notice_party", "Given by"),
    ("property.agreements.party_owner", "The owner"),
    ("property.agreements.party_company", "The company"),
    ("property.agreements.handover_date", "Handover date"),
    (
        "property.agreements.new_holder",
        "Hand over to (party number, empty for the owner)",
    ),
    ("property.agreements.notice", "Notice"),
    ("property.agreements.notice_given", "Notice recorded; the handover pack is started."),
    ("property.agreements.ended_on", "Ended on"),
    ("property.agreements.end", "End the agreement"),
    (
        "property.agreements.end_message",
        "The cheques not deposited are returned, the deposits transferred and the tenants told the new payee.",
    ),
    ("property.agreements.ended", "Agreement ended and handed over."),
    (
        "property.agreements.ended_with_problems",
        "Agreement ended. Some steps need a hand: the accountant has a task.",
    ),
    ("property.agreements.statements", "Owner statements"),
    ("property.handover.title", "Handover pack"),
    ("property.handover.deposit_total", "Deposits held"),
    ("property.handover.deposits", "Deposits held"),
    ("property.handover.deposits_empty", "No deposit held"),
    ("property.handover.cheques", "Cheques not deposited"),
    ("property.handover.cheques_empty", "No cheque to hand over"),
    ("property.handover.invoices", "Open invoices and arrears"),
    ("property.handover.invoices_empty", "No open invoice"),
    ("property.handover.ejari", "Ejari registrations"),
    ("property.handover.ejari_empty", "No Ejari number recorded"),
    ("property.handover.ejari_number", "Ejari number"),
    ("property.handover.line", "Line"),
    ("property.handover.amount", "Amount"),
    ("property.handover.receipt", "New holder's receipt"),
    ("property.handover.lease", "Lease"),
    ("property.handover.due", "Due"),
    ("property.handover.balance", "Balance"),
    ("property.handover.record_receipt", "Record receipt"),
    (
        "property.handover.record_receipt_description",
        "The reference of the receipt the new holder signed for this deposit or cheque.",
    ),
    ("property.handover.receipt_recorded", "Receipt recorded."),
    ("property.handover.final_statement", "Prepare the final statement"),
    (
        "property.handover.final_statement_message",
        "Drafts the owner statement up to the handover date. Another user approves it.",
    ),
    ("property.handover.final_statement_made", "The final statement was drafted."),
    (
        "property.handover.final_statement_exists",
        "A statement for these days exists already.",
    ),
    // Moving a unit out of a management agreement (BL-PRP-23).
    (
        "property.transfers.state",
        "Units leave an active or ending agreement only.",
    ),
    (
        "property.transfers.not_in_agreement",
        "The unit is not under this agreement.",
    ),
    (
        "property.transfers.date_early",
        "The transfer date must be after the day the unit came in.",
    ),
    (
        "property.transfers.open_request",
        "A transfer of this unit waits for its approval.",
    ),
    (
        "property.transfers.same_owner",
        "The new owner is the owner of this agreement.",
    ),
    (
        "property.transfers.new_agreement_inactive",
        "The new owner's agreement is not active. Draft it, list the unit from the transfer date and activate it with the signed agreement first, or choose \"no longer managed\".",
    ),
    (
        "property.transfers.same_user",
        "A property manager other than the requester approves the transfer.",
    ),
    (
        "property.transfers.not_requested",
        "This transfer was decided already.",
    ),
    (
        "property.transfers.receipt_required",
        "Give the new owner's receipt for the deposit that stays held.",
    ),
    (
        "property.transfers.task_problems",
        "Finish the transfer of unit :unit out of :agreement by hand",
    ),
    (
        "property.transfers.task_empty",
        ":agreement has no unit left: give notice if the company stops managing for the owner",
    ),
    ("property.transfers.title", "Units leaving the agreement"),
    ("property.transfers.request", "Move a unit out"),
    (
        "property.transfers.request_description",
        "A sale that the company still manages, or a unit that is no longer managed. A property manager other than you approves it; nothing changes before.",
    ),
    ("property.transfers.unit", "Unit"),
    ("property.transfers.date", "Transfer date"),
    (
        "property.transfers.new_owner",
        "New owner (party number); empty for \"no longer managed\"",
    ),
    ("property.transfers.note", "Note"),
    ("property.transfers.requested", "Transfer requested; a property manager other than you approves it."),
    ("property.transfers.empty", "No unit has been moved out"),
    ("property.transfers.kind", "What"),
    ("property.transfers.kind_sale", "Sold, still managed"),
    ("property.transfers.kind_unmanaged", "No longer managed"),
    ("property.transfers.status", "Status"),
    ("property.transfers.status_requested", "Waiting for approval"),
    ("property.transfers.status_approved", "Approved"),
    ("property.transfers.status_rejected", "Rejected"),
    ("property.transfers.approve", "Approve"),
    ("property.transfers.approved", "Transfer approved and carried out."),
    ("property.transfers.approved_with_problems", "Transfer approved. Some steps need a hand: the accountant has a task."),
    ("property.transfers.reject", "Reject"),
    ("property.transfers.rejected", "Transfer rejected; nothing changed."),
    (
        "property.transfers.deposit_receipt",
        "New owner's receipt for the deposit",
    ),
    ("property.transfers.record_receipt", "Record receipt of a transfer"),
    (
        "property.notices.certificate_expired_subject",
        "Certificate expired: {{ building }}",
    ),
    (
        "property.notices.certificate_expired_body",
        "The {{ kind }} of {{ building }} expired on {{ expired_on }}. The building is flagged non-compliant until a renewed certificate is accepted. Leasing is not blocked.",
    ),
    ("property.compliance.title", "Buildings non-compliant"),
    (
        "property.compliance.subtitle",
        "Buildings with an open compliance flag or an expired compliance certificate. Leasing goes on.",
    ),
    ("property.compliance.empty", "No building is flagged"),
    ("property.compliance.building", "Building"),
    ("property.compliance.cause", "Cause"),
    ("property.compliance.category", "Category or certificate"),
    ("property.compliance.since", "Since"),
    ("property.compliance.count", "Buildings non-compliant"),
    ("property.compliance.cause_visit_missed", "Planned visit missed"),
    ("property.compliance.cause_no_contract", "No service contract"),
    ("property.compliance.cause_vendor_document_expired", "Vendor document expired"),
    ("property.compliance.cause_certificate_expired", "Certificate expired"),
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
