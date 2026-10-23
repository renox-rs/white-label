//! The texts of the test drives and the vehicle incidents (`AUT.ACL.22`,
//! BL-AUT-06), in English. They join the app's other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // Test drives.
    ("auto.drives.title", "Test drives"),
    ("auto.drives.driver", "Driver"),
    ("auto.drives.out", "Out"),
    ("auto.drives.back", "Back"),
    ("auto.drives.km_out", "Odometer out"),
    ("auto.drives.km_in", "Odometer in"),
    ("auto.drives.distance", "Driven"),
    ("auto.drives.still_out", "Still out"),
    ("auto.drives.empty", "No test drive yet."),
    ("auto.drives.record", "Record a test drive"),
    (
        "auto.drives.record_description",
        "The vehicle leaves for a test drive. Record the odometer reading when it leaves; the reading when it is back closes the drive.",
    ),
    ("auto.drives.finish", "Vehicle back"),
    (
        "auto.drives.finish_description",
        "Record the odometer reading now that the vehicle is back. The QC cannot pass while a drive is open.",
    ),
    ("auto.drives.saved", "Test drive recorded."),
    ("auto.drives.closed", "Test drive closed."),
    (
        "auto.drives.problem.open",
        "A test drive has no odometer reading back yet: record it before the QC passes.",
    ),
    (
        "auto.drives.problem.closed",
        "The job card is closed: no test drive can be recorded.",
    ),
    (
        "auto.drives.problem.odometer_out",
        "The odometer cannot be below the check-in reading of :km km.",
    ),
    (
        "auto.drives.problem.odometer_in",
        "The reading back cannot be below the reading out of :km km.",
    ),
    (
        "auto.drives.problem.out",
        "A test drive is still out: record its return first.",
    ),
    (
        "auto.drives.problem.half_return",
        "Give both the time and the odometer reading back, or neither.",
    ),
    (
        "auto.drives.problem.time",
        "The vehicle cannot be back before it left.",
    ),
    (
        "auto.drives.problem.back",
        "This test drive is back already.",
    ),
    // Incidents.
    ("auto.incidents.title", "Vehicle incidents"),
    ("auto.incidents.number", "Incident"),
    ("auto.incidents.description", "What happened"),
    ("auto.incidents.road_accident", "Road accident"),
    (
        "auto.incidents.road_hint",
        "Tick it when the damage came from a road accident, on a test drive or while moving the vehicle.",
    ),
    ("auto.incidents.ticket", "Ticket"),
    ("auto.incidents.liability", "Liability"),
    ("auto.incidents.liability.workshop", "The workshop"),
    ("auto.incidents.liability.not_workshop", "Not the workshop"),
    ("auto.incidents.undecided", "Waiting for the branch manager"),
    ("auto.incidents.reason", "Reason"),
    ("auto.incidents.internal", "Internal job card"),
    ("auto.incidents.empty", "No incident reported."),
    ("auto.incidents.report", "Report an incident"),
    (
        "auto.incidents.report_description",
        "Damage while the vehicle is in the workshop's care or on a test drive. A ticket is opened for the branch manager, who decides the liability.",
    ),
    ("auto.incidents.decide", "Decide the liability"),
    (
        "auto.incidents.decide_description",
        "Compare the check-in photos with the incident photos. If the workshop caused the damage, an internal job card repairs it at no charge to the customer.",
    ),
    ("auto.incidents.reported", "Incident reported."),
    ("auto.incidents.decided", "Liability recorded."),
    ("auto.incidents.is_internal", "Internal: the workshop pays"),
    (
        "auto.incidents.ticket_title",
        "Vehicle incident on :job (:incident)",
    ),
    (
        "auto.incidents.ticket_body",
        ":description. Road accident: :road.",
    ),
    ("auto.incidents.road_yes", "yes"),
    ("auto.incidents.road_no", "no"),
    (
        "auto.incidents.internal_complaint",
        "Damage caused in the workshop: :description",
    ),
    (
        "auto.incidents.resolution_workshop",
        "Liability: the workshop. :reason",
    ),
    (
        "auto.incidents.resolution_not_workshop",
        "Liability: not the workshop. :reason",
    ),
    (
        "auto.incidents.problem.closed",
        "The job card is closed: an incident cannot be reported on it.",
    ),
    ("auto.incidents.problem.description", "Say what happened."),
    (
        "auto.incidents.problem.decided",
        "The liability of this incident is decided already.",
    ),
    (
        "auto.incidents.problem.liability",
        "Choose the workshop or not the workshop.",
    ),
    (
        "auto.incidents.problem.reason",
        "Give the reason for the decision.",
    ),
    (
        "auto.incidents.problem.involved",
        "You were involved in this incident, so you cannot decide it: another branch manager or the owner decides.",
    ),
    (
        "auto.incidents.problem.on_the_job_card",
        "The liability of a vehicle incident is decided by the branch manager on the job card, not here.",
    ),
];
