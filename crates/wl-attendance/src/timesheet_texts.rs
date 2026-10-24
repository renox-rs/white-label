//! The texts of the timesheet pages (09 Manpower `MPW.ACL.09`, BL-MPW-27), in
//! English. They are listed apart from `texts.rs` and read through the same
//! functions; an app changes or translates one by putting the key in its
//! `resources/lang/{locale}.json`. A template reads one as `tx['status.draft']`
//! (the key without `wl_attendance.sheet.`).

/// The prefix a template drops.
pub(crate) const PREFIX: &str = "wl_attendance.sheet.";

/// Every timesheet page key, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("wl_attendance.sheet.title", "Timesheets"),
    (
        "wl_attendance.sheet.subtitle",
        "The hours of each worker and period, and where each timesheet stands.",
    ),
    ("wl_attendance.sheet.worker", "Worker"),
    ("wl_attendance.sheet.client", "Client"),
    ("wl_attendance.sheet.work", "Work"),
    ("wl_attendance.sheet.period", "Period"),
    ("wl_attendance.sheet.month", "Month"),
    ("wl_attendance.sheet.hours", "Hours"),
    ("wl_attendance.sheet.minutes", "Minutes"),
    ("wl_attendance.sheet.reason", "Reason"),
    ("wl_attendance.sheet.period_from", "From"),
    ("wl_attendance.sheet.period_to", "To"),
    ("wl_attendance.sheet.status", "Status"),
    ("wl_attendance.sheet.open", "Open"),
    ("wl_attendance.sheet.adjustment_tag", "Adjustment"),
    ("wl_attendance.sheet.all_clients", "All clients"),
    ("wl_attendance.sheet.all", "All"),
    ("wl_attendance.sheet.show", "Show"),
    (
        "wl_attendance.sheet.status_filter",
        "Show timesheets by status",
    ),
    ("wl_attendance.sheet.empty", "No timesheets"),
    (
        "wl_attendance.sheet.empty_text",
        "A timesheet appears here when the hours of a worker are collected for a period.",
    ),
    ("wl_attendance.sheet.status.draft", "Draft"),
    ("wl_attendance.sheet.status.submitted", "Submitted"),
    ("wl_attendance.sheet.status.escalated", "Escalated"),
    ("wl_attendance.sheet.status.approved", "Approved"),
    (
        "wl_attendance.sheet.status.deemed_approved",
        "Deemed approved",
    ),
    ("wl_attendance.sheet.status.locked", "Locked"),
    ("wl_attendance.sheet.status.billed", "Billed"),
    ("wl_attendance.sheet.status.rejected", "Rejected"),
    ("wl_attendance.sheet.back", "Timesheets"),
    (
        "wl_attendance.sheet.detail_title",
        "Timesheet of {{ worker }}",
    ),
    ("wl_attendance.sheet.tab.hours", "Hours"),
    ("wl_attendance.sheet.tab.history", "History"),
    ("wl_attendance.sheet.tab.adjustments", "Adjustments"),
    ("wl_attendance.sheet.fact.approver", "Approved by"),
    ("wl_attendance.sheet.fact.approver_internal", "Staff"),
    ("wl_attendance.sheet.fact.approver_external", "The client"),
    ("wl_attendance.sheet.fact.kind", "Kind"),
    ("wl_attendance.sheet.fact.kind_regular", "Regular timesheet"),
    (
        "wl_attendance.sheet.fact.kind_adjustment",
        "Adjustment of a locked timesheet",
    ),
    ("wl_attendance.sheet.fact.submitted", "Submitted"),
    ("wl_attendance.sheet.fact.approved", "Approved"),
    ("wl_attendance.sheet.fact.on", "{{ date }} by {{ who }}"),
    (
        "wl_attendance.sheet.fact.deemed_by",
        "{{ date }}, no answer from the client",
    ),
    ("wl_attendance.sheet.fact.system", "the system"),
    ("wl_attendance.sheet.fact.origin", "Why it was opened"),
    ("wl_attendance.sheet.fact.affects", "Flows to"),
    ("wl_attendance.sheet.fact.not_yet", "Not yet"),
    (
        "wl_attendance.sheet.note.escalated",
        "Escalated on {{ date }} ({{ days }} days without answer). The account manager collects the signed sheet.",
    ),
    (
        "wl_attendance.sheet.note.deemed",
        "Deemed approved on {{ date }}: the client gave no answer in {{ days }} days, as the client contract says.",
    ),
    (
        "wl_attendance.sheet.note.deemed_plain",
        "Deemed approved on {{ date }}: the client gave no answer within the days of the client contract.",
    ),
    (
        "wl_attendance.sheet.note.rejected",
        "Sent back on {{ date }}: {{ reason }}",
    ),
    (
        "wl_attendance.sheet.note.on_behalf",
        "Approved on behalf of the client with the signed sheet.",
    ),
    (
        "wl_attendance.sheet.note.billed",
        "Billed on invoice {{ invoice }}. A billed timesheet is read-only; a correction is an adjustment.",
    ),
    (
        "wl_attendance.sheet.note.locked",
        "Locked: payroll or an invoice used these hours. A correction is an adjustment.",
    ),
    (
        "wl_attendance.sheet.note.adjustment",
        "Adjustment of a locked timesheet. Why: {{ reason }}",
    ),
    ("wl_attendance.sheet.open_invoice", "Open the invoice"),
    (
        "wl_attendance.sheet.open_original",
        "Open the timesheet it corrects",
    ),
    ("wl_attendance.sheet.grid.day", "Day"),
    ("wl_attendance.sheet.grid.total", "Total"),
    ("wl_attendance.sheet.grid.normal", "Regular"),
    ("wl_attendance.sheet.grid.overtime", "Overtime"),
    ("wl_attendance.sheet.grid.night", "Night"),
    ("wl_attendance.sheet.grid.rest_day", "Rest day"),
    ("wl_attendance.sheet.grid.public_holiday", "Public holiday"),
    ("wl_attendance.sheet.grid.empty", "No hours yet"),
    (
        "wl_attendance.sheet.grid.empty_text",
        "The hours appear here when attendance is accepted or entered for the period.",
    ),
    ("wl_attendance.sheet.history.when", "When"),
    ("wl_attendance.sheet.history.what", "What"),
    ("wl_attendance.sheet.history.by", "By"),
    ("wl_attendance.sheet.history.reason", "Reason"),
    (
        "wl_attendance.sheet.history.empty",
        "Nothing has happened to this timesheet yet",
    ),
    ("wl_attendance.sheet.history.system", "System"),
    ("wl_attendance.sheet.history.agency", "The agency"),
    (
        "wl_attendance.sheet.history.signed",
        "Signed sheet: {{ name }}, {{ date }}",
    ),
    ("wl_attendance.sheet.tr.submit", "Submitted"),
    (
        "wl_attendance.sheet.tr.submit_by_system",
        "Submitted at the end of the period",
    ),
    ("wl_attendance.sheet.tr.approve", "Approved"),
    (
        "wl_attendance.sheet.tr.approve_on_behalf",
        "Approved on behalf of the client",
    ),
    ("wl_attendance.sheet.tr.deem_approve", "Deemed approved"),
    (
        "wl_attendance.sheet.tr.approve_by_system",
        "Approved with the timesheet it follows",
    ),
    ("wl_attendance.sheet.tr.reject", "Sent back"),
    ("wl_attendance.sheet.tr.reopen", "Reopened"),
    ("wl_attendance.sheet.tr.lock", "Locked"),
    (
        "wl_attendance.sheet.tr.open_adjustment",
        "Opened as an adjustment",
    ),
    ("wl_attendance.sheet.tr.create", "Made"),
    ("wl_attendance.sheet.signed.title", "Signed sheet"),
    ("wl_attendance.sheet.signed.open", "Open the signed sheet"),
    (
        "wl_attendance.sheet.documents",
        "Documents of this timesheet",
    ),
    (
        "wl_attendance.sheet.adjustments.of",
        "Adjustments of this timesheet",
    ),
    ("wl_attendance.sheet.adjustments.empty", "No adjustments"),
    (
        "wl_attendance.sheet.adjustments.empty_text",
        "A locked timesheet is corrected by an adjustment, never edited.",
    ),
    ("wl_attendance.sheet.action.submit", "Submit"),
    (
        "wl_attendance.sheet.action.submit_hint",
        "Sends the hours to the approver. The checks on attendance and on the hours allocated run first.",
    ),
    ("wl_attendance.sheet.action.approve", "Approve"),
    (
        "wl_attendance.sheet.action.approve_hint",
        "Approves the hours as they are.",
    ),
    ("wl_attendance.sheet.action.reject", "Reject"),
    (
        "wl_attendance.sheet.action.reject_hint",
        "Sends the hours back to be corrected, with the reason.",
    ),
    ("wl_attendance.sheet.action.on_behalf", "Approve on behalf"),
    (
        "wl_attendance.sheet.action.on_behalf_hint",
        "Approves for the client from the sheet the client signed. Upload the sheet under the documents of this timesheet first, then enter its number.",
    ),
    ("wl_attendance.sheet.action.reopen", "Reopen"),
    (
        "wl_attendance.sheet.action.reopen_hint",
        "Takes an approved timesheet back to draft while nothing has used it.",
    ),
    ("wl_attendance.sheet.action.adjust", "Open adjustment"),
    (
        "wl_attendance.sheet.action.adjust_hint",
        "A locked timesheet is never edited. The adjustment holds only the difference, in hours, and is approved by someone other than you.",
    ),
    (
        "wl_attendance.sheet.action.creator_blocked",
        "You opened this adjustment: another person must approve it.",
    ),
    ("wl_attendance.sheet.form.reason", "Reason"),
    (
        "wl_attendance.sheet.form.reason_required",
        "Write the reason",
    ),
    (
        "wl_attendance.sheet.form.document",
        "Number of the signed sheet",
    ),
    (
        "wl_attendance.sheet.form.document_hint",
        "Shown on the documents of this timesheet once the signed sheet is uploaded",
    ),
    ("wl_attendance.sheet.form.signed_by", "Signed by"),
    ("wl_attendance.sheet.form.signed_on", "Signed on"),
    ("wl_attendance.sheet.form.origin", "Why"),
    ("wl_attendance.sheet.form.affects", "Flows to"),
    ("wl_attendance.sheet.form.entry_date", "Day to correct"),
    ("wl_attendance.sheet.form.entry_category", "Category"),
    ("wl_attendance.sheet.form.entry_hours", "Hours"),
    (
        "wl_attendance.sheet.form.entry_hours_hint",
        "Negative takes hours off, for example -1.5",
    ),
    (
        "wl_attendance.sheet.form.linked",
        "Linked ticket or dispute",
    ),
    (
        "wl_attendance.sheet.form.linked_hint",
        "Needed for a client dispute, as type:id",
    ),
    (
        "wl_attendance.sheet.origin.client_dispute",
        "Client dispute",
    ),
    (
        "wl_attendance.sheet.origin.attendance_error",
        "Attendance error",
    ),
    ("wl_attendance.sheet.origin.payroll_error", "Payroll error"),
    (
        "wl_attendance.sheet.origin.provisional_reconciliation",
        "Provisional hours reconciled",
    ),
    ("wl_attendance.sheet.affects.billing", "Billing"),
    ("wl_attendance.sheet.affects.payroll", "Payroll"),
    ("wl_attendance.sheet.affects.both", "Billing and payroll"),
    (
        "wl_attendance.sheet.bad_hours",
        "Write the hours as a number, for example 1.5 or -2.",
    ),
    ("wl_attendance.sheet.bad_day", "Choose the day to correct."),
    (
        "wl_attendance.approval.reason_required",
        "Write the reason.",
    ),
];
