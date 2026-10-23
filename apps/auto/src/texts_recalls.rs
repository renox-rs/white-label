//! The texts of a recalled part traced to vehicle owners, fleets and
//! customers (`BL-AUT-16`), in English. They join the app's other texts in
//! [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("auto.recall.title", "Recall: customers who got the part"),
    (
        "auto.recall.subtitle",
        "Parts are not tracked by lot, so every line is possible: the jobs and sales from the first receipt named in the supplier's notice.",
    ),
    ("auto.recall.record", "Job or sale"),
    ("auto.recall.party", "Customer"),
    ("auto.recall.vehicle", "Vehicle"),
    ("auto.recall.day", "Day"),
    ("auto.recall.branch", "Branch"),
    ("auto.recall.mark", "Evidence"),
    ("auto.recall.notice", "Notice"),
    ("auto.recall.back", "Recall"),
    ("auto.recall.empty", "No job card or sale used the part yet."),
    ("auto.recall.walk_in", "Walk-in sales"),
    ("auto.recall.walk_in_count", ":count sales"),
    ("auto.recall.walk_in_one", "1 sale"),
    ("auto.recall.mark_possible", "Possible"),
    ("auto.recall.mark_confirmed", "Proved"),
    ("auto.recall.notice_pending", "Not told"),
    ("auto.recall.notice_sent", "Sent"),
    ("auto.recall.notice_waived", "Waived"),
    ("auto.recall.party_unknown", "Unknown: waive it in the recall"),
    ("auto.recall.send", "Send the notices"),
    (
        "auto.recall.send_description",
        "Tells each customer, fleet contact or vehicle owner still to tell, about their own vehicle or purchase only.",
    ),
    ("auto.recall.sent", "Notices sent."),
    (
        "auto.recall.problem.nothing",
        "Nobody is left to tell: every line with a known customer was told or waived.",
    ),
    ("auto.recall.what_vehicle", "your vehicle :plate"),
    ("auto.recall.what_fleet_vehicle", "the vehicle :plate of your fleet"),
    ("auto.recall.what_sale", "your purchase :record"),
    ("auto.recall.mail.subject", "Safety notice from {{ workshop }}"),
    (
        "auto.recall.mail.body",
        "{{ workshop }}: a part we supplied for {{ what }} on {{ date }} may be affected by a supplier recall. Please bring it in for a check.",
    ),
];
