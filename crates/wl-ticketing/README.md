# wl-ticketing

Ticketing: tickets raised on any subject (a unit, a lease, a vehicle), triaged and assigned
to a staff member or a vendor, with SLA due times in working time, hold reasons, resolve,
confirm and close ([spec](../../docs/modules/wl-ticketing.md)). Other modules open tickets
and react to them only through the kernel (`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `ticketing`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_ticketing::WlTicketing::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`tickets.*`) | §6 | `permissions` |
| Platform categories opened by other modules, seeded when the module is on: `billing_dispute` (invoicing), `labour_case` (hr: confidential, role `hr`, custom fields forum, reference, hearing and reply dates), `delivery_claim` (delivery) | §3, §7 | `seed` |

The module owns `src/texts.rs`, the English texts (`wl_ticketing.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
