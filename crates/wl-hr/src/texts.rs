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
    ("wl_hr.employees.details", "Details"),
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
    // The payroll run page and its rules (`HRM.ACL.04`, `HRM.ACL.06`).
    ("wl_hr.payroll.title", "Payroll"),
    ("wl_hr.payroll.lines", "Lines"),
    ("wl_hr.payroll.no_lines", "No lines in this run"),
    ("wl_hr.payroll.employee", "Employee"),
    ("wl_hr.payroll.wage", "Fixed wage (AED)"),
    ("wl_hr.payroll.net", "Net (AED)"),
    ("wl_hr.payroll.reason", "Reason"),
    ("wl_hr.payroll.amount", "Deductions (AED)"),
    ("wl_hr.payroll.limit", "Cap (AED)"),
    ("wl_hr.payroll.cap", "Cap"),
    ("wl_hr.payroll.approve", "Approve"),
    ("wl_hr.payroll.submit", "Submit"),
    ("wl_hr.payroll.exceptions", "Lines above the Art. 25 caps"),
    (
        "wl_hr.payroll.exceptions_hint",
        "A salary recovery may not exceed its cap or 50% of the wage in total. Nothing is trimmed: reschedule the recovery, then calculate again.",
    ),
    (
        "wl_hr.payroll.approve_blocked",
        "A line is above the Art. 25 caps. Reschedule its recoveries first.",
    ),
    (
        "wl_hr.payroll.submit_no_wps_account",
        "The run cannot be submitted: no active WPS pay account for :employees. Add or activate their pay accounts, or hold their lines.",
    ),
    (
        "wl_hr.payroll.submit_blocked",
        "No active WPS pay account for: :employees.",
    ),
    (
        "wl_hr.payroll.caps_exceeded",
        "Approval is refused: :count cap(s) on salary recoveries are exceeded (Art. 25). Reschedule the recoveries and calculate again.",
    ),
    (
        "wl_hr.payroll.same_user",
        "A payroll run is prepared by one user and approved by another.",
    ),
    (
        "wl_hr.payroll.comment_required",
        "Comment on at least one line to fix before the run is returned.",
    ),
    ("wl_hr.payroll.comment", "Comment"),
    ("wl_hr.payroll.comment_line", "Comment on this line"),
    ("wl_hr.payroll.comment_saved", "Comment saved."),
    ("wl_hr.payroll.comment_submit", "Save comment"),
    (
        "wl_hr.payroll.comment_closed",
        "Lines can be commented while the run is calculated or approved.",
    ),
    ("wl_hr.payroll.return", "Return"),
    ("wl_hr.payroll.return_reason", "Returned because"),
    (
        "wl_hr.payroll.notice_required",
        "Attach the bank's rejection notice.",
    ),
    (
        "wl_hr.payroll.rejection_reason_invalid",
        "Give the reason: file format, establishment ID, funding or other.",
    ),
    (
        "wl_hr.payroll.fix_note_required",
        "Enter a note on what was done to fix the cause.",
    ),
    (
        "wl_hr.payroll.establishment_unchanged",
        "The establishment ID of the legal entity is still the one the rejected file was named with. Correct it first.",
    ),
    (
        "wl_hr.payroll.amounts_changed",
        "A net amount differs from the rejected file. Amounts cannot change on resubmit: correct them after payment with a supplementary run.",
    ),
    (
        "wl_hr.payroll.lines_changed",
        "The lines of the rejected file must all still be failed and unchanged.",
    ),
    (
        "wl_hr.payroll.no_file",
        "The run has no SIF file to resubmit.",
    ),
    ("wl_hr.payroll.status.rejected", "Rejected"),
    ("wl_hr.payroll.record_rejection", "Record rejection"),
    ("wl_hr.payroll.resubmit", "Resubmit"),
    ("wl_hr.payroll.rejection_reason", "Reason of the rejection"),
    (
        "wl_hr.payroll.rejection_notice",
        "Bank's rejection notice (file or reference)",
    ),
    ("wl_hr.payroll.fix_note", "What was done to fix the cause"),
    ("wl_hr.payroll.rejection.file_format", "File format"),
    (
        "wl_hr.payroll.rejection.establishment_id",
        "Establishment ID",
    ),
    ("wl_hr.payroll.rejection.funding", "Funding"),
    ("wl_hr.payroll.rejection.other", "Other"),
    ("wl_hr.payroll.rejected", "Rejection recorded."),
    (
        "wl_hr.payroll.resubmitted",
        "Resubmitted as a new SIF version.",
    ),
    ("wl_hr.payroll.files", "SIF files"),
    ("wl_hr.payroll.file_version", "Version"),
    ("wl_hr.payroll.file_name", "File"),
    ("wl_hr.payroll.file_lines", "Lines"),
    ("wl_hr.payroll.file_total", "Total (AED)"),
    (
        "wl_hr.payroll.result_file_required",
        "Attach the bank's result file.",
    ),
    (
        "wl_hr.payroll.no_failed_line",
        "Mark at least one failed line: a result file with none is a payment, not a partial rejection.",
    ),
    (
        "wl_hr.payroll.line_not_in_file",
        "A failed line must be a line of the file that was sent.",
    ),
    (
        "wl_hr.payroll.nothing_to_regenerate",
        "No line is left for a new file: every failed line was moved out. Confirm the payment instead.",
    ),
    (
        "wl_hr.payroll.failed_lines_confirm",
        "A line is still failed. Fix it, hold it or pay it another way before payment is confirmed.",
    ),
    (
        "wl_hr.payroll.confirmation_missing",
        "The bank's confirmation is missing for :count line(s) in a SIF file.",
    ),
    (
        "wl_hr.payroll.hold_reason_required",
        "Give the reason the line is held.",
    ),
    (
        "wl_hr.payroll.other_payment_invalid",
        "Give an approved way (transfer, cheque or cash) and its reference.",
    ),
    (
        "wl_hr.payroll.line_not_failed",
        "Only a failed line can be moved out of the run.",
    ),
    (
        "wl_hr.payroll.task_title",
        "Fix the failed payroll line of :employee (:reason)",
    ),
    (
        "wl_hr.payroll.task_description",
        "Payroll :period, SIF version :version. Change the pay account (needs approval), hold the line or pay it another approved way, then regenerate the file or resubmit.",
    ),
    ("wl_hr.payroll.record_partial", "Record the bank's result"),
    (
        "wl_hr.payroll.result_file",
        "Bank's result file (file or reference)",
    ),
    ("wl_hr.payroll.failed_reason", "Reason from the result file"),
    ("wl_hr.payroll.regenerate", "Regenerate"),
    ("wl_hr.payroll.confirm_paid", "Confirm paid"),
    (
        "wl_hr.payroll.confirmation",
        "Bank's confirmation (file or reference)",
    ),
    ("wl_hr.payroll.partial_recorded", "Result recorded."),
    ("wl_hr.payroll.paid_confirmed", "Payment confirmed."),
    ("wl_hr.payroll.hold", "Hold"),
    ("wl_hr.payroll.hold_line", "Hold this line"),
    ("wl_hr.payroll.hold_reason", "Why it is held"),
    ("wl_hr.payroll.held", "Line held."),
    ("wl_hr.payroll.pay_other", "Paid another way"),
    ("wl_hr.payroll.pay_other_line", "Paid another approved way"),
    ("wl_hr.payroll.other_method", "Method"),
    ("wl_hr.payroll.other_reference", "Reference"),
    ("wl_hr.payroll.other_paid", "Payment recorded."),
    ("wl_hr.payroll.method.transfer", "Transfer"),
    ("wl_hr.payroll.method.cheque", "Cheque"),
    ("wl_hr.payroll.method.cash", "Cash"),
    ("wl_hr.payroll.payslip", "Payslip"),
    ("wl_hr.payroll.payslip_published", "Published"),
    (
        "wl_hr.payroll.partial_hint",
        "Attach the bank's result file and list each failed line as <line id>=<reason from the file>, one per row. The other lines of the file are paid.",
    ),
    ("wl_hr.payroll.failed_lines", "Failed lines"),
    ("wl_hr.payroll.status.draft", "Draft"),
    ("wl_hr.payroll.status.calculated", "Calculated"),
    ("wl_hr.payroll.status.approved", "Approved"),
    ("wl_hr.payroll.status.submitted", "Submitted"),
    ("wl_hr.payroll.status.partially_rejected", "Partly rejected"),
    ("wl_hr.payroll.status.paid", "Paid"),
    ("wl_hr.payroll.line.included", "Included"),
    ("wl_hr.payroll.line.held", "Held"),
    ("wl_hr.payroll.line.paid", "Paid"),
    ("wl_hr.payroll.line.failed", "Failed"),
    (
        "wl_hr.payroll.cap.overpayment",
        "Overpayment recovery (20%)",
    ),
    ("wl_hr.payroll.cap.fine", "Fines (5%)"),
    ("wl_hr.payroll.cap.court_debt", "Court debts (25%)"),
    ("wl_hr.payroll.cap.damage", "Damage (5 days' wage)"),
    ("wl_hr.payroll.cap.total", "All deductions (50%)"),
    // Correcting a paid run (issue #207).
    (
        "wl_hr.payroll.supplementary",
        "Correct with a supplementary run",
    ),
    ("wl_hr.payroll.supplementary_runs", "Supplementary runs"),
    (
        "wl_hr.payroll.supplementary_reason",
        "Why the run is corrected",
    ),
    (
        "wl_hr.payroll.supplementary_rows",
        "Affected employees, one per row: employee id = corrected net in dirhams = component",
    ),
    (
        "wl_hr.payroll.supplementary_hint",
        "The paid run is not changed. The run lists only the employees whose pay differs from what was paid.",
    ),
    (
        "wl_hr.payroll.supplementary_made",
        "Supplementary run made.",
    ),
    ("wl_hr.payroll.corrects", "Corrects a paid run"),
    ("wl_hr.payroll.corrects_link", "Open the paid run"),
    ("wl_hr.payroll.component", "Component"),
    (
        "wl_hr.payroll.supplementary_reason_required",
        "Give the reason for the correction.",
    ),
    (
        "wl_hr.payroll.supplementary_run_not_paid",
        "Only a paid run is corrected by a supplementary run.",
    ),
    (
        "wl_hr.payroll.supplementary_nothing_to_correct",
        "No employee has a difference to the pay already made.",
    ),
    (
        "wl_hr.payroll.supplementary_correction_invalid",
        "Each employee is listed once, with a corrected net of zero or more.",
    ),
    (
        "wl_hr.payroll.supplementary_recovery_note",
        "Overpayment found by supplementary run :run",
    ),
    // Releasing held lines (issue #208).
    (
        "wl_hr.payroll.late_warning",
        "This pays after the WPS due date :due.",
    ),
    (
        "wl_hr.payroll.carry_reason_required",
        "Give the reason for carrying the line into the next regular run.",
    ),
    (
        "wl_hr.payroll.line_not_releasable",
        "Only a held line of a paid run whose hold reason has cleared can be released.",
    ),
    (
        "wl_hr.payroll.carry_target_invalid",
        "A line is carried into a later regular run of the same legal entity that is still being prepared.",
    ),
    ("wl_hr.payroll.line_not_held", "The line is not held."),
    (
        "wl_hr.payroll.already_released",
        "The line was already released into a run.",
    ),
    (
        "wl_hr.payroll.carryable",
        "Held lines that can be carried here",
    ),
    ("wl_hr.payroll.carry", "Carry into this run"),
    ("wl_hr.payroll.carry_reason", "Why it is carried"),
    ("wl_hr.payroll.carried", "Line carried into the run."),
    ("wl_hr.payroll.released", "Hold released."),
    ("wl_hr.payroll.release", "Release the hold"),
    (
        "wl_hr.payroll.release_hint",
        "The reason for the hold is gone.",
    ),
    ("wl_hr.payroll.released_into", "Paid in run"),
    ("wl_hr.payroll.carried_from", "Pays a held line"),
    ("wl_hr.payroll.warning", "Warning"),
    // The calculation of the lines (issue #206).
    ("wl_hr.payroll.hours", "Hours"),
    ("wl_hr.payroll.provisional", "provisional"),
    ("wl_hr.payroll.variable", "Variable pay (AED)"),
    ("wl_hr.payroll.deductions", "Deductions (AED)"),
    (
        "wl_hr.payroll.exception.missing_timesheet",
        "Missing timesheet",
    ),
    ("wl_hr.payroll.differences", "Difference lines"),
    ("wl_hr.payroll.difference_source", "From"),
    ("wl_hr.payroll.difference_amount", "Difference (AED)"),
    (
        "wl_hr.payroll.difference.adjustment",
        "Adjustment timesheet",
    ),
    (
        "wl_hr.payroll.difference.reconciliation",
        "Reconciliation of provisional hours",
    ),
    ("wl_hr.payroll.convert", "Turn into a recovery plan"),
    (
        "wl_hr.payroll.convert_hint",
        "Spreads the recovery over the next months within the Art. 25 caps; recalculate the run afterwards.",
    ),
    ("wl_hr.payroll.convert_note", "Note"),
    ("wl_hr.payroll.difference_converted", "Recovery plan made."),
    (
        "wl_hr.payroll.difference_note",
        "Recovery of hours taken back after they were paid",
    ),
    (
        "wl_hr.payroll.difference_not_in_open_run",
        "The difference is not in a run that is still being prepared.",
    ),
    (
        "wl_hr.payroll.difference_not_negative",
        "Only a negative difference that is still open becomes a recovery plan.",
    ),
    (
        "wl_hr.payroll.no_room_for_recovery",
        "No wage is known to plan the recovery within the Art. 25 caps.",
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
    (
        "wl_hr.exits.death_date_required",
        "The date of death is required.",
    ),
    (
        "wl_hr.exits.death_date_future",
        "The date of death cannot be in the future.",
    ),
    (
        "wl_hr.payroll.settled_in_settlement",
        "The line was settled in the final settlement of a death: it is not released.",
    ),
    (
        "wl_hr.death.task_visa_cancellation",
        "Cancel the residence visa of :name (died in service)",
    ),
    (
        "wl_hr.death.task_visa_cancellation_description",
        "Complete the cancellation visa case that starts from the death certificate.",
    ),
    (
        "wl_hr.death.task_insurance_claim",
        "File the insurance claim for :name (died in service)",
    ),
    (
        "wl_hr.death.task_insurance_claim_description",
        "File the claim with the insurer of the employee.",
    ),
    (
        "wl_hr.death.task_repatriation",
        "Arrange the repatriation of :name (died in service)",
    ),
    (
        "wl_hr.death.task_repatriation_description",
        "Arrange the return of the body and the belongings; whether the employer bears the cost is open (L-25).",
    ),
    ("wl_hr.exit.label", "Exit"),
    (
        "wl_hr.exit.wrong_state",
        "Only an employee serving notice, absconded or deceased can exit.",
    ),
    (
        "wl_hr.exit.cancellation_required",
        "The cancellation visa case is not completed and the authority's cancellation reference is not recorded.",
    ),
    (
        "wl_hr.exit.settlement_disputed",
        "The final settlement is disputed: resolve the dispute first.",
    ),
    (
        "wl_hr.exit.settlement_approved_required",
        "The final settlement must be approved (or paid) before the employee exits.",
    ),
    (
        "wl_hr.exit.settlement_paid_required",
        "The final settlement must be paid before the employee exits.",
    ),
    (
        "wl_hr.exit.reference_required",
        "Give the authority's cancellation reference.",
    ),
    (
        "wl_hr.exit.reference_saved",
        "The cancellation reference is recorded.",
    ),
    (
        "wl_hr.exit.reference_label",
        "Authority's cancellation reference",
    ),
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
        "wl_hr.exits.return_date_required",
        "Give the date the worker returned.",
    ),
    (
        "wl_hr.exits.return_date_future",
        "The date the worker returned cannot be in the future.",
    ),
    (
        "wl_hr.exits.new_case_open",
        "The new visa case is still open: cancel it or reject it first.",
    ),
    (
        "wl_hr.exits.cancellation_required",
        "A work permit or entry permit was issued: complete a cancellation visa case, or record that no cancellation is needed on the new visa case.",
    ),
    (
        "wl_hr.exits.settlement_required",
        "The employee has attendance: pay a final settlement with the reason onboarding cancelled first.",
    ),
    (
        "wl_hr.exits.cancellation_completed",
        "The cancellation visa case is completed: the visa is gone.",
    ),
    ("wl_hr.exits.settlement_paid", "Settlement already paid."),
    ("wl_hr.exits.settlement_disputed", "Settlement is disputed."),
    (
        "wl_hr.exits.settlement_part_paid",
        "A payment is recorded against the settlement.",
    ),
    (
        "wl_hr.exits.notice_withdrawn_reason",
        "Notice withdrawn by agreement.",
    ),
    (
        "wl_hr.exits.reinstated_reason",
        "The absconded employee was reinstated.",
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
    // Visa cases (#193, #195).
    (
        "wl_hr.visa.reference_required",
        "Give the authority's reference of the refusal.",
    ),
    (
        "wl_hr.visa.task_new_title",
        "Visa refused for :employee: cancel onboarding or open another case",
    ),
    (
        "wl_hr.visa.task_new_description",
        "The authority refused the new visa case. Cancel the onboarding, or open another visa case.",
    ),
    (
        "wl_hr.visa.task_renewal_title",
        "Visa renewal refused for :employee: try another route or give notice",
    ),
    (
        "wl_hr.visa.task_renewal_description",
        "The authority refused the renewal. Open a new case by another route, or give notice and open a cancellation case, by :deadline (before the current residence visa expires).",
    ),
    (
        "wl_hr.visa.not_needed_reason_required",
        "Give the reason no cancellation case is needed.",
    ),
    (
        "wl_hr.visa.not_needed_new_only",
        "Only a new visa case can be marked cancellation not needed.",
    ),
    (
        "wl_hr.visa.not_needed_no_permit",
        "No work permit or entry permit was issued on this case, so a cancellation is not in question.",
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
        "wl_hr.notices.payroll_returned_subject",
        "Payroll run {{ period }} was returned",
    ),
    (
        "wl_hr.notices.payroll_returned_body",
        "The payroll run {{ period }} was returned to draft: {{ reason }}. Lines to fix: {{ comments }}",
    ),
    (
        "wl_hr.notices.payroll_failed_subject",
        "WPS file of payroll {{ period }} failed",
    ),
    (
        "wl_hr.notices.payroll_failed_body",
        "{{ count }} line(s) of the salary file of payroll {{ period }} failed: {{ reasons }}. See the run and fix them before the WPS due date.",
    ),
    (
        "wl_hr.notices.payroll_escalated_subject",
        "WPS file of payroll {{ period }} is still not paid",
    ),
    (
        "wl_hr.notices.payroll_escalated_body",
        "The salary file of payroll {{ period }} was rejected by the bank and the run is still {{ status }} ({{ count }} failed line(s)). The WPS due date is {{ due }}.",
    ),
    (
        "wl_hr.notices.payroll_released_subject",
        "Held pay of payroll {{ period }} can be paid",
    ),
    (
        "wl_hr.notices.payroll_released_body",
        "{{ count }} held line(s) of payroll {{ period }} are released. A draft off-cycle run holds them: calculate, approve and submit it, or carry them into the next regular run with a reason.",
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
    // Pay accounts (`HRM.ACL.05`, issues #209 to #211).
    ("wl_hr.pay_account.title", "Pay account"),
    ("wl_hr.pay_account.accounts", "Pay accounts"),
    ("wl_hr.pay_account.number", "Account"),
    ("wl_hr.pay_account.account_number", "IBAN or card number"),
    ("wl_hr.pay_account.channel_label", "Type"),
    (
        "wl_hr.pay_account.channel.bank_account",
        "Bank account (IBAN)",
    ),
    ("wl_hr.pay_account.channel.salary_card", "Salary card"),
    ("wl_hr.pay_account.routing_code", "Routing code"),
    ("wl_hr.pay_account.holder", "Account holder"),
    ("wl_hr.pay_account.explanation", "Name mismatch explanation"),
    ("wl_hr.pay_account.cooling_ends", "Used by payroll from"),
    ("wl_hr.pay_account.notice", "Notice to the worker"),
    (
        "wl_hr.pay_account.notice_sent",
        "Sent by SMS to the number on record",
    ),
    (
        "wl_hr.pay_account.notice_undelivered_state",
        "Not delivered: reach the worker another way",
    ),
    ("wl_hr.pay_account.reached_label", "Worker reached"),
    ("wl_hr.pay_account.reason", "Reason"),
    ("wl_hr.pay_account.request", "New pay account"),
    (
        "wl_hr.pay_account.request_hint",
        "Another user approves it, the worker is told by SMS, and payroll uses it only after the cooling period.",
    ),
    (
        "wl_hr.pay_account.requested",
        "Pay account requested. Another user has to approve it.",
    ),
    (
        "wl_hr.pay_account.status.pending_approval",
        "Waiting for approval",
    ),
    ("wl_hr.pay_account.status.cooling", "Cooling"),
    ("wl_hr.pay_account.status.active", "Active"),
    ("wl_hr.pay_account.status.retired", "Retired"),
    ("wl_hr.pay_account.status.rejected", "Rejected"),
    ("wl_hr.pay_account.method.call", "Call"),
    ("wl_hr.pay_account.method.visit", "Visit"),
    ("wl_hr.pay_account.method.in_person", "In person"),
    ("wl_hr.pay_account.waive", "Waive cooling"),
    (
        "wl_hr.pay_account.waive_hint",
        "An MD has to approve it. Give the reason, e.g. a failed WPS line and the worker verified with their Emirates ID.",
    ),
    ("wl_hr.pay_account.waive_reason", "Reason"),
    (
        "wl_hr.pay_account.waive_pending",
        "Waiting for an MD to approve the waiver of the cooling.",
    ),
    (
        "wl_hr.pay_account.waive_sent",
        "Sent to an MD for approval. The account stays cooling until then.",
    ),
    ("wl_hr.pay_account.reached", "Record worker reached"),
    (
        "wl_hr.pay_account.reached_hint",
        "The notice was not delivered. Write down who reached the worker, how and when.",
    ),
    ("wl_hr.pay_account.reached_by", "Reached by"),
    ("wl_hr.pay_account.reached_method", "How"),
    ("wl_hr.pay_account.reached_on", "Date"),
    (
        "wl_hr.pay_account.reached_recorded",
        "Worker reached recorded.",
    ),
    (
        "wl_hr.pay_account.stopped",
        "The change of pay account was stopped.",
    ),
    (
        "wl_hr.pay_account.stop_title",
        "Stop the change of your pay account",
    ),
    (
        "wl_hr.pay_account.stop_hint",
        "If you did not ask for this change, stop it. Your salary then keeps going to your current account. You can stop it until",
    ),
    ("wl_hr.pay_account.stop_button", "Stop the change"),
    (
        "wl_hr.pay_account.stop_confirm",
        "Your current account stays in use and HR is told.",
    ),
    (
        "wl_hr.pay_account.stop_done",
        "The change was stopped. Your current account stays in use and the company was told.",
    ),
    (
        "wl_hr.pay_account.stop_closed",
        "This change can no longer be stopped with this link. Contact HR.",
    ),
    ("wl_hr.pay_account.stopped_by_worker", "the worker"),
    ("wl_hr.pay_account.stopped_by_hr", "HR"),
    (
        "wl_hr.pay_account.security_title",
        "Security: pay account change of :employee (…:last4) stopped",
    ),
    (
        "wl_hr.pay_account.security_description",
        "The change to the pay account ending :last4 was stopped by :by during the cooling period. The old account stays in use. Check whether the request was made without the worker's knowledge.",
    ),
    (
        "wl_hr.pay_account.task_title",
        "Reach :employee about the new pay account (…:last4)",
    ),
    (
        "wl_hr.pay_account.task_description",
        "The notice of the new pay account was not delivered (no mobile number on record, or the SMS gave up). The account stays cooling past :date until you reach the worker another way and record it: who, how and when.",
    ),
    (
        "wl_hr.pay_account.iban_invalid",
        "The IBAN is not valid. A UAE IBAN has 23 characters: AE, two check digits and 19 digits.",
    ),
    (
        "wl_hr.pay_account.card_invalid",
        "The salary card number is not valid. It has digits only.",
    ),
    (
        "wl_hr.pay_account.channel_invalid",
        "Choose a bank account or a salary card.",
    ),
    (
        "wl_hr.pay_account.routing_invalid",
        "The routing code is not valid. It has 9 digits and names a known bank or agent.",
    ),
    (
        "wl_hr.pay_account.holder_required",
        "Give the name of the account holder.",
    ),
    (
        "wl_hr.pay_account.explanation_required",
        "The account holder's name differs from the employee's. Explain why.",
    ),
    (
        "wl_hr.pay_account.already_open",
        "This employee already has a pay account waiting for approval or cooling.",
    ),
    (
        "wl_hr.pay_account.same_user",
        "The user who asked for the account cannot approve it.",
    ),
    (
        "wl_hr.pay_account.cooling_not_ended",
        "The cooling period has not ended.",
    ),
    (
        "wl_hr.pay_account.notice_undelivered",
        "The notice to the worker was not delivered. Reach the worker and record it first.",
    ),
    (
        "wl_hr.pay_account.waive_reason_required",
        "Give the reason for waiving the cooling.",
    ),
    (
        "wl_hr.pay_account.not_cooling",
        "The account is not cooling.",
    ),
    (
        "wl_hr.pay_account.reached_invalid",
        "Say who reached the worker, how (call, visit, in person) and when (not after today).",
    ),
    (
        "wl_hr.notices.pay_account_requested_subject",
        "New pay account of {{ employee }} waits for approval",
    ),
    (
        "wl_hr.notices.pay_account_requested_body",
        "A new pay account ending {{ last4 }} was requested for {{ employee }}. Name mismatch explanation: {{ explanation }}. Check it and approve or reject it; you cannot approve your own request.",
    ),
    (
        "wl_hr.notices.pay_account_rejected_subject",
        "Pay account of {{ employee }} was rejected",
    ),
    (
        "wl_hr.notices.pay_account_rejected_body",
        "The new pay account ending {{ last4 }} of {{ employee }} was rejected: {{ reason }}.",
    ),
    (
        "wl_hr.notices.pay_account_stopped_subject",
        "Pay account change of {{ employee }} was stopped",
    ),
    (
        "wl_hr.notices.pay_account_stopped_body",
        "The change to the pay account ending {{ last4 }} of {{ employee }} was stopped by {{ by }}. The old account stays in use and a security task was opened.",
    ),
    (
        "wl_hr.notices.pay_account_change_sms",
        "Your pay account ending {{ last4 }} takes effect on {{ date }}. If this was not you, stop it now: {{ link }}",
    ),
    (
        "wl_hr.notices.pay_account_change_subject",
        "Your pay account is changing",
    ),
    (
        "wl_hr.notices.pay_account_change_body",
        "A new pay account ending {{ last4 }} takes effect on {{ date }}. Until then your salary goes to your current account. If this was not you, stop the change: {{ link }}",
    ),
    // Final settlements (#212 to #215).
    ("wl_hr.settlement.settlements", "Final settlements"),
    ("wl_hr.settlement.title", "Final settlement"),
    ("wl_hr.settlement.reason", "Reason"),
    ("wl_hr.settlement.payee", "Payee"),
    ("wl_hr.settlement.payee.employee", "Employee"),
    ("wl_hr.settlement.payee.estate", "Estate of the employee"),
    ("wl_hr.settlement.status.draft", "Draft"),
    ("wl_hr.settlement.status.approved", "Approved"),
    ("wl_hr.settlement.status.disputed", "Disputed"),
    ("wl_hr.settlement.status.paid", "Paid"),
    ("wl_hr.settlement.status.closed", "Closed"),
    ("wl_hr.settlement.status.cancelled", "Cancelled"),
    ("wl_hr.settlement.source.worker", "The worker"),
    ("wl_hr.settlement.source.mohre_complaint", "MoHRE complaint"),
    ("wl_hr.settlement.source.court", "Court case"),
    ("wl_hr.settlement.outcome", "Outcome"),
    ("wl_hr.settlement.outcome.upheld", "Upheld"),
    ("wl_hr.settlement.outcome.revised", "Revised"),
    ("wl_hr.settlement.method.wps", "WPS"),
    ("wl_hr.settlement.method.transfer", "Bank transfer"),
    ("wl_hr.settlement.last_working_day", "Last working day"),
    ("wl_hr.settlement.deadline", "Legal deadline"),
    ("wl_hr.settlement.unpaid_salary", "Unpaid salary (AED)"),
    (
        "wl_hr.settlement.leave_encashment",
        "Leave encashment (AED)",
    ),
    ("wl_hr.settlement.gratuity", "Gratuity (AED)"),
    ("wl_hr.settlement.notice_pay", "Notice pay (AED)"),
    ("wl_hr.settlement.other_dues", "Other dues (AED)"),
    ("wl_hr.settlement.dues", "Dues (AED)"),
    (
        "wl_hr.settlement.advance_recoveries",
        "Advances and loans to recover",
    ),
    ("wl_hr.settlement.fine_recoveries", "Fines to recover"),
    (
        "wl_hr.settlement.court_debt_recoveries",
        "Court debts to recover",
    ),
    ("wl_hr.settlement.damage_recoveries", "Damage to recover"),
    ("wl_hr.settlement.recoveries", "Recoveries (AED)"),
    (
        "wl_hr.settlement.offset",
        "Recoveries offset within the Art. 25 caps",
    ),
    ("wl_hr.settlement.net", "Net (AED)"),
    (
        "wl_hr.settlement.owed",
        "Due from the employee (recoveries above the caps)",
    ),
    ("wl_hr.settlement.paid_so_far", "Paid so far (AED)"),
    ("wl_hr.settlement.unpaid", "Net still unpaid (AED)"),
    ("wl_hr.settlement.statement", "Statement version"),
    ("wl_hr.settlement.closes_on", "Closes without a dispute on"),
    (
        "wl_hr.settlement.acknowledgement",
        "Acknowledgement signed on",
    ),
    (
        "wl_hr.settlement.representative",
        "Representative of the estate",
    ),
    (
        "wl_hr.settlement.right_document",
        "Document giving the right to receive",
    ),
    ("wl_hr.settlement.enter_amounts", "Enter the amounts"),
    (
        "wl_hr.settlement.amounts_hint",
        "Amounts in dirhams. Recoveries are offset against the dues only within the Art. 25 caps; the rest is owed by the employee.",
    ),
    ("wl_hr.settlement.save", "Save"),
    ("wl_hr.settlement.amounts_saved", "Amounts saved."),
    ("wl_hr.settlement.pay", "Pay"),
    (
        "wl_hr.settlement.pay_hint",
        "Record the payment of the net still unpaid, or leave the amount at 0 when it is already paid or nothing is payable.",
    ),
    ("wl_hr.settlement.paid", "Settlement paid."),
    ("wl_hr.settlement.record_payment", "Record a payment"),
    (
        "wl_hr.settlement.record_payment_hint",
        "A payment against the settlement, e.g. the undisputed amount while it is disputed.",
    ),
    ("wl_hr.settlement.payment_recorded", "Payment recorded."),
    ("wl_hr.settlement.payments", "Payments"),
    ("wl_hr.settlement.payment_date", "Date"),
    ("wl_hr.settlement.payment_amount", "Amount"),
    ("wl_hr.settlement.payment_method", "Method"),
    ("wl_hr.settlement.payment_reference", "Reference"),
    ("wl_hr.settlement.dispute", "Dispute"),
    (
        "wl_hr.settlement.dispute_hint",
        "The worker contests the amounts, or a MoHRE complaint or court case is open. The undisputed amount can still be paid.",
    ),
    ("wl_hr.settlement.dispute_title", "Dispute"),
    ("wl_hr.settlement.dispute_source", "Source"),
    ("wl_hr.settlement.dispute_reference", "Reference"),
    ("wl_hr.settlement.undisputed", "Undisputed amount (AED)"),
    ("wl_hr.settlement.hearing_on", "Hearing date"),
    ("wl_hr.settlement.reply_due_on", "Reply due on"),
    ("wl_hr.settlement.ticket", "Labour-case ticket"),
    ("wl_hr.settlement.disputed", "Settlement disputed."),
    ("wl_hr.settlement.resolve", "Resolve"),
    (
        "wl_hr.settlement.resolve_hint",
        "Upheld keeps the amounts; revised takes the amounts the preparer entered and needs a user other than the preparer.",
    ),
    (
        "wl_hr.settlement.resolution_document",
        "Decision, agreement or judgment",
    ),
    ("wl_hr.settlement.resolved", "Dispute resolved."),
    (
        "wl_hr.settlement.acknowledge",
        "Record the signed acknowledgement",
    ),
    (
        "wl_hr.settlement.acknowledge_hint",
        "The signed receipt of payment closes the settlement. It is not a waiver of any claim.",
    ),
    ("wl_hr.settlement.signed_on", "Signed on"),
    (
        "wl_hr.settlement.acknowledged",
        "Acknowledgement recorded; the settlement is closed.",
    ),
    (
        "wl_hr.settlement.same_user",
        "The user who entered the amounts cannot approve them.",
    ),
    (
        "wl_hr.settlement.amount_negative",
        "An amount cannot be negative.",
    ),
    (
        "wl_hr.settlement.amounts_state",
        "The amounts can be entered only while the settlement is a draft or disputed.",
    ),
    (
        "wl_hr.settlement.payment_state",
        "A payment can be recorded only while the settlement is approved or disputed.",
    ),
    (
        "wl_hr.settlement.payment_amount_required",
        "Give the amount of the payment.",
    ),
    (
        "wl_hr.settlement.payment_method_required",
        "Pay through WPS or a bank transfer.",
    ),
    (
        "wl_hr.settlement.payment_above_net",
        "The payments cannot be more than the net of the settlement.",
    ),
    (
        "wl_hr.settlement.net_unpaid",
        "The net of the settlement is not paid in full.",
    ),
    (
        "wl_hr.settlement.estate_payee_required",
        "Record the estate's representative and the document giving the right to receive before paying.",
    ),
    (
        "wl_hr.settlement.dispute_source_required",
        "Give the source of the dispute (the worker, a MoHRE complaint or a court case) and its reference.",
    ),
    (
        "wl_hr.settlement.dispute_undisputed_required",
        "Enter the undisputed amount, from 0 up to the net.",
    ),
    (
        "wl_hr.settlement.resolve_outcome_required",
        "Give the outcome: upheld or revised.",
    ),
    (
        "wl_hr.settlement.resolve_document_required",
        "Attach the decision, agreement or judgment.",
    ),
    (
        "wl_hr.settlement.revised_amounts_required",
        "Enter the revised amounts before resolving as revised.",
    ),
    (
        "wl_hr.settlement.resolve_revised_approves",
        "A revised settlement goes back to approved.",
    ),
    (
        "wl_hr.settlement.resolve_unpaid_net",
        "The net is not paid in full; the settlement goes back to approved.",
    ),
    (
        "wl_hr.settlement.resolve_closes",
        "The net is paid in full; the settlement closes.",
    ),
    (
        "wl_hr.settlement.not_disputed",
        "The settlement is not disputed.",
    ),
    (
        "wl_hr.settlement.close_not_due",
        "The period to dispute has not ended and the acknowledgement is not signed.",
    ),
    (
        "wl_hr.settlement.cancel_paid",
        "A settlement with a payment recorded against it cannot be cancelled.",
    ),
    (
        "wl_hr.settlement.acknowledge_state",
        "An acknowledgement can be recorded only for a paid settlement.",
    ),
    (
        "wl_hr.settlement.acknowledge_not_asked",
        "The estate is not asked for an acknowledgement.",
    ),
    (
        "wl_hr.settlement.receivable_note",
        "Recoveries above the Art. 25 caps of final settlement :id.",
    ),
    (
        "wl_hr.settlement.recovery_task_title",
        "Recover AED :amount from the employee (final settlement :id)",
    ),
    (
        "wl_hr.settlement.recovery_task_description",
        "The recoveries of the final settlement were above what may be offset against the dues. The amount is a receivable on the employee's party.",
    ),
    (
        "wl_hr.settlement.dispute_ticket_title",
        "Settlement dispute: :employee (:reference)",
    ),
    (
        "wl_hr.settlement.dispute_ticket_description",
        "Source: :source. Reference: :reference. :dates",
    ),
    ("wl_hr.settlement.dispute_hearing", "Hearing: :date."),
    ("wl_hr.settlement.dispute_reply", "Reply due: :date."),
    (
        "wl_hr.settlement.dispute_ticket_resolved",
        "The dispute of the final settlement was resolved (:outcome).",
    ),
    (
        "wl_hr.notices.settlement_disputed_subject",
        "Final settlement of {{ employee }} disputed",
    ),
    (
        "wl_hr.notices.settlement_disputed_body",
        "The final settlement of {{ employee }} is disputed ({{ source }}, reference {{ reference }}). The undisputed amount of AED {{ undisputed }} can still be paid on time. The legal deadline is {{ deadline }}.",
    ),
    (
        "wl_hr.notices.settlement_due_subject",
        "Final settlement of {{ employee }} is due",
    ),
    (
        "wl_hr.notices.settlement_due_body",
        "The final settlement of {{ employee }} (last working day {{ last_working_day }}) is not paid yet. The legal deadline is {{ deadline }}.",
    ),
    (
        "wl_hr.notices.settlement_undisputed_due_subject",
        "Pay the undisputed amount of the settlement of {{ employee }}",
    ),
    (
        "wl_hr.notices.settlement_undisputed_due_body",
        "The final settlement of {{ employee }} is disputed and the undisputed amount of AED {{ undisputed }} is not paid in full. Pay it before the legal deadline, {{ deadline }}.",
    ),
    (
        "wl_hr.notices.settlement_overdue_subject",
        "Final settlement of {{ employee }} is overdue",
    ),
    (
        "wl_hr.notices.visa_rejected_subject",
        "Visa refused for {{ employee }}",
    ),
    (
        "wl_hr.notices.visa_rejected_body",
        "The authority refused the {{ case }} visa case of {{ employee }} (reference {{ reference }}): {{ reason }}. HR has a task to act by {{ deadline }}.",
    ),
    (
        "wl_hr.notices.settlement_overdue_body",
        "The final settlement of {{ employee }} (last working day {{ last_working_day }}) was due on {{ deadline }} and is still not paid.",
    ),
    // The tabs of the employee page (#511).
    ("wl_hr.tabs.overview", "Overview"),
    ("wl_hr.tabs.documents", "Documents"),
    ("wl_hr.tabs.deployments", "Deployments"),
    ("wl_hr.tabs.timesheets", "Timesheets"),
    ("wl_hr.tabs.salary", "Salary"),
    ("wl_hr.tabs.pay_accounts", "Pay accounts"),
    ("wl_hr.documents.kind", "Document"),
    ("wl_hr.documents.number", "Number"),
    ("wl_hr.documents.valid_to", "Valid to"),
    ("wl_hr.documents.validity", "Validity"),
    ("wl_hr.documents.role", "Responsible"),
    ("wl_hr.documents.open_all", "All documents and upload"),
    ("wl_hr.documents.empty", "No documents yet"),
    (
        "wl_hr.documents.empty_hint",
        "Upload the passport, visa, permit and Emirates ID of the worker.",
    ),
    ("wl_hr.documents.validity.valid", "Valid"),
    ("wl_hr.documents.validity.expiring", "Expiring soon"),
    ("wl_hr.documents.validity.expired", "Expired"),
    ("wl_hr.documents.validity.pending", "Waiting for review"),
    ("wl_hr.documents.validity.rejected", "Rejected"),
    ("wl_hr.deployments.client", "Client"),
    ("wl_hr.deployments.site", "Site"),
    ("wl_hr.deployments.role", "Role"),
    ("wl_hr.deployments.period", "Dates"),
    ("wl_hr.deployments.open_ended", "Open-ended"),
    ("wl_hr.deployments.bench", "On the bench"),
    (
        "wl_hr.deployments.bench_hint",
        "This worker has no deployment at the moment.",
    ),
    ("wl_hr.sheets.month", "Month"),
    ("wl_hr.sheets.hours", "Hours"),
    ("wl_hr.sheets.kind", "Kind"),
    ("wl_hr.sheets.empty", "No timesheets yet"),
    ("wl_hr.sheets.status.open", "Open"),
    ("wl_hr.sheets.status.submitted", "Submitted"),
    ("wl_hr.sheets.status.approved", "Approved"),
    ("wl_hr.sheets.status.locked", "Locked"),
    ("wl_hr.sheets.kind.regular", "Regular"),
    ("wl_hr.sheets.kind.adjustment", "Adjustment"),
    (
        "wl_hr.salary.hidden",
        "The amounts are hidden. Revealing them is recorded in the audit log.",
    ),
    ("wl_hr.salary.reveal", "Reveal"),
    (
        "wl_hr.salary.none",
        "No salary structure is recorded for this worker.",
    ),
    ("wl_hr.salary.since", "Applies from"),
    ("wl_hr.salary.basic", "Basic wage"),
    ("wl_hr.salary.housing", "Housing allowance"),
    ("wl_hr.salary.transport", "Transport allowance"),
    ("wl_hr.salary.other", "Other allowances"),
    ("wl_hr.salary.fixed", "Fixed wage a month"),
    ("wl_hr.salary.hourly", "Pay of one hour"),
    ("wl_hr.pay_account.bank", "Bank or channel"),
    ("wl_hr.pay_account.cooling_text", "Cooling ends"),
    ("wl_hr.pay_account.none", "No pay account yet"),
    // The run page: the file and the bank's result (#510).
    (
        "wl_hr.payroll.no_sif",
        "This run has no salary file yet. Submit the approved run first.",
    ),
    ("wl_hr.payroll.download_sif", "Download SIF"),
    ("wl_hr.payroll.file_result", "Bank's result"),
    ("wl_hr.payroll.upload_result", "Upload bank result"),
    (
        "wl_hr.payroll.upload_hint",
        "Upload the file the bank sent back, then record its number below.",
    ),
    (
        "wl_hr.payroll.result_summary",
        "The bank paid :paid line(s) and rejected :failed line(s). Fix each rejected line or move it out of the run, then regenerate the file.",
    ),
    // The lists of payroll runs and final settlements (#510).
    ("wl_hr.payroll.list.title", "Payroll runs"),
    (
        "wl_hr.payroll.list.subtitle",
        "One run for each month: calculate, approve, send to the bank, confirm.",
    ),
    ("wl_hr.payroll.list.new", "New run"),
    (
        "wl_hr.payroll.list.new_hint",
        "Makes the regular run of the month and calculates its lines from the salary structures and the timesheet hours.",
    ),
    ("wl_hr.payroll.list.new_submit", "Start the run"),
    ("wl_hr.payroll.list.period", "Month"),
    ("wl_hr.payroll.list.kind", "Kind"),
    ("wl_hr.payroll.list.total", "Total net"),
    ("wl_hr.payroll.list.empty", "No payroll runs yet"),
    (
        "wl_hr.payroll.list.empty_hint",
        "Start the run of a month to see its lines here.",
    ),
    (
        "wl_hr.payroll.list.bad_period",
        "Give the month as year and month, for example 2026-10.",
    ),
    (
        "wl_hr.payroll.list.period_taken",
        "A regular run for :period already exists. Open it from the list.",
    ),
    ("wl_hr.payroll.kind.regular", "Regular"),
    ("wl_hr.payroll.kind.off_cycle", "Off-cycle"),
    ("wl_hr.payroll.kind.supplementary", "Supplementary"),
    ("wl_hr.settlement.list.title", "Final settlements"),
    (
        "wl_hr.settlement.list.subtitle",
        "What each leaver is owed, with the legal deadline to pay it.",
    ),
    ("wl_hr.settlement.list.employee", "Employee"),
    ("wl_hr.settlement.list.dues", "Dues"),
    ("wl_hr.settlement.list.recoveries", "Recoveries"),
    ("wl_hr.settlement.list.net", "Net"),
    ("wl_hr.settlement.list.paid", "Paid"),
    ("wl_hr.settlement.list.overdue", "Overdue"),
    ("wl_hr.settlement.list.empty", "No final settlements yet"),
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
