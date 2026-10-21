# wl-hr

HR: the minimal employee record and the onboarding of new staff from the templates of their job role
([spec](../../docs/modules/wl-hr.md)). Payroll, the WPS file, visas, leave and the final settlement
come in phase F9 (decision 0005). Other modules ask about employees only through the kernel
(`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `hr`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_hr::WlHr::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`employees.*`, `onboarding.*`) | §6 | `permissions` |
| Manager and owner roles | §3, §7 | `config` |
| The employee record (person, job role, scopes, employment type, hire date, cost rate), created in `onboarding` when a template applies, else `active`; `activate` | §2, §3, §4 | `employee`, `employee_machine` |
| Onboarding templates per job role or trade and for all staff, with steps; the seeded induction (`hr:install`, the seeder) | §3, §7 | `template`, `seed` |
| `HRM.RUL.16`: the onboarding starts from the templates that apply when an employee is created in `onboarding`; HR starts a template by hand; templates are frozen, steps are assigned to HR, the manager, the employee or a role, due from the hire date; assignees follow the people; a corrected hire date moves the due dates; exit, death and a cancelled onboarding cancel it | §3, §4, §5.8 | `onboarding`, `onboarding_machine`, `assignees`, `events` |
| Doing the steps (`HRM.ACL.10`): `task` with evidence, `checklist` from the checklist run, `document` from the accepted document, `sop_acknowledgement` from the SOP acknowledgements, in person with the employee's signature and a witness; waive and reopen by another holder of `onboarding.manage`; the onboarding completes after the last required step | §3, §4 | `steps` |
| `HRM.RUL.17`: daily at 08:00 Asia/Dubai (`hr:step-reminders`), an open step past its due date is reminded to its assignees on the first run and then every 2 days, HR and the manager are told once after 5 days (not for an optional step); paused while the employee is `on_leave` or `absconded`; the due dates of open steps move by the days of approved leave as recorded now | §3, §6, §7 | `reminders`, `leave` |
| A template with *blocks activation* makes `activate` wait for its required steps (not other templates', not optional steps, not a manual onboarding); the refusal lists the open steps and their assignees | §3, §4, §7 | `activation`, `employee_machine` |
| Leave requests: `request`, `approve`, `reject`, `cancel`, `complete` (state machine); `LeaveApproved` and `LeaveCancelled` after the commit; days of sick leave beyond the paid bands and all days of unpaid leave are marked unpaid; permissions `leave.*` | §3, §4, §5.2 | `leave`, `leave_machine`, `leave_service` |
| `Workforce`: `employees_in(role, scope)`, `all_staff`, `staff` (role, scopes, account, manager, leave of one employee), `employee_of_user` | §6 | `workforce` |
| The employees, the employee page with its onboarding, and the templates page | §3, §4 | `pages` |

The module owns `src/texts.rs`, the English texts (`wl_hr.*`). The app changes or translates them
in `resources/lang/{locale}.json`.
