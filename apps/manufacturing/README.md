# manufacturing

The vertical app for light manufacturers ([spec](../../docs/verticals/07-manufacturing.md)): the work
order and its lifecycle, the material issued to it, the output a line reports, and the negative flows
of a work order that is stopped. It reaches the other modules only through the contracts of
`wl-kernel` (stock, bills of materials, sales order lines, tickets, notifications), never through
their crates.

The host mounts it when `client.toml` names `app = "manufacturing"`:

```
use renox::prelude::*;

# let _ =
App::new().module(manufacturing::Manufacturing)
# ;
```

This is the base of 07 (`Base: 07 Work order lifecycle`, and the part of `Production reporting` the
work order needs): the lifecycle the issues BL-MFG-01 to BL-MFG-15 build on.

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`work_orders.*`, `plants.manage`), the plant and the production line as scopes, and the role names | §3.1, §3.2 | `permissions` |
| The plant and the production line (hourly rate, store and finished goods locations) | §3.2, §6 | `plant` |
| The Work order with its machine: every state (`planned`, `released`, `in_progress`, `on_hold`, `completed`, `closed`, `cancelled`, `aborted`) and `release` (needs the line and a released BOM version; makes the pick list and keeps the standard cost), `start`, `hold` (material, machine or quality), `resume`, `complete` (needs good output), `close` (calculates the actual cost), `cancel` (only while nothing is issued or reported), `unlink` and `abort`; `cancel`, `close` and `unlink` have a system twin for `MFG.RUL.09` | §6, §7.1 *Work order* | `work_order`, `work_orders` |
| The material pick list | §7.1 `release` | `pick_list` |
| Material issued to a work order, one row per lot at its posted cost, through the kernel's `StockService` (a short quantity waits as a pending issue) | §4.2, §7.1 | `issue` |
| The production report (good quantity and scrap, `produce` into the output lot in `quarantine`) and its machine with both states `posted` and `voided` | §6, §7.1 | `report` |
| The cost of a work order: issued, returned, scrap value, labour and overhead, actual cost, loss | §7.1, `MFG.CST.01` | `cost` |
| Abort after materials were issued (`MFG.ACL.12`, BL-MFG-01, #382): the planner asks with an outcome, the production manager approves (a second step under `STK.ACL.02` above AED 2,000), then the return and `wastage` movements post, the output stays in quarantine with a final inspection requested, the cost is closed and the loss kept | §3.3, §4.2, §7.1 | `abort` |
| Unlink from the sales order to stock (BL-MFG-02, #383) | §4.2, §7.1 | `unlink` |
| The events `WorkOrderReleased`, `WorkOrderCompleted`, `WorkOrderCancelled`, `WorkOrderUnlinked`, `WorkOrderAborted` | §7.1 | `events` |
| The screens: the work order list and the record page with the actions (disabled with the reason when a guard fails) | `docs/UX.md` | `screens`, `pages` |

Comes with other issues (the states they need are declared):

| Issue | What | Where it plugs in |
|---|---|---|
| BL-MFG-03 | `MFG.RUL.09`: listens to the sales order events and calls `work_orders::cancel_by_system`, `unlink::unlink_by_system`, a quantity change and a new planned work order; `quantity_for_stock_milli` is the column | `work_orders`, `unlink` |
| BL-MFG-04 | the `void` of a production report (state `voided`) with its reversing movements, and the `wastage` movement of a report's scrap | `report`, kernel `ProductionStock` |
| BL-MFG-05 | the hold task for material: `hold` takes the reason `material`; the pending issue rows of `issue` | `issue`, `work_orders::hold` |
| BL-MFG-08, 13, 15 | the NCR lifecycle (`open`, `dispositioned`, `closed`, `cancelled`) is not built yet | a new `ncr` module |
| BL-MFG-09 | the lot required on an issue of a lot-tracked material | `issue::issue` |
| BL-MFG-10 | `TracePartyResolver`: a work order, its issues and its output lot are the genealogy | `issue`, `work_order` |
| The shortage list at release, the schedule board, `MFG.RUL.01`, `02`, `05`, `06` | | `work_orders::release` |

Decisions where the spec is silent are in the module headers.
