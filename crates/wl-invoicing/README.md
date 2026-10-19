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
| Owner drawings: the approval of a drawing on a supplier bill (`FIN.ACL.21`, the sole owner `FIN.ACL.23`), the lines hidden from users without `drawings.approve` (`FIN.ACL.22`), recoding, the VAT that is not recovered | §3, §5.16 | `drawings`, `drawings_machine` |
| Cash-flow forecast: the 13-week grid from the open documents, cheques, payments, refunds and VAT; the opening balance per bank account (statement or hand-entered, plus the payments after it); memo lines and warnings; the cell drill-down; opening cash balances entered and voided (`cashflow.view`, `cashflow.manage`); the lines other modules add through the kernel's `CashForecastSource`; the budget fill of the approved company budget per category; the *adjust by days late* setting (`cash_days_late`) that moves receipts by a party's average lateness. Payroll, final settlements, subscription cycles, payer bills and scheduled charges come through that extension point when their modules exist | §3, §5.14 | `cash_forecast`, `cash_forecast_load`, `cash_balance` |

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
