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
| The cheque register: `Cheque` with its links to the documents it pays and its bank returns; registering a received or issued cheque or a lease's post-dated schedule (`chequer`, the kernel's `Cheques`); `flag_due` (`FIN.RUL.03`), `deposit` (date reached, a field cheque only after a handover, `FIN.ACL.20`) and `clear` (a payment for the open amount) | §3, §4 *Cheque*, §5.3 | `cheque`, `chequer`, `cheque_deposit` |
| A bounced cheque: `bounce` (the bank's reason, the reference of its return certificate for a received cheque, the honoured part as a payment allocated to the documents the cheque pays, the open amount, the bounce fee charge when `WlPayments::bounce_fee` is set, `ChequeBounced`, `FIN.RUL.04` notices and tasks; an issued cheque only leaves a task to replace it) and `represent` (a bounced received cheque deposited again at most twice in all, only when the bank paid nothing, with the party's confirmation or a note) | §4 *Cheque*, §5.3 step 3 and 4 | `cheque_bounce`, `cheque_represent` |
| Settling a bounced cheque: `settle_part` (cash, transfer or card, below the open amount, allocated to the documents the cheque pays), the replacements linked to it (new cheques or payments) and `replace` (they must cover the open amount, `FIN.ACL.02`; `ChequeBounceResolved`; the original is handed back with a receipt, series `CHR`, once its replacement cheques cleared) | §4 *Cheque*, §5.3 step 4 | `cheque_settle` |
| Referring a bounced cheque to a legal case: `refer_to_legal` (`cheques.legal`; a received cheque with its return certificate; civil execution, or a criminal complaint only for the return reasons on the configured list, `WlPayments::criminal_grounds`; the open amount above the minimum, `WlPayments::legal_minimum`; the case owner, by default the user who refers) with the owner's task and evidence pack, the formal notice to the party and the manager's notice | §4 *Cheque*, §5.3 step 4, §7, §10 | `cheque_legal` |
| Writing off a bounced cheque: `write_off` (`cheques.manage`, a reason; a linked charge not yet invoiced is invoiced first; the open balances of the linked invoices must equal the cheque's open amount; one invoice write-off request per invoice through wl-invoicing, with its own approval limit) moves the cheque to `written_off` only when `InvoiceWrittenOff` has arrived for all of them (`write_off_by_system`); a rejected or failed request leaves the cheque and gives the accountant a task; `ChequeBounceResolved` (written off) | §4 *Cheque*, §5.3 step 4, §6 | `cheque_write_off` |
| `FIN.RUL.18`: a bounced cheque is chased, hourly: 3 days after the bounce (Asia/Dubai) the party is reminded with the open amount and the bounce fee (not while blocked) and the responsible staff are told; 14 days after it the accountant and the manager each get a task to re-present, refer to legal or write off (`WlPayments::cheque_chase`); only while the cheque is `bounced`, counted again from a new bounce | §6, §7 | `cheque_chase` |

The module owns `src/texts.rs`, the English texts (`wl_payments.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
