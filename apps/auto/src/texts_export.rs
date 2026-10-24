//! The texts of the export sales and their exit evidence (`AUT.EXP.01`,
//! `AUT.RUL.15`, BL-AUT-14), in English. They join the app's other texts in
//! [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("auto.export.invoice_placeholder", "Pick the zero-rated invoice"),
    ("auto.export.title", "Export sales"),
    (
        "auto.export.subtitle",
        "Zero-rated sales of parts that leave the UAE. Exit evidence must be attached within 90 days of the supply date, or 5% VAT is due.",
    ),
    ("auto.export.new", "Register an export sale"),
    (
        "auto.export.new_description",
        "Register the zero-rated invoice with the day the goods were delivered or collected. The accountant is told at 60 days without exit evidence, and the sale is flagged at 90.",
    ),
    ("auto.export.invoice", "Invoice"),
    (
        "auto.export.invoice_hint",
        "An issued invoice without VAT that is not an export sale yet.",
    ),
    ("auto.export.supply_date", "Supply date"),
    (
        "auto.export.supply_hint",
        "The delivery or collection date: the 60 and 90 days count from it.",
    ),
    ("auto.export.customer", "Customer"),
    ("auto.export.total", "Invoice total"),
    ("auto.export.days", "Days since supply"),
    ("auto.export.evidence", "Exit evidence"),
    ("auto.export.empty", "No export sale is registered."),
    ("auto.export.registered", "Export sale registered."),
    ("auto.export.attach", "Attach exit evidence"),
    (
        "auto.export.attach_description",
        "A customs exit certificate, a bill of lading or an airway bill. Any one of them stops the reminders for this invoice.",
    ),
    ("auto.export.kind", "Kind of evidence"),
    ("auto.export.kind_placeholder", "Pick the kind"),
    ("auto.export.reference", "Document number or file name"),
    ("auto.export.attached", "Exit evidence attached."),
    (
        "auto.export.kind.exit_certificate",
        "Customs exit certificate",
    ),
    ("auto.export.kind.bill_of_lading", "Bill of lading"),
    ("auto.export.kind.airway_bill", "Airway bill"),
    ("auto.export.status.waiting", "Waiting for exit evidence"),
    (
        "auto.export.status.missing",
        "Exit evidence missing, accountant told",
    ),
    ("auto.export.status.vat_due", "5% VAT due"),
    (
        "auto.export.status.vat_due_late",
        "5% VAT due, evidence came late",
    ),
    ("auto.export.status.attached", "Exit evidence attached"),
    (
        "auto.export.notice_title",
        "Exit evidence missing for export invoice :invoice",
    ),
    (
        "auto.export.notice_body",
        "Export invoice :invoice, supplied on :date, is zero-rated and has no customs exit certificate, bill of lading or airway bill 60 days later. Attach the evidence before day 90, or 5% VAT is due on the sale.",
    ),
    (
        "auto.export.task_title",
        "Correct the VAT of export invoice :invoice: 5% VAT is due",
    ),
    (
        "auto.export.task_body",
        "Export invoice :invoice, supplied on :date, has no exit evidence after 90 days, so the zero rate no longer applies. Issue a credit note and a new invoice at 5% by hand. Recharge the VAT to the customer where the sale terms allow, otherwise absorb it. Nothing is issued automatically.",
    ),
    (
        "auto.export.late_title",
        "Exit evidence came late for export invoice :invoice",
    ),
    (
        "auto.export.late_body",
        "A :kind was attached to export invoice :invoice after the 90 days. The 5% VAT flag stays and nothing is reversed: whether the zero rate can be restored is for the accountant to decide.",
    ),
    (
        "auto.export.problem.future",
        "The supply date cannot be in the future.",
    ),
    (
        "auto.export.problem.invoice",
        "That invoice does not exist. Give the number shown on the invoice page.",
    ),
    (
        "auto.export.problem.credited",
        "That invoice was credited in full; it is no longer a sale to follow.",
    ),
    (
        "auto.export.problem.known",
        "That invoice is registered as an export sale already.",
    ),
    (
        "auto.export.problem.kind",
        "Pick a customs exit certificate, a bill of lading or an airway bill.",
    ),
    (
        "auto.export.problem.reference",
        "Give the number or the file name of the document.",
    ),
];
