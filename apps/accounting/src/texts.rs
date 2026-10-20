//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("accounting.button_cancel", "Cancel"),
    ("accounting.none", "—"),
    ("accounting.fields.status", "Status"),
    // The engagements.
    ("accounting.engagements.title", "Engagements"),
    (
        "accounting.engagements.subtitle",
        "The firm's clients and the services it runs for them.",
    ),
    ("accounting.engagements.new", "New engagement"),
    (
        "accounting.engagements.new_description",
        "The client company (a party), the services and the fee. It starts in onboarding.",
    ),
    (
        "accounting.engagements.client_party",
        "Client (party number)",
    ),
    ("accounting.engagements.client", "Client"),
    ("accounting.engagements.services", "Services"),
    ("accounting.engagements.fee_basis", "Fee basis"),
    ("accounting.engagements.fee_basis.retainer", "Retainer"),
    ("accounting.engagements.fee_basis.fixed", "Fixed fee"),
    (
        "accounting.engagements.fee_basis.time",
        "Time and materials",
    ),
    (
        "accounting.engagements.retainer_fee",
        "Monthly retainer (AED)",
    ),
    ("accounting.engagements.number", "Engagement"),
    ("accounting.engagements.reason", "Reason"),
    ("accounting.engagements.empty", "No engagements yet"),
    ("accounting.engagements.created", "Engagement created."),
    ("accounting.engagements.details", "Engagement"),
    ("accounting.engagements.start", "Start"),
    ("accounting.engagements.letter", "Engagement letter"),
    ("accounting.engagements.letter_signed", "Signed"),
    ("accounting.engagements.letter_signed_on", "Signed on :date"),
    ("accounting.engagements.letter_not_signed", "Not signed"),
    (
        "accounting.engagements.record_letter",
        "Record signed letter",
    ),
    (
        "accounting.engagements.record_letter_description",
        "The day the client signed the engagement letter.",
    ),
    ("accounting.engagements.signed_on", "Signed on"),
    (
        "accounting.engagements.letter_recorded",
        "Signed letter recorded.",
    ),
    ("accounting.engagements.tax_profile", "Tax profile"),
    ("accounting.engagements.tax_profile_none", "None recorded"),
    ("accounting.engagements.tax_profile_complete", "Complete"),
    ("accounting.engagements.activated_on", "Activated on"),
    ("accounting.engagements.ended_on", "Ended on"),
    ("accounting.engagements.cdd", "CDD file"),
    ("accounting.engagements.open_cdd", "Open the CDD file"),
    ("accounting.engagements.activate", "Activate"),
    (
        "accounting.engagements.activate_message",
        "The compliance calendar is generated and the retainer starts.",
    ),
    ("accounting.engagements.activated", "Engagement activated."),
    ("accounting.engagements.decline", "Decline"),
    ("accounting.engagements.declined", "Engagement declined."),
    ("accounting.engagements.suspend", "Suspend"),
    ("accounting.engagements.suspended", "Engagement suspended."),
    ("accounting.engagements.resume", "Resume"),
    (
        "accounting.engagements.resume_message",
        "The engagement runs again: its tasks are released, the retainer resumes and the request lists of the periods skipped are created. Say what changed or what the client asked.",
    ),
    ("accounting.engagements.resumed", "Engagement resumed."),
    ("accounting.engagements.resume_note", "Note"),
    (
        "accounting.engagements.resume_pending",
        "The resume waits for the MLRO's approval.",
    ),
    (
        "accounting.engagements.resume_needs_approval",
        "The client was on hold: the resume needs the MLRO's approval.",
    ),
    (
        "accounting.engagements.overdue_invoices",
        "Overdue invoices",
    ),
    ("accounting.engagements.invoice_number", "Invoice"),
    ("accounting.engagements.invoice_due", "Due"),
    ("accounting.engagements.invoice_balance", "Overdue"),
    ("accounting.engagements.overdue_empty", "No overdue invoice"),
    ("accounting.engagements.end", "End"),
    ("accounting.engagements.ended", "Engagement ended."),
    (
        "accounting.engagements.reason_description",
        "Choose the reason. It is shown to staff as it reads here.",
    ),
    ("accounting.engagements.status.onboarding", "Onboarding"),
    ("accounting.engagements.status.active", "Active"),
    ("accounting.engagements.status.suspended", "Suspended"),
    ("accounting.engagements.status.ended", "Ended"),
    ("accounting.engagements.status.declined", "Declined"),
    ("accounting.engagements.transition.activate", "Activated"),
    ("accounting.engagements.transition.decline", "Declined"),
    ("accounting.engagements.transition.auto_decline", "Declined"),
    ("accounting.engagements.transition.suspend", "Suspended"),
    (
        "accounting.engagements.transition.auto_suspend",
        "Suspended",
    ),
    ("accounting.engagements.transition.resume", "Resumed"),
    (
        "accounting.engagements.transition.resume_approved",
        "Resumed (approved)",
    ),
    ("accounting.engagements.transition.end", "Ended"),
    ("accounting.engagements.transition.auto_end", "Ended"),
    (
        "accounting.engagements.block_record",
        "Open the client's Block record",
    ),
    // The neutral templates (`ACC.ACL.14`).
    ("accounting.client_notices.title", "Told to the client"),
    ("accounting.client_notices.template", "Template"),
    (
        "accounting.client_notices.empty",
        "Nothing told to the client",
    ),
    ("accounting.client_notices.send", "Tell the client"),
    ("accounting.client_notices.sent", "The client is told."),
    (
        "accounting.client_notices.wrong_template",
        "This template does not fit the engagement's status.",
    ),
    (
        "accounting.client_notices.on_hold_subject",
        "Your engagement {{ number }}",
    ),
    (
        "accounting.client_notices.on_hold_body",
        "Your engagement is on hold, please contact your partner.",
    ),
    (
        "accounting.client_notices.ended_subject",
        "Your engagement {{ number }}",
    ),
    (
        "accounting.client_notices.ended_body",
        "Our engagement has ended.",
    ),
    // The portal.
    ("accounting.portal.title", "Engagements"),
    ("accounting.portal.status.onboarding", "Onboarding"),
    ("accounting.portal.status.active", "Active"),
    ("accounting.portal.status.suspended", "On hold"),
    ("accounting.portal.status.ended", "Ended"),
    ("accounting.portal.status.declined", "Declined"),
    // The history.
    ("accounting.history.title", "History"),
    ("accounting.history.at", "When"),
    ("accounting.history.action", "What"),
    ("accounting.history.by", "By"),
    ("accounting.history.system", "System"),
    ("accounting.history.empty", "Nothing happened yet"),
    // The neutral reasons (`ACC.ACL.14`).
    ("accounting.reasons.on_hold", "On hold"),
    ("accounting.reasons.relationship_ended", "Ended"),
    ("accounting.reasons.non_payment", "Non-payment"),
    ("accounting.reasons.client_request", "Client request"),
    ("accounting.reasons.other", "Other"),
    ("accounting.reasons.letter_not_signed", "Letter not signed"),
    ("accounting.reasons.client_withdrew", "Client withdrew"),
    ("accounting.reasons.not_accepted", "Declined"),
    ("accounting.reasons.not_accepted_option", "CDD rejected"),
    // Activation.
    (
        "accounting.activation.refused",
        "This engagement cannot be activated:",
    ),
    (
        "accounting.activation.cdd_not_approved",
        "The CDD file is not approved: the MLRO approves it first",
    ),
    (
        "accounting.activation.letter_not_signed",
        "The engagement letter is not signed: record the signed letter",
    ),
    (
        "accounting.activation.no_tax_profile",
        "No tax profile: record the client's tax profile",
    ),
    (
        "accounting.activation.tax_profile_incomplete",
        "The tax profile is incomplete: :missing",
    ),
    ("accounting.activation.document", ":party — :problem"),
    (
        "accounting.activation.on_hold",
        "On hold: the client cannot be served while on hold",
    ),
    // Screening (`PTY.RUL.02`).
    (
        "accounting.screening.stale",
        ":refusal — wait for the list update and a re-run of the screening",
    ),
    ("accounting.screening.pending", "Screening pending"),
    (
        "accounting.screening.possible_match",
        "Possible match on :party (:role): the MLRO decides it",
    ),
    (
        "accounting.screening.not_screened",
        ":party (:role) is not screened yet",
    ),
    // The tax profile.
    (
        "accounting.tax_profile.financial_year_end",
        "financial year end",
    ),
    (
        "accounting.tax_profile.ct_registration_no",
        "corporate tax registration number",
    ),
    ("accounting.tax_profile.trn", "VAT TRN (15 digits)"),
    (
        "accounting.tax_profile.vat_period",
        "VAT period and its first period end",
    ),
    // The calendar and the retainer.
    (
        "accounting.calendar.vat_return",
        "VAT return, period ending :end",
    ),
    (
        "accounting.calendar.ct_return",
        "Corporate tax return, year ending :end",
    ),
    ("accounting.retainer.plan", ":title retainer"),
    // Problems.
    (
        "accounting.problems.reason_unknown",
        "Choose one of the reasons offered.",
    ),
    ("accounting.problems.reason_required", "Give the reason."),
    (
        "accounting.problems.resume_note",
        "Give a note: what changed, or what the client asked.",
    ),
    (
        "accounting.problems.client_on_hold",
        "The client is on hold.",
    ),
    ("accounting.problems.title_required", "Name the services."),
    (
        "accounting.problems.fee_negative",
        "The retainer cannot be negative.",
    ),
    (
        "accounting.problems.client_unknown",
        "The client must be a known party.",
    ),
    (
        "accounting.problems.not_onboarding",
        "Only an engagement in onboarding takes this.",
    ),
    (
        "accounting.problems.cdd_locked",
        "The CDD file is no longer in progress.",
    ),
    // The CDD file.
    ("accounting.cdd.title", "CDD file"),
    ("accounting.cdd.status.in_progress", "In progress"),
    ("accounting.cdd.status.pending_approval", "Pending approval"),
    ("accounting.cdd.status.approved", "Approved"),
    ("accounting.cdd.status.review_due", "Review due"),
    ("accounting.cdd.status.rejected", "Rejected"),
    ("accounting.cdd.status.exited", "Exited"),
    ("accounting.cdd.role.client", "Client company"),
    ("accounting.cdd.role.ubo", "UBO"),
    ("accounting.cdd.role.director", "Director"),
    ("accounting.cdd.role.signatory", "Signatory"),
    ("accounting.cdd.rating", "Risk rating"),
    ("accounting.cdd.rating.unrated", "Not rated"),
    ("accounting.cdd.rating.low", "Low"),
    ("accounting.cdd.rating.medium", "Medium"),
    ("accounting.cdd.rating.high", "High"),
    ("accounting.cdd.risk_factors", "Risk factors"),
    ("accounting.cdd.edd_notes", "EDD notes"),
    ("accounting.cdd.documents", "Identity documents"),
    ("accounting.cdd.documents_attached", "Attached"),
    ("accounting.cdd.documents_missing", "Not attached"),
    ("accounting.cdd.next_review", "Next review"),
    ("accounting.cdd.parties", "Screened parties"),
    ("accounting.cdd.parties_empty", "No party yet"),
    ("accounting.cdd.party", "Party"),
    ("accounting.cdd.party_role", "Role"),
    ("accounting.cdd.add_party", "Add party"),
    (
        "accounting.cdd.add_party_description",
        "A UBO, director or signatory of the client (a party number).",
    ),
    ("accounting.cdd.assess", "Assess"),
    (
        "accounting.cdd.assess_description",
        "Rate the risk and note the factors.",
    ),
    ("accounting.cdd.submit", "Submit for approval"),
    (
        "accounting.cdd.submit_message",
        "The MLRO is asked to approve the file.",
    ),
    ("accounting.cdd.approve", "Approve"),
    (
        "accounting.cdd.approve_message",
        "The client may be served; the next review is set from the rating.",
    ),
    ("accounting.cdd.reject", "Reject"),
    ("accounting.cdd.reopen", "Reopen"),
    (
        "accounting.cdd.reopen_message",
        "The file goes back to in progress.",
    ),
    ("accounting.cdd.exit", "Exit client"),
    ("accounting.cdd.reason", "Reason (MLRO only)"),
    (
        "accounting.cdd.reason_description",
        "Only the MLRO and deputy see this reason.",
    ),
    ("accounting.cdd.opened", "CDD file opened."),
    ("accounting.cdd.party_added", "Party added."),
    ("accounting.cdd.assessed", "Assessment recorded."),
    ("accounting.cdd.submitted", "CDD file submitted."),
    ("accounting.cdd.approved", "CDD file approved."),
    ("accounting.cdd.rejected", "CDD file rejected."),
    ("accounting.cdd.reopened", "CDD file reopened."),
    ("accounting.cdd.exited", "Client exited."),
    (
        "accounting.cdd.submit_refused",
        "This CDD file cannot be submitted:",
    ),
    (
        "accounting.cdd.approve_refused",
        "This CDD file cannot be approved:",
    ),
    (
        "accounting.cdd.no_ubos",
        "No UBO is identified: add the client's UBOs",
    ),
    (
        "accounting.cdd.no_documents",
        "The identity documents are not attached",
    ),
    ("accounting.cdd.not_rated", "The risk is not rated"),
    // The compliance calendar on the engagement page (BL-ACC-03).
    ("accounting.tasks.title", "Compliance calendar"),
    ("accounting.tasks.task", "Task"),
    ("accounting.tasks.due", "Due"),
    ("accounting.tasks.held", "Held"),
    ("accounting.tasks.held_column", "Held"),
    ("accounting.tasks.empty", "No deadline task"),
    ("accounting.tasks.status.upcoming", "Upcoming"),
    ("accounting.tasks.status.in_progress", "In progress"),
    ("accounting.tasks.status.waiting", "Waiting on client"),
    ("accounting.tasks.status.in_review", "In review"),
    ("accounting.tasks.status.overdue", "Overdue"),
    ("accounting.tasks.status.completed", "Completed"),
    ("accounting.tasks.status.missed", "Missed"),
    ("accounting.tasks.status.not_required", "Not required"),
    // The reasons a deadline task is closed as missed (BL-ACC-13).
    (
        "accounting.tasks.missed.client_no_documents",
        "Client did not provide documents",
    ),
    (
        "accounting.tasks.missed.client_filed_itself",
        "Client filed itself",
    ),
    (
        "accounting.tasks.missed.engagement_ended",
        "Engagement ended",
    ),
    ("accounting.tasks.missed.other", "Other"),
    // The CDD update task of an expired KYC document (BL-ACC-12).
    (
        "accounting.cdd_update.title",
        "Update the CDD file: :kind of :party expired",
    ),
    (
        "accounting.cdd_update.description",
        "The :kind of :party (client :client, :engagement) expired on :expired. Collect the renewed document and update the CDD file. The engagement stays active.",
    ),
    (
        "accounting.cdd_update.completed",
        "The renewed :kind was accepted.",
    ),
    (
        "accounting.cdd_update.notice_subject",
        "CDD update for {{ number }}: {{ kind }} expired",
    ),
    (
        "accounting.cdd_update.notice_body",
        "The {{ kind }} of {{ party }} (client {{ client }}) expired on {{ expired }}. The compliance officer has the CDD update task {{ task }}, due {{ due }}; you may need to ask the client for the renewed document.",
    ),
    // The document request lists (ACC.RUL.02, POR.RUL.01).
    (
        "accounting.request_lists.notice_subject",
        "Documents requested for {{ number }}",
    ),
    (
        "accounting.request_lists.notice_body",
        "Please upload the documents for the periods ending {{ periods }} by {{ due }}.",
    ),
    (
        "accounting.request_lists.reminder_subject",
        "Reminder: documents for {{ number }}",
    ),
    (
        "accounting.request_lists.reminder_body",
        "We are still waiting for the documents for the period ending {{ period }}, due {{ due }}.",
    ),
    // The statutory deadline notice of a suspension or an end (BL-ACC-04).
    ("accounting.deadline_notices.title", "Statutory deadlines"),
    ("accounting.deadline_notices.deadlines", "Deadlines"),
    ("accounting.deadline_notices.deadline", "Deadline"),
    (
        "accounting.deadline_notices.none",
        "There are no statutory deadlines in the next :days days.",
    ),
    ("accounting.deadline_notices.line", ":date — :title"),
    (
        "accounting.deadline_notices.wording_suspend",
        "Your engagement :number is suspended. These are the statutory deadlines that fall due in the next :days days: the firm will not act on these while the engagement is suspended.",
    ),
    (
        "accounting.deadline_notices.wording_end",
        "Your engagement :number has ended. These are the statutory deadlines that fall due in the next :days days: the firm no longer acts on these.",
    ),
    ("accounting.deadline_notices.acknowledge", "Acknowledge"),
    (
        "accounting.deadline_notices.acknowledge_message",
        "Confirm that you have read the statutory deadlines and that the firm will not act on them.",
    ),
    (
        "accounting.deadline_notices.acknowledged",
        "Acknowledged on :at.",
    ),
    (
        "accounting.deadline_notices.acknowledged_toast",
        "Thank you: your acknowledgement is recorded.",
    ),
    (
        "accounting.deadline_notices.acknowledged_note",
        "The client approver acknowledged the deadline notice.",
    ),
    (
        "accounting.deadline_notices.sent",
        "The deadline notice was sent to the client.",
    ),
    (
        "accounting.deadline_notices.cancelled",
        "The engagement resumed: the deadline notice needs no more follow-up.",
    ),
    (
        "accounting.deadline_notices.not_prepared",
        "This deadline notice is not waiting to be sent.",
    ),
    (
        "accounting.deadline_notices.send",
        "Send the deadline notice",
    ),
    (
        "accounting.deadline_notices.send_message",
        "The client approvers get the statutory deadlines listed and are asked to acknowledge them in the portal.",
    ),
    ("accounting.deadline_notices.list", "Deadline notices"),
    ("accounting.deadline_notices.empty", "No deadline notice"),
    ("accounting.deadline_notices.cause", "After"),
    ("accounting.deadline_notices.cause.suspend", "Suspension"),
    ("accounting.deadline_notices.cause.end", "End"),
    ("accounting.deadline_notices.state", "Status"),
    ("accounting.deadline_notices.state.prepared", "To send"),
    ("accounting.deadline_notices.state.sent", "Sent"),
    ("accounting.deadline_notices.state.delivered", "Delivered"),
    (
        "accounting.deadline_notices.state.acknowledged",
        "Acknowledged",
    ),
    ("accounting.deadline_notices.state.cancelled", "Cancelled"),
    (
        "accounting.deadline_notices.notice_subject",
        "Statutory deadlines for {{ number }}",
    ),
    (
        "accounting.deadline_notices.notice_body",
        "{{ wording }}\n\n{{ lines }}\n\nPlease acknowledge this notice in the portal: {{ link }}",
    ),
    (
        "accounting.deadline_notices.reminder_subject",
        "Reminder: statutory deadlines for {{ number }}",
    ),
    (
        "accounting.deadline_notices.reminder_body",
        "We have not received your acknowledgement yet.\n\n{{ wording }}\n\n{{ lines }}\n\nPlease acknowledge this notice in the portal: {{ link }}",
    ),
    (
        "accounting.deadline_notices.prepared_title",
        "Deadline notice of :number: send it when you decide",
    ),
    (
        "accounting.deadline_notices.prepared_description",
        "Nothing was sent to the client automatically. When you decide, send the statutory deadline notice of :number from :link with the neutral template below.\n\nStatutory deadlines due by :until:\n:lines\n\nNeutral template: :template",
    ),
    (
        "accounting.deadline_notices.escalation_title",
        "No acknowledgement of the deadline notice of :number",
    ),
    (
        "accounting.deadline_notices.escalation_description",
        "The client approvers have not acknowledged the statutory deadline notice of :number, delivered on :delivered, after :days days. Contact the client.\n\n:lines",
    ),
    ("accounting.reasons.engagement_ended", "Engagement ended"),
    (
        "accounting.reasons.client_files_itself",
        "Client files itself",
    ),
    ("accounting.reasons.not_required", "Not required"),
    ("accounting.returns.title", "Returns"),
    (
        "accounting.returns.subtitle",
        "VAT and corporate tax returns of the firm's clients, from preparation to filing.",
    ),
    ("accounting.returns.new", "New return"),
    (
        "accounting.returns.new_description",
        "The engagement it is prepared under, the tax, the period it covers and the day it is due. It starts in preparation and is linked to the calendar task of the same type and due date.",
    ),
    (
        "accounting.returns.engagement_number",
        "Engagement (number)",
    ),
    ("accounting.returns.engagement", "Engagement"),
    ("accounting.returns.kind", "Tax"),
    ("accounting.returns.period", "Period ending"),
    ("accounting.returns.due", "Due"),
    ("accounting.returns.number", "Return"),
    ("accounting.returns.client", "Client"),
    ("accounting.returns.version", "Version"),
    ("accounting.returns.empty", "No returns yet"),
    ("accounting.returns.created", "Return opened."),
    ("accounting.returns.details", "Return"),
    ("accounting.returns.preparer", "Preparer"),
    ("accounting.returns.reviewer", "Reviewer"),
    ("accounting.returns.figures", "Figures"),
    ("accounting.returns.figures_action", "Enter figures"),
    (
        "accounting.returns.figures_description",
        "The figures of the period, and whether the workpapers are attached.",
    ),
    ("accounting.returns.figures_recorded", "Figures recorded."),
    ("accounting.returns.workpapers", "Workpapers"),
    (
        "accounting.returns.workpapers_attached",
        "Workpapers attached",
    ),
    ("accounting.returns.workpapers_missing", "Not attached"),
    ("accounting.returns.task", "Calendar task"),
    ("accounting.returns.cancel_reason", "Cancelled because"),
    ("accounting.returns.filed_on", "Filed on"),
    ("accounting.returns.filed_on_date", "Filed on :date."),
    ("accounting.returns.acknowledgement", "FTA acknowledgement"),
    (
        "accounting.returns.registration",
        "Tax agent registration number",
    ),
    (
        "accounting.returns.by_client",
        "Filed by the client through its own EmaraTax user",
    ),
    ("accounting.returns.signatures", "Signatures"),
    ("accounting.returns.signatures_empty", "Not approved yet"),
    ("accounting.returns.signer", "Signed by"),
    ("accounting.returns.ip", "IP address"),
    ("accounting.returns.hash", "Document hash"),
    ("accounting.returns.signature_valid", "Valid"),
    (
        "accounting.returns.signature_withdrawn",
        "Withdrawn on :at: :reason",
    ),
    ("accounting.returns.reason", "Reason"),
    ("accounting.returns.submit", "Submit for review"),
    (
        "accounting.returns.submit_message",
        "The reviewer is told. The figures and the workpapers must be in.",
    ),
    (
        "accounting.returns.submitted",
        "Return submitted for review.",
    ),
    ("accounting.returns.review", "Send to the client"),
    (
        "accounting.returns.review_message",
        "The client approver gets the summary to approve in the portal.",
    ),
    ("accounting.returns.reviewed", "Return sent to the client."),
    ("accounting.returns.send_back", "Send back"),
    (
        "accounting.returns.sent_back",
        "Return sent back to preparation.",
    ),
    ("accounting.returns.comment", "Comment"),
    (
        "accounting.returns.comment_description",
        "Say what has to change.",
    ),
    ("accounting.returns.withdraw", "Withdraw approval"),
    (
        "accounting.returns.withdraw_description",
        "On the client's written instruction. Attach it: the return goes back to preparation and the signature stays, marked withdrawn.",
    ),
    (
        "accounting.returns.portal_withdraw_description",
        "The return goes back to preparation. Your signature stays on record, marked as withdrawn.",
    ),
    (
        "accounting.returns.withdraw_reason",
        "Why the approval is withdrawn",
    ),
    (
        "accounting.returns.instruction",
        "The client's written instruction (attach or paste it)",
    ),
    ("accounting.returns.withdrawn", "Approval withdrawn."),
    ("accounting.returns.cancel", "Cancel return"),
    (
        "accounting.returns.cancel_description",
        "The firm will not file this return. The linked calendar task is waived, handed over or listed for the manager.",
    ),
    ("accounting.returns.cancelled", "Return cancelled."),
    ("accounting.returns.file", "Mark as filed"),
    (
        "accounting.returns.file_description",
        "The FTA acknowledgement of the filing on EmaraTax.",
    ),
    ("accounting.returns.filed", "Return marked as filed."),
    ("accounting.returns.approve", "Approve for filing"),
    (
        "accounting.returns.approve_message",
        "Your approval is recorded as your signature, with your name, the time and your address.",
    ),
    ("accounting.returns.approved", "Return approved."),
    (
        "accounting.returns.read_only",
        "The engagement has ended. You can read your returns; nothing can be changed.",
    ),
    ("accounting.returns.summary", "Summary"),
    ("accounting.returns.type.vat", "VAT return"),
    ("accounting.returns.type.ct", "Corporate tax return"),
    ("accounting.returns.status.preparing", "Preparing"),
    ("accounting.returns.status.in_review", "In review"),
    (
        "accounting.returns.status.client_approval",
        "With the client",
    ),
    ("accounting.returns.status.approved", "Approved"),
    ("accounting.returns.status.filed", "Filed"),
    ("accounting.returns.status.superseded", "Superseded"),
    ("accounting.returns.status.cancelled", "Cancelled"),
    (
        "accounting.returns.transition.submit",
        "Submitted for review",
    ),
    ("accounting.returns.transition.send_back", "Sent back"),
    ("accounting.returns.transition.review", "Sent to the client"),
    (
        "accounting.returns.transition.approve",
        "Approved by the client",
    ),
    (
        "accounting.returns.transition.withdraw_approval",
        "Approval withdrawn",
    ),
    ("accounting.returns.transition.file", "Filed"),
    ("accounting.returns.transition.correct", "Corrected"),
    ("accounting.returns.transition.cancel", "Cancelled"),
    (
        "accounting.returns.problem.no_figures",
        "Enter the figures first.",
    ),
    (
        "accounting.returns.problem.no_workpapers",
        "Attach the workpapers first.",
    ),
    (
        "accounting.returns.problem.not_allowed",
        "Only a reviewer or the client approver can do this.",
    ),
    (
        "accounting.returns.problem.same_person",
        "The reviewer cannot be the preparer.",
    ),
    (
        "accounting.returns.problem.not_approver",
        "Only a client approver can approve a return.",
    ),
    (
        "accounting.returns.problem.no_instruction",
        "Attach the client's written instruction.",
    ),
    (
        "accounting.returns.problem.no_acknowledgement",
        "Attach the FTA acknowledgement.",
    ),
    (
        "accounting.returns.problem.not_preparing",
        "The figures can be entered while the return is being prepared.",
    ),
    (
        "accounting.returns.problem.engagement",
        "A return is opened under an active or suspended engagement.",
    ),
    (
        "accounting.returns.submitted_subject",
        "Return {{ number }} is ready for review",
    ),
    (
        "accounting.returns.submitted_body",
        "The {{ return }} for the period ending {{ period }} (due {{ due }}) was submitted for review: {{ link }}",
    ),
    (
        "accounting.returns.sent_back_subject",
        "Return {{ number }} was sent back",
    ),
    (
        "accounting.returns.sent_back_body",
        "The {{ return }} for the period ending {{ period }} was sent back: {{ comment }}\n\n{{ link }}",
    ),
    (
        "accounting.returns.for_approval_subject",
        "Please approve your {{ return }} ({{ number }})",
    ),
    (
        "accounting.returns.for_approval_body",
        "The {{ return }} for the period ending {{ period }} is ready for your approval before it is filed (due {{ due }}). Review the summary and approve it in the portal: {{ portal_link }}",
    ),
    (
        "accounting.returns.withdrawn_subject",
        "Approval of return {{ number }} withdrawn",
    ),
    (
        "accounting.returns.withdrawn_body",
        "The approval of the {{ return }} for the period ending {{ period }} was withdrawn: {{ reason }}\n\nThe return is back in preparation: {{ link }}",
    ),
    (
        "accounting.returns.handed_over_subject",
        "You file the {{ return }} ({{ number }}) yourselves",
    ),
    (
        "accounting.returns.handed_over_body",
        "We will not file the {{ return }} for the period ending {{ period }}. You file it yourselves, and it is due on {{ due }}. See it in the portal: {{ portal_link }}",
    ),
    (
        "accounting.returns.withdrawn.title",
        "Client withdrew approval of :number",
    ),
    (
        "accounting.returns.withdrawn.description",
        "The client withdrew its approval of :number: :reason\n\nRework the return and submit it again: :link",
    ),
    (
        "accounting.returns.withdrawn.linked",
        "Linked task: :title (:status). :hint",
    ),
    (
        "accounting.returns.withdrawn.hint.upcoming",
        "Start it when the rework begins.",
    ),
    (
        "accounting.returns.withdrawn.hint.in_progress",
        "Nothing is needed: it is being worked.",
    ),
    (
        "accounting.returns.withdrawn.hint.waiting",
        "Nothing is needed: it waits on someone else.",
    ),
    (
        "accounting.returns.withdrawn.hint.in_review",
        "Ask the reviewer to send it back (send_back).",
    ),
    (
        "accounting.returns.withdrawn.hint.overdue",
        "Start it (start).",
    ),
    (
        "accounting.returns.withdrawn.hint.completed",
        "It is completed.",
    ),
    (
        "accounting.returns.withdrawn.hint.missed",
        "It was closed as missed.",
    ),
    (
        "accounting.returns.withdrawn.hint.not_required",
        "It is not required.",
    ),
    (
        "accounting.returns.withdrawn.closed",
        "The return was submitted again.",
    ),
    (
        "accounting.returns.withdrawn.overdue",
        "The linked task of :number is now overdue (since :on): start it (start).",
    ),
    (
        "accounting.returns.cancelled.closed",
        "The return was cancelled.",
    ),
    (
        "accounting.returns.cancelled.waive_reason",
        "Return not required",
    ),
    (
        "accounting.returns.cancelled.left_title",
        "Linked task of the cancelled return :number",
    ),
    (
        "accounting.returns.cancelled.left_description",
        "The return :number was cancelled as not required, but its task :title is :status. Complete it or close it.",
    ),
    ("accounting.portal.returns_title", "Returns"),
    ("accounting.portal.you_file", "You file it by :due"),
    ("accounting.portal.due", "due :due"),
    (
        "accounting.portal.return_status.preparing",
        "In preparation",
    ),
    (
        "accounting.portal.return_status.in_review",
        "In preparation",
    ),
    (
        "accounting.portal.return_status.client_approval",
        "Waiting for your approval",
    ),
    (
        "accounting.portal.return_status.approved",
        "Approved by you",
    ),
    ("accounting.portal.return_status.filed", "Filed"),
    (
        "accounting.portal.return_status.superseded",
        "Replaced by a correction",
    ),
    ("accounting.portal.return_status.cancelled", "Cancelled"),
    ("accounting.engagements.end_date", "End date (empty: today)"),
    ("accounting.engagements.end_scheduled", "End scheduled."),
    (
        "accounting.engagements.end_scheduled_on",
        "Ends on :date (:reason). The engagement stays as it is until then.",
    ),
    (
        "accounting.engagements.unschedule_end",
        "Remove the scheduled end",
    ),
    (
        "accounting.engagements.unschedule_end_message",
        "The engagement keeps running.",
    ),
    (
        "accounting.engagements.end_unscheduled",
        "Scheduled end removed.",
    ),
    (
        "accounting.engagements.nothing_to_bill",
        "Confirm nothing to bill",
    ),
    (
        "accounting.engagements.nothing_to_bill_message",
        "No final invoice or credit is owed on this engagement and no time is left to bill.",
    ),
    (
        "accounting.engagements.nothing_confirmed",
        "Confirmed: nothing to bill.",
    ),
    ("accounting.engagements.end_checklist", "Before it can end"),
    ("accounting.engagements.end_open_item", "Still open"),
    (
        "accounting.engagements.end_checklist_clear",
        "Nothing stands in the way of the end.",
    ),
    (
        "accounting.ending.problem.past_date",
        "The end date is today or later.",
    ),
    (
        "accounting.ending.problem.open_return",
        ":number (:type, period ending :period) is still :status: file it or cancel it.",
    ),
    (
        "accounting.ending.problem.unbilled",
        "Unbilled time of :amount: bill it or write it off.",
    ),
    (
        "accounting.ending.problem.no_settlement",
        "No final invoice or credit is issued: bill what is owed, or confirm that there is nothing to bill.",
    ),
    (
        "accounting.ending.problem.not_endable",
        "Only an active or suspended engagement can end.",
    ),
    (
        "accounting.ending.task_title",
        "The scheduled end of :number on :date could not run",
    ),
    (
        "accounting.ending.task_description",
        "The end of :number scheduled for :date did not run. It is tried again every day until these items are done, or you change or remove the date: :link\n\n:items",
    ),
    (
        "accounting.ending.task_removed",
        "The scheduled end was removed.",
    ),
    ("accounting.ending.task_done", "The engagement ended."),
    ("accounting.ending.cancel_reason", "Engagement ended"),
    ("accounting.wip.title", "Unbilled time"),
    ("accounting.wip.unbilled", "Unbilled time (WIP)"),
    ("accounting.wip.write_off", "Write off"),
    (
        "accounting.wip.write_off_description",
        "Write off unbilled time. Above 10% of the WIP a partner decides.",
    ),
    ("accounting.wip.amount", "Amount to write off (AED)"),
    ("accounting.wip.amount_column", "Amount"),
    ("accounting.wip.reason", "Reason"),
    ("accounting.wip.written_off", "Unbilled time written off."),
    (
        "accounting.wip.pending",
        "More than 10% of the unbilled time: sent to a partner for approval.",
    ),
    ("accounting.wip.write_offs", "Write-offs of unbilled time"),
    ("accounting.wip.write_offs_empty", "No write-offs"),
    ("accounting.wip.status.proposed", "Waiting for a partner"),
    ("accounting.wip.status.written_off", "Written off"),
    ("accounting.wip.status.discarded", "Discarded"),
    (
        "accounting.wip.problem.amount",
        "Write off more than AED 0 and at most the unbilled time (:wip).",
    ),
    (
        "accounting.wip.problem.engagement",
        "Time is written off on an active or suspended engagement.",
    ),
    // The engagement letter (`ACC.RUL.11`, BL-ACC-02).
    ("accounting.letters.title", "Engagement letter"),
    ("accounting.letters.version", "Version"),
    ("accounting.letters.sent_on_label", "Sent on"),
    ("accounting.letters.empty", "No letter sent yet"),
    ("accounting.letters.waiting", "Waiting for a signature"),
    ("accounting.letters.replaced", "Replaced by a newer version"),
    ("accounting.letters.signed_on", "Signed on :date"),
    ("accounting.letters.send", "Send letter for acceptance"),
    (
        "accounting.letters.send_message",
        "The client approver signs the letter in the portal. A letter sent before is replaced and the count of days starts again.",
    ),
    (
        "accounting.letters.sent",
        "The letter is sent for acceptance.",
    ),
    ("accounting.letters.sign", "Sign the letter"),
    (
        "accounting.letters.sign_message",
        "Signing records your name, the time and your IP address.",
    ),
    ("accounting.letters.signed", "The letter is signed."),
    (
        "accounting.letters.problem.signed",
        "The engagement letter is signed already.",
    ),
    (
        "accounting.letters.problem.no_letter",
        "No engagement letter is waiting for a signature.",
    ),
    (
        "accounting.letters.reminder_subject",
        "Your engagement letter {{ number }}",
    ),
    (
        "accounting.letters.reminder_body",
        "Please sign your engagement letter in the portal. It was sent on {{ sent_on }}.",
    ),
    (
        "accounting.letters.chase_title",
        "Chase or decline :number: engagement letter not signed",
    ),
    (
        "accounting.letters.chase_description",
        ":number: version :version of the engagement letter was sent on :sent and is not signed after :days days. Chase the client, or decline the engagement with the reason Letter not signed.\n:link",
    ),
    (
        "accounting.letters.chase_closed_signed",
        "The engagement letter is signed.",
    ),
    (
        "accounting.letters.chase_closed_resent",
        "A new version of the letter was sent.",
    ),
    ("accounting.engagements.decline_note", "Text (for Other)"),
    ("accounting.engagements.decline_template", "Tell the client"),
    (
        "accounting.engagements.decline_no_template",
        "Nothing for now",
    ),
    (
        "accounting.problems.decline_note",
        "Write what the reason is for Other.",
    ),
    (
        "accounting.client_notices.declined_option",
        "Not proceeding (letter, withdrawal, other)",
    ),
    (
        "accounting.client_notices.unable_option",
        "Unable to proceed (CDD rejected)",
    ),
    (
        "accounting.client_notices.declined_subject",
        "Your engagement {{ number }}",
    ),
    (
        "accounting.client_notices.declined_body",
        "We are not able to proceed with the proposed engagement. Please contact your partner if you would like to discuss it.",
    ),
    (
        "accounting.client_notices.unable_subject",
        "Your engagement {{ number }}",
    ),
    (
        "accounting.client_notices.unable_body",
        "We are unable to proceed with an engagement at this time.",
    ),
    // The AML case (`ACC.ACL.05`, `ACC.RUL.12`, BL-ACC-10).
    ("accounting.aml.title", "AML cases"),
    (
        "accounting.aml.subtitle",
        "The MLRO's cases. Only the MLRO and deputy see them.",
    ),
    ("accounting.aml.number", "Case"),
    ("accounting.aml.client", "Client"),
    ("accounting.aml.kind", "Kind"),
    ("accounting.aml.kind.concern", "Concern"),
    ("accounting.aml.kind.sanctions", "Sanctions match"),
    ("accounting.aml.status.open", "Open"),
    ("accounting.aml.status.under_review", "Under review"),
    ("accounting.aml.status.reported", "Reported"),
    (
        "accounting.aml.status.closed_no_action",
        "Closed, no action",
    ),
    ("accounting.aml.empty", "No AML cases"),
    ("accounting.aml.transition.review", "Reviewed"),
    ("accounting.aml.transition.report", "Reported"),
    ("accounting.aml.transition.close", "Closed"),
    ("accounting.aml.transition.open", "Opened"),
    ("accounting.aml.details", "Case"),
    ("accounting.aml.facts", "Facts"),
    ("accounting.aml.raised_by", "Raised by"),
    ("accounting.aml.system", "The system (ACC.RUL.12)"),
    ("accounting.aml.block_note_label", "Block"),
    ("accounting.aml.closing_reason", "Closing reason"),
    ("accounting.aml.goaml_reference", "goAML reference"),
    ("accounting.aml.goaml_date", "Date reported"),
    ("accounting.aml.matches", "Confirmed matches"),
    ("accounting.aml.matches_empty", "No linked screening"),
    ("accounting.aml.match_party", "Matched party"),
    ("accounting.aml.match_role", "Role"),
    ("accounting.aml.match_list", "List and entry"),
    ("accounting.aml.match_tasks", "Freeze and report tasks"),
    ("accounting.aml.task_done", "done"),
    ("accounting.aml.task_open", "open"),
    ("accounting.aml.task_due", "due :due"),
    ("accounting.aml.history", "History"),
    ("accounting.aml.review", "Review"),
    (
        "accounting.aml.review_message",
        "Take the case: it goes under review.",
    ),
    ("accounting.aml.reviewed", "The case is under review."),
    ("accounting.aml.close", "Close, no action"),
    (
        "accounting.aml.close_description",
        "Reviewed; no report is needed. The client stays as it is: unblocking is a separate decision.",
    ),
    ("accounting.aml.reason", "Reason"),
    ("accounting.aml.closed", "The case is closed."),
    ("accounting.aml.report", "Record the report"),
    (
        "accounting.aml.report_description",
        "The report was filed on goAML by hand. Record its reference and date.",
    ),
    ("accounting.aml.reported", "The report is recorded."),
    ("accounting.aml.raise", "Raise a concern"),
    (
        "accounting.aml.raise_description",
        "Tell the MLRO what you saw. Do not discuss it with the client. You will not see the case afterwards.",
    ),
    ("accounting.aml.raised", "The MLRO is told."),
    ("accounting.aml.role.client", "the client company"),
    ("accounting.aml.role.ubo", "a UBO"),
    ("accounting.aml.role.director", "a director"),
    ("accounting.aml.role.signatory", "a signatory"),
    (
        "accounting.aml.facts_sanctions",
        "Confirmed sanctions match: :party, :role of :client, on :list (entry :entry), flagged :flagged.",
    ),
    (
        "accounting.aml.block_reason_note",
        "Confirmed sanctions match",
    ),
    (
        "accounting.aml.block_note",
        "The client company was not blocked; ACC.RUL.12 blocked it with the reason sanctions.",
    ),
    (
        "accounting.aml.problem.facts",
        "Write what you saw: the facts are required.",
    ),
    (
        "accounting.aml.problem.reason",
        "Write the reason the case is closed with no action.",
    ),
    (
        "accounting.aml.problem.goaml",
        "Record the goAML reference and the date it was reported.",
    ),
    (
        "accounting.aml.problem.freeze_open",
        "The freeze task of the confirmed match is not done.",
    ),
    (
        "accounting.aml.problem.report_open",
        "The report task of the confirmed match is not done.",
    ),
    ("accounting.aml.notice_subject", "Case {{ number }}"),
    (
        "accounting.aml.notice_body",
        "Case {{ number }} needs your attention.",
    ),
    // A CDD exit (BL-ACC-09).
    (
        "accounting.exits.partner_title",
        "End engagement — relationship ended",
    ),
    (
        "accounting.exits.partner_description",
        "End each engagement below with the reason Relationship ended. The system sends nothing to the client.\n:lines",
    ),
    (
        "accounting.exits.engagement_line",
        "- :number :title (:status) :link",
    ),
    (
        "accounting.exits.return_line",
        "  Open return :number, period ending :period: the manager cancels it with the reason Engagement ended before the end.",
    ),
    (
        "accounting.exits.on_hold",
        "On hold: no new engagement can be created for this client.",
    ),
    (
        "accounting.exits.task_closed",
        "Every engagement of the client has ended.",
    ),
];

/// Whether the app has a text for `key`.
pub(crate) fn has(key: &str) -> bool {
    TEXTS.iter().any(|(k, _)| *k == key)
}

/// The text for `key`: the app's translation when it has one, else the
/// English one.
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

/// Every text of the app in `lang`, keyed without the `accounting.` prefix
/// (`engagements.title`), for the templates: `tx['engagements.title']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("accounting."), t(lang, key)))
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
    use super::{TEXTS, english, has};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("accounting.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(
            english("accounting.calendar.vat_return", &[("end", &"2026-12-31")]),
            "VAT return, period ending 2026-12-31"
        );
        assert!(has("accounting.reasons.on_hold"));
        assert!(!has("accounting.reasons.sanctions"));
    }
}
