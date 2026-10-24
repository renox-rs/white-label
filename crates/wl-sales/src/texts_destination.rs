//! The texts of the delivery of an order and of the promised day of a line
//! (BL-MFG-11 points 6 and 7), in English. [`crate::texts`] reads this list as
//! well.

/// Every key of the delivery, with its English text.
pub(crate) const DESTINATION_TEXTS: &[(&str, &str)] = &[
    ("wl_sales.delivery.title", "Delivery"),
    ("wl_sales.delivery.address", "Delivery address"),
    ("wl_sales.delivery.emirate", "Emirate"),
    (
        "wl_sales.delivery.country",
        "Country (two letters, AE for the UAE)",
    ),
    ("wl_sales.delivery.destination", "Delivered"),
    ("wl_sales.delivery.domestic", "In the UAE (domestic)"),
    ("wl_sales.delivery.export", "Abroad (export)"),
    (
        "wl_sales.delivery.not_stated",
        "Not stated (counts as domestic)",
    ),
    ("wl_sales.delivery.vat", "VAT category of the delivery"),
    ("wl_sales.delivery.vat_domestic", "Standard rated"),
    (
        "wl_sales.delivery.vat_export",
        "Zero rated export, with evidence of exit",
    ),
    ("wl_sales.delivery.change", "Change delivery"),
    (
        "wl_sales.delivery.change_description",
        "Where the goods go. An item that may only be exported is refused on a delivery in the UAE.",
    ),
    ("wl_sales.delivery.saved", "Delivery saved."),
    (
        "wl_sales.delivery.problem.not_draft",
        "The delivery is changed on the draft. Once the order is placed, amend it.",
    ),
    (
        "wl_sales.delivery.problem.choice",
        "Choose domestic or export for the delivery.",
    ),
    (
        "wl_sales.delivery.problem.domestic_abroad",
        "A delivery to {{ country }} is outside the UAE: it is an export, not a domestic delivery.",
    ),
    (
        "wl_sales.delivery.problem.export_home",
        "A delivery in the UAE (country AE) is domestic, not an export.",
    ),
    ("wl_sales.orders.promised", "Promised for"),
    (
        "wl_sales.orders.promised_hint",
        "The day the customer was promised the goods, as 2026-12-31.",
    ),
    (
        "wl_sales.orders.errors.date",
        "Give the day as year-month-day, for example 2026-12-31.",
    ),
    ("wl_sales.amend.promised", "Promised for"),
    ("wl_sales.amend.add_promised", "Promised for"),
];
