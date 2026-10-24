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
| Abort after materials were issued (`MFG.ACL.12`, BL-MFG-01, #382): the planner asks with an outcome, the production manager approves (a second step under `STK.ACL.02` above AED 2,000), then the return and `wastage` movements post, the output stays in quarantine with a final inspection requested, the cost is closed and the loss kept; the aborted work orders report (`/manufacturing/aborted`: reason, who asked, who approved, the loss for `work_orders.cost_view`) | §3.3, §4.2, §7.1 | `abort`, `screens::aborted_page` |
| Unlink from the sales order to stock (BL-MFG-02, #383) | §4.2, §7.1 | `unlink` |
| The output lot reserved for the sale line (BL-MFG-02 point 5, BL-MFG-03, #383, #384): every good quantity reported (and every void of one) sets the lot aside for the line through the kernel's `ProductionStock::reserve_output`, up to the line's open quantity less the part flagged *for stock*; the part for stock and anything above the open quantity is free stock as soon as it is reported; completing keeps it; `unlink` for a cancelled line (and the abort) gives the lot back with `release_reservation`, which answers `true` when a reservation was active. A line that is still open keeps the reservation when the work order is unlinked. Proved with the real stock, sales and pages in `host/tests/manufacturing_reservation.rs` | §7.1, `MFG.RUL.09` | `output_reservation`, `report`, `unlink` |
| The events `NcrRaised`, `NcrDispositioned`, `NcrClosed`, `NcrCancelled`, `NcrReopened`, `WorkOrderReleased`, `WorkOrderCompleted`, `WorkOrderCancelled`, `WorkOrderUnlinked`, `WorkOrderAborted` | §7.1 | `events` |
| The work orders follow their sales order (BL-MFG-03, #384, `MFG.RUL.09`, `SAL.ACL.05`): after the approval `SalesOrderAmended`, `SalesOrderLineCancelled` and `SalesOrderCancelled` (and `SalesOrderLinesReleased` for the paid part of an increase) adjust the work orders linked to the line: a *not started* one (`planned`, or `released` with nothing issued) takes the line's new open quantity (latest planned start gives way first; the pick list of a released one is made again; at 0 it is cancelled and its list withdrawn), a *started* one keeps its quantity, the output above the open quantity is flagged *for stock*, at 0 or with the outcome *finish to stock* it is unlinked, with the outcome *scrap* it is cancelled when nothing was issued or reported and otherwise the production manager gets a task to request the abort (the rule never aborts); an increase takes the *for stock* flag off first, then grows the latest not started work order, else makes a new `planned` one for the difference; *return to stock* changes nothing. Every change is a journal row keyed by the event and the line, so a replay does nothing; the planners are told and the work order's page lists the changes and fills in the abort sheet | §4.1, §7.1, §7.2 | `order_changes`, `work_orders` |
| Returned and refused goods (BL-MFG-07, #388, `MFG.RUL.10`, `MFG.ACL.02`): when wl-inventory books goods of a dispatch into quarantine (`DispatchGoodsQuarantined`: the whole of a dispatch that came back with its complaint ticket, or the refused quantity received back at run close) the rule makes one inspection per lot (a checklist run `returned_goods_inspection` where wl-ticketing provides checklists, else a QC task; a returned dispatch's inspection is linked to its complaint ticket), and one decision per dispatch and order line for the account's sales person (the order's owner, else the sales role): the order, line, quantity and reasons, with *New dispatch* (a draft copy of the original dispatch, `DispatchMaker`) and *Cancel the line*; a task and a notice, nothing cancelled and no dispatch made by the rule. The decision's open quantity goes down by what a new dispatch that left carries (`DispatchSent`) and by what `cancel_line` cancels; at 0 the task closes, a part (10 of 20) keeps it open; a cancelled order closes it. Idempotent per event. QC records the result on `/manufacturing/inspections`: *Pass* releases the lot, *Fail* raises an NCR; a lot with an inspection open is not released. Pages `/manufacturing/returns` (sales, planner, production manager) and `/manufacturing/inspections` (QC) | §4.1, §7.1, §7.2 | `returns`, `screens::returns_page`, `screens::inspections_page` |
| The lines that have started, asked by wl-sales for `SAL.ACL.05` (BL-MFG-03): a line with a started work order, or goods picked or out | §7.1 | `progress` |
| The NCR (Base 07 *NCR lifecycle*, `MFG.QCL.02`, `MFG.ACL.03`): raised on a lot (cause *supplier* from the lot's goods receipt, *internal process* for own production, else *unknown*), cause changed with a reason until close, cancel while `open`; `disposition`, `close`, `reopen` (by QC, or by the system when the Return to supplier is cancelled or the rework work order is cancelled or aborted); a lot with an NCR open, or dispositioned with its action not done, is not released (`LotReleaseGuard`) | §7.1 *NCR* | `ncr`, `ncrs` |
| Disposition of an NCR (BL-MFG-08, #389, `STK.ACL.02`, `STK.ACL.06`): *rework* creates a planned work order, *scrap* writes off through the kernel's `LotDisposal` (above AED 2,000 after a holder of `stock.approve`), *use as is* waits for a holder of `ncrs.approve` other than the requester and the raiser and then releases the lot, *return to supplier* (credit or replace; refused without a supplier lot, not offered for own production) makes a draft Return to supplier from the quarantine location with the NCR as its source; QC gets the task to close when the action is done, and a task to choose again after a reopen; every disposition stays in the history | §4.3, §4.4, §7.1 *NCR* | `ncrs`, `ncr` |
| A quality dispute on an invoice (BL-MFG-13, #394): the buyer disputes in the portal (`/portal/manufacturing/invoices`) with a reason and, for quality, the lines; `InvoiceDisputed` raises or links one open NCR per lot of the disputed lines (every line when none is ticked; the item and the dispatch line when it has no lot), links the ticket, tells QC; the buyer sees the status only | §4.1, §4.5 | `disputes`, `portal` |
| The supplier scorecard (BL-MFG-15, #396): `SupplierIncidentSource` returns the NCRs with cause supplier that are not cancelled, dated when raised | §7.1 *NCR*, wl-inventory §8 | `incidents` |
| The recall by customer (BL-MFG-10, #391): `ProductionParties` is the app's `TracePartyResolver` - a dispatch is its customer (the recipient; *not shipped* while `draft` or `picked`, *shipped* once it left, not listed when `returned`, `discarded` or `cancelled`), a work order for an undelivered order is that order's customer *not shipped*, a work order for stock has no party but its output lots are in scope; the notice goes to the customer's active portal buyers, else to the customer's main contact. A dispatch that has not left is flagged for a re-pick once per recall with a task for the storekeeper; the dispatches that left and are not delivered are listed for the recall owner to bring back (`/manufacturing/recalls/{id}`) | §4.4 step 8, §9 | `recalls` |
| Product conformity (BL-MFG-11, #392, `MFG.ACL.11`, `DOC.RUL.02`): items flagged regulated (accepted certificate kinds: ECAS, EQM, food-contact; none named: all three) or export only (`/manufacturing/conformity`); `pick` and `dispatch` of a dispatch are refused while no accepted kind is valid on the day (valid through the expiry date; a renewal in review does not count and is named; no override), an export only item is refused for a delivery in the UAE; the nightly look-ahead lists the order lines promised after the expiry | §2, §3.3, §7.1, §9 | `conformity`, wl-inventory `dispatch_guard` |
| Export evidence (BL-MFG-12, #393, `MFG.RUL.08`): daily at 08:00 the accountant is warned at day 60 of a zero-rated export dispatch without a customs exit certificate, bill of lading or airway bill, and at day 90 the dispatch is flagged *5% VAT due* with one task naming the invoices, their amounts and the customer's contract note; evidence stops it, late evidence is told and nothing is reversed; compliance-critical (`/manufacturing/exports`) | §2, §4.1, §7.2, §9 | `exports` |
| The buyer's PO upload (BL-MFG-06, #387): `/portal/manufacturing/orders` refuses a customer PO number (with its call-off number) an order of the customer holds, naming that order's number and status only; asked of wl-sales through the kernel's `CustomerPoCheck` | §4.1, §4.5 step 2 | `po_upload` |
| The outcome proposed for a cancelled started line (BL-MFG-14, #395, 07 §7.1 *Sales order*): items flagged *made for one customer* on `/manufacturing/conformity` (printed labels, packaging; off by default) are proposed *scrap*, any other item *finish to stock*; `OutcomeProposals` is the kernel contract wl-sales reads to preselect the outcome in the cancel-order and cancel-remaining sheets; sales may change it per line | §6 *Item*, §7.1 | `outcome_proposals`, `conformity` |
| A production report typed in later (BL-MFG-14, `PLT.ACL.10`): the report sheet takes the time the output was made (`occurred_at`), a reason and an outage reference when the line tablets were down and the supervisor enters the paper job cards at the end of the shift; the report keeps that business time; more than M minutes late it is flagged with the reason, more than N hours late it waits for a holder of `late_entries.approve` of the plant (the production manager) and is posted when one approves; the limits M and N are the entity's (host seed: 30 minutes, 24 hours) | §7.1 *Integrations*, *Late entry* | `late_report`, `report` |
| The result on a returned lot typed in later (BL-MFG-14, BL-MFG-07, `PLT.ACL.10`): the *Pass* and *Fail* sheets of `/manufacturing/inspections` take the time the goods were inspected (`occurred_at`), a reason and an outage reference; the inspection keeps that time as `decided_at` (shown on the page); more than M minutes late it is flagged with the reason (422 in the sheet without one, *Entered late: ...* on the page), more than N hours late it waits for a holder of `late_entries.approve` (the production manager): the inspection stays open, the buttons are disabled with the reason and the lot is released (or the NCR raised) when one approves; the limits are the entity's `manufacturing.late_inspection` (host seed: 30 minutes, 24 hours). Left empty, the result is made now, as before | §7.1 *Late entry*, §7.2 `MFG.RUL.10` | `late_inspection`, `returns`, `screens::inspections_page` |
| Who is told that an item's certificate expires (BL-MFG-14, `DOC.RUL.01`): the QC inspectors of each plant that holds stock of the item (its lines' store and finished goods locations, through `StockService::on_hand`) or has an open work order for it; when no plant does, those of every plant | §7.1 *Responsible role per document kind* | `responsible` |
| The work order as an agreement of a blocked customer (BL-MFG-14, wl-parties §5.6): the kind `work_order` continues and the planner decides (`unlink` to stock); the sales orders not yet dispatched are paused by wl-sales | §7.1 *Party blocked* | `lib` |
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

Seams of the sales module: wl-sales provides `OrderLines` (the customer and open quantity of a
work order's order, BL-MFG-03), `FulfilmentProgress` is provided here (`progress`), `CustomerPoCheck`
(BL-SAL-18) and the `OutcomeProposals` reader are in place. `DeliveryDestination` (whether an order is delivered abroad), `PromisedLines` (the
look-ahead of `DOC.RUL.02`) and the call of `OrderLineRules` at `submit` and `amend` are provided by
wl-sales too (the order's delivery address with its domestic or export answer, and a promised day on
each line), and the host tests run them for real (`manufacturing_conformity.rs`: the order pages refuse an
export only item on a domestic order at `submit` and `amend`, the look-ahead lists the confirmed lines
promised after the expiry, and the 01:00 Asia/Dubai schedule is asserted through the list of scheduled tasks).
The buyer's refusal in the portal (BL-SAL-18 point 2) is tested against real wl-sales orders in
`host/tests/manufacturing_po_number.rs`: it names the number and status of the order holding the PO and nothing else.

Not built for the returned goods: the checklist template `returned_goods_inspection` and its field pages
are `WRK.CHK.01` of wl-ticketing, which only has the kernel contract (`Checklists`) so far; until it
provides one the QC inspector works from `/manufacturing/inspections`. Releasing a lot sets its status;
moving the goods from the quarantine location to a store is a transfer by the storekeeper.

## Deferred in F7 (what stays open, who provides it)

Each row is a Done-when point or a seam that is not built or not testable end to end yet. The list is
kept with the audit of the round (`DEFERRED-F7`).

| Issue, point | What is missing | Provider |
|---|---|---|
| BL-SAL-13 / BL-MFG-14: an invoice already issued, a later delivery | the order keeps one invoice: a delivery accepted after the invoice was issued is not billed by wl-sales (the accountant gets a task naming the invoice); a second invoice per delivery is a client billing choice | wl-sales / client billing |
| BL-MFG-14 *Documents*: calibration certificates | `DOC.EXP.01` lists calibration, but the host seed registers no calibration kind: there is no equipment record to attach it to | a later issue (equipment) |
| BL-MFG-14 *Onboarding*: the checklist as a signed `WRK.CHK.01` | wl-ticketing only has the kernel contract for checklists | wl-ticketing (`WRK.CHK.01`) |
| BL-MFG-09 surface: a scanner or import entry of the lot | no scanner or import entry exists in the app; the lot check is the service every entry shares, tested through the issue route | a later issue (scanner/import) |
| BL-MFG-13 point 3 | the lines are asked only for a quality dispute by a block the portal page shows when the kind is quality (tested as rendered); the choice is a client-side show/hide of Renox, not enforced by the server beyond ignoring the lines of a price dispute | none (by design) |
| Returned goods: the inspection checklist `returned_goods_inspection` | see *Not built for the returned goods* above | wl-ticketing (`WRK.CHK.01`) |

