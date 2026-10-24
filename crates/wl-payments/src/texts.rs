//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_payments.title", "Payments"),
    ("wl_payments.bad_date", "Enter a date as YYYY-MM-DD."),
    ("wl_payments.expense_claims.title", "Expense claims"),
    (
        "wl_payments.expense_claims.description",
        "Spending staff paid for the business. A line in the Owner drawings category waits for the approval of an owner other than the claimant.",
    ),
    ("wl_payments.expense_claims.empty", "No claims yet"),
    (
        "wl_payments.expense_claims.empty_hint",
        "Record a claim to see it here.",
    ),
    ("wl_payments.expense_claims.new", "Record a claim"),
    ("wl_payments.expense_claims.record", "Record"),
    ("wl_payments.expense_claims.recorded", "Claim recorded."),
    ("wl_payments.expense_claims.claim_title", "For"),
    ("wl_payments.expense_claims.total", "Total"),
    ("wl_payments.expense_claims.actions", "Actions"),
    ("wl_payments.expense_claims.claimant", "Claimant"),
    ("wl_payments.expense_claims.vat", "VAT"),
    ("wl_payments.expense_claims.note", "Note"),
    ("wl_payments.expense_claims.drawing", "Owner drawings"),
    ("wl_payments.expense_claims.reason", "Reason"),
    (
        "wl_payments.expense_claims.reason_hint",
        "The claimant reads the reason.",
    ),
    ("wl_payments.expense_claims.status.draft", "Draft"),
    ("wl_payments.expense_claims.status.submitted", "Submitted"),
    ("wl_payments.expense_claims.status.approved", "Approved"),
    ("wl_payments.expense_claims.status.rejected", "Rejected"),
    ("wl_payments.expense_claims.line.category", "Category"),
    ("wl_payments.expense_claims.line.date", "Date"),
    ("wl_payments.expense_claims.line.amount", "Amount paid"),
    (
        "wl_payments.expense_claims.line.amount_hint",
        "In dirhams, VAT included.",
    ),
    ("wl_payments.expense_claims.line.vat", "VAT in it"),
    ("wl_payments.expense_claims.line.merchant", "Paid to"),
    ("wl_payments.expense_claims.line.description", "For"),
    ("wl_payments.expense_claims.submit.action", "Submit"),
    (
        "wl_payments.expense_claims.submit.title",
        "Submit the claim",
    ),
    (
        "wl_payments.expense_claims.submit.message",
        "A line in the Owner drawings category is sent to an owner for approval.",
    ),
    ("wl_payments.expense_claims.submit.done", "Claim submitted."),
    ("wl_payments.expense_claims.approve.action", "Approve"),
    (
        "wl_payments.expense_claims.approve.title",
        "Approve the claim",
    ),
    (
        "wl_payments.expense_claims.approve.message",
        "You cannot approve your own claim, or one whose owner drawing is not approved.",
    ),
    ("wl_payments.expense_claims.approve.done", "Claim approved."),
    ("wl_payments.expense_claims.reject.action", "Reject"),
    (
        "wl_payments.expense_claims.reject.title",
        "Reject the claim",
    ),
    ("wl_payments.expense_claims.reject.done", "Claim rejected."),
    ("wl_payments.expense_claims.return.action", "Return"),
    (
        "wl_payments.expense_claims.return.title",
        "Return the claim",
    ),
    (
        "wl_payments.expense_claims.return.done",
        "Claim returned to the claimant.",
    ),
    (
        "wl_payments.expense_claims.problem.no_lines",
        "A claim needs at least one line.",
    ),
    (
        "wl_payments.expense_claims.problem.amount",
        "Every line needs an amount above 0, and its VAT cannot be more than the amount.",
    ),
    (
        "wl_payments.expense_claims.problem.category",
        "The expense category of a line is unknown or not in use.",
    ),
    (
        "wl_payments.expense_claims.problem.entity",
        "The legal entity is unknown.",
    ),
    (
        "wl_payments.expense_claims.problem.not_draft",
        "Only a draft claim can be changed.",
    ),
    (
        "wl_payments.expense_claims.problem.own_claim",
        "You cannot approve your own claim.",
    ),
    (
        "wl_payments.expense_claims.problem.drawing_waits",
        "The claim has an owner drawing that waits for the approval of an owner.",
    ),
    (
        "wl_payments.expense_claims.problem.drawing_rejected",
        "The owner drawing of the claim was not approved ({reason}). Return the claim, recode or remove the line, and submit it again.",
    ),
    (
        "wl_payments.expense_claims.problem.own_drawing",
        "The claimant cannot approve their own drawing.",
    ),
    ("wl_payments.bank_statements.title", "Bank statements"),
    (
        "wl_payments.bank_statements.description",
        "Uploaded statements with the balance at the end of each period. The cash-flow forecast starts from the latest balance of a completed statement.",
    ),
    ("wl_payments.bank_statements.empty", "No statements yet"),
    (
        "wl_payments.bank_statements.empty_hint",
        "Record a statement upload to see it here.",
    ),
    ("wl_payments.bank_statements.upload", "Record a statement"),
    ("wl_payments.bank_statements.record", "Record"),
    (
        "wl_payments.bank_statements.recorded",
        "Statement recorded.",
    ),
    ("wl_payments.bank_statements.bank_account", "Bank account"),
    ("wl_payments.bank_statements.file_name", "File"),
    ("wl_payments.bank_statements.format", "Format"),
    ("wl_payments.bank_statements.period", "Period"),
    ("wl_payments.bank_statements.period_start", "Period start"),
    ("wl_payments.bank_statements.period_end", "Period end"),
    (
        "wl_payments.bank_statements.closing_balance",
        "Closing balance",
    ),
    (
        "wl_payments.bank_statements.closing_balance_hint",
        "In dirhams, like 125000.00 or -300.00. Enter it when the file carries none; it is the balance at the end of the last day of the period.",
    ),
    (
        "wl_payments.bank_import_failed.subject",
        "A bank statement file was not recognised",
    ),
    (
        "wl_payments.bank_import_failed.body",
        "The statement file {{ file }} could not be read: {{ reason }} See the statements: {{ link }}",
    ),
    (
        "wl_payments.bank_line_drawing_rejected.subject",
        "An owner drawing on a bank statement line was not approved",
    ),
    (
        "wl_payments.bank_line_drawing_rejected.body",
        "The line of {{ amount }} dated {{ date }} marked as an owner drawing was not approved ({{ reason }}). It is unmatched again: {{ link }}",
    ),
    ("wl_payments.bank_statements.content", "File text"),
    (
        "wl_payments.bank_statements.content_hint",
        "Paste the CSV (header date, description, amount; optional reference, counterparty) or the MT940 text.",
    ),
    ("wl_payments.bank_statements.lines", "Lines"),
    ("wl_payments.bank_statements.actions", "Actions"),
    ("wl_payments.bank_statements.no_lines", "No lines"),
    ("wl_payments.bank_statements.open", "Open"),
    ("wl_payments.bank_statements.parse.action", "Read the file"),
    (
        "wl_payments.bank_statements.parse.title",
        "Read the statement file?",
    ),
    (
        "wl_payments.bank_statements.parse.message",
        "The lines are created for review. A file that is not recognised fails the import.",
    ),
    (
        "wl_payments.bank_statements.parse.done",
        "The file was read.",
    ),
    ("wl_payments.bank_statements.complete.action", "Complete"),
    (
        "wl_payments.bank_statements.complete.title",
        "Complete the import?",
    ),
    (
        "wl_payments.bank_statements.complete.message",
        "Every line is matched or ignored. The closing balance becomes the account balance.",
    ),
    (
        "wl_payments.bank_statements.complete.done",
        "The import is completed.",
    ),
    ("wl_payments.bank_statements.revert.action", "Revert"),
    (
        "wl_payments.bank_statements.revert.title",
        "Revert the import?",
    ),
    (
        "wl_payments.bank_statements.revert.message",
        "The lines are removed and no balance is read from this import.",
    ),
    (
        "wl_payments.bank_statements.revert.done",
        "The import was reverted.",
    ),
    ("wl_payments.bank_statements.line.date", "Date"),
    ("wl_payments.bank_statements.line.amount", "Amount"),
    (
        "wl_payments.bank_statements.line.description",
        "Description",
    ),
    ("wl_payments.bank_statements.line.reference", "Reference"),
    ("wl_payments.bank_statements.line.status", "Line"),
    (
        "wl_payments.bank_statements.line.status.unmatched",
        "Unmatched",
    ),
    (
        "wl_payments.bank_statements.line.status.suggested",
        "Suggested",
    ),
    ("wl_payments.bank_statements.line.status.matched", "Matched"),
    ("wl_payments.bank_statements.line.status.ignored", "Ignored"),
    ("wl_payments.bank_statements.line.ignore.action", "Ignore"),
    (
        "wl_payments.bank_statements.line.ignore.title",
        "Ignore this line",
    ),
    (
        "wl_payments.bank_statements.line.ignore.description",
        "Leave the line out of the reconciliation, with the reason.",
    ),
    ("wl_payments.bank_statements.line.ignore.reason", "Reason"),
    ("wl_payments.bank_statements.line.ignore.submit", "Ignore"),
    (
        "wl_payments.bank_statements.line.ignore.done",
        "The line is ignored.",
    ),
    (
        "wl_payments.bank_statements.line.drawing.action",
        "Owner drawing",
    ),
    (
        "wl_payments.bank_statements.line.drawing.title",
        "Mark as an owner drawing",
    ),
    (
        "wl_payments.bank_statements.line.drawing.description",
        "Asks for the drawings approval; the line stays suggested until it is decided.",
    ),
    (
        "wl_payments.bank_statements.line.drawing.category",
        "Drawings category",
    ),
    (
        "wl_payments.bank_statements.line.drawing.submit",
        "Ask for approval",
    ),
    (
        "wl_payments.bank_statements.line.drawing.done",
        "The owner drawing was asked for.",
    ),
    ("wl_payments.bank_statements.no_balance", "Not given"),
    (
        "wl_payments.bank_statements.bad_upload",
        "Check the bank account, the file, the format and that the period does not end before it starts.",
    ),
    ("wl_payments.bank_statements.format.csv", "CSV"),
    ("wl_payments.bank_statements.format.excel", "Excel"),
    ("wl_payments.bank_statements.format.mt940", "MT940"),
    ("wl_payments.bank_statements.status.uploaded", "Uploaded"),
    ("wl_payments.bank_statements.status.in_review", "In review"),
    ("wl_payments.bank_statements.status.completed", "Completed"),
    ("wl_payments.bank_statements.status.failed", "Failed"),
    ("wl_payments.bank_statements.status.reverted", "Reverted"),
    ("wl_payments.empty", "No payments yet"),
    (
        "wl_payments.empty_hint",
        "Record a payment to see it here with its receipt number.",
    ),
    ("wl_payments.new", "Record payment"),
    ("wl_payments.record", "Record"),
    ("wl_payments.cancel", "Cancel"),
    ("wl_payments.recorded", "Payment recorded."),
    ("wl_payments.no_number", "Pending"),
    ("wl_payments.fields.number", "Number"),
    ("wl_payments.fields.date", "Date"),
    ("wl_payments.fields.party", "Party"),
    ("wl_payments.fields.party_code", "Party code"),
    (
        "wl_payments.fields.party_code_hint",
        "The code of the party, like P-00001.",
    ),
    ("wl_payments.fields.entity", "Legal entity"),
    ("wl_payments.fields.method", "Method"),
    ("wl_payments.fields.direction", "Direction"),
    ("wl_payments.fields.amount", "Amount"),
    (
        "wl_payments.fields.amount_hint",
        "In dirhams, like 1500.00.",
    ),
    ("wl_payments.fields.currency", "Currency"),
    ("wl_payments.fields.allocated", "Allocated"),
    ("wl_payments.fields.unallocated", "Unallocated"),
    ("wl_payments.fields.status", "Status"),
    ("wl_payments.fields.reference", "Reference"),
    ("wl_payments.fields.bank_account", "Bank account"),
    ("wl_payments.fields.source", "Source"),
    ("wl_payments.fields.cheque_number", "Cheque number"),
    ("wl_payments.fields.cheque_bank", "Cheque bank"),
    ("wl_payments.fields.cheque_date", "Cheque date"),
    ("wl_payments.fields.created_at", "Created"),
    ("wl_payments.fields.target", "Applied to"),
    ("wl_payments.fields.invoice_id", "Invoice id"),
    (
        "wl_payments.fields.invoice_id_hint",
        "The id of an open invoice of the same party, as in /invoices/{id}.",
    ),
    ("wl_payments.fields.set_off_invoice", "Set-off: invoice id"),
    (
        "wl_payments.fields.set_off_bill",
        "Set-off: supplier bill id",
    ),
    (
        "wl_payments.fields.set_off_hint",
        "For the method set-off only: the party's open invoice and its approved supplier bill, both settled for the amount.",
    ),
    (
        "wl_payments.set_off.documents_required",
        "A set-off names the invoice and the supplier bill it settles.",
    ),
    (
        "wl_payments.set_off.exception_requested",
        "The party is on hold. An exception was requested; the set-off waits for its approval.",
    ),
    ("wl_payments.fields.purpose", "Purpose"),
    ("wl_payments.purpose.none", "Ordinary payment"),
    ("wl_payments.purpose.owner_drawings", "Owner drawings"),
    ("wl_payments.fields.drawing_category", "Drawings category"),
    (
        "wl_payments.fields.drawing_category_hint",
        "Owner drawings only: the code of the drawings category, like owner_drawings. It fixes the payee: the category's owner party.",
    ),
    (
        "wl_payments.fields.drawing_direction",
        "Drawing or repayment",
    ),
    (
        "wl_payments.drawing_direction.out",
        "Drawing: money out to the owner, after approval",
    ),
    (
        "wl_payments.drawing_direction.in",
        "Repayment: money in from the owner, no approval",
    ),
    (
        "wl_payments.drawings.requested",
        "The drawing is saved and waits for the approval of another holder of drawings.approve. Nothing is recorded yet.",
    ),
    (
        "wl_payments.drawings.recorded",
        "The drawing is approved and recorded.",
    ),
    ("wl_payments.drawings.title", "Owner drawing"),
    ("wl_payments.drawings.details", "Drawing"),
    ("wl_payments.drawings.reason", "Reason"),
    (
        "wl_payments.drawings.self_approved",
        "Approved without a second approver (sole owner).",
    ),
    ("wl_payments.drawings.payment", "Open the payment"),
    (
        "wl_payments.drawings.status.requested",
        "Waiting for approval",
    ),
    ("wl_payments.drawings.status.approved", "Approved"),
    ("wl_payments.drawings.status.recorded", "Recorded"),
    ("wl_payments.drawings.status.rejected", "Not approved"),
    (
        "wl_payments.drawings.reversed.subject",
        "An owner drawing was reversed",
    ),
    (
        "wl_payments.drawings.reversed.body",
        "A payment of {{ amount }} with purpose owner drawings was reversed ({{ reason }}). See the payment: {{ link }}",
    ),
    ("wl_payments.sections.details", "Payment"),
    ("wl_payments.sections.allocations", "Allocations"),
    ("wl_payments.sections.reversal", "Reversal"),
    ("wl_payments.sections.tasks", "Follow-up tasks"),
    ("wl_payments.sections.history", "History"),
    ("wl_payments.no_allocations", "Nothing allocated yet"),
    ("wl_payments.no_tasks", "No follow-up tasks"),
    ("wl_payments.no_history", "No history yet"),
    ("wl_payments.removed", "Removed"),
    ("wl_payments.active", "Active"),
    ("wl_payments.removed_on", "Removed on"),
    ("wl_payments.removed_reason", "Reason"),
    ("wl_payments.status.pending", "Pending"),
    ("wl_payments.status.received", "Received"),
    ("wl_payments.status.failed", "Failed"),
    ("wl_payments.status.charged_back", "Charged back"),
    ("wl_payments.status.reversed", "Reversed"),
    ("wl_payments.method.cash", "Cash"),
    ("wl_payments.method.card", "Card"),
    ("wl_payments.method.transfer", "Bank transfer"),
    ("wl_payments.method.cheque", "Cheque"),
    ("wl_payments.method.gateway", "Online"),
    ("wl_payments.method.direct_debit", "Direct debit"),
    ("wl_payments.method.wallet", "Wallet"),
    ("wl_payments.method.set_off", "Set-off"),
    ("wl_payments.direction.in", "Receipt"),
    ("wl_payments.direction.out", "Payment voucher"),
    ("wl_payments.source.manual", "Entered by a user"),
    ("wl_payments.source.cheque", "Cheque register"),
    ("wl_payments.source.gateway", "Gateway"),
    ("wl_payments.source.bank_statement", "Bank statement"),
    ("wl_payments.source.system", "System"),
    ("wl_payments.reason.entered_in_error", "Entered in error"),
    ("wl_payments.reason.bank_return", "Returned by the bank"),
    ("wl_payments.reason.chargeback", "Chargeback"),
    ("wl_payments.reverse.action", "Reverse"),
    ("wl_payments.reverse.title", "Reverse this payment"),
    (
        "wl_payments.reverse.description",
        "The payment is marked reversed and its allocations are removed (they stay on the page). Invoices are reopened by the invoicing module.",
    ),
    ("wl_payments.reverse.reason", "Reason"),
    ("wl_payments.reverse.note", "Note"),
    ("wl_payments.reverse.submit", "Reverse payment"),
    ("wl_payments.reverse.done", "Payment reversed."),
    (
        "wl_payments.reverse.pending",
        "The reversal waits for approval by another user.",
    ),
    ("wl_payments.reverse.reason_required", "Give a reason."),
    ("wl_payments.sections.written_off", "Written-off invoices"),
    ("wl_payments.fields.invoice", "Invoice"),
    ("wl_payments.written_off.left", "Left to recover"),
    ("wl_payments.written_off.proposal", "Proposed amount"),
    (
        "wl_payments.written_off.description",
        "Recovers part of a written-off invoice. What is not allocated stays on the payment as credit.",
    ),
    ("wl_payments.allocate.action", "Allocate"),
    ("wl_payments.allocate.title", "Allocate to an invoice"),
    ("wl_payments.allocate.description", "Left to allocate"),
    ("wl_payments.allocate.submit", "Allocate"),
    ("wl_payments.allocate.done", "Payment allocated."),
    ("wl_payments.tasks.title", "Follow-up tasks"),
    ("wl_payments.tasks.empty", "No open tasks"),
    (
        "wl_payments.tasks.empty_hint",
        "A task shows here when a reversed payment's credit was already refunded.",
    ),
    ("wl_payments.tasks.task", "Task"),
    ("wl_payments.tasks.open_cheque", "Open the cheque"),
    ("wl_payments.tasks.done_action", "Mark done"),
    ("wl_payments.tasks.done_title", "Mark this task done?"),
    (
        "wl_payments.tasks.done_message",
        "Do this once the charge is raised.",
    ),
    ("wl_payments.tasks.done", "Task marked done."),
    ("wl_payments.tasks.open", "Open"),
    ("wl_payments.tasks.finished", "Done"),
    ("wl_payments.tasks.link", "Open tasks"),
    ("wl_payments.tasks.links", "Related"),
    ("wl_payments.tasks.due_on", "Due"),
    ("wl_payments.history.change", "Change"),
    ("wl_payments.history.at", "When"),
    ("wl_payments.history.reason", "Reason"),
    ("wl_payments.unknown_party", "No party has this code."),
    ("wl_payments.unknown_entity", "Choose a legal entity."),
    (
        "wl_payments.bad_amount",
        "Enter an amount above 0, like 1500.00.",
    ),
    (
        "wl_payments.returned.subject",
        "Your payment {{ number }} was returned",
    ),
    (
        "wl_payments.returned.body",
        "Your payment {{ number }} of {{ amount }}, made on {{ payment_date }}, was returned by the bank. It no longer counts against your account.",
    ),
    (
        "wl_payments.refund_failed.subject",
        "Refund {{ refund }} of {{ amount }} failed",
    ),
    (
        "wl_payments.refund_failed.body",
        "Refund {{ refund }} of {{ amount }} to {{ party }} failed: {{ kind }}. Reason: {{ reason }}. See {{ link }}.",
    ),
    (
        "wl_payments.refund_delayed.subject",
        "Your refund is delayed",
    ),
    (
        "wl_payments.refund_delayed.body",
        "Dear customer, your refund of {{ amount }} is delayed; we will contact you.",
    ),
    (
        "wl_payments.refund_method_changed.subject",
        "Your refund will be paid another way",
    ),
    (
        "wl_payments.refund_method_changed.body",
        "Dear customer, your refund of {{ amount }} will now be paid by {{ method }}.",
    ),
    (
        "wl_payments.refund_cancelled.subject",
        "Your refund is cancelled",
    ),
    (
        "wl_payments.refund_cancelled.body",
        "Dear customer, your refund of {{ amount }} is cancelled. The amount stays on your account as credit.",
    ),
    ("wl_payments.refund_method_task.subject", "{{ text }}"),
    (
        "wl_payments.refund_method_task.body",
        "{{ text }}: the refund failed ({{ reason }}). Pay it by transfer, a cheque in the party's name or cash. See {{ link }}.",
    ),
    ("wl_payments.tasks.open_refund", "Open the refund"),
    ("wl_payments.refunds.payee_details", "Pays to"),
    ("wl_payments.refunds.retries", "Retries"),
    ("wl_payments.refunds.cancel_reason", "Cancelled because"),
    ("wl_payments.refunds.retry.action", "Retry"),
    ("wl_payments.refunds.retry.title", "Retry this refund?"),
    (
        "wl_payments.refunds.retry.message",
        "It is paid again by the same method to the same details. A refund is retried once.",
    ),
    ("wl_payments.refunds.retry.done", "Refund retried."),
    ("wl_payments.refunds.change.action", "Change method"),
    (
        "wl_payments.refunds.change.title",
        "Pay this refund another way",
    ),
    (
        "wl_payments.refunds.change.description",
        "The original card through the gateway, cash and a cheque in the party's name are approved at once. New bank details, or a cheque to someone else, need another user's approval.",
    ),
    ("wl_payments.refunds.change.method", "New method"),
    (
        "wl_payments.refunds.change.details",
        "Bank details or payee",
    ),
    (
        "wl_payments.refunds.change.details_hint",
        "The IBAN of a transfer, or the name on a cheque that is not the party's.",
    ),
    ("wl_payments.refunds.change.submit", "Change method"),
    ("wl_payments.refunds.change.done", "The method is changed."),
    (
        "wl_payments.refunds.change.method_required",
        "Choose the new method.",
    ),
    ("wl_payments.refunds.pay.action", "Pay the refund"),
    ("wl_payments.refunds.pay.title", "Pay this refund"),
    (
        "wl_payments.refunds.pay.description",
        "Record how the money was paid back. The payment goes out in the party's name and the refund becomes paid.",
    ),
    ("wl_payments.refunds.pay.method", "Paid by"),
    ("wl_payments.refunds.pay.reference", "Reference"),
    (
        "wl_payments.refunds.pay.reference_hint",
        "The bank reference of the transfer, the cheque number or the cash voucher.",
    ),
    ("wl_payments.refunds.pay.submit", "Pay the refund"),
    ("wl_payments.refunds.pay.done", "The refund is paid."),
    (
        "wl_payments.refunds.pay.method_required",
        "Choose how the refund is paid.",
    ),
    (
        "wl_payments.refunds.pay.reference_required",
        "Give the payment reference.",
    ),
    ("wl_payments.refunds.cancel.action", "Cancel refund"),
    ("wl_payments.refunds.cancel.title", "Cancel this refund"),
    (
        "wl_payments.refunds.cancel.description",
        "Use it when the party cannot be reached or asks to keep the credit. The amount stays on the party's credit balance.",
    ),
    ("wl_payments.refunds.cancel.reason", "Reason"),
    ("wl_payments.refunds.cancel.submit", "Cancel refund"),
    ("wl_payments.refunds.cancel.done", "Refund cancelled."),
    ("wl_payments.refunds.cancel.required", "Give the reason."),
    ("wl_payments.refunds.title", "Refunds"),
    ("wl_payments.refunds.empty", "No refunds yet"),
    (
        "wl_payments.refunds.empty_hint",
        "A refund shows here once someone asks to pay money back.",
    ),
    ("wl_payments.refunds.heading", "Refund"),
    ("wl_payments.refunds.sections.details", "Refund"),
    ("wl_payments.refunds.sections.attempts", "Attempts"),
    ("wl_payments.refunds.no_attempts", "Nothing tried yet"),
    ("wl_payments.refunds.no_method", "Not chosen"),
    ("wl_payments.refunds.source", "Paid back from"),
    ("wl_payments.refunds.gateway_txn", "Gateway transaction"),
    ("wl_payments.refunds.reason", "Reason for the refund"),
    ("wl_payments.refunds.paid_at", "Paid on"),
    ("wl_payments.refunds.attempt", "Attempt"),
    ("wl_payments.refunds.failure_kind", "Failure"),
    ("wl_payments.refunds.failed_on", "Failed on"),
    ("wl_payments.refunds.status.requested", "Requested"),
    ("wl_payments.refunds.status.approved", "Approved"),
    ("wl_payments.refunds.status.paid", "Paid"),
    ("wl_payments.refunds.status.failed", "Failed"),
    ("wl_payments.refunds.status.rejected", "Rejected"),
    ("wl_payments.refunds.status.cancelled", "Cancelled"),
    ("wl_payments.refunds.outcome.pending", "Pending"),
    ("wl_payments.refunds.outcome.paid", "Paid"),
    ("wl_payments.refunds.outcome.failed", "Failed"),
    ("wl_payments.refunds.source_kind.payment", "Payment"),
    ("wl_payments.refunds.source_kind.credit_note", "Credit note"),
    ("wl_payments.refunds.source_kind.deposit", "Deposit"),
    (
        "wl_payments.refunds.source_kind.party_credit",
        "Party credit",
    ),
    (
        "wl_payments.refunds.source_kind.payer_credit",
        "Payer credit",
    ),
    ("wl_payments.refunds.source_kind.order", "Order"),
    ("wl_payments.refunds.source_kind.booking", "Booking"),
    (
        "wl_payments.refunds.kind.gateway_declined",
        "Declined by the gateway",
    ),
    (
        "wl_payments.refunds.kind.card_unavailable",
        "Card unavailable",
    ),
    (
        "wl_payments.refunds.kind.bank_return",
        "Returned by the bank",
    ),
    ("wl_payments.refunds.fail.action", "Mark as failed"),
    (
        "wl_payments.refunds.fail.title",
        "Mark this refund as failed",
    ),
    (
        "wl_payments.refunds.fail.description",
        "Use it when the bank returned the transfer or the card cannot take the refund. A paid refund's outgoing payment is reversed and the money goes back to the party.",
    ),
    ("wl_payments.refunds.fail.kind", "What happened"),
    ("wl_payments.refunds.fail.reason", "Reason"),
    ("wl_payments.refunds.fail.submit", "Mark as failed"),
    ("wl_payments.refunds.fail.done", "Refund marked as failed."),
    (
        "wl_payments.refunds.fail.kind_required",
        "Choose what happened.",
    ),
    ("wl_payments.deposits.title", "Deposits"),
    ("wl_payments.deposits.empty", "No deposits yet"),
    (
        "wl_payments.deposits.empty_hint",
        "A deposit shows here once money is held for an agreement.",
    ),
    ("wl_payments.deposits.heading", "Deposit"),
    ("wl_payments.deposits.sections.details", "Deposit"),
    ("wl_payments.deposits.source", "Held for"),
    ("wl_payments.deposits.held", "Held amount"),
    ("wl_payments.deposits.refund_method", "Refund method"),
    ("wl_payments.deposits.refund", "Refund"),
    ("wl_payments.deposits.no_refund", "None yet"),
    ("wl_payments.deposits.refund_failed", "Refund failed"),
    ("wl_payments.deposits.open_refund", "Open the refund"),
    ("wl_payments.deposits.refund_state", "Refund"),
    (
        "wl_payments.deposits.sections.disputes",
        "Disputes by the party",
    ),
    (
        "wl_payments.deposits.deductions.action",
        "Publish deductions",
    ),
    (
        "wl_payments.deposits.deductions.title",
        "Publish the deductions to the party",
    ),
    (
        "wl_payments.deposits.deductions.description",
        "The next version of the deductions (or the forfeiture) is shown to the party in the portal and its dispute window starts today. A published version is never changed; a revision is the next version.",
    ),
    ("wl_payments.deposits.deductions.lines", "Deductions"),
    (
        "wl_payments.deposits.deductions.lines_hint",
        "One per line: description | amount in dirhams | evidence reference. Leave empty for none, or for the whole deposit when forfeiting.",
    ),
    (
        "wl_payments.deposits.deductions.forfeiture_reason",
        "Forfeiture reason",
    ),
    (
        "wl_payments.deposits.deductions.forfeiture_hint",
        "Fill it in to publish a forfeiture instead of deductions.",
    ),
    ("wl_payments.deposits.deductions.submit", "Publish"),
    (
        "wl_payments.deposits.deductions.done",
        "Deductions published to the party.",
    ),
    (
        "wl_payments.deposits.deductions.version",
        "Published version",
    ),
    (
        "wl_payments.deposits.deductions.kind.deductions",
        "deductions",
    ),
    (
        "wl_payments.deposits.deductions.kind.forfeiture",
        "forfeiture",
    ),
    (
        "wl_payments.deposits.deductions.published_on",
        "Published on",
    ),
    (
        "wl_payments.deposits.deductions.window_ends_on",
        "Dispute window ends",
    ),
    ("wl_payments.deposits.deductions.no_window", "No dispute"),
    ("wl_payments.deposits.deductions.total", "Deducted"),
    ("wl_payments.deposits.deductions.reason", "Reason"),
    ("wl_payments.deposits.deductions.line", "Deduction"),
    ("wl_payments.deposits.deductions.evidence", "Evidence"),
    ("wl_payments.deposits.deductions.no_lines", "No deductions"),
    ("wl_payments.deposits.dispute.flag", "Deductions disputed"),
    (
        "wl_payments.deposits.dispute.flag_message",
        "Refund and forfeit wait until every disputed line is revised or upheld, or the decision of the legal case on the agreement is recorded.",
    ),
    ("wl_payments.deposits.dispute.disputed", "Disputed"),
    ("wl_payments.deposits.dispute.comment", "The party's reason"),
    ("wl_payments.deposits.dispute.raised_on", "Raised on"),
    ("wl_payments.deposits.dispute.response", "Our answer"),
    ("wl_payments.deposits.dispute.status.open", "Open"),
    ("wl_payments.deposits.dispute.status.revised", "Revised"),
    ("wl_payments.deposits.dispute.status.upheld", "Upheld"),
    ("wl_payments.deposits.answer.action", "Answer the dispute"),
    (
        "wl_payments.deposits.answer.title",
        "Answer the party's dispute",
    ),
    (
        "wl_payments.deposits.answer.description",
        "Answer with evidence. Lines you revise are published as a new version and the dispute window starts again; the other disputed lines are upheld. With no revised line the deductions are upheld and the window does not restart.",
    ),
    ("wl_payments.deposits.answer.response", "Answer"),
    ("wl_payments.deposits.answer.evidence", "Evidence"),
    (
        "wl_payments.deposits.answer.evidence_hint",
        "A reference to the inspection photos or the quotes.",
    ),
    ("wl_payments.deposits.answer.changes", "Revised lines"),
    (
        "wl_payments.deposits.answer.changes_hint",
        "One per row: line id | new amount in dirhams (0 removes the line).",
    ),
    ("wl_payments.deposits.answer.submit", "Answer"),
    (
        "wl_payments.deposits.answer.done",
        "The dispute is answered.",
    ),
    ("wl_payments.deposits.legal_case.open", "Legal case opened"),
    (
        "wl_payments.deposits.legal_case.open_title",
        "A legal case is open on the agreement",
    ),
    (
        "wl_payments.deposits.legal_case.open_message",
        "The dispute flag stays until the decision is recorded; refund and forfeit wait.",
    ),
    (
        "wl_payments.deposits.legal_case.close",
        "Record the decision",
    ),
    (
        "wl_payments.deposits.legal_case.close_title",
        "Record the decision of the legal case",
    ),
    (
        "wl_payments.deposits.legal_case.close_message",
        "The dispute flag is released and the settlement can go on.",
    ),
    (
        "wl_payments.deposits.legal_case.done",
        "The legal case is recorded.",
    ),
    ("wl_payments.deposits.refund.action", "Refund"),
    ("wl_payments.deposits.refund.title", "Refund the deposit"),
    (
        "wl_payments.deposits.refund.description",
        "A refund of the held amount is requested and follows the refund flow: approval by another user, then payment. It waits for the deductions dispute.",
    ),
    ("wl_payments.deposits.refund.method", "Method"),
    ("wl_payments.deposits.refund.submit", "Refund"),
    ("wl_payments.deposits.refund.done", "Refund requested."),
    (
        "wl_payments.deposits.refund.exception_requested",
        "The party is on hold. An exception was requested; the refund waits for its approval.",
    ),
    ("wl_payments.deposits.forfeit.action", "Forfeit"),
    ("wl_payments.deposits.forfeit.title", "Forfeit the deposit"),
    (
        "wl_payments.deposits.forfeit.description",
        "The deposit is kept in full. The forfeiture must be published to the party as the deductions first, and its dispute window must have passed. Another user who may refund deposits approves it.",
    ),
    ("wl_payments.deposits.forfeit.reason", "Reason"),
    ("wl_payments.deposits.forfeit.approver", "Approved by"),
    ("wl_payments.deposits.forfeit.submit", "Forfeit"),
    ("wl_payments.deposits.forfeit.done", "Deposit forfeited."),
    (
        "wl_payments.deposits.transfer.action",
        "Transfer to a new holder",
    ),
    (
        "wl_payments.deposits.transfer.title",
        "Hand the deposit to the new holder",
    ),
    (
        "wl_payments.deposits.transfer.description",
        "The source agreement passes to another holder before it ends. The held amount is paid to the new holder against their receipt (an outgoing payment, not a refund), the liability leaves you and the party is told who holds the deposit.",
    ),
    ("wl_payments.deposits.transfer.new_holder", "New holder"),
    (
        "wl_payments.deposits.transfer.new_holder_hint",
        "The code of the party, like P-00007.",
    ),
    (
        "wl_payments.deposits.transfer.receipt",
        "The new holder's receipt",
    ),
    (
        "wl_payments.deposits.transfer.receipt_hint",
        "The reference of the receipt document attached.",
    ),
    ("wl_payments.deposits.transfer.submit", "Transfer"),
    (
        "wl_payments.deposits.transfer.done",
        "Deposit handed to the new holder.",
    ),
    ("wl_payments.deposits.transfer.holder", "Transferred to"),
    (
        "wl_payments.deposits.sections.adjustments",
        "Changes of the amount",
    ),
    ("wl_payments.deposits.adjust.action", "Adjust the amount"),
    (
        "wl_payments.deposits.adjust.title",
        "Adjust the deposit to a new amount",
    ),
    (
        "wl_payments.deposits.adjust.description",
        "The agreement changed the deposit it requires. An increase raises a top-up charge due on the effective date; a decrease asks for a part refund, which another user approves. The deposit stays held and its held amount follows the money actually received or paid back.",
    ),
    ("wl_payments.deposits.adjust.new_amount", "New amount"),
    (
        "wl_payments.deposits.adjust.version",
        "Version of the agreement",
    ),
    (
        "wl_payments.deposits.adjust.version_hint",
        "The amendment or renewal version that changed it.",
    ),
    ("wl_payments.deposits.adjust.effective_on", "Effective on"),
    ("wl_payments.deposits.adjust.submit", "Adjust"),
    ("wl_payments.deposits.adjust.done", "Deposit adjusted."),
    (
        "wl_payments.deposits.adjust.same",
        "The amount is the same: nothing was recorded.",
    ),
    (
        "wl_payments.deposits.adjust.amount_required",
        "Enter the new amount, like 6000.00.",
    ),
    (
        "wl_payments.deposits.adjust.version_required",
        "Enter the version of the agreement.",
    ),
    (
        "wl_payments.deposits.adjust.date_required",
        "Enter the day the change takes effect.",
    ),
    ("wl_payments.deposits.adjust.change", "Change"),
    ("wl_payments.deposits.adjust.difference", "Difference"),
    (
        "wl_payments.deposits.adjust.charge_or_refund",
        "Charge or refund",
    ),
    ("wl_payments.deposits.adjust.kind.top_up", "Top-up"),
    (
        "wl_payments.deposits.adjust.kind.part_refund",
        "Part refund",
    ),
    ("wl_payments.deposits.adjust.status.open", "Open"),
    ("wl_payments.deposits.adjust.status.settled", "Settled"),
    ("wl_payments.deposits.adjust.status.closed", "Closed"),
    ("wl_payments.deposits.status.held", "Held"),
    (
        "wl_payments.deposits.status.settlement_pending",
        "Settlement pending",
    ),
    ("wl_payments.deposits.status.refunded", "Refunded"),
    ("wl_payments.deposits.status.forfeited", "Forfeited"),
    ("wl_payments.deposits.status.transferred", "Transferred"),
    ("wl_payments.cheques.title", "Cheques"),
    ("wl_payments.cheques.heading", "Cheque"),
    ("wl_payments.cheques.empty", "No cheques yet"),
    (
        "wl_payments.cheques.empty_hint",
        "Cheques received with an agreement, and issued to suppliers, are listed here.",
    ),
    ("wl_payments.cheques.sections.details", "Cheque"),
    ("wl_payments.cheques.sections.links", "Documents it pays"),
    ("wl_payments.cheques.sections.returns", "Bank returns"),
    ("wl_payments.cheques.sections.tasks", "Tasks"),
    ("wl_payments.cheques.number", "Cheque number"),
    ("wl_payments.cheques.bank", "Bank"),
    ("wl_payments.cheques.date", "Cheque date"),
    ("wl_payments.cheques.deposit_date", "Deposit date"),
    ("wl_payments.cheques.presentations", "Presentations"),
    ("wl_payments.cheques.honoured", "Paid by the bank"),
    ("wl_payments.cheques.settled", "Settled since"),
    ("wl_payments.cheques.open", "Open amount"),
    ("wl_payments.cheques.fees", "Bounce fees (before VAT)"),
    ("wl_payments.cheques.agreement", "Agreement"),
    ("wl_payments.cheques.linked", "Document"),
    ("wl_payments.cheques.field", "Collected in the field"),
    (
        "wl_payments.cheques.field_yes",
        "Yes: needs a cash handover before deposit",
    ),
    ("wl_payments.cheques.none", "None"),
    ("wl_payments.cheques.no_links", "No document linked"),
    ("wl_payments.cheques.no_returns", "Never returned"),
    ("wl_payments.cheques.return.presentation", "Presentation"),
    ("wl_payments.cheques.return.on", "Returned on"),
    ("wl_payments.cheques.direction.in", "Received"),
    ("wl_payments.cheques.direction.out", "Issued"),
    ("wl_payments.cheques.status.received", "Received"),
    (
        "wl_payments.cheques.status.due_for_deposit",
        "Due for deposit",
    ),
    ("wl_payments.cheques.status.deposited", "Deposited"),
    ("wl_payments.cheques.status.cleared", "Cleared"),
    ("wl_payments.cheques.status.bounced", "Bounced"),
    ("wl_payments.cheques.status.legal_case", "Legal case"),
    ("wl_payments.cheques.status.replaced", "Replaced"),
    ("wl_payments.cheques.status.written_off", "Written off"),
    ("wl_payments.cheques.status.returned", "Returned"),
    (
        "wl_payments.cheques.reason.insufficient_funds",
        "Insufficient funds",
    ),
    (
        "wl_payments.cheques.reason.account_closed",
        "Account closed",
    ),
    (
        "wl_payments.cheques.reason.payment_stopped",
        "Payment stopped by the drawer",
    ),
    (
        "wl_payments.cheques.reason.signature_irregular",
        "Signature or alteration irregular",
    ),
    ("wl_payments.cheques.reason.other", "Other"),
    ("wl_payments.cheques.deposit.action", "Deposit"),
    ("wl_payments.cheques.deposit.title", "Deposit this cheque?"),
    (
        "wl_payments.cheques.deposit.message",
        "The cheque goes to the bank. It counts as presented once more.",
    ),
    ("wl_payments.cheques.deposit.done", "Cheque deposited."),
    ("wl_payments.cheques.clear.action", "Clear"),
    (
        "wl_payments.cheques.clear.title",
        "The bank paid this cheque?",
    ),
    (
        "wl_payments.cheques.clear.message",
        "A payment is recorded for the open amount and allocated to the documents the cheque pays.",
    ),
    (
        "wl_payments.cheques.clear.done",
        "Cheque cleared and its payment recorded.",
    ),
    ("wl_payments.cheques.bounce.action", "Record bounce"),
    (
        "wl_payments.cheques.bounce.title",
        "The bank returned this cheque",
    ),
    (
        "wl_payments.cheques.bounce.description",
        "Give the bank's return reason, attach its return certificate and enter what the bank paid from the available balance, if anything. A bounce fee is added when one is set, with standard VAT until the accountant decides (A-05), and the customer account is put on credit hold.",
    ),
    (
        "wl_payments.cheques.bounce.description_issued",
        "Give the bank's return reason and what the bank paid, if anything. The cheque is replaced by a new cheque or a payment: you get a task for it.",
    ),
    ("wl_payments.cheques.bounce.reason", "Return reason"),
    (
        "wl_payments.cheques.bounce.certificate",
        "Return certificate",
    ),
    (
        "wl_payments.cheques.bounce.certificate_hint",
        "The reference of the bank's return certificate, as filed with the documents.",
    ),
    ("wl_payments.cheques.bounce.honoured", "Paid by the bank"),
    (
        "wl_payments.cheques.bounce.honoured_hint",
        "In dirhams, like 6000.00. Leave empty when the bank paid nothing.",
    ),
    ("wl_payments.cheques.bounce.submit", "Record bounce"),
    ("wl_payments.cheques.bounce.done", "Bounce recorded."),
    (
        "wl_payments.cheques.bounce.reason_required",
        "Choose the return reason.",
    ),
    (
        "wl_payments.cheques.bounce.honoured_invalid",
        "Give the amount in dirhams, like 6000.00.",
    ),
    ("wl_payments.cheques.represent.action", "Present again"),
    (
        "wl_payments.cheques.represent.title",
        "Present this cheque again",
    ),
    (
        "wl_payments.cheques.represent.description",
        "Use it when the party confirms there are funds. A cheque is presented at most twice, and not again once the bank has paid part of it (provisional until the lawyer answers, L-21). The party is told the new date.",
    ),
    (
        "wl_payments.cheques.represent.date",
        "New presentation date",
    ),
    (
        "wl_payments.cheques.represent.funds",
        "The party confirmed there are funds",
    ),
    ("wl_payments.cheques.represent.note", "Note"),
    (
        "wl_payments.cheques.represent.note_hint",
        "Needed when the confirmation is not ticked, e.g. \"funds confirmed by phone\".",
    ),
    ("wl_payments.cheques.represent.submit", "Present again"),
    (
        "wl_payments.cheques.represent.done",
        "The cheque is presented again.",
    ),
    (
        "wl_payments.cheques.represent.date_required",
        "Give the new presentation date.",
    ),
    ("wl_payments.cheques.sections.replacements", "Replacements"),
    ("wl_payments.cheques.custody_note", "Kept because"),
    ("wl_payments.cheques.settle.action", "Settle part"),
    (
        "wl_payments.cheques.settle.title",
        "The party paid part of this cheque",
    ),
    (
        "wl_payments.cheques.settle.description",
        "A payment in cash, by transfer or by card, below the open amount. It is allocated to the documents the cheque pays and the open amount goes down. To settle the whole of it, link the payment as a replacement.",
    ),
    ("wl_payments.cheques.settle.method", "Paid by"),
    (
        "wl_payments.cheques.settle.reference_hint",
        "Required for a transfer or a card payment.",
    ),
    ("wl_payments.cheques.settle.submit", "Record payment"),
    ("wl_payments.cheques.settle.done", "Part payment recorded."),
    (
        "wl_payments.cheques.settle.method_required",
        "Choose cash, transfer or card.",
    ),
    (
        "wl_payments.cheques.settle.amount_required",
        "Give the amount in dirhams, like 4000.00.",
    ),
    (
        "wl_payments.cheques.replacement.action",
        "Link a replacement",
    ),
    (
        "wl_payments.cheques.replacement.title",
        "Link a replacement",
    ),
    (
        "wl_payments.cheques.replacement.description",
        "A new cheque, or a payment received from the party, that takes the place of the bounced cheque. The cheque can be marked replaced once the replacements cover its open amount.",
    ),
    ("wl_payments.cheques.replacement.kind", "Replacement"),
    ("wl_payments.cheques.replacement.cheque", "Cheque"),
    ("wl_payments.cheques.replacement.payment", "Payment"),
    (
        "wl_payments.cheques.replacement.id",
        "Number in the register",
    ),
    (
        "wl_payments.cheques.replacement.id_hint",
        "The id shown in the address of the cheque or payment, like /cheques/12.",
    ),
    ("wl_payments.cheques.replacement.what", "Linked"),
    ("wl_payments.cheques.replacement.covers", "Covers"),
    ("wl_payments.cheques.replacement.submit", "Link"),
    (
        "wl_payments.cheques.replacement.done",
        "Replacement linked.",
    ),
    (
        "wl_payments.cheques.replacement.kind_required",
        "Choose a cheque or a payment.",
    ),
    (
        "wl_payments.cheques.replacement.id_required",
        "Give the number of the replacement.",
    ),
    ("wl_payments.cheques.replace.action", "Mark replaced"),
    (
        "wl_payments.cheques.replace.title",
        "Mark this cheque replaced?",
    ),
    (
        "wl_payments.cheques.replace.message",
        "The linked cheques and payments must cover the open amount. The original is handed back to the party once its replacement cheques clear.",
    ),
    (
        "wl_payments.cheques.replace.done",
        "Cheque marked replaced.",
    ),
    ("wl_payments.cheques.case", "Legal case"),
    ("wl_payments.cheques.case_owner", "Case owner"),
    ("wl_payments.cheques.case_reference", "Case reference"),
    ("wl_payments.cheques.referred_on", "Referred on"),
    (
        "wl_payments.cheques.case.civil_execution",
        "Civil execution",
    ),
    (
        "wl_payments.cheques.case.criminal_complaint",
        "Criminal complaint",
    ),
    ("wl_payments.cheques.refer.action", "Refer to legal"),
    (
        "wl_payments.cheques.refer.title",
        "Refer this cheque to a legal case",
    ),
    (
        "wl_payments.cheques.refer.description",
        "The open amount is handed to a legal case. The case owner gets a task with the evidence pack, the party gets a formal notice and the manager is told. This records the case; it files nothing with a court or the police. A criminal complaint is allowed only for the return reasons on the list in the settings; that list and the text of the notice are provisional until the lawyer answers (L-21).",
    ),
    ("wl_payments.cheques.refer.case_type", "Kind of case"),
    ("wl_payments.cheques.refer.case_owner", "Case owner"),
    ("wl_payments.cheques.refer.submit", "Refer"),
    (
        "wl_payments.cheques.refer.done",
        "Cheque referred to a legal case.",
    ),
    (
        "wl_payments.cheques.refer.case_required",
        "Choose civil execution or a criminal complaint.",
    ),
    (
        "wl_payments.cheques.sections.write_offs",
        "Invoice write-offs",
    ),
    ("wl_payments.cheques.write_off.action", "Write off"),
    (
        "wl_payments.cheques.write_off.title",
        "Write off this cheque",
    ),
    (
        "wl_payments.cheques.write_off.description",
        "The open balance of every invoice this cheque pays is written off as a bad debt through the invoice write-off, which has its own approval limit. The cheque is marked written off when all those write-offs are made. Offered only when the open balances of the invoices equal the cheque's open amount; otherwise write the invoices off directly. The customer's credit hold stays until it is released in sales.",
    ),
    ("wl_payments.cheques.write_off.reason", "Reason"),
    ("wl_payments.cheques.write_off.submit", "Write off"),
    ("wl_payments.cheques.write_off.done", "Cheque written off."),
    (
        "wl_payments.cheques.write_off.requested",
        "Write-off requested: the cheque is written off when the invoice write-offs are approved.",
    ),
    (
        "wl_payments.cheques.write_off.reason_required",
        "A reason is required.",
    ),
    ("wl_payments.cheques.write_off.invoice", "Invoice"),
    ("wl_payments.cheques.write_off.failure", "Why it failed"),
    ("wl_payments.cheques.write_off.status.pending", "Waiting"),
    (
        "wl_payments.cheques.write_off.status.written_off",
        "Written off",
    ),
    ("wl_payments.cheques.write_off.status.failed", "Failed"),
    ("wl_payments.chargeback.action", "Record chargeback"),
    ("wl_payments.chargeback.title", "Record a chargeback"),
    (
        "wl_payments.chargeback.description",
        "The card holder disputed this payment and the gateway took the funds back. Enter what the acquirer's notice says. The invoices it paid reopen.",
    ),
    ("wl_payments.chargeback.submit", "Record chargeback"),
    ("wl_payments.chargeback.reference", "Dispute reference"),
    ("wl_payments.chargeback.deadline", "Evidence due on"),
    ("wl_payments.chargeback.fee", "Dispute fee"),
    ("wl_payments.chargeback.done", "Chargeback recorded."),
    ("wl_payments.evidence.action", "Evidence submitted"),
    ("wl_payments.evidence.title", "Evidence submitted"),
    (
        "wl_payments.evidence.description",
        "Note that the evidence was submitted in the gateway. Only the gateway's decision closes the dispute.",
    ),
    ("wl_payments.evidence.submit", "Evidence was submitted"),
    ("wl_payments.evidence.done", "Noted."),
    ("wl_payments.sections.dispute", "Chargeback"),
    ("wl_payments.dispute.status", "Dispute"),
    (
        "wl_payments.dispute.evidence_submitted",
        "Evidence submitted on",
    ),
    ("wl_payments.dispute.open", "Open"),
    ("wl_payments.dispute.won", "Won"),
    ("wl_payments.dispute.lost", "Lost"),
    ("wl_payments.dispute.accepted", "Accepted"),
    ("wl_payments.accept_chargeback.action", "Accept chargeback"),
    (
        "wl_payments.accept_chargeback.title",
        "Accept the chargeback",
    ),
    (
        "wl_payments.accept_chargeback.description",
        "Do not contest the dispute. Another user with the chargeback permission must approve it; the gateway is told then. The invoices the payment paid stay open.",
    ),
    ("wl_payments.accept_chargeback.submit", "Ask to accept"),
    (
        "wl_payments.accept_chargeback.reason",
        "Why it is not contested",
    ),
    (
        "wl_payments.accept_chargeback.reason_required",
        "Give a reason.",
    ),
    (
        "wl_payments.accept_chargeback.pending",
        "Sent for approval.",
    ),
    ("wl_payments.accept_chargeback.done", "Chargeback accepted."),
    ("wl_payments.lose_chargeback.action", "Dispute lost"),
    ("wl_payments.lose_chargeback.title", "The dispute was lost"),
    (
        "wl_payments.lose_chargeback.description",
        "Record that the acquirer ruled for the card holder. The payment is reversed and the invoices it paid stay open.",
    ),
    ("wl_payments.lose_chargeback.submit", "Record as lost"),
    (
        "wl_payments.lose_chargeback.done",
        "Dispute recorded as lost.",
    ),
    ("wl_payments.reason.chargeback_won", "Chargeback (won)"),
    ("wl_payments.online.failure", "Why it failed"),
    ("wl_payments.pay.title", "Pay online"),
    ("wl_payments.pay.pay", "Pay now"),
    ("wl_payments.pay.retry", "Try again"),
    ("wl_payments.pay.failed", "Your payment did not go through"),
    ("wl_payments.pay.problem", "We could not start the payment"),
    (
        "wl_payments.pay.waiting_title",
        "Your payment is being processed",
    ),
    (
        "wl_payments.pay.waiting",
        "We are waiting for the answer of the payment provider. Open this page again in a few minutes.",
    ),
    ("wl_payments.pay.paid_title", "Paid"),
    (
        "wl_payments.pay.paid",
        "Thank you, this payment has been received.",
    ),
    (
        "wl_payments.pay.expired_title",
        "This payment link has expired",
    ),
    ("wl_payments.pay.expired", "Ask us for a new payment link."),
    (
        "wl_payments.pay.cancelled_title",
        "This payment link was withdrawn",
    ),
    (
        "wl_payments.pay.cancelled",
        "Ask us for a new payment link.",
    ),
    ("wl_payments.pay.invalid_title", "This link does not open"),
    (
        "wl_payments.pay.invalid",
        "Check the link you were sent, or ask us for a new one.",
    ),
    ("wl_payments.online_task.subject", "{{ text }}"),
    (
        "wl_payments.online_task.body",
        "{{ text }}. See {{ link }}.",
    ),
    ("wl_payments.followup.subject", "{{ text }}"),
    (
        "wl_payments.followup.body",
        "{{ text }}: payment {{ number }} was reversed after the party's credit was refunded. See {{ link }}.",
    ),
    (
        "wl_payments.cheque_bounced.subject",
        "Your cheque {{ number }} was returned",
    ),
    (
        "wl_payments.cheque_bounced.body",
        "Dear customer, your cheque {{ number }} for {{ amount }} drawn on {{ bank }} was returned unpaid by the bank (reason: {{ reason }}). {{ open }} is still due. Please contact us to arrange payment.",
    ),
    ("wl_payments.cheque_task.subject", "{{ text }}"),
    (
        "wl_payments.cheque_task.body",
        "{{ text }}. Cheque {{ number }} of {{ party }}, {{ amount }} open. See {{ link }}.",
    ),
    (
        "wl_payments.cheque_presented.subject",
        "Your cheque {{ number }} will be presented again",
    ),
    (
        "wl_payments.cheque_presented.body",
        "Dear customer, as agreed, your cheque {{ number }} for {{ amount }} will be presented to the bank again on {{ date }}. Please make sure the funds are available.",
    ),
    (
        "wl_payments.cheque_notice.subject",
        "Formal notice: returned cheque {{ number }}",
    ),
    (
        "wl_payments.cheque_notice.body",
        "To {{ drawer }}. Cheque {{ number }} of {{ bank }} dated {{ cheque_date }} for {{ amount }} was returned by the bank on {{ returned_on }} (reason: {{ reason }}). The bank paid {{ honoured }}; {{ open }} is still due{{ fee }}. You are asked to pay the amount still due within {{ days }} days of this notice, failing which we will pursue it through a legal case ({{ case_type }}). Contact: {{ contact }}. (Template content pending legal review.)",
    ),
    (
        "wl_payments.cheque_reminder.subject",
        "Reminder: your cheque {{ number }} was returned",
    ),
    (
        "wl_payments.cheque_reminder.body",
        "Dear customer, your cheque {{ number }} of {{ bank }} for {{ amount }} was returned unpaid on {{ returned_on }} and has not been settled. {{ open }} is still due{{ fee }}. Please contact us to arrange payment.",
    ),
    (
        "wl_payments.cheque_reminder_staff.subject",
        "Bounced cheque {{ number }} of {{ party }}: {{ days }} days",
    ),
    (
        "wl_payments.cheque_reminder_staff.body",
        "Cheque {{ number }} of {{ party }} was returned {{ days }} days ago on {{ returned_on }}. {{ open }} is still due{{ fee }}. See {{ link }}.",
    ),
    (
        "wl_payments.cheque_bounced_staff.subject",
        "Cheque {{ number }} of {{ party }} was returned",
    ),
    (
        "wl_payments.cheque_bounced_staff.body",
        "Cheque {{ number }} of {{ party }} was returned by the bank on {{ returned_on }}: {{ reason }}. {{ open }} is open. See {{ link }}.",
    ),
    (
        "wl_payments.deposit_deductions.subject",
        "Deposit {{ deposit }}: the {{ kind }} are published",
    ),
    (
        "wl_payments.deposit_deductions.body",
        "Dear customer, version {{ version }} of the {{ kind }} from your deposit of {{ held }} is published: {{ total }} in all.{{ until }}",
    ),
    (
        "wl_payments.deposit_disputed.subject",
        "{{ party }} disputed the deductions of deposit {{ deposit }}",
    ),
    (
        "wl_payments.deposit_disputed.body",
        "{{ party }} disputed {{ lines }} line(s) of version {{ version }} of the deductions of deposit {{ deposit }}. Answer with evidence within {{ days }} days: revise the deductions or uphold them. Refund and forfeit wait. See {{ link }}.",
    ),
    (
        "wl_payments.deposit_dispute_answered.subject",
        "Your dispute of the deductions of deposit {{ deposit }} was answered",
    ),
    (
        "wl_payments.deposit_dispute_answered.body",
        "Dear customer, {{ outcome }} Our answer: {{ response }}",
    ),
    (
        "wl_payments.deposit_transferred.subject",
        "Your deposit {{ deposit }} is now held by {{ holder }}",
    ),
    (
        "wl_payments.deposit_transferred.body",
        "Dear customer, your agreement has passed to a new holder and your deposit of {{ amount }} (deposit {{ deposit }}) was handed over to {{ holder }}, against their receipt. {{ holder }} now holds it and will settle it with you when your agreement ends.",
    ),
    ("wl_payments.deposit_task.subject", "{{ text }}"),
    (
        "wl_payments.deposit_task.body",
        "{{ text }}. See {{ link }}.",
    ),
    (
        "wl_payments.cheque_referred.subject",
        "Cheque {{ number }} referred to a legal case",
    ),
    (
        "wl_payments.cheque_referred.body",
        "{{ text }}. Case owner: {{ owner }}. See {{ link }}.",
    ),
    ("wl_payments.cash.title", "Cash handovers"),
    (
        "wl_payments.cash.description",
        "The cash staff hold and hand over at the end of a run, shift or day. Another user counts it; a short count is recounted, recovered or written off.",
    ),
    ("wl_payments.cash.heading", "Handover"),
    ("wl_payments.cash.empty", "No handovers yet"),
    (
        "wl_payments.cash.empty_hint",
        "A handover opens when a member of staff collects cash.",
    ),
    (
        "wl_payments.cash.amount_hint",
        "In dirhams, for example 2250.00.",
    ),
    (
        "wl_payments.cash.amount_required",
        "Enter an amount in dirhams, for example 2250.00.",
    ),
    ("wl_payments.cash.note", "Note"),
    ("wl_payments.cash.holder", "Holder"),
    ("wl_payments.cash.responsible", "Responsible person"),
    ("wl_payments.cash.period", "Period"),
    ("wl_payments.cash.kind", "Kind"),
    ("wl_payments.cash.source", "Source"),
    ("wl_payments.cash.expected", "Expected"),
    ("wl_payments.cash.counted", "Counted"),
    ("wl_payments.cash.difference", "Difference"),
    ("wl_payments.cash.overage", "Overage"),
    ("wl_payments.cash.recovered", "Recovered"),
    ("wl_payments.cash.rest", "Left to recover"),
    ("wl_payments.cash.written_off", "Written off"),
    ("wl_payments.cash.reason", "Reason"),
    ("wl_payments.cash.location", "Outlet or branch"),
    ("wl_payments.cash.actions", "Actions"),
    ("wl_payments.cash.counter", "Counted by"),
    (
        "wl_payments.cash.holder_present",
        "Holder present or notified",
    ),
    (
        "wl_payments.cash.absent",
        "The holder was not there (count what was received)",
    ),
    ("wl_payments.cash.absent_short", "Holder absent"),
    ("wl_payments.cash.yes", "Yes"),
    ("wl_payments.cash.no", "No"),
    ("wl_payments.cash.no_recoveries", "No recoveries yet"),
    ("wl_payments.cash.no_counts", "Not counted yet"),
    ("wl_payments.cash.no_entries", "No cash collected yet"),
    ("wl_payments.cash.no_postings", "Nothing posted"),
    ("wl_payments.cash.sections.details", "Handover"),
    ("wl_payments.cash.sections.recoveries", "Recoveries"),
    ("wl_payments.cash.sections.counts", "Counts"),
    ("wl_payments.cash.sections.entries", "Cash collected"),
    ("wl_payments.cash.sections.postings", "Postings"),
    ("wl_payments.cash.status.open", "Open"),
    ("wl_payments.cash.status.counted", "Counted"),
    ("wl_payments.cash.status.short", "Short"),
    ("wl_payments.cash.status.recovered", "Recovered"),
    ("wl_payments.cash.status.written_off", "Written off"),
    ("wl_payments.cash.kind_label.period", "Period"),
    (
        "wl_payments.cash.kind_label.later_shortage",
        "Later shortage",
    ),
    ("wl_payments.cash.period.run", "Run"),
    ("wl_payments.cash.period.shift", "Shift"),
    ("wl_payments.cash.period.day", "Day"),
    ("wl_payments.cash.period.till_session", "Till session"),
    ("wl_payments.cash.count.kind", "Count"),
    ("wl_payments.cash.count.kind_count", "First count"),
    ("wl_payments.cash.count.kind_recount", "Recount"),
    ("wl_payments.cash.count.action", "Count"),
    ("wl_payments.cash.count.title", "Count the cash"),
    (
        "wl_payments.cash.count.description",
        "Count what the holder hands over. You cannot count your own cash.",
    ),
    ("wl_payments.cash.count.submit", "Save the count"),
    ("wl_payments.cash.count.done", "Cash counted."),
    ("wl_payments.cash.recount.action", "Recount"),
    ("wl_payments.cash.recount.title", "Recount the cash"),
    (
        "wl_payments.cash.recount.description",
        "A third person recounts a short handover: not the holder and not the first counter.",
    ),
    ("wl_payments.cash.recount.submit", "Save the recount"),
    ("wl_payments.cash.recount.done", "Cash recounted."),
    ("wl_payments.cash.recovery.action", "Record a recovery"),
    (
        "wl_payments.cash.recovery.title",
        "Record a recovery in cash",
    ),
    (
        "wl_payments.cash.recovery.description",
        "Cash taken back from the person responsible. The handover stays short until the recoveries cover the whole difference.",
    ),
    ("wl_payments.cash.recovery.submit", "Record"),
    ("wl_payments.cash.recovery.done", "Recovery recorded."),
    ("wl_payments.cash.recovery.kind", "Kind"),
    ("wl_payments.cash.recovery.kind_cash", "Cash"),
    (
        "wl_payments.cash.recovery.kind_deduction",
        "Salary deduction",
    ),
    ("wl_payments.cash.recovery.status.recorded", "Recorded"),
    (
        "wl_payments.cash.recovery.status.requested",
        "Waiting for approval",
    ),
    (
        "wl_payments.cash.recovery.status.authorised",
        "Waiting for the holder",
    ),
    ("wl_payments.cash.recovery.status.rejected", "Not approved"),
    ("wl_payments.cash.deduction.action", "Deduct from salary"),
    (
        "wl_payments.cash.deduction.title",
        "Recover by a salary deduction",
    ),
    (
        "wl_payments.cash.deduction.description",
        "Another user must approve it and the person must acknowledge it. HR applies its own limits.",
    ),
    ("wl_payments.cash.deduction.submit", "Ask for approval"),
    (
        "wl_payments.cash.deduction.done",
        "Deduction sent for approval.",
    ),
    ("wl_payments.cash.recover.action", "Recover"),
    ("wl_payments.cash.recover.title", "Recover the shortage"),
    (
        "wl_payments.cash.recover.description",
        "Possible when the recoveries recorded cover the whole difference. Cash given here is recorded first.",
    ),
    ("wl_payments.cash.recover.submit", "Recover"),
    ("wl_payments.cash.recover.amount", "Cash received now"),
    (
        "wl_payments.cash.recover.amount_hint",
        "Leave empty when the recoveries already cover the difference.",
    ),
    ("wl_payments.cash.recover.done", "Shortage recovered."),
    ("wl_payments.cash.write_off.action", "Write off"),
    (
        "wl_payments.cash.write_off.title",
        "Write off the rest of the shortage",
    ),
    (
        "wl_payments.cash.write_off.description",
        "Writes off what is left after the recoveries as a loss of the holder's outlet or branch. Above the limit another approver must agree.",
    ),
    ("wl_payments.cash.write_off.submit", "Write off"),
    ("wl_payments.cash.write_off.reason", "Reason"),
    ("wl_payments.cash.write_off.done", "Shortage written off."),
    (
        "wl_payments.cash.write_off.requested",
        "Write-off sent for approval.",
    ),
    ("wl_payments.cash.acknowledge.action", "Acknowledge"),
    (
        "wl_payments.cash.acknowledge.title",
        "Acknowledge the deduction",
    ),
    (
        "wl_payments.cash.acknowledge.message",
        "You agree that this amount is deducted from your salary. HR will be told.",
    ),
    (
        "wl_payments.cash.acknowledge.done",
        "Deduction acknowledged and passed to HR.",
    ),
    ("wl_payments.cash.posting.kind", "Posting"),
    (
        "wl_payments.cash.posting.overage_liability",
        "Overage held as a liability",
    ),
    ("wl_payments.cash.posting.shortage_loss", "Shortage loss"),
    ("wl_payments.cash.raise.action", "Raise a later shortage"),
    (
        "wl_payments.cash.raise.title",
        "Raise a shortage against a holder",
    ),
    (
        "wl_payments.cash.raise.description",
        "For a shortage found after the holder's cash was counted. It is recovered or written off like any other.",
    ),
    ("wl_payments.cash.raise.submit", "Raise"),
    ("wl_payments.cash.raise.holder", "Holder (user id)"),
    (
        "wl_payments.cash.raise.holder_hint",
        "The member of staff the shortage is raised against.",
    ),
    (
        "wl_payments.cash.raise.holder_required",
        "Enter the holder's user id.",
    ),
    ("wl_payments.cash.raise.source_type", "Source type"),
    (
        "wl_payments.cash.raise.source_type_hint",
        "For example logistics.shipment.",
    ),
    ("wl_payments.cash.raise.source_id", "Source id"),
    (
        "wl_payments.cash.raise.source_required",
        "Name the record that caused it, for example logistics.shipment and its id.",
    ),
    ("wl_payments.cash.raise.reason", "Reason"),
    ("wl_payments.cash.raise.done", "Shortage raised."),
    (
        "wl_payments.cash.deduction_off",
        "Salary deduction is not offered: the client has not turned it on. Recover in cash or write the rest off.",
    ),
    (
        "wl_payments.cash.deduction_reason",
        "Cash shortage of {period}",
    ),
    (
        "wl_payments.cash.deduction_rejected",
        "The salary deduction of {amount} was not approved. Recover the shortage another way.",
    ),
    (
        "wl_payments.cash.no_longer_short",
        "The handover is no longer short.",
    ),
    (
        "wl_payments.cash.changed",
        "What is left to recover changed since the request: ask again.",
    ),
    ("wl_payments.cash.reach_done", "The handover was counted."),
    (
        "wl_payments.cash.escalation.reach_title",
        "Reach {holder}: cash not handed over",
    ),
    (
        "wl_payments.cash.escalation.reach",
        "{holder} has not handed over the cash of {period} ({amount}). Reach the holder.",
    ),
    (
        "wl_payments.cash.escalation.admin",
        "{holder} still has not handed over the cash of {period} ({amount}). The manager will count what was received.",
    ),
    (
        "wl_payments.cash.escalation.count",
        "Count the handover of {holder} for {period} at what was actually received, and record that the holder was absent.",
    ),
    (
        "wl_payments.cash_shortage.subject",
        "Cash shortage of {{ amount }}",
    ),
    (
        "wl_payments.cash_shortage.body",
        "The cash of {{ period }} is short by {{ amount }}. See {{ link }}.",
    ),
    (
        "wl_payments.cash_handed_over.subject",
        "Cash handed over: {{ amount }}",
    ),
    (
        "wl_payments.cash_handed_over.body",
        "{{ from }} handed {{ amount }} over to {{ to }} with the stops of a run. {{ from }} still has {{ rest }} to hand over. See {{ link }}.",
    ),
    ("wl_payments.cash_task.subject", "{{ text }}"),
    ("wl_payments.cash_task.body", "{{ text }} See {{ link }}."),
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

/// Every text of the package in `lang`, keyed without the `wl_payments.`
/// prefix (`fields.number`), for the templates: `tx['fields.number']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_payments."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TEXTS;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_payments.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }
}
