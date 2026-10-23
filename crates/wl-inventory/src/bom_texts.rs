//! The texts of the bill of materials (the refusals of its service and its
//! screens), in English. An app changes or translates one by putting the key
//! in its `resources/lang/{locale}.json`. The keys under `wl_inventory.bom.ui.`
//! are the screens': a template reads them as `tx['name']`.

/// Every key of the bill of materials, with its English text.
pub(crate) const BOM_TEXTS: &[(&str, &str)] = &[
    (
        "wl_inventory.bom.bad_output",
        "The quantity the bill makes must be above zero.",
    ),
    (
        "wl_inventory.bom.bad_yield",
        "The yield must be above 0% and at most 100%.",
    ),
    (
        "wl_inventory.bom.bad_hours",
        "The standard hours cannot be negative.",
    ),
    (
        "wl_inventory.bom.unknown_item",
        "The item the bill makes does not exist.",
    ),
    (
        "wl_inventory.bom.draft_exists",
        "{{ item }} has a draft version already; release or finish it first.",
    ),
    (
        "wl_inventory.bom.locked",
        "Only a draft can be changed; make a new version of a released bill.",
    ),
    (
        "wl_inventory.bom.bad_quantity",
        "The quantity must be above zero.",
    ),
    (
        "wl_inventory.bom.bad_scrap",
        "The scrap must be between 0% and 100%.",
    ),
    (
        "wl_inventory.bom.unknown_component",
        "The component item does not exist.",
    ),
    (
        "wl_inventory.bom.own_component",
        "{{ item }} is the item the bill makes; it cannot be its own component.",
    ),
    (
        "wl_inventory.bom.duplicate_component",
        "{{ item }} is a component already; change its line instead.",
    ),
    (
        "wl_inventory.bom.no_lines",
        "A bill needs at least one component.",
    ),
    (
        "wl_inventory.bom.line_quantity",
        "Line {{ position }}: the quantity must be above zero.",
    ),
    (
        "wl_inventory.bom.line_unknown",
        "Line {{ position }}: the component does not exist.",
    ),
    (
        "wl_inventory.bom.not_checked",
        "The bill was not checked before it was released.",
    ),
    // The screens.
    ("wl_inventory.bom.ui.title", "Bills of materials"),
    (
        "wl_inventory.bom.ui.subtitle",
        "What it takes to make each finished item, by version.",
    ),
    ("wl_inventory.bom.ui.stock", "Stock"),
    ("wl_inventory.bom.ui.new", "New bill"),
    (
        "wl_inventory.bom.ui.new_description",
        "Starts version 1 of an item, or the next version when it has one. Add the components on its page.",
    ),
    ("wl_inventory.bom.ui.new_save", "Start the bill"),
    ("wl_inventory.bom.ui.item", "Item made"),
    ("wl_inventory.bom.ui.version", "Version"),
    ("wl_inventory.bom.ui.number", "Number"),
    ("wl_inventory.bom.ui.status", "Status"),
    ("wl_inventory.bom.ui.lines", "Components"),
    ("wl_inventory.bom.ui.cost", "Materials per unit"),
    ("wl_inventory.bom.ui.cost_pending", "Worked out at release"),
    ("wl_inventory.bom.ui.output", "Makes"),
    (
        "wl_inventory.bom.ui.output_hint",
        "The quantity the components below make, like 1 or 100.",
    ),
    ("wl_inventory.bom.ui.yield", "Yield"),
    (
        "wl_inventory.bom.ui.yield_hint",
        "Per cent of what is made that is good, like 95.",
    ),
    ("wl_inventory.bom.ui.hours", "Standard hours"),
    (
        "wl_inventory.bom.ui.hours_hint",
        "Line hours for the quantity it makes, like 0.5.",
    ),
    ("wl_inventory.bom.ui.released", "Released"),
    ("wl_inventory.bom.ui.obsoleted", "Obsolete since"),
    ("wl_inventory.bom.ui.empty", "No bill of materials yet"),
    (
        "wl_inventory.bom.ui.empty_text",
        "A work order is made from a released bill.",
    ),
    ("wl_inventory.bom.ui.filter", "Filter by status"),
    ("wl_inventory.bom.ui.all", "All"),
    ("wl_inventory.bom.ui.details", "Bill"),
    ("wl_inventory.bom.ui.edit", "Change the bill"),
    (
        "wl_inventory.bom.ui.edit_description",
        "What the components make, the yield and the standard hours.",
    ),
    ("wl_inventory.bom.ui.save", "Save"),
    ("wl_inventory.bom.ui.add_line", "Add a component"),
    (
        "wl_inventory.bom.ui.add_line_description",
        "The quantity is for what the bill makes; the scrap is extra material on top of it.",
    ),
    ("wl_inventory.bom.ui.component", "Component"),
    ("wl_inventory.bom.ui.quantity", "Quantity"),
    ("wl_inventory.bom.ui.scrap", "Scrap"),
    ("wl_inventory.bom.ui.scrap_hint", "Per cent extra, like 5."),
    ("wl_inventory.bom.ui.per_unit", "For one unit"),
    ("wl_inventory.bom.ui.remove", "Remove"),
    (
        "wl_inventory.bom.ui.remove_description",
        "The component leaves the draft.",
    ),
    ("wl_inventory.bom.ui.release", "Release"),
    (
        "wl_inventory.bom.ui.release_description",
        "Locks the version, works out the cost of its materials and makes it the one work orders use. The released version it replaces becomes obsolete.",
    ),
    ("wl_inventory.bom.ui.obsolete", "Make obsolete"),
    (
        "wl_inventory.bom.ui.obsolete_description",
        "No new work order can be made from it. Work orders already made from it go on.",
    ),
    ("wl_inventory.bom.ui.new_version", "New version"),
    (
        "wl_inventory.bom.ui.new_version_description",
        "Copies this version into a draft you can change. A released version is locked.",
    ),
    ("wl_inventory.bom.ui.versions", "Versions of this item"),
    ("wl_inventory.bom.ui.no_lines", "No component yet"),
    (
        "wl_inventory.bom.ui.locked_note",
        "A released bill is locked; make a new version to change it.",
    ),
    (
        "wl_inventory.bom.ui.blocked_draft",
        "Only a draft can be changed.",
    ),
    (
        "wl_inventory.bom.ui.blocked_released",
        "Only a released version can be made obsolete.",
    ),
    (
        "wl_inventory.bom.ui.blocked_release",
        "Only a draft can be released.",
    ),
    (
        "wl_inventory.bom.ui.blocked_has_draft",
        "This item has a draft version already.",
    ),
    ("wl_inventory.bom.ui.none", "None"),
    ("wl_inventory.bom.ui.not_released", "Not released yet"),
    ("wl_inventory.bom.ui.not_obsolete", "Still in use"),
    ("wl_inventory.bom.ui.status.draft", "Draft"),
    ("wl_inventory.bom.ui.status.released", "Released"),
    ("wl_inventory.bom.ui.status.obsolete", "Obsolete"),
    ("wl_inventory.bom.ui.done.made", "The bill was started."),
    ("wl_inventory.bom.ui.done.saved", "The bill was saved."),
    (
        "wl_inventory.bom.ui.done.line_added",
        "The component was added.",
    ),
    (
        "wl_inventory.bom.ui.done.line_removed",
        "The component was removed.",
    ),
    (
        "wl_inventory.bom.ui.done.released",
        "The version was released.",
    ),
    (
        "wl_inventory.bom.ui.done.obsolete",
        "The version is obsolete.",
    ),
    (
        "wl_inventory.bom.ui.done.new_version",
        "A new draft version was made.",
    ),
    ("wl_inventory.bom.ui.problem.item", "Choose the item."),
    (
        "wl_inventory.bom.ui.problem.number",
        "Enter the numbers as digits, like 1 or 0.5.",
    ),
    ("wl_inventory.bom.ui.nav", "Bills of materials"),
];
