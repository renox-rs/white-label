//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_parties.title", "Parties"),
    ("wl_parties.approver.finance_manager", "Finance manager"),
    ("wl_parties.approver.compliance", "Compliance"),
    ("wl_parties.new", "New party"),
    ("wl_parties.create", "Create party"),
    ("wl_parties.cancel", "Cancel"),
    ("wl_parties.created", "Party created."),
    ("wl_parties.empty", "No parties yet"),
    (
        "wl_parties.empty_hint",
        "Parties show up here once they are created.",
    ),
    ("wl_parties.sections.card", "Party"),
    ("wl_parties.sections.names", "Names"),
    ("wl_parties.sections.contact", "Primary contacts"),
    ("wl_parties.sections.contacts", "Contact points"),
    ("wl_parties.sections.roles", "Roles"),
    ("wl_parties.sections.history", "History"),
    ("wl_parties.no_contacts", "No contact points."),
    ("wl_parties.no_roles", "No roles."),
    ("wl_parties.no_history", "No changes yet."),
    ("wl_parties.history.change", "Change"),
    ("wl_parties.history.at", "When"),
    ("wl_parties.history.reason", "Reason"),
    ("wl_parties.fields.code", "Code"),
    ("wl_parties.fields.kind", "Kind"),
    ("wl_parties.fields.display_name", "Display name"),
    (
        "wl_parties.fields.display_name_hint",
        "Shown in lists; the English legal name when left blank.",
    ),
    ("wl_parties.fields.legal_name_en", "Legal name (English)"),
    ("wl_parties.fields.legal_name_ar", "Legal name (Arabic)"),
    ("wl_parties.fields.status", "Status"),
    ("wl_parties.fields.hold_reason", "Hold reason"),
    ("wl_parties.fields.default_language", "Language"),
    ("wl_parties.fields.preferred_channel", "Preferred channel"),
    ("wl_parties.fields.email", "Email"),
    ("wl_parties.fields.mobile", "Mobile"),
    ("wl_parties.fields.trn", "TRN"),
    (
        "wl_parties.fields.trn_hint",
        "15 digits, for a VAT-registered company.",
    ),
    ("wl_parties.fields.legal_form", "Legal form"),
    ("wl_parties.fields.trade_licence_no", "Trade licence no."),
    ("wl_parties.fields.licence_authority", "Licensing authority"),
    ("wl_parties.fields.vat_registered", "VAT registered"),
    ("wl_parties.fields.nationality", "Nationality"),
    ("wl_parties.fields.date_of_birth", "Date of birth"),
    ("wl_parties.fields.created_at", "Created"),
    ("wl_parties.fields.contact_type", "Type"),
    ("wl_parties.fields.value", "Address or number"),
    ("wl_parties.fields.label", "Label"),
    ("wl_parties.fields.is_primary", "Primary"),
    ("wl_parties.fields.verified_at", "Verified"),
    ("wl_parties.fields.role", "Role"),
    ("wl_parties.fields.since", "Since"),
    ("wl_parties.fields.until", "Until"),
    ("wl_parties.fields.external_reference", "External reference"),
    ("wl_parties.kind.person", "Person"),
    ("wl_parties.kind.company", "Company"),
    ("wl_parties.status.active", "Active"),
    ("wl_parties.status.blocked", "On hold"),
    ("wl_parties.status.archived", "Archived"),
    ("wl_parties.status.merged", "Merged"),
    ("wl_parties.language.en", "English"),
    ("wl_parties.language.ar", "Arabic"),
    ("wl_parties.channel.email", "Email"),
    ("wl_parties.channel.sms", "SMS"),
    ("wl_parties.channel.whatsapp", "WhatsApp"),
    ("wl_parties.contact_type.email", "Email"),
    ("wl_parties.contact_type.mobile", "Mobile"),
    ("wl_parties.contact_type.landline", "Landline"),
    ("wl_parties.contact_type.address", "Address"),
    ("wl_parties.reason.on_hold", "On hold"),
    ("wl_parties.reason.non_payment", "Non-payment"),
    ("wl_parties.reason.legal_dispute", "Legal dispute"),
    ("wl_parties.reason.fraud_suspected", "Fraud suspected"),
    ("wl_parties.reason.other", "Other"),
    ("wl_parties.reason.sanctions", "Sanctions"),
    ("wl_parties.reason.aml_concern", "AML concern"),
    ("wl_parties.block.action", "Block"),
    ("wl_parties.block.title", "Put the party on hold"),
    (
        "wl_parties.block.description",
        "New business with the party stops. Nothing is sent to the party.",
    ),
    ("wl_parties.block.submit", "Block party"),
    ("wl_parties.block.done", "The party is on hold."),
    ("wl_parties.block.system", "System"),
    ("wl_parties.block.open", "Open"),
    ("wl_parties.block.closed", "Lifted"),
    ("wl_parties.sections.block_records", "Block records"),
    ("wl_parties.fields.reason_code", "Reason"),
    ("wl_parties.fields.note", "Note"),
    ("wl_parties.fields.restricted", "Restricted"),
    ("wl_parties.fields.blocked_by", "Blocked by"),
    ("wl_parties.fields.blocked_at", "Blocked"),
    ("wl_parties.fields.unblocked_at", "Lifted"),
    ("wl_parties.fields.block_state", "State"),
    ("wl_parties.contact_status.valid", "Valid"),
    ("wl_parties.contact_status.invalid", "Invalid"),
    ("wl_parties.invalid_reason.hard_bounce", "Hard bounce"),
    ("wl_parties.invalid_reason.invalid_number", "Invalid number"),
    (
        "wl_parties.invalid_reason.unknown_account",
        "Unknown account",
    ),
    (
        "wl_parties.contact.needs_updating",
        "Contact needs updating",
    ),
    (
        "wl_parties.contact.needs_updating_hint",
        "Messages cannot reach this party at the contact points below. Update them or verify that they work.",
    ),
    ("wl_parties.contact.since", "since"),
    ("wl_parties.contact.update", "Update contact"),
    ("wl_parties.contact.actions", "Actions"),
    ("wl_parties.contact.edit", "Edit"),
    ("wl_parties.contact.edit_title", "Edit contact point"),
    (
        "wl_parties.contact.edit_description",
        "A new address or number makes the contact point valid again. Changing only the label keeps its status.",
    ),
    ("wl_parties.contact.save", "Save contact"),
    (
        "wl_parties.contact.updated",
        "The contact point is updated.",
    ),
    ("wl_parties.contact.verify", "Verify"),
    ("wl_parties.contact.verify_title", "Verify contact point"),
    (
        "wl_parties.contact.verify_description",
        "Confirm that this address or number works. The contact point becomes valid again.",
    ),
    (
        "wl_parties.contact.verified",
        "The contact point is verified.",
    ),
    ("wl_parties.fields.contact_status", "Status"),
    ("wl_parties.fields.value_edit", "Address or number"),
    ("wl_parties.screening.source_manual", "Manual upload"),
    ("wl_parties.screening.fields.uploaded_by", "Uploaded by"),
    ("wl_parties.screening.fields.source_url", "Source URL"),
    ("wl_parties.screening.upload_action", "Upload official file"),
    (
        "wl_parties.screening.upload_title",
        "Upload the official file",
    ),
    (
        "wl_parties.screening.upload_description",
        "Use this when the download keeps failing. Take the file from the authority's page and give that page and the date of the list.",
    ),
    ("wl_parties.screening.upload_submit", "Upload file"),
    (
        "wl_parties.screening.upload_done",
        "The list file is uploaded.",
    ),
    ("wl_parties.screening.upload_file", "Official file"),
    (
        "wl_parties.screening.upload_file_hint",
        "An XML file, at most 20 MB.",
    ),
    ("wl_parties.screening.upload_source_url", "Source URL"),
    (
        "wl_parties.screening.upload_list_date",
        "List date (YYYY-MM-DD)",
    ),
    (
        "wl_parties.errors.upload_file_required",
        "Choose the official file.",
    ),
    ("wl_parties.errors.manual_list_required", "Choose the list."),
    (
        "wl_parties.errors.manual_result_required",
        "Choose the result.",
    ),
    ("wl_parties.manual_check.action", "Record manual check"),
    ("wl_parties.manual_check.title", "Record a manual check"),
    (
        "wl_parties.manual_check.description",
        "Record that you checked this party by hand against the list file of the date below.",
    ),
    ("wl_parties.manual_check.submit", "Record check"),
    (
        "wl_parties.manual_check.done",
        "The manual check is recorded.",
    ),
    ("wl_parties.manual_check.list", "List"),
    (
        "wl_parties.manual_check.list_date",
        "List date (YYYY-MM-DD)",
    ),
    ("wl_parties.manual_check.result", "Result"),
    ("wl_parties.manual_check.name", "Name on the list"),
    ("wl_parties.manual_check.list_entry", "List entry"),
    (
        "wl_parties.manual_check.candidate_hint",
        "Only for a candidate.",
    ),
    ("wl_parties.manual_check.result.no_match", "No match"),
    (
        "wl_parties.manual_check.result.candidate",
        "Candidate found",
    ),
    ("wl_parties.unblock.action", "Unblock"),
    ("wl_parties.unblock.title", "Lift the hold"),
    (
        "wl_parties.unblock.description",
        "New business with the party can start again. Another authorised user must lift a hold.",
    ),
    ("wl_parties.unblock.submit", "Unblock party"),
    ("wl_parties.unblock.done", "The hold is lifted."),
    ("wl_parties.unblock_basis.delisted", "No longer listed"),
    ("wl_parties.unblock_basis.false_match", "Match found false"),
    ("wl_parties.fields.unblock_basis", "Basis"),
    ("wl_parties.fields.unblock_files", "Evidence files"),
    (
        "wl_parties.fields.unblock_files_hint",
        "PDF, JPG, PNG, HEIC, DOCX or XLSX, up to 20 MB each.",
    ),
    ("wl_parties.fields.unblocked_by", "Lifted by"),
    ("wl_parties.fields.unblock_note", "Lifted because"),
    ("wl_parties.fields.evidence", "Evidence"),
    ("wl_parties.block_effects.title", "Block effects"),
    (
        "wl_parties.block_effects.subtitle",
        "What a block does to each kind of agreement. The vertical sets these values.",
    ),
    ("wl_parties.block_effects.agreement", "Agreement"),
    ("wl_parties.block_effects.owner", "Owner"),
    ("wl_parties.block_effects.effect", "Effect"),
    ("wl_parties.block_effects.decider", "Decider role"),
    ("wl_parties.block_effects.source", "Set by"),
    ("wl_parties.block_effects.has_source", "Lists agreements"),
    ("wl_parties.block_effects.no_decider", "None"),
    ("wl_parties.effect.pause", "Pause"),
    ("wl_parties.effect.continue", "Continue"),
    ("wl_parties.effect.review", "Review"),
    ("wl_parties.rule_source.default", "Default"),
    ("wl_parties.rule_source.module", "Module"),
    ("wl_parties.rule_source.vertical", "Vertical"),
    ("wl_parties.block_kind.engagement", "Engagement"),
    ("wl_parties.block_kind.subscription", "Subscription"),
    ("wl_parties.block_kind.enrolment", "Enrolment"),
    ("wl_parties.block_kind.lease", "Lease"),
    ("wl_parties.block_kind.deployment", "Deployment"),
    ("wl_parties.block_kind.project", "Project"),
    ("wl_parties.block_kind.sales_order", "Sales order"),
    ("wl_parties.block_kind.booking", "Booking"),
    ("wl_parties.block_kind.purchase_order", "Purchase order"),
    ("wl_parties.block_kind.service_contract", "Service contract"),
    ("wl_parties.block_kind.money_out", "Money out"),
    ("wl_parties.block_kind.money_in", "Money in"),
    ("wl_parties.review.title", "Review tasks"),
    (
        "wl_parties.review.subtitle",
        "Agreements of parties on hold that wait for your decision.",
    ),
    ("wl_parties.review.empty", "No review tasks"),
    (
        "wl_parties.review.empty_hint",
        "Tasks show up here when a party is put on hold.",
    ),
    ("wl_parties.review.task", "Review task"),
    ("wl_parties.review.fields.party", "Party"),
    ("wl_parties.review.fields.agreement", "Agreement"),
    ("wl_parties.review.fields.scope", "Scope"),
    ("wl_parties.review.fields.status", "Status"),
    ("wl_parties.review.fields.pending", "Waiting"),
    ("wl_parties.review.fields.opened", "Opened"),
    ("wl_parties.review.fields.closed", "Closed"),
    ("wl_parties.review.fields.decision", "Decision"),
    ("wl_parties.review.fields.decided_by", "Decided by"),
    ("wl_parties.review.fields.decided_at", "Decided"),
    ("wl_parties.review.fields.note", "Note"),
    ("wl_parties.review.fields.by", "By"),
    ("wl_parties.review.fields.at", "When"),
    ("wl_parties.review.sections.items", "Agreements"),
    ("wl_parties.review.sections.notes", "Notes"),
    ("wl_parties.review.no_notes", "No notes."),
    ("wl_parties.review.no_items", "No agreements."),
    ("wl_parties.review.global_scope", "Everywhere"),
    ("wl_parties.review.system", "System"),
    ("wl_parties.review.party_unblocked", "Party unblocked"),
    ("wl_parties.review.status.open", "Open"),
    ("wl_parties.review.status.closed", "Closed"),
    ("wl_parties.review.decision.pending", "Waiting"),
    ("wl_parties.review.decision.pause", "Pause"),
    ("wl_parties.review.decision.continue", "Continue"),
    ("wl_parties.review.decision.end", "End"),
    ("wl_parties.review.decide", "Decide"),
    ("wl_parties.review.decide_title", "Decide on the agreement"),
    (
        "wl_parties.review.decide_description",
        "The agreement's own module carries it out. Nothing is sent to the party.",
    ),
    ("wl_parties.review.decide_submit", "Confirm decision"),
    ("wl_parties.review.decided", "Decision recorded."),
    (
        "wl_parties.review.refused",
        "The agreement was not changed: :reason",
    ),
    (
        "wl_parties.review.already_decided",
        "This agreement is already decided.",
    ),
    ("wl_parties.review.task_closed", "This task is closed."),
    (
        "wl_parties.review.no_source",
        "No module handles this kind of agreement.",
    ),
    (
        "wl_parties.review.no_decider",
        "No decider role is set for this kind; users with the block permission decide.",
    ),
    ("wl_parties.review.neutral", "Send neutral message"),
    (
        "wl_parties.review.neutral_title",
        "Send the neutral message to the party",
    ),
    (
        "wl_parties.review.neutral_description",
        "The party is told that the account is on hold and asked to contact the business. No reason is given.",
    ),
    ("wl_parties.review.neutral_submit", "Send message"),
    (
        "wl_parties.review.neutral_sent",
        "The neutral message is queued.",
    ),
    (
        "wl_parties.review.neutral_unavailable",
        "Notifications are not enabled, so nothing was sent.",
    ),
    (
        "wl_parties.review.neutral_note",
        "Neutral on-hold message sent to the party.",
    ),
    ("wl_parties.review.on_hold", "On hold"),
    (
        "wl_parties.review.on_hold_reason",
        "On hold \u{2014} :reason",
    ),
    (
        "wl_parties.review.notice_title",
        "A party is on hold: :count agreements to review",
    ),
    (
        "wl_parties.review.notice_title_one",
        "A party is on hold: 1 agreement to review",
    ),
    (
        "wl_parties.review.notice_body",
        "Open the review task to pause, continue or end each agreement.",
    ),
    (
        "wl_parties.errors.legal_name_required",
        "Enter the legal name in English.",
    ),
    (
        "wl_parties.errors.details_kind",
        "Person details are for persons and company details for companies.",
    ),
    ("wl_parties.errors.trn_invalid", "A TRN has 15 digits."),
    ("wl_parties.errors.role_required", "Choose a role."),
    (
        "wl_parties.errors.relationship_party_missing",
        "Choose an existing party.",
    ),
    (
        "wl_parties.errors.relationship_person",
        "A contact or guardian must be a person.",
    ),
    (
        "wl_parties.errors.relationship_self",
        "A party cannot be related to itself.",
    ),
    (
        "wl_parties.errors.relationship_dates",
        "The end must be after the start.",
    ),
    (
        "wl_parties.errors.contact_required",
        "Enter the contact's address or number.",
    ),
    (
        "wl_parties.errors.contact_email_invalid",
        "Enter a valid e-mail address.",
    ),
    (
        "wl_parties.errors.contact_mobile_invalid",
        "Enter the number in international format, e.g. +971501234567.",
    ),
    (
        "wl_parties.errors.one_primary",
        "Only one contact of each type can be primary.",
    ),
    ("wl_parties.errors.reason_unknown", "Choose a reason."),
    ("wl_parties.errors.note_required", "Write a note."),
    (
        "wl_parties.errors.compliance_only",
        "Only the compliance role can do this.",
    ),
    (
        "wl_parties.errors.freeze_time_required",
        "Enter when the funds were frozen.",
    ),
    (
        "wl_parties.errors.freeze_not_ready",
        "Every named party must be blocked or archived with an open sanctions record.",
    ),
    (
        "wl_parties.errors.report_details_required",
        "Enter the reference and the date reported.",
    ),
    (
        "wl_parties.errors.reason_restricted",
        "You cannot use this reason.",
    ),
    (
        "wl_parties.errors.unblock_other_user",
        "This hold must be lifted by another authorised user",
    ),
    (
        "wl_parties.errors.unblock_basis_required",
        "Choose why the hold is lifted.",
    ),
    (
        "wl_parties.errors.unblock_evidence_required",
        "Attach at least one evidence file.",
    ),
    (
        "wl_parties.errors.unblock_file_empty",
        "An evidence file is empty.",
    ),
    (
        "wl_parties.errors.unblock_file_large",
        "An evidence file is larger than 20 MB.",
    ),
    (
        "wl_parties.errors.screening_off",
        "Screening is not turned on.",
    ),
    (
        "wl_parties.errors.screening_clear",
        "A candidate is at or above the match threshold.",
    ),
    (
        "wl_parties.errors.screening_match",
        "No candidate is at or above the match threshold.",
    ),
    (
        "wl_parties.errors.screening_reviewer",
        "A manual check needs a reviewer who may review screenings.",
    ),
    (
        "wl_parties.errors.screening_entry",
        "A manual match needs the list entry of every candidate.",
    ),
    (
        "wl_parties.errors.screening_score",
        "A candidate needs a name, a list and a score from 0 to 1.",
    ),
    ("wl_parties.screening.title", "Screening tasks"),
    (
        "wl_parties.screening.subtitle",
        "Possible matches that wait for a compliance decision.",
    ),
    ("wl_parties.screening.empty", "No screening tasks"),
    (
        "wl_parties.screening.empty_hint",
        "Tasks show up here when a screening finds a possible match.",
    ),
    ("wl_parties.screening.task", "Screening task"),
    ("wl_parties.screening.fields.party", "Party"),
    ("wl_parties.screening.fields.code", "Party code"),
    ("wl_parties.screening.fields.status", "Status"),
    ("wl_parties.screening.fields.due", "Due"),
    ("wl_parties.screening.fields.flagged", "Flagged"),
    ("wl_parties.screening.fields.closed", "Closed"),
    ("wl_parties.screening.fields.candidates", "Candidates"),
    ("wl_parties.screening.fields.run_at", "Run"),
    ("wl_parties.screening.fields.trigger", "Trigger"),
    ("wl_parties.screening.fields.method", "Method"),
    ("wl_parties.screening.fields.stale", "Stale lists"),
    ("wl_parties.screening.fields.name", "Name on the list"),
    ("wl_parties.screening.fields.list", "List"),
    ("wl_parties.screening.fields.score", "Score"),
    ("wl_parties.screening.fields.entry", "List entry"),
    ("wl_parties.screening.sections.runs", "Screening runs"),
    ("wl_parties.screening.sections.candidates", "Candidates"),
    ("wl_parties.screening.no_runs", "No screening runs."),
    ("wl_parties.screening.no_candidates", "No candidates."),
    ("wl_parties.screening.status.open", "Open"),
    ("wl_parties.screening.status.closed", "Closed"),
    ("wl_parties.screening.trigger.onboarding", "Onboarding"),
    ("wl_parties.screening.trigger.list_update", "List update"),
    ("wl_parties.screening.trigger.review", "Review"),
    ("wl_parties.screening.trigger.manual", "Manual"),
    ("wl_parties.screening.method.automatic", "Automatic"),
    ("wl_parties.screening.method.manual", "Manual"),
    ("wl_parties.screening.yes", "Yes"),
    ("wl_parties.screening.no", "No"),
    ("wl_parties.screening.page_title", "Screening"),
    (
        "wl_parties.screening.page_subtitle",
        "Sanctions lists with their freshness and the latest screenings.",
    ),
    ("wl_parties.screening.sections.lists", "Sanctions lists"),
    ("wl_parties.screening.sections.latest", "Latest screenings"),
    ("wl_parties.screening.fields.list_name", "List"),
    ("wl_parties.screening.fields.source", "Source"),
    ("wl_parties.screening.fields.version", "Version"),
    (
        "wl_parties.screening.fields.updated",
        "Last successful update",
    ),
    ("wl_parties.screening.fields.error", "Last error"),
    ("wl_parties.screening.fields.lists_used", "Lists used"),
    ("wl_parties.screening.stale_badge", "Stale list"),
    ("wl_parties.screening.no_lists", "No sanctions lists"),
    ("wl_parties.screening.no_screenings", "No screenings yet"),
    ("wl_parties.screening.list_status.current", "Current"),
    ("wl_parties.screening.list_status.stale", "Stale"),
    ("wl_parties.screening.state.pending", "Pending"),
    ("wl_parties.screening.state.clear", "Clear"),
    (
        "wl_parties.screening.state.possible_match",
        "Possible match",
    ),
    (
        "wl_parties.screening.state.false_positive",
        "False positive",
    ),
    (
        "wl_parties.screening.state.confirmed_match",
        "Confirmed match",
    ),
    ("wl_parties.freshness.never", "never"),
    ("wl_parties.freshness.reason_old", "The update is too old"),
    (
        "wl_parties.freshness.notice_title",
        "Sanctions list out of date: :list",
    ),
    (
        "wl_parties.freshness.notice_body",
        "Last successful update: :updated (Asia/Dubai). Reason: :reason.",
    ),
    (
        "wl_parties.screening.notice_title",
        "A possible sanctions match needs review",
    ),
    (
        "wl_parties.screening.notice_body",
        "Party :code is due for review by :due (Asia/Dubai).",
    ),
    (
        "wl_parties.errors.review_note_required",
        "Write a review note to confirm the match.",
    ),
    (
        "wl_parties.confirm.default_note",
        "Match confirmed by the compliance review.",
    ),
    (
        "wl_parties.confirm.notice_title",
        "Freeze and report tasks opened",
    ),
    (
        "wl_parties.confirm.notice_body",
        "Open the compliance tasks to see what is due.",
    ),
    (
        "wl_parties.errors.funds_closed",
        "Funds can only be changed on an open freeze task.",
    ),
    (
        "wl_parties.errors.funds_label_required",
        "Write what the money is.",
    ),
    (
        "wl_parties.errors.amount_invalid",
        "Enter an amount such as 1250.50.",
    ),
    ("wl_parties.confirm.action", "Confirm match"),
    ("wl_parties.confirm.title", "Confirm the sanctions match"),
    (
        "wl_parties.confirm.description",
        "The party and its related companies are put on hold, and freeze and report tasks are opened. A second reviewer may have to approve first.",
    ),
    ("wl_parties.confirm.submit", "Confirm match"),
    ("wl_parties.confirm.note", "Review note"),
    ("wl_parties.confirm.done", "Match confirmed."),
    (
        "wl_parties.confirm.requested",
        "Sent to a second reviewer for approval.",
    ),
    (
        "wl_parties.confirm.not_possible",
        "This screening is no longer a possible match.",
    ),
    (
        "wl_parties.confirm.waiting",
        "Waiting for a second reviewer",
    ),
    (
        "wl_parties.confirm.waiting_hint",
        "The match is confirmed once another reviewer approves it.",
    ),
    ("wl_parties.confirm.follow_up", "Freeze and report tasks"),
    ("wl_parties.tasks.title", "Freeze and report tasks"),
    (
        "wl_parties.tasks.subtitle",
        "Opened by a confirmed sanctions match. Compliance only.",
    ),
    ("wl_parties.tasks.empty", "No freeze or report tasks"),
    (
        "wl_parties.tasks.empty_hint",
        "Tasks show up here when a sanctions match is confirmed.",
    ),
    ("wl_parties.tasks.task", "Compliance task"),
    ("wl_parties.tasks.kind.freeze", "Freeze funds"),
    ("wl_parties.tasks.kind.report", "Report the match"),
    ("wl_parties.tasks.status.open", "Open"),
    ("wl_parties.tasks.status.done", "Done"),
    ("wl_parties.tasks.overdue", "Overdue"),
    ("wl_parties.tasks.fields.kind", "Task"),
    ("wl_parties.tasks.fields.due", "Due"),
    ("wl_parties.tasks.fields.status", "Status"),
    ("wl_parties.tasks.fields.parties", "Parties"),
    ("wl_parties.tasks.fields.party", "Party"),
    ("wl_parties.tasks.fields.code", "Party code"),
    ("wl_parties.tasks.fields.standing", "When confirmed"),
    ("wl_parties.tasks.fields.state", "Now"),
    ("wl_parties.tasks.fields.record", "Sanctions record"),
    ("wl_parties.tasks.fields.completed", "Completed"),
    ("wl_parties.tasks.fields.frozen_at", "Frozen at"),
    ("wl_parties.tasks.fields.reference", "Report reference"),
    ("wl_parties.tasks.fields.reported_on", "Date reported"),
    ("wl_parties.tasks.fields.label", "Money"),
    ("wl_parties.tasks.fields.kind_of_money", "Kind"),
    ("wl_parties.tasks.fields.amount", "Amount"),
    ("wl_parties.tasks.fields.currency", "Currency"),
    ("wl_parties.tasks.fields.confirmed", "Confirmed"),
    ("wl_parties.tasks.fields.note", "Note"),
    ("wl_parties.tasks.standing.blocked", "Blocked now"),
    ("wl_parties.tasks.standing.archived", "Archived"),
    (
        "wl_parties.tasks.standing.blocked_already",
        "Already blocked",
    ),
    ("wl_parties.tasks.state.held", "On hold"),
    ("wl_parties.tasks.state.archived", "Archived"),
    ("wl_parties.tasks.state.open", "Not on hold"),
    ("wl_parties.tasks.yes", "Yes"),
    ("wl_parties.tasks.no", "No"),
    ("wl_parties.tasks.sections.parties", "Parties named"),
    ("wl_parties.tasks.sections.funds", "Funds held"),
    (
        "wl_parties.tasks.no_funds",
        "No module reported funds. Add a line if money is held.",
    ),
    ("wl_parties.tasks.no_parties", "No parties named."),
    (
        "wl_parties.tasks.not_ready",
        "A named party is not yet on hold with a sanctions record, so the freeze cannot be recorded.",
    ),
    ("wl_parties.tasks.edit", "Edit line"),
    ("wl_parties.tasks.edit_title", "Edit the funds line"),
    ("wl_parties.tasks.edit_submit", "Save line"),
    ("wl_parties.tasks.line_saved", "Line saved."),
    ("wl_parties.tasks.add", "Add line"),
    ("wl_parties.tasks.add_title", "Add a funds line"),
    ("wl_parties.tasks.add_submit", "Add line"),
    ("wl_parties.tasks.line_added", "Line added."),
    ("wl_parties.tasks.freeze_action", "Record freeze"),
    ("wl_parties.tasks.freeze_title", "Record the freeze"),
    (
        "wl_parties.tasks.freeze_description",
        "Leave the time empty to use now (Asia/Dubai). This completes the task.",
    ),
    ("wl_parties.tasks.freeze_submit", "Record freeze"),
    ("wl_parties.tasks.report_action", "Record report"),
    ("wl_parties.tasks.report_title", "Record the report"),
    (
        "wl_parties.tasks.report_description",
        "Enter the reference of the filing and the date it was made. This completes the task.",
    ),
    ("wl_parties.tasks.report_submit", "Record report"),
    ("wl_parties.tasks.completed", "Task completed."),
    (
        "wl_parties.tasks.already_done",
        "This task is already done.",
    ),
    (
        "wl_parties.tasks.notice_hint",
        "Times are shown in Asia/Dubai.",
    ),
    ("wl_parties.fields.einvoice_recipient_id", "E-invoicing recipient ID"),
    (
        "wl_parties.fields.einvoice_recipient_id_hint",
        "Peppol participant ID, scheme and identifier, e.g. 0235:100123456700003.",
    ),
    ("wl_parties.einvoice.lookup", "Look up"),
    ("wl_parties.einvoice.save", "Save"),
    (
        "wl_parties.einvoice.found",
        "Found :id. Check it and save to keep it.",
    ),
    (
        "wl_parties.einvoice.not_found",
        "No e-invoicing ID found for this TRN; enter it by hand",
    ),
    (
        "wl_parties.einvoice.lookup_failed",
        "The lookup did not work; enter the ID by hand",
    ),
    (
        "wl_parties.einvoice.unavailable",
        "The lookup is not available now; enter the ID by hand",
    ),
    ("wl_parties.einvoice.saved", "E-invoicing recipient ID saved."),
    (
        "wl_parties.errors.einvoice_person",
        "Only a company has an e-invoicing recipient ID.",
    ),
    (
        "wl_parties.errors.einvoice_lookup_trn",
        "Enter the company's 15-digit TRN to look up its e-invoicing ID.",
    ),
    (
        "wl_parties.errors.unblock_file_type",
        "Evidence files must be PDF, JPG, PNG, HEIC, DOCX or XLSX.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key)
}

/// Like [`t`], filling the `:name` placeholders with `params`.
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn std::fmt::Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    let text = english(key);
    let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
    renox::i18n::format(&text, &params, None)
}

/// The package's English text for `key`, or the key when it has none.
pub(crate) fn english(key: &str) -> String {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or_else(|| key.to_owned(), |(_, text)| (*text).to_owned())
}

/// Every text of the package in `lang`, keyed without the `wl_parties.`
/// prefix (`fields.code`), for the templates: `tx['fields.code']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_parties."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TEXTS;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_parties.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }
}
