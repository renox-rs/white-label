# F7 (Manufacturing, Launch 07): progress notes

Built on branch `feat/f7-manufacturing-issues`, merged in #521 (2026-10-24). 25 issues, 140 Done-when points.
Moved here from `docs/backlog/F7-HANDOFF.md`; the branch hand-over parts (next steps, push and pull request instructions) were dropped
because the branch is merged.

## Issues in scope (GitHub numbers)

| Block | Issues |
|---|---|
| wl-inventory | BL-STK-21..25 = #240..#244 |
| wl-sales | BL-SAL-10..13 = #129..#132, BL-SAL-18 = #137 |
| manufacturing | BL-MFG-01..15 = #382..#396 (MFG-03 #384, 06 #387, 07 #388, 10 #391, 11 #392, 12 #393, 14 #395) |

Map of IDs to numbers: `docs/backlog/github-issues.md`. Issue text: `docs/backlog/verticals/07-manufacturing.md`,
`docs/backlog/modules/wl-sales.md`, `wl-inventory.md`.

## State

F7 is merged: pull request #521 (`feat/f7-manufacturing-issues`, merged 2026-10-24, with F6 and F8 already on main). All 25 issues were built
(code, tests, screens, demo client), audited against their 140 Done-when points, ticked on GitHub with `**Deferred:**` lines and one
`**Verification summary**` comment each, and browser-checked at 1280 and 390 px. Later commits closed more deferred rows (see below).
The test totals of the branch hand-over are not repeated here; they were historic. The demo is `manufacturing:demo`.

## What was built (where to look)

- `apps/manufacturing`: work order (release, start, hold, resume, complete, close, cancel, unlink, abort with approval), BOM, material issue with lot check,
  production reports and void, hold for material, NCR lifecycle and dispositions, recall trace parties, conformity item flag, export evidence (MFG.RUL.08),
  customer PO upload in the portal, order changes and abort sheet, inspections of returned goods, aborted work orders report, settings pages, dashboard tiles.
  Its README holds a table of what is built and a "Deferred in F7" table.
- `crates/wl-sales`: customer credit account, credit hold and release (user and `_approved` twins), account hold, PO number uniqueness (also on amend),
  invoice on delivery, order directory (`order_directory.rs` provides the kernel `OrderLines`, `CustomerPoCheck`, `CreditExposure`, `CreditChecker`).
- `crates/wl-inventory`: Dispatch lifecycle (pick, send, deliver, loss, reverse, return, discard), dispatch guard (documents at pick and dispatch),
  BOM provider, production stock, reservations of stock for sales order lines and work order output lots (`reservation`), lot disposal, recall trace with production lines and dispatch directory.
- `crates/wl-kernel`: new contracts with defaults (`production`, `lot_disposal`, `DispatchDirectory`, `TracePartyResolver::addressees`, `DispatchGuard`,
  `OutcomeProposals`, `CustomerPoCheck`, `OrderLineRules`, `PromisedLines`, `DeliveryDestination`, dispatch events, credit contracts).
- Host: `host/src/app/manufacturing_seed.rs` (07 configuration), `manufacturing_demo/`, `dashboard/manufacturing_tiles.rs`, roles in `host/src/app/roles.rs`.
- Demo: `clients/demo-manufacturing`, command `manufacturing:demo`, data described in `docs/DEMO-DATA.md`.

## Decisions to know

- One `hold` transition for sales orders (F6 `held_from`), release has three targets (`release`, `release_partly`, `release_pending`).
- The amendment re-check of F6 reads the limit through `CreditExposure`; overdue and account hold are checked at dispatch.
- Storekeeper does NOT hold `supplier_returns.manage` (base test `host/tests/inventory.rs` forbids it); NCR returns to supplier use a procurement user.
- `dispatch.cancel` needs a reason (F6 test).
- Dispatch migration `20261114000000_extend_wl_inventory_dispatches` alters F6's tables; F6's smaller dispatch was folded into the full lifecycle.
- One reservation (main merged after F8): the table and model are F8's `StockReservation` (by item and location, written by wl-sales through `StockReservations`); migration `20270301500000` adds `lot_id`, `source`, `source_ref`, `released_reason` for the work order output (`source = work_order`). F7's own listener of `SalesOrderConfirmed` and its create migration were dropped: wl-sales reserves at confirm and, since the merge, at `release`/`release_partly` too, on the channels with back-orders allowed (07 turns them on for phone, sales_rep, email and portal, `manufacturing_seed::BACK_ORDER_CHANNELS`). The order's reservation is by item, not by lot; a pick of one lot is refused when the item as a whole has not got it. Migration timestamps `20270102000000` of wl-inventory and manufacturing were moved to `20270102000050` (fnb has the same one on main).

## Deferred (what stays open, provider, why)

Checked against `apps/manufacturing/README.md` ("Deferred in F7") on 2026-10-24: five rows are still open. The rows numbered 1-5, 7, 9-11 of
the original hand-over list (campaign report, onboarding, SOP, supplier, bounce fee and drawings points, the late entry of goods receipts and
returned-goods inspections, the delivery destination, the promised day and the portal PO) were closed by `e5803491`, `6cef772c` and `fda34d13`.

| # | Issue, point | What is missing | Provider | Why not now |
|---|---|---|---|---|
| 6 | BL-SAL-13 (#132) / BL-MFG-14: delivery after the invoice was issued | not billed (one invoice per order, accountant task) | wl-sales / client billing | open decision |
| 8 | BL-MFG-14 calibration certificates kind | no kind, no equipment record | later issue | no entity |
| 12 | BL-MFG-14 signed checklists `WRK.CHK.01` (onboarding, returned goods inspection) | wl-ticketing has only the kernel contract `Checklists` | wl-ticketing | no provider |
| 13 | BL-MFG-09 (#390) scanner/import entry | no scanner or import entry in the app | later issue | nothing to test |
| 14 | BL-MFG-13 (#394) pt 3 | "no line selection for a price dispute" is a client-side show/hide; server ignores lines for a price dispute (tested). Ticked on the issue | none (by design) | not enforceable on the server |

Also open from the spec, not issues of F7: the schedule board, the shortage list at release and `MFG.RUL.01`, `02`, `03`, `05`, `06`.

Not proven end to end (say so in the issues): notice to a customer's main contact is proved as a queued message, not a delivered email; 09:00/09:15 timing of
MFG.RUL.08 is not asserted (the 01:00 expiry look-ahead schedule is, in `host/tests/manufacturing_conformity.rs`); supplier score, cancellation fee and drawing approval are proved at mechanism level;
390 px is a browser crawl, not an automated width test; Postgres migrations and `rnx db:check`/`view:check` were not run.
