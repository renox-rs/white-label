//! The texts of a job card that ends before delivery: cancelled after the work
//! started (`AUT.ACL.21`), released to the customer, voided, declined and
//! waiting for parts (BL-AUT-01 .. BL-AUT-04), in English. They join the app's
//! other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("auto.jobs.yes", "Yes"),
    // The reasons a job ends.
    ("auto.stop.code.customer_stopped", "The customer stopped the job"),
    ("auto.stop.code.claim_withdrawn", "The claim was withdrawn"),
    ("auto.stop.code.total_loss", "The vehicle is a total loss"),
    ("auto.stop.code.no_response", "No response to the estimate"),
    ("auto.stop.code.customer_left", "The customer left"),
    (
        "auto.stop.code.estimate_declined",
        "The customer declined the estimate",
    ),
    // How the customer acknowledged the work done.
    ("auto.stop.ack.signed", "Signed by the customer"),
    ("auto.stop.ack.refused_to_sign", "Refused to sign"),
    ("auto.stop.ack.insurer_notice", "The insurer's notice"),
    (
        "auto.stop.ack.written_withdrawal",
        "The customer's written withdrawal",
    ),
    // The invoice lines of the work done.
    ("auto.stop.labour_part", ":description (:done h of :hours h)"),
    (
        "auto.stop.labour_part_flat",
        ":description (:done h of :hours flat hours)",
    ),
    ("auto.stop.inspection_fee", "Inspection fee"),
    // The purchaser's task.
    (
        "auto.stop.po_task_title",
        "Revise the purchase order of :part for :job",
    ),
    (
        "auto.stop.po_task_body",
        ":job was cancelled. The customer is billed for the special-order part :part, which goes with the vehicle. The purchase order is unchanged: revise it with the supplier, or leave it.",
    ),
    // What is refused, in words.
    (
        "auto.stop.problem.checked_in",
        "A vehicle that was just checked in is not cancelled: void the job card if it was opened in error, or decline it when the customer leaves.",
    ),
    (
        "auto.stop.problem.ready",
        "The vehicle is ready for collection: deliver it. A job is cancelled before it is ready.",
    ),
    (
        "auto.stop.problem.closed",
        "This job card is over; it cannot be cancelled.",
    ),
    (
        "auto.stop.problem.pending",
        "A cancel of this job card already waits for the branch manager's approval.",
    ),
    (
        "auto.stop.problem.pending_lines",
        "A cancel of this job card waits for the branch manager's approval: no labour, parts or work can be recorded until it is decided.",
    ),
    (
        "auto.stop.problem.code",
        "Pick why the job stops: the customer stopped it, or the claim was withdrawn.",
    ),
    (
        "auto.stop.problem.claim",
        "This job is funded by insurance claim :claim. Cancel the claim instead: it stops the job card.",
    ),
    (
        "auto.stop.problem.line",
        "That line is not an approved line of this job card.",
    ),
    (
        "auto.stop.problem.part_hours",
        "A line part done needs hours above 0 and below the hours of the line. Mark it done when all its hours were worked.",
    ),
    (
        "auto.stop.problem.fitted",
        "A part cannot be fitted before it is issued: give a quantity from 0 up to what was issued.",
    ),
    (
        "auto.stop.problem.bill_negative",
        "The amount billed cannot be below 0.",
    ),
    (
        "auto.stop.problem.bill_over",
        "The amount billed cannot be above the work done, :value before VAT.",
    ),
    (
        "auto.stop.problem.ack_method",
        "Record how the customer acknowledged the work done: signed, refused to sign (with a witness), or the insurer's notice or the written withdrawal.",
    ),
    (
        "auto.stop.problem.ack_signed",
        "Give the name of the customer who signed the summary of the work done.",
    ),
    (
        "auto.stop.problem.ack_refused",
        "Give the name of the member of staff who witnessed that the customer refused to sign.",
    ),
    (
        "auto.stop.problem.ack_document",
        "Give the reference of the insurer's notice or of the customer's written withdrawal.",
    ),
    (
        "auto.stop.problem.work_not_recorded",
        "Record the work done on every line before the job is cancelled.",
    ),
    (
        "auto.stop.problem.not_acknowledged",
        "The customer must acknowledge the work done: a signature, a refusal with a witness, or the insurer's notice or the written withdrawal.",
    ),
    (
        "auto.stop.problem.approval",
        "Billing less than the work done, or a customer who refused to sign, needs the branch manager's approval, and no approval is set up. Ask the administrator.",
    ),
    (
        "auto.stop.problem.not_closed",
        "Only a declined or cancelled job card is billed this way.",
    ),
    (
        "auto.stop.problem.not_special",
        "Only a special-order part has a purchase order.",
    ),
    // The cancel on the job card page.
    ("auto.stop.title", "Cancel the job"),
    (
        "auto.stop.description",
        "Confirm the work done on each line and take the customer's acknowledgement. The customer is billed for the work done only; billing less, or a customer who refuses to sign, goes to the branch manager.",
    ),
    ("auto.stop.reason", "Why the job stops"),
    ("auto.stop.note", "More about the reason"),
    ("auto.stop.ack", "How the customer acknowledged the work done"),
    ("auto.stop.ack_name", "Customer who signed"),
    ("auto.stop.ack_signature", "Signature"),
    (
        "auto.stop.ack_signature_hint",
        "The customer's typed name, as on the signed summary.",
    ),
    ("auto.stop.ack_witness", "Member of staff who witnessed the refusal"),
    (
        "auto.stop.ack_document",
        "Insurer's notice or written withdrawal (reference)",
    ),
    ("auto.stop.bill", "Amount to bill before VAT"),
    (
        "auto.stop.bill_hint",
        "Leave blank to bill the work done. An amount below it needs the branch manager's approval.",
    ),
    ("auto.stop.work", "Work done: :description"),
    ("auto.stop.hours", "Hours worked (for a line part done)"),
    ("auto.stop.fitted", "Fitted: :description (issued :issued)"),
    ("auto.stop.value", "Work done so far"),
    ("auto.stop.cancelled", "Job cancelled."),
    (
        "auto.stop.waits",
        "The cancel waits for the branch manager's approval.",
    ),
    (
        "auto.stop.pending_banner",
        "The cancel of this job waits for the branch manager's approval. Labour, parts and work are not recorded until it is decided.",
    ),
    ("auto.stop.summary", "How the job stopped"),
    ("auto.stop.entry.value", "Recorded"),
    ("auto.stop.entry.reason", "Reason"),
    ("auto.stop.entry.ack", "Acknowledged"),
    ("auto.stop.entry.work_value", "Work done, before VAT"),
    ("auto.stop.entry.billed", "Billed, before VAT"),
    (
        "auto.stop.entry.lowered",
        "Billed below the work done, approved by the branch manager",
    ),
    ("auto.stop.entry.released", "Vehicle released"),
    ("auto.stop.entry.on_account", "Customer account"),
    ("auto.stop.entry.handed_over", "Special orders handed over"),
    (
        "auto.stop.acknowledged_by",
        ":method: :who",
    ),
    ("auto.stop.order", "Purchase order"),
    (
        "auto.stop.order_description",
        "The purchase order of this special-order part. If the job is cancelled and the part billed, the purchaser gets a task to revise it.",
    ),
    ("auto.stop.order_number", "Purchase order number"),
    ("auto.stop.order_saved", "Purchase order saved."),
    (
        "auto.jobs.transition.cancel_approved",
        "Cancelled (approved by the branch manager)",
    ),
    // Declining a job card (`AUT.RUL.17`).
    (
        "auto.decline.problem.recorded",
        "A labour or part line is recorded on this job card. Work that was recorded is cancelled, not declined: use cancel.",
    ),
    (
        "auto.decline.sms_en",
        "{{ workshop }}: please collect your vehicle {{ plate }} (job {{ job }}). Reason: {{ reason }}.{{ fee }} {{ address }} {{ phone }}",
    ),
    (
        "auto.decline.sms_ar",
        "{{ workshop }}: يرجى استلام مركبتك {{ plate }} (بطاقة العمل {{ job }}). السبب: {{ reason }}.{{ fee }} {{ address }} {{ phone }}",
    ),
    ("auto.decline.reason_expired", "estimate expired"),
    ("auto.decline.reason_declined", "estimate declined"),
    (
        "auto.decline.fee",
        " An inspection fee of :amount is invoiced.",
    ),
    (
        "auto.decline.message_failed",
        "The message asking the customer to collect the vehicle was not delivered. Call the customer.",
    ),
    // The wait for parts and the substitute vehicle (`AUT.RUL.16`).
    ("auto.parts.wait_title", "Wait for parts"),
    ("auto.parts.wait_started", "Waiting for parts since"),
    ("auto.parts.legal_date", "Legal supply date"),
    ("auto.parts.legal_source", "Supply period of the part from"),
    ("auto.parts.source", "Where the part comes from"),
    ("auto.parts.expected_saved", "Expected day saved."),
    (
        "auto.parts.problem.no_request",
        "Raise a parts request first: an approved part that is not issued yet.",
    ),
    ("auto.supply.source.stock", "Our stock"),
    ("auto.supply.source.other_branch", "Another branch"),
    ("auto.supply.source.local_supplier", "A local supplier"),
    ("auto.supply.source.import", "Import"),
    (
        "auto.supply.problem.source",
        "Pick where the part comes from: our stock, another branch, a local supplier or import.",
    ),
    (
        "auto.supply.problem.days",
        "The legal supply period is 7, 14 or 30 days.",
    ),
    (
        "auto.parts.advisor_title",
        "Parts for :job: legal supply date :date",
    ),
    (
        "auto.parts.advisor_body",
        "The parts requested for :job (:plate) must be supplied by :date, the end of the legal supply period. The customer is told the new date.",
    ),
    (
        "auto.parts.manager_title",
        "Legal supply date reached: :job",
    ),
    (
        "auto.parts.manager_body",
        ":job (:plate) has waited for parts since :since, and the legal supply date is :date. Act now.",
    ),
    ("auto.parts.date_unknown", "we will confirm the date"),
    (
        "auto.parts.task_date",
        "Find out when the parts for :job arrive",
    ),
    (
        "auto.parts.task_call",
        "Call :customer: the parts for :job, expected :date",
    ),
    (
        "auto.parts.sms_en",
        "{{ workshop }}: the parts for your vehicle {{ plate }} (job {{ job }}) are expected: {{ date }}.",
    ),
    (
        "auto.parts.sms_ar",
        "{{ workshop }}: موعد وصول قطع مركبتك {{ plate }} (بطاقة العمل {{ job }}): {{ date }}.",
    ),
    ("auto.substitute.title", "Substitute vehicle"),
    ("auto.substitute.status", "Where it stands"),
    ("auto.substitute.plate", "Plate"),
    ("auto.substitute.period", "Lent"),
    ("auto.substitute.from", "Lent on"),
    ("auto.substitute.to", "Back on"),
    ("auto.substitute.reason", "Why the customer refused"),
    ("auto.substitute.record", "Record the substitute vehicle"),
    (
        "auto.substitute.record_description",
        "The car lent to the customer: its plate and the day it was lent. This closes the task.",
    ),
    ("auto.substitute.refuse", "The customer refused"),
    (
        "auto.substitute.refuse_description",
        "The customer does not want a substitute vehicle; give the reason. This closes the task.",
    ),
    ("auto.substitute.return", "The car came back"),
    (
        "auto.substitute.return_description",
        "The day the substitute vehicle was returned. The job card cannot be delivered before.",
    ),
    ("auto.substitute.saved", "Saved."),
    ("auto.substitute.status.requested", "Waiting for the advisor"),
    ("auto.substitute.status.lent", "Lent to the customer"),
    ("auto.substitute.status.returned", "Returned"),
    ("auto.substitute.status.refused", "Refused by the customer"),
    (
        "auto.substitute.task_title",
        "Provide a substitute vehicle for :job",
    ),
    (
        "auto.substitute.task_body",
        "This warranty repair is not ready 7 days after check-in. Record the substitute vehicle (plate, from, to), or the customer's refusal with a reason: the task closes only then, whatever was offered by word.",
    ),
    (
        "auto.substitute.problem.plate",
        "Give the plate of the substitute vehicle.",
    ),
    (
        "auto.substitute.problem.dates",
        "The car cannot come back before the day it was lent.",
    ),
    (
        "auto.substitute.problem.refused",
        "The customer refused a substitute vehicle; it cannot be recorded as lent.",
    ),
    (
        "auto.substitute.problem.lent",
        "A substitute vehicle was recorded as lent; the customer cannot have refused it.",
    ),
    (
        "auto.substitute.problem.none_lent",
        "No substitute vehicle was recorded as lent on this job card.",
    ),
    (
        "auto.substitute.problem.reason",
        "Give the reason the customer refused the substitute vehicle.",
    ),
    (
        "auto.substitute.problem.open",
        "A substitute vehicle is still open on this job card: record the day it came back, or the customer's refusal, before it is delivered.",
    ),
    // Voiding a job card opened in error.
    (
        "auto.void.problem.recorded",
        "A labour or part line is recorded on this job card. A job card is voided only before any labour or part is recorded: cancel it instead, or decline it if the customer left.",
    ),
    (
        "auto.void.problem.claim",
        "This job card is linked to insurance claim :claim. Void the claim first when nothing was sent to the insurer, or cancel it; then the job card can be dealt with.",
    ),
    // Releasing the vehicle.
    ("auto.release.title", "Release the vehicle"),
    (
        "auto.release.description",
        "The vehicle leaves when the invoice is paid or the customer is on account. A vehicle released as is needs the customer's signature.",
    ),
    ("auto.release.to", "Who takes the vehicle"),
    ("auto.release.condition", "Condition"),
    ("auto.release.safe", "Safe to drive"),
    ("auto.release.as_is", "As is (not drivable or unsafe)"),
    ("auto.release.signature", "Customer's signature"),
    (
        "auto.release.signature_hint",
        "Needed to release the vehicle as is.",
    ),
    ("auto.release.released", "Vehicle released."),
    ("auto.release.value", ":to · :condition · :at"),
    ("auto.release.on_account", "Customer pays on account"),
    (
        "auto.release.on_account_description",
        "Tick when the customer pays on credit terms: the vehicle then leaves before the invoice is paid.",
    ),
    ("auto.release.on_account_yes", "On account"),
    ("auto.release.on_account_no", "Pays on cash terms"),
    ("auto.release.on_account_saved", "Saved."),
    (
        "auto.release.problem.not_closed",
        "Only the vehicle of a declined or cancelled job card is released here; a delivered vehicle has gone.",
    ),
    (
        "auto.release.problem.released",
        "The vehicle was already released.",
    ),
    (
        "auto.release.problem.recipient",
        "Give the name of the person who takes the vehicle.",
    ),
    (
        "auto.release.problem.condition",
        "Say whether the vehicle leaves safe or as is.",
    ),
    (
        "auto.release.problem.signature",
        "A vehicle released as is needs the customer's signature.",
    ),
    (
        "auto.release.problem.unpaid",
        "The invoice is not paid: :balance is open. The vehicle is released when it is paid, or when the customer is on account.",
    ),
    (
        "auto.release.problem.not_invoiced",
        "The invoice for this job is not raised yet. The vehicle is released when it is paid, or when the customer is on account.",
    ),
    (
        "auto.release.problem.claim_unpaid",
        "The excess and the private items of claim :claim are not paid. The vehicle is released when they are paid, or when the insurer confirms the excess is waived.",
    ),
];
