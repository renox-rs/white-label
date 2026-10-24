# F3 (vertical 03, Logistics): progress

Milestone "Launch 03": 41 issues. Merged 2026-10-22.

## Pull requests
- #498 `feat/f3-logistics-issues` (the app and `wl-delivery`).

## Built
- `apps/logistics`: Shipment (booking to delivery and return, with void, cancel, partial pickup and delivery, re-weigh, hold, reversal, loss, COD difference), COD remittance (approve, reopen, pay, adjustment lines, netting, returned transfer, statement dispute), Freight job (documents, holds, free time and demurrage, reopening) ([apps/logistics/README.md](../../apps/logistics/README.md)).
- `crates/wl-delivery`: runs, drivers, proof of delivery, cash custody, the `/driver` PWA pages.
- Demo: `clients/demo-logistics`, command `logistics:demo` ([DEMO-DATA.md](../DEMO-DATA.md) 03).

## Known gaps
- Rate cards: `wl-invoicing` has them, but `rates::price` still reads the local `ClientRate` stand-in.
- Not built (spec §12): vehicle register with services, daily checks, fines, tolls and odometer CSV; the public tracking link; `LOG.RUL.04` and `LOG.RUL.05`; the per-client switch for freight forwarding (today only the `freight.*` permissions).
