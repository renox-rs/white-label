# Backlog — Negative Flows

> Status: draft · Last updated: 2026-10-18 · Spec version: same commit as this file

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
- **Not in:** the happy path of each entity (create, issue, pay…). Those come in a separate
  backlog. An issue that needs one names it under **Depends on** as
  `Base: <module> <Entity> lifecycle`.
- The specification documents stay the source of truth. The excerpt in an issue is a copy for
  convenience; if it differs from the document at the spec version above, the document wins
  and the issue is corrected.

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
| `ATT` | wl-attendance | | |
| `HRM` | wl-hr | | |

**Labels.**

| Label | Meaning |
|---|---|
| `module:<name>` or `vertical:<NN-name>` | Where the work lives |
| `area:negative-flow` | Issues from the [negative-flow review](../reviews/2026-10-15-negative-flows.md) |
| `area:sme-gap` | Issues for capabilities G1–G7 added from the [SME problem map](../reviews/2026-10-16-sme-problem-map.md) |
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

1. Written per epic from the specification at the spec version above.
2. **Question pass:** a small coding model read each epic as a new developer and listed every
   assumption and question.
3. Each question was answered in the specification (and the spec version updated) or in the
   issue's **Edge cases decided**. Questions that need an adviser point to OPEN-ITEMS.

## 6. Issues on GitHub

Created on 2026-10-16: 24 epics (#1–#24) and 429 issues (#25–#453), each issue a sub-issue of
its epic, with the labels and milestones of §3. Links between issues use the GitHub numbers.
The mapping from backlog ID to issue is in [github-issues.md](github-issues.md). When a backlog
file changes, update the matching GitHub issue too.
