# fnb

The vertical app for cloud kitchens and local F&B chains (06 F&B). The order
is a sales order (`wl-sales`); this app is the kitchen's fulfilment of it, and
every status change goes through a state machine that leaves a history row.

## What is here

- **Order** (`order`, `order_machine`): the fulfilment overlay of a sales
  order, in the nine states of 06 §7.1: `placed`, `accepted`, `preparing`,
  `ready`, `dispatched`, `delivery_failed`, `completed`, `rejected` and
  `cancelled`. It is made from the sales order's events (`placing`):
  `SalesOrderPlaced` (a direct order waiting for the outlet),
  `SalesOrderConfirmed` (accepted, or an aggregator or phone order, which
  starts accepted) and `SalesOrderRejected`. The Accept button asks the sales
  order desk, so the two never disagree.
- **Kitchen ticket** (`ticket`, `kitchen`): one ticket per item line of an
  accepted order. Starting the first starts the order; bumping the last makes
  it `ready`. `/kitchen` is the board (no customer details, `FNB.ACL.01`).
- **Delivery** (`fulfilment`): a `ready` order for delivery asks wl-delivery
  for a perishable delivery within the brand's promise; the run's events move
  the order (`DeliveryOutForDelivery` dispatches it, `DeliveryDelivered`
  completes it and fulfils the sales order).
- **Failed delivery** (`failed`, BL-FNB-01): `DeliveryAttemptFailed` runs
  `fail_delivery` once, with the failure reason mapped from wl-delivery's, the
  side (customer or outlet) and the proposed responsible party. 06 adds
  *rider issue* and the two outlet-side causes of *refused* to the list of
  failure reasons the rider picks from (`failed::FAILURE_REASONS`, installed
  by the host). Cooked food gets one attempt; there is no reattempt and no
  manual failure.
- **Closing** (`close`, `refunds`, BL-FNB-02): `DeliveryReturned` or
  `DeliveryDiscarded` runs `close_failed`: the order is `cancelled` with the
  refund decision of `FNB.ACL.17` (full refund for the outlet's side, no refund
  for the customer's, not applicable on cash), the sales order is cancelled
  with the food scrapped (once, as wastage *failed delivery*), and the refund
  is asked of wl-payments. A cashier asks for a manager-approved refund within
  7 days; the kitchen manager approves it, never the requester. The
  customer-side default is open item L-33 and a setting.
- **Cash on delivery** (`cod`, BL-FNB-03): `FNB.RUL.14` turns cash on
  delivery off for a customer after 2 customer-side failures of cash orders in
  90 days; the kitchen manager turns it back on (`/customers/cod`); the
  checkout (`/checkout/payment`) then offers only online payment.
- **Cancelling** (`cancel`, BL-FNB-04): the customer cancels a `placed` order
  from `/my/orders/{id}` (full refund, nothing in the kitchen; refused once the
  outlet accepted it); staff with `orders.cancel` cancel an order with a reason
  and the responsible party (business, aggregator, customer; *entered in
  error* is always the business's). The cancel is the sales order's: a paid
  order above AED 50, or a started line, waits for a manager, each started line
  needs an outcome (return to stock, scrap), and the order follows
  `SalesOrderCancelled` with `cancel_system`, its open tickets voided. An
  aggregator order cancelled with the aggregator responsible issues no refund
  and expects compensation. A ticket that starts withdraws a waiting request.
  There is no `void` on an order.
- **Changing an order** (`changes`, BL-FNB-05): the cashier amends a `placed`
  or `accepted` direct order after calling the customer (the "customer agreed"
  tick and the time of the call; the price difference is refunded or charged by
  payment link; aggregator orders are never amended here) and cancels a line
  when an item runs out (a started line needs an outcome and a manager; the
  last open line is refused; an aggregator's cancelled line is flagged for the
  payout, `ExpectedRefund`). The kitchen tickets follow the sales order's
  events at once.
- **The aggregator's rider** (`pickup`, BL-FNB-06): `FNB.RUL.16` tells the
  kitchen managers to chase the aggregator 15 minutes after an aggregator
  order is `ready`, and opens a task at 45 for the manager to record *picked
  up late* (the order completes) or *cancelled by the aggregator* (the order
  is cancelled with the aggregator responsible, the food as wastage, no refund,
  compensation expected, no second approval).
- **A reversed delivery** (`reversal`, BL-FNB-07): on `DeliveryReversed` a
  completed order is refunded in full by a sales return of kind *no goods*
  naming the rider, resolved at once, by a refund or (cash on delivery nobody
  handed over) by credit note only; the order stays `completed`.
- **Catalogue** (`catalogue`): brands, outlets and menu items with recipes, as
  far as the orders need them. `progress` gives the shared modules the
  kitchen's `FulfilmentProgress` and `SaleLineStockSource`.

## Permissions

`orders.view`, `.manage`, `.cancel`, `.amend` and `.refund_request`,
`kds.work` and `customers.cod` (`permissions`). The host maps them to the roles of 06 §3.1:
the kitchen manager, the cashier, the line cook, the area manager, the
operations director and the rider; the company admin holds everything.

## Tests

`tests/` boots the app with the real wl-delivery, wl-sales, wl-payments,
wl-inventory and wl-parties, a recording notifier and a ticket desk.
`tests/support` holds the fixtures (an order placed through the sales module,
a run planned, dispatched and failed through the real routes).
