//! The texts of what a blocked party does to the agreements (BL-AUT-17), in
//! English. They join the app's other texts in [`crate::texts`]. The real reason
//! of a block never appears: tasks and messages say *on hold*.

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("auto.blocked.on_hold", "On hold"),
    ("auto.blocked.fleet_label", ":number - :title"),
    ("auto.blocked.card_label", ":number - :plate (:status)"),
    (
        "auto.blocked.problem.approval",
        "This customer is on hold: a new estimate approval waits for the branch manager's review.",
    ),
    (
        "auto.blocked.problem.link",
        "This estimate cannot be approved online now. Please contact the workshop.",
    ),
    (
        "auto.blocked.problem.end_job",
        "A job card of a customer on hold is stopped from the job card (cancel or decline), not from the review.",
    ),
    (
        "auto.blocked.problem.safety_line",
        "Only a line still in the estimate of an open job card can be marked as safety work.",
    ),
    ("auto.blocked.safety_tag", "safety work"),
    ("auto.blocked.line_removed", "Removed: customer on hold"),
    ("auto.blocked.safety", "Mark safety work"),
    (
        "auto.blocked.safety_description",
        "Safety work makes the vehicle safe to drive. If the customer is put on hold, these lines continue and the others not started are taken off.",
    ),
    ("auto.blocked.safety_line", "Line"),
    ("auto.blocked.safety_line_placeholder", "Pick the line"),
    (
        "auto.blocked.safety_tick",
        "Needed to make the vehicle safe to drive",
    ),
    ("auto.blocked.safety_saved", "Safety work saved."),
    (
        "auto.blocked.banner",
        "This customer is on hold: new estimate approvals wait for the branch manager's review.",
    ),
];
