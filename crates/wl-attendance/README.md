# wl-attendance

Attendance: who was at work, where and when ([spec](../../docs/modules/wl-attendance.md)). The
attendance record and its review are here, and the timesheet as far as `submit` and the adjustments need it; rosters and the rest come later. Other
modules ask about attendance only through the kernel (`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `attendance` (which needs `hr`):

```
use renox::prelude::*;

# let _ =
App::new().module(wl_attendance::WlAttendance::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`attendance.*`) | §6 | `permissions` |
| Work locations (name, scope, outdoor flag, the qualifications the context requires) | §3 | `location` |
| The attendance record and its machine (`flag`, `accept`, `correct`, `reject`, `lock`); `record` with the late-entry window (`HRM.ACL.01`), self check-in, check-out | §3, §4 | `record`, `machine` |
| Overlap across locations and projects: self check-in refused, supervisor marking and imports flagged `overlap`; the crew list shows who is marked elsewhere | §4, §5.5 | `overlap` |
| Actions: supervisor marking, the employee's own check-in and check-out, checked against the permission in the scope of the location | §5.1, §5.2, §6 | `pages`, `access` |
| `HRM.RUL.13` overtime cap: the day against the calendar's normal hours (rest days and holidays 0, Ramadan 2 h less) plus 2 h, and the rolling 21 days against 144 h; flags every open record of the day, once; `EntryHours` for the timesheet entries and `on_entry_saved` for when one is saved | §5.5, §6, §7 | `overtime` |
| `HRM.RUL.13` midday break on the worked span minus the breaks, at outdoor locations in the season; the window per scope and year (`MiddayWindow`), else 15 June to 15 September, 12:30 to 15:00; the pre-fill of the crew marking | §5.1, §5.5, §6, §7 | `midday` |
| `HRM.RUL.14` work permit, residence visa (`document_valid`) and the qualifications the work location requires (`qualification_valid`), at the attendance date; flags `permit_invalid`, tells HR and the supervisor, never refuses; a flagged record proposes no hours and holds `submit`, an accepted one proposes non-billable hours | §5.5, §6, §7 | `permit`, `proposal` |
| Review: `accept` and `correct` of a held record (`overtime_cap`, `permit_invalid`, `overlap`) need a reviewer's reason, not the marker, and for an `overlap` hours that no longer overlap (`HRM.ACL.09`); `reject` with a reason; the system accepts only a record with no flag; flags are settled, not deleted | §4, §6 | `review`, `flags` |
| Timesheets and their entries: the machine (`lock`, `reopen`), hours proposed from accepted attendance (non-billable for `permit_invalid`), entries by hand that feed the overtime cap | §3, §4, §5.4 | `timesheet`, `entry` |
| `submit` of a timesheet whose hours come from attendance: no flagged record in the period, every attended day allocated to a cost object, hours on a day over all submitted timesheets not above the attended hours (compared in minutes, locked per employee and day); the system submit at period end runs the same check and leaves a task | §4, §5.4, §5.5 | `submit` |
| Adjustment timesheet on a `locked` one: reason, origin, `affects`, signed entries; the original is never written (`adjusted_by` is the list of links); no net below zero per day and category (`HRM.ACL.08`) | §3, §4, §5.8 | `adjustment` |
| Approval: `approve` (internal approver, or the client's approver by role), `approve_on_behalf` with the signed sheet, `reject` with a reason; the creator of an adjustment never approves it; labour cost posted through the kernel's `CostRateResolver` where the context posts it; `TimesheetApproved` | §4, §5.8 | `approval` |
| The invoice dispute behind a `client_dispute` adjustment: the link must be an open dispute (kernel `Invoicer::dispute_state`) or a ticket; `affects` stays `billing` unless attendance evidence is attached; the approved adjustment takes the credit note of the `credited` dispute; a dispute that ends `upheld` refuses the adjustment's `submit` and tells its creator | §4, §5.7, §5.8 | `dispute` |
| `HRM.RUL.15` at E days (default 6, calendar days from the latest `submit`, daily 08:00 Asia/Dubai): an external timesheet still `submitted` tells the client's escalation contact (kernel `ApproverResolver`) and the account manager's manager, or the staff who may approve on behalf when there is none, and gives the account manager one task to collect a signed sheet; once per `submit`, the state does not change, the task closes on approval or `reject` | §5.6, §6, §7 | `escalation` |
| Closing the open timesheet of a context when its work ends: the kernel's `TimesheetClosing::close_open(context, owner, last_day)` cuts every `open` regular timesheet of the owner in the context that starts on or before the last day to it and submits it as the system at once, with the checks of any submit (one a check refuses stays `open` with its task and is reported); a submitted, approved or locked timesheet is not touched. 09 Manpower calls it at a deployment's and a contract's `end` | 09 §7.1 | `closing` |
| Hours for payroll at the cut-off: the kernel's `Timesheets::hours_for_payroll(period)` gives the approved hours of the timesheets whose last day is in the period, and for `open` or `submitted` ones the accepted attendance hours marked provisional (`flagged` and later records left out; payroll basis `attendance`, or `wait` for an internal approver only, refused for an external one); `record_payroll_use` at the payroll run's `submit` locks approved timesheets with `paid_by` and sets `paid_provisionally_by` on the others, keeping the hours paid; a provisionally paid timesheet is locked when approved and is not reopened | §5.7, §6, §7 | `payroll`, `config` |
| Reconciliation when a provisionally paid timesheet is approved (also on behalf or deemed): it is locked at once, the approved hours are compared with the hours paid per day and category, and a difference becomes an adjustment of origin `provisional_reconciliation` affecting `payroll`, submitted and approved by the system with the approver and basis of the original's approval, and `TimesheetReconciliationNeeded` (kernel); no `TimesheetApproved` for it; once per timesheet and payroll run | §5.7, §6 | `reconcile`, `approval` |
| `deem_approve` (`HRM.RUL.15` at N, in the same daily run, before the escalation): if the contract has a deemed-approval clause (N days, from the kernel `ApproverResolver`, read when the rule runs) an external timesheet still `submitted` N days after its latest `submit` is approved by the system with the basis `deemed` and everything `approve` does; the client's approvers and the account manager are told the hours and the dispute route; none for a context without a clause or an internal approver | §4, §5.6, §6, §7 | `deem`, `approval` |
| A day after the last working day of the context of a work location (a terminated project) is refused: `record` asks the app's `ContextClosure` (`wl-kernel`); attendance already recorded after that day is kept and flagged `after_last_day` for the reviewer through `AttendanceReview::flag_after`, and stays costed until accepted or adjusted | §4 `record`, 02 §7.1 `terminate` | `service`, `after_last_day` |
| The crew list page (`GET /attendance/crew`): without a location or a day it shows a form to pick them (the locations the person may mark at, the day, the crew) instead of an error; the list names the employees. A *Crew list* link joins the sidebar for who may mark attendance | §5.1 | `crew` |
