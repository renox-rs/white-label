//! The texts of the enrolments, their fees, the certificates, the public
//! verification page and the tutor assignments, in English. They join the
//! app's other texts in [`crate::texts`]; an app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The enrolments.
    ("edtech.enrolments.title", "Enrolments"),
    (
        "edtech.enrolments.subtitle",
        "The students in the batches, packages and memberships, with their fees.",
    ),
    ("edtech.enrolments.new", "Enrol a student"),
    (
        "edtech.enrolments.new_description",
        "A course seat is held until the first payment arrives. Pick the student, the guardian of a student under 18 and the payer from the parties.",
    ),
    ("edtech.enrolments.number", "Enrolment"),
    ("edtech.enrolments.student", "Student"),
    ("edtech.enrolments.guardian", "Guardian"),
    ("edtech.enrolments.payer", "Payer"),
    (
        "edtech.enrolments.party_placeholder",
        "Search by name or code, for example P-00023",
    ),
    ("edtech.enrolments.kind", "For"),
    ("edtech.enrolments.kind.course", "Course"),
    ("edtech.enrolments.kind.package", "Package of sessions"),
    ("edtech.enrolments.kind.membership", "Membership"),
    ("edtech.enrolments.subject", "Batch or item"),
    ("edtech.enrolments.batch", "Batch (open ones)"),
    ("edtech.enrolments.seats_free", "seats free"),
    (
        "edtech.enrolments.item_code",
        "Item code (package or membership)",
    ),
    (
        "edtech.enrolments.item_name",
        "Item name (package or membership)",
    ),
    ("edtech.enrolments.credits", "Sessions in the package"),
    ("edtech.enrolments.price", "Price with VAT (AED)"),
    ("edtech.enrolments.discount", "Discount (AED)"),
    (
        "edtech.enrolments.registration_fee",
        "Registration fee (AED)",
    ),
    ("edtech.enrolments.admin_fee", "Admin fee (AED)"),
    ("edtech.enrolments.instalments", "Instalments"),
    (
        "edtech.enrolments.money_hint",
        "For example 2520 or 2520.50",
    ),
    (
        "edtech.enrolments.terms_accepted",
        "The terms and the refund policy are accepted",
    ),
    ("edtech.enrolments.fee", "Fee"),
    ("edtech.enrolments.empty", "No enrolments yet"),
    (
        "edtech.enrolments.created",
        "Student enrolled; the seat is held.",
    ),
    (
        "edtech.enrolments.status.pending_payment",
        "Waiting for payment",
    ),
    ("edtech.enrolments.status.active", "Active"),
    ("edtech.enrolments.status.frozen", "Frozen"),
    ("edtech.enrolments.status.completed", "Completed"),
    ("edtech.enrolments.status.withdrawn", "Withdrawn"),
    ("edtech.enrolments.status.transferred", "Transferred"),
    ("edtech.enrolments.status.lapsed", "Lapsed"),
    ("edtech.enrolments.details", "Details"),
    ("edtech.enrolments.fees", "Fees"),
    ("edtech.enrolments.terms", "Terms accepted"),
    ("edtech.enrolments.terms_missing", "Not yet"),
    ("edtech.enrolments.seat_hold_until", "Seat held until"),
    ("edtech.enrolments.discount_approval", "Discount"),
    (
        "edtech.enrolments.discount_ok",
        "Approved or within the limit",
    ),
    (
        "edtech.enrolments.discount_waiting",
        "Waiting for the centre manager",
    ),
    ("edtech.enrolments.if_leaves", "If the student leaves today"),
    ("edtech.enrolments.refund", "Refund"),
    ("edtech.enrolments.owed", "Amount owed"),
    ("edtech.enrolments.withdrawal", "Withdrawal"),
    ("edtech.enrolments.reason", "Reason"),
    ("edtech.enrolments.instalment_plan", "Instalments"),
    ("edtech.enrolments.instalment_no", "No."),
    ("edtech.enrolments.due", "Due"),
    ("edtech.enrolments.no_instalments", "No instalments"),
    ("edtech.enrolments.instalment.scheduled", "Scheduled"),
    ("edtech.enrolments.instalment.invoiced", "Invoiced"),
    ("edtech.enrolments.instalment.cancelled", "Cancelled"),
    ("edtech.enrolments.freezes", "Freezes"),
    ("edtech.enrolments.cause", "Cause"),
    ("edtech.enrolments.cause.student", "The student asked"),
    ("edtech.enrolments.cause.linked", "The membership is paused"),
    ("edtech.enrolments.cause.block", "On hold"),
    ("edtech.enrolments.since", "From"),
    ("edtech.enrolments.ended", "Ended"),
    ("edtech.enrolments.no_freezes", "Never frozen"),
    ("edtech.enrolments.certificates", "Certificates"),
    ("edtech.enrolments.no_certificates", "No certificate yet"),
    ("edtech.enrolments.accept_terms", "Terms accepted"),
    (
        "edtech.enrolments.accept_terms_message",
        "The student or the guardian accepted the terms and the refund policy.",
    ),
    ("edtech.enrolments.terms_done", "Terms recorded."),
    ("edtech.enrolments.approve_discount", "Approve the discount"),
    (
        "edtech.enrolments.approve_discount_message",
        "The discount is above the limit. Approve it so the enrolment can be activated.",
    ),
    ("edtech.enrolments.discount_done", "Discount approved."),
    ("edtech.enrolments.activate", "Activate"),
    (
        "edtech.enrolments.activate_message",
        "The seat is confirmed and the student starts. This is done by itself when the first payment arrives.",
    ),
    ("edtech.enrolments.activated", "Enrolment active."),
    ("edtech.enrolments.freeze", "Freeze"),
    (
        "edtech.enrolments.freeze_description",
        "Pause the enrolment at the student's request. It counts against the 30 days a year.",
    ),
    ("edtech.enrolments.freeze_until", "Last day of the freeze"),
    ("edtech.enrolments.frozen", "Enrolment frozen."),
    ("edtech.enrolments.unfreeze", "Unfreeze"),
    (
        "edtech.enrolments.unfreeze_message",
        "The student starts again.",
    ),
    ("edtech.enrolments.unfrozen", "Enrolment active again."),
    ("edtech.enrolments.withdraw", "Withdraw"),
    (
        "edtech.enrolments.withdraw_description",
        "The student leaves. The refund or the amount owed follows the refund policy accepted at enrolment; a refund waits for another user to approve it before the enrolment is withdrawn.",
    ),
    ("edtech.enrolments.withdrawn", "Enrolment withdrawn."),
    (
        "edtech.enrolments.problem.terms",
        "The terms and the refund policy are not accepted yet.",
    ),
    (
        "edtech.enrolments.problem.discount",
        "The discount is above the limit and waits for the centre manager's approval.",
    ),
    (
        "edtech.enrolments.problem.freeze_cause",
        "Name why the enrolment is frozen.",
    ),
    (
        "edtech.enrolments.problem.allowance",
        "The freeze would use more than the 30 days a year the student is allowed.",
    ),
    (
        "edtech.enrolments.problem.unfreeze_cause",
        "Name which freeze ends.",
    ),
    (
        "edtech.enrolments.problem.cause_left",
        "Another cause still holds the enrolment frozen.",
    ),
    (
        "edtech.enrolments.problem.refund",
        "The refund is calculated before the student leaves.",
    ),
    (
        "edtech.enrolments.problem.target",
        "Name the batch the student moves to.",
    ),
    (
        "edtech.enrolments.problem.amounts",
        "The amounts cannot be negative and the discount cannot be more than the price.",
    ),
    (
        "edtech.enrolments.problem.instalments",
        "A course fee is paid in 1 to 12 instalments.",
    ),
    (
        "edtech.enrolments.problem.batch",
        "Name the batch the student joins.",
    ),
    (
        "edtech.enrolments.problem.package",
        "A package needs an item code and at least one session.",
    ),
    (
        "edtech.enrolments.problem.membership",
        "A membership needs an item code.",
    ),
    (
        "edtech.enrolments.problem.student",
        "The student is not a known party.",
    ),
    (
        "edtech.enrolments.problem.guardian",
        "The guardian is not a known party.",
    ),
    (
        "edtech.enrolments.problem.payer",
        "The payer is not a known party.",
    ),
    (
        "edtech.enrolments.problem.not_guardian",
        "This party is not recorded as a guardian of the student.",
    ),
    (
        "edtech.enrolments.problem.batch_not_open",
        "The batch is not open for enrolment.",
    ),
    (
        "edtech.enrolments.problem.not_pending",
        "The enrolment is not waiting for payment.",
    ),
    (
        "edtech.enrolments.problem.until",
        "Give the last day of the freeze, today or later.",
    ),
    (
        "edtech.enrolments.problem.already_frozen",
        "The enrolment is already frozen for this cause.",
    ),
    (
        "edtech.enrolments.problem.not_frozen",
        "The enrolment is not frozen for this cause.",
    ),
    // The fees on the invoices.
    ("edtech.fees.registration", "Registration fee"),
    ("edtech.fees.course", "Course fee: :item"),
    (
        "edtech.fees.instalment",
        "Course fee: :item, instalment :number of :total",
    ),
    ("edtech.fees.package", "Package of sessions: :item"),
    ("edtech.fees.membership", "Membership"),
    // The certificates.
    ("edtech.certificates.title", "Certificates"),
    (
        "edtech.certificates.subtitle",
        "Proof of completion, with a verification code anyone can check.",
    ),
    ("edtech.certificates.empty", "No certificates yet"),
    ("edtech.certificates.number", "Certificate"),
    ("edtech.certificates.student", "Name"),
    ("edtech.certificates.course", "Course"),
    ("edtech.certificates.completed", "Completed on"),
    ("edtech.certificates.issued", "Issued on"),
    ("edtech.certificates.status.draft", "Draft"),
    ("edtech.certificates.status.issued", "Issued"),
    ("edtech.certificates.status.superseded", "Superseded"),
    ("edtech.certificates.status.revoked", "Revoked"),
    ("edtech.certificates.details", "Details"),
    ("edtech.certificates.approval", "Course approval"),
    ("edtech.certificates.code", "Verification code"),
    ("edtech.certificates.verify_page", "Public page"),
    ("edtech.certificates.replaces", "Replaces"),
    ("edtech.certificates.replaced_by", "Replaced by"),
    ("edtech.certificates.revoke_reason", "Revoked because"),
    ("edtech.certificates.issue", "Issue"),
    (
        "edtech.certificates.issue_message",
        "The certificate gets its number and the public page shows it as valid.",
    ),
    ("edtech.certificates.issued_done", "Certificate issued."),
    ("edtech.certificates.revoke", "Revoke"),
    (
        "edtech.certificates.revoke_description",
        "The certificate was given in error or by misconduct. The public page will say it is revoked.",
    ),
    ("edtech.certificates.reason", "Reason"),
    ("edtech.certificates.revoked_done", "Certificate revoked."),
    (
        "edtech.certificates.problem.criteria",
        "The completion criteria are not met: the enrolment is not completed.",
    ),
    (
        "edtech.certificates.problem.reissue_criteria",
        "The criteria are no longer met. A certificate that should not stand is revoked, not reissued.",
    ),
    (
        "edtech.certificates.problem.no_draft",
        "The replacement draft was not found.",
    ),
    // The public verification page.
    ("edtech.verify.title", "Certificate verification"),
    (
        "edtech.verify.subtitle",
        "Check a certificate with the verification code printed on it.",
    ),
    (
        "edtech.verify.unknown",
        "No published certificate has this code.",
    ),
    ("edtech.verify.valid", "Valid"),
    ("edtech.verify.superseded", "Superseded"),
    ("edtech.verify.revoked", "Revoked"),
    ("edtech.verify.replaced_by", "Replaced by"),
    (
        "edtech.verify.replacement_pending",
        "Superseded, replacement pending.",
    ),
    // The tutor assignments.
    ("edtech.tutors.change", "Change tutor"),
    (
        "edtech.tutors.change_description",
        "From the day given, the sessions of the batch's tutor are taught by the new tutor. The new tutor must be free and eligible for every one of them, or nothing changes.",
    ),
    ("edtech.tutors.from", "From"),
    ("edtech.tutors.to", "To"),
    ("edtech.tutors.history", "Tutors"),
    ("edtech.tutors.scope", "For"),
    ("edtech.tutors.none", "No tutor assigned yet"),
    ("edtech.tutors.batch", "Whole batch"),
    ("edtech.tutors.session", "Session"),
    ("edtech.tutors.changed", "Tutor changed."),
    (
        "edtech.tutors.problem.batch_over",
        "The batch is over: its tutor cannot change.",
    ),
];
