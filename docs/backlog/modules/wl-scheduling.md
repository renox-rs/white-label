# Epic: wl-scheduling — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the booking negative flows added by the review fix X-17: cancelling after
check-in with work done, business-initiated cancellations that refund the deposit, reason codes,
reassigning a booking, bulk moves when a resource is unavailable, look-ahead eligibility
conflicts, and the last-seat and unpaid-hold rules. It closes X-17, 05-N8-a, the 05 MEDIUM items
"walk-out after check-in", "clinic-side cancellation and deposit refund" and "practitioner sick
on the day", the 10 MEDIUM items "capacity under concurrency" and "tutor reassignment", and the
booking consumers of `PaymentFailed` and pending timeouts from X-06.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-SCH-01 | Record cancellation reason codes and who initiated | medium | s | Launch 05 | Base: wl-scheduling Booking lifecycle |
| BL-SCH-02 | Settle the booking deposit by who cancelled (`SCH.ACL.05`) | high | m | Launch 05 | BL-SCH-01; Epic wl-payments: Refund `request` |
| BL-SCH-03 | Cancel a booking after check-in (`SCH.ACL.06`) | high | m | Launch 05 | BL-SCH-01; BL-SCH-02 |
| BL-SCH-04 | Reassign a booking to another resource at the same time | medium | s | Launch 05 | Base: wl-scheduling Booking lifecycle; BL-SCH-01; BL-SCH-09 |
| BL-SCH-05 | Raise booking conflicts and resolve them in bulk | high | m | Launch 05 | BL-SCH-01; BL-SCH-02; BL-SCH-04; BL-HRM-02 |
| BL-SCH-06 | Escalate open conflicts and stop reminders and no-shows (`SCH.RUL.06`) | medium | s | Launch 05 | BL-SCH-05 |
| BL-SCH-07 | Flag bookings whose resource will not be eligible (`SCH.RUL.05`) | high | m | Launch 05 | BL-SCH-05; BL-HRM-01; Epic wl-documents: `DocumentExpiring` with `affected` |
| BL-SCH-08 | Re-check eligibility at check-in and start | high | s | Launch 05 | Base: wl-scheduling Booking lifecycle; BL-HRM-01; BL-SCH-04 |
| BL-SCH-09 | Give the last seat to exactly one booker | high | m | Launch 05 | Base: wl-scheduling Booking lifecycle |
| BL-SCH-10 | Cancel unpaid holds and refund late payments | high | m | Launch 05 | BL-SCH-01; BL-SCH-09; Epic wl-payments: payment link with expiry, `PaymentLinkExpired`, `PaymentFailed`, `PaymentReceived`, Refund `request` |

---

### BL-SCH-01 — wl-scheduling: record cancellation reason codes and who initiated

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:medium` `surface:back-office` `surface:portal` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** Base: wl-scheduling Booking lifecycle (`cancel`, `decline`) · **Blocks:** BL-SCH-02, BL-SCH-03, BL-SCH-04, BL-SCH-05, BL-SCH-10, BL-CLN-01, BL-CLN-15
**Review items:** X-17

#### Goal
Every cancellation records a reason code and whether the party or the business initiated it.
That one field drives the deposit outcome (BL-SCH-02) and the reports. Each code has a default
*initiated by*; staff may override with a note. `BookingCancelled` carries the full picture.

#### Surfaces
- **Back office:** staff pick a reason code when they cancel (a `decline` records `other`
  with their reason text); the code's default *initiated by* can be overridden only with a
  note; `walked_out` is offered only from `checked_in` or `in_progress`.
- **Portal:** a party who cancels in the portal picks no code; the cancel records
  `party_request`, initiated by `party`.
- **Public:** a party who cancels from the confirmation or reminder link gets
  `party_request`; one who cancels from a change message link (reassign, conflict,
  escalation) is recorded as initiated by `business` with the change's code, also after the
  cut-off.
- **System:** *late* is computed from the forfeit window or the cut-off, and
  `BookingCancelled` carries reason code, initiated by, stage and late.

#### Files to read
- `docs/modules/wl-scheduling.md` §3 (Booking: cancellation), §4.1 *Booking* (`cancel`),
  "Cancellation reason codes", §6 *Events emitted* (`BookingCancelled`), §7

#### Terms
- **Initiated by:** `party` (the customer side) or `business` (the client company).
- **Stage:** before start, checked in, in progress.
- **Cancellation cut-off:** until when the party may cancel by itself (link, portal); default
  24 h before (`SCH.ACL.02`). Later, only staff cancel.
- **Forfeit window:** N hours before the start, set on the service type's booking deposit.
- **Late:** made less than the forfeit window before the start; for a service type without a
  deposit, less than the cancellation cut-off. The two windows are separate settings.
- **Change message:** a message the business sends about a change it made: the reassign
  message (BL-SCH-04), the conflict message (BL-SCH-05) and the escalation (BL-SCH-06).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | requested, booked, confirmed, checked_in, in_progress | cancelled | party (before the cancellation cut-off; never after check-in), `bookings.manage`, or system | reason code given; initiated by `party` or `business` recorded; from `checked_in`/`in_progress`: `bookings.manage` only, and the vertical record opened at check-in (visit, job card) has its work done captured or marked *none* (`SCH.ACL.06`) | slot released (`SlotReleased`) for the remaining time; deposit per `SCH.ACL.05`: business-initiated → refunded in full; party-initiated inside the forfeit window → forfeited, after first settling any work done; walk-out → applied to work done, the remainder forfeited or refunded per the walk-out setting (§7); work done is billed by the vertical (charges from the visit or job card); event `BookingCancelled` |

**Cancellation reason codes.** `party_request`, `walked_out` (party left after check-in),
`unpaid` (prepayment link expired or payment failed; system), `resource_unavailable` (illness,
emergency), `resource_ineligible` (licence or document expired), `site_closed`,
`equipment_failure`, `service_not_suitable` (decided after check-in), `other`. Codes are mapped
to *initiated by*: `party_request`, `walked_out` and `unpaid` are party-initiated; the others are
business-initiated unless staff record otherwise with a note. Verticals may add codes.

| Event | When | Payload |
|---|---|---|
| `BookingCancelled` | `cancel`, `decline` | booking, by (party/staff/system), initiated by (party/business), reason code, stage (before start, checked in, in progress), work done (none or vertical record reference), late (yes/no), deposit outcome (refunded, forfeited, applied to work done) |

Spec §4.1 (after the codes): "A party who cancels from an ordinary link or the portal gets
`party_request` (initiated by `party`). A party who cancels from a *change message* the
business sent (reassign, §5.5 step 3, `SCH.RUL.06`) cancels as initiated by `business`, with
the reason code of the change (e.g. `resource_unavailable`), and the cancellation cut-off does
not apply to that link. A cancellation is *late* (`BookingCancelled`) when it is made less than
the deposit forfeit window before the start; for a service type without a deposit, less than
the cancellation cut-off. Only a late party-initiated cancellation forfeits a deposit."

Setting: Cancellation reason codes and their *initiated by* = §4.1 list.

#### Edge cases decided
- **Party cancels from an ordinary link** (confirmation, reminder) **or the portal:** reason
  code is `party_request`, initiated by `party`; the party does not pick a code.
- **Party cancels from a change message:** initiated by `business`, reason code of the change
  (the reassign's or the block's code, e.g. `resource_unavailable`); allowed after the
  cancellation cut-off. This is the only way a party's own tap records `business`.
- **`decline`:** `decline` records reason code `other` with the staff's
  reason text, initiated by `business`.
- **Override without a note:** refused.
- **`walked_out` before check-in:** refused; `walked_out` is only offered
  from `checked_in` or `in_progress`.
- **Vertical codes:** a vertical adds a code with its default *initiated by*; it cannot remove
  the module codes.

#### Must not
- Must not let the party choose `business` as initiator (it is set only by the change-message
  link, never picked).
- Must not cancel without a reason code.

#### Done when
- [x] Given a `booked` booking, when staff cancel with `site_closed`, then initiated by is
      `business` and `BookingCancelled` carries reason `site_closed`, initiated by `business`,
      stage "before start".
- [x] When staff cancel with `site_closed` and override to `party` with note "patient asked to
      close early", initiated by is `party`; without a note it is refused.
- [x] When the party cancels from the reminder link, the code is `party_request` and initiated by
      `party`.
- [x] Given a booking reassigned with `resource_unavailable`, when the party cancels from the
      reassign message 3 h before the start (cut-off 24 h), the cancel succeeds with code
      `resource_unavailable`, initiated by `business`.
- [x] Given deposit forfeit window 12 h and cut-off 24 h, a cancel 10 h before has late = yes and
      one 20 h before has late = no; with no deposit, a cancel 20 h before has late = yes.
- [x] Cancel without a code is refused.

#### Out of scope
Deposit outcome (BL-SCH-02); cancel after check-in (BL-SCH-03).

---

### BL-SCH-02 — wl-scheduling: settle the booking deposit by who cancelled (`SCH.ACL.05`)

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:high` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SCH-01 · Epic wl-payments: deposit refund through Refund `request` · **Blocks:** BL-SCH-03, BL-SCH-05, BL-CLN-01, BL-CLN-05, BL-CLN-06, BL-EDU-17
**Review items:** X-17, 05 MEDIUM "clinic-side cancellation and deposit refund"

#### Goal
A booking deposit is kept only when the party is at fault: a party-initiated cancellation inside
the forfeit window, a no-show, or a walk-out where the setting forfeits the remainder. When the
business cancels or reschedules, the deposit is refunded in full (or carried to the new booking).
Waiving a forfeiture needs `bookings.waive_deposit`.

#### Surfaces
- **Back office:** a user with `bookings.waive_deposit` waives a forfeited deposit
  (party-initiated late cancel, walk-out or no-show) with a reason on the booking; users
  without the permission are refused.
- **System:** the deposit outcome follows `SCH.ACL.05` from *initiated by* and *late*:
  refunded in full through a Refund `request`, forfeited, or carried to the new booking on
  reschedule.

#### Files to read
- `docs/modules/wl-scheduling.md` §3 (Service type: booking deposit), §4.1 `cancel`,
  `reschedule`, `mark_no_show`, "Cancel after check-in", §5.2 "Booking deposit instead of a
  no-show fee", §6 *Permissions*, §6 *Access rules* (`SCH.ACL.05`), §7, §10 *Booking deposits*
- `docs/OPEN-ITEMS.md` L-09, L-32

#### Terms
- **Booking deposit:** an amount paid through a payment link at booking, per service type, with
  a forfeit window of N hours before the start.
- **Forfeit:** the business keeps the deposit as income.
- **Carry:** the deposit stays held and is linked to the new booking.
- **Walk-out setting:** per service type, `forfeited` (default) or `refunded` (05 default).

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Source | Status |
|---|---|---|---|---|
| `SCH.ACL.05` | A booking deposit is forfeited only on a party-initiated late cancellation, a no-show, or a walk-out where the walk-out setting forfeits the remainder; a business-initiated cancel or reschedule always refunds it in full (or carries it to the new booking); waiving a forfeiture needs `bookings.waive_deposit` | `STA` | X-17 (05, 08, 10) | new |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `mark_no_show` | booked, confirmed | no_show | system (`SCH.RUL.02`), the resource, or `bookings.manage` | start time + grace period passed; no open conflict on the booking (a conflicted booking is resolved by staff, not marked no-show) | slot released; booking deposit forfeited if the service type requires one; event `BookingNoShow` |
| `reschedule` | requested, booked, confirmed | requested, booked | party (before the reschedule cut-off) or `bookings.manage` | new slot free; resource eligible at the new start | old slot released (`SlotReleased`); returns to `booked`, or to `requested` if staff acceptance is required; a held booking deposit is carried to the new booking (`SCH.ACL.05`); reminders rescheduled; event `BookingRescheduled` |

`cancel` effects (deposit part): "deposit per `SCH.ACL.05`: business-initiated → refunded in
full; party-initiated inside the forfeit window → forfeited, after first settling any work done;
walk-out → applied to work done, the remainder forfeited or refunded per the walk-out setting
(§7)".

| Permission | Allows |
|---|---|
| `bookings.waive_deposit` | Waive a deposit forfeiture for a party-initiated late cancellation, walk-out or no-show |

§5.5 step 3: "A party who cancels from this message cancels as business-initiated (§4.1
reason codes) and gets the deposit back in full."
§10: no specific rule on no-show fees was found `⚠ open: lawyer (L-09)`; whether the party is
owed more than the deposit is `⚠ open: lawyer (L-32)`.

#### Edge cases decided
- **Party cancels outside the forfeit window:** refunded in full (not forfeited).
- **Party cancels from a change message:** recorded as `business` (BL-SCH-01), so refunded in
  full even inside the forfeit window. The deposit rule only reads *initiated by* and *late*.
- **Business reschedule:** carried to the new booking (`reschedule` effect); a refund is made
  only if the party then cancels or asks for it. Party reschedule before the cut-off also carries.
- **Waive after the forfeit is recorded:** a user with `bookings.waive_deposit` turns a
  forfeited outcome into a refund with a reason; audited. Waiving a refund makes no sense and is
  not offered.
- **Refund fails** (wl-payments `RefundFailed`): wl-payments owns the retry; the booking keeps
  outcome `refunded` with the refund reference.
- **No deposit on the service type:** no deposit outcome; the event shows none.
- **Double cancel** (two users at once): the engine runs one; the second finds `cancelled`.
- **Rounding:** refunds are the exact deposit in AED, 2 decimals.

#### Must not
- Must not forfeit a deposit on any business-initiated cancel or reschedule.
- Must not invoice a no-show fee after the event, and never bill a deposit to an insurer.
- Must not waive without the permission.

#### Done when
- [x] Given deposit AED 200 and a 24 h window, when the party cancels 30 h before, then a refund
      of AED 200 is requested and the outcome is `refunded`.
- [x] When the party cancels 10 h before, the outcome is `forfeited`.
- [x] When staff cancel with `equipment_failure` 1 h before, a refund of AED 200 is requested.
- [x] When staff reschedule with a business reason, the deposit is carried to the new booking.
- [x] Given a `no_show`, the deposit is forfeited; user B with `bookings.waive_deposit` waives it
      with a reason and a refund of AED 200 is requested; user C without the permission is
      refused.
- [x] Given a cancel 10 h before with initiated by `business` and code `resource_unavailable`
      (what BL-SCH-01 records for a party cancelling from a change message), a refund of AED 200
      is requested and the outcome is `refunded`. The end-to-end test from the conflict message
      is in BL-SCH-05.

#### Out of scope
Work done after check-in and walk-outs (BL-SCH-03); unpaid holds (BL-SCH-10); the refund itself
(Epic wl-payments).

---

### BL-SCH-03 — wl-scheduling: cancel a booking after check-in (`SCH.ACL.06`)

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:high` `surface:back-office` `surface:portal` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SCH-01 · BL-SCH-02 · **Blocks:** BL-AUT-01, BL-AUT-02, BL-AUT-18, BL-CLN-05
**Review items:** X-17, 05 MEDIUM "walk-out after check-in"

#### Goal
A booking in `checked_in` or `in_progress` could not be cancelled. Now staff (only) can cancel
it with a reason code, after the vertical record opened at check-in (a visit, a job card) has
its work done captured or marked *none*. Work done is billed by the vertical. For a
party-initiated cancel (`walked_out`, or `party_request` when the party asks staff to stop) a
held deposit is applied to the work done first and the rest follows the walk-out setting. A
business cancel never applies the deposit: it refunds it in full and charges nothing unless
the party accepts the work done, which the vertical bills separately.

#### Surfaces
- **Back office:** staff cancel a `checked_in` or `in_progress` booking on the staff cancel
  screen with `walked_out`, `party_request` or a business code; it is refused while the
  vertical record's work done is still pending.
- **Portal:** the party has no cancel action on a booking after check-in.
- **Public:** the cancel link in the party's confirmation or reminder is refused once the
  booking is checked in.
- **System:** the slot is released for the remaining time and the deposit is applied to work
  done first for party-initiated cancels; a business cancel refunds it in full.

#### Files to read
- `docs/modules/wl-scheduling.md` §4.1 `cancel`, "Cancel after check-in", §6 *Access rules*
  (`SCH.ACL.06`), §6 *Events emitted* (`BookingCancelled`), §7 (walk-out, partial price)

#### Terms
- **Vertical record:** the record a vertical opens on `BookingCheckedIn` (05 visit, 08 job card).
- **Work done:** what was delivered before the cancel (consumables used, part of the service);
  captured on the vertical record, or marked *none*.
- **Applied to work done:** the deposit pays the work-done charge first (party-initiated
  cancels only).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | requested, booked, confirmed, checked_in, in_progress | cancelled | party (before the cancellation cut-off; never after check-in), `bookings.manage`, or system | reason code given; initiated by `party` or `business` recorded; from `checked_in`/`in_progress`: `bookings.manage` only, and the vertical record opened at check-in (visit, job card) has its work done captured or marked *none* (`SCH.ACL.06`) | slot released (`SlotReleased`) for the remaining time; deposit per `SCH.ACL.05`: business-initiated → refunded in full; party-initiated inside the forfeit window → forfeited, after first settling any work done; walk-out → applied to work done, the remainder forfeited or refunded per the walk-out setting (§7); work done is billed by the vertical (charges from the visit or job card); event `BookingCancelled` |

| ID | Rule | Pattern | Source | Status |
|---|---|---|---|---|
| `SCH.ACL.06` | A booking in `checked_in` or `in_progress` is cancelled only by `bookings.manage`, with a reason code, and only after the vertical record has its work done captured or marked none; the party cannot cancel after check-in | `STA` | X-17 (05, 08) | new |

"**Cancel after check-in.** From `checked_in` or `in_progress`, only staff cancel. The vertical
record opened at check-in decides what was done: nothing (deposit rules apply as above), or work
done (consumables used, a part of the service delivered). Work done is charged through the
vertical's own record at its normal price or a set *partial* price. A party who leaves is
cancelled by staff with `walked_out`; a party who asks staff to stop is cancelled by staff with
`party_request`. For both (party-initiated) the held deposit is applied to work done first; the
remainder is forfeited or refunded per the walk-out setting (§7; default forfeited, 05:
refunded). For a business-initiated cancel the deposit is never applied: it is refunded in full,
and nothing is charged unless the party accepts the work done, which the vertical then bills
separately."

Settings: Walk-out after check-in: deposit remainder after work done = forfeited or refunded
(per service type); default forfeited (05: refunded). Partial price for work done when cancelled
after check-in = the vertical record's prices.

#### Edge cases decided
- **How the module knows work done is captured:** the vertical registers
  a check (`work_done_status(booking)` returning `captured`, `none` or `pending`); `cancel` from
  `checked_in`/`in_progress` is refused while `pending`.
- **Deposit larger than work done** (deposit AED 200, work AED 80, walk-out): AED 80 applied;
  AED 120 forfeited or refunded per setting.
- **Work done larger than deposit:** deposit fully applied; the vertical bills the rest.
- **Slot release:** only from now to the booking end ("for the remaining time"); if the end has
  passed, nothing is released.
- **Party-initiated codes after check-in:** staff pick `walked_out` (the party left) or
  `party_request` (the party asked staff to stop) on the staff cancel screen; the party itself
  has no cancel action after check-in. Both follow the walk-out deposit logic (spec).
- **Business cancel with work the party accepts:** the vertical bills the work in full on its
  own record; the deposit is not applied to it and is refunded in full (spec).

#### Must not
- Must not let the party (portal, link) cancel after check-in.
- Must not cancel while work done is still pending on the vertical record.
- Must not create charges here: billing stays in the vertical record.

#### Done when
- [x] Given a 05 booking `checked_in` and the visit's work done `pending`, staff `cancel` is
      refused.
- [x] Given work done `none`, reason `walked_out`, deposit AED 200, 05 setting refunded, then
      the booking is `cancelled` and AED 200 is refunded; `BookingCancelled` has stage "checked
      in", work done none, deposit outcome refunded.
- [x] Given work done AED 80, `walked_out`, setting forfeited: AED 80 applied, AED 120 forfeited.
- [x] Given `equipment_failure` (business) and work done AED 80 not accepted by the party: no
      charge, AED 200 refunded.
- [x] Given `equipment_failure` and work done AED 80 accepted by the party: AED 200 refunded in
      full, deposit outcome refunded; the vertical record bills AED 80 (no deposit applied).
- [x] Given work done AED 80, staff cancel with `party_request` (party asked to stop), deposit
      AED 200, setting forfeited: initiated by `party`, AED 80 applied, AED 120 forfeited.
- [x] The party's cancel link on a `checked_in` booking is refused.
- [x] Given `in_progress` ending 11:00, cancelled at 10:20, the slot 10:20–11:00 is released.

#### Out of scope
Vertical billing of work done (verticals 05, 08); deposit rules before check-in (BL-SCH-02).

---

### BL-SCH-04 — wl-scheduling: reassign a booking to another resource at the same time

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:medium` `surface:back-office` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** Base: wl-scheduling Booking lifecycle · BL-SCH-01 · BL-SCH-09 (slot lock) · **Blocks:** BL-SCH-05, BL-SCH-08
**Review items:** X-17, 10 MEDIUM "tutor reassignment"

#### Goal
Staff move a booking to another eligible resource at the same time, keeping its state. The party
is told the new resource (not why) and can accept, pick another time or cancel; cancelling from
that message counts as a business-initiated cancel (BL-SCH-01), so the deposit comes back. Any
open conflict on the booking clears.

#### Surfaces
- **Back office:** staff reassign a booking to another free, eligible resource at the same
  time with a reason code; a busy or ineligible resource is refused.
- **Public:** the party's message names the new resource (not the reason) with *accept*,
  *pick another time* (opens the slot picker) and *cancel* links; cancelling there counts as
  business-initiated.
- **System:** the new slot is taken under the slot lock, the old slot is released, the
  conflict is cleared and `BookingReassigned` is emitted.

#### Files to read
- `docs/modules/wl-scheduling.md` §4.1 `reassign`, vertical mapping (10 row), §6 *Events
  emitted* (`BookingReassigned`), §10 *Licences at the time of service*

#### Terms
- **`ResourceEligibility`:** strategy that answers whether a resource may deliver the service at
  a time (05 adds licence and privileges; 10 uses `EDU.ACL.14`).
- **Conflict:** the booking's flag with cause `block` or `eligibility` (BL-SCH-05).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reassign` | requested, booked, confirmed | same state | `bookings.manage` | reason code given (not shown to the party); same time; new resources free and eligible (`ResourceEligibility` at the booking time) | old resources' slot released; party notified of the new resource with *accept*, *pick another time* and *cancel* links (the reason, e.g. illness, is not disclosed); a party who cancels from that message cancels as business-initiated (§4.1 reason codes); conflict cleared; event `BookingReassigned` |

| Event | When | Payload |
|---|---|---|
| `BookingReassigned` | `reassign` | booking, old resources, new resources, reason code |
| `SlotReleased` | any transition that frees a slot | resource(s), start, end, service type |

10 mapping: "`reassign` from `requested` and `confirmed` (new tutor at the same time).
`EDU.ACL.14` (`qualification_valid` for the course's tutor kinds) is the `ResourceEligibility`
guard on `confirm`, `book` and `reassign`."

#### Edge cases decided
- **Reason code:** required, from the cancellation reason list (e.g. `resource_unavailable`).
  `reassign` takes a reason code for the event; not shown to the party.
- **Multi-resource bookings** (practitioner + room): only the resources being replaced change;
  the others stay.
- **Party taps *accept*:** nothing changes (the booking already moved); *pick another time*
  opens the slot picker (party `reschedule`); *cancel* cancels with initiated by `business` and
  the reassign's reason code, also after the cut-off (BL-SCH-01).
- **Concurrency:** the new resource's slot is locked like a booking, with the BL-SCH-09 lock.
- **Reminders:** stay scheduled at the same times.

#### Must not
- Must not change the time.
- Must not tell the party the reason.
- Must not leave the conflict flag set after a successful reassign.

#### Done when
- [x] Given booking K with Dr A at 10:00 and an open conflict, when staff reassign to Dr B (free,
      licensed) with `resource_unavailable`, K keeps its state, has Dr B, the conflict is cleared,
      Dr A's 10:00 slot is released and `BookingReassigned` is emitted.
- [x] The party message names Dr B, has accept / pick another time / cancel links, and does not
      contain the reason.
- [x] When the party cancels from that message, the code is `resource_unavailable`, initiated by
      is `business` and the AED 200 deposit is refunded in full.
- [x] Reassigning to a resource busy at 10:00, or not eligible on that date, is refused.

#### Out of scope
Bulk actions (BL-SCH-05).

---

### BL-SCH-05 — wl-scheduling: raise booking conflicts and resolve them in bulk

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:high` `surface:back-office` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SCH-01 (reason codes, change-message cancel) · BL-SCH-02 (deposit refund) · BL-SCH-04 · BL-HRM-02 (`record_sick` → `LeaveApproved`) · **Blocks:** BL-SCH-06, BL-SCH-07, BL-EDU-10, BL-EDU-17
**Review items:** X-17, 05 MEDIUM "practitioner sick on the day"

#### Goal
When a new block (leave, a manual *sick* block, a roster change, a holiday) overlaps existing
bookings, the bookings are not cancelled: each gets a conflict flag with cause `block` and
`BookingConflict` lists them. Staff open a conflict list and, in one action, reassign or cancel
(as business-initiated) the selected bookings, or send their parties a reschedule offer (the
bookings stay conflicted until each party picks a slot). Each party gets one message with
*accept*, *pick another time* and *cancel* links.

#### Surfaces
- **Back office:** staff open the conflict list, select bookings and in one action reassign,
  cancel (business-initiated) or send a reschedule offer; the result lists successes and
  failures.
- **Public:** each party gets one message per day with *accept*, *pick another time* (up to
  3 next free slots of the same resource) and *cancel* links; picking a slot reschedules and
  clears the conflict, cancelling refunds the deposit in full.
- **System:** a new block (sick leave, manual *sick* block, roster, holiday) flags the
  overlapping bookings with cause `block` and emits one `BookingConflict`; removing the
  block clears those conflicts.

#### Files to read
- `docs/modules/wl-scheduling.md` §3 (Availability block, Booking: conflict), §5.4 step 3, §5.5,
  §6 *Provides* (`Bookings::conflicts`, `bulk_resolve`), §6 *Events emitted* (`BookingConflict`)
- `docs/modules/wl-hr.md` §4 *Leave request* (`record_sick`)

#### Terms
- **Block:** time a resource is not bookable, with source (manual, leave, roster, session,
  public holiday).
- **Conflict flag:** booking field with cause `block` or `eligibility`; set by `BookingConflict`,
  cleared on reassign, reschedule or cancel.
- **Bulk resolve:** one action applied to many selected conflicted bookings.

#### Spec excerpt (authoritative for this issue)

Booking field: "conflict (flag with cause `block` or `eligibility`, set by `BookingConflict`,
cleared on reassign, reschedule or cancel)".

§5.4 step 3: "A new block that overlaps existing bookings does not cancel them. It lists them for
staff to move (event `BookingConflict`, cause `block`)."

§5.5:
1. "A practitioner, tutor or technician calls in sick. Same-day sick leave from `wl-hr`
   (`LeaveApproved`) or a manual block with reason *sick* blocks the rest of their day and raises
   `BookingConflict` for every affected booking."
2. "Staff open the conflict list and act in bulk: **reassign** each booking to another eligible
   resource at the same time (`reassign`), **reschedule** to the resource's next free slots
   (offered to the party as a choice), or **cancel** as business-initiated. A bulk reschedule
   does not move the bookings: it sends each party up to 3 next free slots of the same resource
   (not held). The booking stays conflicted until the party picks one (the party's `reschedule`
   then runs, re-checking the slot, and clears the conflict) or staff act; `SCH.RUL.06`
   escalates if it is still open."
3. "Each party gets one message with the change and *accept*, *pick another time* and *cancel*
   links. The reason is not disclosed. A party who cancels from this message cancels as
   business-initiated (§4.1 reason codes) and gets the deposit back in full."

| Service / trait | Purpose | Called by |
|---|---|---|
| `Bookings::conflicts(resource?, period?) / bulk_resolve(bookings, action)` | List conflicted bookings; reassign or cancel them in one action, or send their parties a reschedule offer in one action (§5.5 step 2) | front desk, verticals |

| Event | When | Payload |
|---|---|---|
| `BookingConflict` | a new block overlaps bookings; a resource will not be eligible at a booking's start (`SCH.RUL.05`) | resource, cause (`block` or `eligibility`), block or the expiring qualification/document, affected bookings, earliest start |

#### Edge cases decided
- **"Rest of their day":** from the time the sick leave or block is recorded until the end of the
  resource's availability that day (Asia/Dubai). Bookings already `checked_in` or later are not
  flagged.
- **Bulk reschedule** (`bulk_resolve(bookings, reschedule)`): one action sends each selected
  party the offer of up to 3 next free slots of the same resource (spec). Nothing is moved or
  held; each booking stays conflicted. The party's pick runs `reschedule` (if the slot was
  taken meanwhile, the party sees the nearest free slots); staff can still reassign or cancel.
- **Partial failure in bulk:** each booking is its own transaction; the result lists successes
  and failures (e.g. new resource taken meanwhile).
- **Block removed** (leave cancelled): conflicts with cause `block` that
  came from that block are cleared.
- **One message per party:** a party with two affected bookings the same day gets one message
  listing both.
- **Idempotency:** a booking already flagged with cause `block` for the same block is not flagged
  again.

#### Must not
- Must not cancel bookings automatically when a block is added.
- Must not disclose the reason (illness) to the party.

#### Done when
- [x] Given Dr A with bookings at 11:00, 12:00 and 15:00 today, when same-day sick leave is
      recorded at 10:30 (`LeaveApproved`), then all three are flagged (cause `block`) and one
      `BookingConflict` lists them with earliest start 11:00.
- [x] `Bookings::conflicts(Dr A, today)` returns the three.
- [x] When staff bulk-reassign 11:00 and 12:00 to Dr B, both conflicts clear; each party gets one
      message with accept, pick another time and cancel links and no reason.
- [x] When staff bulk-cancel 15:00 with `resource_unavailable`, it is cancelled with initiated by
      `business` and its AED 200 deposit refunded in full.
- [x] If Dr B became busy at 12:00 meanwhile, 11:00 succeeds and 12:00 is reported failed and
      stays conflicted.
- [x] Given Dr A free tomorrow at 09:00, 09:30 and 10:00, when staff bulk-reschedule the 15:00
      booking, its party gets those 3 slots, the booking is not moved and stays conflicted; when
      the party picks 09:30, it is rescheduled to 09:30 and the conflict clears.
- [x] Given a 15:00 booking with deposit AED 200 (forfeit window 24 h), when its party cancels
      from the conflict message at 10:45, it is cancelled with initiated by `business`, code
      `resource_unavailable`, and AED 200 is refunded.

#### Out of scope
Escalation and reminder suppression (BL-SCH-06); eligibility cause (BL-SCH-07).

---

### BL-SCH-06 — wl-scheduling: escalate open conflicts and stop reminders and no-shows (`SCH.RUL.06`)

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:medium` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-SCH-05 · **Blocks:** None
**Review items:** X-17

#### Goal
A conflicted booking must not get a normal reminder ("see you with Dr A") nor be marked no-show.
If the conflict is still open H hours before the start (default 2 h), the branch manager is told
and the party is asked to choose from the change message.

#### Surfaces
- **Back office:** staff `mark_no_show` on a conflicted booking is refused.
- **System:** `SCH.RUL.06` H hours before the start (at once inside H) notifies the branch
  manager (or the `bookings.manage` users) and sends the party the change message;
  `SCH.RUL.01` reminders and `SCH.RUL.02` no-shows skip conflicted bookings.

#### Files to read
- `docs/modules/wl-scheduling.md` §4.1 `mark_no_show`, §5.5 step 4, §6 *Automations*
  (`SCH.RUL.01`, `SCH.RUL.02`, `SCH.RUL.06`), §7

#### Terms
- **Open conflict:** the booking's conflict flag is set.
- **Branch manager:** the users who hold the vertical's manager role in the booking's location
  scope (branch, workshop, campus), resolved by the kernel `ApproverResolver`; if nobody holds
  it, the users with `bookings.manage` in that scope (spec §6 *Automations*).
- **H:** hours before start (default 2).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `SCH.RUL.06` | H hours before the start | booking has an open conflict | notify the branch manager; send the party the change message with *accept*, *pick another time* and *cancel* links; suppress the normal reminder | H = 2 h (at once when the conflict is raised inside H) |
| `SCH.RUL.01` | N hours before a booking | booking `booked` or `confirmed` | send a reminder with confirm/reschedule/cancel links | N = 24 and 2 (05, 10); 24 (08) |
| `SCH.RUL.02` | M minutes after the start time | booking `booked` or `confirmed` (not checked in or completed) | booking `mark_no_show`; notify the front desk; offer the slot to the waiting list | M = 15 |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `mark_no_show` | booked, confirmed | no_show | system (`SCH.RUL.02`), the resource, or `bookings.manage` | start time + grace period passed; no open conflict on the booking (a conflicted booking is resolved by staff, not marked no-show) | slot released; booking deposit forfeited if the service type requires one; event `BookingNoShow` |

§5.5 step 4: "Reminders (`SCH.RUL.01`) are not sent for conflicted bookings."

§6: "*Branch manager* (`SCH.RUL.06`): the users who hold the vertical's manager role in the
booking's location scope (branch, workshop, campus), resolved by the kernel `ApproverResolver`;
if nobody holds it, the users with `bookings.manage` in that scope."

#### Edge cases decided
- **Conflict raised inside H** (e.g. 1 h before): escalate at once.
- **Conflict cleared before H:** nothing sent; normal reminders resume for later reminder times.
- **Repeated runs:** one escalation per booking per conflict.
- **`mark_no_show` by staff** on a conflicted booking is refused too (the guard applies to all).

#### Must not
- Must not send `SCH.RUL.01` reminders to a conflicted booking.
- Must not mark a conflicted booking no-show by any actor.

#### Done when
- [x] Given a conflicted booking at 15:00, at 13:00 the branch manager is notified and the party
      gets the change message; the 2 h reminder is not sent.
- [x] Given a conflict raised at 14:10 for 15:00, escalation is sent at 14:10.
- [x] At 15:15 `SCH.RUL.02` does not mark it no-show; a staff `mark_no_show` is refused.
- [x] Given a branch with no user in the manager role, the 13:00 escalation goes to the users with
      `bookings.manage` in that branch.
- [x] Given the conflict cleared at 12:00, the 13:00 reminder goes out normally.

#### Out of scope
Raising conflicts (BL-SCH-05, BL-SCH-07).

---

### BL-SCH-07 — wl-scheduling: flag bookings whose resource will not be eligible (`SCH.RUL.05`)

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SCH-05 · BL-HRM-01 (`qualification_valid`, `QualificationExpiring`/`Expired`) · Epic wl-documents: `document_valid`, `DocumentExpiring` with `affected` from the `FutureWork` provider · **Blocks:** BL-CLN-12, BL-EDU-17
**Review items:** X-17, X-13, 05-N8-a

#### Goal
A practitioner whose licence expires on 20 November must not see patients booked for 25
November. Every day, and at once on expiry events, the module checks future bookings up to D
days ahead (default 30) against the resource's required qualifications and documents *at the
booking's start*. Failing bookings get a conflict with cause `eligibility` and the front desk a
task. A renewal before the start clears the conflict.

#### Surfaces
- **Back office:** the front desk gets a task to reassign or reschedule each booking flagged
  with cause `eligibility`; the booking shows the conflict.
- **System:** `SCH.RUL.05` runs daily at 02:00 and at once on the expiry events, checks
  bookings up to 30 days ahead at their start, flags each once and clears the conflict when
  the item is renewed.

#### Files to read
- `docs/modules/wl-scheduling.md` §3 (Service type: required qualifications and documents),
  §5.6, §6 *Consumes* (paragraph), §6 *Automations* (`SCH.RUL.05`), §7, §10 *Licences*
- `docs/modules/wl-documents.md` §5.6 (guard, `BlockSubjects`, `FutureWork`, `DOC.RUL.02`)
- `docs/modules/wl-hr.md` §2 *Standard guard (X-13)*

#### Terms
- **`FutureWork` provider:** registered by this module; returns bookings of a resource dated on
  or after a date. wl-documents uses it to fill `affected` on `DocumentExpiring`.
- **`BlockSubjects` provider:** maps a booking to its resources' employees and assets.
- **D:** look-ahead in days (default 30).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `SCH.RUL.05` | daily at T; `DocumentExpiring`, `DocumentExpired`, `QualificationExpiring`, `QualificationExpired` | a future booking within D days whose resource fails `qualification_valid` or `document_valid` for the service type's required kinds at the booking's start (on `DocumentExpiring`: the bookings in its `affected` list within D days); not already flagged with the same cause | raise `BookingConflict` (cause `eligibility`); task for the front desk to reassign or reschedule; clear the conflict when the item is renewed | D = 30 days; T = 02:00 Asia/Dubai |

§5.6:
1. "`SCH.RUL.05` checks future bookings daily (02:00), and at once on `DocumentExpiring`,
   `DocumentExpired`, `QualificationExpiring` or `QualificationExpired`. For each booking up
   to D days ahead it asks `qualification_valid(employee, kind, on_date, role, scope)`
   (`wl-hr`, `HRM.QUA.01`; role and scope = the booking's resource role and branch, needed
   for role-bound kinds such as practitioner licences) and `document_valid(subject, kind,
   on_date)` (`wl-documents`) with `on_date` = **the booking's start**. On
   `DocumentExpiring` it does not search again: it takes the event's `affected` list, which
   wl-documents built from this module's `FutureWork` provider (bookings of the resource
   dated after the expiry date), and applies the same D-day window, so the nightly check and
   the event flag the same bookings once."
2. "A booking that will fail raises `BookingConflict` with cause `eligibility`. Staff reassign or
   reschedule (§5.5 steps 2–3); the party is told only that the booking changed."
3. "When the licence or document is renewed before the start, the conflict clears and the
   booking stays as it is."

"this module registers the `BlockSubjects` (booking → its resources' employees and assets) and
`FutureWork` (bookings of a resource from a date) providers for `booking.book` and
`booking.check_in`."

#### Edge cases decided
- **Booking with both a `block` and an `eligibility` conflict:** both causes are kept; clearing
  one leaves the other.
- **Renewal:** on `DocumentAccepted` / qualification renewed, re-check flagged bookings; clear
  only those that now pass at their start.
- **Booking beyond D days:** not flagged now; the daily run flags it when it enters the window.
- **Daily run time:** 02:00 Asia/Dubai (parameter T).
- **Same cause twice:** not flagged again (rule condition).
- **Booking states checked:** `requested`, `booked`, `confirmed`.
- **New bookings:** never flagged by this rule; `book`, `accept`, `reschedule` and `reassign`
  refuse a time when the resource will be ineligible (`SCH.ACL.04`, BL-SCH-08).

§5.6 step 5: "A new booking is never made against a known expiry: `book`, `accept`,
`reschedule` and `reassign` ask `ResourceEligibility` for the booking's start and are refused
when it fails (`SCH.ACL.04`). The conflict and warning path above is for bookings made before
the expiry was known or before the item lapsed, i.e. work already planned (wl-documents §5.6)."

#### Must not
- Must not cancel or move bookings automatically.
- Must not check against today's date: use the booking's start.
- Must not tell the party the reason.

#### Done when
- [x] Given Dr A's licence expires 2026-11-20 and bookings on 2026-11-18 and 2026-11-25, the daily
      run on 2026-11-01 flags only the 25 November booking (cause `eligibility`) and creates a
      front-desk task.
- [x] A booking on 2026-12-15 (beyond 30 days) is not flagged on 2026-11-01.
- [x] On `DocumentExpiring` with `affected` = [25 Nov booking], it is flagged once; the next
      nightly run does not flag it again.
- [x] When the renewed licence (valid to 2027-11-20) is accepted, the conflict clears and the
      booking is unchanged.

#### Out of scope
Re-check at check-in and start (BL-SCH-08); the expiry events (BL-HRM-01; Epic wl-documents).

---

### BL-SCH-08 — wl-scheduling: re-check eligibility at check-in and start

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:high` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** Base: wl-scheduling Booking lifecycle · BL-HRM-01 · BL-SCH-04 (`reassign`) · **Blocks:** BL-CLN-01
**Review items:** X-17, 05-N8-a

#### Goal
A booking that slipped through the look-ahead must not be served by an ineligible resource.
`check_in` and `start` ask `ResourceEligibility` for now. If `check_in` refuses, staff reassign
first (the booking is still `booked`/`confirmed`). If `start` refuses, reassign is no longer
possible (`checked_in`): staff cancel with `resource_ineligible` and rebook, or the vertical
changes the practitioner on its own record. `book`, `accept`, `reschedule` and `reassign` ask it
for the booking's start and refuse a time when the resource will be ineligible (`SCH.ACL.04`).

#### Surfaces
- **Back office:** `check_in` with an ineligible resource is refused with "licence expired
  on <date>; reassign first"; `start` is refused with the hint "cancel with
  resource_ineligible and rebook"; `book`, `accept`, `reschedule` and `reassign` name the
  expiry to staff only.
- **System:** `ResourceEligibility` is asked for now at `check_in` and `start`, and for the
  booking's start on the other transitions; there is no override.

#### Files to read
- `docs/modules/wl-scheduling.md` §4.1 `book`, `accept`, `reschedule`, `check_in`, `start`, §5.6
  steps 4–5, §6 *Access rules*
  (`SCH.ACL.04`), §10 *Licences*

#### Terms
- **Eligible now:** `ResourceEligibility` with time = the moment of the transition.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `check_in` | booked, confirmed | checked_in | `bookings.checkin` | service type requires check-in; resource eligible now (`ResourceEligibility`), otherwise reassign first | event `BookingCheckedIn` (verticals open their record: a visit, a job card) |
| `start` | checked_in | in_progress | the resource (e.g. practitioner) or `bookings.checkin` | service type tracks in-progress; resource eligible now (`ResourceEligibility`) | — |
| `book` | — | booked | `bookings.manage`, or party (self-booking without acceptance) | slot free for all required resources; resource eligible at the booking's start (`ResourceEligibility`; refused, not warned, `SCH.ACL.04`) | confirmation sent; reminders scheduled; event `BookingBooked` |
| `accept` | requested | booked | `bookings.manage` | slot still free; resource eligible at the booking's start | party notified; reminders scheduled |
| `reschedule` | requested, booked, confirmed | requested, booked | party (before the reschedule cut-off) or `bookings.manage` | new slot free; resource eligible at the new start | old slot released (`SlotReleased`); returns to `booked`, or to `requested` if staff acceptance is required; a held booking deposit is carried to the new booking (`SCH.ACL.05`); reminders rescheduled; event `BookingRescheduled` |

| ID | Rule | Pattern | Source | Status |
|---|---|---|---|---|
| `SCH.ACL.04` | A resource cannot be booked for a service type when `ResourceEligibility` refuses at the booking's start (expired licence, missing skill): `book`, `accept`, `reschedule` and `reassign` are refused; bookings made earlier get a conflict instead (`SCH.RUL.05`) | `STA`, `TIM` | `CLN.ACL.04` | in use |

#### Edge cases decided
- **No override:** there is no override permission (05: "legal block, no override").
- **Refused at `start` after check-in:** the booking stays `checked_in`; staff reassign is not
  available from `checked_in` (`reassign` is only from requested/booked/confirmed). Default (spec
  silent): staff cancel with `resource_ineligible` (business-initiated, BL-SCH-03) and rebook, or
  the vertical changes the practitioner on its own record; show that hint.
- **Message:** names the expired item and its expiry date to staff only.

#### Must not
- Must not check in or start with an ineligible resource.
- Must not show the reason to the party.

#### Done when
- [x] Given Dr A's licence expired yesterday and a `confirmed` booking now, `check_in` is refused
      with "licence expired on <date>; reassign first".
- [x] After reassigning to Dr B, `check_in` succeeds.
- [x] Given a booking `checked_in` whose resource's licence expires before `start`, `start` is
      refused with the hint "cancel with resource_ineligible and rebook"; the booking stays
      `checked_in`.
- [x] Given Dr A's licence expires 2026-11-20, `book` (or `reschedule`, `accept`, `reassign`) to
      Dr A on 2026-11-25 is refused with the expiry named to staff; on 2026-11-18 it succeeds.

#### Out of scope
Look-ahead (BL-SCH-07).

---

### BL-SCH-09 — wl-scheduling: give the last seat to exactly one booker

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:high` `surface:back-office` `surface:portal` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: wl-scheduling Booking lifecycle · **Blocks:** BL-SCH-04, BL-SCH-10
**Review items:** X-17, 10 MEDIUM "capacity under concurrency"

#### Goal
Two people booking the last seat of a group slot (or the only slot of a practitioner) at the
same moment must not both win. The capacity check and the insert happen in one transaction that
locks the resource's slot. Holds count towards capacity while they last. The loser is told the
slot was just taken and shown the nearest free slots, or offered the waiting list.

#### Surfaces
- **Back office:** a staff `book` on a slot held by a request, an unpaid hold or a
  waiting-list offer is refused until the hold lapses.
- **Portal:** on self-booking in the portal the losing party is told the slot was just taken
  and shown up to 3 nearest free slots, or the waiting list.
- **Public:** the same "just taken" message with nearest slots or the waiting list on the
  self-booking page.
- **System:** the capacity check and insert run in one transaction that locks every required
  resource's slot; holds count until their expiry time.

#### Files to read
- `docs/modules/wl-scheduling.md` §3 "No double-booking", §4.1 `request`, `book`, `accept`,
  §4.2 *Waiting list entry* (`offer`), §7 (request hold time, prepayment hold, booking
  horizon), §9 (no double-booking under concurrency)

#### Terms
- **Capacity:** 1, or N for group slots.
- **Holds:** a `requested` hold (request hold time, §7 default 15 min, while staff decide), an
  unpaid self-booking hold (prepayment hold, §7 default 15 min, BL-SCH-10), and a waiting-list
  offer (§7 default 30 min).
- **Booking horizon:** how far ahead parties can book (§7, default 60 days).

#### Spec excerpt (authoritative for this issue)

"**No double-booking:** the module guarantees that two bookings never hold the same resource at
overlapping times beyond its capacity, even when two people book at once. The check and the
insert happen in one transaction that locks the resource's slot, so for the last seat of a group
slot exactly one booker wins. Holds count towards capacity while they last: a `requested` hold,
an unpaid self-booking hold and a waiting-list offer. The loser is told the slot was just taken
and shown the nearest free slots, or offered the waiting list. If a payment arrives for a hold that already expired (the booking was cancelled as *unpaid*),
the payment is always refunded in full automatically (`wl-payments` refund with reason *slot not
available*), whether or not the slot is still free. The cancelled booking is not revived; the
party is told to book again."

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `request` | — | requested | party (self-booking) or `bookings.manage` | service type requires acceptance; slot free | slot held for the request hold time (§7; counts towards capacity until then, §3); staff notified |
| `book` | — | booked | `bookings.manage`, or party (self-booking without acceptance) | slot free for all required resources; resource eligible at the booking's start (`ResourceEligibility`; refused, not warned, `SCH.ACL.04`) | confirmation sent; reminders scheduled; event `BookingBooked` |

§5.2 step 2: "the booking is `requested` and the slot is held for the request hold time (§7)
while staff accept or decline; after that the booking stays `requested` without a hold, and
`accept` checks the slot again."

| Setting | Default |
|---|---|
| Request hold time (`request`) | 15 min |
| Booking horizon (how far ahead parties can book) | 60 days |

#### Edge cases decided
- **Requested hold lapsed:** the booking stays `requested` but no longer counts; `accept` then
  re-checks under the same lock and is refused if the slot was taken.
- **Multi-resource services:** lock every required resource in a fixed order (e.g. by resource
  id) to avoid deadlocks; all-or-nothing.
- **Expired hold:** stops counting at its expiry time even if the cleanup job has not run yet
  (compare with now).
- **Overlap, not same start:** a 30-minute booking 10:15–10:45 conflicts with a 10:00–10:30
  booking on a capacity-1 resource; buffers before/after count as occupied.
- **`reschedule` and `reassign`:** use the same lock.
- **Nearest free slots:** up to 3 slots, same service type and location,
  starting from the requested time and no later than the booking horizon (60 days from today).
  First the same resource's free slots; if fewer than 3, the list is filled with the earliest
  free slots of other eligible resources (any time, not only the requested time), sorted by
  start. If none, only the waiting list is offered.

#### Must not
- Must not check capacity outside the transaction.
- Must not ignore holds when counting.

#### Done when
- [x] Given a group slot with capacity 5 and 4 bookings, when users A and B book at the same
      moment, exactly one succeeds and the other gets "just taken" with nearest slots and a
      waiting-list option. Test harness: two `book` calls on two separate database connections,
      both started behind a barrier so they reach the capacity check together; repeat 50 times;
      every run ends with exactly 5 bookings. The same test with 10 parallel callers on the last
      seat ends with 5 bookings and 9 "just taken". This test is what proves the check is inside
      the transaction: a check outside it lets two win.
- [x] Given capacity 1 and a `requested` hold made 10 minutes ago (hold 15 min), a second `book`
      is refused; at 16 minutes it succeeds and a later `accept` of the request is refused.
- [x] Given that hold expired 1 minute ago (cleanup not run), `book` succeeds.
- [x] Given a waiting-list offer holding the slot, a staff `book` is refused until it lapses.

#### Out of scope
Payment after an expired hold (BL-SCH-10).

---

### BL-SCH-10 — wl-scheduling: cancel unpaid holds and refund late payments

**Labels:** `module:wl-scheduling` `area:negative-flow` `priority:high` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SCH-01 · BL-SCH-09 · Epic wl-payments: events `PaymentLinkExpired`, `PaymentFailed`, `PaymentReceived`; Refund `request` · **Blocks:** BL-CLN-15
**Review items:** X-17, X-06

#### Goal
A self-booking that needs prepayment (`requested` if the service type needs acceptance,
otherwise `booked`) holds the slot until its payment link expires. The link is created with
expiry = the prepayment hold time (default 15 min), so one timer ends the hold; a failed payment
does not end it, the party may retry on the same link. At expiry the system cancels with reason
`unpaid` (party-initiated; nothing to forfeit or refund), releases the slot to the waiting list
and tells the party. A payment confirmed after that is refunded in full automatically, even if
the slot is still free.

#### Surfaces
- **Public page:** the party pays the self-booking's prepayment through the payment link
  and, after a failed payment, may retry on the same link until it expires.
- **System:** `PaymentLinkExpired` or `SCH.RUL.04` (whichever comes first) cancels the
  unpaid booking with `unpaid`, releases the slot to the waiting list and tells the party; a
  `PaymentFailed` keeps the hold; a payment confirmed after the cancel is refunded in full
  once (reason *slot not available*) and the booking stays `cancelled`.

#### Files to read
- `docs/modules/wl-scheduling.md` §3 "No double-booking" (last sentence), §5.2 steps 2–3, §6
  *Consumes* (paragraph), §6 *Automations* (`SCH.RUL.03`, `SCH.RUL.04`), §7 (prepayment hold)
- `docs/modules/wl-payments.md` §4 *Payment link* (`expire`), "Who reacts to `PaymentFailed`"
  (booking prepayment row), Refund `request`

#### Terms
- **Prepayment reference:** the booking field linking the payment link.
- **Late payment:** `PaymentReceived` for a booking that is already `cancelled` with reason
  `unpaid`.

#### Spec excerpt (authoritative for this issue)

§5.2 step 3: "Where configured, a prepayment or deposit is requested through a payment link
(`FIN.PAY.02`). The booking (`requested` or `booked`) holds the slot until the payment link
expires; the link is created with expiry = the prepayment hold time (§7), so there is one timer.
A `PaymentFailed` does not end the hold: the party may retry on the same link until it expires
(wl-payments §4). On `PaymentLinkExpired` for the booking's prepayment, or when `SCH.RUL.04`
finds the hold time passed and the booking still unpaid (whichever comes first; the second finds
it `cancelled` and does nothing), the system runs `cancel` with reason code *unpaid* (initiated
by `party`; nothing to forfeit or refund); the slot is released (`SlotReleased`, offered to the
waiting list) and the party is told. A payment confirmed after that is always refunded in full
automatically, even if the slot is still free; the party books again (§3)."

wl-payments "Who reacts to `PaymentFailed`": "Prepayment for a booking (wl-scheduling) ... failed
or timed out: the subject is not confirmed or placed: wl-scheduling keeps the slot on hold until
the link expiry, then releases it."

§3: "If a payment arrives for a hold that already expired (the booking was cancelled as *unpaid*),
the payment is always refunded in full automatically (`wl-payments` refund with reason *slot not
available*), whether or not the slot is still free. The cancelled booking is not revived; the
party is told to book again."

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `SCH.RUL.04` | prepayment hold time passed (same moment as the payment link's expiry, §5.2) | booking `requested` or `booked`, prepayment required and unpaid, not yet cancelled by `PaymentLinkExpired` | booking `cancel` (system, reason `unpaid`) | 15 min |
| `SCH.RUL.03` | `SlotReleased` | a waiting entry matches | waiting entry `offer` | hold H = 30 min |

#### Edge cases decided
- **`PaymentFailed`:** no cancel; the hold continues and the party may retry on the same link
  until it expires (spec §5.2, wl-payments). A retry that succeeds before expiry keeps the
  booking.
- **Two triggers, one timer:** `PaymentLinkExpired` and `SCH.RUL.04` fire at the same moment;
  the first runs `cancel`, the second finds the booking `cancelled` (or paid) and does nothing.
- **Requested self-booking with prepayment:** the request hold time does not shorten the
  prepayment hold; a `requested` booking still waiting for payment is held until the link
  expires. Staff `accept` while unpaid moves it to `booked` with the same unpaid hold.
- **Late payment and the slot is still free:** always refund in full and tell the party to book
  again; do not revive a cancelled booking (§3, §5.2).
- **Idempotency:** one refund per payment; a second `PaymentReceived` for the same payment does
  nothing.
- **`PaymentLinkExpired` for a booking already paid:** ignored.

#### Must not
- Must not forfeit or charge on an `unpaid` cancel.
- Must not reinstate a cancelled booking on late payment.

#### Done when
- [x] Given a self-booking with prepayment AED 150 and hold 15 min, the payment link's expiry is
      15 min after booking; when `PaymentLinkExpired` arrives, the booking is `cancelled` with
      `unpaid`, initiated by `party`, deposit outcome none; `SlotReleased` is emitted and the
      party is told.
- [x] A matching waiting-list entry is offered the slot.
- [x] When `PaymentReceived` for AED 150 arrives afterwards, a refund of AED 150 with reason
      *slot not available* is requested once.
- [x] Given `PaymentFailed` at minute 5, the booking stays held; when the party retries and
      `PaymentReceived` arrives at minute 8, the booking is kept and no refund is requested.
- [x] Given a `requested` self-booking with prepayment and no payment, at minute 15 exactly one
      cancel runs (whether `PaymentLinkExpired` or `SCH.RUL.04` arrives first) and one
      `BookingCancelled` is emitted.
- [x] Given the slot is still free when the late payment arrives, the refund is still requested and
      the booking stays `cancelled`.

#### Out of scope
Refund execution and failure (Epic wl-payments).
