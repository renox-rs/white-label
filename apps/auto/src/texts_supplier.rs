//! The texts of the supplier warranty claims (`AUT.WAR.02`), in English. They
//! join the app's other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("auto.supplier.return_placeholder", "Pick the return to the supplier"),
    ("auto.supplier.terms_label", "Warranty terms"),
    ("auto.supplier.claim_letter", "Claim email sent to the supplier (:number)"),
    ("auto.supplier.title", "Supplier warranty claims"),
    (
        "auto.supplier.subtitle",
        "The failed parts claimed back from the supplier, from the warranty tickets that were closed as accepted.",
    ),
    ("auto.supplier.new", "Raise a claim"),
    (
        "auto.supplier.new_description",
        "A claim is raised from a warranty ticket that was closed as accepted, for a part that carries a supplier warranty. It is refused when the failure is out of the supplier's terms.",
    ),
    ("auto.supplier.raised", "Claim raised."),
    ("auto.supplier.number", "Claim"),
    ("auto.supplier.job_card", "Job card or sale of the ticket"),
    (
        "auto.supplier.job_card_hint",
        "The job card the warranty ticket is about.",
    ),
    ("auto.supplier.job_card_placeholder", "Pick the job card"),
    ("auto.supplier.ticket", "Warranty ticket"),
    (
        "auto.supplier.ticket_hint",
        "The number of the ticket, for example TKT-2026-0012; it must be closed.",
    ),
    (
        "auto.supplier.accepted",
        "The ticket's warranty decision is accepted",
    ),
    (
        "auto.supplier.accepted_hint",
        "Only a claim the customer was accepted for is raised to the supplier.",
    ),
    ("auto.supplier.about", "Job card"),
    ("auto.supplier.supplier", "Supplier"),
    ("auto.supplier.supplier_placeholder", "Pick the supplier"),
    ("auto.supplier.part", "Failed part"),
    ("auto.supplier.item_code", "Item code and brand"),
    ("auto.supplier.brand", "Brand"),
    ("auto.supplier.quantity", "Quantity"),
    ("auto.supplier.part_cost", "Cost of the part"),
    (
        "auto.supplier.money_hint",
        "In dirhams, for example 450 or 450.50.",
    ),
    ("auto.supplier.purchased", "Bought from the supplier on"),
    ("auto.supplier.warranty_months", "Warranty (months)"),
    ("auto.supplier.warranty_km", "Warranty (km)"),
    (
        "auto.supplier.warranty_km_hint",
        "Leave blank when the terms set no mileage.",
    ),
    ("auto.supplier.terms", ":months months"),
    ("auto.supplier.terms_km", ":months months or :km km"),
    ("auto.supplier.labour_included", "The terms include labour"),
    (
        "auto.supplier.labour_included_hint",
        "The labour of the replacement is added to the claim.",
    ),
    ("auto.supplier.labour", "Labour of the replacement"),
    ("auto.supplier.labour_not_included", "Not included"),
    ("auto.supplier.fitted_job_card", "Fitted on job card"),
    (
        "auto.supplier.fitted_hint",
        "Leave blank for a part sold over the counter; it is then checked on the period only.",
    ),
    ("auto.supplier.replacement_job_card", "Replacement job card"),
    (
        "auto.supplier.fitted_odometer",
        "Odometer at the fitting (km)",
    ),
    ("auto.supplier.fitted", "Fitted"),
    ("auto.supplier.over_the_counter", "Sold over the counter"),
    ("auto.supplier.failure_date", "Failed on"),
    (
        "auto.supplier.failure_odometer",
        "Odometer at the failure (km)",
    ),
    ("auto.supplier.failure", "Failure"),
    ("auto.supplier.part_kept", "The failed part was kept"),
    ("auto.supplier.kept", "Kept"),
    ("auto.supplier.not_kept", "Not kept"),
    ("auto.supplier.claimed", "Claimed"),
    ("auto.supplier.details", "Claim"),
    ("auto.supplier.search", "Search by claim, ticket or part"),
    ("auto.supplier.show", "Show"),
    ("auto.supplier.filter", "Show claims"),
    ("auto.supplier.tab.all", "All"),
    ("auto.supplier.tab.raised", "Raised"),
    ("auto.supplier.tab.ended", "Ended"),
    ("auto.supplier.empty", "No supplier claim yet."),
    ("auto.supplier.status.raised", "Raised"),
    ("auto.supplier.status.recovered", "Recovered"),
    ("auto.supplier.status.written_off", "Written off"),
    ("auto.supplier.answer", "Supplier's answer"),
    ("auto.supplier.answer.pending", "Waiting for the supplier"),
    ("auto.supplier.answer.accepted", "Accepted"),
    ("auto.supplier.answer.rejected", "Rejected"),
    ("auto.supplier.answer_note", "Note"),
    (
        "auto.supplier.answer_note_hint",
        "Why the supplier rejected the claim; required for a rejection.",
    ),
    ("auto.supplier.reference", "Supplier's reference"),
    ("auto.supplier.document", "Record the claim document"),
    (
        "auto.supplier.document_description",
        "The claim document that was sent to the supplier by email or as a PDF.",
    ),
    (
        "auto.supplier.document_hint",
        "The name or reference of the document.",
    ),
    ("auto.supplier.claim_document", "Claim document"),
    ("auto.supplier.document_saved", "Claim document recorded."),
    (
        "auto.supplier.record_answer",
        "Record the supplier's answer",
    ),
    (
        "auto.supplier.answer_description",
        "The supplier accepted or rejected the claim. The claim ends when it is credited, replaced or written off.",
    ),
    ("auto.supplier.answer_saved", "Answer recorded."),
    ("auto.supplier.link_return", "Link the return to supplier"),
    (
        "auto.supplier.return_description",
        "The return to supplier of the kept part. When it is credited the claim is recovered by itself.",
    ),
    ("auto.supplier.return", "Return to supplier"),
    ("auto.supplier.return_linked", "Return to supplier linked."),
    ("auto.supplier.recover", "Recover"),
    (
        "auto.supplier.recover_description",
        "The supplier credited the claim or sent a replacement. Give its credit note, or tick that a replacement was received.",
    ),
    ("auto.supplier.credit_reference", "Credit note or receipt"),
    (
        "auto.supplier.replacement",
        "A replacement part was received (at zero cost)",
    ),
    ("auto.supplier.recovered", "Claim recovered."),
    ("auto.supplier.recovery", "Recovery"),
    ("auto.supplier.basis.credit_note", "Credit note"),
    ("auto.supplier.basis.replacement", "Replacement"),
    ("auto.supplier.basis.refund_received", "Refund received"),
    ("auto.supplier.basis.debit_note", "Debit note"),
    ("auto.supplier.basis.manual", "Recorded by hand"),
    ("auto.supplier.write_off", "Write off"),
    (
        "auto.supplier.write_off_description",
        "The supplier refused and the cost is absorbed as warranty expense.",
    ),
    ("auto.supplier.reason", "Reason"),
    ("auto.supplier.written_off", "Claim written off."),
    ("auto.supplier.written_off_because", "Written off because:"),
    // What is refused.
    ("auto.supplier.problem.part", "Name the failed part."),
    (
        "auto.supplier.problem.amounts",
        "The cost, the quantity and the warranty period must be above zero.",
    ),
    (
        "auto.supplier.problem.dates",
        "Give the day it was bought and the day it failed; it did not fail before it was bought or in the future.",
    ),
    (
        "auto.supplier.problem.supplier",
        "Pick the supplier from the parties.",
    ),
    (
        "auto.supplier.problem.ticket",
        "That is not a warranty ticket of the job card or sale.",
    ),
    (
        "auto.supplier.problem.ticket_open",
        "The warranty ticket is not closed yet.",
    ),
    (
        "auto.supplier.problem.not_accepted",
        "Only a warranty ticket closed as accepted is claimed from the supplier.",
    ),
    (
        "auto.supplier.problem.once",
        "A claim was raised for that ticket already.",
    ),
    (
        "auto.supplier.problem.job_card",
        "Pick the job card from the list.",
    ),
    (
        "auto.supplier.problem.period",
        "Out of terms: the part failed after the warranty period.",
    ),
    (
        "auto.supplier.problem.mileage",
        "Out of terms: mileage out of terms.",
    ),
    (
        "auto.supplier.problem.mileage_unknown",
        "Give the mileage at the failure: the terms set a mileage and the part was fitted on a job card.",
    ),
    (
        "auto.supplier.problem.answer",
        "Record accepted or rejected for a claim that is raised.",
    ),
    (
        "auto.supplier.problem.rejection_note",
        "Say why the supplier rejected the claim.",
    ),
    (
        "auto.supplier.problem.document",
        "Give the name or reference of the claim document.",
    ),
    (
        "auto.supplier.problem.return",
        "A return to supplier is linked to a raised claim whose part was kept.",
    ),
    (
        "auto.supplier.problem.no_credit",
        "Give the supplier's credit note, or tick that a replacement was received.",
    ),
    (
        "auto.supplier.problem.approval",
        "A write-off needs the approval of a branch manager, and that approval rule is not in force.",
    ),
    (
        "auto.supplier.writeoff_waits",
        "The write-off waits for the branch manager.",
    ),
    ("auto.supplier.writeoff_done", "Claim written off."),
    (
        "auto.supplier_mail.subject",
        "Warranty claim {{ claim }}: {{ part }}",
    ),
    (
        "auto.supplier_mail.body",
        "Dear {{ supplier }}, we claim under your warranty: {{ part }} ({{ item }}), quantity {{ quantity }}, bought on {{ purchased }}, failed on {{ failed }}, our warranty ticket {{ ticket }}. Amount claimed: {{ amount }}. The failed part is kept for you to collect. Please send your credit note or a replacement and quote {{ claim }}. Thank you.",
    ),
    (
        "auto.supplier.task_raise_title",
        "Raise the supplier claim for warranty ticket :ticket",
    ),
    (
        "auto.supplier.task_raise_body",
        "Warranty ticket :ticket was closed. If its warranty decision was accepted and the part carries a supplier warranty, raise the claim to the supplier.",
    ),
    (
        "auto.supplier.task_send_title",
        "Send the claim :claim to the supplier by hand",
    ),
    (
        "auto.supplier.task_send_body",
        "The email with the claim :claim could not be sent. Send it to the supplier by hand and record the claim document.",
    ),
    (
        "auto.supplier.task_return_title",
        "Draft the return to supplier of :claim by hand",
    ),
    (
        "auto.supplier.task_return_body",
        "The failed part of :claim could not be put on a draft Return to supplier (the part has no item code, or the workshop has no store). Draft it in the stock module and link it to the claim.",
    ),
    (
        "auto.supplier.task_write_off_title",
        "Write off :claim: the supplier refused the credit",
    ),
    (
        "auto.supplier.task_write_off_body",
        "The return of :claim was closed by hand with the reason supplier refused, so the claim is not recovered. Write it off with the branch manager's approval, or keep arguing it.",
    ),
    (
        "auto.supplier.chase_title",
        ":claim is not recovered after :days days",
    ),
    (
        "auto.supplier.chase_body",
        "Supplier claim :claim was raised :days days ago and the supplier has not credited or replaced the part. Chase the supplier, or write the claim off.",
    ),
];
