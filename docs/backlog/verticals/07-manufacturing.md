# Epic: 07 Manufacturing — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows that belong to the
[07 Manufacturing](../../verticals/07-manufacturing.md) vertical itself: the work order's `abort`
and `unlink`, how work orders follow an amended or cancelled sales order (`MFG.RUL.09`), voiding a
production report, the material-shortage hold, the duplicate customer PO guard, returned and
refused goods (`MFG.RUL.10`), the NCR's return to supplier, lot capture on material issue, the
recall resolver, the conformity-certificate block (`MFG.ACL.11`), the export evidence alert
(`MFG.RUL.08`), quality disputes, and the vertical's values for every shared negative-flow
mechanism. Shared behaviour (sales order amend and cancel, credit re-check, partial delivery,
stock holds, recalls, returns to suppliers, write-off, cheques) is built in the module epics and
linked by ID. It closes 07-N2-a, 07-N4-a, 07-N4-c and 07-N10-a (vertical parts), the 07 MEDIUM
items of review §6, and the 07 parts of X-02, X-05, X-08, X-09, X-11, X-13, X-14 and X-19.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-MFG-01 | Abort a work order after materials were issued (`MFG.ACL.12`) | high | m | Launch 07 | Base: 07 Work order lifecycle; BL-KRN-01 |
| BL-MFG-02 | Unlink a work order from its sales order to stock | high | s | Launch 07 | Base: 07 Work order lifecycle |
| BL-MFG-03 | Adjust work orders when a sales order is amended or cancelled (`MFG.RUL.09`) | high | m | Launch 07 | BL-MFG-01; BL-MFG-02; BL-SAL-05; BL-SAL-07; BL-SAL-08; BL-SAL-09 |
| BL-MFG-04 | Void a production report entered on the wrong work order | medium | s | Launch 07 | Base: 07 Production reporting; BL-KRN-03 |
| BL-MFG-05 | Hold a work order for material when an issue is pending | medium | s | Launch 07 | Base: 07 Work order lifecycle; BL-STK-18; BL-STK-20 |
| BL-MFG-06 | Refuse a second sales order with the same customer PO number | medium | s | Launch 07 | BL-SAL-18; Base: wl-portal reorder and PO upload |
| BL-MFG-07 | Inspect returned or refused goods and ask sales to re-deliver or cancel (`MFG.RUL.10`) | high | s | Launch 07 | BL-STK-22; BL-SAL-13; BL-SAL-08 |
| BL-MFG-08 | Disposition an NCR as return to supplier or scrap, and close it | high | m | Launch 07 | Base: 07 NCR lifecycle; BL-STK-03; BL-STK-05; BL-INV-13; BL-STK-12 |
| BL-MFG-09 | Record the lot on every issue of a lot-tracked material to a work order | high | s | Launch 07 | Base: wl-inventory Lot tracking and stock movements |
| BL-MFG-10 | Resolve recall trace lines to customers (`TracePartyResolver` for 07) | high | m | Launch 07 | BL-STK-14; BL-STK-15; BL-MFG-09 |
| BL-MFG-11 | Block picking of regulated items without a valid conformity certificate (`MFG.ACL.11`) | high | m | Launch 07 | BL-DOC-01; BL-DOC-04; BL-DOC-05 |
| BL-MFG-12 | Alert on missing export evidence and open the VAT correction (`MFG.RUL.08`) | high | m | Launch 07 | BL-KRN-08; Base: wl-invoicing Invoice and Credit note lifecycle |
| BL-MFG-13 | Link a quality dispute on an invoice to an NCR | medium | s | Launch 07 | BL-INV-04; BL-INV-05 |
| BL-MFG-14 | 07 Manufacturing: configure shared negative-flow settings | high | m | Launch 07 | BL-KRN-03; BL-KRN-04; BL-KRN-05; BL-KRN-08; BL-DOC-01; BL-PTY-03; BL-INV-01; BL-INV-02; BL-INV-04; BL-PAY-09; BL-PAY-10; BL-PAY-14; BL-SAL-07; BL-SAL-12; BL-STK-03; BL-STK-11; BL-STK-18; BL-DLV-02; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-STK-28; BL-STK-29; BL-STK-30; BL-STK-31; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |
| BL-MFG-15 | Report supplier-caused NCRs to the supplier scorecard | medium | s | Launch 07 | BL-STK-29 |

---

### BL-MFG-01 — 07 Manufacturing: abort a work order after materials were issued (`MFG.ACL.12`)

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** Base: 07 Work order lifecycle (`release`, `start`, `hold`, `complete`) · Base: wl-inventory stock movements (`return`, `wastage`) and lots · BL-KRN-01 (approval ends without a yes) · **Blocks:** BL-MFG-03
**Review items:** 07-N2-a, X-14, 07 MEDIUM "work order aborted after issue"

#### Goal
A work order that has already consumed material or reported output cannot simply be cancelled.
The planner or supervisor asks to `abort` it with a reason and an outcome for everything on it:
unused materials go back to the store, the work in progress is scrapped, and good output already
made becomes a lot in quarantine. The production manager (never the requester) approves. The
actual cost is closed and the loss reported, so the business sees what the stop cost.

#### Files to read
- `docs/verticals/07-manufacturing.md` §3.3 (`MFG.ACL.12`, `STK.ACL.02`), §4.2 "When it goes wrong" (first row), §7.1 *Work order* (states, transitions, diagram), §5 `MFG.CST.01`
- `docs/modules/wl-kernel.md` §4.1 *Approval request* (what happens when the answer is not yes)
- `docs/modules/wl-inventory.md` §4 *Lot* (creation in `quarantine`), stock movements `return` and `wastage`

#### Terms
- **Work order (WO):** an instruction to produce a quantity of one item on one line, from a BOM version.
- **Issued material:** stock moved from the store to the WO (movement `issue`), with lot when lot-tracked.
- **Work in progress (WIP):** material issued to the WO that is neither back in the store nor in a good output lot.
- **Good output:** quantity reported as good on production reports of this WO.
- **Outcome line:** one row of the abort request: an issued material to return (item, lot, qty), a WIP scrap amount, or good output to quarantine (qty).
- **`MFG.ACL.12`:** access rule: abort only with outcomes, approved by the production manager, not by the requester.
- **`STK.ACL.02`:** shared access rule: wastage above AED 2,000 (07 value) needs approval; the recorder cannot approve.
- **Quarantine:** lot state "received or produced; not usable until inspected".
- **Output lot:** the WO's one lot of its item, created in `quarantine` by the first production report with good quantity (`produce` movement); the finished lot.
- **Line rate:** the hourly rate of the WO's line, covering labour and overhead (`MFG.CST.01`).
- **Start/stop hours:** hours from the start time recorded by `start` to the stop time (for `abort`, the final approval time), time `on_hold` included; 0 when the WO never started.
- **Standard cost per unit:** the BOM version's rolled-up cost: materials at landed cost (the *standard material cost per unit*) plus standard hours × the line rate.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `aborted` | error | yes | Stopped after materials were issued; work in progress returned, scrapped or put to quarantine; actual cost closed |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `abort` | released, in_progress, on_hold | aborted | `work_orders.release` | material issued or output reported; reason given; outcome per line: unused materials back to store (not more than issued less returned per item and lot), work in progress scrapped, good output to a lot in quarantine; one approval request: step 1 the production manager, not the requester (`MFG.ACL.12`), step 2 under `STK.ACL.02` only when the scrap value is above its limit; the posted quantities still match the request at final approval | at final approval, in one transaction: return and `wastage` movements posted; good output stays in the work order's output lot in `quarantine` with a final inspection requested; actual cost calculated and the loss reported (`MFG.CST.01`); linked sales order line's owner told the outcome; event `WorkOrderAborted` |
| `cancel` | planned, released, in_progress, on_hold | cancelled | `work_orders.release`, or system (`MFG.RUL.09`) | no material issued and no output reported; reason given from `in_progress` or `on_hold` | pick list withdrawn; sales order link removed; an open breakdown ticket stays open |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `MFG.ACL.12` | A work order that has issued materials or reported output is stopped only by `abort`, with an outcome for the work in progress (materials back to store, scrap, partial output to quarantine), approved by the production manager; the user who asked for the abort cannot approve it | `STA`, `SOD` | planner, supervisor, production manager | on |
| `STK.ACL.02` *(shared, formerly `MFG.ACL.09`)* | Stock adjustments above a value need approval, and the user who made the adjustment cannot approve it | `LIM`, `SOD` | storekeeper | AED 2,000 |

§4.2 "When it goes wrong": *A work order must stop after materials were issued (order
cancelled, wrong BOM, material defect)* → Work order `abort` with approval (`MFG.ACL.12`): unused
issued materials go back to the store, the work in progress is scrapped (wastage, `STK.ACL.02`
above AED 2,000), good output already made goes to quarantine as a lot. Actual cost is closed
and the loss reported.

§7.1 (paragraphs under the Work order table): a work order is *started* once it is
`in_progress` or `on_hold`, or has material issued or output reported. Each production report
posts a `produce` movement of its good quantity into the work order's output lot and a
`wastage` movement of its scrap; the output lot (one per work order) is created in
`quarantine` by the first report with good quantity. While an `abort` request waits, new
issues and production reports on the work order are refused ("abort pending"); if the posted
quantities differ at final approval (a late entry), the request fails and a new one is made.
The scrap value is the cost of materials issued, less materials returned, less good output ×
the BOM's standard material cost per unit, never below 0, at the cost each issue was posted
with; a returned quantity is valued at the average unit cost of this work order's issues of
the same item and lot (issued value ÷ issued quantity). The loss is the actual cost
(materials issued less returned, plus start/stop hours × the line rate) less good output at
standard cost; it is shown on the work order and in the aborted work orders report.
Start/stop hours run from the start time recorded by `start` to the stop time (for `abort`,
the final approval time; 0 when the work order never started), time `on_hold` included. The
line rate is the hourly rate of the work order's line (`MFG.CST.01`). Standard cost per unit
is the BOM version's rolled-up cost: materials at landed cost plus standard hours × the line
rate.

Effects in order (one transaction, at final approval):
1. Post a `return` movement per material line back to the WO's store location (with its lot).
2. Post one `wastage` movement per scrapped WIP material line, reason = abort reason.
3. Keep the good output in the WO's output lot in `quarantine` (it is already there from the production reports; no new lot) and request a final inspection.
4. Calculate actual cost (`MFG.CST.01`) and store the loss (see edge cases).
5. Notify the linked sales order line's owner (sales) with the outcome.
6. Emit `WorkOrderAborted { work_order, reason, returned, scrapped, output_lot, loss, approver }`.

#### Edge cases decided
- **Who may ask:** holders of `work_orders.release` (planner, supervisor). The request is a kernel approval request (`PLT.APR.01`); the WO stays in its state while it waits.
- **One request, two rules:** one approval request with step 1 = production manager (`MFG.ACL.12`) and, only when the scrap value is above AED 2,000, step 2 = an approver under `STK.ACL.02`. The same production manager may approve both steps if they hold both permissions; the requester can approve neither.
- **Scrap value:** cost of materials issued − cost of materials returned − (good output qty × the BOM's standard material cost per unit), never below 0, at the cost each issue was posted with. A return of an item and lot is valued at the average unit cost of this WO's issues of that item and lot (e.g. 300 kg R1 issued at AED 8 and 200 kg R1 at AED 9 → AED 8.40/kg; 100 kg returned = AED 840).
- **Loss reported:** actual cost (materials issued − returned + start/stop hours × line rate) − good output qty × standard cost per unit; may be negative (a gain) and is stored as calculated. Shown on the WO and in the "aborted work orders" report.
- **Changes while the approval waits:** while an abort request is pending, new issues and production reports on the WO are refused with "abort pending". If the posted quantities still differ at final approval (e.g. a late entry), the kernel `fail`s the request (BL-KRN-01) and the requester submits a new one.
- **Rejected or returned:** the WO stays in the state it had (released, in_progress or on_hold); nothing is posted.
- **Return quantity:** a material line cannot return more than issued − already returned for that item and lot.
- **Nothing issued and nothing reported:** use `cancel`, also from `in_progress` or `on_hold` (with a reason); `abort` is not offered. An open breakdown ticket stays open.
- **Breakdown ticket open (reason *machine*):** stays open; abort does not close tickets.
- **Time:** movement and abort timestamps in Asia/Dubai; the WO's actual stop time = approval time.

#### Must not
- Must not post any movement before final approval.
- Must not let the requester approve any step, even as an admin.
- Must not allow `cancel` once any material was issued or any output reported.
- Must not put good output straight into `released`: it always goes to `quarantine` (QC releases under `MFG.ACL.02`).
- Must not write `status` directly.

#### Done when
- [ ] Given WO-100 on line L1 (rate AED 120/hour), started 2026-11-02 08:00, `in_progress` with 500 kg resin lot R1 issued at AED 8/kg (AED 4,000), 200 kg to return and 50 good units reported in output lot F100, BOM standard material cost AED 18/unit and standard cost per unit AED 24, when planner A asks to `abort` with reason "wrong BOM" and outcome lines (return 200 kg R1; scrap the rest; 50 units to quarantine), then an approval request exists with scrap value AED 1,500 (4,000 − 1,600 − 50 × 18) and WO-100 stays `in_progress`.
- [ ] Given that request, when A tries to approve, it is refused.
- [ ] When production manager B approves at 13:00 on 2026-11-02, then 200 kg R1 (AED 1,600) is returned to the store, a `wastage` movement of AED 1,500 is posted, F100 with 50 units stays in `quarantine` with a final inspection requested, WO-100 is `aborted`, the loss stored is AED 1,800 (2,400 + 5 h × 120 − 50 × 24), sales is notified and `WorkOrderAborted` is emitted.
- [ ] Given the same WO with no material returned (scrap value AED 3,100 = 4,000 − 50 × 18), the request has a second step under `STK.ACL.02` and nothing posts until it is approved.
- [ ] When B rejects, WO-100 stays `in_progress` and no movement exists.
- [ ] While the request is pending, a production report on WO-100 is refused.
- [ ] Given a `released` WO with no material issued, `abort` is not offered and `cancel` is.
- [ ] Given an `in_progress` WO with nothing issued and nothing reported, `abort` is not offered and `cancel` with reason "order cancelled" makes it `cancelled`.
- [ ] Given a `completed` WO, `abort` is not offered.

#### Out of scope
How a sales order cancellation asks for the abort (BL-MFG-03); final inspection of the output lot (base QC); reversing a completed WO (`MFG.ACL.10`, reversing movements, base).

---

### BL-MFG-02 — 07 Manufacturing: unlink a work order from its sales order to stock

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: 07 Work order lifecycle · **Blocks:** BL-MFG-03
**Review items:** 07-N2-a, X-14

#### Goal
When the customer cancels but the goods are worth finishing (standard items), the work order
keeps running but no longer belongs to the sales order. `unlink` changes its source to stock, so
its output goes to free stock instead of being reserved for the cancelled order.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.2 "When it goes wrong" (second row), §7.1 *Work order* (row `unlink`), §7.2 `MFG.RUL.09`

#### Terms
- **Source:** where a WO's demand comes from: a sales order line, or stock (minimum level or plan).
- **Free stock:** finished goods not reserved for any sales order.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `unlink` | planned, released, in_progress, on_hold | same state | `work_orders.release` or system (`MFG.RUL.09`: outcome *finish to stock*, or the line's open quantity cut to 0) | reason given; source is still the sales order | source changed from the sales order to stock; output goes to free stock, and output lots already reserved for a cancelled line are released to free stock; planner notified |

§4.2: *A sales order is cancelled but the goods are worth finishing* → Work order `unlink`: it
keeps running and its output goes to free stock.

#### Edge cases decided
- **State:** unchanged; history row written with the reason (self-transition).
- **Already unlinked (source = stock):** `unlink` is not offered; a system call is a no-op (idempotent).
- **Output already reported and reserved for the order:** output lots reserved to a sales order line that is still open stay reserved; reservations for a cancelled line are released to free stock at `unlink`.
- **Cost:** actual cost stays on the WO; it is no longer attributed to the order's margin.
- **Completed WO:** not allowed (not in From); its output is handled by the sales cancellation outcome *return to stock*.

#### Must not
- Must not change the WO's quantity, line or state.
- Must not keep reserving output for a cancelled line.

#### Done when
- [ ] Given WO-200 `in_progress` linked to SO-50 line 1, when planner A runs `unlink` with reason "customer cancelled, standard item", then WO-200 is `in_progress`, source = stock, the planner is notified and a history row has the reason.
- [ ] When WO-200 later completes 1,000 units, the lot is not reserved for SO-50.
- [ ] `unlink` without a reason is refused.
- [ ] Given WO-200 already source = stock, `unlink` is not offered.
- [ ] Given lot F9 of WO-201 reserved for SO-51 line 2, when line 2 is cancelled and WO-201 is `unlink`ed, F9 is free stock.

#### Out of scope
The automatic call from `MFG.RUL.09` (BL-MFG-03).

---

### BL-MFG-03 — 07 Manufacturing: adjust work orders when a sales order is amended or cancelled (`MFG.RUL.09`)

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** BL-MFG-01 · BL-MFG-02 · BL-SAL-05 (amend) · BL-SAL-07 (cancel with outcome) · BL-SAL-08 (`cancel_line`) · BL-SAL-09 (`cancel_remaining`) · **Blocks:** None
**Review items:** 07-N2-a, X-14, 07 MEDIUM "order amendment"

#### Goal
When sales amends a sales order or cancels it (or a line), the linked work orders must follow:
not-started work orders take the new quantity or are cancelled; started ones keep running (for
the line or, unlinked, for stock), or, for outcome *scrap*, the production manager gets a task
to request the `abort` (the rule never aborts by itself), all after the approval under
`SAL.ACL.05`. The planner is always told.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.1 "When it goes wrong" (rows 1–2), §7.1 *Sales order* (rows `amend`, `cancel_line`, `cancel_remaining`, `cancel`, paragraph "Cancellation outcomes"), §7.1 *Work order* (rows `cancel`, `unlink`, `abort`), §7.2 `MFG.RUL.09`
- `docs/modules/wl-sales.md` §4.2 (events `SalesOrderAmended`, `SalesOrderLineCancelled`, `SalesOrderCancelled` and their payload)
- `docs/OPEN-ITEMS.md` A-04

#### Terms
- **Started WO:** a WO in `in_progress` or `on_hold`, or any WO with material issued or output reported (§7.1). Every other WO (`planned`, or `released` with nothing issued) is *not started*.
- **Outcome:** what happens to a cancelled line's goods, chosen by sales per line under `SAL.ACL.05`: *finish to stock*, *scrap*, *return to stock* (picked finished goods only).
- **`SAL.ACL.05`:** shared rule: amending or cancelling after the WO started or goods were picked needs an outcome per line and a manager's approval; fee 0% by default.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MFG.RUL.09` | `SalesOrderAmended`, `SalesOrderLineCancelled` or `SalesOrderCancelled` | linked work orders exist | not started: change the quantity to the line's new open quantity or, at 0, `cancel`; started: an increase creates a planned work order for the difference; a decrease keeps the work order's quantity and flags the output above the line's new open quantity *for stock* (released to free stock when reported); an open quantity cut to 0 or outcome *finish to stock* runs `unlink`; outcome *scrap* gives the production manager a task to request `abort` (`MFG.ACL.12`), or runs `cancel` when nothing was issued or reported; notify planner | — | on | MVP |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `amend` (sales order) | on_credit_hold, confirmed, partially_delivered | same state, or on_credit_hold | `sales_orders.amend` | reason; no line below its delivered quantity; `SAL.ACL.02`; approval when a linked work order started or goods were picked (`SAL.ACL.05`) | new revision, previous kept; work orders adjusted (`MFG.RUL.09`); credit check re-run when the value rises; revised confirmation sent; event `SalesOrderAmended` |
| `cancel` (work order) | planned, released, in_progress, on_hold | cancelled | `work_orders.release`, or system (`MFG.RUL.09`) | no material issued and no output reported; reason given from `in_progress` or `on_hold` | pick list withdrawn; sales order link removed; an open breakdown ticket stays open |

§7.1: Cancellation outcomes offered in this vertical: *finish to stock* (default for standard
items), *scrap* (default for customer-specific items such as printed labels and packaging),
*return to stock* for picked finished goods. Cancellation fee: 0% by default; the client may
set a percentage per customer. VAT on a fee kept from a prepayment is `⚠ open: accountant (A-04)`.

§4.1: Work orders not yet started take the new quantity; a started work order keeps its
quantity, an increase creates an extra planned work order and a decrease finishes the excess to
stock (`MFG.RUL.09`).

§7.1: A work order is *started* once it is `in_progress` or `on_hold`, or has material issued
or output reported; this decides whether `SAL.ACL.05` and the outcomes of `MFG.RUL.09` apply.

#### Edge cases decided
- **Runs after approval only:** the rule listens to the events, which wl-sales emits after any `SAL.ACL.05` approval. Nothing happens to WOs while the amendment or cancellation waits for approval.
- **Not started, quantity changed:** the WO quantity is set to the line's new open quantity; the pick list is regenerated. New quantity 0 → WO `cancel`.
- **Started, increase:** a new `planned` WO for the difference, same item and BOM version, linked to the same line.
- **Started, decrease:** when the line still has open quantity, the WO stays linked, keeps its quantity, and its output above the line's new open quantity is flagged *for stock* (field *quantity for stock*; released to free stock when reported); when the open quantity becomes 0, the WO is `unlink`ed.
- **Several WOs on one line:** adjust the not-started ones first (latest planned start first), then started ones.
- **Outcome *scrap*:** the rule does not abort by itself: it creates a task for the production manager to request `abort` (BL-MFG-01) with the WO prefilled; the planner is notified. A started WO with nothing issued and nothing reported is `cancel`led by the rule instead.
- **Outcome *return to stock*:** concerns picked finished goods (wl-sales/wl-inventory, BL-STK-26); no WO change.
- **Duplicate events:** idempotent per (event id, WO); a replay changes nothing.
- **Cancellation fee VAT:** not computed here (A-04); fee invoicing is wl-sales.

#### Must not
- Must not abort a WO automatically.
- Must not change the quantity of a started WO.
- Must not act before the sales approval is final.

#### Done when
- [ ] Given SO-60 line 1 qty 1,000 with WO-300 `planned` for 1,000, when the amendment to 800 is final, then WO-300 is for 800 and the planner is notified.
- [ ] Given amendment to 0 on that line, WO-300 is `cancelled`, its pick list withdrawn.
- [ ] Given WO-301 `in_progress` for 1,000 and an increase to 1,300, a new `planned` WO for 300 is linked to the line.
- [ ] Given WO-301 `in_progress` and the line cancelled with outcome *finish to stock*, WO-301 is `unlink`ed.
- [ ] Given WO-301 `in_progress` and outcome *scrap*, the production manager gets an abort task; WO-301 stays `in_progress`.
- [ ] Given WO-302 `in_progress` with nothing issued or reported and outcome *scrap*, WO-302 is `cancelled`.
- [ ] Given WO-301 `in_progress` and a decrease from 1,000 to 600, WO-301 keeps 1,000 and 400 of its output is flagged *for stock*.
- [ ] Replaying the same `SalesOrderLineCancelled` changes nothing.

#### Out of scope
The sales order transitions and their approvals (BL-SAL-05, BL-SAL-07–09); stock effects of *return to stock* (BL-STK-26); fee VAT (A-04).

---

### BL-MFG-04 — 07 Manufacturing: void a production report entered on the wrong work order

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: 07 Production reporting (`MFG.PRD.02`) · BL-KRN-03 (void) · **Blocks:** None
**Review items:** X-19 (void)

#### Goal
Operators on a shared tablet sometimes report output on the wrong work order. Before that work
order is completed, the report can be voided with a reason and its stock movements reversed.
After completion the work order is locked and corrections use reversing movements instead.

#### Files to read
- `docs/verticals/07-manufacturing.md` §7.1 *Work order* (paragraph under the transitions table), §7.1 "Negative-flow settings" row *Void*, §3.3 `MFG.ACL.10`
- `docs/modules/wl-kernel.md` §5.12 (void), `PLT.ACL.09`

#### Terms
- **Production report:** one entry of good quantity and scrap for a WO, by operator, at a time. It posts a `produce` movement of the good quantity into the WO's output lot and a `wastage` movement of the scrap.
- **Output lot:** the WO's one lot of its item, created in `quarantine` at the line's finished goods location by the first report with good quantity.
- **Void:** "this record should never have existed"; terminal `voided`, number kept, effects reversed.

#### Spec excerpt (authoritative for this issue)

§7.1: Each production report posts a `produce` movement of its good quantity into the work
order's output lot and a `wastage` movement of its scrap (`wl-inventory`). The output lot (one
per work order, source = the work order) is created in `quarantine` at the line's finished
goods location by the first report with good quantity; it is the finished lot of `complete`
and cannot be picked or issued before QC releases it (`MFG.ACL.02`).

§7.1: A completed work order stays locked (`MFG.ACL.10`); a production report entered on the wrong
work order before `complete` is voided with a reason (`PLT.ACL.09`) and its movements reversed.
The void is approved by the line's supervisor or the production manager, not the requester; it
is refused when the output lot no longer holds the report's good quantity (part of it was
moved, issued or picked; the production manager then corrects with a reversing movement).

Negative-flow settings, *Void*: Production reports before the work order is `completed`, goods
receipts not yet billed (`reverse`), draft dispatches (`cancel`). Never invoices, delivery notes
or certificates already sent to a customer: those are corrected by credit note or a new version.

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `PLT.ACL.09` | A record can be voided only with a reason, only from the states its module allows, and never after it was given to a third party or an authority as a legal or tax document; when it had posted money or moved stock, the void needs approval by another user | `STA`, `SOD` | approval when money or stock was posted: on |
| `MFG.ACL.10` | A completed work order is locked; corrections are made with reversing stock movements | `STA` | — |

#### Edge cases decided
- **Allowed while** the WO is `released`, `in_progress` or `on_hold`. Refused once the WO is `completed`, `closed` or `aborted`.
- **Approval:** a production report moves stock (output, scrap), so every void needs approval by another user (`PLT.ACL.09`): the supervisor or production manager of the line, not the requester.
- **Reversal:** the report's `produce` movement (out of the output lot) and its `wastage` movement are reversed in one transaction; the WO's reported totals are recalculated. When the lot's quantity becomes 0 it stays open in `quarantine` for later reports.
- **Output no longer in the lot:** when the output lot holds less than the report's good quantity (moved, issued or picked), the void is refused with "output already used"; the production manager corrects with a reversing movement.
- **WO completed while the void waits:** the kernel `fail`s the approval (guard re-check).
- **Entering it again:** the operator enters a new report on the right WO; the voided one stays visible struck through.

#### Must not
- Must not delete the report or reuse its number.
- Must not void reports of a completed WO.

#### Done when
- [ ] Given WO-400 `in_progress` with report PR-9 of 120 good units and 5 scrap, output lot F400 holding 300 units, when operator A voids PR-9 with reason "wrong WO" and supervisor B approves, then PR-9 is `voided`, its `produce` and `wastage` movements are reversed, F400 holds 180 units, WO-400 shows 120 fewer good units, and `RecordVoided` is emitted.
- [ ] Given F400 holding 100 units (200 transferred out), a void of PR-9 (120 units) is refused with "output already used".
- [ ] A cannot approve their own void.
- [ ] Given WO-400 `completed`, void is not offered on its reports.
- [ ] A void without a reason is refused.

#### Out of scope
Late entry of paper job cards (BL-MFG-14 setting, BL-KRN-04).

---

### BL-MFG-05 — 07 Manufacturing: hold a work order for material when an issue is pending

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: 07 Work order lifecycle (`hold`, `resume`) · BL-STK-18 (pending issues) · BL-STK-20 (`STK.RUL.06`) · **Blocks:** None
**Review items:** X-10

#### Goal
In this vertical a location never goes below zero: when a storekeeper issues material to a work
order that is not on hand, the short quantity waits as a pending issue. The line's supervisor
is then asked (by a task; the system never holds by itself) to put the work order on hold with
reason *material*, so the schedule board shows why the line is idle, and is asked to resume it
when the material arrives.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.2 "When it goes wrong" (last row), §7.1 *Work order* (rows `hold`, `resume`)
- `docs/modules/wl-inventory.md` §3 *Pending issue*, paragraph "Negative stock", §6 `STK.RUL.06`

#### Terms
- **Pending issue:** the short quantity of a consumption recorded while stock is short; `open` until it is `posted` (stock arrives) or `cleared` (the storekeeper clears it, or its source is cancelled).
- **Work order material issue:** posted by the WO as consumption through `ConsumptionSource` (wl-inventory), which is why a short quantity becomes a pending issue instead of being refused.
- **Negative stock policy `block`:** the location never goes below zero; a manual transfer or dispatch below zero is refused.

#### Spec excerpt (authoritative for this issue)

§4.2: *A material is short when it should be issued* → The location's negative stock policy is
`block`. A material issue to a work order is posted by the work order as consumption
(`ConsumptionSource`, `wl-inventory`), so its short quantity waits as a pending issue
(`STK.RUL.06`), without a lot until it posts; any other movement below zero (transfer,
dispatch) is refused. `hold` stays a user transition: for an `in_progress` work order the
line's supervisor gets one task to put it `on_hold` with reason *material* (no breakdown
ticket); a `released` one is not held and the pending issue shows on the schedule board. When
no pending issue of the work order is `open` any more (each one `posted` or `cleared`): a
work order `on_hold` with reason *material* gives the supervisor a task to `resume` it; a work
order still `in_progress` has its open hold task closed instead (no resume task).

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `hold` | in_progress | on_hold | `work_orders.report` | reason chosen | breakdown ticket created when the reason is *machine* |
| `resume` | on_hold | in_progress | `work_orders.report` | — | — |

#### Edge cases decided
- **Who holds:** `hold` is a user transition (`work_orders.report`). When a pending issue is created for an `in_progress` WO, the supervisor of its line gets a task "hold WO for material" with the reason prefilled; the system does not run `hold` itself.
- **WO still `released`:** no hold (not in From); the pending issue shows on the schedule board.
- **Material arrives:** when no pending issue of the WO is `open` any more (each `posted` or `cleared`) and the WO is `on_hold` with reason *material*, the supervisor gets a task "resume"; the system does not resume.
- **Never held:** when the last pending issue posts or is cleared while the WO is still `in_progress`, the open "hold WO for material" task is closed automatically; no resume task is created.
- **Held for another reason** (machine, quality): no resume task; the reason's own flow applies.
- **Pending issue cleared instead of posted** (storekeeper clears it with a reason, or the source is cancelled): counts as no longer open, as above.
- **Hold task done:** the task closes when the WO goes `on_hold` (any reason).
- **Several pending issues:** one task per WO, not per issue.

#### Must not
- Must not issue below zero at a `block` location.
- Must not create a breakdown ticket for reason *material*.

#### Done when
- [ ] Given WO-500 `in_progress` and 0 kg of pigment P at the store, when storekeeper A issues 20 kg P, then a pending issue exists and supervisor B has a task to hold WO-500 with reason *material*.
- [ ] When B runs `hold`, WO-500 is `on_hold` with reason *material* and no ticket is created.
- [ ] When 20 kg P is received and the pending issue posts, B has a task to resume WO-500.
- [ ] Given WO-501 `in_progress` with a pending issue of 10 kg P and B's hold task still open, when the storekeeper clears the pending issue with reason "used, not booked", the hold task closes and no resume task is created.
- [ ] Given 0 kg P at the store, a transfer of 5 kg P to another location is refused.

#### Out of scope
Pending issue posting and `STK.RUL.06` alerts (BL-STK-18, BL-STK-20).

---

### BL-MFG-06 — 07 Manufacturing: refuse a second sales order with the same customer PO number

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** BL-SAL-18 (customer PO uniqueness check in `submit` and `amend`) · Base: wl-portal reorder and PO upload (`POR.SLF.01`) · **Blocks:** None
**Review items:** X-14, 07 MEDIUM "duplicate PO upload"

#### Goal
B2B customers email a PO and sometimes also upload it in the portal. The same customer PO number
(with the same call-off number) must not create two orders for the same customer: `submit`
refuses the second one and points to the existing order. Call-offs against a blanket PO are told
apart by a call-off number. The check itself is built in wl-sales (BL-SAL-18, setting *Unique
customer PO number per customer*, off by default); this issue turns the setting on in the 07
deployment template, adds the call-off number to the 07 order entry and portal upload screens,
and shows the refusal in the portal upload.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.1 "When it goes wrong" (row "The same customer PO…"), §4.5 step 2, §7.1 *Sales order* (row `submit`)
- `docs/modules/wl-sales.md` §4.2 paragraph "Customer PO number", §7 setting "Unique customer PO number per customer"

#### Terms
- **Customer PO number:** the purchase order number printed on the customer's own PO.
- **Blanket PO / call-off:** one framework PO under which the customer calls off deliveries; each call-off has its own number.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | confirmed, on_credit_hold | `sales_orders.create` | lines priced; `SAL.ACL.02` for price exceptions; customer PO number not already used by an order of the same customer (call-offs carry a call-off number) | credit check decides the target; confirmation PDF sent; event `SalesOrderConfirmed` |

§4.1: `submit` refuses a second order with the same customer PO number and call-off number for
the same customer and points to the existing order (wl-sales setting *Unique customer PO number
per customer*, on in this vertical); numbers are compared trimmed and case-insensitive, other
characters as entered ("PO 8812" and "PO-8812" are different numbers), and a cancelled order
still holds its number. A call-off against a blanket PO is entered with its own call-off
number, so the same PO number with a new call-off number is accepted.
§4.5 step 2: An upload whose customer PO number and call-off number already have an order is
refused and the buyer is shown the existing order's number and status.

| Setting (wl-sales) | Default | 07 value |
|---|---|---|
| Unique customer PO number per customer (with call-off number) | off | on |

#### Edge cases decided
- **Key:** (customer, customer PO number, call-off number or empty), compared trimmed and case-insensitive (BL-SAL-18). Nothing else is normalised: "PO 8812", "PO-8812" and "PO8812" are three different numbers.
- **Same PO number, different customers:** allowed.
- **Earlier order cancelled:** still counts as used (the customer must send a new number or a call-off number); sales may see the cancelled order from the refusal.
- **Empty customer PO number:** not checked (orders without a PO are allowed unless the customer account requires one).
- **Concurrent submits:** enforced by a unique key so only one passes.
- **Portal message:** names the existing order number and its status only (no prices or internal notes).
- **Amend:** changing the customer PO number on `amend` applies the same check.

#### Must not
- Must not create a second order or reservation for a duplicate.
- Must not show the buyer orders of another customer.

#### Done when
- [ ] In a new 07 deployment the setting *Unique customer PO number per customer* is on.
- [ ] Given SO-70 for customer C with PO "PO-8812", when sales submits a new order for C with "po-8812 ", it is refused and SO-70 is named; an order for C with "PO 8812" is accepted.
- [ ] When buyer B of C uploads "PO-8812" in the portal, the upload is refused and B sees SO-70 and its status.
- [ ] Given PO "BL-1" with call-off "CO-1" on SO-71, an order with "BL-1" / "CO-2" is accepted and "BL-1" / "CO-1" is refused.
- [ ] Given customer D, an order with "PO-8812" is accepted.
- [ ] Given SO-72 for C with "PO-9000" `cancelled`, a new order for C with "PO-9000" is refused and SO-72 is named.

#### Out of scope
The uniqueness check in `submit` and `amend` (BL-SAL-18); duplicate external numbers per channel
for aggregators (BL-SAL-01).

---

### BL-MFG-07 — 07 Manufacturing: inspect returned or refused goods and ask sales to re-deliver or cancel (`MFG.RUL.10`)

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** BL-STK-22 (refused goods to quarantine) · BL-SAL-13 (accepted quantities only, returned delivery) · BL-SAL-08 (`cancel_line`) · Base: wl-ticketing checklists (`WRK.CHK.01`) · **Blocks:** None
**Review items:** 07-N4-a, X-11, 07 MEDIUM "returned delivery outcome"

#### Goal
When a customer refuses part of a delivery or the whole dispatch comes back, the goods are back
in quarantine. QC must inspect them before they are released or rejected, and sales must decide
whether to deliver the open quantity again or cancel it. Without this rule, refused goods sit in
quarantine and the order stays open forever.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.1 "When it goes wrong" (rows "accepts only part", "whole delivery is refused"), §7.1 *Dispatch* (rows `confirm_delivery`, `return`, paragraph under the table), §7.2 `MFG.RUL.10`

#### Terms
- **Refused quantity:** quantity per line the customer did not accept at proof of delivery, with a reason.
- **Returned dispatch:** a dispatch in `returned` (every line refused, or brought back).
- **QC inspection:** a checklist (`WRK.CHK.01`) on a lot in quarantine; the result releases the lot or raises an NCR.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MFG.RUL.10` | the goods of a dispatch → `returned`, or the refused quantity of `DispatchDelivered`, are booked into quarantine (run close) | — | create a QC inspection per returned lot in quarantine; create a task for sales to re-deliver or cancel the open quantity (`cancel_line`), closed when all of it is re-dispatched or cancelled (a part re-dispatched lowers the task's open quantity); notify the account's sales person (the sales role in scope when the account has none) | — | on | MVP |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `return` (dispatch) | dispatched | returned | `dispatch.manage` | reason recorded | goods back to quarantine; complaint ticket created; QC inspection and sales decision (`MFG.RUL.10`) |

§4.1: Refused goods come back to quarantine for QC (`MFG.RUL.10`); sales re-delivers them or
cancels the open quantity (`cancel_line`). QC inspects: release back to stock, NCR (rework or scrap).

#### Edge cases decided
- **One inspection per lot per event:** a refusal of two lines from two lots creates two inspections.
- **Idempotent:** keyed on (dispatch, event); a replay creates nothing new.
- **Timing:** the inspection is created when the goods are booked into quarantine (run close in wl-delivery), not when the driver records the refusal.
- **Task content:** the order, lines, refused quantities and reasons; actions "new dispatch" or `cancel_line`. Closed automatically when the whole refused quantity is re-dispatched or cancelled (re-dispatched = on a dispatch that reached `dispatched`). A partial re-dispatch or cancel (e.g. 10 of 20) lowers the task's open quantity to 10 and keeps it open.
- **Sales person missing on the account:** the task goes to the sales role in scope.
- **Complaint ticket:** created by the dispatch `return` (above); `MFG.RUL.10` links the inspection to it.
- **Partial refusal (`DispatchDelivered` with refused quantity):** no complaint ticket is created; the refusal reason stays on the proof-of-delivery line and on the sales task. QC's inspection result (release or NCR) is the quality record.

#### Must not
- Must not release refused goods to stock without the inspection (`MFG.ACL.02`).
- Must not cancel the open quantity automatically.

#### Done when
- [ ] Given DN-90 delivered with 80 accepted and 20 refused (reason "damaged") of lot L5, when the goods are booked to quarantine, then one inspection on L5 for 20 exists, the sales person has a task for SO line 1 (20 open) and is notified.
- [ ] Given DN-91 `returned`, an inspection per lot and one sales task exist, linked to the complaint ticket.
- [ ] When sales runs `cancel_line` for the 20, the task closes.
- [ ] Given that DN-90 refusal, no complaint ticket exists.
- [ ] Given a new dispatch DN-92 for 10 of the 20 that is `dispatched`, the task stays open with 10 open; after `cancel_line` for the other 10 it closes.
- [ ] Replaying the event creates no second inspection.

#### Out of scope
Per-line proof of delivery and quarantine booking (BL-DLV-02, BL-STK-22); order fulfilment (BL-SAL-13).

---

### BL-MFG-08 — 07 Manufacturing: disposition an NCR as return to supplier or scrap, and close it

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** Base: 07 NCR lifecycle · BL-STK-03 (dispatch a return to supplier) · BL-STK-05 (credit a return) · BL-INV-13 (supplier debit note) · BL-STK-12 (stock hold line decided) · **Blocks:** None
**Review items:** 07-N10-a, X-08

#### Goal
A raw material lot that fails incoming inspection gets an NCR. QC decides what happens: rework,
scrap, use as is, or return to the supplier. Return to supplier must create a real Return to
supplier in wl-inventory (the old "supplier return note" did not exist), scrap must post a real
write-off, and the NCR closes only when that action is done.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.3 "When it goes wrong" (row "A lot fails incoming inspection"), §4.4 steps 3–4, §7.1 *Lot* (rows `reject`, `discard`), §7.1 *NCR* (states, transitions)
- `docs/modules/wl-inventory.md` §4 *Return to supplier* (row `dispatch`), §4 *Lot* vertical mapping row 07

#### Terms
- **NCR (non-conformance report):** a record of a defect on a lot: quantity, defect, disposition, approval.
- **Disposition:** the decision on the NCR: rework, scrap, use as is, return to supplier.
- **Return to supplier (RTS):** wl-inventory record that sends goods back for *credit* or *replace*.
- **Supplier lot:** the supplier's lot number captured at goods receipt.
- **Partial NCR:** an NCR whose quantity is less than the lot's remaining quantity.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `disposition` | open | dispositioned | `ncrs.disposition` | disposition chosen (rework, scrap, use as is, return to supplier); `MFG.ACL.03` for use as is; return to supplier: supplier lot known, outcome *credit* or *replace* | rework → work order; scrap → lot `discard` or a `wastage` movement (`STK.ACL.02`); return → a draft Return to supplier in `wl-inventory` for the lot's supplier, from the quarantine location (`STK.ACL.06` on dispatch) |
| `close` | dispositioned | closed | `ncrs.disposition` | rework work order completed, scrap posted, or the Return to supplier `dispatched` | supplier credit chased on the Return to supplier (`STK.RUL.08`), not on the NCR |
| `reopen` | dispositioned | open | system (the linked Return to supplier is `cancelled`, or the rework work order is `cancelled` or `aborted`), or `ncrs.disposition` | the disposition's action did not happen; reason given | previous disposition kept in history; QC gets a task to choose a new disposition (`MFG.ACL.03` again for *use as is*) |
| `cancel` | open | cancelled | `ncrs.disposition` | reason given | — |
| `reject` (lot) | quarantine, on_hold | rejected | `lots.release`, or system (Return to supplier `dispatch` takes the last quantity) | failed inspection recorded | NCR created; remaining quantity to scrap or to a Return to supplier per the NCR disposition |
| `discard` (lot) | on_hold, expired | rejected | `lots.release`, or system (stock hold line decided `discard`) | reason given; `STK.ACL.02` above AED 2,000 | remaining quantity written off to scrap with the reason |

§4.3: The NCR disposition *return to supplier* creates a Return to supplier from the quarantine
location (`STK.ACL.06` above AED 5,000). On dispatch, a supplier debit note is drafted for
*credit* lines (`FIN.RUL.17`); *replace* lines reopen the PO quantity. `STK.RUL.08` chases the
supplier after 30 days. If the Return to supplier is cancelled (the supplier refuses collection),
the NCR is reopened for a new disposition.

§7.1 *NCR*: When the rework work order completes, the scrap posts or the Return to supplier is
`dispatched`, QC gets a task to `close` the NCR; it is not closed by the system.

§7.1 *NCR*: An NCR may cover only part of a lot. Then the lot is not `reject`ed or `discard`ed
(those move the whole lot): it keeps its state and is not `release`d while the NCR is `open` or
`dispositioned`. The NCR quantity leaves the lot through the disposition's own movement: a
`wastage` movement for *scrap* (`STK.ACL.02`), or `return_out` when the Return to supplier is
`dispatched`. QC may then release what remains (`MFG.ACL.02`); *use as is* releases the whole
lot. When the NCR covers the whole remaining quantity, lot `reject` or `discard` applies as in
the table.

#### Edge cases decided
- **Scrap path (NCR covers the whole remaining quantity):** a lot in `on_hold` or `expired` → lot `discard`; a lot already `rejected` (after `reject`) → a `wastage` movement for its remaining quantity. **Partial NCR:** a `wastage` movement of the NCR quantity from the lot, which keeps its state. All above AED 2,000 go through `STK.ACL.02`; the NCR records the movement or approval.
- **RTS lines:** one line per NCR lot: item, lot, NCR quantity, unit cost from the source goods receipt, reason = NCR defect, outcome = the NCR's *credit* or *replace*, PO line from the receipt. Source = the NCR.
- **Supplier lot unknown** (no goods receipt link, e.g. old opening stock): *return to supplier* is refused; QC chooses another disposition.
- **Lot produced in-house (source = work order):** *return to supplier* is not offered.
- **NCR quantity less than the lot (partial NCR):** QC does not run lot `reject` or `discard`; the lot keeps its state (`quarantine` or `on_hold`) and cannot be `release`d while the NCR is `open` or `dispositioned`. The RTS takes only the NCR quantity (`return_out` at its `dispatch`); a scrap disposition posts a `wastage` of the NCR quantity. Once that movement is posted, QC may release the rest (`MFG.ACL.02`). *Use as is* releases the whole lot after its approval (`MFG.ACL.03`).
- **RTS cancelled** (supplier refuses collection): the system runs `reopen`; the NCR is `open` again with the previous disposition in history, and QC gets a task to choose a new one. The same applies when the rework WO is `cancelled` or `aborted`. Re-disposition happens on the same NCR (no second NCR for the lot).
- **Close guard** is checked at `close`; the system offers `close` (task to QC) when the linked WO completes, the scrap posts or the RTS is `dispatched`; it does not close by itself.
- **Use as is:** unchanged (`MFG.ACL.03`).

#### Must not
- Must not chase the supplier's credit on the NCR (it is on the RTS).
- Must not create the RTS outside the quarantine location.
- Must not close an NCR whose RTS is still `draft`.

#### Done when
- [ ] Given lot L7 (supplier S, receipt GR-12, 100 kg at AED 30) `rejected` with NCR-5 for 100 kg, when QC dispositions *return to supplier* with outcome *credit*, then a draft RTS for S from quarantine with 100 kg L7, AED 30/kg, source NCR-5 exists and NCR-5 is `dispositioned`.
- [ ] Given the RTS value AED 3,000 is dispatched by the storekeeper, `close` is allowed on NCR-5.
- [ ] Given NCR-5 with its RTS `draft`, `close` is refused.
- [ ] Given lot L8 `on_hold` with NCR-6 disposition *scrap* of value AED 2,500, an approval under `STK.ACL.02` is requested and the lot is `rejected` only after approval.
- [ ] Given a lot made by a work order, *return to supplier* is not offered.
- [ ] Given NCR-7 `dispositioned` *return to supplier* with its RTS `draft`, when the RTS is `cancelled`, NCR-7 is `open`, its history shows the earlier disposition and QC has a task; QC can then disposition *scrap*.
- [ ] Given lot L9 in `quarantine` with 100 kg and NCR-8 for 30 kg dispositioned *return to supplier*, `release` of L9 is refused while the RTS is `draft`; after the RTS is `dispatched`, L9 holds 70 kg, is still `quarantine`, and QC can `release` it.

#### Out of scope
RTS dispatch, credit, debit note and chase (BL-STK-03, BL-STK-05, BL-STK-07, BL-INV-13); rejection at goods receipt (BL-STK-01).

---

### BL-MFG-09 — 07 Manufacturing: record the lot on every issue of a lot-tracked material to a work order

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: wl-inventory Lot tracking and stock movements (`STK.LOT.01`, `STK.MOV.01`) · **Blocks:** BL-MFG-10
**Review items:** 07-N4-c, X-09

#### Goal
A recall can only name the affected customers if the lot genealogy is complete. Every issue of a
lot-tracked raw material to a work order must therefore record its lot; an issue without a lot
is refused.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.2 step 5, §9 "Traceability and recall", §13 question 2

#### Terms
- **Lot-tracked item:** an item whose flag "lot-tracked" is on (default for food-contact materials, chemicals and certified materials).
- **Lot genealogy:** the links raw lot → work order → finished lot → dispatch.
- **Pending issue:** the short quantity of a work order material issue at a `block` location (BL-MFG-05); it has no lot until it posts, when wl-inventory records the lot that covered it as *assigned on arrival*.

#### Spec excerpt (authoritative for this issue)

§9: lot genealogy is kept so that a recall can name the affected customers. Every issue of a
lot-tracked material to a work order records its lot; only the short quantity of an issue that
waits as a pending issue (§4.2) gets its lot when it posts (*assigned on arrival*, a recall
treats it as *lot uncertain*).
§4.2 step 5: Materials are issued to the work order, with lot numbers captured (`MFG.PRD.02`,
`STK.MOV.01`, `STK.LOT.01`).

#### Edge cases decided
- **Several lots for one line:** one movement per lot.
- **Pending issue (stock short):** the only case without a lot at entry. The storekeeper names lots for the quantity available; only the quantity no lot can cover becomes a pending issue, and wl-inventory assigns the lot when it posts ("assigned on arrival"). Until then, and afterwards, the trace marks it *lot uncertain*.
- **Item flag switched on later:** applies to new issues only.
- **Late entry from paper job cards:** the lot is still required.
- **Finished lot link:** the output lot of the WO records the WO; the WO records its issued lots, so no extra field is needed.

#### Must not
- Must not accept an issue of a lot-tracked item without a lot, through the UI, a scanner or an import, when stock of the item is available at the location; the only exception is the short quantity that becomes a pending issue.

#### Done when
- [ ] Given resin R (lot-tracked) and WO-600, when storekeeper A issues 50 kg R without a lot, it is refused.
- [ ] When A issues 30 kg of lot R1 and 20 kg of lot R2, two movements with lots exist on WO-600.
- [ ] Given cartons K (not lot-tracked), an issue without a lot is accepted.
- [ ] Given 10 kg of resin R (lot R3) at the store, an issue of 25 kg R to WO-600 with lot R3 for 10 kg posts 10 kg from R3 and a pending issue of 15 kg with no lot; an issue of 10 kg R without a lot is refused.

#### Out of scope
The recall trace (BL-STK-15, BL-MFG-10).

---

### BL-MFG-10 — 07 Manufacturing: resolve recall trace lines to customers (`TracePartyResolver` for 07)

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** BL-STK-14 (open a recall) · BL-STK-15 (forward trace) · BL-MFG-09 · **Blocks:** None
**Review items:** 07-N4-c, X-09

#### Goal
The recall in wl-inventory follows the movements; this vertical says who the party is for each
subject it finds. A dispatch belongs to its sales order's customer. A work order for an order not
yet delivered belongs to that order's customer, marked *not shipped*; its dispatches that have
not left (`draft`, `picked`) are flagged for re-pick so the goods do not leave, and one that
has already left (`dispatched`) is listed for the recall owner to bring back.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.4 step 8, §9 "Traceability and recall"
- `docs/modules/wl-inventory.md` §4 *Recall* (forward trace table, paragraph about the party), §6 `TracePartyResolver`, §4 vertical mapping row *Recall*
- `docs/OPEN-ITEMS.md` AU-20

#### Terms
- **`TracePartyResolver`:** the vertical's provider: given a trace subject, return its party and a status.
- **Affected line:** subject, party, lot, quantity, date, notice status.
- **Not shipped:** the goods of the subject have not left the plant.

#### Spec excerpt (authoritative for this issue)

§4.4 step 8: A recall is opened from a supplier or authority notice, or an internal finding, by the
general manager or the production manager (`recalls.manage`). Every lot in scope is held at every
location. The forward trace follows the lot genealogy through work orders to the finished lots,
and through dispatches to the customers (`TracePartyResolver`: dispatch → its sales order's
customer; a work order for an undelivered order → that order's customer, marked *not shipped*,
and its dispatch is stopped: its `draft` or `picked` dispatches holding the work order's output
lots are flagged for re-pick with a task to the storekeeper, and a `dispatched` one is listed for
the recall owner to bring back). Notices go to every active customer buyer in EN/AR, or to the
customer's main contact email when it has none. Each held lot gets an
outcome: discard, return to supplier, or release by a second person (`STK.ACL.05`). The recall
report is kept with the lot documents. Whether the business must notify MoIAT or another
authority is `⚠ open: authority (AU-20)`.

wl-inventory mapping: 07: sales order / dispatch → customer.

#### Edge cases decided
- **Dispatch `delivered` or `dispatched`:** party = the sales order's customer; status *shipped*.
- **Dispatch `draft` or `picked` with a lot in scope:** the lot is held, so `dispatch` is already refused (re-pick, BL-STK-21); party = customer, status *not shipped*.
- **WO linked to an undelivered sales order:** party = that order's customer, *not shipped*; "dispatch is stopped" = every `draft` or `picked` dispatch of that order holding output lots of the WO is flagged for re-pick and the storekeeper gets a task; a `dispatched` one is listed for the recall owner to recall by hand (wl-delivery return).
- **WO with source = stock:** no party; the trace continues through its output lots.
- **Dispatch `returned` or `cancelled`:** not listed (goods are back on site and held).
- **Customer buyers:** notices go to every active portal buyer of the customer; when the customer has none, to the customer's main contact email.
- **Authority notice:** not built (AU-20).

#### Must not
- Must not resolve a party from the item or lot alone; always through the dispatch or WO.
- Must not send notices from the resolver (wl-inventory sends them).

#### Done when
- [ ] Given raw lot R1 issued to WO-700 producing lot F1, F1 dispatched on DN-1 (delivered) to customer C1 and reserved on DN-2 (`picked`) for C2, when a recall on R1 is traced, then lines exist for C1 *shipped* and C2 *not shipped*, and DN-2 is flagged for re-pick.
- [ ] Given WO-701 `in_progress` from R1 for SO-80 of customer C3, a line exists for C3 *not shipped*.
- [ ] Given WO-702 from R1 with source stock, no party line exists for it and its output lots are in scope.
- [ ] Given customer C4 in scope with no portal buyer, the notice goes to C4's main contact email.

#### Out of scope
Opening, holding, notices and close (BL-STK-14–BL-STK-16); authority notification (AU-20).

---

### BL-MFG-11 — 07 Manufacturing: block picking of regulated items without a valid conformity certificate (`MFG.ACL.11`)

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** BL-DOC-01 (`document_valid` guard) · BL-DOC-04 (look-ahead list) · BL-DOC-05 (`DocumentExpired`, lift on renewal) · Base: wl-inventory Dispatch lifecycle · **Blocks:** None
**Review items:** X-13

#### Goal
Regulated products (low-voltage electrical goods, lighting, toys, paints, food-contact items) may
only leave the plant while a valid ECAS, EQM or food-contact certificate is on file. Picking such
an item is refused without one, with no override. Items under the single-use plastics ban are
*export only* and cannot be sold for domestic delivery. Orders planned to ship after a
certificate expires are listed 30 days ahead so it can be renewed in time.

#### Files to read
- `docs/verticals/07-manufacturing.md` §2 (product conformity, single-use plastics), §3.3 `MFG.ACL.11`, `DOC.ACL.02`, §7.1 *Dispatch* (row `pick`), §7.1 "Negative-flow settings" row *Document blocks*, §9 "Product conformity"
- `docs/modules/wl-documents.md` §5.6
- `docs/OPEN-ITEMS.md` L-19, C-02

#### Terms
- **Regulated flag:** item flag; the item needs a conformity certificate to be sold.
- **Export-only flag:** item flag; the item may not be delivered in the UAE.
- **Conformity certificate kinds:** ECAS certificate (1 year), Emirates Quality Mark licence (3 years), food-contact certificate; document subject = the item.
- **Accepted kinds (item field):** the conformity kinds that count for a regulated item, set per item at onboarding (C-02); default all three.
- **Renewal in review:** a new document of the kind uploaded for the item and still in `pending_review` (wl-documents); it does not pass the guard.
- **`DOC.RUL.02`:** wl-documents rule, nightly at 01:00 Asia/Dubai: for documents of a blocking kind that expire within the look-ahead (30 days from the run date), it asks this vertical's `FutureWork` provider for planned items dated after the expiry date.
- **`document_valid(subject, kind, on_date)`:** wl-documents guard; yes when an accepted, unexpired document exists on that date.
- **Domestic delivery:** the sales order's delivery address VAT category is not *export*.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `MFG.ACL.11` | A regulated item is dispatched only while a valid conformity certificate is on file; an *export only* item cannot be sold for domestic delivery | `STA` | sales, storekeeper | — |
| `DOC.ACL.02` *(shared)* | A transition blocked by an expired document runs only with a one-use override approved by a second user; never for legal blocks | `SOD`, `TIM`, `STA` | general manager | conformity certificates: never |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `pick` (dispatch) | draft | picked | `dispatch.manage` | only `released` lots, no held quantity; `MFG.ACL.11` (a regulated item needs one of its conformity certificate kinds accepted and valid on the pick date; export-only flag) | stock reserved |
| `dispatch` (dispatch) | picked | dispatched | `dispatch.manage` | vehicle and driver set; no picked lot on hold (re-pick otherwise); `MFG.ACL.11` again on the dispatch date (an expired certificate refuses it; the dispatch may be `cancel`led); `credit_recheck` for orders on account (fails → sales order `hold`) | stock moved out; delivery note PDF |

Settings, *Document blocks*: Conformity certificate (ECAS, EQM, food-contact) of a regulated item
blocks `dispatch.pick` and `dispatch.dispatch` (block; any one valid kind among the item's
accepted kinds passes; a renewal still in `pending_review` does not); override `never`
(proposed; legal block `⚠ open: lawyer (L-19)`).
Industrial licence, Industrial Registry and ICV certificate: reminders only (`DOC.RUL.01`), no
block. Expiry look-ahead 30 days.

§9: Certificates are documents with expiry (`DOC.EXP.01`) whose kind blocks `dispatch.pick` and
`dispatch.dispatch` without override; orders planned to ship after the expiry date are listed 30 days ahead
(`DOC.RUL.02`).

#### Edge cases decided
- **Which certificate:** a regulated item passes when any one kind among its *accepted kinds* has an `accepted` document valid on the pick date (Asia/Dubai). The accepted kinds are set per item at onboarding from MoIAT's product conformity service (C-02); default all three, so an item with only an EQM licence on file passes.
- **`BlockSubjects`:** a dispatch → the distinct regulated items on its lines.
- **Valid on its expiry date:** the document is valid through its expiry date; picking on the day after is refused.
- **Pick then expiry before `dispatch`:** `dispatch` also calls the guard (`MFG.ACL.11`, "dispatched only while valid"); on failure it is refused and the storekeeper may `cancel` the dispatch.
- **Export-only:** checked at sales order `submit` and `amend` (a domestic line is refused) and again at `pick`.
- **`FutureWork` for `DOC.RUL.02`:** on each nightly run (01:00 Asia/Dubai), for a certificate whose expiry date is within 30 days of the run date, the provider returns the confirmed sales order lines of the item, not yet picked, whose promised date is after the expiry date (any distance ahead). Sales and the planner get one message with the list; later runs message only new lines. When the item has another accepted kind valid beyond the promised date, the line is not listed.
- **Override:** none; the override button is not shown for these kinds.
- **Renewal in review:** when a renewal of the kind is in `pending_review`, the guard still refuses; the refusal message adds "renewal pending review since <date>" and the document goes to the top of the review queue (wl-documents).

#### Must not
- Must not offer or accept an override for conformity kinds.
- Must not block items that are not regulated.
- Must not block on industrial licence, Industrial Registry or ICV certificate expiry.

#### Done when
- [ ] Given LED fitting X (regulated) with an ECAS certificate expiring 2026-11-30, when storekeeper A picks X on 2026-11-30, the pick succeeds; on 2026-12-01 it is refused with the certificate and expiry shown.
- [ ] Given a renewal uploaded on 2026-12-01 and still `pending_review`, picking X is refused with "renewal pending review since 2026-12-01".
- [ ] Given a renewed certificate accepted on 2026-12-02, picking X succeeds again.
- [ ] Given item Z (regulated, accepted kinds: food-contact only) with a valid ECAS certificate and no food-contact certificate, picking Z is refused.
- [ ] Given X picked on 2026-11-30 and not yet dispatched, `dispatch` on 2026-12-01 is refused and `cancel` of the dispatch is offered.
- [ ] Given cup lids Y (export only), a domestic sales order line for Y is refused at `submit`; an export order line is accepted.
- [ ] Given SO line for X promised 2026-12-10 and the certificate expiring 2026-11-30, on the 2026-11-01 01:00 run (29 days before expiry) sales and the planner get the look-ahead list with that line; the 2026-11-02 run sends nothing new.
- [ ] No override request can be created for an ECAS block.
- [ ] Given an expired industrial licence, picking is not blocked.

#### Out of scope
The guard engine and override (BL-DOC-01, BL-DOC-06); which products are regulated (C-02, onboarding).

---

### BL-MFG-12 — 07 Manufacturing: alert on missing export evidence and open the VAT correction (`MFG.RUL.08`)

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** BL-KRN-08 (owner alert for compliance-critical rules) · Base: wl-invoicing Invoice and Credit note lifecycle · Base: wl-documents attachments · **Blocks:** None
**Review items:** 07 MEDIUM "export 90-day alert"

#### Goal
An export is zero-rated only if the goods leave the UAE within 90 days with evidence. The rule
tracks evidence per dispatch: at 60 days the accountant is warned; at 90 days without evidence
the dispatch is flagged "5% VAT due" and the accountant gets a task to correct the invoice. The
rule is compliance-critical: if it fails, the owner is alerted.

#### Files to read
- `docs/verticals/07-manufacturing.md` §2 (VAT), §4.1 "When it goes wrong" (last row), §7.2 `MFG.RUL.08`, §9 VAT
- `docs/OPEN-ITEMS.md` A-12

#### Terms
- **Zero-rated export dispatch:** a dispatch whose invoice lines carry the VAT category *export, zero-rated*.
- **Exit evidence:** customs exit certificate, bill of lading or airway bill, attached to the dispatch.
- **Compliance-critical:** a rule whose failure alerts the business owner at once (`PLT.RUL.04`).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MFG.RUL.08` | N days after a zero-rated export dispatch (checked daily at 08:00 Asia/Dubai) | exit evidence (customs exit certificate, bill of lading or airway bill) not attached | notify the accountant; at M days flag that 5% VAT is due and create a task to correct the invoice (credit note and a standard-rated invoice); whether the VAT is recharged to the customer depends on the contract. Evidence attached after day M keeps the flag and notifies the accountant "evidence received after correction"; nothing is reversed (A-12). Compliance-critical | N = 60, M = 90 | on | MVP |

§9: Export evidence is tracked per dispatch; `MFG.RUL.08` alerts at 60 days and flags at 90 days
that the zero rate no longer holds. Whether a zero rate can be restored when evidence arrives
after the correction is `⚠ open: accountant (A-12)`.

#### Edge cases decided
- **Day count:** calendar days from the dispatch's `dispatched` date, Asia/Dubai; the rule runs daily at 08:00 Asia/Dubai. Day 60 and day 90 each fire once (idempotent per dispatch and step).
- **Failed or missed run (BL-KRN-08):** a run that ends `failed` (the job raises an error) or is `stop`ped, or a daily occurrence with no run 60 minutes after 08:00 (found by the 09:15 scheduler check at the latest), alerts the general manager / owner role and the company admin and opens a task for the owner role.
- **Evidence:** any one of the three document kinds attached to the dispatch stops the rule for that dispatch.
- **Missed run:** a late run still fires the steps whose day has passed (60, then 90 in the same run if both passed).
- **Task:** names the dispatch, the zero-rated invoice(s) and their amounts; the correction (credit note + new 5% invoice) is made by the accountant with the normal invoicing tools; the rule does not issue documents.
- **Recharge:** the task shows the customer's contract note; the decision is the accountant's.
- **Evidence after day 90:** (A-12 open) the flag stays, the accountant is notified "evidence received after correction"; nothing is reversed automatically.
- **Invoice issued after dispatch:** the clock still starts at the dispatch date.

#### Must not
- Must not change or credit the invoice automatically.
- Must not stop alerting because the invoice is paid.

#### Done when
- [ ] Given export dispatch DN-5 dispatched 2026-08-01 with invoice INV-50 zero-rated AED 40,000 and no evidence, on 2026-09-30 (day 60) the accountant is notified.
- [ ] On 2026-10-30 (day 90) DN-5 is flagged "5% VAT due" and the accountant has a task naming INV-50.
- [ ] Given a bill of lading attached on 2026-09-15, nothing fires at day 60 or 90.
- [ ] Given the 2026-10-30 08:00 run of `MFG.RUL.08` ends `failed` (test: the job raises an error), the general manager / owner role and the company admin get the alert at once and the owner role has a task (BL-KRN-08); a rerun that succeeds closes the task.
- [ ] Given the scheduler is stopped so no run exists for 2026-10-31 08:00, the 09:00 or 09:15 check raises the same alert once.
- [ ] Running the rule twice on 2026-10-30 creates one task.

#### Out of scope
Restoring the zero rate (A-12); the correction documents themselves (base invoicing).

---

### BL-MFG-13 — 07 Manufacturing: link a quality dispute on an invoice to an NCR

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** BL-INV-04 (invoice dispute) · BL-INV-05 (`FIN.RUL.15`) · Base: 07 NCR lifecycle · **Blocks:** None
**Review items:** X-02

#### Goal
Customers dispute invoices for quantity, price or quality. In this vertical the customer buyer
may flag the dispute in the portal; a quality dispute must link an NCR on the delivered lot, so
QC investigates the product and not only the accountant the money.

#### Files to read
- `docs/verticals/07-manufacturing.md` §4.1 "When it goes wrong" (row "An invoice is disputed"), §4.5 step 5, §7.1 "Negative-flow settings" row *Disputes*

#### Terms
- **Disputed flag:** an invoice flag (not a state) that pauses reminders and links a ticket.
- **Quality dispute:** a dispute whose reason is *quality*.
- **Disputed lines:** the invoice lines the buyer (or accountant) ticks on the 07 quality dispute form; all lines when none is ticked. Stored with the NCR link by this vertical (wl-invoicing keeps only the disputed amount).
- **Invoice line → lots:** each 07 invoice line is drafted from a dispatch line (`MFG.RUL.04`), which records its lots and quantities.

#### Spec excerpt (authoritative for this issue)

§4.1: Invoice `dispute` by the accountant, or by the customer buyer in the portal (on in this
vertical); reminders pause and a ticket is linked; resolved as upheld or by credit note. A quality
dispute opens an `open` NCR on each invoiced lot of the disputed lines with the buyer's text (or
links that lot's open NCR; an item not lot-tracked gets one on the item and dispatch) and
notifies QC; the NCR follows its own lifecycle. The disputed lines are the invoice lines ticked
on the quality dispute form (all lines when none is ticked); an invoice line leads to its lots
through the dispatch line it was drafted from (`MFG.RUL.04`).
§4.5 step 5: They may flag an invoice as disputed with a reason; for reason *quality* they may
tick the affected invoice lines (all lines when none is ticked). Sales and the accountant see
it on the linked ticket.
Settings, *Disputes*: Invoices, by the accountant or the customer buyer in the portal (on); a
quality dispute links an NCR; `FIN.RUL.15` escalates after 14 days.

#### Edge cases decided
- **Reasons offered in the portal:** quantity, price, quality, other + text.
- **Which NCR:** on a quality dispute, for each disputed line the lots of its dispatch line are taken; an `open` NCR is created on each such lot with the buyer's text, or an existing open NCR of that lot is linked; QC is notified. Several lots → one NCR per lot; a lot on two disputed lines → one NCR.
- **No lot (item not lot-tracked):** the NCR is raised on the item and the dispatch.
- **Resolution:** the dispute is resolved in wl-invoicing; the NCR follows its own lifecycle and is not closed by the dispute.
- **Buyer sees:** the ticket status only, never the NCR.

#### Must not
- Must not show NCR content, costs or other customers' data in the portal.

#### Done when
- [ ] Given INV-60 for lot F3, when buyer B disputes it in the portal with reason *quality* and "colour off", then INV-60 is disputed, a ticket is linked, an NCR on F3 with the text exists and QC is notified.
- [ ] Given INV-61 with line 1 (dispatch line lots F4 and F5) and line 2 (lot F6), when B disputes with reason *quality* and ticks line 1 only, NCRs exist on F4 and F5 and none on F6; with no line ticked, NCRs exist on F4, F5 and F6.
- [ ] Given reason *price*, no NCR is created and no line selection is shown.
- [ ] B sees the ticket status and not the NCR.

#### Out of scope
Dispute flag, reminders pause and escalation (BL-INV-04, BL-INV-05).

---

### BL-MFG-14 — 07 Manufacturing: configure shared negative-flow settings

**Labels:** `vertical:07-manufacturing` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** BL-KRN-03 · BL-KRN-04 · BL-KRN-05 · BL-KRN-08 · BL-DOC-01 · BL-PTY-03 · BL-INV-01 · BL-INV-02 · BL-INV-04 · BL-PAY-09 · BL-PAY-10 · BL-PAY-14 · BL-SAL-07 · BL-SAL-12 · BL-STK-03 · BL-STK-11 · BL-STK-18 · BL-DLV-02 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-STK-28 · BL-STK-29 · BL-STK-30 · BL-STK-31 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None
**Review items:** X-01, X-02, X-05, X-08, X-09, X-10, X-11, X-13, X-14, X-18, X-19

#### Goal
The shared modules build each negative-flow mechanism once with settings. This issue seeds the
07 deployment template with the vertical's values, so a new manufacturing client gets the right
limits, blocks, roles and fallbacks on day one.
It also seeds the 07 values for the cash-flow forecast alert (`FIN.CFL.01`), the budget with
owner drawings (`FIN.BUD.01`), lead sources and campaigns (`SAL.CRM.02`), the supplier scorecard
(`STK.SUP.01`), onboarding and SOPs (`HRM.ONB.01`) and registration kinds (`DOC.EXP.01`), added
on 2026-10-16: `FIN.RUL.23`, `FIN.RUL.24`, `FIN.ACL.21`, `SAL.ACL.07`, `STK.RUL.09`,
`STK.ACL.07`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`.
Onboarding runs on the minimal employee record (`HRM.EMP.01`, MVP since 2026-10-16).

#### Files to read
- `docs/verticals/07-manufacturing.md` §3.3, §4 "When it goes wrong" tables, §7.1 "Negative-flow settings", §7.2 (reused rules)
- `docs/WORKFLOW.md` §8
- `docs/modules/wl-invoicing.md` §5.14–§5.16 and §7; `docs/modules/wl-payments.md` (open
  cheques, owner-drawings payments); `docs/modules/wl-sales.md` §4.6, §5.8 and §7;
  `docs/modules/wl-inventory.md` §5.9 and §7; `docs/modules/wl-hr.md` §5.8 and §7;
  `docs/modules/wl-documents.md` §3, §5.7 and §7
- `docs/OPEN-ITEMS.md` A-04, L-19

#### Terms
- **Deployment template:** the seed configuration applied when a client of a vertical is created.
- **Owner role:** the role alerted by `PLT.RUL.04` when a compliance-critical rule fails.

#### Spec excerpt (authoritative for this issue)

| Setting | Value (07) | Spec reference |
|---|---|---|
| **Void** — allowed | production reports before the WO is `completed`; goods receipts not yet billed (`reverse`); draft dispatches (`cancel`) | §7.1 settings *Void* |
| **Void** — never | invoices, delivery notes, certificates already sent to a customer (credit note or new version instead) | §7.1 settings *Void* |
| Sales cancellation outcomes offered | *finish to stock* (default for standard items), *scrap* (default for items flagged *customer-specific*, e.g. printed labels, packaging; flag off by default), *return to stock* (picked finished goods); sales may change the outcome per line | §7.1 *Sales order* paragraph, §6 *Item* |
| Cancellation fee (`SAL.ACL.05`) | 0% default; percentage settable per customer; VAT on a fee kept from a prepayment `⚠ open: accountant (A-04)`: fee line VAT category seeded *standard rated (5%)* until answered | §3.3, §7.1, wl-sales §10 |
| Unique customer PO number per customer (wl-sales) | on (set by BL-MFG-06) | §4.1 |
| `SAL.ACL.05` applies to / approver | sales, production manager | §3.3 |
| `MFG.ACL.12` | on | §3.3 |
| `SAL.ACL.02` discount limit | 5% | §3.3 |
| `SAL.ACL.03` overdue threshold | overdue > 30 days | §3.3 |
| Credit re-check at dispatch | on for orders on account | §7.1 *Dispatch* `dispatch` |
| `SAL.RUL.05` bounced cheque hold | on: every open order on account of the customer | §4.1, §7.2 |
| Partial acceptance at proof of delivery | on | §7.1 *Dispatch* paragraph |
| Document kinds ECAS, EQM, food-contact (subject item) | `blocks`: `dispatch.pick` and `dispatch.dispatch` (block; any one valid kind among the item's accepted kinds passes); override `never` (proposed, `⚠ open: lawyer (L-19)`); compliance-critical; look-ahead 30 days | §7.1 settings *Document blocks*, §3.3 `DOC.ACL.02` |
| Industrial licence, Industrial Registry, ICV certificate | reminders only (`DOC.RUL.01`), no `blocks`; ICV reminder 60 days before expiry | §7.1 settings, §13 q7 |
| Party blocked — sales orders | module default *pause* (not yet dispatched held); decider sales manager | §7.1 settings *Party blocked* |
| Party blocked — work orders | continue; the planner may `unlink` to stock | §7.1 settings *Party blocked* |
| Party blocked — money in | continue | §7.1 settings *Party blocked* |
| Integration: line tablets | manual fallback: paper job cards entered by the supervisor at end of shift with late-entry flag | §7.1 settings *Integrations*, §8 |
| Integration: e-invoicing ASP (from 2027) | invoices queued, sent on recovery (`PLT.RUL.05`) | §7.1 settings *Integrations* |
| Integration: email | fallback: documents downloaded from the record and sent by hand | §7.1 settings *Integrations* |
| Late entry (`PLT.ACL.10`) | production reports, inspections, goods receipts keep business time; > 24 h late needs the production manager | §7.1 settings *Late entry*, §4.2 |
| Invoice disputes | by accountant and customer buyer in the portal: on; `FIN.RUL.15` escalation 14 days | §7.1 settings *Disputes* |
| Compliance-critical rules | `MFG.RUL.08`; `DOC.RUL.03` for conformity certificate kinds | §7.1 settings |
| Owner role (`PLT.RUL.04`) | general manager / owner | §7.1 settings |
| Write-off limit (`FIN.ACL.15`) | AED 0 (always approval); applies to accountant, general manager | §3.3, §7.1 settings |
| Bad-debt relief proposal (`FIN.RUL.14`) | after 6 months | §4.1 |
| Bounced cheque | bounce fee off unless the client sets a charge type and amount; reminders at 3 and 14 days (`FIN.RUL.18`); maximum two presentations | §4.1, §7.1 settings |
| `STK.ACL.02` | AED 2,000 | §3.3 |
| `STK.ACL.06` | AED 5,000; procurement, storekeeper | §3.3 |
| `STK.ACL.05` (recall release by a second person) | on | §3.3 |
| Negative stock policy (all 07 locations) | `block` (pending issues, `STK.RUL.06`) | §4.2 |
| `STK.RUL.07` open stock hold reminder | 24 hours | §4.4 step 7 |
| `STK.RUL.08` return chase | 30 days | §4.3 |
| `STK.RUL.02` low stock | on | §7.2 |
| Recall openers | `recalls.manage`: general manager, production manager | §4.4 step 8 |
| Responsible role per document kind (wl-documents §5.2) | conformity certificates (ECAS, EQM, food-contact) → QC inspectors of each plant that holds stock of the item or has an open work order for it (every plant when none does); calibration certificates → QC inspector of the plant; industrial licence, Industrial Registry, ICV certificate → general manager / owner; export exit evidence → accountant | §7.1 settings |
| Notification types (wl-notify §5.4 group) | `mfg.recall_notice`: no group, urgency `urgent` (task to the recall owner with an in-app alert); no statutory types; staff-only alerts (`MFG.RUL.01`–`10`, WO `unlink` and `abort` notices): no group, give-up `none` | §7.1 *Notification types registered by 07* |
| Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 200,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the general manager / owner and the accountant, checked daily at 06:00; foreign-currency supplier bills at the latest rate; due dates adjusted by each customer's average days late: on; disputed amounts counted: off | §5; §7.1 settings |
| Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories owned by the accountant and mapped to the export accounts; one budget per legal entity and fiscal year, by month, approved by the general manager / owner; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.1 settings |
| Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of operating cost, product cost and profit; every drawing approved, whatever its amount (`FIN.ACL.21` limit AED 0), by the general manager / owner, never by the requester (`FIN.ACL.21`) | §5; §7.1 settings |
| Lead sources used (`SAL.CRM.02`) | referral, website, event, partner (distributors, agents), paid ads, other; enquiries and RFQs are leads (`SAL.CRM.01`); no source requires a campaign | §5; §7.1 settings |
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | trade exhibitions, LinkedIn, search ads, email; converted value: the customer's first sales order; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.1 settings |
| Supplier scorecard review (`STK.SUP.01`, `STK.RUL.09`, `STK.ACL.07`) | score on a 0–100 scale over a 3-month window (monthly snapshots, at least 3 receipts); below 70: `STK.RUL.09` opens a review task to the procurement officer, the QC inspector told; preferred and alternative suppliers per raw material offered on purchase requests; changing an item's preferred supplier needs a second person (`STK.ACL.07`: on) | §5; §7.1 settings |
| Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for operator, line supervisor, QC inspector, storekeeper and maintenance technician, started by `HRM.RUL.16` when the employee record (`HRM.EMP.01`) is created in `onboarding` (none for a record created directly `active`, e.g. by a data import); checklist steps use `WRK.CHK.01`; tasks to the production manager, the line supervisor and the employee; the employee's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds the assignees of an overdue step on the first run after its due date, then every 2 days, and tells the production manager and the line supervisor after 5 days; completion blocks nothing (role activation not blocked) | §5; §7.1 settings |
| SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: production manager (work instructions per line) and QC inspector (inspection and quality SOPs); required readers by role per plant; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP's owner role (the SOP owner named above) after 7 days overdue; acknowledged only by the reader (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.1 settings |
| Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Halal certificate* (subject: item or legal entity), *ISO certificate* and *Other certification* (legal entity; the plants covered in the certificate's scope): responsible role QC inspector; *Trademark registration* (legal entity): general manager / owner; product conformity stays on the 07 conformity kinds (ECAS, EQM, food-contact) and the ICV certificate on its own 07 kind, not on these seeded kinds; `DOC.RUL.01` at 180, 90 and 30 days for trademarks and 90, 30 and 7 days for the others; no `blocks` | §5; §7.1 settings |

#### Edge cases decided
- **Client changes:** every value except the override mode `never` for conformity kinds and `PLT.ACL.09` may be changed per client by the company admin; the template is only the seed.
- **Existing clients:** the template applies to new 07 deployments; existing ones get a migration that sets only values still at the platform default.
- **Customer-specific items:** the item flag *customer-specific* (off by default) decides the default outcome *scrap*; sales may change the outcome per line.
- **A-04:** the fee rate stays 0% (no fee) by default. When a client sets a fee percentage for a customer, wl-sales gives the fee line a configurable VAT category until A-04 is answered; the 07 template seeds it as *standard rated (5% VAT)*. This issue carries `blocked:open-item` for that VAT category only.
- **Responsible staff for an item certificate:** the item has no scope, so the QC inspectors of each plant that holds stock of the item or has an open work order for it are notified; when no plant does, the QC inspectors of every plant.

#### Must not
- Must not allow switching the conformity override to `with_approval`.
- Must not hard-code any 07 value in a module.

#### Done when
- [ ] **Void:** in a new 07 deployment, `void` is offered on a production report of an `in_progress` WO and not on an issued invoice or a sent certificate.
- [ ] **Cancellation:** cancelling a line of a standard item after its WO started proposes *finish to stock*; for a customer-specific item *scrap*; fee 0%.
- [ ] **Credit and cheques:** a customer with an invoice 31 days overdue gets the next order on hold; a bounced cheque puts all open orders on account on hold; no bounce fee is invoiced; a third presentation is refused.
- [ ] **Documents:** `Documents::blocks_for("dispatch", "pick")` and `blocks_for("dispatch", "dispatch")` return the three conformity kinds with mode `block` and override `never`; the industrial licence has no `blocks`.
- [ ] **Responsible roles:** given plants P1 (stock of item X) and P2 (no stock, no work order for X), an expiring ECAS certificate of X (a 07 conformity kind, row *Document kinds ECAS, EQM, food-contact*, not a BL-DOC-10 kind) notifies the QC inspectors of P1 only; given item Y with no stock or work order anywhere, its expiring certificate notifies the QC inspectors of P1 and P2; an expiring ICV certificate (07 kind, row *Industrial licence, Industrial Registry, ICV certificate*) notifies the general manager / owner.
- [ ] **Notifications:** `mfg.recall_notice` is registered with no group and urgency `urgent`; when it gives up, the recall owner gets a task and an in-app alert; `MFG.RUL.02` is registered with no group and give-up `none`.
- [ ] **Party block:** blocking customer C holds its undelivered sales orders; its WOs stay `in_progress`.
- [ ] **Integrations and late entry:** a production report entered 30 hours late creates an approval request to the production manager; one entered 5 hours late gets only the flag.
- [ ] **Compliance:** a `MFG.RUL.08` run that ends `failed` (test: the job raises an error) alerts the general manager / owner role and the company admin and opens an owner task (BL-KRN-08).
- [ ] **Money:** every invoice write-off creates an approval request (AED 0).
- [ ] **Stock:** a wastage of AED 2,100 needs approval; an RTS of AED 5,100 needs approval; a work order material issue below zero creates a pending issue, a transfer below zero is refused.
- [ ] **Disputes:** a portal buyer sees "Dispute this invoice"; an open dispute escalates on day 14.
- [ ] **Cash-flow forecast:** in a fresh 07 deployment, a projected balance of AED 199,999 in week
      5 alerts the general manager / owner and the accountant once (`FIN.RUL.23`, 06:00 run);
      AED 200,000 or more in every week alerts no one; the forecast shows 13 weekly buckets from
      Monday.
- [ ] **Budget and drawings:** a monthly budget for a legal entity prepared by the accountant
      needs approval by the general manager / owner; an expense with category *owner drawings* is
      left out of operating cost and profit reports, and the user who records it cannot approve
      it (`FIN.ACL.21`); an expense line reaching 100% of its month's budget line warns the
      category's owner role and the accountant once (`FIN.RUL.24`).
- [ ] **Lead sources:** a new lead or party offers the sources referral, website, event, partner,
      paid ads and other; a campaign with channel *trade exhibitions*, a budget and cost entries
      reports leads, conversion, cost per lead, CAC and ROAS; a cost entry over the budget waits
      for a manager (`SAL.ACL.07`).
- [ ] **Supplier scorecard:** a supplier scoring 69 of 100 over the 3-month window creates a
      review task for the procurement officer (`STK.RUL.09`); 70 creates none; an item's
      alternative supplier is offered on a purchase request for the item; making it the preferred
      supplier waits for a second person (`STK.ACL.07`).
- [ ] **Onboarding and SOPs:** creating an operator's employee record in `onboarding` creates the
      operator checklist with tasks to the production manager, the line supervisor and the employee
      (`HRM.RUL.16`), and the role is active before it is complete; a step still open on the first
      run after its due date reminds its assignees, and again 2 days later (`HRM.RUL.17`); a new
      version of an SOP owned by the production manager asks every required reader to acknowledge
      again within 7 days (`DOC.RUL.04`), and no one else can acknowledge for them (`HRM.ACL.10`).
- [ ] **Registrations:** a fresh 07 deployment lists *Halal certificate*, *ISO certificate*,
      *Other certification* and *Trademark registration* with responsible role QC inspector, and
      general manager / owner for the trademark; a trademark registration expiring in 90 days
      notifies the general manager / owner.

#### Out of scope
The mechanisms themselves (module epics listed in Depends on).

### BL-MFG-15 — 07 Manufacturing: report supplier-caused NCRs to the supplier scorecard

**Labels:** `vertical:07-manufacturing` `area:sme-gap` `priority:medium` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** BL-STK-29 · Base: 07 NCR lifecycle · **Blocks:** None
**Review items:** G3 (SME problem map 27, 49)

#### Goal
The supplier scorecard counts quality incidents per supplier. In 07 these are NCRs on incoming
lots. Implement the `SupplierIncidentSource` adapter so that NCRs whose cause is the supplier
reach the score, and so that a cancelled NCR or a changed cause drops out again.

#### Files to read
- `docs/verticals/07-manufacturing.md` §7.1 *NCR* (states, transitions, the cause paragraph)
- `docs/modules/wl-inventory.md` §3 *Supplier score* (quality incidents), §8 `SupplierIncidentSource`

#### Terms
- **NCR:** non-conformance report on a lot (`MFG.QCL.02`).
- **Cause:** NCR field *supplier*, *internal process* or *unknown*.
- **`SupplierIncidentSource`:** the wl-inventory extension point that lists quality incidents
  per supplier and date for the scorecard, and reports later voids or cause changes.

#### Spec excerpt (authoritative for this issue)
From 07 §7.1: "Every NCR records a cause: supplier, internal process or unknown. An NCR raised at
incoming inspection defaults to supplier with the lot's supplier; QC may change it until close,
with a reason. Only NCRs with cause supplier that are not cancelled are reported to the supplier
scorecard (`STK.SUP.01`) through `SupplierIncidentSource`; a later change of cause or a
cancellation is reported too, so the live score drops the incident."

From wl-inventory §8: `SupplierIncidentSource` — "Quality incidents per supplier and date for the
scorecard: only incidents whose cause is the supplier and that are not voided; a later void or
change of cause is reported too. 07: NCRs on incoming lots (apps/manufacturing)."

#### Edge cases decided
- **Incident date:** the date the NCR was raised (Asia/Dubai), not the disposition date.
- **Supplier:** the lot's supplier at the time of the NCR; an NCR on a lot with no supplier (own
  production) is never reported.
- **Cause changed after the monthly snapshot:** the snapshot stays as it was (snapshots are never
  edited); the live view and the next snapshot use the new cause.
- **Reopened NCR:** stays one incident; reopening does not count it twice.

#### Must not
- Must not report NCRs with cause *internal process* or *unknown*.
- Must not edit a stored monthly snapshot.

#### Done when
- [ ] Given an incoming-inspection NCR on lot L-77 from supplier S raised on 2027-02-10, when the
      adapter is read for S and February, then it returns one incident dated 2027-02-10.
- [ ] Given QC changes that NCR's cause to *internal process* with a reason, then the adapter
      no longer returns it and reports the change.
- [ ] Given an NCR that is `cancelled`, then it is not returned.
- [ ] Given an NCR on a lot from own production, then nothing is returned.
- [ ] Given the March snapshot was stored before a cause change on a March NCR, then the stored
      snapshot is unchanged and the live view drops the incident.

#### Out of scope
The score itself and its weights (BL-STK-29); supplier recalls (wl-inventory).

