//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_invoicing.title", "Invoices"),
    ("wl_invoicing.empty", "No invoices yet"),
    (
        "wl_invoicing.empty_hint",
        "Invoices drafted for leases, orders and tickets show here.",
    ),
    ("wl_invoicing.draft_number", "Draft"),
    ("wl_invoicing.pending_approval", "Pending approval"),
    ("wl_invoicing.fields.number", "Number"),
    ("wl_invoicing.fields.party", "Party"),
    ("wl_invoicing.fields.entity", "Issued by"),
    ("wl_invoicing.fields.issue_date", "Issue date"),
    ("wl_invoicing.fields.due_date", "Due date"),
    ("wl_invoicing.fields.date_of_supply", "Date of supply"),
    ("wl_invoicing.fields.currency", "Currency"),
    ("wl_invoicing.fields.net_total", "Net"),
    ("wl_invoicing.fields.vat_total", "VAT"),
    ("wl_invoicing.fields.gross_total", "Total"),
    ("wl_invoicing.fields.allocated_total", "Paid and credited"),
    ("wl_invoicing.fields.balance", "Balance"),
    ("wl_invoicing.fields.status", "Status"),
    ("wl_invoicing.fields.name", "Name"),
    ("wl_invoicing.fields.trn", "TRN"),
    ("wl_invoicing.fields.address", "Address"),
    ("wl_invoicing.fields.billing_email", "Billing e-mail"),
    ("wl_invoicing.fields.description", "Description"),
    ("wl_invoicing.fields.quantity", "Quantity"),
    ("wl_invoicing.fields.unit_price", "Unit price"),
    ("wl_invoicing.fields.discount", "Discount"),
    ("wl_invoicing.fields.net_amount", "Net"),
    ("wl_invoicing.fields.vat_category", "VAT category"),
    ("wl_invoicing.fields.vat_amount", "VAT"),
    ("wl_invoicing.fields.gross_amount", "Total"),
    ("wl_invoicing.fields.source", "Source"),
    ("wl_invoicing.fields.amount", "Amount"),
    ("wl_invoicing.fields.created_at", "Created"),
    ("wl_invoicing.status.draft", "Draft"),
    ("wl_invoicing.status.issued", "Issued"),
    ("wl_invoicing.status.partially_paid", "Partially paid"),
    ("wl_invoicing.status.overdue", "Overdue"),
    ("wl_invoicing.status.paid", "Paid"),
    ("wl_invoicing.status.written_off", "Written off"),
    ("wl_invoicing.status.credited", "Credited"),
    ("wl_invoicing.credit_note.title", "Credit notes"),
    ("wl_invoicing.credit_note.empty", "No credit notes yet"),
    ("wl_invoicing.credit_note.draft_number", "Draft"),
    ("wl_invoicing.credit_note.fields.invoice", "Invoice"),
    ("wl_invoicing.credit_note.fields.reason", "Reason"),
    (
        "wl_invoicing.credit_note.fields.remaining",
        "Remaining credit",
    ),
    (
        "wl_invoicing.credit_note.fields.release_origins",
        "Bill the origins again",
    ),
    ("wl_invoicing.credit_note.status.draft", "Draft"),
    ("wl_invoicing.credit_note.status.issued", "Issued"),
    ("wl_invoicing.credit_note.status.applied", "Applied"),
    ("wl_invoicing.credit_note.kind.partial", "Partial"),
    ("wl_invoicing.credit_note.kind.full", "Full"),
    ("wl_invoicing.credit_note.sections.list", "Credit notes"),
    (
        "wl_invoicing.credit_note.sections.movements",
        "Where it went",
    ),
    ("wl_invoicing.credit_note.no_movements", "Nothing used yet"),
    (
        "wl_invoicing.credit_note.no_credit_notes",
        "No credit notes",
    ),
    ("wl_invoicing.credit_note.fields.target", "To"),
    ("wl_invoicing.credit_note.fields.kind", "Type"),
    (
        "wl_invoicing.credit_note.amount_hint",
        "In dirhams, like 150.00.",
    ),
    ("wl_invoicing.credit_note.fields.attempt", "Attempt"),
    (
        "wl_invoicing.credit_note.movement.invoice_allocated",
        "Used on an invoice",
    ),
    (
        "wl_invoicing.credit_note.movement.invoice_credited",
        "Invoice cancelled",
    ),
    (
        "wl_invoicing.credit_note.movement.refund_taken",
        "Taken by a refund",
    ),
    (
        "wl_invoicing.credit_note.movement.refund_restored",
        "Given back, refund failed",
    ),
    (
        "wl_invoicing.credit_note.movement.allocation_restored",
        "Given back, use removed",
    ),
    ("wl_invoicing.credit_note.issue.action", "Issue"),
    (
        "wl_invoicing.credit_note.issue.confirm_title",
        "Issue the credit note?",
    ),
    (
        "wl_invoicing.credit_note.issue.confirm_message",
        "It gets its number and is used on the invoice. This cannot be undone.",
    ),
    (
        "wl_invoicing.credit_note.issue.done",
        "The credit note is issued.",
    ),
    ("wl_invoicing.credit_note.use.action", "Use on an invoice"),
    ("wl_invoicing.credit_note.use.title", "Use on an invoice"),
    (
        "wl_invoicing.credit_note.use.description",
        "Takes an amount of the remaining credit, {{ remaining }}, off another open invoice of the party.",
    ),
    ("wl_invoicing.credit_note.use.submit", "Use the credit"),
    ("wl_invoicing.credit_note.use.invoice", "Invoice"),
    ("wl_invoicing.credit_note.use.done", "The credit is used."),
    ("wl_invoicing.credit_note.refund.action", "Request a refund"),
    ("wl_invoicing.credit_note.refund.title", "Refund the credit"),
    (
        "wl_invoicing.credit_note.refund.description",
        "Sends an amount of the remaining credit, {{ remaining }}, back to the party. Payments approves it.",
    ),
    (
        "wl_invoicing.credit_note.refund.submit",
        "Request the refund",
    ),
    ("wl_invoicing.credit_note.refund.method", "Method"),
    ("wl_invoicing.credit_note.refund.reason", "Reason"),
    (
        "wl_invoicing.credit_note.refund.done",
        "The refund is requested.",
    ),
    (
        "wl_invoicing.credit_note.method.default",
        "Payments' default",
    ),
    ("wl_invoicing.credit_note.method.cash", "Cash"),
    ("wl_invoicing.credit_note.method.card", "Card"),
    ("wl_invoicing.credit_note.method.transfer", "Bank transfer"),
    ("wl_invoicing.credit_note.method.cheque", "Cheque"),
    ("wl_invoicing.credit_note.method.gateway", "Online payment"),
    (
        "wl_invoicing.credit_note.method.direct_debit",
        "Direct debit",
    ),
    ("wl_invoicing.credit_note.method.wallet", "Wallet"),
    ("wl_invoicing.credit_note.method.set_off", "Set-off"),
    ("wl_invoicing.credit_note.credit.action", "Credit in full"),
    (
        "wl_invoicing.credit_note.credit.title",
        "Credit the invoice in full",
    ),
    (
        "wl_invoicing.credit_note.credit.description",
        "Cancels the invoice with a credit note for all its lines. Only an invoice with nothing paid or credited can be credited in full.",
    ),
    (
        "wl_invoicing.credit_note.credit.submit",
        "Credit the invoice",
    ),
    (
        "wl_invoicing.credit_note.credit.done",
        "The invoice is credited.",
    ),
    ("wl_invoicing.credit_note.new.title", "New credit note"),
    (
        "wl_invoicing.credit_note.new.description",
        "Type the net amount to credit on each line, like 150.00. Leave a line empty to credit nothing on it.",
    ),
    ("wl_invoicing.credit_note.new.action", "New credit note"),
    (
        "wl_invoicing.credit_note.new.submit",
        "Issue the credit note",
    ),
    ("wl_invoicing.credit_note.new.line", "Net to credit"),
    ("wl_invoicing.credit_note.new.up_to", "Up to"),
    (
        "wl_invoicing.credit_note.new.done",
        "The credit note is issued.",
    ),
    (
        "wl_invoicing.credit_note.errors.amount",
        "Enter an amount like 150.00.",
    ),
    (
        "wl_invoicing.credit_note.errors.invoice",
        "Choose an invoice.",
    ),
    (
        "wl_invoicing.credit_note.errors.method",
        "Choose one of the methods.",
    ),
    (
        "wl_invoicing.credit_note.errors.reason",
        "Give a reason for the credit note.",
    ),
    ("wl_invoicing.vat_category.standard", "Standard (5%)"),
    ("wl_invoicing.vat_category.zero_rated", "Zero-rated"),
    ("wl_invoicing.vat_category.exempt", "Exempt"),
    ("wl_invoicing.vat_category.out_of_scope", "Out of scope"),
    ("wl_invoicing.vat_category.reverse_charge", "Reverse charge"),
    ("wl_invoicing.ageing.title", "Receivables ageing"),
    ("wl_invoicing.ageing.as_of", "As of"),
    ("wl_invoicing.ageing.entity", "Legal entity"),
    ("wl_invoicing.ageing.all_entities", "All entities"),
    ("wl_invoicing.ageing.filter", "Show"),
    ("wl_invoicing.ageing.days_past_due", "Days past due"),
    ("wl_invoicing.ageing.party", "Party"),
    ("wl_invoicing.ageing.currency", "Currency"),
    ("wl_invoicing.ageing.disputed", "Disputed"),
    ("wl_invoicing.ageing.total", "Balance"),
    ("wl_invoicing.ageing.empty", "No open receivables"),
    ("wl_invoicing.vat.title", "VAT summaries"),
    ("wl_invoicing.vat.status.open", "Open"),
    ("wl_invoicing.vat.status.prepared", "Prepared"),
    ("wl_invoicing.vat.status.reviewed", "Reviewed"),
    ("wl_invoicing.vat.status.locked", "Locked"),
    (
        "wl_invoicing.vat.period_running",
        "The tax period has not ended yet.",
    ),
    (
        "wl_invoicing.vat.same_person",
        "A VAT summary is reviewed by someone other than its preparer.",
    ),
    (
        "wl_invoicing.vat.reviewer_unknown",
        "The summary has no preparer, or the reviewer is unknown.",
    ),
    (
        "wl_invoicing.vat.reason_required",
        "Give a reason for sending the summary back.",
    ),
    ("wl_invoicing.vat.empty", "No VAT summaries yet"),
    (
        "wl_invoicing.vat.empty_hint",
        "A summary is opened for each tax period of a VAT-registered entity.",
    ),
    ("wl_invoicing.vat.fields.entity", "Legal entity"),
    ("wl_invoicing.vat.fields.period", "Period"),
    ("wl_invoicing.vat.fields.period_start", "From"),
    ("wl_invoicing.vat.fields.period_end", "To"),
    ("wl_invoicing.vat.fields.status", "Status"),
    ("wl_invoicing.vat.fields.prepared_by", "Prepared by"),
    ("wl_invoicing.vat.fields.prepared_at", "Prepared on"),
    ("wl_invoicing.vat.fields.reviewed_by", "Reviewed by"),
    ("wl_invoicing.vat.fields.reviewed_at", "Reviewed on"),
    ("wl_invoicing.vat.fields.locked_by", "Locked by"),
    ("wl_invoicing.vat.fields.locked_at", "Locked on"),
    (
        "wl_invoicing.vat.fields.send_back_reason",
        "Sent back because",
    ),
    (
        "wl_invoicing.vat.fields.filing_reference",
        "Filing reference",
    ),
    ("wl_invoicing.vat.sections.summary", "Summary"),
    ("wl_invoicing.vat.sections.adjustments", "Adjustments"),
    ("wl_invoicing.vat.adjustments.kind", "Kind"),
    ("wl_invoicing.vat.adjustments.invoice", "Invoice"),
    ("wl_invoicing.vat.adjustments.amount", "VAT"),
    ("wl_invoicing.vat.adjustments.status", "Status"),
    ("wl_invoicing.vat.adjustments.reason", "Reason"),
    ("wl_invoicing.vat.adjustments.empty", "No adjustments"),
    ("wl_invoicing.vat.adjustments.include", "Include"),
    ("wl_invoicing.vat.adjustments.exclude", "Exclude"),
    (
        "wl_invoicing.vat.adjustments.include_title",
        "Include the adjustment",
    ),
    (
        "wl_invoicing.vat.adjustments.exclude_title",
        "Exclude the adjustment",
    ),
    (
        "wl_invoicing.vat.adjustments.description",
        "Say why. The reason is kept with the decision.",
    ),
    ("wl_invoicing.vat.adjustments.reason_label", "Reason"),
    (
        "wl_invoicing.vat.adjustments.included",
        "Adjustment included.",
    ),
    (
        "wl_invoicing.vat.adjustments.excluded",
        "Adjustment excluded.",
    ),
    (
        "wl_invoicing.adjustment_kind.late_document",
        "Late document",
    ),
    (
        "wl_invoicing.adjustment_kind.earlier_period_credit_note",
        "Earlier-period credit note",
    ),
    (
        "wl_invoicing.adjustment_kind.bad_debt_relief",
        "Bad-debt relief",
    ),
    (
        "wl_invoicing.adjustment_kind.bad_debt_recovery",
        "Bad-debt recovery",
    ),
    (
        "wl_invoicing.adjustment_kind.supplier_debit_note",
        "Supplier debit note",
    ),
    ("wl_invoicing.adjustment_kind.manual", "Manual"),
    ("wl_invoicing.sections.issuer", "Issued by"),
    ("wl_invoicing.sections.party", "Billed to"),
    ("wl_invoicing.sections.totals", "Totals"),
    ("wl_invoicing.sections.lines", "Lines"),
    ("wl_invoicing.sections.vat", "VAT by category"),
    ("wl_invoicing.sections.allocations", "Payments and credits"),
    ("wl_invoicing.sections.write_off", "Write-off"),
    ("wl_invoicing.sections.history", "History"),
    ("wl_invoicing.no_lines", "No lines"),
    (
        "wl_invoicing.no_allocations",
        "Nothing paid or credited yet",
    ),
    ("wl_invoicing.no_history", "No changes yet"),
    ("wl_invoicing.history.change", "Change"),
    ("wl_invoicing.history.at", "When"),
    ("wl_invoicing.history.reason", "Reason"),
    ("wl_invoicing.issue.action", "Issue"),
    ("wl_invoicing.issue.done", "The invoice was issued."),
    ("wl_invoicing.issue.confirm_title", "Issue this invoice?"),
    (
        "wl_invoicing.issue.confirm_message",
        "The invoice gets its number and can no longer be changed.",
    ),
    ("wl_invoicing.write_off.action", "Write off"),
    ("wl_invoicing.write_off.title", "Write off the balance"),
    (
        "wl_invoicing.write_off.description",
        "The open balance of {{ balance }} is written off as a bad debt. The tax invoice stays valid: no credit note is issued. Another approver decides the request.",
    ),
    ("wl_invoicing.write_off.submit", "Request the write-off"),
    ("wl_invoicing.write_off.reason_code", "Reason"),
    ("wl_invoicing.write_off.reason_text", "Details"),
    (
        "wl_invoicing.write_off.done",
        "The balance was written off.",
    ),
    (
        "wl_invoicing.write_off.sent",
        "The write-off was sent for approval.",
    ),
    ("wl_invoicing.write_off.amount", "Amount written off"),
    ("wl_invoicing.write_off.reason", "Reason"),
    ("wl_invoicing.write_off.date", "Write-off date"),
    ("wl_invoicing.write_off.notice_on", "Notice sent on"),
    (
        "wl_invoicing.adjustment.summary_required",
        "A proposal needs the VAT summary to put the adjustment on.",
    ),
    (
        "wl_invoicing.adjustment.reason_required",
        "Give a reason for including or excluding the adjustment.",
    ),
    (
        "wl_invoicing.adjustment.recovered_before_claim",
        "recovered before claim",
    ),
    (
        "wl_invoicing.adjustment.summary_closed",
        "The adjustment can only be decided while its VAT summary is open or prepared.",
    ),
    ("wl_invoicing.write_off.relief", "VAT bad-debt relief"),
    ("wl_invoicing.write_off.relief_status", "Relief status"),
    ("wl_invoicing.write_off.relief_detail", "Relief details"),
    (
        "wl_invoicing.write_off.relief_on",
        "On the VAT summary for {{ period }}",
    ),
    ("wl_invoicing.write_off.recovered", "Recovered so far"),
    ("wl_invoicing.write_off.left", "Left to recover"),
    ("wl_invoicing.sections.recoveries", "Recoveries"),
    ("wl_invoicing.recoveries.date", "Recovered on"),
    ("wl_invoicing.recoveries.source", "Payment"),
    ("wl_invoicing.recoveries.amount", "Amount"),
    ("wl_invoicing.recoveries.vat_share", "VAT share"),
    ("wl_invoicing.recoveries.effect", "VAT effect"),
    ("wl_invoicing.recoveries.empty", "Nothing recovered yet."),
    (
        "wl_invoicing.recoveries.effect.adjustment",
        "Recovery adjustment of {{ amount }} on {{ period }}",
    ),
    (
        "wl_invoicing.recoveries.effect.pending",
        "Recovery adjustment waits for an open VAT summary",
    ),
    ("wl_invoicing.recoveries.effect.reduced", "Relief reduced"),
    ("wl_invoicing.recoveries.effect.none", "No VAT effect"),
    ("wl_invoicing.adjustment_status.pending", "Pending"),
    ("wl_invoicing.adjustment_status.proposed", "Proposed"),
    ("wl_invoicing.adjustment_status.included", "Included"),
    ("wl_invoicing.adjustment_status.excluded", "Excluded"),
    ("wl_invoicing.write_off_reason.insolvent", "Insolvent"),
    ("wl_invoicing.write_off_reason.untraceable", "Untraceable"),
    (
        "wl_invoicing.write_off_reason.uneconomic_to_pursue",
        "Uneconomic to pursue",
    ),
    (
        "wl_invoicing.write_off_reason.settlement_agreed",
        "Settlement agreed",
    ),
    ("wl_invoicing.write_off_reason.other", "Other"),
    (
        "wl_invoicing.write_off.unknown_reason",
        "Choose one of the write-off reasons.",
    ),
    (
        "wl_invoicing.write_off.reason_text_required",
        "This reason needs a text.",
    ),
    (
        "wl_invoicing.write_off_notice.subject",
        "Invoice {{ number }} written off as a bad debt",
    ),
    (
        "wl_invoicing.write_off_notice.greeting",
        "Dear {{ party_name }},",
    ),
    (
        "wl_invoicing.write_off_notice.intro",
        "We write to you about invoice {{ number }}, issued to you on {{ issue_date }}.",
    ),
    (
        "wl_invoicing.write_off_notice.statement",
        "The amount of {{ written_off }} still open on this invoice was written off as a bad debt on {{ write_off_date }}. This is not a credit note: the tax invoice stands as issued.",
    ),
    (
        "wl_invoicing.write_off_notice.closing",
        "If you have any questions about this notice, please reply to this message.",
    ),
    ("wl_invoicing.write_off_notice.number", "Invoice number"),
    ("wl_invoicing.write_off_notice.issue_date", "Issue date"),
    (
        "wl_invoicing.write_off_notice.date_of_supply",
        "Date of supply",
    ),
    (
        "wl_invoicing.write_off_notice.gross_total",
        "Total including VAT",
    ),
    (
        "wl_invoicing.write_off_notice.written_off",
        "Amount written off",
    ),
    (
        "wl_invoicing.write_off_notice.vat_share",
        "VAT in the amount written off",
    ),
    (
        "wl_invoicing.write_off_notice.write_off_date",
        "Write-off date",
    ),
    (
        "wl_invoicing.write_off_notice.off",
        "The customer write-off notice is turned off, so none was sent.",
    ),
    (
        "wl_invoicing.write_off_notice.no_email",
        "The party has no billing e-mail address, so no notice was sent.",
    ),
    (
        "wl_invoicing.write_off_notice.waiting",
        "The notice is being sent.",
    ),
    (
        "wl_invoicing.reminder.before_due.subject",
        "Invoice {{ number }} is due on {{ due_date }}",
    ),
    (
        "wl_invoicing.reminder.before_due.body",
        "Dear {{ party_name }},\n\nThis is a reminder that invoice {{ number }} of {{ issue_date }} is due on {{ due_date }}. The balance of {{ balance }} is still open.\n\nIf you have already paid, please ignore this message.",
    ),
    (
        "wl_invoicing.reminder.after_due.subject",
        "Invoice {{ number }} is overdue",
    ),
    (
        "wl_invoicing.reminder.after_due.body",
        "Dear {{ party_name }},\n\nInvoice {{ number }} of {{ issue_date }} was due on {{ due_date }}, {{ days }} day(s) ago. The balance of {{ balance }} is still open. Please pay it as soon as possible.\n\nIf you have already paid, please ignore this message.",
    ),
    (
        "wl_invoicing.adjustment.recovery_reason",
        "recovery of a written-off invoice",
    ),
    (
        "wl_invoicing.recovered.subject",
        "Bad debt recovered on invoice {{ number }}",
    ),
    (
        "wl_invoicing.recovered.body",
        "{{ amount }} was recovered on the written-off invoice {{ number }} ({{ party_name }}); {{ recovered_total }} of {{ written_off }} is now recovered. VAT relief claimed: {{ relief_claimed }}. {{ adjustment }} See the invoice: {{ link }}",
    ),
    ("wl_invoicing.dispute.tag", "Disputed"),
    ("wl_invoicing.dispute.grid.column", "Dispute"),
    ("wl_invoicing.dispute.grid.clear", "Not disputed"),
    ("wl_invoicing.dispute.action", "Dispute"),
    ("wl_invoicing.dispute.title", "Dispute the invoice"),
    (
        "wl_invoicing.dispute.description",
        "Flags the invoice as disputed and opens a ticket for its owner. Payment reminders pause until it is resolved. Tick the disputed lines or give an amount of up to the balance of {{ balance }}.",
    ),
    ("wl_invoicing.dispute.submit", "Flag as disputed"),
    (
        "wl_invoicing.dispute.done",
        "The invoice is flagged as disputed.",
    ),
    ("wl_invoicing.dispute.fields.reason", "Reason"),
    ("wl_invoicing.dispute.fields.amount", "Disputed amount"),
    (
        "wl_invoicing.dispute.fields.amount_hint",
        "Leave empty to use the total of the ticked lines.",
    ),
    ("wl_invoicing.dispute.fields.lines", "Disputed lines"),
    ("wl_invoicing.dispute.sections.panel", "Dispute"),
    ("wl_invoicing.dispute.panel.raised_by", "Raised by"),
    ("wl_invoicing.dispute.panel.raised_at", "Raised"),
    ("wl_invoicing.dispute.panel.ticket", "Ticket"),
    ("wl_invoicing.dispute.panel.lines", "Disputed lines"),
    (
        "wl_invoicing.dispute.panel.pending_credit",
        "Credit note waiting for approval",
    ),
    ("wl_invoicing.dispute.via.staff", "back office"),
    ("wl_invoicing.dispute.via.portal", "portal"),
    ("wl_invoicing.dispute.via.system", "system"),
    ("wl_invoicing.dispute.resolve.action", "Resolve dispute"),
    ("wl_invoicing.dispute.resolve.title", "Resolve the dispute"),
    (
        "wl_invoicing.dispute.resolve.description",
        "Upheld keeps the invoice as it is and resumes reminders after a grace period. Credited issues a credit note for the agreed amount.",
    ),
    ("wl_invoicing.dispute.resolve.submit", "Resolve"),
    (
        "wl_invoicing.dispute.resolve.done",
        "The dispute is resolved.",
    ),
    (
        "wl_invoicing.dispute.resolve.sent",
        "The credit is sent for approval.",
    ),
    ("wl_invoicing.dispute.resolve.outcome", "Outcome"),
    (
        "wl_invoicing.dispute.resolve.upheld",
        "Upheld: the invoice stands",
    ),
    (
        "wl_invoicing.dispute.resolve.credited",
        "Credited: issue a credit note",
    ),
    ("wl_invoicing.dispute.resolve.note", "Resolution note"),
    (
        "wl_invoicing.dispute.resolve.agreed_amount",
        "Agreed amount (for credited)",
    ),
    (
        "wl_invoicing.dispute.errors.outcome",
        "Choose how the dispute is resolved.",
    ),
    (
        "wl_invoicing.dispute.errors.agreed_amount",
        "Give the agreed amount to credit, like 150.00.",
    ),
    (
        "wl_invoicing.dispute.acknowledged.subject",
        "We received your dispute of invoice {{ number }}",
    ),
    (
        "wl_invoicing.dispute.acknowledged.body",
        "Dear {{ party_name }}, we received your dispute of invoice {{ number }} for {{ amount }} (reason: {{ reason }}). {{ ticket }} We will come back to you. Until it is resolved, no payment reminders are sent for this invoice.",
    ),
    (
        "wl_invoicing.dispute.unassigned.subject",
        "Dispute on invoice {{ number }} has no owner",
    ),
    (
        "wl_invoicing.dispute.unassigned.body",
        "{{ party_name }} disputed invoice {{ number }} for {{ amount }} (reason: {{ reason }}). The invoice owner is not available, so the ticket is unassigned. Please take it: {{ link }}",
    ),
    (
        "wl_invoicing.dispute.resolved.subject",
        "Your dispute of invoice {{ number }} is resolved",
    ),
    (
        "wl_invoicing.dispute.resolved.body",
        "Dear {{ party_name }}, we resolved your dispute of invoice {{ number }}. {{ result }} Note: {{ note }}",
    ),
    (
        "wl_invoicing.dispute.result_upheld",
        "The invoice stands. Payment reminders resume after {{ grace_until }}.",
    ),
    (
        "wl_invoicing.dispute.result_credited",
        "We issued credit note {{ credit_note }} for {{ amount }}.",
    ),
    (
        "wl_invoicing.dispute.no_ticket",
        "No ticket (ticketing is off).",
    ),
    (
        "wl_invoicing.dispute.ticket_text",
        "Your reference is {{ ticket }}.",
    ),
    (
        "wl_invoicing.dispute.ticket_title",
        "Dispute on {{ number }}",
    ),
    (
        "wl_invoicing.restored.subject",
        "Credit note {{ number }} is available again",
    ),
    (
        "wl_invoicing.restored.body",
        "{{ amount }} of credit note {{ number }} ({{ party_name }}) is available again because {{ cause }}: {{ reason }}. Remaining credit: {{ remaining }}. See the credit note: {{ link }}",
    ),
    (
        "wl_invoicing.restored.cause.refund_failed",
        "the refund failed",
    ),
    (
        "wl_invoicing.restored.cause.allocation_removed",
        "the allocation was removed",
    ),
    (
        "wl_invoicing.relief.proposed.subject",
        "Bad-debt VAT relief proposed for invoice {{ number }}",
    ),
    (
        "wl_invoicing.relief.proposed.body",
        "Bad-debt VAT relief of {{ amount }} for invoice {{ number }} ({{ party_name }}) was proposed on the VAT summary for {{ period }}. Include or exclude it, with a reason: {{ link }}",
    ),
];

/// The English text for `key`, or `key` itself when the package has none.
/// Notification templates are registered with it: the client changes them
/// on the stored notification type, not in the language files.
pub(crate) fn english(key: &str) -> &str {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| text)
}

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key).to_owned()
}

/// Every text of the package in `lang`, keyed without the `wl_invoicing.`
/// prefix (`fields.number`), for the templates: `tx['fields.number']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_invoicing."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TEXTS;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_invoicing.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }
}
