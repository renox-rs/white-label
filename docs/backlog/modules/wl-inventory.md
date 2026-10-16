# Epic: wl-inventory — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows in [wl-inventory](../../modules/wl-inventory.md): goods
rejected at receipt, returns to suppliers and free replacements, purchase order revision and
reopening, stock holds and recalls with a forward trace, the negative stock policy with
pending issues, landed cost adjustments after allocation, and the dispatch's reactions to
refused, discarded, lost and reversed deliveries and to cancelled sales order lines, and the
blocked-supplier guard on purchase orders. It closes
X-08, X-09 and X-10 and the stock parts of X-11, X-12 and X-14, and the HIGH items 02-N10-a,
05-N10-a, 06-N10-b, 06-N10-c, 06-N10-e, 07-N4-a, 07-N4-c and 07-N10-a. BL-STK-28 to 31 add the
supplier scorecard and alternative suppliers (`STK.SUP.01`, gap G3 of the
[SME problem map](../../reviews/2026-10-16-sme-problem-map.md)).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-STK-01 | Post a goods receipt with accepted and rejected quantity per line | high | m | Launch 02 | Base: wl-inventory Purchase order and Goods receipt lifecycle |
| BL-STK-02 | Refuse to reverse a goods receipt once its return left | high | s | Launch 02 | BL-STK-01; BL-STK-03 |
| BL-STK-03 | Dispatch a return to supplier | high | m | Launch 02 | BL-STK-01; Epic wl-kernel: approval returned |
| BL-STK-04 | Reopen a received PO when goods go back for replacement | high | s | Launch 02 | BL-STK-03 |
| BL-STK-05 | Credit or cancel a return to supplier | high | m | Launch 02 | BL-STK-03; Epic wl-invoicing: supplier debit note |
| BL-STK-06 | Receive a free replacement against a return to supplier | high | s | Launch 02 | BL-STK-03; BL-STK-05 |
| BL-STK-07 | Chase returns to supplier not credited (`STK.RUL.08`) | medium | s | Launch 02 | BL-STK-03 |
| BL-STK-08 | Revise an issued purchase order | high | m | Launch 02 | Base: wl-inventory Purchase order lifecycle; Epic wl-kernel: approval returned |
| BL-STK-09 | Adjust an allocated landed cost | medium | m | Launch 02 | Base: wl-inventory Landed cost lifecycle; Epic wl-kernel: approval returned |
| BL-STK-10 | Draft a landed cost adjustment from a late supplier bill | medium | s | Launch 02 | BL-STK-09; Epic wl-invoicing: `SupplierBillApproved` |
| BL-STK-11 | Place a stock hold and block held quantity | high | m | Launch 05 | Base: wl-inventory Lot lifecycle and stock movements |
| BL-STK-12 | Decide stock hold lines, close or cancel the hold | high | m | Launch 05 | BL-STK-11; BL-STK-03; Epic wl-kernel: approval returned |
| BL-STK-13 | Remind and escalate open stock holds (`STK.RUL.07`) | medium | s | Launch 05 | BL-STK-11 |
| BL-STK-14 | Open a recall and hold every lot in scope | high | m | Launch 05 | BL-STK-11 |
| BL-STK-15 | Trace a recall forward to subjects and parties | high | m | Launch 05 | BL-STK-14; BL-STK-18 |
| BL-STK-16 | Send recall notices and close the recall | high | m | Launch 05 | BL-STK-12; BL-STK-15; Epic wl-notify: `MessageGaveUp` |
| BL-STK-17 | Recall items not tracked by lot | high | m | Launch 08 | BL-STK-15 |
| BL-STK-18 | Record pending issues where negative stock is blocked | medium | m | Launch 05 | Base: wl-inventory stock movements |
| BL-STK-19 | Allow automatic consumption below zero (`allow_consumption`) | medium | s | Launch 06 | BL-STK-18; BL-STK-11 |
| BL-STK-20 | Alert on pending issues and negative balances (`STK.RUL.06`) | medium | s | Launch 05 | BL-STK-18 |
| BL-STK-21 | Block dispatch on a failed credit re-check or a held lot | high | m | Launch 07 | Base: wl-inventory Dispatch lifecycle; BL-STK-11; BL-SAL-10 |
| BL-STK-22 | Confirm delivery per line and take refused goods to quarantine | high | s | Launch 07 | Base: wl-inventory Dispatch lifecycle; BL-DLV-02; BL-DLV-11 |
| BL-STK-23 | Record a dispatch discarded by wl-delivery | high | s | Launch 07 | Base: wl-inventory Dispatch lifecycle; BL-DLV-07 |
| BL-STK-24 | Hold a lost dispatch as a loss and reverse it when found | high | s | Launch 07 | Base: wl-inventory Dispatch lifecycle; BL-DLV-06 |
| BL-STK-25 | Reopen a delivered dispatch on a delivery reversal | high | s | Launch 07 | Base: wl-inventory Dispatch lifecycle; BL-DLV-05 |
| BL-STK-26 | Apply sales cancellation outcomes to stock | high | m | Launch 06 | BL-SAL-07; BL-SAL-08; BL-SAL-09 |
| BL-STK-27 | Refuse a new or extended purchase order to a blocked supplier | high | s | Launch 02 | Base: wl-inventory Purchase order lifecycle; BL-PTY-04; BL-STK-08 |
| BL-STK-28 | Keep preferred and alternative suppliers per item and use them in reorder suggestions | medium | m | Launch 02 | Base: wl-inventory Item catalogue and reorder suggestions; Epic wl-parties: `BlockGuard`; BL-DOC-10 |
| BL-STK-29 | Compute the supplier score and keep monthly snapshots | medium | m | Launch 02 | BL-STK-01; BL-STK-03; BL-STK-05; BL-STK-28 (`SupplierIncidentSource` defined here) |
| BL-STK-30 | Ask procurement to review a supplier below the threshold (`STK.RUL.09`) | medium | s | Launch 02 | BL-STK-29; BL-PTY-01 |
| BL-STK-31 | Approve a change of an item's preferred supplier (`STK.ACL.07`) | medium | s | Launch 02 | BL-STK-28; BL-STK-29; Epic wl-kernel: approval returned |

---

### BL-STK-01 — wl-inventory: post a goods receipt with accepted and rejected quantity per line

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: wl-inventory Purchase order and Goods receipt lifecycle · **Blocks:** BL-STK-02, BL-STK-03, BL-STK-29, BL-CON-17, BL-FNB-17
**Review items:** X-08, 02-N10-a, 06-N10-b, 07-N10-a

#### Goal
At receiving, the storekeeper records per line how much was accepted and how much was rejected
(damaged, wrong, warm, short-dated). Only accepted quantity enters stock, updates the PO and
reaches the supplier bill match. Rejected goods never enter stock: they go on a Return to
supplier, dispatched at once when refused at the door, or a draft when kept for collection.

#### Files to read
- `docs/modules/wl-inventory.md` §3 (*Goods receipt*, *Purchase order*, *Return to supplier*), §4 *Goods receipt* (row `post`, rejection reasons and over-receipt paragraphs), §4 *Purchase order* (row `receive`), §5.1 step 4, §5.6 steps 1–2, §6 event `GoodsReceived`, §7 setting "Goods receipt rejection reasons"

#### Terms
- **Accepted qty / rejected qty:** per receipt line; accepted + rejected = received.
- **Refused at the door:** the supplier's driver takes the goods back immediately.
- **Kept for collection:** rejected goods wait aside (not in stock) until the supplier collects.
- **Outstanding qty:** PO line quantity not yet accepted.
- **Over-receipt tolerance:** default 5% (`STK.ACL.04`).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `post` | draft | posted | `goods_receipts.post` | location in the user's scope; source is a PO, or a dispatched Return to supplier (no PO: free replacement, lines and qty not above the returned lines); lot and expiry for lot-tracked items; per line accepted + rejected = received; a reason for every rejected qty; over-receipt within tolerance, otherwise approval | `receive` movements for accepted qty only; against a Return to supplier: (BL-STK-06); lots created (`quarantine` or `released` per item); average cost updated; PO `receive`; rejected lines → a Return to supplier (dispatched at once when refused at the door, `draft` when kept for collection; rejected goods never enter stock); event `GoodsReceived` with accepted and rejected qty per line |

Purchase order:

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `receive` | issued, partially_received | partially_received, received | system (goods receipt `post`) | over-receipt tolerance on accepted qty | only accepted qty counts; rejected qty stays outstanding; `received` when every line is fully accepted |

Rejection reasons (configurable): `temperature`, `quality`, `damaged`, `wrong_item`,
`short_shelf_life`, `excess`, `other`. A vertical may require a reading or photo for a reason
(06: a temperature reading from the receiving checklist). The storekeeper sees the rejected qty
on the receipt; procurement and the supplier contact are notified through the Return to supplier.

Event `GoodsReceived`: receipt id, PO id, lines with accepted qty, rejected qty and reason, values.

Over-receipt above the tolerance (`STK.ACL.04`) sends the receipt to approval (`PLT.APR.01`)
by a holder of `purchase_orders.issue` in the location's scope who is not the user posting it;
the receipt stays `draft` until approved, and a rejection returns it for correction.

#### Edge cases decided
- **Mixed lines on one receipt:** one Return to supplier for door-refused lines (dispatched at once, collection proof = the goods receipt) and a separate draft one for kept lines.
- **Wanted outcome of rejected lines:** `replace`, because rejected qty stays outstanding on the PO; the user may switch a line to `credit` and must then short-close the PO line if no replacement is wanted.
- **Excess:** rejected *excess* does not change outstanding; outstanding = ordered − accepted, never below 0.
- **Over-receipt tolerance:** checked on accepted qty only (5% default). Above it, `post` creates an approval request (`PLT.APR.01`) for a holder of `purchase_orders.issue` in the location's scope who is not the poster; the receipt stays `draft` until approved; a rejection returns it for correction.
- **Return to supplier in this issue:** `post` creates the Return to supplier with its lines (rejected at receipt, reason, wanted outcome, *refused at the door* or *kept for collection*) in `draft`. The system `dispatch` of door-refused returns (state `dispatched`, return note, `SupplierReturnDispatched`) is built in BL-STK-03; until BL-STK-03 is merged a door-refused return stays `draft`.
- **Evidence per reason:** when the vertical requires it (06 temperature reading), `post` is refused without it.
- **All rejected:** allowed; no `receive` movement; PO state unchanged; the return is created.
- **Decimals:** quantities in the PO unit converted to base unit by the fixed factor; accepted + rejected must equal received exactly.

#### Must not
- Must not post any movement for rejected qty.
- Must not count rejected qty in the 3-way match or as received on the PO.

#### Done when
- [ ] Given PO line 100 kg chicken, when the storekeeper records received 100, accepted 80, rejected 20 *temperature* (reading 9 °C) refused at the door, then stock +80 kg, PO `partially_received` with 20 outstanding, a Return to supplier for 20 kg marked *refused at the door* and *rejected at receipt* exists (its system `dispatch` is checked in BL-STK-03), and `GoodsReceived` shows 80/20.
- [ ] Given rejected 20 without a reason, `post` is refused.
- [ ] Given accepted 106 of 100 (tolerance 5%), storekeeper A's `post` creates an approval request and the receipt stays `draft`; A cannot approve; procurement user B approves → `posted`, stock +106.
- [ ] Given accepted 105 of 100, `post` needs no approval.
- [ ] Given rejected qty *kept for collection*, the Return to supplier is `draft` and nothing enters stock.
- [ ] The supplier bill match sees 80 kg.

#### Out of scope
Return `dispatch`, including the system dispatch of door-refused goods, and credit (BL-STK-03, BL-STK-05); receipts against a return (BL-STK-06); debit note (Epic wl-invoicing).

---

### BL-STK-02 — wl-inventory: refuse to reverse a goods receipt once its return left

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-STK-01 · BL-STK-03 · **Blocks:** None.
**Review items:** X-08

#### Goal
A goods receipt entered by mistake can be reversed, but not after a Return to supplier
created from it has left with the supplier. Draft returns from it are cancelled with the
reversal.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Goods receipt* row `reverse`

#### Terms
- **Reverse:** posts reversing movements; the receipt becomes `reversed` (terminal).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reverse` | posted | reversed | `goods_receipts.reverse` | stock still available: the full accepted qty of every line (and lot) is still available at the receipt's location, none of it consumed, moved, reserved or held (otherwise correct it by an adjustment or a Return to supplier); not yet billed; no Return to supplier from it dispatched | reversing movements; PO quantities restored; draft returns from it cancelled |

#### Edge cases decided
- **"Stock still available":** all or nothing. If any part of the accepted qty was consumed, transferred, reserved or put on hold, `reverse` is refused with the line and the missing qty; there is no partial reversal. The user corrects with a stock adjustment (`STK.ACL.02`) or a Return to supplier.
- **"PO quantities restored":** outstanding goes back up by the accepted qty only.
- **Draft return cancellation** uses Return to supplier `cancel` with reason "receipt reversed" (BL-STK-05).
- **Kept-for-collection goods** of a cancelled draft return: physically still there; the cancel effect applies (received on a new receipt or discarded).

#### Must not
- Must not reverse when any return from it is `dispatched` or `credited`.

#### Done when
- [ ] Given a posted receipt (80 accepted) with a door-refused return `dispatched`, `reverse` is refused.
- [ ] Given a posted receipt with only a `draft` return, `reverse` posts −80, PO outstanding +80, and the draft return is `cancelled`.
- [ ] Given a posted receipt of 80 of which 10 were issued to a job, `reverse` is refused naming the line and the 10 missing; nothing is posted.

#### Out of scope
None.

---

### BL-STK-03 — wl-inventory: dispatch a return to supplier

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-STK-01 · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-STK-02, BL-STK-04, BL-STK-05, BL-STK-06, BL-STK-07, BL-STK-12, BL-STK-29, BL-AUT-11, BL-AUT-18, BL-CON-17, BL-FNB-17, BL-MFG-08, BL-MFG-14
**Review items:** X-08, 02-N10-a, 07-N10-a

#### Goal
Goods go back to the supplier for a credit or a replacement, from a receipt rejection, a
stock hold, a recall or an NCR. Dispatching the return moves stock out (for lines that were
in stock), restores PO quantity for replacements, and makes wl-invoicing draft a supplier
debit note for credit lines. Above AED 5,000 a second person approves.

#### Files to read
- `docs/modules/wl-inventory.md` §3 *Return to supplier*, §4 *Return to supplier* (states, row `dispatch`, paragraph below), §4 *Lot* row `reject`, §5.6 steps 2–4, §6 (`STK.ACL.06`, event `SupplierReturnDispatched`), §7

#### Terms
- **Line from stock / rejected at receipt:** whether the goods were in stock (quarantine or source location) or never entered.
- **Wanted outcome:** `credit` (debit note) or `replace` (new goods).
- **Collection proof:** supplier driver signature (`DOC.SIG.01`), courier number, or the goods receipt.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Lines being prepared, or rejected goods waiting for collection |
| `dispatched` | info | no | Goods left with the supplier or their driver; credit or replacement awaited |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `dispatch` | draft | dispatched | `supplier_returns.manage`, or system (goods receipt `post` for goods refused at the door) | supplier and lines set; lines from stock available in the quarantine or source location; collection proof (supplier driver signature `DOC.SIG.01`, courier no., or the goods receipt); `STK.ACL.06` above the value (value = qty × unit cost per line, lines rejected at receipt at the PO price, excl. VAT), except for the system dispatch of goods refused at the door, which never entered stock and already left | `return_out` movements for lines from stock, valued at the source receipt's unit cost when known, else average cost (difference to the price variance); none for lines rejected at receipt; lot `reject` (system) when nothing of a held lot remains; `replace` lines from stock: PO outstanding qty restored (PO `reopen` when `received`); lines rejected at receipt restore nothing (their qty stayed outstanding at `post`); return note PDF; supplier contact notified; event `SupplierReturnDispatched` |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `STK.ACL.06` | A landed cost adjustment or a Return to supplier above a value needs approval; the creator cannot approve | `LIM`, `SOD` | AED 5,000 (02: AED 10,000, approver project manager) |

`SupplierReturnDispatched` makes wl-invoicing draft a supplier debit note for the `credit`
lines at the PO price until the supplier bill arrives, then at the bill's prices
(`FIN.RUL.17`; input VAT adjusted there). When the supplier bill is not yet posted,
the 3-way match bills accepted qty only and no debit note is needed. Returns after a PO is
`closed` are credited only (no replacement on that PO).

Event `SupplierReturnDispatched`: return id, supplier, source, lines (item, lot, qty, PO line,
PO price, wanted outcome, reason).

#### Edge cases decided
- **System dispatch of door-refused goods:** `STK.ACL.06` does not apply (guard exception: the goods never entered stock and already left); runs at goods receipt `post` with the goods receipt as collection proof.
- **Value for `STK.ACL.06`:** sum of line qty × unit cost, excl. VAT; lines rejected at receipt count at the PO price. This matters for returns of goods *kept for collection*, which are dispatched by a user and do go through `STK.ACL.06`.
- **Approval pending:** the return stays `draft`; stock reserved for its lines stays reserved. Rejected or returned: stays `draft`.
- **Lines rejected at receipt with `replace`:** their qty stayed outstanding on the PO at receipt (BL-STK-01), so dispatch restores nothing for them; only `replace` lines from stock restore outstanding (no double count).
- **Debit note price:** wl-invoicing drafts at the PO price (event payload) and re-prices to the bill's prices when the bill arrives; nothing to do here beyond the payload.
- **`replace` on a `closed` PO:** refused on that PO; the line must be `credit`, or a free replacement against the return (BL-STK-06); the user may keep `replace` without a PO line, which is then received only through BL-STK-06.
- **Lot `reject`:** only when the lot was `on_hold` and its remaining quantity reaches 0 with this return.
- **Valuation difference:** (source receipt unit cost − average cost) × qty goes to the purchase price variance through `CostPoster`.

#### Must not
- Must not post `return_out` for lines rejected at receipt.
- Must not let the creator approve above the value.
- Must not create the debit note here (wl-invoicing reacts to the event).

#### Done when
- [ ] Given a draft return of 10 units from quarantine (receipt cost AED 50, average AED 48) with outcome `credit`, when dispatched with a driver signature, then −10 `return_out` at AED 50 is posted, AED 20 goes to price variance, a return note PDF exists, and `SupplierReturnDispatched` is emitted.
- [ ] Given a return worth AED 6,000 (limit 5,000), creator A's dispatch creates an approval request; A cannot approve; B approves → `dispatched`.
- [ ] Given a `replace` line from stock on a `received` PO, PO outstanding +qty and the PO is `partially_received`.
- [ ] Given a PO line 100 with 80 accepted and 20 rejected at receipt (outstanding 20), when the return of the 20 with `replace` is dispatched, outstanding stays 20 (not 40).
- [ ] Given a door-refused return of AED 8,000 from BL-STK-01, posting the receipt dispatches it at once (system) with no movement and no approval request.
- [ ] Given a kept-for-collection return of 120 units rejected at receipt at PO price AED 50 (AED 6,000), creator A's `dispatch` creates an approval request.
- [ ] Given the last 5 units of an `on_hold` lot returned, the lot becomes `rejected`.

#### Out of scope
PO `reopen` itself (BL-STK-04); credit and cancel (BL-STK-05); debit note (Epic wl-invoicing).

---

### BL-STK-04 — wl-inventory: reopen a received PO when goods go back for replacement

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-STK-03 · **Blocks:** BL-CON-17
**Review items:** X-08

#### Goal
When goods from a fully received PO are returned for replacement, the PO must expect them
again so the replacement can be received on it and committed cost is restored.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Purchase order* (state `received` not terminal, row `reopen`, sentence below the table)

#### Terms
- **Committed cost:** the value of open POs per cost object (`FIN.CST.01`).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `received` | success | no | All quantities received (or short-closed) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reopen` | received | partially_received | system (Return to supplier `dispatch` with outcome `replace`) | PO not `closed` | returned qty outstanding again; committed cost restored |

#### Edge cases decided
- **Short-closed line:** reopening restores only the returned qty on that line; the short-closed remainder stays released.
- **PO `closed`:** guard fails; the return line cannot be `replace` on this PO (BL-STK-03).
- **Several returns:** each adds its qty; idempotent per return line.

#### Must not
- Must not be offered to users.

#### Done when
- [ ] Given a `received` PO line 50 (all accepted), when a return of 10 with `replace` is dispatched, the PO is `partially_received`, outstanding 10, committed cost +10 × PO price.
- [ ] Given a `closed` PO, no reopen happens.

#### Out of scope
None.

---

### BL-STK-05 — wl-inventory: credit or cancel a return to supplier

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-STK-03 · Epic wl-invoicing: supplier debit note (`SupplierDebitNoteIssued`, `SupplierDebitNoteCancelled`) · **Blocks:** BL-STK-06, BL-STK-29, BL-AUT-11, BL-CON-17, BL-MFG-08
**Review items:** X-08

#### Goal
A dispatched return ends `credited` when the supplier's debit note covers every credit line
and replacements cover every replace line, or when a manager settles it otherwise with a
reason. A supplier that refuses the credit is a loss. A draft return that will not be sent is
cancelled and its goods stay.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Return to supplier* (state `credited`, `cancelled`, rows `credit`, `cancel`), §6 *Consumes* wl-invoicing row, `STK.ACL.02`

#### Terms
- **Debit note:** the buyer's document reducing the supplier bill (wl-invoicing).
- **`STK.ACL.02`:** adjustments, write-offs and wastage above a value need approval; recorder cannot approve (AED 1,000; 06: 200; 07: 2,000).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `credited` | success | yes | Supplier debit note issued for `credit` lines and replacements received for `replace` lines, or settled otherwise with a reason |
| `cancelled` | neutral | yes | Not sent; goods kept |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `credit` | dispatched | credited | system (`SupplierDebitNoteIssued` from wl-invoicing covers every `credit` line and replacement receipts cover every `replace` line), or `supplier_returns.manage` | manual: reason (refund received in wl-payments, supplier refused); a refused credit is a loss within `STK.ACL.02` | event `SupplierReturnCredited`; refused value posted as wastage with reason |
| `cancel` | draft | cancelled | `supplier_returns.manage` | reason given | reserved stock released back to its location; rejected-at-receipt goods kept are then received on a new goods receipt or discarded |

wl-invoicing `SupplierDebitNoteCancelled`: the return stays `dispatched`; task to procurement.

#### Edge cases decided
- **Partial coverage:** a debit note covering some credit lines, or replacements covering some replace lines, leaves the return `dispatched`; coverage is tracked per line.
- **Manual *supplier refused*:** the uncovered value is posted as wastage (no movement for lines rejected at receipt; value only through `CostPoster`) with reason *supplier refused credit*; above `STK.ACL.02` → approval; requester cannot approve.
- **Debit note cancelled after `credited`:** `credited` is terminal; procurement gets the task and finance handles the bill; the return does not reopen.
- **Cancel of rejected-at-receipt goods kept:** the user chooses *receive on a new goods receipt* (opens a draft receipt on the PO) or *discard* (no movement; the value never entered stock).

#### Must not
- Must not mark `credited` while a credit line has no debit note and no manual reason.

#### Done when
- [ ] Given a dispatched return with credit lines A (AED 400) and B (AED 200), when `SupplierDebitNoteIssued` covers A only, it stays `dispatched`; when a second covers B, it is `credited` and `SupplierReturnCredited` is emitted.
- [ ] Given manual credit reason *supplier refused* on AED 1,500 at a client with limit AED 1,000, an approval is needed and, when approved, AED 1,500 is posted as wastage.
- [ ] Given `SupplierDebitNoteCancelled` for a dispatched return, procurement gets a task and the return stays `dispatched`.
- [ ] Given a draft return with 5 units reserved from quarantine, `cancel` releases them.

#### Out of scope
Replacement receipt (BL-STK-06); debit note (Epic wl-invoicing).

---

### BL-STK-06 — wl-inventory: receive a free replacement against a return to supplier

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-STK-03 · BL-STK-05 · **Blocks:** BL-AUT-11
**Review items:** X-08

#### Goal
A supplier may replace returned goods free of charge with no PO (warranty, or long after the
PO closed). The storekeeper receives them on a goods receipt against the dispatched return, at
zero cost, outside the 3-way match.

#### Files to read
- `docs/modules/wl-inventory.md` §3 *Goods receipt*, §4 *Goods receipt* row `post`, §5.6 step 6

#### Terms
- **Average cost dilution:** receiving at zero cost lowers the moving average.

#### Spec excerpt (authoritative for this issue)

`post` guard (part): source is a PO, or a dispatched Return to supplier (no PO: free
replacement, lines and qty not above the returned lines).

`post` effect (part): against a Return to supplier: received at zero cost (average cost
diluted accordingly), no PO update and nothing to match in wl-invoicing, the return line
counted as replaced (Return to supplier `credit` when all lines are covered).

#### Edge cases decided
- **Which lines:** only `replace` lines without a PO line, or on a `closed` PO. `replace` lines with an open PO line are received on the PO, not here.
- **Partial replacement:** allowed in several receipts; the sum per line never above the returned qty.
- **Rejected again at receipt:** the rejected qty is not counted as replaced and goes on a new return (as BL-STK-01).

#### Must not
- Must not create a PO receipt or anything to match in wl-invoicing.

#### Done when
- [ ] Given a dispatched return with a `replace` line of 4 units and no PO line, a receipt of 4 at zero cost posts +4, average cost falls accordingly, and the return becomes `credited`.
- [ ] A receipt of 5 against it is refused.

#### Out of scope
None.

---

### BL-STK-07 — wl-inventory: chase returns to supplier not credited (`STK.RUL.08`)

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-STK-03 · **Blocks:** BL-CON-17, BL-FNB-17
**Review items:** X-08

#### Goal
Money waits on returns that stay `dispatched`. After 30 days the rule tells procurement and
finance and gives procurement a task to chase the supplier.

#### Files to read
- `docs/modules/wl-inventory.md` §6 `STK.RUL.08`, §7

#### Terms
None beyond the epic.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default |
|---|---|---|---|---|---|
| `STK.RUL.08` | daily | Return to supplier `dispatched` for more than N days | notify procurement and finance on the first run after day N, then every N days from that notice; one task to procurement to chase the supplier, kept while the return stays `dispatched` | N = 30 | on |

#### Edge cases decided
- **Repeat:** one task per return while it stays open (no new task daily). The first notice goes on the first run where the return is `dispatched` for more than N days (day N + 1 from the `dispatch` date); the notice then repeats every N days counted from that first notice (days N + 1, 2N + 1, 3N + 1…), until the return leaves `dispatched`.
- **Day count:** calendar days in Asia/Dubai from the `dispatch` date; runs at 07:00.

#### Must not
- Must not create a task per day.

#### Done when
- [ ] Given a return dispatched 2026-09-01, the run on 2026-10-01 does nothing (30 days); the run on 2026-10-02 notifies and creates one task; the run on 2026-10-03 creates none.
- [ ] The same return still `dispatched` is notified again on 2026-11-01, with no new task; credited on 2026-11-10, it is not notified on 2026-12-01.

#### Out of scope
None.

---

### BL-STK-08 — wl-inventory: revise an issued purchase order

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: wl-inventory Purchase order lifecycle · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-CON-17, BL-STK-27
**Review items:** X-08 (02 MEDIUM: PO revision)

#### Goal
Quantities, prices or dates change after a PO was sent. Procurement revises it: a new
revision number, the earlier one kept read-only, the supplier gets the revised PDF and
committed cost follows. A higher total goes through the approval limits again, and the
current revision stays valid until then.

#### Files to read
- `docs/modules/wl-inventory.md` §3 *Purchase order* (revision fields), §4 *Purchase order* row `revise`, §5.1 step 7, §6 (`STK.ACL.01`, `STK.ACL.03`, event `PurchaseOrderRevised`)

#### Terms
- **`STK.ACL.01`:** the user who creates a PO cannot approve it.
- **`STK.ACL.03`:** POs above value limits need approval in steps (02: AED 10,000 / 50,000; 06: AED 5,000).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `revise` | issued, partially_received | same state | `purchase_orders.issue` | reason given; currency unchanged (a currency change needs `cancel` and a new PO); no line reduced below its accepted qty; `STK.ACL.01`; a higher total or a later delivery date needs `BlockGuard::check(supplier, extend)` (`PTY.ACL.03`); a higher total re-checks `STK.ACL.03` on the new total: when the new total is above a limit, an approval request for the steps the new total needs, and the current revision stays valid until it is approved; a higher total that stays below the first limit, or a total not higher, applies at once | revision number + 1; previous revision kept read-only; PDF marked "Revision N" re-sent to the supplier; committed cost adjusted; open receipts use the new lines; event `PurchaseOrderRevised` |

Event `PurchaseOrderRevised`: PO id, revision number, changed lines, old and new total, reason.

#### Edge cases decided
- **Total not higher:** no approval; applied at once.
- **Higher total below the first limit:** no approval; applied at once (02: AED 9,000 → 9,500 with limit AED 10,000).
- **Approval steps:** the new total decides the steps, even when the previous revision was already approved (02: AED 9,000 → 12,000 → first step; AED 12,000 → 13,000 → first step again; AED 40,000 → 60,000 → both steps; 06: any new total above AED 5,000).
- **Goods received while the approval waits:** receipts use the current revision; at approval the guard "not below accepted qty" is re-checked; failing → request `failed`.
- **Draft receipt open during revise:** re-validated against the new lines when posted.
- **Currency:** a revision cannot change the PO currency. currency change needs cancel and a new PO.
- **Reducing a line to its accepted qty:** allowed; the line is then fully received.

#### Must not
- Must not edit the current revision in place.
- Must not send the supplier a revision that waits for approval.

#### Done when
- [ ] Given an `issued` PO rev 1 AED 9,000 (02 limits), when user A raises a line price so the total is AED 12,000, an approval request is created and rev 1 stays current; A cannot approve.
- [ ] When B approves, rev 2 is current, rev 1 is read-only, the PDF "Revision 2" is sent, committed cost +3,000, `PurchaseOrderRevised` is emitted.
- [ ] Given a line with 30 accepted, reducing it to 25 is refused.
- [ ] Given a lower total, the revision applies without approval.
- [ ] Given an `issued` PO rev 1 AED 9,000 (02), a revision to AED 9,500 applies at once as rev 2, without approval.

#### Out of scope
None.

---

### BL-STK-09 — wl-inventory: adjust an allocated landed cost

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:medium` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: wl-inventory Landed cost lifecycle · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-STK-10, BL-AUT-18
**Review items:** X-10 (08 MEDIUM: landed cost adjustment)

#### Goal
A freight, duty or clearing bill often arrives after the landed cost was allocated, or a
duty refund comes later. The purchaser posts a landed cost adjustment: the amount is spread on
the original basis; the share on stock still on hand re-values it, the share on stock already
consumed goes to the landed cost variance (or to the cost object charged on receipt). Nothing
already posted changes.

#### Files to read
- `docs/modules/wl-inventory.md` §3 (*Landed cost*, *Landed cost adjustment*, paragraph "Landed cost after allocation"), §4 *Landed cost* (states, row `adjust`, paragraph below), §5.8 steps 1–2, §6 (`STK.ACL.06`, event `LandedCostAdjusted`), §7 settings

#### Terms
- **Allocation basis:** `value`, `quantity`, `weight` or `volume`, chosen at `allocate`.
- **Landed cost variance:** an account for cost that can no longer be put on stock.
- **Cost object charged on receipt:** the project of a site store receipt (02).
- **Open period:** a period not locked in wl-invoicing.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `allocated` | success | no | Costs spread over the received items; average cost updated; late costs can still be adjusted |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `adjust` | allocated | allocated | `landed_costs.post` | a landed cost adjustment with cost lines and the supplier bill reference; posting date in an open period; `STK.ACL.06` above the value | amount spread on the original basis; per item, the share on qty still on hand re-values stock (average cost updated), the share on qty already consumed goes to the landed cost variance (or to the cost object charged on receipt); event `LandedCostAdjusted` |

Landed cost adjustment: landed cost, cost lines (positive or negative) with the supplier bill
reference, posting date, split per item into re-valued (on hand) and variance (consumed) amounts.
A negative adjustment (duty refund, credit from the forwarder) works the same way. Earlier
movements are never changed.

Setting: variance destination: variance account; the cost object charged on receipt for
site-store receipts. `STK.ACL.06` value: AED 5,000.

#### Edge cases decided
- **"Qty still on hand":** weighted average does not track receipt layers. On-hand share per item = min(current on-hand qty of the item at the receipt's location, qty received on the allocated receipts) ÷ qty received.
- **Negative adjustment larger than the on-hand value:** the re-valuation stops at zero value; the rest goes to the variance.
- **Rounding:** spread in fils, rounding each item's share down; the remainder (a few fils) goes to the item line with the largest share of the adjustment; on a tie, the first of them in receipt line order. Within an item, the on-hand and consumed split is rounded to fils and the remainder goes to the on-hand part.
- **`STK.ACL.06` value:** absolute amount of the adjustment (a −6,000 refund also needs approval).
- **Approval pending:** nothing posts; rejected → draft adjustment stays editable.
- **Posting date in a locked period:** refused; the user picks a date in the open period.

#### Must not
- Must not change earlier movements or the original allocation.
- Must not put the consumed share on stock.

#### Done when
- [ ] Given a landed cost allocated by value on a receipt of 100 units (60 still on hand), when an adjustment of +AED 1,000 is posted, then AED 600 re-values stock (average cost updated) and AED 400 goes to the variance; `LandedCostAdjusted` shows both.
- [ ] Given the same receipt of 100 units posted into a site store of project P (02), with 60 still on hand at that site store, when the +AED 1,000 adjustment is posted, then AED 600 re-values stock and the AED 400 goes to project P's cost object, not to the variance account.
- [ ] Given +AED 1,000 allocated by value over items X (AED 1,000 received) and Y (AED 2,000 received), all on hand, X gets AED 333.33 and Y AED 666.67 (remainder 0.01 to Y, the largest share).
- [ ] Given an adjustment of −AED 6,000, creator A cannot approve; B approves → posted.
- [ ] Given a posting date in a locked period, `adjust` is refused.

#### Out of scope
Draft adjustment from a supplier bill (BL-STK-10).

---

### BL-STK-10 — wl-inventory: draft a landed cost adjustment from a late supplier bill

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-STK-09 · Epic wl-invoicing: `SupplierBillApproved` with a landed cost reference · **Blocks:** None.
**Review items:** X-10

#### Goal
When a supplier bill coded to an already allocated landed cost is approved in wl-invoicing,
the module drafts the adjustment and tells the purchaser, so late costs are never forgotten.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Landed cost* (paragraph below the table), §5.8 step 1, §6 *Consumes* (wl-invoicing row)

#### Terms
- **Coded to a landed cost:** a supplier bill line carrying the landed cost reference.

#### Spec excerpt (authoritative for this issue)

A supplier bill coded to an allocated landed cost (wl-invoicing) creates a draft adjustment and
notifies the purchaser. Consumes: `SupplierBillApproved` with a landed cost reference per line
(drafts a landed cost adjustment when the landed cost is already allocated).

#### Edge cases decided
- **Landed cost still `draft`:** no adjustment; the bill line is added as a cost line of the draft.
- **Idempotency:** one draft adjustment per bill; a second event for the same bill does nothing.
- **Another bill while a draft exists:** a second, different bill coded to the same landed cost creates its own draft adjustment, even when an earlier draft is still unposted; drafts are never merged, so each is posted, approved (`STK.ACL.06`, per adjustment) and traced to its own bill.
- **Several lines of one bill:** one adjustment with several cost lines.

#### Must not
- Must not post the adjustment automatically.

#### Done when
- [ ] Given an allocated landed cost LC-7, when a freight bill of AED 800 coded to LC-7 is approved, a draft adjustment with that line and bill reference exists and the purchaser is notified.
- [ ] The same event twice creates one draft.
- [ ] Given that draft still unposted, when a duty bill of AED 300 coded to LC-7 is approved, a second draft adjustment exists with only the AED 300 line; the first is unchanged.

#### Out of scope
Posting (BL-STK-09).

---

### BL-STK-11 — wl-inventory: place a stock hold and block held quantity

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: wl-inventory Lot lifecycle and stock movements · **Blocks:** BL-STK-12, BL-STK-13, BL-STK-14, BL-STK-19, BL-STK-21, BL-CLN-13, BL-CLN-16, BL-FNB-11, BL-FNB-12, BL-FNB-17, BL-MFG-14
**Review items:** X-09, 06-N10-e

#### Goal
Staff (or an automation such as a fridge alarm) block stock without moving it: whole lots
everywhere, or the stock at one location. Held quantity cannot be picked, issued, consumed,
transferred or dispatched until someone decides.

#### Files to read
- `docs/modules/wl-inventory.md` §3 (*Stock balance* on hold, *Lot* open holds, *Stock hold*), paragraph "Holds", "Picking order", §4 *Stock hold* (rows `place` and `add_lines`, paragraph on stock moved into a held location), §4 *Lot* (state `on_hold`, row `hold`), §4 *Transfer* row `send`, §6 (`StockHoldService`, events)

#### Terms
- **Whole-lot scope:** every location holding the lot; the lot goes `on_hold`.
- **Location scope:** all or selected items at one location; lot status unchanged; only that balance is held.
- **Available:** on hand − reserved − on hold.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `open` | warning | no | Stock blocked; lines waiting for a decision |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `place` | — | open | `stock.hold`, or system (recall `open`, vertical automation) | reason and reference; scope set; at least one balance with quantity | lines created from the balances in scope (item, location, lot, qty); held qty leaves available; whole-lot scope → lot `hold`; holders of `lots.release` in the location's scope notified (push and in-app); event `StockHoldPlaced` |
| `add_lines` | open | open | system (goods receipt `post` or transfer `receive` into the location of an open location-scoped hold, after the warning), or `stock.hold` | location scope; the item is in the hold's scope; quantity moved in | new lines (item, location, lot, qty) with the hold's reason and reference; the quantity never becomes available; deciders notified |

Stock moved into a location while a location-scoped hold on it is open joins that hold. The
goods receipt and transfer screens warn first (e.g. "Fridge 2 is on temperature hold"); if the
user goes on, `add_lines` adds the moved quantity as new lines, decided like the others. A
whole-lot hold needs no new lines: it covers the lot wherever it moves.

Lot:

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `hold` | quarantine, released | on_hold | system (stock hold `place` with whole-lot scope, or recall `open`) | reason given | status before the hold stored on the lot (for `unhold`); lot blocked from picking, issue, consumption, transfer and dispatch; picked dispatches with the lot flagged; event `LotHeld` |

Transfer `send` guard: available stock; released lots only; no held quantity.
"Negative stock", `allow_consumption` (part): Held quantity is never consumed: while any
quantity of the item is held at the location, automatic consumption beyond the available
quantity becomes a pending issue, as under `block`.
Stock hold reasons: `recall`, `quality`, `temperature`, `complaint`, `other`.
"Holds": Held quantity is not available for picking, issue, consumption, transfer or dispatch.

#### Edge cases decided
- **Status before the hold:** stored on the lot when `hold` runs, so a hold cancelled in error can restore it (`unhold`, BL-STK-12).
- **Lot already `on_hold` from another hold:** a second whole-lot hold adds lines; the lot stays `on_hold` (it runs `release` only when no hold is open, BL-STK-12).
- **Reserved quantity:** reservations stay but cannot be picked; affected picked dispatches are flagged (dispatch guard in BL-STK-21).
- **Automatic consumption of held stock** (visit, recipe): held quantity is never consumed; consumption beyond the available quantity becomes a pending issue (BL-STK-18). This holds at every location, also one set to `allow_consumption`: while any quantity of the item is held there, the balance is not taken below the held quantity or below zero (BL-STK-19).
- **Stock arriving later at a location-scoped hold:** a goods receipt or transfer into the location warns first; if the user goes on, `add_lines` adds the moved quantity as new lines (same reason and reference), never available while the hold is open (06 fridge). Whole-lot holds cover the lot wherever it moves.
- **Expired lot:** an `expired` lot cannot run `hold` (not in From); its balance is still added as a hold line, the lot stays `expired`.
- **Notification:** to every user with `lots.release` in the location's scope.

#### Must not
- Must not move stock when placing a hold.
- Must not let any movement except the hold outcomes take held quantity.

#### Done when
- [ ] Given lot L1 (40 at Store A, 10 in transit), when a whole-lot hold *quality* is placed, lines 40 and 10 exist, L1 is `on_hold`, available is 0, `StockHoldPlaced` and `LotHeld` are emitted.
- [ ] Given a location hold *temperature* on Fridge 2 (items X 5, Y 3), lot statuses are unchanged and X, Y at Fridge 2 have available 0.
- [ ] A transfer, pick or manual issue of held quantity is refused.
- [ ] Given a hold with no balance in scope, `place` is refused.
- [ ] Given the open *temperature* hold on Fridge 2, when a transfer of 4 kg of item X is received
      into Fridge 2, the screen warns "Fridge 2 is on temperature hold"; on going on, the hold
      gets a new line X 4, X at Fridge 2 still has available 0, and the deciders are notified.

#### Out of scope
Decisions and closing (BL-STK-12); dispatch guard (BL-STK-21); recall placement (BL-STK-14).

---

### BL-STK-12 — wl-inventory: decide stock hold lines, close or cancel the hold

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-STK-11 · BL-STK-03 · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-STK-16, BL-FNB-11, BL-FNB-12, BL-MFG-08
**Review items:** X-09, 06-N10-e

#### Goal
Each held line is decided by a holder of `lots.release`: `release` (available again),
`discard` (wastage) or `return_to_supplier` (to quarantine and onto a draft return). The
effect posts once the decision is final (after its approval when one is needed). Recall lines
need a second person to release. When the last line is decided, the hold closes. A hold placed in error can be cancelled before any decision.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Stock hold* (states, rows `close`, `cancel`, line decisions), §4 *Lot* (rows `release`, `expire`, `reject`, `discard`, `unhold`), §6 (`STK.ACL.02`, `STK.ACL.05`), §7 settings

#### Terms
- **Decider:** a user with `lots.release` in the location's scope.
- **Listed reasons (`STK.ACL.05`):** hold reasons whose release needs a second person; default `recall`.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `closed` | success | yes | Every line decided and its effect posted |
| `cancelled` | neutral | yes | Placed in error; stock available again |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `close` | open | closed | system (last line decided) | every line decided, its approval given where needed and its effect posted | event `StockHoldClosed` with per-line outcomes |
| `cancel` | open | cancelled | `lots.release` | no line decided yet; reason `placed in error`; not a recall hold | held qty available again; lots back to their status before the hold (lot `unhold`, when no other hold is open on the lot) |

Each line is decided by a holder of `lots.release` in the location's scope. The effect posts
when the decision is final: at once, or only after its approval when one is needed (`release`
for a listed reason such as `recall`, `STK.ACL.05`; `discard` above the value, `STK.ACL.02`).
The line counts as decided only then; a rejected approval leaves it undecided. When the
last line is decided, the hold closes. The effects:
- `release`: the quantity is available again; the lot runs `release` when no other hold is open on it.
- `discard`: a `wastage` movement to scrap with the hold reason; the value goes to wastage (or to the recall's cost object); the lot runs `discard` when nothing of it remains.
- `return_to_supplier`: the quantity is moved to the quarantine location and added to a draft Return to supplier for the lot's supplier.

Lot:

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `release` | quarantine, on_hold | released | `lots.release`, or system (stock hold line decided `release`) | passed inspection when the item requires it; vertical `SOD` rule (e.g. `MFG.ACL.02`); from `on_hold`: no other open hold on the lot, and `STK.ACL.05` for a recall hold | event `LotReleased` |
| `expire` | released, on_hold | expired | system | expiry date reached | lot blocked; storekeeper notified; an open hold stays open |
| `reject` | quarantine, on_hold | rejected | `lots.release`, or system (Return to supplier `dispatch` takes the last quantity) | failed inspection or return recorded | remaining quantity moved to scrap or to a Return to supplier; vertical may open an NCR |
| `discard` | on_hold, expired | rejected | `lots.release`, or system (stock hold line decided `discard`) | reason given; `STK.ACL.02` above the value | `wastage` movements to scrap for the remaining quantity, with the hold or expiry reason; event `LotDiscarded` |
| `unhold` | on_hold | quarantine, released | system (stock hold `cancel`) | no other open hold on the lot | back to the status the lot had before `hold` (stored on the lot at `hold`); no inspection or approval, as the hold was placed in error |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `STK.ACL.05` | Releasing held stock for a listed hold reason (a hold line decided `release`, including the lines of a recall's hold) needs approval by a second holder of `lots.release`, not the one who decides; other holds are released by one holder of `lots.release` | `SOD` | on; listed reasons = `recall` |
| `STK.ACL.02` | Stock adjustments, write-offs and wastage above a value need approval; the recorder or counter cannot approve | `LIM`, `SOD` | AED 1,000 (06: 200; 07: 2,000) |

#### Edge cases decided
- **Approval timing:** the approval (`STK.ACL.05`, `STK.ACL.02`) is asked when the line is decided; the line's effect posts only after approval, and the line counts as decided only then. A rejected approval leaves the line undecided.
- **Lot `expired` during the hold:** the hold stays open; a `release` decision makes the quantity available but the lot stays `expired` (still blocked); `release` is not offered for lines of an expired lot; only `discard` or `return_to_supplier`.
- **`return_to_supplier` with an existing draft return for the same supplier:** added to it; otherwise a new draft.
- **Lot without a supplier (produced lot):** `return_to_supplier` not offered.
- **Cancel with lots also under another hold:** lots stay `on_hold`.
- **Status before the hold:** stored on the lot when `hold` runs (BL-STK-11 stores it); `unhold` restores it: `released` or `quarantine`. A lot held from `quarantine` goes back to `quarantine` and still needs its inspection and `release`.
- **Recall holds:** cannot be cancelled; they close with the recall (BL-STK-16).

#### Must not
- Must not let one person decide and approve a recall release.
- Must not post a discard above the value before approval.

#### Done when
- [ ] Given a *temperature* hold at a 06 outlet with line chicken 5 kg (AED 150), when the chef decides `discard`, a wastage movement of 5 kg is posted at once (below AED 200) and, being the last line, the hold is `closed` with `StockHoldClosed`.
- [ ] Given a line worth AED 300, the discard waits for approval by someone other than the chef; no wastage movement is posted and the hold stays `open` until it is approved.
- [ ] Given a `recall` line decided `release` by user A, it waits for user B; A cannot approve.
- [ ] Given a whole-lot *quality* hold decided `release` on all lines and no other hold on the lot, the lot is `released`.
- [ ] Given `return_to_supplier`, the qty moves to quarantine and a draft return for the lot's supplier lists it.
- [ ] Given a whole-lot hold on a lot that was `released`, with no decided lines, `cancel` with *placed in error* makes the stock available and the lot returns to `released` (`unhold`).
- [ ] Given the same for a lot that was `quarantine` before the hold, the lot returns to `quarantine`.

#### Out of scope
Reminders (BL-STK-13); recall close (BL-STK-16).

---

### BL-STK-13 — wl-inventory: remind and escalate open stock holds (`STK.RUL.07`)

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-STK-11 · **Blocks:** BL-FNB-12
**Review items:** X-09, 06-N10-e

#### Goal
Held food and medicines cannot wait forever. While a hold has undecided lines, the deciders
are reminded every H hours, and after 2 × H the escalation role is told.

#### Files to read
- `docs/modules/wl-inventory.md` §6 `STK.RUL.07`, §7 settings

#### Terms
- **Escalation role:** owner by default (06: kitchen manager).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default |
|---|---|---|---|---|---|
| `STK.RUL.07` | every H hours | stock hold `open` with undecided lines | remind the deciders (`lots.release` in scope); after 2 × H notify the escalation role, again every further 2 × H while lines stay undecided | H = 24 (a vertical may set fewer hours for food); escalation role = owner (06: kitchen manager, H = 2) | on |

Setting: escalation role per reason or location.

#### Edge cases decided
- **Clock:** from `place`; reminders at H, 2H, 3H…; escalation once at 2H. escalation repeats every further 2H while undecided.
- **H per reason or location:** the most specific setting wins (location over reason over default).

#### Must not
- Must not remind for closed or cancelled holds.

#### Done when
- [ ] Given a 06 hold placed 10:00 (H = 2), deciders are reminded at 12:00 and the kitchen manager is notified at 14:00.
- [ ] Given the last line decided at 13:00, no notice at 14:00.

#### Out of scope
None.

---

### BL-STK-14 — wl-inventory: open a recall and hold every lot in scope

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-STK-11 · **Blocks:** BL-STK-15, BL-SAL-15, BL-AUT-16, BL-CLN-14, BL-FNB-11, BL-FNB-13, BL-MFG-10
**Review items:** X-09, 05-N10-a, 06-N10-c, 07-N4-c

#### Goal
When a supplier, authority or internal check says lots may be unsafe, a recall is opened. In
the same step every lot in scope is held at every location, including in transit, picked
dispatches with those lots are blocked, and owner, storekeepers and procurement are told.

#### Files to read
- `docs/modules/wl-inventory.md` §3 *Recall*, §4 *Recall* (state `open`, row `open`), §4 *Stock hold*, §5.7 step 3, §6 event `RecallOpened`

#### Terms
- **Scope:** items and lot or supplier lot numbers (this issue); item + supplier + received date range for items not tracked by lot (BL-STK-17).
- **Mass hold:** one stock hold, reason `recall`, with a line per lot and location.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `open` | error | no | Lots in scope on hold; trace not yet built |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `open` | — | open | `recalls.manage` | source, reason and scope set; at least one lot in scope, or, for items not tracked by lot, the item and a received date range | number assigned; one stock hold (reason `recall`) placed on every lot in scope at every location, including in transit (mass hold); for an item not tracked by lot, the whole on-hand quantity of the item is held (stock cannot be told apart by receipt); picked dispatches with those lots blocked; owner, storekeepers and procurement notified; event `RecallOpened` |

Recall source: `supplier`, `authority`, `internal`, `complaint`. Numbering `RCL-`.

#### Edge cases decided
- **Supplier lot numbers:** matched to lots by the lot's *supplier lot* field (exact, trimmed).
- **Lot with zero balance everywhere:** stays in scope for the trace; no hold line for it.
- **Lot `expired` or `rejected`:** in scope; held quantity as BL-STK-11 (expired lots keep their status).
- **Trace starts at once:** `trace` runs right after `open` (BL-STK-15).
- **Concurrent movement:** placing the hold and reading balances run in one transaction.
- **"Picked dispatches blocked":** this issue flags them (as lot `hold` does, BL-STK-11); the refusal at `dispatch` is BL-STK-21. Dispatches are only used from Launch 07.

#### Must not
- Must not open a recall without placing the hold.
- Must not allow `cancel` of the recall hold (BL-STK-12).

#### Done when
- [ ] Given lot L9 at Branch 1 (12), Branch 2 (4) and in transit (6), when a recall *supplier* on L9 is opened, one hold *recall* with 3 lines exists, L9 is `on_hold`, owner, storekeepers and procurement are notified, `RecallOpened` is emitted.
- [ ] A picked dispatch containing L9 is flagged *lot on hold* (the `dispatch` guard that refuses it is built in BL-STK-21, Launch 07).
- [ ] Given a scope with no matching lot, `open` is refused.

#### Out of scope
Trace (BL-STK-15); items not tracked by lot (BL-STK-17).

---

### BL-STK-15 — wl-inventory: trace a recall forward to subjects and parties

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-STK-14 · BL-STK-18 · **Blocks:** BL-STK-16, BL-STK-17, BL-AUT-16, BL-CLN-14, BL-FNB-13, BL-MFG-10
**Review items:** X-09, 05-N10-a, 06-N10-c, 07-N4-c

#### Goal
The recall must find who got the goods. The trace follows the immutable movements of every
lot in scope: transfers, production (adding output lots to the scope and holding them),
issues to visits, jobs and work orders, and dispatches, and lists each subject with its party.

#### Files to read
- `docs/modules/wl-inventory.md` §3 (*Lot* genealogy, *Pending issue*), paragraph "Picking order", "Negative stock" (lot assigned on arrival), §4 *Recall* (row `trace`, forward trace table, paragraph about the party), §6 (`LotTrace`, `TracePartyResolver`, event `RecallTraced`), §4 vertical mapping row *Recall*

#### Terms
- **Affected line:** subject, party, lot, qty, date, notice status (`pending` / `sent` / `waived`).
- **`TracePartyResolver`:** vertical provider: subject → party (05 visit → patient; 06 order → customer; 07 sales order/dispatch → customer; 08 job card → vehicle owner or fleet).
- **Lot uncertain:** a pending issue whose lot was assigned on arrival.
- **Theoretical consumption:** recipe or BOM consumption posted without a scan (06 sales).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `traced` | warning | no | Affected subjects and parties listed; notices being sent; outcomes being decided |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `trace` | open, traced | traced | system (right after `open`) or `recalls.manage` (re-run) | — | forward trace built (see below); lots made from lots in scope added to the scope and held; affected lines created with notice status `pending`; event `RecallTraced` |

| Movement | What the trace lists |
|---|---|
| `transfer_out` / `transfer_in` | The lot at the destination (already held) |
| `consume` / `issue` with a `produce` from the same source | The output lots (lot genealogy through production batches and work orders), added to the scope and traced in turn |
| `issue` / `consume` to a cost object | The subject (visit, job card, work order, project, sales order) and its party |
| `dispatch` | The dispatch, its recipient and the delivery status (an undelivered delivery can still be stopped) |
| Pending issues with lots *assigned on arrival* | The subject, marked "lot uncertain" |
| Theoretical `consume` (recipe or BOM consumption without a scan, e.g. 06 sales) of the item of a lot in scope | Every subject that consumed the item at a location while that lot had a balance there (from its first receipt or transfer into the location until its balance there reached zero or was held), with its party, marked *possible* whichever lot the posting used (this row replaces the cost-object row for theoretical consumption); never marked confirmed |

Picking order: consumption that a `ConsumptionSource` derives from a recipe or BOM without a
scan (06: an F&B sale) is *theoretical*: it posts against the FEFO lot to keep lot balances
right, but the lot is not proof of what was used.

Sales without a known party (walk-in POS) are listed as a count per outlet and day; the owner
decides on a public notice.

#### Edge cases decided
- **Re-run:** idempotent: existing affected lines keep their notice status; only new ones are added.
- **Cycles in genealogy:** each lot is traced once.
- **Output lots:** added to the recall's hold (new lines on the same hold).
- **No party resolver for a subject type:** the line is listed with party *unknown* for manual handling.
- **Undelivered dispatch:** listed with its delivery status; stopping it is a manual action of the recall owner (wl-delivery `cancel` or return).

#### Must not
- Must not change any movement.

#### Done when
- [ ] Given L9 issued to visits V1 (patient P1) and V2 (P2) and consumed in batch B producing lot L20 sold to customer C, then affected lines exist for V1/P1, V2/P2 and C, and L20 is in scope and held.
- [ ] Given a pending issue of L9's item (BL-STK-18) at a location holding L9, posted later with a lot *assigned on arrival*, it is listed "lot uncertain".
- [ ] Given 15 walk-in POS sales of L20 at Outlet 1 on 2026-10-10, one line "15 sales, Outlet 1, 2026-10-10" is listed.
- [ ] Re-running `trace` creates no duplicates.
- [ ] Given lot T7 on hand at Outlet 1 from 2026-10-03 until its balance there reached zero on
      2026-10-06, and orders consuming T7's item theoretically at Outlet 1 on 2026-10-02, 2026-10-04
      and 2026-10-07, only the 2026-10-04 order is listed, marked *possible*, even when its posting
      used another lot.

#### Out of scope
Notices and close (BL-STK-16); items not tracked by lot (BL-STK-17); resolvers (verticals).

---

### BL-STK-16 — wl-inventory: send recall notices and close the recall

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-STK-12 · BL-STK-15 · Epic wl-notify: `MessageGaveUp` · **Blocks:** BL-CLN-14, BL-FNB-13
**Review items:** X-09, 05-N10-a, 06-N10-c, 07-N4-c

#### Goal
The recall owner sends notices to the affected parties in English and Arabic, follows up
notices that could not be delivered, decides every held line, and closes the recall with a
report kept as evidence.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Recall* (state `closed`, rows `send_notices`, `close`), §5.7 steps 5–6, §6 event `RecallClosed`, §7 recall templates, §10 "Recalls"
- `docs/OPEN-ITEMS.md` AU-20

#### Terms
- **Waived:** the owner decides a party need not be told, with a reason.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `closed` | success | yes | Every lot has an outcome and every affected party was told or waived |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `send_notices` | traced | traced | `recalls.manage` | notice template chosen; lines selected | notices sent through wl-notify per party (EN/AR); line status `sent`; on `MessageGaveUp` the line returns to `pending` and the recall owner gets a task |
| `close` | traced | closed | `recalls.manage` | the recall's stock hold is `closed` (every line decided and posted, `STK.ACL.05` for `release`); every affected line `sent` or `waived` with a reason | outcomes taken from the closed hold; recall report PDF (scope, trace, notices, outcomes) stored in wl-documents; event `RecallClosed` |

Templates (EN/AR) per party type: customer, patient, internal. §10: whether a client must
notify an authority is not verified; the recall report is the evidence pack `⚠ open: authority (AU-20)`.

#### Edge cases decided
- **Hold already closed:** the hold closes itself when its last line is decided (BL-STK-12); `close` of the recall only checks that it is closed.
- **Lines with party *unknown*:** must be `waived` with a reason to close.
- **Resend:** a `pending` line after give-up can be sent again (another channel).
- **Authority notice:** not built; the report is the evidence (`blocked:open-item` AU-20 only for that part).

#### Must not
- Must not close with a `pending` line.
- Must not file anything with an authority.

#### Done when
- [ ] Given 3 affected lines, when notices are sent with the *patient* template, lines are `sent`.
- [ ] Given `MessageGaveUp` for line 2, it is `pending` and the owner has a task.
- [ ] Given line 2 waived with a reason and the recall's hold `closed`, `close` succeeds, a report PDF is stored, `RecallClosed` is emitted.
- [ ] Given an undecided hold line, or a `release` still waiting for its `STK.ACL.05` approval, the hold is `open` and `close` is refused.

#### Out of scope
Authority notification (AU-20).

---

### BL-STK-17 — wl-inventory: recall items not tracked by lot

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** BL-STK-15 · **Blocks:** BL-SAL-15, BL-AUT-16
**Review items:** X-09

#### Goal
Some items have no lots (08 parts in the MVP). They are recalled by item, supplier and received
date range. The whole on-hand quantity is held because units cannot be told apart, and every
later issue or sale of the item is listed as *possible*, with a softer notice.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Recall* (row `open`, last row of the trace table, paragraph "Items not tracked by lot"), §4 vertical mapping row *Recall*

#### Terms
- **Possible line:** an affected line where the unit cannot be proven to come from the recalled receipts.

#### Spec excerpt (authoritative for this issue)

`open` guard: … or, for items not tracked by lot, the item and a received date range. Effect:
for an item not tracked by lot, the whole on-hand quantity of the item is held (stock cannot be
told apart by receipt).

| Movement | What the trace lists |
|---|---|
| `issue` / `consume` / `dispatch` of an item not tracked by lot, dated on or after the first receipt in the scope's date range | The subject and its party, marked *possible* (the unit used cannot be proven to come from the recalled receipts); the trace ends on the scope's trace end date when one is given, otherwise today (the received date range only chooses the receipts that start the trace) |

"Items not tracked by lot": the trace finds the receipts in the range and lists every later
movement of the item as *possible*; notices to *possible* lines use a softer template ("may be
affected; please bring it in for a check"), and the recall owner may waive lines with a reason.

#### Edge cases decided
- **Supplier filter:** only receipts from the scope's supplier start the window.
- **Two dates:** the received date range chooses the receipts; the trace runs from the first of those receipts to the optional trace end date of the scope, or to today when none is given.
- **No receipt in range:** `open` is refused.
- **Location-scoped:** the hold covers the item at every location (location-scoped lines, item not lot-tracked).

#### Must not
- Must not hold other items.

#### Done when
- [ ] Given part P received from supplier S on 2026-08-05 and 2026-09-01, and a scope S / P / received 2026-08-01–2026-08-31 with no trace end date, then all on-hand P is held and every issue or sale of P from 2026-08-05 to today is a *possible* line (also those after 2026-08-31).
- [ ] Given the same scope with trace end date 2026-09-15, issues and sales after 2026-09-15 are not listed.
- [ ] *Possible* lines default to the softer template.

#### Out of scope
None.

---

### BL-STK-18 — wl-inventory: record pending issues where negative stock is blocked

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:medium` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: wl-inventory stock movements · **Blocks:** BL-STK-15, BL-STK-19, BL-STK-20, BL-CLN-13, BL-MFG-05, BL-MFG-14
**Review items:** X-10 (05 MEDIUM: negative stock)

#### Goal
A clinic visit or an F&B sale must not fail because the stock count is behind. Where a location
blocks negative stock, manual movements stop at zero, but consumption posted automatically by
another module becomes a pending issue for the short quantity. It posts by itself when stock
arrives, or the storekeeper clears it.

#### Files to read
- `docs/modules/wl-inventory.md` §3 (*Location* policy, *Pending issue*), paragraph "Negative stock (decided 2026-10-15)", §5.8 step 3, §6 (`StockService`, event `StockNegative`), §7 setting

#### Terms
- **`ConsumptionSource`:** extension through which another module posts automatic consumption (F&B sale, clinic visit, work order backflush, and in 07 every material issue to a work order).
- **Pending issue:** item, location, qty, source and cost object, recorded at, cleared by, lot assigned, status (`open`, `posted`, `cleared`).

#### Spec excerpt (authoritative for this issue)

"Negative stock": `block` (default for every location type): a movement that would take the
available quantity below zero is refused. Manual issues, transfers, dispatches and returns to
supplier stop there. Consumption that another module posts automatically (an F&B sale, a
clinic visit, a work order backflush, through `ConsumptionSource`) does not block the business
event. A material issue to a work order (07) is such consumption: the work order posts it
through `ConsumptionSource`, not as a manual issue. The short quantity becomes a *pending
issue* and `STK.RUL.06` tells the storekeeper. The pending issue posts automatically, oldest
first, when stock arrives at the location (receipt, transfer or approved count), or the
storekeeper clears it with a reason (the stock was used but never on the books): an `adjust`
movement adds the pending quantity at the item's current average cost (lot entered for
lot-tracked items; value within `STK.ACL.02`), and the pending consumption then posts against
it to its source and cost object, so the balance does not change and the pending issue is
`cleared`.

A lot balance is never negative. For lot-tracked items, short consumption is always a pending
issue. When it posts, the lot that covers it is recorded as *assigned on arrival*.

`StockService`: returns the movements, or a pending issue when automatic consumption is short.
Event `StockNegative`: a balance goes below zero or a pending issue is recorded.

#### Edge cases decided
- **Partly short:** the available part posts now; only the short part becomes a pending issue.
- **Partial arrival:** pending issues post oldest first as far as the arrival covers; a partly covered one is split (posted part, open remainder).
- **Lot on arrival:** FEFO among released lots at the location.
- **Cost:** the movement posts at the average cost at posting time.
- **Clear:** reason required; `adjust` +qty at the current average cost, then the `consume` −qty to the source and cost object; net balance unchanged. The `adjust` value above `STK.ACL.02` needs approval by someone other than the storekeeper; until then nothing posts and the pending issue stays `open`; status `cleared` once posted.
- **Source cancelled** (visit voided) before posting: the pending issue is `cleared` with reason "source cancelled", no movement.

#### Must not
- Must not let a lot balance go negative.
- Must not block the visit, the sale or the work order material issue.

#### Done when
- [ ] Given 2 units at Branch 1 and a visit consuming 5, 2 post now and a pending issue for 3 is `open`; `StockNegative` is emitted.
- [ ] Given a receipt of 10 later, the pending issue posts 3 from the FEFO lot, marked *assigned on arrival*.
- [ ] A manual issue of 5 with 2 available is refused.
- [ ] Given 4 kg of a component at a 07 production store and a work order material issue of 10 kg, 4 kg post to the work order and a pending issue for 6 kg is `open` (not refused like a manual issue).
- [ ] Given a pending issue of 3 units (average cost AED 40) at a 05 location with balance 0 (`STK.ACL.02` limit AED 1,000), clearing with reason "counted, used from sample stock" posts `adjust` +3 (AED 120) and `consume` −3 to the visit's cost object, the balance stays 0, the pending issue is `cleared`, and no approval is needed.
- [ ] Given a pending issue of 30 units (AED 1,200), clearing creates an approval request; the storekeeper cannot approve; nothing posts until approved.

#### Out of scope
`allow_consumption` (BL-STK-19); alerts (BL-STK-20).

---

### BL-STK-19 — wl-inventory: allow automatic consumption below zero (`allow_consumption`)

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-STK-18 · BL-STK-11 · **Blocks:** BL-FNB-17
**Review items:** X-10

#### Goal
F&B outlet kitchens sell before deliveries are posted. At a location set to
`allow_consumption`, automatic consumption of items that are not lot-tracked may take the
balance below zero; the next receipt covers it first.

#### Files to read
- `docs/modules/wl-inventory.md` paragraph "Negative stock", §7

#### Terms
- **Purchase price variance:** account for cost differences between receipt cost and cost already used.

#### Spec excerpt (authoritative for this issue)

`allow_consumption`: automatic consumption of items that are **not** lot-tracked may take the
balance below zero at the current average cost. Every other movement is still blocked. The
next receipt covers the negative quantity first; the difference between its unit cost and the
cost already used goes to the purchase price variance. `STK.RUL.06` alerts the storekeeper while
the balance stays negative. Held quantity is never consumed: while any quantity of the item is
held at the location, automatic consumption beyond the available quantity becomes a pending
issue, as under `block`.

#### Edge cases decided
- **Lot-tracked items at such a location:** still pending issues (BL-STK-18).
- **Held quantity at such a location:** never consumed; while any quantity of the item is held there, consumption beyond the available quantity becomes a pending issue (BL-STK-11).
- **Average cost while negative:** unchanged by the negative consumption; on the covering receipt, the negative qty is valued at the receipt cost, and (receipt cost − cost used) × qty goes to the variance.

#### Must not
- Must not allow manual movements below zero.

#### Done when
- [ ] Given flour at Outlet 1 = 2 kg (avg AED 3), a sale consuming 5 kg posts and leaves −3 kg.
- [ ] A receipt of 10 kg at AED 4 leaves 7 kg; AED 3 (3 × 1) goes to the price variance.
- [ ] A manual issue at −3 kg is refused.
- [ ] Given flour (not lot-tracked) at Outlet 1 (`allow_consumption`) with 5 kg on hand, all held, a sale consuming 2 kg posts nothing from stock and records a pending issue of 2 kg; on hand stays 5, held 5.

#### Out of scope
None.

---

### BL-STK-20 — wl-inventory: alert on pending issues and negative balances (`STK.RUL.06`)

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-STK-18 · **Blocks:** BL-CLN-16, BL-MFG-05
**Review items:** X-10

#### Goal
The storekeeper must know at once when stock runs short, and the manager when it stays short.

#### Files to read
- `docs/modules/wl-inventory.md` §6 `STK.RUL.06`, §7

#### Terms
None beyond the epic.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default |
|---|---|---|---|---|---|
| `STK.RUL.06` | a pending issue is recorded, or a balance goes below zero | — | notify the location's storekeeper (at most one notification per item and location per hour); still open or negative after H hours → task to the storekeeper (count or clear) and notify the manager | H = 24 | on |

#### Edge cases decided
- **Batching:** one notification per item and location per hour at most.
- **Task:** one open task per item and location.

#### Must not
- Must not create a task when the pending issue posted before H hours.

#### Done when
- [ ] Given a pending issue at 09:00, the storekeeper is notified; at 09:00 next day, still open → task and manager notified.
- [ ] Given it posted at 15:00, nothing next day.

#### Out of scope
None.

---

### BL-STK-21 — wl-inventory: block dispatch on a failed credit re-check or a held lot

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** Base: wl-inventory Dispatch lifecycle · BL-STK-11 · BL-SAL-10 · **Blocks:** None.
**Review items:** X-14, X-09, 07-N3-b

#### Goal
Goods must not leave when the customer's credit failed since confirmation, or when a picked
lot was put on hold after picking. The dispatch stays `picked` and reserved; held lots are
re-picked.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Dispatch* rows `pick`, `dispatch`, §5.4 step 3
- `docs/modules/wl-sales.md` §4.2 paragraph "Credit re-check at dispatch"

#### Terms
- **`credit_recheck(order)`:** wl-sales `CreditChecker` guard; on failure it puts the order on `credit_hold` itself.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `pick` | draft | picked | `dispatch.manage` | released lots only, no held quantity (FEFO suggested); vertical item-flag guards (e.g. `MFG.ACL.11`) | stock reserved |
| `dispatch` | picked | dispatched | `dispatch.manage` | no picked lot on hold (re-pick otherwise); source is a sales order on account: `credit_recheck(order)` passes (wl-sales `CreditChecker`; a failure puts the order on `credit_hold` and the dispatch stays `picked`, reserved); handover chosen: own delivery (wl-delivery), courier with tracking no., or customer collection with the collector's signature (`DOC.SIG.01`) | `dispatch` movements; delivery note PDF; event `DispatchSent` → wl-delivery creates the delivery |

#### Edge cases decided
- **Re-pick:** the user removes the held lot lines and picks other released lots (FEFO suggested); the held quantity's reservation is released.
- **Order released later:** dispatch can be tried again; the guard runs again (wl-sales decides).
- **Order of guards:** lot hold first, then credit (no credit hold for a dispatch that cannot leave anyway).

#### Must not
- Must not release the reservation on a credit failure.

#### Done when
- [ ] Given a picked dispatch for an on-account order whose `credit_recheck` fails, `dispatch` is refused, the dispatch is `picked`, stock still reserved, the order `credit_hold`.
- [ ] Given a picked lot put on hold, `dispatch` is refused with "re-pick"; after re-pick from another lot it succeeds.
- [ ] `pick` of a held quantity is refused.

#### Out of scope
The credit check itself (BL-SAL-10).

---

### BL-STK-22 — wl-inventory: confirm delivery per line and take refused goods to quarantine

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: wl-inventory Dispatch lifecycle · BL-DLV-02 · BL-DLV-11 · **Blocks:** BL-SAL-13, BL-MFG-07
**Review items:** X-11, 07-N4-a

#### Goal
The dispatch records the delivered and refused quantity per line from the proof of delivery.
Refused goods stay *in return* until the driver brings them back; at run close every refused
quantity received back, whatever its refusal reason (damaged, wrong item, not ordered,
short-dated, temperature, other), is received into quarantine with that reason. The source only fulfils and bills the delivered
quantity.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Dispatch* (state `delivered`, row `confirm_delivery`, paragraph *Refused goods coming back*), §5.4 step 4, §6 (consumed `DeliveryRefusedReceived`, event `DispatchDelivered`)

#### Terms
- **Courier proof:** for a courier hand-over, the user enters the quantities from the courier's proof.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `delivered` | success | no | Proof of delivery received; refused lines *in return* until received back into quarantine. Not terminal: an approved delivery reversal reopens it, and a loss during a review moves it to `lost` |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `confirm_delivery` | dispatched | delivered | system (`DeliveryDelivered` from wl-delivery) or `dispatch.manage` with courier proof | delivered qty per line taken from the proof (`DeliveryDelivered` lines); manual: delivered and refused qty entered per line | delivered qty recorded per line; refused qty recorded as *in return* with the refusal reason (no movement yet: the goods are still with the driver); event `DispatchDelivered` with delivered and refused qty (e.g. sales order `fulfil` of delivered qty, draft invoice) |

**Refused goods coming back.** Refused quantities stay *in return* on the dispatch line until
the driver brings them back. When the run closes, `DeliveryRefusedReceived` (wl-delivery)
posts `return_in` movements to quarantine for the quantity received back, with the refusal
reason and the picked lot; a quantity reported missing is posted as a loss (`CostPoster`,
reason *missing on return*, `STK.ACL.02` above the value) and the dispatcher gets a task. For a
courier or a manual confirmation, `dispatch.manage` records the refused goods received back.
The dispatch state does not change.

#### Edge cases decided
- **Timing:** `confirm_delivery` posts no movement for refused qty; it is *in return*. `return_in` to quarantine posts on `DeliveryRefusedReceived` at run close, for the quantity received back; a missing quantity is a loss with a task to the dispatcher.
- **Courier:** `dispatch.manage` records the refused goods received back by hand.
- **Lot of refused goods:** the lot picked for the line; FEFO order if several.
- **Every refusal reason goes to quarantine:** not only damage or temperature; a wrong, unordered, excess or short-dated item is inspected in quarantine before it is released or returned (wl-delivery BL-DLV-11 uses the same rule).
- **Idempotency:** one `confirm_delivery` per delivery id.

#### Must not
- Must not return refused goods to free stock.

#### Done when
- [ ] Given a dispatch line 10 and `DeliveryDelivered` 8 delivered, 2 refused *damaged*, the dispatch is `delivered` with 2 *in return* and no movement, and `DispatchDelivered` shows 8/2.
- [ ] On `DeliveryRefusedReceived` with 2 received back, `return_in` 2 to quarantine with reason *damaged* and the picked lot is posted; the dispatch stays `delivered`.
- [ ] Given 1 received and 1 missing, `return_in` 1 is posted, the value of 1 goes to a loss (*missing on return*) and the dispatcher gets a task.
- [ ] Given a courier dispatch, a user enters 10/0 and the dispatch is `delivered`.

#### Out of scope
Sales order fulfilment (BL-SAL-13).

---

### BL-STK-23 — wl-inventory: record a dispatch discarded by wl-delivery

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: wl-inventory Dispatch lifecycle · BL-DLV-07 · **Blocks:** None.
**Review items:** X-12

#### Goal
When wl-delivery destroys perishable goods after a failed attempt, the dispatch ends
`discarded` and its value goes to wastage. No stock movement is needed because the goods
already left.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Dispatch* (state `discarded`, row `discard`)

#### Terms
- **`CostPoster`:** wl-invoicing service posting cost and wastage values.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `discarded` | error | yes | Perishable goods destroyed by wl-delivery after a failed attempt; wastage recorded |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `discard` | dispatched | discarded | system (`DeliveryDiscarded` from wl-delivery) | — | the dispatched lines are recorded as wastage with the delivery's reason and photo (no new stock movement: the goods already left); value to wastage through `CostPoster`; source notified; event `DispatchDiscarded` |

#### Edge cases decided
- **Value:** the dispatch movement's value per line.
- **Approval:** already done in wl-delivery (`STK.ACL.02` at `discard` there); not repeated.

#### Must not
- Must not post a stock movement.

#### Done when
- [ ] Given a dispatched dispatch worth AED 180, on `DeliveryDiscarded` it is `discarded`, AED 180 is posted as wastage with the reason and photo link, `DispatchDiscarded` is emitted.

#### Out of scope
None.

---

### BL-STK-24 — wl-inventory: hold a lost dispatch as a loss and reverse it when found

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: wl-inventory Dispatch lifecycle · BL-DLV-06 · **Blocks:** BL-SAL-13
**Review items:** X-11

#### Goal
When wl-delivery marks a delivery lost, its dispatch goes `lost` and its value is posted as a
loss pending the claim, also when the dispatch was already `delivered` (delivery under review). If the goods are found, the loss is reversed and the dispatch continues.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Dispatch* (states `delivered`, `lost`, rows `mark_lost`, `found`)

#### Terms
See BL-STK-23.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `lost` | error | no | wl-delivery marked the delivery lost (also after delivery, while under review); value held as a loss while the claim runs |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `mark_lost` | dispatched, delivered | lost | system (`DeliveryLost` from wl-delivery; from `delivered` when the delivery was `under_review`) | — | from `delivered`: delivered qty set back to zero, refused qty stays in return or in quarantine; value of the lost lines to a loss pending the claim (`CostPoster`); source notified; event `DispatchLost` (from `delivered`, the source reverses its fulfilment) |
| `found` | lost | dispatched | system (`DeliveryFound` from wl-delivery) | — | loss reversed; the delivery continues and ends in `confirm_delivery` or `return` |

#### Edge cases decided
- **`DeliveryLost` while the dispatch is `delivered`** (delivery was `under_review`): `mark_lost` from `delivered`; delivered qty back to zero, refused qty stays in return or in quarantine; `DispatchLost` tells the source to reverse its fulfilment (BL-SAL-13).
- **Idempotency:** per event id.

#### Must not
- Must not post stock movements.

#### Done when
- [ ] Given a dispatched dispatch AED 500, on `DeliveryLost` it is `lost` and AED 500 is a pending loss; `DispatchLost` is emitted.
- [ ] On `DeliveryFound`, it is `dispatched` and the AED 500 loss is reversed.
- [ ] Given a `delivered` dispatch (8 delivered, 2 refused) whose delivery was under review, on `DeliveryLost` it is `lost`, delivered = 0, the 8 are a pending loss, the 2 refused are unchanged, and `DispatchLost` is emitted.

#### Out of scope
Claim handling (Epic wl-ticketing).

---

### BL-STK-25 — wl-inventory: reopen a delivered dispatch on a delivery reversal

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: wl-inventory Dispatch lifecycle · BL-DLV-05 · **Blocks:** BL-SAL-13
**Review items:** X-11

#### Goal
When an approved delivery reversal says the goods were not really delivered, the dispatch goes
back to `dispatched` with delivered quantity zero, so the source can reverse its fulfilment and
the goods follow the delivery again.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Dispatch* row `reverse`

#### Terms
None beyond the epic.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reverse` | delivered | dispatched | system (`DeliveryReversed` from wl-delivery, approved there under `WRK.ACL.10`) | — | delivered qty set back to zero; refused qty stays in return or in quarantine; original proof kept, marked reversed; event `DispatchReversed` (source reverses its fulfilment) |

#### Edge cases decided
- **Next outcome:** the dispatch then ends by a new `confirm_delivery`, `return`, `discard` or `mark_lost` from the delivery.
- **Refused lines:** their quantity stays in return or in quarantine; a later `confirm_delivery` uses only the remaining (non-refused) quantity.

#### Must not
- Must not undo the refused quantities (in return, or their `return_in` movements).

#### Done when
- [ ] Given a `delivered` dispatch 8 delivered / 2 refused, on `DeliveryReversed` it is `dispatched`, delivered = 0, the 2 refused stay as they were (in return or in quarantine), `DispatchReversed` is emitted.

#### Out of scope
Sales order fulfilment reversal (BL-SAL-13).

---

### BL-STK-26 — wl-inventory: apply sales cancellation outcomes to stock

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-SAL-07 · BL-SAL-08 · BL-SAL-09 · **Blocks:** BL-FNB-02, BL-FNB-04, BL-FNB-05
**Review items:** X-14, 07-N2-a

#### Goal
When a sales order or line is cancelled after fulfilment started, wl-inventory acts on the
outcome in the event: *return to stock* puts picked goods back and cancels the dispatch;
*scrap* posts the work in progress or prepared goods as wastage with reason *cancelled order*.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Dispatch* row `cancel`, §5.2 step 3, §6 *Consumes* (wl-sales row)
- `docs/modules/wl-sales.md` §4.2 paragraph "Cancelling after fulfilment started" (outcome table)

#### Terms
- **Outcome:** see BL-SAL-07. *Finish to stock* is handled by the vertical (work order), not here.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | draft, picked | cancelled | `dispatch.manage` or system (sales order line cancelled with outcome *return to stock*) | — | reservation released; picked goods put back |

§5.2 step 3: A sales order line cancelled after fulfilment started with the outcome *scrap*
(wl-sales) posts its work in progress or prepared goods as wastage with reason *cancelled
order* and the order as reference (`STK.ACL.02` above the limit); the outcome *return to stock*
puts picked goods back and releases the reservation.

Consumes: events `SalesOrderCancelled` and `SalesOrderLineCancelled` with outcome per line
(*return to stock* cancels the dispatch, *scrap* posts wastage).

#### Edge cases decided
- **Dispatch with other lines:** when only some lines are cancelled, the dispatch is not cancelled; those lines are removed from it and their reservation released.
- **Dispatch already `dispatched`:** no `cancel` from there. the event is logged and the dispatcher gets a task; the goods follow the delivery (return or refusal).
- **Scrap of a recipe item (06):** when the recipe consumption was not yet posted, `wastage` movements of the exploded components for the cancelled qty; when already posted, no second movement, the order line is reported as wastage value.
- **`STK.ACL.02`:** the wastage approval was given with the cancellation (`SAL.ACL.05`); a wastage above the `STK.ACL.02` value still needs its own approval by someone other than the recorder.
- **Idempotency:** per event and line.

#### Must not
- Must not post wastage for *return to stock* or *finish to stock*.

#### Done when
- [ ] Given a picked dispatch for one line and `SalesOrderLineCancelled` *return to stock*, the dispatch is `cancelled` and the reservation released.
- [ ] Given a two-line dispatch and one line cancelled, the dispatch keeps the other line.
- [ ] Given *scrap* of 3 prepared meals (recipe not yet consumed) worth AED 90 at a 06 outlet, wastage movements of the components are posted with reason *cancelled order* and the order as reference.
- [ ] Given *finish to stock*, no movement.

#### Out of scope
The cancellation (BL-SAL-07 to 09); finish to stock (Epic 07 Manufacturing).

---

### BL-STK-27 — wl-inventory: refuse a new or extended purchase order to a blocked supplier

**Labels:** `module:wl-inventory` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** Base: wl-inventory Purchase order lifecycle · BL-PTY-04 (`BlockGuard`, exception) · BL-STK-08 (`revise`) · **Blocks:** BL-CON-17
**Review items:** X-18

#### Goal
While a supplier is blocked (fraud, compliance hold, sanctions), procurement must not start new
business with it. PO `issue` calls the shared `BlockGuard` and is refused unless the compliance
role approves an exception; a revision that raises the total or moves the delivery date later
counts as extending the agreement. POs already issued may still be received; paying the
supplier is stopped in wl-payments.

#### Files to read
- `docs/modules/wl-inventory.md` §4 *Purchase order* (rows `issue`, `revise`), §6 *Consumes*
  (wl-parties row)
- `docs/modules/wl-parties.md` §5.6 (block effect row "Purchase orders"), §6 (`BlockGuard`,
  `PTY.ACL.03`)

#### Terms
- **`BlockGuard::check(party, action_kind)`:** wl-parties guard; action kinds `start`, `renew`,
  `extend`, `money_out`; returns allowed, or blocked with the display reason.
- **Neutral reason:** users without `parties.block_reason.view` (`PTY.ACL.02`) see only *on hold*.
- **Exception:** an approval by the compliance role (or the finance manager when screening is
  off) that lets one blocked action run once.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `issue` | draft | issued | `purchase_orders.issue` | supplier, prices, delivery location set; supplier not blocked: `BlockGuard::check(supplier, start)` passes (`PTY.ACL.03`; refused with the neutral *on hold* reason unless an exception is approved); `STK.ACL.01` (SoD) and vertical value limits → approval request when not met | number assigned; PDF sent to supplier; event `PurchaseOrderIssued` (committed cost) |
| `revise` | issued, partially_received | same state | `purchase_orders.issue` | reason given; currency unchanged (a currency change needs `cancel` and a new PO); no line reduced below its accepted qty; `STK.ACL.01`; a higher total or a later delivery date needs `BlockGuard::check(supplier, extend)` (`PTY.ACL.03`); a higher total re-checks `STK.ACL.03` on the new total: when the new total is above a limit, an approval request for the steps the new total needs, and the current revision stays valid until it is approved; a higher total that stays below the first limit, or a total not higher, applies at once | revision number + 1; previous revision kept read-only; PDF marked "Revision N" re-sent to the supplier; committed cost adjusted; open receipts use the new lines; event `PurchaseOrderRevised` |

| Service | Purpose |
|---|---|
| `BlockGuard` | `check(party, action_kind)` with action kind `start`, `renew`, `extend` or `money_out`: allowed, or blocked with the display reason (neutral *on hold* without `PTY.ACL.02`) and the way to request an exception; every module calls it in the transitions `PTY.ACL.03` guards (e.g. PO `issue` in wl-inventory, vendor `assign` in wl-ticketing) |

| Agreement kind | Owner | Default effect | Stop (end) decided by | Verticals |
|---|---|---|---|---|
| Purchase orders (party as supplier) | `wl-inventory` | continue: issued POs may still be received; no new PO and no revision that raises the total or extends the delivery date; supplier payments stop (`PTY.ACL.03`) | procurement (`cancel` or `short_close`) | 02, 06, 07, 08 |

#### Edge cases decided
- **Order of guards:** `BlockGuard` runs before `STK.ACL.01` and the value limits, so no
  approval request for the PO itself is created for a blocked supplier.
- **PO waiting for approval when the supplier is blocked:** the final approval re-checks the
  guard and fails (BL-KRN-01); the PO stays `draft`.
- **Revision that lowers the total or only changes lines down:** allowed (not an extension).
- **Exception approved:** the PO is issued once, as the requester; a later revision needs its
  own exception.
- **Receipts against issued POs:** never refused by this guard.

#### Must not
- Must not show a restricted reason (e.g. *sanctions*) to procurement.
- Must not refuse a goods receipt or a return to a blocked supplier.

#### Done when
- [ ] Given supplier "Gulf Spare Parts" blocked with `fraud_suspected`, `issue` of PO-310
      (AED 4,000) by procurement user A is refused with *on hold* and an option to request an
      exception; PO-310 stays `draft`.
- [ ] After the finance manager (screening off) approves the exception, PO-310 is issued once.
- [ ] Given issued PO-300 to that supplier, a goods receipt against it posts normally.
- [ ] `revise` of PO-300 from AED 5,000 to AED 6,000 is refused; to AED 4,500 it applies.
- [ ] Given a `sanctions` block, procurement sees *on hold*, the MLRO sees "Blocked: sanctions".

#### Out of scope
The guard and exception (BL-PTY-04); supplier payments to a blocked party (Epic wl-payments);
the block effect review for open POs (BL-PTY-03).

---

### BL-STK-28 — wl-inventory: keep preferred and alternative suppliers per item and use them in reorder suggestions

**Labels:** `module:wl-inventory` `area:sme-gap` `priority:medium` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: wl-inventory Item catalogue and reorder suggestions (`STK.RUL.04`) · Epic wl-parties: `BlockGuard` · BL-DOC-10 (supplier certificates) · **Blocks:** BL-STK-29, BL-STK-31, BL-AUT-18, BL-CON-17, BL-FNB-17, BL-MFG-14
**Review items:** G3 (SME problem map 27, 49)

#### Goal
Procurement lists, per item, one preferred supplier and ranked alternatives with lead time and
supplier item code. The last price per item and supplier fills in from receipts. Reorder
suggestions go to the preferred supplier, or to the first usable alternative when the preferred
one is inactive or blocked, so that one supplier failing does not stop replenishment.

#### Files to read
- `docs/modules/wl-inventory.md` §3 (*Item*, *Item supplier*, *Reorder suggestion*, paragraph
  *Supplier scorecard and alternative suppliers*), §5.9 steps 1 and 3, §6 *Shared automations*
  (`STK.RUL.04` and the paragraph after the table), *Consumes* (wl-parties, wl-documents rows)
- `docs/modules/wl-parties.md` §4 (`merge`), §6 (`BlockGuard`, `PartiesMerged`)

#### Terms
- **Item supplier:** a row linking an item to a supplier party, role `preferred` or `alternative`.
- **Last price:** the unit price per base unit, excl. VAT, of the latest posted receipt line from
  that supplier for the item; else the latest issued PO line price. Kept in the document's
  currency (compared in AED only across currencies, BL-STK-29).
- **`BlockGuard::check(party, start)`:** wl-parties guard; fails while the party is blocked.
- **`ReorderPolicy`:** the base reorder rule of an item (capability `STK.ROP.01`, reorder points
  and par levels): min/max, par, or *reorder point + lead time* (08 Auto). Only the last one
  uses a lead time, and it takes the lead time of the item supplier the suggestion goes to.

#### Spec excerpt (authoritative for this issue)

| Entity | Description | Important fields |
|---|---|---|
| Item supplier | A supplier that can deliver an item (`STK.SUP.01`) | item, supplier (`Party`), role (`preferred` or `alternative`), rank among alternatives, supplier item code, purchase unit, lead time (days), minimum order qty, last price (derived), active flag, notes. At most one preferred per item |
| Reorder suggestion | Proposed purchase from stock levels | item, location, supplier, supplier role (`preferred` or `alternative`, with the reason the preferred one was skipped), alternatives shown (supplier, lead time, last price, score), suggested qty, reason, status (`open`, `converted`, `dismissed`) |

| ID | When | If | Then | Parameters (default) | Default |
|---|---|---|---|---|---|
| `STK.RUL.04` | daily at time T | items at or below reorder point / par with a preferred supplier | create reorder suggestions per supplier, or draft POs when the client opts in; notify purchaser | T = 07:00; mode = suggestions | on |

§3: Each item lists its suppliers: one `preferred` and ranked `alternative`s, each with lead
time, last price and the supplier's current score, and a warning when a certificate kept on
the supplier for the item (e.g. a *Halal certificate*, wl-documents §3) has expired; the
warning blocks nothing unless a vertical adds `blocks`. Reorder suggestions (`STK.RUL.04`)
use the preferred supplier. When it is inactive for the item or blocked
(`BlockGuard::check(supplier, start)` fails, `PTY.ACL.03`), the suggestion goes to the first
active alternative that passes the check, marked *alternative* with the reason; with none
left, the suggestion has no supplier and the purchaser picks one. Where the item's
`ReorderPolicy` adds a lead time (policy *reorder point + lead time*, used by 08 Auto), the
lead time is the chosen item supplier's; the suggested qty is rounded up to its
minimum order qty in its purchase unit (an item supplier whose purchase unit has no
conversion for the item is refused on save). Converting a purchase request line into a PO
proposes the supplier the same way and lists the alternatives. The purchaser can switch a
suggestion to another listed supplier with a reason; that changes nothing on the item, is
audited, and the scorecard lists the switches away from the preferred supplier per
purchaser and month. A blocked supplier is listed as *on hold* without the reason
(`PTY.ACL.02`).

§3: When two supplier parties are merged (`PartiesMerged`), their item supplier rows move to
the survivor; where both are on one item, the row with the stronger role stays (preferred,
else the better rank). When a supplier party is archived, its rows are deactivated; an item
left without an active preferred supplier gets a task to procurement, and nothing is
promoted automatically.

#### Edge cases decided
- **First preferred supplier:** set directly by `item_suppliers.manage` only when the item never
  had one; deactivating or removing the preferred row, or any later preferred supplier, is a
  request under BL-STK-31.
- **Same supplier twice on an item:** refused; one row per item and supplier.
- **Rank gaps:** ranks are renumbered 1..n on save; a deactivated row keeps its rank but is skipped.
- **Last price in another unit:** converted to the base unit with the item's fixed conversion.
- **Draft PO mode (06):** the draft PO goes to the chosen supplier, preferred or alternative.
- **Idempotency:** one open suggestion per item, location and run; a re-run updates it.
- **Minimum order qty:** suggested 130 m with a minimum of 500 m per roll order → 500 m, the
  rounding shown on the suggestion.
- **Switching a suggestion:** a reason is required; the switch is audited and counted on the
  scorecard per purchaser and month.
- **Party merge:** rows move to the survivor; a duplicate on the same item keeps the stronger
  role. **Party archived:** rows deactivated, task to procurement for items left without a
  preferred supplier.
- **Purchase request conversion (02, 07):** the PO line proposes the supplier as a suggestion
  would.

#### Must not
- Must not send a suggestion or draft PO to a blocked supplier.
- Must not show a block reason among the alternatives (only *on hold*).
- Must not change the item's preferred supplier when the purchaser switches a suggestion.
- Must not show last price to users without `stock.cost.view`.

#### Done when
- [ ] Given item "Cement 50 kg" with preferred "Al Noor Trading" and alternative "Gulf Build"
      (rank 1), when Al Noor is blocked and the item is below reorder point, the 07:00 run
      suggests Gulf Build, marked *alternative*, reason *preferred supplier on hold*.
- [ ] Given both suppliers blocked, the suggestion has no supplier.
- [ ] Given an 08 item with policy *reorder point + lead time*, Al Noor's lead time 5 days and
      Gulf Build's 10 days, when Al Noor is blocked the suggestion to Gulf Build uses 10 days.
- [ ] Given a receipt of 100 bags at AED 18.50 from Gulf Build, Gulf Build's last price for
      the item is AED 18.50.
- [ ] Given purchaser A switches an open suggestion from Al Noor to Gulf Build, the item's
      preferred supplier stays Al Noor.
- [ ] A second row for the same item and supplier is refused.
- [ ] Deactivating Al Noor's preferred row by purchaser A opens a BL-STK-31 request; until it
      is approved, suggestions still go to Al Noor.
- [ ] Switching a suggestion without a reason is refused; with a reason it is audited.
- [ ] Given Gulf Build's Halal certificate for the item expired yesterday, its row and the
      suggestion show a warning and the suggestion can still be converted.
- [ ] Given "Gulf Build LLC" merged into "Gulf Build", both rows on "Cement 50 kg" become one
      row for "Gulf Build" with the stronger role.

#### Out of scope
The supplier score (BL-STK-29); preferred supplier change approval (BL-STK-31).

---

### BL-STK-29 — wl-inventory: compute the supplier score and keep monthly snapshots

**Labels:** `module:wl-inventory` `area:sme-gap` `priority:medium` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-STK-01 · BL-STK-03 · BL-STK-05 · BL-STK-28 · the `SupplierIncidentSource` extension point is defined in this issue (wl-inventory §8); its 07 implementation (NCRs on incoming lots, apps/manufacturing) is a vertical task, so the checks below use a test source · **Blocks:** BL-STK-30, BL-STK-31, BL-AUT-18, BL-CON-17, BL-FNB-17, BL-MFG-14, BL-MFG-15
**Review items:** G3 (SME problem map 27, 49)

#### Goal
Procurement sees, per supplier, how reliable it is: deliveries on time, value rejected,
quality incidents, debit notes and price increases, combined into a score from 0 to 100. A
live view covers the last 3 months; on day 1 of each month a snapshot is kept so that trends
and the low-score review (BL-STK-30) use fixed numbers.

#### Files to read
- `docs/modules/wl-inventory.md` §3 (*Supplier score*, paragraph *Supplier scorecard and
  alternative suppliers*), §4 *Vertical mapping* (rows *Supplier score*), §5.9 step 2, §6
  (`SupplierScorecard`, *Consumes* wl-invoicing and Verticals rows), §7 (supplier score rows),
  §8 (`SupplierIncidentSource`), §4 *Purchase order* (row `revise`: who asked for a date change)
- `docs/modules/wl-kernel.md` §5.13 (late entry and business time)

#### Terms
- **Window:** a snapshot dated day 1 of a month covers the W = 3 calendar months ending with
  the month before (the snapshot of 2026-12-01 covers 2026-09-01 to 2026-11-30, Asia/Dubai
  dates); the live view covers the 3 months up to today. A snapshot is named by its date.
- **Last price (for price variance):** the unit price per base unit, excl. VAT, of the latest
  receipt line from the supplier for the item posted before the PO line's revision in force
  was issued (receipts against that same PO not used); else the latest PO line price issued
  before it; else none (the line has no last price and drops out).
- **Receipt:** a posted goods receipt against a PO (free replacements excluded).
- **`SupplierIncidentSource`:** extension point (built here) through which a vertical reports
  quality incidents per supplier and date, with later voids and changes of cause; 07
  apps/manufacturing will report NCRs on incoming lots through it. Until a vertical provides
  one, incidents are supplier recalls only.

#### Spec excerpt (authoritative for this issue)

| Entity | Description | Important fields |
|---|---|---|
| Supplier score | A monthly snapshot of one supplier's performance (`STK.SUP.01`) | supplier, window (the W calendar months ending with the month before the snapshot date; W = 3 by default), receipts counted, on-time %, rejected value %, quality incidents (NCRs, supplier recalls), debit notes (count, value), price variance %, score 0–100, band (`good`, `watch`, `below_threshold`, `not_enough_data`), computed at. Derived and never edited |

Inputs per supplier in the window (§3; window = the W calendar months ending with the month
before the snapshot date, W = 3; one score per supplier across the deployment, filtered by
the PO's legal entity in the report):

1. **On time:** each goods receipt against a PO is on time when its business date (the day
   the goods arrived: the posting date, or the business time of a late entry, wl-kernel
   §5.13) is on or before the expected date plus the grace days (default 1, Asia/Dubai
   dates). The expected date is the one of the first issued revision; a later date set by
   `revise` counts only when the revision records *requested by us*. A receipt that waited
   for over-receipt approval, or replaces a reversed one, keeps the day the goods arrived. A
   PO delivered in three receipts counts three times. Free replacements are not counted.
2. **Rejected value:** rejected qty at receipt (reason `excess` excluded) at the PO price, plus
   Return to supplier lines from stock (from a stock hold, recall or NCR) at their unit cost,
   counted when the return is `dispatched` (a `cancelled` draft never counts), divided by the
   value received at PO price.
3. **Quality incidents:** NCRs from `SupplierIncidentSource` whose cause is the supplier and
   that are not voided, plus recalls with source `supplier` whose lots came from the
   supplier. An NCR voided or closed as *not the supplier's cause* drops out of the live view.
4. **Debit notes:** count and value (`SupplierDebitNoteIssued`; `SupplierDebitNoteCancelled`
   removes it); shown, not scored.
5. **Price variance:** for PO lines issued in the window that have a last price,
   Σ (PO price − last price) × qty ÷ Σ last price × qty, with the PO price of the revision in
   force; only an increase lowers the score. Compared in the PO's currency when both are in
   the same currency; otherwise both in AED, each at its own document's rate.

§3: A snapshot is never recomputed. A record changed after it changes the live view only;
procurement may add a review note to a snapshot (text, by, at), shown on the scorecard and
in the `STK.RUL.09` task. When the day-1 run did not happen, the next run builds every
missing month, once per supplier and month, and `STK.RUL.09` fires once per new snapshot.

Score = W1 × on-time % + W2 × (1 − min(1, rejected % ÷ R)) + W3 × max(0, 1 − incidents ÷ I)
\+ W4 × (1 − min(1, max(0, price variance %) ÷ P)), rounded to a whole number. Defaults:
W1 = 40, W2 = 30, W3 = 15, W4 = 15; R = 10%, I = 3, P = 10%. A component without data drops
out and its weight is spread over the others in proportion. With fewer than N receipts in the
window (N = 3) the band is `not_enough_data` and no score is shown. Bands: `good` at 85 or
more, `watch` from the threshold to 84, `below_threshold` under the threshold T (70).

| Setting | Default |
|---|---|
| Supplier score weights W1–W4 (on time, rejected value, incidents, price) and caps R, I, P (§3) | 40, 30, 15, 15; 10%, 3, 10% |
| Supplier score grace days, window, minimum receipts, threshold (`STK.RUL.09`) | 1 day, 3 months, 3, 70 |

#### Edge cases decided
- **Rounding:** percentages kept to 4 decimals in the computation; the score is rounded half up.
- **Weight spread:** with no price data, W1–W3 become 40/85, 30/85, 15/85 of 100.
- **Late events:** a debit note or NCR recorded after a snapshot changes the live view only; a
  snapshot is never recomputed.
- **Reversed receipt:** not counted (live view); already in a snapshot, it stays there.
- **02, 06, 08 (no NCR source):** incidents are supplier recalls only.
- **Foreign currency:** values in AED at each document's rate for totals; price variance in
  the PO's currency when the last price is in the same currency (a USD price unchanged at
  USD 10 while the rate moves is 0% variance).
- **Snapshot run:** day 1 at 06:00 Asia/Dubai; one snapshot per supplier and month (idempotent);
  a missed month is built by the next run.
- **Date pushed by the supplier:** a revision moving the date from 2026-11-10 to 2026-11-20
  recorded *by the supplier* leaves 2026-11-10 as the expected date for the score.
- **Review note:** added to a snapshot, never changes its numbers.

#### Must not
- Must not edit a snapshot.
- Must not score debit notes on top of rejected value.
- Must not show a score for `not_enough_data`.
- Must not show values to users without `stock.cost.view`; the score and percentages need `supplier_scores.view`.

#### Done when
- [ ] Given 10 receipts in the window, 8 on time, rejected value 2%, 0 incidents, price
      variance +5%, the score is 40 × 0.8 + 30 × 0.8 + 15 × 1 + 15 × 0.5 = 78.5 → 79, band `watch`.
- [ ] Given a receipt posted on 2026-11-11 for a PO expected 2026-11-10, it is on time (grace 1).
- [ ] Given rejected qty with reason `excess` only, rejected value is 0%.
- [ ] Given 2 receipts in the window, the band is `not_enough_data`.
- [ ] Given an incident reported through a test `SupplierIncidentSource` (standing in for a 07
      NCR on a lot from the supplier, cause *supplier*), incidents = 1.
- [ ] Running the snapshot twice on 2026-12-01 keeps one snapshot per supplier.
- [ ] Given goods that arrived on 2026-11-10 entered late on 2026-11-13 with the late-entry
      flag (business time 2026-11-10), the receipt is on time for a PO expected 2026-11-10.
- [ ] Given a PO revised from 2026-11-10 to 2026-11-20 *by the supplier* and received on
      2026-11-18, the receipt is late; *requested by us*, it is on time.
- [ ] Given that incident later reported voided by the source, incidents in the live view go
      back to 0.
- [ ] Given a USD supplier at USD 10 on both the last receipt and the new PO, price variance is
      0% although the AED rate changed.
- [ ] Given no run on 2026-12-01 (scheduler down), the run on 2026-12-02 builds the snapshots
      dated 2026-12-01 (window 2026-09-01 to 2026-11-30) once.
- [ ] Given a receipt posted on 2026-12-01, it is not in the snapshot of 2026-12-01 and counts
      in the snapshot of 2027-01-01.
- [ ] Given receipts from the supplier at AED 10 on 2026-10-05 and AED 11 on 2026-10-20 and a PO
      line issued on 2026-10-15 at AED 12, its last price is AED 10 (the receipt before the
      issue) and its variance is +20%.

#### Out of scope
The review task (BL-STK-30); the scorecard dashboard layout (base reporting).

---

### BL-STK-30 — wl-inventory: ask procurement to review a supplier below the threshold (`STK.RUL.09`)

**Labels:** `module:wl-inventory` `area:sme-gap` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-STK-29 · BL-PTY-01 (block, for *request block*) · **Blocks:** BL-AUT-18, BL-CON-17, BL-FNB-17, BL-MFG-14
**Review items:** G3 (SME problem map 27, 49)

#### Goal
When a supplier's monthly snapshot falls below the threshold, procurement gets one review
task with the score breakdown, the items where the supplier is preferred and their
alternatives, and closes it with a decision.

#### Files to read
- `docs/modules/wl-inventory.md` §5.9 steps 4–5, §6 *Shared automations* (`STK.RUL.09`),
  *Events emitted* (`SupplierScoreBelowThreshold`)

#### Terms
- **Review decision** (each one closes the task and is kept on the snapshot):
  - *keep*: nothing else changes;
  - *watch*: the supplier shows *watched* on the scorecard and its item supplier rows until a
    later snapshot is at or above the threshold;
  - *change preferred*: one BL-STK-31 request per item selected;
  - *request block*: a task to the holders of `parties.block` (wl-parties §4 `block`,
    BL-PTY-01) with the review attached; nothing is blocked automatically.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default |
|---|---|---|---|---|---|
| `STK.RUL.09` | monthly, day 1 at time T | a supplier's new snapshot score is under the threshold (band `below_threshold`; `not_enough_data` never fires) | one review task to procurement per supplier, with the score breakdown, the items where it is preferred and their alternatives; notify the procurement manager; while that task is open a new snapshot under the threshold updates it instead of creating another; event `SupplierScoreBelowThreshold` | T = 06:00; threshold = 70; minimum receipts N = 3 | on |

| Event | When | Payload |
|---|---|---|
| `SupplierScoreBelowThreshold` | Monthly snapshot under the threshold (`STK.RUL.09`) | supplier, window, score, components |

§5.9 step 5: Procurement closes the task with a decision: *keep*, *watch*, *change preferred*
(a request per item, approved by a second person, `STK.ACL.07`) or *request block* (a task
to the holders of `parties.block` to decide on wl-parties `block`; nothing is blocked
automatically). Every decision closes the task and is kept on the snapshot; *keep* changes
nothing; *watch* shows the supplier as *watched* on the scorecard and on item supplier rows
until a later snapshot is at or above the threshold. A later snapshot under the threshold
opens a new task once the previous one is closed.

#### Edge cases decided
- **Order:** the rule runs after the snapshot run of the same day.
- **Closing:** a decision is required; *change preferred* opens one request per item selected.
- **Supplier back above the threshold** while the task is open: the task stays; the new score is added to it.
- **Blocked supplier:** the rule still fires (the review may confirm the block).
- **Review notes:** notes on the snapshot (e.g. the supplier contests an NCR) are shown in the
  task; adding one does not close it.
- **Missed run:** a snapshot built late by the catch-up run fires the rule once.

#### Must not
- Must not create a second open review task for the same supplier.
- Must not block a supplier by itself (*request block* only asks).
- Must not change any item supplier by itself.

#### Done when
- [ ] Given a snapshot of 64 on 2026-12-01, one review task for procurement and a message to
      the procurement manager are created.
- [ ] Given the task still open and a snapshot of 60 on 2027-01-01, the same task is updated.
- [ ] Given `not_enough_data`, nothing is created.
- [ ] Closing with *change preferred* for two items creates two BL-STK-31 requests.
- [ ] Closing with *keep* closes the task and changes nothing; given a snapshot of 62 on
      2027-02-01, a new review task is created.
- [ ] Closing with *watch* shows the supplier *watched*; given a snapshot of 75 on 2027-02-01,
      the mark is removed and no task is created.
- [ ] Closing with *request block* creates a task for the `parties.block` holders; the
      supplier stays `active` until one of them runs `block`.

#### Out of scope
The `block` transition and its effects (BL-PTY-01).

---

### BL-STK-31 — wl-inventory: approve a change of an item's preferred supplier (`STK.ACL.07`)

**Labels:** `module:wl-inventory` `area:sme-gap` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-STK-28 · BL-STK-29 (scores shown on the request) · Epic wl-kernel: approval returned · **Blocks:** BL-AUT-18, BL-CON-17, BL-FNB-17, BL-MFG-14
**Review items:** G3 (SME problem map 27, 49)

#### Goal
Moving an item's regular business to another supplier is a common way to favour a related
supplier. Making another supplier preferred therefore needs a second person, who sees both
suppliers' scores, lead times and last prices.

#### Files to read
- `docs/modules/wl-inventory.md` §3 paragraph *Supplier scorecard and alternative suppliers*
  (paragraph *Changing who is preferred*), §6 *Shared access rules* (`STK.ACL.07`), *Events emitted*
  (`PreferredSupplierChanged`), *Permissions*, §7 (*Second person for a preferred supplier
  change*)

#### Terms
- **`item_suppliers.approve`:** permission to approve the change.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Parameter (default) | Generalises |
|---|---|---|---|---|
| `STK.ACL.07` | Changing who is the preferred supplier of an item (making another supplier preferred, deactivating or removing the preferred row, or setting a preferred supplier on an item that had one before) needs approval by a holder of `item_suppliers.approve` who is not the requester; the request shows both suppliers' scores, lead times and last prices. Only the first preferred supplier of an item that never had one, alternatives (add, rank, deactivate; audited) and switching a single reorder suggestion (reason required; audited) need no approval | `SOD` | on | — |

§3: One request per item may be pending. A rejected or withdrawn request changes nothing and
the requester is told; a returned one is edited and resubmitted; the system cancels a pending
request when the item is archived, either row is deactivated or another request for the item
is approved. On approval `BlockGuard` is checked again for the new supplier, the new supplier
becomes `preferred` and the old one `alternative` rank 1 (when the request removes the
preferred row without a successor, the item has no preferred supplier and `STK.RUL.04` uses
the alternatives).

| Event | When | Payload |
|---|---|---|
| `PreferredSupplierChanged` | Another item supplier made preferred (after `STK.ACL.07`) | item, old and new supplier, requester, approver |

#### Edge cases decided
- **On approval:** the new supplier becomes `preferred`; the old one becomes `alternative` rank 1.
- **Pending request:** one per item; a second is refused until the first ends.
- **New supplier blocked at approval time:** approval fails (`BlockGuard`), the request is returned.
- **Setting off:** when the client turns off the §7 setting *Second person for a preferred
  supplier change (`STK.ACL.07`)* (default on), a change of preferred supplier applies at
  once without a request, audited with the user, and `PreferredSupplierChanged` is emitted
  with the requester as approver; pending requests stay pending until decided.
- **Removing or deactivating the preferred row:** a request like any other; until approved the
  row stays preferred and active.
- **Delete and re-add:** an item that ever had a preferred supplier never gets a "first" one
  without approval.
- **Rejected / withdrawn / returned:** nothing changes, requester told; returned requests are
  edited and resubmitted (Epic wl-kernel: approval returned).
- **Pending request cancelled by the system:** item archived, either row deactivated, or another
  request for the item approved.

#### Must not
- Must not let the requester approve their own request.
- Must not require approval for the first preferred supplier of an item that never had one, or for
  alternatives.

#### Done when
- [ ] Given purchaser A requests Gulf Build as preferred for "Cement 50 kg", A cannot approve it;
      manager B approves and Gulf Build is preferred, Al Noor alternative rank 1, event emitted.
- [ ] The request screen shows both scores, lead times and last prices.
- [ ] Adding a new alternative needs no approval.
- [ ] Purchaser A deactivates the preferred row of "Cement 50 kg": a request is created, the row
      stays preferred until manager B approves.
- [ ] Given A removed the preferred row (approved) and later adds Gulf Build as preferred, a
      request is needed (the item had a preferred supplier before).
- [ ] Given B rejects A's request, the item is unchanged and A is told.
- [ ] Given the setting *Second person for a preferred supplier change* off, A makes Gulf
      Build preferred directly; the change is audited and no request is created.
- [ ] Given Gulf Build with no snapshot yet (`not_enough_data`), the request screen shows
      *not enough data* for its score.

#### Out of scope
The approval engine (Epic wl-kernel).
