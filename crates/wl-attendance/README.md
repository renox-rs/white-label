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
