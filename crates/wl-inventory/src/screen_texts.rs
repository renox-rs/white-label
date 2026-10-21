//! The texts of the screens (the HTML pages of the suppliers of an item, a
//! reorder suggestion, a supplier's score and the preferred-supplier
//! requests), in English. An app changes or translates one by putting the key
//! in its `resources/lang/{locale}.json`.

/// Every key of the screens, with its English text.
pub(crate) const SCREEN_TEXTS: &[(&str, &str)] = &[
    // Shared words.
    ("wl_inventory.screen.status", "Status"),
    ("wl_inventory.screen.supplier", "Supplier"),
    ("wl_inventory.screen.item", "Item"),
    ("wl_inventory.screen.role", "Role"),
    ("wl_inventory.screen.lead_time", "Lead time (days)"),
    ("wl_inventory.screen.last_price", "Last price"),
    ("wl_inventory.screen.score", "Score"),
    ("wl_inventory.screen.none", "None."),
    ("wl_inventory.screen.no_value", "\u{2014}"),
    ("wl_inventory.screen.days", "{{ days }} day(s)"),
    ("wl_inventory.screen.cancel", "Cancel"),
    // Words for values.
    ("wl_inventory.screen.role.preferred", "Preferred"),
    ("wl_inventory.screen.role.alternative", "Alternative"),
    ("wl_inventory.screen.band.good", "Good"),
    ("wl_inventory.screen.band.watch", "Watch"),
    (
        "wl_inventory.screen.band.below_threshold",
        "Below threshold",
    ),
    (
        "wl_inventory.screen.band.not_enough_data",
        "Not enough data",
    ),
    ("wl_inventory.screen.state.open", "Open"),
    (
        "wl_inventory.screen.state.converted",
        "Converted to an order",
    ),
    ("wl_inventory.screen.state.dismissed", "Dismissed"),
    (
        "wl_inventory.screen.state.pending",
        "Waiting for a decision",
    ),
    ("wl_inventory.screen.state.approved", "Approved"),
    ("wl_inventory.screen.state.rejected", "Rejected"),
    ("wl_inventory.screen.state.withdrawn", "Withdrawn"),
    ("wl_inventory.screen.state.cancelled", "Cancelled"),
    (
        "wl_inventory.screen.request_kind.replace",
        "Replace the preferred supplier",
    ),
    (
        "wl_inventory.screen.request_kind.deactivate",
        "Take the preferred supplier off the item",
    ),
    // The suppliers of an item.
    (
        "wl_inventory.screen.suppliers.title",
        "Suppliers of {{ item }}",
    ),
    (
        "wl_inventory.screen.suppliers.subtitle",
        "The preferred supplier first, then the alternatives by rank.",
    ),
    ("wl_inventory.screen.suppliers.rank", "Rank"),
    ("wl_inventory.screen.suppliers.code", "Their code"),
    ("wl_inventory.screen.suppliers.min_order", "Minimum order"),
    ("wl_inventory.screen.suppliers.flags", "Flags"),
    ("wl_inventory.screen.suppliers.on_hold", "On hold"),
    ("wl_inventory.screen.suppliers.watched", "Watched"),
    ("wl_inventory.screen.suppliers.inactive", "Inactive"),
    (
        "wl_inventory.screen.suppliers.missing",
        "Missing: {{ kind }}",
    ),
    ("wl_inventory.screen.suppliers.score", "Supplier score"),
    ("wl_inventory.screen.suppliers.empty", "No suppliers yet"),
    (
        "wl_inventory.screen.suppliers.empty_text",
        "Add the suppliers this item may be bought from.",
    ),
    (
        "wl_inventory.screen.suppliers.request",
        "A change of the preferred supplier waits for a decision",
    ),
    (
        "wl_inventory.screen.suppliers.price_hidden",
        "Last prices are shown to holders of the stock cost permission.",
    ),
    // A reorder suggestion.
    (
        "wl_inventory.screen.reorder.title",
        "Reorder suggestion {{ id }}",
    ),
    ("wl_inventory.screen.reorder.location", "Location"),
    ("wl_inventory.screen.reorder.quantity", "Suggested quantity"),
    ("wl_inventory.screen.reorder.rounded", "Rounded from"),
    ("wl_inventory.screen.reorder.reason", "Why now"),
    ("wl_inventory.screen.reorder.supplier", "Goes to"),
    ("wl_inventory.screen.reorder.suppliers", "Item suppliers"),
    (
        "wl_inventory.screen.reorder.warnings",
        "Certificates to check",
    ),
    (
        "wl_inventory.screen.reorder.alternatives",
        "Other suppliers",
    ),
    (
        "wl_inventory.screen.reorder.no_alternatives",
        "No other supplier is listed for this item.",
    ),
    ("wl_inventory.screen.reorder.back", "Item suppliers"),
    // A supplier's score.
    ("wl_inventory.screen.score.title", "Score of {{ supplier }}"),
    (
        "wl_inventory.screen.score.subtitle",
        "The live window and the monthly snapshots, newest first.",
    ),
    ("wl_inventory.screen.score.live", "Live window"),
    ("wl_inventory.screen.score.snapshots", "Monthly snapshots"),
    ("wl_inventory.screen.score.window", "Window"),
    ("wl_inventory.screen.score.receipts", "Receipts"),
    ("wl_inventory.screen.score.on_time", "On time"),
    ("wl_inventory.screen.score.rejected", "Rejected"),
    ("wl_inventory.screen.score.incidents", "Quality incidents"),
    ("wl_inventory.screen.score.debit_notes", "Debit notes"),
    ("wl_inventory.screen.score.price_variance", "Price variance"),
    ("wl_inventory.screen.score.received_value", "Value received"),
    ("wl_inventory.screen.score.rejected_value", "Value rejected"),
    (
        "wl_inventory.screen.score.debit_value",
        "Value of debit notes",
    ),
    ("wl_inventory.screen.score.notes", "Notes"),
    (
        "wl_inventory.screen.score.no_snapshots",
        "No monthly snapshot yet.",
    ),
    ("wl_inventory.screen.score.watched", "On the watch list"),
    (
        "wl_inventory.screen.score.review_open",
        "A review of this supplier is open.",
    ),
    // The preferred-supplier requests.
    (
        "wl_inventory.screen.requests.title",
        "Preferred supplier requests",
    ),
    (
        "wl_inventory.screen.requests.subtitle",
        "Changes of an item's preferred supplier that wait for a second person.",
    ),
    ("wl_inventory.screen.requests.kind", "Change"),
    ("wl_inventory.screen.requests.from", "From"),
    ("wl_inventory.screen.requests.to", "To"),
    ("wl_inventory.screen.requests.number", "Request"),
    ("wl_inventory.screen.requests.requested_by", "Requested by"),
    (
        "wl_inventory.screen.requests.empty",
        "Nothing waits for a decision",
    ),
    (
        "wl_inventory.screen.requests.empty_text",
        "A request appears here when someone asks to change an item's preferred supplier.",
    ),
    ("wl_inventory.screen.requests.nobody", "Nobody"),
    // One request.
    (
        "wl_inventory.screen.request.title",
        "Preferred supplier request {{ id }}",
    ),
    ("wl_inventory.screen.request.back", "Requests"),
    (
        "wl_inventory.screen.request.current",
        "Preferred supplier now",
    ),
    ("wl_inventory.screen.request.proposed", "Proposed"),
    ("wl_inventory.screen.request.on_hold", "On hold"),
    ("wl_inventory.screen.request.decided_by", "Decided by"),
    ("wl_inventory.screen.request.reason", "Reason"),
    ("wl_inventory.screen.request.returned", "Note"),
    (
        "wl_inventory.screen.request.item",
        "Open the item's suppliers",
    ),
    ("wl_inventory.screen.request.approve", "Approve"),
    (
        "wl_inventory.screen.request.approve_message",
        "The proposed supplier becomes the preferred supplier of the item.",
    ),
    ("wl_inventory.screen.request.reject", "Reject"),
    (
        "wl_inventory.screen.request.reject_message",
        "Say why the preferred supplier stays as it is.",
    ),
    ("wl_inventory.screen.request.reject_reason", "Reason"),
    ("wl_inventory.screen.request.none", "No supplier"),
];
