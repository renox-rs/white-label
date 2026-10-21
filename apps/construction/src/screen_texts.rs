//! The texts of the screens (the HTML pages of `/construction/...`), in
//! English. An app changes or translates one by putting the key in its
//! `resources/lang/{locale}.json`.

/// Every key of the screens, with its English text.
pub(crate) const SCREEN_TEXTS: &[(&str, &str)] = &[
    // Shared words.
    ("construction.screen.status", "Status"),
    ("construction.screen.kind", "Kind"),
    ("construction.screen.amount", "Amount"),
    ("construction.screen.reference", "Reference"),
    ("construction.screen.description", "Description"),
    ("construction.screen.project", "Project"),
    ("construction.screen.projects", "Projects"),
    ("construction.screen.none", "None."),
    ("construction.screen.related", "Related records"),
    // The statuses and kinds the pages write in words.
    ("construction.screen.status.setup", "Setting up"),
    ("construction.screen.status.in_progress", "In progress"),
    ("construction.screen.status.on_hold", "On hold"),
    (
        "construction.screen.status.practical_completion",
        "Practical completion",
    ),
    ("construction.screen.status.terminated", "Terminated"),
    ("construction.screen.status.closed", "Closed"),
    ("construction.screen.status.cancelled", "Cancelled"),
    ("construction.screen.status.draft", "Draft"),
    ("construction.screen.status.submitted", "Submitted"),
    ("construction.screen.status.certified", "Certified"),
    ("construction.screen.status.invoiced", "Invoiced"),
    ("construction.screen.status.superseded", "Superseded"),
    ("construction.screen.status.approved", "Approved"),
    ("construction.screen.status.rejected", "Rejected"),
    ("construction.screen.status.claimed", "Claimed"),
    ("construction.screen.status.abandoned", "Abandoned"),
    ("construction.screen.status.withdrawn", "Withdrawn"),
    ("construction.screen.status.undecided", "Undecided"),
    ("construction.screen.status.accepted", "Accepted"),
    ("construction.screen.status.contested", "Contested"),
    ("construction.screen.kind.interim", "Interim payment"),
    ("construction.screen.kind.final_account", "Final account"),
    ("construction.screen.kind.addition", "Addition"),
    ("construction.screen.kind.omission", "Omission"),
    ("construction.screen.kind.time", "Extension of time"),
    ("construction.screen.kind.back_charge", "Back-charge"),
    ("construction.screen.kind.contra_charge", "Contra-charge"),
    (
        "construction.screen.kind.liquidated_damages",
        "Liquidated damages",
    ),
    ("construction.screen.kind.other", "Other"),
    // Why a project was terminated (the words of the termination form).
    (
        "construction.screen.reason.terminated_by_client",
        "Terminated by the client",
    ),
    (
        "construction.screen.reason.our_default",
        "Terminated for our default",
    ),
    (
        "construction.screen.reason.main_contractor_insolvent",
        "Main contractor insolvent",
    ),
    ("construction.screen.reason.agreed", "Stopped by agreement"),
    // The projects list.
    ("construction.screen.projects.title", "Projects"),
    (
        "construction.screen.projects.subtitle",
        "Awarded contracts with their payment applications and reports.",
    ),
    ("construction.screen.projects.code", "Code"),
    ("construction.screen.projects.name", "Project"),
    ("construction.screen.projects.client", "Client"),
    (
        "construction.screen.projects.contract_value",
        "Contract value",
    ),
    ("construction.screen.projects.ipas", "IPAs"),
    ("construction.screen.projects.reports", "Reports"),
    ("construction.screen.projects.waiting", "Needs attention"),
    (
        "construction.screen.projects.waiting_hint",
        "IPAs and variations the client has not answered in {{ days }} days; guarantees ending within {{ expiry }} days.",
    ),
    ("construction.screen.projects.closing", "Closing"),
    ("construction.screen.projects.variations", "Variations"),
    (
        "construction.screen.projects.in_progress",
        "Projects in progress",
    ),
    (
        "construction.screen.projects.contract_total",
        "Contract value in total",
    ),
    (
        "construction.screen.projects.attention",
        "Waiting on the client",
    ),
    ("construction.screen.projects.no_ipas", "No IPAs yet"),
    ("construction.screen.projects.empty", "No projects yet"),
    (
        "construction.screen.projects.empty_text",
        "A project appears here when a contract is awarded.",
    ),
    (
        "construction.screen.projects.ipas_waiting",
        "{{ count }} IPA(s) waiting",
    ),
    (
        "construction.screen.projects.guarantees_expiring",
        "{{ count }} guarantee(s) ending",
    ),
    (
        "construction.screen.projects.variations_waiting",
        "{{ count }} variation(s) unanswered",
    ),
    ("construction.screen.projects.ipa_label", "IPA {{ number }}"),
    // An IPA.
    ("construction.screen.ipa.title", "IPA {{ number }}"),
    (
        "construction.screen.ipa.title_revision",
        "IPA {{ number }} (revision {{ revision }})",
    ),
    ("construction.screen.ipa.back", "Projects"),
    ("construction.screen.ipa.summary", "The application"),
    ("construction.screen.ipa.valuation_on", "Valued to"),
    ("construction.screen.ipa.claimed", "Claimed"),
    ("construction.screen.ipa.certified", "Certified"),
    (
        "construction.screen.ipa.claim_gap",
        "Claimed less certified",
    ),
    (
        "construction.screen.ipa.previous_certified",
        "Certified before this IPA",
    ),
    ("construction.screen.ipa.submitted_on", "Submitted on"),
    ("construction.screen.ipa.certified_on", "Certified on"),
    (
        "construction.screen.ipa.certificate",
        "Client's certificate",
    ),
    (
        "construction.screen.ipa.late_certificate",
        "Certificate of the superseded IPA",
    ),
    ("construction.screen.ipa.decision", "Decision reference"),
    ("construction.screen.ipa.decided", "Amount decided"),
    (
        "construction.screen.ipa.previous_pending",
        "Drafted while an earlier IPA still waits for its certificate.",
    ),
    (
        "construction.screen.ipa.replaces",
        "Replaces IPA {{ number }}",
    ),
    ("construction.screen.ipa.closing", "Project closing"),
    ("construction.screen.ipa.variations", "Variation report"),
    ("construction.screen.ipa.invoice", "Tax invoice"),
    ("construction.screen.ipa.credit_note", "Credit note"),
    ("construction.screen.ipa.dispute", "Dispute"),
    ("construction.screen.ipa.supplier_bill", "Supplier bill"),
    ("construction.screen.ipa.deductions", "Deductions"),
    (
        "construction.screen.ipa.deductions_hint",
        "What the client deducts from the certified work, excluding VAT.",
    ),
    (
        "construction.screen.ipa.no_deductions",
        "No deductions on this IPA.",
    ),
    ("construction.screen.ipa.no_value", "Not yet"),
    // The closing screen.
    ("construction.screen.closing.title", "Closing"),
    ("construction.screen.closing.back", "Projects"),
    ("construction.screen.closing.termination", "Termination"),
    ("construction.screen.closing.reason", "Reason"),
    (
        "construction.screen.closing.effective_on",
        "Last working day",
    ),
    ("construction.screen.closing.final_account", "Final account"),
    (
        "construction.screen.closing.no_final_account",
        "No final account yet.",
    ),
    (
        "construction.screen.closing.open_final",
        "Open the final account",
    ),
    ("construction.screen.closing.valuation_on", "Valued to"),
    ("construction.screen.closing.certified", "Certified work"),
    ("construction.screen.closing.decided", "Decided amount"),
    ("construction.screen.closing.decision", "Decision reference"),
    ("construction.screen.closing.invoice", "Open the invoice"),
    ("construction.screen.closing.warnings", "Still open"),
    (
        "construction.screen.closing.nothing_open",
        "Nothing is open: the project can be settled or closed.",
    ),
    (
        "construction.screen.closing.deductions",
        "Contested deductions",
    ),
    (
        "construction.screen.closing.retention_deductions",
        "Contested deductions from the retention",
    ),
    (
        "construction.screen.closing.calls",
        "Disputed guarantee calls",
    ),
    ("construction.screen.closing.demand", "Demand notice"),
    ("construction.screen.closing.paid", "Paid"),
    (
        "construction.screen.closing.claims",
        "Variation claims in the final account",
    ),
    (
        "construction.screen.closing.unrecovered",
        "Unrecovered variations",
    ),
    ("construction.screen.closing.variation", "Variation"),
    (
        "construction.screen.closing.unrecovered_cost",
        "Unrecovered cost",
    ),
    ("construction.screen.closing.approved", "Approved"),
    ("construction.screen.closing.ipa", "IPA"),
    ("construction.screen.closing.deduction", "Deduction"),
    ("construction.screen.closing.reason_column", "Reason"),
    // The variation report.
    ("construction.screen.variations.title", "Variation report"),
    ("construction.screen.variations.back", "Projects"),
    ("construction.screen.variations.variation", "Variation"),
    ("construction.screen.variations.priced", "Priced"),
    ("construction.screen.variations.approved", "Approved"),
    ("construction.screen.variations.gap", "Not approved"),
    ("construction.screen.variations.days_unit", "days"),
    ("construction.screen.variations.unrecovered", "Unrecovered"),
    ("construction.screen.variations.late", "Notice late"),
    (
        "construction.screen.variations.total_gap",
        "Not approved in total",
    ),
    (
        "construction.screen.variations.total_approved",
        "Approved in total",
    ),
    (
        "construction.screen.variations.total_unrecovered",
        "Unrecovered in total",
    ),
    ("construction.screen.variations.empty", "No variations yet"),
    (
        "construction.screen.variations.empty_text",
        "Variations of this project appear here once they are recorded.",
    ),
    ("construction.screen.variations.closing", "Project closing"),
    (
        "construction.screen.variations.revision",
        "{{ code }} rev. {{ revision }}",
    ),
];
