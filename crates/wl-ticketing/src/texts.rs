//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_ticketing.title", "Tickets"),
    (
        "wl_ticketing.notice.assigned.subject",
        "Ticket {{ number }} is assigned to you",
    ),
    (
        "wl_ticketing.notice.assigned.body",
        "{{ title }} ({{ priority }} priority). Visit: {{ visit }}. Respond by {{ response_due }}; resolve by {{ resolution_due }}.",
    ),
    (
        "wl_ticketing.notice.visit_missed.subject",
        "Planned visit {{ number }} was missed",
    ),
    (
        "wl_ticketing.notice.visit_missed.body",
        "{{ title }} ({{ category }}) was planned for {{ planned_due }} and is still not resolved {{ days }} days later.",
    ),
    (
        "wl_ticketing.notice.withdrawn.subject",
        "Ticket {{ number }} was reassigned",
    ),
    (
        "wl_ticketing.notice.withdrawn.body",
        "{{ title }} is no longer yours; it now goes to {{ assignee }}. Reason: {{ reason }}.",
    ),
    (
        "wl_ticketing.notice.visit_rescheduled.subject",
        "The visit for ticket {{ number }} has moved",
    ),
    (
        "wl_ticketing.notice.visit_rescheduled.body",
        "{{ title }}: the visit is now {{ visit }}.",
    ),
    ("wl_ticketing.notice.not_scheduled", "not scheduled"),
    ("wl_ticketing.empty", "No tickets yet"),
    (
        "wl_ticketing.empty_hint",
        "Tickets raised for units, contracts and requests show here.",
    ),
    ("wl_ticketing.new", "New ticket"),
    ("wl_ticketing.create", "Create ticket"),
    ("wl_ticketing.cancel", "Cancel"),
    ("wl_ticketing.created", "Ticket created."),
    ("wl_ticketing.none", "None"),
    ("wl_ticketing.fields.number", "Number"),
    ("wl_ticketing.fields.title", "Title"),
    ("wl_ticketing.fields.description", "Description"),
    ("wl_ticketing.fields.subject", "About"),
    ("wl_ticketing.fields.subject_type", "Subject type"),
    (
        "wl_ticketing.fields.subject_type_hint",
        "For example property.unit.",
    ),
    ("wl_ticketing.fields.subject_id", "Subject id"),
    ("wl_ticketing.fields.category", "Category"),
    ("wl_ticketing.fields.priority", "Priority"),
    ("wl_ticketing.fields.status", "Status"),
    ("wl_ticketing.fields.assignee", "Assignee"),
    ("wl_ticketing.fields.requester", "Requester"),
    ("wl_ticketing.fields.requester_code", "Requester party code"),
    (
        "wl_ticketing.fields.requester_code_hint",
        "For example P-00001. Leave empty for an internal task.",
    ),
    ("wl_ticketing.fields.channel", "Channel"),
    ("wl_ticketing.fields.visit_at", "Visit"),
    ("wl_ticketing.fields.due_at", "Due"),
    ("wl_ticketing.fields.created_at", "Created"),
    (
        "wl_ticketing.fields.sla_started_at",
        "Service level started",
    ),
    ("wl_ticketing.fields.response_due", "Response due"),
    ("wl_ticketing.fields.resolution_due", "Resolution due"),
    ("wl_ticketing.fields.responded_at", "Responded"),
    ("wl_ticketing.fields.resolved_at", "Resolved"),
    ("wl_ticketing.fields.hold_reason", "On hold for"),
    ("wl_ticketing.fields.cancel_reason", "Cancelled because"),
    ("wl_ticketing.fields.confidential", "Confidential"),
    ("wl_ticketing.status.open", "Open"),
    ("wl_ticketing.status.assigned", "Assigned"),
    ("wl_ticketing.status.in_progress", "In progress"),
    ("wl_ticketing.status.on_hold", "On hold"),
    ("wl_ticketing.status.resolved", "Resolved"),
    ("wl_ticketing.status.closed", "Closed"),
    ("wl_ticketing.status.cancelled", "Cancelled"),
    ("wl_ticketing.priority.urgent", "Urgent"),
    ("wl_ticketing.priority.high", "High"),
    ("wl_ticketing.priority.normal", "Normal"),
    ("wl_ticketing.priority.low", "Low"),
    ("wl_ticketing.channel.portal", "Portal"),
    ("wl_ticketing.channel.staff", "Staff"),
    ("wl_ticketing.channel.email", "E-mail"),
    ("wl_ticketing.channel.phone", "Phone"),
    ("wl_ticketing.channel.system", "System"),
    ("wl_ticketing.sections.details", "Ticket"),
    ("wl_ticketing.sections.sla", "Service level"),
    ("wl_ticketing.sections.assignments", "Reassignment history"),
    ("wl_ticketing.sections.history", "History"),
    ("wl_ticketing.sections.comments", "Comments"),
    ("wl_ticketing.no_comments", "No comments yet"),
    ("wl_ticketing.comments.author", "From"),
    ("wl_ticketing.comments.text", "Comment"),
    ("wl_ticketing.comments.visibility", "Seen by"),
    ("wl_ticketing.comments.system", "System"),
    ("wl_ticketing.comments.internal", "Staff only"),
    ("wl_ticketing.comments.requester", "Staff and requester"),
    ("wl_ticketing.no_assignments", "Not assigned yet"),
    ("wl_ticketing.no_history", "No history yet"),
    ("wl_ticketing.history.change", "Change"),
    ("wl_ticketing.history.at", "When"),
    ("wl_ticketing.history.reason", "Reason"),
    ("wl_ticketing.history.by", "By"),
    ("wl_ticketing.history.visit", "Visit"),
    ("wl_ticketing.assignment.first", "Assigned to {{ to }}"),
    ("wl_ticketing.assignment.reassign", "{{ from }} to {{ to }}"),
    (
        "wl_ticketing.assignment.visit_change",
        "Visit of {{ to }} rescheduled",
    ),
    ("wl_ticketing.assign.action", "Assign"),
    ("wl_ticketing.assign.reassign_action", "Reassign"),
    ("wl_ticketing.assign.title", "Assign ticket"),
    ("wl_ticketing.assign.reassign_title", "Reassign ticket"),
    (
        "wl_ticketing.assign.description",
        "Choose a staff member or a vendor. The service level starts with the first assignment and is never restarted.",
    ),
    ("wl_ticketing.assign.submit", "Assign"),
    ("wl_ticketing.assign.user", "Staff member"),
    ("wl_ticketing.assign.party_code", "Vendor party code"),
    (
        "wl_ticketing.assign.party_code_hint",
        "For example P-00007. Use this or a staff member.",
    ),
    ("wl_ticketing.assign.reason_code", "Reason for reassigning"),
    ("wl_ticketing.assign.reason_text", "Explanation"),
    ("wl_ticketing.assign.done", "Ticket assigned."),
    ("wl_ticketing.assign.choose", "Choose who gets the ticket."),
    (
        "wl_ticketing.assign.both",
        "Choose a staff member or a vendor, not both.",
    ),
    (
        "wl_ticketing.assign.unknown_party",
        "No party has this code.",
    ),
    ("wl_ticketing.hold.action", "Put on hold"),
    ("wl_ticketing.hold.title", "Put ticket on hold"),
    (
        "wl_ticketing.hold.description",
        "Some reasons pause the service level clock.",
    ),
    ("wl_ticketing.hold.reason", "Hold reason"),
    ("wl_ticketing.hold.submit", "Put on hold"),
    ("wl_ticketing.hold.done", "Ticket put on hold."),
    ("wl_ticketing.cancel_ticket.action", "Cancel ticket"),
    ("wl_ticketing.cancel_ticket.title", "Cancel ticket"),
    (
        "wl_ticketing.cancel_ticket.description",
        "The ticket will not be worked. The reason is kept in its history.",
    ),
    ("wl_ticketing.cancel_ticket.reason", "Reason"),
    ("wl_ticketing.cancel_ticket.submit", "Cancel ticket"),
    ("wl_ticketing.cancel_ticket.done", "Ticket cancelled."),
    (
        "wl_ticketing.cancel_ticket.required",
        "Give a reason for cancelling.",
    ),
    ("wl_ticketing.action.start", "Start work"),
    ("wl_ticketing.action.resume", "Resume"),
    ("wl_ticketing.action.resolve", "Mark resolved"),
    ("wl_ticketing.action.confirm", "Confirm the work"),
    ("wl_ticketing.action.close", "Close"),
    ("wl_ticketing.action.reopen", "Reopen"),
    ("wl_ticketing.action.confirm_title", "Are you sure?"),
    ("wl_ticketing.action.done", "Done."),
    (
        "wl_ticketing.charge.not_chargeable",
        "This ticket is not chargeable.",
    ),
    (
        "wl_ticketing.charge.no_description",
        "A charge line needs a description.",
    ),
    (
        "wl_ticketing.charge.bad_quantity",
        "A charge line needs a quantity above 0.",
    ),
    (
        "wl_ticketing.charge.bad_price",
        "A charge line cannot have a negative price.",
    ),
    (
        "wl_ticketing.charge.no_party",
        "No invoice draft was made: the ticket has no party to bill.",
    ),
    (
        "wl_ticketing.charge.amount_shown",
        "The charge for this work is :amount including VAT. You will be invoiced when you confirm it.",
    ),
    (
        "wl_ticketing.charge.needed",
        "Add the charge before resolving.",
    ),
    ("wl_ticketing.sections.charges", "Charge"),
    ("wl_ticketing.charge.description", "Description"),
    ("wl_ticketing.charge.quantity", "Quantity"),
    ("wl_ticketing.charge.quantity_hint", "1 when left empty."),
    ("wl_ticketing.charge.unit_price", "Unit price (AED)"),
    (
        "wl_ticketing.charge.unit_price_hint",
        "Before VAT, e.g. 333.33.",
    ),
    ("wl_ticketing.charge.vat", "VAT"),
    ("wl_ticketing.charge.total", "With VAT"),
    ("wl_ticketing.charge.amount", "Total with VAT"),
    ("wl_ticketing.charge.draft", "Invoice draft"),
    ("wl_ticketing.charge.add", "Add charge"),
    ("wl_ticketing.charge.add_title", "Add a charge line"),
    (
        "wl_ticketing.charge.add_description",
        "The lines are the amount the requester is shown and what the invoice carries.",
    ),
    ("wl_ticketing.charge.remove", "Remove"),
    ("wl_ticketing.charge.remove_title", "Remove this charge line?"),
    ("wl_ticketing.charge.empty", "No charge yet"),
    (
        "wl_ticketing.charge.empty_hint",
        "Add the charge before resolving this ticket.",
    ),
    ("wl_ticketing.charge.added", "Charge added."),
    ("wl_ticketing.charge.removed", "Charge removed."),
    (
        "wl_ticketing.charge.bad_price_text",
        "Enter the price in AED, like 333.33.",
    ),
    (
        "wl_ticketing.charge.bad_quantity_text",
        "Enter a quantity, like 1 or 2.5.",
    ),
    ("wl_ticketing.vat.standard", "Standard rated 5%"),
    ("wl_ticketing.vat.zero_rated", "Zero rated"),
    ("wl_ticketing.vat.exempt", "Exempt"),
    ("wl_ticketing.vat.out_of_scope", "Out of scope"),
    ("wl_ticketing.vat.reverse_charge", "Reverse charge"),
    ("wl_ticketing.categories.compliance", "Compliance"),
    ("wl_ticketing.categories.critical", "Compliance-critical"),
    ("wl_ticketing.categories.not_critical", "Not compliance-critical"),
    ("wl_ticketing.categories.compliance_change", "Change compliance"),
    (
        "wl_ticketing.categories.compliance_title",
        "Compliance-critical category",
    ),
    (
        "wl_ticketing.categories.compliance_description",
        "A covered subject that is not compliant for it gets a compliance flag, and a missed planned visit also tells the manager.",
    ),
    ("wl_ticketing.flags.title", "Compliance flags"),
    (
        "wl_ticketing.flags.subtitle",
        "Covered subjects that are not compliant for a compliance-critical category.",
    ),
    ("wl_ticketing.flags.subject", "Subject"),
    ("wl_ticketing.flags.category", "Category"),
    ("wl_ticketing.flags.reason", "Reason"),
    ("wl_ticketing.flags.raised", "Raised"),
    ("wl_ticketing.flags.age", "Open for"),
    ("wl_ticketing.flags.days", ":count days"),
    ("wl_ticketing.flags.visit", "Visit"),
    ("wl_ticketing.flags.contract", "Contract"),
    ("wl_ticketing.flags.empty", "No open compliance flags"),
    (
        "wl_ticketing.flags.empty_hint",
        "Every covered subject is compliant.",
    ),
    ("wl_ticketing.flags.reason_no_contract", "No contract"),
    ("wl_ticketing.flags.reason_visit_missed", "Planned visit missed"),
    (
        "wl_ticketing.flags.reason_vendor_document_expired",
        "Vendor document expired",
    ),
    ("wl_ticketing.categories.charging", "Charging"),
    ("wl_ticketing.categories.chargeable", "Chargeable"),
    ("wl_ticketing.categories.not_chargeable", "Not chargeable"),
    ("wl_ticketing.categories.invoice_on", "Invoice"),
    ("wl_ticketing.categories.on_resolve", "At resolve"),
    ("wl_ticketing.categories.on_close", "At close"),
    (
        "wl_ticketing.categories.charging_resolve",
        "Chargeable, invoiced at resolve",
    ),
    (
        "wl_ticketing.categories.charging_close",
        "Chargeable, invoiced at close",
    ),
    ("wl_ticketing.categories.charging_change", "Change charging"),
    (
        "wl_ticketing.categories.charging_title",
        "Charging for this category",
    ),
    (
        "wl_ticketing.categories.charging_description",
        "Applies to tickets opened from now on. At close means when the requester confirms, staff close it, or three days pass.",
    ),
    ("wl_ticketing.categories.title", "Ticket categories"),
    (
        "wl_ticketing.categories.subtitle",
        "Rename a category, turn it off or delete it. The categories other modules open tickets in can only be renamed.",
    ),
    ("wl_ticketing.categories.name", "Name"),
    ("wl_ticketing.categories.name_ar", "Arabic name"),
    ("wl_ticketing.categories.key", "Key"),
    ("wl_ticketing.categories.assignee_role", "Assignee role"),
    ("wl_ticketing.categories.platform", "Platform"),
    (
        "wl_ticketing.categories.platform_hint",
        "Other modules open tickets in it: it cannot be turned off or deleted.",
    ),
    ("wl_ticketing.categories.rename", "Rename"),
    ("wl_ticketing.categories.rename_title", "Rename the category"),
    ("wl_ticketing.categories.status", "Status"),
    ("wl_ticketing.categories.on", "On"),
    ("wl_ticketing.categories.off", "Off"),
    ("wl_ticketing.categories.status_change", "Change status"),
    (
        "wl_ticketing.categories.status_title",
        "Turn the category on or off",
    ),
    ("wl_ticketing.categories.delete", "Delete"),
    ("wl_ticketing.categories.delete_title", "Delete this category?"),
    (
        "wl_ticketing.categories.delete_message",
        "A category with tickets cannot be deleted.",
    ),
    ("wl_ticketing.categories.empty", "No categories yet"),
    ("wl_ticketing.categories.renamed", "Category renamed."),
    ("wl_ticketing.categories.updated", "Category updated."),
    ("wl_ticketing.categories.deleted", "Category deleted."),
];

/// The English text for `key`, or `key` itself when the package has none.
/// Notification templates are registered with it: the client changes them on
/// the stored notification type, not in the language files.
pub(crate) fn english(key: &str) -> &str {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| text)
}

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key).to_owned()
}

/// Every text of the package in `lang`, keyed without the `wl_ticketing.`
/// prefix (`fields.number`), for the templates: `tx['fields.number']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_ticketing."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TEXTS;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_ticketing.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }
}
