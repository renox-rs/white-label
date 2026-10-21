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
    ("wl_sales.counter.unit_price", "Unit price (AED, before VAT)"),
    ("wl_sales.counter.discount", "Discount (AED)"),
    ("wl_sales.counter.loyalty_excluded", "Not counted by the loyalty programme"),
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
    ("wl_sales.counter.errors.amount", "Give an amount such as 105.00."),
    ("wl_sales.counter.errors.item", "Give the item code."),
    ("wl_sales.counter.errors.choice", "Choose one of the offered values."),
    ("wl_sales.counter.problem.no_lines", "The sale has no lines."),
    ("wl_sales.counter.problem.tenders", "What was taken must add up to the total."),
    ("wl_sales.counter.problem.method", "Use cash, card, account or other."),
    (
        "wl_sales.counter.problem.account",
        "Charging to the customer account needs a customer on the sale and the setting switched on.",
    ),
    ("wl_sales.counter.problem.not_open", "The sale is completed and cannot change."),
    ("wl_sales.counter.problem.line", "Give a quantity above zero and a price of zero or more."),
    ("wl_sales.orders.title", "Sales orders"),
    ("wl_sales.orders.subtitle", "Orders from the customers, from the draft to closed."),
    ("wl_sales.orders.create", "Enter an order"),
    ("wl_sales.orders.customer", "Customer (party id)"),
    ("wl_sales.orders.channel", "Channel"),
    ("wl_sales.orders.payment_mode", "Payment"),
    ("wl_sales.orders.number", "Order"),
    ("wl_sales.orders.total", "Total"),
    ("wl_sales.orders.empty", "No sales orders"),
    ("wl_sales.orders.created", "Order entered."),
    ("wl_sales.orders.add_line", "Add a line"),
    ("wl_sales.orders.line_added", "Line added."),
    ("wl_sales.orders.submit", "Submit the order"),
    ("wl_sales.orders.submit_message", "The order is placed and its lines can no longer change."),
    ("wl_sales.orders.submitted", "Order submitted."),
    ("wl_sales.orders.fulfilled", "Handed over"),
    ("wl_sales.orders.reason", "Reason"),
    ("wl_sales.orders.payment_failed_reason", "The online payment failed."),
    ("wl_sales.orders.status.draft", "Draft"),
    ("wl_sales.orders.status.awaiting_payment", "Awaiting payment"),
    ("wl_sales.orders.status.pending_acceptance", "Pending acceptance"),
    ("wl_sales.orders.status.credit_hold", "On credit hold"),
    ("wl_sales.orders.status.confirmed", "Confirmed"),
    ("wl_sales.orders.status.partially_fulfilled", "Partially fulfilled"),
    ("wl_sales.orders.status.fulfilled", "Fulfilled"),
    ("wl_sales.orders.status.closed", "Closed"),
    ("wl_sales.orders.status.rejected", "Rejected"),
    ("wl_sales.orders.status.cancelled", "Cancelled"),
    ("wl_sales.orders.errors.number", "Give a whole number."),
    ("wl_sales.orders.errors.amount", "Give an amount such as 105.00."),
    ("wl_sales.orders.errors.item", "Give the item code."),
    ("wl_sales.orders.errors.choice", "Choose one of the offered values."),
    ("wl_sales.orders.problem.no_lines", "The order has no lines."),
    ("wl_sales.orders.problem.not_draft", "Only a draft order can change."),
    ("wl_sales.orders.problem.line", "Give a quantity above zero and a price of zero or more."),
    ("wl_sales.orders.problem.amount", "The amount paid is not the order total."),
    ("wl_sales.orders.problem.input", "The transition was run without what it needs."),
    (
        "wl_sales.orders.problem.quantity",
        "A quantity is not above zero, is repeated, or is more than is open or held.",
    ),
    (
        "wl_sales.orders.problem.completeness",
        "The order does not end in the state this transition leads to.",
    ),
    ("wl_sales.orders.problem.not_invoiced", "What was fulfilled is not invoiced yet."),
    ("wl_sales.orders.problem.unknown_line", "A line does not belong to the order."),
    ("wl_sales.orders.problem.unknown_source", "The order was not fulfilled from this hand-over."),
    ("wl_sales.orders.problem.no_quantities", "Give at least one quantity."),
    ("wl_sales.returns.title", "Sales returns"),
    ("wl_sales.returns.subtitle", "Goods that come back, and refunds after completion."),
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
    ("wl_sales.returns.receive_description", "The goods are back in quarantine."),
    ("wl_sales.returns.approved", "Accept outside the return window"),
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
    ("wl_sales.returns.resolution.refunded_by_channel", "Refunded by the channel"),
    ("wl_sales.returns.errors.choice", "Choose one of the offered values."),
    ("wl_sales.returns.errors.number", "Give a whole number."),
    ("wl_sales.returns.errors.lines", "Give the lines as line id:quantity, separated by commas."),
    ("wl_sales.returns.problem.no_lines", "Name at least one line to take back."),
    ("wl_sales.returns.problem.not_completed", "Only a completed counter sale can be returned."),
    (
        "wl_sales.returns.problem.not_handed_over",
        "Only an order that was handed over can be returned.",
    ),
    (
        "wl_sales.returns.problem.sale_type",
        "Only a counter sale or a sales order can be returned.",
    ),
    ("wl_sales.returns.problem.unknown_line", "A line does not belong to the sale."),
    (
        "wl_sales.returns.problem.quantity",
        "A quantity is not above zero, is repeated, or is more than the sale holds.",
    ),
    (
        "wl_sales.returns.problem.no_goods_to_receive",
        "A return with no goods has nothing to receive.",
    ),
    ("wl_sales.returns.problem.window", "The return window has passed; a manager must accept it."),
    ("wl_sales.returns.problem.resolution", "Choose how the return is resolved."),
    ("wl_sales.returns.problem.inspection", "Receive the goods and record the inspection first."),
    (
        "wl_sales.returns.problem.complaint",
        "Give the complaint category and the responsible party.",
    ),
    (
        "wl_sales.returns.problem.channel",
        "Only a return with no goods can be refunded by the channel.",
    ),
    ("wl_sales.returns.problem.credit_note", "Name the credit note that was issued."),
    (
        "wl_sales.returns.problem.not_by_channel",
        "Only a refund made by the channel can be reversed.",
    ),
    ("wl_sales.loyalty.title", "Loyalty"),
    ("wl_sales.loyalty.subtitle", "The loyalty programme, its members and their points."),
    ("wl_sales.loyalty.enrol", "Enrol a member"),
    (
        "wl_sales.loyalty.enrol_description",
        "Records the version of the terms the member accepted. It is not a marketing consent.",
    ),
    ("wl_sales.loyalty.party", "Member (party id)"),
    ("wl_sales.loyalty.mobile", "Mobile number"),
    ("wl_sales.loyalty.guardian", "Guardian (party id, for a minor)"),
    ("wl_sales.loyalty.accept_terms", "The member accepted the terms"),
    ("wl_sales.loyalty.turn_on", "Turn the programme on"),
    ("wl_sales.loyalty.name", "Name"),
    ("wl_sales.loyalty.earn_kind", "Earn (per_aed or per_visit)"),
    ("wl_sales.loyalty.earn_rate", "Points per AED, or visits per reward"),
    ("wl_sales.loyalty.minimum", "Minimum qualifying amount (AED, before VAT)"),
    ("wl_sales.loyalty.point_value", "Point value (AED)"),
    ("wl_sales.loyalty.validity", "Points valid for (months)"),
    ("wl_sales.loyalty.terms_version", "Terms version"),
    ("wl_sales.loyalty.terms_en", "Terms in English"),
    ("wl_sales.loyalty.terms_ar", "Terms in Arabic"),
    ("wl_sales.loyalty.no_programme", "There is no loyalty programme yet."),
    ("wl_sales.loyalty.accounts", "Members"),
    ("wl_sales.loyalty.code", "Code"),
    ("wl_sales.loyalty.balance", "Balance"),
    ("wl_sales.loyalty.no_accounts", "No members yet"),
    ("wl_sales.loyalty.status.active", "Active"),
    ("wl_sales.loyalty.status.closed", "Closed"),
    ("wl_sales.loyalty.programme_created", "Loyalty programme turned on."),
    ("wl_sales.loyalty.enrolled", "Member enrolled."),
    ("wl_sales.loyalty.earn.per_aed", "Points per AED 1: :rate"),
    ("wl_sales.loyalty.earn.per_visit", "A reward after :rate visits"),
    ("wl_sales.loyalty.errors.number", "Give a whole number."),
    ("wl_sales.loyalty.errors.amount", "Give an amount such as 0.01."),
    ("wl_sales.loyalty.errors.choice", "Choose one of the offered values."),
    ("wl_sales.loyalty.problem.name", "Give the programme a name."),
    (
        "wl_sales.loyalty.problem.terms",
        "The rate, the point value and the validity must be at least 1, and the minimum, the redemption terms and the share cannot be negative.",
    ),
    ("wl_sales.loyalty.problem.brand_taken", "This brand already has a loyalty programme."),
    ("wl_sales.loyalty.problem.no_programme", "There is no loyalty programme to enrol in."),
    ("wl_sales.loyalty.problem.terms_not_accepted", "The member must accept the terms."),
    ("wl_sales.loyalty.problem.mobile", "Give a mobile number with 8 to 15 digits."),
    (
        "wl_sales.loyalty.problem.already_member",
        "The party is already a member of this programme.",
    ),
    (
        "wl_sales.loyalty.problem.mobile_taken",
        "This mobile number belongs to another member of the programme.",
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
