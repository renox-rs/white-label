# manufacturing

The vertical app for light manufacturers ([spec](../../docs/verticals/07-manufacturing.md)): the work
order and its lifecycle, the material issued to it, the output a line reports, and the negative flows
of a work order that is stopped. It reaches the other modules only through the contracts of
`wl-kernel` (stock, bills of materials, sales order lines, tickets, notifications), never through
their crates. The bill of materials is wl-inventory's (`/boms`, `bom`): the real host releases a work order from a released version.

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
| The production report (good quantity and scrap, `produce` into the output lot in `quarantine`, the scrap as a `wastage` movement) and its machine with both states `posted` and `voided` | §6, §7.1 | `report` |
| Void of a production report entered on the wrong work order (BL-MFG-04, #385, `PLT.ACL.09`, `MFG.ACL.10`): the operator asks with a reason, the line's supervisor or the production manager (`work_orders.void_approve`) approves, never the requester; the report's `produce` and `wastage` movements are reversed, the totals of the work order recalculated and `RecordVoided` emitted; refused with "output already used" once the work order is completed or when the output lot holds less than the report made; the voided report stays struck through with its number and reason | §7.1 *Work order*, §3.3 | `report` |
| Hold for material (BL-MFG-05, #386): a pending issue recorded for an `in_progress` work order gives the supervisor of its line one task to hold it with the reason *material* (a `released` one is not held; its page says it waits); the task closes when the work order goes `on_hold`, stops, or its last pending issue posts or is cleared; when none is open any more and the work order is `on_hold` for material the supervisor gets one task to resume it, none for another reason. The system never holds or resumes. The row of the short part takes the lot and cost it posted with (split in two for a part-covered pending issue) and goes when its source is cancelled | §4.2, §7.1 *Work order* | `material_hold`, `issue` |
| The lot on every issue of a lot-tracked material (BL-MFG-09, #390, `MFG.PRD.02`, `STK.MOV.01`, `STK.LOT.01`): an issue names its lot (the number on the pack), one row and one movement per lot, and is refused without one while a lot of the material is available at the store, whether it comes from the page, a scanner or an import; the refusal lists the lots; only the short quantity that becomes a pending issue has none until it posts; a material that is not lot-tracked needs none. Read through the kernel's `StockService::choices_of_item` | §4.2 step 5, §9 | `issue` |
| The cost of a work order: issued, returned, scrap value, labour and overhead, actual cost, loss | §7.1, `MFG.CST.01` | `cost` |
| Abort after materials were issued (`MFG.ACL.12`, BL-MFG-01, #382): the planner asks with an outcome, the production manager approves (a second step under `STK.ACL.02` above AED 2,000), then the return and `wastage` movements post, the output stays in quarantine with a final inspection requested, the cost is closed and the loss kept | §3.3, §4.2, §7.1 | `abort` |
| Unlink from the sales order to stock (BL-MFG-02, #383) | §4.2, §7.1 | `unlink` |
| The events `NcrRaised`, `NcrDispositioned`, `NcrClosed`, `NcrCancelled`, `NcrReopened`, `WorkOrderReleased`, `WorkOrderCompleted`, `WorkOrderCancelled`, `WorkOrderUnlinked`, `WorkOrderAborted` | §7.1 | `events` |
| The work orders follow their sales order (BL-MFG-03, #384, `MFG.RUL.09`, `SAL.ACL.05`): after the approval `SalesOrderAmended`, `SalesOrderLineCancelled` and `SalesOrderCancelled` (and `SalesOrderLinesReleased` for the paid part of an increase) adjust the work orders linked to the line: a *not started* one (`planned`, or `released` with nothing issued) takes the line's new open quantity (latest planned start gives way first; the pick list of a released one is made again; at 0 it is cancelled and its list withdrawn), a *started* one keeps its quantity, the output above the open quantity is flagged *for stock*, at 0 or with the outcome *finish to stock* it is unlinked, with the outcome *scrap* it is cancelled when nothing was issued or reported and otherwise the production manager gets a task to request the abort (the rule never aborts); an increase takes the *for stock* flag off first, then grows the latest not started work order, else makes a new `planned` one for the difference; *return to stock* changes nothing. Every change is a journal row keyed by the event and the line, so a replay does nothing; the planners are told and the work order's page lists the changes and fills in the abort sheet | §4.1, §7.1, §7.2 | `order_changes`, `work_orders` |
| The NCR (Base 07 *NCR lifecycle*, `MFG.QCL.02`, `MFG.ACL.03`): raised on a lot (cause *supplier* from the lot's goods receipt, *internal process* for own production, else *unknown*), cause changed with a reason until close, cancel while `open`; `disposition`, `close`, `reopen` (by QC, or by the system when the Return to supplier is cancelled or the rework work order is cancelled or aborted); a lot with an NCR open, or dispositioned with its action not done, is not released (`LotReleaseGuard`) | §7.1 *NCR* | `ncr`, `ncrs` |
| Disposition of an NCR (BL-MFG-08, #389, `STK.ACL.02`, `STK.ACL.06`): *rework* creates a planned work order, *scrap* writes off through the kernel's `LotDisposal` (above AED 2,000 after a holder of `stock.approve`), *use as is* waits for a holder of `ncrs.approve` other than the requester and the raiser and then releases the lot, *return to supplier* (credit or replace; refused without a supplier lot, not offered for own production) makes a draft Return to supplier from the quarantine location with the NCR as its source; QC gets the task to close when the action is done, and a task to choose again after a reopen; every disposition stays in the history | §4.3, §4.4, §7.1 *NCR* | `ncrs`, `ncr` |
| A quality dispute on an invoice (BL-MFG-13, #394): the buyer disputes in the portal (`/portal/manufacturing/invoices`) with a reason and, for quality, the lines; `InvoiceDisputed` raises or links one open NCR per lot of the disputed lines (every line when none is ticked; the item and the dispatch line when it has no lot), links the ticket, tells QC; the buyer sees the status only | §4.1, §4.5 | `disputes`, `portal` |
| The supplier scorecard (BL-MFG-15, #396): `SupplierIncidentSource` returns the NCRs with cause supplier that are not cancelled, dated when raised | §7.1 *NCR*, wl-inventory §8 | `incidents` |
| The recall by customer (BL-MFG-10, #391): `ProductionParties` is the app's `TracePartyResolver` - a dispatch is its customer (the recipient; *not shipped* while `draft` or `picked`, *shipped* once it left, not listed when `returned`, `discarded` or `cancelled`), a work order for an undelivered order is that order's customer *not shipped*, a work order for stock has no party but its output lots are in scope; the notice goes to the customer's active portal buyers, else to the customer's main contact. A dispatch that has not left is flagged for a re-pick once per recall with a task for the storekeeper; the dispatches that left and are not delivered are listed for the recall owner to bring back (`/manufacturing/recalls/{id}`) | §4.4 step 8, §9 | `recalls` |
| Product conformity (BL-MFG-11, #392, `MFG.ACL.11`, `DOC.RUL.02`): items flagged regulated (accepted certificate kinds: ECAS, EQM, food-contact; none named: all three) or export only (`/manufacturing/conformity`); `pick` and `dispatch` of a dispatch are refused while no accepted kind is valid on the day (valid through the expiry date; a renewal in review does not count and is named; no override), an export only item is refused for a delivery in the UAE; the nightly look-ahead lists the order lines promised after the expiry | §2, §3.3, §7.1, §9 | `conformity`, wl-inventory `dispatch_guard` |
| Export evidence (BL-MFG-12, #393, `MFG.RUL.08`): daily at 08:00 the accountant is warned at day 60 of a zero-rated export dispatch without a customs exit certificate, bill of lading or airway bill, and at day 90 the dispatch is flagged *5% VAT due* with one task naming the invoices, their amounts and the customer's contract note; evidence stops it, late evidence is told and nothing is reversed; compliance-critical (`/manufacturing/exports`) | §2, §4.1, §7.2, §9 | `exports` |
| The buyer's PO upload (BL-MFG-06, #387): `/portal/manufacturing/orders` refuses a customer PO number (with its call-off number) an order of the customer holds, naming that order's number and status only; asked of wl-sales through the kernel's `CustomerPoCheck` | §4.1, §4.5 step 2 | `po_upload` |
| The screens: the work order list and the record page with the actions (disabled with the reason when a guard fails) | `docs/UX.md` | `screens`, `pages` |

Comes with other issues (the states they need are declared):

| Issue | What | Where it plugs in |
|---|---|---|
| The shortage list at release, the schedule board, `MFG.RUL.01`, `02`, `05`, `06` | | `work_orders::release` |

The BOM base is wl-inventory's (`crates/wl-inventory/src/bom.rs`) and is kept to what the work order
release and the F7 issues need. Left out, to come when an issue needs it: sub-BOMs and recipes with
yield by step, the routing (the work order needs the standard hours of the version only), the
explosion of a BOM against stock (`BomExplosion`, the shortage list at release), the Excel import of
BOMs, and a picker for the BOM version on the new work order sheet (it takes the number shown on the
BOM page). The void and the production manager: the approval of the void of a report is looked for in
the scope of the report's line, so a production manager assigned to the plant only is not found there;
assign the role on the line or globally.

Decisions where the spec is silent are in the module headers.

Not built for the NCR: the lot's user `reject` (and the NCR it creates at incoming inspection) and
the user `release` of a lot are the lot issues of wl-inventory; an NCR is raised by QC on a lot
(`/manufacturing/ncrs`), and the release guard is asked where wl-inventory releases a lot (the
decision of a stock hold). The quarantine location of a Return to supplier is the one of
wl-inventory; the scrap limit of `STK.ACL.02` is AED 2,000 here, as for the abort.

Seams left for the sales module (nothing provides them in the real host yet,
so the matching Done-when points are proved with fakes): `OrderLines`
(the customer of a work order's order), `DeliveryDestination` (whether an
order is delivered abroad), `PromisedLines` (the look-ahead of `DOC.RUL.02`),
`CustomerPoCheck` (BL-SAL-18) and `OrderLineRules` (the refusal of an export
only item at `submit` and `amend`). wl-sales provides or calls them once F6 is
merged.
