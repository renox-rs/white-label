//! The texts of the insurance claims and the insurers (`AUT.INS.01` ..
//! `AUT.INS.04`), in English. They join the app's other texts in
//! [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The insurers.
    ("auto.insurers.title", "Insurers"),
    (
        "auto.insurers.subtitle",
        "The insurance companies the workshop repairs for, with the terms agreed: claim email, payment terms and rates.",
    ),
    (
        "auto.insurers.subtitle_one",
        "The terms agreed with this insurer. Its rate card and payer agreement are kept in Invoicing.",
    ),
    ("auto.insurers.new", "Record an insurer"),
    (
        "auto.insurers.new_description",
        "The insurance company is a party. Give the terms the workshop agreed with it.",
    ),
    ("auto.insurers.recorded", "Insurer recorded."),
    ("auto.insurers.saved", "Insurer saved."),
    ("auto.insurers.edit", "Change the terms"),
    (
        "auto.insurers.edit_description",
        "The insurance company stays; the terms and rates change.",
    ),
    ("auto.insurers.details", "Terms"),
    ("auto.insurers.name", "Insurer"),
    ("auto.insurers.party", "Insurance company"),
    ("auto.insurers.party_placeholder", "Pick the company"),
    ("auto.insurers.agreement", "Payer agreement"),
    (
        "auto.insurers.agreement_hint",
        "The number of the payer agreement in Invoicing; the claims are billed under it.",
    ),
    ("auto.insurers.email", "Claim email"),
    ("auto.insurers.surveyors", "Surveyors"),
    ("auto.insurers.terms", "Payment terms (days)"),
    (
        "auto.insurers.terms_hint",
        "How many days the insurer takes to pay.",
    ),
    ("auto.insurers.days", ":days days"),
    ("auto.insurers.labour_rate", "Labour rate"),
    ("auto.insurers.storage_rate", "Storage rate per day"),
    (
        "auto.insurers.money_hint",
        "In dirhams, for example 120 or 120.50; leave blank for none.",
    ),
    ("auto.insurers.rate_card", "Rate card in force"),
    ("auto.insurers.no_rate_card", "None accepted"),
    ("auto.insurers.active", "Takes new claims"),
    ("auto.insurers.stopped", "Stopped"),
    ("auto.insurers.stop", "Stop new claims"),
    (
        "auto.insurers.stop_message",
        "No new claim can be registered for this insurer. Claims under way are not changed.",
    ),
    ("auto.insurers.resume", "Take new claims"),
    (
        "auto.insurers.resume_message",
        "New claims can be registered for this insurer again.",
    ),
    (
        "auto.insurers.stopped_message",
        "This insurer takes no new claims.",
    ),
    ("auto.insurers.empty", "No insurer is recorded yet."),
    (
        "auto.insurers.problem.party",
        "Pick the insurance company from the parties.",
    ),
    (
        "auto.insurers.problem.taken",
        "This company is an insurer already.",
    ),
    (
        "auto.insurers.problem.email",
        "Give the claim email as an address, for example claims@insurer.example.",
    ),
    (
        "auto.insurers.problem.terms",
        "Payment terms are between 0 and 365 days.",
    ),
    (
        "auto.insurers.problem.rate",
        "A rate cannot be less than zero.",
    ),
    // The claims.
    ("auto.claims.title", "Insurance claims"),
    (
        "auto.claims.subtitle",
        "Accident repairs funded by an insurer: the estimate, the insurer's approval, the supplementary and the split bill.",
    ),
    ("auto.claims.new", "Register a claim"),
    (
        "auto.claims.new_description",
        "Record the insurer, the policy and claim number, the accident and the police report. The claim funds a job card.",
    ),
    ("auto.claims.registered", "Claim registered."),
    ("auto.claims.number", "Claim"),
    ("auto.claims.insurer", "Insurer"),
    ("auto.claims.insurer_placeholder", "Pick the insurer"),
    ("auto.claims.customer", "Policyholder"),
    ("auto.claims.vehicle", "Vehicle"),
    ("auto.claims.vehicle_placeholder", "Pick the vehicle"),
    ("auto.claims.job_card", "Job card"),
    (
        "auto.claims.job_card_hint",
        "The job card the vehicle was checked in on; the claim funds it.",
    ),
    ("auto.claims.job_card_placeholder", "Pick the job card"),
    ("auto.claims.open_job_card", "Open the job card"),
    ("auto.claims.policy_no", "Policy number"),
    ("auto.claims.claim_no", "Insurer's claim number"),
    (
        "auto.claims.claim_no_hint",
        "The number the insurer gave the claim; one insurer cannot have it twice.",
    ),
    ("auto.claims.accident", "Accident date"),
    ("auto.claims.police_report", "Police report number"),
    (
        "auto.claims.police_hint",
        "The police accident report is the repair permit; the insurer is not asked to approve without it.",
    ),
    ("auto.claims.police_date", "Police report date"),
    ("auto.claims.police_missing", "Not recorded"),
    ("auto.claims.police", "Record the police report"),
    (
        "auto.claims.police_description",
        "The number and date of the police accident report (the repair permit).",
    ),
    ("auto.claims.police_saved", "Police report recorded."),
    ("auto.claims.repair", "Repair"),
    ("auto.claims.repair.agency", "Agency repair"),
    ("auto.claims.repair.non_agency", "Non-agency repair"),
    ("auto.claims.excess", "Excess"),
    (
        "auto.claims.money_hint",
        "In dirhams, for example 500 or 500.50.",
    ),
    ("auto.claims.excess_line", "Excess of claim :claim"),
    ("auto.claims.estimate", "Estimate sent"),
    ("auto.claims.approved", "Approved"),
    ("auto.claims.rate_card", "Insurer's rate card"),
    ("auto.claims.payer_bill", "Payer bill"),
    ("auto.claims.customer_invoice", "Customer invoice"),
    ("auto.claims.details", "Claim"),
    ("auto.claims.search", "Search by claim or policy number"),
    ("auto.claims.show", "Show"),
    ("auto.claims.filter", "Show claims"),
    ("auto.claims.tab.all", "All"),
    ("auto.claims.tab.open", "Open"),
    ("auto.claims.tab.invoiced", "Invoiced"),
    ("auto.claims.tab.closed", "Closed"),
    ("auto.claims.empty", "No claim yet."),
    ("auto.claims.closed_because", "Closed because:"),
    ("auto.claims.status.registered", "Registered"),
    ("auto.claims.status.awaiting_survey", "Awaiting survey"),
    (
        "auto.claims.status.estimate_submitted",
        "Estimate submitted",
    ),
    ("auto.claims.status.approved", "Approved"),
    (
        "auto.claims.status.supplementary_pending",
        "Supplementary pending",
    ),
    ("auto.claims.status.repair_complete", "Repair complete"),
    ("auto.claims.status.total_loss", "Total loss"),
    ("auto.claims.status.invoiced", "Invoiced"),
    ("auto.claims.status.settled", "Settled"),
    ("auto.claims.status.rejected", "Rejected"),
    ("auto.claims.status.cancelled", "Withdrawn"),
    ("auto.claims.status.voided", "Voided"),
    ("auto.claims.kind.estimate", "Estimate"),
    ("auto.claims.kind.supplementary", "Supplementary"),
    ("auto.claims.kind.charges", "Total-loss charges"),
    ("auto.claims.outcome.pending", "Waiting for the insurer"),
    ("auto.claims.outcome.approved", "Approved"),
    ("auto.claims.outcome.declined", "Declined"),
    ("auto.claims.requests", "Asked of the insurer"),
    (
        "auto.claims.requests_empty",
        "Nothing was sent to the insurer yet.",
    ),
    ("auto.claims.request", "Request"),
    ("auto.claims.asked", "Asked"),
    ("auto.claims.reference", "Insurer's reference"),
    (
        "auto.claims.reference_hint",
        "The approval or LPO number the insurer gave.",
    ),
    ("auto.claims.document", "Written approval"),
    (
        "auto.claims.document_hint",
        "The name or reference of the insurer's email or letter that is attached.",
    ),
    ("auto.claims.insurer_reason", "Insurer's reason"),
    ("auto.claims.reason", "Reason"),
    ("auto.claims.line_approved", "approved amount"),
    ("auto.claims.line_price", "Estimated"),
    // The actions.
    ("auto.claims.survey", "Book the survey"),
    (
        "auto.claims.survey_message",
        "The surveyor is asked to inspect the vehicle on site or from photos.",
    ),
    ("auto.claims.survey_booked", "Survey booked."),
    ("auto.claims.submit", "Submit the estimate"),
    (
        "auto.claims.submit_message",
        "The estimate sent on the job card goes to the insurer for approval.",
    ),
    ("auto.claims.submitted", "Estimate submitted."),
    ("auto.claims.approve", "Record the approval"),
    (
        "auto.claims.approve_description",
        "Give the insurer's reference and written approval, and the amount approved for each line. A line left blank is not approved and is declined on the job card.",
    ),
    ("auto.claims.approved_done", "Approval recorded."),
    ("auto.claims.reject", "Record the rejection"),
    (
        "auto.claims.reject_description",
        "The insurer declined the claim. The customer can pay privately on the same job card.",
    ),
    ("auto.claims.rejected", "Rejection recorded."),
    ("auto.claims.supplementary", "Request a supplementary"),
    (
        "auto.claims.supplementary_description",
        "Hidden damage found during strip-down goes back to the insurer. Send the added work on the job card first.",
    ),
    ("auto.claims.findings", "What was found"),
    (
        "auto.claims.findings_hint",
        "Describe the damage; attach the photos to the job card.",
    ),
    (
        "auto.claims.supplementary_requested",
        "Supplementary requested.",
    ),
    (
        "auto.claims.supp_approve",
        "Record the supplementary approval",
    ),
    (
        "auto.claims.supplementary_approved",
        "Supplementary approved.",
    ),
    (
        "auto.claims.supp_decline",
        "Record the supplementary decline",
    ),
    (
        "auto.claims.supp_decline_description",
        "The insurer declined the hidden damage. The added work is declined on the job card; offer it to the customer as a private item.",
    ),
    (
        "auto.claims.supplementary_declined",
        "Supplementary declined.",
    ),
    (
        "auto.claims.invoice",
        "Invoice the insurer and the customer",
    ),
    (
        "auto.claims.invoice_message",
        "The insurer is billed the approved amount less the excess; the customer is invoiced the excess and the private items.",
    ),
    ("auto.claims.invoiced", "Claim invoiced."),
    ("auto.claims.total_loss_action", "Declare a total loss"),
    (
        "auto.claims.total_loss_description",
        "The insurer declared the vehicle a total loss. Attach its notice.",
    ),
    ("auto.claims.total_loss_notice", "Total-loss notice"),
    ("auto.claims.total_loss_declared", "Total loss declared."),
    ("auto.claims.cancel", "Withdraw the claim"),
    (
        "auto.claims.cancel_description",
        "The customer withdraws the claim. After the insurer approved it, attach the customer's written withdrawal.",
    ),
    ("auto.claims.withdrawal", "Written withdrawal"),
    ("auto.claims.cancelled", "Claim withdrawn."),
    ("auto.claims.void", "Void"),
    (
        "auto.claims.void_description",
        "The claim was registered in error. It can be voided before anything was sent to the insurer.",
    ),
    ("auto.claims.voided", "Claim voided."),
    // What is refused.
    (
        "auto.claims.problem.insurer",
        "Pick the insurer from the list.",
    ),
    (
        "auto.claims.problem.insurer_inactive",
        "This insurer takes no new claims.",
    ),
    (
        "auto.claims.problem.vehicle",
        "Pick the vehicle from the register.",
    ),
    ("auto.claims.problem.policy", "Give the policy number."),
    (
        "auto.claims.problem.claim_no",
        "Give the insurer's claim number.",
    ),
    (
        "auto.claims.problem.claim_taken",
        "This insurer has a claim with that number already.",
    ),
    (
        "auto.claims.problem.accident_date",
        "Give the day of the accident; it cannot be in the future.",
    ),
    (
        "auto.claims.problem.police_date",
        "The police report cannot be dated before the accident or in the future.",
    ),
    (
        "auto.claims.problem.police_no",
        "Give the number of the police report.",
    ),
    (
        "auto.claims.problem.excess",
        "The excess cannot be less than zero.",
    ),
    (
        "auto.claims.problem.job_card",
        "Pick the job card from the open ones.",
    ),
    (
        "auto.claims.problem.job_card_vehicle",
        "That job card is for another vehicle.",
    ),
    (
        "auto.claims.problem.job_card_closed",
        "That job card is closed.",
    ),
    (
        "auto.claims.problem.job_card_linked",
        "Another claim funds that job card already.",
    ),
    ("auto.claims.problem.closed", "This claim is over."),
    (
        "auto.claims.problem.details",
        "Record the policy number and the claim number first.",
    ),
    (
        "auto.claims.problem.no_job_card",
        "Link a job card to the claim first.",
    ),
    (
        "auto.claims.problem.not_sent",
        "Send the estimate on the job card first; the claim submits what was sent.",
    ),
    (
        "auto.claims.problem.not_priced",
        "The estimate has no lines, or a line has no price.",
    ),
    (
        "auto.claims.problem.reference",
        "Give the insurer's approval reference or LPO number.",
    ),
    (
        "auto.claims.problem.document",
        "Attach the insurer's written approval; an approved amount is changed only by it.",
    ),
    (
        "auto.claims.problem.lines",
        "Give the amount approved for at least one line; each amount is above zero.",
    ),
    ("auto.claims.problem.findings", "Describe what was found."),
    (
        "auto.claims.problem.nothing_asked",
        "Nothing was asked of the insurer that waits for an answer.",
    ),
    (
        "auto.claims.problem.line_unknown",
        "That line is not one the estimate sent.",
    ),
    (
        "auto.claims.problem.line_too_high",
        "The amount approved for :line is more than its price.",
    ),
    (
        "auto.claims.problem.line_twice",
        "A line is approved twice.",
    ),
    (
        "auto.claims.problem.charges",
        "Record the insurer's approval of the charges first.",
    ),
    (
        "auto.claims.problem.nothing_approved",
        "The insurer approved nothing to bill.",
    ),
    (
        "auto.claims.problem.no_agreement",
        "The insurer has no payer agreement; record it on the insurer first.",
    ),
];
