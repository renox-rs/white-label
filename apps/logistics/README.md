# logistics

The vertical app for courier and freight companies (03 Logistics). This
first part is the **Shipment lifecycle**, with the two negative flows that
need only it: voiding a duplicate booking and cancelling a shipment after
pickup, then a partial pickup, the re-weigh at the hub and the hold of a
shipment for an exception. The COD remittance and the freight job come with
their own issues.

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
- **Partial pickup** (`pickup`, BL-LOG-03): `pick_up` (and a drop-off) marks
  the parcels not handed over *not received*, recalculates the weight and the
  price from the parcels received and tells the client; a parcel not received
  is never scanned in later. With nothing scanned the pickup fails, a
  *pickup failed* event keeps the reason and the shipment stays `created`.
- **Re-weigh** (`reweigh`, `billing`, BL-LOG-04): hub staff record the
  measured weight and size with a photo of the scale; above the client's
  tolerance (on the chargeable weight) the shipment is priced on the measured
  value and a surcharge line waits for the next monthly draft, held while a
  billing ticket disputes it. The latest measurement counts.
- **Hold** (`hold`, BL-LOG-05): `hold` (damaged with photos, address issue,
  client instruction, re-weigh dispute), `release` (only with the confirmed
  address or the client's recorded answer) and `return`. A failed attempt with
  *wrong address* holds the shipment. The address is corrected with an audit
  until the shipment is out for delivery, and "company liable" links a claim
  ticket.
- **COD remittance** (`remit`, `remittance_machine`, BL-LOG-14, BL-LOG-15):
  the payout of the COD collected to a client, `draft`, `approve` (another
  user than the preparer, every adjustment line approved), `reopen` and `pay`
  (the payout is an outgoing transfer recorded through the kernel's payments
  contract; a blocked client, no bank account or a net of 0 stops it);
  `payment_returned` is declared and comes with BL-LOG-17. `LOG.RUL.06`
  (`remit::run_weekly`, scheduled on Monday) creates or rebuilds one draft per
  client from the delivered shipments with COD recorded, delivered before the
  period end, on no remittance, not frozen by the delivery module (under
  review) and not marked already remitted, less the client's COD fee and the
  differences the client absorbed; the cash handovers are never read. An
  **adjustment line** (`adjustment`, `LOG.ACL.12`) is a signed correction
  with a reason: prepared by one user, approved or rejected by another;
  only an approved line counts; a paid remittance is never edited, a line goes
  on the next draft; a negative net is set to 0 and the rest carried as an
  open line.
- **Review** (`review`, `WRK.ACL.06`): a disputed delivery runs the
  shipment's `open_review` and an upheld review its `close_review`, both
  the system's on the wl-delivery events. Under review the shipment is out of
  the draft remittance (a line on a draft is removed, an approved remittance
  goes back to a draft with `reopen_system` and the accountants are told, a
  paid one is left), out of the billing (`Shipment::billable`), and the client
  is told once. Staff raise the dispute from the shipment page.
- **Partial delivery** (`partial`, `WRK.POD.01`): a shipment of several parcels
  asks wl-delivery for partial acceptance; a refused parcel needs a reason.
  `deliver` with refused parcels creates a linked return shipment directly in
  `returning` (`create_return`: parcels moved with their barcodes, COD 0, the
  return charge on it only) and emits `ShipmentDelivered`; the client's e-POD
  makes it `returned`. The expected COD stays the full amount.
- **COD differences** (`cod`, `WRK.ACL.09`): the shipment's rule on who absorbs
  a difference the operations manager approves: never the customer, the client
  only with its instruction, the business as a COD loss, the driver as cash
  still expected in the handover. The report is at `/cod-differences`.
- **Reversed deliveries** (`reversal`, `WRK.ACL.10`, `LOG.ACL.12`): when
  wl-delivery reverses a delivery the shipment runs the system's
  `reverse_delivery` (back to `attempted`) or `reverse_delivery_on_road` (back
  to `out_for_delivery`), as the event says. The shipment is out of billing
  until it is delivered again; an invoiced one gets a credit due and the
  invoice owner a task. The COD comes off a draft or approved remittance; when
  it was already paid, *refund to payer* drafts a negative adjustment line
  (source `delivery_reversal`, approved by another user) and *keep* marks it
  already remitted so it is never remitted twice. Liability *driver* or
  *business* opens one claim ticket; *driver* gives the operations manager a
  task (`/reversals`) to raise a later shortage against the driver
  (wl-payments, through the kernel's `ShortageRaiser`) or open an HR case;
  *business* records the loss; *client* and *none* post nothing.
- **Lost shipments** (`loss`, `WRK.RUL.17`): when wl-delivery marks the
  delivery lost the shipment runs the system's `mark_lost_system`; the same
  effects serve a loss off a delivery (BL-LOG-12). The shipment is `lost`,
  expects no COD (and expects it again when found), leaves billing (an
  invoiced one gets a credit due and the invoice owner a task), is linked to
  one claim ticket (the delivery's, else its own, whatever the liability), the
  client is told once and `ShipmentLost` is emitted. Scanning it at a depot
  (`found`, or the system on `DeliveryFound`) puts it `at_hub`, tells the
  client and comments on the claim ticket. After a settled claim the system
  holds it (*client instruction*) until the client chooses: delivery drafts an
  invoice for the claim credit and releases it, keeping the settlement sends
  it back.
- **Tracing and losing off a delivery** (`trace`, `LOG.RUL.12`, `LOG.ACL.14`):
  every day at 02:00 (Asia/Dubai) a shipment `picked_up`, `at_hub`, `on_hold`
  or `returning` with no scan, tracking event or note for 3 days gets one
  trace task for the operations manager; after 7 days from the same instant
  the task is escalated to *request mark lost now*. A scan at a depot
  (`/shipments/{id}/depot-scan`), a note or any change closes the task and
  restarts the clock. A user with `shipments.manage` requests `mark_lost` with
  a reason and a liability at any time; the kernel defers it as an approval
  that a holder of `shipments.approve_loss` (the operations manager, or the
  company admin) other than the requester decides; a scan or note after the
  request fails it at the final approval. The effects are those of a loss on a
  delivery.
- **Disposal** (`disposal`, `LOG.ACL.14`, `LOG.RUL.07`): a user with
  `shipments.manage` requests `dispose` on an `on_hold` shipment with a reason,
  a photo, the reference of the client's written instruction (not needed when
  *dangerous* is ticked) and a liability (*client*, *business* or *none*). The
  kernel defers it as an approval that a holder of
  `shipments.approve_disposal` other than the requester decides, also for
  dangerous goods; the shipment stays `on_hold` while it waits and after a
  rejection. On approval it is `disposed`, expects no COD, its delivery is
  cancelled and the client is told. *Business* opens one claim ticket;
  *client* records a disposal charge from the client's prices (AED 0 and a
  task for the accountant without a disposal line) and the shipment stays
  billable; *none* records the liability only. After a settled claim, *keep the
  settlement* with `then=dispose` on `/claim-choice` makes the same request.
- **Cash not handed over** (`cash_watch`, `LOG.RUL.13`, replaces `FIN.RUL.22`
  for drivers): the host collects a run's cash period with the escalation
  replaced, and every 15 minutes the rule reads those open handovers from the
  custody contract. The clock starts at the run's return to the depot (an abort
  returns it), else at the end of the driver's shift (`ShiftCalendar`), else
  at 23:59 of the run date. 12 hours later the operations manager gets a task
  to call the driver (and, for a run still out, to abort it for *driver
  unreachable*); 48 hours after the same start the company admin is told, the
  manager is asked to count at what was received and, unless an attendance
  check-in followed the start, an absence case is opened on the driver's HR
  record (`AbsenceCaseOpened`; reporting absconding is an HR decision). A
  handover counted closes the tasks. The client's remittance is never reduced;
  recovery in cash, the salary deduction (off while the client's setting is
  off, `FIN.ACL.19`, open item L-20) and the write-off with approval are
  wl-payments'.
- **Netting** (`netting`, `LOG.COD.01`, `FIN.PAY.01`): when the client's
  contract allows it (`ClientTerms::netting_allowed`), the draft remittance
  offsets the client's open, undisputed invoices (`Invoicer::open_of`), oldest
  due date first and capped so the net is not negative. At `pay` each offset is
  re-checked: an invoice that became disputed, paid or written off sends the
  remittance back to a draft with the message "offset INV-2 removed: invoice
  disputed". Otherwise the payout of the net and one incoming payment per
  offset, allocated in full to its invoice, are recorded in wl-payments and the
  statement is published (`statement`, `GET /remittances/{id}/statement`).
- **A returned transfer** (`return_payment`, `repay`, `LOG.ACL.13`): the
  accountant attaches the bank's return advice; the payout voucher is reversed
  as a bank return, the remittance is `payment_returned` and the client is told
  and asked to confirm its bank account. `repay` pays the cash part again with
  a new reference, to a changed account that is active, or to the same account
  once the client's confirmation is recorded; the statement is published again
  as a new version. A change of the client's remittance bank account
  (`account`) takes effect only when a second user approved it and the client
  confirmed it with the signed link sent to its primary e-mail on record; it
  lapses after 14 days and the requester is told. Until it is active
  remittances use the old account.
- **A statement dispute** (`dispute`, `LOG.ACL.12`): the `disputed` flag and a
  ticket for the accountant; a second dispute adds a message to the ticket;
  *upheld* clears the flag, *adjusted* also puts an adjustment line on the next
  draft that another user approves; the client is told. Nothing pauses, and a
  dispute is not offered while the transfer is returned.
- **A blocked client** (`block`, `freight`, `PTY.ACL.03`): new bookings, CSV
  uploads and freight jobs are refused with the neutral text; shipments already
  booked or picked up finish and the dispatcher sees *client on hold*; the
  Monday draft is made and approved, but `pay` and `repay` wait for the
  unblock or for one exception that the company admin (never the requester)
  approves.
- **The monthly invoice** (`billing`, `LOG.RUL.07`): on day 1 at 01:00 each
  client gets one draft invoice, through the kernel's invoicer, of the previous
  month: a delivery line for each delivered shipment (and for a returned one
  that was attempted, and a disposed one the client caused), its re-weigh
  surcharge, the return charge in the month of the e-POD, the disposal charge
  and the cancellation fee. Shipments under review, lost, voided, reversed,
  held or disposed for another cause are left out and picked up by the first
  draft after they are billable again; a shipment and a charge line are billed
  once, and running the rule twice leaves one draft (`BillingRun`).
- **The freight job** (`freight`, `freight_machine`, `freight_flow`,
  `freight_costing`, `freight_pages`, `LOG.JOB.01`): a job of a client with its
  mode, route, ETA, containers and their free-time end dates moves booked,
  documents pending, in transit, customs clearance, ready for delivery,
  delivered, closed (or cancelled before departure) by the documentation
  officer (`jobs.manage`, `jobs.deliver`, `jobs.close`). `depart` needs the
  BL/AWB, `clear` the declaration, duties and delivery order, `deliver` the
  e-POD and `close` every cost allocated and the invoice issued; the margin at
  each close is kept. Revenue and cost lines are the stand-in for job costing
  (`FIN.CST.01`), supplier bills and disbursements until wl-invoicing has them.
- **Hold and release of a freight job** (`freight_hold`, `LOG.RUL.09`): the
  documentation officer puts a job in transit, in customs clearance or ready
  for delivery on hold with a reason (customs hold or inspection, declaration
  rejected, duty unpaid, documents missing, customs platform down, client
  instruction) and a note. The state before the hold is stored with the state
  change and `release` (a note on how the reason was resolved) returns to
  exactly that state. The client and the officers are told, the portal page
  shows the reason label and the note, a second reason is a note on the open
  hold, no ETA alert is raised while on hold and the hold periods are left
  out of `is_on_time`.
- **Reopening a closed freight job** (`freight_reopen`): `jobs.close` reopens
  a closed job with a reason (late supplier bill, demurrage, port storage, or
  *other* with a note); a cancelled job is never reopened. The late cost is
  allocated with its liability (*company* only lowers the margin, *client* may
  be recharged) and the supplementary invoice is drafted through the kernel's
  invoicer and issued by the officer: a disbursement is `out_of_scope` when
  the three conditions of VATP013 are recorded, any other recharge carries
  the main service's 5% (zero-rated only for an international job with its
  evidence). `close` has the same guards and the margin of each close stays in
  the history; the first invoice is never changed.
- **Prices** (`rates`): the cancellation fee and the return charge are read
  from `ClientRate` rows until the rate cards of wl-invoicing (`FIN.RAT.01`)
  exist; `rates::price` is the one place to change. The zone prices by weight
  (`WeightBand`, `rates::band_price`) and the re-weigh terms (`ClientTerms`) are
  stand-ins in the same way.

## Routes

See `pages` and `remittance_pages`. Staff list and page at `/shipments` and
`/remittances`; every action is a POST that answers 303 with a toast, 422 with
the reason when refused.

## Permissions

`shipments.view`, `.book`, `.manage`, `.scan`, `.deliver`, `.void`,
`.cancel`, `.approve_loss` and `.approve_disposal`, `freight.manage`, `jobs.view`, `.manage`, `.deliver` and `.close`, and `remittances.view`, `.prepare`, `.approve`, `.pay`, `.request_account` and `.dispute`
(`permissions`). The host maps them to the operations manager, the
dispatcher, hub staff, the driver and the client user, and the remittances to
the accountant.

## Tests

`tests/` boots the app with the real wl-delivery, a recording notifier and a
portal fake; `tests/support` holds the fixtures.
