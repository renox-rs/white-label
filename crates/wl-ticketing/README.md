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
| Category settings: rename, turn off, delete (a platform category, or one with tickets, is refused), the page `/ticket-categories` | §7 | `categories`, `category_pages` |
| Chargeable tickets: charge lines (the amount the requester is shown and the lines the draft carries), `resolve` refused without one, one draft invoice per ticket made at `resolve` or at close as the category says (`InvoiceOn`), through `Invoicer` | §3, §4.1 | `charge`, `work` |

The module owns `src/texts.rs`, the English texts (`wl_ticketing.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
