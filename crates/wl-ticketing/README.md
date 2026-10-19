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
| Compliance flags on a covered subject (`no_contract`, `visit_missed`, `vendor_document_expired`): one open per subject, category and reason, raised only for a compliance-critical category, `ComplianceFlagRaised` and `ComplianceFlagCleared`; a `visit_missed` flag clears when its visit is resolved | §3, §6 | `flags` |
| `WRK.RUL.14`, nightly and compliance-critical: a planned visit not resolved 3 days (`WlTicketing::visit_missed_days`) after its planned due date tells the coordinator (`coordinator_role`) and, for a compliance-critical category, the manager in scope (`compliance_manager_role`) and raises the flag `visit_missed` | §6 | `visit_check` |
| The compliance report: the open flags by subject, reason and age (`/compliance-flags`), and the compliance-critical setting of a category | §6, §7 | `flag_pages`, `category_pages` |
| Service contracts with a vendor (draft, active, expiring, expired, terminated; `renew` and `discard` come later) covering subjects for categories, one maintenance plan per covered subject and category at `activate`, `ServiceContracts::coverage` (a ticket on a covered subject records the contract and takes its SLA policy) | §3, §4.3, §4.6 | `contract`, `plans` |
| Auto-close after resolve (`WRK.RUL.02`): the hourly task `ticketing:auto_close` confirms, as the system, a resolved ticket whose requester did nothing for 3 days (`WlTicketing::auto_close_days`) | §7 | `auto_close` |

The module owns `src/texts.rs`, the English texts (`wl_ticketing.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
