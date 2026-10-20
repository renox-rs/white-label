//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_sales.title", "Campaigns"),
    ("wl_sales.cancel", "Cancel"),
    ("wl_sales.lead_sources.title", "Lead sources"),
    (
        "wl_sales.lead_sources.subtitle",
        "Where campaigns and leads come from. A source in use can be switched off but not deleted.",
    ),
    ("wl_sales.lead_sources.empty", "No lead sources"),
    (
        "wl_sales.lead_sources.empty_hint",
        "Run the sales installer to store the defaults.",
    ),
    ("wl_sales.lead_sources.add", "Add a lead source"),
    ("wl_sales.lead_sources.add_title", "Add a lead source"),
    (
        "wl_sales.lead_sources.add_description",
        "The code cannot be changed later.",
    ),
    ("wl_sales.lead_sources.added", "Lead source added."),
    ("wl_sales.lead_sources.rename", "Rename"),
    ("wl_sales.lead_sources.rename_title", "Rename :code"),
    ("wl_sales.lead_sources.renamed", "Lead source renamed."),
    ("wl_sales.lead_sources.activate", "Activate"),
    ("wl_sales.lead_sources.deactivate", "Deactivate"),
    (
        "wl_sales.lead_sources.deactivate_title",
        "Deactivate :code?",
    ),
    (
        "wl_sales.lead_sources.deactivate_message",
        "It can no longer be chosen for new campaigns. Existing ones keep it.",
    ),
    ("wl_sales.lead_sources.activated", "Lead source activated."),
    (
        "wl_sales.lead_sources.deactivated",
        "Lead source deactivated.",
    ),
    ("wl_sales.lead_sources.in_use", "In use"),
    (
        "wl_sales.lead_sources.bad_code",
        "Use 2 to 30 lower case letters, digits or underscores.",
    ),
    (
        "wl_sales.lead_sources.code_taken",
        "A lead source has this code.",
    ),
    ("wl_sales.lead_sources.name_required", "Give the name."),
    (
        "wl_sales.lead_sources.in_use_refused",
        "This lead source is in use; deactivate it instead.",
    ),
    (
        "wl_sales.campaigns.name_required",
        "Give the campaign a name.",
    ),
    (
        "wl_sales.campaigns.bad_code",
        "Use 2 to 20 letters, digits, hyphens or underscores for the code.",
    ),
    (
        "wl_sales.campaigns.code_taken",
        "A campaign has or had this code.",
    ),
    (
        "wl_sales.campaigns.bad_promo_code",
        "Use 2 to 20 letters, digits, hyphens or underscores for the promo code.",
    ),
    (
        "wl_sales.campaigns.promo_code_taken",
        "Another campaign has this promo code.",
    ),
    (
        "wl_sales.campaigns.end_before_start",
        "The end date cannot be before the start date.",
    ),
    (
        "wl_sales.campaigns.not_draft",
        "Only a draft campaign can be deleted.",
    ),
    ("wl_sales.campaigns.not_found", "There is no such campaign."),
    (
        "wl_sales.notices.ended_subject",
        "Campaign {{ code }} has ended",
    ),
    (
        "wl_sales.notices.ended_body",
        "The campaign {{ name }} ({{ code }}) ended; its last day was {{ end_date }}.",
    ),
    (
        "wl_sales.notices.budget_alert_subject",
        "Campaign {{ code }} reached {{ percent }}% of its budget",
    ),
    (
        "wl_sales.notices.budget_alert_body",
        "The approved costs of {{ name }} ({{ code }}) are AED {{ approved }} of AED {{ budget }}, {{ percent }}% of the budget.",
    ),
    (
        "wl_sales.notices.cost_rejected_subject",
        "A cost of campaign {{ code }} was rejected",
    ),
    (
        "wl_sales.notices.cost_rejected_body",
        "The cost of AED {{ amount }} ({{ description }}) on {{ name }} ({{ code }}) was rejected: {{ reason }}",
    ),
    (
        "wl_sales.notices.cost_bill_voided_subject",
        "A cost of campaign {{ code }} was voided",
    ),
    (
        "wl_sales.notices.cost_bill_voided_body",
        "The cost of AED {{ amount }} ({{ description }}) on {{ name }} ({{ code }}) was voided because its supplier bill was voided.",
    ),
    (
        "wl_sales.notices.lead_win_reverted_subject",
        "Lead {{ name }} is qualified again",
    ),
    (
        "wl_sales.notices.lead_win_reverted_body",
        "The lead {{ name }} was won with {{ voided }}, which was voided as entered in error. The lead is qualified again.",
    ),
    ("wl_sales.leads.source_required", "Pick the lead source."),
    (
        "wl_sales.leads.unknown_source",
        "There is no such lead source, or it is switched off.",
    ),
    (
        "wl_sales.leads.campaign_required",
        "This source needs a campaign.",
    ),
    (
        "wl_sales.leads.web_form_note",
        "Another web form enquiry arrived on campaign :code.",
    ),
    ("wl_sales.enquiry.name_required", "Tell us your name."),
    ("wl_sales.enquiry.thanks", "Thank you, we will be in touch."),
    ("wl_sales.leads.problem.note", "Record the contact note."),
    (
        "wl_sales.leads.problem.converted",
        "A lead is won with the conversion record.",
    ),
    (
        "wl_sales.leads.problem.lost_reason",
        "Give the reason the lead is lost.",
    ),
    (
        "wl_sales.acquisitions.source_required",
        "A new customer needs a lead source before the sale completes.",
    ),
    (
        "wl_sales.acquisitions.unknown_source",
        "There is no such lead source, or it is switched off.",
    ),
    (
        "wl_sales.acquisitions.unknown_campaign",
        "There is no such campaign.",
    ),
    (
        "wl_sales.acquisitions.campaign_required",
        "This source needs a campaign.",
    ),
    (
        "wl_sales.acquisitions.campaign_in_use",
        "Leads or customers are attributed to this campaign.",
    ),
    ("wl_sales.acquisitions.title", "Acquisition exceptions"),
    (
        "wl_sales.acquisitions.subtitle",
        "Customers whose first sale came with no source, and the changes of source or campaign.",
    ),
    (
        "wl_sales.acquisitions.missing.title",
        "Sales without a source",
    ),
    (
        "wl_sales.acquisitions.missing.empty",
        "Every sale has a source",
    ),
    ("wl_sales.acquisitions.party", "Customer"),
    ("wl_sales.acquisitions.first_sale", "First sale"),
    ("wl_sales.acquisitions.first_sale_date", "Date"),
    ("wl_sales.acquisitions.value", "Value excl. VAT"),
    ("wl_sales.changes.title", "Changes of source or campaign"),
    ("wl_sales.changes.empty", "No changes yet"),
    ("wl_sales.changes.kind", "Of"),
    ("wl_sales.changes.kind.acquisition", "Customer"),
    ("wl_sales.changes.kind.lead", "Lead"),
    ("wl_sales.changes.what", "Change"),
    ("wl_sales.changes.reason", "Reason"),
    ("wl_sales.changes.source", "Source"),
    ("wl_sales.changes.campaign", "Campaign code"),
    ("wl_sales.changes.requested_by", "Asked by"),
    ("wl_sales.changes.decided_by", "Approved by"),
    ("wl_sales.changes.request", "Change source"),
    (
        "wl_sales.changes.request_title",
        "Change the source or campaign",
    ),
    ("wl_sales.changes.approve", "Approve"),
    ("wl_sales.changes.reject", "Reject"),
    (
        "wl_sales.changes.status.pending",
        "Waiting for a second person",
    ),
    ("wl_sales.changes.status.applied", "Applied"),
    ("wl_sales.changes.status.rejected", "Rejected"),
    (
        "wl_sales.changes.requested",
        "The change waits for another manager to approve it.",
    ),
    ("wl_sales.changes.applied", "The change is applied."),
    ("wl_sales.changes.approved", "Change approved."),
    ("wl_sales.changes.rejected", "Change rejected."),
    ("wl_sales.changes.reason_required", "Give the reason."),
    ("wl_sales.changes.not_found", "There is no such change."),
    (
        "wl_sales.changes.unknown_source",
        "There is no such lead source, or it is switched off.",
    ),
    (
        "wl_sales.changes.unknown_campaign",
        "There is no such campaign.",
    ),
    (
        "wl_sales.changes.campaign_required",
        "This source needs a campaign.",
    ),
    (
        "wl_sales.changes.unchanged",
        "The source and campaign are already these.",
    ),
    (
        "wl_sales.changes.same_person",
        "The one who asked for a change cannot approve it.",
    ),
    (
        "wl_sales.changes.not_pending",
        "The change was already decided.",
    ),
    ("wl_sales.changes.no_decider", "Say who decides."),
    ("wl_sales.report.title", "Acquisition report"),
    (
        "wl_sales.report.subtitle",
        "Leads, conversion, cost per lead, customer acquisition cost and return on ad spend. Amounts are AED excluding VAT; \"—\" means undefined.",
    ),
    ("wl_sales.report.csv", "Export CSV"),
    ("wl_sales.report.show", "Show"),
    ("wl_sales.report.all", "All"),
    ("wl_sales.report.total", "Total"),
    (
        "wl_sales.report.empty",
        "Nothing to report for these filters",
    ),
    ("wl_sales.report.group", "Group by"),
    ("wl_sales.report.source", "Source"),
    ("wl_sales.report.campaign", "Campaign code"),
    ("wl_sales.report.channel", "Marketing channel"),
    ("wl_sales.report.brand", "Brand"),
    ("wl_sales.report.branch", "Branch"),
    ("wl_sales.report.branch_hint", "As type:id"),
    ("wl_sales.report.entity", "Legal entity"),
    ("wl_sales.report.month", "Month"),
    ("wl_sales.report.by.source", "Source"),
    ("wl_sales.report.by.campaign", "Campaign"),
    ("wl_sales.report.by.channel", "Marketing channel"),
    ("wl_sales.report.by.brand", "Brand"),
    ("wl_sales.report.by.branch", "Branch"),
    ("wl_sales.report.by.month", "Month"),
    ("wl_sales.report.leads", "Leads"),
    ("wl_sales.report.won", "Won"),
    ("wl_sales.report.conversion", "Conversion"),
    ("wl_sales.report.customers", "New customers"),
    ("wl_sales.report.value", "Converted value"),
    ("wl_sales.report.cost", "Cost"),
    ("wl_sales.report.cost_linked", "Linked to bills"),
    ("wl_sales.report.cost_manual", "Manual"),
    ("wl_sales.report.cost_per_lead", "Cost per lead"),
    ("wl_sales.report.cac", "CAC"),
    ("wl_sales.report.roas", "ROAS"),
    ("wl_sales.report.none.source", "No source"),
    ("wl_sales.report.none.campaign", "No campaign"),
    ("wl_sales.report.none.channel", "No channel"),
    ("wl_sales.report.none.brand", "No brand"),
    ("wl_sales.report.none.branch", "No branch"),
    ("wl_sales.report.none.month", "No month"),
    (
        "wl_sales.costs.campaign_not_found",
        "There is no such campaign.",
    ),
    ("wl_sales.costs.not_found", "There is no such cost entry."),
    ("wl_sales.costs.no_amount", "The amount cannot be zero."),
    (
        "wl_sales.costs.negative_needs_manager",
        "A negative cost needs the permission to manage campaign costs.",
    ),
    (
        "wl_sales.costs.not_requester",
        "Only the one who entered the cost can change it.",
    ),
    (
        "wl_sales.costs.approved",
        "An approved cost cannot be edited.",
    ),
    (
        "wl_sales.costs.not_editable",
        "This cost entry cannot be edited in its state.",
    ),
    (
        "wl_sales.costs.in_use",
        "This campaign has cost entries; it cannot be deleted",
    ),
    (
        "wl_sales.costs.over_budget",
        "The cost takes the approved costs over the budget.",
    ),
    (
        "wl_sales.costs.within_budget",
        "The cost is within the budget.",
    ),
    ("wl_sales.costs.no_check", "The budget check is missing."),
    (
        "wl_sales.costs.no_policy",
        "No approval policy is set for a cost over the budget.",
    ),
    (
        "wl_sales.costs.line_not_found",
        "There is no such supplier bill line.",
    ),
    (
        "wl_sales.costs.bill_voided",
        "The supplier bill is voided; it cannot be linked.",
    ),
    (
        "wl_sales.costs.not_positive",
        "A cost linked to a supplier bill line must be above zero.",
    ),
    (
        "wl_sales.costs.line_exceeded",
        "The costs linked to this supplier bill line would exceed the line.",
    ),
    (
        "wl_sales.costs.not_linked",
        "The cost is not linked to a supplier bill line.",
    ),
    ("wl_sales.costs.bill_voided_reason", "supplier bill voided"),
    ("wl_sales.campaigns.title", "Campaigns"),
    (
        "wl_sales.campaigns.subtitle",
        "Marketing campaigns with a budget and their cost entries.",
    ),
    ("wl_sales.campaigns.empty", "No campaigns yet"),
    ("wl_sales.campaigns.none", "Not set"),
    ("wl_sales.campaigns.no_budget", "No budget"),
    ("wl_sales.campaigns.sections.basics", "Campaign"),
    (
        "wl_sales.campaigns.sections.period",
        "Period, budget and promo code",
    ),
    (
        "wl_sales.campaigns.sections.retention",
        "Retention message (retention campaigns only)",
    ),
    ("wl_sales.campaigns.sections.details", "Details"),
    ("wl_sales.campaigns.sections.costs", "Costs"),
    ("wl_sales.campaigns.sections.cost_entries", "Cost entries"),
    ("wl_sales.campaigns.sections.history", "History"),
    ("wl_sales.campaigns.fields.code", "Code"),
    ("wl_sales.campaigns.fields.name", "Name"),
    ("wl_sales.campaigns.fields.purpose", "Purpose"),
    ("wl_sales.campaigns.fields.channel", "Marketing channel"),
    ("wl_sales.campaigns.fields.lead_source", "Lead source"),
    ("wl_sales.campaigns.fields.brand", "Brand"),
    ("wl_sales.campaigns.fields.owner", "Owner"),
    ("wl_sales.campaigns.fields.entity", "Legal entity"),
    ("wl_sales.campaigns.fields.scope_type", "Branch type"),
    ("wl_sales.campaigns.fields.scope_id", "Branch id"),
    ("wl_sales.campaigns.fields.start_date", "Start date"),
    ("wl_sales.campaigns.fields.end_date", "End date"),
    ("wl_sales.campaigns.fields.budget", "Budget"),
    ("wl_sales.campaigns.fields.budget_aed", "Budget (AED)"),
    (
        "wl_sales.campaigns.fields.approved_aed",
        "Approved cost (AED)",
    ),
    ("wl_sales.campaigns.fields.used_percent", "Budget used (%)"),
    ("wl_sales.campaigns.fields.promo_code", "Promo code"),
    (
        "wl_sales.campaigns.fields.promo_valid_to",
        "Promo code valid to",
    ),
    ("wl_sales.campaigns.fields.promo_uses", "Promo code uses"),
    ("wl_sales.campaigns.fields.segment", "Segment"),
    (
        "wl_sales.campaigns.fields.template",
        "Notification template",
    ),
    (
        "wl_sales.campaigns.fields.message_channel",
        "Message channel",
    ),
    ("wl_sales.campaigns.fields.send_at", "Send at"),
    ("wl_sales.campaigns.fields.status", "Status"),
    ("wl_sales.campaigns.status.draft", "Draft"),
    ("wl_sales.campaigns.status.active", "Active"),
    ("wl_sales.campaigns.status.ended", "Ended"),
    ("wl_sales.campaigns.purpose.acquisition", "Acquisition"),
    ("wl_sales.campaigns.purpose.retention", "Retention"),
    ("wl_sales.campaigns.channel.google", "Google"),
    ("wl_sales.campaigns.channel.meta", "Meta"),
    ("wl_sales.campaigns.channel.tiktok", "TikTok"),
    ("wl_sales.campaigns.channel.other_online", "Other online"),
    ("wl_sales.campaigns.channel.print", "Print"),
    (
        "wl_sales.campaigns.channel.radio_outdoor",
        "Radio or outdoor",
    ),
    ("wl_sales.campaigns.channel.event", "Event"),
    ("wl_sales.campaigns.channel.email", "E-mail"),
    ("wl_sales.campaigns.channel.sms", "SMS"),
    ("wl_sales.campaigns.channel.whatsapp", "WhatsApp"),
    ("wl_sales.campaigns.channel.partner", "Partner"),
    ("wl_sales.campaigns.channel.other", "Other"),
    ("wl_sales.campaigns.cost_status.draft", "Draft"),
    (
        "wl_sales.campaigns.cost_status.awaiting_bill",
        "Awaiting bill",
    ),
    (
        "wl_sales.campaigns.cost_status.pending_approval",
        "Pending approval",
    ),
    ("wl_sales.campaigns.cost_status.approved", "Approved"),
    ("wl_sales.campaigns.cost_status.rejected", "Rejected"),
    ("wl_sales.campaigns.cost_status.voided", "Voided"),
    ("wl_sales.campaigns.form.new_title", "New campaign"),
    ("wl_sales.campaigns.form.edit_title", "Change campaign"),
    ("wl_sales.campaigns.form.edit", "Change"),
    ("wl_sales.campaigns.form.save", "Save"),
    ("wl_sales.campaigns.form.cancel", "Cancel"),
    ("wl_sales.campaigns.form.saved", "Campaign saved."),
    (
        "wl_sales.campaigns.form.code_hint",
        "2 to 20 letters, digits, hyphens or underscores; stored in upper case.",
    ),
    ("wl_sales.campaigns.form.choose", "Choose"),
    ("wl_sales.campaigns.form.none", "None"),
    (
        "wl_sales.campaigns.form.default_entity",
        "The client's default entity",
    ),
    (
        "wl_sales.campaigns.form.scope_hint",
        "Leave both branch fields empty for a global campaign.",
    ),
    (
        "wl_sales.campaigns.form.budget_hint",
        "In AED, e.g. 5000.00. Empty means no budget check.",
    ),
    (
        "wl_sales.campaigns.form.send_at_hint",
        "Asia/Dubai time; between the start and end date.",
    ),
    ("wl_sales.campaigns.errors.number", "Give a whole number."),
    ("wl_sales.campaigns.errors.date", "Give a date."),
    (
        "wl_sales.campaigns.errors.amount",
        "Give an amount in AED, e.g. 400.00.",
    ),
    (
        "wl_sales.campaigns.errors.choice",
        "Choose one of the options.",
    ),
    (
        "wl_sales.campaigns.errors.scope",
        "Give both the branch type and the branch id, or neither.",
    ),
    ("wl_sales.campaigns.delete.action", "Delete"),
    (
        "wl_sales.campaigns.delete.confirm_title",
        "Delete this campaign?",
    ),
    (
        "wl_sales.campaigns.delete.confirm_message",
        "Only a draft with no cost entries can be deleted. Its code cannot be used again.",
    ),
    ("wl_sales.campaigns.delete.done", "Campaign deleted."),
    ("wl_sales.campaigns.reopen.action", "Reopen"),
    ("wl_sales.campaigns.reopen.title", "Reopen the campaign"),
    (
        "wl_sales.campaigns.reopen.description",
        "Give the new end date; it must be after today.",
    ),
    ("wl_sales.campaigns.reopen.done", "Campaign reopened."),
    ("wl_sales.campaigns.sums.approved", "Approved costs"),
    (
        "wl_sales.campaigns.sums.bill_linked",
        "Of them, linked to bills",
    ),
    ("wl_sales.campaigns.sums.manual", "Of them, entered by hand"),
    ("wl_sales.campaigns.sums.pending", "Pending approval"),
    ("wl_sales.campaigns.sums.awaiting_bill", "Awaiting bill"),
    (
        "wl_sales.campaigns.sums.not_counted",
        "Only approved costs count. Pending and awaiting-bill amounts are not counted.",
    ),
    ("wl_sales.campaigns.cost.add", "Add cost"),
    ("wl_sales.campaigns.cost.add_title", "Add a cost"),
    (
        "wl_sales.campaigns.cost.add_description",
        "Enter the cost by hand, or give a supplier bill line to link it.",
    ),
    ("wl_sales.campaigns.cost.date", "Date"),
    ("wl_sales.campaigns.cost.amount", "Amount (AED)"),
    (
        "wl_sales.campaigns.cost.amount_hint",
        "Empty with a bill line means what is left of the line.",
    ),
    ("wl_sales.campaigns.cost.description", "Description"),
    ("wl_sales.campaigns.cost.bill_line", "Supplier bill line"),
    (
        "wl_sales.campaigns.cost.bill_line_hint",
        "The line's id. Leave empty for a manual entry.",
    ),
    ("wl_sales.campaigns.cost.bill", "Bill / line"),
    ("wl_sales.campaigns.cost.requested_by", "Entered by"),
    ("wl_sales.campaigns.cost.approved_by", "Approved by"),
    ("wl_sales.campaigns.cost.reason", "Reason"),
    ("wl_sales.campaigns.cost.empty", "No cost entries"),
    ("wl_sales.campaigns.cost.added", "Cost added."),
    (
        "wl_sales.campaigns.cost.pending",
        "The cost is over the budget and waits for approval.",
    ),
    (
        "wl_sales.campaigns.cost.awaiting_bill",
        "The cost waits for its supplier bill to be approved.",
    ),
    ("wl_sales.campaigns.history.change", "Change"),
    ("wl_sales.campaigns.history.at", "When"),
    ("wl_sales.campaigns.history.reason", "Reason"),
    ("wl_sales.campaigns.history.empty", "No history yet"),
    ("wl_sales.fields.code", "Code"),
    ("wl_sales.fields.name_en", "Name (English)"),
    ("wl_sales.fields.name_ar", "Name (Arabic)"),
    ("wl_sales.fields.active", "Active"),
    ("wl_sales.fields.status", "Status"),
    ("wl_sales.status.active", "Active"),
    ("wl_sales.status.inactive", "Inactive"),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], replacing `:name` placeholders with `params`.
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    english(key, params)
}

/// Every text of the package in `lang`, keyed without the `wl_sales.`
/// prefix (`fields.code`), for the templates: `tx['fields.code']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_sales."), t(lang, key)))
        .collect()
}

/// The English text for `key` with `params` filled in; the key itself when
/// it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &dyn Display)]) -> String {
    let text = TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
    renox::i18n::format(text, &params, None)
}

#[cfg(test)]
mod tests {
    use super::{TEXTS, english};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_sales.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(
            english(
                "wl_sales.lead_sources.rename_title",
                &[("code", &"walk_in")]
            ),
            "Rename walk_in"
        );
    }
}
