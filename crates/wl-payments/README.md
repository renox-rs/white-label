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
| Enabled payment methods, and the retry of a failed gateway refund (once, after 24 hours: `WlPayments::refund_retry`) | §7 | `config` |
| Recovering a failed refund: `retry` (at most once, manual retries count, a gateway refund goes to the gateway again), `change_method` (the original card, a transfer, a cheque or cash; new bank details or a cheque to someone else go back to `requested` for another user's approval), `cancel` (a reason; the amount stays on the party's credit) | §4, §5.5a | `refund_recover` |
| `FIN.RUL.21`: a failed gateway refund is retried by the system after N hours, any other failed refund gives the accountant a "change method" task at once | §6, §7 | `refund_retry` |

The module owns `src/texts.rs`, the English texts (`wl_payments.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
