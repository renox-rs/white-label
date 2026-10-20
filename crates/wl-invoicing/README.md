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
| Reminders (`FIN.RUL.01`) skip an invoice whose source a vertical holds (`wl_kernel::chase_holds`, a legal case on a lease) and, when the hold ended, do not send the steps reached meanwhile | §5.3 | `reminders` |
| Settings: payment terms, reminder steps, write-off reasons and notice, approvers, fiscal year start, budget reminder | §7 | `config` |
| Company budget: categories and their mapping, versions with lines per category and month, submit, approval (`PLT.APR.01`), supersede, discard, CSV import, `FIN.RUL.26` | §3, §4, §5.15 | `budget`, `budget_category`, `budget_lines`, `budget_machine`, `budget_reminder` |
| Budget vs actual per category and month, the owner drawings line, `FIN.RUL.24` | §5.15 | `budget_actual`, `budget_report`, `budget_variance` |
| Owner drawings: the approval of a drawing on a supplier bill (`FIN.ACL.21`, the sole owner `FIN.ACL.23`), the lines hidden from users without `drawings.approve` (`FIN.ACL.22`), recoding, the VAT that is not recovered | §3, §5.16 | `drawings`, `drawings_machine` |
| Cash-flow forecast: the 13-week grid from the open documents, cheques, payments, refunds and VAT; the opening balance per bank account (statement or hand-entered, plus the payments after it); memo lines and warnings; the cell drill-down; opening cash balances entered and voided (`cashflow.view`, `cashflow.manage`); the lines other modules add through the kernel's `CashForecastSource`; the budget fill of the approved company budget per category; the *adjust by days late* setting (`cash_days_late`) that moves receipts by a party's average lateness. Payroll, final settlements, subscription cycles, payer bills and scheduled charges come through that extension point when their modules exist | §3, §5.14 | `cash_forecast`, `cash_forecast_load`, `cash_balance` |
| The daily cash check at 06:00 (`invoicing:cash-alerts`): `FIN.RUL.23` tells the finance manager and the company admin once when a week closes below the entity's threshold (income budget fill left out unless counted) and emits `CashForecastBelowThreshold`; `FIN.RUL.25` opens one task for the accountant per bank account whose balance is missing or older than the stale limit, closed by a newer balance | §5.14, §6, §7 | `cash_alert` |
| Owner drawings on payments (`FIN.ACL.21`, `FIN.ACL.23`): the kernel's `OwnerDrawings` (the drawings category and its owner party; the approval of a drawing on wl-payments' record, the same path a supplier bill's drawing takes, a sole owner's own drawing approved at once and flagged with the accountant told), `OwnerDrawingDecided` when a waiting request ends, and the drawings payments in the actual of budget vs actual (a drawing adds its amount, a repayment by the owner takes it off) | §3, §5.15, §5.16 | `owner_drawings`, `drawings`, `budget_actual` |
| E-invoicing (`einvoicing`, `asp_adapter`, `einvoice_scope`): an `EInvoiceTransmission` per issued invoice or credit note of an entity past its go-live date, queued after commit and sent through the entity's ASP adapter (`EInvoiceAdapter`; `FakeAsp` for tests), retried at +5 min, +30 min, +2 h, +6 h then every 6 h from issue; the board at `/einvoices` (`einvoices.view`, `einvoices.manage`); `EInvoiceAccepted` and `EInvoiceRejected` events. `FIN.RUL.16`: a rejection opens an accountant task with the codes, a resubmit rebuilds the payload, a credit note with reason *e-invoice correction* supersedes it (and a replacing invoice is linked), a transmission queued for 24 hours opens one task, and an outage of all transmissions tells the admin once | §4, §5.12, §6 | `einvoice`, `einvoice_send`, `einvoice_tasks`, `einvoice_pages`, `einvoice_adapter`, `einvoice_payload` |
| Contract bill match (`FIN.ACL.03`): a bill against a service contract is held in `exception` with the reason *work after contract end*, naming the lines, when a line's service date is after the contract's end or termination date (the kernel's `ServiceContractCoverage`); like a possible duplicate it is not approved by its creator | §4 *Supplier bill*, §5.4 | `bill_submit`, `bill_machine` |
| Supplier debit note: drafted by hand against a bill, or by `FIN.RUL.17` from a dispatched return (once per return; at the bill's prices, or waiting at the PO price and re-priced when the bill is approved); `issue` (`supplier_bills.approve`, not the creator of a manual note, `FIN.ACL.17`; not in a locked VAT period, `FIN.ACL.09`) takes a gapless `DN-` number, reduces the bill through `pay` or opens the supplier's debit balance, reverses the cost on the cost objects, tells the supplier, emits `SupplierDebitNoteIssued` and reduces input VAT per `debit_note_vat_timing` | §3, §4, §5.13 | `supplier_debit_note`, `debit_notes`, `debit_note_machine`, `debit_note_contract`, `debit_note_events`, `debit_note_vat` |
| Settle and cancel a supplier debit note: the supplier's tax credit note recorded (number, date, file; the accountant's task *record the tax credit note* is opened at `issue` and closed then), the debit balance set off on a later approved or partly paid bill of the same supplier, entity and currency (the bill runs `pay`), the supplier's refund applied through the kernel's `settle_from_payment` (a refund, not a bill payment; one key, one change), `settle` once everything is applied and the tax credit note is there (or `tax_credit_note_required` is off), with an accepted difference and its reason; `cancel` with a reason reopens every bill the note reduced through `unallocate` (also from `paid`), closes the debit balance, puts the cost back, tells the supplier and emits `SupplierDebitNoteCancelled`; a note the supplier paid back on is not cancelled | §4, §5.13 steps 4 and 5 | `debit_note_settle`, `debit_note_movement`, `debit_note_machine`, `debit_note_events` |
| Supplier bill approval posts the actual cost: each line coded to a cost object is posted to it through the kernel's `CostPoster` (the net in AED fils, keyed on the line); a freight, duty or clearing line coded to a wl-inventory landed cost (checked at `submit` through the kernel's `LandedCosts`: it must exist and belong to the receiving entity, and the line has no cost object) posts none; `SupplierBillApproved` carries the cost objects and the landed cost reference per line; the VAT of a landed cost line is input VAT as any other | §3, §4, §5.4 step 4, §6 | `bill_events`, `bill_submit`, `bill_line` |
| Hours corrected after billing (`FIN.ACL.01`): usage records (`usage::record`, priced per hour with the multipliers and the rate date) and the usage billing run (`usage::bill`: a draft per party, entity and currency); an approved adjustment timesheet (`TimesheetApproved` kind `adjustment`, `affects` billing or both; payroll only is ignored) is netted per hours category and applied at the original record's own price: not yet invoiced, the hours change and the draft bills them with the original; invoiced, fewer hours give a credit note against the original invoice naming the adjustment (the invoice is never edited), more hours an extra usage record for the next usage invoice or, with `adjustment_plus(AdjustmentPlus::SeparateInvoice)`, a draft invoice of its own. A credit above what the invoice can still be credited by opens a task for the `coordinator` role; an event received twice changes nothing. A dispute resolved `credited` with the adjustment timesheet links to that credit note | §5.2 step 5, §5.10 step 4 | `usage`, `usage_adjustment`, `dispute_resolve` |

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
