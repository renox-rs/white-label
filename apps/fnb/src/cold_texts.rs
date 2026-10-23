//! The texts of the cold units and their temperature holds (BL-FNB-12): keys
//! `fnb.cold.*` (toasts, refusals, the task and the notice) and
//! `fnb.screen.cold.*` (what a template reads as `tx['cold...']`).

/// Every text of the cold units.
pub(crate) const COLD_TEXTS: &[(&str, &str)] = &[
    // Toasts.
    ("fnb.cold.created", "Cold unit set up."),
    ("fnb.cold.recorded", "Reading recorded."),
    (
        "fnb.cold.recorded_out_of_range",
        "Reading recorded: it is out of range, so the food in the unit is on hold and the chef decides it.",
    ),
    ("fnb.cold.decided", "Decision recorded."),
    (
        "fnb.cold.decision_waiting",
        "The discard waits for the kitchen manager to approve the wastage.",
    ),
    // The range in words.
    ("fnb.cold.at_most", "at most {{ limit }}"),
    ("fnb.cold.at_least", "at least {{ limit }}"),
    // Refusals.
    (
        "fnb.cold.problem.identity",
        "Give the unit a code and a name.",
    ),
    (
        "fnb.cold.problem.kind",
        "Choose fridge, freezer or hot-holding unit.",
    ),
    (
        "fnb.cold.problem.location",
        "A unit that logs temperatures needs a stock location: choose the location of the unit.",
    ),
    (
        "fnb.cold.problem.code_used",
        "The code {{ code }} is used by another unit.",
    ),
    (
        "fnb.cold.problem.range",
        "The lowest reading in range is above the highest.",
    ),
    (
        "fnb.cold.problem.inactive",
        "This unit no longer logs temperatures.",
    ),
    (
        "fnb.cold.problem.temperature",
        "That is not a temperature: write it in degrees, like 4 or 8.5.",
    ),
    (
        "fnb.cold.problem.decision",
        "Choose release, discard or return to the supplier.",
    ),
    (
        "fnb.cold.problem.evidence",
        "Record the reading and the minutes the food was out of range before deciding the line.",
    ),
    ("fnb.cold.problem.no_hold", "The unit has no stock on hold."),
    (
        "fnb.cold.problem.not_the_units_line",
        "That line is not on this unit's hold.",
    ),
    (
        "fnb.cold.problem.stock_off",
        "The stock module is off: the line cannot be decided.",
    ),
    // The equipment task and the notice to the chef and the kitchen manager.
    (
        "fnb.cold.task_title",
        "{{ unit }} reads {{ reading }}, out of range",
    ),
    (
        "fnb.cold.task_body",
        "{{ unit }} read {{ reading }} degrees; its range is {{ range }}. The stock in it is on hold until the chef decides each line. Check the equipment and the door seal, then call the technician if it stays out of range. {{ link }}",
    ),
    (
        "fnb.cold.notice_title",
        "{{ unit }} is out of range: its stock is on hold",
    ),
    (
        "fnb.cold.notice_body",
        "{{ unit }} read {{ reading }} degrees (range {{ range }}). The food in it is on hold: do not use, move or sell it. The chef decides each line: release, discard or return to the supplier.",
    ),
    // The screens.
    ("fnb.screen.cold.title", "Fridges and freezers"),
    (
        "fnb.screen.cold.subtitle",
        "Temperature readings of the cold units; a reading out of range holds the food in the unit.",
    ),
    ("fnb.screen.cold.create", "Set up a unit"),
    (
        "fnb.screen.cold.create_title",
        "Set up a fridge, freezer or hot-holding unit",
    ),
    (
        "fnb.screen.cold.create_description",
        "Each unit is a stock location of the outlet: the food in it is what a reading out of range holds.",
    ),
    ("fnb.screen.cold.code", "Code"),
    ("fnb.screen.cold.name", "Name"),
    ("fnb.screen.cold.kind", "Kind"),
    ("fnb.screen.cold.location", "Stock location"),
    (
        "fnb.screen.cold.location_hint",
        "Required: a unit cannot log temperatures without one.",
    ),
    ("fnb.screen.cold.min", "Lowest reading in range"),
    ("fnb.screen.cold.max", "Highest reading in range"),
    (
        "fnb.screen.cold.range_hint",
        "Degrees. Leave both empty for the kind's range: fridge at most 5, freezer at most minus 18, hot at least 60.",
    ),
    ("fnb.screen.cold.range", "Range"),
    ("fnb.screen.cold.reading", "Reading"),
    ("fnb.screen.cold.read_at", "Read at"),
    ("fnb.screen.cold.on_hold", "On temperature hold"),
    ("fnb.screen.cold.in_use", "In use"),
    ("fnb.screen.cold.record", "Record a reading"),
    (
        "fnb.screen.cold.record_title",
        "Record a temperature reading",
    ),
    (
        "fnb.screen.cold.record_description",
        "A reading outside the range holds the food in the unit, opens an equipment task and tells the chef and the kitchen manager.",
    ),
    ("fnb.screen.cold.temperature", "Temperature"),
    (
        "fnb.screen.cold.temperature_hint",
        "Degrees, like 4 or 8.5.",
    ),
    ("fnb.screen.cold.empty", "No cold units yet"),
    (
        "fnb.screen.cold.empty_text",
        "The kitchen manager sets up each fridge and freezer with its stock location.",
    ),
    ("fnb.screen.cold.details", "Unit"),
    ("fnb.screen.cold.hold", "Stock hold"),
    ("fnb.screen.cold.ticket", "Equipment task"),
    ("fnb.screen.cold.out_since", "Out of range since"),
    ("fnb.screen.cold.lines", "Food on hold"),
    ("fnb.screen.cold.item", "Item"),
    ("fnb.screen.cold.quantity", "Quantity"),
    ("fnb.screen.cold.value", "Value"),
    ("fnb.screen.cold.recorded", "Recorded"),
    ("fnb.screen.cold.minutes_unit", "minutes"),
    ("fnb.screen.cold.waiting", "Waiting for the kitchen manager"),
    ("fnb.screen.cold.decide", "Decide"),
    ("fnb.screen.cold.decide_title", "Decide this line"),
    (
        "fnb.screen.cold.decide_description",
        "Release the food, discard it as wastage, or send it back to the supplier. Record the reading and how long the food was out of range. Food that is not released is never sold.",
    ),
    ("fnb.screen.cold.decision", "Decision"),
    ("fnb.screen.cold.minutes_out", "Minutes out of range"),
    ("fnb.screen.cold.minutes_hint", "A whole number of minutes."),
    ("fnb.screen.cold.no_lines", "Nothing is on hold"),
    ("fnb.screen.cold.readings", "Readings"),
    ("fnb.screen.cold.no_readings", "No readings yet"),
    (
        "fnb.screen.cold.no_readings_text",
        "Record the temperature of the unit at each check.",
    ),
    ("fnb.screen.cold.kind.fridge", "Fridge"),
    ("fnb.screen.cold.kind.freezer", "Freezer"),
    ("fnb.screen.cold.kind.hot", "Hot holding"),
    ("fnb.screen.cold.line.undecided", "Waiting for the chef"),
    ("fnb.screen.cold.line.released", "Released"),
    ("fnb.screen.cold.line.discarded", "Discarded"),
    (
        "fnb.screen.cold.line.returned_to_supplier",
        "Returned to the supplier",
    ),
    ("fnb.screen.cold.decision.release", "Release"),
    ("fnb.screen.cold.decision.discard", "Discard (wastage)"),
    (
        "fnb.screen.cold.decision.return_to_supplier",
        "Return to the supplier",
    ),
    (
        "fnb.screen.cold.note_on_hold",
        "The food in this unit is on temperature hold: it cannot be used in a recipe, moved or counted as available. Stock moved in while the hold is open joins it. Each line is decided by the chef; a discard above AED 200 needs the kitchen manager.",
    ),
];
