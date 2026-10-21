# wl-scheduling

Bookable resources and their availability, bookings with their lifecycle, and the guarantee that nobody is
double-booked ([spec](../../docs/modules/wl-scheduling.md)). A booking knows nothing about patients, vehicles or
students: those are its `Subject`. Other modules reach the booking only through the kernel (`wl_kernel::scheduling`):
its events and the `ResourceEligibility` strategy.

The host mounts it when `client.toml` enables `scheduling`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_scheduling::WlScheduling::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`bookings.*`, `resources.manage`) | §6 | `permissions` |
| Resources (person, room, bay, equipment, online), service types, which resources deliver them, weekly availability rules and blocks | §3, `SCH.RES.01` | `resource` |
| The booking and its lifecycle (`requested`, `booked`, `confirmed`, `checked_in`, `in_progress`, `completed`, `no_show`, `cancelled`; `accept`, `decline`, `confirm`, `reschedule`, `check_in`, `start`, `complete`, `mark_no_show`, `cancel`), booked by staff or requested by a party, with the events of the kernel | §3, §4.1, §6 | `booking`, `booking_machine`, `service`, `events` |
| Free time: rules less blocks less bookings, buffers, seats per slot, all required resources or none | §3, `SCH.RES.01` | `slots` |
| `ResourceEligibility` asked at the booking's start (the default says yes); the bookings as `PrepaidSubjects` for `wl-payments` | §8, `SCH.ACL.04` | `service`, `prepaid` |
| The staff pages (list, booking, book, accept, decline, confirm, reschedule, cancel, check in, start, complete, no-show) | §5.1 | `pages` |
| Cancellation reason codes and who initiated: staff pick a code (the side is the code's default; the other side needs a note), a party records `party_request` from the portal or a confirmation link and the code of the change, as the business, from a change message; *late* follows the deposit forfeit window or the cancellation cut-off; `BookingCancelled` carries it all | §4.1, `SCH.ACL.02` | `service`, `links`, `booking` |
| No double-booking (`SCH.BKG.01`): the check of the seats and the insert of a booking, a move or an accept run in one write transaction that locks the resources in the order of their ids (`begin_immediate`, `FOR UPDATE`), so that for the last seat of a group slot exactly one booker wins; a request counts until its hold time, an unpaid booking until its payment link expires, an offered waiting-list slot until the offer lapses, judged by the clock and not by a cleanup job; the booker who lost is told the slot was just taken, shown up to three nearest free slots (the same resources first, then the earliest of other eligible resources of the location, up to the booking horizon) and offered the waiting list; `Refusal::Taken`, `slots::nearest` | §3, §7 | `slots`, `service`, `refusal` |
| The waiting list (`waiting`, `offered`, `booked`, `expired`, `withdrawn`; `offer`, `accept`, `lapse`, `expire`, `withdraw`): an offered entry holds its slot for the offer hold time; the portal joins and leaves it and books without naming a resource (the first free one) | §3, §4.2 | `waiting`, `links` |
| The booking deposit (`SCH.ACL.05`): a cancellation by the business, or by the party outside the forfeit window, asks the whole deposit back from `wl-payments` (`RefundRequest`, source: the booking); a late party cancellation and a no-show forfeit it; a reschedule keeps it on the booking and `BookingRescheduled` says how much; a user with `bookings.waive_deposit` turns a forfeiture into a refund with a reason on the booking page. A refund `wl-payments` refuses leaves the cancellation standing and `deposit::settle` runs again | §4.1, §5.2, `SCH.ACL.05` | `deposit`, `service`, `pages` |
| Cancel after check-in (`SCH.ACL.06`): only staff cancel a `checked_in` or `in_progress` booking, with a reason code, and not while the vertical's `WorkDoneStatus` (kernel strategy; the default says nothing was done) answers `pending`; a party who leaves (`walked_out`, `party_request`) has the deposit applied to the work done first and the rest forfeited or refunded by the service type's walk-out setting; a business cancel refunds it in full and applies nothing; only the time that is left is released; `BookingCancelled` carries the work done and the deposit applied for the vertical to bill; `service::link_work_done` ties the vertical's record to the booking | §4.1, `SCH.ACL.06` | `service`, `deposit`, `booking_machine` |
| Reassign (`reassign`): staff move a `requested`, `booked` or `confirmed` booking to another free, eligible resource at the same time with a reason code; the state stays, only the replaced resource's slot is released (`SlotReleased`), the new one is taken under the slot lock, the conflict clears and `BookingReassigned` is emitted; the party is told the new resource (not the reason) with *accept*, *pick another time* and *cancel* links, and cancelling from the message is business-initiated; *pick another time* lists the nearest free times of the same resources and moves the booking also after the reschedule cut-off; check-in and start ask `ResourceEligibility` again | §4.1, §5.5, `SCH.ACL.04` | `service`, `links`, `notices`, `booking_machine` |
| Conflicts (`BookingConflict`): a new block (approved leave from `wl-hr`, sick leave recorded at once, a manual *sick* block, an `AvailabilitySource` block) flags the `requested`, `booked` and `confirmed` bookings it overlaps with the cause `block` and cancels none; one `BookingConflict` lists them with the earliest start; a block removed (`LeaveCancelled`, staff) clears only its own conflicts; staff list the conflicts and in one action reassign, cancel as the business or send each party up to three next free times of the same resource (nothing moved or held; the party's pick runs `reschedule` and clears the conflict); every booking is its own transaction and the result lists successes and failures; a party gets one message for all their bookings, with *accept*, *pick another time* and *cancel* links and no reason | §3, §5.4, §5.5 | `conflict`, `conflict_pages`, `notices` |
| Open conflicts and the last hours (`SCH.RUL.06`, with the parts of `SCH.RUL.01` and `SCH.RUL.02` it governs): `scheduling:rules` (every five minutes) tells the branch manager (the vertical's manager role in the booking's location, else the users with `bookings.manage` there) and sends the party the change message `H` hours (2) before the start of a booking that still has a conflict, at once when the conflict is raised inside `H`, once per conflict; reminders (24 h and 2 h before) are not sent for a conflicted booking and a no-show is neither marked by the sweep nor by staff (the machine refuses `mark_no_show` while a conflict is open). The reminder has the cancel link only; its confirm and reschedule links are not built | §4.1, §5.5, §6 | `rules`, `booking_machine` |

The rest of the negative flows, the self-booking pages, the waiting list and the reminders are other parts of the
issues listed in the backlog (`docs/backlog/modules/wl-scheduling.md`).

The module owns `src/texts.rs`, the English texts (`wl_scheduling.*`). The app changes or translates them in
`resources/lang/{locale}.json`.
