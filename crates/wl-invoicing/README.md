# wl-invoicing

Invoicing: tax invoices with lines and UAE VAT categories, gapless numbers per legal entity,
allocations and balances, overdue, reminders and bad-debt write-offs
([spec](../../docs/modules/wl-invoicing.md)). Other modules draft invoices and allocate
money only through the kernel (`wl-kernel`), never through this crate.

The host mounts it when `client.toml` enables `invoicing`:

```
use renox::prelude::*;

# let _ =
App::new().module(wl_invoicing::WlInvoicing::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`invoices.*`, `receivables.view`) | §6 | `permissions` |
| Settings: payment terms, reminder steps, write-off reasons and notice, approvers, fiscal year start, budget reminder | §7 | `config` |
| Company budget: categories and their mapping, versions with lines per category and month, submit, approval (`PLT.APR.01`), supersede, discard, CSV import, `FIN.RUL.26` | §3, §4, §5.15 | `budget`, `budget_category`, `budget_lines`, `budget_machine`, `budget_reminder` |
| Budget vs actual per category and month, the owner drawings line, `FIN.RUL.24` | §5.15 | `budget_actual`, `budget_report`, `budget_variance` |

The settings are client defaults set in code (§7); the host changes one with a builder call:

```
# let _ =
wl_invoicing::WlInvoicing::new()
    .payment_terms_days(14)
    .reminder_steps(&[-3, 1, 7])
    .write_off_notice(false)
# ;
```

The module owns `src/texts.rs`, the English texts (`wl_invoicing.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
