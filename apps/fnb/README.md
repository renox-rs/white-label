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
- **Aggregator payout statements** (`payout`, `payout_csv`, `payout_machine`,
  `payouts`, `payout_pages`; BL-FNB-09, BL-FNB-10, `FNB.REC.01`): the
  accountant imports the aggregator's CSV with the column preset of that
  aggregator (Excel is saved as CSV first). A statement with the same
  aggregator and reference (trimmed, without case) as one that is not void is
  refused and the refusal names the existing one. Lines are matched to orders
  by the aggregator's own number (or to an order entered late by the cashier).
  `match` books the net payout as the receivable and turns each refund line
  into a sales return of kind *no goods*, resolved *refunded by the channel*
  (no refund payment, no credit note, no approval), with the reason mapped to a
  category and a party; an unmapped reason is set by the accountant first.
  `dispute` / `resolve` raise a follow-up task and let a refund the aggregator
  takes back run `reverse`. `void` is the kernel's (`PLT.ACL.09`): a matched or
  disputed statement needs a second holder's approval, its receivable and
  matching are reversed, its returns reversed with *statement voided*, the
  number is kept, the reference can be imported again. A settled statement
  cannot be voided. `FNB.RUL.10` opens a task for the accountant when the
  unmatched lines of an import exceed AED 100.
- **Reports and rider faults** (`reports`, `report_pages`; BL-FNB-08): a *no
  goods* return of an order is kept as a refund fact (category, party, net and
  gross) for the refunds report, which includes the aggregators' refunds; the
  failed deliveries report groups failures by reason. A return or a closed
  failed delivery blamed on the rider writes a *rider fault* for the rider of
  the last run, for the kitchen manager and HR only, with no pay effect.
- **Production batches** (`batch`, `batch_machine`, `batches`, `batch_pages`;
  BL-FNB-11): the central kitchen plans a batch of a recipe (`/batches`), the
  chef starts it picking one lot per ingredient (the earliest expiry is
  proposed; a lot on hold, expired, of another item or short is refused, and the
  lots used are kept as the batch's genealogy) and finishes it with the yield
  and an expiry not later than the earliest expiry of its ingredients. The
  stock module consumes the lots and makes the output lot (kernel
  `BatchStock`); an ingredient lot on hold at that moment holds the output lot at
  once. A held lot (`LotHeld`) puts its batch `on_hold`, `LotReleased` brings it
  back and `LotDiscarded` discards it with the lot's own wastage linked, never a
  second movement; a hold on a location (temperature) leaves it `produced`. A
  chef discards a `produced` batch (wastage posted once; above AED 200 a second
  holder of `stock.approve` approves); a batch on hold is discarded only through
  its stock hold line. Cancelling a planned batch deletes it.
- **Cold units and temperature holds** (`cold`, `cold_pages`; BL-FNB-12): the
  kitchen manager sets up each fridge, freezer or hot-holding unit with a stock
  location (required) and a range (`/cold-units`); cooks record readings.
  `FNB.RUL.05` runs as a compliance-critical kernel rule on a reading out of
  range: one stock hold (reason `temperature`, the reading as reference) on the
  stock in the unit through `StockHoldService`, one equipment task for the
  kitchen manager, and a notice to the chef and the kitchen manager; later
  readings join the hold and stock moved in joins it as new lines (the stock
  module warns on its transfer screen). The chef decides each line (release,
  discard, return to the supplier) recording the reading and the minutes out of
  range; a temperature hold is released by one person, a discard above AED 200
  waits for the kitchen manager. `STK.RUL.07` is the stock module's reminder
  (host: 2 hours, then the kitchen manager after 4). A failed run is retried and
  alerts the owner.
- **Recall trace** (`recall`, `recall_pages`, `consumption`; BL-FNB-13): with
  the setting `recipe_consumption` on (the 06 client) a completed order
  consumes its recipe from the outlet's stock as theoretical consumption, so the
  trace of a recalled ingredient lot lists the orders of the outlets that had it
  (or a batch lot made from it) on hand as *possible*, never *confirmed*. The
  app is the `TracePartyResolver`: an order is its customer, or nobody known for
  an aggregator's order and a counter sale. `/recalls/{id}/orders` (the stock
  module's recall page links to it) lists one line per direct-order customer
  with their possible orders and one line per outlet and day for the rest; the
  head chef or the owner tells the customers (one notice each, English and
  Arabic, listing all their orders; a closed account is waived), and the owner
  decides the public notice and the aggregators (a task opens when the trace
  finds such sales; the decision waives or tells those lines). The authority's
  notification is open item AU-20, not built (setting `authority_notice_task`
  gives the owner a task to do it by hand); the recall report is exported from
  the recall page.
- **Catalogue** (`catalogue`): brands, outlets and menu items with recipes, as
  far as the orders need them. `progress` gives the shared modules the
  kitchen's `FulfilmentProgress` and `SaleLineStockSource`.

## Permissions

`orders.view`, `.manage`, `.cancel`, `.amend` and `.refund_request`,
`kds.work`, `customers.cod`, `reconciliation.view` and `.manage`,
`orders.late_entry`, `fnb_reports.view`, `rider_faults.view`, `production.view`
and `.manage`, `temperature.view` and `.log`, `cold_units.manage`,
`fnb_recalls.view` and `.decide` (`permissions`). The host maps them to the roles of 06 §3.1:
the kitchen manager, the cashier, the line cook, the area manager, the
operations director and the rider; the company admin holds everything.

## Tests

`tests/` boots the app with the real wl-delivery, wl-sales, wl-payments,
wl-inventory and wl-parties, a recording notifier and a ticket desk.
`tests/support` holds the fixtures (an order placed through the sales module,
a run planned, dispatched and failed through the real routes).
