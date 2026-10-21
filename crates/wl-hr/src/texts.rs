//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_hr.cancel", "Cancel"),
    ("wl_hr.none", "—"),
    ("wl_hr.fields.status", "Status"),
    // Employees.
    ("wl_hr.employees.title", "Employees"),
    (
        "wl_hr.employees.subtitle",
        "Staff records with their onboarding.",
    ),
    ("wl_hr.employees.empty", "No employees yet"),
    ("wl_hr.employees.name", "Employee"),
    ("wl_hr.employees.number", "Number"),
    ("wl_hr.employees.job_role", "Job role"),
    ("wl_hr.employees.job_title", "Job title"),
    ("wl_hr.employees.employment_type", "Employment type"),
    ("wl_hr.employees.hire_date", "Hire date"),
    ("wl_hr.employees.scopes", "Works at"),
    ("wl_hr.employees.progress", "Onboarding"),
    ("wl_hr.employees.open", "Open"),
    ("wl_hr.employees.activate", "Activate"),
    ("wl_hr.employees.activated", "Employee activated."),
    (
        "wl_hr.employees.job_role_required",
        "Give the job role or trade.",
    ),
    (
        "wl_hr.employees.bad_initial_state",
        "A new employee starts in onboarding or active.",
    ),
    ("wl_hr.employees.status.onboarding", "Onboarding"),
    ("wl_hr.employees.status.active", "Active"),
    ("wl_hr.employees.status.on_leave", "On leave"),
    ("wl_hr.employees.status.notice", "On notice"),
    ("wl_hr.employees.status.absconded", "Absconded"),
    ("wl_hr.employees.status.deceased", "Deceased"),
    ("wl_hr.employees.status.exited", "Exited"),
    (
        "wl_hr.employees.status.onboarding_cancelled",
        "Onboarding cancelled",
    ),
    ("wl_hr.employees.type.full_time", "Full-time"),
    ("wl_hr.employees.type.part_time", "Part-time"),
    ("wl_hr.employees.type.temporary", "Temporary"),
    ("wl_hr.employees.type.freelance", "Freelance"),
    // Non-standard exits (`HRM.ACL.07`).
    // The absence of an employee and `report_absconding` (#197).
    ("wl_hr.absence.title", "Absence"),
    ("wl_hr.absence.attempts", "Attempts to reach the worker"),
    ("wl_hr.absence.record", "Record a contact attempt"),
    ("wl_hr.absence.submit", "Record"),
    ("wl_hr.absence.recorded", "Contact attempt recorded."),
    ("wl_hr.absence.none", "No attempts recorded"),
    ("wl_hr.absence.date", "Date"),
    ("wl_hr.absence.method", "Method (phone, SMS, visit)"),
    ("wl_hr.absence.outcome", "Outcome"),
    ("wl_hr.absence.note", "Note"),
    ("wl_hr.absence.by", "By"),
    ("wl_hr.absence.outcome.no_answer", "No answer"),
    ("wl_hr.absence.outcome.unreachable", "Unreachable"),
    ("wl_hr.absence.outcome.reached", "Reached"),
    (
        "wl_hr.absence.method_required",
        "Say how you tried to reach the worker.",
    ),
    (
        "wl_hr.absence.future_date",
        "The attempt cannot be dated after today.",
    ),
    (
        "wl_hr.absence.not_absent",
        "Attempts are recorded for an employee who is working or absconded.",
    ),
    (
        "wl_hr.absconding.no_case",
        "No absence case is open for this employee.",
    ),
    (
        "wl_hr.absconding.too_early",
        "More than :days days must have passed since the last day with attendance or approved leave (:last); :passed so far.",
    ),
    (
        "wl_hr.absconding.not_unreachable",
        "The worker counts as unreachable after :attempts failed contact attempts on different days of the absence.",
    ),
    (
        "wl_hr.payroll.reason_required",
        "Each failed line needs the reason from the bank's result file.",
    ),
    (
        "wl_hr.payroll.failed_lines_left",
        "Fix each failed line or move it out of the run (held) first.",
    ),
    ("wl_hr.exits.title", "Non-standard exit"),
    ("wl_hr.exits.request", "Request an exit action"),
    ("wl_hr.exits.submit", "Send for approval"),
    ("wl_hr.exits.sent", "Sent for approval."),
    ("wl_hr.exits.action", "Action"),
    ("wl_hr.exits.reason", "Reason"),
    ("wl_hr.exits.pending", "Waiting for approval"),
    ("wl_hr.exits.pending_by", "Requested by"),
    ("wl_hr.exits.documents", "Documents"),
    ("wl_hr.exits.report_number", "MoHRE absence report number"),
    ("wl_hr.exits.report_date", "Report date"),
    ("wl_hr.exits.report_file", "Report file (document number)"),
    (
        "wl_hr.exits.agreement",
        "Written agreement (document number or reference)",
    ),
    (
        "wl_hr.exits.reference",
        "Withdrawal or cancellation reference",
    ),
    (
        "wl_hr.exits.death_document",
        "Death certificate (document number or reference)",
    ),
    (
        "wl_hr.exits.authority_notice",
        "Authority notice (document number or reference)",
    ),
    ("wl_hr.exits.death_date", "Date of death"),
    ("wl_hr.exits.return_date", "Date the worker returned"),
    (
        "wl_hr.exits.transition.cancel_onboarding",
        "Cancel onboarding",
    ),
    ("wl_hr.exits.transition.withdraw_notice", "Withdraw notice"),
    (
        "wl_hr.exits.transition.report_absconding",
        "Report absconding",
    ),
    ("wl_hr.exits.transition.reinstate", "Reinstate"),
    ("wl_hr.exits.transition.record_death", "Record death"),
    ("wl_hr.exits.reason_required", "Give the reason."),
    (
        "wl_hr.exits.agreement_required",
        "Attach the other party's written agreement.",
    ),
    (
        "wl_hr.exits.report_required",
        "Give the MoHRE absence report number and date.",
    ),
    (
        "wl_hr.exits.reference_required",
        "Give the reference of the withdrawal or cancellation of the absence report.",
    ),
    (
        "wl_hr.exits.death_document_required",
        "Attach the death certificate or the authority's notice.",
    ),
    (
        "wl_hr.exits.unknown_action",
        "This is not an action that needs approval.",
    ),
    (
        "wl_hr.exits.wrong_state",
        "This action cannot be requested in the employee's current state.",
    ),
    (
        "wl_hr.exits.already_pending",
        "A request for this employee is already waiting for approval.",
    ),
    (
        "wl_hr.exits.no_policy",
        "No approval policy is active for this action, so it cannot be requested.",
    ),
    // Templates.
    ("wl_hr.templates.title", "Onboarding templates"),
    (
        "wl_hr.templates.subtitle",
        "The steps every new employee in a role goes through.",
    ),
    ("wl_hr.templates.empty", "No onboarding templates"),
    ("wl_hr.templates.name", "Template"),
    ("wl_hr.templates.applies_to", "Applies to"),
    ("wl_hr.templates.all_staff", "All staff"),
    ("wl_hr.templates.version", "Version"),
    ("wl_hr.templates.active", "Active"),
    ("wl_hr.templates.inactive", "Inactive"),
    ("wl_hr.templates.blocks", "Blocks activation"),
    ("wl_hr.templates.steps", "Steps"),
    ("wl_hr.templates.step", "Step"),
    ("wl_hr.templates.owner", "Owner"),
    ("wl_hr.templates.due", "Due (days)"),
    ("wl_hr.templates.kind", "Kind"),
    ("wl_hr.templates.required", "Required"),
    ("wl_hr.templates.optional", "Optional"),
    ("wl_hr.templates.name_required", "Give the template a name."),
    (
        "wl_hr.templates.no_role",
        "Name the job roles it applies to, or make it a template for all staff.",
    ),
    ("wl_hr.templates.no_steps", "A template needs a step."),
    ("wl_hr.templates.step_title", "Step :n needs a title."),
    (
        "wl_hr.templates.step_owner",
        "Step :n needs an owner: hr, manager, employee or role:<name>.",
    ),
    (
        "wl_hr.templates.step_due",
        "Step :n cannot be due before the start.",
    ),
    (
        "wl_hr.templates.step_kind",
        "Step :n needs the checklist template or the document kind of its kind.",
    ),
    ("wl_hr.owner.hr", "HR"),
    ("wl_hr.owner.manager", "Manager"),
    ("wl_hr.owner.employee", "Employee"),
    ("wl_hr.owner.role", "Role :name"),
    ("wl_hr.kind.task", "Task"),
    ("wl_hr.kind.checklist", "Checklist"),
    ("wl_hr.kind.document", "Document"),
    ("wl_hr.kind.sop_acknowledgement", "SOP acknowledgement"),
    // Onboarding.
    ("wl_hr.onboarding.title", "Onboarding"),
    ("wl_hr.onboarding.started_on", "Started"),
    ("wl_hr.onboarding.trigger", "Started by"),
    ("wl_hr.onboarding.trigger.hire", "Hire"),
    ("wl_hr.onboarding.trigger.manual", "HR"),
    ("wl_hr.onboarding.empty", "No onboarding"),
    ("wl_hr.onboarding.start", "Start an onboarding"),
    (
        "wl_hr.onboarding.start_title",
        "Start an onboarding by hand",
    ),
    (
        "wl_hr.onboarding.start_description",
        "Pick a template, e.g. for a new role; the steps are due from the start date.",
    ),
    ("wl_hr.onboarding.template", "Template"),
    ("wl_hr.onboarding.start_date", "Start date"),
    ("wl_hr.onboarding.started", "Onboarding started."),
    ("wl_hr.onboarding.cancel", "Cancel onboarding"),
    ("wl_hr.onboarding.cancel_title", "Cancel this onboarding?"),
    ("wl_hr.onboarding.cancelled", "Onboarding cancelled."),
    ("wl_hr.onboarding.reason", "Reason"),
    ("wl_hr.onboarding.status.in_progress", "In progress"),
    ("wl_hr.onboarding.status.completed", "Completed"),
    ("wl_hr.onboarding.status.cancelled", "Cancelled"),
    (
        "wl_hr.onboarding.employee_not_found",
        "There is no such employee.",
    ),
    (
        "wl_hr.onboarding.no_template",
        "No active onboarding template applies.",
    ),
    (
        "wl_hr.onboarding.already_running",
        "An onboarding from this template is already in progress for the employee.",
    ),
    (
        "wl_hr.onboarding.problem.open_steps",
        "A required step is still open.",
    ),
    ("wl_hr.onboarding.problem.reason", "Give the reason."),
    ("wl_hr.onboarding.cancelled_exit", "The employee exited."),
    (
        "wl_hr.onboarding.cancelled_death",
        "The employee died in service.",
    ),
    (
        "wl_hr.onboarding.manual_only",
        "Only a manual onboarding is cancelled by hand.",
    ),
    // Activation.
    (
        "wl_hr.activation.blocked",
        "Activation waits for these required onboarding steps:",
    ),
    ("wl_hr.activation.line", ":step (:template): :who"),
    // Steps.
    ("wl_hr.steps.title", "Step"),
    ("wl_hr.steps.assignees", "Assigned to"),
    ("wl_hr.steps.nobody", "In person"),
    ("wl_hr.steps.due", "Due"),
    ("wl_hr.steps.overdue", "Overdue"),
    ("wl_hr.steps.status.open", "Open"),
    ("wl_hr.steps.status.done", "Done"),
    ("wl_hr.steps.status.waived", "Waived"),
    ("wl_hr.steps.status.cancelled", "Cancelled"),
    ("wl_hr.steps.done_by", "Done by"),
    ("wl_hr.steps.witness", "Witness"),
    ("wl_hr.steps.failed_items", "Failed items"),
    ("wl_hr.steps.mark_done", "Mark done"),
    ("wl_hr.steps.mark_done_title", "Mark this step done"),
    ("wl_hr.steps.note", "Evidence note"),
    ("wl_hr.steps.document", "Evidence document number"),
    ("wl_hr.steps.in_person", "Record in person"),
    (
        "wl_hr.steps.in_person_title",
        "Record in person for the employee",
    ),
    (
        "wl_hr.steps.in_person_description",
        "The employee signs on this device; you are recorded as the witness, and the employee is told.",
    ),
    ("wl_hr.steps.signature", "Employee's drawn signature"),
    ("wl_hr.steps.waive", "Waive"),
    ("wl_hr.steps.reopen", "Reopen"),
    ("wl_hr.steps.reason", "Reason"),
    ("wl_hr.steps.start_checklist", "Start the checklist"),
    ("wl_hr.steps.done_ok", "Step done."),
    ("wl_hr.steps.waived_ok", "Step waived."),
    ("wl_hr.steps.reopened_ok", "Step reopened."),
    ("wl_hr.steps.checklist_ok", "Checklist started."),
    ("wl_hr.steps.no_sops", "No SOPs"),
    ("wl_hr.steps.problem.reason", "Give the reason."),
    ("wl_hr.steps.refused.not_found", "There is no such step."),
    (
        "wl_hr.steps.refused.closed",
        "The onboarding is no longer in progress.",
    ),
    ("wl_hr.steps.refused.not_open", "The step is not open."),
    (
        "wl_hr.steps.refused.not_settled",
        "Only a done or waived step is reopened.",
    ),
    (
        "wl_hr.steps.refused.not_assignee",
        "The step is not assigned to you.",
    ),
    (
        "wl_hr.steps.refused.employee_only",
        "The employee does this step in their own session, or in person with their signature.",
    ),
    (
        "wl_hr.steps.refused.signature",
        "An in-person record needs the employee's drawn signature.",
    ),
    (
        "wl_hr.steps.refused.not_manager",
        "You may not manage onboarding for this employee.",
    ),
    (
        "wl_hr.steps.refused.evidence",
        "This step needs an evidence file or note.",
    ),
    (
        "wl_hr.steps.refused.by_evidence",
        "This step is done by its checklist run or document, not by hand.",
    ),
    ("wl_hr.steps.refused.reason", "Give the reason."),
    (
        "wl_hr.steps.refused.assignee_waive",
        "An assignee cannot waive their own step.",
    ),
    (
        "wl_hr.steps.refused.same_user",
        "Another user than the one who marked the step reopens it.",
    ),
    (
        "wl_hr.steps.refused.cannot_reopen",
        "A step done by its checklist, document or acknowledgement follows its evidence.",
    ),
    (
        "wl_hr.steps.refused.no_checklist",
        "No checklist run could be started for this step.",
    ),
    // Notices.
    (
        "wl_hr.notices.step_assigned_subject",
        "Onboarding step for {{ employee }}: {{ step }}",
    ),
    (
        "wl_hr.notices.step_assigned_body",
        "The step \"{{ step }}\" of the onboarding of {{ employee }} is assigned to you. It is due {{ due }}.",
    ),
    (
        "wl_hr.notices.completed_subject",
        "Onboarding of {{ employee }} completed",
    ),
    (
        "wl_hr.notices.completed_body",
        "Every required step of the onboarding of {{ employee }} is done or waived.",
    ),
    (
        "wl_hr.notices.step_reopened_subject",
        "Onboarding step reopened: {{ step }}",
    ),
    (
        "wl_hr.notices.step_reopened_body",
        "The step \"{{ step }}\" of the onboarding of {{ employee }} was reopened: {{ reason }}. It was due {{ due }}.",
    ),
    (
        "wl_hr.notices.in_person_subject",
        "A step of your onboarding was recorded",
    ),
    (
        "wl_hr.notices.in_person_body",
        "The step \"{{ step }}\" of your onboarding was recorded for you in person on {{ at }}, witnessed by {{ witness }}. If this is not right, tell HR.",
    ),
    (
        "wl_hr.notices.step_reminder_subject",
        "Overdue onboarding step for {{ employee }}: {{ step }}",
    ),
    (
        "wl_hr.notices.step_reminder_body",
        "The step \"{{ step }}\" of the onboarding of {{ employee }} was due {{ due }} and is still open ({{ days }} days overdue).",
    ),
    (
        "wl_hr.notices.qualification_expired_subject",
        "Qualification of {{ employee }} expired: {{ kind }}",
    ),
    (
        "wl_hr.notices.qualification_expired_body",
        "The {{ kind }} of {{ employee }} expired on {{ expires_on }}. Open or planned work it blocks: {{ count }} {{ work }}",
    ),
    (
        "wl_hr.qualifications.kind_required",
        "Give the document kind of the qualification.",
    ),
    (
        "wl_hr.notices.step_escalated_subject",
        "Onboarding step of {{ employee }} is {{ days }} days overdue",
    ),
    (
        "wl_hr.notices.step_escalated_body",
        "The step \"{{ step }}\" of the onboarding of {{ employee }} was due {{ due }} and is still open, {{ days }} days overdue. Its assignees were reminded.",
    ),
    // Leave requests.
    (
        "wl_hr.leave.problem.sick_note",
        "A sick leave needs its sick note before it is approved.",
    ),
    (
        "wl_hr.leave.problem.started",
        "The leave has started; it can no longer be cancelled.",
    ),
    ("wl_hr.leave.problem.not_over", "The leave is not over yet."),
    ("wl_hr.leave.problem.type", "Choose a known leave type."),
    (
        "wl_hr.leave.problem.dates",
        "The last day cannot be before the first day.",
    ),
    ("wl_hr.leave.requested", "Leave requested."),
    ("wl_hr.leave.approved", "Leave approved."),
    ("wl_hr.leave.rejected", "Leave rejected."),
    ("wl_hr.leave.cancelled", "Leave cancelled."),
    // Same-day sick leave.
    (
        "wl_hr.sick.problem.not_today",
        "Same-day sick leave starts today.",
    ),
    (
        "wl_hr.sick.problem.not_sick",
        "Only a sick leave takes a sick note.",
    ),
    ("wl_hr.sick.task_title", "Sick note missing: :employee"),
    (
        "wl_hr.sick.task_description",
        ":employee called in sick on :from. The sick note was due by :due and is not attached.",
    ),
    ("wl_hr.sick.task_resolved", "The sick note was attached."),
    ("wl_hr.sick.title", "Leave"),
    ("wl_hr.sick.record", "Record sick leave"),
    ("wl_hr.sick.submit", "Record"),
    ("wl_hr.sick.from", "First day (today)"),
    ("wl_hr.sick.to", "Last day"),
    ("wl_hr.sick.recorded", "Sick leave recorded and approved."),
    ("wl_hr.sick.attach", "Attach sick note"),
    ("wl_hr.sick.attach_submit", "Attach"),
    ("wl_hr.sick.attached", "Sick note attached."),
    ("wl_hr.sick.request", "Leave"),
    ("wl_hr.sick.document", "Sick note number"),
    ("wl_hr.sick.type", "Type"),
    ("wl_hr.sick.period", "Period"),
    ("wl_hr.sick.unpaid", "Unpaid days"),
    ("wl_hr.sick.note", "Sick note"),
    ("wl_hr.sick.note_due", "Due by :date"),
    ("wl_hr.sick.note_attached", "Attached"),
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

/// Every text of the package in `lang`, keyed without the `wl_hr.` prefix
/// (`employees.title`), for the templates: `tx['employees.title']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_hr."), t(lang, key)))
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
        assert!(keys.iter().all(|key| key.starts_with("wl_hr.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_placeholder_is_filled_in() {
        assert_eq!(
            english("wl_hr.templates.step_title", &[("n", &3)]),
            "Step 3 needs a title."
        );
    }
}
