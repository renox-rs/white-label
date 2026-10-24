# F4 (vertical 04, Accounting): progress

Milestone "Launch 04": 33 issues. Merged 2026-10-21.

## Pull requests
- #484 (the F4 branch and the invoicing subscription lifecycle it needed), #485 (deferred gaps), #497 (the CDD file and the first tax profile from the screens).

## Built
- `apps/accounting`: Engagement, CDD file, Return, AML case and KPI pack machines, plus TaxProfileChange and WipWriteOff; compliance calendar, retainer, neutral reasons, document request lists, engagement letter signature record, tax agent check, voluntary disclosure and correction ([apps/accounting/README.md](../../apps/accounting/README.md)).
- Host seed `accounting_seed.rs` (lead sources, checklists, SOPs) and `clients/demo-accounting`.

## Known gaps
- No demo command: there is no `accounting:demo` (see [DEMO-DATA.md](../DEMO-DATA.md) 04).
- Time entries and time-based WIP billing (`ACC.TIM.01`) are not built: `wl-attendance` exists but does not implement `wl_kernel::wip::UnbilledTime`, so WIP is zero.
- KPI presets for Zoho Books, Xero, QuickBooks and Tally (`ACC.KPI.01`) are not built; only the generic CSV layout.
- Open on GitHub with a deferred point: `#346` (BL-ACC-22).
