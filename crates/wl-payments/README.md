# wl-payments

Payments: money received from or paid to a party by cash, transfer, cheque or card, receipts
numbered per legal entity, allocation to invoices, reversal, refunds and the party's credit
([spec](../../docs/modules/wl-payments.md)). Other modules record payments and read credit
only through the kernel (`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `payments`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_payments::WlPayments::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`payments.*`, `refunds.*`) | §6 | `permissions` |
| Enabled payment methods | §7 | `config` |

The module owns `src/texts.rs`, the English texts (`wl_payments.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
