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

The rest of the negative flows, the self-booking pages, the waiting list and the reminders are other parts of the
issues listed in the backlog (`docs/backlog/modules/wl-scheduling.md`).

The module owns `src/texts.rs`, the English texts (`wl_scheduling.*`). The app changes or translates them in
`resources/lang/{locale}.json`.
