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

The rest of the negative flows, the self-booking pages, the waiting list and the reminders are other parts of the
issues listed in the backlog (`docs/backlog/modules/wl-scheduling.md`).

The module owns `src/texts.rs`, the English texts (`wl_scheduling.*`). The app changes or translates them in
`resources/lang/{locale}.json`.
