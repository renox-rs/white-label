//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_sales.title", "Campaigns"),
    ("wl_sales.cancel", "Cancel"),
    ("wl_sales.orders.created_at", "Entered"),
    (
        "wl_sales.orders.empty_hint",
        "Enter an order to see it here.",
    ),
    ("wl_sales.returns.created_at", "Requested"),
    (
        "wl_sales.returns.empty_hint",
        "Request a return to see it here.",
    ),
    ("wl_sales.counter.customer_name", "Customer"),
    ("wl_sales.counter.completed_at", "Completed"),
    ("wl_sales.counter.created_at", "Opened"),
    (
        "wl_sales.counter.empty_hint",
        "Open a sale to ring up the first one.",
    ),
    ("wl_sales.till.float_label", "Opening float"),
    (
        "wl_sales.till.empty_hint",
        "Open a till to start the first session.",
    ),
    (
        "wl_sales.loyalty.no_accounts_hint",
        "Enrol a member to see them here.",
    ),
    (
        "wl_sales.loyalty.review.empty_hint",
        "Accounts over the daily limit appear here.",
    ),
    ("wl_sales.adjust.created_at", "Requested"),
    (
        "wl_sales.adjust.empty_hint",
        "Adjust a member's points from the members list.",
    ),
    ("wl_sales.lead_sources.created_at", "Added"),
    ("wl_sales.loyalty.review.created_at", "Flagged"),
    ("wl_sales.segments.counted_at", "Counted"),
    ("wl_sales.segments.archived", "Archived"),
    (
        "wl_sales.segments.empty_hint",
        "Make a segment to pick customers for a campaign.",
    ),
    (
        "wl_sales.acquisitions.missing.empty_hint",
        "A first sale with no source shows up here.",
    ),
    ("wl_sales.changes.created_at", "Asked"),
    (
        "wl_sales.changes.empty_hint",
        "A change of source or campaign shows up here.",
    ),
    (
        "wl_sales.campaigns.empty_hint",
        "Make a campaign to follow its costs and what it brings.",
    ),
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
    ("wl_sales.report.sort", "Sort by"),
    ("wl_sales.report.own_order", "The report's order"),
    ("wl_sales.report.up", "lowest or A first"),
    ("wl_sales.report.down", "highest or Z first"),
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
    (
        "wl_sales.packages.problem.credits",
        "The package does not have that many credits left.",
    ),
    (
        "wl_sales.packages.problem.not_last",
        "These credits do not use up the package.",
    ),
    (
        "wl_sales.packages.problem.is_last",
        "These credits use up the package.",
    ),
    (
        "wl_sales.packages.problem.expired",
        "The package has expired.",
    ),
    (
        "wl_sales.packages.problem.not_paid",
        "The package is not paid yet.",
    ),
    (
        "wl_sales.packages.problem.reverse",
        "Give back at least one credit and no more than were used.",
    ),
    (
        "wl_sales.packages.problem.not_ended",
        "The package is still valid.",
    ),
    (
        "wl_sales.packages.problem.new_end",
        "Give an end date after the current one.",
    ),
    (
        "wl_sales.packages.problem.extension_limit",
        "The package was extended as often as allowed.",
    ),
    (
        "wl_sales.packages.problem.paid",
        "Something was paid on this package; it cannot be cancelled.",
    ),
    (
        "wl_sales.packages.problem.credits_total",
        "A package holds at least one credit.",
    ),
    (
        "wl_sales.packages.problem.price",
        "The price cannot be negative.",
    ),
    (
        "wl_sales.packages.problem.validity",
        "A package is valid for at least one month.",
    ),
    (
        "wl_sales.packages.problem.not_pending",
        "Only a package waiting for payment can be linked to an invoice.",
    ),
    (
        "wl_sales.packages.problem.unpaid",
        "A reopened balance on the package is unpaid; the credit was not consumed.",
    ),
    (
        "wl_sales.packages.collect_title",
        "Collect :balance before the next use of :package",
    ),
    (
        "wl_sales.packages.collect_description",
        "The payment for :package was reversed and its invoice is open again with :balance due. The package stays active and the credits used stay used. Collect the balance before the next use.",
    ),
    (
        "wl_sales.packages.refused_comment",
        "Credit not consumed: unpaid. :quantity for :source. Consume it by hand once the balance is paid.",
    ),
    (
        "wl_sales.notices.package_balance_due_subject",
        "A balance is due on your package",
    ),
    (
        "wl_sales.notices.package_balance_due_body",
        "The payment for your package {{ package }} was reversed. {{ balance }} AED is due again. Please pay it before your next visit.",
    ),
    ("wl_sales.counter.title", "Counter sales"),
    ("wl_sales.counter.subtitle", "Sales rung up at the till."),
    ("wl_sales.counter.open", "Open a sale"),
    (
        "wl_sales.counter.open_description",
        "A cart for a walk-in customer, or for a customer by party id.",
    ),
    ("wl_sales.counter.customer", "Customer (party id)"),
    ("wl_sales.counter.number", "Receipt"),
    ("wl_sales.counter.total", "Total"),
    ("wl_sales.counter.empty", "No counter sales"),
    ("wl_sales.counter.opened", "Sale opened."),
    ("wl_sales.counter.add_line", "Add a line"),
    ("wl_sales.counter.item", "Item code"),
    ("wl_sales.counter.description", "Description"),
    ("wl_sales.counter.quantity", "Quantity"),
    (
        "wl_sales.counter.unit_price",
        "Unit price (AED, before VAT)",
    ),
    ("wl_sales.counter.discount", "Discount (AED)"),
    (
        "wl_sales.counter.loyalty_excluded",
        "Not counted by the loyalty programme",
    ),
    ("wl_sales.counter.complete", "Complete the sale"),
    (
        "wl_sales.counter.complete_description",
        "Enter what was taken. It must add up to the total.",
    ),
    ("wl_sales.counter.card_payment", "Card payment id"),
    ("wl_sales.counter.method", "Method"),
    ("wl_sales.counter.method.cash", "Cash"),
    ("wl_sales.counter.method.card", "Card"),
    ("wl_sales.counter.method.account", "Customer account"),
    ("wl_sales.counter.method.other", "Other"),
    ("wl_sales.counter.lines", "Lines"),
    ("wl_sales.counter.no_lines", "No lines yet"),
    ("wl_sales.counter.totals", "Totals"),
    ("wl_sales.counter.net", "Before VAT"),
    ("wl_sales.counter.vat", "VAT"),
    ("wl_sales.counter.payments", "Payments"),
    ("wl_sales.counter.no_payments", "Nothing taken yet"),
    ("wl_sales.counter.line_added", "Line added."),
    ("wl_sales.counter.completed", "Sale completed."),
    ("wl_sales.counter.status.open", "Open"),
    ("wl_sales.counter.status.completed", "Completed"),
    ("wl_sales.counter.errors.number", "Give a whole number."),
    (
        "wl_sales.counter.errors.amount",
        "Give an amount such as 105.00.",
    ),
    ("wl_sales.counter.errors.item", "Give the item code."),
    (
        "wl_sales.counter.errors.choice",
        "Choose one of the offered values.",
    ),
    (
        "wl_sales.counter.problem.no_lines",
        "The sale has no lines.",
    ),
    (
        "wl_sales.counter.problem.tenders",
        "What was taken must add up to the total.",
    ),
    (
        "wl_sales.counter.problem.method",
        "Use cash, card, account or other.",
    ),
    (
        "wl_sales.counter.problem.account",
        "Charging to the customer account needs a customer on the sale and the setting switched on.",
    ),
    (
        "wl_sales.counter.problem.not_open",
        "The sale is completed and cannot change.",
    ),
    (
        "wl_sales.counter.problem.line",
        "Give a quantity above zero and a price of zero or more.",
    ),
    ("wl_sales.orders.title", "Sales orders"),
    (
        "wl_sales.orders.subtitle",
        "Orders from the customers, from the draft to closed.",
    ),
    ("wl_sales.orders.create", "Enter an order"),
    ("wl_sales.orders.customer", "Customer (party id)"),
    ("wl_sales.orders.customer_name", "Customer"),
    ("wl_sales.orders.channel", "Channel"),
    ("wl_sales.orders.payment_mode", "Payment"),
    ("wl_sales.orders.number", "Order"),
    ("wl_sales.orders.total", "Total"),
    ("wl_sales.orders.empty", "No sales orders"),
    ("wl_sales.orders.created", "Order entered."),
    ("wl_sales.orders.add_line", "Add a line"),
    ("wl_sales.orders.line_added", "Line added."),
    ("wl_sales.orders.submit", "Submit the order"),
    (
        "wl_sales.orders.submit_message",
        "The order is placed and its lines can no longer change.",
    ),
    ("wl_sales.orders.submitted", "Order submitted."),
    ("wl_sales.orders.fulfilled", "Handed over"),
    ("wl_sales.orders.reason", "Reason"),
    (
        "wl_sales.orders.payment_failed_reason",
        "The online payment failed.",
    ),
    ("wl_sales.orders.status.draft", "Draft"),
    (
        "wl_sales.orders.status.awaiting_payment",
        "Awaiting payment",
    ),
    (
        "wl_sales.orders.status.pending_acceptance",
        "Pending acceptance",
    ),
    ("wl_sales.orders.status.credit_hold", "On credit hold"),
    ("wl_sales.orders.status.confirmed", "Confirmed"),
    (
        "wl_sales.orders.status.partially_fulfilled",
        "Partially fulfilled",
    ),
    ("wl_sales.orders.status.fulfilled", "Fulfilled"),
    ("wl_sales.orders.status.closed", "Closed"),
    ("wl_sales.orders.status.rejected", "Rejected"),
    ("wl_sales.orders.status.cancelled", "Cancelled"),
    ("wl_sales.orders.errors.number", "Give a whole number."),
    (
        "wl_sales.orders.errors.amount",
        "Give an amount such as 105.00.",
    ),
    ("wl_sales.orders.errors.item", "Give the item code."),
    (
        "wl_sales.orders.errors.choice",
        "Choose one of the offered values.",
    ),
    (
        "wl_sales.orders.problem.no_lines",
        "The order has no lines.",
    ),
    (
        "wl_sales.orders.problem.not_draft",
        "Only a draft order can change.",
    ),
    (
        "wl_sales.orders.problem.line",
        "Give a quantity above zero and a price of zero or more.",
    ),
    (
        "wl_sales.orders.problem.amount",
        "The amount paid is not the order total.",
    ),
    (
        "wl_sales.orders.problem.input",
        "The transition was run without what it needs.",
    ),
    (
        "wl_sales.orders.problem.quantity",
        "A quantity is not above zero, is repeated, or is more than is open or held.",
    ),
    (
        "wl_sales.orders.problem.completeness",
        "The order does not end in the state this transition leads to.",
    ),
    (
        "wl_sales.orders.problem.not_invoiced",
        "What was fulfilled is not invoiced yet.",
    ),
    (
        "wl_sales.orders.problem.unknown_line",
        "A line does not belong to the order.",
    ),
    (
        "wl_sales.orders.problem.unknown_source",
        "The order was not fulfilled from this hand-over.",
    ),
    (
        "wl_sales.orders.problem.no_quantities",
        "Give at least one quantity.",
    ),
    (
        "wl_sales.orders.problem.duplicate_external",
        "This channel already sent that order number: it is order :number (id :id). No second order was made.",
    ),
    (
        "wl_sales.orders.problem.promo_unknown",
        "There is no promo code with this name.",
    ),
    (
        "wl_sales.orders.problem.promo_ended",
        "This promo code has ended.",
    ),
    (
        "wl_sales.orders.problem.promo_used_up",
        "This promo code has no uses left.",
    ),
    ("wl_sales.orders.external_number", "External order number"),
    ("wl_sales.orders.promo_code", "Promo code"),
    ("wl_sales.orders.payment_label", "Online payment"),
    (
        "wl_sales.orders.payment_status.not_required",
        "Not paid online",
    ),
    (
        "wl_sales.orders.payment_status.pending",
        "Waiting for the gateway",
    ),
    ("wl_sales.orders.payment_status.received", "Received"),
    ("wl_sales.orders.payment_status.failed", "Failed"),
    ("wl_sales.orders.payment_due", "Pay by"),
    ("wl_sales.orders.payment_link", "Payment link"),
    (
        "wl_sales.orders.awaiting_hint",
        "Waiting for the gateway until :due. The outlet, kitchen and dispatch do not see this order until the full amount is confirmed; if it is not, the order is cancelled as payment failed.",
    ),
    ("wl_sales.orders.tab.active", "Orders"),
    ("wl_sales.orders.tab.awaiting", "Awaiting payment"),
    (
        "wl_sales.orders.awaiting_empty",
        "No order is waiting for its payment",
    ),
    ("wl_sales.orders.payments", "Payments"),
    ("wl_sales.orders.payment_id", "Payment"),
    ("wl_sales.orders.payment_amount", "Amount"),
    ("wl_sales.orders.payment_outcome", "What was done"),
    ("wl_sales.orders.payment_refund", "Refund"),
    (
        "wl_sales.orders.payments_empty",
        "No online payment was received for this order",
    ),
    ("wl_sales.orders.outcome.claimed", "Being handled"),
    ("wl_sales.orders.outcome.confirmed", "Confirmed the order"),
    (
        "wl_sales.orders.outcome.refund_requested",
        "Refund requested (late payment)",
    ),
    (
        "wl_sales.orders.outcome.refund_refused",
        "Refund could not be requested: a person must check it",
    ),
    ("wl_sales.orders.late_payment_reason", "late payment"),
    (
        "wl_sales.orders.late_task_title",
        "Late payment for order :number",
    ),
    (
        "wl_sales.orders.late_task_body",
        "Payment :payment of :amount arrived for order :number, which cannot take it (cancelled, rejected, already paid or a different amount). A full refund was requested; the order did not change.",
    ),
    (
        "wl_sales.orders.late_task_unrefunded",
        "Payment :payment of :amount arrived for order :number, which cannot take it, but the refund could not be requested. Please refund it.",
    ),
    ("wl_sales.returns.title", "Sales returns"),
    (
        "wl_sales.returns.subtitle",
        "Goods that come back, and refunds after completion.",
    ),
    ("wl_sales.returns.request", "Request a return"),
    (
        "wl_sales.returns.request_description",
        "Name the sale (counter_sale or sales_order), its id and the lines to take back as line id:quantity, separated by commas.",
    ),
    ("wl_sales.returns.sale_type", "Sale type"),
    ("wl_sales.returns.sale_id", "Sale id"),
    ("wl_sales.returns.sale", "Sale"),
    ("wl_sales.returns.kind", "Kind (goods or no_goods)"),
    ("wl_sales.returns.lines", "Lines (id:quantity)"),
    ("wl_sales.returns.reason", "Reason"),
    ("wl_sales.returns.number", "Return"),
    ("wl_sales.returns.value", "Value with VAT"),
    ("wl_sales.returns.net", "Value before VAT"),
    ("wl_sales.returns.empty", "No sales returns"),
    ("wl_sales.returns.none", "None"),
    ("wl_sales.returns.details", "Details"),
    ("wl_sales.returns.receive", "Receive the goods"),
    (
        "wl_sales.returns.receive_description",
        "The goods are back in quarantine.",
    ),
    (
        "wl_sales.returns.approved",
        "Accept outside the return window",
    ),
    ("wl_sales.returns.resolve", "Resolve"),
    ("wl_sales.returns.resolution", "Resolution"),
    ("wl_sales.returns.credit_note", "Credit note id"),
    ("wl_sales.returns.inspected", "The goods were inspected"),
    ("wl_sales.returns.complaint_category", "Complaint category"),
    ("wl_sales.returns.responsible_party", "Responsible party"),
    ("wl_sales.returns.closing_reason", "Why it was closed"),
    ("wl_sales.returns.requested", "Return requested."),
    ("wl_sales.returns.received", "Goods received."),
    ("wl_sales.returns.resolved", "Return resolved."),
    ("wl_sales.returns.status.requested", "Requested"),
    ("wl_sales.returns.status.received", "Received"),
    ("wl_sales.returns.status.resolved", "Resolved"),
    ("wl_sales.returns.status.reversed", "Reversed"),
    ("wl_sales.returns.status.rejected", "Rejected"),
    ("wl_sales.returns.kind.goods", "Goods"),
    ("wl_sales.returns.kind.no_goods", "No goods"),
    ("wl_sales.returns.resolution.credit_note", "Credit note"),
    ("wl_sales.returns.resolution.refund", "Refund"),
    ("wl_sales.returns.resolution.replacement", "Replacement"),
    (
        "wl_sales.returns.resolution.refunded_by_channel",
        "Refunded by the channel",
    ),
    (
        "wl_sales.returns.errors.choice",
        "Choose one of the offered values.",
    ),
    ("wl_sales.returns.errors.number", "Give a whole number."),
    (
        "wl_sales.returns.errors.lines",
        "Give the lines as line id:quantity, separated by commas.",
    ),
    (
        "wl_sales.returns.problem.no_lines",
        "Name at least one line to take back.",
    ),
    (
        "wl_sales.returns.problem.not_completed",
        "Only a completed counter sale can be returned.",
    ),
    (
        "wl_sales.returns.problem.not_handed_over",
        "Only an order that was handed over can be returned.",
    ),
    (
        "wl_sales.returns.problem.sale_type",
        "Only a counter sale or a sales order can be returned.",
    ),
    (
        "wl_sales.returns.problem.unknown_line",
        "A line does not belong to the sale.",
    ),
    (
        "wl_sales.returns.problem.quantity",
        "A quantity is not above zero, is repeated, or is more than the sale holds.",
    ),
    (
        "wl_sales.returns.problem.no_goods_to_receive",
        "A return with no goods has nothing to receive.",
    ),
    (
        "wl_sales.returns.problem.window",
        "The return window has passed; a manager must accept it.",
    ),
    (
        "wl_sales.returns.problem.resolution",
        "Choose how the return is resolved.",
    ),
    (
        "wl_sales.returns.problem.inspection",
        "Receive the goods and record the inspection first.",
    ),
    (
        "wl_sales.returns.problem.complaint",
        "Give the complaint category and the responsible party.",
    ),
    (
        "wl_sales.returns.problem.channel",
        "Only a return with no goods can be refunded by the channel.",
    ),
    (
        "wl_sales.returns.problem.credit_note",
        "Name the credit note that was issued.",
    ),
    (
        "wl_sales.returns.problem.not_by_channel",
        "Only a refund made by the channel can be reversed.",
    ),
    ("wl_sales.loyalty.title", "Loyalty"),
    (
        "wl_sales.loyalty.subtitle",
        "The loyalty programme, its members and their points.",
    ),
    ("wl_sales.loyalty.enrol", "Enrol a member"),
    (
        "wl_sales.loyalty.enrol_description",
        "Records the version of the terms the member accepted. It is not a marketing consent.",
    ),
    ("wl_sales.loyalty.party", "Member (party id)"),
    ("wl_sales.loyalty.mobile", "Mobile number"),
    (
        "wl_sales.loyalty.guardian",
        "Guardian (party id, for a minor)",
    ),
    (
        "wl_sales.loyalty.accept_terms",
        "The member accepted the terms",
    ),
    ("wl_sales.loyalty.turn_on", "Turn the programme on"),
    ("wl_sales.loyalty.name", "Name"),
    ("wl_sales.loyalty.earn_kind", "Earn (per_aed or per_visit)"),
    (
        "wl_sales.loyalty.earn_rate",
        "Points per AED, or visits per reward",
    ),
    (
        "wl_sales.loyalty.minimum",
        "Minimum qualifying amount (AED, before VAT)",
    ),
    ("wl_sales.loyalty.point_value", "Point value (AED)"),
    ("wl_sales.loyalty.validity", "Points valid for (months)"),
    ("wl_sales.loyalty.terms_version", "Terms version"),
    ("wl_sales.loyalty.terms_en", "Terms in English"),
    ("wl_sales.loyalty.terms_ar", "Terms in Arabic"),
    (
        "wl_sales.loyalty.no_programme",
        "There is no loyalty programme yet.",
    ),
    ("wl_sales.loyalty.accounts", "Members"),
    ("wl_sales.loyalty.code", "Code"),
    ("wl_sales.loyalty.balance", "Balance"),
    ("wl_sales.loyalty.no_accounts", "No members yet"),
    ("wl_sales.loyalty.status.active", "Active"),
    ("wl_sales.loyalty.status.closed", "Closed"),
    (
        "wl_sales.loyalty.programme_created",
        "Loyalty programme turned on.",
    ),
    ("wl_sales.loyalty.enrolled", "Member enrolled."),
    ("wl_sales.loyalty.earn.per_aed", "Points per AED 1: :rate"),
    (
        "wl_sales.loyalty.earn.per_visit",
        "A reward after :rate visits",
    ),
    ("wl_sales.loyalty.errors.number", "Give a whole number."),
    (
        "wl_sales.loyalty.errors.amount",
        "Give an amount such as 0.01.",
    ),
    (
        "wl_sales.loyalty.errors.choice",
        "Choose one of the offered values.",
    ),
    (
        "wl_sales.loyalty.problem.name",
        "Give the programme a name.",
    ),
    (
        "wl_sales.loyalty.problem.terms",
        "The rate, the point value and the validity must be at least 1, and the minimum, the redemption terms and the share cannot be negative.",
    ),
    (
        "wl_sales.loyalty.problem.brand_taken",
        "This brand already has a loyalty programme.",
    ),
    (
        "wl_sales.loyalty.problem.no_programme",
        "There is no loyalty programme to enrol in.",
    ),
    (
        "wl_sales.loyalty.problem.terms_not_accepted",
        "The member must accept the terms.",
    ),
    (
        "wl_sales.loyalty.problem.mobile",
        "Give a mobile number with 8 to 15 digits.",
    ),
    (
        "wl_sales.loyalty.problem.already_member",
        "The party is already a member of this programme.",
    ),
    (
        "wl_sales.loyalty.problem.mobile_taken",
        "This mobile number belongs to another member of the programme.",
    ),
    ("wl_sales.loyalty.attach", "Attach a member"),
    (
        "wl_sales.loyalty.attach_description",
        "Give the member's code, mobile number or party. A member can only be attached before the sale completes.",
    ),
    ("wl_sales.loyalty.member", "Loyalty member"),
    ("wl_sales.loyalty.attached", "Member attached to the sale."),
    (
        "wl_sales.loyalty.errors.member",
        "Give the member's code, mobile number or party.",
    ),
    (
        "wl_sales.loyalty.problem.no_member",
        "There is no open loyalty account for this member.",
    ),
    (
        "wl_sales.loyalty.problem.programme_off",
        "The loyalty programme is not earning.",
    ),
    (
        "wl_sales.loyalty.problem.own_account",
        "A cashier cannot attach a sale they process to their own loyalty account.",
    ),
    (
        "wl_sales.loyalty.problem.sale_completed",
        "The sale is completed or ended; a member is attached before it completes.",
    ),
    (
        "wl_sales.notices.loyalty_reward_subject",
        "A reward is waiting for you",
    ),
    (
        "wl_sales.notices.loyalty_reward_body",
        "You reached the visits for a reward ({{ reward }}). Show your code {{ code }} before {{ valid_until }}.",
    ),
    (
        "wl_sales.notices.loyalty_reversed_subject",
        "Points were taken back",
    ),
    (
        "wl_sales.notices.loyalty_reversed_body",
        "{{ points }} points were taken back from your account {{ code }} because {{ why }}.",
    ),
    ("wl_sales.loyalty.why.return", "goods were returned"),
    (
        "wl_sales.loyalty.why.credit_note",
        "a credit note was issued",
    ),
    (
        "wl_sales.loyalty.why.payment_reversed",
        "a payment was reversed",
    ),
    (
        "wl_sales.loyalty.why.written_off",
        "an invoice was written off",
    ),
    (
        "wl_sales.loyalty.why.fulfilment_reversed",
        "a delivery was reversed",
    ),
    ("wl_sales.loyalty.review.title", "Accounts to review"),
    (
        "wl_sales.loyalty.review.subtitle",
        "Members who earned on more sales in a day than the limit. They earned; a manager looks at them.",
    ),
    ("wl_sales.loyalty.review.day", "Day"),
    ("wl_sales.loyalty.review.sales", "Earning sales"),
    ("wl_sales.loyalty.review.resolve", "Mark as reviewed"),
    (
        "wl_sales.loyalty.review.resolved",
        "Account marked as reviewed.",
    ),
    ("wl_sales.loyalty.review.empty", "Nothing to review"),
    ("wl_sales.orders.cancel_line", "Cancel a line"),
    (
        "wl_sales.orders.cancel_line_description",
        "Cancels open quantity of one line. Points redeemed on the order are given back in proportion.",
    ),
    ("wl_sales.orders.line_no", "Line"),
    ("wl_sales.orders.line_cancelled", "Line cancelled."),
    ("wl_sales.orders.errors.reason", "Give the reason."),
    (
        "wl_sales.orders.problem.cancel_line_last",
        "A line cancellation must leave some quantity open; cancel the order or its remaining quantities instead.",
    ),
    ("wl_sales.redeem.line", "Loyalty redemption"),
    (
        "wl_sales.redeem.problem.sale_type",
        "Points can be redeemed on a counter sale or a sales order.",
    ),
    (
        "wl_sales.redeem.problem.sale_closed",
        "Points can only be redeemed on a sale that is still open.",
    ),
    (
        "wl_sales.redeem.problem.no_member",
        "Attach the loyalty member to the sale before redeeming points.",
    ),
    (
        "wl_sales.redeem.problem.programme_off",
        "The loyalty programme no longer takes redemptions.",
    ),
    (
        "wl_sales.redeem.problem.visits",
        "A reward of a visit programme is not redeemed as points.",
    ),
    (
        "wl_sales.redeem.problem.points",
        "Give a number of points above zero.",
    ),
    (
        "wl_sales.redeem.problem.below_minimum",
        "The member needs to redeem at least :min points.",
    ),
    (
        "wl_sales.redeem.problem.above_share",
        "At most :max points can pay for this sale.",
    ),
    (
        "wl_sales.redeem.problem.not_enough",
        "The member does not have :points points to spend now.",
    ),
    (
        "wl_sales.redeem.problem.locked",
        "Redemption was refused for this sale after three wrong codes. The sale goes on without it.",
    ),
    (
        "wl_sales.redeem.problem.locked_now",
        "Three wrong codes: redemption is refused for this sale. The sale goes on without it.",
    ),
    (
        "wl_sales.redeem.problem.already",
        "Points are already redeemed on this sale.",
    ),
    (
        "wl_sales.redeem.problem.none_pending",
        "No redemption is waiting for a code on this sale.",
    ),
    (
        "wl_sales.redeem.problem.no_code",
        "Give the code the member received.",
    ),
    (
        "wl_sales.redeem.problem.wrong",
        "Wrong code. :left tries left.",
    ),
    (
        "wl_sales.redeem.problem.expired",
        "The code expired. Ask for a new one.",
    ),
    (
        "wl_sales.redeem.problem.short",
        "The member's points are not all there to spend.",
    ),
    (
        "wl_sales.redeem.problem.input",
        "The transition was run without what it needs.",
    ),
    ("wl_sales.redeem.redeem", "Redeem points"),
    (
        "wl_sales.redeem.redeem_description",
        "The member gets a code on their mobile. The points are held for the sale once the code is entered.",
    ),
    ("wl_sales.redeem.points", "Points"),
    ("wl_sales.redeem.code", "Code the member received"),
    ("wl_sales.redeem.confirm", "Confirm with the code"),
    (
        "wl_sales.redeem.code_sent",
        "Code sent to the member's mobile.",
    ),
    ("wl_sales.redeem.held", "Points held for the sale."),
    ("wl_sales.redeem.confirmed", "Points redeemed on the sale."),
    ("wl_sales.redeem.waiting", "Waiting for the code"),
    (
        "wl_sales.loyalty.why.cancel_line",
        "a line of the order was cancelled",
    ),
    (
        "wl_sales.loyalty.why.order_cancelled",
        "the order was cancelled",
    ),
    (
        "wl_sales.loyalty.why.order_rejected",
        "the order was rejected",
    ),
    (
        "wl_sales.notices.loyalty_given_back_subject",
        "Your points were given back",
    ),
    (
        "wl_sales.notices.loyalty_given_back_body",
        "{{ points }} points were given back to your account {{ code }} because {{ why }}.",
    ),
    (
        "wl_sales.notices.back_order_reserved_subject",
        "Your back-ordered goods are in stock",
    ),
    (
        "wl_sales.notices.back_order_reserved_body",
        "Goods back-ordered on your order {{ number }} arrived and are reserved for you: {{ lines }}.",
    ),
    ("wl_sales.orders.backordered", "Back-ordered"),
    ("wl_sales.redeem.status.pending", "Waiting for the code"),
    ("wl_sales.redeem.status.held", "Held"),
    ("wl_sales.redeem.status.applied", "Redeemed"),
    ("wl_sales.redeem.status.released", "Released"),
    ("wl_sales.redeem.status.refused", "Refused"),
    ("wl_sales.loyalty.min_redeem", "Fewest points to redeem"),
    (
        "wl_sales.loyalty.max_share",
        "Largest share of a sale points may pay (%)",
    ),
    (
        "wl_sales.notices.loyalty_expiry_subject",
        "Your points are about to expire",
    ),
    (
        "wl_sales.notices.loyalty_expiry_body",
        "{{ points }} points (AED {{ value }}) on your account {{ code }} expire on {{ expires_on }}. Use them before then.",
    ),
    (
        "wl_sales.notices.adjustment_rejected_subject",
        "A points adjustment was rejected",
    ),
    (
        "wl_sales.notices.adjustment_rejected_body",
        "Your adjustment of {{ points }} points on account {{ code }} was rejected: {{ reason }}. Nothing was changed.",
    ),
    (
        "wl_sales.notices.adjustment_returned_subject",
        "A points adjustment was returned to you",
    ),
    (
        "wl_sales.notices.adjustment_returned_body",
        "Your adjustment of {{ points }} points on account {{ code }} was returned: {{ reason }}. Change it and submit it again.",
    ),
    ("wl_sales.adjust.closed_by_staff", "Closed by staff:"),
    (
        "wl_sales.adjust.problem.no_account",
        "There is no such loyalty account.",
    ),
    (
        "wl_sales.adjust.problem.closed",
        "The loyalty account is closed.",
    ),
    (
        "wl_sales.adjust.problem.more_than_held",
        "The member does not have that many points to take off.",
    ),
    (
        "wl_sales.adjust.problem.needs_manager",
        "This adjustment needs a manager's approval.",
    ),
    (
        "wl_sales.adjust.problem.within_limit",
        "This adjustment is within the limit and needs no approval.",
    ),
    (
        "wl_sales.adjust.problem.no_limit",
        "The adjustment was run without the limit.",
    ),
    (
        "wl_sales.adjust.problem.no_policy",
        "The approval policy of points adjustments is missing.",
    ),
    ("wl_sales.adjust.problem.reason", "Give the reason."),
    (
        "wl_sales.adjust.problem.points",
        "Give a number of points other than zero.",
    ),
    (
        "wl_sales.adjust.problem.not_requester",
        "Only the one who asked for the adjustment changes it.",
    ),
    (
        "wl_sales.adjust.problem.not_draft",
        "Only an adjustment that was returned can be changed.",
    ),
    ("wl_sales.adjust.title", "Points adjustments"),
    (
        "wl_sales.adjust.subtitle",
        "Corrections of a member's points by hand. Above the limit, and when staff close an account with points, a manager decides under Needs your action.",
    ),
    ("wl_sales.adjust.adjust", "Adjust points"),
    (
        "wl_sales.adjust.adjust_description",
        "Add points (+) or take points off (-). Give the reason.",
    ),
    ("wl_sales.adjust.points", "Points (+ or -)"),
    ("wl_sales.adjust.reason", "Reason"),
    ("wl_sales.adjust.close", "Close the account"),
    (
        "wl_sales.adjust.close_description",
        "Staff close the account. Points left go with it and a manager approves.",
    ),
    ("wl_sales.adjust.resubmit", "Change and submit again"),
    ("wl_sales.adjust.requested", "Adjustment saved."),
    (
        "wl_sales.adjust.waiting",
        "Adjustment sent to a manager for approval.",
    ),
    ("wl_sales.adjust.closed", "Account closed."),
    ("wl_sales.adjust.resubmitted", "Adjustment submitted again."),
    ("wl_sales.adjust.empty", "No adjustments"),
    ("wl_sales.adjust.account", "Member"),
    ("wl_sales.adjust.outcome", "Outcome"),
    ("wl_sales.adjust.status.draft", "Returned"),
    (
        "wl_sales.adjust.status.pending_approval",
        "Waiting for a manager",
    ),
    ("wl_sales.adjust.status.applied", "Applied"),
    ("wl_sales.adjust.status.rejected", "Rejected"),
    (
        "wl_sales.loyalty.reminder_not_delivered",
        "Reminder not delivered",
    ),
    ("wl_sales.loyalty.adjustments", "Adjustments"),
    ("wl_sales.liability.title", "Points liability"),
    (
        "wl_sales.liability.subtitle",
        "Outstanding points and what they are worth, by brand, entity, age and expiry month. Negative balances count as zero.",
    ),
    ("wl_sales.liability.as_at", "As at"),
    ("wl_sales.liability.show", "Show"),
    ("wl_sales.liability.brand", "Brand"),
    ("wl_sales.liability.entity", "Entity"),
    ("wl_sales.liability.age", "Age of the points"),
    ("wl_sales.liability.expiry_month", "Expiry month"),
    ("wl_sales.liability.points", "Points"),
    ("wl_sales.liability.value", "Value"),
    ("wl_sales.liability.total", "Total"),
    ("wl_sales.liability.empty", "No points outstanding"),
    ("wl_sales.liability.own_brand", "(client)"),
    ("wl_sales.liability.age.0_90", "Up to 90 days"),
    ("wl_sales.liability.age.91_180", "91 to 180 days"),
    ("wl_sales.liability.age.181_365", "181 to 365 days"),
    ("wl_sales.liability.age.over_365", "Over a year"),
    ("wl_sales.errors.date", "Give a date such as 2026-12-31."),
    ("wl_sales.loyalty.why.expire", "the points expired"),
    ("wl_sales.loyalty.reminder", "Expiry reminder"),
    ("wl_sales.segments.title", "Customer segments"),
    (
        "wl_sales.segments.subtitle",
        "Saved rules that pick customers for a retention campaign. The members are worked out each time a segment is used.",
    ),
    ("wl_sales.segments.new_title", "New segment"),
    ("wl_sales.segments.name", "Name"),
    ("wl_sales.segments.match", "Customers must meet"),
    ("wl_sales.segments.rules", "Rules"),
    ("wl_sales.segments.count", "Last count (estimate)"),
    ("wl_sales.segments.empty", "No segments yet"),
    ("wl_sales.segments.saved", "Segment saved."),
    (
        "wl_sales.segments.previewed",
        "About :count customers match now. The count is an estimate; the segment is worked out again at send time.",
    ),
    ("wl_sales.segments.preview", "Count the members now"),
    ("wl_sales.segments.export", "Export the members"),
    ("wl_sales.segments.members", "Members"),
    ("wl_sales.segments.estimate", "Estimate at the last count:"),
    ("wl_sales.segments.save", "Save the segment"),
    ("wl_sales.segments.mode.all", "All of the rules"),
    ("wl_sales.segments.mode.any", "Any of the rules"),
    ("wl_sales.segments.enrolled.yes", "Enrolled in loyalty"),
    ("wl_sales.segments.enrolled.no", "Not enrolled in loyalty"),
    ("wl_sales.segments.errors.number", "Give a whole number."),
    (
        "wl_sales.segments.errors.amount",
        "Give an amount such as 500 or 500.00.",
    ),
    (
        "wl_sales.segments.q.no_sale_days",
        "No sale for at least this many days",
    ),
    (
        "wl_sales.segments.q.spend_amount",
        "Spent more than (AED, before VAT)",
    ),
    (
        "wl_sales.segments.q.spend_days",
        "... in the last days (default 365)",
    ),
    (
        "wl_sales.segments.q.sales_count",
        "At least this many sales",
    ),
    (
        "wl_sales.segments.q.sales_days",
        "... in the last days (default 365)",
    ),
    ("wl_sales.segments.q.item", "Bought the item"),
    (
        "wl_sales.segments.q.item_days",
        "... within the last days (empty: ever)",
    ),
    (
        "wl_sales.segments.q.balance",
        "Loyalty balance of at least (points)",
    ),
    (
        "wl_sales.segments.q.credits",
        "Package credits left, at least",
    ),
    ("wl_sales.segments.q.enrolled", "Loyalty membership"),
    ("wl_sales.segments.q.any", "Not used"),
    (
        "wl_sales.segments.rule.no_sale_for_days",
        "No sale for :days days or more",
    ),
    (
        "wl_sales.segments.rule.spend_over",
        "Spent more than :amount in the last :days days",
    ),
    (
        "wl_sales.segments.rule.sales_at_least",
        "At least :count sales in the last :days days",
    ),
    ("wl_sales.segments.rule.bought_item", "Bought :item"),
    (
        "wl_sales.segments.rule.bought_item_within",
        "Bought :item in the last :days days",
    ),
    (
        "wl_sales.segments.rule.loyalty_balance",
        "Loyalty balance of :points points or more",
    ),
    (
        "wl_sales.segments.rule.package_credits",
        ":credits package credits or more left",
    ),
    ("wl_sales.segments.rule.enrolled", "Enrolled in loyalty"),
    (
        "wl_sales.segments.rule.not_enrolled",
        "Not enrolled in loyalty",
    ),
    ("wl_sales.segments.rule.vertical", "Rule :name"),
    ("wl_sales.segments.problem.name", "Give the segment a name."),
    (
        "wl_sales.segments.problem.name_taken",
        "Another segment has this name.",
    ),
    (
        "wl_sales.segments.problem.no_rules",
        "Give at least one rule: a segment with none would pick everybody.",
    ),
    (
        "wl_sales.segments.problem.days",
        "The days must be 1 or more.",
    ),
    (
        "wl_sales.segments.problem.spend",
        "The amount cannot be negative and the days must be 1 or more.",
    ),
    (
        "wl_sales.segments.problem.count",
        "The number of sales and the days must be 1 or more.",
    ),
    (
        "wl_sales.segments.problem.item",
        "Name the item; the days, when given, must be 1 or more.",
    ),
    (
        "wl_sales.segments.problem.points",
        "The points cannot be negative.",
    ),
    (
        "wl_sales.segments.problem.credits",
        "The credits must be 1 or more.",
    ),
    (
        "wl_sales.segments.problem.vertical",
        "Name the kind of rule.",
    ),
    (
        "wl_sales.segments.problem.clinical",
        ":item is a clinical service; segments may not use clinical services.",
    ),
    (
        "wl_sales.segments.problem.not_found",
        "There is no such segment.",
    ),
    ("wl_sales.retention.title", "Retention message"),
    ("wl_sales.retention.requester", "Requested for"),
    ("wl_sales.retention.state", "Broadcast"),
    ("wl_sales.retention.state.none", "Not requested"),
    (
        "wl_sales.retention.state.requested",
        "Requested: waiting or scheduled",
    ),
    ("wl_sales.retention.state.completed", "Sent"),
    ("wl_sales.retention.state.cancelled", "Not sent"),
    ("wl_sales.retention.state.failed", "Not sent"),
    ("wl_sales.retention.recipients", "Recipients"),
    ("wl_sales.retention.delivered", "Delivered"),
    ("wl_sales.retention.opted_out", "Opted out"),
    ("wl_sales.retention.returned", "Returned"),
    ("wl_sales.retention.days", "days"),
    (
        "wl_sales.retention.returned_value",
        "Sales of returned customers before VAT",
    ),
    ("wl_sales.retention.cost", "Campaign cost"),
    ("wl_sales.retention.excluded", "Left out, by reason"),
    ("wl_sales.retention.reason", "Reason"),
    ("wl_sales.retention.count", "Customers"),
    ("wl_sales.retention.no_excluded", "Nobody left out"),
    ("wl_sales.retention.not_sent", "Not sent"),
    (
        "wl_sales.retention.request_again",
        "Request the broadcast again",
    ),
    (
        "wl_sales.retention.requested",
        "The broadcast was requested.",
    ),
    (
        "wl_sales.retention.not_sent_toast",
        "The broadcast was refused; the reason is shown on the campaign.",
    ),
    (
        "wl_sales.retention.reason.campaign_ended",
        "The campaign ended",
    ),
    (
        "wl_sales.retention.reason.requester_guard_failed",
        "The campaign no longer allowed the send",
    ),
    (
        "wl_sales.retention.reason.approval_expired",
        "Nobody approved it in time",
    ),
    ("wl_sales.retention.reason.rejected", "Rejected"),
    ("wl_sales.retention.reason.cancelled", "Cancelled"),
    (
        "wl_sales.retention.excluded.no_consent",
        "No marketing consent",
    ),
    ("wl_sales.retention.excluded.opted_out", "Opted out"),
    (
        "wl_sales.retention.excluded.blocked_party",
        "On hold, archived or merged",
    ),
    (
        "wl_sales.retention.excluded.suppressed_address",
        "Address suppressed",
    ),
    (
        "wl_sales.retention.excluded.no_contact_point",
        "No contact point",
    ),
    ("wl_sales.retention.excluded.frequency_cap", "Frequency cap"),
    ("wl_sales.retention.excluded.expired", "Too late to send"),
    (
        "wl_sales.retention.excluded.broadcast_stopped",
        "Broadcast stopped",
    ),
    (
        "wl_sales.retention.problem.not_found",
        "There is no such campaign.",
    ),
    (
        "wl_sales.retention.problem.not_retention",
        "Only a retention campaign sends a message.",
    ),
    (
        "wl_sales.retention.problem.not_active",
        "The campaign is not active.",
    ),
    (
        "wl_sales.retention.problem.incomplete",
        "Set the segment, template, channel and send time first.",
    ),
    (
        "wl_sales.retention.problem.open",
        "A broadcast is already requested for this campaign.",
    ),
    (
        "wl_sales.retention.problem.already_sent",
        "This campaign has sent its message. A new campaign is needed to send again.",
    ),
    (
        "wl_sales.retention.problem.channel",
        "The message channel is not one wl-notify knows.",
    ),
    ("wl_sales.closure.member_request", "Member request (:via)"),
    (
        "wl_sales.closure.problem.closed",
        "The account is closed already.",
    ),
    (
        "wl_sales.closure.errors.confirm",
        "The member must confirm before the points are forfeited.",
    ),
    ("wl_sales.closure.done", "The account is closed."),
    (
        "wl_sales.closure.portal_title",
        "Leave the loyalty programme",
    ),
    ("wl_sales.closure.leave", "Leave the programme"),
    ("wl_sales.closure.forfeit", "Points that will be forfeited:"),
    (
        "wl_sales.closure.confirm",
        "I understand that I lose these points and want to leave.",
    ),
    (
        "wl_sales.closure.confirm_counter",
        "The member asked to leave and confirmed that the points are forfeited.",
    ),
    (
        "wl_sales.closure.closed",
        "This account is closed. Enrolling again starts with no points.",
    ),
    ("wl_sales.merge.closed_reason", "Merged into :code"),
    (
        "wl_sales.terms.summary.earn_rate",
        "the earn rate becomes :rate",
    ),
    (
        "wl_sales.terms.summary.minimum",
        "the least amount of a sale becomes AED :amount",
    ),
    ("wl_sales.terms.summary.excluded", "items left out: :items"),
    (
        "wl_sales.terms.summary.validity",
        "new points are valid for :months months",
    ),
    (
        "wl_sales.terms.summary.terms",
        "the terms of the programme change",
    ),
    (
        "wl_sales.terms.problem.no_programme",
        "There is no such programme.",
    ),
    (
        "wl_sales.terms.problem.not_running",
        "The programme is not running.",
    ),
    (
        "wl_sales.terms.problem.nothing",
        "Change at least one term.",
    ),
    (
        "wl_sales.terms.problem.notice",
        "A change takes effect at least :days days after it is saved, so that members are told in time.",
    ),
    (
        "wl_sales.terms.saved",
        "The change is saved. :members members were told.",
    ),
    (
        "wl_sales.terms.turned_off",
        "The programme is turned off. Points can be redeemed until :date.",
    ),
    ("wl_sales.terms.closing", "Points can be redeemed until"),
    ("wl_sales.terms.change", "Change the terms"),
    (
        "wl_sales.terms.change_description",
        "The new terms apply to sales from the effective date, at least 30 days ahead. Points already earned keep their value and expiry. Members are told now.",
    ),
    ("wl_sales.terms.effective_from", "Effective from"),
    (
        "wl_sales.terms.excluded",
        "Items left out (codes, separated by commas)",
    ),
    ("wl_sales.terms.turn_off", "Turn the programme off"),
    (
        "wl_sales.terms.turn_off_description",
        "Earning stops now. Members can redeem for 90 days and are told. The points left expire after that.",
    ),
    ("wl_sales.terms.turn_off_confirm", "Turn the programme off"),
    (
        "wl_sales.terms.errors.confirm",
        "Confirm that the programme is turned off.",
    ),
    (
        "wl_sales.notices.loyalty_terms_changed_subject",
        "The loyalty terms change",
    ),
    (
        "wl_sales.notices.loyalty_terms_changed_body",
        "From {{ effective_from }} the terms of the loyalty programme change: {{ summary }}. Points you have already earned keep their value and expiry. Your account is {{ code }}.",
    ),
    (
        "wl_sales.notices.loyalty_programme_off_subject",
        "The loyalty programme is ending",
    ),
    (
        "wl_sales.notices.loyalty_programme_off_body",
        "The loyalty programme has stopped earning points. You can redeem the {{ points }} points on your account {{ code }} until {{ closing_date }}; what is left then expires.",
    ),
    (
        "wl_sales.notices.order_payment_failed_subject",
        "Your payment did not go through",
    ),
    (
        "wl_sales.notices.order_payment_failed_body",
        "The payment for order {{ number }} ({{ total }}) failed, so the order was cancelled and nothing was charged. Payment failed, please try again: place a new order.",
    ),
    (
        "wl_sales.notices.order_late_payment_subject",
        "Your payment is being refunded",
    ),
    (
        "wl_sales.notices.order_late_payment_body",
        "Your payment of {{ amount }} for order {{ number }} arrived after the order could no longer take it, so it is being refunded in full.",
    ),
    (
        "wl_sales.notices.loyalty_closed_subject",
        "Your loyalty account is closed",
    ),
    (
        "wl_sales.notices.loyalty_closed_body",
        "Your loyalty account {{ code }} is closed. {{ points }} points went with it.",
    ),
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
        .chain(crate::credit_texts::TEXTS)
        .chain(crate::texts_changes::CHANGE_TEXTS.iter())
        .chain(crate::texts_deliveries::DELIVERY_TEXTS.iter())
        .chain(crate::texts_destination::DESTINATION_TEXTS.iter())
        .chain(crate::texts_till::TILL_TEXTS.iter())
        .map(|(key, _)| (key.trim_start_matches("wl_sales."), t(lang, key)))
        .collect()
}

/// The English text for `key` with `params` filled in; the key itself when
/// it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &dyn Display)]) -> String {
    let text = TEXTS
        .iter()
        .chain(crate::credit_texts::TEXTS)
        .chain(crate::texts_changes::CHANGE_TEXTS.iter())
        .chain(crate::texts_deliveries::DELIVERY_TEXTS.iter())
        .chain(crate::texts_destination::DESTINATION_TEXTS.iter())
        .chain(crate::texts_till::TILL_TEXTS.iter())
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
        let mut keys: Vec<&str> = TEXTS
            .iter()
            .chain(crate::credit_texts::TEXTS)
            .chain(crate::texts_changes::CHANGE_TEXTS.iter())
            .chain(crate::texts_deliveries::DELIVERY_TEXTS.iter())
            .chain(crate::texts_destination::DESTINATION_TEXTS.iter())
            .chain(crate::texts_till::TILL_TEXTS.iter())
            .map(|(key, _)| *key)
            .collect();
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
