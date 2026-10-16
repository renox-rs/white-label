# Epic: 06 F&B — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows the review fixes added to
[06-fnb.md](../../verticals/06-fnb.md): failed own-rider deliveries and the refund rule
(`fail_delivery`, `close_failed`, `FNB.ACL.16`, `FNB.ACL.17`, `FNB.RUL.14`), cancellations before
and after acceptance, amendments and line cancels, the aggregator rider who never comes
(`FNB.RUL.16`), refunds after a reversed delivery or a complaint, payout statement duplicates,
voids and aggregator refund lines, production batch holds, fridge temperature holds
(`FNB.RUL.05`), recalls traced to orders, the outage pause (`FNB.RUL.15`), the business-day
cut-off, and 06's values for the shared mechanisms. It closes 06-N9-b, 06-N3-a, 06-N4-a, 06-N4-b,
06-N10-b, 06-N10-c, 06-N10-e, 06-N7-c, D-12 and the 06 MEDIUM items (customer cancel before
acceptance, aggregator rider never arrives, item runs out after acceptance, duplicate external
orders, payout file revert, business-day cut-off), on top of the module epics for X-06, X-07,
X-08, X-09, X-10, X-11, X-12, X-14 and X-19.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-FNB-01 | Move an order to `delivery_failed` when its own-rider delivery fails | high | m | Launch 06 | Base: 06 Order fulfilment lifecycle; BL-DLV-07; BL-DLV-08; BL-DLV-10 |
| BL-FNB-02 | Close a failed delivery with the refund decision (`close_failed`, `FNB.ACL.17`) | high | m | Launch 06 | BL-FNB-01; BL-SAL-07; BL-STK-26; BL-PAY-06; BL-PAY-07 |
| BL-FNB-03 | Turn cash on delivery off after repeated failures (`FNB.RUL.14`) | medium | s | Launch 06 | BL-FNB-01 |
| BL-FNB-04 | Cancel an order before acceptance by the customer, and after acceptance with outcomes | high | m | Launch 06 | Base: 06 Order fulfilment lifecycle; BL-SAL-07; BL-STK-26 |
| BL-FNB-05 | Amend a direct order and cancel a line when an item runs out | high | m | Launch 06 | BL-SAL-05; BL-SAL-06; BL-SAL-08; BL-STK-26 |
| BL-FNB-06 | Chase an aggregator rider who does not arrive (`FNB.RUL.16`) | medium | s | Launch 06 | Base: 06 Order fulfilment lifecycle; BL-FNB-04 |
| BL-FNB-07 | Refund a completed order when its delivery is reversed | high | s | Launch 06 | BL-DLV-04; BL-DLV-05; BL-SAL-14 |
| BL-FNB-08 | Record rider faults and report refunds and failures by responsible party | medium | s | Launch 06 | BL-SAL-14; BL-FNB-02; BL-FNB-07 |
| BL-FNB-09 | Refuse a duplicate payout statement and void a wrong one | high | m | Launch 06 | Base: 06 Payout statement lifecycle; BL-KRN-03; BL-SAL-14 |
| BL-FNB-10 | Turn aggregator refund lines into sales returns on payout match | high | m | Launch 06 | Base: 06 Payout statement lifecycle; BL-SAL-14; BL-KRN-04 |
| BL-FNB-11 | Hold, release and discard production batches with their lot | high | m | Launch 06 | Base: 06 Production batch lifecycle; BL-STK-11; BL-STK-12; BL-STK-14 |
| BL-FNB-12 | Hold a cold unit's stock on an out-of-range reading (`FNB.RUL.05`) | high | m | Launch 06 | BL-STK-11; BL-STK-12; BL-STK-13; Base: wl-ticketing tickets and checklists |
| BL-FNB-13 | Trace a recalled ingredient lot to orders and customers | high | m | Launch 06 | BL-FNB-11; BL-STK-14; BL-STK-15; BL-STK-16 |
| BL-FNB-14 | Pause an outlet's channels during an outage and resume (`FNB.RUL.15`) | high | m | Launch 06 | BL-KRN-04; BL-KRN-06; BL-KRN-07 |
| BL-FNB-15 | Assign orders to a business day by the cut-off and reopen a closing for late entries | medium | m | Launch 06 | Base: wl-sales Till session; BL-KRN-04; BL-PAY-19 |
| BL-FNB-16 | 06: configure shared negative-flow settings (delivery, sales, payments, cash) | high | m | Launch 06 | BL-DLV-02; BL-DLV-03; BL-DLV-05 to BL-DLV-10; BL-SAL-01 to BL-SAL-09; BL-SAL-14; BL-PAY-02 to BL-PAY-07; BL-PAY-17; BL-PAY-19; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26 |
| BL-FNB-17 | 06: configure shared negative-flow settings (stock, compliance, connections) | high | m | Launch 06 | BL-STK-01; BL-STK-03; BL-STK-07; BL-STK-11 to BL-STK-19; BL-DOC-02; BL-KRN-03 to BL-KRN-08; BL-SAL-01; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-SAL-22; BL-SAL-23; BL-SAL-24; BL-SAL-25; BL-SAL-26; BL-NTF-07; BL-NTF-08; BL-NTF-09; BL-STK-28; BL-STK-29; BL-STK-30; BL-STK-31; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |

---

### BL-FNB-01 — 06 F&B: move an order to `delivery_failed` when its own-rider delivery fails

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: 06 Order fulfilment lifecycle (`dispatch`, `complete`) · BL-DLV-07 (perishable policy) · BL-DLV-08 (run `abort`) · BL-DLV-10 (perishable hand-over window) · **Blocks:** BL-FNB-02, BL-FNB-03
**Review items:** 06-N9-b, X-12

#### Goal
Cooked food gets one delivery attempt. When the outlet's own rider cannot deliver (customer
unreachable, wrong address, refused, no cash, rider issue, run aborted), the order moves from
`dispatched` to `delivery_failed` with the failure reason. Nothing is collected on cash on
delivery, the customer is told, and the food comes back to the outlet. The refund decision is
made when the order is closed (BL-FNB-02).

#### Files to read
- `docs/verticals/06-fnb.md` §4.2 "When it goes wrong" (rows "Own-rider delivery fails", "Rider's
  run is aborted"), §7.1 *Order* (states `delivery_failed`, `cancelled`; row `fail_delivery`)
- `docs/modules/wl-delivery.md` §4 (Delivery `attempt`, `return`, `discard`; Run `abort`,
  `hand_over`), `WRK.RUL.16`, vertical mapping row 06

#### Terms
- **Own rider:** the outlet's employee delivering direct (D2C) orders in the driver PWA.
- **Perishable delivery:** max one attempt, no reattempt; after a failure the policy is *return*
  (to the outlet) or *discard* (with a photo).
- **Customer-side reason:** customer unreachable, wrong address, refused, no cash.
- **Outlet-side reason:** rider issue, run aborted (and late or wrong item, see BL-FNB-02).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `delivery_failed` | warning | no | Own-rider delivery failed; the food is coming back or was discarded; refund decision pending |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `fail_delivery` | dispatched | delivery_failed | system (`DeliveryAttemptFailed`, or a run `abort` with no hand-over) | failure reason recorded, mapped from the wl-delivery reason (table below) | one attempt only (perishable, `WRK.RUL.16`): no reattempt; food returned to the outlet (policy *return*); customer told; nothing collected on cash on delivery; `FNB.RUL.14` counts customer-side failures on cash orders |

§7.1 **Failure reasons from wl-delivery.** 06 adds *rider issue* to the wl-delivery failure reason
list (not counted, no reattempt), and the rider picks the customer's stated cause when
recording *refused*: *late*, *wrong item* or *other*. The wl-delivery reason is kept on the
order too; the kitchen manager may change the proposed responsible party with a note.

| wl-delivery reason | Order failure reason | Side (`FNB.ACL.17`) | Proposed responsible party |
|---|---|---|---|
| not home, access denied | customer unreachable | customer | customer |
| wrong address | wrong address | customer | customer |
| refused, cause *other* | refused | customer | customer |
| refused to pay, no cash | no cash | customer | customer |
| refused, cause *late* or *wrong item* | refused (late / wrong item) | outlet | business |
| rider issue (vehicle, lost, late beyond the promise), not attempted | rider issue | outlet | rider |
| run aborted | run aborted | outlet | business |

| Situation | What happens |
|---|---|
| Own-rider delivery fails (customer unreachable, wrong address, refuses, cannot pay) | The delivery is perishable: one attempt, no reattempt (`WRK.RUL.16`). The order goes `delivery_failed`; the rider brings the food back and the outlet logs it as wastage (`FNB.ACL.16`). Refund per `FNB.ACL.17`; nothing is collected on a cash order. Repeated failed cash orders turn cash on delivery off for that customer (`FNB.RUL.14`). A remake, if the customer still wants the food, is a new order |
| Rider's run is aborted (breakdown, accident) | Food not yet delivered is handed over to another rider (`hand_over`, the receiving rider confirms orders and cash) when it can still arrive within the brand's delivery promise; otherwise the stops fail with *run aborted* and the outlet refunds in full |

#### Edge cases decided
- **Reason mapping from wl-delivery:** as the §7.1 table; *rider issue* is a 06 entry in the
  wl-delivery failure reason list (not counted, no reattempt); the original wl-delivery reason
  is kept on the order too.
- **Perishable policy:** the 06 value *return* (not the wl-delivery default *discard* for
  cooked food) applies; *discard* only after a run abort the rider cannot return from.
- **Hand-over succeeded:** no `fail_delivery`; the order stays `dispatched`.
- **Hand-over not possible within the brand's promise:** the stop fails with *run aborted* at
  once (BL-DLV-10) and this transition runs.
- **Duplicate event:** a second `DeliveryAttemptFailed` for the same attempt is ignored.
- **Order no longer `dispatched`** (e.g. completed by a late `DeliveryDelivered`): the event is
  ignored and logged; no state change.
- **Customer message:** "We could not deliver your order <number>"; no blame text.
- **Remake:** never reopens this order; staff create a new order.

#### Must not
- Must not schedule a reattempt.
- Must not record any cash collected on a failed cash-on-delivery order.
- Must not let a user run `fail_delivery` by hand (system only).

#### Done when
- [ ] Given a `dispatched` prepaid order ORD-501 (AED 85.00), when the rider records *not home*,
      the order is `delivery_failed` with reason customer unreachable, the customer is told, and no
      reattempt is queued.
- [ ] Given a cash order ORD-502 and the rider records *no cash*, the order is `delivery_failed`
      and the rider's expected cash does not include ORD-502.
- [ ] Given run R1 aborted at 20:10 and stop ORD-503 whose brand promise ends 20:25 when the
      receiving run cannot arrive before 20:40, ORD-503 is `delivery_failed` with *run aborted*.
- [ ] Given the same stop handed over to a run arriving 20:20, ORD-503 stays `dispatched`.
- [ ] A second identical event leaves one history row.
- [ ] Given *refused* with cause *late*, the order reason is refused (late), outlet side; with
      cause *other*, customer side.

#### Out of scope
The refund decision and closing (BL-FNB-02); `FNB.RUL.14` (BL-FNB-03); delivery mechanics
(Epic wl-delivery).

---

### BL-FNB-02 — 06 F&B: close a failed delivery with the refund decision (`close_failed`, `FNB.ACL.17`)

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-FNB-01 · BL-SAL-07 (cancel with outcome per line) · BL-STK-26 (outcome *scrap* to wastage) · BL-PAY-06 · BL-PAY-07 (failed refunds) · **Blocks:** BL-FNB-08
**Review items:** 06-N9-b, X-12

#### Goal
When the food from a failed delivery is back at the outlet (or was discarded), the order is
closed as `cancelled` with a refund decision and a responsible party. A prepaid order is refunded
in full at once when the reason is on the outlet's side; for a customer-side reason no refund is
made unless the kitchen manager approves one. The returned food is never sold again and is
logged as wastage.

#### Files to read
- `docs/verticals/06-fnb.md` §3.3 (`FNB.ACL.16`, `FNB.ACL.17`), §4.2 "When it goes wrong" (row
  "Own-rider delivery fails"), §7.1 *Order* (`close_failed`), §9 (Returned and held food; Refunds
  after a failed delivery)
- `docs/modules/wl-sales.md` §4 *Sales order* (`cancel` with outcomes); `docs/modules/wl-delivery.md`
  (`DeliveryReturned`, `DeliveryDiscarded`)
- `docs/OPEN-ITEMS.md` L-33

#### Terms
- **Refund decision:** *full refund*, *no refund*, or *manager-approved refund* (amount).
- **Responsible party:** customer, rider or business.
- **Wastage:** a stock movement to scrap with a reason; here reason *failed delivery*.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `close_failed` | delivery_failed | cancelled | system (`DeliveryReturned` or `DeliveryDiscarded`) or `orders.manage` | refund decision per `FNB.ACL.17` (the system proposes *full refund* for an outlet-side reason, *no refund* for a customer-side one, *not applicable* on cash on delivery); responsible party recorded (customer, rider, business); no `SAL.ACL.05` approval | refund per decision (everything paid: items and delivery fee); food logged as wastage once (`FNB.ACL.16`): with policy *return*, `SalesOrderCancelled` with outcome *scrap* posts it (reason *failed delivery*); with *discard*, the wastage record of the wl-delivery `discard` is the posting, the event carries its reference and no second movement is posted |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `FNB.ACL.16` | Food that comes back from a failed delivery, or was held for temperature and not released by the chef, is never sold or reused; it is logged as wastage | `STA` | outlet staff | on |
| `FNB.ACL.17` | After a failed own-rider delivery, a prepaid order is refunded in full at once when the reason is on the outlet's side (rider issue, run aborted, or refused because the order was late or had the wrong item; mapping in §7.1); for a customer-side reason (unreachable, wrong address, refused for another cause, no cash) no refund is made unless the kitchen manager approves one: the cashier requests it from the cancelled order within 7 days of the cancellation (`close_failed`), full or partial up to the amount paid, and the requester never approves it (`FIN.ACL.04`) | `STA`, `LIM`, `SOD` | cashier, kitchen manager | customer-side: no refund `⚠ open: lawyer (L-33)` (consumer protection) |

| Setting | 06 value |
|---|---|
| Perishable policy for own-rider deliveries (`WRK.RUL.16`) | *return* to the outlet (replaces the wl-delivery default *discard* for cooked food); *discard* only when the rider cannot return (run aborted), with a photo |

#### Edge cases decided
- **Late or wrong item:** carried as the cause of *refused* (§7.1 mapping): *late* or *wrong
  item* are outlet side, *other* is customer side. Unreachable, wrong address, refused (other)
  and no cash are customer side; rider issue and run aborted are outlet side.
- **7 days:** counted from the time the order became `cancelled` (`close_failed`), Asia/Dubai;
  a request at 7 days 0 hours after it is still accepted, later is refused with "refund request
  window closed".
- **Responsible party proposed:** outlet-side → *business* (late, wrong item) or *rider* (rider
  issue); run aborted → *business*; customer-side → *customer*. The kitchen manager may change it
  with a note.
- **System close:** on `DeliveryReturned` or `DeliveryDiscarded` the system runs `close_failed`:
  outlet side → *full refund* at once; customer side → *no refund*.
- **Manager-approved refund on a customer-side failure:** requested by the
  cashier from the cancelled order within 7 days, full or partial (up to the amount paid),
  approved by the kitchen manager, never by the requester (`FIN.ACL.04`).
- **Full refund amount:** everything paid on the order (items, delivery fee), in fils.
- **Cash on delivery:** nothing was collected, so no refund; decision recorded as *not
  applicable*.
- **Wastage posted once:** with policy *return*, `SalesOrderCancelled` outcome *scrap* posts the
  wastage (BL-STK-26, reason *failed delivery*). With *discard* (run aborted), wl-delivery already
  posted the wastage; the event carries the existing wastage reference and no second movement
  is posted (§7.1 `close_failed`).
- **`SAL.ACL.05` approval:** not asked for `close_failed` (system transition; the refund rule is
  `FNB.ACL.17`).
- **Refund fails at the gateway:** handled by wl-payments (BL-PAY-06, BL-PAY-07); the order stays
  `cancelled`.
- **L-33 open:** build the default (customer side: no refund); flagged.

#### Must not
- Must not return food from a failed delivery to sellable stock.
- Must not refund a customer-side failure without the kitchen manager's approval.
- Must not post the wastage twice.

#### Done when
- [ ] Given prepaid ORD-501 (AED 85.00) `delivery_failed` with *run aborted* and the food
      discarded, on `DeliveryDiscarded` the order is `cancelled`, AED 85.00 is refunded at once,
      responsible party *business*, and exactly one wastage movement exists.
- [ ] Given prepaid ORD-504 (AED 60.00) failed with customer unreachable and returned, on
      `DeliveryReturned` the order is `cancelled`, no refund is made, responsible party *customer*,
      and the food is posted as wastage reason *failed delivery*.
- [ ] Cashier A requests a AED 30.00 refund on ORD-504; A cannot approve it; kitchen manager B
      approves and AED 30.00 is refunded. Given ORD-504 cancelled on 2026-10-01 20:00, a request
      on 2026-10-08 19:00 is accepted and one on 2026-10-09 09:00 is refused.
- [ ] Given *refused* with cause *wrong item*, the refund is full and the party *business*.
- [ ] For ORD-504 the only stock movement of its items is the wastage (reason *failed
      delivery*): no receipt or return-to-stock movement exists, and the outlet's on-hand of those
      items after the return equals the on-hand before it.

#### Out of scope
`fail_delivery` (BL-FNB-01); rider record and KPIs (BL-FNB-08); refund failure handling
(Epic wl-payments).

---

### BL-FNB-03 — 06 F&B: turn cash on delivery off after repeated failures (`FNB.RUL.14`)

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-FNB-01 · **Blocks:** None.
**Review items:** 06-N9-b

#### Goal
A customer whose cash-on-delivery orders keep failing for customer-side reasons costs the
outlet food each time. After 2 such failures within 90 days, cash on delivery is turned off for
that customer (online payment only) and the kitchen manager is told and can turn it back on.

#### Files to read
- `docs/verticals/06-fnb.md` §4.2 "When it goes wrong" (row "Own-rider delivery fails"), §7.2
  (`FNB.RUL.14`)

#### Terms
- **Customer:** the direct-order customer party, found by phone number. D2C orders need OTP
  login (there is no guest checkout, §4.2 step 3); phone orders entered by the cashier are linked
  to the party with that number (created if new), so their failures count too.
- **COD allowed flag:** a per-customer flag the D2C checkout reads.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `FNB.RUL.14` | own-rider delivery of a cash-on-delivery order fails for a customer-side reason (unreachable, wrong address, refused, no cash) | the customer has N such failures within D days (customer = the customer party found by phone number: D2C orders need OTP login, there is no guest checkout; phone orders are linked to the party with that number, created if new) | turn cash on delivery off for the customer (online payment only); notify the kitchen manager, who can turn it back on; failures before it was last turned back on are not counted | N = 2, D = 90 | on | MVP |

#### Edge cases decided
- **Counting:** failures with `fail_delivery` on cash orders with a customer-side reason, by
  failure time, in a rolling 90-day window ending now (Asia/Dubai). The current failure counts.
- **Across brands and outlets:** counted per customer party across all brands and outlets of the
  deployment.
- **Turned back on:** failures before the time the kitchen manager turns
  COD back on no longer count.
- **Which kitchen manager:** the one of the outlet of the failure that triggered the rule.
- **Refused for a wrong item** (outlet side, BL-FNB-02): not counted.
- **Checkout:** with COD off, the D2C site offers only online payment with a neutral line
  "Online payment only for this order".

#### Must not
- Must not count outlet-side failures or prepaid orders.
- Must not block the customer from ordering.

#### Done when
- [ ] Given customer C with a failed cash order on 2026-08-01 (unreachable) and another on
      2026-10-15 (refused, cause other), COD is off for C and the kitchen manager is notified.
- [ ] Given failures on 2026-06-01 and 2026-10-15 (more than 90 days apart), COD stays on.
- [ ] After the manager turns COD on at 2026-10-16 10:00, one new failure does not turn it off.
- [ ] With COD off, C's checkout shows only online payment.
- [ ] Given a D2C failure (unreachable) and a phone order failure (no cash) for the same number
      within 90 days, COD is turned off for that customer.

#### Out of scope
The failure itself (BL-FNB-01).

---

### BL-FNB-04 — 06 F&B: cancel an order before acceptance by the customer, and after acceptance with outcomes

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: 06 Order fulfilment lifecycle · BL-SAL-07 (cancel with outcome per line) · BL-STK-26 · Base: 06 Kitchen ticket lifecycle · **Blocks:** BL-FNB-06
**Review items:** 06 MEDIUM "customer cancel before acceptance", X-14

#### Goal
A customer can cancel a direct order from the order page while it is still `placed`: full
refund, nothing reaches the kitchen. After acceptance only staff cancel, with a reason and a
responsible party (*business*, *aggregator* or *customer*: the wl-sales party list, where the
kitchen is *business*); a paid order above AED 50 or after
preparation started needs a manager, and once the kitchen started, each started line needs an
outcome (*return to stock* or *scrap*). An order entered by mistake is cancelled with reason
*entered in error* and responsible party *business*; orders are never voided.

#### Files to read
- `docs/verticals/06-fnb.md` §3.3 (`SAL.ACL.01`, `SAL.ACL.05`), §4.2 "When it goes wrong" (row
  "Customer cancels before the outlet accepts"), §7.1 *Order* (`cancel`, paragraph after the
  table), *Kitchen ticket* (`void`)
- `docs/modules/wl-sales.md` §4 *Sales order* (`cancel`, outcomes)

#### Terms
- **Placed:** a direct order received and paid (or cash on delivery), waiting for acceptance.
- **Outcome per line:** *return to stock* (untouched packed items) or *scrap* (food started,
  posted as wastage). A remake is a new order and the cancelled line's outcome is *scrap*.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | placed, accepted, preparing, ready | cancelled | customer (from `placed` only, before acceptance) or `orders.cancel` | from `placed`: none; after acceptance: `SAL.ACL.01`, and `SAL.ACL.05` once the kitchen started; reason and responsible party (business / aggregator / customer) | full refund from `placed`; otherwise refund if paid; prepared food logged as wastage |
| `void` (kitchen ticket) | queued, in_prep | voided | system (order `cancel`) | — | — |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `SAL.ACL.01` *(shared)* | Cancelling or refunding a paid order above a value, or after preparation started, needs a manager | `LIM`, `STA` | cashier | AED 50 |
| `SAL.ACL.05` *(shared)* | Cancelling an order or a line after the kitchen started it needs an outcome per line (*return to stock*, or *scrap* as wastage) and a manager's approval; a remake is a new order, with the cancelled line's outcome *scrap* | `STA` | cashier, kitchen manager | — |

§7.1: "An order entered by mistake is cancelled with reason *entered in error* and responsible
party *business*; the order itself
does not use `void`, because its receipt may already be issued (a receipt is corrected by a sales
return, `SAL.ACL.04`)."

#### Edge cases decided
- **Race between customer cancel and accept:** whichever runs first wins; the other is refused
  ("order already accepted" / "order already cancelled") and the customer page refreshes.
- **Customer cancel and `FNB.RUL.01`:** a customer cancel stops the auto-reject timer.
- **Kitchen started:** at least one kitchen ticket of the order is `in_prep` or `done`.
- **Aggregator orders:** no refund is issued here (the aggregator holds the customer's money);
  when the responsible party is *aggregator*, the order is flagged "compensation expected" for
  payout reconciliation.
- **Cash on delivery not yet collected:** nothing to refund.
- **Counter order paid in cash:** refunded in cash from the till.
- **Receipt already issued:** the refund issues a sales return / credit (module), not an edit.
- **Done tickets:** stay `done`; their food follows the line outcome.

#### Must not
- Must not let the customer cancel after acceptance.
- Must not use `void` on an order.
- Must not cancel a started line without an outcome and the manager's approval.

#### Done when
- [ ] Given prepaid ORD-601 (AED 40.00) `placed`, when the customer cancels from the order page,
      it is `cancelled`, AED 40.00 is refunded, no kitchen ticket exists.
- [ ] Given ORD-601 accepted a second earlier, the customer cancel is refused.
- [ ] Given accepted ORD-602 (AED 120.00) not started, cashier A's cancel creates an approval
      request (above AED 50).
- [ ] Given ORD-603 `preparing` with a burger ticket `in_prep`, cancel requires an outcome per
      started line and manager approval; with *scrap*, the burger is posted as wastage and the
      tickets are `voided`.
- [ ] Cancelling with reason *entered in error* records responsible party *business* (not
      editable for that reason); no `void` action exists on orders.
- [ ] An aggregator order cancelled with party *aggregator* is flagged "compensation expected"
      and issues no refund.

#### Out of scope
Line cancel and amend (BL-FNB-05); failed deliveries (BL-FNB-01, BL-FNB-02); outcome stock
posting (BL-STK-26).

---

### BL-FNB-05 — 06 F&B: amend a direct order and cancel a line when an item runs out

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-SAL-05 (amend as revision) · BL-SAL-06 (money of an amendment) · BL-SAL-08 (`cancel_line`) · BL-STK-26 · **Blocks:** None.
**Review items:** 06 MEDIUM "item runs out after acceptance", X-14

#### Goal
When an item runs out after acceptance, the cashier calls the customer and either substitutes
(`amend`: price difference refunded or charged by link), drops the line (`cancel_line`: line
refunded), or cancels the order. On aggregator orders, the change is made on the aggregator's
tablet and only the result is recorded here: a line cancel or an order cancel with responsible
party *business*. Kitchen tickets follow. The cashier holds `orders.manage` (amend) and
`orders.cancel` (line cancel) for the outlet (§3.1); `SAL.ACL.05` still asks the manager for a
started line.

#### Files to read
- `docs/verticals/06-fnb.md` §4.1 "When it goes wrong" (row "An item runs out after the order was
  accepted"), §4.2 "When it goes wrong" (row "An item runs out after acceptance"), §7.1 *Order*
  (`amend`, `cancel_line`), §7.2 settings (amendments on aggregator channels)
- `docs/modules/wl-sales.md` §4 *Sales order* (`amend`, `cancel_line`)

#### Terms
- **Amend:** a new revision of the sales order; earlier revisions are kept read-only.
- **Line not started:** its kitchen ticket item is `queued`.
- **"86":** marking an item unavailable for an outlet (`FNB.RUL.06`).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `amend` | placed, accepted | same state | `orders.manage` | direct orders only (aggregator orders are changed on the aggregator); customer agreed; changed lines not started by the kitchen | sales order `amend`: price difference refunded, or charged through a payment link; kitchen tickets updated |
| `cancel_line` | accepted, preparing | same state | `orders.cancel` | item unavailable or customer request; line not yet bumped; at least one other line stays open (otherwise order `cancel`, wl-sales); `SAL.ACL.05` when the kitchen started it | sales order `cancel_line`; line refunded if paid; started food logged as wastage |

| Situation | What happens |
|---|---|
| An item runs out after the order was accepted (aggregator) | The cashier changes the order on the aggregator's tablet (the aggregator owns the change; amendments are off on aggregator channels) and records the result here: the line is cancelled, or the order is cancelled with responsible party *business*. `FNB.RUL.06` marks the item unavailable |
| An item runs out after acceptance (direct) | The cashier calls the customer: substitute (order `amend`, price difference refunded or charged by link), drop the line (`cancel_line`, line refunded), or cancel the order (full refund) |

#### Edge cases decided
- **Who runs it:** the cashier role holds `orders.manage` and `orders.cancel` at its outlet
  (§3.1); the kitchen manager too. A cook cannot amend or cancel.
- **"Customer agreed":** a required tick with the time of the call; no signature.
- **Last open line:** `cancel_line` on the only remaining open line is refused (wl-sales
  `cancel_line` guard); use order `cancel` (BL-FNB-04).
- **Ticket updates:** an amended or cancelled line is removed from its station ticket at once;
  a ticket left with no items is `voided`; added lines create or extend `queued` tickets.
- **Price increase charged by link:** follows BL-SAL-06; the kitchen does not wait for the link
  payment (the customer agreed by phone). An unpaid link is a receivable on the order.
- **Aggregator line cancel:** no refund here (the aggregator refunds); flagged for payout
  reconciliation as an expected refund line.
- **Promo codes:** recalculated by the sales order amend (module).
- **Bumped line:** cannot be cancelled with `cancel_line`; use a refund after completion if needed.

#### Must not
- Must not allow `amend` on aggregator channels.
- Must not change a line the kitchen started through `amend`.

#### Done when
- [ ] Given direct ORD-701 `accepted` with Salmon (AED 55.00, queued), when the cashier amends it
      to Chicken (AED 45.00) with "customer agreed", revision 2 exists, AED 10.00 is refunded, and
      the grill ticket shows Chicken.
- [ ] Amend to Steak (AED 70.00) sends a AED 15.00 payment link; the ticket updates at once.
- [ ] `cancel_line` on Fries (AED 12.00, prepaid) refunds AED 12.00 and removes it from the ticket.
- [ ] `amend` on aggregator order ORD-702 is refused; `cancel_line` with "item unavailable" is
      recorded with no refund and the line is flagged for payout reconciliation.
- [ ] Cancelling the only open line is refused.

#### Out of scope
Order cancel (BL-FNB-04); `FNB.RUL.06` (base).

---

### BL-FNB-06 — 06 F&B: chase an aggregator rider who does not arrive (`FNB.RUL.16`)

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** Base: 06 Order fulfilment lifecycle · BL-FNB-04 · **Blocks:** None.
**Review items:** 06 MEDIUM "aggregator rider never arrives"

#### Goal
An aggregator order that is `ready` but not picked up 15 minutes later alerts the kitchen manager
to chase the aggregator on its tablet. At 45 minutes, a task asks the manager to record what
happened: picked up late, or cancelled by the aggregator (order cancelled with responsible party
*aggregator*, food wasted, payout line flagged for compensation).

#### Files to read
- `docs/verticals/06-fnb.md` §4.1 "When it goes wrong" (row "The aggregator's rider does not
  arrive"), §7.2 (`FNB.RUL.16`)

#### Terms
- **Compensation expected:** a flag on the order that tells the payout reconciliation to expect
  the aggregator to pay for the food.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `FNB.RUL.16` | N minutes after an aggregator order is `ready` | not picked up | notify the kitchen manager to chase the aggregator; at M minutes, task to record the outcome (picked up late, or cancelled by the aggregator with the food as wastage) | N = 15, M = 45 | on | MVP |

| Situation | What happens |
|---|---|
| The aggregator's rider does not arrive | `FNB.RUL.16` alerts the kitchen manager 15 minutes after `ready`; the manager chases the aggregator on its tablet. If the aggregator cancels, the order is cancelled with responsible party *aggregator*, the food is logged as wastage, and the line is flagged for the payout reconciliation (compensation expected). The 45-minute task goes to the kitchen manager, whose choice *cancelled by the aggregator* is the `SAL.ACL.05` approval (outcome *scrap* for every line). A flagged order with no compensation on the payout statement is listed as an unmatched difference for the accountant (`FNB.RUL.10`) |

#### Edge cases decided
- **"Not picked up":** the order is still `ready` (not `completed` or `cancelled`).
- **Task outcomes:** *picked up late* runs `complete`; *cancelled by aggregator* runs `cancel`
  with responsible party *aggregator*, outcome *scrap* for all lines (no separate `SAL.ACL.05`
  approval: the task is assigned to the kitchen manager, whose choice is the approval).
- **Payout match:** a flagged order with no matching compensation amount
  on the statement is listed as an unmatched difference for the accountant (`FNB.RUL.10`).
- **Order picked up before 45 minutes:** the task is not created; an open task closes itself.
- **Timers:** from the `ready` time; outside opening hours they still run.

#### Must not
- Must not cancel the order automatically.
- Must not refund the customer (the aggregator owns the customer's money).

#### Done when
- [ ] Given aggregator ORD-801 `ready` at 19:00, at 19:15 the kitchen manager is notified.
- [ ] At 19:45 the manager has a task; choosing *cancelled by aggregator* cancels the order with
      party *aggregator*, posts the food as wastage and sets "compensation expected".
- [ ] Given pickup at 19:30, no task is created.
- [ ] Choosing *picked up late* completes the order.

#### Out of scope
Payout reconciliation (BL-FNB-10).

---

### BL-FNB-07 — 06 F&B: refund a completed order when its delivery is reversed

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-DLV-04 (delivery under review) · BL-DLV-05 (`reverse_delivery`, `WRK.ACL.10`) · BL-SAL-14 (sales return *no goods*) · **Blocks:** BL-FNB-08
**Review items:** 06-N4-b, X-11

#### Goal
A customer says a delivered order never arrived. The delivery goes under review and the order
stays `completed`. If the kitchen manager finds the rider was wrong and reverses the delivery,
the customer gets a full refund through a sales return of kind *no goods*, with the rider as the
responsible party. On cash on delivery with no cash handed over, nothing is refunded: the same
return is resolved by credit note only. If the review is upheld, nothing changes.

#### Files to read
- `docs/verticals/06-fnb.md` §4.2 "When it goes wrong" (row "Customer says the order never
  arrived"), §7.1 *Order* (paragraph after the table), §3.3 (`WRK.ACL.10`)
- `docs/modules/wl-delivery.md` (`open_review`, `close_review`, `reverse_delivery`)

#### Terms
- **Under review:** a delivery state in wl-delivery; cash and billing are held.
- **Sales return *no goods*:** a refund after completion with nothing coming back.

#### Spec excerpt (authoritative for this issue)

| Situation | What happens |
|---|---|
| Customer says the order never arrived | The delivery goes `under_review` (cash and billing held); the kitchen manager checks the proof. If the rider was wrong, `reverse_delivery` with approval (`WRK.ACL.10`) and a full refund; otherwise the review closes and the order stays completed. The refund is a sales return of kind *no goods* on every line (responsible party *rider*), resolved at once; the kitchen manager's reversal approval counts as its `SAL.ACL.06` approval. On cash on delivery, nothing is refunded if no cash was handed over for the order: the same return is then resolved by credit note only (no refund payment) and the order's cash is removed from the rider's expected cash; otherwise the cashier refunds in cash or by transfer and records the method |

§7.1: "A delivered order the customer says never arrived stays `completed`; the delivery goes
`under_review` in wl-delivery and a refund follows `reverse_delivery` or a sales return of kind
*no goods*."

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `WRK.ACL.10` *(shared)* | Reversing a delivery marked delivered, or marking it lost by hand, needs the kitchen manager; the rider neither requests nor approves | `SOD` | rider, kitchen manager | — |

#### Edge cases decided
- **Refund route:** on `DeliveryReversed` for an F&B order, the system creates a sales return of
  kind *no goods* for every line, category *other* with text "not delivered (delivery reversed)",
  responsible party *rider*, resolved at once with *refund* (prepaid, or cash on delivery with
  cash handed over) or with *credit note* only (cash on delivery, no cash handed over).
- **`SAL.ACL.06` approval:** the kitchen manager's reversal approval counts;
  no second approval for the generated return.
- **Cash on delivery:** if no cash was handed over for the order, the return is still created and
  resolved by credit note only: no refund payment, the order's amount due is cleared, and the
  order's cash is removed from the rider's expected cash (wl-delivery / wl-payments). If cash
  was handed over, the return is resolved with *refund* in cash or by transfer and the cashier
  records the method.
- **Partial complaint** (one item missing): not this issue; a normal *no goods* return (BL-SAL-14).
- **Order state:** stays `completed`; the order's amount refunded becomes the full amount.

#### Must not
- Must not reopen or cancel the order.
- Must not let the rider request or approve the reversal.

#### Done when
- [ ] Given prepaid ORD-901 (AED 95.00) `completed`, when the kitchen manager reverses the
      delivery after review, a *no goods* return of AED 95.00 with party *rider* is resolved,
      AED 95.00 is refunded, and ORD-901 stays `completed` with amount refunded AED 95.00.
- [ ] Given COD ORD-902 (AED 70.00) with no cash handed over, the reversal creates a *no goods*
      return of AED 70.00 resolved by credit note, no refund payment, and the rider's expected
      cash no longer includes ORD-902.
- [ ] When the review closes upheld, no return exists.
- [ ] The rider cannot open the reversal.

#### Out of scope
Review and reversal mechanics (BL-DLV-04, BL-DLV-05); rider record (BL-FNB-08).

---

### BL-FNB-08 — 06 F&B: record rider faults and report refunds and failures by responsible party

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-SAL-14 · BL-FNB-02 · BL-FNB-07 · **Blocks:** None.
**Review items:** 06-N4-b

#### Goal
Complaints after delivery (cold, late, missing, wrong item) are refunded through a sales return
of kind *no goods* with a category and a responsible party. 06 adds two things: a fault on the
rider is written to the rider's record, and the reports show failed own-rider deliveries by reason
and refunds after completion by category and responsible party, including aggregator refunds
deducted from payouts.

#### Files to read
- `docs/verticals/06-fnb.md` §4.2 "When it goes wrong" (row "Complaint after delivery"), §10
  (KPIs: failed deliveries, refunds after completion)
- `docs/modules/wl-sales.md` §4 *Sales return* ("Refund after completion")

#### Terms
- **Rider's record:** the rider's employee record in wl-hr (`HRM.EMP.01`), with its timeline.

#### Spec excerpt (authoritative for this issue)

| Situation | What happens |
|---|---|
| Complaint after delivery: cold, late, missing or wrong item | A sales return of kind *no goods* from the order or the complaint ticket, with the category and the responsible party (business, rider, supplier, customer). Refund full or partial per line; above AED 50 the kitchen manager approves (`SAL.ACL.06`). A rider fault (responsible party *rider*, here, at `close_failed` or on a reversed delivery) adds a *rider fault* entry (order, category, amount, date) to the timeline of the rider's employee record, for the rider of the delivery's last run; the kitchen manager and HR see it, other riders do not; it never touches pay |

§10: "Failed own-rider deliveries by reason; refunds after completion by category and responsible
party, including aggregator refunds deducted from payouts."

#### Edge cases decided
- **Rider fault entry:** a timeline entry of type *rider fault* on the
  rider's employee record (order number, category, amount, date), visible to the kitchen manager
  and HR; not visible to other riders. Created on `ReturnResolved` with party *rider*, and on
  `close_failed` with party *rider* (BL-FNB-02) and on the generated return of BL-FNB-07.
- **Which rider:** the rider of the delivery's last run.
- **Reports:** amounts net of VAT and gross both shown; period by business date (BL-FNB-15);
  filters brand, outlet, channel.
- **Aggregator refunds:** taken from the returns created by payout `match` (BL-FNB-10).

#### Must not
- Must not deduct anything from the rider's pay (no salary effect).

#### Done when
- [ ] Given a *no goods* return on ORD-950 (cold, AED 30.00, party *rider*, rider R), R's record
      shows one *rider fault* entry for ORD-950.
- [ ] The refunds report for October shows AED 30.00 under *cold* / *rider*.
- [ ] The failed deliveries report lists ORD-504 under customer unreachable.
- [ ] The refunds report lists ORD-1001's aggregator refund (BL-FNB-10) AED 25.00 under *cold* /
      *business*, the party set on `match`; the party values are the wl-sales list (business,
      rider, aggregator, supplier, customer).

#### Out of scope
The refund itself (BL-SAL-14); payout refund lines (BL-FNB-10).

---

### BL-FNB-09 — 06 F&B: refuse a duplicate payout statement and void a wrong one

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: 06 Payout statement lifecycle · BL-KRN-03 (void pattern, `PLT.ACL.09`) · BL-SAL-14 (sales return `reverse`) · **Blocks:** BL-FNB-10
**Review items:** 06 MEDIUM "payout file revert", X-19

#### Goal
The accountant imports aggregator payout statements by file. A statement with the same aggregator
and reference as one already imported is refused. A wrong file imported by mistake is voided
before it is settled: matching, the sales returns created from its refund lines, and the expected
receivable are reversed, and the right file is imported. The number is kept.

#### Files to read
- `docs/verticals/06-fnb.md` §4.3 "When it goes wrong" (row "Wrong file imported, or a file
  imported twice"), §6 (Payout statement), §7.1 *Payout statement* (`voided`, `import`, `void`),
  §7.2 settings (records that support `void`)
- `docs/modules/wl-kernel.md` §5.12 (void), `PLT.ACL.09`

#### Terms
- **Statement reference:** the aggregator's own identifier for the payout (unique per aggregator).
- **Expected receivable:** the net payout booked as owed by the aggregator on `match`.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `voided` | neutral | yes | Wrong file, imported in error; matching and receivable reversed |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `import` | — | imported | `reconciliation.manage` | aggregator and statement reference (trimmed, case-insensitive) not already on a statement that is not `voided` (a duplicate is refused) | lines matched to orders by aggregator order number |
| `void` | imported, matched, disputed | voided | `reconciliation.manage` | reason given; no bank receipt allocated (`PLT.ACL.09`) | matching and the expected receivable reversed; sales returns from refund lines run wl-sales `reverse` (reason *statement voided*) and the orders' amount refunded reduced back (no money moves); the reference may be imported again; number kept |

#### Edge cases decided
- **Reference comparison:** trimmed, case-insensitive, per aggregator.
- **A voided statement's reference:** may be imported again (the void says
  the file was wrong; the right file can carry the same reference).
- **Approval:** `PLT.ACL.09` asks for approval when money was posted; a `matched` or `disputed`
  statement has a receivable booked, so its void needs approval (BL-KRN-03 approver); an
  `imported` one does not.
- **Sales returns from refund lines:** each runs wl-sales `reverse` (state `reversed`, reason
  *statement voided*; BL-SAL-14), so the order's amount refunded and the refund KPI are reduced
  back; no money moves (none moved on match).
- **Settled statement:** void refused; corrections go through `dispute` on a later statement.
- **Open dispute task:** closed with the void reason.

#### Must not
- Must not delete a voided statement or reuse its number.
- Must not void once a bank receipt is allocated.

#### Done when
- [ ] Given Talabat statement "TB-2026-10-W2" imported, importing it again (also " tb-2026-10-w2 ")
      is refused and points to the existing statement.
- [ ] Given that statement `matched` with receivable AED 12,400.00 and two refund returns, `void`
      with reason "wrong week" needs approval; after approval it is `voided`, the receivable is
      reversed, both returns are `reversed` with reason *statement voided* and the two orders'
      amount refunded go back to 0.
- [ ] After the void, "TB-2026-10-W2" can be imported again.
- [ ] `void` on a `settled` statement is refused.

#### Out of scope
Refund lines on `match` (BL-FNB-10).

---

### BL-FNB-10 — 06 F&B: turn aggregator refund lines into sales returns on payout match

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: 06 Payout statement lifecycle · BL-SAL-14 (sales return *no goods*, resolution *refunded by the channel*, `reverse`) · BL-KRN-04 (late entry) · BL-FNB-09 · **Blocks:** None.
**Review items:** 06-N4-a

#### Goal
Aggregators refund customers and deduct it from the payout. On `match`, each refund line becomes
a sales return of kind *no goods* on its order, resolved as *refunded by the channel* (no
money leaves the outlet), with the aggregator's reason as category and a responsible party, so
the order's amount refunded and the refund KPI are true. A refund the kitchen did not cause is
disputed. An order on the statement that was never recorded is entered as a late entry.

#### Files to read
- `docs/verticals/06-fnb.md` §4.3 "When it goes wrong" (rows "Aggregator refunds a customer",
  "An order on the statement is missing here"), §7.1 *Payout statement* (`match`, `dispute`,
  `resolve`)
- `docs/modules/wl-sales.md` §4 *Sales return*; `docs/modules/wl-kernel.md` §5.13 (late entry)

#### Terms
- **Refund line:** a statement line with a refund amount for an order.
- **Reason mapping:** a per-aggregator preset from the aggregator's refund reason text to a
  complaint category and a proposed responsible party.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `match` | imported | matched | `reconciliation.manage` | no unmatched lines, or each one explained | receivable booked for the net payout; each refund line creates a sales return of kind *no goods* on its order (resolution *refunded by the channel*: no credit note, no refund payment and no `SAL.ACL.06` approval, as no money leaves the outlet; settled through the payout) with the aggregator's reason and a responsible party |
| `dispute` | imported, matched | disputed | `reconciliation.manage` | disputed lines and reason | follow-up task created |
| `resolve` | disputed | matched | `reconciliation.manage` | aggregator answer recorded | adjustments booked; a refund the aggregator reverses runs wl-sales `reverse` on its sales return (reason *aggregator took the refund back*) |

| Situation | What happens |
|---|---|
| Aggregator refunds a customer and deducts it from the payout | On `match`, each refund line becomes a sales return of kind *no goods* on the order (resolution *refunded by the channel*, no money out from the outlet) with the aggregator's reason as category and responsible party *business* or *aggregator*. The order's amount refunded and the refund KPI update. A refund the kitchen did not cause is disputed with the aggregator |
| An order on the statement is missing here | Listed as unmatched; the cashier enters it as a late entry with the aggregator order number, or the line is explained |

#### Edge cases decided
- **Responsible party:** proposed from the reason mapping; an unmapped reason must be set by the
  accountant (*business* or *aggregator*) before `match` is allowed.
- **Approval:** no `SAL.ACL.06` approval for these returns (no money leaves
  the outlet); the accountant is the recorder.
- **Resolution value:** *refunded by the channel* (the wl-sales name), a sales return
  resolution that creates no credit note and no refund payment (BL-SAL-14).
- **Party values:** the wl-sales list; the kitchen's own fault is *business*.
- **Partial refunds:** the return carries the line amount from the statement, per order (fils as
  on the statement).
- **Refund taken back by the aggregator** (`resolve` with the aggregator reversing it): the
  return runs wl-sales `reverse` (state `reversed`, reason *aggregator took the refund back*)
  and the amount refunded is reduced (same handling as BL-FNB-09).
- **Late entry:** the missing order is entered by the cashier with the aggregator order number and
  the business time from the statement, with the late-entry flag (BL-KRN-04); matching re-runs.
- **Idempotency:** one return per refund line; re-matching after a dispute does not create a
  second.

#### Must not
- Must not create a refund payment or credit note for aggregator refund lines.
- Must not `match` while a refund line has no responsible party.

#### Done when
- [ ] Given a statement with a refund line of AED 25.00 on ORD-1001 reason "Food was cold" mapped to
      category *cold* and party *business*, on `match` a *no goods* return of AED 25.00 resolved
      *refunded by the channel* exists, ORD-1001's amount refunded is AED 25.00, and no payment
      left the outlet.
- [ ] A line with reason "Customer changed mind" (unmapped) blocks `match` until a party is set.
- [ ] Disputing that line and resolving it with "aggregator reverses the refund" marks the return
      `reversed` with reason *aggregator took the refund back* and ORD-1001's amount refunded
      back to 0.
- [ ] Given a statement line for ORD-1099 not recorded, the cashier enters it as a late entry with
      that number and `match` passes.

#### Out of scope
Void and duplicates (BL-FNB-09); reports (BL-FNB-08).

---

### BL-FNB-11 — 06 F&B: hold, release and discard production batches with their lot

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: 06 Production batch lifecycle · BL-STK-11 · BL-STK-12 · BL-STK-14 · **Blocks:** BL-FNB-13
**Review items:** 06-N10-c, X-09

#### Goal
A central kitchen batch (sauce, marinated protein, dough) made from a recalled or suspect
ingredient lot must not be used. The batch records the ingredient lots it used (genealogy) when
it starts, cannot start with an ingredient lot on hold, goes `on_hold` when its own lot is held,
back to `produced` when the lot is released, and to `discarded` with wastage when the lot is
discarded.

#### Files to read
- `docs/verticals/06-fnb.md` §7.1 *Production batch* (states, rows `start`, `hold`, `release`,
  `discard`), §4.4 "When it goes wrong" (recall row)
- `docs/modules/wl-inventory.md` §4 *Lot* (`hold`, `release`, `discard`; events)

#### Terms
- **Lot genealogy:** which ingredient lots went into which output lot; used by recall traces.
- **Whole-lot hold:** a hold on a lot everywhere (`LotHeld`); a location-scoped hold (fridge)
  does not emit `LotHeld`.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `on_hold` | warning | no | Its lot is held (recall of an ingredient lot, quality, temperature); not usable |
| `discarded` | error | yes | Expired, failed quality or discarded after a hold; logged as wastage |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `start` | planned | in_production | `production.manage` | ingredients in stock; no ingredient lot on hold | ingredient lots used are recorded (lot genealogy for recalls) |
| `finish` | in_production | produced | `production.manage` | actual yield recorded | ingredients consumed; lot created with the expiry entered at `finish` (required; not later than the earliest expiry of the ingredient lots used); labels printed; if an ingredient lot used is on hold at that moment, the output lot is created on hold with the same reason and the batch runs `hold` at once |
| `hold` | produced | on_hold | system (`LotHeld` for the batch's lot, e.g. a recall traced through an ingredient lot) | — | batch blocked at every outlet it was transferred to; chef notified |
| `release` | on_hold | produced | system (`LotReleased`) | — | — |
| `discard` | produced, on_hold | discarded | `production.manage` (from `produced`), or system (`LotDiscarded`) | reason; `STK.ACL.02`; from `on_hold` only through the stock hold line decision *discard*, so the hold's approvals apply (`STK.ACL.05` for a recall) | wastage entry created and posted once for the remaining quantity: from `produced` the batch posts the `wastage` movement; on `LotDiscarded` the lot's discard movement is the posting and the entry links it |

#### Edge cases decided
- **Lots picked at start:** proposed earliest expiry first, excluding held and expired lots; the
  user may change, never to a held lot.
- **Lot held after start:** a batch `in_production` is not covered by `hold` (the batch has no
  output lot yet). `finish` creates the output lot directly `on_hold` when
  any ingredient lot is on hold at that time, and the batch goes `produced` → `on_hold` at once.
- **Location-scoped holds** (temperature): the batch stays `produced`; only that stock is blocked.
- **Manual `discard` from `on_hold`:** offered as the stock hold line
  decision *discard* (so the hold's approvals apply, `STK.ACL.05` for a recall); the batch then
  moves through `LotDiscarded`.
- **Wastage once:** on `LotDiscarded` the lot discard already posts the wastage movement; the
  batch's wastage entry links it and posts no second movement.
- **Partially consumed batch:** hold, release and discard act on the remaining quantity.
- **Output lot expiry:** entered by the chef at `finish` (required); an expiry later than the
  earliest expiry of the ingredient lots used is refused.

#### Must not
- Must not start a batch with a held ingredient lot.
- Must not let a held batch be transferred, issued or consumed.

#### Done when
- [ ] Given tomato lot T7 on hold, `start` of batch B1 picking T7 is refused.
- [ ] Batch B2 started with T5 records T5 in its genealogy.
- [ ] When B2's lot L20 is held by a recall, B2 is `on_hold`, its stock at Outlets 1 and 2 is
      blocked, and the chef is notified; on `LotReleased` it is `produced`.
- [ ] Given B2 made with T5 (expiry 2026-10-20), `finish` with expiry 2026-10-22 is refused; with
      2026-10-18 it creates lot L20 with expiry 2026-10-18.
- [ ] On `LotDiscarded` for L20 (8 kg left), B2 is `discarded` and exactly one wastage movement of
      8 kg exists.
- [ ] A temperature hold on Outlet 1's fridge leaves B2 `produced`.

#### Out of scope
Recall trace (BL-FNB-13); hold mechanics (BL-STK-11, BL-STK-12).

---

### BL-FNB-12 — 06 F&B: hold a cold unit's stock on an out-of-range reading (`FNB.RUL.05`)

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-STK-11 (`StockHoldService`) · BL-STK-12 (line decisions) · BL-STK-13 (`STK.RUL.07`) · Base: wl-ticketing tickets and checklists · **Blocks:** None.
**Review items:** 06-N10-e

#### Goal
When a fridge or freezer reading is out of range, the stock in that unit must not be used until
the chef decides. `FNB.RUL.05` opens an equipment ticket and places a stock hold (reason
`temperature`, the reading as reference). The chef decides per line: release, discard (wastage)
or return to supplier, recording the reading and how long the food was out of range. Food that is
not released is never sold.

#### Files to read
- `docs/verticals/06-fnb.md` §4.6 "When it goes wrong" (row "A fridge or freezer reads out of
  range"), §3.3 (`STK.ACL.02`, `STK.ACL.05`, `FNB.ACL.16`), §7.2 (`FNB.RUL.05`, `STK.RUL.07`,
  compliance-critical list), §9 (Returned and held food)
- `docs/modules/wl-inventory.md` §4 *Stock hold*, `StockHoldService`

#### Terms
- **Cold unit:** a fridge or freezer with a temperature log and a range (Dubai: cold ≤ 5°C).
- **Stock hold:** a wl-inventory record that blocks stock until each line is decided.
- **The chef:** the outlet's head chef (role *Head chef / central kitchen chef*, scoped to the
  outlet), holder of `lots.release`; the outlet's kitchen manager is the deputy (§7.2 settings).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `FNB.RUL.05` | temperature reading recorded | outside the unit's range | create an equipment ticket; place a stock hold (reason `temperature`, the reading as reference) on the stock in that unit through `StockHoldService` (each temperature-logged cold unit is a stock location of the outlet, required before the unit can log temperatures); notify chef and kitchen manager (the chef decides each line) | range per unit (Dubai: cold ≤ 5°C, hot ≥ 60°C ✔ [S5]) | on | MVP |
| `STK.RUL.07` *(shared)* | a stock hold stays open H hours | lines undecided | remind the deciders (chef); after 2H, the kitchen manager | H = 2 (module default 24; lowered for food) | on | MVP |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `STK.ACL.05` *(shared)* | Releasing stock held for a recall needs a second holder of `lots.release`; a temperature hold is released by one (the chef) | `SOD` | chef, kitchen manager | listed reason: `recall` |
| `STK.ACL.02` *(shared)* | Wastage entries above a value need the kitchen manager's approval; the person who records wastage cannot approve it | `LIM`, `SOD` | chef, cook | AED 200 |
| `FNB.ACL.16` | Food that comes back from a failed delivery, or was held for temperature and not released by the chef, is never sold or reused; it is logged as wastage | `STA` | outlet staff | on |

Situation row: "The chef (`lots.release`) decides per line: release, discard (wastage,
`STK.ACL.02`) or return to supplier, recording the reading and how long the food was out of
range. Held food cannot be used meanwhile; stock moved into the unit while its hold is open is added
to that hold as new lines (the receiving and transfer screens warn first). `STK.RUL.07` reminds
after 2 hours and escalates to the kitchen manager after 4."

#### Edge cases decided
- **Unit → stock:** each temperature-logged cold unit is mapped to a stock
  location (sub-location of the outlet); a unit cannot be set up for temperature logs without one.
- **Repeated out-of-range readings:** one open ticket and one open hold per unit; later readings
  are added as references, no second hold.
- **Stock moved into the unit after the hold:** the receiving and transfer screens warn "Fridge 2
  is on temperature hold"; if the user goes on, the moved stock is added to the open hold as new
  lines (same reason and reference) and is decided like the others. It is never available while
  the hold is open.
- **Decision fields:** each decided line needs the reading and the minutes out of range (number).
- **Release by one person:** the chef releases a temperature line alone (not a listed reason).
- **Discard above AED 200:** needs the kitchen manager, not the chef who recorded it.
- **Compliance-critical:** a failure of `FNB.RUL.05` alerts the owner (BL-KRN-08).

#### Must not
- Must not let held stock be consumed by recipe, transferred or counted as available.
- Must not release a line without the reading and minutes recorded.

#### Done when
- [ ] Given Fridge 2 (range ≤ 5°C, location OUT1-F2) with 6 kg chicken and 2 kg cream, a reading of
      9°C at 14:00 creates one equipment ticket and one hold on both lines; chef and kitchen
      manager are notified.
- [ ] A second reading of 8°C at 14:30 adds to the same hold and ticket.
- [ ] An order needing cream from OUT1-F2 while it is held posts no consumption from the held
      cream: the short quantity becomes a pending issue (held quantity is never consumed under
      either negative stock policy, wl-inventory §3).
- [ ] 3 kg of chicken transferred into OUT1-F2 at 15:00 shows the warning and is added to the open
      hold as a new line.
- [ ] At 16:00 undecided lines remind the chef; at 18:00 the kitchen manager.
- [ ] The chef releases cream with reading 9°C, 40 minutes; discards chicken worth AED 240.00,
      which needs the kitchen manager's approval; discarded chicken is wastage and never sold.

#### Out of scope
Hold mechanics (BL-STK-11 to BL-STK-13); receiving rejections (settings BL-FNB-17).

---

### BL-FNB-13 — 06 F&B: trace a recalled ingredient lot to orders and customers

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-FNB-11 · BL-STK-14 · BL-STK-15 (`TracePartyResolver`) · BL-STK-16 · **Blocks:** None.
**Review items:** 06-N10-c, X-09

#### Goal
When a supplier or authority recalls an ingredient lot, the trace must follow transfers and
production batches to orders. 06 provides the resolver: direct orders → their customer; aggregator
and counter sales without a known customer are listed per outlet and day. The owner decides on a
public notice and tells the aggregators. Authority notification is open (AU-20).

#### Files to read
- `docs/verticals/06-fnb.md` §4.4 "When it goes wrong" (recall row), §7.2 settings (recall
  owner), §9 (Recalls)
- `docs/modules/wl-inventory.md` §5.7 (recall), `TracePartyResolver`
- `docs/OPEN-ITEMS.md` AU-20

#### Terms
- **Affected line:** a subject + party in the recall trace with a notice status.
- **Unknown-customer group:** aggregator and counter sales of one outlet on one business day.

#### Spec excerpt (authoritative for this issue)

| Situation | What happens |
|---|---|
| Supplier or authority recalls an ingredient lot, or a contaminated lot is found | The head chef or owner opens a recall (wl-inventory §5.7). Every lot in scope is held at every outlet and in transit; the trace follows transfers, production batches (the batches made from the lot are held too) and orders. Sales consume ingredients by recipe without recording which lot went into the food, so an order is linked to a lot only as *possible*: when its outlet had the lot (or a batch lot made from it) on hand at the order's time, between the lot's first receipt or transfer into the outlet and the time it reached zero there or was held; never *confirmed*. Direct-order customers are found through the order (`TracePartyResolver`: order → customer; one notice per customer listing their possible orders); aggregator and counter sales without a known customer are listed per outlet and day, and the owner decides on a public notice and tells the aggregators. Held stock is discarded, returned to the supplier or released by a second person (`STK.ACL.05`). Authority notification is `⚠ open: authority (AU-20)` (wl-inventory §10) |

| Setting | 06 value |
|---|---|
| Recall owner | owner / operations director with the head chef |

#### Edge cases decided
- **Orders and lots:** sales consume ingredients by recipe (theoretical) without recording lots.
  An order is linked as *possible* when its outlet had the lot (or a batch
  made from it) on hand at the order's time, between the lot's first receipt or transfer into the
  outlet and the time it reached zero there or was held. Orders are never marked *confirmed*.
- **Direct orders:** one affected line per customer, listing all their possible orders; customer
  template notice (EN/AR) by SMS or email.
- **Unknown-customer groups:** one line per outlet and business day with order count and channels;
  the owner gets a task "decide public notice and tell aggregators" listing the groups; closing
  the task records the decision. Lines are `waived` with that decision when no notice is sent.
- **Customers who deleted their account:** listed without contact; `waived` with reason.
- **AU-20 open:** no authority notification is built; the recall report is exportable.

#### Must not
- Must not message aggregator customers directly (no contact data).
- Must not mark an order *confirmed* when lots were not recorded.

#### Done when
- [ ] Given lot T7 received at the central kitchen on 2026-10-01, used in batch B2 transferred to
      Outlet 1 on 2026-10-03 and used up on 2026-10-06, a recall of T7 holds B2's lot and lists
      direct orders of Outlet 1 from 2026-10-03 to 2026-10-06 with recipes containing B2's item as
      *possible*, grouped per customer.
- [ ] Talabat and counter orders of Outlet 1 on those days appear as 4 unknown-customer groups
      and the owner gets the decision task.
- [ ] Customer C with 2 possible orders gets one notice.

#### Out of scope
Recall opening, holds, notices engine (BL-STK-14 to BL-STK-16); batch holds (BL-FNB-11).

---

### BL-FNB-14 — 06 F&B: pause an outlet's channels during an outage and resume (`FNB.RUL.15`)

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-KRN-07 (device heartbeat connections) · BL-KRN-04 (late entry) · BL-KRN-06 (recovery) · **Blocks:** None.
**Review items:** 06-N7-c, X-19

#### Goal
When an outlet loses internet or power, its kitchen devices go silent. After 5 minutes in opening
hours, `FNB.RUL.15` shows the outlet's brands as "temporarily closed" on the D2C site and tells the
kitchen manager and area manager to pause aggregators on their tablets. The kitchen works on
paper. When the devices answer again, the kitchen manager is asked to resume the channels and to
enter the paper orders as late entries.

#### Files to read
- `docs/verticals/06-fnb.md` §4.6 "When it goes wrong" (row "Internet or power outage at the
  outlet"), §7.2 (`FNB.RUL.15`)
- `docs/modules/wl-kernel.md` §5.13 (late entry), §5.14 (connections), `PLT.ACL.10`

#### Terms
- **Kitchen devices connection:** the outlet's kitchen displays and tills grouped as one device
  connection (BL-KRN-07), active during opening hours, `down` after 5 minutes.
- **Paused (D2C):** the brand shows "temporarily closed" for that outlet and takes no orders there.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `FNB.RUL.15` | an outlet's kitchen devices stop answering for N minutes during opening hours | — | pause the outlet's brands on the D2C site ("temporarily closed"); notify the kitchen manager and the area manager to pause aggregators on their tablets; when devices answer again, ask the kitchen manager to resume and to enter paper orders as late entries, repeated every R minutes while devices are up and channels paused (never resumed automatically) | N = 5, R = 15 | on | MVP |

| Situation | What happens |
|---|---|
| Internet or power outage at the outlet | `FNB.RUL.15` pauses the outlet's brands on the D2C site when its devices stop answering; staff pause aggregators on their tablets. The kitchen works from aggregator tablet printouts and paper tickets; counter sales go on paper. When the outlet is back, orders, sales, wastage and temperature readings are entered with the late-entry flag and their business time (wl-kernel §5.13); after 24 hours, or into a submitted daily closing, the area manager approves (`PLT.ACL.10`). The kitchen manager resumes the channels |

#### Edge cases decided
- **Trigger:** the kitchen devices connection going `down` (`IntegrationStatusChanged`); silence
  outside opening hours is ignored.
- **Scope of the pause:** only this outlet's brand assignments; a brand also cooked by another
  outlet stays open there for its delivery areas.
- **D2C orders already `placed` at the outlet:** left to `FNB.RUL.01` (auto-reject and refund
  after 10 minutes), since nobody can see them.
- **Resume:** manual by the kitchen manager (or area manager); the system never resumes itself.
  The prompt repeats every 15 minutes (R) while devices are up and channels paused.
- **Late entries:** order time, counter sale time, wastage, temperature readings and delivery proof
  take the business time; entries into a submitted closing reopen it via BL-FNB-15.
- **Flapping:** a second `down` while already paused does nothing new.

#### Must not
- Must not resume channels automatically.
- Must not pause other outlets.

#### Done when
- [ ] Given Outlet 1 open 11:00–02:00 cooking brands A and B, devices silent from 20:00, at 20:05
      A and B show "temporarily closed" for Outlet 1's areas and both managers are notified.
- [ ] Brand A cooked also by Outlet 2 stays orderable in Outlet 2's areas.
- [ ] Devices silent at 03:00 trigger nothing.
- [ ] At 21:10 devices answer; the kitchen manager is asked to resume and enter paper orders;
      channels stay paused until the manager resumes.
- [ ] A paper order entered at 21:30 with business time 20:20 carries the late-entry flag.

#### Out of scope
Heartbeat connections (BL-KRN-07); late-entry approval mechanics (BL-KRN-04).

---

### BL-FNB-15 — 06 F&B: assign orders to a business day by the cut-off and reopen a closing for late entries

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:medium` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Till session (daily closing) · BL-KRN-04 · BL-PAY-19 (till handover from a closed session) · **Blocks:** None.
**Review items:** 06 MEDIUM "business-day cut-off", 06-N7-c, D-12

#### Goal
Late-night trading must not be split across two closings. Each order gets a business date by
the outlet's cut-off (default 04:00): orders before it belong to the previous day. The daily
closing can be submitted only after the cut-off. When late entries after an outage land in a
submitted closing, the area manager reopens it. A cash shortage at closing goes to the till's
cash handover.

#### Files to read
- `docs/verticals/06-fnb.md` §4.6 step 4, §4.6 "When it goes wrong" (rows "Till cash is short at
  closing", "Internet or power outage"), §6 (Order: business date), §7.1 *Daily closing*
  (`submit`, `reopen`), §7.2 settings (business-day cut-off)
- `docs/modules/wl-sales.md` §4 *Till session*, vertical mapping 06

#### Terms
- **Business date:** the trading day an order belongs to.
- **Cut-off:** local time (Asia/Dubai) at which the business day changes; per outlet.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | open, reopened | submitted | `closing.submit` | closing checklist completed; cash counted; business-day cut-off passed (orders up to the cut-off belong to this day), also for an outlet that closes earlier: it submits after the cut-off, e.g. next morning (`FNB.RUL.08`) | cash variance calculated; a shortage goes to the till's cash handover (`TillSessionClosed`), recovered from the cashier or written off (`FIN.ACL.06`, AED 20) |
| `approve` | submitted | approved | `closing.approve` | — | — |
| `reopen` | submitted | reopened | `closing.approve` (area manager) | reason given (also: late entries after an outage, `PLT.ACL.10`) | kitchen manager notified |

States: `open`, `submitted`, `reopened`, `approved` (terminal: reviewed by the area manager or
accountant).

§4.6 outage row (part): "after 24 hours, or into a submitted daily closing, the area manager
approves (`PLT.ACL.10`) and the closing is `reopened`. A late entry into an `approved` closing is
refused; the area manager gets a task to record it as an adjustment in the next day's closing."

§4.6 step 4: "Each outlet has a business-day cut-off (default 04:00): orders before it belong to
the previous day's closing, so late-night trading is not split."

#### Edge cases decided
- **Which time sets the business date:** the order's creation time (business time for a late
  entry); an order created 03:59 and completed 04:10 belongs to the previous day.
- **Submitting before the cut-off:** refused with the time it opens (e.g. "after 04:00").
  An outlet closing at 23:00 may still submit only after 04:00; the kitchen
  manager can submit next morning (`FNB.RUL.08` chases at 10:00).
- **Late entry into a submitted closing:** saved with the flag and an approval request to the area
  manager (`PLT.ACL.10`); on approval, the closing is `reopened` with reason "late entries after
  an outage", then counted and submitted again. An `approved` closing is not reopened (terminal);
  the late entry is refused and the area manager gets a task to record it
  as an adjustment in the next day's closing.
- **Cut-off change:** applies from the next business day only.
- **Shortage:** handed to the till's handover (BL-PAY-19); write-off up to AED 20 without
  approval, above by the area manager (settings BL-FNB-16).

#### Must not
- Must not split an order across two business days.
- Must not let anyone but the area manager reopen a submitted closing.

#### Done when
- [ ] Given cut-off 04:00, an order at 2026-10-16 01:30 has business date 2026-10-15 and appears in
      the 2026-10-15 closing.
- [ ] Submitting the 2026-10-15 closing at 2026-10-16 03:00 is refused; at 04:05 it succeeds.
- [ ] A late entry with business time 2026-10-15 21:00 entered on 2026-10-16 09:00 into the
      submitted closing creates an approval for the area manager; on approval the closing is
      `reopened` and the kitchen manager is notified.
- [ ] A closing with a AED 35.00 shortage creates a till handover with AED 35.00 short.
- [ ] Given the 2026-10-15 closing `approved`, a late entry with business time 2026-10-15 22:00 is
      refused, no `reopen` is offered, and the area manager gets a task to record it as an
      adjustment in the 2026-10-16 closing.

#### Out of scope
Till session mechanics (Epic wl-sales); handover recovery and write-off (BL-PAY-17, BL-PAY-19).

---

### BL-FNB-16 — 06 F&B: configure shared negative-flow settings (delivery, sales, payments, cash)

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-DLV-02 · BL-DLV-03 · BL-DLV-05 to BL-DLV-10 · BL-SAL-01 to BL-SAL-09 · BL-SAL-14 · BL-PAY-02 to BL-PAY-07 · BL-PAY-17 · BL-PAY-19 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · **Blocks:** None.
**Review items:** X-06, X-07, X-11, X-12, X-14, D-12, 06-N3-a, 06-N9-b

#### Goal
Seed 06's values for the shared delivery, sales, payment and cash mechanisms: perishable policy,
partial acceptance, COD tolerance, refunds after completion, amendments off on aggregator
channels, cancel outcomes, online payment timing, chargebacks, failed refunds, and cash custody
for riders (MVP, D-12) and tills.
It also seeds the 06 values for the cash-flow forecast alert (`FIN.CFL.01`) and the
company budget with owner drawings (`FIN.BUD.01`), added on 2026-10-16: `FIN.RUL.23`,
`FIN.RUL.24` and `FIN.ACL.21`.

#### Files to read
- `docs/verticals/06-fnb.md` §3.3 (shared rules), §4.2 "When it goes wrong", §4.6 (till), §7.2
  (settings table, `FIN.RUL.19`)
- `docs/modules/wl-delivery.md` §7; `docs/modules/wl-sales.md` §7; `docs/modules/wl-payments.md` §7
- `docs/modules/wl-invoicing.md` §5.14–§5.16 and §7 (cash-flow forecast, company budget, owner
  drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments)
- `docs/OPEN-ITEMS.md` L-20

#### Terms
- **Seeded setting:** the value installed for a 06 deployment; the module provides the mechanism.

#### Spec excerpt (authoritative for this issue)

| Setting | 06 value | Spec reference |
|---|---|---|
| Perishable source types | F&B orders (own-rider direct orders) | §4.2 |
| Perishable policy (`WRK.RUL.16`) | *return* to the outlet (replaces the wl-delivery default *discard* for cooked food); *discard* only when the rider cannot return (run aborted), with a photo | §7.2 settings |
| Perishable hand-over window | the brand's delivery promise | §4.2; wl-delivery §7 |
| Partial acceptance at proof of delivery | on (the customer may refuse a wrong item); the refused line is refunded by a *no goods* return (category *wrong item*, party *business*; above AED 50 the kitchen manager approves), on cash on delivery only the accepted lines are collected, and the refused food is wastage on `DeliveryRefusedReceived` | §7.2 settings; §4.2 row "Customer refuses one item at the door" |
| COD difference tolerance and approver (`WRK.ACL.09`) | AED 5; kitchen manager | §3.3, §7.2 |
| Delivery reversal and manual lost (`WRK.ACL.10`) | kitchen manager; the rider neither requests nor approves | §3.3 |
| Delivery lost after (`WRK.RUL.17`) | 7 days | §7.2 |
| Midday-break warning on rider runs (`WRK.RUL.15`) | on | §7.2 |
| Refund after completion with nothing returned (`SAL.ACL.06`) | AED 50; categories as wl-sales | §7.2 settings |
| Paid-order cancel or refund needing a manager (`SAL.ACL.01`) | AED 50, or after preparation started | §3.3 |
| Cancel after the kitchen started (`SAL.ACL.05`): outcomes offered | *return to stock*, *scrap*; remake = new order, outcome *scrap*; fee 0 | §3.3 |
| Order amendments on aggregator channels | off (changed on the aggregator) | §7.2 settings |
| Unique external order number per channel | on | §4.1 |
| Pending online payment check (`FIN.RUL.19`) | 10 minutes | §7.2 |
| Late payment after reject or cancel (`SAL.RUL.06`) | refunded in full without staff action; customer and kitchen manager told | §4.2 |
| Chargeback acceptance (`FIN.ACL.18`) | second person, AED 0 (always) | §3.3 |
| Refund retry (`FIN.RUL.21`) | once after 24 hours, then another method or customer credit | §4.2 |
| Cash holders | own riders per person per shift; tills in till session mode | §7.2 settings; D-12 |
| Counter of rider and till cash (`FIN.ACL.05`) | kitchen manager or cashier, never the holder | §3.3, §4.2 step 7 |
| Till and rider shortage write-off without approval (`FIN.ACL.06`) | AED 20 per handover; above: area manager | §7.2 settings |
| Shortage recovery by salary deduction (`FIN.ACL.19`) | off `⚠ open: lawyer (L-20)` | §4.2 |
| Delivery failure reasons (wl-delivery) | adds *rider issue*; *refused* asks the cause (*late*, *wrong item*, *other*); mapping in §7.1 | §7.2 settings |
| Online payment window before `payment_failed` | 15 minutes (wl-sales default; `FIN.RUL.19` asks the gateway at 10) | §7.2 settings |
| Till variance needing approval (wl-sales) | AED 20, the same as the write-off limit | §7.2 settings |
| Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 50,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the owner / operations director and the accountant, checked daily at 06:00; aggregator payouts by their payout schedule (a `CashForecastSource` of 06); due dates adjusted by average days late: off (customers pay at order); disputed amounts counted: off | §5; §7.2 settings |
| Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories (food cost, packaging, staff, rent, aggregator commission, marketing) owned by the accountant and mapped to the export accounts; approved by the owner / operations director; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.2 settings |
| Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of food cost, operating cost and profit; every drawing approved, whatever its amount (`FIN.ACL.21` limit AED 0), by the owner / operations director, never by the requester (`FIN.ACL.21`) | §5; §7.2 settings |

#### Edge cases decided
- **Online payment window before `payment_failed`:** the wl-sales default 15 minutes stays; `FIN.RUL.19` asks the gateway at 10 minutes, so most orders are decided
  before the window ends.
- **Till variance needing approval in wl-sales:** AED 20, the same as the write-off limit.
- **Rider handover period:** per shift; a rider with two shifts in a day has two handovers.
- **Refused item at the door:** the delivery is `delivered` with the refused line and the order
  `completed` for the accepted lines. Prepaid: the system raises a *no goods* return for the
  refused line (category *wrong item*, party *business*), resolved at once with *refund* up to
  AED 50, otherwise waiting for the kitchen manager (`SAL.ACL.06`). Cash on delivery: expected
  cash is the accepted lines' amount (wl-delivery per-line amounts), no refund. At run close the
  refused food ticked received back is posted as wastage (reason *refused at delivery*,
  `FNB.ACL.16`).

#### Must not
- Must not enable a reattempt for F&B deliveries.
- Must not let a rider count their own cash.
- Must not enable salary deduction while L-20 is open.

#### Done when
- Delivery
  - [ ] A failed F&B delivery is never queued for reattempt; with the rider able to return it ends
        `returned`; after a run abort without hand-over it ends `discarded` with a photo.
  - [ ] A COD order of AED 80.00 paid AED 76.00 needs no approval; paid AED 74.00 needs the
        kitchen manager.
  - [ ] Given prepaid ORD-1201 with Burger (AED 40.00) and Fries (AED 12.00), when the customer
        refuses Burger as *wrong item* at proof of delivery, the delivery is `delivered` with
        Burger refused, ORD-1201 is `completed`, a *no goods* return of AED 40.00 (*wrong item*,
        *business*) is resolved with a AED 40.00 refund, and at run close the Burger is posted as
        wastage. On the same order paid by cash on delivery, the rider's expected cash is
        AED 12.00 and nothing is refunded.
  - [ ] The rider's failure reasons include *rider issue*; choosing *refused* asks for the cause.
- Sales
  - [ ] An aggregator order offers no `amend`; a second order with the same Talabat number is refused.
  - [ ] A *no goods* refund of AED 51.00 needs the kitchen manager; the recorder cannot approve.
  - [ ] Cancel after start offers only *return to stock* and *scrap*.
- Payments
  - [ ] A D2C payment pending 10 minutes is checked with the gateway; one still pending at
        15 minutes is `payment_failed`.
  - [ ] A chargeback cannot be accepted by the person who opened it.
- Cash
  - [ ] Rider R's shift handover cannot be counted by R; a AED 15.00 shortage is written off
        without approval; AED 25.00 needs the area manager; salary recovery is not offered.
  - [ ] Closing a till session with a shortage creates the till's handover; a variance of
        AED 20.01 needs approval, AED 20.00 does not.
- Cash-flow forecast and budget
  - [ ] in a fresh 06 deployment, a projected balance of AED 49,999 in week 5 alerts the owner /
        operations director and the accountant once (`FIN.RUL.23`, 06:00 run); AED 50,000 or
        more in every week alerts no one; the forecast shows 13 weekly buckets from Monday.
  - [ ] a monthly budget for a legal entity prepared by the accountant needs approval by the owner
        / operations director; an expense with category *owner drawings* is left out of operating
        cost and profit reports, and the user who records it cannot approve it (`FIN.ACL.21`);
        an expense line reaching 100% of its month's budget line warns the category's owner role
        and the accountant once (`FIN.RUL.24`).

#### Out of scope
Module mechanisms (their epics); stock, compliance and connections (BL-FNB-17).

---

### BL-FNB-17 — 06 F&B: configure shared negative-flow settings (stock, compliance, connections)

**Labels:** `vertical:06-fnb` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-STK-01 · BL-STK-03 · BL-STK-07 · BL-STK-11 to BL-STK-19 · BL-DOC-02 · BL-KRN-03 to BL-KRN-08 · BL-SAL-01 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-SAL-22 · BL-SAL-23 · BL-SAL-24 · BL-SAL-25 · BL-SAL-26 · BL-NTF-07 · BL-NTF-08 · BL-NTF-09 · BL-STK-28 · BL-STK-29 · BL-STK-30 · BL-STK-31 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None.
**Review items:** X-08, X-09, X-10, X-13, X-19, 06-N10-b, 06 MEDIUM "duplicate external orders"

#### Goal
Seed 06's values for the shared stock, document, kernel and connection mechanisms: negative stock
per location, receiving rejection evidence, hold deciders and reminders, recalls, the
compliance-critical food safety rules and documents, void, late entry, device connections and
the integration fallbacks (Deliverect, gateway, SMS).
It also seeds the 06 values for lead sources and campaigns (`SAL.CRM.02`), the supplier
scorecard (`STK.SUP.01`), loyalty (`SAL.LOY.01`), onboarding and SOPs (`HRM.ONB.01`) and
registration kinds (`DOC.EXP.01`), added on 2026-10-16: `SAL.ACL.07`, `STK.RUL.09`,
`STK.ACL.07`, `SAL.RUL.07`, `SAL.RUL.08`, `NTF.RUL.02`, `NTF.ACL.03`, `HRM.RUL.16`, `HRM.RUL.17`,
`HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`.

#### Files to read
- `docs/verticals/06-fnb.md` §4.1 "When it goes wrong" (row "Deliverect is down"), §4.4 "When it
  goes wrong", §4.6 "When it goes wrong" (health card row), §7.2 (compliance-critical list,
  settings table)
- `docs/modules/wl-inventory.md` §7; `docs/modules/wl-documents.md` §7; `docs/modules/wl-kernel.md` §7
- `docs/modules/wl-sales.md` §4.6, §4.7, §5.8, §5.10 and §7 (campaigns, sources, loyalty);
  `docs/modules/wl-notify.md` §4.3, §5.3 and §7 (marketing broadcasts); `docs/modules/wl-inventory.md`
  §5.9 (supplier scorecard); `docs/modules/wl-hr.md` §5.8 and §7 (onboarding);
  `docs/modules/wl-documents.md` §3, §5.7 (SOPs, registration kinds)
- `docs/OPEN-ITEMS.md` AU-15, AU-20

#### Terms
- **Lot-tracked item:** an item whose lots and expiry are recorded (received goods and batches).

#### Spec excerpt (authoritative for this issue)

| Setting | 06 value | Spec reference |
|---|---|---|
| Negative stock policy | outlet kitchens: `allow_consumption` for items that are not lot-tracked; central kitchen and lot-tracked items: `block` | §7.2 settings |
| Pending issue and negative balance alert (`STK.RUL.06`) | storekeeper | §4.4 |
| Goods receipt rejection evidence | `temperature` needs the reading from the receiving checklist | §7.2 settings |
| Returns to supplier not credited (`STK.RUL.08`) | chased after 30 days | §4.4 |
| Stock hold deciders (`lots.release`) | the head chef of the outlet (role *Head chef / central kitchen chef*, scoped to that outlet; the central kitchen chef for the central kitchen); the outlet's kitchen manager as deputy | §7.2 settings |
| Hold reminder (`STK.RUL.07`) | H = 2 hours; escalation to the kitchen manager after 4 hours | §7.2 |
| Hold reasons needing a second person (`STK.ACL.05`) | `recall` (temperature released by one) | §3.3 |
| Wastage approval (`STK.ACL.02`) | above AED 200, by the kitchen manager, not the recorder | §3.3 |
| Recall owner | owner / operations director with the head chef | §7.2 settings |
| Compliance-critical rules (owner alert, `PLT.RUL.04`) | `FNB.RUL.03`, `FNB.RUL.04`, `FNB.RUL.05`, `DOC.RUL.03` for the trade licence, food permits, health cards and PIC certificate | §7.2 |
| Owner alert recipients | owner / operations director | §7.2 |
| Health card and PIC certificate expiry | reminders (`DOC.RUL.01`); owner alerted on expiry; no transition blocked in the MVP (flag on attendance Later) | §4.6 |
| Records that support `void` | payout statement; orders are cancelled, receipts reversed by a sales return | §7.2 settings |
| Late entry after an outage (business time) | order time, counter sale time, wastage, temperature readings, delivery proof | §7.2 settings |
| Late entry approval (`PLT.ACL.10`) | area manager, after 24 hours or into a submitted daily closing | §4.6 |
| Device connections | kitchen devices per outlet, active in opening hours, down after 5 minutes | wl-kernel §7 |
| Integration fallbacks (wl-kernel §5.14) | Deliverect down → aggregator tablets and quick entry; gateway down → D2C online payment paused, cash on delivery and counter card terminal stay; SMS down → OTP login unavailable, orders by phone | §7.2 settings |
| Deliverect recovery | imported orders that match a quick-entered number are refused as duplicates and listed for the kitchen manager | §4.1 |
| Business-day cut-off per outlet | 04:00 (BL-FNB-15) | §7.2 settings |
| Responsible role per document kind (wl-documents §5.2) | trade licence: owner / operations director; food permits, health cards and PIC certificate: kitchen manager of the outlet | §7.2 settings |
| Notification types and groups (wl-notify §5.4) | `fnb.order_status`: *Marketing, delivery tracking* (`none`); `fnb.login_otp`: *OTP*; `fnb.recall_notice`: no group, urgent (task to the recall owner) | §7.2 |
| Lead sources used (`SAL.CRM.02`) | walk-in, website, social, paid ads, aggregator, referral, event, partner (corporate catering), other; corporate and B2B enquiries are leads (`SAL.CRM.01`); a direct customer takes the source of their first order; no source requires a campaign | §5; §7.2 settings |
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | Instagram, TikTok, search ads, aggregator promotions, SMS, email; a promo code (`SAL.DSC.01`) may belong to a campaign; converted value: the customer's first direct order; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.2 settings |
| Supplier scorecard review (`STK.SUP.01`, `STK.RUL.09`, `STK.ACL.07`) | score on a 0–100 scale from the module weights; a monthly snapshot below 70 over a 1-month window (module: 3 months) makes `STK.RUL.09` open a review task to the storekeeper / purchaser, the area manager told; preferred and alternative suppliers per ingredient used by par-level reorder suggestions (capability `STK.ROP.01`, reorder points and par levels; built by `STK.RUL.04` with BL-STK-28); making another supplier preferred needs a second person (`STK.ACL.07`: on) | §5; §7.2 settings |
| Loyalty earn and redeem (`SAL.LOY.01`, `SAL.RUL.07`) | `SAL.RUL.07` earns 1 point per AED 1 on direct orders (D2C site and counter); none on aggregator orders; 100 points = an AED 5 discount line, at most 50% of the order, confirmed by a one-time code to the member (VAT on the discounted supply: `⚠ open: accountant (A-15)`) | §5; §7.2 settings |
| Loyalty expiry, segments and campaigns (`SAL.LOY.01`, `SAL.RUL.08`, `SAL.ACL.07`, `NTF.RUL.02`, `NTF.ACL.03`) | points expire 12 months after they are earned, the member reminded 30 days before (`SAL.RUL.08`); a manual adjustment above 500 points needs a manager (`SAL.ACL.07`); segments: first order only, regulars (3 or more orders in 30 days), lapsed (no order in 45 days), corporate catering customers; campaigns only to opted-in customers (`NTF.ACL.01`), with opt-out, sent 07:00–21:00 and at most 2 marketing messages per customer in 7 days (`NTF.RUL.02`); a broadcast to more than 200 recipients needs a second person (`NTF.ACL.03`) | §5; §7.2 settings |
| Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for line cook, cashier / order taker, rider, storekeeper / purchaser and outlet / kitchen manager, started by `HRM.RUL.16` when the employee is created in `onboarding` (none for a record created directly `active`, e.g. by a data import); checklist steps use `WRK.CHK.01`; tasks to the outlet / kitchen manager, the head chef and the employee; the employee's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds the assignees of an overdue step on the first run after its due date, then every 2 days, and tells HR and the line manager after 5 days; completion blocks nothing (health cards stay tracked by `DOC.EXP.01`) | §5; §7.2 settings |
| SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: head chef / central kitchen chef (recipes, food safety) and area manager (service, cash and delivery); required readers by role per outlet; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP's owner role (the SOP owner named above) after 7 days overdue; acknowledged only by the reader (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.2 settings |
| Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Halal certificate* (subject: item for central kitchen products, supplier party for supplier ingredients; responsible role storekeeper / purchaser, replacing the seeded *quality role, else owner* because 06 has no quality role; an expired supplier certificate is a warning on its reorder suggestions); *Trademark registration* (subject: the legal entity; the brand is the registered mark, field *mark*) and *Other certification* (legal entity; owner / operations director); `DOC.RUL.01` at 180, 90 and 30 days for trademarks and 90, 30 and 7 days for the others; no `blocks` | §5; §7.2 settings |

#### Edge cases decided
- **Loyalty VAT:** a redeemed points discount reduces the VAT of the discounted supply only
  (`⚠ open: accountant (A-15)`); the rest of the loyalty settings can be built.
- **Health card kinds:** no `blocks` entries in the MVP (no shift attendance); the kinds are
  compliance-critical for the owner alert only. Their validity is the expiry printed on the card
  (`⚠ open: authority (AU-15)`).
- **Duplicate list on Deliverect recovery:** one list per outlet per recovery, as a task for the
  kitchen manager with each refused import and the existing order link.
- **Gateway down:** the D2C checkout hides online payment and shows COD where the customer is
  allowed it (BL-FNB-03); pickup orders pay at the counter.
- **SMS down:** the D2C login screen shows "Login unavailable, please call <outlet phone>".
- **Hold deciders:** `lots.release` is granted to the *Head chef / central kitchen chef* role at
  its outlet (an outlet's head chef is "the chef" of BL-FNB-12) and to the kitchen manager as
  deputy; a chef of another outlet cannot decide.
- **Give-up reactions:** the group defaults of the three 06 types (§7.2); 06 changes no
  reaction.

#### Must not
- Must not set `allow_consumption` for lot-tracked items or the central kitchen.
- Must not offer `void` on orders.
- Must not block any transition on an expired health card in the MVP.

#### Done when
- Stock
  - [ ] Outlet 1 sells 2 kg of onions (not lot-tracked) with 1 kg on hand: balance −1 kg and the
        storekeeper is alerted; the same for lot-tracked cream creates a pending issue.
  - [ ] Rejecting a receipt line with reason `temperature` without a reading is refused.
  - [ ] Releasing a recall-held line needs a second `lots.release` holder; a temperature line is
        released by the chef alone.
  - [ ] Outlet 1's head chef can decide a hold line at Outlet 1 and not at Outlet 2; Outlet 1's
        kitchen manager can decide it as deputy.
- Compliance
  - [ ] A missed `FNB.RUL.04` run alerts the owner / operations director.
  - [ ] A health card expiring 2026-10-31 alerts the owner on 2026-11-01 and blocks nothing; its
        reminders go to the kitchen manager of the employee's outlet.
  - [ ] The trade licence kind has responsible role owner / operations director.
- Notifications
  - [ ] The three 06 notification types are registered with their groups; an order status message
        that gives up creates no task; a recall notice that gives up gives the recall owner a task.
- Kernel and connections
  - [ ] `void` is offered on a payout statement, not on an order.
  - [ ] A late wastage entry 30 hours after its business time needs the area manager.
  - [ ] With Deliverect `down`, quick entry is offered; on recovery an import with a number
        already quick-entered is refused and listed for the kitchen manager.
  - [ ] With the gateway `down`, the D2C checkout offers COD only and online payment is hidden.
  - [ ] With SMS `down`, the login screen shows the outlet phone.
- Sales, finance, staff and registrations
  - [ ] a new lead or party offers the sources walk-in, website, social, paid ads, aggregator,
        referral, event, partner and other; a campaign with channel *Instagram*, a budget and cost
        entries reports leads, conversion, cost per lead, CAC and ROAS; a cost entry over the
        budget waits for a manager (`SAL.ACL.07`).
  - [ ] a supplier scoring 69 of 100 for a month creates a review task for the storekeeper /
        purchaser (`STK.RUL.09`); 70 creates none; an item's alternative supplier is offered on
        the par-level reorder suggestion; making it the preferred supplier waits for a second
        person (`STK.ACL.07`).
  - [ ] a direct order of AED 200 earns 200 points and an aggregator order earns none; redeeming
        100 points adds an AED 5 discount line, refused above 50% of the order; points earned on
        2026-11-01 expire on 2027-11-01 (`SAL.RUL.08`, reminder on 2027-10-02); a manual
        adjustment of 501 points waits for a manager (`SAL.ACL.07`); a customer who has not opted
        in is left out of a campaign; a campaign message queued at 22:00 is deferred to 07:00 and
        a third marketing message to one customer in 7 days is suppressed (`NTF.RUL.02`); a
        broadcast to 201 customers waits for a second person (`NTF.ACL.03`).
  - [ ] hiring a line cook creates the line cook checklist with tasks to the outlet / kitchen
        manager, the head chef and the employee, and the role is active before it is complete; a
        step still open on the first run after its due date reminds its assignees, and again 2 days
        later (`HRM.RUL.17`); a new version of an SOP owned by the head chef / central kitchen chef
        asks every required reader to acknowledge again within 7 days (`DOC.RUL.04`), and no one
        else can acknowledge for them (`HRM.ACL.10`).
  - [ ] a fresh 06 deployment lists *Halal certificate*, *Trademark registration* and *Other
        certification* with responsible role storekeeper / purchaser for Halal, owner / operations
        director for the others; a trademark registration expiring in 90 days notifies the owner /
        operations director.

#### Out of scope
Module mechanisms (their epics); delivery, sales, payments and cash (BL-FNB-16).
