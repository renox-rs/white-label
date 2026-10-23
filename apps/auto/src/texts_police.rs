//! The texts of the police report guard and of clearing the accident flag
//! (`AUT.ACL.20`, BL-AUT-07), in English. They join the app's other texts in
//! [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    (
        "auto.police.required",
        "Police report required (AUT.ACL.20): attach the police accident report and have a second user accept it before the repair is approved. :detail",
    ),
    ("auto.police.attached", "Attached and accepted"),
    ("auto.police.title", "Accident damage"),
    ("auto.police.report", "Police accident report"),
    ("auto.police.documents", "Open the documents of this record"),
    ("auto.police.clear", "Clear the accident flag"),
    (
        "auto.police.clear_description",
        "The workshop manager decides: the flag is cleared only with a reason, and never while an insurance claim is linked.",
    ),
    ("auto.police.clear_reason", "Why is this not accident damage?"),
    ("auto.police.clear_waiting", "Clearing asked"),
    ("auto.police.clear_asked", "Sent to the workshop manager."),
    ("auto.police.decide", "Decide the request"),
    (
        "auto.police.decide_description",
        "Approve to clear the accident flag, or refuse to keep it.",
    ),
    ("auto.police.decision", "Decision"),
    ("auto.police.decision.approve", "Approve"),
    ("auto.police.decision.refuse", "Refuse"),
    ("auto.police.cleared", "The accident flag is cleared."),
    ("auto.police.kept", "The accident flag stays."),
    (
        "auto.police.problem.clear_by_approval",
        "The accident flag cannot be taken off here: ask to clear it, with a reason, and the workshop manager decides.",
    ),
    (
        "auto.police.problem.not_flagged",
        "This job card is not flagged as accident damage.",
    ),
    (
        "auto.police.problem.clear_reason",
        "Give the reason the flag is cleared.",
    ),
    (
        "auto.police.problem.clear_waiting",
        "A request to clear the flag is already waiting for the workshop manager.",
    ),
    (
        "auto.police.problem.clear_claim",
        "An insurance claim is linked to this job card, so the accident flag stays.",
    ),
    (
        "auto.police.problem.clear_none",
        "No request to clear the accident flag is waiting.",
    ),
    (
        "auto.police.problem.clear_self",
        "The one who asked cannot decide the request: another holder of the approval decides.",
    ),
];
