# F6 (vertical 06, F&B): progress

Milestone "Launch 06": 34 issues. Merged 2026-10-23.

## Pull requests
- #520 `feat/f6-fnb-issues`. F7 and F8 were merged after it and unified the sales order hold (`hold`/`held_from`) with it.

## Built
- `apps/fnb`: the order as the kitchen's fulfilment of a sales order (nine states), kitchen ticket, production batch, payout statement, daily closing, plus the Outage machine; recalls traced to direct orders, paper orders, refused-food refunds ([apps/fnb/README.md](../../apps/fnb/README.md)).
- Shared changes in `wl-sales` (back-orders, credit) and `wl-inventory` (dispatch).
- Demo: `clients/demo-fnb`, command `fnb:demo` ([DEMO-DATA.md](../DEMO-DATA.md) 06).

## Known gaps
- Not built (spec §12): aggregator orders through Deliverect, the D2C ordering site, `FNB.RUL.02`, `06`, `07`, `09`.
- The authority notification of a recall (open item AU-20) and wastage posting for refused food are not built (README).
