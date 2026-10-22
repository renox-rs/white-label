//! The texts of the rate card pages (09 Manpower `MPW.ACL.16`, BL-MPW-26), in
//! English. They are listed apart from `texts.rs` and read through the same
//! functions; an app changes or translates one by putting the key in its
//! `resources/lang/{locale}.json`.

/// Every rate card page key, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("wl_invoicing.rate_card.screen.card_title", "Rate card"),
    (
        "wl_invoicing.rate_card.screen.card_title_of",
        "Rate card of {{ client }}",
    ),
    (
        "wl_invoicing.rate_card.screen.version_title",
        "Version {{ version }}",
    ),
    ("wl_invoicing.rate_card.screen.back_contract", "Contract"),
    ("wl_invoicing.rate_card.screen.back_card", "Rate card"),
    ("wl_invoicing.rate_card.screen.versions", "Versions"),
    ("wl_invoicing.rate_card.screen.version", "Version"),
    ("wl_invoicing.rate_card.screen.effective", "Effective from"),
    ("wl_invoicing.rate_card.screen.status", "Status"),
    ("wl_invoicing.rate_card.screen.accepted", "Accepted"),
    ("wl_invoicing.rate_card.screen.lines", "Lines"),
    ("wl_invoicing.rate_card.screen.open", "Open"),
    ("wl_invoicing.rate_card.screen.not_yet", "Not yet"),
    ("wl_invoicing.rate_card.screen.empty", "No versions yet"),
    (
        "wl_invoicing.rate_card.screen.empty_text",
        "Make the first version with its effective date, then add the lines.",
    ),
    ("wl_invoicing.rate_card.screen.new_version", "New version"),
    (
        "wl_invoicing.rate_card.screen.new_version_hint",
        "A change of price is a new version with its own effective date; the client accepts it before it prices anything.",
    ),
    (
        "wl_invoicing.rate_card.screen.effective_date",
        "Effective date",
    ),
    (
        "wl_invoicing.rate_card.screen.copy",
        "Start from the lines in force",
    ),
    (
        "wl_invoicing.rate_card.screen.created",
        "The version is made. Add its lines, then ask the client to accept it.",
    ),
    ("wl_invoicing.rate_card.screen.status_pending", "Pending"),
    (
        "wl_invoicing.rate_card.screen.status_waiting_md",
        "Waiting for the MD",
    ),
    ("wl_invoicing.rate_card.screen.status_accepted", "Accepted"),
    ("wl_invoicing.rate_card.screen.status_in_force", "In force"),
    (
        "wl_invoicing.rate_card.screen.status_superseded",
        "Superseded",
    ),
    (
        "wl_invoicing.rate_card.screen.status_rejected",
        "Not approved",
    ),
    (
        "wl_invoicing.rate_card.screen.note_pending",
        "This version prices nothing until the client accepts it.",
    ),
    (
        "wl_invoicing.rate_card.screen.note_waiting_md",
        "The client accepted a version dated in the past. It prices nothing until the MD approves it.",
    ),
    (
        "wl_invoicing.rate_card.screen.note_accepted",
        "Accepted. It prices the hours worked from its effective date.",
    ),
    (
        "wl_invoicing.rate_card.screen.note_in_force",
        "In force: it prices the hours worked today.",
    ),
    (
        "wl_invoicing.rate_card.screen.note_superseded",
        "A later version prices the hours now. This one is kept as it was.",
    ),
    (
        "wl_invoicing.rate_card.screen.note_rejected",
        "Not approved. It never priced anything.",
    ),
    (
        "wl_invoicing.rate_card.screen.fact_accepted_on",
        "Accepted on",
    ),
    (
        "wl_invoicing.rate_card.screen.fact_accepted_by",
        "Accepted by",
    ),
    (
        "wl_invoicing.rate_card.screen.fact_document",
        "Signed acceptance",
    ),
    (
        "wl_invoicing.rate_card.screen.fact_approved_by",
        "Approved by",
    ),
    (
        "wl_invoicing.rate_card.screen.fact_reason",
        "Why it was not approved",
    ),
    ("wl_invoicing.rate_card.screen.fact_made_on", "Made on"),
    (
        "wl_invoicing.rate_card.screen.fact_past_dated",
        "Dated in the past",
    ),
    (
        "wl_invoicing.rate_card.screen.yes",
        "Yes, it needs the MD to approve it",
    ),
    ("wl_invoicing.rate_card.screen.no", "No"),
    (
        "wl_invoicing.rate_card.screen.client_portal",
        "The client in the portal",
    ),
    ("wl_invoicing.rate_card.screen.lines_title", "Lines"),
    ("wl_invoicing.rate_card.screen.role", "Role"),
    ("wl_invoicing.rate_card.screen.shift", "Shift"),
    ("wl_invoicing.rate_card.screen.rate", "Rate"),
    ("wl_invoicing.rate_card.screen.overtime", "Overtime"),
    ("wl_invoicing.rate_card.screen.holiday", "Holiday"),
    ("wl_invoicing.rate_card.screen.minimum", "Minimum per shift"),
    ("wl_invoicing.rate_card.screen.lines_empty", "No lines yet"),
    (
        "wl_invoicing.rate_card.screen.lines_empty_text",
        "Add a line for each role and shift the client is billed for.",
    ),
    (
        "wl_invoicing.rate_card.screen.read_only",
        "The lines of an accepted version are never edited. A change is a new version.",
    ),
    ("wl_invoicing.rate_card.screen.unit_hour", "per hour"),
    ("wl_invoicing.rate_card.screen.unit_day", "per day"),
    ("wl_invoicing.rate_card.screen.unit_month", "per month"),
    ("wl_invoicing.rate_card.screen.add_line", "Add line"),
    (
        "wl_invoicing.rate_card.screen.add_line_hint",
        "One line for each role and shift. Rates are in AED with two decimals.",
    ),
    ("wl_invoicing.rate_card.screen.edit_line", "Edit"),
    ("wl_invoicing.rate_card.screen.field_role", "Job role"),
    (
        "wl_invoicing.rate_card.screen.field_role_hint",
        "For example security guard",
    ),
    ("wl_invoicing.rate_card.screen.field_shift", "Shift type"),
    ("wl_invoicing.rate_card.screen.field_unit", "Priced"),
    ("wl_invoicing.rate_card.screen.field_rate", "Rate (AED)"),
    (
        "wl_invoicing.rate_card.screen.field_overtime",
        "Overtime multiplier",
    ),
    (
        "wl_invoicing.rate_card.screen.field_holiday",
        "Holiday multiplier",
    ),
    (
        "wl_invoicing.rate_card.screen.field_minimum",
        "Minimum hours per shift",
    ),
    (
        "wl_invoicing.rate_card.screen.line_saved",
        "The line is saved.",
    ),
    (
        "wl_invoicing.rate_card.screen.accept",
        "Accept for the client",
    ),
    (
        "wl_invoicing.rate_card.screen.accept_hint",
        "Record the client's signed acceptance. A version dated in the past then waits for the MD.",
    ),
    (
        "wl_invoicing.rate_card.screen.accept_blocked",
        "Add at least one line before the client accepts this version.",
    ),
    (
        "wl_invoicing.rate_card.screen.field_document",
        "Signed acceptance",
    ),
    (
        "wl_invoicing.rate_card.screen.field_document_hint",
        "The name of the signed document the client returned",
    ),
    ("wl_invoicing.rate_card.screen.no_minimum", "No minimum"),
    (
        "wl_invoicing.rate_card.screen.accepted_done",
        "The acceptance is recorded.",
    ),
    (
        "wl_invoicing.rate_card.screen.waiting_banner",
        "Waiting for the MD to approve this version.",
    ),
    ("wl_invoicing.rate_card.screen.approve", "Approve"),
    ("wl_invoicing.rate_card.screen.reject", "Do not approve"),
    ("wl_invoicing.rate_card.screen.comment", "Note"),
    ("wl_invoicing.rate_card.screen.comment_optional", "Optional"),
    ("wl_invoicing.rate_card.screen.comment_required", "Say why"),
    (
        "wl_invoicing.rate_card.screen.preview_title",
        "Preview a price",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_hint",
        "Prices the sample hours by the version in force on the work date.",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_line",
        "Role and shift",
    ),
    ("wl_invoicing.rate_card.screen.preview_date", "Work date"),
    (
        "wl_invoicing.rate_card.screen.preview_regular",
        "Regular hours",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_overtime",
        "Overtime hours",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_holiday",
        "Holiday hours",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_submit",
        "Preview price",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_none",
        "No rate card in force on that date.",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_no_line",
        "The version in force on that date has no line for that role and shift.",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_bad",
        "Check the date and the hours.",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_result",
        "Price on {{ date }} by version {{ version }}",
    ),
    ("wl_invoicing.rate_card.screen.preview_what", "Hours"),
    ("wl_invoicing.rate_card.screen.preview_hours", "Billed"),
    ("wl_invoicing.rate_card.screen.preview_amount", "Amount"),
    ("wl_invoicing.rate_card.screen.preview_total", "Total"),
    (
        "wl_invoicing.rate_card.screen.preview_split",
        "Month rate split by version, {{ from }} to {{ to }} ({{ days }} of {{ month_days }} days)",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_category_normal",
        "Regular",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_category_overtime",
        "Overtime",
    ),
    (
        "wl_invoicing.rate_card.screen.preview_category_holiday",
        "Holiday",
    ),
    ("wl_invoicing.rate_card.screen.start", "Start the rate card"),
    (
        "wl_invoicing.rate_card.screen.start_done",
        "The rate card is started.",
    ),
];
