//! The texts of the notification types 08 registers (BL-AUT-18), in English
//! and Arabic. They join the app's other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    (
        "auto.notice.ready_failed",
        "The message that the vehicle is ready was not delivered; call the customer.",
    ),
    ("auto.late.problem.no_time", "Give the time the vehicle left."),
    (
        "auto.late.problem.future",
        "A time in the future cannot be entered.",
    ),
    (
        "auto.notice.ready_sms_en",
        "{{ workshop }}: your vehicle {{ plate }} (job {{ job }}) is ready for collection. {{ address }} {{ phone }}",
    ),
    (
        "auto.notice.ready_sms_ar",
        "{{ workshop }}: مركبتك {{ plate }} (بطاقة العمل {{ job }}) جاهزة للاستلام. {{ address }} {{ phone }}",
    ),
    (
        "auto.notice.claim_approved_sms_en",
        "{{ workshop }}: the insurer approved the repair of your vehicle {{ plate }} (claim {{ claim }}).",
    ),
    (
        "auto.notice.claim_approved_sms_ar",
        "{{ workshop }}: وافقت شركة التأمين على إصلاح مركبتك {{ plate }} (المطالبة {{ claim }}).",
    ),
];
