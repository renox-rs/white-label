//! The texts of the daily closing and the business day (BL-FNB-15): keys
//! `fnb.closing.*` (toasts, refusals, the notice) and `fnb.screen.closing.*`
//! (what a template reads as `tx['closing...']`).

/// Every text of the daily closing.
pub(crate) const CLOSING_TEXTS: &[(&str, &str)] = &[
    // History.
    (
        "fnb.closing.history.opened",
        "Closing opened for the business day",
    ),
    ("fnb.closing.history.submitted", "Closing submitted"),
    ("fnb.closing.till_reopened", "The closing was reopened"),
    (
        "fnb.closing.reopen_late_reason",
        "Late entries after an outage",
    ),
    // Toasts.
    ("fnb.closing.opened", "Closing opened."),
    (
        "fnb.closing.submitted",
        "Closing submitted. Its cash was counted in the till.",
    ),
    ("fnb.closing.approved", "Closing approved."),
    ("fnb.closing.reopened", "Closing reopened for correction."),
    (
        "fnb.closing.cutoff_set",
        "The cut-off is saved. It applies from the next business day.",
    ),
    // When the closing opens, in words.
    ("fnb.closing.opens", "after {{ time }} on {{ date }}"),
    // Refusals.
    (
        "fnb.closing.problem.cutoff",
        "The closing of this day can be submitted only {{ opens }}: the business day runs to the cut-off.",
    ),
    (
        "fnb.closing.problem.checklist",
        "Complete the closing checklist before you submit the closing.",
    ),
    (
        "fnb.closing.problem.not_counted",
        "Enter the cash counted in the till (zero or more).",
    ),
    (
        "fnb.closing.problem.not_submittable",
        "Only a closing that is open or reopened is submitted.",
    ),
    (
        "fnb.closing.problem.cashier",
        "Choose the cashier the till belongs to: a shortage is recovered from them.",
    ),
    (
        "fnb.closing.problem.no_sales",
        "The sales module is off: the till cannot be counted.",
    ),
    (
        "fnb.closing.problem.till_reviewed",
        "The till session of this closing was reviewed and cannot be counted again.",
    ),
    (
        "fnb.closing.problem.cutoff_time",
        "Give the cut-off as a time, like 04:00.",
    ),
    // The notice to the kitchen manager when a late entry reopened the closing.
    (
        "fnb.closing.reopened_title",
        "The closing of {{ outlet }} for {{ date }} is reopened",
    ),
    (
        "fnb.closing.reopened_body",
        "The area manager approved a late entry ({{ reference }}) into the closing of {{ outlet }} for {{ date }}, so it is reopened. Count the till again and submit the closing.",
    ),
    // The screens.
    ("fnb.screen.closing.title", "Daily closings"),
    (
        "fnb.screen.closing.subtitle",
        "One closing per outlet and business day. The day runs to the outlet's cut-off, so late-night trading is not split.",
    ),
    ("fnb.screen.closing.outlet", "Outlet"),
    ("fnb.screen.closing.date", "Business day"),
    ("fnb.screen.closing.orders", "Orders"),
    ("fnb.screen.closing.sales", "Sales with VAT"),
    ("fnb.screen.closing.counted", "Cash counted"),
    ("fnb.screen.closing.expected", "Cash the till should hold"),
    ("fnb.screen.closing.variance", "Difference"),
    ("fnb.screen.closing.empty", "No closings yet"),
    (
        "fnb.screen.closing.empty_text",
        "A closing is made with the first order of a business day, or open today's here.",
    ),
    ("fnb.screen.closing.open", "Open today's closing"),
    (
        "fnb.screen.closing.open_title",
        "Open the closing of today's business day",
    ),
    (
        "fnb.screen.closing.open_description",
        "The closing of an outlet exists once its first order of the day arrives; open it here for a day with no orders yet.",
    ),
    ("fnb.screen.closing.cutoff", "Business-day cut-off"),
    ("fnb.screen.closing.cutoff_set", "Change the cut-off"),
    (
        "fnb.screen.closing.cutoff_title",
        "Change the business-day cut-off",
    ),
    (
        "fnb.screen.closing.cutoff_description",
        "Orders before the cut-off belong to the previous day. A change applies from the next business day only.",
    ),
    ("fnb.screen.closing.cutoff_time", "Cut-off (Dubai time)"),
    ("fnb.screen.closing.cutoff_pending", "Changes to"),
    ("fnb.screen.closing.cutoff_from", "from the business day"),
    ("fnb.screen.closing.opens_at", "Can be submitted"),
    ("fnb.screen.closing.status.open", "Open"),
    ("fnb.screen.closing.status.submitted", "Submitted"),
    ("fnb.screen.closing.status.reopened", "Reopened"),
    ("fnb.screen.closing.status.approved", "Approved"),
    ("fnb.screen.closing.submit", "Submit the closing"),
    (
        "fnb.screen.closing.submit_title",
        "Count the till and submit the closing",
    ),
    (
        "fnb.screen.closing.submit_description",
        "Enter the cash in the drawer and confirm the closing checklist. A shortage goes to the till's cash handover, to recover from the cashier or write off.",
    ),
    (
        "fnb.screen.closing.checklist",
        "The closing checklist is completed",
    ),
    ("fnb.screen.closing.cash", "Cash counted (AED)"),
    ("fnb.screen.closing.cashier", "Cashier"),
    (
        "fnb.screen.closing.cashier_hint",
        "A shortage is recovered from the cashier.",
    ),
    ("fnb.screen.closing.approve", "Approve"),
    ("fnb.screen.closing.approve_title", "Approve the closing"),
    (
        "fnb.screen.closing.approve_message",
        "Approve this closing? It cannot be reopened afterwards.",
    ),
    ("fnb.screen.closing.reopen", "Reopen"),
    ("fnb.screen.closing.reopen_title", "Reopen the closing"),
    (
        "fnb.screen.closing.reopen_description",
        "Send the closing back for correction. The reason is kept and the kitchen manager counts and submits again.",
    ),
    ("fnb.screen.closing.reason", "Reason"),
    ("fnb.screen.closing.channels", "Sales by channel"),
    ("fnb.screen.closing.channel", "Channel"),
    ("fnb.screen.closing.no_sales", "No sales on this day yet"),
    ("fnb.screen.closing.till", "Till session"),
    ("fnb.screen.closing.checklist_done", "Checklist"),
    ("fnb.screen.closing.reopened_times", "Reopened"),
    ("fnb.screen.closing.last_reason", "Last reopening"),
    ("fnb.screen.closing.paper_orders", "Paper orders of the day"),
    ("fnb.screen.closing.late_count", "Late entries"),
    (
        "fnb.screen.closing.notes",
        "Why some actions are not available",
    ),
    (
        "fnb.screen.closing.note_cutoff",
        "The closing can be submitted only after the cut-off of the next morning.",
    ),
    (
        "fnb.screen.closing.note_approved",
        "This closing is approved. A late entry into it is refused; the area manager records it in the next day's closing.",
    ),
    (
        "fnb.screen.closing.note_submitted",
        "This closing is submitted and locked. Only the area manager reopens it; a late entry reopens it once the area manager approves the entry.",
    ),
    ("fnb.screen.closing.none", "None"),
    ("fnb.screen.closing.yes", "Yes"),
    ("fnb.screen.closing.no", "No"),
    ("fnb.screen.closing.mode.cash", "Cash"),
];
