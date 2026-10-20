# wl-inventory

Inventory for the white-label platform ([`docs/modules/wl-inventory.md`](../../docs/modules/wl-inventory.md)):
items, locations, purchase orders, goods receipts and the stock they put in
place. Quantities are kept in thousandths of the item's base unit, money in fils.

```
# use renox::prelude::*;
# let _ =
App::new().module(wl_inventory::WlInventory::new())
# ;
```

What is here so far:

| Part | Spec | Module |
|---|---|---|
| Permissions (`items.*`, `stock.view`, `purchase_orders.*`, `goods_receipts.*`, `supplier_returns.manage`) | §6 | `permissions` |
| The base entities receiving needs: `Item` (base unit, lot-tracked flag, moving weighted average cost), `Location` (type, scope, negative stock policy), `PurchaseOrder` with its lines and the machine's states, `Lot` and its machine's states, `StockMovement` (immutable; the balance of an item at a location is derived from the movements) | §3, §4 | `item`, `location`, `purchase_order`, `lot`, `movement` |
| Settings: the over-receipt tolerance (5%) and the rejection reasons, with the ones that need a reading or a photo | §7 | `config` |
| Posting a goods receipt: the storekeeper enters received, accepted and rejected quantity per line with a reason and what became of the rejected goods (*refused at the door* or *kept for collection*), and `post` (`goods_receipts.post` in the location's scope) puts only the accepted quantity into stock: a `receive` movement and a lot per accepted line, the item's average cost, the purchase order's `receive` (rejected quantity stays outstanding), and a draft Return to supplier for the door-refused lines and one for the lines kept for collection; `GoodsReceived` carries accepted and rejected quantity | §4 *Goods receipt*, §5.1, §5.6 | `goods_receipt`, `receipt_post`, `supplier_return`, `events` |
| Over-receipt (`STK.ACL.04`): an accepted quantity above the ordered one by more than the tolerance turns `post` into an approval request (`PLT.APR.01`) for a holder of `purchase_orders.issue` in the location's scope, never the poster; the receipt stays `draft` until `post_approved` | §4 *Goods receipt* | `receipt_post` |
| Reversing a posted goods receipt (`goods_receipts.reverse` in the location's scope): all or nothing, refused when a Return to supplier made from it is `dispatched` or `credited`, when its purchase order is `closed` (billed), or when part of the accepted quantity (of a line or lot) is no longer at the location or the lot is on hold, naming the line and the missing quantity; otherwise `adjust` movements take the accepted quantity out, the order's outstanding quantity goes back up, and the draft returns from the receipt are cancelled | §4 *Goods receipt* | `receipt_reverse` |
| Dispatching a Return to supplier (`supplier_returns.manage`): with a collection proof (the driver's signature, a courier number or the goods receipt), the lines taken from stock leave with `return_out` movements at the source receipt's cost (the difference to the average cost is kept on the line as the price variance), a `replace` line from stock gives its quantity back to its order line, a held lot with nothing left is `rejected`, and a return note PDF is kept; above AED 5,000 (`return_approval_fils`, `STK.ACL.06`) `dispatch` becomes an approval request for a second holder of the permission, never the requester. The goods refused at the door are dispatched by the system as the receipt posts (no movement, no approval); `SupplierReturnDispatched` carries the lines for the debit note. A return of stock is made with `supplier_return::create_from_stock` | §4 *Return to supplier*, §5.6 | `return_dispatch`, `return_note`, `supplier_return`, `events` |
| Cancelling and crediting a Return to supplier (`supplier_returns.manage`): `cancel` of a `draft` return needs a reason, releases the stock it held aside (`supplier_return::reserved_milli`) and, for goods rejected at receipt and kept for collection, asks whether they are received again (a draft goods receipt on the order) or discarded; the system withdraws the draft returns of a reversed receipt. `credit` ends a `dispatched` return: the system's `credit_covered` when the supplier's debit notes (`SupplierDebitNoteIssued`, which now names the debited lines) cover every `credit` line and replacement receipts cover every `replace` line, coverage being kept per line; or by hand with a reason (*refund received* or *supplier refused*), a refused credit posting the uncovered value as wastage through the `CostPoster` and, above AED 1,000 (`wastage_approval_fils`, `STK.ACL.02`), asking a second holder of the permission first. `SupplierDebitNoteCancelled` leaves the return `dispatched` (its cover is taken off) and gives procurement a task; `SupplierReturnCredited` follows each credit | §4 *Return to supplier*, §6 | `return_credit`, `supplier_return`, `events` |
| A free replacement against a dispatched Return to supplier (`goods_receipt::create_replacement_draft`, then the same `post`): no purchase order, so nothing is received on an order or left to match in wl-invoicing; the lines are `replace` lines of the return that have no order line, or whose order is `closed`, and what is received of a line never goes above what is still to be replaced. The accepted quantity comes in at zero cost (the item's average cost is diluted), is counted on the return line, goods rejected again go on a new return, and the return is `credited` when every line is covered; such a receipt cannot be reversed | §4 *Goods receipt* `post`, §5.6 | `goods_receipt`, `receipt_post`, `events` |
| `STK.RUL.08`, chasing a return that stays `dispatched` (`chase`): daily at 07:00 Asia/Dubai, a return `dispatched` for more than N days (`return_chase_days`, default 30, calendar days from the dispatch date) notifies procurement and finance and gives procurement one task to chase the supplier (one task per return, resolved when it is credited); the notice repeats every N days from the last one (days N + 1, 2N + 1...) until the return leaves `dispatched` | §6 | `chase`, `config` |
| Reopening a received purchase order: the system's `reopen` (`received` to `partially_received`) runs when a `replace` return from stock is dispatched or a receipt is reversed; the returned quantity is outstanding again and `committed_value` (outstanding at the order's prices) is back; a line restores once, a `closed` order is never reopened | §4 *Purchase order* | `purchase_order` |
| The supplier bill match reads the ordered and accepted quantity of an order's lines through the kernel's `PurchaseMatchSource`; rejected goods are not in it | §6 | `match_source` |

Not built yet: `issue`, `revise`, `short_close`, `close` and `cancel` of a purchase order, the screens of the field PWA and the other entities.

The settings are client defaults set in code (§7); the host changes one with a builder call:

```
# let _ =
wl_inventory::WlInventory::new()
    .over_receipt_tolerance_bps(1_000)
    .rejection_reason("temperature", true)
# ;
```

The module owns `src/texts.rs`, the English texts (`wl_inventory.*`). The app changes or
translates them in `resources/lang/{locale}.json`.
