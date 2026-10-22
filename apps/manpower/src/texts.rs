//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

/// Every key the app uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    (
        "manpower.bad_input",
        "Check the form: a value is missing or not in the right form.",
    ),
    // The notices.
    ("manpower.notice.subject", "{{ title }}"),
    ("manpower.notice.body", "{{ body }}"),
    // What a refusal says.
    (
        "manpower.problems.client_unknown",
        "The client is not a known party.",
    ),
    ("manpower.problems.site_name", "Give the site a name."),
    (
        "manpower.problems.site_duplicate",
        "This client already has a site with that name.",
    ),
    (
        "manpower.problems.reference",
        "Give the contract a reference.",
    ),
    (
        "manpower.problems.dates",
        "The end date must be after the start date.",
    ),
    (
        "manpower.problems.payment_terms",
        "The payment terms, the notice period and the deemed-approval days cannot be negative or zero.",
    ),
    (
        "manpower.problems.deemed_days",
        "The deemed-approval days are whole days of 1 or more, or empty for no clause.",
    ),
    (
        "manpower.problems.terms_closed",
        "The contract is cancelled or ended: its timesheet terms cannot change.",
    ),
    (
        "manpower.problems.signed_in_future",
        "A contract cannot be signed on a day that has not come.",
    ),
    (
        "manpower.problems.signed_document",
        "Attach the signed contract first.",
    ),
    (
        "manpower.problems.not_draft",
        "The contract is no longer a draft.",
    ),
    // Client contract `activate` (§7.1).
    (
        "manpower.contract.not_signed",
        "The client has not signed the contract: record the signed contract first.",
    ),
    (
        "manpower.contract.no_rate_card",
        "Accept a rate card version first: no rate card version accepted by the client is in force on the start date of the contract.",
    ),
    (
        "manpower.contract.over",
        "The contract has already ended: its end date is in the past.",
    ),
    (
        "manpower.contract.history_cancel",
        "Contract cancelled before it was signed into force",
    ),
    // The credit hold (§7.1 `hold`, `MPW.RUL.10`, BL-MPW-01).
    (
        "manpower.hold.reason_required",
        "Give the reason for the credit hold: overdue, bounced cheque, or another reason with its text.",
    ),
    (
        "manpower.hold.text_required",
        "Write what the other reason is.",
    ),
    (
        "manpower.hold.already_held",
        "The contract is already on hold for that reason.",
    ),
    (
        "manpower.hold.not_holdable",
        "Only a contract that is active, terminating or already on hold can be put on credit hold.",
    ),
    (
        "manpower.hold.history_overdue",
        "Credit hold: invoice {{ what }} overdue",
    ),
    (
        "manpower.hold.history_bounced_cheque",
        "Credit hold: cheque {{ what }} bounced",
    ),
    ("manpower.hold.history_other", "Credit hold: {{ text }}"),
    ("manpower.hold.held_title", "Credit hold: {{ contract }}"),
    (
        "manpower.hold.held_body",
        "Contract {{ contract }} of {{ client }} is on credit hold. No request, activation or extension goes through until it is released. The workers on site stay and are billed; the MD decides whether to withdraw them.\n{{ reasons }}\nWorkers on site ({{ count }}):\n{{ workers }}",
    ),
    (
        "manpower.hold.added_title",
        "Another reason for the credit hold: {{ contract }}",
    ),
    (
        "manpower.hold.added_body",
        "Contract {{ contract }} of {{ client }} was already on credit hold. It is held for:\n{{ reasons }}\nWorkers on site ({{ count }}):\n{{ workers }}",
    ),
    (
        "manpower.hold.line_overdue",
        "Overdue: {{ amount }} ({{ invoices }}).",
    ),
    (
        "manpower.hold.line_cheque",
        "Bounced cheque {{ cheque }}: {{ amount }}.",
    ),
    ("manpower.hold.line_other", "Other: {{ text }}."),
    // The early termination (§7.1 `terminate`, `MPW.ACL.17`, BL-MPW-04).
    (
        "manpower.terminate.initiated_by",
        "Say who ended the contract: the client or the agency.",
    ),
    (
        "manpower.terminate.reason",
        "Give the reason for ending the contract early.",
    ),
    (
        "manpower.terminate.dates",
        "Give the day the notice was given and the new end date.",
    ),
    (
        "manpower.terminate.later",
        "The new end date is later than the end date of the contract: that is a renewal.",
    ),
    (
        "manpower.terminate.before_notice",
        "The new end date is before the day the notice was given.",
    ),
    (
        "manpower.terminate.short_notice",
        "The end date is inside the notice period of {{ days }} days: the contract can end on {{ earliest }} at the earliest, unless the other side's waiver is attached.",
    ),
    ("manpower.terminate.cancel_reason", "contract terminated"),
    (
        "manpower.terminate.history",
        "Early termination started by the {{ by }}: {{ reason }}",
    ),
    (
        "manpower.terminate.approved_title",
        "Contract {{ contract }} ends early on {{ date }}",
    ),
    (
        "manpower.terminate.approved_body",
        "The MD approved the early termination of contract {{ contract }} (started by the {{ by }}, notice given on {{ notice }}: {{ reason }}). It ends on {{ date }}. The deployments end on that day at the latest; the later ones are cancelled and the client has the confirmation.",
    ),
    (
        "manpower.terminate.letter_title",
        "Confirmation: contract {{ contract }} ends on {{ date }}",
    ),
    (
        "manpower.terminate.letter_body",
        "We confirm the early termination of contract {{ contract }}, notice given on {{ notice }}. The contract ends on {{ date }}. The workers stay on site until then.",
    ),
    (
        "manpower.terminate.withdrawn_title",
        "Termination of {{ contract }} withdrawn",
    ),
    (
        "manpower.terminate.withdrawn_body",
        "The request to end contract {{ contract }} early was withdrawn: {{ reason }}.",
    ),
    // Renewing a contract and the warnings before it ends (`renew`, `MPW.RUL.11`, BL-MPW-05).
    (
        "manpower.renew.later",
        "The new end date must be later than the current end date, {{ end }}.",
    ),
    ("manpower.renew.no_date", "Give the new end date."),
    (
        "manpower.renew.not_signed",
        "Record the renewal signed by the client: the day it was signed, not in the future, and the document.",
    ),
    (
        "manpower.renew.rates_date",
        "The new rates must take effect after the contract starts ({{ start }}) and not after its new end date ({{ end }}).",
    ),
    (
        "manpower.renew.rate_version",
        "Changed rates need a new rate card version, accepted by the client, that takes effect on {{ date }}.",
    ),
    ("manpower.renew.history", "Renewed to {{ end }}"),
    ("manpower.renew.withdraw_reason", "contract renewed"),
    (
        "manpower.expiry.title",
        "Contract {{ contract }} ends on {{ end }}",
    ),
    (
        "manpower.expiry.body",
        "Contract {{ contract }} ends on {{ end }}, {{ days }} days from now, and is not renewed. {{ count }} workers on site end with it. Renew it, or plan their cover; the deployments:",
    ),
    (
        "manpower.expiry.line",
        "{{ worker }}, {{ role }}, {{ site }}, until {{ until }}",
    ),
    ("manpower.expiry.none", "No worker is on site."),
    // Billing the hours of a deployment (`MPW.RUL.05`, BL-MPW-06, BL-MPW-11).
    (
        "manpower.billing.title",
        "Draft invoice for {{ client }}: {{ total }}",
    ),
    (
        "manpower.billing.body",
        "The approved hours of {{ client }} are drafted as an invoice of {{ total }} without VAT, {{ count }} lines. Review it and issue it.",
    ),
    (
        "manpower.billing.unpriced_title",
        "Hours not billed: no rate for {{ worker }}",
    ),
    (
        "manpower.billing.unpriced_body",
        "The approved hours of {{ worker }} ({{ role }}, {{ shift }} shift) in timesheet {{ timesheet }} on {{ days }} are not billed: contract {{ contract }} has no accepted rate card version with a line for them on those days. Accept a rate card version that prices them, then bill them by hand.",
    ),
    // A visa case step that is late (`MPW.RUL.14`, BL-MPW-18). The target is the
    // configured one, not a legal deadline.
    (
        "manpower.visa_step.first_title",
        "Visa step late: {{ step }} for {{ worker }}",
    ),
    (
        "manpower.visa_step.first_body",
        "The step {{ step }} of the {{ case }} visa case of {{ worker }} started on {{ started }} and has taken {{ days }} days against a target of {{ target }}. The target is the agency's own, not a legal deadline. Please follow it up.",
    ),
    (
        "manpower.visa_step.second_title",
        "Visa step still late: {{ step }} for {{ worker }}",
    ),
    (
        "manpower.visa_step.second_body",
        "The step {{ step }} of the {{ case }} visa case of {{ worker }} is still not done, {{ days }} days after it started on {{ started }} (target {{ target }} days). The PRO was told a week ago.",
    ),
    // Ending a contract with its deployments, requests and timesheets (`end`, BL-MPW-06).
    (
        "manpower.end.not_yet",
        "The contract ends on {{ end }}: it is ended the day after, not on {{ day }}.",
    ),
    ("manpower.end.cancel_reason", "contract ended"),
    ("manpower.end.history", "End date {{ end }} reached"),
    ("manpower.end.withdraw_reason", "contract ended"),
    ("manpower.end.bench_none", "none"),
    ("manpower.end.title", "Contract {{ contract }} has ended"),
    (
        "manpower.end.body",
        "Contract {{ contract }} ended on {{ end }}. {{ ended }} deployments ended and {{ cancelled }} planned ones were cancelled with the open requests. {{ timesheets }} last timesheets were closed and sent to the client for approval, {{ open }} need attention before they can be sent. Back on the bench: {{ bench }}.",
    ),
    (
        "manpower.release.not_on_hold",
        "Only a contract on credit hold can be released.",
    ),
    (
        "manpower.release.state_before",
        "The contract can only go back to the state it had before the hold.",
    ),
    ("manpower.release.by_payment", "released by payment"),
    (
        "manpower.release.released_title",
        "Credit hold released: {{ contract }}",
    ),
    (
        "manpower.release.released_body",
        "The credit hold of contract {{ contract }} is released. It is {{ note }} again and takes requests and deployments as before.",
    ),
    (
        "manpower.release.rejected_title",
        "Release of the credit hold refused: {{ contract }}",
    ),
    (
        "manpower.release.rejected_body",
        "The release of the credit hold of contract {{ contract }} was not approved. It stays on hold. {{ note }}",
    ),
    (
        "manpower.hold.refusal",
        "Contract on credit hold since {{ since }}: {{ reason }}",
    ),
    ("manpower.hold.summary_overdue", "overdue ({{ what }})"),
    ("manpower.hold.summary_overdue_plain", "overdue"),
    ("manpower.hold.summary_cheque_plain", "bounced cheque"),
    (
        "manpower.hold.history_overdue_plain",
        "Credit hold: overdue",
    ),
    (
        "manpower.hold.history_bounced_cheque_plain",
        "Credit hold: bounced cheque",
    ),
    ("manpower.hold.summary_cheque", "bounced cheque {{ what }}"),
    (
        "manpower.hold.worker",
        "{{ worker }}, {{ role }} at {{ site }}",
    ),
    // The manpower request (§7.1).
    (
        "manpower.request.positions",
        "The request has no position left for this worker.",
    ),
    (
        "manpower.request.contract_unknown",
        "The client contract does not exist.",
    ),
    (
        "manpower.request.contract_over",
        "The client contract has ended or was cancelled.",
    ),
    (
        "manpower.request.site",
        "Pick a site of the client that is in use.",
    ),
    (
        "manpower.request.fields",
        "Give the role and the shift, and ask for at least one worker.",
    ),
    (
        "manpower.request.dates",
        "The request must not end before it starts.",
    ),
    (
        "manpower.request.contract_not_active",
        "The contract does not take new requests now.",
    ),
    (
        "manpower.request.portal_refused",
        "This request cannot be submitted now. Please contact your account manager.",
    ),
    (
        "manpower.request.outside_contract",
        "The request runs past the end date of the contract.",
    ),
    (
        "manpower.request.not_on_rate_card",
        "The rate card does not price {{ role }} on the {{ shift }} shift.",
    ),
    (
        "manpower.request.submitted_title",
        "New request for {{ role }}",
    ),
    (
        "manpower.request.submitted_body",
        "The client asks for {{ count }} {{ role }} from {{ from }}.",
    ),
    (
        "manpower.request.cancelled_deployments",
        "request cancelled",
    ),
    // A blocked client (BL-MPW-15).
    (
        "manpower.blocked.task_title",
        "Client on hold: review its deployments",
    ),
    (
        "manpower.blocked.task_body",
        "The client is on hold. Workers on site continue; no deployment is activated or extended, no contract renewed and no request submitted until it is released. The MD decides any ending. Its deployments:",
    ),
    (
        "manpower.blocked.line",
        "{{ worker }}, {{ role }} at {{ site }}, {{ status }}, {{ from }} to {{ until }}",
    ),
    // The documents a deployment depends on (BL-MPW-09).
    (
        "manpower.deployment.document_expiring",
        "document expiring {{ kind }} {{ date }}",
    ),
    (
        "manpower.deployment.document_label",
        "{{ role }} at {{ site }}, {{ from }} to {{ until }}",
    ),
    (
        "manpower.deployment.checked_on",
        "The documents were judged on {{ date }}: the day of the action plus {{ days }} days, or the end date of the deployment if that is earlier.",
    ),
    // The deployment (§7.1).
    (
        "manpower.deployment.contract_unknown",
        "The client contract does not exist.",
    ),
    (
        "manpower.deployment.contract_closed",
        "The client contract is a draft, has ended or was cancelled: no worker is planned under it.",
    ),
    (
        "manpower.deployment.employee",
        "The employee is not known to HR.",
    ),
    (
        "manpower.deployment.dates",
        "The deployment must start and end within the dates of the contract, and not end before it starts.",
    ),
    (
        "manpower.deployment.request",
        "Pick a request of this contract that is still open for the same role and site.",
    ),
    (
        "manpower.deployment.shift",
        "Give the shift times as HH:MM and the working days as a list such as mon,tue,wed.",
    ),
    (
        "manpower.deployment.overlap",
        "The worker already has deployment #{{ id }} ({{ status }}) as {{ role }} at {{ site }}, {{ from }} to {{ until }}, {{ shift }}, at overlapping hours.",
    ),
    (
        "manpower.deployment.end_reason",
        "Give a reason you may use: the end date has come, or another worker took the place.",
    ),
    // The checks of an activation, in words (BL-MPW-24).
    (
        "manpower.check.expired",
        "{{ kind }} of {{ worker }} expired on {{ date }}",
    ),
    (
        "manpower.check.missing",
        "{{ kind }} of {{ worker }} is missing",
    ),
    (
        "manpower.check.rejected",
        "{{ kind }} of {{ worker }} was rejected",
    ),
    (
        "manpower.check.pending",
        "{{ kind }} of {{ worker }} is not accepted yet",
    ),
    (
        "manpower.check.override_allowed",
        "(can be overridden with approval)",
    ),
    ("manpower.check.override_never", "(cannot be overridden)"),
    (
        "manpower.check.hold",
        "The contract is on hold. {{ detail }}",
    ),
    (
        "manpower.deployment.employee_not_active",
        "The employee is not active in HR: onboarding, leave, notice and exit do not start a deployment.",
    ),
    (
        "manpower.deployment.contract_not_active",
        "The client contract does not take deployments now.",
    ),
    (
        "manpower.deployment.over",
        "The end date of the deployment has passed.",
    ),
    (
        "manpower.deployment.request_closed",
        "The request this deployment fills no longer takes workers.",
    ),
    (
        "manpower.deployment.open_ended",
        "The deployment has no end date to extend.",
    ),
    (
        "manpower.deployment.extend_later",
        "The new end date must be later than the current one.",
    ),
    (
        "manpower.deployment.beyond_contract",
        "The new end date is after the end of the contract.",
    ),
    (
        "manpower.deployment.end_date_not_come",
        "The end date has not come: end the deployment for another reason.",
    ),
    (
        "manpower.deployment.activated_title",
        "{{ worker }} starts at {{ site }}",
    ),
    (
        "manpower.deployment.activated_body",
        "{{ worker }} starts as {{ role }} at {{ site }} on {{ from }}.",
    ),
    (
        "manpower.deployment.extended_title",
        "Deployment of {{ worker }} extended",
    ),
    (
        "manpower.deployment.extended_body",
        "{{ worker }} stays as {{ role }} at {{ site }} until {{ until }}.",
    ),
    (
        "manpower.deployment.ended_title",
        "{{ worker }} leaves {{ site }}",
    ),
    (
        "manpower.deployment.ended_body",
        "The deployment of {{ worker }} as {{ role }} at {{ site }} ends. The last day is {{ until }}.",
    ),
    // The exits of the worker (`MPW.RUL.12`, BL-MPW-13). The client is told
    // only that the worker is no longer available.
    (
        "manpower.deployment.unavailable_title",
        "{{ worker }} is no longer available",
    ),
    (
        "manpower.deployment.unavailable_body",
        "{{ worker }} is no longer available for {{ role }} at {{ site }}. The deployment runs from {{ from }} to {{ until }}.",
    ),
    (
        "manpower.deployment.notice_withdrawn_title",
        "{{ worker }} stays at {{ site }}",
    ),
    (
        "manpower.deployment.notice_withdrawn_body",
        "The deployment of {{ worker }} as {{ role }} at {{ site }} continues until {{ until }}.",
    ),
    ("manpower.exit.reason.absconded", "worker absconded"),
    ("manpower.exit.reason.deceased", "worker deceased"),
    ("manpower.exit.reason.exited", "worker left the company"),
    (
        "manpower.exit.reason.onboarding_cancelled",
        "onboarding cancelled",
    ),
    (
        "manpower.exit.staff_title",
        "{{ worker }}: {{ what }} ({{ contract }})",
    ),
    (
        "manpower.exit.staff_body",
        "{{ worker }}: {{ what }}.\n{{ lines }}",
    ),
    (
        "manpower.exit.line_ended",
        "Deployment ended at {{ site }} as {{ role }}, last day {{ until }}.",
    ),
    (
        "manpower.exit.line_cancelled",
        "Planned deployment at {{ site }} as {{ role }} cancelled (it was to start on {{ from }}).",
    ),
    (
        "manpower.exit.line_request",
        "Request #{{ request }} stays unfilled by this worker ({{ filled }} of {{ headcount }} filled). It does not reopen: create a replacement if the client still needs one.",
    ),
    (
        "manpower.exit.notice_restored_title",
        "Notice withdrawn: {{ worker }} ({{ contract }})",
    ),
    (
        "manpower.exit.notice_restored_body",
        "{{ worker }}: the notice was withdrawn.\n{{ lines }}",
    ),
    (
        "manpower.exit.line_restored",
        "{{ site }} as {{ role }}: the end date is {{ until }} again (the notice had set {{ was }}).",
    ),
    (
        "manpower.exit.visa_title",
        "Renewal visa of {{ worker }} rejected",
    ),
    (
        "manpower.exit.visa_body",
        "The renewal of the residence visa of {{ worker }} was rejected. The current visa expires on {{ expiry }}. These deployments run past it:\n{{ lines }}",
    ),
    (
        "manpower.exit.line_visa",
        "{{ contract }}: {{ site }} as {{ role }}, {{ status }}, ends {{ until }}.",
    ),
    // Cover not found in time (`MPW.RUL.13`, BL-MPW-16).
    (
        "manpower.cover.missing_title",
        "Cover not found yet for {{ role }} at {{ site }}",
    ),
    (
        "manpower.cover.missing_body",
        "We have not found cover yet: {{ missing }} of {{ headcount }} {{ role }} still to be found at {{ site }} from {{ from }}. We are still working on it.",
    ),
    // The absence case (`MPW.RUL.09`, BL-MPW-17).
    (
        "manpower.absence.ticket_title",
        "Absence: {{ worker }} at {{ site }}, {{ days }} days",
    ),
    (
        "manpower.absence.ticket_body",
        "{{ worker }} has not turned up at {{ site }} (deployment #{{ deployment }}) since {{ last }}: no attendance and no approved leave for {{ days }} days. Record the attempts to reach the worker (two failed attempts on different days) and the MoHRE absence report number. Then request report_absconding. If the worker returns, close this case with the reason.",
    ),
    (
        "manpower.absence.client_title",
        "{{ worker }} has not turned up",
    ),
    (
        "manpower.absence.client_body",
        "{{ worker }} has not turned up at {{ site }} since {{ since }}. We are looking into it.",
    ),
    (
        "manpower.absence.reminder_title",
        "Absence of {{ worker }}: {{ days }} days",
    ),
    (
        "manpower.absence.reminder_body",
        "{{ worker }} has been absent for {{ days }} days. Failed attempts to reach the worker on different days recorded: {{ done }} of {{ need }}. MoHRE absence report number: {{ number }}. Report absconding only when both are recorded.",
    ),
    ("manpower.absence.number_missing", "not recorded"),
    ("manpower.absence.number_recorded", "recorded"),
    (
        "manpower.absence.report",
        "Give the MoHRE absence report number and its date (not in the future).",
    ),
    (
        "manpower.absence.report_number",
        "Enter the MoHRE report number.",
    ),
    (
        "manpower.absence.report_date",
        "The report date cannot be in the future.",
    ),
    (
        "manpower.absence.report_comment",
        "MoHRE absence report {{ number }} of {{ date }}.",
    ),
    (
        "manpower.absence.reason",
        "Give the reason the case is closed: the worker returned, was hospitalised...",
    ),
    ("manpower.absence.closed", "The absence case is closed."),
    ("manpower.absence.reinstated", "worker reinstated"),
    // Ending for a reason that needs more (§7.1 `end`, BL-MPW-14).
    (
        "manpower.end.note_required",
        "Write the client's note: why the client rejected the worker.",
    ),
    (
        "manpower.end.before_today",
        "The last day cannot be in the past.",
    ),
    (
        "manpower.end.short_notice",
        "The contract's notice period is {{ days }} days: the last day cannot be before {{ earliest }} unless you attach a waiver.",
    ),
    ("manpower.end.error_reason", "deployment entered in error"),
    (
        "manpower.end.adjust_title",
        "Adjustment needed for {{ worker }}",
    ),
    (
        "manpower.end.adjust_body",
        "The deployment of {{ worker }} was entered in error. {{ count }} attendance records are accepted or in a locked timesheet and were not rejected: make an adjustment timesheet.",
    ),
    (
        "manpower.end.conduct_title",
        "Conduct: {{ worker }} at {{ site }}",
    ),
    (
        "manpower.end.conduct_body",
        "The client rejected {{ worker }} at {{ site }}: {{ note }}",
    ),
    (
        "manpower.deployment.withdrawn_title",
        "{{ worker }} will leave {{ site }}",
    ),
    (
        "manpower.deployment.withdrawn_body",
        "The deployment of {{ worker }} as {{ role }} at {{ site }} ends on {{ until }}, after the notice period of the contract.",
    ),
    // Suspension (§7.1 `suspend`, `MPW.RUL.12`).
    (
        "manpower.attendance.suspended",
        "The deployment of this worker at the site is suspended: attendance is not recorded until it resumes.",
    ),
    (
        "manpower.deployment.suspended_note",
        "suspended: {{ reasons }}, {{ days }} days suspended",
    ),
    (
        "manpower.deployment.suspended_note_one",
        "suspended: {{ reasons }}, {{ days }} day suspended",
    ),
    (
        "manpower.deployment.suspend_reason",
        "Give the reason: site access withdrawn, investigation or another.",
    ),
    (
        "manpower.deployment.suspend_state",
        "Only a deployment that is active, or already suspended, can be suspended.",
    ),
    ("manpower.deployment.suspend_document", "{{ kind }} expired"),
    (
        "manpower.deployment.suspended_title",
        "{{ worker }} is temporarily unavailable",
    ),
    (
        "manpower.deployment.suspended_body",
        "{{ worker }} is temporarily unavailable at {{ site }} from {{ from }}. We will confirm the return date.",
    ),
    (
        "manpower.deployment.suspended_staff_title",
        "Deployment of {{ worker }} suspended",
    ),
    (
        "manpower.deployment.suspended_staff_body",
        "The deployment of {{ worker }} at {{ site }} is suspended from {{ from }}: {{ reasons }}. Attendance is refused and the days are not billed until it resumes.",
    ),
    // Resume (§7.1 `resume`, `MPW.RUL.12`).
    (
        "manpower.deployment.resume_contract_ended",
        "The client contract has ended: a deployment under it does not resume.",
    ),
    (
        "manpower.deployment.resumed_title",
        "{{ worker }} is available again",
    ),
    (
        "manpower.deployment.resumed_body",
        "{{ worker }} is available again at {{ site }} from {{ on }}.",
    ),
    (
        "manpower.deployment.resume_failing_title",
        "Deployment of {{ worker }} stays suspended",
    ),
    (
        "manpower.deployment.resume_failing_body",
        "The renewed document is accepted, but {{ worker }} cannot resume yet:\n{{ problems }}",
    ),
    // Cover (`MPW.DEP.02`, `MPW.RUL.06`).
    (
        "manpower.cover.history",
        "Cover request opened: {{ cause }}",
    ),
    (
        "manpower.cover.not_covered",
        "Only a worker on site can be covered, from a day to a day not before it.",
    ),
    (
        "manpower.cover.contract_closed",
        "The client contract is a draft, has ended or was cancelled: it takes no cover request.",
    ),
    (
        "manpower.cover.opened_title",
        "Cover needed for {{ worker }}",
    ),
    (
        "manpower.cover.opened_body",
        "{{ worker }} ({{ role }} at {{ site }}) is away from {{ from }} until {{ until }}. A cover request for one {{ role }} is open.",
    ),
    // The screens.
    ("manpower.screen.status", "Status"),
    ("manpower.screen.client", "Client"),
    ("manpower.screen.none", "None."),
    ("manpower.screen.contracts.title", "Client contracts"),
    (
        "manpower.screen.contracts.subtitle",
        "The agreements with the client companies.",
    ),
    ("manpower.screen.contracts.reference", "Contract"),
    ("manpower.screen.contracts.period", "Period"),
    ("manpower.screen.contracts.billing", "Billing"),
    ("manpower.screen.contracts.cover", "Cover"),
    ("manpower.screen.contracts.cover_required", "Cover required"),
    ("manpower.screen.contracts.cover_none", "Not required"),
    ("manpower.screen.open_ended", "Open-ended"),
    (
        "manpower.screen.contracts.held_since",
        "On credit hold since",
    ),
    ("manpower.screen.contracts.held_before", "before:"),
    ("manpower.screen.contracts.empty", "No client contracts yet"),
    (
        "manpower.screen.contracts.empty_text",
        "A contract is made for a client company before its workers are requested.",
    ),
    ("manpower.screen.requests.title", "Manpower requests"),
    (
        "manpower.screen.requests.subtitle",
        "The workers the clients ask for.",
    ),
    ("manpower.screen.requests.request", "Request"),
    ("manpower.screen.requests.workers", "Workers"),
    ("manpower.screen.requests.period", "Period"),
    ("manpower.screen.requests.cover", "Cover"),
    ("manpower.screen.requests.empty", "No requests yet"),
    (
        "manpower.screen.requests.empty_text",
        "A client asks for workers by role, headcount, site and shift.",
    ),
    ("manpower.screen.deployments.title", "Deployments"),
    (
        "manpower.screen.deployments.subtitle",
        "The workers placed at the client sites.",
    ),
    ("manpower.screen.deployments.worker", "Worker"),
    ("manpower.screen.deployments.shift", "Shift"),
    ("manpower.screen.deployments.period", "Period"),
    ("manpower.screen.deployments.cover", "Cover"),
    ("manpower.screen.deployments.empty", "No deployments yet"),
    (
        "manpower.screen.deployments.empty_text",
        "A worker is planned at a site, then activated when the documents and the contract allow.",
    ),
    ("manpower.screen.status.planned", "Planned"),
    ("manpower.screen.status.suspended", "Suspended"),
    ("manpower.screen.status.draft", "Draft"),
    ("manpower.screen.status.open", "Open"),
    ("manpower.screen.status.partially_filled", "Partly filled"),
    ("manpower.screen.status.filled", "Filled"),
    ("manpower.screen.status.active", "Active"),
    ("manpower.screen.status.on_hold", "On hold"),
    ("manpower.screen.status.terminating", "Terminating"),
    ("manpower.screen.status.ended", "Ended"),
    ("manpower.screen.status.cancelled", "Cancelled"),
    ("manpower.screen.billing.weekly", "Weekly"),
    ("manpower.screen.billing.fortnightly", "Every two weeks"),
    ("manpower.screen.billing.monthly", "Monthly"),
    (
        "manpower.problems.client_missing",
        "Pick the client company.",
    ),
    (
        "manpower.problems.contract_missing",
        "Pick the client contract.",
    ),
    (
        "manpower.problems.dates_missing",
        "Give the start date and the end date.",
    ),
    (
        "manpower.problems.headcount_missing",
        "Say how many workers are asked for.",
    ),
    (
        "manpower.problems.signed_date_missing",
        "Give the day the client signed.",
    ),
    ("manpower.problems.site_missing", "Pick the site."),
    ("manpower.problems.start_missing", "Give the first day."),
    ("manpower.screen.not_set", "Not set"),
    ("manpower.screen.fields.status", "Status"),
    ("manpower.screen.action.sign", "Mark signed"),
    (
        "manpower.screen.action.sign_hint",
        "Record the day the client signed and the signed contract. The contract stays a draft until it is activated.",
    ),
    ("manpower.screen.action.activate", "Activate"),
    (
        "manpower.screen.action.activate_hint",
        "The contract comes into force: requests and deployments can start under it.",
    ),
    ("manpower.screen.action.cancel", "Cancel"),
    (
        "manpower.screen.action.cancel_hint",
        "A contract that was never signed into force is cancelled. This cannot be undone.",
    ),
    ("manpower.screen.action.hold", "Put on hold"),
    (
        "manpower.screen.action.hold_hint",
        "No new request, activation or extension goes through while the contract is on credit hold. The workers on site stay.",
    ),
    ("manpower.screen.action.release", "Release"),
    (
        "manpower.screen.action.release_hint",
        "End the credit hold. While a hold condition remains, the MD decides.",
    ),
    ("manpower.screen.action.renew", "Renew"),
    (
        "manpower.screen.action.renew_hint",
        "Move the end date later with the signed renewal. Changed rates need a rate card version accepted by the client.",
    ),
    ("manpower.screen.action.terminate", "Terminate early"),
    (
        "manpower.screen.action.terminate_hint",
        "Ask to end the contract before its end date. The MD approves.",
    ),
    ("manpower.screen.action.terms", "Timesheet terms"),
    (
        "manpower.screen.action.terms_hint",
        "The days after which a timesheet the client does not answer counts as approved, and who the reminders go to.",
    ),
    ("manpower.screen.action.submit", "Submit"),
    (
        "manpower.screen.action.submit_hint",
        "Send the request to the agency: workers can then be found for it.",
    ),
    ("manpower.screen.action.plan", "Plan a deployment"),
    (
        "manpower.screen.action.plan_hint",
        "Choose the worker for this request. The request, the contract, the site and the shift are already set.",
    ),
    ("manpower.screen.tab.overview", "Overview"),
    ("manpower.screen.tab.deployments", "Deployments"),
    ("manpower.screen.tab.rates", "Rate cards"),
    ("manpower.screen.tab.approvals", "Approvals"),
    ("manpower.screen.tab.activity", "Activity"),
    ("manpower.screen.contract.title", "Contract"),
    ("manpower.screen.contract.period", "Period"),
    ("manpower.screen.contract.payment_terms", "Payment terms"),
    ("manpower.screen.contract.days", "{{ days }} days"),
    ("manpower.screen.contract.notice", "Notice period"),
    (
        "manpower.screen.contract.deemed",
        "Timesheet deemed approved",
    ),
    ("manpower.screen.contract.no_deemed", "No deemed approval"),
    ("manpower.screen.contract.escalation", "Escalation contact"),
    ("manpower.screen.contract.manager", "Account manager"),
    ("manpower.screen.contract.signed", "Signature"),
    ("manpower.screen.contract.signed_on", "Signed on {{ date }}"),
    ("manpower.screen.contract.not_signed", "Not signed yet"),
    ("manpower.screen.contract.document", "Signed contract"),
    ("manpower.screen.contract.no_document", "Not attached"),
    ("manpower.screen.contract.activated", "Active since"),
    (
        "manpower.screen.contract.cancel_reason",
        "Cancelled because",
    ),
    ("manpower.screen.contract.termination", "Early termination"),
    (
        "manpower.screen.contract.terminating_line",
        "Ended by {{ by }}: {{ reason }}. Notice given on {{ notice }}. The end date was {{ was }}.",
    ),
    ("manpower.screen.contract.renewal", "Latest renewal"),
    (
        "manpower.screen.contract.renewed_line",
        "Renewed on {{ on }} to {{ end }} (it ended on {{ was }}). New rates from {{ rates }}.",
    ),
    (
        "manpower.screen.contract.checks",
        "Before it can be activated",
    ),
    (
        "manpower.screen.contract.deployments_empty",
        "The workers placed under this contract show here.",
    ),
    (
        "manpower.screen.contract.no_licences",
        "No licence asked for",
    ),
    (
        "manpower.screen.hold.banner",
        "This contract is on credit hold. No new request, activation or extension goes through; the workers on site stay.",
    ),
    ("manpower.screen.hold.title", "Credit hold"),
    ("manpower.screen.hold.since", "On hold since"),
    ("manpower.screen.hold.before", "State before the hold"),
    ("manpower.screen.hold.by", "Held by"),
    ("manpower.screen.hold.by_rule", "The daily check"),
    ("manpower.screen.hold.reasons", "Reasons"),
    (
        "manpower.screen.hold.overdue",
        "Invoice overdue {{ what }} {{ amount }}",
    ),
    (
        "manpower.screen.hold.cheque",
        "Bounced cheque {{ what }} {{ amount }}",
    ),
    ("manpower.screen.hold.other", "{{ text }}"),
    (
        "manpower.screen.hold.line",
        "{{ cause }}, since {{ since }}",
    ),
    ("manpower.screen.hold_reason.overdue", "Invoice overdue"),
    (
        "manpower.screen.hold_reason.bounced_cheque",
        "Bounced cheque",
    ),
    ("manpower.screen.hold_reason.other", "Another reason"),
    ("manpower.screen.initiator.client", "the client"),
    ("manpower.screen.initiator.agency", "the agency"),
    ("manpower.screen.sites.title", "Sites of the client"),
    ("manpower.screen.sites.new", "New site"),
    ("manpower.screen.sites.edit", "Edit"),
    ("manpower.screen.sites.save", "Save the site"),
    ("manpower.screen.sites.name", "Name"),
    ("manpower.screen.sites.address", "Address"),
    ("manpower.screen.sites.licences", "Required licences"),
    (
        "manpower.screen.sites.licences_hint",
        "In words, for example: SIRA licence, site pass.",
    ),
    ("manpower.screen.sites.actions", "Actions"),
    ("manpower.screen.sites.empty", "No site yet"),
    ("manpower.screen.rates.version", "Version"),
    ("manpower.screen.rates.open_card", "Open the rate card"),
    ("manpower.screen.rates.start", "Start the rate card"),
    (
        "manpower.screen.rates.start_hint",
        "Makes the empty rate card of this contract. Then add its first version with the effective date and the lines.",
    ),
    ("manpower.screen.rates.effective", "Prices from"),
    ("manpower.screen.rates.accepted", "Accepted"),
    ("manpower.screen.rates.lines", "Roles priced"),
    ("manpower.screen.rates.empty", "No rate card yet"),
    (
        "manpower.screen.rates.empty_text",
        "The rate card of the contract and its versions show here.",
    ),
    (
        "manpower.screen.rate_status.pending",
        "Waiting for the client",
    ),
    (
        "manpower.screen.rate_status.awaiting_approval",
        "Waiting for the MD",
    ),
    ("manpower.screen.rate_status.accepted", "Accepted"),
    ("manpower.screen.rate_status.rejected", "Rejected"),
    ("manpower.screen.approval.request", "Request"),
    ("manpower.screen.approval.requester", "Asked by"),
    ("manpower.screen.approval.requested", "Asked on"),
    ("manpower.screen.approval.decided", "Decided on"),
    ("manpower.screen.approval.decision", "Decision"),
    ("manpower.screen.approval.approve", "Approve"),
    ("manpower.screen.approval.reject", "Reject"),
    ("manpower.screen.approval.comment", "Comment"),
    ("manpower.screen.approval.comment_optional", "Optional."),
    (
        "manpower.screen.approval.comment_required",
        "Say why. Required.",
    ),
    ("manpower.screen.approval.waiting", "Waiting for the MD"),
    (
        "manpower.screen.approval.waiting_for",
        "Waiting for the MD: {{ action }}.",
    ),
    ("manpower.screen.approval.empty", "No approval asked"),
    (
        "manpower.screen.approval.empty_text",
        "A release of a credit hold or an early termination waits here for the MD.",
    ),
    ("manpower.screen.approval.approved", "Approved"),
    ("manpower.screen.approval.rejected", "Rejected"),
    ("manpower.screen.approval.returned", "Sent back"),
    ("manpower.screen.approval.withdrawn", "Withdrawn"),
    ("manpower.screen.approval.expired", "Expired"),
    (
        "manpower.screen.approval.failed",
        "Could not be carried out",
    ),
    ("manpower.screen.approval.pending", "Waiting for the MD"),
    ("manpower.screen.activity.when", "When"),
    ("manpower.screen.activity.what", "What"),
    ("manpower.screen.activity.by", "By"),
    ("manpower.screen.activity.reason", "Reason"),
    ("manpower.screen.activity.system", "The system"),
    ("manpower.screen.activity.empty", "Nothing yet"),
    ("manpower.screen.transition.insert", "Recorded"),
    ("manpower.screen.transition.activate", "Activated"),
    ("manpower.screen.transition.cancel", "Cancelled"),
    ("manpower.screen.transition.hold", "Put on hold"),
    (
        "manpower.screen.transition.auto_hold",
        "Put on hold by the daily check",
    ),
    (
        "manpower.screen.transition.release",
        "Released from the hold",
    ),
    (
        "manpower.screen.transition.release_terminating",
        "Released from the hold",
    ),
    (
        "manpower.screen.transition.release_approved",
        "Released from the hold, approved",
    ),
    (
        "manpower.screen.transition.release_terminating_approved",
        "Released from the hold, approved",
    ),
    (
        "manpower.screen.transition.auto_release",
        "Released from the hold by the daily check",
    ),
    (
        "manpower.screen.transition.auto_release_terminating",
        "Released from the hold by the daily check",
    ),
    (
        "manpower.screen.transition.terminate",
        "Early termination asked",
    ),
    (
        "manpower.screen.transition.terminate_approved",
        "Early termination approved",
    ),
    ("manpower.screen.transition.renew", "Renewed"),
    (
        "manpower.screen.transition.renew_on_hold",
        "Renewed while on hold",
    ),
    ("manpower.screen.transition.end", "Ended"),
    ("manpower.screen.transition.submit", "Submitted"),
    ("manpower.screen.transition.fill", "Filled"),
    ("manpower.screen.transition.fill_partly", "Partly filled"),
    (
        "manpower.screen.transition.auto_cancel",
        "Cancelled by the system",
    ),
    ("manpower.screen.done.created", "The contract was created."),
    ("manpower.screen.done.signed", "The signature was recorded."),
    ("manpower.screen.done.activated", "The contract is active."),
    (
        "manpower.screen.done.cancelled",
        "The contract was cancelled.",
    ),
    ("manpower.screen.done.held", "The contract is on hold."),
    (
        "manpower.screen.done.release_asked",
        "Waiting for the MD to approve the release.",
    ),
    ("manpower.screen.done.released", "The contract is released."),
    (
        "manpower.screen.done.terminate_asked",
        "Waiting for the MD to approve the early termination.",
    ),
    (
        "manpower.screen.done.terminated",
        "The contract is ending early.",
    ),
    ("manpower.screen.done.renewed", "The contract was renewed."),
    (
        "manpower.screen.done.terms",
        "The timesheet terms were saved.",
    ),
    ("manpower.screen.done.site_created", "The site was added."),
    ("manpower.screen.done.site_updated", "The site was saved."),
    (
        "manpower.screen.done.request_created",
        "The request was saved as a draft.",
    ),
    (
        "manpower.screen.done.request_submitted",
        "The request was submitted.",
    ),
    (
        "manpower.screen.done.request_cancelled",
        "The request was cancelled.",
    ),
    (
        "manpower.screen.done.cover_asked",
        "The cover request was opened.",
    ),
    ("manpower.screen.form.client", "Client company"),
    (
        "manpower.screen.form.client_placeholder",
        "Search the client by name",
    ),
    ("manpower.screen.form.reference", "Reference"),
    (
        "manpower.screen.form.reference_hint",
        "The number the agency gives the contract, for example CC-2026-004.",
    ),
    ("manpower.screen.form.start_date", "First day"),
    ("manpower.screen.form.end_date", "Last day"),
    (
        "manpower.screen.form.end_date_hint",
        "Leave empty when there is no end date.",
    ),
    ("manpower.screen.form.billing", "Billing cycle"),
    ("manpower.screen.form.payment_terms", "Days to pay"),
    ("manpower.screen.form.notice_days", "Notice days"),
    (
        "manpower.screen.form.cover_required",
        "The agency must cover absent workers",
    ),
    ("manpower.screen.form.deemed_days", "Deemed-approval days"),
    (
        "manpower.screen.form.deemed_days_hint",
        "A timesheet the client does not answer in this many days counts as approved. Empty for none.",
    ),
    ("manpower.screen.form.escalation", "Escalation contact"),
    (
        "manpower.screen.form.escalation_placeholder",
        "Search the contact by name",
    ),
    ("manpower.screen.form.signed_on", "Signed on"),
    ("manpower.screen.form.document", "Signed contract"),
    (
        "manpower.screen.form.document_hint",
        "The file name or number of the signed contract.",
    ),
    ("manpower.screen.form.hold_reason", "Reason"),
    ("manpower.screen.form.hold_text", "Details"),
    (
        "manpower.screen.form.hold_text_hint",
        "The invoice or cheque number, or what the other reason is.",
    ),
    ("manpower.screen.form.new_end", "New end date"),
    (
        "manpower.screen.form.renewal_signed_on",
        "Renewal signed on",
    ),
    ("manpower.screen.form.renewal_document", "Signed renewal"),
    ("manpower.screen.form.rates_from", "New rates from"),
    (
        "manpower.screen.form.rates_from_hint",
        "Only when the rates change: the day the new rate card version takes effect.",
    ),
    ("manpower.screen.form.initiated_by", "Ended by"),
    ("manpower.screen.form.terminate_reason", "Reason"),
    ("manpower.screen.form.notice_on", "Notice given on"),
    ("manpower.screen.form.last_day", "Last day of the contract"),
    ("manpower.screen.form.waiver", "Waiver"),
    (
        "manpower.screen.form.waiver_hint",
        "The signed waiver when the notice is shorter than the notice period.",
    ),
    ("manpower.screen.form.cancel_reason", "Reason"),
    ("manpower.screen.form.contract", "Contract"),
    ("manpower.screen.form.site", "Site"),
    ("manpower.screen.form.role", "Job role"),
    ("manpower.screen.form.positions", "Positions"),
    ("manpower.screen.form.shift", "Shift"),
    ("manpower.screen.form.shift_start", "Shift starts"),
    ("manpower.screen.form.shift_end", "Shift ends"),
    ("manpower.screen.form.worker", "Worker"),
    ("manpower.screen.form.worker_placeholder", "Pick the worker"),
    ("manpower.screen.shift.day", "Day shift"),
    ("manpower.screen.shift.night", "Night shift"),
    ("manpower.screen.contracts.new", "New contract"),
    (
        "manpower.screen.contracts.new_hint",
        "A draft contract for a client. It comes into force once it is signed and has an accepted rate card version.",
    ),
    ("manpower.screen.contracts.new_save", "Create the contract"),
    ("manpower.screen.requests.new", "New request"),
    (
        "manpower.screen.requests.new_hint",
        "A client's order for workers. It stays a draft until it is submitted.",
    ),
    ("manpower.screen.requests.new_save", "Save the request"),
    (
        "manpower.screen.requests.contract_option",
        "{{ reference }}, {{ client }}",
    ),
    ("manpower.screen.deployments.role", "Role"),
    ("manpower.screen.deployments.site", "Site"),
    ("manpower.screen.request.title", "Request"),
    ("manpower.screen.request.contract", "Contract"),
    ("manpower.screen.request.cause", "Why this request exists"),
    (
        "manpower.screen.request.cause_link",
        "Open the deployment it stands in for",
    ),
    ("manpower.screen.request.deployments", "Deployments"),
    ("manpower.screen.request.no_deployments", "No worker yet"),
    (
        "manpower.screen.request.no_deployments_text",
        "The workers planned for this request show here.",
    ),
    ("manpower.screen.requests.site", "Site"),
    ("manpower.screen.requests.shift", "Shift"),
    (
        "manpower.screen.requests.shift_hours",
        "{{ shift }}, {{ start }} to {{ end }}",
    ),
    ("manpower.screen.requests.submitted", "Submitted on"),
    (
        "manpower.screen.requests.cancel_reason",
        "Cancelled because",
    ),
    (
        "manpower.screen.requests.positions",
        "{{ filled }} of {{ headcount }} filled, {{ missing }} missing",
    ),
    (
        "manpower.screen.requests.suspended",
        "{{ count }} suspended",
    ),
    (
        "manpower.screen.requests.active_one",
        "{{ count }} deployment is still active.",
    ),
    (
        "manpower.screen.requests.active_many",
        "{{ count }} deployments are still active.",
    ),
    (
        "manpower.screen.requests.on_hold_blocked",
        "The contract is on hold. New requests wait for the release.",
    ),
    (
        "manpower.screen.requests.contract_blocked",
        "The contract does not take new requests now.",
    ),
    (
        "manpower.screen.requests.plan_blocked",
        "Submit the request first.",
    ),
    (
        "manpower.screen.requests.an_absent_worker",
        "an absent worker",
    ),
    (
        "manpower.screen.requests.cause_rejection",
        "Replacement for {{ worker }}, ended by client rejection on {{ date }}",
    ),
    (
        "manpower.screen.requests.cause_leave",
        "Cover for {{ worker }} on leave",
    ),
    (
        "manpower.screen.requests.cause_suspension",
        "Cover for {{ worker }}, whose deployment is suspended",
    ),
    (
        "manpower.screen.requests.cause_absence",
        "Cover for an absent worker, {{ worker }}",
    ),
    (
        "manpower.screen.requests.cause_manual",
        "Cover asked for by hand for {{ worker }}",
    ),
    (
        "manpower.screen.requests.client_note",
        "The client wrote: {{ note }}",
    ),
    ("manpower.screen.status.voided", "Voided"),
    ("manpower.screen.deployment.title", "Deployment"),
    ("manpower.screen.deployment.worker", "Worker"),
    ("manpower.screen.deployment.client", "Client"),
    (
        "manpower.screen.deployment.is_cover",
        "Stands in for another worker",
    ),
    ("manpower.screen.deployment.activated", "Activated on"),
    ("manpower.screen.deployment.suspension", "Suspension"),
    (
        "manpower.screen.deployment.cancel_reason",
        "Cancelled because",
    ),
    (
        "manpower.screen.deployment.days_suspended_one",
        "{{ days }} day suspended",
    ),
    (
        "manpower.screen.deployment.days_suspended",
        "{{ days }} days suspended",
    ),
    (
        "manpower.screen.deployment.suspended_because",
        "Suspended: {{ reasons }}. {{ days }}.",
    ),
    (
        "manpower.screen.deployment.withdrawal_planned",
        "The worker is withdrawn after {{ date }} because the client has not paid.",
    ),
    ("manpower.screen.deployment.ended", "How it ended"),
    (
        "manpower.screen.deployment.ended_because",
        "Ended on {{ date }}: {{ reason }}.",
    ),
    (
        "manpower.screen.deployment.client_note",
        "The client's note: {{ note }}",
    ),
    (
        "manpower.screen.deployment.waiver_note",
        "Waiver of the notice period: {{ waiver }}",
    ),
    (
        "manpower.screen.deployment.replacement_link",
        "Open the replacement request",
    ),
    (
        "manpower.screen.deployment.covers",
        "This deployment stands in for {{ worker }}",
    ),
    (
        "manpower.screen.deployment.cover_request",
        "Cover request, {{ period }} ({{ status }})",
    ),
    ("manpower.screen.deployment.cover_title", "Cover"),
    (
        "manpower.screen.deployment.checks",
        "Why it cannot start yet",
    ),
    (
        "manpower.screen.deployment.request_override",
        "Request override (MD approval)",
    ),
    ("manpower.screen.deployment.documents", "Documents"),
    ("manpower.screen.deployment.document", "Document"),
    ("manpower.screen.deployment.validity", "Validity"),
    (
        "manpower.screen.deployment.documents_empty",
        "No document is needed for this role.",
    ),
    (
        "manpower.screen.deployment.doc_no_expiry",
        "Does not expire",
    ),
    (
        "manpower.screen.deployment.doc_valid_until",
        "Valid until {{ date }}",
    ),
    (
        "manpower.screen.deployment.doc_expired_on",
        "Expired on {{ date }}",
    ),
    ("manpower.screen.deployment.timesheets", "Timesheets"),
    (
        "manpower.screen.deployment.timesheets_empty",
        "No timesheet yet.",
    ),
    ("manpower.screen.deployment.sheet_kind", "Kind"),
    (
        "manpower.screen.deployment.activate_hint",
        "Starts the deployment today. Every check that fails is listed and nothing starts until they pass.",
    ),
    ("manpower.screen.deployment.extend", "Extend"),
    (
        "manpower.screen.deployment.extend_hint",
        "Moves the last day later. The documents must stay valid to the new date.",
    ),
    ("manpower.screen.deployment.suspend", "Suspend"),
    (
        "manpower.screen.deployment.suspend_hint",
        "The worker stops working at the site and the days are not billed.",
    ),
    ("manpower.screen.deployment.suspend_reason", "Reason"),
    ("manpower.screen.deployment.resume", "Resume"),
    (
        "manpower.screen.deployment.resume_hint",
        "The worker returns to the site. The documents must be valid again.",
    ),
    ("manpower.screen.deployment.cover", "Ask for cover"),
    (
        "manpower.screen.deployment.cover_hint",
        "Opens a request for a worker to stand in for the days the worker is away.",
    ),
    ("manpower.screen.deployment.cover_from", "From"),
    ("manpower.screen.deployment.cover_to", "To"),
    (
        "manpower.screen.deployment.cover_to_hint",
        "Leave empty while the return is not known.",
    ),
    ("manpower.screen.deployment.end", "End"),
    (
        "manpower.screen.deployment.end_hint",
        "Ends the deployment. The reason decides what else is asked.",
    ),
    ("manpower.screen.deployment.end_reason", "Reason"),
    ("manpower.screen.deployment.end_note", "The client's note"),
    (
        "manpower.screen.deployment.end_note_hint",
        "Write what the client said. A replacement request opens when the contract needs cover.",
    ),
    (
        "manpower.screen.deployment.end_conduct",
        "Conduct is involved: open a complaint ticket",
    ),
    (
        "manpower.screen.deployment.end_last_day",
        "Last day on site",
    ),
    (
        "manpower.screen.deployment.end_last_day_hint",
        "At least the notice period of the contract from today, unless a waiver is given.",
    ),
    (
        "manpower.screen.deployment.cancel_hint",
        "The deployment never starts.",
    ),
    ("manpower.screen.end_reason.end_date", "End date reached"),
    (
        "manpower.screen.end_reason.replaced",
        "Replaced by another worker",
    ),
    (
        "manpower.screen.end_reason.client_rejected",
        "Client rejected the worker",
    ),
    (
        "manpower.screen.end_reason.client_non_payment",
        "Client does not pay",
    ),
    (
        "manpower.screen.end_reason.entered_in_error",
        "Entered in error",
    ),
    (
        "manpower.screen.end_reason.contract_end",
        "The contract ended",
    ),
    (
        "manpower.screen.end_reason.exit",
        "The worker left the agency",
    ),
    (
        "manpower.screen.end_reason.absconded",
        "The worker absconded",
    ),
    ("manpower.screen.end_reason.deceased", "The worker died"),
    ("manpower.screen.doc_state.valid", "Valid"),
    ("manpower.screen.doc_state.expired", "Expired"),
    ("manpower.screen.doc_state.missing", "Missing"),
    ("manpower.screen.doc_state.rejected", "Rejected"),
    ("manpower.screen.doc_state.pending", "Waiting for review"),
    ("manpower.screen.sheet_kind.regular", "Regular"),
    ("manpower.screen.sheet_kind.adjustment", "Adjustment"),
    ("manpower.screen.sheet_status.open", "Open"),
    ("manpower.screen.sheet_status.submitted", "Submitted"),
    ("manpower.screen.sheet_status.approved", "Approved"),
    ("manpower.screen.sheet_status.locked", "Locked"),
    ("manpower.screen.deployments.request", "Request"),
    (
        "manpower.screen.deployments.request_option",
        "{{ role }} at {{ site }}, {{ client }}, {{ period }}",
    ),
    (
        "manpower.screen.deployments.plan_hint_list",
        "Pick the request to fill and the worker. The deployment is planned; Activate starts it.",
    ),
    ("manpower.screen.tab.timesheets", "Timesheets"),
    (
        "manpower.screen.done.deployment_planned",
        "The deployment is planned.",
    ),
    (
        "manpower.screen.done.deployment_activated",
        "The deployment is active.",
    ),
    (
        "manpower.screen.done.deployment_extended",
        "The deployment was extended.",
    ),
    (
        "manpower.screen.done.deployment_suspended",
        "The deployment is suspended.",
    ),
    (
        "manpower.screen.done.deployment_resumed",
        "The deployment is active again.",
    ),
    (
        "manpower.screen.done.deployment_ended",
        "The deployment has ended.",
    ),
    (
        "manpower.screen.done.deployment_cancelled",
        "The deployment was cancelled.",
    ),
    (
        "manpower.screen.deployment.end_reason_unknown",
        "Choose one of the reasons in the list.",
    ),
    (
        "manpower.screen.done.absence_reported",
        "The report was recorded.",
    ),
    ("manpower.screen.done.absence_closed", "The case is closed."),
    ("manpower.screen.absence_status.open", "Open"),
    ("manpower.screen.absence_status.closed", "Closed"),
    ("manpower.screen.ticket_status.open", "Open"),
    ("manpower.screen.ticket_status.assigned", "Assigned"),
    ("manpower.screen.ticket_status.in_progress", "In progress"),
    ("manpower.screen.ticket_status.on_hold", "On hold"),
    ("manpower.screen.ticket_status.resolved", "Resolved"),
    ("manpower.screen.ticket_status.closed", "Closed"),
    ("manpower.screen.ticket_status.cancelled", "Cancelled"),
    ("manpower.screen.absence.title", "Absence cases"),
    (
        "manpower.screen.absence.subtitle",
        "Workers who have not turned up. Record the attempts to reach them and the MoHRE report, or close the case when the worker is back.",
    ),
    ("manpower.screen.absence.case", "Absence case"),
    ("manpower.screen.absence.worker", "Worker"),
    ("manpower.screen.absence.client", "Client"),
    ("manpower.screen.absence.site", "Site"),
    ("manpower.screen.absence.last_present", "Last day present"),
    ("manpower.screen.absence.opened", "Opened on"),
    ("manpower.screen.absence.days", "Days absent"),
    ("manpower.screen.absence.days_column", "Days absent"),
    ("manpower.screen.absence.days_absent", "{{ days }} days"),
    ("manpower.screen.absence.contacts", "Contact attempts"),
    (
        "manpower.screen.absence.contacts_of",
        "{{ done }} of {{ need }} failed contact days",
    ),
    ("manpower.screen.absence.report", "MoHRE report"),
    (
        "manpower.screen.absence.report_line",
        "{{ number }}, {{ date }}",
    ),
    ("manpower.screen.absence.report_none", "Not recorded"),
    ("manpower.screen.absence.status", "Status"),
    ("manpower.screen.absence.empty", "No absence cases"),
    (
        "manpower.screen.absence.empty_text",
        "A case opens when a worker on site has not turned up for more than a week.",
    ),
    ("manpower.screen.absence.failed", "Could not be reached"),
    (
        "manpower.screen.absence.contact_days",
        "Failed contact days",
    ),
    ("manpower.screen.absence.contact_day", "Day"),
    ("manpower.screen.absence.contact_outcome", "Outcome"),
    (
        "manpower.screen.absence.contacts_empty",
        "No failed contact attempt recorded yet.",
    ),
    ("manpower.screen.absence.tickets", "Tickets"),
    ("manpower.screen.absence.ticket", "Ticket"),
    ("manpower.screen.absence.ticket_title", "Title"),
    (
        "manpower.screen.absence.tickets_empty",
        "No ticket for this worker.",
    ),
    ("manpower.screen.absence.record_report", "Record report"),
    (
        "manpower.screen.absence.record_report_hint",
        "The number of the absence report filed with MoHRE and its date.",
    ),
    (
        "manpower.screen.absence.report_number",
        "MoHRE report number",
    ),
    ("manpower.screen.absence.report_date", "Report date"),
    ("manpower.screen.absence.close", "Close case"),
    (
        "manpower.screen.absence.close_hint",
        "The worker is back. The held pay is released and proposed in an off-cycle run.",
    ),
    ("manpower.screen.absence.close_reason", "Reason"),
    ("manpower.screen.absence.abscond", "Report as absconded"),
    (
        "manpower.screen.absence.abscond_blocked",
        "Two failed contact days and the MoHRE number are needed.",
    ),
    ("manpower.screen.absence.employee_link", "Open the worker"),
    (
        "manpower.screen.absence.deployment_link",
        "Open the deployment",
    ),
    (
        "manpower.screen.absence.closed_line",
        "Closed on {{ date }} by {{ by }}: {{ reason }}.",
    ),
    (
        "manpower.screen.absence.pay_released",
        "The held pay was released.",
    ),
    (
        "manpower.screen.absence.pay_proposed",
        "The held pay was released and is proposed in an off-cycle run.",
    ),
    ("manpower.screen.absence.run_link", "Open the payroll run"),
    ("manpower.screen.absence.closed_title", "How it ended"),
    ("manpower.screen.request_status.draft", "Draft"),
    ("manpower.screen.request_status.open", "Submitted"),
    (
        "manpower.screen.request_status.partially_filled",
        "Partly filled",
    ),
    ("manpower.screen.request_status.filled", "Filled"),
    ("manpower.screen.request_status.cancelled", "Cancelled"),
    ("manpower.screen.request_status.closed", "Closed"),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key, &[])
}

/// The English text for `key` with `params` filled in as `{{ name }}`; the
/// key itself when it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &(dyn Display + Sync))]) -> String {
    let text = TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let mut data = renox::serde_json::Map::new();
    for (name, value) in params {
        data.insert(
            (*name).to_owned(),
            renox::serde_json::Value::String(value.to_string()),
        );
    }
    wl_kernel::notify::fill(text, &renox::serde_json::Value::Object(data))
}

/// An amount in fils as `AED 42,000` (`AED 42,000.50` with fils).
pub(crate) fn money(fils: i64) -> String {
    let whole = (fils / 100).abs().to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if fils < 0 { "-" } else { "" };
    match (fils % 100).abs() {
        0 => format!("{sign}AED {grouped}"),
        cents => format!("{sign}AED {grouped}.{cents:02}"),
    }
}

/// The English text of `key`, unfilled: a notification template.
pub(crate) fn template(key: &str) -> &'static str {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, text)| *text)
}

/// Every text of the screens, by the name a template reads
/// (`tx['contracts.title']` is the key `manpower.screen.contracts.title`).
pub(crate) fn screen_texts(lang: &Lang) -> std::collections::BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix("manpower.screen.")?, t(lang, key))))
        .collect()
}

/// The text for `key` in the viewer's language with `params` filled in as
/// `{{ name }}`: the app's translation when it has one, else the English one.
pub(crate) fn tf(lang: &Lang, key: &str, params: &[(&str, &(dyn Display + Sync))]) -> String {
    let found = lang.t(key, &[]);
    let text = if found == key { template(key) } else { &found };
    if text.is_empty() {
        return key.to_owned();
    }
    let mut data = renox::serde_json::Map::new();
    for (name, value) in params {
        data.insert(
            (*name).to_owned(),
            renox::serde_json::Value::String(value.to_string()),
        );
    }
    wl_kernel::notify::fill(text, &renox::serde_json::Value::Object(data))
}

/// The names of the screen texts (`tx['contracts.title']`), for the test that
/// every text a template reads exists.
#[cfg(test)]
pub(crate) fn screen_text_keys() -> Vec<&'static str> {
    TEXTS
        .iter()
        .filter_map(|(key, _)| key.strip_prefix("manpower.screen."))
        .collect()
}

/// The word for `value` in `group` (`status`, `billing`); the value itself
/// when it has no text.
pub(crate) fn word(lang: &Lang, group: &str, value: &str) -> String {
    let key = format!("manpower.screen.{group}.{value}");
    let found = t(lang, &key);
    if found == key {
        value.replace('_', " ")
    } else {
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("manpower.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn every_key_the_code_asks_for_has_a_text() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "rs") || path.ends_with("texts.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for call in source.split("english(").skip(1) {
                let call = call.trim_start();
                let Some(rest) = call.strip_prefix('"') else {
                    continue;
                };
                let key = rest.split('"').next().unwrap_or_default();
                if key.starts_with("manpower.") && !TEXTS.iter().any(|(k, _)| *k == key) {
                    missing.push(format!("{}: {key}", path.display()));
                }
            }
        }
        assert!(missing.is_empty(), "keys without a text: {missing:?}");
    }

    #[test]
    fn an_amount_is_grouped_by_thousands() {
        assert_eq!(money(4_200_000), "AED 42,000");
        assert_eq!(money(1_500_000), "AED 15,000");
        assert_eq!(money(12_345), "AED 123.45");
        assert_eq!(money(0), "AED 0");
        assert_eq!(money(100_000_000), "AED 1,000,000");
    }

    #[test]
    fn a_text_is_filled_with_its_parameters() {
        assert_eq!(
            english("manpower.notice.subject", &[("title", &"Hello")]),
            "Hello"
        );
        assert_eq!(english("manpower.nothing", &[]), "manpower.nothing");
    }
}
