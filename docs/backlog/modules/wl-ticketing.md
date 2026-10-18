# Epic: wl-ticketing — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

What happens when maintenance and recurring work go wrong: a service contract ends with visits
still open, a vendor does not turn up, a vendor's approval lapses, a compliance-critical subject
is left without a valid contract or visit, a vendor party is blocked, a chargeable job is
invoiced before the tenant confirms, and a recurring task whose agreement ended or paused. It
also seeds the ticket categories other modules open for their disputes. Review items D-13, D-14,
X-13 (vendor documents), X-18 (vendor blocked), 01-N8-b, the 01 MEDIUM items "vendor no-show
reassignment", and the 04 task changes (D-09 area, 04-N8-a).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-TKT-01 | Reassign a ticket with a recorded reason without restarting the SLA | medium | s | Build 1 | Base: wl-ticketing Ticket lifecycle |
| BL-TKT-02 | Invoice a chargeable ticket on resolve or on close per category | high | s | Build 1 | Base: wl-ticketing Ticket lifecycle; Epic wl-invoicing: `Invoicer` draft |
| BL-TKT-03 | Raise and clear compliance flags, including missed planned visits | high | m | Build 1 | Base: wl-ticketing Service contract lifecycle; BL-KRN-08 |
| BL-TKT-04 | Handle open visits and compliance when a service contract ends | high | m | Build 1 | BL-TKT-01, BL-TKT-03, BL-KRN-08 |
| BL-TKT-05 | Record a vendor no-show and offer reassignment | medium | m | Build 1 | BL-TKT-01 |
| BL-TKT-06 | Refuse contract activation and visit completion without a valid vendor document | high | s | Build 1 | BL-DOC-01 |
| BL-TKT-07 | Flag a contract and its subjects while the vendor's document is expired | high | m | Build 1 | BL-TKT-03, BL-TKT-04, BL-TKT-06, BL-DOC-05, BL-TKT-01 |
| BL-TKT-08 | Put a blocked vendor's service contracts under review | high | s | Build 1 | BL-PTY-03, BL-PTY-04, BL-TKT-01 |
| BL-TKT-09 | Waive a task when its agreement ends or a started task no longer applies | medium | s | Launch 04 | Base: wl-ticketing Task lifecycle |
| BL-TKT-10 | Hold tasks while their source agreement is paused | medium | m | Launch 04 | Base: wl-ticketing Task lifecycle |
| BL-TKT-11 | Seed the ticket categories that other modules open for negative flows | medium | s | Build 1 | Base: wl-ticketing Ticket lifecycle |

---

### BL-TKT-01 — wl-ticketing: reassign a ticket with a recorded reason without restarting the SLA

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-ticketing Ticket lifecycle · **Blocks:** BL-TKT-04, BL-TKT-05, BL-TKT-07, BL-TKT-08, BL-PRP-27
**Review items:** D-13, 01 MEDIUM (vendor no-show reassignment)

#### Goal
When a ticket already assigned is assigned to someone else (the vendor did not turn up, the
contract ended, the vendor's approval lapsed), the reassignment records a reason, the previous
assignee is told the job was withdrawn, and the SLA clocks keep running from the first
assignment.

#### Surfaces
- **Back office:** the coordinator reassigns an assigned ticket choosing a reason
  (`vendor_no_show`, `contract_ended`, `vendor_document_expired`, `workload`, `other` +
  text); the ticket shows the reassignment history.
- **System:** the SLA due times are not restarted or recomputed, and the previous assignee
  is told the job was withdrawn.

#### Files to read
- `docs/modules/wl-ticketing.md` §3 (*Ticket*: reassignment history, visit scheduled at), §4.1
  (`assign`)

#### Terms
- **Reassignment:** `assign` from `assigned` to `assigned` with a different assignee.
- **Reason codes:** at least `vendor_no_show`, `contract_ended`, `vendor_document_expired`, plus
  free text.
- **SLA clocks:** response and resolution timers of the ticket.

#### Spec excerpt (authoritative for this issue)

*Ticket* fields added: visit scheduled at (optional appointment with the requester or a vendor),
reassignment history with reason (e.g. `vendor_no_show`).

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `assign` | open, assigned | assigned | `tickets.assign` | priority and category set; a reassignment records a reason (e.g. `vendor_no_show`, `contract_ended`) | SLA clocks start (first time; a reassignment does not restart them); assignee notified, and the previous assignee told the job was withdrawn; event `TicketAssigned` |

#### Edge cases decided
- **Reason list:** (spec §7 "Reassignment reasons") `vendor_no_show`, `contract_ended`,
  `vendor_document_expired`, `workload`, `other` (+ text). The first three are also used by the
  system.
- **Reassigning to the same assignee** with a new visit time is not a reassignment (no reason,
  no withdrawal message); the requester is told the new time.
- **Previous assignee is a vendor without a login:** the withdrawal goes to the vendor party's
  contact through `wl-notify`.
- **Ticket in `in_progress` or `on_hold`:** `assign` is not available (from `open`, `assigned`
  only); the coordinator must have the work stopped first through the normal path.

#### Must not
- Must not restart or recompute the SLA due times on reassignment.
- Must not allow a reassignment without a reason.

#### Done when
- [ ] Given ticket TKT-2026-0042 assigned to vendor "CoolAir" at 2026-11-03 09:00 with response
      due 17:00, when the coordinator reassigns it to technician "Imran" with reason
      `vendor_no_show` at 13:00, then the reassignment history shows CoolAir → Imran,
      `vendor_no_show`, 13:00; CoolAir is told the job was withdrawn; Imran is notified; response
      due stays 17:00.
- [ ] A reassignment without a reason is refused.
- [ ] Changing only the visit time for CoolAir sends no withdrawal.

#### Out of scope
No-show detection (BL-TKT-05); contract-end reassignment task (BL-TKT-04).

---

### BL-TKT-02 — wl-ticketing: invoice a chargeable ticket on resolve or on close per category

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:field-pwa` `surface:portal` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-ticketing Ticket lifecycle · Epic wl-invoicing: `Invoicer` invoice draft · **Blocks:** BL-PRP-24, BL-PRP-26
**Review items:** D-14

#### Goal
A chargeable job must not be invoiced before the requester had the chance to dispute the work.
Each category says whether its invoice draft is made on `resolve` or on close (`confirm` or
`close`). 01 Property invoices tenants on close: at `resolve` the tenant only sees the amount.

#### Surfaces
- **Back office:** the coordinator may change the charge lines after resolve; the invoice
  draft appears for finance at `resolve` or at close, per category.
- **Field PWA:** the assignee enters the charge lines before Resolve; resolving a chargeable
  ticket without one is refused with "add the charge before resolving".
- **Portal:** for a category that invoices on close, the tenant sees the total with VAT when
  asked to confirm, and no invoice yet.
- **System:** one draft per ticket is made through `Invoicer` at `resolve` or at `confirm`,
  `close` or the `WRK.RUL.02` auto-close.

#### Files to read
- `docs/modules/wl-ticketing.md` §4.1 (`resolve`, `confirm`, `close`; 01 row of the vertical
  mapping), §5.1 step 5, §7 (ticket categories)
- `docs/verticals/01-property.md` §7.1 *Ticket* (`resolve` effects)

#### Terms
- **Chargeable:** the ticket's chargeable flag; the requester's party pays.
- **Invoice draft:** a draft invoice created through `Invoicer` (wl-invoicing), not issued.
- **Charge lines:** lines on a chargeable ticket (description, quantity, unit price, VAT
  category), entered by the assignee or coordinator before `resolve`. They are both the amount
  the requester is shown and the lines the draft will carry; no `Invoicer` call is needed to
  show the amount.

#### Spec excerpt (authoritative for this issue)

Ticket field: "charge lines (chargeable tickets: description, quantity, unit price, VAT category;
the amount the requester is shown and the lines the invoice draft carries)".

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `resolve` | in_progress | resolved | `tickets.work` | after-photo attached when the category requires it; required custom fields filled; for a category guarded by a document kind, `document_valid(assignee, kind, today)` for the assigned vendor (`WRK.ACL.11`); a chargeable ticket has at least one charge line | requester asked to confirm; chargeable and the category invoices on *resolve* → invoice draft through `Invoicer` with the ticket's charge lines (one draft per ticket: after `reopen` the draft stays a draft, is not issued while the ticket is `in_progress`, and the next `resolve` updates it instead of creating a second); when it invoices on *close*, the requester sees the amount (the charge lines' total including VAT at each line's VAT category), no invoice yet |
| `confirm` | resolved | closed | requester, or system (`WRK.RUL.02`) | — | chargeable and the category invoices on *close* → invoice drafted through `Invoicer` with the ticket's charge lines; event `TicketClosed` |
| `close` | resolved | closed | `tickets.assign` | category does not require requester sign-off, or no requester (internal ticket) | as `confirm`; event `TicketClosed` |

| Setting | Default |
|---|---|
| Ticket categories (… chargeable, invoice on *resolve* or *close*) | Seeded per vertical; invoice on *resolve* (01: *close*) |

01 mapping: "chargeable jobs invoice the tenant on *close* (`confirm` or `close`), not on
`resolve`".

#### Edge cases decided
- **`reopen` after resolve** in a category that invoices on *resolve*: the draft stays a draft and is not issued while the ticket is `in_progress`; on the next `resolve`
  no second draft is created (one draft per ticket).
- **Auto-close (`WRK.RUL.02`)** counts as `confirm`: the draft is created then.
- **Amount shown at resolve** is the total of the charge lines including VAT, computed on the
  ticket; it is the amount the draft will carry. Changes to the charge lines after resolve
  (coordinator only) show on the ticket before close.
- **Chargeable ticket without charge lines:** `resolve` is refused ("add the charge before
  resolving").
- **Idempotent:** one draft per ticket, keyed by ticket id.
- **Disputes** of the invoice after close are an invoice `dispute` (`Epic wl-invoicing`).

#### Must not
- Must not create an invoice draft at `resolve` for a category set to *close*.
- Must not create two drafts for one ticket.

#### Done when
- [ ] Given a 01 chargeable category (invoice on *close*) and one charge line "Replace AC
      capacitor", 1 × AED 333.33, VAT standard 5%, at `resolve` the tenant sees AED 350.00
      (333.33 + 16.67 VAT) and no draft exists; at `confirm` one draft exists with that line and
      a total of AED 350.00.
- [ ] Resolving a chargeable ticket with no charge line is refused.
- [ ] If the tenant does nothing, `WRK.RUL.02` closes the ticket after 3 days and the draft is
      created then.
- [ ] Given a category set to *resolve* (default), the draft is created at `resolve`, and a
      `reopen` + second `resolve` creates no second draft.

#### Out of scope
Issuing and disputing the invoice (`Epic wl-invoicing`).

---

### BL-TKT-03 — wl-ticketing: raise and clear compliance flags, including missed planned visits

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-ticketing Service contract lifecycle, Maintenance plan lifecycle; BL-KRN-08 · **Blocks:** BL-TKT-04, BL-TKT-07, BL-PRP-25, BL-PRP-27
**Review items:** D-13, 01-N8-b

#### Goal
For compliance-critical categories (01: fire and life safety, lifts), the module keeps a
Compliance flag on a covered subject whenever it is not compliant: no contract, a planned visit
missed, or the vendor's approval expired. Flags show on the subject, the dashboard and the
compliance report, and clear when the cause is fixed. This issue builds the flag and its
`visit_missed` cause (`WRK.RUL.14`).

#### Surfaces
- **Back office:** open compliance flags show on the subject, on the dashboard and in the
  compliance report (by subject, reason and age); the coordinator, and the manager for
  compliance-critical categories, are notified.
- **System:** `WRK.RUL.14` runs daily, raises `visit_missed` N days after the planned due
  date, clears it when the visit is resolved, and emits `ComplianceFlagRaised` /
  `ComplianceFlagCleared`.

#### Files to read
- `docs/modules/wl-ticketing.md` §3 (*Compliance flag*, *Service contract* compliance contract),
  §4.6 vertical mapping, §6 (events `ComplianceFlagRaised` / `ComplianceFlagCleared`,
  `WRK.RUL.14`, compliance-critical note), §7 (compliance-critical categories), §10 (Fire and
  life-safety systems)
- `docs/OPEN-ITEMS.md` AU-08

#### Terms
- **Compliance-critical category:** a ticket category set by the vertical (01: fire and life
  safety, lifts).
- **Compliance contract:** a service contract with at least one compliance-critical covered
  category.
- **Covered subject:** a building, building system or asset under a contract's coverage.
- **Planned visit:** a ticket created by a maintenance plan (`MaintenanceDue`).
- **Planned due date:** the Ticket field holding the due date of the plan occurrence that
  created the visit (from `MaintenanceDue`). It is not the *visit scheduled at* appointment
  (BL-TKT-01) and not the SLA response or resolution due time.

#### Spec excerpt (authoritative for this issue)

Entity *Compliance flag*: subject (building, building system, asset), category, reason
(`no_contract`: a compliance contract ended with no replacement; `visit_missed`: `WRK.RUL.14`;
`vendor_document_expired`: the vendor's required approval expired), contract, raised at, cleared
at, cleared by (replacement contract `active`, visit resolved, document renewed).

Ticket field: "planned due date (planned visits only: the due date of the maintenance plan
occurrence that created the ticket, from `MaintenanceDue`; not the visit scheduled at and not an
SLA due time)".

*Service contract* field: compliance contract (yes/no; yes when a covered category is
compliance-critical).

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `WRK.RUL.14` | N days after a planned visit's planned due date (the Ticket field) | visit ticket not `resolved` | notify the coordinator; for compliance-critical categories (01: fire and life safety, lifts) also notify the manager in scope and raise the compliance flag `visit_missed` on the subject | N = 3 |

| Event | When | Payload |
|---|---|---|
| `ComplianceFlagRaised` / `ComplianceFlagCleared` | a compliance flag is raised or cleared | subject, category, reason, contract |

"`WRK.RUL.14` and `WRK.RUL.18` are compliance-critical: a failed run alerts the business owner
(`PLT.RUL.04`)."

Setting: compliance-critical categories (compliance contracts, compliance flags): from the
vertical (01: fire and life safety, lifts).

#### Edge cases decided
- **One open flag per subject, category and reason:** raising it again while open does nothing.
- **Clearing `visit_missed`:** when that visit ticket is `resolved`; a visit `cancel`led by the coordinator does not clear it.
- **Days:** N = 3 calendar days after the planned due date, checked daily (Asia/Dubai). A visit
  rescheduled to a later *visit scheduled at* keeps its planned due date, so moving the
  appointment does not postpone the flag.
- **Dashboard and compliance report** list open flags by subject with reason and age; the report is a registered report definition in the report library.
- **Rules outside Dubai** for fire and life safety are `⚠ open: authority (AU-08)`; the flag logic
  is the same in every emirate until answered.

#### Must not
- Must not raise compliance flags for categories that are not compliance-critical.
- Must not clear a flag except by its own clearing event.

#### Done when
- [ ] Given a planned fire-safety visit for "Tower A fire alarm" with planned due date
      2026-11-01 (visit scheduled at 2026-11-02 10:00) and not resolved,
      on 2026-11-04 the coordinator and the property manager are notified, a flag `visit_missed`
      is raised and `ComplianceFlagRaised` is emitted.
- [ ] A planned cleaning visit missed the same way only notifies the coordinator.
- [ ] When the visit is resolved on 2026-11-06, the flag clears with "visit resolved" and
      `ComplianceFlagCleared` is emitted.
- [ ] The compliance report lists the open flags of Tower A.
- [ ] A failed `WRK.RUL.14` run alerts the business owner (BL-KRN-08).

#### Out of scope
`no_contract` (BL-TKT-04) and `vendor_document_expired` (BL-TKT-07) causes; 01's dashboard tile
(`Epic 01 Property`).

---

### BL-TKT-04 — wl-ticketing: handle open visits and compliance when a service contract ends

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-TKT-01, BL-TKT-03, BL-KRN-08 · Base: wl-ticketing Service contract lifecycle · **Blocks:** BL-TKT-07, BL-INV-24, BL-PRP-24, BL-PRP-25, BL-PRP-27
**Review items:** D-13

#### Goal
When a service contract expires or is terminated, `WRK.RUL.18` cancels the planned visits due
after the end date, leaves visits already in progress to finish, gives the coordinator one task to
reassign or cancel the remaining open visits, and for a compliance contract without a replacement
raises `no_contract` on each covered subject. Activating a replacement contract clears it.

#### Surfaces
- **Back office:** the coordinator gets one task listing the open visits due on or before
  the end date, with reassign or cancel per visit (reassign only where the category says
  so); covered subjects show the `no_contract` flag.
- **System:** `WRK.RUL.18` runs on expiry or the termination date, cancels visits due after
  it, raises `no_contract` without a replacement and emits `ServiceContractEnded`;
  `activate` of a replacement clears the flag.

#### Files to read
- `docs/modules/wl-ticketing.md` §4.1 (`cancel`, "Service contract coverage"), §4.6
  (`activate`, `expire`, `terminate`; vertical mapping), §5.6 step 6, §6
  (`ServiceContractEnded`, `WRK.RUL.18`), §7 (settings "Open visits on contract end" and
  "Coordinator options in the `WRK.RUL.18` task")

#### Terms
- **Planned visit:** a ticket created by the contract's maintenance plan.
- **End date:** the contract's end date (`expire`) or the termination date (`terminate`).
- **Replacement:** another `active` contract covering the same subject and category.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` (ticket) | open, assigned, on_hold | cancelled | `tickets.assign`, or system (`WRK.RUL.18`, contract ended) | reason given | requester and assignee notified |
| `activate` | draft | active | `service_contracts.manage` | (see BL-TKT-06 for the document guard) | …; compliance flags `no_contract` on the covered subjects and categories cleared; event `ServiceContractActivated` |
| `expire` | expiring | expired | system (`WRK.RUL.13`) | end date passed | maintenance plans `end`; open planned visits handled per §5.6 step 6 (`WRK.RUL.18`); for a compliance contract with no replacement, compliance flag `no_contract` on each covered subject; covered tickets fall back to the category SLA; event `ServiceContractEnded` |
| `terminate` | active, expiring | terminated | `service_contracts.manage` | reason and termination date given | the contract is `terminated` at once, but until the termination date it still covers its subjects (coverage, SLA, planned visits); on the termination date (`WRK.RUL.18`, date reached; at once when the date is today) maintenance plans `end`, and open planned visits and compliance flags are handled as for `expire`; event `ServiceContractEnded` when `WRK.RUL.18` runs |

§5.6 step 6 — when a contract ends without renewal (`expire`, or the termination date of a
`terminate` is reached; a contract terminated with a future date keeps covering its subjects
until then), `WRK.RUL.18`:
- cancels the open planned visits due after the end date (`open`, `assigned` or `on_hold`;
  system `cancel`, reason `contract_ended`; the vendor is told);
- leaves visits already `in_progress` to finish, under the contract's terms;
- gives the coordinator one task listing the other open visits (due on or before the end date,
  not started), to reassign each to another vendor or to staff (`assign`, reason
  `contract_ended`) or cancel it with a reason; a vertical may limit the task to reassign
  for some categories (§7; 01: compliance-critical visits cannot be cancelled);
- for a compliance contract, unless another `active` contract covers the same subject and
  category, raises the compliance flag `no_contract` on each covered subject and tells the
  manager in scope.
Reactive tickets already assigned to the vendor stay with it unless reassigned; they fall back to
the category SLA. Money: the vendor's bills after the end date are matched only for work done
before it (`FIN.PAB.01`).

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `WRK.RUL.18` | service contract → `expired`; or the termination date of a `terminated` contract reached (at once when it is today) | — | cancel open planned visits due after the end date; task for the coordinator to reassign or cancel the other open visits; for a compliance contract with no replacement covering the subject and category, raise the compliance flag `no_contract` on each covered subject and notify the manager in scope | task due 1 working day |

| Event | When | Payload |
|---|---|---|
| `ServiceContractEnded` | `renew`, `expire`, or the termination date of a `terminate` reached (`WRK.RUL.18`) | contract, outcome, new version (if renewed), visits cancelled, visits left for reassignment |

Settings: Open visits on contract end (`WRK.RUL.18`): cancel those due after the end date
(for `terminate`: the termination date, when it is reached); task for the rest. Coordinator
options in the `WRK.RUL.18` task, per category: reassign or cancel (01: reassign only for
compliance-critical categories, `PRP.ACL.15`).

Compliance-critical: `WRK.RUL.18` (owner alert on failure, `PLT.RUL.04`).

#### Edge cases decided
- **Termination date in the future:** `terminate` moves the contract to `terminated` at once,
  but `WRK.RUL.18` runs on the termination date (date reached). Until then the contract still
  covers its subjects (vendor proposal, SLA, planned visits). Visits due after that date are
  cancelled then, and `ServiceContractEnded` is emitted then.
- **Visits due exactly on the end date** belong to the coordinator's task, not to cancellation.
- **`on_hold` visits** due after the end date: cancelled like `open` and `assigned` ones (spec
  §5.6 step 6); `in_progress` ones are left alone.
- **Task due:** 1 working day after the rule runs (business calendar, Asia/Dubai). No task when
  there are no remaining visits.
- **Renewal** (`renew`) does not trigger the rule.
- **Replacement activated later:** `activate` clears the `no_contract` flags for its covered
  subjects and categories.
- **Reassign only:** for a category set to reassign only, the task offers no cancel for its
  visits, and a user `cancel` of such a visit from the task is refused; the system cancel of
  visits due after the end date still runs.
- **Idempotent:** one run per contract end.

#### Must not
- Must not cancel visits that are `in_progress`.
- Must not raise `no_contract` when another active contract covers the subject and category.
- Must not cancel reactive (non-planned) tickets.

#### Done when
- [ ] Given lift contract AMC-2026-0007 (compliance) for "Tower A lifts" ending 2026-12-31 with
      planned visits on 2026-12-20 (assigned), 2026-12-31 (open), 2027-01-15 and 2027-02-15, and
      one visit `in_progress`, when `expire` runs, then the 2027 visits are `cancelled` with
      `contract_ended` and the vendor is told; the coordinator has one task listing the 12-20
      and 12-31 visits; the in-progress visit is untouched.
- [ ] No other active lift contract covers Tower A: flag `no_contract` is raised and the property
      manager is told; `ServiceContractEnded` lists 2 cancelled and 2 left.
- [ ] When AMC-2027-0001 covering Tower A lifts is activated, the flag clears.
- [ ] A cleaning contract ending the same way raises no flag.
- [ ] A termination entered on 2026-11-10 with termination date 2026-11-30 cancels nothing
      until 2026-11-30; a ticket on a covered subject raised on 2026-11-20 still gets the
      contract's vendor proposed and its SLA; `ServiceContractEnded` is emitted on 2026-11-30.
- [ ] A termination entered with today's date runs `WRK.RUL.18` at once.
- [ ] A reassignment from the task records reason `contract_ended`.
- [ ] With lifts set to reassign only, the task for AMC-2026-0007 offers only "reassign" for
      the 12-20 and 12-31 visits; a cleaning contract's task still offers reassign or cancel.

#### Out of scope
Vendor bill matching after the end date (BL-INV-24).

---

### BL-TKT-05 — wl-ticketing: record a vendor no-show and offer reassignment

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-TKT-01 · **Blocks:** BL-PRP-27
**Review items:** 01 MEDIUM (vendor no-show reassignment)

#### Goal
When a vendor has not started a ticket 2 hours after its scheduled visit time (or, without an
appointment, by its response due time), the system records a no-show against the vendor and the
contract, and offers the coordinator to reassign it or set a new time. The requester is told the
new time; for compliance-critical categories the manager is told too. No-shows are counted on
the contract page.

#### Surfaces
- **Back office:** the coordinator gets the no-show with "Reassign" and "New time" actions;
  the contract page counts no-shows per vendor and contract.
- **System:** `WRK.RUL.19` records a no-show M hours after the visit time (or at response
  due) when a vendor has not started, and emits `VendorNoShow`.

#### Files to read
- `docs/modules/wl-ticketing.md` §4.1 (paragraph "Vendor no-show"), §6 (`VendorNoShow`,
  `WRK.RUL.19`), §7 (setting)

#### Terms
- **Not started:** the ticket is still `assigned` (not `in_progress`).
- **No-show record:** an entry against the vendor and contract with the ticket and the scheduled
  time.

#### Spec excerpt (authoritative for this issue)

"When a vendor does not start a ticket M hours after its visit scheduled time (elapsed clock
hours, not working hours; or, without an appointment, by its response due time, with no extra
M), `WRK.RUL.19` records a no-show against the vendor and
the contract, tells the coordinator and offers reassignment: `assign` to another vendor or to
staff with reason `vendor_no_show`, or a new visit time with the same vendor. The requester is
told the new time. For compliance-critical categories the manager in scope is told as well.
No-shows per vendor and contract appear on the contract page."

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `WRK.RUL.19` | M hours after a ticket's visit scheduled time (elapsed clock hours, not working hours), or at its response due time when it has no appointment | assigned to a vendor; not started | record a vendor no-show (event `VendorNoShow`); notify the coordinator with a reassign action; for compliance-critical categories also the manager in scope | M = 2 clock hours |

| Event | When | Payload |
|---|---|---|
| `VendorNoShow` | `WRK.RUL.19` records a no-show | ticket, vendor, contract, scheduled at |

#### Edge cases decided
- **Clock:** M is elapsed clock hours after the scheduled time, not working hours (a visit at
  17:00 is a no-show at 19:00 even after working hours). Without an appointment the trigger is
  the response due time itself (no extra M).
- **Ticket `on_hold`** is not `assigned`, so no no-show.
- **New time with the same vendor:** a new occurrence; another no-show can be recorded 2 hours
  after the new time.
- **Visit time changed before the trigger:** the rule uses the current scheduled time; the old
  occurrence is skipped (occurrence key includes the scheduled time).
- **No contract** (vendor ad hoc): the no-show is recorded against the vendor only.
- **Late entry:** if the vendor's start is entered late with a business time before the
  trigger, no no-show is recorded (BL-KRN-04).

#### Must not
- Must not reassign automatically; the coordinator decides.
- Must not record a no-show for staff assignees.

#### Done when
- [ ] Given ticket TKT-2026-0042 assigned to vendor "CoolAir" under AMC-2026-0003 with visit at
      2026-11-03 10:00, still `assigned` at 12:00, then a no-show is recorded, `VendorNoShow` is
      emitted and the coordinator is notified with "Reassign" and "New time" actions.
- [ ] Reassigning to "Imran" records `vendor_no_show` (BL-TKT-01); setting 15:00 with CoolAir
      tells the tenant 15:00.
- [ ] For a fire-safety ticket the property manager is notified too.
- [ ] The contract page shows 1 no-show for CoolAir under AMC-2026-0003.
- [ ] A ticket started at 11:30 records nothing.

#### Out of scope
Vendor performance reports beyond the count.

---

### BL-TKT-06 — wl-ticketing: refuse contract activation and visit completion without a valid vendor document

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:high` `blocked:open-item` `size:s` `surface:back-office` `surface:field-pwa` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-DOC-01 · **Blocks:** BL-TKT-07, BL-PRP-27
**Review items:** X-13, 01-N8-b

#### Goal
A contract covering a guarded category (01: fire and life safety) cannot be activated unless the
vendor's required document (Civil Defence approval) is valid on the start date, and a ticket or
planned visit in that category cannot be resolved while the assigned vendor's document is
expired or missing. In 01 the Civil Defence approval kind is seeded as a legal block (override
mode `never`, locked). Whether the law really forbids the work without it is still open
(`⚠ open: lawyer (L-19)`); until it is answered the seeded `never` stays, so this issue builds no
override for it.

#### Surfaces
- **Back office:** activating a contract covering a guarded category is refused while the
  vendor's document is not valid on the start date; no override action is offered in 01.
- **Field PWA:** the technician's Resolve on a fire-safety visit of a vendor with an expired
  Civil Defence approval is refused naming the document and date; a plumbing ticket resolves
  normally.

#### Files to read
- `docs/modules/wl-ticketing.md` §4.1 (`resolve` guard), §4.6 (`activate` guard), §6
  (`WRK.ACL.11`), §7 (vendor document kinds)
- `docs/modules/wl-documents.md` §5.6 (guard, 01 example row)
- `docs/OPEN-ITEMS.md` L-19

#### Terms
- **Guarded category:** a category whose vendor document kind lists `ticket.resolve` /
  `service_contract.activate` in its `blocks`.
- **`document_valid(subject, kind, on_date)`:** the standard guard from wl-documents.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `resolve` | in_progress | resolved | `tickets.work` | after-photo attached when the category requires it; required custom fields filled; for a category guarded by a document kind, `document_valid(assignee, kind, today)` for the assigned vendor (`WRK.ACL.11`); a chargeable ticket has at least one charge line | requester asked to confirm; chargeable and the category invoices on *resolve* → invoice draft through `Invoicer` with the ticket's charge lines (one draft per ticket: after `reopen` the draft stays a draft, is not issued while the ticket is `in_progress`, and the next `resolve` updates it instead of creating a second); when it invoices on *close*, the requester sees the amount (the charge lines' total including VAT at each line's VAT category), no invoice yet |
| `activate` | draft | active | `service_contracts.manage` | vendor, dates, at least one covered subject, SLA policy and visit schedule set; contract document attached; `document_valid(vendor, kind, start date)` for every vendor document kind whose `blocks` include `service_contract.activate` (e.g. 01: Civil Defence approval for fire and life safety) | (unchanged) |

| ID | Rule | Pattern | Source | Status |
|---|---|---|---|---|
| `WRK.ACL.11` | A ticket or planned visit in a category guarded by a vendor document kind (01: Civil Defence approval for fire and life safety) cannot be resolved while that document of the assigned vendor is expired or missing (`document_valid`); override only where the kind allows it (`DOC.ACL.02`), never for a legal block | `STA` | 01 | module |

Setting: vendor document kinds that guard categories: from the vertical, through the document
kind's `blocks` (01: Civil Defence approval → `ticket.resolve` and `service_contract.activate`
for fire and life safety).

#### Edge cases decided
- **Category condition:** the `BlockSubjects` provider for `ticket.resolve` returns the vendor
  only when the ticket's category is guarded (and for `service_contract.activate` only when a
  covered category is guarded); otherwise the guard is not applied (see BL-DOC-01).
- **Staff assignee** (not a vendor): the vendor guard does not apply.
- **Contract with a future start date:** the guard checks the document on the start date; a
  document expiring before the start refuses activation.
- **Override:** offered only when the kind's mode is `with_approval` (BL-DOC-06). 01's Civil
  Defence approval is seeded `never` (legal block, locked); L-19 may later confirm or change
  that seed, which is a configuration change, not a change to this issue.

#### Must not
- Must not let any role resolve a guarded fire-safety visit with an expired vendor approval in 01.

#### Done when
- [ ] Given vendor "SafeFire LLC" whose Civil Defence approval expired 2026-11-30, on 2026-12-02
      the technician's `resolve` of a fire-alarm visit is refused naming the document and date.
- [ ] A plumbing ticket by the same vendor resolves normally.
- [ ] Activating a fire-safety contract with start 2027-01-01 is refused while the approval
      expires 2026-12-31, and allowed after a renewal valid to 2027-12-31 is accepted.
- [ ] No override action is offered in 01.

#### Out of scope
Flags while the document is expired (BL-TKT-07).

---

### BL-TKT-07 — wl-ticketing: flag a contract and its subjects while the vendor's document is expired

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-TKT-03, BL-TKT-04, BL-TKT-06, BL-DOC-05, BL-TKT-01 · **Blocks:** BL-PRP-25, BL-PRP-27
**Review items:** X-13, 01-N8-b, D-13

#### Goal
When a vendor's guarding document expires (`DocumentExpired`), the contract carries the vendor
document flag, its open planned visits are flagged, the covered subjects get a compliance flag
`vendor_document_expired`, and the coordinator and manager are told. Everything clears when a
renewed document is accepted; otherwise the coordinator reassigns the visits or terminates the
contract. The vendor may still attend, but a visit it completes does not count.

#### Surfaces
- **Back office:** the contract shows the vendor document flag, its open planned visits are
  flagged and the covered subjects show `vendor_document_expired`; the coordinator reassigns
  visits with that reason or terminates the contract.
- **System:** on `DocumentExpired` the flags are raised once, and they clear on
  `DocumentAccepted`, on a replacement contract becoming `active`, or at contract end
  (`WRK.RUL.18`).

#### Files to read
- `docs/modules/wl-ticketing.md` §3 (*Service contract*: vendor document flag; *Compliance
  flag*), §4.6 paragraph "Vendor document expired", §5.6 step 7
- `docs/modules/wl-documents.md` §5.6 (table "Who reacts to `DocumentExpired`": wl-ticketing),
  §6 (events)

#### Terms
- **Vendor document flag:** a field on the service contract, set while a vendor document the
  contract's categories need is expired or missing.

#### Spec excerpt (authoritative for this issue)

"**Vendor document expired.** While a vendor document of a kind that guards the contract's
categories (01: Civil Defence approval for fire and life safety) is expired or missing
(`DocumentExpired` from `wl-documents`), the contract carries the vendor document flag, its open
planned visits are flagged, `resolve` is refused on them (`WRK.ACL.11`), the covered subjects get
a compliance flag `vendor_document_expired`, and the coordinator and the manager in scope are
told. The vendor may still attend, but a visit completed by an unapproved vendor does not count.
All flags clear on `DocumentAccepted` for a renewed document; otherwise the coordinator
reassigns the visits (`assign`, reason `vendor_document_expired`), which clears that visit's
flag, or terminates the contract. A subject's `vendor_document_expired` flag also clears when a
replacement contract covering that subject and category becomes `active`; when the contract ends
without one, `WRK.RUL.18` clears it and raises `no_contract` instead."

wl-documents: `wl-ticketing` reacts to `DocumentExpired`: "open visits of the vendor flagged;
`resolve` refused for guarded categories".

Entity *Compliance flag*, cleared by: "replacement contract `active`, visit resolved, document
renewed" (for `vendor_document_expired`: document renewed or replacement contract `active`).

#### Edge cases decided
- **Which contracts:** all `active` or `expiring` contracts of that vendor with at least one
  covered category guarded by the expired kind.
- **Which subjects** get `vendor_document_expired`: those covered for a guarded category.
- **Visits reassigned to another vendor** lose the visit flag. The subject's compliance flag
  stays until one of its clearing events (spec §4.6): the document is renewed
  (`DocumentAccepted`), a replacement contract covering the subject and category becomes
  `active`, or the contract ends and `WRK.RUL.18` replaces it with `no_contract` (BL-TKT-04).
- **Look-ahead** (`DocumentExpiring` with `affected`): the coordinator is told in advance (BL-DOC-04);
  no flag is raised before expiry.
- **Repeated events:** no duplicate flags (one per subject, category, reason).

#### Must not
- Must not clear a flag except by its clearing event: the contract and visit flags by
  `DocumentAccepted` for a renewed document (a visit flag also by reassignment); the subject's
  compliance flag by `DocumentAccepted`, a replacement contract becoming `active`, or the
  contract's end (`WRK.RUL.18`, which raises `no_contract`).
- Must not count a visit resolved by an unapproved vendor (the guard refuses it).

#### Done when
- [ ] Given vendor "SafeFire LLC" with contract AMC-2026-0011 covering "Tower A fire alarm" and 2
      open planned visits, when its Civil Defence approval expires, then the contract shows the
      vendor document flag, both visits are flagged, Tower A fire alarm has a
      `vendor_document_expired` flag, and the coordinator and the property manager are told.
- [ ] When a renewed approval is accepted, the contract flag, visit flags and compliance flag
      clear and `ComplianceFlagCleared` is emitted.
- [ ] Reassigning a visit to "BlueFire" records `vendor_document_expired` and removes that visit's
      flag; the Tower A compliance flag stays.
- [ ] Activating replacement contract AMC-2026-0020 with "BlueFire" covering Tower A fire alarm
      clears the Tower A `vendor_document_expired` flag (cleared by "replacement contract
      `active`") while AMC-2026-0011 keeps its vendor document flag.
- [ ] When AMC-2026-0011 is terminated with no replacement, on the termination date the
      `vendor_document_expired` flag clears and `no_contract` is raised (BL-TKT-04).

#### Out of scope
The guards (BL-TKT-06); contract end (BL-TKT-04).

---

### BL-TKT-08 — wl-ticketing: put a blocked vendor's service contracts under review

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PTY-03, BL-PTY-04, BL-TKT-01 · **Blocks:** BL-PRP-27
**Review items:** X-18

#### Goal
When a vendor party is blocked, its service contracts are under review: no new visits are
assigned to it, and its planned visits are listed for the facility manager (the decider) to
reassign. Nothing ends automatically; ending is the manager's `terminate`.

#### Surfaces
- **Back office:** the facility manager's review task lists the blocked vendor's planned
  visits with "Reassign" and "Terminate contract" ("On hold" only); assigning a ticket to
  the blocked vendor is refused.
- **System:** `assign` to a vendor calls `BlockGuard::check(vendor, start)`, and new planned
  visits of its contracts are created unassigned.

#### Files to read
- `docs/modules/wl-parties.md` §5.6 (row "Service contracts (party as vendor)")
- `docs/modules/wl-ticketing.md` §4.1 (`assign` guard), §4.6, §5.6

#### Terms
- **Review effect:** nothing changes on its own; the decider gets a task.
- **Facility manager:** the decider role (01: property manager / maintenance lead per vertical
  config).

#### Spec excerpt (authoritative for this issue)

| Agreement kind | Owner | Default effect | Stop (end) decided by | Verticals |
|---|---|---|---|---|
| Service contracts (party as vendor) | `wl-ticketing` | review: no new visits assigned; planned visits listed for reassignment | facility manager | 01, 02 |

01 §7.1 checklist: Ticket — party blocked: "vendor: review".

| Transition | From | To | By | Guards |
|---|---|---|---|---|
| `assign` | open, assigned | assigned | `tickets.assign` | …; when the assignee is a vendor party, `BlockGuard::check(vendor, start)` passes: assigning work to a vendor counts as starting new business with it (`PTY.ACL.03`) |

#### Edge cases decided
- **"No new visits assigned":** `assign` to the blocked vendor is refused. `assign` with a
  vendor assignee calls `BlockGuard::check(vendor, start)` (BL-PTY-04): assigning work to a
  vendor counts as starting new business with it (spec §4.1 `assign` guard, `PTY.ACL.03`). A
  reassignment *away* from the blocked vendor only checks the new assignee. The contract's
  plans do not auto-assign the blocked vendor; new planned visits are created unassigned (spec
  §4.1 "Service contract coverage").
- **Visits already `in_progress`** continue.
- **The review task** comes from BL-PTY-03; this issue provides the list of planned visits and
  the actions (reassign with reason, terminate the contract).
- **Restricted reason:** the task and visits show only "On hold".
- **Unblock:** the vendor can be assigned again; unassigned planned visits are listed for the
  coordinator.

#### Must not
- Must not terminate contracts or cancel visits automatically.
- Must not show the block reason to the coordinator or the vendor.

#### Done when
- [ ] Given vendor "CoolAir" blocked with `fraud_suspected` and contract AMC-2026-0003 with 4
      planned visits, the facility manager's task lists the 4 visits with "Reassign" and
      "Terminate contract".
- [ ] Assigning a new ticket to CoolAir is refused; the next planned visit is created unassigned.
- [ ] A visit in progress by CoolAir can still be resolved.

#### Out of scope
The block and the generic review task (BL-PTY-01, BL-PTY-03).

---

### BL-TKT-09 — wl-ticketing: waive a task when its agreement ends or a started task no longer applies

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: wl-ticketing Task lifecycle · **Blocks:** BL-ACC-06, BL-ACC-15, BL-ACC-17, BL-ACC-24
**Review items:** 04-N8-b, 04 MEDIUM (ending mid-period)

#### Goal
Recurring tasks must stop when their source agreement ends (e.g. a 04 engagement `end`): the
system waives them and no next occurrence is generated. A setting decides whether a started
task (in progress or waiting) may also be waived; 04 keeps it off, so its started tasks are
listed for the manager instead.

#### Surfaces
- **Back office:** a `tasks.manage` holder waives a task with a reason; with *waive a
  started task* off (04) the manager gets one task listing the started tasks of the ended
  agreement.
- **System:** when the source agreement ends, `upcoming` tasks are waived with "source
  agreement ended" and no next occurrence is generated.

#### Files to read
- `docs/modules/wl-ticketing.md` §4.4 (`waive`, diagram, vertical mapping), §7 (setting "waive a
  started task")
- `docs/verticals/04-accounting.md` §7.1 *Task* (`waive`)

#### Terms
- **Source agreement:** the record the task template is attached to (04: engagement).
- **Started task:** a task in `in_progress` or `waiting`.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `waive` | upcoming, in_progress, waiting | not_required | `tasks.manage` or system (the source agreement ended, e.g. engagement `end`) | reason given; from `in_progress` or `waiting` only when *waive a started task* is on (§7) | next occurrence generated, unless the source agreement ended; from a started task: work recorded so far kept, assignee and counterpart told |

Diagram additions: `in_progress --> not_required: waive`, `waiting --> not_required: waive`.

| Setting | Default |
|---|---|
| Tasks: *waive a started task* allowed | on (04: off) |

04 mapping: "it keeps *waive a started task* off (a started task that no longer applies is
listed for the manager, who completes it or closes it as missed once overdue)".

#### Edge cases decided
- **System waive on agreement end** applies to `upcoming` tasks always, and to started tasks
  only when the setting is on; with it off (04), started tasks are listed for the manager on a
  task (one task listing the started tasks).
- **`in_review` and `overdue` tasks** cannot be waived (not in From).
- **No next occurrence** when the agreement ended, even for `tasks.manage` waives after the end.
- **Reason for system waive:** "source agreement ended".
- **Milestone note:** only 04 uses tasks in its MVP, with the setting off; the "on" path is built
  here but not used by any MVP vertical.

#### Must not
- Must not waive started tasks while the setting is off.
- Must not generate a next occurrence after the source agreement ended.

#### Done when
- [ ] Given engagement E-12 with tasks "VAT Q4" `upcoming` and "CT return" `in_progress`, when E-12
      ends, "VAT Q4" is `not_required` with reason "source agreement ended" and no next
      occurrence; with the 04 setting off, "CT return" stays `in_progress` and is listed for the
      manager.
- [ ] With the setting on, the same end also waives "CT return", keeps its recorded work, and
      tells the assignee and counterpart.
- [ ] `waive` without a reason by `tasks.manage` is refused.

#### Out of scope
Held tasks (BL-TKT-10); 04's `ACC.RUL.10` regeneration (`Epic 04 Accounting`).

---

### BL-TKT-10 — wl-ticketing: hold tasks while their source agreement is paused

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** Base: wl-ticketing Task lifecycle · **Blocks:** BL-ACC-03, BL-ACC-24
**Review items:** 04-N8-a, 04 MEDIUM (suspension and retainer)

#### Goal
While a task's source agreement is paused (a 04 engagement `suspend`, including a party block
with effect pause), its tasks carry a *held* flag. They keep their state and due date; client
reminders and new request lists stop; overdue marking still runs because the due date is a fact.
When the agreement resumes, the flag clears and the manager gets the list of what went overdue
meanwhile.

#### Surfaces
- **Back office:** held tasks show the held flag (no reason for a restricted block); on
  release the manager gets the list of tasks that went overdue while held.
- **System:** `Tasks::hold(source)` and `Tasks::release(source)` set the flag; client
  reminders and new request lists stop while `mark_overdue` keeps running.

#### Files to read
- `docs/modules/wl-ticketing.md` §4.4 (paragraph "Held tasks", vertical mapping)
- `docs/verticals/04-accounting.md` §7.1 *Engagement* (`suspend`)
- `docs/modules/wl-portal.md` §5.5 (request lists, `POR.RUL.01`)

#### Terms
- **Held flag:** a boolean on the task, not a state.
- **Client reminders:** reminders to the client about the task's open items (e.g. `POR.RUL.01`).

#### Spec excerpt (authoritative for this issue)

"**Held tasks.** While the source agreement is paused (e.g. a 04 engagement `suspend`, including
a party block with effect pause), its tasks carry the *held* flag (not a state). A held task keeps
its state and due date; client reminders and new request lists stop; `mark_overdue` still runs
because the due date is a fact. The flag clears when the agreement resumes, and anything that went
overdue meanwhile is listed for the manager."

04 mapping: "Its tasks are held while the engagement is suspended".

#### Edge cases decided
- **API:** `Tasks::hold(source)` and `Tasks::release(source)` called by the
  owning module on pause and resume.
- **New occurrences generated while held** are created held.
- **Owner notifications** of `mark_overdue` still go to staff; only messages to the client stop.
- **The overdue list** on release covers tasks whose `mark_overdue` ran between hold and
  release; sent once to the manager.
- **Agreement ends while held:** BL-TKT-09 applies; no release list.
- **Restricted block:** the held flag shows no reason to client-facing staff.

#### Must not
- Must not change task state or due date when holding.
- Must not stop `mark_overdue` for held tasks.
- Must not send client reminders or create request lists for held tasks.

#### Done when
- [ ] Given engagement E-12 suspended on 2026-11-01 with task "VAT Q3" due 2026-11-28 and an open
      document request, the task is held, its due date stays 2026-11-28, and no client reminder
      is sent on 2026-11-05.
- [ ] On 2026-11-29 the task becomes `overdue` (and stays held).
- [ ] When E-12 resumes on 2026-12-03, the flag clears and the manager gets a list with "VAT Q3
      (overdue since 2026-11-29)".
- [ ] A task generated on 2026-11-15 while held is created held.

#### Out of scope
The 04 engagement transitions (`Epic 04 Accounting`); request list reminders themselves
(`POR.RUL.01`, base).

---

### BL-TKT-11 — wl-ticketing: seed the ticket categories that other modules open for negative flows

**Labels:** `module:wl-ticketing` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-ticketing Ticket lifecycle · **Blocks:** BL-MPW-20, BL-AUT-06, BL-CON-15, BL-PRP-13, BL-PRP-21, BL-PRP-27
**Review items:** X-02, X-11, X-15

#### Goal
Other modules open tickets for their negative flows: invoice disputes, labour cases after a
settlement dispute, delivery claims. These platform categories are seeded whenever the opening
module is enabled, with fixed properties, so the opening module can rely on them. The vertical
may rename them but not remove them.

#### Surfaces
- **Back office:** the platform categories appear in the ticket category settings; renaming
  works and deleting is refused; *labour case* tickets are visible only with
  `tickets.confidential.view`.
- **System:** the categories are seeded with fixed keys when the opening module is enabled,
  and *billing dispute* resolves its assignee from the invoice owner.

#### Files to read
- `docs/modules/wl-ticketing.md` §3 (*Ticket category*), §7 (setting "Platform categories opened
  by other modules"), §10 (Confidential tickets)

#### Terms
- **Platform category:** a ticket category owned by the platform, seeded per enabled module.
- **Invoice owner:** the user who ran the invoice's `issue`; when the system issued it (e.g. a
  subscription cycle) or that user is no longer active, the holders of the receivables role in
  the invoice's scope, i.e. holders of `invoices.issue` (wl-invoicing §3 *Invoice owner*).
  `wl-invoicing` passes the invoice; the category resolves the assignee.
- **Confidential category:** visible only with `tickets.confidential.view` and to the requester
  (`WRK.ACL.03`).

#### Spec excerpt (authoritative for this issue)

| Setting | Default |
|---|---|
| Platform categories opened by other modules (seeded when the opening module is on; the vertical may rename them, not remove them; priority and SLA are the category defaults, Normal, until the vertical sets them) | *billing dispute* (`wl-invoicing` `dispute`; assignee: invoice owner, i.e. the user who ran the invoice's `issue`, or, when the system issued it or that user is no longer active, the holders of the receivables role in the invoice's scope); *labour case* (`wl-hr` settlement `dispute`; confidential, assignee role HR, visible to HR, the accountant and MD only; custom fields: forum (MoHRE, court), reference, hearing or reply dates; no requester sign-off); *delivery claim* (`wl-delivery` `open_review` and lost deliveries) |

#### Edge cases decided
- **Stable key:** each category has a fixed key (`billing_dispute`, `labour_case`,
  `delivery_claim`) used by the opening module; renaming changes only the labels EN/AR.
- **Module enabled later:** its category is seeded at the next boot.
- **"Visible to HR, the accountant and MD only":** the category is confidential; 09 grants
  `tickets.confidential.view` only to HR / payroll officer, accountant and admin / MD
  (09 §3.2, seeded in BL-MPW-20).
- **Other properties** (priority, SLA) use the category defaults (Normal priority, 1 day / 7 days)
  until the vertical sets them (spec §7).
- **Vertical categories:** categories that only the vertical opens (e.g. 08 *vehicle incident*,
  wl-ticketing §4 vertical mapping row 08) are seeded by the vertical's starter data (BL-AUT-06),
  not here.

#### Must not
- Must not let a client disable or delete a platform category.

#### Done when
- [ ] With wl-invoicing, wl-hr and wl-delivery enabled, the three categories exist with the
      properties above.
- [ ] With wl-delivery disabled, *delivery claim* does not exist.
- [ ] Deleting *billing dispute* is refused; renaming it to "Invoice query" works and
      `wl-invoicing` still opens tickets in it.
- [ ] A labour case ticket is not visible to a site supervisor.
- [ ] A dispute on invoice INV-ACME-2026-0310 issued by accountant "Nadia" opens a billing
      dispute ticket assigned to Nadia; on an invoice issued by a subscription cycle, or after
      Nadia is deactivated, it is assigned to the receivables role in the invoice's scope.

#### Out of scope
Opening the tickets (`Epic wl-invoicing: invoice dispute`, `Epic wl-hr: settlement dispute`,
`Epic wl-delivery: review and lost deliveries`).
