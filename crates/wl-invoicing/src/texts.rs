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
    ("wl_invoicing.supplier_bill.title", "Supplier bills"),
    ("wl_invoicing.supplier_bill.empty", "No supplier bills yet"),
    ("wl_invoicing.supplier_bill.fields.supplier", "Supplier"),
    (
        "wl_invoicing.supplier_bill.fields.number",
        "Supplier invoice number",
    ),
    ("wl_invoicing.supplier_bill.fields.entity", "Received by"),
    ("wl_invoicing.supplier_bill.fields.bill_date", "Bill date"),
    ("wl_invoicing.supplier_bill.fields.due_date", "Due date"),
    ("wl_invoicing.supplier_bill.fields.contract", "Contract"),
    ("wl_invoicing.supplier_bill.fields.period", "Billing period"),
    (
        "wl_invoicing.supplier_bill.fields.service_date",
        "Service date",
    ),
    (
        "wl_invoicing.supplier_bill.fields.cost_object",
        "Cost object",
    ),
    (
        "wl_invoicing.supplier_bill.fields.approved_by",
        "Approved by",
    ),
    (
        "wl_invoicing.supplier_bill.fields.approval_reason",
        "Approval reason",
    ),
    (
        "wl_invoicing.supplier_bill.fields.void_reason",
        "Void reason",
    ),
    ("wl_invoicing.supplier_bill.fields.cost_code", "Cost code"),
    (
        "wl_invoicing.supplier_bill.form.cost_code_hint",
        "Budget vs actual files the line under the category that collects this code.",
    ),
    ("wl_invoicing.supplier_bill.status.draft", "Draft"),
    ("wl_invoicing.supplier_bill.status.matched", "Matched"),
    ("wl_invoicing.supplier_bill.status.exception", "Exception"),
    ("wl_invoicing.supplier_bill.status.approved", "Approved"),
    (
        "wl_invoicing.supplier_bill.status.partially_paid",
        "Partially paid",
    ),
    ("wl_invoicing.supplier_bill.status.paid", "Paid"),
    ("wl_invoicing.supplier_bill.status.void", "Void"),
    (
        "wl_invoicing.supplier_bill.exception.possible_duplicate",
        "Possible duplicate",
    ),
    (
        "wl_invoicing.supplier_bill.exception.work_after_contract_end",
        "Work after contract end",
    ),
    (
        "wl_invoicing.supplier_bill.problem.supplier",
        "The supplier was not found.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.lines",
        "A supplier bill needs at least one line.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.entity",
        "The receiving legal entity is not set or not active.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.rate",
        "Set the exchange rate of a bill in a foreign currency.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.service_dates",
        "Every line of a bill against a contract needs a service date.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.not_held",
        "The bill is held, as a possible duplicate or for work after the contract end; it is not matched.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.held",
        "The bill is not held.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.approve_reason",
        "Give a reason to approve a held bill.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.creator",
        "A bill held as a possible duplicate or for work after the contract end cannot be approved by the user who entered it.",
    ),
    (
        "wl_invoicing.supplier_bill.problem.void_reason",
        "Give a reason to void the bill.",
    ),
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
        "wl_invoicing.adjustment.recovery_reversed",
        "recovery reversed",
    ),
    (
        "wl_invoicing.adjustment.recovery_reversed_relief",
        "relief claimed again after a reversed recovery",
    ),
    (
        "wl_invoicing.adjustment.not_recovered_before_claim",
        "Only a relief excluded as recovered before claim can be reinstated.",
    ),
    (
        "wl_invoicing.recovery_reversed.subject",
        "Recovery reversed on invoice {{ number }}",
    ),
    (
        "wl_invoicing.recovery_reversed.body",
        "The recovery of {{ amount }} on the written-off invoice {{ number }} ({{ party_name }}) was reversed; {{ recovered_total }} of {{ written_off }} is now recovered. VAT: {{ vat_effect }}. See the invoice: {{ link }}",
    ),
    (
        "wl_invoicing.recovery_reversed.effect.none",
        "No VAT adjustment was needed",
    ),
    (
        "wl_invoicing.recovery_reversed.effect.relief_restored",
        "The VAT relief got the share back",
    ),
    (
        "wl_invoicing.recovery_reversed.effect.relief_reinstated",
        "The VAT relief is pending again",
    ),
    (
        "wl_invoicing.recovery_reversed.effect.adjustment_reduced",
        "The recovery adjustment was reduced",
    ),
    (
        "wl_invoicing.recovery_reversed.effect.adjustment_excluded",
        "The recovery adjustment was excluded",
    ),
    (
        "wl_invoicing.recovery_reversed.effect.relief_created",
        "New bad-debt relief raised for the share",
    ),
    (
        "wl_invoicing.reopened.subject",
        "Invoice {{ number }} is open again",
    ),
    (
        "wl_invoicing.reopened.body",
        "Invoice {{ number }} ({{ party_name }}) was reopened by {{ amount }}: {{ reason }}. New balance: {{ balance }}. New state: {{ new_status }}. See the invoice: {{ link }}",
    ),
    (
        "wl_invoicing.reopen.reason.payment_reversed",
        "payment reversed",
    ),
    (
        "wl_invoicing.reopen.reason.allocation_moved",
        "allocation moved",
    ),
    (
        "wl_invoicing.reopen.reason.recovery_reversed",
        "recovery reversed",
    ),
    ("wl_invoicing.reopen.reason.bank_return", "bank return"),
    (
        "wl_invoicing.reopen.reason.entered_in_error",
        "entered in error",
    ),
    ("wl_invoicing.reopen.reason.chargeback", "chargeback"),
    (
        "wl_invoicing.reopen.reason.debit_note_cancelled",
        "debit note cancelled",
    ),
    (
        "wl_invoicing.supplier_bill_reopened.subject",
        "Supplier bill {{ number }} is open again",
    ),
    (
        "wl_invoicing.supplier_bill_reopened.body",
        "Supplier bill {{ number }} of {{ supplier }} was reopened by {{ amount }}: {{ reason }}. New balance: {{ balance }}. New state: {{ new_status }}. {{ hint }} See the bill: {{ link }}",
    ),
    (
        "wl_invoicing.supplier_bill_reopened.hint.payment_reversed",
        "Pay again or fix the bank details.",
    ),
    (
        "wl_invoicing.supplier_bill_reopened.hint.debit_note_cancelled",
        "",
    ),
    (
        "wl_invoicing.task.subject",
        "Reversal on invoice {{ number }} needs follow-up",
    ),
    (
        "wl_invoicing.task.body",
        "Invoice {{ number }} ({{ party_name }}): {{ message }} See the invoice: {{ link }}",
    ),
    (
        "wl_invoicing.task.reversal_after_write_off",
        "Payment {{ source }} of {{ amount }} allocated before the write-off was reversed: write off more or pursue the party.",
    ),
    (
        "wl_invoicing.task.reversal_refused",
        "Reversal of {{ amount }} from {{ source }} refused: {{ reason }}",
    ),
    (
        "wl_invoicing.task.bill_reversal_refused",
        "Reversal of {{ amount }} refused: {{ reason }}",
    ),
    (
        "wl_invoicing.task.statement_balance_stale",
        "The balance of bank account {{ account }} of {{ entity }} is missing or old (latest: {{ as_of }}, limit {{ days }} days). Import a statement or enter a balance.",
    ),
    ("wl_invoicing.einvoice.title", "E-invoices"),
    (
        "wl_invoicing.einvoice.subtitle",
        "The invoices and credit notes sent to the accredited service provider, per legal entity and state. A rejection changes nothing already issued.",
    ),
    (
        "wl_invoicing.einvoice.banner",
        "{{ count }} e-invoice transmissions are waiting for the service provider. They are tried again until they are sent, in order.",
    ),
    ("wl_invoicing.einvoice.entity", "Legal entity"),
    ("wl_invoicing.einvoice.all_entities", "All entities"),
    ("wl_invoicing.einvoice.all_statuses", "All states"),
    ("wl_invoicing.einvoice.status", "State"),
    ("wl_invoicing.einvoice.filter", "Show"),
    ("wl_invoicing.einvoice.document", "Document"),
    ("wl_invoicing.einvoice.recipient", "Recipient identifier"),
    ("wl_invoicing.einvoice.attempt", "Attempt"),
    ("wl_invoicing.einvoice.reference", "ASP reference"),
    ("wl_invoicing.einvoice.problem", "Error"),
    ("wl_invoicing.einvoice.empty", "No e-invoice transmissions"),
    (
        "wl_invoicing.einvoice.empty_hint",
        "From an entity's go-live date, every invoice and credit note to a recipient in scope is sent to its service provider.",
    ),
    ("wl_invoicing.einvoice.status.queued", "Queued"),
    ("wl_invoicing.einvoice.status.submitted", "Submitted"),
    ("wl_invoicing.einvoice.status.accepted", "Accepted"),
    ("wl_invoicing.einvoice.status.rejected", "Rejected"),
    ("wl_invoicing.einvoice.status.superseded", "Superseded"),
    ("wl_invoicing.einvoice.resubmit", "Resubmit"),
    (
        "wl_invoicing.einvoice.resubmit_title",
        "Resubmit this e-invoice",
    ),
    (
        "wl_invoicing.einvoice.resubmit_description",
        "The payload is built again from the same document, with the recipient identifier and the party data read again. Use it only when the cause is outside the document; a cause in the document is corrected with a credit note with the reason e-invoice correction and a new invoice.",
    ),
    (
        "wl_invoicing.einvoice.confirm",
        "The cause is outside the issued document and has been fixed",
    ),
    (
        "wl_invoicing.einvoice.resubmitted",
        "The e-invoice is queued again.",
    ),
    (
        "wl_invoicing.einvoice.replaces_saved",
        "The invoice it replaces is saved. The superseded transmission is linked to this invoice when it is issued.",
    ),
    (
        "wl_invoicing.einvoice.replaces_unknown",
        "No invoice has that number.",
    ),
    (
        "wl_invoicing.einvoice.task.rejected",
        "The service provider rejected {{ number }} ({{ codes }}): {{ messages }} Fix the cause outside the document and resubmit it, or credit the invoice with the reason e-invoice correction and issue a new one.",
    ),
    (
        "wl_invoicing.einvoice.task.stuck",
        "{{ number }} has not been sent to the service provider {{ hours }} hours after it was issued ({{ error }}).",
    ),
    (
        "wl_invoicing.einvoice.task_notice.subject",
        "E-invoice {{ number }} needs follow-up",
    ),
    (
        "wl_invoicing.einvoice.task_notice.body",
        "{{ message }} See the e-invoices: {{ link }}",
    ),
    (
        "wl_invoicing.einvoice.outage.subject",
        "The e-invoice service provider of {{ entity }} does not answer",
    ),
    (
        "wl_invoicing.einvoice.outage.body",
        "No e-invoice of {{ entity }} has been sent for {{ hours }} hours: the service provider seems unreachable. The transmissions stay queued and are tried again. See the e-invoices: {{ link }}",
    ),
    (
        "wl_invoicing.tasks.kind.einvoice_rejected",
        "E-invoice rejected",
    ),
    (
        "wl_invoicing.tasks.kind.einvoice_stuck",
        "E-invoice not sent",
    ),
    ("wl_invoicing.allocations.removed", "Removed"),
    ("wl_invoicing.allocations.removed_at", "Removed on"),
    ("wl_invoicing.allocations.removed_reason", "Removed because"),
    ("wl_invoicing.sections.reopened", "Reopened"),
    ("wl_invoicing.reopened.date", "Date"),
    ("wl_invoicing.reopened.amount", "Amount"),
    ("wl_invoicing.reopened.reason", "Reason"),
    ("wl_invoicing.reopened.outcome", "Outcome"),
    ("wl_invoicing.reopened.outcome.done", "Reopened"),
    ("wl_invoicing.reopened.outcome.failed", "Refused"),
    ("wl_invoicing.reopened.empty", "Never reopened"),
    ("wl_invoicing.sections.tasks", "Open tasks"),
    ("wl_invoicing.tasks.title", "Reversal tasks"),
    (
        "wl_invoicing.tasks.subtitle",
        "Reversals that could not reopen their invoice or supplier bill and need the accountant.",
    ),
    ("wl_invoicing.tasks.empty", "No open tasks"),
    (
        "wl_invoicing.tasks.empty_hint",
        "Reversals that cannot reopen an invoice or supplier bill show here.",
    ),
    ("wl_invoicing.tasks.invoice", "Invoice"),
    ("wl_invoicing.tasks.kind", "Kind"),
    ("wl_invoicing.tasks.source", "Source"),
    ("wl_invoicing.tasks.amount", "Amount"),
    ("wl_invoicing.tasks.message", "What to do"),
    ("wl_invoicing.tasks.created", "Opened"),
    ("wl_invoicing.tasks.open_invoice", "Open invoice"),
    ("wl_invoicing.tasks.mark_done", "Mark done"),
    (
        "wl_invoicing.tasks.confirm",
        "Mark this task as done? It leaves the list.",
    ),
    ("wl_invoicing.tasks.done", "The task is marked done."),
    (
        "wl_invoicing.tasks.already_done",
        "The task was already done.",
    ),
    (
        "wl_invoicing.tasks.kind.reversal_after_write_off",
        "Reversal after write-off",
    ),
    (
        "wl_invoicing.tasks.kind.reversal_refused",
        "Reversal refused",
    ),
    (
        "wl_invoicing.tasks.kind.bill_reversal_refused",
        "Bill reversal refused",
    ),
    ("wl_invoicing.tasks.bill", "Supplier bill"),
    (
        "wl_invoicing.relief.proposed.subject",
        "Bad-debt VAT relief proposed for invoice {{ number }}",
    ),
    (
        "wl_invoicing.relief.proposed.body",
        "Bad-debt VAT relief of {{ amount }} for invoice {{ number }} ({{ party_name }}) was proposed on the VAT summary for {{ period }}. Include or exclude it, with a reason: {{ link }}",
    ),
    (
        "wl_invoicing.escalation.subject",
        "Dispute of invoice {{ number }} open for {{ days }} days",
    ),
    (
        "wl_invoicing.escalation.body",
        "The dispute of invoice {{ number }} ({{ party_name }}) is still open after {{ days }} days (escalation step {{ step }}). Disputed amount: {{ amount }}. Reason: {{ reason }}. Raised on {{ raised_on }}. Ticket: {{ ticket }}. Owner: {{ owner }}. See the invoice: {{ link }}",
    ),
    (
        "wl_invoicing.supplier_bill_duplicate.subject",
        "Possible duplicate supplier bill {{ number }}",
    ),
    (
        "wl_invoicing.supplier_bill_duplicate.body",
        "The supplier bill {{ number }} of {{ supplier }} ({{ amount }}) may duplicate the earlier bill {{ earlier_number }}. It is held until another user approves it with a reason or it is voided. See the bill: {{ link }}",
    ),
    ("wl_invoicing.supplier_bill.none", "None"),
    ("wl_invoicing.supplier_bill.reason", "Reason"),
    ("wl_invoicing.supplier_bill.fields.created_by", "Entered by"),
    (
        "wl_invoicing.supplier_bill.fields.exchange_rate",
        "Exchange rate to AED",
    ),
    (
        "wl_invoicing.supplier_bill.fields.gross_aed",
        "Total in AED",
    ),
    (
        "wl_invoicing.supplier_bill.fields.contract_type",
        "Contract type",
    ),
    (
        "wl_invoicing.supplier_bill.fields.contract_id",
        "Contract id",
    ),
    (
        "wl_invoicing.supplier_bill.fields.period_start",
        "Period start",
    ),
    ("wl_invoicing.supplier_bill.fields.period_end", "Period end"),
    (
        "wl_invoicing.supplier_bill.fields.earlier_bill",
        "Earlier bill",
    ),
    ("wl_invoicing.supplier_bill.sections.header", "Bill"),
    (
        "wl_invoicing.supplier_bill.sections.contract",
        "Contract and period",
    ),
    (
        "wl_invoicing.supplier_bill.sections.exceptions",
        "Why the bill is held",
    ),
    (
        "wl_invoicing.supplier_bill.form.new_title",
        "New supplier bill",
    ),
    (
        "wl_invoicing.supplier_bill.form.edit_title",
        "Change supplier bill",
    ),
    ("wl_invoicing.supplier_bill.form.edit", "Change"),
    ("wl_invoicing.supplier_bill.form.save", "Save"),
    ("wl_invoicing.supplier_bill.form.cancel", "Cancel"),
    (
        "wl_invoicing.supplier_bill.form.saved",
        "Supplier bill saved.",
    ),
    (
        "wl_invoicing.supplier_bill.form.supplier_hint",
        "The supplier's party code, like P-00001.",
    ),
    (
        "wl_invoicing.supplier_bill.form.default_entity",
        "The default legal entity",
    ),
    (
        "wl_invoicing.supplier_bill.form.rate_hint",
        "AED for one unit of the currency; needed for a foreign currency.",
    ),
    (
        "wl_invoicing.supplier_bill.form.contract_hint",
        "Only for a bill against a service contract, like ticketing.contract.",
    ),
    ("wl_invoicing.supplier_bill.submit.action", "Submit"),
    (
        "wl_invoicing.supplier_bill.submit.confirm_title",
        "Submit this supplier bill?",
    ),
    (
        "wl_invoicing.supplier_bill.submit.confirm_message",
        "The bill can no longer be changed. A bill that may duplicate an earlier one is held until another user approves it.",
    ),
    (
        "wl_invoicing.supplier_bill.submit.done",
        "Supplier bill submitted.",
    ),
    (
        "wl_invoicing.supplier_bill.submit.held",
        "Supplier bill submitted and held: see the reasons on the bill.",
    ),
    ("wl_invoicing.supplier_bill.discard.action", "Delete"),
    (
        "wl_invoicing.supplier_bill.discard.confirm_title",
        "Delete this draft?",
    ),
    (
        "wl_invoicing.supplier_bill.discard.confirm_message",
        "The draft and its lines are deleted.",
    ),
    (
        "wl_invoicing.supplier_bill.discard.done",
        "Supplier bill deleted.",
    ),
    ("wl_invoicing.supplier_bill.approve.action", "Approve"),
    (
        "wl_invoicing.supplier_bill.approve.title",
        "Approve the supplier bill",
    ),
    (
        "wl_invoicing.supplier_bill.approve.description",
        "The bill is approved for payment. A held bill needs a reason, and not from the user who entered it.",
    ),
    ("wl_invoicing.supplier_bill.approve.submit", "Approve"),
    ("wl_invoicing.supplier_bill.void.action", "Void"),
    (
        "wl_invoicing.supplier_bill.void.title",
        "Void the supplier bill",
    ),
    (
        "wl_invoicing.supplier_bill.void.description",
        "A voided bill is kept but never paid. Give the reason.",
    ),
    ("wl_invoicing.supplier_bill.void.submit", "Void"),
    (
        "wl_invoicing.supplier_bill.errors.supplier",
        "No supplier has this party code.",
    ),
    (
        "wl_invoicing.supplier_bill.errors.date",
        "Enter a date like 2026-10-05.",
    ),
    (
        "wl_invoicing.supplier_bill.errors.entity",
        "Choose a legal entity from the list.",
    ),
    (
        "wl_invoicing.supplier_bill.errors.rate",
        "Enter a rate like 3.6725.",
    ),
    (
        "wl_invoicing.supplier_bill.errors.contract",
        "Give the contract type and its id together.",
    ),
    (
        "wl_invoicing.supplier_bill.errors.quantity",
        "Enter a quantity like 2 or 1.5.",
    ),
    (
        "wl_invoicing.supplier_bill.errors.amount",
        "Enter an amount like 150.00.",
    ),
    (
        "wl_invoicing.supplier_bill.errors.category",
        "Choose a VAT category from the list.",
    ),
    (
        "wl_invoicing.supplier_bill.errors.not_draft",
        "Only a draft supplier bill can be changed.",
    ),
    ("wl_invoicing.debit_note.title", "Supplier debit notes"),
    (
        "wl_invoicing.debit_note.empty",
        "No supplier debit notes yet",
    ),
    (
        "wl_invoicing.debit_note.waiting",
        "Waiting for the supplier bill",
    ),
    ("wl_invoicing.debit_note.status.draft", "Draft"),
    ("wl_invoicing.debit_note.status.issued", "Issued"),
    ("wl_invoicing.debit_note.status.settled", "Settled"),
    ("wl_invoicing.debit_note.status.cancelled", "Cancelled"),
    ("wl_invoicing.debit_note.origin.manual", "Manual"),
    (
        "wl_invoicing.debit_note.origin.return",
        "Return to supplier",
    ),
    (
        "wl_invoicing.debit_note.reason.price_difference",
        "Price difference",
    ),
    (
        "wl_invoicing.debit_note.reason.short_delivery",
        "Short delivery",
    ),
    ("wl_invoicing.debit_note.reason.quality", "Quality"),
    ("wl_invoicing.debit_note.reason.other", "Other"),
    ("wl_invoicing.debit_note.priced_at.po", "PO"),
    ("wl_invoicing.debit_note.priced_at.bill", "Bill"),
    (
        "wl_invoicing.debit_note.errors.not_draft",
        "Only a draft supplier debit note can be changed.",
    ),
    (
        "wl_invoicing.debit_note.errors.bill_state",
        "A debit note needs an approved, partially paid or paid supplier bill.",
    ),
    (
        "wl_invoicing.debit_note.errors.reason",
        "Give a reason for the debit note.",
    ),
    (
        "wl_invoicing.debit_note.errors.lines",
        "A debit note needs at least one line.",
    ),
    (
        "wl_invoicing.debit_note.errors.over_bill_line",
        "A line is above what is left on its bill line.",
    ),
    ("wl_invoicing.budget.title", "Company budgets"),
    ("wl_invoicing.budget.empty", "No company budgets yet"),
    (
        "wl_invoicing.budget.empty_hint",
        "The accountant prepares the budget of an entity for its fiscal year and submits it for approval.",
    ),
    ("wl_invoicing.budget.fields.entity", "Entity"),
    ("wl_invoicing.budget.fields.year", "Fiscal year"),
    ("wl_invoicing.budget.fields.version", "Version"),
    ("wl_invoicing.budget.fields.name", "Name"),
    ("wl_invoicing.budget.fields.status", "Status"),
    ("wl_invoicing.budget.fields.start", "Started as"),
    ("wl_invoicing.budget.fields.prepared_by", "Prepared by"),
    ("wl_invoicing.budget.fields.submitted_by", "Submitted by"),
    ("wl_invoicing.budget.fields.approved_by", "Approved by"),
    ("wl_invoicing.budget.fields.approved_at", "Approved on"),
    (
        "wl_invoicing.budget.fields.revision_reason",
        "Revision reason",
    ),
    (
        "wl_invoicing.budget.fields.return_reason",
        "Returned because",
    ),
    (
        "wl_invoicing.budget.fields.discard_reason",
        "Discarded because",
    ),
    ("wl_invoicing.budget.status.draft", "Draft"),
    ("wl_invoicing.budget.status.submitted", "Submitted"),
    ("wl_invoicing.budget.status.approved", "Approved"),
    ("wl_invoicing.budget.status.superseded", "Superseded"),
    ("wl_invoicing.budget.status.discarded", "Discarded"),
    ("wl_invoicing.budget.start.empty", "Empty"),
    (
        "wl_invoicing.budget.start.last_budget",
        "A copy of last year's budget",
    ),
    (
        "wl_invoicing.budget.start.last_actual",
        "Last year's actual",
    ),
    (
        "wl_invoicing.budget.start.revision",
        "A revision of the approved budget",
    ),
    ("wl_invoicing.budget.sections.summary", "Budget"),
    (
        "wl_invoicing.budget.sections.lines",
        "Amounts per category and month (AED, without VAT)",
    ),
    ("wl_invoicing.budget.lines.category", "Category"),
    ("wl_invoicing.budget.lines.total", "Total"),
    ("wl_invoicing.budget.lines.locked", "locked"),
    ("wl_invoicing.budget.lines.empty", "No amounts yet"),
    ("wl_invoicing.budget.new.title", "New budget"),
    ("wl_invoicing.budget.new.done", "The draft budget was made."),
    (
        "wl_invoicing.budget.new.year_hint",
        "Named by the calendar year of its first month.",
    ),
    (
        "wl_invoicing.budget.new.start_hint",
        "Chosen once. An entity and year with an approved budget get a revision that copies it.",
    ),
    (
        "wl_invoicing.budget.new.reason_hint",
        "Needed to submit a revision.",
    ),
    ("wl_invoicing.budget.new.cancel", "Cancel"),
    ("wl_invoicing.budget.new.save", "Make the draft"),
    ("wl_invoicing.budget.set.action", "Set an amount"),
    ("wl_invoicing.budget.set.title", "Set an amount"),
    (
        "wl_invoicing.budget.set.description",
        "The same amount for a category in each month of the range.",
    ),
    ("wl_invoicing.budget.set.submit", "Set"),
    ("wl_invoicing.budget.set.category", "Category"),
    ("wl_invoicing.budget.set.from", "From month"),
    ("wl_invoicing.budget.set.to", "To month"),
    (
        "wl_invoicing.budget.set.to_hint",
        "Leave empty for the one month.",
    ),
    ("wl_invoicing.budget.set.same_month", "Only the first month"),
    ("wl_invoicing.budget.set.amount", "Amount per month"),
    (
        "wl_invoicing.budget.set.amount_hint",
        "AED without VAT, like 20000.00.",
    ),
    ("wl_invoicing.budget.set.note", "Note"),
    ("wl_invoicing.budget.set.done", "The amounts were set."),
    ("wl_invoicing.budget.clear.action", "Remove a line"),
    ("wl_invoicing.budget.clear.title", "Remove a line"),
    (
        "wl_invoicing.budget.clear.description",
        "Takes one category and month out of the draft.",
    ),
    ("wl_invoicing.budget.clear.submit", "Remove"),
    ("wl_invoicing.budget.clear.month", "Month"),
    ("wl_invoicing.budget.clear.done", "The line was removed."),
    ("wl_invoicing.budget.import.action", "Import CSV"),
    ("wl_invoicing.budget.import.title", "Import amounts"),
    (
        "wl_invoicing.budget.import.description",
        "Paste rows of category code, month and amount. The Excel import of the platform is not built yet: save the workbook as CSV.",
    ),
    ("wl_invoicing.budget.import.submit", "Import"),
    ("wl_invoicing.budget.import.csv", "Rows"),
    (
        "wl_invoicing.budget.import.hint",
        "category,month,amount,note: for example marketing,2027-03,10000.00. A first line that starts with category is the header.",
    ),
    ("wl_invoicing.budget.import.dry_run", "Check only"),
    (
        "wl_invoicing.budget.import.dry_run_hint",
        "Reads and checks every row and stores nothing. Untick it to store the lines; a file with a rejected row stores nothing.",
    ),
    (
        "wl_invoicing.budget.import.empty",
        "Paste the rows to import.",
    ),
    ("wl_invoicing.budget.import.row", "Row"),
    (
        "wl_invoicing.budget.import.dry_run_ok",
        "{{ rows }} rows are valid. Nothing was stored.",
    ),
    (
        "wl_invoicing.budget.import.done",
        "{{ rows }} rows were stored.",
    ),
    (
        "wl_invoicing.budget.import.unknown_category",
        "There is no category {{ value }}.",
    ),
    (
        "wl_invoicing.budget.import.inactive_category",
        "The category {{ value }} is not active.",
    ),
    (
        "wl_invoicing.budget.import.bad_month",
        "{{ value }} is not a month like 2027-03.",
    ),
    (
        "wl_invoicing.budget.import.month_outside",
        "{{ value }} is not in the fiscal year of this budget.",
    ),
    (
        "wl_invoicing.budget.import.bad_amount",
        "{{ value }} is not an amount like 150.00.",
    ),
    (
        "wl_invoicing.budget.import.locked",
        "The line of {{ value }} is locked.",
    ),
    (
        "wl_invoicing.budget.import.repeated",
        "Rows {{ first }} and {{ second }} repeat the same category and month.",
    ),
    ("wl_invoicing.budget.reason.action", "Revision reason"),
    ("wl_invoicing.budget.reason.title", "Revision reason"),
    (
        "wl_invoicing.budget.reason.description",
        "Why this version changes the approved budget.",
    ),
    ("wl_invoicing.budget.reason.submit", "Save"),
    ("wl_invoicing.budget.reason.done", "The reason was saved."),
    ("wl_invoicing.budget.submit.action", "Submit for approval"),
    (
        "wl_invoicing.budget.submit.confirm_title",
        "Submit this budget?",
    ),
    (
        "wl_invoicing.budget.submit.confirm_message",
        "The holders of budgets.approve are asked to approve it. It cannot be changed while it waits.",
    ),
    (
        "wl_invoicing.budget.submit.done",
        "The budget was submitted for approval.",
    ),
    ("wl_invoicing.budget.discard.action", "Discard"),
    ("wl_invoicing.budget.discard.title", "Discard this draft?"),
    (
        "wl_invoicing.budget.discard.description",
        "The entity and year are free for a new version. An approved version stays in force.",
    ),
    ("wl_invoicing.budget.discard.reason", "Reason"),
    ("wl_invoicing.budget.discard.submit", "Discard the draft"),
    (
        "wl_invoicing.budget.discard.reason_required",
        "Give a reason to discard the draft.",
    ),
    (
        "wl_invoicing.budget.discard.done",
        "The draft was discarded.",
    ),
    (
        "wl_invoicing.budget.errors.entity",
        "Choose an entity from the list.",
    ),
    (
        "wl_invoicing.budget.errors.year",
        "Enter the fiscal year like 2027.",
    ),
    (
        "wl_invoicing.budget.errors.category",
        "Choose a category from the list.",
    ),
    (
        "wl_invoicing.budget.errors.month",
        "Choose a month of the fiscal year.",
    ),
    (
        "wl_invoicing.budget.errors.range",
        "The last month cannot be before the first.",
    ),
    (
        "wl_invoicing.budget.errors.amount",
        "Enter an amount like 150.00.",
    ),
    (
        "wl_invoicing.budget.errors.kind",
        "Choose a kind from the list.",
    ),
    (
        "wl_invoicing.budget.errors.party",
        "Enter the id of the party.",
    ),
    (
        "wl_invoicing.budget.problem.no_lines",
        "A budget needs at least one line.",
    ),
    (
        "wl_invoicing.budget.problem.bad_lines",
        "A line has an inactive category or a month outside the fiscal year.",
    ),
    (
        "wl_invoicing.budget.problem.revision_reason",
        "A revision needs a reason.",
    ),
    (
        "wl_invoicing.budget.problem.submitter_unknown",
        "Only a signed-in user can submit a budget.",
    ),
    (
        "wl_invoicing.budget.problem.no_approval_policy",
        "No approval policy is in force for company budgets.",
    ),
    (
        "wl_invoicing.budget.problem.year",
        "The fiscal year must be between 2000 and 2100.",
    ),
    (
        "wl_invoicing.budget.problem.entity",
        "The legal entity is not found or not active.",
    ),
    (
        "wl_invoicing.budget.problem.open_version",
        "{{ entity }} already has a draft or submitted budget for {{ year }}. Return, approve or discard it first.",
    ),
    (
        "wl_invoicing.budget.problem.no_last_budget",
        "There is no approved budget for {{ year }} to copy.",
    ),
    (
        "wl_invoicing.budget.problem.not_editable",
        "Only the lines of a draft budget can be changed.",
    ),
    (
        "wl_invoicing.budget.problem.month_outside",
        "{{ month }} is not in the fiscal year of this budget.",
    ),
    (
        "wl_invoicing.budget.problem.negative",
        "An amount cannot be negative.",
    ),
    (
        "wl_invoicing.budget.problem.category",
        "The category is not found or not active.",
    ),
    (
        "wl_invoicing.budget.problem.locked",
        "The {{ month }} line of {{ category }} is locked: the month had ended when this version was made.",
    ),
    ("wl_invoicing.budget.category.title", "Budget categories"),
    ("wl_invoicing.budget.category.new_title", "New category"),
    (
        "wl_invoicing.budget.category.edit_title",
        "Change the category",
    ),
    ("wl_invoicing.budget.category.code", "Code"),
    ("wl_invoicing.budget.category.name_en", "Name in English"),
    ("wl_invoicing.budget.category.name_ar", "Name in Arabic"),
    ("wl_invoicing.budget.category.kind", "Kind"),
    (
        "wl_invoicing.budget.category.kind_hint",
        "The kind cannot change once the category has a budget line or an actual.",
    ),
    ("wl_invoicing.budget.category.parent", "Parent"),
    (
        "wl_invoicing.budget.category.parent_hint",
        "The code of the category this one is a part of.",
    ),
    ("wl_invoicing.budget.category.owner_role", "Owner role"),
    (
        "wl_invoicing.budget.category.owner_role_hint",
        "Told when the actual reaches the budget; none: the finance manager only.",
    ),
    ("wl_invoicing.budget.category.owner_party", "Owner party"),
    (
        "wl_invoicing.budget.category.owner_party_hint",
        "Owner drawings only: the id of the owner or partner whose drawings it records.",
    ),
    ("wl_invoicing.budget.category.keys", "Collects"),
    (
        "wl_invoicing.budget.category.keys_hint",
        "One key per line as kind: value, with the kinds charge_type, item_group, cost_code, expense_category, payroll and ledger_account. A key belongs to one category.",
    ),
    ("wl_invoicing.budget.category.active", "Active"),
    ("wl_invoicing.budget.category.on", "Active"),
    ("wl_invoicing.budget.category.off", "Inactive"),
    ("wl_invoicing.budget.category.edit", "Change"),
    (
        "wl_invoicing.budget.category.empty",
        "No budget categories yet",
    ),
    ("wl_invoicing.budget.category.save", "Save the category"),
    (
        "wl_invoicing.budget.category.saved",
        "The category was saved.",
    ),
    (
        "wl_invoicing.budget.category.problem.blank",
        "Give the category a code and a name.",
    ),
    (
        "wl_invoicing.budget.category.problem.code_used",
        "The code {{ code }} is used by another category.",
    ),
    (
        "wl_invoicing.budget.category.problem.owner_party",
        "An owner drawings category needs the owner party whose drawings it records.",
    ),
    (
        "wl_invoicing.budget.category.problem.kind_locked",
        "The kind cannot change: the category has a budget line or an actual. Make a new category instead.",
    ),
    (
        "wl_invoicing.budget.category.problem.parent",
        "There is no other category with the code {{ code }} to be the parent.",
    ),
    (
        "wl_invoicing.budget.category.problem.key_kind",
        "{{ key }} cannot be mapped to a category of kind {{ kind }}.",
    ),
    (
        "wl_invoicing.budget.category.problem.mapping_used",
        "{{ key }} is already mapped to {{ category }}; a mapping key belongs to one category.",
    ),
    (
        "wl_invoicing.budget.category.problem.mapping_line",
        "Cannot read {{ line }}: write one key per line as kind: value, for example cost_code: fuel.",
    ),
    ("wl_invoicing.budget.kind.income", "Income"),
    ("wl_invoicing.budget.kind.expense", "Expense"),
    ("wl_invoicing.budget.kind.owner_drawings", "Owner drawings"),
    (
        "wl_invoicing.budget_reminder.subject",
        "Budget needed for {{ entity }}, fiscal year {{ year }}",
    ),
    (
        "wl_invoicing.budget_reminder.body",
        "{{ entity }} has no approved company budget for fiscal year {{ year }}, which {{ when }}. Prepare the budget and submit it for approval: {{ link }}",
    ),
    ("wl_invoicing.cash.title", "Cash-flow forecast"),
    (
        "wl_invoicing.cash.subtitle",
        "What is expected to come in and go out, week by week, from the documents recorded so far. Read only; amounts in whole AED.",
    ),
    ("wl_invoicing.cash.entity", "Legal entity"),
    ("wl_invoicing.cash.all_entities", "All entities"),
    ("wl_invoicing.cash.show", "Show"),
    ("wl_invoicing.cash.week", "Week"),
    ("wl_invoicing.cash.row.opening", "Opening balance"),
    ("wl_invoicing.cash.row.receipts", "Expected receipts"),
    ("wl_invoicing.cash.row.payments", "Expected payments"),
    ("wl_invoicing.cash.row.balance", "Projected balance"),
    ("wl_invoicing.cash.row.memo", "not counted"),
    ("wl_invoicing.cash.memo.disputed", "Disputed amounts"),
    ("wl_invoicing.cash.memo.bounced_cheque", "Bounced cheques"),
    (
        "wl_invoicing.cash.memo.blocked_party",
        "Receivables of blocked parties",
    ),
    (
        "wl_invoicing.cash.memo.duplicate_bill",
        "Bills held as possible duplicates",
    ),
    ("wl_invoicing.cash.kind.invoice", "Invoice"),
    ("wl_invoicing.cash.kind.cheque_in", "Cheque received"),
    ("wl_invoicing.cash.kind.bill", "Supplier bill"),
    ("wl_invoicing.cash.kind.cheque_out", "Cheque issued"),
    ("wl_invoicing.cash.kind.refund", "Refund"),
    ("wl_invoicing.cash.kind.vat", "VAT payable"),
    ("wl_invoicing.cash.kind.source", "Other"),
    ("wl_invoicing.cash.kind.budget_fill", "Budget fill"),
    ("wl_invoicing.cash.row.fill", "Budget fill"),
    ("wl_invoicing.cash.after", "After week"),
    (
        "wl_invoicing.cash.later_hint",
        "These receipts are expected after the last week because of the days-late adjustment. They are not counted in the grid.",
    ),
    ("wl_invoicing.cash.drawings", "Owner drawings"),
    (
        "wl_invoicing.cash.warning.no_balance",
        "no balance known: counted as zero",
    ),
    (
        "wl_invoicing.cash.warning.stale",
        "the balance is old, as of",
    ),
    (
        "wl_invoicing.cash.warning.unpriced",
        "left out: no exchange rate for its currency",
    ),
    (
        "wl_invoicing.cash.warning.vat_past_due",
        "VAT for the period past due: check that it is paid",
    ),
    (
        "wl_invoicing.cash.warning.no_budget",
        "no approved budget for the months",
    ),
    ("wl_invoicing.cash.accounts", "Bank accounts"),
    ("wl_invoicing.cash.account", "Bank account"),
    ("wl_invoicing.cash.balance_of", "Balance"),
    ("wl_invoicing.cash.as_of", "As of"),
    ("wl_invoicing.cash.source", "Source"),
    ("wl_invoicing.cash.movements", "Payments since"),
    ("wl_invoicing.cash.opening", "Opening"),
    ("wl_invoicing.cash.source.statement", "Bank statement"),
    ("wl_invoicing.cash.source.manual", "Entered by hand"),
    ("wl_invoicing.cash.source.none", "None"),
    ("wl_invoicing.cash.no_accounts", "No bank account known"),
    (
        "wl_invoicing.cash.no_accounts_hint",
        "Enter an opening cash balance, or upload a bank statement with its closing balance.",
    ),
    (
        "wl_invoicing.cash.balances.add",
        "Enter an opening cash balance",
    ),
    ("wl_invoicing.cash.balances.amount", "Balance"),
    (
        "wl_invoicing.cash.balances.amount_hint",
        "In dirhams, like 60000.00; below 0 for an overdraft. It is the balance at the end of the day.",
    ),
    ("wl_invoicing.cash.balances.note", "Note"),
    ("wl_invoicing.cash.balances.save", "Save"),
    (
        "wl_invoicing.cash.balances.history",
        "Opening cash balances",
    ),
    ("wl_invoicing.cash.balances.status", "Status"),
    ("wl_invoicing.cash.balances.voided", "Voided"),
    ("wl_invoicing.cash.balances.void", "Void"),
    ("wl_invoicing.cash.balances.void_title", "Void this balance"),
    ("wl_invoicing.cash.balances.void_reason", "Reason"),
    (
        "wl_invoicing.cash.balances.reason_required",
        "Give the reason for voiding.",
    ),
    (
        "wl_invoicing.cash.balances.bad_entity",
        "Choose a legal entity.",
    ),
    (
        "wl_invoicing.cash.balances.bad_amount",
        "Enter the balance in dirhams, like 60000.00.",
    ),
    (
        "wl_invoicing.cash.balances.bad_date",
        "Give the bank account and a day that is not in the future.",
    ),
    (
        "wl_invoicing.cash.balances.entered_toast",
        "Balance entered.",
    ),
    ("wl_invoicing.cash.balances.voided_toast", "Balance voided."),
    ("wl_invoicing.cash.cell.line", "Line"),
    ("wl_invoicing.cash.cell.kind", "Kind"),
    ("wl_invoicing.cash.cell.party", "Party"),
    ("wl_invoicing.cash.cell.date", "Expected"),
    ("wl_invoicing.cash.cell.amount", "Amount"),
    ("wl_invoicing.cash.cell.flags", "Notes"),
    ("wl_invoicing.cash.cell.total", "Total"),
    ("wl_invoicing.cash.cell.empty", "Nothing in this cell"),
    (
        "wl_invoicing.cash.cell.empty_hint",
        "No document is expected in this week.",
    ),
    ("wl_invoicing.cash.flag.not_approved", "not yet approved"),
    ("wl_invoicing.cash.flag.estimate", "estimate"),
    ("wl_invoicing.cash.flag.not_counted", "not counted"),
    ("wl_invoicing.budget.report.title", "Budget vs actual"),
    (
        "wl_invoicing.budget.report.subtitle",
        "AED without recoverable VAT. Owner drawings are never part of the operating result.",
    ),
    ("wl_invoicing.budget.report.entity", "Entity"),
    ("wl_invoicing.budget.report.all_entities", "All entities"),
    ("wl_invoicing.budget.report.month", "Month"),
    ("wl_invoicing.budget.report.ytd", "Year to date"),
    ("wl_invoicing.budget.report.show", "Show"),
    (
        "wl_invoicing.budget.report.no_budget",
        "No approved budget for the year:",
    ),
    ("wl_invoicing.budget.report.category", "Category"),
    ("wl_invoicing.budget.report.budget", "Budget"),
    ("wl_invoicing.budget.report.actual", "Actual"),
    ("wl_invoicing.budget.report.variance", "Variance"),
    ("wl_invoicing.budget.report.percent", "Variance %"),
    ("wl_invoicing.budget.report.flag", "Status"),
    ("wl_invoicing.budget.report.unfavourable", "Unfavourable"),
    ("wl_invoicing.budget.report.income", "Income"),
    ("wl_invoicing.budget.report.income_total", "Total income"),
    ("wl_invoicing.budget.report.expense", "Expense"),
    ("wl_invoicing.budget.report.expense_total", "Total expense"),
    ("wl_invoicing.budget.report.operating", "Operating result"),
    ("wl_invoicing.budget.report.drawings", "Owner drawings"),
    (
        "wl_invoicing.budget.report.drawings_total",
        "Total owner drawings",
    ),
    (
        "wl_invoicing.budget.report.after_drawings",
        "Result after drawings",
    ),
    ("wl_invoicing.budget.report.uncategorised", "Uncategorised"),
    (
        "wl_invoicing.budget.report.mapping_changed",
        "The actual follows the mapping of today; it was last changed on",
    ),
    (
        "wl_invoicing.budget.report.drawings_lines",
        "Owner drawings lines",
    ),
    ("wl_invoicing.budget.report.description", "Description"),
    ("wl_invoicing.budget.report.payee", "Payee"),
    ("wl_invoicing.budget.report.amount", "Amount"),
    ("wl_invoicing.budget.report.self_approved", "Self-approved"),
    ("wl_invoicing.budget.report.open", "Open"),
    ("wl_invoicing.drawings.title", "Owner drawings"),
    (
        "wl_invoicing.drawings.total",
        "Drawing, with the VAT that is not recovered",
    ),
    ("wl_invoicing.drawings.status", "Approval of the drawing"),
    ("wl_invoicing.drawings.status.none", "Not asked for yet"),
    ("wl_invoicing.drawings.status.draft", "Draft"),
    (
        "wl_invoicing.drawings.status.requested",
        "Waiting for a holder of drawings.approve",
    ),
    ("wl_invoicing.drawings.status.approved", "Approved"),
    ("wl_invoicing.drawings.status.rejected", "Not approved"),
    (
        "wl_invoicing.drawings.status.self_approved",
        "Approved by the sole owner (self-approved)",
    ),
    (
        "wl_invoicing.drawings.rejected_because",
        "Not approved because",
    ),
    (
        "wl_invoicing.drawings.open_item",
        "How drawings are treated for VAT and corporate tax waits for the accountant's answer (open item A-16). Meanwhile the input VAT on a drawing is not recovered and is shown as blocked in the VAT summary. The bill is approved only after the drawing is approved.",
    ),
    (
        "wl_invoicing.drawings.hidden",
        "Owner drawing (shown to holders of drawings.approve)",
    ),
    ("wl_invoicing.drawings.request.action", "Ask for approval"),
    (
        "wl_invoicing.drawings.request.title",
        "Ask for the approval of the drawing?",
    ),
    (
        "wl_invoicing.drawings.request.message",
        "The holders of drawings.approve, other than you, are asked to approve it.",
    ),
    ("wl_invoicing.drawings.recode.action", "Recode a line"),
    ("wl_invoicing.drawings.recode.title", "Recode a line"),
    (
        "wl_invoicing.drawings.recode.description",
        "Codes a line of this bill to another cost code. The change is logged and shown to whoever approves the bill.",
    ),
    ("wl_invoicing.drawings.recode.submit", "Recode"),
    ("wl_invoicing.drawings.recode.line", "Line"),
    ("wl_invoicing.drawings.recode.code", "Cost code"),
    (
        "wl_invoicing.drawings.recode.hint",
        "For example other, or marketing.",
    ),
    ("wl_invoicing.drawings.recode.from", "Was"),
    ("wl_invoicing.drawings.recode.to", "Now"),
    ("wl_invoicing.drawings.recode.by", "Recoded by"),
    ("wl_invoicing.drawings.recodings", "Recoded lines"),
    (
        "wl_invoicing.drawings.requested",
        "The approval was asked for.",
    ),
    (
        "wl_invoicing.drawings.none",
        "The bill has no owner drawing.",
    ),
    ("wl_invoicing.drawings.recoded", "The line was recoded."),
    (
        "wl_invoicing.drawings.errors.line",
        "Choose a line from the list.",
    ),
    (
        "wl_invoicing.drawings.problem.requester_unknown",
        "Only a signed-in user can ask for the approval of a drawing.",
    ),
    (
        "wl_invoicing.drawings.problem.no_approval_policy",
        "No approval policy is in force for owner drawings.",
    ),
    (
        "wl_invoicing.drawings.problem.waiting",
        "The bill has an owner drawing that waits for the approval of a holder of drawings.approve.",
    ),
    (
        "wl_invoicing.drawings.problem.rejected",
        "The owner drawing was not approved: {{ reason }}. Recode or remove the line, or ask again.",
    ),
    (
        "wl_invoicing.drawings.problem.needed",
        "The bill has an owner drawing that needs the approval of a holder of drawings.approve first.",
    ),
    (
        "wl_invoicing.drawings.problem.bill_state",
        "The drawing of a bill can be asked for or recoded only before the bill is approved.",
    ),
    (
        "wl_invoicing.drawings.problem.not_found",
        "The drawing is not found.",
    ),
    (
        "wl_invoicing.drawings.problem.amount_changed",
        "The drawing changed while it waited; ask for its approval again.",
    ),
    (
        "wl_invoicing.drawings.problem.code_blank",
        "Give the cost code.",
    ),
    (
        "wl_invoicing.drawings.problem.cost_object",
        "A line coded to owner drawings cannot have a cost object.",
    ),
    (
        "wl_invoicing.drawings.problem.line_cost_object",
        "Line {{ line }} is coded to owner drawings, which are never posted to a cost object.",
    ),
    (
        "wl_invoicing.drawing_self_approved.subject",
        "Owner drawing recorded without a second approver",
    ),
    (
        "wl_invoicing.drawing_self_approved.body",
        "The sole owner recorded the drawing on supplier bill {{ bill }} ({{ amount }}) without a second approver. See the bill: {{ link }}",
    ),
    (
        "wl_invoicing.drawing_payment_self_approved.subject",
        "Owner drawing paid without a second approver",
    ),
    (
        "wl_invoicing.drawing_payment_self_approved.body",
        "The sole owner asked for an owner drawing of {{ amount }} ({{ category }}) without a second approver, and it is recorded. See the payments: {{ link }}",
    ),
    ("wl_invoicing.drawings.payment", "Owner drawing paid"),
    ("wl_invoicing.drawings.repayment", "Owner drawing repaid"),
    (
        "wl_invoicing.drawings.problem.category",
        "The category is not an owner drawings category.",
    ),
    (
        "wl_invoicing.drawing_corrected.subject",
        "A bill with an approved owner drawing was voided",
    ),
    (
        "wl_invoicing.drawing_corrected.body",
        "Supplier bill {{ bill }} ({{ amount }}) had an approved owner drawing and was voided: {{ reason }}. See the bill: {{ link }}",
    ),
    (
        "wl_invoicing.vat.blocked_drawings",
        "Input VAT blocked on owner drawings",
    ),
    (
        "wl_invoicing.vat.blocked_drawings_note",
        "Not recovered until the accountant answers the open item on drawings (A-16).",
    ),
    (
        "wl_invoicing.cash_alert.subject",
        "Cash forecast: {{ entity }} falls below {{ threshold }} in week {{ week }}",
    ),
    (
        "wl_invoicing.cash_alert.body",
        "The projected balance of {{ entity }} is {{ balance }} at the end of week {{ week }} ({{ from }} to {{ to }}), below the threshold of {{ threshold }}. Largest payments that week: {{ payments }}. {{ warnings }} See the forecast: {{ link }}",
    ),
    (
        "wl_invoicing.tasks.kind.statement_balance_stale",
        "Bank balance needed",
    ),
    (
        "wl_invoicing.budget_variance.subject",
        "{{ category }} reached its budget in {{ month }} ({{ entity }})",
    ),
    (
        "wl_invoicing.budget_variance.body",
        "In {{ month }} {{ entity }} has {{ actual }} of actual in {{ category }} against a budget of {{ budget }} ({{ percent }} used). See the report: {{ link }}",
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
