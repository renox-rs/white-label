# Epic: wl-documents — negative flows

> Status: draft · Last updated: 2026-10-15 · Spec version: same commit as this file

Expired documents no longer only remind: a document kind names the transitions it guards
(`blocks`), and every guarded module calls the standard guard `document_valid`. This epic
builds the guard (block and flag modes), the warning when future work is planned, the nightly
look-ahead (`DOC.RUL.02`), the expiry reaction (`DOC.RUL.03`), lifting the block on renewal,
one-use overrides (`DOC.ACL.02`) and the rule that blocking documents are never edited in place
(`DOC.ACL.03`). Review item X-13 (with 01-N8-b, 03-N8-a, 05-N8-a, 09-N8-b, 10-N8-a, 10-N8-c).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-DOC-01 | Refuse a guarded transition while a required document is expired or missing | high | m | Build 1 | Base: wl-documents Document lifecycle |
| BL-DOC-02 | Flag a record instead of refusing it for kinds in flag mode | high | s | Build 1 | BL-DOC-01 |
| BL-DOC-03 | Warn when future work is planned past a document's expiry | medium | s | Build 1 | BL-DOC-01 |
| BL-DOC-04 | List planned work affected by a blocking document before it expires | high | m | Build 1 | BL-DOC-01 |
| BL-DOC-05 | Emit the blocked records on expiry and lift the block on renewal | high | m | Build 1 | BL-DOC-01, BL-DOC-04, BL-KRN-08 |
| BL-DOC-06 | Approve a one-use override of a document block | high | m | Build 1 | BL-DOC-01, BL-KRN-01 |
| BL-DOC-07 | Correct a blocking document only through a new version accepted by another user | high | s | Build 1 | Base: wl-documents Document lifecycle |

---

### BL-DOC-01 — wl-documents: refuse a guarded transition while a required document is expired or missing

**Labels:** `module:wl-documents` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-documents Document lifecycle · **Blocks:** BL-DOC-02, BL-DOC-03, BL-DOC-04, BL-DOC-05, BL-DOC-06, BL-TKT-06, BL-ACC-01, BL-ACC-19, BL-ACC-23, BL-AUT-07, BL-AUT-18, BL-CLN-15, BL-EDU-07, BL-EDU-08, BL-EDU-17, BL-LOG-25, BL-MFG-11, BL-MFG-14, BL-MPW-08, BL-MPW-09, BL-MPW-20, BL-PRP-27
**Review items:** X-13, 01-N8-b, 09-N8-b

#### Goal
A document kind lists the transitions it guards (`blocks`). Every module that owns a guarded
transition calls one standard guard, `document_valid(subject, kind, on_date)`. In `block` mode
the transition is refused while the document is expired or missing, and the user sees which
document, since when, and who can renew it or approve an override. 09 (residence visa and work
permit block `deployment.activate`) and 01 (vendor Civil Defence approval) need it first.

#### Files to read
- `docs/modules/wl-documents.md` §3 (*Document kind*: `blocks`, legal block), §5.6 (intro, "The
  standard guard", "When it is checked" step 1, example table), §6 (`is_valid`, `blocks_for`,
  `BlockSubjects`), §7 (settings `blocks`), §10 (Expired documents)
- `docs/WORKFLOW.md` §8 (pattern "Expired document blocks")
- `docs/OPEN-ITEMS.md` L-19

#### Terms
- **`blocks`:** list on a document kind of `<entity>.<transition>` entries, each with mode
  `block` or `flag` and an optional condition (a field of the guarded record and a list of
  values).
- **Document subject:** the record the document is attached to (employee, vehicle, vendor party,
  legal entity, course).
- **`BlockSubjects` provider:** registered by the guarded module; maps its record to the document
  subjects (a run → its driver and vehicle; a booking → its practitioner).
- **Legal block:** a kind the law requires for the act; override mode `never`, locked by the
  platform.
- **`qualification_valid`:** wl-hr's wrapper of the same check for employees (out of scope).

#### Spec excerpt (authoritative for this issue)

*Document kind* fields added: `blocks` (list of guarded `<entity>.<transition>` entries, each with
mode `block` or `flag` and an optional condition on the guarded record), override mode (`never` /
`with_approval`), legal block (locked by the platform), look-ahead days, compliance-critical.

§5.6 conditions: "An entry may carry an optional **condition** on the guarded record: one field
the guarded module exposes for conditions and a list of values (e.g. `ticket.resolve` when the
ticket's category is one of the fire and life safety categories; `service_contract.activate`
when the contract covers one of those categories). Without a condition the entry applies to
every record. The guarded module's `BlockSubjects` provider receives the condition and returns
no document subject for a record that does not match, so the guard answers *yes* for it."

§5.6 "The standard guard": "Every module that owns a guarded transition calls
`document_valid(subject, kind, on_date)`. It answers *yes* when the subject has an `accepted`
document of that kind whose expiry date is on or after `on_date` (or that has no expiry).
`on_date` defaults to today and may be any future date; a past date is checked against the
document that was accepted and valid on that date (used for late entries and attendance). A
missing document, a `rejected` one, or a renewal still in `pending_review` answers *no*; a pending
renewal is shown in the refusal message and pushed to the top of the review queue. The guarded
module maps its record to the document subjects through a `BlockSubjects` provider."

"When it is checked" step 1: "At the time of the action: the guard runs with `on_date` = today
when the transition is attempted. In `block` mode the user sees which document is expired, since
when, and who can renew it or approve an override."

Services: `Documents::current(subject, kind)` / `is_valid(subject, kind, on_date)` (the guard
`document_valid`; `on_date` optional, default today, future dates allowed);
`Documents::blocks_for(entity, transition)`: document kinds that guard a transition, with mode and
override mode; modules call it before `document_valid`.

| Setting | Default |
|---|---|
| `blocks` per kind (guarded transitions, mode and optional condition) | the vertical's seeded values (§5.6); a client may add entries, not remove a legal block |
| Compliance-critical kinds (owner also told on expiry) | legal blocks |

Seeded examples (§5.6): 09 residence visa, work permit (employee): `deployment.activate`,
`deployment.resume` (block); `attendance_record.accept` (flag); never (legal block). 01 vendor's
Civil Defence approval (vendor party): `ticket.resolve` with condition category in the fire and
life safety categories (block); `service_contract.activate` with condition contract covers one
of those categories (block); never.

#### Edge cases decided
- **Date semantics** (§3 expiry status): dates are calendar dates in Asia/Dubai. A document is
  valid through its expiry date: expiring 2026-11-30 it is valid on 2026-11-30 and expired from
  2026-12-01.
- **Several subjects or several kinds** guard one transition: all must answer yes; the message
  lists each failing document.
- **Past `on_date`:** a document `superseded` later still counts if it was accepted and valid on
  that date.
- **Category conditions** ("for fire and life safety categories"): the `blocks` entry stores
  the condition (field and values); the guarded module's `BlockSubjects` provider evaluates it
  and returns no subject for records it does not match (e.g. a plumbing ticket). An entry
  without a condition applies to every record.
- **No caching:** the guard reads the current documents at the moment of the transition, so a
  renewal accepted a second earlier is seen.
- **Client changes to `blocks`:** adding entries is allowed; removing or switching to `flag` an
  entry of a legal-block kind is refused.
- **Legal blocks** are seeded per the table; which kinds are legal blocks is `⚠ open: lawyer
  (L-19)`; until answered, seeded `never` kinds stay locked.

#### Must not
- Must not let a module implement its own expiry check instead of `document_valid`.
- Must not treat a `pending_review` renewal as valid.
- Must not let a client remove a legal-block entry.

#### Done when
- [ ] Given worker "Ravi S." whose residence visa expired on 2026-11-30, on 2026-12-01 the
      coordinator's `deployment.activate` is refused with "Residence visa expired on
      2026-11-30; renew: PRO; override: not allowed".
- [ ] On 2026-11-30 the same activation is allowed.
- [ ] With a renewal uploaded and `pending_review`, the refusal names it and it moves to the top
      of the review queue.
- [ ] A run with a valid driver licence and an expired vehicle insurance is refused naming only
      the insurance.
- [ ] `document_valid(ravi, residence_visa, 2026-11-15)` answers yes after a renewal superseded
      the old visa.
- [ ] A client admin cannot remove `deployment.activate` from the work permit kind.
- [ ] With the Civil Defence approval of vendor "SafeFire LLC" expired, `ticket.resolve` is
      refused for its fire alarm ticket and allowed for its plumbing ticket (condition not
      matched).

#### Out of scope
Flag mode (BL-DOC-02); overrides (BL-DOC-06); each module's `BlockSubjects` provider and call
sites (module epics); `qualification_valid` (`Epic wl-hr`).

---

### BL-DOC-02 — wl-documents: flag a record instead of refusing it for kinds in flag mode

**Labels:** `module:wl-documents` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-DOC-01 · **Blocks:** BL-ACC-19, BL-ACC-23, BL-CON-18, BL-EDU-11, BL-EDU-17, BL-FNB-17, BL-MPW-20
**Review items:** X-13, 09-N8-b, 02-N5-d

#### Goal
Some facts cannot be refused: a worker who did turn up is marked present. For `blocks` entries
in `flag` mode the guarded transition runs, but the record is flagged for review with the
document that was invalid.

#### Files to read
- `docs/modules/wl-documents.md` §5.6 (modes), example table (09, 02, 10 flag entries)

#### Terms
- **Flag:** a marker on the record ("document invalid: <kind>, expired <date>") shown in lists
  and on the record, for a reviewer.

#### Spec excerpt (authoritative for this issue)

"`flag`: the transition runs, but the record is flagged for review (used where the fact cannot be
refused, e.g. a worker who did turn up and is marked present)."

Seeded: 09 `attendance_record.accept` (flag); 02 worker visa and work permit
`attendance_record.accept` (flag); 10 `session.submit_attendance` (flag).

#### Edge cases decided
- **`on_date` for flag checks** is the record's business date (e.g. the attendance date), so a
  late entry is judged against the document valid on that day.
- **Clearing:** the flag stays as a historical fact; a reviewer marks it
  "reviewed" with a note. A later renewal does not remove it.
- **Same transition with a block-mode kind failing:** block wins; the transition is refused.

#### Must not
- Must not refuse the transition in flag mode.

#### Done when
- [ ] Given worker "Ravi S." with work permit expired 2026-11-30, accepting his attendance for
      2026-12-02 succeeds and the record is flagged "Work permit expired 2026-11-30".
- [ ] Accepting his attendance for 2026-11-29 (entered late on 2026-12-02) is not flagged.
- [ ] A reviewer marks the flag reviewed with a note; the flag shows reviewer and note.

#### Out of scope
What the attendance module does with flagged records (`Epic wl-attendance: HRM.RUL.14`).

---

### BL-DOC-03 — wl-documents: warn when future work is planned past a document's expiry

**Labels:** `module:wl-documents` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-DOC-01 · **Blocks:** None.
**Review items:** X-13, 05-N8-a, 09-N8-b

#### Goal
When a user creates or reschedules a dated item (session, run, deployment, planned visit), the
guard is called with the planned date. A *no* is a warning, not a refusal, because the document
may be renewed in time. Bookings are different: the booking module refuses a slot whose resource
will not be eligible on its date (`SCH.ACL.04`, `Epic wl-scheduling`).

#### Files to read
- `docs/modules/wl-documents.md` §5.6 "When it is checked" step 2

#### Terms
- **Planned date:** the date of the future item (session date, run day, deployment start).
- **Bookable slot:** a booking (or another future slot promised to a party); refused, not warned.

#### Spec excerpt (authoritative for this issue)

"When future work is planned: creating or rescheduling a dated item (a booking, a session, a
run, a deployment) calls the guard with `on_date` = the planned date.
- A **booking**, or another bookable future slot promised to a party, is **refused** by the
  booking module when its resource will not be eligible on the slot's date: `wl-scheduling`
  refuses `book`, `reschedule` and `reassign` through `ResourceEligibility` (`SCH.ACL.04`),
  and staff choose another resource or date.
- For **other planned work** (a run, a deployment, a batch's sessions, a planned visit), a *no*
  gives a warning, not a refusal: the document may be renewed in time, and `DOC.RUL.02` lists
  the item again inside the look-ahead window. Bookings made before the problem was known are
  not refused either: step 3 flags them."

#### Edge cases decided
- **Warning content:** document kind, subject, expiry date, planned date.
- **No confirmation step:** the warning is shown and the item is saved; `DOC.RUL.02` will list
  it again inside the look-ahead window.
- **Items spanning several days** (a deployment): checked at the start date only; Default (spec
  silent), consistent with "the planned date".
- **Bulk creation** (a batch of 12 sessions): one warning listing the affected dates.

#### Must not
- Must not refuse the creation or reschedule of planned work that is not a bookable slot.
- Must not turn a booking refusal into a warning (the booking module refuses).

#### Done when
- [ ] Given worker "Ravi S." whose residence visa expires 2026-12-15, a deployment planned to
      start 2026-12-20 is saved with the warning "Residence visa expires 2026-12-15".
- [ ] Moving its start to 2026-12-15 (the expiry date, still valid) shows no warning.
- [ ] Given a practitioner licence expiring 2026-12-15, booking a visit on 2026-12-20 gives no
      wl-documents warning: `wl-scheduling` refuses the booking (`SCH.ACL.04`).
- [ ] Creating 12 weekly sessions starting Monday 2026-11-02 for a tutor whose clearance expires
      2026-12-31 saves all 12 and gives one warning listing the 3 dates after the expiry
      (2027-01-04, 2027-01-11, 2027-01-18).

#### Out of scope
The look-ahead and module flags (BL-DOC-04); the booking refusal itself (`Epic wl-scheduling:
SCH.ACL.04`).

---

### BL-DOC-04 — wl-documents: list planned work affected by a blocking document before it expires

**Labels:** `module:wl-documents` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-DOC-01 · **Blocks:** BL-DOC-05, BL-ACC-23, BL-CON-18, BL-EDU-07, BL-EDU-08, BL-LOG-25, BL-MFG-11, BL-MPW-09, BL-PRP-27
**Review items:** X-13, 05-N8-a, 01-N8-b

#### Goal
Every night, for documents of blocking kinds that expire within the look-ahead window (default
30 days), each guarded module lists its planned items dated after the expiry date. The
list is added to `DocumentExpiring` as `affected`, the responsible staff get one message with it,
and modules flag the items (e.g. scheduling raises `BookingConflict`).

#### Files to read
- `docs/modules/wl-documents.md` §5.2 step 3, §5.6 "When it is checked" step 3, §6 (`FutureWork`
  provider, event `DocumentExpiring`), `DOC.RUL.02`, §7 (look-ahead, who is notified), §8
  `ReminderSchedule`

#### Terms
- **`FutureWork` provider:** registered per guarded transition by the owning module; returns the
  planned items of a subject from a date.
- **Look-ahead window:** days before expiry during which the check runs (per kind).
- **Responsible staff:** the users holding the document kind's *responsible role* (a field of
  the kind, seeded per kind by the vertical, editable by the client) in the scope unit of the
  document's subject; for a subject without a scope (a vendor party), in the scope units of the
  `affected` records; with no such records (an item's certificate), in the scope units the
  vertical's `ReminderSchedule` names for the kind, else every scope (spec §5.2 step 3).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `DOC.RUL.02` | nightly (01:00 Asia/Dubai) | document of a kind with `blocks` expires within N days, and guarded modules report planned items dated after the expiry date | emit `DocumentExpiring` with `affected`; notify the responsible staff once with the list (again only for new items, listing only those); modules flag the items (e.g. `BookingConflict`) | N = 30 (look-ahead, per kind) |

| Event | When | Payload |
|---|---|---|
| `DocumentExpiring` | an expiry offset is reached, or a blocking kind enters its look-ahead window (`DOC.RUL.02`) | document id, subject, kind, expiry date, days left, `affected` (planned records per guarded transition) |

§5.2 step 3: "The responsible staff are the users who hold the kind's responsible role in the
scope unit of the document's subject; for a subject without a scope (a party, e.g. a vendor),
in the scope units of the records the document guards or lists as `affected` (e.g. the
coordinator of the building where the vendor's visits are planned). For a subject without a
scope and without such records (e.g. an item's certificate), the vertical's `ReminderSchedule`
for the kind names the scope units (07: each plant that holds stock of the item or has an open
work order for it, every plant when none does); without one, the role's holders in every scope
are notified. The vertical seeds the role per kind and the client may change it (§7)."

#### Edge cases decided
- **"Notify once":** one message when the document enters the window; a
  new message only when new items appear in `affected` on a later night (listing only the new
  ones).
- **"After the expiry date":** items dated on the expiry date are not included, because the
  document is valid through its expiry date and the guard answers yes on that day.
- **A renewal accepted** before the night run: the old document is superseded; nothing is
  reported.
- **No affected items:** no `DocumentExpiring` from this rule (the reminder offsets of
  `DOC.RUL.01` still apply).
- **Idempotent** per document, night and item set.
- **Time zone:** "nightly" runs at 01:00 Asia/Dubai.

#### Must not
- Must not change the state of planned items here; modules only flag them.

#### Done when
- [ ] Given kind *Civil Defence approval* with responsible role *coordinator*, an approval of
      vendor "SafeFire LLC" expiring 2026-12-15 and 2 planned fire-safety visits in building
      "Tower A" on 2026-12-20 and 2027-01-10, on the night of 2026-11-15 the coordinator of
      Tower A gets one message listing both, the coordinator of Tower B gets nothing, and
      `DocumentExpiring { affected: [2 visits] }` is emitted.
- [ ] On 2026-11-16 nothing new is sent; when a third visit on 2026-12-28 is planned, the next
      night sends one message listing only it.
- [ ] With the look-ahead set to 45 days for the kind, the first message comes on 2026-10-31.
- [ ] Given an ECAS certificate of item "Valve V1" (no scope) with responsible role *QC
      inspector*, no `affected` records, and a `ReminderSchedule` naming Plant 1 (holds stock of
      V1), the 30-day reminder goes to Plant 1's QC inspectors only; without a
      `ReminderSchedule`, it goes to the QC inspectors of every plant.

#### Out of scope
Module reactions to `affected` (`Epic wl-scheduling: SCH.RUL.05`, BL-TKT-07, vertical epics).

---

### BL-DOC-05 — wl-documents: emit the blocked records on expiry and lift the block on renewal

**Labels:** `module:wl-documents` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-DOC-01, BL-DOC-04, BL-KRN-08 · **Blocks:** BL-TKT-07, BL-ACC-12, BL-CLN-12, BL-EDU-07, BL-LOG-25, BL-MFG-11, BL-MPW-11, BL-PRP-27
**Review items:** X-13, 01-N8-b, 09-N8-b

#### Goal
When a document of a blocking kind expires, `DocumentExpired` is emitted with the open and planned
records it now blocks, the responsible staff are told (and the business owner for
compliance-critical kinds), and a renewal task is opened. When a renewed document is accepted,
the guard answers yes again and consumers re-check what they suspended or flagged; nothing
resumes without that event.

#### Files to read
- `docs/modules/wl-documents.md` §5.6 "When it is checked" step 4, "Lifting the block", table
  "Who reacts to `DocumentExpired`", §6 (events), `DOC.RUL.03`, §7

#### Terms
- **Compliance-critical kind:** a kind whose expiry is also told to the business owner (default:
  legal blocks).
- **Renewal task:** a task (ticket via `TaskCreator`) for the responsible role to renew the
  document.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `DOC.RUL.03` | nightly (01:00 Asia/Dubai), on the first run after the expiry date (the document is valid through that date) | document of a kind with `blocks` | emit `DocumentExpired` with the blocked records; notify the responsible staff; for *compliance-critical* kinds also the business owner; open a renewal task for the responsible role | — |

| Event | When | Payload |
|---|---|---|
| `DocumentExpired` | the day after the expiry date (`DOC.RUL.03`) | document id, subject, kind, expiry date, `blocks`, blocked records (open and planned) |
| `DocumentAccepted` | a document becomes accepted | document id, subject, kind, expiry date |

"Lifting the block. When a renewed document is accepted (`DocumentAccepted`, the old one
`superseded`), the guard answers *yes* again. Consumers that suspended or flagged records on
`DocumentExpired` re-check them on `DocumentAccepted`; nothing resumes without that event."

Who reacts (for information): wl-hr, wl-scheduling, wl-delivery, wl-attendance, wl-ticketing, and
the apps (09 suspends deployments, 10 flags batches and sessions, 04 opens a CDD refresh task, 01
flags the contract and building).

#### Edge cases decided
- **When `DOC.RUL.03` fires:** on the first nightly run where today (Asia/Dubai) is after the
  expiry date, consistent with the guard (BL-DOC-01) and §5.2 (documents whose expiry date was
  yesterday).
- **Missing document** (never uploaded) emits nothing; the guard alone refuses.
- **Renewal task** is not duplicated if one is open for the same subject and kind; it closes when
  a renewal is accepted.
- **Rule failure:** `DOC.RUL.03` is compliance-critical in 01 and 09; a failed or missed run
  alerts the owner (BL-KRN-08).
- **Renewal valid from a later date:** if the accepted renewal's expiry is in the past (wrong
  data), the guard still answers no and no lifting happens.

#### Must not
- Must not resume any record automatically on expiry or before `DocumentAccepted`.
- Must not emit `DocumentExpired` twice for the same document.

#### Done when
- [ ] Given worker "Ravi S." whose work permit (compliance-critical, legal block) expires
      2026-11-30 with 1 active deployment and 1 planned, on the night of 2026-12-01
      `DocumentExpired` lists both, HR and the PRO are notified, the MD as business owner is
      notified, and a renewal task is open for the PRO.
- [ ] Running the rule again that night emits nothing.
- [ ] When the renewed permit is accepted on 2026-12-05, `DocumentAccepted` is emitted, the old
      permit is `superseded`, the renewal task closes and `deployment.resume` is allowed again.
- [ ] A non-compliance-critical kind's expiry does not notify the business owner.

#### Out of scope
Consumer reactions (module and vertical epics: `Epic wl-hr: HRM.RUL.10`, `Epic 09 Manpower:
deployment suspend`, BL-TKT-07).

---

### BL-DOC-06 — wl-documents: approve a one-use override of a document block

**Labels:** `module:wl-documents` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-DOC-01, BL-KRN-01 · Base: wl-kernel Approval request lifecycle · **Blocks:** BL-ACC-01, BL-ACC-23, BL-CLN-15, BL-EDU-08, BL-EDU-17, BL-MPW-08, BL-MPW-20
**Review items:** X-13, 09-N8-b

#### Goal
For kinds whose override mode is `with_approval`, a user can ask to run one blocked transition on
one record once, with a reason. A second user with `documents.block.override` in the scope
approves it; it is valid for one use within 72 hours and audited. Kinds marked legal block are
never overridable, whatever the client configures.

#### Files to read
- `docs/modules/wl-documents.md` §3 (*Block override*), §5.6 "Override", §6
  (`request_override`, event `DocumentBlockOverridden`, permission `documents.block.override`,
  `DOC.ACL.02`), §7, §10, §12 question 6
- `docs/OPEN-ITEMS.md` L-19

#### Terms
- **Override:** a Block override record for one kind, subject, guarded entry and record.
- **Override mode:** `never` or `with_approval`, per kind; `never` is locked for legal blocks.

#### Spec excerpt (authoritative for this issue)

Entity *Block override*: document kind, subject, guarded entry (`<entity>.<transition>`), record,
reason, requested by, approved by, valid until, used at.

"Override (`DOC.ACL.02`). Only for kinds whose override mode is `with_approval`. The user
asks for an override for one record and one transition with a reason. A second user with
`documents.block.override` in the scope approves it (`PLT.APR.01`). The override is valid
for one use, within a time limit (default 72 hours), only by the user who requested it, and
is audited with both users and the reason. Kinds marked *legal block* have override mode
`never`: the platform locks it, and no client setting or role can change it. Which kinds are
legal blocks in each vertical is to be confirmed `⚠ open: lawyer (L-19)`."

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `DOC.ACL.02` | A transition blocked by an expired or missing document runs only with a one-use override: reason given, approved by another user with `documents.block.override`, within a time limit, audited. Never for kinds marked *legal block* | `SOD`, `TIM`, `STA` | valid 72 hours |

| Event | When | Payload |
|---|---|---|
| `DocumentBlockOverridden` | an override is approved and used | override id, document kind, subject, record, transition, reason, users |

| Setting | Default |
|---|---|
| Override mode per kind | `never` for legal blocks (locked); otherwise `with_approval` |
| Override validity | one use, within 72 hours |

Seeded example: 09 Emirates ID, health insurance, health card, site pass: with approval (MD);
SIRA licence: never.

#### Edge cases decided
- **72 hours** run from approval, elapsed time; shown in Asia/Dubai.
- **One use:** the first successful run of that transition on that record stamps `used at`;
  concurrent attempts lock the override row, so only one succeeds.
- **Who may use it:** only the requester. Another user (including the approver) running the same
  transition on the record is refused by the guard as if no override existed, and the override
  stays unused.
- **Transition fails for another reason:** the override is not used and stays valid.
- **Event timing:** `DocumentBlockOverridden` is emitted at use (spec: "approved and used").
- **The document is renewed before use:** the override is not needed and simply expires.
- **Several kinds failing:** each needs its own override; a legal-block kind among them refuses
  the transition regardless.
- **Approver:** must differ from the requester and hold the permission in the record's scope
  (09: MD).
- **Legal-block list** is `⚠ open: lawyer (L-19)`; seeded `never` kinds stay locked until answered.

#### Must not
- Must not offer an override for a `never` kind, or let any client setting change a legal block to
  `with_approval`.
- Must not allow a second use or a use after 72 hours.
- Must not let the requester approve.
- Must not let anyone but the requester use an approved override.

#### Done when
- [ ] Given worker "Ravi S." with health card expired and `deployment.activate` refused, user A
      requests an override with reason "renewal booked 2026-12-03"; the MD (user B) approves;
      user A activates the deployment once; `used at` is set and `DocumentBlockOverridden` is
      emitted.
- [ ] A second activation attempt with the same override is refused.
- [ ] Given a new approved override requested by user A, when user B (the approver) or user C
      tries to activate the deployment, the guard refuses it and `used at` stays empty; user A
      can still use it.
- [ ] An override approved 2026-12-01 10:00 and not used is refused at 2026-12-04 10:01.
- [ ] For the SIRA licence (never), no override action is offered and a direct request is
      refused.
- [ ] A client admin cannot set the work permit kind to `with_approval`.
- [ ] User A cannot approve their own request.

#### Out of scope
The guard (BL-DOC-01).

---

### BL-DOC-07 — wl-documents: correct a blocking document only through a new version accepted by another user

**Labels:** `module:wl-documents` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-documents Document lifecycle · **Blocks:** None.
**Review items:** X-13

#### Goal
The expiry date and number of an accepted document of a blocking kind decide whether work may go
on, so nobody may edit them in place (for example to push an expiry date forward), and nobody
may bring one in alone: every document of a blocking kind, new, renewed or corrected, is
reviewed by someone other than the uploader, and audited.

#### Files to read
- `docs/modules/wl-documents.md` §3 (rules for all entities), §4.1 *Document*, §6 (`DOC.ACL.03`)

#### Terms
- **Blocking kind:** a document kind with at least one `blocks` entry.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `DOC.ACL.03` | Documents of a blocking kind (a kind with `blocks`) always go through review: they are submitted (never `record`ed) and accepted by a user other than the uploader. Their expiry date and number are not edited in place: a correction is a new document of the same kind and subject, with the file as its version, submitted to `pending_review` and accepted by a user other than the one who submitted it; accepting it supersedes the old one. Audited | `STA`, `SOD` | — |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `accept` | pending_review | accepted | `documents.review` | number and expiry date filled when the kind has them; for a kind with `blocks`, the reviewer is not the uploader (`DOC.ACL.03`) | expiry reminders scheduled; previous accepted document of the same kind and subject `supersede`; event `DocumentAccepted` |
| `record` | — | accepted | `documents.upload` | kind does not need review and has no `blocks` entry | same as `accept` |

Also used: `submit` (— → pending_review), `supersede` (system). *Document kind* field: needs
review (always on for kinds with `blocks`). Setting: kinds with `blocks` always need review,
whoever uploads (not editable).

#### Edge cases decided
- **How a correction is made:** the user submits a correction, which creates a new Document of
  the same kind and subject in `pending_review` (with the corrected number and dates and the
  file as its version); accepting it supersedes the old one. Acceptance is on the Document, not
  on a version.
- **Who accepts:** a user with `documents.review` other than the one who submitted the
  correction (or the new document or renewal).
- **Kinds that would not otherwise need review:** a kind with `blocks` always needs review;
  `record` is refused for it, and the seeded or client "needs review" setting cannot switch
  this off.
- **Other fields** (title, issuing authority) may still be edited in place with `documents.manage`.
- **Audit:** the old and new values and both users are in the audit trail.

#### Must not
- Must not allow editing expiry date or number in place, even for `documents.manage` holders or
  admins.

#### Done when
- [ ] Given an accepted work permit (blocking kind) with expiry 2026-11-30, the edit form shows
      expiry and number read-only.
- [ ] User A submits a correction with expiry 2027-11-30; it is `pending_review`; user A cannot
      accept it; user B accepts it; the old document is `superseded`.
- [ ] For a non-blocking kind, expiry can still be edited in place.
- [ ] Staff user A cannot `record` a driving licence (kind with `blocks`) directly; it is
      submitted to `pending_review`, and only a user other than A can accept it.

#### Out of scope
Ordinary renewals (base).
