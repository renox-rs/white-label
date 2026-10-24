# F2 (vertical 02, Construction): progress

Milestone "Launch 02": 51 issues. Merged 2026-10-21.

## Pull requests
- #486 `feat/f2-construction-issues` (the app, `wl-attendance`, `wl-inventory`), #487 (HTML screens), #488 (mobile layout of the activity page), #457 (backlog re-milestoned by phase).

## Built
- `apps/construction`: Project (including `complete`, `close`, termination and the final account), Variation (addition, omission, time), IPA, Retention, Guarantee, plus the app's own IpaDeduction and RetentionDeduction machines ([apps/construction/README.md](../../apps/construction/README.md)).
- `crates/wl-attendance` (crew attendance, timesheets, absence), `crates/wl-inventory` (items, locations, purchase orders, goods receipts, returns to suppliers).
- Seeds for the 02 values of the shared modules; absence, snag and defect ticket categories.
- Demo: `clients/demo-construction`, command `construction:demo` ([DEMO-DATA.md](../DEMO-DATA.md) 02). The client has no `sales` module.

## Known gaps
- Not built (README and spec §12): sites and geofences, BOQ import and the IPA calculation, `revise` and `CON.RUL.04`, advance recovery, daily site reports (`CON.RUL.02`), snag lists, `CON.RUL.05`, crews, project pages and the client portal.
- Open on GitHub with a deferred point: `#298` (BL-CON-19).
