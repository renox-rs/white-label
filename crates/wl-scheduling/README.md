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

Bookings, the lock that stops double-booking, the waiting list and the reminders are other parts of the issues
listed in the backlog (`docs/backlog/modules/wl-scheduling.md`).

The module owns `src/texts.rs`, the English texts (`wl_scheduling.*`). The app changes or translates them in
`resources/lang/{locale}.json`.
