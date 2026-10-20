# Epic: wl-hr — negative flows

> Status: draft · Last updated: 2026-10-20 · Spec version: same commit as this file

This epic builds the HR and payroll negative flows added by the review fix X-15 and the wl-hr
part of X-13: the qualification guard and expiry event, same-day sick leave, visa rejection,
non-standard employee exits (onboarding cancelled, notice withdrawn, absconding, reinstatement,
death), payroll `return`, WPS rejection and partial failure, supplementary and off-cycle runs,
recovery caps, pay account change control, and settlement dispute, negative net, closure and the
14-day rule. It closes X-15, X-13 (wl-hr part), 09-N12-b, 09-N6-b, 09-N7-b, 09-N3-d,
09-N11-b, 09-N8-c, 09-N4-b, 09-N5-d, 09-N12-c, the 09 MEDIUM item "notice withdrawal", and
supports 05-N8-a, 10-N8-c and 04-N8-d. BL-HRM-26 to 29 add staff onboarding checklists
(`HRM.ONB.01`, gap G6 of the [SME problem map](../../reviews/2026-10-16-sme-problem-map.md)).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-HRM-01 | Answer `qualification_valid` and emit `QualificationExpired` (`HRM.RUL.10`) | high | m | Launch 02 | Base: wl-hr Qualification records; Epic wl-documents: `document_valid`, `DocumentExpired` |
| BL-HRM-02 | Record same-day sick leave as approved (`record_sick`) | medium | s | Launch 05 | Base: wl-hr Leave request lifecycle |
| BL-HRM-03 | Reject a visa case with follow-up tasks | high | m | Launch 09 | Base: wl-hr Visa case lifecycle; Epic wl-documents: `DocumentExpiring` |
| BL-HRM-04 | Approve non-standard exits by a second user (`HRM.ACL.07`) | high | s | Launch 02 | Base: wl-hr Employee lifecycle; Epic wl-kernel: approval requests |
| BL-HRM-05 | Cancel onboarding (`cancel_onboarding`) | high | m | Launch 09 | BL-HRM-03; BL-HRM-04; BL-HRM-24 |
| BL-HRM-06 | Withdraw a notice by agreement (`withdraw_notice`) | medium | s | Launch 09 | BL-HRM-04; BL-HRM-24 |
| BL-HRM-07 | Report an employee as absconded (`report_absconding`) | high | m | Launch 02 | BL-HRM-04; Epic wl-attendance: attendance records; Epic wl-ticketing: absence case tickets |
| BL-HRM-08 | Reinstate an absconded employee (`reinstate`) | high | s | Launch 09 | BL-HRM-04; BL-HRM-07; BL-HRM-18; BL-HRM-24; BL-HRM-30 |
| BL-HRM-09 | Record an employee's death (`record_death`) | high | m | Launch 09 | BL-HRM-04 |
| BL-HRM-10 | Exit an absconded or deceased employee | high | s | Launch 09 | BL-HRM-07; BL-HRM-09; BL-HRM-30 |
| BL-HRM-11 | Return a payroll run to draft (`return`) | high | s | Launch 09 | Base: wl-hr Payroll run lifecycle |
| BL-HRM-12 | Record a rejected WPS file and resubmit | high | m | Launch 09 | Base: wl-hr Payroll run lifecycle |
| BL-HRM-13 | Record a partly failed WPS file and regenerate | high | m | Launch 09 | BL-HRM-12; BL-HRM-18; BL-HRM-20; BL-HRM-21 |
| BL-HRM-14 | Escalate WPS rejections before the due date (`HRM.RUL.11`) | high | s | Launch 09 | BL-HRM-12; BL-HRM-13 |
| BL-HRM-15 | Keep salary recoveries within the Art. 25 caps (`HRM.ACL.06`) | high | s | Launch 09 | Base: wl-hr Adjustments and payroll calculation |
| BL-HRM-16 | Calculate payroll with provisional hours and timesheet corrections | high | m | Launch 09 | BL-ATT-08; BL-ATT-12; BL-ATT-13; BL-HRM-15 |
| BL-HRM-17 | Correct a paid run with a supplementary run | high | m | Launch 09 | BL-HRM-15; BL-HRM-18 |
| BL-HRM-18 | Release held lines through an off-cycle run (`HRM.RUL.12`) | high | m | Launch 09 | Base: wl-hr Payroll run lifecycle; Vertical 09: absence case |
| BL-HRM-19 | Request and approve a new pay account with SMS notice | high | m | Launch 09 | Base: wl-hr Employee records; Epic wl-notify: SMS |
| BL-HRM-20 | Activate a pay account after cooling and use it at submit | high | s | Launch 09 | BL-HRM-19; BL-HRM-18 |
| BL-HRM-21 | Stop a pay account change, waive cooling, handle undelivered notice | high | m | Launch 09 | BL-HRM-19; BL-HRM-20; Epic wl-ticketing: security tickets; Epic wl-notify: `MessageGaveUp`; Epic wl-kernel: approval requests |
| BL-HRM-22 | Dispute and resolve a final settlement | high | m | Launch 09 | Base: wl-hr Final settlement lifecycle; Epic wl-ticketing: labour-case tickets |
| BL-HRM-23 | Handle a negative final settlement | high | s | Launch 09 | Base: wl-hr Final settlement lifecycle; BL-HRM-15 |
| BL-HRM-24 | Close or cancel a final settlement | medium | s | Launch 09 | Base: wl-hr Final settlement lifecycle |
| BL-HRM-25 | Alert on the 14-day settlement deadline (`HRM.RUL.09`) | high | s | Launch 09 | Base: wl-hr Final settlement lifecycle; BL-HRM-22 |
| BL-HRM-26 | Start an onboarding from the role's templates when an employee is hired (`HRM.RUL.16`) | medium | m | Build 1 | Base: wl-hr Employee lifecycle |
| BL-HRM-27 | Complete and waive onboarding steps (`HRM.ACL.10`) | medium | m | Build 1 | BL-HRM-26; BL-DOC-08; BL-DOC-09; Epic wl-ticketing: checklist runs |
| BL-HRM-28 | Remind and escalate overdue onboarding steps (`HRM.RUL.17`) | medium | s | Build 1 | BL-HRM-26 |
| BL-HRM-29 | Block employee activation until a blocking onboarding is complete | medium | s | Build 1 | BL-HRM-26; BL-HRM-27 |
| BL-HRM-30 | Hold pay, draft the settlement and open the cancellation case when an employee is reported absconded | high | m | Launch 09 | BL-HRM-07; Base: wl-hr Payroll run lifecycle; Base: wl-hr Final settlement lifecycle; Base: wl-hr Visa case lifecycle |

---

### BL-HRM-01 — wl-hr: answer `qualification_valid` and emit `QualificationExpired` (`HRM.RUL.10`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:system` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: wl-hr Qualification records · Epic wl-documents: `document_valid`, `DocumentExpired`, `DocumentExpiring` (with `affected`), `DocumentAccepted` · **Blocks:** BL-ATT-04, BL-SCH-07, BL-SCH-08, BL-ACC-19, BL-ACC-23, BL-CLN-12, BL-CON-18, BL-EDU-08, BL-EDU-10, BL-LOG-25, BL-MPW-08, BL-MPW-11
**Review items:** X-13, 05-N8-a, 09-N8-b, 10-N8-c, 04-N8-d

#### Goal
Other modules ask one question: "may employee E do kind K (in role R at scope S) on date D?".
`qualification_valid` answers it through the wl-documents guard plus role and scope. On the
expiry date of a blocking kind, `HRM.RUL.10` makes the guard answer *no*, emits
`QualificationExpired` with the employee's open and planned work (the blocked records that
`DocumentExpired` carries), and tells the employee's manager and HR.
Bookings (05), deployments (09), tutor assignments (10), run dispatch (03) and return filing
(04) all rely on it.

#### Surfaces
- **System:** `qualification_valid(employee, kind, on_date[, role, scope])` answers other
  modules' guards; on the day after a blocking kind expires `HRM.RUL.10` emits
  `QualificationExpired` with the blocked work and notifies the manager and HR.

#### Files to read
- `docs/modules/wl-hr.md` §2 (`HRM.QUA.01`, *Standard guard (X-13)*), §3 (Qualification), §5.5,
  §6 *Provides* (`Qualifications`), §6 *Consumes* (wl-documents), §6 *Events emitted*,
  §6 *Automations* (`HRM.RUL.10`)
- `docs/modules/wl-documents.md` §5.6

#### Terms
- **Qualification:** a licence, permit or certificate with type, number, issuer, scope
  (branches, roles), valid from/to and a stored document.
- **Role-bound kind:** a kind whose validity also depends on role and scope (e.g. a practitioner
  licence for branch X).
- **Blocking kind:** a document kind whose `blocks` setting names transitions it guards.
- **The employee's manager:** every user holding the *manager* role in one of the employee's
  scope assignments (wl-kernel `ApproverResolver`); none: HR only. There is no manager field.

#### Spec excerpt (authoritative for this issue)

"**Standard guard (X-13).** Other modules call `qualification_valid(employee, kind, on_date)`
(the `Qualifications` service), or `qualification_valid(employee, kind, on_date, role,
scope)` for a role-bound kind. It answers *yes* when the employee has a valid qualification
or employee document of that kind on `on_date` (through `document_valid` in wl-documents §5.6)
and, for role-bound kinds, the qualification's roles and scope include the `role` and `scope`
passed (the role and branch of the work being guarded, e.g. the booking's service role and
branch). For a role-bound kind called without `role` and `scope`, it answers *no*. The
document kind's `blocks` setting names the transitions it guards."

§3: "**The employee's manager.** The employee record has no manager field. Where this module
says "the manager" or "the line manager" of an employee, it means every user who holds the
*manager* role in one of the employee's scope assignments, resolved through wl-kernel's
`ApproverResolver` ("manager of the employee's branch"). When no such user exists, the
message or right falls to HR (`employees.manage`)."

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.10` (from `CLN.RUL.01`) | qualification or employee document of a blocking kind reaches its expiry date (`DocumentExpired`) | — | `qualification_valid` answers *no* for that role and scope; emit `QualificationExpired` with the employee's open and planned work (the blocked records carried by `DocumentExpired`); notify the employee's manager (§3) and HR | — |

| Event | When | Payload |
|---|---|---|
| `QualificationExpiring` / `QualificationExpired` | from document expiry (`HRM.RUL.10`) | employee, qualification, scope, affected work (`QualificationExpiring`: the `affected` list of `DocumentExpiring`; `QualificationExpired`: the blocked open and planned records of `DocumentExpired`) |

§6 Consumes: "wl-documents | `DocumentExpiring` (with `affected`), `DocumentExpired`,
`DocumentAccepted` for employee kinds → qualification events".

§5.5 point 5: "Planned work after the expiry date is flagged ahead (`DOC.RUL.02`). On the expiry
date `HRM.RUL.10` emits `QualificationExpired`; work in progress reacts in the vertical: 09
suspends active deployments (no attendance, no billing) until the renewed document is accepted."

#### Edge cases decided
- **`on_date` in the past or future:** passed through to `document_valid` (it handles both).
- **Expiry date itself:** valid *on* the expiry date (wl-documents: expiry date on or after
  `on_date`); `HRM.RUL.10` runs at 00:00 Asia/Dubai the day after. Default (spec silent on the
  exact hour; follows the "on or after" rule).
- **Renewal accepted:** `DocumentAccepted` makes the guard answer *yes* again; Default (spec
  silent on a "renewed" event): no event other than the wl-documents one; consumers listen to
  `DocumentAccepted`.
- **`QualificationExpiring`:** emitted from `DocumentExpiring`, carrying its `affected` list
  unchanged.
- **Idempotency:** one `QualificationExpired` per qualification per expiry date.
- **Freelancers:** the guard works the same (their permit is a document kind).
- **Source of "open and planned work":** wl-hr does not collect it; it copies the blocked
  records of `DocumentExpired` (wl-documents `DOC.RUL.03`, built from the `FutureWork` and
  `BlockSubjects` providers) into `QualificationExpired`.
- **Role-bound kind called with three arguments:** answers *no* (the caller must say which
  role and scope the work needs).

#### Must not
- Must not keep a separate expiry check that could disagree with `document_valid`.
- Must not ignore role and scope for role-bound kinds.

#### Done when
- [ ] Given Dr A's licence (role `practitioner`, scope Branch 1) valid to 2026-11-20,
      `qualification_valid(A, practitioner_licence, 2026-11-20, practitioner, Branch 1)` is
      *yes* and for 2026-11-21 is *no*.
- [ ] With scope Branch 2 (not in the licence's scope) it is *no* on any date; called without
      role and scope it is *no*.
- [ ] Given `DocumentExpired` for A's licence lists two bookings on 2026-11-24, on 2026-11-21
      `QualificationExpired` is emitted once with those two bookings, and the manager of
      Branch 1 and HR are notified (HR only when Branch 1 has no manager).
- [ ] Given a renewal still `pending_review`, the guard answers *no*.
- [ ] After the renewal is accepted, the guard answers *yes* for 2026-11-25.

#### Out of scope
Consumers' reactions (BL-SCH-07, BL-ATT-04; vertical 09 deployment suspend; Epic wl-delivery run
dispatch).

---

### BL-HRM-02 — wl-hr: record same-day sick leave as approved (`record_sick`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:medium` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** Base: wl-hr Leave request lifecycle · **Blocks:** BL-SCH-05
**Review items:** X-17 (05 MEDIUM "practitioner sick on the day")

#### Goal
An employee who calls in sick today is recorded directly as approved sick leave, so
`LeaveApproved` goes out at once and today's bookings and rosters are moved. The sick note may
follow within N working days (default 2); if it does not, HR gets a task.

#### Surfaces
- **Back office:** the employee's manager (or a `leave.approve` user) runs **Record sick
  leave** for today on the employee; it is approved at once, and the HR task for a missing
  sick note appears in HR's inbox.
- **System:** at 00:00 Asia/Dubai after the Nth working day without a sick note the HR task
  is raised; attaching the note closes it.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Leave request*, §5.2 step 3, §7 (sick note due)

#### Terms
- **`LeaveApproved`:** event consumed by wl-scheduling (blocks, conflicts), rosters, 09 cover.
- **The employee's manager (spec "line manager"):** every user holding the *manager* role in
  one of the employee's scope assignments (wl-hr §3, via wl-kernel `ApproverResolver`); they
  may run `record_sick` for that employee without `leave.approve`.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `record_sick` | — | approved | `leave.approve`, or the employee's manager (§3) | type sick; starts today (the employee called in sick); the sick note may follow within N days | same effects as `approve` at once, so today's bookings and rosters are moved (`wl-scheduling` §5.5); event `LeaveApproved`; a task to HR when the sick note is not attached by the end of the Nth working day after the start date (raised at 00:00 Asia/Dubai the next day) |
| `approve` | requested | approved | `leave.approve` | balance sufficient, or the excess marked unpaid; required document attached (e.g. sick note) | calendars blocked (`wl-scheduling`, rosters); event `LeaveApproved` |

Setting: Sick note due after same-day sick leave (`record_sick`) = 2 working days after the
start date; HR task at 00:00 Asia/Dubai the next day.

#### Edge cases decided
- **Balance:** sick pay bands (15 full, 30 half, rest unpaid; unpaid in probation) still apply;
  days beyond the paid bands are marked unpaid automatically, as `approve` does.
- **"Today":** Asia/Dubai date; the leave starts at the time recorded (rest of the day blocked).
- **Working days:** from the business calendar; the start date itself is not counted.
- **When the task is raised:** the note is due by 23:59 Asia/Dubai on the Nth working day after
  the start date; the task is raised at 00:00 the next day if no note is attached.
- **Manager of another scope:** a manager who is not the employee's manager (§3) and lacks
  `leave.approve` is refused.
- **Repeated:** one task per request, even if the job runs twice.
- **Note attached later:** the task closes automatically.

#### Must not
- Must not require the sick note before recording.
- Must not allow a start date other than today.

#### Done when
- [ ] Given employee A (Branch 1) calls in on Monday 2026-11-02, when the manager of Branch 1
      (without `leave.approve`) runs `record_sick` for 2026-11-02 to 2026-11-03, the request
      is `approved` and `LeaveApproved` is emitted; the manager of Branch 2 is refused.
- [ ] A start date of 2026-11-03 is refused.
- [ ] With no sick note by 23:59 on Wednesday 2026-11-04 (2nd working day after the start), HR
      gets one task at 00:00 on 2026-11-05; with the note attached on 2026-11-04, no task.
- [ ] Attaching the note closes the task.

#### Out of scope
Booking conflicts (BL-SCH-05).

---

### BL-HRM-03 — wl-hr: reject a visa case with follow-up tasks

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** Base: wl-hr Visa case lifecycle · Epic wl-documents: `DocumentExpiring` (`DOC.RUL.02`) · **Blocks:** BL-HRM-05, BL-MPW-13
**Review items:** X-15, 09-N8-c, 09-N12-b

#### Goal
When the authority refuses a visa, the case moves to `rejected` (no longer hidden in
`cancelled`). For a *new* visa, HR gets a task to cancel onboarding or open another case. For a
*renewal*, HR gets a task due 30 days before the current residence visa expires, to try another
route or give notice; active deployments are told (through `VisaCaseRejected`, then
`DocumentExpiring`) and the MD is notified.

#### Surfaces
- **Back office:** the PRO runs **Reject** on the visa case with the authority reference and
  reason; HR sees the follow-up task (cancel onboarding or open another case, or the renewal
  deadline) under "Needs your action".
- **System:** `VisaCaseRejected` carries the deadline and the current visa expiry so the
  vertical can tell active deployments; the MD is notified.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Visa case: rejection), §4 *Visa case*, §5.4 "Renewal refused",
  §6 *Events emitted*, §7 (deadline task after a rejected renewal), §10 *Renewal refused*
- `docs/OPEN-ITEMS.md` AU-23

#### Terms
- **Case types:** `new`, `renewal`, `cancellation` (plus configured `transfer`, `status_change`).
- **`DocumentExpiring` (`affected`):** wl-documents event listing planned work affected by an
  expiring document.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `rejected` | error | yes | Refused by the authority (MoHRE, GDRFA or ICP) |
| `cancelled` | neutral | yes | Abandoned (candidate withdrew, no longer needed) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reject` | in_progress, on_hold | rejected | `visas.manage` | authority reference and reason recorded | fees already paid stay posted; event `VisaCaseRejected`. *New*: HR task to `cancel_onboarding` or open another case. *Renewal*: HR task with a deadline N days before the current residence visa expires (default 30; due today when that day has passed) to open a new case by another route, or to `give_notice` and open a *cancellation* case; active deployments told through `VisaCaseRejected`, which carries the current visa's expiry date (the vertical reacts; 09 deployments), and again through `DocumentExpiring` (`affected`) once the visa enters its look-ahead window (`DOC.RUL.02`); MD notified |
| `cancel` | open, in_progress, on_hold | cancelled | `visas.manage` | reason given | fees already paid stay posted |

| Event | When | Payload |
|---|---|---|
| `VisaCaseRejected` | visa case `reject` | case, type, employee, deadline, current residence visa expiry date (*renewal*) |

§10: "what the worker's status is after a refused renewal, and how long they may stay after the
visa expires, is `⚠ open: authority (AU-23)`; the module only sets a task deadline before the
current visa expires."

#### Edge cases decided
- **Deadline already passed** (visa expires in less than 30 days): the task is due today.
- **Telling active deployments:** wl-hr emits `VisaCaseRejected` with the current residence
  visa's expiry date; the vertical reacts (09 lists the employee's active deployments). wl-hr
  does not call wl-documents and does not build an `affected` list; `DocumentExpiring` with
  `affected` comes later from wl-documents' own nightly `DOC.RUL.02` when the visa enters its
  look-ahead window.
- **`deadline` and visa expiry in the event:** for *new* cases, both empty.
- **Other types** (`cancellation`, `transfer`): only the event and fees; no task. Default (spec
  silent).

#### Must not
- Must not reverse fees already posted.
- Must not change the employee's state automatically (HR decides).

#### Done when
- [ ] Given a *renewal* case `in_progress` for employee A whose visa expires 2026-12-31, when PRO
      records reference "GDRFA-123" and reason, the case is `rejected`, `VisaCaseRejected` has
      deadline 2026-12-01 and visa expiry 2026-12-31, HR has a task due 2026-12-01, and the
      MD is notified.
- [ ] Given a *new* case, HR gets a task "cancel onboarding or open another case".
- [ ] Without an authority reference, `reject` is refused.
- [ ] From `open`, `reject` is not offered.

#### Out of scope
`cancel_onboarding` (BL-HRM-05); `give_notice` (base).

---

### BL-HRM-04 — wl-hr: approve non-standard exits by a second user (`HRM.ACL.07`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** Base: wl-hr Employee lifecycle · Epic wl-kernel: approval requests (approval returned or rejected sends the record back) · **Blocks:** BL-HRM-05, BL-HRM-06, BL-HRM-07, BL-HRM-08, BL-HRM-09, BL-CON-18, BL-MPW-21
**Review items:** X-15

#### Goal
Five transitions change an employee's status outside the normal path: `cancel_onboarding`,
`withdraw_notice`, `report_absconding`, `reinstate`, `record_death`. HR requests them with a
reason and documents; another user with `employees.exit.approve` (default the MD) approves; only
then the transition runs. Everything is audited.

#### Surfaces
- **Back office:** HR requests a non-standard exit with reason and documents; the employee
  keeps its state with a badge showing the request; the MD (`employees.exit.approve`)
  approves or rejects it under "Needs your action".
- **System:** guards run again at final approval; a failed check ends the request `failed`;
  request, decisions and transition are audited.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Employee* (transitions), §6 *Permissions*, §6 *Access rules*
  (`HRM.ACL.07`)
- `docs/modules/wl-kernel.md` §4 *Approval request*

#### Terms
- **Approval request:** wl-kernel record; the action runs as the requester when the last
  approver approves; `reject`/`return` leave the subject unchanged.
- **`failed` (wl-kernel):** terminal state of an approval request reached by the system
  transition `fail` when "the action's guards no longer pass at final approval"; the subject
  stays as it was and requester and approver are notified with the reason
  (wl-kernel §4 *Approval request*). Nothing to build here beyond calling the guards.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Generalises | Pattern | Default |
|---|---|---|---|---|
| `HRM.ACL.07` | A non-standard exit or reversal (`cancel_onboarding`, `withdraw_notice`, `report_absconding`, `reinstate`, `record_death`) is requested by HR and approved by another user with `employees.exit.approve`, with reason and documents; one pending request per employee (a second is refused until the first ends); audited | new (X-15) | `SOD` | approver: MD |

| Permission | Allows |
|---|---|
| `employees.manage` | create and edit employees; activate, notice, exit; request the non-standard exits (approved under `HRM.ACL.07`) |
| `employees.exit.approve` | approve `cancel_onboarding`, `withdraw_notice`, `report_absconding`, `reinstate`, `record_death` (default: MD) |

#### Edge cases decided
- **Reason and documents per transition:** a reason is always required. The documents are the
  ones each transition's guard names: `cancel_onboarding` none beyond the visa case states;
  `withdraw_notice` the other party's written agreement; `report_absconding` the MoHRE absence
  report number and date (and the report file when available); `reinstate` the withdrawal or
  cancellation reference; `record_death` the death certificate or the authority's notice. A
  request missing them is refused at request (guard), not at approval.
- **Audit:** request, each decision and the transition are written to the audit log with
  user, time, reason and the attached documents.
- **Guards checked twice:** at request and again at final approval; if they fail at approval the
  request ends `failed` (wl-kernel) and nothing runs.
- **Two requests for the same employee:** only one pending non-standard
  exit request per employee; a second is refused until the first ends.
- **MD is the requester:** another holder of `employees.exit.approve` must approve; if none,
  the request waits (no self-approval).
- **No waiting state:** the employee keeps its state while pending; a badge shows the request.

#### Must not
- Must not run the transition before approval.
- Must not let the requester approve, even an admin or the MD.

#### Done when
- [ ] Given HR user A requests `report_absconding` for employee E with reason and report number,
      E's state is unchanged and an approval request exists.
- [ ] When A tries to approve, it is refused; when MD B approves, E moves and the history shows
      requester A and approver B.
- [ ] When B rejects, E is unchanged and A is notified with the comment.
- [ ] A second request for E while the first is pending is refused.
- [ ] A `record_death` request without the certificate or authority notice is refused at
      request; no approval request is created.
- [ ] After B approves `report_absconding`, the audit log has three entries (request by A,
      approval by B, transition) each with the reason "absent since 2026-11-01" and the report
      file.

#### Out of scope
Each transition's guards and effects (BL-HRM-05 to BL-HRM-09).

---

### BL-HRM-05 — wl-hr: cancel onboarding (`cancel_onboarding`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** BL-HRM-04 · BL-HRM-03 · BL-HRM-24 (settlement `cancel`/paid) · **Blocks:** BL-MPW-13
**Review items:** X-15, 09-N12-b

#### Goal
An employee in `onboarding` who never starts (visa refused, candidate withdrew, medically
unfit) had no way out. `cancel_onboarding` ends the record in `onboarding_cancelled`: open
visa work is closed, a cancellation case runs if a permit was issued, any days worked are
paid by a final settlement that HR drafts beforehand, planned deployments are cancelled
through an event, and visa costs stay with the employer.

#### Surfaces
- **Back office:** HR drafts the *onboarding cancelled* settlement when needed and requests
  **Cancel onboarding** on the employee; the PRO records *cancellation not needed* on the
  *new* visa case.
- **System:** after approval the record moves to `onboarding_cancelled`, the portal account
  is closed and `EmployeeOnboardingCancelled` lets the apps cancel planned deployments.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Employee: exit route), §4 *Employee*, §5.4 "Onboarding cancelled",
  §6 *Events emitted*, §10 *Visas and IDs* (Art. 6(4))

#### Terms
- **Cost object:** the employee as a `Subject` that costs are posted to (`FIN.CST.01`).
- **Art. 6(4):** recruitment and employment costs are borne by the employer.
- **Cancellation not needed:** a record on the *new* visa case (by whom, reason) made by a
  user with `visas.manage` (usually the PRO) when a permit was issued but no cancellation
  case is required. It replaces the earlier wording "PRO note".

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `onboarding_cancelled` | neutral | yes | Never started work (visa refused or cancelled, candidate withdrew, medically unfit) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel_onboarding` | onboarding | onboarding_cancelled | `employees.manage`, approved under `HRM.ACL.07` | reason given; open *new* visa case `cancel`led or `rejected`; when a work permit or entry permit was already issued, a *cancellation* visa case `completed` (or *cancellation not needed* recorded on the *new* case by `visas.manage`, with a reason); when the employee has attendance, wages for those days paid by a final settlement (`paid`) that HR drafted with reason *onboarding cancelled* (§5.4); no settlement needed without attendance | planned deployments and assignments cancelled through event `EmployeeOnboardingCancelled`; visa and onboarding costs stay on the employee cost object and are not recovered from the person (Art. 6(4)); portal account (if any) closed |

| Event | When | Payload |
|---|---|---|
| `EmployeeOnboardingCancelled` | `cancel_onboarding` | employee, reason |

Final settlement reason list includes `onboarding cancelled`.

§5.4: "The employee never enters `notice`, so nothing drafts a settlement automatically: when
the employee has attendance, HR drafts the final settlement (reason *onboarding cancelled*:
unpaid salary for the days worked), has it approved and paid, and then requests
`cancel_onboarding`."

#### Edge cases decided
- **No days worked:** the guard "wages … paid by a final settlement" is met when there is no
  attendance for the employee; no settlement needed (guard).
- **Who creates the settlement:** HR (`settlements.prepare`) drafts it by hand with reason
  *onboarding cancelled* before the request; another user approves (`HRM.ACL.04`) and it is
  paid (`pay`). No automation drafts it (§5.4).
- **"Permit already issued":** read from the *new* case's steps (work permit or entry permit step
  done).
- **Exit route:** set to `onboarding cancelled`.
- **Adjustment types:** no visa or recruitment cost adjustment may be created (none exists).

#### Must not
- Must not create a recovery from the person for visa or onboarding costs.
- Must not cancel deployments directly: emit the event; apps react.

#### Done when
- [ ] Given E in `onboarding` with the *new* case `rejected`, no permit issued, no days worked,
      when the approved request runs, E is `onboarding_cancelled`, exit route set, portal closed,
      `EmployeeOnboardingCancelled` emitted.
- [ ] Given the *new* case still `in_progress`, it is refused.
- [ ] Given a work permit issued and no completed *cancellation* case and no *cancellation not
      needed* record, it is refused; after PRO user P records *cancellation not needed* with
      reason "permit expired unused", it succeeds.
- [ ] Given 3 days of attendance and no `paid` settlement, it is refused; after HR drafts a
      settlement (reason *onboarding cancelled*, AED 300 unpaid salary), B approves and it is
      `paid`, it succeeds.
- [ ] Visa fees AED 3,000 stay on E's cost object and no adjustment against E exists.

#### Out of scope
Approval mechanics (BL-HRM-04); 09 deployment reaction (vertical 09).

---

### BL-HRM-06 — wl-hr: withdraw a notice by agreement (`withdraw_notice`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:medium` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** BL-HRM-04 · BL-HRM-24 · **Blocks:** BL-MPW-13
**Review items:** X-15, 09 MEDIUM "notice withdrawal"

#### Goal
An employee in `notice` can return to `active` when both sides agree in writing and the visa is
not yet cancelled. The draft or approved settlement and the cancellation case are cancelled and
the last working day is cleared.

#### Surfaces
- **Back office:** HR attaches the other party's written agreement and requests **Withdraw
  notice**; the refusal names why (settlement paid, disputed or a payment recorded).
- **System:** after approval the settlement and the cancellation case are cancelled, the
  last working day is cleared and `EmployeeNoticeWithdrawn` is emitted.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Employee*, §4 *Final settlement* (`cancel`), §5.4 "Notice
  withdrawn", §10 *Withdrawing notice*
- `docs/OPEN-ITEMS.md` L-26

#### Terms
- **Other party:** the employee when the employer gave notice; the employer when the employee
  resigned.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `withdraw_notice` | notice | active | `employees.manage`, approved under `HRM.ACL.07` | the other party's written agreement attached (`⚠ open: lawyer (L-26)`, §10); *cancellation* visa case not `completed`; final settlement `draft` or `approved` with no payment recorded against it (a settlement `paid`, `disputed` or partly paid cannot be cancelled, so the notice stands) | draft or approved final settlement `cancel`led; *cancellation* visa case `cancel`led; last working day cleared; event `EmployeeNoticeWithdrawn` |
| `cancel` (settlement) | draft, approved | cancelled | `settlements.prepare` or system (`withdraw_notice`, `reinstate`) | nothing paid against it | — |

#### Edge cases decided
- **Settlement already `paid` or `disputed`, or a payment recorded against it:** it cannot be
  cancelled, so `withdraw_notice` is refused (guard), with a message per case: `paid` or
  `closed`: "settlement already paid"; `disputed`: "settlement is disputed"; `approved` with a
  payment recorded: "a payment is recorded against the settlement".
- **"Written agreement attached":** the system checks only that a document of kind *notice
  withdrawal agreement* is attached and names the other party (employee or employer). It does
  not verify the signature; the approver under `HRM.ACL.07` checks it before approving. Whether
  agreement is needed at all is L-26 (does not block the build).
- **No cancellation case** (not sponsored): the second guard passes.
- **Notice reason and dates:** kept in history; cleared on the employee.

#### Must not
- Must not withdraw without the written agreement attached.
- Must not run when the cancellation case is `completed`.

#### Done when
- [ ] Given E in `notice`, settlement `approved`, cancellation case `in_progress`, agreement
      attached, when approved, E is `active`, settlement and case `cancelled`, last working day
      empty, `EmployeeNoticeWithdrawn` emitted.
- [ ] Given the cancellation case `completed`, it is refused.
- [ ] Given the settlement `paid`, it is refused with "settlement already paid"; given it
      `disputed`, with "settlement is disputed"; given `approved` with AED 500 recorded, with
      "a payment is recorded against the settlement".
- [ ] Without the agreement document, it is refused.

#### Out of scope
Approval mechanics (BL-HRM-04).

---

### BL-HRM-07 — wl-hr: report an employee as absconded (`report_absconding`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-HRM-04 · Epic wl-attendance: attendance records (last attended day) · Epic wl-ticketing: the *absence* ticket the vertical's rule opens (02 `CON.RUL.08`, 09 `MPW.RUL.09`) · **Blocks:** BL-HRM-08, BL-HRM-10, BL-HRM-30, BL-CON-15, BL-CON-18, BL-LOG-13, BL-MPW-13, BL-MPW-17
**Review items:** X-15, 09-N12-c

#### Goal
After more than 7 consecutive days of absence with the worker unreachable and a MoHRE absence
report filed, HR moves the employee to `absconded`. This issue builds the transition, its
guards, approval, the closing of portal and attendance, and `EmployeeAbsconded`, which ends
deployments and crew membership. Holding pay outside WPS, drafting the final settlement and
opening the cancellation case need payroll, settlement and visa cases (phase F9) and are
provided by BL-HRM-30 (Launch 09).

#### Surfaces
- **Back office:** HR records contact attempts and the MoHRE report number and date on the
  employee and requests **Report absconding**; the MD approves it.
- **System:** on approval portal and attendance are disabled and `EmployeeAbsconded` is
  emitted. The payroll hold, the settlement draft and the cancellation case are provided by
  BL-HRM-30.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Employee: MoHRE absence report), §4 *Employee*, §5.4
  "Absconding", §6 *Consumes* (apps 09 absence case), §7, §10 *Absconding*
- `docs/OPEN-ITEMS.md` L-24

#### Terms
- **Absence case:** a ticket of category *absence* for HR, opened by the vertical's rule after
  N consecutive days without attendance or leave (02: `CON.RUL.08`, BL-CON-15; 09:
  `MPW.RUL.09`, BL-MPW-17). The guard only asks that one is open for the employee. The 09
  case also holds the next salary; that is 09's part (BL-MPW-17).
- **Held line:** a payroll line left out of the SIF, with a reason (BL-HRM-30).
- **Contact attempt:** a record on the employee (date, method, by whom, outcome) made by HR
  while the worker is absent.
- **Absconding step template:** the *absconding* variant of the *cancellation* visa step
  template (`VisaStepTemplate`, spec §8): it starts from the MoHRE absence report and needs no
  worker signature.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `absconded` | error | no | Absent without notice; MoHRE absence report filed; no line is put in a new SIF file while the report is valid (a file already sent is not recalled) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `report_absconding` | active, on_leave, notice | absconded | `employees.manage`, approved under `HRM.ACL.07` | an absence case is open and more than N consecutive calendar days (default 7) have passed since the last day with attendance or approved leave, and the worker cannot be reached: at least two failed contact attempts recorded on different days of the absence (ER Art. 28 ✔ [S1]); MoHRE absence report number and date recorded | payroll lines in runs not yet `submitted` held with reason *absence report* (outside WPS while the report is valid, MR 340/2026 Art. 4 ✔ [S2]; a line in a file already sent stays, and if the bank fails it, it is held); deployments ended; portal and attendance disabled; final settlement drafted (or, from `notice`, the existing one gets reason *absconding* and no second one is drafted); *cancellation* visa case opened from the *absconding* variant of the cancellation step template (§8); event `EmployeeAbsconded` |

| Event | When | Payload |
|---|---|---|
| `EmployeeAbsconded` / `EmployeeReinstated` | `report_absconding` / `reinstate` | employee, report number and date |

Setting: Absence days before `report_absconding` is allowed = 7 consecutive days.
§10: what may lawfully be withheld is `⚠ open: lawyer (L-24)`.

**Split (decided 2026-10-20).** The spec row above is built in two issues. Here (Launch 02): the
guards, approval, *portal and attendance disabled*, *deployments ended* (through the event) and
the event. In BL-HRM-30 (Launch 09): *payroll lines held*, *final settlement drafted* and the
*cancellation* visa case. Launch 02 has no payroll run, settlement or visa case to act on.

#### Edge cases decided
- **"More than N consecutive days":** strictly more than 7 calendar days since the last
  attendance or approved leave day (ER: "more than 7"), Asia/Dubai. The age of the absence
  case does not matter; it only has to be open (it may open on day 3 or day 8).
- **"Cannot be reached":** at least two contact attempts with outcome *no answer* (or
  equivalent), on two different days after the last attended day.
- **From `notice`:** the existing draft settlement is kept and its reason changed to
  `absconding`; no second settlement is drafted.
- **Open payroll lines, settlement amounts:** provided by BL-HRM-30 (lines in runs not yet
  `submitted` only; a line in a sent SIF is left; L-24 decides withholding, do not invent
  forfeiture).
- **Deployments ended, crew membership ended:** via the event; apps end them (02: the worker
  leaves the crew).

#### Must not
- Must not run without the report number and date.

#### Done when
- [ ] Given E `active` with an *absence* ticket open after the last attendance on 2026-11-01
      and today 2026-11-09, contact attempts on 2026-11-04 and 2026-11-06 (no answer), with
      report "MOHRE-777" dated 2026-11-09, when approved, E is `absconded`, portal and
      attendance are disabled, and `EmployeeAbsconded` is emitted with the report number and
      date.
- [ ] On 2026-11-08 (7 days) the request is refused.
- [ ] With only one contact attempt recorded (2026-11-05), it is refused.
- [ ] Given the absence case opened on 2026-11-09 (late) and last attendance 2026-11-01, on
      2026-11-09 the request is allowed (case open, 8 days).
- [ ] Without the report number, it is refused.
- [ ] Without an open *absence* ticket for E, it is refused.

#### Out of scope
`reinstate` (BL-HRM-08); exit (BL-HRM-10). Provided by BL-HRM-30 (Launch 09): the payroll hold
(*absence report*, including a line in a sent SIF that the bank fails), the final settlement
draft and the *cancellation* visa case.

---

### BL-HRM-08 — wl-hr: reinstate an absconded employee (`reinstate`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** BL-HRM-04 · BL-HRM-07 · BL-HRM-18 · BL-HRM-24 · BL-HRM-30 · **Blocks:** BL-MPW-13
**Review items:** X-15, 09-N12-c

#### Goal
When a reported worker returns and the absence report is withdrawn or cancelled, HR reinstates
them to `active`. Held lines are released for payment, and the draft settlement and cancellation
case are cancelled.

#### Surfaces
- **Back office:** HR enters the return date and the withdrawal reference and requests
  **Reinstate**; the payroll officer then sees the proposed off-cycle run for the held
  lines.
- **System:** after approval the settlement and cancellation case are cancelled,
  `HRM.RUL.12` proposes the off-cycle run and `EmployeeReinstated` is emitted.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Employee*, §4 *Final settlement* (`cancel`), §6 *Automations*
  (`HRM.RUL.12`), §10 *Absconding*
- `docs/OPEN-ITEMS.md` AU-22

#### Terms
- **Withdrawal reference:** the authority's reference that the absence report was withdrawn or
  cancelled.
- **Date the worker returned:** a field of the MoHRE absence report on the employee, entered
  with the request; it is how "the worker returned" is recorded.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reinstate` | absconded | active | `employees.manage`, approved under `HRM.ACL.07` | the date the worker returned recorded (not in the future) and the report's withdrawal or cancellation reference recorded (`⚠ open: authority (AU-22)`, §10); *cancellation* visa case not `completed`; final settlement `draft` or `approved` with no payment recorded against it (otherwise refused: the worker is rehired as a new employee) | held lines released through `HRM.RUL.12`; draft or approved final settlement and *cancellation* case `cancel`led; the employee returns to `active` whatever the state before absconding (a notice or leave that was running is not resumed; HR gives notice or records leave again); event `EmployeeReinstated` |

#### Edge cases decided
- **Settlement `approved` (not draft):** cancelled too when nothing was paid against it. If a
  payment is recorded, or it is `paid` or `disputed`: refused (guard); the worker is rehired as
  a new employee.
- **Cancellation case `completed`:** the visa is gone; refused (guard).
- **Previous state:** always `active`, even when the employee absconded from `notice` or
  `on_leave` (spec effect). Notice dates are not restored (they stay in history; the
  settlement that carried them is cancelled); if the notice still applies, HR runs
  `give_notice` again. An approved leave is not resumed; HR records a new leave request.
- **Absence case:** closed by the vertical on `EmployeeReinstated`.

#### Must not
- Must not reinstate without the withdrawal reference.
- Must not release held lines directly: go through `HRM.RUL.12`.

#### Done when
- [ ] Given E `absconded` with a held November line, draft settlement and
      *cancellation* case `open`, when reinstated with reference "WD-55" and return date
      2026-11-20, E is `active`, the settlement and case are `cancelled`, `HRM.RUL.12`
      proposes an off-cycle run for the held line, `EmployeeReinstated` is emitted.
- [ ] Without the reference, it is refused; without the return date, or with a return date in
      the future, it is refused.
- [ ] Given E absconded from `notice` (last working day 2026-11-30), after reinstatement E is
      `active` with no last working day.
- [ ] With the cancellation case `completed`, it is refused.
- [ ] Given the settlement `approved` (nothing paid), it is `cancelled` on reinstatement; given
      an AED 500 payment recorded against it, reinstatement is refused.

#### Out of scope
Off-cycle run (BL-HRM-18).

---

### BL-HRM-09 — wl-hr: record an employee's death (`record_death`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** BL-HRM-04 · **Blocks:** BL-HRM-10, BL-MPW-13
**Review items:** X-15

#### Goal
When an employee dies in service, HR records it with the death certificate. Every message to the
employee stops, open payroll lines are held and the salary owed up to the date of death moves
into the final settlement (so it is paid once), deployments end, the final settlement is
drafted with the estate as payee, a cancellation case opens and an HR task list starts.

#### Surfaces
- **Back office:** HR attaches the death certificate and requests **Record death**; HR then
  sees the three tasks (visa cancellation, insurance claim, repatriation) and records the
  estate's representative on the settlement.
- **System:** after approval lines are held and moved into the settlement, unstarted leave
  is cancelled, every pending message to the employee is cancelled and `EmployeeDeceased` is
  emitted.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Employee: date of death, estate's representative; Final
  settlement: payee), §4 *Employee*, §5.4 "Death", §10 *Death in service*
- `docs/OPEN-ITEMS.md` L-25

#### Terms
- **Estate's representative:** the person or body receiving the settlement; who that is is open
  (L-25).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `deceased` | neutral | no | Died in service; settlement due to the estate |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `record_death` | active, on_leave, notice, absconded | deceased | `employees.manage`, approved under `HRM.ACL.07` | date of death and death certificate (or the authority's notice) attached | payroll lines in runs not yet `submitted` held with reason *death* and never released by `HRM.RUL.12`: the salary they owe up to the date of death is moved into the final settlement as unpaid salary, and the line is marked *settled in the final settlement*; deployments ended; portal disabled and every message to the employee stopped; leave requests `requested` or `approved` that have not started cancelled by the system; final settlement drafted with payee *estate*; *cancellation* visa case opened; HR task list (visa cancellation, insurance claim, repatriation) opened; event `EmployeeDeceased` |

`pay` (settlement) guard, part: "to the payee (for payee *estate*: the estate's representative
and the document giving the right recorded, `⚠ open: lawyer (L-25)`)".

#### Edge cases decided
- **Pay up to the date of death:** the current run's line is held whole with reason *death*;
  salary up to the date of death goes into the final settlement as unpaid salary and the line
  is marked *settled in the final settlement*, so `HRM.RUL.12` never releases it (paid once).
- **Who receives the estate's payment:** the settlement keeps payee *estate*; HR records the
  representative (name, document giving the right, account) when known. `pay` is refused until
  it is recorded (L-25 decides which documents are enough; the build only requires one).
- **Existing settlement** (from `notice` or `absconded`): its payee changes to *estate* and reason
  to `death`; no second one.
- **Messages stopped:** every pending and future notification with the employee as recipient is
  cancelled, including payslip and pay-account SMS.
- **Future leave requests:** `requested`/`approved` requests not yet started
  are cancelled by system.
- **Date of death in the past** (late report): allowed; lines already paid stay.

#### Must not
- Must not send any further message to the employee.
- Must not request a signature from the estate (spec §4 *Final settlement*, `pay` effect:
  "not for the estate"; built in the base `pay` and BL-HRM-23).

#### Done when
- [ ] Given E `active`, death on 2026-11-10 with certificate, when approved, E is `deceased`,
      November line held, settlement drafted with payee *estate* including unpaid salary
      1–10 November, *cancellation* case opened, three HR tasks, `EmployeeDeceased` emitted.
- [ ] A payslip notification queued for E is cancelled.
- [ ] E's November line is marked *settled in the final settlement*; no off-cycle run is
      proposed for it.
- [ ] `pay` of E's settlement without a representative recorded is refused.
- [ ] Without the certificate or authority notice, it is refused.

#### Out of scope
Exit (BL-HRM-10).

---

### BL-HRM-10 — wl-hr: exit an absconded or deceased employee

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `blocked:open-item` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** BL-HRM-07 · BL-HRM-09 · BL-HRM-30 · **Blocks:** None
**Review items:** X-15, 09-N12-c

#### Goal
An absconded worker never signs, and a deceased one cannot. They exit without the standard
guards: with the cancellation done (or the authority's reference), and the settlement `approved`
(absconded) or `paid` to the estate (deceased).

#### Surfaces
- **Back office:** HR runs **Exit** on an absconded or deceased employee once the
  cancellation (or the authority's reference) and the settlement allow it.
- **Portal:** the employee portal turns read-only with the exit banner, then closes after 60
  days.
- **System:** the portal closes 60 days after exit; `EmployeeExited` carries the exit route.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Employee* (both `exit` rows), §5.4, §6 *Events emitted*
- `docs/OPEN-ITEMS.md` L-24, L-25

#### Terms
- **Exit route:** standard, onboarding cancelled, absconding, death.
- **TIM rule:** `POR.ACL.01` (wl-portal; formerly `MPW.ACL.11`): after exit the employee portal
  is read-only, then closes after 60 days (setting "Employee portal after exit").

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `exit` | notice | exited | `employees.manage` | final settlement `paid` or `closed`; *cancellation* visa case `completed` when sponsored | portal becomes read-only, then closes (TIM rule); event `EmployeeExited` with exit route *standard* |
| `exit` | absconded, deceased | exited | `employees.manage` | *cancellation* visa case `completed`, or the authority's cancellation reference recorded; final settlement `approved`, `paid` or `closed` (absconded; `disputed` blocks the exit) or `paid` or `closed` to the estate (deceased); no signed acknowledgement needed | portal becomes read-only, then closes (TIM rule); event `EmployeeExited` with exit route *absconding* or *death* |

| Event | When | Payload |
|---|---|---|
| `EmployeeExited` | `exit` | employee, exit date, exit route (standard, absconding, death) |

§5.4: "Exit does not wait for the worker's signature or for payment; the approved settlement
stays as a liability until it is paid as legal advice says (`⚠ open: lawyer (L-24)`)."

#### Edge cases decided
- **Settlement `disputed` or `paid` for an absconded worker:** `paid` or `closed` also satisfy
  the guard; `disputed` blocks exit.
- **`EmployeeExited` payload:** employee, exit date, exit route (standard, absconding, death);
  the standard `exit` from `notice` emits it too, with route *standard*.

#### Must not
- Must not require a signed acknowledgement.
- Must not require payment for absconded workers.

#### Done when
- [ ] Given E `absconded`, cancellation case `completed`, settlement `approved`, `exit` makes E
      `exited` and `EmployeeExited` has route `absconding`.
- [ ] Given E `deceased` and settlement `approved` (not paid), `exit` is refused; after `paid`, it
      succeeds with route `death`.
- [ ] Given E `absconded`, settlement `approved`, no completed case but the authority's
      cancellation reference recorded, it succeeds.
- [ ] Given E exited on 2026-11-10, the portal is read-only at once and closes 60 days later,
      on 2027-01-09.
- [ ] Given E absconded with the settlement `disputed`, `exit` is refused.

#### Out of scope
Standard exit (base).

---

### BL-HRM-11 — wl-hr: return a payroll run to draft (`return`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** Base: wl-hr Payroll run lifecycle · **Blocks:** None
**Review items:** X-15

#### Goal
The approver can send a `calculated` or `approved` run back to `draft` with a reason and comments
on the lines to fix. The approval is cleared and the preparer is notified.

#### Surfaces
- **Back office:** the approver opens the run, comments on the lines to fix and runs
  **Return** with a reason; the preparer sees the run back in `draft` with the comments on
  the lines.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Payroll run: return reason; Payroll line: approver comment), §4
  *Payroll run*, §5.3 step 4

#### Terms
- **Preparer:** the user with `payroll.prepare` who calculated the run.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `return` | calculated, approved | draft | `payroll.approve` | returner ≠ the preparer; reason given; at least one comment on a line to fix | approval cleared; preparer notified with the reason and comments; event `PayrollRunReturned` |
| `resubmit` | rejected | submitted | `payroll.prepare` | cause fixed: a note on the fix entered (every reason); for reason *establishment ID*, the legal entity's MoHRE establishment ID differs from the one in the rejected SIF version; amounts unchanged (`return` is not possible once the file was sent: an amount error is corrected after payment by a supplementary run, §5.6) | new SIF version generated for the same lines; lines reset from `failed` to `included`; the previous file kept |

| Event | When | Payload |
|---|---|---|
| `PayrollRunReturned` | `return` | run, reason |

#### Edge cases decided
- **"Comments on the lines to fix":** at least one line comment.
- **Who may return:** any `payroll.approve` user except the preparer.
- **From `rejected`:** not allowed. Once a file was sent, an amount error is corrected after
  payment by a supplementary run (`resubmit` guard).
- **Lines kept:** lines stay; the next `calculate` recalculates them.

#### Must not
- Must not delete lines or SIF files.
- Must not keep the approver id after return.

#### Done when
- [ ] Given a run `approved` by B, when B returns it with reason "wrong OT" and a comment on E's
      line, the run is `draft`, approver cleared, preparer A notified, `PayrollRunReturned`.
- [ ] Without a line comment, it is refused.
- [ ] From `submitted`, `return` is not offered.

#### Out of scope
None.

---

### BL-HRM-12 — wl-hr: record a rejected WPS file and resubmit

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** Base: wl-hr Payroll run lifecycle (`submit`) · **Blocks:** BL-HRM-13, BL-HRM-14
**Review items:** X-15, 09-N7-b

#### Goal
When the bank or exchange house rejects the whole SIF, the run moves to `rejected`, every line is
marked `failed`, and the preparer fixes the cause and resubmits a new SIF version for the same
lines. WPS deadline alerts keep running.

#### Surfaces
- **Back office:** the preparer runs **Record rejection** on the submitted run with the
  bank's notice and reason, then **Resubmit** with a note on the fix; earlier SIF versions
  stay listed on the run.
- **System:** the WPS deadline alerts (`HRM.RUL.03`) keep running for the rejected run.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Payroll run: SIF files; Payroll line: WPS result), §4 *Payroll
  run*, §5.3 step 6, §6 *Automations* (`HRM.RUL.03`)

#### Terms
- **SIF:** Salary Information File for WPS, one per legal entity, versioned.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `rejected` | error | no | The bank or exchange house rejected the whole file |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `record_rejection` | submitted | rejected | `payroll.prepare` | the bank's rejection notice attached; reason (file format, establishment ID, funding, other) | every line marked `failed`; preparer and approver notified (`HRM.RUL.11`); the WPS deadline alerts (`HRM.RUL.03`) keep running; event `PayrollLinesFailed` |
| `resubmit` | rejected | submitted | `payroll.prepare` | cause fixed: a note on the fix entered (every reason); for reason *establishment ID*, the legal entity's MoHRE establishment ID differs from the one in the rejected SIF version; amounts unchanged (`return` is not possible once the file was sent: an amount error is corrected after payment by a supplementary run, §5.6) | new SIF version generated for the same lines; lines reset from `failed` to `included`; the previous file kept |

| Event | When | Payload |
|---|---|---|
| `PayrollLinesFailed` | `record_rejection`, `record_partial` | run, failed lines with reasons |

#### Edge cases decided
- **"Cause fixed":** the preparer enters a note on `resubmit` (required
  for every reason). For reason `establishment ID` the system also compares the legal entity's
  MoHRE establishment ID now with the ID stored in the rejected SIF version (the SIF is named
  with it, `HRM.WPS.01`); if they are equal, `resubmit` is refused. Other reasons (file format,
  funding, other) are not checked by the system.
- **Lines on resubmit:** exactly the lines of the rejected version: none added, none removed.
  A line that must not be paid is not removed here: the run is paid as resubmitted and the
  error corrected by a supplementary run (BL-HRM-17).
- **"Amounts unchanged":** every line's net equals the rejected version's; otherwise refused.
- **Lines marked `failed` on resubmit:** reset to `included` (effect).
- **`PayrollLinesFailed`** emitted on `record_rejection` too (effect).
- **An amount that must change:** not on resubmit; corrected after payment by a supplementary run
  (BL-HRM-17).

#### Must not
- Must not overwrite or delete the previous SIF version.
- Must not allow amount changes on resubmit.

#### Done when
- [ ] Given a run `submitted` with SIF v1, when A records rejection "funding" with the notice,
      the run is `rejected`, all lines `failed`, `PayrollLinesFailed` emitted, preparer and
      approver notified.
- [ ] When A resubmits, SIF v2 exists, v1 is kept, lines are `included`, run `submitted`.
- [ ] If a line's net changed, resubmit is refused.
- [ ] Without the note, resubmit is refused.
- [ ] Given rejection reason `establishment ID` with v1 named "EST-1001", resubmit is refused
      while the entity's ID is still "EST-1001"; after it is corrected to "EST-1010", v2 is
      generated named with "EST-1010".
- [ ] Without the notice, `record_rejection` is refused.

#### Out of scope
Partial failure (BL-HRM-13); escalation (BL-HRM-14).

---

### BL-HRM-13 — wl-hr: record a partly failed WPS file and regenerate

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:portal` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** BL-HRM-12 · BL-HRM-18 (held lines, off-cycle) · BL-HRM-20 (account `active` at submit) · BL-HRM-21 (`waive_cooling`) · **Blocks:** BL-HRM-14
**Review items:** X-15, 09-N7-b

#### Goal
When some lines are paid and others fail, paid lines are confirmed and their payslips published;
each failed line gets a task. Each failed line is then fixed (usually the pay account) or moved
out (held, or paid another approved way), and a SIF with only the fixed lines is generated. The
run reaches `paid` when no line is left `failed`.

#### Surfaces
- **Back office:** the preparer attaches the bank's result file and marks failed lines with
  reasons, fixes or holds each failed line, runs **Regenerate**, and the approver runs
  **Confirm paid**.
- **Portal:** employees whose lines were paid see their payslips in the employee portal.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Payroll run*, §4 *Pay account* (last paragraph), §5.3 steps 6–7

#### Terms
- **Moved out:** the line is held (paid later off-cycle) or marked paid by an approved non-WPS
  method.
- **Bank confirmation:** the bank's result file for a SIF version. The file attached at
  `record_partial` is the confirmation for the lines it shows paid, so their payslips go out
  then; `confirm_paid` needs a confirmation only for the lines of later SIF versions.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `partially_rejected` | warning | no | Some lines paid; others failed with the bank's reason |
| `paid` | success | yes | Payment confirmed for every line not held; payslips published |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `record_partial` | submitted | partially_rejected | `payroll.prepare` | the bank's result file attached; each failed line has a reason (invalid IBAN or card, routing code, name or labour card mismatch, account closed) | paid lines marked `paid` and their payslips published; failed lines marked `failed`; a task per failed line (`HRM.RUL.11`); event `PayrollLinesFailed` |
| `regenerate` | partially_rejected | submitted | `payroll.prepare` | each failed line fixed (pay account changed under `HRM.ACL.05`) or moved out of the run (held, or paid by an approved non-WPS method) | SIF generated with only the fixed lines, as a new version linked to the run |
| `confirm_paid` | submitted, partially_rejected | paid | `payroll.approve` | bank or exchange-house confirmation attached for every line not held with net > 0 that is in a SIF version (the result file attached at `record_partial` confirms the lines it shows paid; none needed when no SIF was generated); a line moved out to an approved non-WPS method has its payment recorded (`FIN.PAY.01`) instead; no line left `failed` (fix it or hold it) | payslips generated (`DOC.GEN.01`) and published to the employee portal; salary cost posted to cost objects (`FIN.CST.01`); held lines wait for `HRM.RUL.12`; for a supplementary run, overpayment recovery adjustments created for the negative differences; event `PayrollRunPaid` |

#### Edge cases decided
- **Line paid by a non-WPS method:** it needs no bank confirmation; its payment (method,
  reference, date) is recorded through wl-payments (`FIN.PAY.01`) and the line is marked
  `paid` when the payment is recorded.
- **All failed lines moved out:** `regenerate` would produce an empty SIF, so the
  run goes straight to `confirm_paid` from `partially_rejected` (allowed by its *From*).
- **Second partial failure after regenerate:** `record_partial` again from `submitted`; already
  `paid` lines stay paid.
- **Fixed line with a new pay account still cooling:** not fixed; hold it (BL-HRM-20) or waive
  cooling (BL-HRM-21).
- **Payslips:** published once per line; `confirm_paid` does not republish lines already published.
- **Cost posting:** once per line.

#### Must not
- Must not include already-paid lines in the regenerated SIF.
- Must not reach `paid` with a `failed` line.

#### Done when
- [ ] Given 50 lines, when the result file shows 47 paid and 3 failed (invalid IBAN), the run is
      `partially_rejected`, 47 payslips published, 3 lines `failed`, 3 tasks.
- [ ] After 2 lines get approved active accounts and 1 is held, `regenerate` builds SIF v2 with
      2 lines.
- [ ] `confirm_paid` with confirmation for the 2 lines makes the run `paid`; the held line waits.
- [ ] `confirm_paid` with a `failed` line left is refused.
- [ ] Given 1 failed line instead marked paid by an approved transfer with reference "TR-88",
      `confirm_paid` does not ask for a bank confirmation for it.

#### Out of scope
Pay account change (BL-HRM-19 to BL-HRM-21); off-cycle (BL-HRM-18).

---

### BL-HRM-14 — wl-hr: escalate WPS rejections before the due date (`HRM.RUL.11`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** BL-HRM-12 · BL-HRM-13 · **Blocks:** BL-MPW-21
**Review items:** X-15, 09-N7-b

#### Goal
A rejected or partly rejected run gives a task per failed line and tells the approver. If it is
still not resubmitted N days (default 1) before the WPS due date, the MD and accountant are told.

#### Surfaces
- **Back office:** the preparer sees one task per failed line under "Needs your action".
- **System:** at 00:00 Asia/Dubai N days before the WPS due date a run still `rejected` or
  `partially_rejected` is escalated once to the MD and accountant.

#### Files to read
- `docs/modules/wl-hr.md` §6 *Automations* (`HRM.RUL.03`, `HRM.RUL.11`), §6 `PayrollCalendar`,
  §7, §10 *WPS*

#### Terms
- **WPS due date:** the 1st of the following month (MR 340/2026), from `PayrollCalendar`.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.11` | run `rejected` or `partially_rejected` | — | task per failed line for the preparer; notify the approver; if the run is still `rejected` (not resubmitted) or `partially_rejected` (not regenerated or confirmed paid) at 00:00 Asia/Dubai N days before the WPS due date, notify the MD and accountant (at once when the rejection is recorded after that point) | N = 1 |
| `HRM.RUL.03` | N days before the WPS due date (1st of the following month, MR 340/2026), and on day 2 and day 5 after it | payroll run not `paid` | notify the MD and accountant; from day 5, warn that new work permits are suspended | N = 3 |

#### Edge cases decided
- **"Not resubmitted":** run still `rejected` (no `resubmit`) or still `partially_rejected` (no
  `regenerate` and no `confirm_paid`) at 00:00 Asia/Dubai on the day N days before the due
  date. A run back in `submitted` or `paid` is not escalated.
- **Rejection after that point:** escalate at once.
- **Idempotency:** tasks per line once per rejection event; escalation once per run.

#### Must not
- Must not stop `HRM.RUL.03`.

#### Done when
- [ ] Given a November run `rejected` on 2026-11-28, the preparer has one task per line and the
      approver is notified.
- [ ] If still `rejected` on 2026-11-30 (due 2026-12-01), MD and accountant are notified once.
- [ ] If resubmitted on 2026-11-29, no escalation.
- [ ] Given a November run `partially_rejected` on 2026-11-28: if regenerated (or confirmed
      paid with the failed line held) on 2026-11-29, no escalation; if still
      `partially_rejected` on 2026-11-30, MD and accountant are notified once.

#### Out of scope
`HRM.RUL.03` itself (base).

---

### BL-HRM-15 — wl-hr: keep salary recoveries within the Art. 25 caps (`HRM.ACL.06`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** Base: wl-hr Adjustments and payroll calculation · **Blocks:** BL-HRM-16, BL-HRM-17, BL-HRM-23, BL-MPW-21
**Review items:** X-15, 09-N6-b

#### Goal
Any recovery from salary (overpayment, advance, damage, settlement offset, negative timesheet
difference) stays within the legal caps per type and 50% in total per payroll line. A line above
the caps is refused, not trimmed silently, and no approval can exceed the caps.

#### Surfaces
- **Back office:** the preparer sees a line above the Art. 25 caps flagged as an exception
  on the run; **Approve** stays disabled while any line exceeds the caps, for every
  approver.
- **System:** the cap check runs on every recovery per type and in total, with no trimming.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Adjustment), §5.3 step 3, §6 *Access rules* (`HRM.ACL.06`), §7
  (maximum total deductions), §10 *Corrections and recoveries*

#### Terms
- **Wage:** the basis for caps: the line's gross fixed wage for the period (basic plus fixed
  allowances).
- **A day's wage:** the monthly gross fixed wage ÷ 30 (spec §7), for the damage cap.
- **Instalment plan:** an adjustment spread over several runs.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Generalises | Pattern | Default |
|---|---|---|---|---|
| `HRM.ACL.06` | A salary recovery (overpayment from a supplementary run, advance, damage, settlement offset) stays within the Art. 25 caps per type and 50% in total per payroll line, computed on the line's gross fixed wage for the period (a settlement offset: on the settlement's dues); a line above the caps is refused, not trimmed silently, and no approval can exceed the legal caps | new (X-15) | `LIM` | caps per §7 |

Setting: Maximum total deductions = 50% of wage (Art. 25); per type: overpayments 20%, fines 5%,
court debts 25%, damage 5 days' wage a month; advances: the 50% total only. Wage = the line's
gross fixed wage for the period (basic plus fixed allowances); a day's wage = the monthly
gross fixed wage ÷ 30.

#### Edge cases decided
- **What "refused" means for a calculated line:** the line is an exception; `approve` of the run
  is refused while any line exceeds the caps. Instalment plans are sized so each instalment fits.
- **Advances:** no per-type cap; only the 50% total applies (§7).
- **Order when the total exceeds 50%:** not decided; the preparer reschedules an instalment. No
  automatic trimming.
- **Rounding:** caps computed in AED to 2 decimals, rounded down.

#### Must not
- Must not trim a deduction silently.
- Must not offer an override or approval above the caps.

#### Done when
- [ ] Given wage AED 3,000 and an overpayment recovery of AED 700 this month, the line is
      flagged (cap AED 600) and run `approve` is refused.
- [ ] Given wage AED 3,000, overpayment recovery AED 600, court debt AED 750, fine AED 150 and
      advance recovery AED 100 (each within its type cap), total AED 1,600 exceeds 50%
      (AED 1,500) and approval is refused.
- [ ] Given wage AED 3,000 (a day's wage AED 100), a damage recovery of AED 600 is flagged
      (cap AED 500); AED 500 passes.
- [ ] An approver with every permission cannot approve it.

#### Out of scope
Settlement offsets beyond caps (BL-HRM-23, L-23).

---

### BL-HRM-16 — wl-hr: calculate payroll with provisional hours and timesheet corrections

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** BL-ATT-08 · BL-ATT-12 · BL-ATT-13 · BL-HRM-15 · **Blocks:** None
**Review items:** X-15, X-16, 09-N5-c, 09-N4-a

#### Goal
`calculate` reads hours through `hours_for_payroll` (approved hours, plus provisional hours for
timesheets not yet approved), shows provisional hours as such, and adds lines for approved
adjustment timesheets that affect payroll and for reconciliation differences. + hours are paid;
− hours are recovered within the Art. 25 caps. `submit` marks provisional timesheets.

#### Surfaces
- **Back office:** the preparer sees provisional hours marked on each line, *missing
  timesheet* exceptions and the difference lines from adjustments and reconciliations on the
  run.
- **System:** `submit` locks the timesheets used and sets `paid_provisionally_by`; each
  difference is linked to the run that used it.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Payroll run* (`calculate`, `submit`), §5.3 step 2, §6 *Consumes*
  (wl-attendance)
- `docs/modules/wl-attendance.md` §5.7, "Correcting a locked timesheet" (affects table)

#### Terms
- **Difference line:** a component on the next regular run for an adjustment or reconciliation
  delta.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `calculate` | draft, calculated | calculated | `payroll.prepare` | hours for the period from `Timesheets::hours_for_payroll(period)` (approved hours, and accepted attendance hours marked *provisional* for timesheets not yet approved, per the wl-attendance cut-off policy); an employee with no hours for the period does not block the run: the line is listed as an exception (missing timesheet, §5.3 step 3) | lines recalculated for the run's legal entity only (`freelance` employees excluded) from salary structures, timesheets (`wl-attendance`; provisional hours shown as such on the line), approved adjustment timesheets affecting payroll, reconciliation differences (`TimesheetReconciliationNeeded`), leave, adjustments and expense reimbursements; lines of employees with an open absence case marked *held* |
| `submit` | approved | submitted | `payroll.prepare` | every paid employee has a WPS-eligible pay account (or an approved non-WPS method) | SIF file generated per entity, named with its MoHRE establishment ID (`HRM.WPS.01`); held lines and lines with net ≤ 0 left out (no SIF when no line is left, e.g. a supplementary run with only negative differences); approved timesheets used are `lock`ed; timesheets paid on provisional hours get `paid_provisionally_by` = this run |

§5.3 step 2 (part): "Approved adjustment timesheets that affect payroll and reconciliation
differences (`TimesheetReconciliationNeeded`) enter the next regular run of the employee's
entity after approval: + hours paid, − hours recovered as a deduction within the Art. 25 limits
(`HRM.ACL.06`), both at the rates of the salary structure effective on the date of the hours (not
today's). Each difference is linked to the run that paid or recovered it and is used once. A
correction to a run already `paid` that cannot wait for the next run goes through a
supplementary run (§5.6)."

#### Edge cases decided
- **Rate for difference hours:** the overtime and hourly rates of the salary structure effective
  on the hours' date, not today's.
- **Each delta used once:** a delta is linked to the run that paid or recovered it; a second
  `calculate` in another run skips it. The adjustment timesheet then `lock`s with `paid_by`.
- **Negative delta above the cap:** no automatic plan and no trimming (as BL-HRM-15): the
  difference line is created in full, the line is flagged as an exception and `approve` is
  refused; the preparer turns the excess into an *overpayment recovery* adjustment with an
  instalment plan whose instalments fit the caps, then recalculates.
- **"Listed exceptions":** employees with no hours from `hours_for_payroll` do not block
  `calculate`; their lines are listed as *missing timesheet* exceptions for the preparer's
  review (spec §5.3 step 3).
- **Which run:** the next `regular` run of the employee's entity after approval.
- **Source of a reconciliation difference:** only `TimesheetReconciliationNeeded`. The system
  reconciliation adjustment emits no `TimesheetApproved` (wl-attendance §4), so it is not
  read a second time as an adjustment timesheet. A timesheet paid provisionally is `locked`
  at its approval, with `paid_provisionally_by` kept as the consumer.

#### Must not
- Must not edit a paid run.
- Must not pay a delta twice.

#### Done when
- [ ] Given T provisional with 160 h, the November line shows 160 h marked provisional; on
      `submit`, T gets `paid_provisionally_by` = the November run.
- [ ] Given employee G with no timesheet, `calculate` succeeds and G's line is listed as a
      *missing timesheet* exception.
- [ ] Given wage AED 3,000 and a −50 h reconciliation at AED 15/h (−AED 750, above the AED 600
      overpayment cap), the line is flagged and `approve` refused; after the preparer converts
      it to a recovery of AED 600 now and AED 150 next month, `approve` succeeds.
- [ ] Given a −8 h reconciliation at AED 15/h, December's line has −AED 120 within caps.
- [ ] Given an approved `payroll` adjustment +4 h overtime at 125% of AED 15/h, December pays
      AED 75.
- [ ] Recalculating December twice does not double the line; the delta is linked to the run.
- [ ] Given a `billing`-only adjustment, payroll ignores it.

#### Out of scope
Supplementary runs (BL-HRM-17).

---

### BL-HRM-17 — wl-hr: correct a paid run with a supplementary run

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** BL-HRM-15 · BL-HRM-18 · **Blocks:** None
**Review items:** X-15, 09-N6-b

#### Goal
An error found after `paid` is corrected by a supplementary run linked to the paid run,
which is never edited. It holds only the affected employees and the difference per
component. Positive differences are paid in this run; negative ones become overpayment
recovery adjustments over the next regular runs, within 20% a month and 50% total. Corrected
payslips are published and the cost difference posted.

#### Surfaces
- **Back office:** the preparer creates a supplementary run from the paid run with a reason;
  it lists only the affected employees and differences; another user approves it.
- **Portal:** employees with a corrected line see the corrected payslip in the employee
  portal.
- **System:** at **Confirm paid** recovery adjustments, settlement recoveries or receivables
  are created and the cost differences posted.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Payroll run: type, corrects; Adjustment), §4 "Run types", §5.6
  step 1, §6 *Access rules* (`HRM.ACL.04`, `HRM.ACL.06`)

#### Terms
- **Supplementary run:** run type `supplementary` with `corrects` = the paid run.

#### Spec excerpt (authoritative for this issue)

"`supplementary`: a correction of a `paid` run, which is never edited. Created by
`payroll.prepare` from the paid run with a reason; it holds only the affected employees and
the difference per component. A positive difference is paid in this run, through its own
SIF; only a line held at its `submit` (e.g. no usable pay account) is paid later in an
off-cycle run (`HRM.RUL.12`). A negative difference becomes an *overpayment recovery*
adjustment spread over the next regular runs, at most 20% of the wage a month and within the
50% total (Art. 25 ✔ [S1]; `HRM.ACL.06`). Corrected payslips are published and the cost
difference is posted to the same cost objects. Approval is by another user (`HRM.ACL.04`).
Only positive lines go to the bank: a run with only negative differences generates no SIF at
`submit` and needs no bank confirmation at `confirm_paid`. The recovery adjustments are
created at `confirm_paid`, when the correction is final. A paid run may get several
supplementary runs; each difference is against the paid run plus the earlier corrections,
and `corrects` always names the original run. For an employee who has left since, a negative
difference is added to the final settlement's recoveries while it is not yet `paid`,
otherwise it becomes a receivable from the employee as for a negative net."

| ID | Rule | Generalises | Pattern | Default |
|---|---|---|---|---|
| `HRM.ACL.04` | A payroll run or a final settlement is prepared by one user and approved by another | `MPW.ACL.05`, `MPW.ACL.12` | `SOD` | on |

#### Edge cases decided
- **A run with only negative lines:** nothing to send to the bank: `submit` generates no SIF and
  `confirm_paid` needs no bank confirmation for lines with net ≤ 0; the adjustments are created
  at `confirm_paid`.
- **When adjustments are created:** at `confirm_paid` (the correction is final), not at approve.
- **Several supplementary runs on one paid run:** allowed; each difference is against the paid
  run plus earlier supplementary runs.
- **Correcting a supplementary run:** `corrects` always points to the original regular run.
- **Employee exited since:** negative difference goes to the final settlement as a recovery if
  not yet paid; else a receivable.
- **Where a positive difference is paid:** in this supplementary run, through its own SIF.
  Only a line held at its `submit` (e.g. no usable pay account) goes to an off-cycle run
  (`HRM.RUL.12`).
- **Cost posting:** at `confirm_paid`, as for any run, to the paid run's cost objects.

#### Must not
- Must not change the paid run's lines, totals or payslips.
- Must not recover more than 20% a month.

#### Done when
- [ ] Given October run R `paid` with E overpaid AED 900 (wage AED 3,000) and F underpaid AED 200,
      A creates a supplementary run with reason; it has two lines: E −900, F +200; R unchanged.
- [ ] B approves (A cannot); SIF has only F's AED 200; on `confirm_paid` F's corrected payslip is
      published and E has an overpayment recovery of AED 900 at AED 600/month over two runs.
- [ ] Cost differences −900 and +200 are posted to R's cost objects.
- [ ] Given a supplementary run with only E's −900 line, `submit` generates no SIF and
      `confirm_paid` succeeds without a bank confirmation, creating E's recovery.
- [ ] Given G exited with settlement `approved` and a −300 difference, on `confirm_paid` the
      AED 300 is added to G's settlement recoveries; given G's settlement `paid`, a receivable
      of AED 300 is created on G's party instead.

#### Out of scope
Off-cycle mechanics (BL-HRM-18).

---

### BL-HRM-18 — wl-hr: release held lines through an off-cycle run (`HRM.RUL.12`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** Base: wl-hr Payroll run lifecycle · Vertical 09: absence case closed (`MPW.RUL.09`) · **Blocks:** BL-HRM-08, BL-HRM-13, BL-HRM-17, BL-HRM-20, BL-MPW-17
**Review items:** X-15, 09-N3-d

#### Goal
Held lines were never released. When a hold reason clears (absence case closed, report
withdrawn, line fixed), `HRM.RUL.12` proposes an off-cycle run, paid before the WPS due date
where possible. Carrying a line into the next regular run needs a reason and warns when it pays
late; lines held for a valid absence report carry no warning.

#### Surfaces
- **Back office:** the payroll officer finds a draft off-cycle run with the released lines;
  carrying a line into the next regular run asks for a reason and shows the late-payment
  warning.
- **System:** `HRM.RUL.12` watches cleared hold reasons and each cut-off and proposes the
  off-cycle run.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Payroll line: hold, released into), §4 "Run types", §5.6 step 2,
  §6 *Automations* (`HRM.RUL.12`), §7 (released held lines)

#### Terms
- **Off-cycle run:** run type `off_cycle` with its own SIF and the same approval.
- **Released into:** the run that finally pays the held line.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.12` | hold reason cleared (absence case closed, report withdrawn, line fixed), and at each cut-off for lines still held | payroll line held in a `paid` run | propose an off-cycle run for the released lines; when carried into the next regular run, require a reason and warn if payment falls after the WPS due date of its period | — |

"`off_cycle`: pays held lines released after the regular run (`HRM.RUL.12`), lines that failed
and were moved out, and positive differences of a supplementary run that were held at its
`submit`. It has its own SIF file and the same approval."

§5.6 step 2: "… Lines held because of a valid absence report are outside WPS (MR 340/2026 Art. 4
✔ [S2]) and carry no warning. When the reason clears before its run is `paid`, the line is un-held
in that run if the run is not yet `submitted`; otherwise it waits until the run is `paid` and
`HRM.RUL.12` applies."

#### Edge cases decided
- **Reason cleared before the run is `paid`:** un-hold the line in that run
  if it is not yet `submitted`; otherwise wait until `paid`.
- **"Propose":** creates a `draft` off-cycle run with the released lines and notifies the payroll
  officer; does not calculate or submit.
- **Several released lines:** one draft off-cycle run per entity collects them until calculated.
- **Line amount:** the held line's amount as calculated; changes need a supplementary run.
- **Each line released once:** `released into` set; cannot be added to two runs.
- **Absence case closed:** consumed from the vertical app (09, `MPW.RUL.09`); wl-hr only reads
  open or closed. Until 09 exists, a test double emits it.

#### Must not
- Must not edit the paid run.
- Must not warn for absence-report holds.

#### Done when
- [ ] Given E's November line held in run R (`paid`) and the absence case closed on 2026-12-03, a
      draft off-cycle run with E's line is proposed.
- [ ] Carrying E's line into the December regular run asks for a reason and warns "pays after the
      WPS due date 2026-12-01" for a closed-account hold, but not for an absence-report hold.
- [ ] After the off-cycle run is paid, E's line has `released into` = that run; it cannot be
      added again.

#### Out of scope
Supplementary positive differences (BL-HRM-17).

---

### BL-HRM-19 — wl-hr: request and approve a new pay account with SMS notice

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:portal` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** Base: wl-hr Employee records · Epic wl-notify: SMS · **Blocks:** BL-HRM-20, BL-HRM-21, BL-MPW-21
**Review items:** X-15, 09-N11-b

#### Goal
A change of pay account is the classic payroll fraud. A new account is requested by HR or the
employee, checked for format and holder name, approved by a different user, and the worker is told
by SMS on the number on record before the request, naming the last 4 digits and the date it takes
effect.

#### Surfaces
- **Back office:** HR enters a new IBAN or card on the employee (with a name mismatch
  explanation when needed); another user sees the request with the explanation and approves
  or rejects it.
- **Portal:** the employee requests a new pay account in the employee portal.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Pay account), §4 *Pay account*, §5.7, §6 *Access rules*
  (`HRM.ACL.05`), §7

#### Terms
- **Number on record:** the employee's mobile number as it was before the request; a number
  changed in the same request is never used.
- **Cooling:** waiting period after approval (default 5 days) before payroll uses the account.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `pending_approval` | warning | no | Requested by HR or by the employee in the portal |
| `cooling` | info | no | Approved; the worker has been told; not yet used by payroll |
| `rejected` | neutral | yes | Not approved, or stopped by the worker |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `request` | — | pending_approval | `employees.manage`, or the employee (portal) | IBAN or card format and routing code valid; account holder name matches the employee, or the mismatch explained; no other request of the employee `pending_approval` or `cooling` (the same flow applies to a new employee's first account) | approver notified |
| `approve` | pending_approval | cooling | `pay_accounts.approve` (`HRM.ACL.05`) | approver ≠ requester | SMS (and e-mail where known) to the worker's mobile number on record before the request, naming the last 4 digits and the date it takes effect (no mobile number on record: handled as an undelivered notice, below); cooling ends after N days (default 5) |
| `reject` | pending_approval | rejected | `pay_accounts.approve` | reason given | requester notified |

Spec §4 *Pay account*: "When the change notice gives up (`MessageGaveUp` from wl-notify), or
there is no mobile number on record at `approve`, the account records *notice undelivered* and
HR gets a task to reach the worker." §3 Pay account fields include "name mismatch explanation
(text, optional document)".

| ID | Rule | Generalises | Pattern | Default |
|---|---|---|---|---|
| `HRM.ACL.05` | A new pay account is approved by a user other than the requester, the worker is told by SMS on the number on record, and payroll uses it only after the cooling period; waiving the cooling needs MD approval and a reason | new (X-15) | `SOD`, `TIM` | 5 days |

#### Edge cases decided
- **Second request while one is pending or cooling:** refused until the
  first is rejected, stopped or active.
- **First account of a new employee:** the same flow applies.
- **Employee requests in the portal:** the requester is the employee; any HR user with
  `pay_accounts.approve` may approve.
- **"Date it takes effect":** cooling end date (approval + 5 days, Asia/Dubai).
- **IBAN shown:** sensitive (`SEN`); only last 4 digits in messages.
- **Format checks** (Default, spec silent on the detail): IBAN: `AE` + 2 check digits + 19
  digits (23 characters), ISO 13616 mod-97 check passes. Salary card: digits only, length per
  the exchange house's `SalaryFileFormat`. Routing code: 9 digits and present in the
  bank or agent list kept with the `SalaryFileFormat`.
- **Name mismatch explanation:** a required text field (optional supporting document) on the
  request when the holder name does not match the employee's party name (case and spacing
  ignored); shown to the approver.
- **No mobile number on record:** `approve` still runs; the notice counts as undelivered, so
  the account stays `cooling` past its end date and HR gets a task (BL-HRM-21).

#### Must not
- Must not send the notice to a number entered with the change.
- Must not let the requester approve.

#### Done when
- [ ] Given HR user A requests a new IBAN for E, the account is `pending_approval`; with an invalid
      IBAN it is refused.
- [ ] A cannot approve; B approves on 2026-11-02; SMS to E's previous number names "…1234" and
      2026-11-07; the account is `cooling`.
- [ ] A name mismatch without explanation is refused; with "name on card is the short form"
      it is accepted and the approver sees it.
- [ ] A card number with letters, or routing code "12345", is refused.
- [ ] Given E has no mobile number on record, B's `approve` makes it `cooling`, no SMS is sent,
      *notice undelivered* is set and HR has a task.
- [ ] B rejects with a reason: `rejected`, A notified.

#### Out of scope
Activation (BL-HRM-20); stop and waive (BL-HRM-21).

---

### BL-HRM-20 — wl-hr: activate a pay account after cooling and use it at submit

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** BL-HRM-19 · BL-HRM-18 · **Blocks:** BL-HRM-13, BL-HRM-21
**Review items:** X-15, 09-N11-b

#### Goal
When cooling ends, the new account becomes `active` and the previous one `retired`. A payroll run
uses the account that is active when the run is submitted. If that account is closed and the new
one is still cooling, the line is held and paid off-cycle.

#### Surfaces
- **Back office:** the payroll officer sees a line held with reason "account closed" when
  the active account is closed and the new one is still cooling.
- **System:** the account activates at 00:00 on the cooling end date unless the notice was
  undelivered; `submit` reads the account active at that moment.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Pay account* (and last paragraph), §6 *Events emitted*

#### Terms
- **Closed account:** the bank reported "account closed" for the active account.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `active` | success | no | Used by the next payroll run submitted |
| `retired` | neutral | yes | Replaced by a newer account |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `activate` | cooling | active | system (cooling ended, or worker reached recorded after it ended) | the change notice was not given up (`MessageGaveUp`), or HR recorded that the worker was reached (by whom, method, date) | previous active account `retired`; event `PayAccountChanged` |
| `retire` | active | retired | system | a newer account activated | — |

"A run uses the account that is `active` when it is submitted. If that account is closed and the
new one is still cooling, the line is held and paid in an off-cycle run."

| Event | When | Payload |
|---|---|---|
| `PayAccountChanged` | pay account `activate`, `waive_cooling` | employee, last 4 digits, approver |

#### Edge cases decided
- **SMS gave up:** the account stays in cooling (BL-HRM-21); `activate` does not run until HR
  records the worker reached (guard).
- **Build split with BL-HRM-21:** this issue builds `activate` with its full guard, reading the
  Pay account fields *notice undelivered* and *worker reached* (spec §3). They are empty by
  default, so the guard passes and this issue does not need BL-HRM-21. BL-HRM-21 sets those
  fields (from `MessageGaveUp` and the HR action) and tests the blocked path.
- **Cooling end time:** 00:00 Asia/Dubai on the end date.
- **Run already `approved`, account activates before submit:** the run uses the new account at
  submit (the account is read at submit).
- **Snapshot:** the SIF stores the account used; later changes do not alter it.

#### Must not
- Must not use a `cooling` account at submit.

#### Done when
- [ ] Given cooling ends 2026-11-07, on that day the new account is `active`, the old `retired`,
      `PayAccountChanged` emitted.
- [ ] A run submitted on 2026-11-06 uses the old account; on 2026-11-07 the new one.
- [ ] Given the old account reported closed and the new still cooling, the line is held with
      reason "account closed".
- [ ] Given *notice undelivered* set by hand on the account (test setup), on the end date it
      stays `cooling`.

#### Out of scope
Stop and waive (BL-HRM-21).

---

### BL-HRM-21 — wl-hr: stop a pay account change, waive cooling, handle undelivered notice

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` `surface:portal` `surface:public` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** BL-HRM-19 · BL-HRM-20 (`activate`) · Epic wl-ticketing: security tickets · Epic wl-notify: `MessageGaveUp` · Epic wl-kernel: approval requests · **Blocks:** BL-HRM-13
**Review items:** X-15, 09-N11-b

#### Goal
The worker can stop a change during cooling from the SMS link or portal: the old account stays,
a security ticket opens and the MD is told. Payroll can waive cooling only with MD approval and a
reason (e.g. a failed WPS line, worker verified in person). If the SMS could not be delivered,
the change stays in cooling and HR gets a task to reach the worker.

#### Surfaces
- **Back office:** the payroll user runs **Waive cooling** with a reason and an MD approves
  it under "Needs your action"; HR sees the task to reach the worker and records *worker
  reached* (by whom, method, date).
- **Portal:** the worker stops the change from the employee portal during cooling.
- **Public page:** the worker opens the signed stop link from the SMS, without login, and
  stops the change; the link works once and expires with the cooling period.
- **System:** `MessageGaveUp` keeps the account in cooling and raises the HR task; a stop
  opens a security ticket.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Pay account*, §5.7 step 3, §6 *Consumes* (wl-notify, wl-ticketing)

#### Terms
- **`MessageGaveUp`:** wl-notify event when a message could not be delivered on any channel.
- **Record worker reached:** an action on a `cooling` account by a user with
  `employees.manage`: by whom, method (call, visit, in person), date. A field update, not a
  transition; audited.
- **MD role:** the company role *MD* (wl-kernel roles); "MD-level approver" means another user
  holding that role.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `stop` | cooling | rejected | the worker (reply link, portal) or `employees.manage` | — | old account stays active; security ticket opened; MD notified |
| `waive_cooling` | cooling | active | `payroll.approve` + MD approval (`HRM.ACL.05`) | reason given (e.g. a failed WPS line, worker verified in person with their Emirates ID); approved through a wl-kernel approval request with one step for the *MD* role; the approver is neither the `payroll.approve` user who asks nor the requester of the change (if the MD is either, another user with the *MD* role approves; with none, the request waits) | as `activate`; audited |

| `activate` | cooling | active | system (cooling ended, or worker reached recorded after it ended) | the change notice was not given up (`MessageGaveUp`), or HR recorded that the worker was reached (by whom, method, date) | previous active account `retired`; event `PayAccountChanged` |

§6 Consumes: "wl-notify | SMS of a pay account change to the worker's known number;
`MessageGaveUp` on that SMS → the change stays in cooling (no `activate`) and HR gets a task to
reach the worker; recording the worker reached lets `activate` run".

§5.7 step 5: "If the SMS cannot be delivered (`MessageGaveUp`), or there is no mobile number
on record, the change stays in cooling past its end date and HR gets a task to reach the
worker by another way. When HR records that the worker was reached (by whom, method, date), the
account activates at once if the cooling period has ended."

§4 *Pay account*: "A user with `employees.manage` then records *worker reached* (by whom,
method, date) on the cooling account; this is a field update, not a transition."

#### Edge cases decided
- **Undelivered notice and cooling end:** stays in `cooling` past the end date until the worker
  is reached. HR records "worker reached" (by whom, method, date) and `activate` then runs at
  once if the end date has passed (`activate` guard).
- **Reply link security:** signed, expiring with the cooling period; works once.
- **MD is the requester of the change:** another MD-level approver needed (`waive_cooling`
  guard).
- **How MD approval works:** C (`payroll.approve`) runs `waive_cooling` with the reason; a
  `HRM.ACL.05` policy turns it into a wl-kernel approval request with one *MD* step; the
  transition runs when an MD approves. Rejected, returned or expired: the account stays
  `cooling`.

#### Must not
- Must not activate an account whose notice was not delivered.
- Must not waive without MD approval and reason.

#### Done when
- [ ] Given a cooling account, when E taps the stop link, it is `rejected`, the old account stays
      active, a security ticket exists, the MD is notified.
- [ ] `waive_cooling` by payroll user C with reason "failed WPS line, verified EID" creates an
      approval request; when MD M approves, the account is `active`; while it is pending, or
      after M rejects, it stays `cooling`.
- [ ] Given M requested the pay account change, M cannot approve the waiver; MD N can.
- [ ] Given `MessageGaveUp` for the SMS, on the cooling end date it stays `cooling` and HR has a
      task; after HR records the worker reached, it activates.
- [ ] Given the worker reached is recorded before the cooling end date, it activates on the end
      date, not earlier.

#### Out of scope
Ticket handling (Epic wl-ticketing).

---

### BL-HRM-22 — wl-hr: dispute and resolve a final settlement

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:back-office` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** Base: wl-hr Final settlement lifecycle · Epic wl-ticketing: labour-case tickets · **Blocks:** BL-HRM-25
**Review items:** X-15, 09-N4-b

#### Goal
When the worker contests the settlement or files a MoHRE complaint or court case, the settlement
moves to `disputed` and a labour-case ticket tracks it. The undisputed amount can still be paid on
time. Resolution is `upheld` (closed when the net is already paid in full, otherwise back to
`approved` to pay the rest) or `revised` (recalculated by the preparer, resolved by another
user to `approved`, the difference paid).

#### Surfaces
- **Back office:** HR runs **Dispute** with source, reference and undisputed amount;
  payments are recorded while `disputed`; the preparer enters revised amounts and another
  user runs **Resolve** with the decision attached.

#### Files to read
- `docs/modules/wl-hr.md` §3 (Final settlement: dispute), §4 *Final settlement*, §5.4 step 6

#### Terms
- **Undisputed amount:** the part both sides accept; entered by the preparer on `dispute`
  (guard).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `disputed` | warning | no | The worker contests the amounts, or a MoHRE complaint or court case is open |
| `paid` | success | no | Paid (or nothing payable); acknowledgement requested |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `dispute` | approved, paid | disputed | `settlements.prepare` | source (worker, MoHRE complaint, court) and reference given; undisputed amount entered | labour-case ticket opened (`wl-ticketing`, category *labour case*) with the hearing or reply dates; HR, accountant and MD notified; the undisputed amount can still be paid on time as a payment against the settlement; event `SettlementDisputed` |
| `resolve` | disputed | approved, closed | `settlements.approve` | outcome (`upheld`, `revised`) and the MoHRE decision, agreement or judgment attached | `revised`: while `disputed`, a `settlements.prepare` user enters the recalculated amounts; `resolve` is then run by a `settlements.approve` user other than the one who entered them (`HRM.ACL.04`); the settlement goes to `approved` with a new statement and the difference is paid with `pay` (a revised net below the amount already paid makes the excess a receivable from the employee, as for a negative net); `upheld`: `closed` when the net is paid in full (or nothing is payable), otherwise `approved` so the rest is paid with `pay`; labour-case ticket closed |

#### Edge cases decided
- **`revised` steps:** preparer A edits the amounts while `disputed` (allowed only in that
  state, audited); B (`settlements.approve`, not A) runs `resolve` `revised` with the
  document: that is the second-user approval; no separate `approve` transition.
- **`revised` lower than already paid:** the overpaid part becomes a receivable from the
  employee (as a negative net, BL-HRM-23).
- **`upheld` when the net is not yet paid in full** (disputed before payment, or only the
  undisputed amount paid): back to `approved` so `pay` pays the rest; `closed` only when the net
  is paid in full or nothing is payable.
- **Payments while disputed:** recorded against the settlement; state stays `disputed`.
- **Second dispute after resolve:** allowed from `approved` or `paid` again.

#### Must not
- Must not block paying the undisputed amount.
- Must not let the preparer approve the revised amounts.

#### Done when
- [ ] Given a `paid` settlement, `dispute` with source "MoHRE complaint" and ref "C-9" makes it
      `disputed`, opens a labour-case ticket, notifies HR, accountant and MD.
- [ ] Given `approved` AED 12,000 with AED 10,000 undisputed, a payment of AED 10,000 is recorded
      while `disputed`.
- [ ] A enters revised amounts of AED 11,000 while `disputed`; A's `resolve` is refused; B runs
      `resolve` `revised` with the decision: `approved`, new statement, `pay` for AED 1,000;
      ticket closed.
- [ ] `resolve` without a document is refused.
- [ ] Given `disputed` from `approved` with AED 10,000 of AED 12,000 paid, `resolve` `upheld` makes
      it `approved`; `pay` for AED 2,000 makes it `paid`. Given `disputed` from `paid`, `upheld`
      makes it `closed`.

#### Out of scope
14-day alerts (BL-HRM-25).

---

### BL-HRM-23 — wl-hr: handle a negative final settlement

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `blocked:open-item` `surface:back-office` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** Base: wl-hr Final settlement lifecycle · BL-HRM-15 · **Blocks:** None
**Review items:** X-15

#### Goal
A settlement "goes negative" when its recoveries are larger than what may be offset; the net
itself never falls below 0. Recoveries are offset against the dues only within the Art. 25
caps (until a lawyer confirms otherwise, L-23). The recoveries above the caps are the amount
due from the employee: `pay` creates a receivable for it on the employee's party with a
recovery task for the accountant. The net after the capped offset may still be positive and
is then paid; a net of 0 is settled with no payment.

#### Surfaces
- **Back office:** the settlement shows the capped offset and the net; on **Pay** the
  accountant gets the recovery task for the receivable on the employee's party.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Final settlement* (`pay`, "Negative net"), §10
- `docs/OPEN-ITEMS.md` L-23

#### Terms
- **Receivable:** an amount owed by the employee's party, in wl-invoicing/wl-payments.
- **Negative net:** the spec's name for the case above; net = dues − capped offset ≥ 0, and the
  "negative" part is the amount due from the employee (recoveries above the caps).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `pay` | approved | paid | `settlements.pay` | payment of the net still unpaid recorded through WPS or transfer (`FIN.PAY.01`) to the payee (for payee *estate*: the estate's representative and the document giving the right recorded, `⚠ open: lawyer (L-25)`); or nothing payable (net 0) | acknowledgement sent for signature (`DOC.SIG.01`; not for the estate); employee may `exit`; recoveries above the caps create a receivable from the employee (see *Negative net* below); event `SettlementPaid` |

"**Negative net (recoveries above the caps).** A settlement "goes negative" when its
recoveries are larger than what may be offset; the net itself never falls below 0.
Recoveries (advances, loans, overpayments, damage) are offset against the
dues only within the Art. 25 caps (`HRM.ACL.06`: per type and 50% in total, computed on the
settlement's dues). The settlement counts as one period: each monthly per-type cap applies
once, as a share of the dues (overpayments 20%, fines 5%, court debts 25%), and damage at
most 5 days' wage of the last salary structure (a day's wage per §7). The recoveries above
the caps are the amount due from the employee: `pay`
creates a receivable for it on the employee's party (a non-VAT receivable, not a tax invoice) with
a recovery task for the accountant. The net (dues minus the capped offset) may therefore still be
positive, and is then paid as usual; a net of 0 is settled by `pay` with no payment. Whether the
employer may set off more than the caps against end-of-service dues, and how to recover the rest,
is `⚠ open: lawyer (L-23)` (§10); until confirmed the system offsets only within the caps and
leaves the rest as the receivable."

#### Edge cases decided
- **Which cap applies to a settlement:** the 50% total and per-type caps computed on the
  settlement's dues (L-23 open); the excess becomes the receivable, so net may then be > 0 and is
  paid.
- **Monthly caps on a one-off settlement:** the settlement is one period; each per-type cap
  applies once (overpayments 20% of dues, fines 5%, court debts 25%; damage 5 days' wage of the
  last salary structure, wage ÷ 30 per day); advances only the 50% total.
- **When net is 0:** only when the dues are 0 (with dues > 0 the capped offset leaves at least
  half of them).
- **Net = 0:** `pay` with no payment, no receivable.
- **Receivable document:** a non-VAT receivable record, not a tax invoice.

#### Must not
- Must not offset beyond the caps.
- Must not create a payment for a negative net.

#### Done when
- [ ] Given dues AED 4,000 and an advance balance AED 6,000, with cap 50% (AED 2,000 offset), net
      paid AED 2,000 and a receivable of AED 4,000 with an accountant task.
- [ ] Given dues AED 0 and no recoveries (net 0), `pay` succeeds with no payment and no
      receivable.
- [ ] Given dues AED 0 and an advance balance AED 1,000, `pay` succeeds with no payment and a
      receivable of AED 1,000 with an accountant task.
- [ ] Given dues AED 10,000 and an overpayment AED 3,000, the offset is AED 2,000 (20%), net
      AED 8,000 is paid and the receivable is AED 1,000.

#### Out of scope
Recovery collection (Epic wl-payments).

---

### BL-HRM-24 — wl-hr: close or cancel a final settlement

**Labels:** `module:wl-hr` `area:negative-flow` `priority:medium` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** Base: wl-hr Final settlement lifecycle · **Blocks:** BL-HRM-05, BL-HRM-06, BL-HRM-08
**Review items:** X-15

#### Goal
`paid` is no longer the end, so a dispute can still be recorded until the settlement closes. A
paid settlement closes 30 days after payment with no dispute, or as soon as the acknowledgement
is signed. A settlement that is no
longer needed (notice withdrawn, worker reinstated) is cancelled while nothing was paid.

#### Surfaces
- **Back office:** HR runs **Cancel** on a draft or approved settlement with nothing paid
  against it.
- **System:** a paid settlement closes 30 days after `pay`, or at once when the
  acknowledgement is signed.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Final settlement*, §7 (settlement closes after)

#### Terms
- **Acknowledgement:** the signed receipt requested on `pay` (`DOC.SIG.01`).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `closed` | success | yes | No dispute open; the period to dispute passed or the dispute ended |
| `cancelled` | neutral | yes | Not needed (notice withdrawn, worker reinstated) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `close` | paid | closed | system | N days after `pay` with no dispute (default 30), or acknowledgement signed | a later claim (e.g. a MoHRE case) is handled outside the machine on a *labour case* ticket on the employee |
| `cancel` | draft, approved | cancelled | `settlements.prepare` or system (`withdraw_notice`, `reinstate`) | nothing paid against it | — |

#### Edge cases decided
- **Estate payee:** no acknowledgement; closes at 30 days.
- **Dispute after `closed`:** not possible in the machine; a later MoHRE case is handled outside
  (labour-case ticket on the employee).
- **Signed acknowledgement:** a receipt of payment, not a waiver. Signing closes the
  settlement in the system only; it does not decide whether the worker may still claim (a
  legal point the system does not decide). A later claim follows the previous bullet.
- **Day count:** 30 calendar days, Asia/Dubai.

#### Must not
- Must not cancel a settlement with any payment recorded.

#### Done when
- [ ] Given `pay` on 2026-11-01 and no signature, on 2026-12-01 it is `closed`.
- [ ] Given the acknowledgement signed on 2026-11-03, it closes on 2026-11-03.
- [ ] Given `approved` with an AED 500 payment recorded, `cancel` is refused.

#### Out of scope
Dispute (BL-HRM-22).

---

### BL-HRM-25 — wl-hr: alert on the 14-day settlement deadline (`HRM.RUL.09`)

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `surface:system` · **Milestone:** Launch 09 · **Size:** s
**Depends on:** Base: wl-hr Final settlement lifecycle · BL-HRM-22 · **Blocks:** BL-MPW-21
**Review items:** X-15, 09-N5-d

#### Goal
The final settlement is due within 14 days of the end of the contract (Art. 53). HR and the
accountant are told 10 days after the last working day; on the deadline the MD is told it is
overdue. For a disputed settlement the reminder is to pay the undisputed amount.

#### Surfaces
- **System:** `HRM.RUL.09` runs at 08:00 Asia/Dubai and notifies HR and the accountant on
  day N and the MD on the deadline.

#### Files to read
- `docs/modules/wl-hr.md` §5.4 step 4, §6 *Automations* (`HRM.RUL.09`), §7, §10 *Final settlement*

#### Terms
- **End of the contract:** the last working day (as in `HRM.RUL.09`).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.09` | N days after the last working day, and on the legal deadline | final settlement `draft` or `approved`, or `disputed` with less than the undisputed amount paid (not `paid`, `closed` or `cancelled`) | notify HR and the accountant at N; on the deadline notify the MD that it is overdue; for `disputed`, remind to pay the undisputed amount | N = 10; deadline 14 days after the end of the contract (Art. 53 ✔ [S1]) |

#### Edge cases decided
- **`closed` settlements:** skipped (condition).
- **`disputed`:** alerted only while the amount paid is below the undisputed amount (condition),
  whether it was disputed before or after `pay`.
- **Day count:** calendar days, Asia/Dubai, at 08:00.
- **Idempotency:** one alert per point per settlement.

#### Must not
- Must not alert on `paid`, `closed` or `cancelled` settlements.

#### Done when
- [ ] Given last working day 2026-11-01 and settlement `approved`, HR and accountant are told on
      2026-11-11 and the MD on 2026-11-15.
- [ ] Given `paid` on 2026-11-05, nothing is sent.
- [ ] Given `disputed` with nothing paid, the message reminds to pay the undisputed amount.
- [ ] Given `disputed` with the undisputed amount paid in full, or `closed`, nothing is sent.

#### Out of scope
None.

---

### BL-HRM-26 — wl-hr: start an onboarding from the role's templates when an employee is hired (`HRM.RUL.16`)

**Labels:** `module:wl-hr` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-hr Employee lifecycle · **Blocks:** BL-HRM-27, BL-HRM-28, BL-HRM-29, BL-DOC-08, BL-DOC-09, BL-ACC-23, BL-AUT-18, BL-CLN-15, BL-CON-18, BL-EDU-17, BL-FNB-17, BL-LOG-25, BL-MFG-14, BL-MPW-20, BL-PRP-27
**Review items:** G6 (SME problem map 7, 18)

#### Goal
HR defines onboarding templates per job role (and one for all staff). When HR creates an
employee in `onboarding`, the system starts an onboarding: one step per template step,
assigned to HR, the manager or the employee, with a due date counted from the hire date. HR can
also start a template by hand, e.g. for a role change. New staff then get the same induction
every time instead of what the person on duty remembers.

#### Surfaces
- **Back office:** HR keeps onboarding templates per job role and starts a template by hand
  on the employee; the onboarding with its steps, assignees and due dates appears on the
  employee record.
- **System:** `HRM.RUL.16` starts the onboarding when an employee is created in
  `onboarding`; open steps are re-assigned on assignment changes and cancelled on exit,
  death or cancelled onboarding.

#### Files to read
- `docs/modules/wl-hr.md` §2 (`HRM.ONB.01` row, paragraphs on the *minimal* record and
  *Onboarding and SOPs*), §3
  (*Onboarding template*, *Onboarding template step*, *Onboarding*, *Onboarding step*,
  paragraph *Onboarding*, paragraph *The employee's manager*), §4 *Onboarding* (rows `start`,
  `cancel`), §5.8 steps 1–2, §6 (*Provides* `Workforce::employees_in`, `HRM.RUL.16`, events
  `OnboardingStarted`, `EmployeeAssignmentChanged`, permissions `onboarding.*`), §7
  (onboarding rows)

#### Terms
- **Owner of a step:** `hr` = holders of `onboarding.manage` in the employee's scope, other than
  users who hold it only through the owner role (they get `hr` steps only when nobody else holds
  it); `manager` = the employee's manager (§3); `employee` = the employee; or a named role's
  holders in the scope. A step with several assignees is one step: it is assigned to all of
  them, any one marks it done, and all are reminded.
- **Step kinds:** `task`, `checklist`, `document`, `sop_acknowledgement`. This issue only creates
  the steps with their kind, assignees and due date; how each kind is done is BL-HRM-27 (which
  depends on BL-DOC-08 and BL-DOC-09 for SOPs).
- **Required / evidence:** each template step is required unless HR marks it optional; a `task`
  step needs evidence only when its template step says so. Seeded steps: all required, none
  needs evidence.
- **Trigger:** `hire` (by the rule) or `manual` (by HR).
- **Minimal employee record:** person, scope, cost rate; onboarding runs on it too.
- **`Workforce::employees_in(role, scope)`:** built here: employees (`onboarding`, `active`,
  `on_leave`, `notice`) with a job role or trade in a scope; used by wl-documents for SOP
  required readers (BL-DOC-08, BL-DOC-09).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `start` | — | in_progress | system (`HRM.RUL.16`, employee created in `onboarding`) or `onboarding.manage` (trigger `manual`) | at least one active template applies (or is chosen); no other `in_progress` onboarding with the same template for the employee | template versions frozen; one step per template step, assignees resolved, due date = hire date (manual: start date) + due days; assignees notified; event `OnboardingStarted` |
| `cancel` | in_progress | cancelled | system (`EmployeeOnboardingCancelled`, `EmployeeExited`, `EmployeeDeceased`) or `onboarding.manage` (manual onboarding) | reason given (manual) | open steps `cancelled`; checklist runs in progress voided by the system with that reason; no further reminders |

§2: A minimal record is created in `onboarding` when an onboarding template applies to it,
else directly `active`. Its `activate` does not check the statutory items (visa case, labour
contract, mandatory documents), which the client keeps outside the platform: only the
document kinds the client marks as required for it (none by default) and a blocking
onboarding.

§3: Assignees follow the people, not the start date. Open steps are re-resolved when the
employee's scope assignment or job role changes, when an assignee's own assignment changes
or ends (`EmployeeAssignmentChanged`), and when an assignee's user account is deactivated;
with nobody left the step goes to HR. A corrected hire date recomputes the due dates of open
steps. A role change does not remove steps of the old role's template: HR waives those that
no longer apply and starts a manual onboarding for the new role. While the employee is
`on_leave`, due dates of open steps move by the days of leave and reminders pause; while
`absconded`, reminders and escalations pause (resumed on `reinstate`). Death
(`EmployeeDeceased`), exit and a cancelled onboarding cancel the onboarding.

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.16` | an employee is created in `onboarding` | at least one active onboarding template applies to the job role (or trade) and employment type, or an *all staff* template exists | `start` an onboarding (trigger `hire`); notify the assignees of the first steps | — (on) |

| Event | When | Payload |
|---|---|---|
| `OnboardingStarted` / `OnboardingCompleted` | onboarding `start` / `complete` | onboarding, employee, job role, scope, trigger |
| `EmployeeAssignmentChanged` | a scope assignment or the job role of an employee is added, changed or ended | employee, job role, scope, from, to |

| Service | Purpose | Used by |
|---|---|---|
| `Workforce::employees_in(role, scope)` | Employees (`onboarding`, `active`, `on_leave`, `notice`) with a job role or trade in a scope | wl-documents (SOP required readers, `DOC.RUL.04`) |

| Setting | Default |
|---|---|
| Onboarding templates per role (`HRM.ONB.01`) | one *all staff* induction template per vertical: welcome and introductions (manager, day 1), system account and access (HR, day 1), workplace and safety induction (manager, checklist, day 3), uniform or PPE handed over (HR, checklist with the employee's signature, day 3), SOPs acknowledged (employee, day 7); every step required, none needs evidence; role templates are added by the vertical or the client |

§3: `hr` = holders of `onboarding.manage` in the employee's scope, other than users who hold
it only through the owner role (the owner role holds it by default as the second person for
`waive` and `reopen`, and gets `hr` steps only when nobody else holds it). A step with
several assignees is one step: any one of them marks it done, and all of them are reminded.

#### Edge cases decided
- **Several templates apply** (all staff + role): one onboarding with the steps of all of them,
  each step keeping its template.
- **No manager in scope:** `manager` steps go to HR (§3).
- **Template edited after start:** the running onboarding keeps the frozen version.
- **Employee created directly `active`** (data import): no onboarding is started.
- **Due dates:** calendar days, Asia/Dubai; a due date on a holiday is kept (no shift).
- **`EmployeeAssignmentChanged`:** emitted here; it does not start an onboarding by itself
  (HR starts a manual one when wanted).
- **Manager changes or an assignee leaves:** open steps are re-resolved to the current holders;
  nobody left → HR. Done steps keep who did them.
- **Hire date corrected:** due dates of open steps recomputed from the new date.
- **Role changed during onboarding:** the old role's steps stay (HR waives them); the new role
  needs a manual onboarding.
- **Employee dies (`EmployeeDeceased`):** the onboarding is cancelled; nothing more is sent.
- **Minimal record (01, 04, 07, 08):** created in `onboarding` when a template applies, else
  `active`; its `activate` skips the statutory items (BL-HRM-29).

#### Must not
- Must not start a second onboarding with the same template while one is `in_progress`.
- Must not let a manual onboarding block anything.

#### Done when
- [ ] Given template "All staff" and template "Driver" (role driver), when HR creates driver
      "Ravi K." in `onboarding` with hire date 2026-11-02, one onboarding with both templates'
      steps starts; the "SOPs acknowledged" step is due 2026-11-09 and assigned to Ravi.
- [ ] Given the branch has no manager and HR users H1 and H2 hold `onboarding.manage` there, the
      "welcome" step is assigned to H1 and H2; when H1 marks it done, it is done for both.
- [ ] Given HR runs the Employee transition `cancel_onboarding` for Ravi and it is approved
      (`HRM.ACL.07`), the event `EmployeeOnboardingCancelled` makes the system run the
      onboarding's `cancel` (no reason asked: the system cancel takes the event's reason); the
      onboarding is `cancelled` and its open steps `cancelled`.
- [ ] Given HR runs `cancel` on the manual "Supervisor" onboarding of an active employee
      without a reason, it is refused; with a reason it is `cancelled`.
- [ ] Given HR starts "Supervisor" manually for an active employee, its steps are due from today.
- [ ] Given a minimal employee record created in `onboarding` (01), the onboarding starts too.
- [ ] `Workforce::employees_in(driver, branch B)` returns Ravi while he is `onboarding` or
      `active`, and not after he is `absconded` or has exited.
- [ ] Given Ravi's branch manager M1 is replaced by M2 while the "welcome" step is open, the
      step is assigned to M2.
- [ ] Given Ravi's hire date corrected from 2026-11-02 to 2026-11-05, the open "SOPs
      acknowledged" step is due 2026-11-12.
- [ ] Given `record_death` for an active employee with a manual onboarding in progress, the
      onboarding is `cancelled` and no reminder is sent afterwards.

#### Out of scope
Completing and waiving steps (BL-HRM-27); reminders (BL-HRM-28); the activation block (BL-HRM-29).

---

### BL-HRM-27 — wl-hr: complete and waive onboarding steps (`HRM.ACL.10`)

**Labels:** `module:wl-hr` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-HRM-26 · BL-DOC-08 · BL-DOC-09 · Epic wl-ticketing: checklist runs (`WRK.CHK.01`) · **Blocks:** BL-HRM-29, BL-ACC-23, BL-AUT-18, BL-CLN-15, BL-CON-18, BL-EDU-17, BL-FNB-17, BL-LOG-25, BL-MFG-14, BL-MPW-20, BL-PRP-27
**Review items:** G6 (SME problem map 7, 18)

#### Goal
Each onboarding step is done in the way its kind says: ticked with evidence, a passed checklist,
an accepted document, or every required SOP acknowledged. Steps owned by the employee are done
only by the employee, in their own session or in person with a signature and a witness. A step
that does not apply is waived by HR with a reason. The onboarding completes when every required
step is done or waived.

#### Surfaces
- **Back office:** assignees mark `task` steps done (with evidence when required); HR
  records an employee step in person with the drawn signature on a staff device as witness,
  and waives or reopens steps with a reason.
- **Portal:** the employee marks their own steps and acknowledges SOPs in their own session
  (screen not specified yet).
- **System:** `checklist`, `document` and `sop_acknowledgement` steps are marked done from
  `ChecklistCompleted`, `DocumentAccepted` and SOP events; the onboarding completes after
  the last required step.

#### Files to read
- `docs/modules/wl-hr.md` §3 paragraph *Onboarding*, §4 *Onboarding* (row `complete`) and
  *Onboarding step*, §5.8 steps 3–6, §6 (*Consumes* wl-ticketing and wl-documents rows,
  `HRM.ACL.10`)
- `docs/modules/wl-ticketing.md` §4.2 (checklist run), §6 (`Checklists::start`, `ChecklistCompleted`)
- `docs/modules/wl-documents.md` §4.3, §6 (`Sops::pending_for`, `SopAcknowledged`)

#### Terms
- **Step kinds:** `task`, `checklist` (a `WRK.CHK.01` run on the step), `document` (an accepted
  document of a kind on the employee), `sop_acknowledgement`.
- **In person:** the employee signs on a staff device; the staff user is the witness.
- **Required / evidence required:** fields of the template step (BL-HRM-26), set by HR per
  template step: a step is required unless marked optional; *evidence required* applies to
  `task` steps only (default no; the other kinds carry their own evidence). Seeded steps: all
  required, none needs evidence.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `mark_done` | open | done | an assignee (kind `task`), or system (`ChecklistCompleted` passed, `DocumentAccepted`, last required SOP acknowledged) | evidence attached when required; for an `employee` step, recorded by the employee (own session, or in person with signature and witness, `HRM.ACL.10`) | done by and at recorded; onboarding progress updated; `complete` when it was the last required step |
| `waive` | open | waived | `onboarding.manage` | reason given; the user is not an assignee of the step (`HRM.ACL.10`) | audited; progress updated |
| `reopen` | done, waived | open | `onboarding.manage` | onboarding `in_progress`; reason given; the user is not the one who marked the step done or waived it (`HRM.ACL.10`); a `done` step only of kind `task` (steps done by the system follow their evidence: a new checklist run, document or acknowledgement); a `waived` step of any kind (a reopened `document` or `sop_acknowledgement` step is re-evaluated at once and is `done` again when its evidence is already there) | done or waive record kept in the history; assignees notified; due date unchanged (overdue shown at once when passed); audited |
| `complete` (Onboarding) | in_progress | completed | system (last required step `done` or `waived`) | every required step `done` or `waived` | optional steps still `open` are left open; HR and the manager notified; event `OnboardingCompleted` |

| ID | Rule | Generalises | Pattern | Default |
|---|---|---|---|---|
| `HRM.ACL.10` | An onboarding step owned by the employee, and an SOP acknowledgement (wl-documents), is recorded only by the employee: in their own session, or in person with a drawn signature on a staff device, the staff user recorded as witness; no one records it for them otherwise. After an in-person record the employee is told on their own channel (SMS to the number on record, or the portal inbox when it opens). Waiving a step needs `onboarding.manage`, a reason, and a user who is not an assignee of the step; reopening a done or waived step needs `onboarding.manage`, a reason, and a user other than the one who marked it; audited | new (G6) | `OWN`, `SOD` | on |

§3: `checklist`: done when its `WRK.CHK.01` run, started on the onboarding step, completes with
outcome *passed* (a *failed* run leaves the step open with the failed items shown, and a new
run can be started). `sop_acknowledgement`: done when the employee has acknowledged the current
version of every SOP that lists the employee's role and scope among its required readers.
§6: the `sop_acknowledgement` step is re-evaluated on `SopPublished`, `SopAcknowledged`,
`SopWithdrawn` and `SopReadersChanged`, and is done when `Sops::pending_for(employee)` is
empty. Evidence files on steps are stored as documents of an employee kind and take that
kind's sensitivity class; onboarding records follow the worker-file retention (wl-hr §10).

#### Edge cases decided
- **SOP step with no SOP for the role:** done at start (nothing to acknowledge), noted *no SOPs*.
- **New SOP added while the step is open:** the step waits for it too.
- **Document accepted before the onboarding started:** the step is done at start.
- **Employee without an account:** only the in-person way is offered.
- **Optional steps:** do not hold `complete`; they stay open and can still be done.
- **Concurrent completion:** the step is done once; a second event is ignored (idempotent).
- **SOP withdrawn or role removed from its readers** while the step is open: the step is
  re-evaluated and done when nothing else is pending.
- **Ticked by mistake:** `reopen` by another HR user with a reason; a `completed` onboarding
  is not reopened (HR starts a manual one).
- **In-person record:** the employee gets an SMS (or a portal message when the account opens)
  naming the step and the witness.
- **One HR user only:** the owner role also holds `onboarding.manage` by default, and a user who
  holds it only through the owner role is not an `hr` assignee while someone else holds it
  (BL-HRM-26). So the owner (a different user, e.g. the MD) is the second person who waives
  the HR user's steps and reopens what the HR user marked; the HR user can never reopen their
  own marks.

#### Must not
- Must not let HR or the manager mark an `employee` step done without the employee's signature.
- Must not let an assignee of a step waive it.
- Must not mark a `checklist` step done on a *failed* run.

#### Done when
- [ ] Given a `checklist` step "PPE handed over", when the run completes *passed* with Ravi's
      signature, the step is `done`.
- [ ] Given the run completes *failed*, the step stays `open` with the failed items.
- [ ] Given Ravi has no portal account, HR user A records the SOP acknowledgement in person with
      Ravi's drawn signature; A is stored as witness.
- [ ] HR user A cannot mark Ravi's `task` step "read the handbook" (owner `employee`) done without Ravi.
- [ ] HR user A, the only HR holder of `onboarding.manage` in the branch and so the sole assignee
      of "system account", cannot waive it; owner O (holds it through the owner role, not an
      assignee) can, with a reason.
- [ ] Given a `task` step "copy of driving licence" with *evidence required*, Ravi cannot mark it
      done without a file or note; a step without *evidence required* is marked done without.
- [ ] Given an optional step still `open` when the last required step is done, the onboarding
      is `completed` and the optional step stays `open`.
- [ ] Given the `document` step "Emirates ID" waived by A and the document already accepted, O
      reopens it with a reason and it is `done` at once; a waived `checklist` step reopened by
      O is `open` and needs a new run.
- [ ] When the last required step is done, the onboarding is `completed` and `OnboardingCompleted` is emitted.
- [ ] Given HR user A marked "system account" done by mistake, A cannot reopen it; HR user B
      reopens it with a reason and the step is `open` again with the history kept.
- [ ] Given A records Ravi's SOP acknowledgement in person, Ravi receives an SMS naming the SOP
      and witness A.
- [ ] Given the only SOP for drivers is withdrawn while Ravi's `sop_acknowledgement` step is
      open, the step becomes `done`.

#### Out of scope
Acknowledgement requests and reminders (BL-DOC-09); overdue reminders (BL-HRM-28).

---

### BL-HRM-28 — wl-hr: remind and escalate overdue onboarding steps (`HRM.RUL.17`)

**Labels:** `module:wl-hr` `area:sme-gap` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-HRM-26 · **Blocks:** BL-ACC-23, BL-AUT-18, BL-CLN-15, BL-CON-18, BL-EDU-17, BL-FNB-17, BL-LOG-25, BL-MFG-14, BL-MPW-20, BL-PRP-27
**Review items:** G6 (SME problem map 7, 18)

#### Goal
A step left open after its due date is reminded to its assignees, and after a few more days
HR and the employee's manager are told, so that onboarding does not stall silently.

#### Surfaces
- **Back office:** HR and assignees see open steps marked *overdue* from the day after the
  due date.
- **System:** `HRM.RUL.17` runs daily at 08:00 Asia/Dubai, reminds every R days, escalates
  once after E days, and pauses for leave or absconding.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Onboarding step* (state `open`, *overdue*), §5.8 step 5, §6
  (`HRM.RUL.17`), §7 (onboarding step reminders)

#### Terms
- **Overdue:** shown from the day after the due date (no separate state).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.17` | daily at 08:00 Asia/Dubai | onboarding step `open` and its due date has passed; the employee is not `on_leave` or `absconded` (paused; on leave the due date moves by the days of leave) | remind the assignees on the first run after the due date, then every R days; after E days overdue notify HR and the employee's manager once | R = 2; E = 5 |

§3: While the employee is `on_leave`, due dates of open steps move by the days of leave and
reminders pause; the move always equals the calendar days of the approved leave as currently
recorded, so a leave shortened, extended or cancelled moves the due dates again. While
`absconded`, reminders and escalations pause and due dates do not move; after `reinstate`, an
overdue step is reminded on the first run and then every R days, and HR and the manager are
told on that run when E days have passed and they were not told before.

#### Edge cases decided
- **Count:** calendar days after the due date, Asia/Dubai.
- **Optional steps:** reminded like required ones, but HR is not told.
- **Onboarding `completed` or `cancelled`:** no more reminders.
- **Idempotency:** one reminder per step per day at most.
- **Employee on leave:** reminders pause; the due date of open steps moves by the leave days.
- **Leave changed after the move:** the shift is recomputed from the leave as recorded now
  (original due date + days of approved leave), so shortening or extending the leave moves
  the due date again; a cancelled leave puts it back.
- **Employee absconded:** reminders and escalations pause and due dates do not move; on
  `reinstate` an overdue step is reminded on the next run, then every R days from it; HR and
  the manager are told on that run if E days have passed and they were not told before; the
  onboarding is cancelled on exit or death.

#### Must not
- Must not remind for steps `done`, `waived` or `cancelled`.

#### Done when
- [ ] Given a step due 2026-11-09 still open, the assignee is reminded on 2026-11-10, 2026-11-12, 2026-11-14.
- [ ] On 2026-11-14 (5 days overdue) HR and the manager are told once.
- [ ] Given the step done on 2026-11-11, nothing more is sent.
- [ ] Given an employee with a manual onboarding goes on 10 days' leave from 2026-11-08, a step
      due 2026-11-09 becomes due 2026-11-19 and no reminder is sent during the leave.
- [ ] Given the employee is `absconded`, no reminder or escalation is sent.
- [ ] Given that 10-day leave shortened to 6 days, the step becomes due 2026-11-15.
- [ ] Given a step due 2026-11-09, the employee `absconded` from 2026-11-05 and reinstated on
      2026-11-20 at 15:00, the step stays due 2026-11-09; on 2026-11-21 the assignee is
      reminded and HR and the manager are told once; the next reminder is on 2026-11-23.

#### Out of scope
SOP acknowledgement reminders (`DOC.RUL.04`, BL-DOC-09).

---

### BL-HRM-29 — wl-hr: block employee activation until a blocking onboarding is complete

**Labels:** `module:wl-hr` `area:sme-gap` `priority:medium` `size:s` `surface:back-office` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-HRM-26 · BL-HRM-27 · **Blocks:** None.
**Review items:** G6 (SME problem map 7, 18)

#### Goal
Onboarding blocks nothing by default. A client may set *blocks activation* on a template (for
example, a safety induction for site workers); Employee `activate` is then refused until every
required step from that template is done or waived. The statutory items of `activate` stay as
they are.

#### Surfaces
- **Back office:** HR's **Activate** is refused while a blocking template has open required
  steps; the message lists those steps and their assignees.

#### Files to read
- `docs/modules/wl-hr.md` §2 (paragraph on the *minimal* record), §3 paragraph *Onboarding*
  (last paragraph), §4 *Employee* (row `activate`), §5.1 step 5, §7 (*Onboarding blocks
  activation*)

#### Terms
- **Statutory onboarding items:** the existing `activate` guard (visa case, labour contract,
  mandatory documents).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `activate` | onboarding | active | `employees.manage` | statutory onboarding items complete: when visa cases are enabled, a *new* visa case `completed`; labour contract signed; mandatory documents (Emirates ID, passport, health insurance) attached with expiry; ILOE is not a blocker, the worker has four months from the work permit to subscribe ✔ [S6]. For `freelance`: permit or freelance visa and the freelance agreement attached instead. For a *minimal* record (§2): no statutory items, only the document kinds the client marks required (none by default). For every record: when the employee's onboarding includes a template with *blocks activation* (§3 *Onboarding*), every required step from such a template `done` or `waived` |

§3: A template may set *blocks activation*: Employee `activate` then also needs every
required step that comes from that template `done` or `waived`; steps of other templates in
the same onboarding do not block. portal account created (where the vertical has an employee portal); event `EmployeeActivated` |

| Setting | Default |
|---|---|
| Onboarding blocks activation | off for every template |

#### Edge cases decided
- **Several templates, one blocking:** only the required steps that come from the blocking
  template are checked; open steps of the other templates in the same onboarding do not block.
- **Manual onboarding:** never blocks (trigger `manual`).
- **Refusal message:** lists the open required steps and their assignees.
- **Employee steps before the portal account exists:** done in person (BL-HRM-27).
- **Minimal record (01, 04, 07, 08):** `activate` checks no statutory items; only the client's
  required document kinds (none by default) and a blocking onboarding.

#### Must not
- Must not weaken the statutory items.
- Must not block when the setting is off.

#### Done when
- [ ] Given template "Site induction" with *blocks activation* and one required step open,
      `activate` is refused and the open step is listed.
- [ ] After the step is waived by HR user B with a reason, `activate` runs (statutory items complete).
- [ ] Given the setting off, `activate` ignores open onboarding steps.
- [ ] Given one onboarding with "All staff" (not blocking, a required step open) and "Site
      induction" (blocking, every required step done), `activate` runs.
- [ ] Given a 07 operator on a minimal record in `onboarding`, no visa case, labour contract or
      Emirates ID, and no blocking template, `activate` runs.

#### Out of scope
The statutory items (base Employee lifecycle).

---

### BL-HRM-30 — wl-hr: hold pay, draft the settlement and open the cancellation case when an employee is reported absconded

**Labels:** `module:wl-hr` `area:negative-flow` `priority:high` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 09 · **Size:** m
**Depends on:** BL-HRM-07 · Base: wl-hr Payroll run lifecycle · Base: wl-hr Final settlement lifecycle · Base: wl-hr Visa case lifecycle · **Blocks:** BL-HRM-08, BL-HRM-10
**Review items:** X-15, 09-N12-c

#### Goal
BL-HRM-07 moves the employee to `absconded` and emits `EmployeeAbsconded`. This issue adds the
effects of the spec row that need payroll, final settlement and visa cases, which exist from
phase F9 (decision 0005): the pay not yet sent is held outside WPS, the final settlement is
drafted and the *cancellation* visa case opens.

#### Surfaces
- **Back office:** the payroll officer sees the employee's lines in runs not yet `submitted` held
  with *absence report*; HR sees a draft final settlement with reason *absconding* and a
  *cancellation* case from the *absconding* variant of the step template.
- **System:** the effects run in the `report_absconding` transition, after the BL-HRM-07
  guards pass.

#### Files to read
- `docs/modules/wl-hr.md` §4 *Employee* (`report_absconding`), §5.4 "Absconding", §8 (visa
  step templates), §10 *Absconding*
- `docs/OPEN-ITEMS.md` L-24

#### Terms
- **Held line:** a payroll line left out of the SIF, with a reason.
- **Absconding step template:** the *absconding* variant of the *cancellation* visa step
  template (`VisaStepTemplate`, spec §8): it starts from the MoHRE absence report and needs no
  worker signature.

#### Spec excerpt (authoritative for this issue)

The effects of `report_absconding` (spec §4 *Employee*, built in two issues: guards, approval,
portal and attendance, deployments and the event are BL-HRM-07):

payroll lines in runs not yet `submitted` held with reason *absence report* (outside WPS while
the report is valid, MR 340/2026 Art. 4 ✔ [S2]; a line in a file already sent stays, and if the
bank fails it, it is held); final settlement drafted (or, from `notice`, the existing one gets
reason *absconding* and no second one is drafted); *cancellation* visa case opened from the
*absconding* variant of the cancellation step template (§8).

§10: what may lawfully be withheld is `⚠ open: lawyer (L-24)`.

#### Edge cases decided
- **Open payroll lines:** lines in runs not yet `submitted`. A line already in a sent SIF (run
  `submitted`) cannot be recalled and is left; if the bank pays it, it stays paid; if the bank
  fails it (`record_partial`), it is held with reason *absence report*, not regenerated.
- **From `notice`:** the existing draft settlement is kept and its reason changed to
  `absconding`; no second settlement is drafted.
- **Settlement amounts:** drafted as for any exit; L-24 decides withholding later; do not invent
  forfeiture.
- **Release:** held lines are released by `reinstate` through `HRM.RUL.12` (BL-HRM-08), not here.

#### Must not
- Must not pay held lines through WPS while the report is valid.
- Must not run when the BL-HRM-07 guards refuse.

#### Done when
- [ ] Given E `active` reported absconded (the BL-HRM-07 case), E's line in the `calculated`
      November run is held with *absence report*, a draft settlement with reason *absconding*
      exists, and a *cancellation* case from the *absconding* template is open.
- [ ] Given E in `notice` with a draft settlement, the settlement keeps its id and gets reason
      *absconding*; no second settlement exists.
- [ ] Given E's line in a November run already `submitted`, it stays in the sent SIF; when the
      bank fails it, it is held with *absence report* and `regenerate` leaves it out.
- [ ] When BL-HRM-07 refuses the request, no line is held and no settlement or case is created.

#### Out of scope
The guards, approval, portal and attendance, deployments and the event (BL-HRM-07);
`reinstate` (BL-HRM-08); exit (BL-HRM-10).
