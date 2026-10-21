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
        "manpower.problems.reference",
        "Give the contract a reference.",
    ),
    (
        "manpower.problems.dates",
        "The contract must end after it starts.",
    ),
    (
        "manpower.problems.payment_terms",
        "The payment terms, the notice period and the deemed-approval days cannot be negative or zero.",
    ),
    (
        "manpower.problems.signed_in_future",
        "A contract cannot be signed on a day that has not come.",
    ),
    (
        "manpower.problems.signed_document",
        "Attach the signed contract.",
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
        "No rate card version accepted by the client is in force on the start date of the contract.",
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
        "The notice period of the contract is {{ days }} days: the contract can end on {{ earliest }} at the earliest, unless the other side's waiver is attached.",
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
    // Ending for a reason that needs more (§7.1 `end`, BL-MPW-14).
    (
        "manpower.end.note_required",
        "Give the client's note: why the client rejected the worker.",
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
