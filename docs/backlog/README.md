# Backlog — Negative Flows

> Status: in use. F1–F10 are built and merged; the open items are in §7 · Last updated: 2026-10-24

GitHub issues for building the negative flows fixed in the
[negative-flow review](../reviews/2026-10-15-negative-flows.md). They were **created on GitHub
on 2026-10-16** (§6, [github-issues.md](github-issues.md)). Each file is one epic (a module or a
vertical) with its issues.

The issues are written so that a small coding model or a new developer can
implement one without reading the whole specification. A format test on 2026-10-15 gave two
small models three sample issues; both planned them correctly, and their questions were fixed in the
specification before this backlog was written.

## 1. Scope

- **In:** the negative flows: states, transitions, guards, effects, rules and access rules added
  or changed by the review fixes.
- **Also in:** the capabilities G1–G7 approved from the
  [SME problem map](../reviews/2026-10-16-sme-problem-map.md) (label `area:sme-gap`); their
  **Review items** line names the gap and the problem rows it solves.
- **Not in:** the happy path of each entity (create, issue, pay…). There is no separate
  happy-path backlog: the `Base: <module> <Entity> lifecycle` lines under **Depends on** name
  lifecycles that were built together with the verticals (F1–F10), not issues of this backlog.
- The specification documents stay the source of truth. The excerpt in an issue is a copy for
  convenience; if it differs from the document in `docs/` at the commit that last changed the
  backlog file, the document wins and the issue is corrected.

## 2. Files

| File | Epic |
|---|---|
| [modules/wl-kernel.md](modules/wl-kernel.md) | Platform patterns (WORKFLOW §8): void, approval returned, integration down, late entry, owner alert |
| [modules/wl-parties.md](modules/wl-parties.md) | Party block effects, restricted reasons, sanctions freeze and list freshness |
| [modules/wl-documents.md](modules/wl-documents.md) | Expired documents block transitions; overrides |
| [modules/wl-notify.md](modules/wl-notify.md) | Give-up reactions |
| [modules/wl-portal.md](modules/wl-portal.md) | Neutral text for blocked parties |
| [modules/wl-ticketing.md](modules/wl-ticketing.md) | Service contract end, vendor no-show, compliance flags, task waive, invoice timing |
| [modules/wl-invoicing.md](modules/wl-invoicing.md) | Write-off, dispute, unallocate, e-invoice transmission, supplier debit note, payer bill negatives, subscription pause |
| [modules/wl-payments.md](modules/wl-payments.md) | Payment reversal and chargeback, cheque after bounce, failed refund, cash custody, deposit transfer and dispute |
| [modules/wl-sales.md](modules/wl-sales.md) | Order amend, cancel after fulfilment, credit re-check, refunds after completion, payment-first orders |
| [modules/wl-delivery.md](modules/wl-delivery.md) | Partial proof of delivery, review, lost, reversal, perishables, run abort and hand-over, COD difference |
| [modules/wl-inventory.md](modules/wl-inventory.md) | Receipt rejection, return to supplier, PO revise, holds, recall, negative stock, landed cost adjustment, dispatch reactions |
| [modules/wl-scheduling.md](modules/wl-scheduling.md) | Cancel after check-in, business cancel, bulk move, eligibility conflicts, last seat |
| [modules/wl-attendance.md](modules/wl-attendance.md) | Adjustment timesheets, flags, deemed approval, payroll at cut-off |
| [modules/wl-hr.md](modules/wl-hr.md) | Payroll return and WPS rejection, correction runs, employee exits, settlement dispute, pay account control |
| [modules/wl-agents.md](modules/wl-agents.md) | Global search, related records and AI agents ([decision 0006](../decisions/0006-search-and-ai-agents.md)); not from a review |
| [verticals/01-property.md](verticals/01-property.md) … [verticals/10-edtech.md](verticals/10-edtech.md) | The vertical's own negative flows and its settings for the shared ones |

## 3. Issue IDs, labels and milestones

**ID.** `BL-<CODE>-NN`, unique and never reused. Dependencies refer to these IDs; when the
issues are created on GitHub, the IDs stay in the title prefix so links can be rewritten.

| Code | Epic | Code | Epic |
|---|---|---|---|
| `KRN` | wl-kernel | `PRP` | 01 Property |
| `PTY` | wl-parties | `CON` | 02 Construction |
| `DOC` | wl-documents | `LOG` | 03 Logistics |
| `NTF` | wl-notify | `ACC` | 04 Accounting |
| `POR` | wl-portal | `CLN` | 05 Clinics |
| `TKT` | wl-ticketing | `FNB` | 06 F&B |
| `INV` | wl-invoicing | `MFG` | 07 Manufacturing |
| `PAY` | wl-payments | `AUT` | 08 Auto |
| `SAL` | wl-sales | `MPW` | 09 Manpower |
| `DLV` | wl-delivery | `EDU` | 10 EdTech |
| `STK` | wl-inventory | | |
| `SCH` | wl-scheduling | | |
| `AGT` | Search and AI agents (wl-kernel, wl-portal, wl-notify; no crate of its own) | | |
| `ATT` | wl-attendance | | |
| `HRM` | wl-hr | | |

**Labels.**

| Label | Meaning |
|---|---|
| `module:<name>` or `vertical:<NN-name>` | Where the work lives |
| `area:negative-flow` | Issues from the [negative-flow review](../reviews/2026-10-15-negative-flows.md) |
| `area:sme-gap` | Issues for capabilities G1–G7 added from the [SME problem map](../reviews/2026-10-16-sme-problem-map.md) |
| `area:ai-agents` | Search, related records and AI agents ([decision 0006](../decisions/0006-search-and-ai-agents.md)) |
| `priority:high` / `priority:medium` | From the review's impact: money, legal or data loss = high |
| `blocked:open-item` | Waits for an answer in [OPEN-ITEMS.md](../OPEN-ITEMS.md); the default in the issue may be built, the open part is flagged |
| `size:s` / `size:m` | s = one transition or rule; m = a small state machine or several linked transitions. Nothing larger: split it |
| `surface:back-office` | Screens or actions for staff on desktop ([UX.md](../UX.md) §1, surface 1) |
| `surface:field-pwa` | Phone screens for field roles: technician, driver, supervisor, storekeeper, line operator, cook (surface 2) |
| `surface:portal` | What a logged-in external user sees or does: tenant, client, patient, guardian, student, buyer (surface 3) |
| `surface:public` | Pages without login: self-booking, direct ordering, tracking link, payment link (surface 4) |
| `surface:system` | No screen: automation rules, event consumers, scheduled jobs, and guards or effects that run without a user (e.g. a system transition). A guard that refuses a user's own action belongs to that user's surface, not to `system` |

**Milestones.**

| Milestone | Contains |
|---|---|
| **Build 1** | Phase F1: 01 Property, and every module issue that 01 needs in its MVP |
| **Launch NN** | Phase F*n* (02–10, [decision 0005](../decisions/0005-phases-follow-vertical-numbers.md)): the vertical's own issues, and module issues first needed by that vertical |
| **Later** | Issues for capabilities marked Later |

A module issue takes the earliest milestone of any vertical that needs it.

**Built early.** An issue built before its milestone keeps the milestone (it says which vertical
first needs it). Examples: the sanctions screening issues BL-PTY-05 to BL-PTY-09
(#38–#42, Launch 04) and the marketing broadcast BL-NTF-07 (#61, Launch 05) were built in
Build 1; other module issues were built in a later phase than their milestone (for example
BL-HRM-30, #480, Launch 09). No list is kept; the issue's state on GitHub is the record.

Milestones and [OPEN-ITEMS](../OPEN-ITEMS.md) gates are different things. A milestone says
when an issue is built; a gate says when an open question must be answered. An issue labelled
`blocked:open-item` whose item has the gate **Launch NN** is built in its milestone with the
default it states (the open part flagged in the app); the answer is needed before selling
vertical NN, not before building the issue. Only items with the gate **Build 1** must be
answered before the issue is coded.

## 4. Issue template

Every issue uses exactly these headings, in this order (**Surfaces** added 2026-10-16). Sections with nothing to say keep the
heading and write "None."

````markdown
### BL-XXX-NN — <module or vertical>: <what it does, as a verb phrase>

**Labels:** … · **Milestone:** … · **Size:** s | m
**Depends on:** BL-… · Base: … · **Blocks:** BL-…
**Review items:** X-.., D-.., NN-Nx-y (from the negative-flow review)

#### Goal
Two to four sentences: who does what, what must happen, why it matters.

#### Surfaces
One line per surface the issue touches, naming the role and what appears or is done there
(back office, field PWA, portal, public page, system). Every surface named here has its
`surface:*` label. Notifications are effects, not surfaces: name the channel in the effect.

#### Files to read
Paths with sections (`docs/modules/wl-invoicing.md` §4 Invoice). Only what is needed.

#### Terms
Every ID, state or word a newcomer would not know, defined in one line each.

#### Spec excerpt (authoritative for this issue)
The exact rows copied from the document: states table, transitions table (only the rows this
issue implements, plus the rows it changes), rules and access rules with their parameters and
defaults. Then the effects as a numbered list when a table cell is long.

#### Edge cases decided
Each predictable question with its answer: concurrent changes, amounts changing while an
approval waits, rounding, repeated runs (idempotency), time zone (Asia/Dubai), what happens
at period ends, what happens when the record already moved. If the specification does not
answer one, say so, give the default the spec implies, and add `blocked:open-item` only when
an OPEN-ITEMS ID covers it.

#### Must not
Bullet list of the mistakes to avoid (writing the status directly, skipping SoD, issuing
documents the spec forbids…).

#### Done when
Checkboxes in Given / When / Then form, with concrete values (amounts in AED, dates, users
"A" and "B"). One per guard, one per effect, one per must-not where testable. These become
the tests ([TESTING.md](../TESTING.md)).

#### Out of scope
What a reader might expect here but belongs to another issue (with its ID).
````

## 5. How the backlog was checked

1. Written per epic from the specification as it stood on 2026-10-16.
2. **Question pass:** a small coding model read each epic as a new developer and listed every
   assumption and question.
3. Each question was answered in the specification or in the
   issue's **Edge cases decided**. Questions that need an adviser point to OPEN-ITEMS.

## 6. Issues on GitHub

Created on 2026-10-16: 24 epics (#1–#24) and 429 issues (#25–#453), each issue a sub-issue of
its epic, with the labels and milestones of §3. Added on 2026-10-20: 22 Build 1 issues (#458–#479)
for the work the closed F1 issues deferred, and BL-HRM-30 (#480, Launch 09) for the effects of
`report_absconding` that need payroll, so the backlog has 452 issues. Added on 2026-10-22: 12
Launch 09 issues (#502–#513, BL-MPW-22 to BL-MPW-33) for the Manpower screens that the browser
test of the F9 app found missing, so the backlog has 464 issues (BL-MPW-33, the Arabic
translation, is created but not built in the same round). They use `area:sme-gap` as the
closest existing area label; there is no `area:screens`. Added on 2026-10-23: BL-MPW-34 (#518,
Launch 09), the pay accounts waiting for approval in the approvals inbox, so the backlog has 465
issues. Added on 2026-10-24 from [decision 0006](../decisions/0006-search-and-ai-agents.md): the
epic #540 and BL-AGT-01 to BL-AGT-10 (#541–#550: five Build 1, five Later), label
`area:ai-agents`, so the backlog has 475 issues. Added the same day, after F1–F10 were merged:
BL-AGT-11 to BL-AGT-13 (#552–#554, milestones Launch 02 and Launch 06 but not built with those
launches: see §7), the search registrations of the modules and verticals built in F2–F10, so
the backlog has 478 issues. Links between issues
use the GitHub numbers.
The mapping from backlog ID to issue is in [github-issues.md](github-issues.md). When a backlog
file changes, update the matching GitHub issue too.

Not backlog issues: the repository also holds issues without a `BL-` ID. They are not in
[github-issues.md](github-issues.md) and do not change the numbering: the standard-grid tracking
issues (#523 and #524–#538, #556, #557, #564), defect reports from browser tests (#491–#496,
#558, #565, #566) and a UX issue (#567). Counts in this file are of `BL-` issues only:
429 + 22 + 1 + 12 + 1 + 10 + 3 = 478, equal to the `### BL-` blocks in the 25 files.

## 7. State (2026-10-24)

**Built.** Phases F1 to F10 are built and merged to `main` (pull requests from `git log --merges`):

| Phase | Milestone | Pull requests |
|---|---|---|
| F1 | Build 1 (01 Property) | #456 (Build 1 issues), #482 (demo and deploy) |
| F2 | Launch 02 (02 Construction) | #486, #487 (HTML screens) |
| F3 | Launch 03 (03 Logistics) | #498 |
| F4 | Launch 04 (04 Accounting) | #484, #485 (deferred gaps), #497 (activation blockers) |
| F5 | Launch 05 (05 Clinics) | #499, #514, #515 (demo data, staff screens) |
| F6 | Launch 06 (06 F&B) | #520 |
| F7 | Launch 07 (07 Manufacturing) | #521 |
| F8 | Launch 08 (08 Auto) | #522, #561 (demo and grids) |
| F9 | Launch 09 (09 Manpower) | #500, #501, #516, #519 |
| F10 | Launch 10 (10 EdTech) | #517 |

"Built" means the issue's work is merged. It does not mean every Done-when point is proved: points
that wait for a provider are listed as deferred (below). Nothing is marked "planned" for a
module or vertical that exists: all ten `apps/*` verticals and the `crates/wl-*` modules exist.

**Still open on GitHub, and why.**

| Issues | What | Why open |
|---|---|---|
| #458–#479 (22) | Build 1 follow-ups: BL-KRN-10/11, PTY-11, POR-02/03, DOC-11, NTF-10, INV-31, PAY-27 to 30, SAL-27/28, TKT-12 to 14, PRP-30 to 34 | Created on 2026-10-20 for work the closed F1 issues deferred; not built |
| #541–#550, #552–#554 (13) and epic #540 | BL-AGT-01 to 13: search, related records, agent connector | Not built (no search or related-records code exists). Milestone note below |
| #502, #503, #505, #506, #507, #509, #510, #513 | BL-MPW-22, 23, 25, 26, 27, 29, 30, 33: Manpower screens | Screens are built except the Arabic translation (#513, not built); the others stay open for points marked `**Deferred:**` in the issue body |
| #207, #213, #278, #279, #298, #346, #434, #435 | BL-HRM-17, HRM-23, PRP-28, PRP-29, CON-19, ACC-22, MPW-20, MPW-21 | Built in part; each has one or more points deferred (a kernel contract, a provider) |
| #523–#538, #556, #557, #564 | Standard-grid conversion (not `BL-` issues) | List screens move to the standard grid module by module |
| #491–#496, #558, #565–#567 | Defects and UX reports from browser tests (not `BL-` issues) | Open reports |
| Epics #1–#24 | Epics | Stay open while a child is open |

**BL-AGT-11, -12, -13 and their milestone.** These three carry the milestones Launch 02 (11, 12)
and Launch 06 (13) because they register the records of the F2 to F10 modules and verticals.
Those launches are already built, while the three issues depend on BL-AGT-01 and BL-AGT-03
(Build 1), which are not built. The honest reading: the milestone names the records, not the
build time. AGT-01 to AGT-13 are one track: BL-AGT-01 and -03, then -02, -04, -05, then -11, then
-12 and -13 ([modules/wl-agents.md](modules/wl-agents.md)). The GitHub milestone of #552–#554 is
not changed by this file.

**Where the build state and the deferred points live.**

- The GitHub issue body: ticked Done-when points, `**Deferred:**` lines, one
  `**Verification summary**` comment (the rule of the `build-issues` skill, `.claude/skills/build-issues`).
  The backlog files are the text as created; they are not updated with ticks.
- `docs/progress/`: the deferred lists of the phases, for example
  [f8-auto.md](../progress/f8-auto.md) and [f8-auto-deferred.md](../progress/f8-auto-deferred.md)
  (08 Auto and the module issues #134, #136, #111) and the F7 list in
  `docs/progress/f7-manufacturing.md` (07 Manufacturing, #132, #390, #394, #395). The F7 list is
  also in the "Deferred in F7" table of `apps/manufacturing/README.md`. Blocks of issues with
  deferred points carry a pointer line under **Done when**.
- [DEMO-DATA.md](../DEMO-DATA.md) and the `*:demo` commands: the executable evidence per vertical.
- Leftovers of the other open issues in the table above are in the `**Deferred:**` lines of
  their GitHub bodies; there is no repo list yet.
