//! The texts of the reissue of a certificate and of its printed copy
//! (BL-EDU-16), in English. They join the app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // Reissue.
    ("edtech.certificates.reissue", "Reissue"),
    (
        "edtech.certificates.reissue_description",
        "A certificate is never edited. This one becomes superseded and a new draft, linked to it, carries the corrections; the new draft gets its own number when it is issued. For a name or date correction, or a changed result that still meets the criteria. Otherwise revoke it.",
    ),
    (
        "edtech.certificates.reissue_reason",
        "Reason for the reissue",
    ),
    (
        "edtech.certificates.corrected_name",
        "Name as it should read",
    ),
    (
        "edtech.certificates.corrected_date",
        "Completed on, as it should read",
    ),
    (
        "edtech.certificates.copy_attestation",
        "Copy the recorded KHDA attestation to the new certificate",
    ),
    (
        "edtech.certificates.reissued_done",
        "Certificate reissued. The new draft is ready to issue.",
    ),
    (
        "edtech.certificates.reissue_reason_shown",
        "Reissued because",
    ),
    ("edtech.certificates.attestation", "KHDA attestation"),
    // The student's notice.
    (
        "edtech.certificates.reissued_title",
        "Your corrected certificate is ready",
    ),
    (
        "edtech.certificates.reissued_body",
        "Your certificate for :course was corrected. The new certificate is :number; check it at :path. The earlier certificate :old no longer stands.",
    ),
    // A copy.
    ("edtech.certificates.copy", "Print a copy"),
    (
        "edtech.certificates.copy_message",
        "The same certificate is printed again, with the same number and verification code. Nothing about it changes; the copy is kept in its history.",
    ),
    ("edtech.certificates.copy_done", "Copy ready to print."),
    ("edtech.certificates.copies", "Copies printed again"),
    (
        "edtech.certificates.print_title",
        "Certificate of completion",
    ),
    ("edtech.certificates.print_awarded", "Awarded to"),
    (
        "edtech.certificates.problem.not_printable",
        "Only an issued certificate can be printed again.",
    ),
];
