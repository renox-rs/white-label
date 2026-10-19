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
| `WRK.RUL.18`, compliance-critical, when a contract ends without renewal (`expire`, or the termination date of a `terminate`; at once when it is today): cancels the open planned visits due after the end date, leaves started visits alone, opens one task for the coordinator with the other open visits (reassign with reason `contract_ended`, or cancel, except in a category set to reassign only), ends the maintenance plans, raises `no_contract` unless another active contract covers the subject and category, emits `ServiceContractEnded`; the nightly `ticketing:contract_end` also runs `WRK.RUL.12` and `WRK.RUL.13` | §4.6, §5.6 | `contract_end` |
| The service contract page (`/service-contracts/{id}`): details, coverage, planned visits with reassign and cancel, flags; activate and terminate | §4.6 | `contract_pages` |
| The vendor document guard (`WRK.ACL.11`): `ticket.resolve` and `service_contract.activate` are guardable transitions of the document kinds' `blocks`; the `BlockSubjects` providers return the assigned vendor (a ticket in a category matching the entry's `category` condition) or the contract's vendor (a covered category matches) | §4.1, §4.6 | `vendor_guard` |
| The vendor document flags: on `DocumentExpired` of a document that guards `resolve` or `activate`, the vendor's `active` and `expiring` contracts with a guarded covered category carry the vendor document flag, their open planned visits are flagged, the covered subjects get `vendor_document_expired` and the coordinator and the manager are told; they clear on `DocumentAccepted` of a renewed document (a visit also when reassigned, a subject also by a replacement contract or the contract's end) | §3, §4.6 | `vendor_flags` |
| A blocked vendor's contracts are reviewed (`PTY.ACL.03`): `assign` to a blocked vendor is refused through the kernel's `BlockGuard` (neutral text, `Start`), a planned visit of its contracts is created unassigned, `assign` does not propose it, and the `service_contract` agreement kind (review, decided by `facility_manager`) lists its contracts and open visits for the review task; the decider reassigns the visits or terminates the contract on its page | §4.1, §4.6 | `vendor_block` |
| Auto-close after resolve (`WRK.RUL.02`): the hourly task `ticketing:auto_close` confirms, as the system, a resolved ticket whose requester did nothing for 3 days (`WlTicketing::auto_close_days`) | §7 | `auto_close` |

The module owns `src/texts.rs`, the English texts (`wl_ticketing.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
