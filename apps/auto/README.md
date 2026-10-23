# auto

The vertical app for auto parts suppliers and fleet and workshop services
([spec](../../docs/verticals/08-auto.md)): the vehicle register, workshop job
cards with their estimates, intake condition and customer consent, parts
issued from stock, and the invoice on delivery. The app reaches the other
modules only through the contracts of `wl-kernel` (the scheduling events, the
stock service, the invoicer, the party directory), never through their crates.

The host mounts it when `client.toml` names `app = "auto"`:

```
use renox::prelude::*;

# let _ =
App::new().module(auto::Auto)
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`vehicles.*`, `job_cards.*`, `labour_menu.*`, and those of the insurance, fleet and supplier-claim screens) and the role names the rules use (owner, branch manager, service advisor, workshop manager, technician, counter sales, storekeeper, purchaser, accountant, insurance coordinator, fleet account manager) | §3.1, §3.3 | `permissions` |
| The starter tables the host installs: the document kinds (police accident report, claim documents, customer trade licence, fleet contract), the workshop service types and the ticket categories (warranty, vehicle incident, complaint) | §4.4, §7.1 *Booking*, §9 | `seed` |
| The cancellation reason codes 08 adds to wl-scheduling (`estimate_declined`, `duplicate_check_in`, `job_stopped`) | §7.1 *Booking* | `reasons` |
