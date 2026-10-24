//! The texts of the till sessions (wl-sales §4.3, `SAL.POS.01`), in English,
//! listed apart from the rest of the package's texts. [`crate::texts`] reads
//! this list as well, so an app translates one the same way: by putting the key
//! in its `resources/lang/{locale}.json`.

/// Every key of the till sessions, with its English text.
pub(crate) const TILL_TEXTS: &[(&str, &str)] = &[
    // Refusals.
    (
        "wl_sales.till.problem.not_counted",
        "Enter the cash counted in the till (zero or more).",
    ),
    (
        "wl_sales.till.problem.float",
        "The opening float cannot be negative.",
    ),
    (
        "wl_sales.till.problem.till",
        "Give the till a name, like T1.",
    ),
    (
        "wl_sales.till.problem.exists",
        "This till already has a session for that day. Reopen it to correct it.",
    ),
    (
        "wl_sales.till.problem.not_open",
        "Only a session that is open is counted and closed.",
    ),
    (
        "wl_sales.till.problem.amount",
        "That is not an amount: write it like 250.00.",
    ),
    // History.
    ("wl_sales.till.history.opened", "Till session opened"),
    // Toasts.
    ("wl_sales.till.opened", "Till session opened."),
    ("wl_sales.till.counted_toast", "Till counted and closed."),
    ("wl_sales.till.reopened", "Till session reopened."),
    ("wl_sales.till.reviewed", "Till session marked reviewed."),
    // The screens.
    ("wl_sales.till.title", "Till sessions"),
    (
        "wl_sales.till.subtitle",
        "One cash drawer for a shift or a day: open it with a float, count it at the end and close it.",
    ),
    ("wl_sales.till.open", "Open a till"),
    (
        "wl_sales.till.open_description",
        "The cash of the counter sales rung up while the session is open counts toward what the till should hold.",
    ),
    ("wl_sales.till.till", "Till"),
    ("wl_sales.till.location", "Outlet"),
    ("wl_sales.till.float", "Opening float (AED)"),
    ("wl_sales.till.date", "Day"),
    ("wl_sales.till.cashier", "Cashier"),
    ("wl_sales.till.expected", "Should hold"),
    ("wl_sales.till.counted", "Counted"),
    ("wl_sales.till.variance", "Difference"),
    ("wl_sales.till.cash_sales", "Cash sales"),
    ("wl_sales.till.card_sales", "Card sales"),
    ("wl_sales.till.empty", "No till sessions yet"),
    ("wl_sales.till.details", "Session"),
    ("wl_sales.till.status.open", "Open"),
    ("wl_sales.till.status.counting", "Counting"),
    ("wl_sales.till.status.closed", "Closed"),
    ("wl_sales.till.status.reviewed", "Reviewed"),
    ("wl_sales.till.count", "Count and close"),
    (
        "wl_sales.till.count_title",
        "Count the cash and close the till",
    ),
    (
        "wl_sales.till.count_description",
        "Enter the cash in the drawer. A shortage goes to the till's cash handover, to recover from the cashier or write off.",
    ),
    ("wl_sales.till.counted_amount", "Cash counted (AED)"),
    ("wl_sales.till.reopen", "Reopen"),
    ("wl_sales.till.reopen_title", "Reopen the session"),
    (
        "wl_sales.till.reopen_description",
        "Put the closed session back to open to correct it. The reason is kept.",
    ),
    ("wl_sales.till.reason", "Reason"),
    ("wl_sales.till.review", "Mark reviewed"),
    (
        "wl_sales.till.review_message",
        "Mark this closed session as checked?",
    ),
    ("wl_sales.till.reopened_times", "Reopened"),
    ("wl_sales.till.last_reason", "Last reopening"),
    ("wl_sales.till.none", "None"),
];
