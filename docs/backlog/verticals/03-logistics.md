# Epic: 03 Logistics — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows of the 03 Logistics app (`apps/logistics`): the shipment's
own exceptions (void, cancel after pickup, partial pickup, re-weigh, holds, disposal, partial
acceptance, COD differences, review, reversal, lost and found), the driver who does not hand over
cash, the COD remittance corrections (adjustment lines, netting, returned transfers, bank account
control, statement disputes), the freight job's holds, late costs and container free time, and the
03 values for the shared mechanisms. Module behaviour (delivery review, reversal, lost, run abort
and hand-over, cash counts, invoice disputes) is built in the module epics and only consumed here.
Review items: D-11, X-11, X-12, X-13, X-18, X-19, 03-N3-a, 03-N5-a, 03-N6-c, 03-N7-c, 03-N8-a,
03-N9-a, 03-N10-a, 03-N11-a and the 03 MEDIUM items.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-LOG-01 | Void a duplicate booking and refuse a reused client reference | medium | s | Launch 03 | Base: 03 Shipment lifecycle; BL-KRN-03 |
| BL-LOG-02 | Cancel a shipment after pickup and send it back (`cancel_return`, `LOG.ACL.09`) | medium | m | Launch 03 | Base: 03 Shipment lifecycle; BL-LOG-26 |
| BL-LOG-03 | Pick up fewer parcels than booked | medium | s | Launch 03 | Base: 03 Shipment lifecycle |
| BL-LOG-04 | Record a re-weigh at the hub and price on the measured values | medium | s | Launch 03 | Base: 03 Shipment lifecycle; Base: wl-invoicing rate cards (`FIN.RAT.01`) |
| BL-LOG-05 | Hold a shipment for an exception and release it | medium | s | Launch 03 | Base: 03 Shipment lifecycle |
| BL-LOG-06 | Dispose of goods on the client's written instruction | medium | s | Launch 03 | BL-LOG-05; BL-LOG-12 |
| BL-LOG-07 | Deliver part of a multi-parcel shipment and return the refused parcels | high | s | Launch 03 | BL-DLV-02; BL-DLV-11 |
| BL-LOG-08 | Post a COD difference according to who absorbs it | high | s | Launch 03 | BL-DLV-03; BL-INV-22; BL-LOG-14 |
| BL-LOG-09 | Hold COD and billing while a delivery is under review | high | s | Launch 03 | BL-DLV-04; BL-INV-06 |
| BL-LOG-10 | Reverse a delivered shipment and settle COD, billing and liability | high | m | Launch 03 | BL-DLV-05; BL-LOG-09; BL-LOG-15; BL-PAY-17; BL-PAY-23 |
| BL-LOG-11 | Mark a shipment lost on a delivery, open the claim and handle found goods | high | m | Launch 03 | BL-DLV-06; BL-LOG-20 |
| BL-LOG-12 | Trace a shipment off a delivery and mark it lost with approval (`LOG.RUL.12`, `LOG.ACL.14`) | high | m | Launch 03 | BL-LOG-11; BL-KRN-01 |
| BL-LOG-13 | Escalate a driver who does not hand over cash (`LOG.RUL.13`) | high | m | Launch 03 | BL-PAY-17; BL-PAY-21; BL-DLV-08; BL-HRM-07 |
| BL-LOG-14 | Build the draft COD remittance from recorded COD, without held shipments (`LOG.RUL.06`) | high | m | Launch 03 | Base: 03 COD remittance lifecycle |
| BL-LOG-15 | Add approved adjustment lines to a COD remittance (`LOG.ACL.12`) | high | s | Launch 03 | BL-LOG-14; BL-KRN-01 |
| BL-LOG-16 | Net open invoices against COD in the remittance | medium | s | Launch 03 | BL-LOG-14; Base: wl-payments Payment lifecycle |
| BL-LOG-17 | Handle a returned remittance transfer and control bank account changes (`LOG.ACL.13`) | high | m | Launch 03 | BL-LOG-14; BL-KRN-01 |
| BL-LOG-18 | Flag and resolve a COD statement dispute | medium | s | Launch 03 | BL-LOG-15; Base: wl-ticketing Ticket lifecycle |
| BL-LOG-19 | Refuse new bookings and hold remittance payment for a blocked client | high | s | Launch 03 | BL-PTY-01; BL-PTY-04; BL-POR-01; BL-LOG-14 |
| BL-LOG-20 | Leave held and lost shipments out of the monthly invoice and add surcharges (`LOG.RUL.07`) | high | s | Launch 03 | Base: wl-invoicing usage billing (`FIN.USG.01`); BL-LOG-04 |
| BL-LOG-21 | Hold and release a freight job, keeping the state before the hold | high | m | Launch 03 | Base: 03 Freight job lifecycle |
| BL-LOG-22 | Reopen a closed freight job for a late cost | medium | s | Launch 03 | Base: 03 Freight job lifecycle; Base: wl-invoicing job costing (`FIN.CST.01`) |
| BL-LOG-23 | Cancel a freight job before departure with charges and incurred costs | medium | s | Launch 03 | Base: 03 Freight job lifecycle; Base: wl-invoicing quotations (`FIN.QUO.01`) |
| BL-LOG-24 | Alert on container free time and allocate demurrage by liability (`LOG.RUL.14`) | high | m | Launch 03 | Base: 03 Freight job lifecycle; Base: wl-invoicing supplier bills and disbursements; BL-LOG-22 |
| BL-LOG-25 | 03 Logistics: configure shared negative-flow settings — documents, rules and integrations | high | m | Launch 03 | BL-DOC-01; BL-DOC-04; BL-DOC-05; BL-HRM-01; BL-DLV-01; BL-KRN-04; BL-KRN-05; BL-KRN-08; BL-KRN-09; BL-INV-09; BL-INV-10; BL-NTF-06; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |
| BL-LOG-26 | 03 Logistics: configure shared negative-flow settings — delivery, cash, money and access | high | m | Launch 03 | BL-DLV-02; BL-DLV-03; BL-DLV-05; BL-DLV-06; BL-DLV-08; BL-DLV-09; BL-PAY-09; BL-PAY-17; BL-PAY-21; BL-INV-01; BL-INV-02; BL-INV-04; BL-INV-05; BL-PTY-04; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26 |

---

### BL-LOG-01 — 03 Logistics: void a duplicate booking and refuse a reused client reference

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** Base: 03 Shipment lifecycle (booking from the portal and CSV) · BL-KRN-03 (void pattern) · **Blocks:** None.
**Review items:** X-19, 03 MEDIUM "duplicate bookings"

#### Goal
Clients book by portal and CSV, and the same file is often uploaded twice. A shipment whose
client reference is already used by the same client is refused at booking. A duplicate or a
booking entered in error that is found later is voided while it is still `created`: the label is
invalidated, the number is kept and the shipment is never billed.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 "When it goes wrong" (row *Duplicate booking*), §6 *Shipment* (client reference unique per client), §7.1 *Shipment* (state `voided`, row `void`, checklist row *Shipment*)
- `docs/modules/wl-kernel.md` §5 void steps, §6 `PLT.ACL.09`
- `docs/WORKFLOW.md` §8 (pattern "void")

#### Terms
- **Client reference:** the shipper's own order or reference number on the booking; unique per client.
- **Void:** the platform pattern for a record entered in error: it ends `voided`, keeps its number and has no effect on money or stock.
- **`PLT.ACL.09`:** wl-kernel rule: a record is voided only with a reason, only from the states its module allows, never after it was given to a third party or an authority as a legal or tax document; approval by another user when it had posted money or moved stock.
- **Parcel scanned:** any scan of a parcel of the shipment (pickup or hub).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `voided` | neutral | yes | Entered in error or duplicate; number kept, never charged |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `void` | created | voided | `shipments.manage`, or the client user who booked it | reason (duplicate, entered in error); no parcel scanned (`PLT.ACL.09`) | label invalidated; never billed; number kept; event `RecordVoided` |

§4.1 row: "Duplicate booking — A CSV row or API shipment with a client reference already used
by the same client, in any state including `voided` and `cancelled`, is refused (a real re-booking
needs a new reference); a booking without a reference is not checked; a duplicate found later is
voided while `created` (no charge, number kept) — Shipment `void`, `PLT.ACL.09`."

§6 Shipment fields include "client reference (optional; when given, unique per client across all
states)".

#### Edge cases decided
- **Uniqueness scope:** per client party, across all states, including `voided` and `cancelled`. A voided shipment keeps its reference, so a real re-booking of the same order needs a new reference or the client's suffix; this avoids a second upload silently re-creating voided rows.
- **Empty client reference:** allowed and not checked for duplicates (references are optional on manual portal bookings).
- **CSV upload with duplicates:** the import validation (dry run) lists every refused row with the reason "client reference already used (shipment number …)"; the other rows import. Two rows with the same reference in one file: the first imports, the second is refused.
- **Concurrent uploads:** the uniqueness is enforced by a database unique index on (client, client reference), not only by a read check; the second insert fails and is reported as a duplicate.
- **Void after a scan:** a parcel scan at pickup or at the hub moves the shipment out of `created` (`pick_up` or `receive_at_hub`), so `void` is no longer offered; the shipment is sent back with `cancel_return` (BL-LOG-02, offered from `picked_up` and `at_hub`) or held with `hold` (BL-LOG-05). The guard "no parcel scanned" is a safety check for a scan recorded without a state change (for example a scan saved while the pickup failed).
- **Scan while `void` runs:** `void` needs no approval, so it is one action. The state change, the scan check and a pickup or hub scan of the same shipment lock the shipment row; whichever commits second finds the state changed and is refused (a scan of a voided label is refused as "label invalid").
- **Client user:** may void only shipments they booked themselves; other client users of the same company may not. Staff with `shipments.manage` may void any `created` shipment in scope.
- **Approval:** none: a `created` shipment has posted no money and moved no stock, so `PLT.ACL.09` needs no second user.
- **Cancellation fee:** never charged on `void` (unlike `cancel`).
- **Delivery record:** the shipment's wl-delivery delivery (if already created as `pending`) is cancelled with the shipment.

#### Must not
- Must not delete the shipment or reuse its number.
- Must not void a shipment with any parcel scanned.
- Must not bill a voided shipment (`LOG.RUL.07` leaves it out, BL-LOG-20).

#### Done when
- [ ] Given client "Souq Express" has shipment S-1001 with reference "ORD-77", when a CSV row with reference "ORD-77" is imported for the same client, then the row is refused with "client reference already used (S-1001)" and the other rows import.
- [ ] Given another client books with reference "ORD-77", it is accepted.
- [ ] Given a `created` shipment with no scan, when the client user who booked it runs `void` with reason *duplicate*, then it is `voided`, its label no longer validates at scan, its number is kept, and `RecordVoided` is emitted.
- [ ] Given a shipment with one parcel scanned at pickup (now `picked_up`), `void` is not offered and an API call to `void` is refused; `cancel_return` is offered.
- [ ] Given a `created` shipment voided at 10:00:00 and a pickup scan of its label committed after it, the scan is refused with "label invalid"; given the scan committed first, `void` is refused.
- [ ] Given a client user who did not book the shipment, `void` is not offered.
- [ ] A voided shipment never appears on a monthly invoice draft.

#### Out of scope
Cancellation before pickup with a fee (`cancel`, unchanged) and after pickup (BL-LOG-02). The
kernel void mechanics and digest (BL-KRN-03, BL-KRN-09).

---

### BL-LOG-02 — 03 Logistics: cancel a shipment after pickup and send it back (`cancel_return`, `LOG.ACL.09`)

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: 03 Shipment lifecycle · BL-LOG-26 (failure reason *cancelled by client*) · **Blocks:** None.
**Review items:** D-11, 03 MEDIUM "cancel after pickup"

#### Goal
A client may change its mind after the parcel was collected. Before pickup the client cancels
(`cancel`). After pickup only a dispatcher or operations manager can cancel, on the client's
recorded instruction: the shipment goes back to the client (`returning`) with the return charge
from the rate card. A shipment already out for delivery is first stopped by a failed attempt with
the reason *cancelled by client*.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `LOG.ACL.09`, §4.1 "When it goes wrong" (row *Client cancels*), §7.1 *Shipment* (rows `cancel`, `cancel_return`, `fail_attempt`, `return_delivered`)
- `docs/modules/wl-delivery.md` §4.1 *Delivery* (cancel of an open delivery)

#### Terms
- **Client's instruction:** a note on the shipment with who asked (client user or contact), when and how (portal message, e-mail, call), and an optional attachment.
- **Return charge:** the rate card line for returning a shipment to the client (`FIN.RAT.01`).
- **Open delivery:** the wl-delivery delivery of the shipment that is not yet delivered.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `LOG.ACL.09` | A client user can cancel a shipment only while it is `created`. After pickup only a dispatcher or operations manager can cancel it, on the client's instruction (a note of who asked, when and how, or the client user's portal request "ask us to return it", a ticket of category *client instruction*): the shipment goes back to the client (`cancel_return` to `returning`, return charge per rate card). A shipment out for delivery is first stopped by a failed attempt with the reason *cancelled by client* | `STA` | client user, dispatcher | — |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | created | cancelled | client user or `shipments.manage` (`LOG.ACL.09`) | not picked up | cancellation fee only if the rate card has one; delivery cancelled |
| `cancel_return` | picked_up, at_hub, attempted, on_hold | returning | `shipments.manage` (dispatcher, `LOG.ACL.09`) | client's instruction recorded | return charge per rate card; open delivery cancelled; client notified |
| `fail_attempt` | out_for_delivery | attempted | `shipments.deliver` | failure reason chosen (including *refused to pay*, *no cash*, *cancelled by client*, *run aborted*, *wrong address*); *cancelled by client* and *run aborted* are not counted as attempts (`WRK.RUL.08`) | recipient and client notified; *wrong address*: system `hold` with reason *address issue* |
| `return_delivered` | returning | returned | `shipments.deliver` | e-POD from the client | — |

§4.1 row *Client cancels*: "Before pickup: `cancel` (cancellation fee only if the rate card has
one). After pickup: the dispatcher runs `cancel_return` and the goods go back with the return
charge; a shipment returned before any delivery attempt is billed the return charge only. Out for
delivery: the driver fails the stop with *cancelled by client* (not counted as an attempt), then
`cancel_return`."

#### Edge cases decided
- **Who may run `cancel_return`:** users with `shipments.manage` in the shipment's depot (dispatcher, operations manager, company admin). Client users never see it; in the portal a client user on a shipment past `created` gets "ask us to return it", which opens a request to the depot's dispatchers. That request is a ticket of category *client instruction* and its text counts as the recorded instruction.
- **Out for delivery:** `cancel_return` is not offered from `out_for_delivery`. The dispatcher calls the driver, who fails the stop with *cancelled by client*; that attempt does not count against the maximum attempts. *cancelled by client* is not counted, like *run aborted*, so `WRK.RUL.08` does not return it automatically before the dispatcher acts.
- **Already `returning`** (for example after the maximum attempts): `cancel_return` is not offered; no second return charge.
- **Return charge:** exactly one return charge line per shipment, whichever path sent it back (`return` or `cancel_return`); it is recorded when the return starts and billed on the monthly invoice of the month the shipment reaches `returned` (`LOG.RUL.07`, BL-LOG-20). The delivery charge stays billable when the shipment was attempted at least once; a shipment cancelled and returned before any delivery attempt is billed the return charge only.
- **No return line on the rate card:** the return is still allowed and billed at AED 0; the accountant is told on the draft invoice ("no return price for client X").
- **COD:** a cancelled or returned shipment expects no COD.
- **Concurrency:** if the driver delivers the shipment while the dispatcher runs `cancel_return`, the second action finds the state changed and is refused.

#### Must not
- Must not let a client user cancel after pickup.
- Must not move a picked-up shipment to `cancelled` (a terminal state for not-picked-up shipments only).
- Must not run `cancel_return` without a recorded instruction.

#### Done when
- [ ] Given a `created` shipment and a rate card with a cancellation fee of AED 10, the client user can `cancel` it and one cancellation fee line of AED 10 is recorded for the next monthly invoice; given a rate card without a cancellation fee, `cancel` records no fee line.
- [ ] Given a `picked_up` shipment, the client user has no cancel action; when dispatcher A records the instruction "client e-mail 2026-11-03 10:15" and runs `cancel_return`, the shipment is `returning`, its open delivery is cancelled, one return charge is recorded, and the client is notified.
- [ ] Given `cancel_return` without an instruction, it is refused.
- [ ] Given an `out_for_delivery` shipment, `cancel_return` is not offered; after the driver's `fail_attempt` with *cancelled by client*, the attempt count is unchanged and `cancel_return` is offered.
- [ ] When the client signs the e-POD for the returned goods, `return_delivered` makes the shipment `returned`.

#### Out of scope
Return after the maximum attempts (`WRK.RUL.08`, unchanged). Monthly billing of the return charge
(BL-LOG-20). The failure reason list (BL-LOG-26).

---

### BL-LOG-03 — 03 Logistics: pick up fewer parcels than booked

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** Base: 03 Shipment lifecycle (`pick_up`) · **Blocks:** None.
**Review items:** 03 MEDIUM "partial pickup"

#### Goal
The client sometimes hands over fewer parcels than booked. The driver scans the parcels received
and marks the rest *not received*; the shipment goes ahead with what was received, its weight and
price are recalculated, and the client is told. When nothing is handed over, the pickup fails and
the shipment stays `created`.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 "When it goes wrong" (row *Fewer parcels handed over than booked*), §6 *Shipment* and *Parcel*, §7.1 *Shipment* (row `pick_up`)

#### Terms
- **Parcel:** one labelled piece of a shipment, with its own barcode and weight.
- ***Not received*:** a mark on a parcel that the client did not hand over at pickup.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `pick_up` | created | picked_up | `shipments.scan` | at least one parcel scanned; parcels not handed over marked *not received* | tracking event; weight and price recalculated from the parcels received; client told of missing parcels |

| `receive_at_hub` | created, picked_up, attempted, returning | at_hub | `shipments.scan` | from `created` (drop-off): as `pick_up`, parcels not handed over marked *not received* | tracking event; from `created`: as `pick_up` |

§4.1 row: "`pick_up` (or `receive_at_hub` for a drop-off) records the missing parcels as *not
received*; the shipment goes ahead with the parcels received, weight and price are recalculated,
the client is told. Parcels *not received* are never picked up later on that shipment: the client
books a new one. The COD amount is not changed; the client may edit it before dispatch (audited).
No parcel handed over: the pickup attempt fails and the shipment stays `created`."

#### Edge cases decided
- **Parcels *not received* are not cancelled separately:** they stay on the shipment with the mark, are excluded from weight, price and labels for scanning, and are listed on the client notice. They cannot be picked up later on the same shipment; the client books a new shipment for them.
- **Recalculated weight:** sum of the booked weights of the received parcels (measured weights come later at the hub, BL-LOG-04).
- **COD amount:** unchanged; the client may edit the COD amount before dispatch (audited). The client notice says the COD amount was not changed.
- **Failed pickup (no parcel):** a tracking event *pickup failed* with a reason (client absent, nothing ready) is recorded; the shipment stays `created` and can be picked up again or cancelled by the client.
- **Several pickups:** a shipment is picked up once; there is no second `pick_up` from `picked_up`.
- **Drop-off at the hub:** `receive_at_hub` from `created` follows the same rule (parcels not handed over marked *not received*), with the same marking and notice.

#### Must not
- Must not refuse `pick_up` because some parcels are missing.
- Must not keep pricing the shipment on parcels not received.

#### Done when
- [ ] Given a `created` shipment of 3 parcels (5 kg each, zone price for 15 kg = AED 30, for 10 kg = AED 25), when the driver scans 2 and marks 1 *not received*, then the shipment is `picked_up`, its weight is 10 kg, its price is AED 25, and the client is told that 1 parcel was not received.
- [ ] Given no parcel scanned, `pick_up` is refused, a *pickup failed* event is recorded with the reason, and the shipment stays `created`.
- [ ] The parcel marked *not received* cannot be scanned in at the hub for this shipment.

#### Out of scope
Re-weigh at the hub (BL-LOG-04). Monthly billing (BL-LOG-20).

---

### BL-LOG-04 — 03 Logistics: record a re-weigh at the hub and price on the measured values

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** Base: 03 Shipment lifecycle (`LOG.HUB.01` scanning) · Base: wl-invoicing rate cards (`FIN.RAT.01`) · **Blocks:** BL-LOG-20
**Review items:** 03 MEDIUM "re-weigh surcharge"

#### Goal
Clients often declare a lower weight than the parcel has. Hub staff record the measured weight and
dimensions with a photo of the scale. When the difference is above the client's tolerance, the
shipment is priced on the measured values, the re-weigh surcharge goes on the monthly invoice, and
the client is told. The client may dispute it through a ticket before the invoice is issued.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 "When it goes wrong" (row *Weight or size differs at the hub*), §6 *Shipment* (booked and measured weight), §7.2 `LOG.RUL.07`
- `docs/modules/wl-invoicing.md` rate cards (`FIN.RAT.01`: surcharges)

#### Terms
- **Measured values:** weight and dimensions recorded at the hub.
- **Chargeable weight:** the actual weight; when the rate card prices by volume, the higher of the actual weight and the volumetric weight (length × width × height in cm ÷ the rate card's divisor, for example 5,000). Booked and measured chargeable weights are compared.
- **Tolerance:** the client's accepted difference between booked and measured weight, on the client's rate card.
- **Re-weigh surcharge:** the price difference between measured and booked values, shown as its own line on the monthly invoice.

#### Spec excerpt (authoritative for this issue)

§4.1 row *Weight or size differs at the hub*: "Hub staff record the measured weight and dimensions
with a photo of the scale. Above the client's tolerance (0 kg when the rate card sets none) the
shipment is priced on the measured values (re-weigh surcharge on the monthly invoice) and the
client is told; a measured weight lower than the booked one never reprices. The tolerance is
compared on the chargeable weight: the higher of actual and volumetric weight when the rate card
prices by volume (volumetric divisor from the rate card). When the shipment is re-weighed, the
latest measurement counts. The client may dispute it through a ticket before the invoice is issued; the surcharge is held out of the draft while the ticket is open, and
the shipment is held (`hold`, *re-weigh dispute*) only if the client asks to stop delivery —
`LOG.HUB.01`, `FIN.RAT.01`."

§6 Shipment fields include "booked and measured weight".

`LOG.RUL.07` (part): "create draft invoice from the rate card, with re-weigh surcharges and return
charges".

§7.1 *Shipment* state `on_hold` meaning includes "re-weigh dispute".

#### Edge cases decided
- **Tolerance missing on the rate card:** 0 kg, so any increase reprices; a decrease never reprices (the booked price stays; no refund for a lighter parcel).
- **Photo:** required to save a measured value above the tolerance; optional within it.
- **Tolerance compared on:** the chargeable weight (booked vs measured), not the actual weight alone.
- **Re-weigh twice:** the latest measurement counts; all measurements stay in the history. The price is recomputed from the latest one against the booked weight: above booked + tolerance → priced on the latest measured value (even when it is lower than an earlier measurement); within tolerance or lower than booked → the booked price, and the surcharge is removed.
- **Dispute:** the client raises a ticket (category *billing*) from the shipment in the portal. While the ticket is open, the surcharge line is held out of the draft invoice. The shipment itself is not put `on_hold` for a re-weigh dispute unless the client asks to stop delivery (then `hold` with reason *re-weigh dispute*, BL-LOG-05). The ticket outcome either removes the surcharge or confirms it.
- **After the invoice is issued:** a re-weigh dispute is an invoice dispute (wl-invoicing `dispute`, BL-INV-04), not a ticket against the draft.
- **Measurement recorded after the shipment was invoiced** (no dispute): the issued invoice is never changed; the surcharge is an unbilled line and goes on the client's next monthly draft (`LOG.RUL.07` picks unbilled surcharges cumulatively), referencing the shipment.
- **COD:** unchanged by a re-weigh.
- **Rounding:** weights to 0.01 kg as entered; price from the rate card bands, AED to 2 decimals.

#### Must not
- Must not change the price without a recorded measurement.
- Must not issue the surcharge while a re-weigh ticket is open.

#### Done when
- [ ] Given a shipment booked at 2 kg (AED 20) with tolerance 0.5 kg, when hub staff record 4 kg with a photo, then the shipment is priced at the 4 kg band (AED 28), a re-weigh surcharge of AED 8 is recorded, and the client is told.
- [ ] Given a measured 2.4 kg, no surcharge and no notice.
- [ ] Given the same shipment re-weighed at 4 kg and then at 3 kg (3 kg band AED 24), the latest counts: price AED 24, surcharge AED 4; re-weighed again at 1.8 kg, price AED 20 and no surcharge.
- [ ] Given a rate card pricing by volume with divisor 5,000, a shipment booked at 5 kg (5 kg band) and measured 4.8 kg with 50 × 40 × 30 cm (volumetric 12 kg), it is priced at the 12 kg band.
- [ ] Given a shipment on the issued November invoice and a measurement of 4 kg recorded on 2026-12-03, the November invoice is unchanged and the AED 8 surcharge is on the 1 January draft.
- [ ] Given a measured 4 kg without a photo, saving is refused.
- [ ] Given an open re-weigh ticket on the shipment, the AED 8 surcharge is not on the draft invoice; when the ticket confirms it, the next draft includes it.

#### Out of scope
The monthly invoice rule itself (BL-LOG-20). Invoice disputes after issue (BL-INV-04).

---

### BL-LOG-05 — 03 Logistics: hold a shipment for an exception and release it

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** Base: 03 Shipment lifecycle · **Blocks:** BL-LOG-06
**Review items:** 03 MEDIUM "address correction", 03 MEDIUM "damaged parcel outcome"

#### Goal
A shipment with an exception (wrong address, damaged parcel, client instruction, re-weigh dispute)
is put `on_hold` with a reason so that nobody dispatches it. Hub staff may hold it from a scan. It
is released back to `at_hub` only when the address is confirmed or the client's instruction is
recorded. For a damaged parcel the client chooses: deliver as is (`release`), return, or dispose.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 "When it goes wrong" (rows *Wrong or incomplete address*, *Parcel damaged at the hub*), §7.1 *Shipment* (state `on_hold`, rows `hold`, `release`, `return`)

#### Terms
- **Exception reason:** damaged, address issue, client instruction, re-weigh dispute (§7.1 state meaning).
- **Zone:** the delivery area band on the rate card; a corrected address in another zone changes the price.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `on_hold` | warning | no | Exception: damaged, address issue, client instruction, re-weigh dispute |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `hold` | created, at_hub, attempted | on_hold | `shipments.manage` or `shipments.scan` (hub exception), or system (failed attempt with *wrong address*) | exception reason; photos when damaged | client notified; *address issue*: client asked to confirm the address in the portal |
| `release` | on_hold | at_hub | `shipments.manage` | address confirmed or client instruction recorded | — |
| `return` | attempted, on_hold | returning | `shipments.manage` or system (`WRK.RUL.08`) | — | return charge per rate card; client notified |

`WRK.RUL.08` (part): if "failure reason not *wrong address* (that one goes `on_hold`, §7.1
`fail_attempt`)".

§4.1 rows:
- *Wrong or incomplete address*: "Before dispatch, the client or customer service corrects it
  (audited; a zone change re-prices the shipment). After a failed attempt with *wrong address*,
  the system puts the shipment `on_hold` (reason *address issue*) instead of queuing a reattempt,
  and the client is asked to confirm the address in the portal; then `release`."
- *Parcel damaged at the hub*: "Shipment `hold` with reason *damaged* and photos; the client
  chooses: deliver as is (`release`), return (`return`) or dispose (`dispose`, written
  instruction, `LOG.ACL.14`). When the company is liable (set on the hold or later, while
  `on_hold`), a claim ticket is opened."

#### Edge cases decided
- **Address correction before dispatch:** an edit of the address fields (no state change), audited with old and new values; allowed for the client user and customer service until the shipment is `out_for_delivery`. A zone change re-prices the shipment at once.
- **Failed attempt with *wrong address*:** the shipment is put `on_hold` by the system with reason *address issue* when the failure reason is *wrong address*, instead of being queued for reattempt by `WRK.RUL.08`; the client is asked to confirm the address in the portal.
- **Release guard:** for *address issue*, the confirmed address (or "address unchanged, confirmed by client") is recorded; for *damaged*, *client instruction* and *re-weigh dispute*, the client's instruction is recorded. `release` is refused otherwise.
- **Damaged and company liable:** the user ticks "company liable" on the hold; a claim ticket (category *claim*) is opened and linked. The liability can be set when holding or later, before the shipment leaves `on_hold`.
- **Hold from `created`:** for a booking with a known problem before pickup (address incomplete); the driver does not pick it up while held.
- **Timer:** the shipment on hold is not traced as lost while it has notes or scans (BL-LOG-12 resets on notes).

#### Must not
- Must not dispatch a shipment `on_hold` (it is not `at_hub`).
- Must not hold a damaged shipment without photos.
- Must not release without the recorded confirmation or instruction.

#### Done when
- [ ] Given an `at_hub` shipment, hub staff scan it and hold it with reason *damaged* and 2 photos; it is `on_hold` and the client is notified. Without photos the hold is refused.
- [ ] Given the client's instruction "deliver as is" recorded, `release` puts it `at_hub`; without an instruction `release` is refused.
- [ ] Given an `attempted` shipment with failure reason *wrong address*, it is `on_hold` with reason *address issue*; after the client confirms a new address in zone B (price AED 25 instead of AED 20), `release` puts it `at_hub` and its price is AED 25, with the change audited.
- [ ] Given the client chooses return, `return` puts it `returning` with one return charge.
- [ ] Given "company liable" ticked on a damaged hold, a claim ticket is linked.

#### Out of scope
Disposal (BL-LOG-06). Lost shipments (BL-LOG-11, BL-LOG-12).

---

### BL-LOG-06 — 03 Logistics: dispose of goods on the client's written instruction

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-LOG-05 · BL-LOG-12 (approval policy `LOG.ACL.14`) · **Blocks:** None.
**Review items:** 03 MEDIUM "damaged parcel outcome"

#### Goal
A damaged or dangerous shipment on hold may be destroyed instead of delivered or returned. The
disposal needs a reason and a photo, the client's written instruction (unless the goods are
dangerous), and an operations manager's approval by someone other than the requester. A claim
ticket opens when the company is liable; the disposal is charged only when the client caused it.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `LOG.ACL.14`, §4.1 (row *Parcel damaged at the hub*), §4.4 step 3, §7.1 *Shipment* (state `disposed`, row `dispose`), §7.2 `LOG.RUL.07`

#### Terms
- **Written instruction:** a document from the client (e-mail, letter, portal message exported) attached to the shipment.
- **Dangerous goods:** goods the hub classifies as dangerous (leaking, hazardous); for these the written instruction is not required.
- **Client-caused:** the damage or danger comes from the client's packing or contents.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `disposed` | neutral | yes | Destroyed on the client's written instruction (damaged, dangerous) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `dispose` | on_hold | disposed | `shipments.manage` | reason and photo; client's written instruction attached and approval (`LOG.ACL.14`) | claim ticket when the company is liable; disposal charged only when the client caused it (AED 0 and a note to the accountant when the rate card has no disposal line); client notified |

`LOG.ACL.14` (part): "Disposal also needs the client's written instruction attached, unless the
goods are dangerous"; marked `disposed` "only with an operations manager's approval; the requester
cannot approve".

`LOG.RUL.07` (part): shipments "`disposed` (unless client-caused) left out" of the monthly invoice.

#### Edge cases decided
- **Approval:** the request goes through the approval policy built in BL-LOG-12; the shipment stays `on_hold` while it waits. Rejected or returned: it stays `on_hold` (BL-KRN-01).
- **Dangerous goods:** the requester ticks *dangerous* and the photo shows it; the instruction becomes optional, the approval is still required. The client is notified after disposal.
- **Liability:** the requester chooses *client*, *business* or *none*; the approver may change it. *business* opens a claim ticket (category *claim*); *client* adds a disposal charge from the rate card; no disposal line on the rate card means AED 0 and a note to the accountant. *none* (nobody at fault, for example goods that became dangerous in normal handling): no disposal charge, no claim ticket, the shipment is not billed (not client-caused, BL-LOG-20), and the liability *none* is recorded on the shipment.
- **COD:** none expected after disposal.
- **Billing:** a disposed shipment is billed (delivery charge and disposal charge) only when client-caused (BL-LOG-20).
- **Stock or remains:** not tracked (no warehouse stock in the MVP).

#### Must not
- Must not dispose without approval, even for dangerous goods.
- Must not let the requester approve.
- Must not dispose from any state other than `on_hold`.

#### Done when
- [ ] Given an `on_hold` shipment (reason *damaged*), when user A requests `dispose` with reason, photo and the client's e-mail attached, and operations manager B approves, then it is `disposed` and the client is notified.
- [ ] A cannot approve their own request; without an attachment and without *dangerous* the request is refused.
- [ ] Given *dangerous* ticked and no attachment, the request is accepted and still needs B's approval.
- [ ] Given liability *business*, a claim ticket is opened; given *client*, a disposal charge is recorded and the shipment is on the next monthly draft; given *none*, no claim ticket, no disposal charge and the shipment is not on the monthly draft.
- [ ] When B rejects, the shipment stays `on_hold`.

#### Out of scope
The approval policy and lost marking off a delivery (BL-LOG-12). Claim valuation (Later, `LOG.CLM.01`).

---

### BL-LOG-07 — 03 Logistics: deliver part of a multi-parcel shipment and return the refused parcels

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-DLV-02 (partial acceptance per line) · BL-DLV-11 (run close with refused goods) · **Blocks:** None.
**Review items:** X-11

#### Goal
A recipient may accept some parcels of a multi-parcel shipment and refuse the others. The shipment
becomes `delivered` for the accepted parcels; each refused parcel has a reason, comes back with the
driver and goes back to the client on a linked return shipment that carries the return charge.
Single-parcel shipments have no partial acceptance: refusing means a failed attempt.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 "When it goes wrong" (row *Recipient refuses some parcels of a multi-parcel shipment*), §6 *Shipment* (return shipment link), §7.1 *Shipment* (row `deliver`, row `return_delivered`)
- `docs/modules/wl-delivery.md` §4.1 paragraph "Partial acceptance", event `DeliveryDelivered`, `DeliveryRefusedReceived`

#### Terms
- **Delivery line (03):** one parcel of the shipment, quantity 1.
- **Linked return shipment:** a new shipment for the refused parcels, from the recipient's area back to the client, linked to the original.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `deliver` | out_for_delivery | delivered | `shipments.deliver` (`WRK.ACL.05`) | e-POD captured; COD collected as due, or the difference approved (`WRK.ACL.09`); in a multi-parcel shipment every refused parcel has a reason | tracking event; refused parcels go back to the client on a linked return shipment (`create_return`); event `ShipmentDelivered` |
| `create_return` | — (new record) | returning | system (`deliver` with refused parcels) | at least one refused parcel with a reason | new shipment linked to the original, created directly in `returning` because the parcels are on the vehicle: refused parcels moved to it (barcodes kept), client = the original client, address = the client's pickup address, COD = 0; the return charge is on this shipment only |
| `return_delivered` | returning | returned | `shipments.deliver` | e-POD from the client | — |

§4.1 row: "Partial acceptance: delivered for the accepted parcels; refused parcels come back with
the driver and go back to the client on a linked return shipment with the return charge. Parcels
carry no COD amount, so the shipment's COD amount stays expected in full; a shortfall follows the
COD difference row below. Single-parcel shipments: partial acceptance is off — `WRK.POD.01`."

wl-delivery §4.1 *Cash expected after partial acceptance*: when the source gives only one amount,
the expected cash stays the full amount and a shortfall goes through the COD difference flow.

wl-delivery setting: partial acceptance per source type: on (03 single parcels: off).

#### Edge cases decided
- **Return shipment creation:** `create_return` runs when `DeliveryDelivered` carries refused parcels; the record starts in `returning` without passing through `created` (it is physically on the vehicle), with the refused parcels moved to it (barcodes kept), client = the original client, address = the client's pickup address, COD = 0.
- **Return charge:** charged once on the return shipment, not on the original.
- **Parcels not back at run close:** a refused parcel ticked *missing* at run close (BL-DLV-11) stays on the return shipment, which is then traced by `LOG.RUL.12` (BL-LOG-12).
- **COD:** a 03 shipment has one COD amount and its parcels carry none, so the expected COD stays the full amount when parcels are refused (wl-delivery: no line amounts → full amount). If the recipient pays less for the accepted parcels, the difference goes through the COD difference flow (approval and absorber, BL-LOG-08); the return shipment has COD = 0.
- **Billing:** the original shipment is billed as delivered; the return shipment carries the return charge (BL-LOG-20).
- **Single parcel:** the stop screen offers only `fail_attempt` when the recipient refuses.

#### Must not
- Must not deliver a multi-parcel shipment with a refused parcel that has no reason.
- Must not leave refused parcels on the delivered shipment.
- Must not offer partial acceptance on a single-parcel shipment.

#### Done when
- [ ] Given a shipment of 3 parcels, when the recipient accepts 2 and refuses 1 with reason *damaged* and a photo, then the shipment is `delivered` with 2 parcels, a linked return shipment created by `create_return` exists in `returning` (with no `created` history row) with the refused parcel and one return charge, and `ShipmentDelivered` is emitted.
- [ ] Given the refused parcel without a reason, `deliver` is refused.
- [ ] Given a single-parcel shipment, no refused quantity can be entered.
- [ ] Given a 3-parcel shipment with COD AED 300, 1 parcel refused and AED 200 paid, the cash expected is AED 300, the AED 100 difference needs the operations manager's approval with an absorber (BL-LOG-08), and the return shipment's COD is 0.
- [ ] When the client signs the e-POD for the return, the return shipment is `returned`.

#### Out of scope
Proof of delivery per line (BL-DLV-02). Run close (BL-DLV-11). COD differences (BL-LOG-08).

---

### BL-LOG-08 — 03 Logistics: post a COD difference according to who absorbs it

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-DLV-03 (COD collected short, `WRK.ACL.09`) · BL-INV-22 (COD difference in billing) · BL-LOG-14 (remittance draft) · **Blocks:** None.
**Review items:** 03-N3-a

#### Goal
When a recipient pays less than the COD amount, the operations manager approves the difference from
the phone and chooses who absorbs it (wl-delivery). In 03 the choice has three outcomes: the
*client* (only on the client's instruction, deducted on the remittance line with the reason and
shown on the statement), the *business* (a COD loss; the client still gets the full amount) or
the *driver* (the full amount stays expected in the driver's cash handover). The client is never
short-paid because of a driver or a business decision.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 steps 10–11, "When it goes wrong" (row *Recipient pays less than the COD amount or refuses to pay*), §6 *Shipment* (COD collected and difference) and *COD remittance* (COD differences absorbed by the client), §7.2 `LOG.RUL.06`, §9 "COD and the client's money"
- `docs/modules/wl-delivery.md` §4.1 paragraph "Cash on delivery short-paid or refused", event `DeliveryDelivered`

#### Terms
- **COD amount:** the amount the client asked the courier to collect from the recipient; the client's money.
- **COD difference:** the COD amount minus cash collected. Refused parcels do not lower the expected COD in 03 (parcels carry no COD amount, BL-LOG-07).
- **COD loss:** an expense of the business recorded for the difference it absorbs.
- **Client's instruction:** recorded consent of the client to receive less (for example an agreed discount), attached to the approval.

#### Spec excerpt (authoritative for this issue)

§4.1 row: "The driver records the cash collected. Above the tolerance (AED 0), the driver enters a
reason and asks the operations manager from the phone (`WRK.ACL.09`), who must choose who absorbs
the difference (none preselected): *client* (only with the client's instruction, e.g. an agreed discount: deducted on the
remittance line with the reason, shown on the statement), *business* (COD loss) or *driver* (stays
expected in the cash handover). No answer during the stop, or nothing paid: forced failed attempt
*refused to pay* or *no cash* — wl-delivery §4.1."

§4.1 step 11: "The amount owed is the COD recorded on delivered shipments, whatever happened to the
cash after collection: a driver's shortage is the company's loss, never the client's."

`WRK.ACL.09` (part): the operations manager "chooses who absorbs it (*client*, *business* or
*driver*; none preselected, the wl-delivery option *customer* not offered)".

§9: "a driver's shortage, a written-off shortage or a driver who absconds never reduces what the
client is remitted; only a COD difference the client agreed to absorb, or an approved adjustment
line, does (`LOG.ACL.12`)."

#### Edge cases decided
- **Absorber options in 03:** *client*, *business*, *driver*. The wl-delivery option *customer* (balance stays open on the source invoice) does not apply: the courier does not invoice the recipient. *customer* is hidden for shipments, and no default absorber is preselected; the approver must choose.
- ***Client* without instruction:** refused; the approver attaches or notes the client's instruction (who, when, how).
- **What the shipment stores:** COD amount, COD collected, difference, reason, absorbed by, approver, and the instruction for *client*.
- **Remittance amount per shipment:** COD amount − difference when absorbed by *client*; otherwise the full COD amount (BL-LOG-14 builds the draft from these values).
- ***Business*:** a COD loss record (shipment, amount, reason, approver) is created for the depot; it appears in the report "COD differences by who absorbed them".
- ***Driver*:** nothing is posted here; the driver's handover expects the full COD amount (wl-delivery), and a short count follows the cash shortage flow (BL-PAY-17).
- **Client agrees after the statement:** not this issue; it is an adjustment line (BL-LOG-15).
- **Rounding:** AED with 2 decimals; no rounding of differences.

#### Must not
- Must not reduce the client's remittance for a *business* or *driver* difference.
- Must not deduct for *client* without a recorded instruction.
- Must not create an invoice balance for the recipient.

#### Done when
- [ ] Given COD AED 300 and AED 250 collected, approved with *client* and the instruction "agreed discount, client e-mail 2026-11-04", the shipment stores difference AED 50 absorbed by *client*, and the next draft remittance shows AED 250 for it with the reason on the statement line.
- [ ] Same with *business*: the remittance shows AED 300 and a COD loss of AED 50 is recorded for the depot.
- [ ] Same with *driver*: the remittance shows AED 300, no COD loss, and the driver's handover expects AED 300.
- [ ] *client* without an instruction is refused; *customer* is not offered.

#### Out of scope
The approval on the phone and the forced failed attempt (BL-DLV-03). The draft remittance rule
(BL-LOG-14). Cash shortages (BL-PAY-17).

---

### BL-LOG-09 — 03 Logistics: hold COD and billing while a delivery is under review

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-DLV-04 (delivery review) · BL-INV-06 (invoice flag from a disputed delivery) · **Blocks:** BL-LOG-10
**Review items:** 03-N11-a

#### Goal
When a recipient or a client denies a delivery, customer service opens a dispute ticket and the
delivery goes `under_review` in wl-delivery. The shipment follows into `under_review`: its COD is
held out of remittances and its billing is held (or the issued invoice is flagged disputed), and
the client is told. If the review upholds the delivery, the shipment goes back to `delivered` and
COD and billing are released.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `WRK.ACL.06`, §4.1 "When it goes wrong" (row *Recipient denies the delivery, or it was marked by mistake*), §7.1 *Shipment* (states `delivered`, `under_review`; rows `open_review`, `close_review`), §7.1 *COD remittance* (paragraph after the diagram)
- `docs/modules/wl-delivery.md` events `DeliveryDisputed`, `DeliveryDisputeClosed`

#### Terms
- **Dispute ticket:** the wl-ticketing ticket linked to the delivery review.
- **Held out of remittances:** the shipment's COD is not included in a draft COD remittance until the outcome is known.
- **Remitted:** the shipment's COD is on a remittance that is `paid` (or `payment_returned`, still owed to the client). A COD on a `draft` or `approved` remittance is not yet remitted: its line can still be removed.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `delivered` | success | no | Delivered with e-POD; reversible only through review |
| `under_review` | warning | no | The recipient or the client denies the delivery, or it was marked by mistake; COD and billing held |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `open_review` | delivered | under_review | system (`DeliveryDisputed`) | dispute ticket linked | COD held out of remittances (its line removed from a `draft`; an `approved` remittance holding it goes back to `draft` with `reopen` and the accountant is told); billing held, or the issued invoice set `disputed`; client notified |
| `close_review` | under_review | delivered | system (`DeliveryDisputeClosed`) | outcome *upheld* | COD and billing released |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `WRK.ACL.06` | A delivery's e-POD is never edited once the shipment is `delivered`. A correction is only an audited `reverse_delivery` that keeps the original proof and marks it reversed; a dispute is a ticket and puts the shipment `under_review` | `STA` | all staff | — |

COD remittance (§7.1): "Shipments `under_review` or `lost` are left out of the draft until their
outcome is known."

#### Edge cases decided
- **Who starts it:** customer service raises the dispute ticket from the shipment (category *delivery dispute*); wl-delivery runs `open_review`, and the shipment's `open_review` runs on `DeliveryDisputed`. Users never run the shipment transitions directly.
- **Invoice flag:** set by wl-invoicing (BL-INV-06) when the shipment was already on an issued invoice; this issue does not touch the invoice.
- **COD already remitted** (on a `paid` remittance): nothing is taken back while under review; the remitted amount stays paid. Only a later reversal creates a negative adjustment line (BL-LOG-10).
- **COD collected but not yet remitted:** left out of every draft created while `under_review`; if a draft already contains it and is not `paid`, the line is removed from the draft (a `draft` is rebuilt; an `approved` draft is sent back with `reopen` and the accountant is told).
- **Client notice:** one notice per review, naming the shipment and the ticket; restricted to the client's users (`LOG.ACL.02`).
- **Release:** on `close_review`, the shipment's COD goes into the next draft remittance and, if it was not yet invoiced, into the next monthly invoice.
- **Other outcomes of the review:** `close_review` runs only for *upheld* (wl-delivery `DeliveryDisputeClosed` carries no other outcome). A review that ends in a reversal arrives as `DeliveryReversed` and runs `reverse_delivery` from `under_review` (BL-LOG-10: COD, billing and liability settled there); a delivery untraced for 7 days while under review arrives as `DeliveryLost` and runs `mark_lost` (BL-LOG-11). In both cases the hold set here is replaced by those effects.
- **Event received twice:** the second finds the shipment already in the target state and does nothing.

#### Must not
- Must not edit the e-POD.
- Must not include an `under_review` shipment in a draft remittance or a monthly invoice draft.
- Must not run `open_review` or `close_review` from a user action.

#### Done when
- [ ] Given a `delivered` shipment with COD AED 300 not yet remitted, when `DeliveryDisputed` arrives, the shipment is `under_review`, the client is notified, and the next draft remittance leaves it out.
- [ ] Given that shipment already in an `approved` remittance, the remittance goes back to `draft` without the line and the accountant is told.
- [ ] When `DeliveryDisputeClosed` with *upheld* arrives, the shipment is `delivered` and the next draft remittance includes AED 300.
- [ ] No user can run `open_review` or `close_review` on the shipment.

#### Out of scope
The delivery review itself (BL-DLV-04). The invoice flag (BL-INV-06). Reversal from review
(BL-LOG-10). Lost from review (BL-LOG-11).

---

### BL-LOG-10 — 03 Logistics: reverse a delivered shipment and settle COD, billing and liability

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-DLV-05 (delivery reversal, `WRK.ACL.10`) · BL-LOG-09 · BL-LOG-15 (adjustment lines) · BL-PAY-17 (cash shortage) · BL-PAY-23 (later cash shortage, `raise_shortage`) · **Blocks:** None.
**Review items:** 03-N11-a, X-11

#### Goal
A delivery marked by mistake or denied by the recipient is reversed in wl-delivery with an
operations manager's approval and a liability decision. The shipment follows back to
`out_for_delivery` or `attempted`: it leaves billing until it is delivered again (credited if
already invoiced), its COD is kept for the next attempt or refunded to the payer (a negative
adjustment line when already remitted), the liability is recorded, and the client is told.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `WRK.ACL.10`, `LOG.ACL.12`, §4.1 "When it goes wrong" (row *Recipient denies the delivery, or it was marked by mistake*), §4.4 step 3, §6 *Shipment* (liability, claim ticket), §7.1 *Shipment* (row `reverse_delivery`)
- `docs/modules/wl-delivery.md` event `DeliveryReversed`

#### Terms
- **Liability:** driver, business, client or none; who carries the cost of the mistake.
- **Payer:** the recipient who paid the COD.
- **Negative adjustment line:** a line on the next draft COD remittance that takes back an amount already paid to the client (BL-LOG-15).
- **Cash treatment:** *keep for next attempt* or *refund to payer*; chosen by the operations manager who gives the final approval of the reversal in wl-delivery (BL-DLV-05) and carried on `DeliveryReversed` (field *cash treatment*) with the liability. The shipment stores both; this issue only applies them.
- **Already remitted:** the shipment's COD is on a `paid` remittance (BL-LOG-09).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reverse_delivery` | delivered, under_review | out_for_delivery, attempted | system (`DeliveryReversed`) | approved under `WRK.ACL.10` | original proof kept, marked reversed; out of billing until delivered again (credit note if invoiced); COD kept for the next attempt (marked *already remitted* when it was, so it is never remitted again) or refunded to the payer, as a negative adjustment line if already remitted (`LOG.ACL.12`); liability: *driver* → task for the operations manager naming the amount, to raise a later cash shortage against the driver (`raise_shortage` in wl-payments; while the driver's handover for that cash is still `open`, the amount is added to its expected total instead) or open an HR case, *business* → loss; claim ticket when *driver* or *business*; client notified |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `WRK.ACL.10` | `reverse_delivery` and a manual `mark_lost` of a delivery need an operations manager's approval; the requester cannot approve, and the driver of the run can neither request nor approve | `SOD`, `STA` | all staff | on |

§4.1 row (part): "Outcome: `close_review` (upheld) or `reverse_delivery` with an operations
manager's approval and a liability decision (driver, business, client, none); *driver* gives the
operations manager a task to raise a later cash shortage against the driver (`raise_shortage`,
wl-payments) or open an HR case. Already remitted COD
that is refunded to the payer becomes a negative adjustment line on the next remittance
(`LOG.ACL.12`); an invoiced shipment gets a credit note."

§4.4 step 3: the claim ticket (category *claim*) is "opened automatically for every lost shipment
… and for a reversed or disposed shipment where the company is liable (*driver* or *business*)".

#### Edge cases decided
- **Target state:** taken from `DeliveryReversed` (wl-delivery decides it at final approval).
- **Billing:** not yet invoiced → the shipment is simply left out of the next monthly draft until delivered again (BL-LOG-20). Already invoiced → the invoice dispute stays open and the invoice owner resolves it `credited` for this shipment's charge (BL-INV-06 task); this issue marks the shipment "credit due" so the task shows the amount. When it is delivered again later, it is billed again in the month of that delivery.
- **COD not yet remitted, kept for the next attempt:** the shipment stays expecting its COD amount; nothing goes to the remittance.
- **COD refunded to the payer and not yet remitted:** the shipment's COD amount becomes 0 (no COD expected any more, the delivery is still due); it is left out of the remittance.
- **COD refunded to the payer and already remitted:** a negative adjustment line for the COD amount, with the reason "delivery reversed, COD refunded to recipient", is drafted on the client's next remittance and goes through the `LOG.ACL.12` approval (BL-LOG-15).
- **COD kept and already remitted:** nothing changes on the remittance; the cash stays for the next attempt and is not remitted twice. The shipment is marked "COD already remitted" so the next delivery does not add it to a remittance again.
- **Liability *driver*:** the operations manager gets a task naming the amount and the driver, with two actions: *raise shortage* runs wl-payments `raise_shortage` against the driver with the shipment as source (BL-PAY-23: a new handover of kind *later shortage*, `short` from the start, then recovered or written off within `FIN.ACL.06`), or *open HR case* opens an HR case ticket. If the driver's handover for that cash is still `open`, `raise_shortage` is not used: wl-payments adds the amount to its expected total and it shows at the count (BL-PAY-23); the task then only records that. **Business:** a loss record like the COD loss (BL-LOG-08). **Client** or **none:** nothing posted.
- **Claim ticket:** opened (category *claim*) only when liability is *business* or *driver* (company liable); linked to the shipment.
- **Repeat:** a shipment can be delivered and reversed again; each reversal is a separate history row.

#### Must not
- Must not edit or delete the original e-POD.
- Must not remit the same COD twice.
- Must not run the shipment's `reverse_delivery` from a user action (only from `DeliveryReversed`).

#### Done when
- [ ] Given a `delivered` shipment with COD AED 120 already remitted on 2026-11-02, when `DeliveryReversed` arrives with cash treatment *refund to payer* and liability *driver*, then the shipment is `attempted`, its proof is marked reversed, a draft adjustment line of −AED 120 with the reason appears on the client's next remittance awaiting approval, a claim ticket is linked, the operations manager has a task for AED 120, and the client is notified. With the driver's handover of that run already `counted`, *raise shortage* on the task creates a later shortage handover `short` AED 120 with the shipment as source.
- [ ] Given a `delivered` shipment not yet invoiced, after reversal it is not on the next monthly draft; after it is delivered again on 2026-12-03, it is on the December invoice.
- [ ] Given an invoiced shipment (AED 25 charge), after reversal the invoice owner's task shows a credit due of AED 25.
- [ ] Given *keep for next attempt* and COD already remitted, the later delivery does not add the COD to a remittance again.
- [ ] Given liability *client*, no claim ticket and no task are created.

#### Out of scope
The reversal request, approval and cash treatment choice (BL-DLV-05). The adjustment line approval
(BL-LOG-15). Cash shortage records (BL-PAY-17, BL-PAY-23). The invoice credit note itself
(BL-INV-06).

---

### BL-LOG-11 — 03 Logistics: mark a shipment lost on a delivery, open the claim and handle found goods

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-DLV-06 (delivery lost and found, `WRK.RUL.17`) · BL-LOG-20 · **Blocks:** BL-LOG-12
**Review items:** 03-N10-a

#### Goal
A shipment whose delivery is untraced for 7 days is marked `lost` by wl-delivery, and the shipment
follows. The client is told, a claim ticket is linked, the shipment leaves billing (credited if
already invoiced) and no COD is expected. If the goods turn up, the shipment goes back to `at_hub`
and the claim ticket is updated, or reopened if it was already settled; the client then chooses
delivery (the claim credit is reversed by an invoice) or keeping the settlement.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 "When it goes wrong" (row *Shipment cannot be found*), §4.4 steps 3–4, §6 *Shipment* (liability, claim ticket) and *Claim*, §7.1 *Shipment* (state `lost`, rows `mark_lost`, `found`), §7.2 `WRK.RUL.17`
- `docs/modules/wl-delivery.md` events `DeliveryLost`, `DeliveryFound`

#### Terms
- **Claim ticket:** a wl-ticketing ticket of category *claim*; in the MVP customer service values it by hand against the client's terms and records the decision and liability.
- **Settled claim:** a claim ticket resolved with an approved amount that was credited to the client.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `lost` | error | no | Untraced; out of billing; can be found again |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `mark_lost` | out_for_delivery, attempted, under_review, picked_up, at_hub, on_hold, returning | lost | system (`DeliveryLost`, `WRK.RUL.17`) while on a delivery; otherwise a user with `shipments.manage` requests it (usually prompted by the `LOG.RUL.12` trace task; it may be requested earlier) | manual: reason, liability and approval (`WRK.ACL.10` on a delivery, `LOG.ACL.14` otherwise); no scan since the request, checked at final approval | claim ticket opened (always, whatever the liability; liability is decided on the claim, §4.4); client notified; out of billing (if invoiced: a task for the invoice owner to credit the charge); no COD expected (COD already remitted is not taken back automatically: customer service decides through the claim); event `ShipmentLost` |
| `found` | lost | at_hub | `shipments.scan`, or system (`DeliveryFound`) | scanned at a depot | client notified; claim ticket updated, reopened if already settled (§4.4); after a settled claim, system `hold` with reason *client instruction* until the client chooses |
| `hold` | created, at_hub, attempted | on_hold | `shipments.manage` or `shipments.scan` (hub exception), or system (failed attempt with *wrong address*; `found` after a settled claim, reason *client instruction*) | exception reason; photos when damaged | client notified; *address issue*: client asked to confirm the address in the portal |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `WRK.RUL.17` *(shared)* | a delivery `out_for_delivery`, `attempted` or `under_review` untraced for N days | goods not received back or traced | delivery and shipment `mark_lost`; notify the dispatcher and the client; open a claim ticket | N = 7 | on | MVP |

§4.4 step 4: "An approved claim becomes a credit note (`FIN.INV.01`). A shipment marked lost is
left out of the monthly invoice; if it was already invoiced, its charge is credited too. If a lost
shipment is found after the claim was settled, the claim ticket is reopened and the client chooses
delivery (the claim credit is reversed by an invoice) or keeping the settlement (the goods are
returned to the client or disposed of with its instruction)."

#### Edge cases decided
- **This issue covers** the shared effects of `mark_lost` (used also by BL-LOG-12) and `found`, and the system `mark_lost` from `DeliveryLost`.
- **One claim ticket:** wl-delivery opens a claim ticket on `DeliveryLost`; the shipment links that ticket instead of opening a second. For a manual `mark_lost` off a delivery (BL-LOG-12) the shipment opens it.
- **Claim ticket and liability:** a lost shipment always gets a claim ticket, whatever its liability (the goods are gone, so the client may claim). The liability is decided on the claim; with *client* or *none* customer service resolves it with no payment. This differs from reversal and disposal, which open a claim only when the company is liable (§4.4 step 3).
- **One client notice:** the shipment's notice; the wl-delivery "sender notified" for 03 is the same notice (no duplicate).
- **Billing:** not yet invoiced → left out (BL-LOG-20). Invoiced → the invoice owner gets a task to credit the shipment's charge (the invoice dispute from BL-INV-06 when it came from a review; otherwise a task "credit lost shipment" with the amount).
- **COD:** no COD expected; a COD already collected and handed over (lost after a reversal) is handled by the reversal rules, not here. If COD was already remitted, nothing is taken back automatically; customer service decides through the claim.
- **Found, claim not settled:** the claim ticket gets a comment "goods found at <depot>" and its owner is notified; the shipment is `at_hub` and goes through normal dispatch; billing restarts when it is delivered.
- **Found, claim settled:** two transitions in one action: `found` (`lost` → `at_hub`), then the system runs `hold` (`at_hub` → `on_hold`) with reason *client instruction*, so the shipment cannot be dispatched before the client chooses; the ticket is reopened. *Delivery*: an invoice for the claim credit amount is drafted (reason "claim reversed, goods found") and the shipment is released. *Keep the settlement*: `return` or `dispose` (BL-LOG-06) per the instruction.
- **Liability on system marking:** *none* (wl-delivery default); customer service sets it on the claim.
- **Time zone:** the 7 days are counted by wl-delivery (02:00 Asia/Dubai run).

#### Must not
- Must not open a second claim ticket for the same loss.
- Must not bill a `lost` shipment or expect its COD.
- Must not reverse a claim credit without the client's choice.

#### Done when
- [ ] Given an `attempted` shipment whose delivery becomes `lost` on 2026-11-09, the shipment is `lost`, linked to the delivery's claim ticket, the client gets one notice, `ShipmentLost` is emitted, and the November draft invoice leaves it out.
- [ ] Given the shipment was on the October invoice (charge AED 25), the invoice owner gets a task to credit AED 25.
- [ ] Given a `lost` shipment scanned at Depot B, it is `at_hub`, the client is notified and the claim ticket gets a comment.
- [ ] Given a shipment marked lost by the system with liability *none*, a claim ticket is linked.
- [ ] Given the claim was settled with a credit of AED 400, scanning it reopens the ticket and the history shows `found` then a system `hold` (*client instruction*), leaving the shipment `on_hold`; when the client chooses delivery, a draft invoice of AED 400 is created and the shipment is released.

#### Out of scope
Untraced timer and delivery marking (BL-DLV-06). Lost off a delivery (BL-LOG-12). Claim valuation
from liability limits (Later, `LOG.CLM.01`).

---

### BL-LOG-12 — 03 Logistics: trace a shipment off a delivery and mark it lost with approval (`LOG.RUL.12`, `LOG.ACL.14`)

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-LOG-11 (effects of `mark_lost` and `found`) · BL-KRN-01 (approval returned) · **Blocks:** BL-LOG-06
**Review items:** 03-N10-a

#### Goal
A shipment that is not on a delivery (picked up, at the hub, on hold, being returned) can also go
missing. A daily rule opens a trace task after 3 days without a scan and asks for `mark_lost` after
7. Marking lost (and disposing, BL-LOG-06) off a delivery needs an operations manager's approval by
someone other than the requester.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `LOG.ACL.14`, §4.1 (row *Shipment cannot be found*), §7.1 *Shipment* (row `mark_lost`), §7.2 `LOG.RUL.12`

#### Terms
- **Last depot:** the depot of the shipment's last scan.
- **Trace task:** a task for the operations manager of the last depot to look for the shipment (search the hub, call the driver, check the run sheet).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `LOG.RUL.12` | daily | shipment `picked_up`, `at_hub`, `on_hold` or `returning` (not on a delivery) with no scan, tracking event or note for N days | trace task for the operations manager of the last depot (one open task per shipment; a scan closes it); after M days still untraced (M counted, like N, from the last scan, tracking event or note), the task is escalated to the depot's operations manager as "request `mark_lost` now": a user requests it and another approves (`LOG.ACL.14`); the rule never requests it itself | N = 3; M = 7 | on | MVP |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `LOG.ACL.14` | A shipment that is not on a delivery (`picked_up`, `at_hub`, `on_hold`, `returning`) is marked `lost` or `disposed` only with an operations manager's approval; the requester cannot approve. Disposal also needs the client's written instruction attached, unless the goods are dangerous | `SOD`, `STA` | hub staff, dispatcher | on |

`mark_lost` row (§7.1): By "a user with `shipments.manage` requests it (usually prompted by the
`LOG.RUL.12` trace task; it may be requested earlier)" off a delivery; guards "manual: reason,
liability and approval (… `LOG.ACL.14` otherwise); no scan since the request, checked at final
approval".

§3.3 (after the table): "Where a rule above asks for an operations manager's approval
(`WRK.ACL.09`, `WRK.ACL.10`, `FIN.ACL.06`, `LOG.ACL.14`), the company admin may also approve,
because it holds every permission (for example in a depot with a single operations manager); the
requester never may."

#### Edge cases decided
- **"No scan for N days":** 3 × 24 hours since the latest scan or tracking event of the shipment, checked by the daily run at 02:00 Asia/Dubai.
- **M counted from:** the same point as N (the latest scan, tracking event or note), not from the trace task: last scan 2026-11-01 10:00 → task on the run of 2026-11-05 02:00 (first run after 72 hours) → escalation on the run of 2026-11-09 02:00 (first run after 7 × 24 hours). A note on the shipment also resets the clock, as in `WRK.RUL.17`, so an actively handled hold is not traced.
- **`returning` with a delivery:** a return in progress on a run is a delivery and is handled by `WRK.RUL.17` (BL-LOG-11); `LOG.RUL.12` handles `returning` shipments not on a run.
- **"Request `mark_lost`" after M days:** the rule cannot be the requester of an approval; it updates the trace task to "request `mark_lost` now" and escalates it to the depot's operations manager. The user who submits the request is the requester; another operations manager or the company admin approves.
- **Single operations manager in the depot:** the company admin may approve (§3.3 note: it counts as an approver wherever an operations manager's approval is asked); the requester still cannot approve.
- **Idempotency:** one open trace task per shipment; the daily run does not create a second.
- **Found during the trace:** a scan closes the trace task automatically.
- **Approval rejected or returned:** the shipment keeps its state (BL-KRN-01); the trace task stays open.
- **Approval pending and the shipment is scanned:** the request `fail`s at final approval (the state or scan changed) and nothing happens: the guard "no scan since the request" is checked at final approval.
- **Manual `mark_lost` before 7 days:** allowed with the same approval; the trace task is only a prompt, not a guard (§7.1 "usually prompted by").

#### Must not
- Must not mark a shipment off a delivery `lost` without approval.
- Must not let the requester approve.
- Must not trace shipments on a delivery here (BL-LOG-11).

#### Done when
- [ ] Given an `at_hub` shipment last scanned 2026-11-01 10:00, the run of 2026-11-04 02:00 creates nothing; the run of 2026-11-05 02:00 creates a trace task for Depot A's operations manager.
- [ ] On 2026-11-09 02:00, still untraced, the trace task asks for `mark_lost`; when dispatcher D requests it with reason and liability *business* and operations manager M approves, the shipment is `lost` with the BL-LOG-11 effects (claim ticket opened, client notified).
- [ ] D cannot approve; a request without a reason or liability is refused; in a depot whose only operations manager is D, the company admin can approve.
- [ ] Given an `at_hub` shipment last scanned 2026-11-03, dispatcher D can request `mark_lost` on 2026-11-04 (before any trace task) and it still needs another approver.
- [ ] A scan on 2026-11-06 closes the trace task and restarts the clock.
- [ ] Running the rule twice on the same day creates no second task.

#### Out of scope
Effects of `mark_lost` and `found` (BL-LOG-11). Disposal (BL-LOG-06).

---

### BL-LOG-13 — 03 Logistics: escalate a driver who does not hand over cash (`LOG.RUL.13`)

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-PAY-17 (count, recover, write off) · BL-PAY-21 (handover escalation, `FIN.RUL.22`) · BL-DLV-08 (run abort) · BL-HRM-07 (absconding) · **Blocks:** None.
**Review items:** 03-N9-a

#### Goal
A driver who keeps the COD cash or disappears must not leave money unaccounted for, and the client
must still be paid. 12 hours after the run's return to depot (or the shift end for a run still out),
the operations manager gets a task to call the driver and abort the run; after 48 hours the company
admin is told, the operations manager counts the handover at what was received (often zero), and
the driver's HR record gets an absence note for the absconding path. The client's remittance is not
reduced.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `FIN.ACL.06`, `FIN.ACL.19`, §4.1 "When it goes wrong" (row *Driver does not hand over the cash, or disappears*), §7.1 *Cash handover* (rows `count`, `recover`, `write_off`), §7.2 `LOG.RUL.13`, §13 question 13
- `docs/modules/wl-payments.md` §5.5 step 4, `FIN.RUL.22`
- `docs/OPEN-ITEMS.md` L-20

#### Terms
- **Cash handover:** the wl-payments record of the cash a driver must hand over for a run (`open`, `counted`, `short`, `recovered`, `written_off`).
- **Absence note:** a note on the employee record that starts the HR absconding follow-up (`report_absconding` after 7 days of absence, wl-hr).
- **Shift end:** the end time of the driver's shift on the run date (wl-attendance).
- **Checked in:** a wl-attendance check-in (`HRM.ATT.01`, the depot check-in of §4.1 step 6) recorded for the driver after the clock start. Calls, messages or a scan by another user do not count.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `LOG.RUL.13` | N hours after a run's `return_to_depot`, or after the end of the driver's shift (23:59 of the run date when no shift is recorded) for a run still `dispatched`; M counted from the same start | cash handover `open`, or the driver has not returned | task for the operations manager (call the driver, abort the run); after M hours notify the company admin, and the operations manager counts the handover at what was received; if the driver has not checked in since the start (no wl-attendance check-in, `HRM.ATT.01`, recorded after it), the driver's HR record gets an absence note for the absconding path. Replaces `FIN.RUL.22` for drivers | N = 12; M = 48 | on | MVP |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `count` | open | counted, short | `cash.count` | counter ≠ driver (`FIN.ACL.05`); for a driver who does not come back (`LOG.RUL.13`), the operations manager counts what was received, often nothing | run `close`; cash moved to the depot safe; on `short`, driver and operations manager notified |
| `recover` | short | recovered | `cash.count` | recovery recorded: cash from the driver, or a salary or final-settlement deduction with approval (`FIN.ACL.19`, within the legal limits) | — |
| `write_off` | short | written_off | `cash.write_off` | `FIN.ACL.06` limit, otherwise approval | loss posted to the depot; the client's COD remittance is not reduced |

§4.1 row: "After 48 hours the operations manager aborts the run if needed and counts the handover at
what was received (often zero): the full amount is a shortage. HR follows the absconding path
(`report_absconding` after 7 days of absence, wl-hr); recovery by deduction from salary or final
dues only if allowed (`FIN.ACL.19`, §13 question 13); the rest is written off with approval
(`FIN.ACL.06`). The client is remitted in full in any case."

#### Edge cases decided
- **Relation to `FIN.RUL.22`:** in 03, `LOG.RUL.13` replaces the module rule for driver handovers (same N and M; the 03 trigger and the extra actions); BL-PAY-21 makes `FIN.RUL.22` skip them, and other 03 holders (for example counter staff) keep `FIN.RUL.22`. Only one task and one admin notice are sent per handover.
- **Clock start:** the run's `return_to_depot` time; for a run still `dispatched`, the end of the driver's shift on the run date (Asia/Dubai); when no shift is recorded, 23:59 of the run date. M is counted from the same start (not from the N task): start 2026-11-03 20:00 → task 2026-11-04 08:00 → admin notice and count request 2026-11-05 20:00.
- **Run still `dispatched` at M:** the task asks the operations manager to `abort` the run first (BL-DLV-08, reason *driver unreachable*); the count is then possible because aborting opens the handover.
- **Absence note:** written once at M hours, only if no wl-attendance check-in of the driver is recorded after the clock start. The absconding report itself is an HR decision (BL-HRM-07).
- **Salary deduction:** hidden while `FIN.ACL.19` is off (03 default, `⚠ open: lawyer (L-20)`); recovery is in cash only, the rest goes to `write_off`.
- **Write-off limit:** AED 50 in 03 (`FIN.ACL.06`); above it, approval.
- **Remittance:** unaffected in every case; the client's COD is computed from delivered shipments (BL-LOG-14).
- **Driver returns before M:** the normal count runs; the task closes; no admin notice, no absence note.

#### Must not
- Must not reduce any client's remittance because of the shortage.
- Must not deduct from salary while `FIN.ACL.19` is off.
- Must not let the driver count their own handover.

#### Done when
- [ ] Given a run returned to depot 2026-11-03 20:00 with the handover `open` (AED 1,800 expected), the operations manager gets a task at 2026-11-04 08:00.
- [ ] Given a run still `dispatched` and the driver's shift ending 2026-11-03 18:00, the task at 2026-11-04 06:00 asks to call the driver and abort the run.
- [ ] At 2026-11-05 20:00 the company admin is notified, the operations manager is asked to count, and the driver's HR record has an absence note; counting AED 0 makes the handover `short` AED 1,800.
- [ ] The client's next remittance still includes the AED 1,800 of COD on the run's delivered shipments.
- [ ] With `FIN.ACL.19` off, `recover` offers cash only; `write_off` of AED 1,800 needs approval (limit AED 50).
- [ ] A handover counted at 2026-11-04 07:00 produces no task.
- [ ] Given the driver checked in at the depot on 2026-11-04 08:30 but the handover is still `open` at 2026-11-05 20:00, the admin notice and count request are sent and no absence note is written.
- [ ] For the driver's handover, `FIN.RUL.22` creates no task and no admin notice (only `LOG.RUL.13` runs); a counter-staff handover left `open` still gets the `FIN.RUL.22` task.

#### Out of scope
Count, recount, recover and write-off mechanics (BL-PAY-17). Run abort (BL-DLV-08). Absconding
(BL-HRM-07). Remittance rule (BL-LOG-14).

---

### BL-LOG-14 — 03 Logistics: build the draft COD remittance from recorded COD, without held shipments (`LOG.RUL.06`)

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: 03 COD remittance lifecycle (`draft`, `approve`, `pay`) · **Blocks:** BL-LOG-08, BL-LOG-15, BL-LOG-16, BL-LOG-17, BL-LOG-19
**Review items:** 03-N9-a, 03-N6-c, 03-N11-a, 03-N10-a

#### Goal
The weekly draft remittance must pay the client what it is owed, whatever happened to the cash
after collection: the COD recorded on delivered shipments, less fees and differences the client
agreed to absorb, plus approved adjustment lines and, where allowed, offsets of open invoices.
Shipments under review or lost are left out until their outcome is known. A driver's shortage never
reduces the client's money.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 step 11, §6 *COD remittance*, §7.1 *COD remittance* (state `draft`, paragraph after the diagram), §7.2 `LOG.RUL.06`, §9 "COD money is the client's money" and "COD and the client's money"

#### Terms
- **COD recorded:** the COD amount of a `delivered` shipment, less a difference absorbed by the client (BL-LOG-08).
- **Fees:** the client's COD fee (and delivery fees when the contract deducts them) from the rate card.
- **Adjustment line:** a signed amount with a reason and an approver, added to a draft (BL-LOG-15).
- **Open adjustment line:** a line not yet on a `paid` remittance (§7.1 paragraph after the diagram).
- **Offset:** an open, undisputed delivery invoice deducted from the remittance when netting is allowed (BL-LOG-16).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `LOG.RUL.06` | schedule: weekly on day D | client has COD collected in the period, or open adjustment lines | create (or rebuild the client's existing `draft`) a draft COD remittance from every delivered shipment with COD recorded that is not yet on a remittance and was delivered by the period end, so shipments released from review later are picked up cumulatively (whatever the state of the drivers' cash handovers), less fees and client-absorbed COD differences, plus adjustment lines and, where the contract allows netting, offsets of open undisputed invoices capped so the net is not negative; shipments `under_review`, `lost` or marked *already remitted* left out; a net below zero from adjustment lines is set to AED 0 and the rest carried as an open adjustment line to the next draft; notify accountant | D = Monday | on | MVP |

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Collected COD for the period, minus fees, adjustments and offsets |

"A paid remittance is never edited. Any later correction (a reversed or disputed delivery, a
statement dispute, a COD difference the client agreed to absorb after the statement) is an
adjustment line on the next draft. Shipments `under_review` or `lost` are left out of the draft
until their outcome is known. … An adjustment line is *open* until the remittance carrying it is
`paid`: a line with no draft yet waits for the next weekly run, and rebuilding a `draft`
recomputes its shipment lines and offsets but keeps the adjustment lines attached to it (approved
or not)."

§4.1 step 11: "The amount owed is the COD recorded on delivered shipments, whatever happened to the
cash after collection: a driver's shortage is the company's loss, never the client's."

#### Edge cases decided
- **Which shipments:** every shipment of the client in state `delivered` with COD recorded > 0 that is not yet on a remittance, delivered on or before the period end (Sunday 23:59 Asia/Dubai for D = Monday). The selection is cumulative, so a shipment released from review after its week is picked up by the next draft.
- **Left out:** `under_review`, `lost`, and shipments whose COD is marked "already remitted" (BL-LOG-10).
- **Cash handovers:** never read; `open`, `short` or `written_off` handovers do not change the draft.
- **One draft per client per run:** running the rule twice for the same day creates no second draft; an existing `draft` for the client is rebuilt instead (shipment lines and offsets added or removed), never duplicated. Adjustment lines already attached to the draft stay attached with their approval status; open lines not yet on any draft are attached. A rebuild never removes or un-approves an adjustment line.
- **Net below zero** (adjustments and offsets larger than the COD): offsets are capped so they never make the net negative (BL-LOG-16); a negative net from adjustment lines leaves the remittance at AED 0 and carries the rest as an open adjustment line to the next draft; the accountant is told.
- **Rounding:** each line in AED with 2 decimals; the net is the sum of the rounded lines.
- **Blocked client:** the draft is still created; payment waits (BL-LOG-19).

#### Must not
- Must not reduce the client's COD for a driver shortage, a written-off shortage or an absconding driver.
- Must not include `under_review` or `lost` shipments.
- Must not put one shipment on two remittances.

#### Done when
- [ ] Given client C with 3 `delivered` shipments in the week (COD AED 200, 300, 500), a COD fee of 2% and one driver handover `short` by AED 300, the Monday draft shows COD AED 1,000, fees AED 20 and net AED 980, and the accountant is notified.
- [ ] Given a fourth shipment `under_review` (COD AED 150), it is not on the draft; after `close_review` on Wednesday it is on the next Monday's draft.
- [ ] Given a shipment with AED 50 absorbed by the client, its line shows AED 250 of AED 300 with the reason.
- [ ] Running the rule twice on Monday leaves one draft per client.
- [ ] Given a `draft` with an approved +AED 70 adjustment line and a shipment released from review on Tuesday, the next rebuild adds the shipment line and the +AED 70 line stays, still approved.
- [ ] Given a client with no COD but an open adjustment line of −AED 40, a draft is created with net AED 0 and the −AED 40 carried forward; the accountant is told.

#### Out of scope
Adjustment lines (BL-LOG-15). Offsets (BL-LOG-16). Payment and returned transfers (BL-LOG-17).
Blocked clients (BL-LOG-19).

---

### BL-LOG-15 — 03 Logistics: add approved adjustment lines to a COD remittance (`LOG.ACL.12`)

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-LOG-14 · BL-KRN-01 (approval returned) · **Blocks:** BL-LOG-10, BL-LOG-18
**Review items:** 03-N6-c

#### Goal
A paid remittance is never edited. A correction (an error found after payment, a reversed or
disputed delivery, a statement dispute outcome, a COD difference the client agreed to absorb after
the statement) is an adjustment line, positive or negative, with a reason, on the client's next
draft. One user prepares it, another with `remittances.approve` approves it, and the client sees the
line and its reason on the next statement.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `LOG.ACL.12`, §4.1 "When it goes wrong" (row *Remittance wrong after payment*), §6 *COD remittance* (adjustment lines), §7.1 *COD remittance* (row `approve`, paragraph after the diagram)

#### Terms
- **Adjustment line:** amount (positive: owed to the client; negative: taken back), reason, source (manual, delivery reversal, statement dispute, COD difference), the shipment or remittance it corrects, preparer, approver.
- **Statement:** the PDF published to the client portal when a remittance is paid.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `LOG.ACL.12` | An adjustment line on a COD remittance (a correction after payment, a reversed or disputed delivery, a statement dispute outcome) is prepared by one user and approved by another with `remittances.approve`; a user may approve both a line and the remittance if they prepared neither; the client sees the line and its reason on the next statement | `SOD` | accountant | on |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `approve` | draft | approved | `remittances.approve` | approver ≠ preparer; adjustment lines approved (`LOG.ACL.12`) | — |

§4.1 row *Remittance wrong after payment*: "Never edited: an adjustment line (positive or negative,
with reason) goes on the next remittance, approved by a second user (`LOG.ACL.12`), and the client
sees it on the statement."

#### Edge cases decided
- **Preparer:** the user who creates the line; for system-drafted lines (reversal, BL-LOG-10) the user who confirms the draft line is the preparer. The approver must differ from the preparer.
- **Lines and the remittance approval are separate:** a line is approved on its own; the remittance `approve` is refused while any line on it is unapproved. The same user may approve a line and the remittance as long as they did not prepare that line or the remittance.
- **No draft yet:** the line is created "open" for the client and waits; the next weekly run of `LOG.RUL.06` (day D) attaches it. An open line is enough for that run to create a draft for a client with no COD in the period (the rule's condition "or open adjustment lines", BL-LOG-14); the line itself starts no run and no draft before day D.
- **Rejected or returned line:** it stays unapproved (BL-KRN-01); the preparer edits and resubmits, or deletes it while unapproved.
- **Approved line edited:** not allowed; a wrong line is corrected by another line.
- **Line on an `approved` remittance:** the remittance must be `reopen`ed to draft first.
- **Statement:** each line shows date, amount, reason and the shipment or statement it corrects; internal notes are not shown.

#### Must not
- Must not edit a paid remittance or its statement.
- Must not let the preparer approve their own line.
- Must not approve a remittance with an unapproved line.

#### Done when
- [ ] Given remittance R-10 paid with a COD line wrong by AED 70, accountant A adds a +AED 70 line with reason "COD under-remitted on S-2001"; accountant B approves it; the next draft shows +AED 70 and the next statement shows the line and reason.
- [ ] A cannot approve the line; the remittance with the unapproved line cannot be approved.
- [ ] When B rejects the line, it stays unapproved and is not counted.
- [ ] R-10 cannot be edited.

#### Out of scope
Drafting the reversal line (BL-LOG-10). Dispute outcome (BL-LOG-18). Draft rule (BL-LOG-14).

---

### BL-LOG-16 — 03 Logistics: net open invoices against COD in the remittance

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-LOG-14 · Base: wl-payments Payment lifecycle (record and allocate, `FIN.PAY.01`) · **Blocks:** None.
**Review items:** 03 MEDIUM "COD netting against invoices"

#### Goal
Many clients owe delivery invoices while the courier holds their COD. When the client's contract
allows netting, the draft remittance deducts open, undisputed delivery invoices; when the remittance
is paid, each offset is recorded as a payment allocated to its invoice. A disputed invoice is never
netted.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 "When it goes wrong" (row *Client owes invoices*), §6 *COD remittance* (invoice offsets), §7.1 *COD remittance* (rows `pay`, `return_payment`), §7.2 `LOG.RUL.06`

#### Terms
- **Netting allowed:** a flag on the client's contract (rate card agreement); off unless set.
- **Open invoice:** an issued delivery invoice of the client with a balance > 0, not `disputed`, not `written_off`.

#### Spec excerpt (authoritative for this issue)

§4.1 row *Client owes invoices*: "When the client's contract allows netting, the draft remittance
offsets open, undisputed delivery invoices; on `pay` the offsets are recorded as payments allocated
to those invoices — `LOG.COD.01`, `FIN.PAY.01`."

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `pay` | approved | paid | `remittances.pay` | payment reference recorded; client not blocked, or an exception approved (`PTY.ACL.03`); bank account approved (`LOG.ACL.13`); each invoice offset re-checked: one that became `disputed`, paid or written off sends the remittance back to `draft` with a message, never a silently smaller payment | statement PDF published to the client portal; invoice offsets recorded as payments allocated to those invoices |
| `return_payment` | paid | payment_returned | `remittances.pay` | bank return advice attached | client told and asked to confirm its bank account; offsets stay allocated |

#### Edge cases decided
- **Order:** oldest due date first; an invoice may be offset in part (the last one).
- **Cap:** total offsets never exceed the remittance's amount before offsets (net ≥ 0).
- **Invoice changes between draft and pay:** at `pay`, each offset is re-checked; an invoice that became `disputed`, paid or written off meanwhile is removed and the remittance goes back to `draft` with a message (the accountant re-approves): re-check at `pay`, never silently pay a smaller amount.
- **Payment record:** one payment per invoice, method *COD netting*, date = the remittance payment date, reference = the remittance number; allocated in full to that invoice.
- **Returned transfer:** the offsets stay allocated (spec); only the cash part is owed again (BL-LOG-17).
- **Statement:** lists each offset invoice with number and amount.

#### Must not
- Must not net a disputed invoice.
- Must not net when the contract does not allow it.
- Must not record the offset payments before `pay`.

#### Done when
- [ ] Given netting allowed, COD net AED 2,000 and open invoices INV-1 (AED 500, due 2026-10-01) and INV-2 (AED 1,800, due 2026-11-01), the draft offsets AED 500 and AED 1,500 (part of INV-2) and the net to pay is AED 0.
- [ ] Given INV-1 `disputed`, it is not offset.
- [ ] On `pay`, two payments (AED 500 and AED 1,500, method *COD netting*) are allocated to INV-1 and INV-2.
- [ ] Given INV-2 disputed after approval, `pay` does not pay: the remittance goes back to `draft` without INV-2's offset, the accountant sees the message "offset INV-2 removed: invoice disputed", no payment and no statement are recorded, and it must be approved again before `pay`.
- [ ] Given netting not allowed, no offsets appear.

#### Out of scope
Payment allocation mechanics (wl-payments). Returned transfers (BL-LOG-17).

---

### BL-LOG-17 — 03 Logistics: handle a returned remittance transfer and control bank account changes (`LOG.ACL.13`)

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-LOG-14 · BL-KRN-01 (approval returned) · **Blocks:** None.
**Review items:** 03-N6-c

#### Goal
A remittance transfer can come back from the bank (closed account, wrong IBAN). The paid
remittance moves to `payment_returned`: the amount is owed to the client again and the client is
asked to confirm its bank account. A change to a client's remittance bank account takes effect only
after a second user approves it and the client confirms it from its known contact, so a fraudulent
change cannot divert the client's money. The remittance is then repaid.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `LOG.ACL.13`, §4.1 "When it goes wrong" (row *Bank returns the remittance transfer*), §7.1 *COD remittance* (states `paid`, `payment_returned`; rows `pay`, `return_payment`, `repay`)

#### Terms
- **Bank return advice:** the bank's document saying the transfer was returned, with the reason.
- **Known contact:** the client's contact on record before the change request (e-mail or phone), not one given in the request.
- **Pending account:** the new bank account while it waits for approval and confirmation.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `paid` | success | no | Paid to the client; statement in the portal |
| `payment_returned` | error | no | The bank returned the transfer; the amount is owed to the client again |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `pay` | approved | paid | `remittances.pay` | payment reference recorded; client not blocked, or an exception approved (`PTY.ACL.03`); bank account approved (`LOG.ACL.13`); each invoice offset re-checked (BL-LOG-16) | statement PDF published to the client portal; invoice offsets recorded as payments allocated to those invoices |
| `return_payment` | paid | payment_returned | `remittances.pay` | bank return advice attached | client told and asked to confirm its bank account; offsets stay allocated |
| `repay` | payment_returned | paid | `remittances.pay` | new payment reference; client not blocked, or an exception approved (`PTY.ACL.03`); a changed bank account approved (`LOG.ACL.13`), or, for the same account, the client's confirmation that it is correct recorded | statement updated |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `LOG.ACL.13` | A change to a client's remittance bank account takes effect only after a second user approves it and the client confirms it from its known contact (the primary contact on record before the request, never a contact added or changed in the same request); until then remittances use the old account or wait. One pending change per client at a time; a pending change not both approved and confirmed within 14 days lapses and the requester is told | `SOD` | accountant | on; lapses after 14 days |

#### Edge cases decided
- **Account change flow:** the requester (accountant or a client user in the portal) enters the new account; a second staff user with `remittances.approve` approves; the client confirms through a link sent to the known contact. Both are needed, in either order; the account becomes active when the second of the two happens.
- **Requested by a client user:** the client user is the requester; the staff approval and the known-contact confirmation are still needed. The confirmation link goes to the client's primary contact, never to the requesting user's own e-mail if it is the only contact changed in the same request.
- **Until active:** remittances use the old account; when the old account is the one returned, `repay` waits.
- **Repay to the same account:** allowed when the client confirms the account is correct (for example a temporary bank issue); the guard is met by that confirmation recorded on the remittance; "changed bank account approved" applies only when the account changed.
- **Repeated return:** `repay` → `paid` → `return_payment` again is allowed; each return keeps its advice.
- **Offsets:** stay allocated; only the cash amount is owed again.
- **Statement:** `repay` re-publishes the statement with the new payment reference and date; the earlier version stays in the history.
- **Rejected approval:** the pending account is discarded (BL-KRN-01); the requester is told.
- **Lapse:** a pending account approved but not confirmed, or confirmed but not approved, within 14 days of the request lapses: it is discarded, the open approval request is withdrawn, the confirmation link stops working, and the requester is told. The old account stays.
- **Second change while one is pending:** refused ("a bank account change is already pending"); the requester may withdraw the pending one first.

#### Must not
- Must not pay to a pending account.
- Must not let the requester approve the account change.
- Must not take the client's confirmation from a contact added in the same request.

#### Done when
- [ ] Given remittance R-12 `paid` AED 4,000 and a bank return advice attached, `return_payment` makes it `payment_returned` and the client is asked to confirm its bank account.
- [ ] Given accountant A enters a new IBAN, it is pending; after accountant B approves and the client confirms from its known e-mail, it is active. A cannot approve.
- [ ] `repay` with a new reference before the account is active is refused; after activation it makes R-12 `paid` and the statement shows the new reference.
- [ ] Given only B's approval and no client confirmation, the next remittance's `pay` uses the old account.
- [ ] Given a change requested 2026-11-02 and approved but not confirmed by 2026-11-16, it lapses: the confirmation link is refused and A is told; a second request made on 2026-11-05 while the first was pending was refused.

#### Out of scope
Blocked clients (BL-LOG-19). Netting (BL-LOG-16).

---

### BL-LOG-18 — 03 Logistics: flag and resolve a COD statement dispute

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-LOG-15 · Base: wl-ticketing Ticket lifecycle · **Blocks:** None.
**Review items:** 03-N6-c

#### Goal
A client may contest a COD statement. The remittance gets a `disputed` flag and a ticket for the
accountant; nothing pauses, because the next remittance is due anyway. The outcome is *upheld* (the
statement stands) or *adjusted* (an adjustment line on the next draft), and the client is told.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 "When it goes wrong" (row *Client disputes a COD statement*), §6 *COD remittance* (dispute flag and ticket), §7.1 *COD remittance* (paragraph on the `disputed` flag; rows `dispute`, `resolve_dispute`)

#### Terms
- **`disputed` flag:** a field on the remittance, not a state.

#### Spec excerpt (authoritative for this issue)

"The `disputed` flag (not a state) marks a statement the client contests: a ticket is linked and
customer service answers it; it pauses nothing, because the next remittance is due anyway."
"`dispute` is not offered while a transfer is returned (`payment_returned`); the client's question
is a normal ticket."

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `dispute` | approved, paid | approved, paid | client user in the portal or customer service | reason given | `disputed` set; ticket opened for the accountant |
| `resolve_dispute` | approved, paid | approved, paid | `remittances.approve` | outcome *upheld* or *adjusted* with a note | `disputed` cleared; *adjusted*: an adjustment line on the next draft remittance (`LOG.ACL.12`); client told of the outcome |

#### Edge cases decided
- **Portal:** a client user sees only `paid` remittances (the statement is published at `pay`), so the portal offers `dispute` on `paid` only; customer service may also flag an `approved` one on the client's call.
- **Already disputed:** no second flag; a new message is added to the open ticket.
- ***Adjusted*:** the resolver enters the amount and reason; the line is created on the next draft as prepared by the resolver and needs a second approver (BL-LOG-15).
- **`payment_returned`:** `dispute` is not offered while the transfer is returned; the client's question is a normal ticket for customer service.
- **Nothing pauses:** `pay`, `LOG.RUL.06` and the next drafts run as usual.

#### Must not
- Must not edit the disputed remittance.
- Must not pause remittances because of the flag.

#### Done when
- [ ] Given remittance R-14 `paid`, when the client user disputes with reason "shipment S-3001 missing", R-14 has `disputed` set and the accountant has a ticket.
- [ ] When accountant B resolves *adjusted* with +AED 150, the flag clears, a +AED 150 line is on the next draft awaiting another user's approval, and the client is told.
- [ ] Resolving *upheld* clears the flag with no line and tells the client.
- [ ] While R-14 is disputed, the next Monday's draft is still created.

#### Out of scope
Adjustment line approval (BL-LOG-15). Invoice disputes (BL-INV-04).

---

### BL-LOG-19 — 03 Logistics: refuse new bookings and hold remittance payment for a blocked client

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-PTY-01 (block) · BL-PTY-04 (no new agreements, no money out) · BL-POR-01 (neutral portal text) · BL-LOG-14 · **Blocks:** None.
**Review items:** X-18

#### Goal
When a client is blocked (for example non-payment or a legal dispute), it cannot book new shipments
or open new freight jobs, and the message it sees is neutral. Shipments already picked up are
delivered or returned as normal, and open freight jobs continue. COD remittances are drafted but not
paid until the client is unblocked or an exception is approved. Invoices and receipts continue.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 `PTY.ACL.03`, §4.1 "When it goes wrong" (row *Client blocked*), §7.1 *COD remittance* (row `pay`), checklist rows *Shipment*, *COD remittance*, *Freight job* (column *Party blocked*)
- `docs/modules/wl-parties.md` §5.6

#### Terms
- **Blocked client:** a client party in state `blocked` (wl-parties).
- **Exception:** a one-off approval to pay a blocked party (`PTY.ACL.03`).

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PTY.ACL.03` *(shared)* | While a client is blocked, no COD remittance or refund is paid to it and no rate card is renewed or extended, unless the company admin approves an exception (03 has no finance manager role; the requester cannot approve; screening is off in 03) | `STA`, `LIM` | accountant | — |

§4.1 row *Client blocked*: "New bookings are refused with a neutral message; shipments booked before
the block may still be picked up (the dispatcher sees "client on hold"); shipments already picked
up are delivered or returned as normal; COD remittances wait for unblock or an approved
exception (`PTY.ACL.03`); invoices and receipts continue — wl-parties §5.6."

`pay` guard (part): "client not blocked, or an exception approved (`PTY.ACL.03`)".

Checklist: Shipment — "new bookings refused; shipments in progress finish"; COD remittance — "`pay`
waits (`PTY.ACL.03`)"; Freight job — "new jobs refused; open jobs continue".

#### Edge cases decided
- **Where bookings are refused:** portal booking, CSV import (every row refused with the neutral text), and staff booking for that client. The text is the neutral portal text (BL-POR-01), never the reason.
- **`created` shipments booked before the block:** they may still be picked up (the booking existed before the block, §4.1 row *Client blocked*); the dispatcher sees "client on hold" on the pickup list. There is no other exception: a shipment is not an agreement, so the wl-parties block effect per agreement kind (§5.6 there) does not apply to it. The client may still `cancel` them before pickup.
- **Exception approver:** the company admin (03 has no finance manager role). The approver must differ from the requester.
- **Rate card renewal or extension:** refused while blocked unless the same exception is approved.
- **Drafts:** `LOG.RUL.06` still drafts and `approve` still runs; only `pay` and `repay` wait.
- **Unblock:** waiting remittances become payable with no other change.

#### Must not
- Must not show the block reason to the client or in the refusal text.
- Must not stop deliveries of shipments already picked up.
- Must not pay or repay a blocked client's remittance without an approved exception.

#### Done when
- [ ] Given client C blocked for *non-payment*, a portal booking and a CSV upload are refused with the neutral text; the reason is not shown.
- [ ] C's `picked_up` shipments are dispatched and delivered as normal; a `created` shipment booked before the block can be picked up and shows "client on hold" on the pickup list.
- [ ] C's Monday draft is created and can be approved; `pay` is refused with "client on hold".
- [ ] With an exception approved by the company admin, `pay` succeeds once; the accountant who requested the exception cannot approve it.
- [ ] Opening a new freight job for C is refused; C's open freight jobs continue.

#### Out of scope
Block and unblock (BL-PTY-01, BL-PTY-02). Portal text (BL-POR-01). Screening (off in 03).

---

### BL-LOG-20 — 03 Logistics: leave held and lost shipments out of the monthly invoice and add surcharges (`LOG.RUL.07`)

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** Base: wl-invoicing usage billing (`FIN.USG.01`) and rate cards (`FIN.RAT.01`) · BL-LOG-04 (re-weigh surcharges and their hold) · **Blocks:** BL-LOG-11
**Review items:** 03-N10-a, 03-N11-a, 03 MEDIUM "re-weigh surcharge"

#### Goal
The monthly invoice must bill only what the client owes: delivered shipments with their re-weigh
surcharges and return charges, and never shipments that are under review, lost, voided, or
disposed unless the client caused the disposal.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 step 12, §4.4 step 4, §7.2 `LOG.RUL.07`

#### Terms
- **Return charge:** recorded on the shipment when its return starts (`return`, `cancel_return`, or `create_return` for a linked return shipment), and billed once the shipment is `returned` (`return_delivered`).
- **Client-caused disposal:** a disposed shipment with liability *client* (BL-LOG-06).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `LOG.RUL.07` | schedule: monthly on day D | client has billable shipments in the previous month (delivered, returned to it, or disposed when client-caused) | create draft invoice from the rate card, with re-weigh surcharges and return charges (a return charge is recorded when the return starts, at `return`, `cancel_return` or `create_return`, and billed in the month of `return_delivered`); shipments `under_review`, `lost`, `voided` or `disposed` (unless client-caused) left out | D = 1 | on | MVP |

§4.4 step 4: "A shipment marked lost is left out of the monthly invoice; if it was already invoiced,
its charge is credited too."

#### Edge cases decided
- **Month:** the previous calendar month in Asia/Dubai, by the delivery (or return, or disposal) date.
- **Shipments delivered in a month but `under_review` on day 1:** left out; billed on the first monthly draft after `close_review` (cumulative selection of shipments not yet billed).
- **Returned shipments:** the return charge, recorded when the return started (BL-LOG-02, BL-LOG-05, BL-LOG-07), is billed in the month of `return_delivered` (the e-POD from the client). A shipment still `returning` at the month end is billed in the month it reaches `returned`; one lost while returning is not billed (`lost`). A client with only returns and no deliveries in the month still gets a draft (the rule's condition is billable shipments).
- **Unbilled surcharges:** a re-weigh surcharge recorded after its shipment was invoiced goes on the next draft (BL-LOG-04).
- **Re-weigh surcharge held by an open re-weigh ticket:** left out until the ticket is resolved (BL-LOG-04).
- **Disposed, client-caused:** delivery charge and disposal charge billed.
- **Idempotency:** a shipment is billed once; running the rule twice leaves one draft per client.

#### Must not
- Must not bill a shipment `under_review`, `lost` or `voided`.
- Must not bill a disposal the company caused.

#### Done when
- [ ] Given client C's October: 100 shipments delivered (3 with re-weigh surcharges totalling AED 24), 2 returned, 1 `under_review`, 1 `lost`, 1 `voided`, 1 disposed with liability *business*, the 1 November draft has 100 delivery lines, AED 24 of surcharges and 2 return charges, and none of the other 4.
- [ ] After the `under_review` shipment's review is closed on 2026-11-10, it is on the 1 December draft.
- [ ] Given a `cancel_return` on 2026-10-29 and `return_delivered` on 2026-11-02, the return charge is not on the 1 November draft and is on the 1 December draft.
- [ ] Given a disposed shipment with liability *client*, its delivery and disposal charges are billed.
- [ ] Running the rule twice on 1 November leaves one draft for C.

#### Out of scope
Credit notes for already-invoiced lost or reversed shipments (BL-LOG-10, BL-LOG-11). Re-weigh
recording (BL-LOG-04).

---

### BL-LOG-21 — 03 Logistics: hold and release a freight job, keeping the state before the hold

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: 03 Freight job lifecycle · **Blocks:** None.
**Review items:** 03-N7-c

#### Goal
A freight job can stop for reasons outside the company's control: a customs hold or inspection, a
rejected declaration, unpaid duty, missing documents, the customs platform down, or the client's
instruction. The documentation officer puts the job `on_hold` with a reason; the job remembers where
it was, its milestone clock stops, the client and the documentation officer are told and the reason
shows in the portal. `release` returns the job to exactly where it was.

#### Files to read
- `docs/verticals/03-logistics.md` §4.2 "When it goes wrong" (rows *Customs hold…*, *Mirsal or ATLP down*), §6 *Freight job* (hold reason), §7.1 *Freight job* (state `on_hold`, rows `hold`, `release`), §7.2 `LOG.RUL.09`, §8 *Integration down* paragraph

#### Terms
- **State before the hold:** `in_transit`, `customs_clearance` or `ready_for_delivery`, stored on the job at `hold`.
- **Milestone clock:** the time measured against the job's milestones and ETA (`LOG.RUL.09` alert, on-time reports).
- **Mirsal / ATLP:** the Dubai and Abu Dhabi customs platforms; filed by hand in the MVP (no integration).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `on_hold` | warning | no | Stopped: customs hold or inspection, declaration rejected, duty unpaid, documents missing, customs platform down, client instruction |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `hold` | in_transit, customs_clearance, ready_for_delivery | on_hold | `jobs.manage` | reason given | client and documentation officer notified; reason shown in the portal; the state before the hold is kept |
| `release` | on_hold | in_transit, customs_clearance, ready_for_delivery | `jobs.manage` | hold reason resolved (e.g. inspection passed, declaration accepted, duty paid) | back to the state before the hold; client notified |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `LOG.RUL.09` | freight job ETA date reached | job not `ready_for_delivery` and not `on_hold` (hold periods are stored and left out of on-time measures; after `release` the alert applies to the ETA as recorded) | notify documentation officer | — | on | MVP |

§4.2 rows:
- "Customs hold, inspection, declaration rejected, duty unpaid, documents missing — Freight job
  `hold` with a reason: the job is `on_hold` and its milestone clock stops; the client and the
  documentation officer are told; the reason is shown in the portal. `release` returns the job to
  where it was."
- "Mirsal or ATLP down — Declarations are filed on the portal or by the clearing agent anyway (there
  is no integration in the MVP); a hold with reason *customs platform down* records the delay for
  the client."

#### Edge cases decided
- **Reason list and portal labels** (EN; AR labels are translations of the same text): *customs hold or inspection* → "Customs hold or inspection"; *declaration rejected* → "Customs declaration rejected"; *duty unpaid* → "Customs duty unpaid"; *documents missing* → "Documents missing"; *customs platform down* → "Customs system unavailable"; *client instruction* → "On hold at your request". Plus a free-text note. The portal shows the label and the note.
- **Release guard:** the user records how the reason was resolved (a short note, optionally a document); `release` is refused without it.
- **Target of `release`:** always the stored state; the user cannot choose another.
- **Hold while on hold:** not offered; a second reason is added as a note on the open hold.
- **Milestone clock:** `LOG.RUL.09` does not alert while the job is `on_hold`; the hold periods are stored (start, end, reason) and excluded from on-time measures; after release the ETA alert applies again to the ETA date as recorded.
- **Costs during the hold** (storage, demurrage) are allocated normally (BL-LOG-24).
- **`booked` and `documents_pending`:** not holdable (documents missing before departure is `documents_pending`).
- **Concurrency:** the stored state is written in the same transaction as the state change.

#### Must not
- Must not let `release` go to a state other than the stored one.
- Must not hide the hold reason from the client (it is shown in the portal).
- Must not raise `LOG.RUL.09` alerts for a job on hold.

#### Done when
- [ ] Given a job in `customs_clearance`, when the documentation officer holds it with reason *customs hold or inspection* and the note "physical inspection at Jebel Ali", it is `on_hold`, the stored state is `customs_clearance`, the client and documentation officer are notified, and the portal shows "Customs hold or inspection" with the note.
- [ ] `release` with the note "inspection passed 2026-11-06" returns it to `customs_clearance` and notifies the client; without a note it is refused.
- [ ] Given a job in `in_transit` held with reason *customs platform down*, its ETA date passing while on hold raises no `LOG.RUL.09` alert.
- [ ] A job in `booked` cannot be held.

#### Out of scope
Container free time (BL-LOG-24). Customs system integration (Later).

---

### BL-LOG-22 — 03 Logistics: reopen a closed freight job for a late cost

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** Base: 03 Freight job lifecycle (`close`) · Base: wl-invoicing job costing (`FIN.CST.01`), supplier bills (`FIN.PAB.01`) and disbursements (`FIN.DSB.01`) · **Blocks:** BL-LOG-24
**Review items:** 03 MEDIUM "late costs on closed jobs"

#### Goal
Shipping lines and ports often bill weeks after a job closed. The closed job is reopened with a
reason, the late cost is allocated, a supplementary invoice is issued when the cost is rechargeable,
and the job is closed again with the new margin.

#### Files to read
- `docs/verticals/03-logistics.md` §4.2 "When it goes wrong" (row *Supplier bill arrives after the job is closed*), §7.1 *Freight job* (state `closed`, rows `close`, `reopen`), §9 (VAT, disbursements)

#### Terms
- **Late cost:** a supplier bill or disbursement for the job received after `close`.
- **Supplementary invoice:** a new invoice to the client for rechargeable late costs, referencing the job and the first invoice.
- **Job margin:** revenue − costs on the job (`FIN.CST.01`).

#### Spec excerpt (authoritative for this issue)

§9 *VAT* (part): "Each charge line on a freight job carries its VAT category. `zero_rated` can be
chosen only when the job is marked international with the client contracted for the whole move,
and the invoice cannot be issued until the evidence … is attached." §9 *Disbursements* (part):
"When all are met it is billed as `out_of_scope` in a separate invoice section …; otherwise it is
billed as a reimbursement with the VAT of the main service (`FIN.DSB.01`)."

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `closed` | success | no | All costs recorded and invoiced; reopened only for a late cost |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `close` | delivered | closed | `jobs.close` | all supplier bills and disbursements allocated; invoice issued | job margin final |
| `reopen` | closed | delivered | `jobs.close` | reason (late supplier bill, demurrage, port storage, or *other* with a note) | late cost allocated; supplementary invoice when rechargeable; margin recalculated at the next `close` |

§4.2 row: "The job is reopened (`reopen`, reason: late supplier bill, demurrage, port storage, or
*other* with a note), the cost allocated, a supplementary invoice issued
if rechargeable, and the job closed again with the new margin."

#### Edge cases decided
- **Reason list:** late supplier bill, demurrage, port storage, plus *other* with a note.
- **Rechargeable:** decided per cost line by the user (client liability per the quotation terms, BL-LOG-24); a non-rechargeable cost only lowers the margin.
- **Supplementary invoice VAT** (§9, the same rule as BL-LOG-24): each recharged late cost is either a disbursement, `out_of_scope` in the disbursement section, when the three VATP013 conditions are recorded (`FIN.DSB.01`), or otherwise a reimbursement with the VAT category of the job's main service: 5% for a domestic job, or a job not marked international with the client contracted for the whole move; zero-rated only for an international job with its evidence attached.
- **Margin history:** the margin at each `close` is kept; reports show the latest.
- **Several reopenings:** allowed; each has its reason.
- **Close again:** the same guards; the supplementary invoice must be issued if one was drafted.
- **Period:** the supplementary invoice is dated on its issue date; the first invoice is never changed.

#### Must not
- Must not edit the first invoice.
- Must not reopen a `cancelled` job.
- Must not reopen without a reason.

#### Done when
- [ ] Given a job closed 2026-10-20 with margin AED 3,000, when a shipping line bill of AED 800 for demurrage arrives on 2026-11-15, the documentation officer reopens it with reason *demurrage*; the job is `delivered`.
- [ ] Given the job is not marked international (clearance and local delivery only, main service at 5%), the AED 800 is allocated with liability *client*; a supplementary invoice of AED 800 + AED 40 VAT (a reimbursement with the main service's 5%, VATP013 conditions not met) is drafted and issued; with the three conditions recorded it is AED 800 `out_of_scope` in the disbursement section.
- [ ] On `close`, the margin is AED 3,000 (revenue +800, cost +800) and the earlier margin stays in the history.
- [ ] With liability *company*, no supplementary invoice and the new margin is AED 2,200.

#### Out of scope
Free-time alerts and the liability decision (BL-LOG-24).

---

### BL-LOG-23 — 03 Logistics: cancel a freight job before departure with charges and incurred costs

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** Base: 03 Freight job lifecycle · Base: wl-invoicing quotations (`FIN.QUO.01`) · **Blocks:** None.
**Review items:** 03 freight job checklist (WORKFLOW §8 "cancel after work started")

#### Goal
A client may cancel a freight move before it departs. The job is cancelled with a reason; the
cancellation charges in the quotation are invoiced, and costs already incurred (booking fees,
documents) are recharged per the quotation's terms. After departure a job cannot be cancelled; it is
held or completed.

#### Files to read
- `docs/verticals/03-logistics.md` §4.2 "When it goes wrong" (row *Client cancels before departure*), §7.1 *Freight job* (row `cancel`), checklist row *Freight job*

#### Terms
- **Cancellation charges:** amounts the accepted quotation states for cancellation: one fixed amount, or one amount per stage. The stage is the job's state at `cancel`: `booked` or `documents_pending`.
- **Incurred cost:** a supplier bill or disbursement already allocated to the job.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | booked, documents_pending | cancelled | `jobs.manage` | reason given | cancellation charges per quotation and rechargeable incurred costs drafted on one invoice; no cancellation terms: no charge, task for the documentation officer; costs billed later are allocated to the cancelled job and the accountant decides whether to invoice them |

§4.2 row: "Client cancels before departure — `cancel` with cancellation charges per the quotation;
costs already incurred are recharged per its terms. A quotation without cancellation terms drafts
no charge, and the documentation officer gets a task to decide the incurred costs. A cost billed
after `cancel` is allocated to the cancelled job (it stays `cancelled`) and the accountant decides
whether to invoice it."

§9 *VAT* (part): "Cancellation charges of a freight job cancelled before departure are standard
rated (5%): no transport took place, so there is no evidence for zero rating."

Checklist (Freight job): "no (`cancel` before departure)" for void; "`hold`, `reopen` for late
costs" for cancel after work started.

#### Edge cases decided
- **Invoice:** one invoice for the cancellation charges and the rechargeable incurred costs, drafted at `cancel`; when the quotation has no cancellation terms, no charge is drafted and the documentation officer is told to decide the incurred costs by hand.
- **Costs that arrive after `cancel`:** the job is terminal; the late bill is allocated to the cancelled job's cost record (the job stays `cancelled`) and the accountant decides whether to invoice it.
- **Stage:** taken from the job's state when `cancel` runs; a quotation with per-stage charges but no amount for that stage is treated as having no cancellation terms (no charge, task).
- **VAT:** the cancellation charge is standard rated at 5% (§9: no transport took place, so there is no evidence for zero rating). A rechargeable incurred cost is a reimbursement at 5% unless it is a disbursement with the VATP013 conditions recorded (`out_of_scope`).
- **Disbursements:** recharged at exact cost, `out_of_scope` only with the VATP013 conditions recorded.
- **After `depart`:** `cancel` is not offered; the client's instruction is a `hold` (BL-LOG-21) with reason *client instruction*.

#### Must not
- Must not cancel a job that has departed.
- Must not drop incurred costs silently: they are either recharged or shown as the job's loss.

#### Done when
- [ ] Given a `documents_pending` job whose quotation has a cancellation charge of AED 500 and an allocated booking fee of AED 200 (rechargeable), `cancel` with reason "client cancelled order" makes it `cancelled` and drafts one invoice of AED 500 + AED 200 + AED 35 VAT (5% on both, booking fee without VATP013 conditions) = AED 735.
- [ ] Given a quotation with per-stage charges (`booked` AED 300, `documents_pending` AED 500) and a `booked` job, `cancel` drafts AED 300 + 5% VAT.
- [ ] Given an `in_transit` job, `cancel` is not offered.
- [ ] Given no cancellation terms in the quotation, `cancel` drafts no charge and the documentation officer gets a task.

#### Out of scope
Holds (BL-LOG-21). Late costs on closed jobs (BL-LOG-22).

---

### BL-LOG-24 — 03 Logistics: alert on container free time and allocate demurrage by liability (`LOG.RUL.14`)

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: 03 Freight job lifecycle (containers) · Base: wl-invoicing supplier bills (`FIN.PAB.01`) and disbursements (`FIN.DSB.01`) · BL-LOG-22 (supplementary invoice after close) · **Blocks:** None.
**Review items:** 03-N5-a

#### Goal
Ports and shipping lines charge per day once a container's free time ends (port storage,
demurrage, detention). Each container records its free-time end dates; a daily rule warns the
documentation officer and the client 2 days before and on the day, and after the date adds a daily
line to the job's free-time log so the later supplier bill can be checked. When the bill comes, the
daily charges are allocated to the job with a liability: the client (recharged per the quotation)
or the company (absorbed in the margin).

#### Files to read
- `docs/verticals/03-logistics.md` §4.2 "When it goes wrong" (row *Container free time running out*), §6 *Freight job* (containers with free-time end dates), §7.2 `LOG.RUL.14`, §9 *Disbursements*

#### Terms
- **Port storage:** charged by the port for a container left at the terminal after its free days.
- **Demurrage:** charged by the shipping line for a full container kept in the port after its free days.
- **Detention:** charged by the shipping line for a container kept outside the port (not returned empty) after its free days.
- **Free-time log:** a list of daily lines on the job (container, charge type, date) past the end date.
- **VATP013 conditions:** third-party invoice in the client's name, the client's authorisation, exact amount shown separately; only then is a recharge a disbursement outside VAT.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `LOG.RUL.14` | daily | a container on an open freight job (any state but `closed` and `cancelled`) is N days before a free-time end date (port storage, demurrage, detention), on that date, or past it and not yet collected or returned empty | N days before and on the date: notify the documentation officer and the client, once each per container, charge type and end date (an extended end date gets its own notices); past the date: no notice, add a daily line to the job's free-time log so the later supplier bill can be checked | N = 2 | on | MVP |

§4.2 row: "Each container records its free-time end dates. `LOG.RUL.14` alerts the documentation
officer and the client 2 days before and on the day, then only logs each day past it. Daily
charges, when billed by the shipping line or port, are allocated to the job as costs with a
liability, required before the bill is approved for the job: *client* (late documents, late duty
payment, late collection by the client) recharged per the quotation terms, as a disbursement only
when the VATP013 conditions are met [S17]; *company* (company delay, other) absorbed in the job
margin."

#### Edge cases decided
- **When notices go out:** on the day that is 2 days before the end date and on the end date itself, once each per container and charge type; no notices after the end date, only log lines.
- **Run time:** daily at 06:00 Asia/Dubai; dates are Asia/Dubai calendar dates. The run on date X writes the log line for X when the container was not recorded as collected or returned empty before the run (the port and the line charge the day of collection too).
- **Open job:** any state except `closed` and `cancelled`; detention often runs after `delivered`, which is still open.
- **End dates changed** (free time extended by the line): the user edits the date (audited); notices are recalculated: "once" is per container, charge type and end date, so a new end date gets its own 2-days-before and on-the-day notices even when the old date's notices were sent; a notice of the old date not yet sent is dropped. Log lines already written stay with a note; no line is written for days before the new end date.
- **Log stops** when the user records the container as collected (storage, demurrage) or returned empty (detention), with the date: the line for that date stays (or is written if the run has not yet passed), and no line is written for later dates.
- **Allocation:** each charge line of the supplier bill coded to the job and container gets a liability; the liability is required before the bill can be approved for the job, and the reason list is late documents, late duty payment, late collection by the client, company delay, other.
- **Recharge:** *client* → a line on the job's invoice (or a supplementary invoice after close, BL-LOG-22): `out_of_scope` disbursement only with the three VATP013 conditions recorded; otherwise a reimbursement with the VAT category of the job's main service (§9, same rule as BL-LOG-22).
- **Bill differs from the log:** the bill screen shows the log's count of days next to the billed days; a difference is shown, not blocked.

#### Must not
- Must not recharge a *company* liability to the client.
- Must not bill a recharge outside VAT without the VATP013 conditions.
- Must not stop the log while the container is still at the port or not returned.

#### Done when
- [ ] Given container MSKU123 with demurrage free time ending 2026-11-10, the documentation officer and the client are notified on 2026-11-08 and 2026-11-10 at 06:00.
- [ ] On 2026-11-11, 12 and 13 a daily log line is added; nothing more is sent.
- [ ] When the line bills 3 days (AED 1,050) with liability *client* and the conditions not met, on a job whose main service is at 5% the invoice line is AED 1,050 + AED 52.50 VAT; with all three conditions recorded, AED 1,050 `out_of_scope` in the disbursement section.
- [ ] With liability *company*, nothing is recharged and the margin drops by AED 1,050.
- [ ] Recording "collected 2026-11-13" at 15:00 on 2026-11-13 keeps the 13 November line (written at 06:00) and no line is written on 2026-11-14: the log has 3 lines, matching the 3 billed days.
- [ ] Given the end date extended on 2026-11-09 from 2026-11-10 to 2026-11-15 (the 2026-11-08 notice already sent), notices go out on 2026-11-13 and 2026-11-15, none on 2026-11-10, and the first log line is for 2026-11-16.

#### Out of scope
Reopening a closed job (BL-LOG-22). Holds (BL-LOG-21).

---

### BL-LOG-25 — 03 Logistics: configure shared negative-flow settings — documents, rules and integrations

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-DOC-01 · BL-DOC-04 · BL-DOC-05 · BL-HRM-01 · BL-DLV-01 · BL-KRN-04 · BL-KRN-05 · BL-KRN-08 · BL-KRN-09 · BL-INV-09 · BL-INV-10 · BL-NTF-06 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None.
**Review items:** X-13, X-19, 03-N8-a

#### Goal
Seed the 03 deployment's values for the shared mechanisms that concern documents, compliance-critical
rules, integrations, notifications and late entries: which document kinds block run dispatch and
delivery assignment and which role is responsible for each kind, which rules alert the owner, which
connections have a manual fallback, which wl-notify group each 03 notification type names, the
late-entry limits, and the e-invoicing scope. No new behaviour: each value configures a module mechanism.
It also seeds the 03 values for lead sources and campaigns (`SAL.CRM.02`), onboarding and SOPs
(`HRM.ONB.01`) and registration kinds (`DOC.EXP.01`), added on 2026-10-16: `SAL.ACL.07`,
`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`.

#### Files to read
- `docs/verticals/03-logistics.md` §4.1 (row *Driver without signal at a stop*), §5 `HRM.QUA.01`, §7.1 checklist, §7.2 (reused rules and the compliance-critical paragraph), §8 (E-invoicing ASP row, *Integration down*), §9 (*E-invoicing*, *Document blocks*, *Notification types*), §13 question 12
- `docs/modules/wl-documents.md` §5.2, §5.6; `docs/modules/wl-kernel.md` §5.13, §5.14, `PLT.RUL.04`; `docs/modules/wl-notify.md` §5.4
- `docs/modules/wl-sales.md` §4.6, §5.8 and §7 (campaigns, sources); `docs/modules/wl-hr.md` §5.8 and §7 (onboarding); `docs/modules/wl-documents.md` §3, §5.7 and §7 (SOPs, registration kinds)
- `docs/OPEN-ITEMS.md` L-19

#### Terms
- **`blocks`:** a document kind's list of guarded transitions with mode (`block`, `flag`) and override (`never`, `with_approval`).
- **Look-ahead:** days before expiry when planned work is listed (`DOC.RUL.02`).
- **`BlockSubjects` / `FutureWork`:** providers mapping a guarded record to its document subjects and listing planned work.
- **Owner role:** the role that gets the owner alert (`PLT.RUL.04`) when a compliance-critical rule fails.
- **Responsible role:** the role whose holders in the subject's scope get a kind's expiry reminders and renewal task (wl-documents §5.2).
- **Type group:** the row of the wl-notify give-up table a notification type names when it is registered; it sets the default give-up reaction.

#### Spec excerpt (authoritative for this issue)

§9 *Document blocks*:

| Document kind (subject) | `blocks` (mode) | Override | Look-ahead | Responsible role |
|---|---|---|---|---|
| Driving licence (driver, through `HRM.QUA.01`) | `run.dispatch`, `delivery.assign` (block) | never (legal block) | 30 days | operations manager (driver's depot) |
| RTA rider permit and qualification certificate (motorbike rider, Dubai) [S9] | `run.dispatch`, `delivery.assign` (block) | never (legal block) | 30 days | operations manager (driver's depot) |
| Mulkiya, vehicle insurance, Asateel or other vehicle permit (vehicle) [S1] | `run.dispatch` (block); the vehicle is also grounded (`LOG.RUL.10`) | never (legal block) | 30 days | fleet manager |

"Each guarded record maps to its subjects through `BlockSubjects` (a run → its driver and vehicle; a
delivery → the run's driver) and lists planned runs through `FutureWork`. A run dispatched before
midnight finishes its day; the next day's dispatch is refused. Whether these kinds are legal blocks
in the strict sense … is assumed and `⚠ open: lawyer (L-19)` per wl-documents; until confirmed no
override is offered and the override mode of these kinds cannot be changed in the settings. They
are compliance-critical: on expiry the company admin is told as well. The responsible role is
seeded per kind (wl-documents §5.2): it receives the expiry reminders (`DOC.RUL.01`) and the
renewal task (`DOC.RUL.03`) in the subject's scope. Other 03 kinds: shipping and customs documents
of a freight job → documentation officer; client contracts and rate card agreements → accountant."

§9 *Notification types*: "each type 03 registers names its wl-notify group (§5.4 there), which sets
its give-up reaction. Client shipment notices (pickup short, re-weigh, hold, failed attempt, lost,
found, returned, disposed) and recipient tracking SMS → *marketing and tracking* (`none`). Requests
that ask the client to act (address confirmation, instruction for a damaged parcel, bank account
confirmation, freight documents requested) and freight job holds and free-time alerts → *portal
and signatures* (contact marked invalid, staff task to the request's owner: the dispatcher, the
accountant or the documentation officer). COD remittance statements and adjustment notices →
*invoices* (the client's other finance contact, then the accountant as collector role). No 03 type
is statutory."

§7.2: "**Compliance-critical** (owner alert `PLT.RUL.04` to the company admin, the owner role in
this vertical): `LOG.RUL.10` (expired vehicle documents ground the vehicle), `DOC.RUL.03` for the
blocking kinds in §9, and `WRK.RUL.15` (midday break)."

§8 *Integration down*: "Connections registered with a manual fallback: SMS gateway (tracking SMS
queued; the dispatcher calls recipients on today's runs; the client is told by email), e-invoicing
ASP (invoices issued and sent as PDF; transmissions sent when it recovers). The driver PWA has no
offline mode in the MVP: stops done without signal are written on the run sheet and entered later
with the late-entry flag (§4.1)."

§8 E-invoicing ASP row: "While the ASP is down, transmissions stay queued and `FIN.RUL.16` raises a
task after 24 hours."

§4.1 row *Driver without signal at a stop*: "the e-POD keeps the stop time as its business time and
gets a late-entry flag with a reason; beyond 24 hours an operations manager approves (`PLT.ACL.10`)."

§9 *E-invoicing*: "The wl-invoicing `EInvoiceScope` for this vertical leaves out invoices for
international goods transport with an air waybill during that window [24 months from go-live]; sea
and road freight are not [excluded]."

**Settings (setting → value → spec reference)**

| Setting | 03 value | Spec reference |
|---|---|---|
| Kind *Driving licence*: `blocks` | `run.dispatch` block, `delivery.assign` block; override never; look-ahead 30 days | §9 |
| Kind *RTA rider permit and qualification certificate*: `blocks` | `run.dispatch` block, `delivery.assign` block; override never; look-ahead 30 days | §9 |
| Kinds *Mulkiya*, *vehicle insurance*, *Asateel or other vehicle permit*: `blocks` | `run.dispatch` block; override never; look-ahead 30 days | §9 |
| Override mode of the three blocking kinds | `never`, not editable in the settings until L-19 is answered | §9 |
| Responsible role per kind | driving licence, rider permit: operations manager of the driver's depot; vehicle kinds: fleet manager; freight shipping and customs documents: documentation officer; client contracts and rate card agreements: accountant | §9 |
| Notification type groups | shipment notices and tracking SMS: *marketing and tracking*; client action requests, freight holds and free-time alerts: *portal and signatures*; COD statements and adjustment notices: *invoices*; no statutory type | §9 |
| `BlockSubjects` | run → driver (employee) and vehicle (asset); delivery → the run's driver | §9 |
| `FutureWork` | planned runs of the driver or vehicle | §9 |
| `HRM.QUA.01` qualification kinds | driving licence; RTA rider permit and certificate (riders) | §5 |
| Compliance-critical rules | `LOG.RUL.10`, `DOC.RUL.03` (blocking kinds above), `WRK.RUL.15` | §7.2 |
| Owner role (`PLT.RUL.04`) | company admin | §7.2 |
| Integration connections with manual fallback | SMS gateway; e-invoicing ASP | §8 |
| SMS gateway fallback | tracking SMS queued; dispatcher task to call recipients on today's runs; client told by email | §8 |
| E-invoicing ASP fallback | invoices issued and sent as PDF; transmissions queued; `FIN.RUL.16` task after 24 hours | §8 |
| `EInvoiceScope` | exclude invoices for international goods transport with an air waybill for 24 months from the entity's go-live; include sea and road | §9 |
| Late entry (`PLT.ACL.10`) | M = 30 minutes (flag with a reason), N = 24 hours (operations manager approves) | §4.1, wl-kernel |
| Digest of voids and late entries (`PLT.RUL.06`) | on, to operations managers per depot | §7.2 |
| Lead sources used (`SAL.CRM.02`) | referral, website, social, paid ads, partner (e-commerce platforms, freight agents), event, other; recorded on the lead (`SAL.CRM.01`) and carried to the client party when it converts; no source requires a campaign | §5; §7.2 settings |
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | LinkedIn, search ads, email, trade events, marketplaces; converted value: the client's first invoice; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.2 settings |
| Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for driver (licence and documents, vehicle handover, PWA and e-POD training, cash handover), hub staff, dispatcher and customer service, started by `HRM.RUL.16` when the employee record (`HRM.EMP.01`) is created in `onboarding` (none for a record created directly `active`, e.g. by a data import); checklist steps use `WRK.CHK.01`; tasks to the operations manager, the line manager and the employee; the employee's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds the assignees of an overdue step on the first run after its due date, then every 2 days, and tells the operations manager and the line manager after 5 days; completion blocks nothing (run dispatch stays guarded by the driving licence) | §5; §7.2 settings |
| SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: operations manager (dispatch, delivery, cash handover, hub) and fleet manager (daily vehicle check); required readers by role per depot; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP's owner role (the SOP owner named above) after 7 days overdue; acknowledged only by the reader (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.2 settings |
| Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Trademark registration* and *Other certification* (subject: legal entity); responsible role company admin; `DOC.RUL.01` at 180, 90 and 30 days for trademarks and 90, 30 and 7 days for the others; no `blocks` | §5; §7.2 settings |

#### Edge cases decided
- **Overrides:** with override `never`, no override button is shown anywhere; this changes only after L-19 is answered (`blocked:open-item` covers that answer, the block itself is built).
- **"A run dispatched before midnight finishes its day":** a run dispatched on 2026-11-10 with a licence expiring 2026-11-10 continues until it returns; the guard is only checked at `dispatch` and `assign` (BL-DLV-01).
- **Company admin on expiry:** the compliance-critical owner alert goes to the company admin in addition to the expiry notices of `DOC.RUL.03`.
- **No Mirsal or ATLP connection:** they are not registered as connections (no integration); their outage is a freight job hold (BL-LOG-21).
- **Seed, not code:** values are loaded by the 03 deployment template and editable by the company admin, except the override mode for the three kinds, which stays `never` and is not editable in the UI until L-19 is answered.

#### Must not
- Must not hard-code these values in module code.
- Must not offer any override for the three kinds.

#### Done when
- [ ] Documents: a 03 deployment lists the three kind rows with the values above; refusing `run.dispatch` for an expired Mulkiya shows no override; a driver licence expiring in 25 days lists the driver's planned runs; the company admin cannot change the override mode of the three kinds.
- [ ] Responsible roles: the expiry reminder of a Depot A driver's licence goes to Depot A's operations managers; the renewal task for an expired Mulkiya goes to the fleet manager.
- [ ] Notifications: a COD statement e-mail that gives up goes to the client's other finance contact, then creates a task for the accountant; a failed-attempt notice that gives up creates no task; an address confirmation request that gives up marks the contact invalid and creates a task for the dispatcher.
- [ ] Rules: when `LOG.RUL.10` fails to run, the company admin gets the owner alert.
- [ ] Integrations: when the SMS gateway is `down`, tracking SMS are queued, the dispatcher has a task listing today's recipients, and clients receive the e-mail; when the ASP is down for 24 hours, a `FIN.RUL.16` task exists.
- [ ] Late entry: an e-POD entered 3 hours after the stop is flagged with a reason; one entered 30 hours after needs an operations manager's approval.
- [ ] E-invoicing: an invoice for an international air job with an AWB dated within 24 months of go-live is not transmitted; a sea freight invoice is.
- [ ] Lead sources: a new lead or party offers the sources referral, website, social, paid ads, partner, event and other; a campaign with channel *LinkedIn*, a budget and cost entries reports leads, conversion, cost per lead, CAC and ROAS; a cost entry over the budget waits for a manager (`SAL.ACL.07`).
- [ ] Onboarding and SOPs: hiring a driver creates the driver checklist with tasks to the operations manager, the line manager and the employee, and the role is active before it is complete; a step still open on the first run after its due date reminds its assignees, and again 2 days later (`HRM.RUL.17`); a new version of an SOP owned by the operations manager asks every required reader to acknowledge again within 7 days (`DOC.RUL.04`), and no one else can acknowledge for them (`HRM.ACL.10`).
- [ ] Registrations: a fresh 03 deployment lists *Trademark registration* and *Other certification* with responsible role company admin; a trademark registration expiring in 90 days notifies the company admin.

#### Out of scope
The guard, look-ahead, override and alert mechanics (BL-DOC-01, BL-DOC-04, BL-DOC-05, BL-HRM-01,
BL-DLV-01, BL-KRN-08). Delivery, cash and money settings (BL-LOG-26).

---

### BL-LOG-26 — 03 Logistics: configure shared negative-flow settings — delivery, cash, money and access

**Labels:** `vertical:03-logistics` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-DLV-02 · BL-DLV-03 · BL-DLV-05 · BL-DLV-06 · BL-DLV-08 · BL-DLV-09 · BL-PAY-17 · BL-PAY-21 · BL-INV-01 · BL-INV-02 · BL-INV-04 · BL-INV-05 · BL-PAY-09 · BL-PTY-04 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · **Blocks:** BL-LOG-02
**Review items:** X-01, X-02, X-05, X-11, X-12, X-18, 03-N3-a, 03-N9-a

#### Goal
Seed the 03 deployment's values for the shared delivery, cash, receivables and access mechanisms:
partial acceptance, failure and abort reasons, COD tolerance and approver, reversal and lost
approval, cash count and write-off limits, salary deduction, invoice write-off and dispute,
bounced-cheque fee, blocked-client exceptions and portal closure.
It also seeds the 03 values for the cash-flow forecast alert (`FIN.CFL.01`) and the
company budget with owner drawings (`FIN.BUD.01`), added on 2026-10-16: `FIN.RUL.23`,
`FIN.RUL.24` and `FIN.ACL.21`.

#### Files to read
- `docs/verticals/03-logistics.md` §3.3 (`WRK.ACL.09`, `WRK.ACL.10`, `FIN.ACL.05`, `FIN.ACL.06`, `FIN.ACL.19`, `PTY.ACL.03`, `FIN.ACL.15`, `POR.ACL.01`), §4.1 "When it goes wrong", §4.4 step 5, §7.1 *Shipment* (`fail_attempt` reasons), *Run* (`abort` reasons, `hand_over`), §7.2 (reused rules), §9 *Bad debts*, §13 question 13
- `docs/modules/wl-delivery.md` §7; `docs/modules/wl-payments.md` §7; `docs/modules/wl-invoicing.md` §5.9, §5.10
- `docs/modules/wl-invoicing.md` §5.14–§5.16 and §7 (cash-flow forecast, company budget, owner drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments)
- `docs/OPEN-ITEMS.md` L-20

#### Terms
- **Tolerance:** the COD difference accepted without approval.
- **Write-off limit:** the amount above which a write-off needs approval.
- **Exception (`PTY.ACL.03`):** a one-off approval to pay a blocked client.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `WRK.ACL.09` *(shared)* | COD collected below the amount due by more than the tolerance is accepted only with a difference reason and an operations manager's approval from the phone, who chooses who absorbs it (*client*, *business* or *driver*; none preselected, the wl-delivery option *customer* not offered); the driver cannot approve; without an answer the attempt fails | `LIM`, `SOD` | driver, operations manager | AED 0 |
| `WRK.ACL.10` *(shared)* | `reverse_delivery` and a manual `mark_lost` of a delivery need an operations manager's approval; the requester cannot approve, and the driver of the run can neither request nor approve | `SOD`, `STA` | all staff | on |
| `FIN.ACL.05` *(shared, formerly `LOG.ACL.05`)* | The cash handover is counted by a cashier or operations manager, never by the driver who hands it over | `SOD` | driver, cashier | on |
| `FIN.ACL.06` *(shared, formerly `LOG.ACL.06`)* | Writing off a driver's cash shortage needs an operations manager's approval above a threshold | `LIM` | cashier | AED 50 |
| `FIN.ACL.19` *(shared)* | Recovering a cash shortage by salary deduction needs approval by another user than the counter, and the driver's acknowledgement | `SOD` | cashier, operations manager | off (`⚠ open: lawyer (L-20)`, as in wl-payments) |
| `PTY.ACL.03` *(shared)* | While a client is blocked, no COD remittance or refund is paid to it and no rate card is renewed or extended, unless the company admin approves an exception (03 has no finance manager role; the requester cannot approve; screening is off in 03) | `STA`, `LIM` | accountant | — |
| `FIN.ACL.15` *(shared)* | Writing off an invoice balance as a bad debt needs approval; the requester cannot approve | `LIM`, `SOD` | accountant | AED 0 (every write-off approved by the operations manager or company admin) |
| `POR.ACL.01` *(shared)* | After a client's contract ends, its portal users keep read-only access to shipments, invoices and COD statements, then the portal closes | `TIM` | client user | closes after 90 days |

§7.1 `fail_attempt` guard: "failure reason chosen (including *refused to pay*, *no cash*, *cancelled
by client*, *run aborted*, *wrong address*); *cancelled by client* and *run aborted* are not counted
as attempts (`WRK.RUL.08`)". Run `abort` guard: "reason (breakdown, accident, driver unwell, driver
unreachable, weather, other)". §4.1 row *Recipient refuses some parcels…*: "Single-parcel shipments:
partial acceptance is off". §4.1 row *Shipment cannot be found*: `WRK.RUL.17` "after 7 days
untraced". Run `hand_over` guard: "receiving driver scans each parcel and counts the cash".

§9 *Bad debts*: "an uncollectable client invoice is written off with approval (`FIN.ACL.15`, every
write-off approved), the write-off notice is sent to the client, and the VAT bad-debt relief is
proposed by `FIN.RUL.14` once its conditions are met (wl-invoicing §5.9). A bounce fee on a returned
cheque is off by default."

§4.4 step 5: "A complaint about an invoice is a `disputed` invoice (wl-invoicing): reminders pause
until the dispute is resolved as *upheld* or *credited*. Client users may raise it in the portal."

§7.2: `FIN.RUL.14`–`16` used (bad-debt relief proposal, dispute escalation after 14 days, e-invoice
rejected or late).

**Settings (setting → value → spec reference)**

| Setting | 03 value | Spec reference |
|---|---|---|
| Partial acceptance (source type *shipment*) | on for multi-parcel shipments; off for single-parcel shipments | §4.1 |
| Failure reasons (shipment) | standard list plus *refused to pay*, *no cash*, *cancelled by client*, *run aborted*, *wrong address*; *run aborted* and *cancelled by client* do not count against max attempts; *wrong address* puts the shipment `on_hold` (*address issue*) instead of a reattempt | §7.1, §7.2 `WRK.RUL.08` |
| Run abort reasons | breakdown, accident, driver unwell, driver unreachable, weather, other | §7.1 Run |
| Hand-over between drivers | receiving driver scans each parcel: on; counts the cash: on | §7.1 Run |
| COD difference tolerance (`WRK.ACL.09`) | AED 0; short payment allowed with approval | §3.3 |
| COD difference approver | operations manager (`deliveries.approve_cod_difference`) | §3.3 |
| COD absorbers offered | client (with instruction), business, driver; *customer* hidden; no preselected default | §3.3, §4.1 |
| COD approval timeout and fallback | 10 minutes; forced failed attempt | wl-delivery §7 |
| Reversal and manual lost approval (`WRK.ACL.10`) | on; approver operations manager | §3.3 |
| Untraced days before `lost` (`WRK.RUL.17`) | 7 | §4.1 |
| Cash holders | drivers: per person, per run | wl-payments mapping |
| Cash counted by another user (`FIN.ACL.05`) | on; counter cashier or operations manager | §3.3 |
| Cash shortage write-off limit (`FIN.ACL.06`) | AED 50 | §3.3 |
| Shortage recovery by salary deduction (`FIN.ACL.19`) | off (L-20) | §3.3 |
| Handover escalation (`FIN.RUL.22`) | replaced for drivers by `LOG.RUL.13` (N = 12 h, M = 48 h) | §7.2 |
| Invoice write-off (`FIN.ACL.15`) | limit AED 0; approvers operations manager or company admin; requester cannot approve | §3.3 |
| Write-off notice to the client | sent | §9 |
| Bad-debt relief proposal (`FIN.RUL.14`) | on | §9 |
| Invoice dispute from the portal | client users may raise it; outcomes *upheld* or *credited*; escalation `FIN.RUL.15` after 14 days | §4.4, §7.2 |
| Bounce fee on a returned cheque | off | §9 |
| Blocked-client exception approver (`PTY.ACL.03`) | company admin; the requester cannot approve | §3.3 |
| Sanctions screening | off | §3.3 |
| Portal after contract end (`POR.ACL.01`) | read-only, closes after 90 days | §3.3 |
| Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 100,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the company admin and the accountant, checked daily at 06:00; COD remittances to clients counted as payments on their schedule (a `CashForecastSource` of 03); disbursements by their recharge invoice; due dates adjusted by each client's average days late: on; disputed amounts counted: off | §5; §7.2 settings |
| Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories (fuel, tolls and fines, vehicle maintenance, rent, staff, carriers) owned by the accountant and mapped to the export accounts; one budget per legal entity and fiscal year, by month, approved by the company admin; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.2 settings |
| Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of operating cost and profit; every drawing approved, whatever its amount (`FIN.ACL.21` limit AED 0), by the company admin, never by the requester (`FIN.ACL.21`) | §5; §7.2 settings |

#### Edge cases decided
- **Operations manager approvals in a depot with one manager:** the company admin may approve; the requester never may.
- **AED 50 write-off limit:** a shortage of AED 50.00 or less is written off without approval; AED 50.01 needs approval.
- **Salary deduction:** stays hidden until L-20 is answered and the company admin turns it on (`blocked:open-item`).
- **Seed, not code:** all values come from the 03 deployment template and are editable by the company admin within the module's allowed ranges.

#### Must not
- Must not let a driver approve a COD difference, a reversal or a lost marking.
- Must not offer salary deduction while `FIN.ACL.19` is off.
- Must not hard-code these values in module code.

#### Done when
- [ ] Delivery: a single-parcel shipment offers no refused quantity; a 3-parcel one does; the failure reason list contains the five reasons above (*refused to pay*, *no cash*, *cancelled by client*, *run aborted*, *wrong address*); the abort list contains *driver unreachable*.
- [ ] COD: AED 0.01 short needs an operations manager's approval; the approver sees *client*, *business*, *driver*; after 10 minutes without an answer the phone offers only the failed attempt.
- [ ] Reversal and lost: a dispatcher's reversal request needs an operations manager's approval; the run's driver cannot request.
- [ ] Cash: a AED 40 shortage can be written off by the cashier; AED 60 needs approval; salary deduction is not offered.
- [ ] Receivables: every invoice write-off needs approval by the operations manager or company admin, not the requester; the client receives the write-off notice; a returned cheque adds no bounce fee; a client user can dispute an invoice in the portal.
- [ ] Access: a blocked client's remittance exception is approved by the company admin, not by the requesting accountant; a client whose contract ended 2026-11-01 has read-only portal access until 2027-01-30 and none from 2027-01-31.
- [ ] Cash-flow forecast: in a fresh 03 deployment, a projected balance of AED 99,999 in week 5 alerts the company admin and the accountant once (`FIN.RUL.23`, 06:00 run); AED 100,000 or more in every week alerts no one; the forecast shows 13 weekly buckets from Monday.
- [ ] Budget and drawings: a monthly budget for a legal entity prepared by the accountant needs approval by the company admin; an expense with category *owner drawings* is left out of operating cost and profit reports, and the user who records it cannot approve it (`FIN.ACL.21`); an expense line reaching 100% of its month's budget line warns the category's owner role and the accountant once (`FIN.RUL.24`).

#### Out of scope
The mechanisms themselves (module epics listed in Depends on). Documents, rules and integrations
(BL-LOG-25).
