//! The texts of the seats, the waiting list, the transfer and the lapse of a
//! seat hold (BL-EDU-02, BL-EDU-01), in English. They join the app's other
//! texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The seats.
    (
        "edtech.seats.problem.full_with_alternatives",
        "The batch has no free seat. These other open batches of the course have one: :batches. You can also put the student on the waiting list of this batch.",
    ),
    (
        "edtech.seats.problem.full_no_alternative",
        "The batch has no free seat and no other open batch of the course has one. You can put the student on the waiting list of this batch.",
    ),
    ("edtech.seats.alternative", ":batch (:free free)"),
    // The waiting list.
    ("edtech.waiting.title", "Waiting list"),
    ("edtech.waiting.add", "Add to the waiting list"),
    (
        "edtech.waiting.add_description",
        "A plain list of the students who asked for a seat of this full batch. No seat is held and nothing is offered when one frees: look at the list and enrol the student.",
    ),
    ("edtech.waiting.student", "Student"),
    ("edtech.waiting.payer", "Payer"),
    ("edtech.waiting.since", "Added"),
    ("edtech.waiting.remove", "Take off"),
    (
        "edtech.waiting.remove_message",
        "The student is taken off the waiting list of this batch.",
    ),
    ("edtech.waiting.empty", "Nobody is waiting for a seat"),
    ("edtech.waiting.added", "Added to the waiting list."),
    ("edtech.waiting.removed", "Taken off the waiting list."),
    (
        "edtech.waiting.problem.not_open",
        "Only a batch that is open for enrolment has a waiting list.",
    ),
    (
        "edtech.waiting.problem.seated",
        "The student has a seat in this batch already.",
    ),
    (
        "edtech.waiting.problem.listed",
        "The student is on the waiting list of this batch already.",
    ),
    // The transfer.
    ("edtech.transfer.action", "Move to another batch"),
    (
        "edtech.transfer.description",
        "The student keeps what was paid for the sessions not yet used and moves to the batch you pick. The target batch must have a free seat.",
    ),
    ("edtech.transfer.target", "Target batch (number)"),
    ("edtech.transfer.done", "Student moved to the other batch."),
    (
        "edtech.transfer.problem.not_course",
        "Only a seat in a batch can be moved to another batch.",
    ),
    (
        "edtech.transfer.problem.status",
        "Only an active or frozen enrolment can be moved to another batch.",
    ),
    (
        "edtech.transfer.problem.same",
        "The student is in this batch already.",
    ),
    (
        "edtech.transfer.problem.course",
        "The other batch runs another course; enrol the student in it instead.",
    ),
    (
        "edtech.transfer.problem.not_open",
        "The other batch is not open or running, so it takes no student.",
    ),
];
