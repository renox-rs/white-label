# wl-attendance

Attendance: who was at work, where and when ([spec](../../docs/modules/wl-attendance.md)). Only the
attendance record is here so far; timesheets, rosters and the rest of the checks come later. Other
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
| The attendance record and its machine (`flag`); `record` with the late-entry window (`HRM.ACL.01`), self check-in, check-out | §3, §4 | `record`, `machine` |
| Overlap across locations and projects: self check-in refused, supervisor marking and imports flagged `overlap`; the crew list shows who is marked elsewhere | §4, §5.5 | `overlap` |
