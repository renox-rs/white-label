# Epic: wl-attendance — negative flows

> Status: draft · Last updated: 2026-10-15 · Spec version: same commit as this file

This epic builds the attendance and timesheet negative flows added by the review fix X-16:
overlap, overtime-cap, midday-break and invalid-permit flags with a separate reviewer
(`HRM.ACL.09`), adjustment timesheets that correct a `locked` timesheet (`HRM.ACL.08`), the
approval escalation and deemed approval of external timesheets (`HRM.RUL.15`), and payroll from
attendance at the cut-off with later reconciliation. It closes X-16, 02-N5-d, 02-N8-a, 09-N4-a,
09-N5-b, 09-N5-c and the 09 MEDIUM items "overlap guards" and "overtime cap flags".

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-ATT-01 | Detect overlapping attendance across locations and projects | high | m | Build 1 | Base: wl-attendance Attendance record lifecycle |
| BL-ATT-02 | Flag hours above the overtime cap (`HRM.RUL.13`) | high | s | Build 1 | Base: wl-attendance Attendance record lifecycle; Base: wl-kernel business calendar |
| BL-ATT-03 | Flag marked hours that cross the midday break (`HRM.RUL.13`) | high | s | Build 1 | Base: wl-attendance Attendance record lifecycle; BL-ATT-02 |
| BL-ATT-04 | Flag attendance without a valid permit, visa or qualification (`HRM.RUL.14`) | high | s | Build 1 | Epic wl-documents: `document_valid`; BL-HRM-01 |
| BL-ATT-05 | Accept held flags only by a separate reviewer (`HRM.ACL.09`) | high | s | Build 1 | BL-ATT-01; BL-ATT-02; BL-ATT-04 |
| BL-ATT-06 | Refuse timesheet submit when allocated hours exceed attended hours | high | s | Build 1 | Base: wl-attendance Timesheet lifecycle; BL-ATT-01 |
| BL-ATT-07 | Open an adjustment timesheet on a locked timesheet (`HRM.ACL.08`) | high | m | Build 1 | Base: wl-attendance Timesheet lifecycle |
| BL-ATT-08 | Approve an adjustment timesheet with segregation of duties | high | s | Build 1 | BL-ATT-07 |
| BL-ATT-09 | Link a credited invoice dispute to a `client_dispute` adjustment | high | s | Build 1 | BL-ATT-07; Epic wl-invoicing: invoice dispute and `resolve_dispute` |
| BL-ATT-10 | Escalate an unanswered external timesheet (`HRM.RUL.15` at E days) | medium | s | Build 1 | Base: wl-attendance `HRM.RUL.08` |
| BL-ATT-11 | Deem an external timesheet approved per contract (`deem_approve`) | high | s | Build 1 | BL-ATT-10; BL-ATT-13 |
| BL-ATT-12 | Give payroll provisional hours at the cut-off (`hours_for_payroll`) | high | m | Build 1 | Base: wl-attendance Timesheet lifecycle; Base: wl-hr `PayrollCalendar` |
| BL-ATT-13 | Reconcile provisional hours when the timesheet is approved | high | s | Build 1 | BL-ATT-07; BL-ATT-12 |

---

### BL-ATT-01 — wl-attendance: detect overlapping attendance across locations and projects

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-attendance Attendance record lifecycle (`record`, `flag`, `accept`) · **Blocks:** BL-ATT-05, BL-ATT-06, BL-CON-18
**Review items:** X-16, 09 MEDIUM "overlap guards"

#### Goal
One employee cannot be present at two places at the same time. Self check-in at a second
location is refused until the employee checks out of the first. Supervisor marking or an import
that overlaps another record of the same employee (any location or project) still saves, but
flags both records `overlap` so a reviewer sorts it out. The supervisor sees the clash before
submitting. This stops the same hours being paid or billed twice.

#### Files to read
- `docs/modules/wl-attendance.md` §3 (Attendance record, Attendance flag), §4 *Attendance record*
  (states, transitions), §5.1 step 4, §5.5 point 4
- `docs/WORKFLOW.md` §3.2 (state machine engine guarantees)

#### Terms
- **Attendance record:** presence of one employee on one day at one work location, with
  check-in and check-out times.
- **Open check-in:** a record with a check-in time and no check-out time.
- **Overlap:** two records of the same employee whose time spans (check-in to check-out) share
  at least one minute. A record with no check-out counts as running until now (for self
  check-in) or until the end of the day (for supervisor marking without an out time).
- **Method:** `supervisor` (marked by a supervisor), `self` (PWA check-in), `device`, `import`.
- **Late-entry window (`HRM.ACL.01`):** a supervisor may mark today or up to 1 day back
  (default, setting "Late-entry and correction window for supervisors"); older dates are
  refused at `record`.
- **Flag `overlap`:** one of the three flags that hold a record for review (`HRM.ACL.09`).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `recorded` | neutral | no | Checked in; check-out may be missing |
| `flagged` | warning | no | Needs review (see flags) |
| `accepted` | success | no | Counts towards hours |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `record` | — | recorded | `attendance.mark` (supervisor), `attendance.self`, or system (import) | employee active; within the late-entry window (`HRM.ACL.01`); self check-in: the employee has no open check-in at another location (they must check out first) and none at the same location (a duplicate, refused with "already checked in here") | supervisor marking or import that overlaps another record of the same employee (any location or project; `rejected` records do not count) flags both `overlap`, or only the new one when the other is `locked`; `HRM.RUL.13` and `HRM.RUL.14` run; event `AttendanceRecorded` |
| `flag` | recorded, accepted | flagged | system | a flag rule matched (outside geofence, midday break, missing check-out, overtime cap, permit invalid, overlap) | reviewer notified |

§5.1 step 4 (excerpt): "A worker already marked at another site or project for the same hours
is shown to the supervisor before submission."

§5.5 point 4: "One employee cannot be present at two places at once. Self check-in elsewhere is
refused until the employee checks out. Supervisor marking or import that overlaps another record
flags both `overlap`."

Event `AttendanceFlagged`: record, flag type.

`HRM.ACL.01` (§6): "A supervisor marks attendance only for crews or people assigned to their
location, and only for today or within the late-entry window" — default 1 day back.

#### Edge cases decided
- **Self check-in at the same location twice:** the guard is about *another* location. A second
  check-in at the same location while one is open is a duplicate: `record` refuses
  it with "you are already checked in here" and do not create a record.
- **The other record is `rejected`:** rejected records do not count. Only `recorded`, `flagged`,
  `accepted` and `locked` records are compared.
- **The other record is `locked`:** it cannot move to `flagged` (`locked` is terminal). Only the
  new record is flagged `overlap`; the note names the locked record.
- **Two supervisors submit at the same moment for the same worker:** the overlap check and the
  insert run in one transaction per employee and day, so the second insert sees the first and
  both are flagged.
- **Touching spans** (one ends 12:00, the other starts 12:00) are not an overlap.
- **Time zone:** all times are Asia/Dubai (no daylight saving). A day is 00:00–23:59 local.
- **Late entry (`late_entry`) records** are compared the same way as same-day records.
- **Correction (`correct`) that creates or removes an overlap** (spec §4 `correct` row): the
  check re-runs on `correct`. A correction that creates an overlap flags both records
  `overlap`. A record already flagged `overlap` is cleared only by a reviewer under
  `HRM.ACL.09` (BL-ATT-05), by `accept` or `correct` once the hours no longer overlap; the
  other record keeps its flag until a reviewer accepts it too.

#### Must not
- Must not refuse supervisor marking or an import because of an overlap: it flags, it does not
  block (the worker may really have been there).
- Must not let self check-in through while an open check-in exists at another location.
- Must not write `status` directly; use the `flag` transition so history and events are kept.
- Must not flag a `rejected` record.

#### Done when
- [ ] Given employee A checked in at Site 1 at 07:00 with no check-out, when A tries to self
      check in at Site 2 at 09:00, then the check-in is refused with a message naming Site 1, and
      no record is created.
- [ ] Given A checked out of Site 1 at 08:30, when A self checks in at Site 2 at 09:00, then the
      record is `recorded` with no `overlap` flag.
- [ ] Given A has an `accepted` record at Site 1 for 07:00–15:00 on 2026-11-02, when supervisor B
      marks A present at Site 2 for 13:00–17:00 the same day, then both records are `flagged`
      with `overlap`, two `AttendanceFlagged` events are emitted and the reviewer is notified.
- [ ] Given the same data, when supervisor B opens the crew list for Site 2 before submitting,
      then A is shown as already marked at Site 1 for 07:00–15:00.
- [ ] Given a record 07:00–12:00 and a new one 12:00–16:00, no `overlap` flag is set.
- [ ] Given the other record is `locked`, then only the new record is flagged and the locked one
      is unchanged.
- [ ] Given A has no record on 2026-11-02, when supervisors B (Site 1, 07:00–15:00) and C
      (Site 2, 13:00–17:00) submit their marking for A in two parallel requests started at the
      same moment, then both records exist and both are `flagged` `overlap` (run 20 times;
      never one unflagged record).
- [ ] After an `overlap` flag is set, each flagged record's history shows a `flag` transition
      by system with flag type `overlap`, and one `AttendanceFlagged` event per record exists
      (proves the state was moved by the transition, not written directly).
- [ ] On 2026-11-04, when supervisor B marks A for 2026-11-03, the record is saved; for
      2026-11-02, it is refused (outside the 1-day late-entry window).

#### Out of scope
Accepting an `overlap` record (BL-ATT-05); the timesheet `submit` check on summed hours
(BL-ATT-06); the deployment overlap guard `MPW.ACL.15` (vertical 09).

---

### BL-ATT-02 — wl-attendance: flag hours above the overtime cap (`HRM.RUL.13`)

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-attendance Attendance record lifecycle · Base: wl-kernel business calendar (normal hours, Ramadan hours) · **Blocks:** BL-ATT-03, BL-ATT-05, BL-CON-18, BL-MPW-21
**Review items:** X-16, 02-N8-a, 09 MEDIUM "overtime cap flags"

#### Goal
The UAE labour law allows at most 2 overtime hours a day and 144 overtime hours in any three
weeks. When an employee's hours across all locations and timesheets go above that, the record is
flagged `overtime_cap` for review. The flag does not block: the law has exceptions, and the
employer records which one applies in the reviewer's reason.

#### Files to read
- `docs/modules/wl-attendance.md` §5.5 point 1, §6 *Automations* (`HRM.RUL.13`), §7 (overtime
  cap, work calendar, reduced hours), §10 *Overtime cap*
- `docs/OPEN-ITEMS.md` L-27

#### Terms
- **Normal hours:** the daily normal hours from the business calendar (default 8 h; in Ramadan
  2 h less, so 6 h).
- **Overtime (per day):** every hour that day above the day's normal hours, as split by
  `OvertimePolicy` (spec §8 default: "over the daily normal hours = overtime"). It is not
  only the hours above the 2 h cap: a 10 h day with 8 h normal hours has 2 h of overtime.
- **Overtime cap:** 2 h a day above normal hours, and 144 h of overtime in any rolling 21-day
  window (three weeks), summing the per-day overtime above.
- **Hours that day:** the sum of worked hours of the employee on that date across every
  location, attendance record (not `rejected`) and timesheet entry.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.13` | attendance recorded or corrected; timesheet entry saved | the employee's hours that day (all locations and timesheets) exceed normal hours + the daily overtime cap, or overtime exceeds the 3-week cap; or, at an outdoor location in the season, the worked span minus breaks crosses the midday break window | flag `overtime_cap` or `midday_break`; notify the supervisor (and the HSE officer for the midday break) | cap 2 h/day, 144 h per 3 weeks ✔ [S1]; window as `HRM.RUL.01` |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `flag` | recorded, accepted | flagged | system | a flag rule matched (outside geofence, midday break, missing check-out, overtime cap, permit invalid, overlap) | reviewer notified |

§5.5 point 1: "hours above the daily normal hours plus the overtime cap (default 2 h a day; in
Ramadan the normal hours are 2 h shorter) and overtime above 144 hours in any three weeks ✔ [S1]
flag `overtime_cap`. The flag does not block; a reviewer accepts with a reason (`HRM.ACL.09`).
Whether any of the law's exceptions to the cap applies is the employer's call, recorded in the
reason."

Setting: Overtime cap checked by `HRM.RUL.13` = 2 h a day, 144 h per 3 weeks.
§10: which exceptions apply and the penalty are `⚠ open: lawyer (L-27)`.

#### Edge cases decided
- **Which record is flagged:** the record (or records) of that day whose save made the total
  exceed the cap. Flag every non-`locked`, non-`rejected` record of the
  employee on that day, so the reviewer sees the whole day.
- **Records already `locked`:** cannot be flagged; only the others are. If all are locked (the
  check fires from a later timesheet entry), notify the supervisor without a flag.
- **Rest days and public holidays:** normal hours on those days are 0, so any hour is overtime;
  the daily cap is then 2 h. Use the business calendar's normal hours for
  that date (0 on a rest day or holiday).
- **3-week window:** rolling 21 days ending on the attendance date, Asia/Dubai dates.
- **Repeated runs:** saving the same record again does not add a second `overtime_cap` flag; a
  flag type is held at most once per record.
- **Exceptions to the cap:** not modelled; the reviewer writes the exception in the reason
  (L-27 open). Do not add exception logic.

#### Must not
- Must not refuse the record or trim hours above the cap.
- Must not count `rejected` records.
- Must not hard-code 8 h or 2 h: read the calendar and the setting.

#### Done when
- [ ] Given normal hours 8 h on Tuesday 2026-11-03, when employee A has 07:00–13:00 at Site 1 and
      13:30–18:00 at Site 2 (10.5 h), then both records are flagged `overtime_cap` and the
      supervisor is notified.
- [ ] Given the same day with 10 h in total, no flag is set.
- [ ] Given a Ramadan day (normal hours 6 h), when A works 8.5 h, then the records are flagged.
- [ ] Given A has 143 h of overtime (hours above normal hours) in the 20 days 2026-11-01 to
      2026-11-20, when A works 10 h (2 h overtime, within the daily cap) on 2026-11-21, then
      the record is flagged `overtime_cap` (145 h in the 21-day window); with 142 h before,
      no flag is set (144 h is not above the cap).
- [ ] Given a timesheet entry is saved that raises the day to 11 h, then the day's open records
      are flagged.
- [ ] Saving a flagged record again does not create a second `overtime_cap` flag.

#### Out of scope
Midday break part of `HRM.RUL.13` (BL-ATT-03); accepting the flag (BL-ATT-05); overtime pay
rates (wl-hr payroll).

---

### BL-ATT-03 — wl-attendance: flag marked hours that cross the midday break (`HRM.RUL.13`)

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-attendance Attendance record lifecycle · BL-ATT-02 (same rule) · **Blocks:** BL-CON-18
**Review items:** X-16, 02-N8-a

#### Goal
Outdoor work is prohibited 12:30–15:00 from 15 June to 15 September. `HRM.RUL.01` already flags
a check-in inside the window. Crews marked as a block by a supervisor (07:00–17:00) slip past
it, so `HRM.RUL.13` also checks the worked span minus breaks. The crew marking page pre-fills the
break with the window in the season; removing it, or times that cross the window, flag the
record `midday_break` and notify the supervisor and the HSE officer.

#### Files to read
- `docs/modules/wl-attendance.md` §3 (Attendance record: breaks taken; Work location: outdoor
  flag; Work calendar: midday break window), §5.1 step 2, §5.5 point 2, §6 *Automations*
  (`HRM.RUL.01`, `HRM.RUL.13`), §7 (midday break window), §10 *Midday break*

#### Terms
- **Outdoor location:** a work location with the outdoor flag set.
- **Season / window:** the midday break dates and times per scope (default 15 Jun–15 Sep,
  12:30–15:00), set yearly from the MoHRE announcement.
- **Worked span minus breaks:** the time from check-in to check-out, with the "breaks taken"
  intervals removed.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.13` | attendance recorded or corrected; timesheet entry saved | the employee's hours that day (all locations and timesheets) exceed normal hours + the daily overtime cap, or overtime exceeds the 3-week cap; or, at an outdoor location in the season, the worked span minus breaks crosses the midday break window | flag `overtime_cap` or `midday_break`; notify the supervisor (and the HSE officer for the midday break) | cap 2 h/day, 144 h per 3 weeks ✔ [S1]; window as `HRM.RUL.01` |
| `HRM.RUL.01` | attendance recorded | location is outdoor; time inside the midday break window and period | flag `midday_break`; notify the HSE officer or supervisor | 12:30–15:00, 15 Jun–15 Sep ✔ [S3], [S4]; MoHRE announces it each year |

§5.1 step 2: "Inside the midday break season, the break field of outdoor locations is pre-filled
with the break window; removing it, or times that cross the window, flag the record
`midday_break` (`HRM.RUL.13`)."

§5.5 point 2: "`HRM.RUL.13` also checks the worked span (in to out, minus the breaks taken) of
supervisor marking and manual timesheet entries at outdoor locations, so crews marked as a block
are caught too."

§10: "`HRM.RUL.01` flags; it does not block, because some work is exempt."

#### Edge cases decided
- **Partial break:** a break 12:30–14:00 leaves 14:00–15:00 worked inside the window: flagged.
- **Season boundaries:** inclusive, Asia/Dubai dates: 15 June and 15 September are in the season.
- **Manual timesheet entries** carry hours, not times. An entry at an
  outdoor location in the season is checked only when it has from–to times; an entry with hours
  only is not checked (no span to compare).
- **Double flag:** a record already flagged `midday_break` by `HRM.RUL.01` is not flagged again.
- **`midday_break` does not hold the record for `HRM.ACL.09`:** it is not one of the three held
  flags; any `attendance.review` user may accept it.

#### Must not
- Must not block the save (exempt work exists).
- Must not pre-fill the break outside the season or at indoor locations.
- Must not hard-code the window; read it per scope.

#### Done when
- [ ] Given outdoor Site 1 and date 2026-07-10, when supervisor B opens crew marking, then the
      break field is pre-filled 12:30–15:00.
- [ ] When B marks the crew 07:00–17:00 and keeps the break, then no `midday_break` flag is set.
- [ ] When B removes the break, then every record is flagged `midday_break` and the supervisor
      and HSE officer are notified.
- [ ] Given break 12:30–14:00, the record is flagged.
- [ ] Given date 2026-10-01 (out of season), no pre-fill and no flag.
- [ ] Given an indoor location, no pre-fill and no flag.

#### Out of scope
Overtime-cap part of the rule (BL-ATT-02); the check-in-time check `HRM.RUL.01` (base).

---

### BL-ATT-04 — wl-attendance: flag attendance without a valid permit, visa or qualification (`HRM.RUL.14`)

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Epic wl-documents: `document_valid(subject, kind, on_date)` · BL-HRM-01 (`qualification_valid`) · **Blocks:** BL-ATT-05, BL-CON-18, BL-MPW-21
**Review items:** X-16, X-13, 02-N5-d

#### Goal
When a worker is marked present on a day their work permit or residence visa is not valid, or a
qualification the context needs (e.g. a SIRA licence) is not valid, the record is flagged
`permit_invalid` the same day. HR and the supervisor are told. While flagged, the record proposes
no hours and holds the timesheet's `submit`. HR's review is the reviewer's `accept`
(`HRM.ACL.09`, BL-ATT-05); the hours are then proposed non-billable and count for payroll.

#### Files to read
- `docs/modules/wl-attendance.md` §5.5 point 3, §6 *Automations* (`HRM.RUL.14`), §6 *Consumes*
  (wl-documents, wl-hr), §7 (work permit and visa check), §10 *Work without a valid permit*
- `docs/modules/wl-documents.md` §5.6 (the standard guard; a past date is checked against the
  document valid on that date)
- `docs/OPEN-ITEMS.md` L-28

#### Terms
- **`document_valid(subject, kind, on_date)`:** wl-documents guard; *yes* when the subject has an
  `accepted` document of that kind valid on `on_date`.
- **`qualification_valid(employee, kind, on_date[, role, scope])`:** wl-hr guard
  (`HRM.QUA.01`) that adds role and scope to the same check; for role-bound kinds the caller
  passes the context's role and scope (without them the answer is *no*).
- **Context:** the `Subject` the attendance is for (deployment, project). It may require
  qualification kinds.
- **Non-billable:** the proposed timesheet entry has billable = no.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.14` | attendance recorded | `document_valid(employee, work_permit, date)` or `document_valid(employee, residence_visa, date)` fails at the attendance date, or `qualification_valid(employee, kind, date)` fails for a qualification the context requires | flag `permit_invalid`; notify HR and the supervisor; on `accept` the hours are proposed non-billable | kinds checked: work permit, residence visa (per client) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `accept` | recorded, flagged | accepted | `attendance.review`, or system at check-out when no flag | flags `overtime_cap`, `permit_invalid` and `overlap` need a reviewer's reason and are not accepted by the person who marked the record (`HRM.ACL.09`); an `overlap` is accepted only once the hours no longer overlap | hours proposed on the open timesheet; hours from a `permit_invalid` record are proposed non-billable |

§5.5 point 3 (excerpt): "While the record is `flagged`, no hours are proposed on the timesheet
and the timesheet cannot be submitted (`submit` guard). HR's review is the `accept` by a
reviewer (`HRM.ACL.09`); its hours are then proposed non-billable, and they count for payroll."

Setting: Work permit and visa check (`HRM.RUL.14`) = on; kinds: work permit, residence visa;
plus qualifications required by the context.
§10: "The employer's exposure, and whether the hours must still be paid in full (the default
here: they are paid, not billed until HR reviews), are `⚠ open: lawyer (L-28)`."

#### Edge cases decided
- **`on_date`:** always the attendance date, also for a late entry made the next day (the guard
  answers for a past date with the document valid on that date).
- **Renewal pending review:** `document_valid` answers *no*; the record is flagged.
- **Document renewed later the same day:** the flag stays; HR clears it by review (BL-ATT-05).
- **Freelancers** (employment type `freelance`): they have no work permit kind on the company;
  Check only the kinds configured for the client and the context's
  required qualifications; a freelancer's permit is checked only if the client adds its kind.
- **Setting off:** no check runs.
- **Payroll:** the hours count for payroll (L-28 default) once accepted; a record still
  `flagged` at the payroll cut-off is left out and reaches payroll through reconciliation
  (BL-ATT-12, BL-ATT-13).
- **Between flag and review:** the record proposes no timesheet hours (only `accept` does), and
  the timesheet `submit` is refused while it is `flagged` (BL-ATT-06 base guard).
- **Billable later:** the non-billable value is a proposal; the timesheet submitter may set the
  entry billable before `submit`, and the change is kept in the entry's history.

#### Must not
- Must not refuse the attendance (the worker did turn up; `flag` mode, wl-documents §5.6).
- Must not drop the hours from payroll.
- Must not propose `permit_invalid` hours as billable.

#### Done when
- [ ] Given employee A's work permit expired on 2026-11-01, when A is marked present on
      2026-11-02, then the record is flagged `permit_invalid` and HR and the supervisor are
      notified.
- [ ] Given a context requiring a SIRA licence and A's licence expired, the record is flagged.
- [ ] Given a late entry on 2026-11-03 for 2026-11-02 and the permit was valid on 2026-11-02
      but expired on 2026-11-03, no flag is set.
- [ ] While the record is `flagged`, the open timesheet has no entry from it and `submit` is
      refused naming the flagged record.
- [ ] When the record is later accepted, the proposed timesheet entry is non-billable and its
      hours count for payroll.
- [ ] With the setting off, no flag is set.

#### Out of scope
Accepting the flag (BL-ATT-05); deployment `suspend` on `DocumentExpired` (vertical 09).

---

### BL-ATT-05 — wl-attendance: accept held flags only by a separate reviewer (`HRM.ACL.09`)

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-ATT-01 · BL-ATT-02 · BL-ATT-04 · **Blocks:** BL-CON-18
**Review items:** X-16

#### Goal
A record flagged `overtime_cap`, `permit_invalid` or `overlap` is accepted only by a user with
`attendance.review`, with a reason, and never by the supervisor who marked it. An `overlap` can
only be accepted once the hours no longer overlap (one record corrected or rejected). This keeps
a supervisor from waving through their own questionable marking.

#### Files to read
- `docs/modules/wl-attendance.md` §3 (Attendance flag), §4 *Attendance record*, §6 *Access
  rules* (`HRM.ACL.09`), §6 *Permissions*
- `docs/ACCESS-CONTROL.md` (rule kinds `STA`, `SOD`)

#### Terms
- **Held flags:** `overtime_cap`, `permit_invalid`, `overlap`. Other flags (`outside_geofence`,
  `midday_break`, `missing_checkout`, `late`, `late_entry`, `manual_correction`, `duplicate`) do
  not need a separate reviewer.
- **Marked by:** the `recorded by` user of the record.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `accept` | recorded, flagged | accepted | `attendance.review`, or system at check-out when no flag | flags `overtime_cap`, `permit_invalid` and `overlap` need a reviewer's reason and are not accepted by the person who marked the record (`HRM.ACL.09`); an `overlap` is accepted only once the hours no longer overlap | hours proposed on the open timesheet; hours from a `permit_invalid` record are proposed non-billable |
| `correct` | recorded, flagged, accepted | accepted | `attendance.correct` | reason given; within the correction window; on a record with a held flag (`overtime_cap`, `permit_invalid`, `overlap`) the `accept` conditions of `HRM.ACL.09` apply: a reviewer's reason, not the person who marked it, and an `overlap` only if the corrected hours no longer overlap | original values kept in history; `manual_correction` flag noted; the overlap check, `HRM.RUL.13` and `HRM.RUL.14` run again on the corrected values, so a correction that creates an overlap flags both records `overlap` |
| `reject` | recorded, flagged | rejected | `attendance.review` | reason given | proposed hours removed |

| ID | Rule | Generalises | Pattern | Default |
|---|---|---|---|---|
| `HRM.ACL.09` | An attendance record flagged `overtime_cap`, `permit_invalid` or `overlap` is accepted only by `attendance.review` with a reason, and not by the supervisor who marked it | new (X-16) | `STA`, `SOD` | — |

#### Edge cases decided
- **`correct` on a held record** (spec §4 `correct` guard): `correct` moves to `accepted`
  directly, so it carries the same `HRM.ACL.09` conditions as `accept`: a reviewer's reason,
  not the marker, and for `overlap` the corrected hours must no longer overlap; otherwise it is
  refused with the other record's location and times. `correct` cannot bypass the rule.
- **Self check-in records:** "the person who marked" is the employee; an employee never accepts
  their own record.
- **Several flags on one record:** one reason covers all; the reviewer sees every flag.
- **Overlap still present:** accept is refused with the other record's location and times.
- **Admin users:** the SoD applies to admins too.

#### Must not
- Must not let the system auto-accept a record with a held flag at check-out.
- Must not let the marking supervisor accept or `correct` past a held flag.
- Must not accept a held flag without a reason.

#### Done when
- [ ] Given a record flagged `overtime_cap` marked by supervisor B, when B tries `accept`, then it
      is refused.
- [ ] When reviewer C accepts with reason "emergency repair, Art. exception", then it is
      `accepted` and the reason is in history.
- [ ] When C accepts without a reason, it is refused.
- [ ] Given two records flagged `overlap`, when C tries to accept one while both still overlap,
      it is refused; after C rejects the other, accept succeeds.
- [ ] Given a record flagged only `midday_break` marked by B, then B (with `attendance.review`)
      may accept it.
- [ ] Given a held flag, when B tries `correct`, it is refused.
- [ ] Given records R1 (Site 1, 07:00–15:00) and R2 (Site 2, 13:00–17:00) flagged `overlap`,
      when reviewer C corrects R2 to 13:00–17:00 (unchanged overlap) with a reason, it is
      refused; when C corrects R2 to 15:00–17:00 with a reason, R2 is `accepted` and R1 is
      still `flagged` until C accepts it.

#### Out of scope
How the flags are raised (BL-ATT-01, BL-ATT-02, BL-ATT-04).

---

### BL-ATT-06 — wl-attendance: refuse timesheet submit when allocated hours exceed attended hours

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-attendance Timesheet lifecycle (`submit`) · BL-ATT-01 · **Blocks:** BL-CON-18
**Review items:** X-16, 09 MEDIUM "overlap guards"

#### Goal
The same hours must not be allocated twice across projects or timesheets. On `submit`, entries
whose hours for an employee and day, summed across all timesheets and projects, exceed the
attended hours are refused.

#### Files to read
- `docs/modules/wl-attendance.md` §4 *Timesheet* (`submit`), §5.5 point 4

#### Terms
- **Attended hours:** the hours of the employee's `accepted` or `locked` attendance records on
  that day.
- **Allocated hours:** the hours in timesheet entries for the employee and day, across every
  timesheet that is not an adjustment.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | open | submitted | `timesheets.submit`, or system (period end) | every accepted attendance day has hours allocated to a cost object; no attendance left `flagged` | approver notified (staff, or external party through the portal or an emailed approval link) |

§5.5 point 4: "Timesheet `submit` refuses entries whose hours for an employee and day, summed
across all timesheets and projects, exceed the attended hours."

#### Edge cases decided
- **Timesheets without attendance** (04 manual time, 10 sessions taught): there are no attended
  hours. The check applies only to contexts whose source of hours is
  attendance (02, 09; §4 vertical mapping); manual and generated sources are not checked.
- **Adjustment timesheets** hold signed deltas; they are excluded from the
  sum (their correction is approved explicitly).
- **System submit at period end** (`HRM.RUL.08`): if the guard fails, the timesheet stays `open`
  and the submitter gets a task listing the days.
- **Rounding:** compare in minutes.
- **Concurrent submits** of two timesheets for the same employee: lock per employee and day so
  the second one sees the first one's entries.

#### Must not
- Must not trim entries automatically.
- Must not skip the check for system submits.

#### Done when
- [ ] Given employee A attended 8 h on 2026-11-02 and timesheet T1 (Project X) has 5 h for that
      day, when timesheet T2 (Project Y) with 4 h is submitted, then submit is refused naming
      2026-11-02 and the 1 h excess.
- [ ] With T2 at 3 h, submit succeeds.
- [ ] Given a 04 engagement timesheet with manual entries, no attendance check runs.
- [ ] When the period-end system submit fails the check, the timesheet stays `open` and a task is
      created.
- [ ] Given A attended 8 h on 2026-11-02, T1 with 5 h and T2 with 4 h for that day, when T1 and
      T2 are submitted in two parallel requests started at the same moment, then exactly one
      is `submitted` and the other is refused naming the 1 h excess (run 20 times).

#### Out of scope
Attendance overlap flags (BL-ATT-01).

---

### BL-ATT-07 — wl-attendance: open an adjustment timesheet on a locked timesheet (`HRM.ACL.08`)

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-attendance Timesheet lifecycle (`lock`) · **Blocks:** BL-ATT-08, BL-ATT-09, BL-ATT-13, BL-CON-18, BL-EDU-14
**Review items:** X-16, 09-N4-a

#### Goal
A `locked` timesheet (used by payroll, an invoice or an export) is never edited or reopened. A
correction is a new timesheet of kind `adjustment` that holds only the signed difference (+ or −
hours per day, cost object and category), with a reason, an origin and where it flows (`affects`).
The original stays as it is and points to its adjustments.

#### Files to read
- `docs/modules/wl-attendance.md` §3 (Timesheet, Adjustment timesheet, Timesheet entry), §4
  *Timesheet* (`open_adjustment`, `locked`), the paragraph "Correcting a locked timesheet", §5.8,
  §6 *Permissions* (`timesheets.adjust`), §6 *Access rules* (`HRM.ACL.08`)

#### Terms
- **`locked`:** terminal timesheet state after the first consumer (payroll run, usage invoice,
  WIP billing, export) used it.
- **Origin:** `client_dispute`, `attendance_error`, `payroll_error`, `provisional_reconciliation`.
- **Affects:** `billing`, `payroll` or `both`: which consumers act on the adjustment.
- **Signed hours:** entry hours may be negative (hours removed) or positive (hours added).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `locked` | success | yes | Consumed by a payroll run, an invoice or an export; never changes again. Corrected only by an adjustment timesheet (`HRM.ACL.08`) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `open_adjustment` | — | open | `timesheets.adjust`, or system (`provisional_reconciliation`, §5.7) | the original is `locked` and of kind `regular` (an adjustment always points to the original, never to another adjustment); reason and origin given; for `client_dispute`, the linked ticket or invoice dispute, still open | adjustment timesheet created with kind `adjustment`, linked to the original (the link is added to `adjusted_by` on the original); the original's entries, totals, state and consumers are not changed |

| ID | Rule | Generalises | Pattern | Default |
|---|---|---|---|---|
| `HRM.ACL.08` | A `locked` timesheet is never edited or reopened; it is corrected only by an adjustment timesheet with a reason, approved by the original's approver kind, and not by the user who created it | new (X-16) | `STA`, `SOD` | — |

Adjustment timesheet fields: adjusts (original timesheet), reason, origin (`client_dispute`,
`attendance_error`, `payroll_error`, `provisional_reconciliation`), linked ticket or invoice
dispute, affects (`billing`, `payroll` or `both`), entries with signed hours (+ or −) per day,
cost object and category.

"It runs through the same machine and is approved by the same approver kind as the original"
(owner, context, approver kind and approver copied from the original).

| Affects | Payroll (`wl-hr`) | Billing (`wl-invoicing`) |
|---|---|---|
| `billing` | nothing | original not yet invoiced: billed with it. Already invoiced: − hours → credit note against the original invoice (`FIN.ACL.01`); + hours → an extra line on the next usage invoice (or a separate invoice, per client setting) |
| `payroll` | + hours paid in the next run; − hours recovered as a deduction within the Art. 25 limits ([wl-hr](wl-hr.md)); after `paid`, through a supplementary run | nothing |
| `both` | as `payroll` | as `billing` |

#### Edge cases decided
- **Several adjustments on one original:** allowed; each is linked; `adjusted_by` is a list.
- **`adjusted_by` is a link, not an edit:** the spec (§3 Timesheet, "adjusted by") makes it a
  list of links; adding one is the only write on the original and is not an edit under
  `HRM.ACL.08`.
- **Adjustment of an adjustment:** an adjustment always points to the
  original regular timesheet, never to another adjustment.
- **Net result below zero:** the signed delta for a day and category may not take the original
  plus all approved adjustments below 0 h. Refused on save.
- **Original not `locked`** (`approved` and unconsumed): refused; use `reopen` instead.
- **Concurrency:** two users opening adjustments at once is fine (separate records).
- **Default `affects` for `client_dispute`:** `billing` (BL-ATT-09).

#### Must not
- Must not change any entry, total, state or consumer of the original; the only write is the new
  link in its `adjusted_by` list.
- Must not allow `reopen` on a `locked` timesheet.
- Must not create an adjustment without reason and origin.

#### Done when
- [ ] Given timesheet T `locked` by payroll run R, with 8 h normal on 2026-11-02 on cost object
      "Project X / cost code 03-100", when user A with `timesheets.adjust` opens an adjustment
      with origin `attendance_error`, affects `both`, reason "marked present, was absent" and
      one entry of −8 h, category normal, cost object "Project X / cost code 03-100" on
      2026-11-02, then an `open` timesheet of kind `adjustment` exists linked to T, and T's
      `adjusted_by` lists it.
- [ ] T's entries and status are unchanged.
- [ ] `reopen` on T is refused.
- [ ] Given T is `approved` (not locked), `open_adjustment` is refused.
- [ ] Without a reason, it is refused; with origin `client_dispute` and no linked ticket or
      dispute, it is refused.
- [ ] A −10 h entry on a day where T has 8 h is refused.

#### Out of scope
Approval of the adjustment (BL-ATT-08); dispute link (BL-ATT-09); system-created reconciliation
adjustments (BL-ATT-13); payroll reaction (BL-HRM-16); billing reaction (Epic wl-invoicing:
credit note or extra line from an adjustment timesheet).

---

### BL-ATT-08 — wl-attendance: approve an adjustment timesheet with segregation of duties

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-ATT-07 · **Blocks:** BL-HRM-16, BL-CON-18, BL-EDU-14
**Review items:** X-16, 09-N4-a

#### Goal
An adjustment is approved by the same approver kind as the original (when the client approved
the original, the client approves the adjustment, or staff with `approve_on_behalf` and the
signed agreed outcome), and never by the user who created it. On approval the signed delta is
posted as labour cost where the context posts cost from hours, and `TimesheetApproved` (kind
`adjustment`) tells payroll and billing.

#### Files to read
- `docs/modules/wl-attendance.md` §4 *Timesheet* (`approve`, `approve_on_behalf`), "Correcting
  a locked timesheet", *Labour cost*, §5.8, §6 *Events emitted*

#### Terms
- **Approver kind:** `internal` (staff) or `external` (the client's approver via portal or
  signed emailed link).
- **Labour cost posting:** `FIN.CST.01` via `CostPoster`; only for contexts that post cost from
  hours (02 on; 09, 10 off; 04 optional).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `approve` | submitted | approved | `timesheets.approve` (internal), or an external approver role on the context's party (portal or signed emailed link), or system for a `provisional_reconciliation` adjustment (§5.7 step 3) | approver kind matches the timesheet; adjustment: approver kind as the original's and the creator is not the approver (`HRM.ACL.08`) | attendance `lock`; labour cost posted to cost objects when the context posts cost from hours (`FIN.CST.01`, *Labour cost* below); adjustment: the signed delta is posted the same way; if payroll already used provisional hours (§5.7), the timesheet is `lock`ed at once (payroll is its first consumer) and the difference is sent to payroll (`TimesheetReconciliationNeeded`); event `TimesheetApproved` (not for a `provisional_reconciliation` adjustment, whose only event is `TimesheetReconciliationNeeded`) |
| `approve_on_behalf` | submitted | approved | `timesheets.approve_on_behalf` | approver kind is `external`; approval evidence attached (signed sheet) | as `approve`; evidence stored with the timesheet |
| `reject` | submitted | open | approver | reason given | submitter notified |

| Event | When | Payload |
|---|---|---|
| `TimesheetApproved` | `approve` / `approve_on_behalf` / `deem_approve` | timesheet, owner, period, context, totals per category, approver kind, approval basis; for an adjustment: kind, adjusts, origin, affects, signed totals |

#### Edge cases decided
- **Creator with `approve_on_behalf`:** still refused (creator ≠ approver applies to every
  approving transition).
- **System-created adjustments** (`provisional_reconciliation`): not approved by a user. Per
  spec §5.7 step 3 and §5.8 step 2, the system submits and approves them in the same step as
  the original's approval, copying its approver and approval basis; they emit no
  `TimesheetApproved` of their own (BL-ATT-13). This issue covers user-created adjustments.
- **`deem_approve` on an adjustment:** allowed when the original's contract has the clause
  (same machine); approval basis `deemed`.
- **Attendance lock:** adjustments have no attendance of their own; nothing to lock.
- **Negative cost delta:** posted as a negative cost on the same cost objects and categories.

#### Must not
- Must not let the creator approve, even an admin.
- Must not let an internal approver approve an adjustment of an externally approved timesheet
  except through `approve_on_behalf` with evidence.

#### Done when
- [ ] Given adjustment J (created by A) of an internally approved 02 timesheet with −8 h at a
      trade rate of AED 25/h, when A approves, it is refused; when B approves, J is `approved`,
      a cost of −AED 200 is posted to the original cost object and `TimesheetApproved` carries
      kind `adjustment`, adjusts, origin, affects and signed totals −8 h.
- [ ] Given J adjusts a 09 timesheet approved by the client, when internal approver B uses
      `approve`, it is refused; `approve_on_behalf` with a signed sheet succeeds.
- [ ] Given a 09 context, no labour cost is posted.
- [ ] `reject` returns J to `open` with the reason.

#### Out of scope
Payroll and billing reactions (BL-HRM-16; Epic wl-invoicing); reconciliation (BL-ATT-13).

---

### BL-ATT-09 — wl-attendance: link a credited invoice dispute to a `client_dispute` adjustment

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-ATT-07 · Epic wl-invoicing: invoice `disputed` flag and `resolve_dispute` outcome `credited` · **Blocks:** None
**Review items:** X-16, 09-N4-a

#### Goal
When a client disputes billed hours, the invoice is flagged `disputed` (wl-invoicing). When the
business and the client agree to cut hours, a user opens an adjustment timesheet with origin
`client_dispute`, linked to the open dispute. Its approval raises the credit note, and the
dispute is then resolved `credited`, naming the adjustment. By default it affects `billing`
only: the worker's pay is not cut because a client disputed an invoice, unless the attendance
itself was wrong.

#### Files to read
- `docs/modules/wl-attendance.md` "Correcting a locked timesheet" (last paragraph), §5.7 point
  4, §5.8, §6 *Consumes* (wl-invoicing)
- `docs/modules/wl-invoicing.md` §4 *Invoice* (`dispute`, `resolve_dispute`), §5.2 point 5,
  §5.10 point 4

#### Terms
- **Credited:** the dispute outcome where a credit note is issued for the disputed part.
- **Attendance evidence:** the attendance records (or a document) showing the worker was not
  present.

#### Spec excerpt (authoritative for this issue)

"A client dispute about billed hours sets `disputed` on the invoice (`wl-invoicing`). When the
business and the client agree to cut hours, a user with `timesheets.adjust` opens an adjustment
timesheet with origin `client_dispute`, linked to the open dispute, while the invoice is still
`disputed`; nothing opens it automatically. Its approval raises the credit note (`wl-invoicing`
§5.2), and the dispute is then resolved `credited`, naming the adjustment; the credit note and
the adjustment link to each other. A dispute resolved `upheld` needs no adjustment. By default it affects `billing` only: the worker's pay is not reduced because
the client disputes an invoice, unless the attendance itself was wrong (then `both`, with the
attendance evidence). Each adjustment records its own consumers; the original keeps its own."

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `open_adjustment` | — | open | `timesheets.adjust`, or system (`provisional_reconciliation`, §5.7) | the original is `locked` and of kind `regular` (an adjustment always points to the original, never to another adjustment); reason and origin given; for `client_dispute`, the linked ticket or invoice dispute, still open | adjustment timesheet created with kind `adjustment`, linked to the original (the link is added to `adjusted_by` on the original); the original's entries, totals, state and consumers are not changed |

§5.7 point 4: "If the client cuts billable hours but the attendance stands, the cut is a billing
matter (`affects` = `billing`); the worker keeps the pay."

wl-invoicing `resolve_dispute` (excerpt): "for hours billed from timesheets, `credited` names
the adjustment timesheet (origin `client_dispute`) that carries the agreed hours". §5.10 point
4: "For disputed hours, the credit note is the one raised from the client-dispute adjustment
timesheet (§5.2)."

#### Edge cases decided
- **Who opens it, and when:** a user with `timesheets.adjust` (usually the account manager who
  owns the dispute ticket), while the invoice is `disputed` and once the cut is agreed. It is
  never created automatically, and no extra task is raised here: the dispute ticket
  (wl-invoicing §5.10) is the work item. The order is: open adjustment → approve (by the
  original's approver kind, BL-ATT-08) → credit note from the approval (wl-invoicing §5.2) →
  `resolve_dispute` `credited` naming the adjustment.
- **`affects` = `both`:** allowed only with attendance evidence attached; otherwise refused.
- **Credit note amount vs hours:** hours in the adjustment are what the client and business
  agreed; the credit note amount is wl-invoicing's. The link is informational both ways.
- **Dispute ends `upheld`:** no adjustment is needed. If one was opened for that dispute and is
  not yet approved, its `submit` is refused ("dispute resolved upheld") and its creator is
  notified; it has no payroll or billing effect.
- **Dispute no longer open:** `open_adjustment` with origin `client_dispute` linked to a dispute
  already resolved is refused; a linked ticket instead of a dispute is allowed (spec guard).

#### Must not
- Must not default `affects` to `payroll` or `both` for `client_dispute`.
- Must not reduce pay without attendance evidence.

#### Done when
- [ ] Given invoice INV-1 built from `locked` timesheet T and flagged `disputed`, when A opens a
      `client_dispute` adjustment J (−8 h) linked to INV-1's dispute, J is `open` and `affects`
      defaults to `billing`; no adjustment or task was created automatically when the dispute
      was raised.
- [ ] When J is approved and the credit note is raised from it and INV-1's dispute is resolved
      `credited` naming J, then J links to the credit note and the credit note links to J.
- [ ] Given INV-1's dispute is resolved `upheld` with no adjustment, no adjustment exists for T
      and none is created; given J was opened and the dispute then ends `upheld`, J's `submit`
      is refused and A is notified.
- [ ] Choosing `both` without attendance evidence is refused; with evidence it is accepted.
- [ ] A `client_dispute` adjustment without a linked dispute or ticket is refused.

#### Out of scope
The invoice dispute and credit note (Epic wl-invoicing); approval (BL-ATT-08).

---

### BL-ATT-10 — wl-attendance: escalate an unanswered external timesheet (`HRM.RUL.15` at E days)

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:medium` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-attendance `HRM.RUL.08` (submit and reminders) · **Blocks:** BL-ATT-11, BL-MPW-19, BL-MPW-21
**Review items:** X-16, 09-N5-b

#### Goal
When a client does not approve a submitted timesheet, the reminder ladder of `HRM.RUL.08`
continues: at E days after submit (default 6) the client's escalation contact and the account
manager's manager are told, and the account manager gets a task to collect a signed sheet so
staff can `approve_on_behalf`.

#### Files to read
- `docs/modules/wl-attendance.md` §5.6, §6 *Automations* (`HRM.RUL.08`, `HRM.RUL.15`), §7
  (approval escalation), §8 (`ApproverResolver`)

#### Terms
- **E:** days after `submit` at which the escalation happens.
- **Escalation contact:** a client contact from the deployment contract, returned by
  `ApproverResolver`.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.08` (from `MPW.RUL.03`, `MPW.RUL.04`) | period end; N days after submit | open timesheet with entries; not approved | submit to the approver; remind the approver, escalate on the second reminder | N = 3 |
| `HRM.RUL.15` | E days after submit; N days after submit | external timesheet still `submitted` | at E: notify the client's escalation contact and the account manager's manager, task to collect a signed sheet; at N, if the contract has a deemed-approval clause: timesheet `deem_approve` and notify the client | E = 6; N = per contract (off when the contract has no clause); calendar days, checked daily at 08:00 Asia/Dubai |

§5.6 step 3 (part): "Without the clause, the timesheet stays `submitted` and the task stays open."

#### Edge cases decided
- **Days:** calendar days in Asia/Dubai from the `submit` date; the rule runs once a day at
  08:00 Asia/Dubai and escalates timesheets whose submit date + E is today or earlier.
- **Account manager's manager:** the account manager's line manager in wl-hr. If none is set,
  the escalation goes to the users with `timesheets.approve_on_behalf` in the context's scope,
  and the task says no manager was found.
- **Re-submit after `reject`:** the clock restarts from the new `submit`.
- **Repeated runs:** the escalation is sent once per submit; the task is created once.
- **Approved in between:** no escalation; an open task is closed automatically.
- **No escalation contact on the contract:** notify the account manager's manager only and say
  so in the task.
- **Internal approver kind:** not escalated by this rule.

#### Must not
- Must not change the timesheet's state at E.
- Must not send the escalation twice for the same submit.

#### Done when
- [ ] Given an external timesheet submitted 2026-11-01 09:00 and still `submitted`, at the
      08:00 run on 2026-11-07 the escalation contact and the account manager's manager are
      notified and one task is created for the account manager; the 08:00 run on 2026-11-06
      sends nothing.
- [ ] Given the account manager has no line manager, the users with
      `timesheets.approve_on_behalf` in scope are notified and the task says so.
- [ ] Running the rule again on 2026-11-08 sends nothing new.
- [ ] If the timesheet is approved on 2026-11-05, nothing is sent and no task exists.
- [ ] After `reject` and re-submit on 2026-11-06, escalation comes on 2026-11-12.

#### Out of scope
Deemed approval at N (BL-ATT-11).

---

### BL-ATT-11 — wl-attendance: deem an external timesheet approved per contract (`deem_approve`)

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-ATT-10 · BL-ATT-13 (reconciliation on deemed approval) · **Blocks:** BL-MPW-19
**Review items:** X-16, 09-N5-b

#### Goal
If the client contract has a deemed-approval clause (N days), the system approves an external
timesheet that has had no answer for N days after `submit`. The client and the account manager
are told the hours, that they were deemed approved, and the dispute route. Without a clause
nothing happens and the timesheet stays `submitted`.

#### Files to read
- `docs/modules/wl-attendance.md` §4 *Timesheet* (`deem_approve`), §5.6, §6 *Automations*
  (`HRM.RUL.15`), §7 (deemed approval), §10 *Deemed approval*
- `docs/OPEN-ITEMS.md` L-29

#### Terms
- **Deemed approval:** approval by silence, valid only where the client contract says so.
- **Approval basis:** `approved`, `on_behalf` or `deemed`, stored on the timesheet.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `deem_approve` | submitted | approved | system (`HRM.RUL.15`) | approver kind is `external`; the context's contract has a deemed-approval clause (N days, from `ApproverResolver`); N days since `submit` with no approval or rejection | as `approve`, approval basis `deemed`; the external approver and the account manager are told the hours and that they were deemed approved |

`HRM.RUL.15` at N (see BL-ATT-10 for the full row): "at N, if the contract has a deemed-approval
clause: timesheet `deem_approve` and notify the client"; N = per contract (off when the contract
has no clause).

§5.6 point 3: "The client is told the hours and the dispute route. A later dispute is handled as
an adjustment (§4)." Point 4: "Billing waits for approval (deemed or not). Payroll does not wait."
§10: whether a deemed-approved timesheet is enough evidence in a billing dispute is
`⚠ open: lawyer (L-29)`.

#### Edge cases decided
- **N read when:** at the time the rule runs, from `ApproverResolver` for the context. If the
  contract changes N while waiting, the current value applies.
- **Rejection then re-submit:** the N days count from the latest `submit`.
- **N ≤ E (6):** both run at the same daily run (BL-ATT-10: 08:00 Asia/Dubai). `deem_approve`
  runs first; the escalation at E then finds the timesheet no longer `submitted` and sends
  nothing. With N < E the escalation never happens.
- **Client approves at the same moment:** the transition engine runs only one; the second
  attempt finds `approved` and does nothing.
- **Adjustment timesheets:** deemed approval applies if the original's contract has the clause.
- **Downstream:** `deem_approve` emits `TimesheetApproved` with approval basis `deemed` and does
  everything `approve` does (lock attendance, reconciliation per BL-ATT-13).

#### Must not
- Must not deem-approve internal timesheets or contexts without a clause.
- Must not hide that approval was deemed: store the basis and show it on invoices' support.

#### Done when
- [ ] Given a contract with deemed approval N = 10 and a timesheet submitted 2026-11-01, on
      2026-11-11 it is `approved` with basis `deemed`, attendance is locked, and the client
      approver and the account manager get the hours and the dispute route.
- [ ] Given a contract with no clause, on 2026-11-11 it is still `submitted`.
- [ ] Given an internal approver kind, it is never deemed approved.
- [ ] `TimesheetApproved` carries approval basis `deemed`.
- [ ] Given a contract with N = 6 and a timesheet submitted 2026-11-01, on 2026-11-07 it is
      deemed approved and no escalation (BL-ATT-10) is sent.
- [ ] Given the timesheet was paid provisionally with 160 h and is deemed approved with 152 h,
      the reconciliation of BL-ATT-13 runs (adjustment −8 h).

#### Out of scope
Escalation at E (BL-ATT-10); later disputes (BL-ATT-07, BL-ATT-09).

---

### BL-ATT-12 — wl-attendance: give payroll provisional hours at the cut-off (`hours_for_payroll`)

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-attendance Timesheet lifecycle · Base: wl-hr `PayrollCalendar` · **Blocks:** BL-ATT-13, BL-HRM-16, BL-MPW-21
**Review items:** X-16, 09-N5-c

#### Goal
Salary does not wait for the client. At the payroll cut-off, `Timesheets::hours_for_payroll`
returns approved hours, plus, for timesheets still `open` or `submitted`, the accepted
attendance hours marked *provisional*. The payroll run records itself as `paid_provisionally_by`
on those timesheets; from then on they count as consumed and cannot be reopened without an
adjustment.

#### Files to read
- `docs/modules/wl-attendance.md` §3 (Timesheet: consumers), §4 *Timesheet* (`reopen`), §5.7,
  §6 *Provides* (`Timesheets`), §7 (payroll basis), §10 *Salary timing*
- `docs/modules/wl-hr.md` §4 *Payroll run* (`calculate`, `submit`), §6 `PayrollCalendar`

#### Terms
- **Cut-off:** the payroll cut-off date per legal entity and period from wl-hr `PayrollCalendar`
  (default day 25).
- **Provisional hours:** accepted attendance hours for a timesheet that is not yet approved,
  used so the worker is paid on time.
- **Policy:** `attendance` (default) or `wait` (only allowed for internal approvers who are the
  employer's own staff).

#### Spec excerpt (authoritative for this issue)

§5.7:
1. "At the payroll cut-off, `Timesheets::hours_for_payroll(period)` returns approved hours, and
   for timesheets still `open` or `submitted` the accepted attendance hours, marked
   *provisional*. Only `accepted` attendance counts: a record still `flagged` at the cut-off is
   left out, and its hours reach payroll later through the reconciliation in step 3 once it is
   accepted. The policy is a setting: `attendance` (default) or `wait` (only for internal
   approvers who are the employer's own staff)."
2. "The payroll run records `paid_provisionally_by` on those timesheets. They can no longer be
   reopened without an adjustment."

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reopen` | approved | open | `timesheets.reopen` | not consumed (provisional payroll use counts as consumed); for external approval, a new external approval will be needed (`HRM.ACL.02`) | labour cost reversed (if posted); attendance unlocked |
| `lock` | approved | locked | system (first consumer: payroll run, usage invoice, WIP billing, export; or at `approve` of a timesheet that already has `paid_provisionally_by`) | — | consumer recorded (`paid_provisionally_by` stays as the payroll consumer); later consumers still read it |

wl-hr `submit` effect (excerpt): "approved timesheets used are `lock`ed; timesheets paid on
provisional hours get `paid_provisionally_by` = this run".

Setting: Payroll basis for timesheets not approved at the cut-off = `attendance` (accepted
attendance hours, provisional; `flagged` records left out until accepted); alternative `wait`
only for internal approvers.

#### Edge cases decided
- **When `paid_provisionally_by` is written:** at payroll run `submit` (wl-hr), not at
  `calculate`; a run that is `return`ed or `cancel`led does not mark anything.
- **Flagged attendance at the cut-off:** only `accepted` hours count; `flagged` records are left
  out and come later through reconciliation.
- **`wait` chosen for an external approver:** refused in settings (09 fixes `attendance`).
- **A provisional timesheet later approved** (spec §4 `approve` and `lock` rows, §5.7 step 3):
  at `approve` it is `lock`ed at once, payroll being its first consumer;
  `paid_provisionally_by` stays as the consumer (no `paid_by` is added). The reconciliation
  (BL-ATT-13) then opens its adjustment on the locked timesheet.
- **Repeated calls** of `hours_for_payroll` for the same period return the same result until data
  changes; it has no side effect.
- **Period boundaries:** hours dated after the cut-off and up to period end belong to the next
  run's provisional set only if the policy says the period ends at the cut-off. The period is the payroll period from `PayrollCalendar`; hours not yet recorded at the
  cut-off are picked up by reconciliation.

#### Must not
- Must not wait for client approval when the policy is `attendance`.
- Must not allow `reopen` of a timesheet with `paid_provisionally_by` set.
- Must not mark provisional hours as approved.

#### Done when
- [ ] Given timesheet T (client approver) `submitted` with 160 accepted attendance hours for
      November and the cut-off on 2026-11-25, `hours_for_payroll` returns 160 h marked provisional.
- [ ] Given an `approved` timesheet U with 150 h, it returns 150 h not provisional.
- [ ] Given T also has an 8 h attendance record still `flagged` at the cut-off, it returns 160 h,
      not 168 h.
- [ ] When the payroll run is submitted, T has `paid_provisionally_by` = the run and U is
      `locked` with `paid_by` = the run.
- [ ] When T is later approved, it moves to `locked` in the same step, with
      `paid_provisionally_by` = the run and no `paid_by`; `reopen` on T is refused.
- [ ] With policy `wait` on an internal context, unapproved T returns 0 h.
- [ ] Setting `wait` for an external approver context is refused.

#### Out of scope
Reconciliation on approval (BL-ATT-13); how payroll shows provisional hours (BL-HRM-16).

---

### BL-ATT-13 — wl-attendance: reconcile provisional hours when the timesheet is approved

**Labels:** `module:wl-attendance` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-ATT-07 · BL-ATT-12 · **Blocks:** BL-HRM-16, BL-ATT-11
**Review items:** X-16, 09-N5-c

#### Goal
When a timesheet that payroll used provisionally is approved (or deemed approved), the approved
hours are compared with the provisional hours. A difference creates an adjustment timesheet with
origin `provisional_reconciliation`, affecting `payroll`, and emits
`TimesheetReconciliationNeeded`, so wl-hr puts the difference in the next run.

#### Files to read
- `docs/modules/wl-attendance.md` §4 *Timesheet* (`approve` effects, `open_adjustment`), §5.7
  point 3, §6 *Events emitted*

#### Terms
- **Delta per category:** approved hours minus provisional hours, per day, cost object and
  category (normal, overtime, rest day, public holiday).

#### Spec excerpt (authoritative for this issue)

§5.7 point 3: "When the timesheet is approved (or deemed approved), the approved hours are
compared with the provisional hours. A difference creates an adjustment timesheet with origin
`provisional_reconciliation`, affecting `payroll`, and emits `TimesheetReconciliationNeeded`;
`wl-hr` puts the difference in the next run (− within the Art. 25 limits)."

| Event | When | Payload |
|---|---|---|
| `TimesheetReconciliationNeeded` | approval of a timesheet paid provisionally, with different hours | timesheet, payroll run, provisional hours, approved hours, delta per category, adjustment timesheet |

`open_adjustment` By: "`timesheets.adjust`, or system (`provisional_reconciliation`, §5.7)".
`approve` By (excerpt): "or system for a `provisional_reconciliation` adjustment (§5.7 step 3)".

§5.7 step 3 (rest): "When the timesheet is approved (or deemed approved), it is `lock`ed at
once (payroll is its first consumer) … The original's approval already covers these hours, so
the system submits and approves the adjustment in the same step: approver and approval basis
are copied from the original's approval, it is not sent to the approver again, and it emits no
`TimesheetApproved` of its own (payroll applies the difference once)."

#### Edge cases decided
- **State of the system adjustment** (spec §5.7 step 3): created by `open_adjustment` (system),
  then `submit` and `approve` by the system in the same step, ending `approved`; approver and
  approval basis are copied from the original's approval (e.g. the client user and `approved`,
  or `deemed`). It is not sent to the approver again. BL-ATT-08's user approval does not apply.
- **Events:** `TimesheetReconciliationNeeded` only; no `TimesheetApproved` for this adjustment,
  so wl-hr applies the difference once.
- **Order:** the original is `lock`ed first (BL-ATT-12), so the `open_adjustment` guard "the
  original is `locked`" holds.
- **No difference:** nothing is created and no event is emitted.
- **Approval moved through `approve_on_behalf` or `deem_approve`:** same reconciliation.
- **Run idempotency:** one reconciliation per (timesheet, payroll run); re-running the handler
  does not create a second adjustment.
- **Category moves** (same total, different categories): a difference per category counts.

#### Must not
- Must not change the paid payroll run.
- Must not affect billing (`affects` = `payroll`).

#### Done when
- [ ] Given T paid provisionally with 160 h normal by run R, when the client approves 152 h
      normal, then an adjustment with −8 h normal, origin `provisional_reconciliation`, affects
      `payroll` exists and `TimesheetReconciliationNeeded` carries provisional 160, approved 152,
      delta −8 normal and the adjustment.
- [ ] Given the client approves 160 h, nothing is created.
- [ ] Given 150 normal + 10 overtime approved vs 160 normal provisional, delta −10 normal, +10
      overtime.
- [ ] The adjustment of the first item is `approved` with the approver and basis of T's
      approval, the approver got no new request, and no `TimesheetApproved` is emitted for it.
- [ ] Handling the approval event twice creates one adjustment.

#### Out of scope
Payroll applying the delta (BL-HRM-16).
