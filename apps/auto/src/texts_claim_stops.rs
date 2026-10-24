//! The texts of the claims that end before the repair is paid: withdrawn by the
//! customer, a total loss, registered in error (BL-AUT-08 .. BL-AUT-10), and of
//! what the workshop writes to the insurer, in English. They join the app's
//! other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The actions on the claim page.
    ("auto.claims.survey_request", "Email the survey request"),
    (
        "auto.claims.survey_request_message",
        "The insurer's claim email is asked to send the surveyor. The claim keeps its state.",
    ),
    ("auto.claims.survey_requested", "Survey request sent."),
    ("auto.claims.choice", "If work started"),
    (
        "auto.claims.choice_hint",
        "Needed once labour is done or parts are issued: the customer pays the repair on a private estimate, or stops the job and pays the work done.",
    ),
    ("auto.claims.choice.stop", "Stop the job"),
    (
        "auto.claims.choice.continue_privately",
        "Continue privately",
    ),
    (
        "auto.claims.withdrawal_hint",
        "The customer's signed letter. Needed once the insurer approved the claim.",
    ),
    ("auto.claims.unlinked_job_card", "Unlinked job card"),
    (
        "auto.claims.charges_action",
        "Record the approval of the charges",
    ),
    (
        "auto.claims.charges_description",
        "Give the insurer's reference and written approval, and the amount approved for each charge. A charge left blank, or approved for less, is billed to the customer.",
    ),
    ("auto.claims.charges_approved", "Charges approved."),
    ("auto.claims.charges_title", "Total-loss charges"),
    ("auto.claims.charges_empty", "No charges."),
    ("auto.claims.charge", "Charge"),
    (
        "auto.claims.withdrawal_reference",
        "Claim :claim withdrawn by the customer",
    ),
    // What the total loss lists and bills.
    (
        "auto.total_loss.storage_line",
        "Storage for :days days at :rate a day",
    ),
    (
        "auto.total_loss.customer_line",
        ":description (not approved by the insurer)",
    ),
    // What is refused, in words.
    (
        "auto.claims.problem.withdrawal",
        "Attach the customer's written withdrawal: the insurer already approved the claim.",
    ),
    (
        "auto.claims.problem.choice",
        "Work has started: say whether the customer continues privately or stops the job.",
    ),
    (
        "auto.claims.problem.choice_unknown",
        "Choose to continue privately or to stop the job.",
    ),
    (
        "auto.claims.problem.private_state",
        "The customer can continue privately only while the job is in progress. Choose to stop the job.",
    ),
    (
        "auto.claims.problem.private_unsigned",
        "The customer has not signed the private estimate yet: no labour or parts are recorded until then.",
    ),
    (
        "auto.claims.problem.sent",
        "Something was already sent to the insurer for this claim: withdraw it instead of voiding it.",
    ),
    (
        "auto.claims.problem.notice",
        "Attach the insurer's total-loss notice.",
    ),
    (
        "auto.claims.problem.not_total_loss",
        "The insurer has not declared this claim a total loss.",
    ),
    ("auto.release.instruction", "Insurer's written instruction"),
    (
        "auto.release.instruction_hint",
        "The name or reference of the insurer's letter that says who takes the vehicle.",
    ),
    (
        "auto.release.problem.instruction",
        "After a total loss the vehicle leaves only on the insurer's written instruction: attach it.",
    ),
    (
        "auto.release.problem.total_loss_signature",
        "After a total loss the person who takes the vehicle signs for it.",
    ),
    // The letters and the notice.
    (
        "auto.mail.withdrawal_subject",
        "Claim {{ claim_no }}: withdrawn by the customer",
    ),
    (
        "auto.mail.withdrawal_body",
        "Dear {{ insurer }}, our customer {{ customer }} has withdrawn claim {{ claim_no }} (policy {{ policy_no }}, our reference {{ claim }}). Reason: {{ reason }}. The customer's written withdrawal: {{ document }}. {{ choice }} Please stop any further approval or payment on this claim and confirm by reply. Thank you.",
    ),
    (
        "auto.mail.survey_subject",
        "Claim {{ claim_no }}: please send the surveyor",
    ),
    (
        "auto.mail.survey_body",
        "Dear {{ insurer }}, please send the surveyor to inspect the vehicle of {{ customer }} for claim {{ claim_no }} (policy {{ policy_no }}, our reference {{ claim }}), on site or from our photos. Thank you.",
    ),
    (
        "auto.mail.total_loss_sms_en",
        "{{ workshop }}: your insurer declared your vehicle {{ plate }} a total loss (claim {{ claim_no }}). Work has stopped. We will contact you about the vehicle.",
    ),
    (
        "auto.mail.total_loss_sms_ar",
        "{{ workshop }}: أعلنت شركة التأمين أن مركبتك {{ plate }} خسارة كلية (المطالبة {{ claim_no }}). توقفت الأعمال وسنتواصل معك بشأن المركبة.",
    ),
    (
        "auto.mail.task_title",
        "Send the letter about :claim to the insurer by hand",
    ),
    (
        "auto.mail.task_body",
        "The email to the insurer about :claim could not be sent. Send it by hand and keep a copy with the claim.",
    ),
];
