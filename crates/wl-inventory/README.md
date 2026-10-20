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
| The supplier bill match reads the ordered and accepted quantity of an order's lines through the kernel's `PurchaseMatchSource`; rejected goods are not in it | §6 | `match_source` |

Not built yet: `issue`, `revise`, `short_close`, `close` and `cancel` of a purchase order, the system `dispatch` of a door-refused return
(BL-STK-03; until then it stays `draft`), receipts against a return (BL-STK-06), the screens of the field PWA and the other entities.

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
