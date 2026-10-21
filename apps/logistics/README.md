# logistics

The vertical app for courier and freight companies (03 Logistics). This
first part is the **Shipment lifecycle**, with the two negative flows that
need only it: voiding a duplicate booking and cancelling a shipment after
pickup. The COD remittance and the freight job come with their own issues.

## What is here

- **Shipment** (`shipment`, `shipment_machine`): a consignment of one or more
  parcels for one recipient, booked for a client party. It has a number from
  the `SHP-0000` series that it keeps for ever, an optional client reference
  that is unique per client across all states, and the fourteen states of
  03 §7.1. The base transitions are `pick_up`, `receive_at_hub`, `dispatch`,
  `deliver`, `fail_attempt`, `cancel`, `cancel_return`, `return_delivered` and
  `void`. Every status change goes through the machine; none is written.
- **Booking** (`booking`): one shipment from the portal or the back office,
  or many from a CSV file with a dry run that lists each refused row. A
  client reference already used by the same client is refused with
  "client reference already used (SHP-0007)"; a unique index on
  (client, client reference) backs it for two uploads at once.
- **Scans** (`scan`): the driver's pickup and the hub's scan (`LOG.HUB.01`).
  The label of a voided shipment is invalid at every scan.
- **Delivery link** (`events`): booking asks wl-delivery for the shipment's
  delivery (the shipment is its source `Subject`, type `logistics.shipment`).
  `DeliveryOutForDelivery`, `DeliveryDelivered` and `DeliveryAttemptFailed`
  run the shipment's `dispatch`, `deliver` and `fail_attempt` as the system.
  `cancel`, `cancel_return` and `void` cancel the open delivery.
- **Void** (`voiding`, BL-LOG-01): the kernel's void pattern. A `created`
  shipment with no parcel scanned is voided by the client user who booked it
  or by staff with `shipments.manage`, with a reason. The number is kept, the
  label is invalid, the delivery is cancelled, `RecordVoided` is emitted and
  the shipment is left out of `Shipment::billable`.
- **Cancel** (`cancel`, BL-LOG-02, `LOG.ACL.09`): before pickup a client user
  `cancel`s (a fee line only if the rate card has one). After pickup a
  dispatcher records the client's instruction and runs `cancel_return`: one
  return charge, the open delivery cancelled, the client told. Out for delivery
  the driver first fails the stop with *cancelled by client*, which does not
  count as an attempt. The client's e-POD makes the returned shipment
  `returned`.
- **Prices** (`rates`): the cancellation fee and the return charge are read
  from `ClientRate` rows until the rate cards of wl-invoicing (`FIN.RAT.01`)
  exist; `rates::price` is the one place to change.

## Routes

See `pages`. Staff list and page at `/shipments`; every action is a POST that
answers 303 with a toast, 422 with the reason when refused.

## Permissions

`shipments.view`, `.book`, `.manage`, `.scan`, `.deliver`, `.void` and
`.cancel` (`permissions`). The host maps them to the operations manager, the
dispatcher, hub staff, the driver and the client user.

## Tests

`tests/` boots the app with the real wl-delivery, a recording notifier and a
portal fake; `tests/support` holds the fixtures.
