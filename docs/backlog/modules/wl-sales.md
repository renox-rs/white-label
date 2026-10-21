# Epic: wl-sales — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows of the Sales order, Sales return and Package machines in
[wl-sales](../../modules/wl-sales.md): payment-first online orders and late payments, duplicate
external orders, order amendment, cancellation of an order or a line after fulfilment started
with an outcome per line, the credit re-check at dispatch and the bounced-cheque hold, partial
fulfilment from refused deliveries, refunds after completion with nothing returned, returns of
recalled goods, a package whose payment is reversed, back-orders for lines without stock and
duplicate customer PO numbers. It closes X-14 and the sales parts of X-03, X-05 and X-09,
and the HIGH items 06-N3-a, 06-N4-b, 07-N2-a, 07-N3-b and 08-N3-a.

BL-SAL-19 to BL-SAL-26 (label `area:sme-gap`, added 2026-10-16) build two capabilities from
the [SME problem map](../../reviews/2026-10-16-sme-problem-map.md) §3: lead sources and
campaigns with acquisition reports (`SAL.CRM.02`, gap G1) and loyalty with segments and
retention campaigns (`SAL.LOY.01`, gap G4). They are not negative flows; they follow the same
template. BL-SAL-26 and the negative paths in BL-SAL-19 to BL-SAL-25 come from the gap review
(GR-G1, GR-G4).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-SAL-01 | Refuse a duplicate external order number per channel | high | s | Launch 06 | Base: wl-sales Sales order lifecycle |
| BL-SAL-02 | Hold an online-paid order in `awaiting_payment` until the gateway confirms | high | m | Launch 06 | Base: wl-sales Sales order lifecycle; Epic wl-payments: `PaymentReceived` |
| BL-SAL-03 | Cancel an order whose online payment failed or timed out | high | s | Launch 06 | BL-SAL-02; Epic wl-payments: `PaymentFailed` |
| BL-SAL-04 | Refund a payment confirmed after cancel or reject (`SAL.RUL.06`) | high | s | Launch 06 | BL-SAL-02; BL-SAL-03; Epic wl-payments: refunds |
| BL-SAL-05 | Amend an order as a new revision | high | m | Launch 06 | Base: wl-sales Sales order lifecycle; Epic wl-kernel: approval returned |
| BL-SAL-06 | Settle the money of an amendment (prepaid difference, invoice correction) | high | m | Launch 06 | BL-SAL-05; Epic wl-payments: payment links and refunds; Epic wl-invoicing: credit note and extra invoice |
| BL-SAL-07 | Cancel an order after fulfilment started, with an outcome per line | high | m | Launch 06 | Base: wl-sales Sales order lifecycle; Epic wl-kernel: approval returned |
| BL-SAL-08 | Cancel one line of an order (`cancel_line`) | high | m | Launch 06 | BL-SAL-07 |
| BL-SAL-09 | Cancel the remaining lines with outcomes (`cancel_remaining`) | medium | s | Launch 06 | BL-SAL-07 |
| BL-SAL-10 | Re-check credit at dispatch and hold the order | high | m | Launch 07 | Base: wl-sales Sales order lifecycle and credit check |
| BL-SAL-11 | Release a held order back to the state before the hold | high | s | Launch 07 | BL-SAL-10; Epic wl-kernel: approval returned |
| BL-SAL-12 | Put the customer account and its orders on hold after a bounced cheque (`SAL.RUL.05`) | high | m | Launch 07 | BL-SAL-10; BL-SAL-11; Epic wl-payments: `ChequeBounced` |
| BL-SAL-13 | Fulfil only accepted quantities, handle a returned delivery and reverse fulfilment | high | m | Launch 07 | Base: wl-sales Sales order lifecycle; BL-SAL-08; BL-DLV-02; BL-STK-22; BL-STK-24; BL-STK-25 |
| BL-SAL-14 | Refund after completion with nothing returned (`SAL.ACL.06`) | high | m | Launch 06 | Base: wl-sales Sales return lifecycle; Epic wl-ticketing: complaint tickets |
| BL-SAL-15 | Accept returns of recalled goods into the recall's hold | high | s | Launch 08 | Base: wl-sales Sales return lifecycle; BL-STK-14; BL-STK-17 |
| BL-SAL-16 | Keep a package active when its payment is reversed | high | m | Launch 05 | Base: wl-sales Package lifecycle; Epic wl-invoicing: `InvoiceReopened` |
| BL-SAL-17 | Back-order lines without stock and serve them oldest first | medium | m | Launch 08 | Base: wl-sales Sales order lifecycle; BL-SAL-08; BL-SAL-09; BL-SAL-11; Base: wl-inventory `STK.RUL.04` and goods receipt |
| BL-SAL-18 | Refuse a second order with the same customer PO number for a customer | medium | s | Launch 07 | Base: wl-sales Sales order lifecycle; BL-SAL-05 |
| BL-SAL-19 | Run campaigns with a budget and cost entries | medium | m | Build 1 | Base: wl-kernel state machines and approvals; Base: wl-invoicing supplier bills |
| BL-SAL-20 | Record the source and campaign of leads and parties and link the first sale | medium | m | Build 1 | BL-SAL-19; Base: wl-sales Lead lifecycle; Base: wl-invoicing `InvoiceIssued`; Base: wl-kernel `void` |
| BL-SAL-21 | Report leads, conversion, cost per lead, CAC and ROAS | medium | m | Build 1 | BL-SAL-19; BL-SAL-20 |
| BL-SAL-22 | Enrol members and earn points or visits on qualifying sales (`SAL.RUL.07`) | medium | m | Launch 05 | Base: wl-sales Counter sale, Sales order and Sales return lifecycles; Base: wl-invoicing `InvoicePaid`, `CreditNoteIssued` |
| BL-SAL-23 | Redeem points as a discount line, confirmed by the member | medium | m | Launch 05 | BL-SAL-22; Base: `DiscountEngine`; Base: wl-notify `send_otp` |
| BL-SAL-24 | Expire points, adjust by hand with approval and report the points liability (`SAL.RUL.08`, `SAL.ACL.07`) | medium | m | Launch 05 | BL-SAL-22; BL-SAL-23; Base: wl-kernel approvals; Base: wl-notify `notify` |
| BL-SAL-25 | Define customer segments and send retention campaigns through wl-notify | medium | m | Launch 05 | BL-SAL-19; BL-SAL-21; BL-SAL-22; BL-NTF-07; BL-NTF-08; BL-NTF-09 |
| BL-SAL-26 | Keep loyalty points through account closure, party merge and block, and programme changes | medium | m | Launch 05 | BL-SAL-22; BL-SAL-24; Base: wl-parties `PartiesMerged`, `PartyBlocked`, `BlockEffects` |
| BL-SAL-27 | Count a vertical's own enquiries as campaign leads through a kernel lead-capture contract | medium | s | Build 1 | BL-SAL-20; BL-SAL-21 |
| BL-SAL-28 | Keep marketing channels as a configurable list seeded per vertical | medium | s | Build 1 | BL-SAL-19 |

---

### BL-SAL-01 — wl-sales: refuse a duplicate external order number per channel

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` `surface:system` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** Base: wl-sales Sales order lifecycle · **Blocks:** BL-FNB-16, BL-FNB-17
**Review items:** X-14 (06 MEDIUM: duplicate external orders)

#### Goal
An aggregator or website may send the same order twice. The order's external order number
must be unique per channel, so a second import is refused and points to the existing order
instead of cooking and charging twice.

#### Surfaces
- **System:** an import whose external order number (trimmed, case-insensitive) is already
  used on the channel is refused by a unique key, and the response returns the existing
  order's number and id; no second order, reservation or kitchen ticket is created.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Sales order* (external order number), §4.2 row `submit`, §7 setting "Unique external order number per channel"

#### Terms
- **External order number:** the order number in the channel's own system (e.g. an aggregator order number).
- **Channel:** where orders come from (counter, phone, portal, website, aggregator, sales rep).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | awaiting_payment, pending_acceptance, credit_hold, confirmed | `sales_orders.create`, or the external party through the portal | lines priced; manual prices and discounts within limits (`SAL.ACL.02`) or approved; the external order number (compared trimmed and case-insensitive) is not already used on this channel (a duplicate import is refused and points to the existing order); payment on account: the customer account is not on hold (`SAL.RUL.05`; refused, the order stays `draft`) | target: `awaiting_payment` if paid online and not yet confirmed; else `pending_acceptance` if the channel needs acceptance; otherwise credit check → `credit_hold` or `confirmed`; on `confirmed`: stock reserved, confirmation sent, event `SalesOrderConfirmed` |

Setting: Unique external order number per channel: on.

#### Edge cases decided
- **Comparison:** trimmed, case-insensitive, per channel. The same number on two different channels is allowed.
- **Earlier order cancelled or rejected:** still counts as used; a re-import is refused (the channel must send a new number).
- **Empty external number** (counter, phone): not checked.
- **Concurrent imports:** enforced by a unique key on (channel, normalised number) so two simultaneous imports cannot both pass; the loser gets the duplicate refusal.
- **Setting off:** no check.
- **Response to an integration:** the refusal returns the existing order's number and id, so the integration can link to it.

#### Must not
- Must not create a second order, reservation or kitchen ticket for a duplicate.
- Must not overwrite the existing order with the duplicate's data.

#### Done when
- [ ] Given order SO-2026-0101 on channel *Talabat* with external number "TB-555", when an import with "tb-555 " arrives on *Talabat*, then it is refused and the response names SO-2026-0101.
- [ ] Given "TB-555" on channel *Deliveroo*, the import is accepted.
- [ ] Given two imports of "TB-777" at the same moment, exactly one order exists.
- [ ] Given SO with "TB-555" `cancelled`, a new import "TB-555" is refused.

#### Out of scope
Aggregator integration and payout lines (Epic 06 F&B).

---

### BL-SAL-02 — wl-sales: hold an online-paid order in `awaiting_payment` until the gateway confirms

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · Epic wl-payments: `PaymentReceived` · **Blocks:** BL-SAL-03, BL-SAL-04
**Review items:** X-14, 06-N3-a

#### Goal
An order paid online must not reach the kitchen or warehouse before the gateway confirms the
payment. The order waits in `awaiting_payment`, invisible to fulfilment. When the payment is
confirmed for the full amount, the order continues to acceptance or confirmation; prepaid
orders skip the credit check.

#### Surfaces
- **Back office:** an `awaiting_payment` order does not appear in the outlet, kitchen,
  dispatch or acceptance lists; it shows up there only after the payment is confirmed.
- **System:** `PaymentReceived` for the full order total runs `confirm_payment` (idempotent
  per payment id) and moves the order to `pending_acceptance` or `confirmed` without a
  credit check.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Sales order* (payment, payment status), §4.2 (states, rows `submit`, `confirm_payment`, paragraph "Online payment"), §5.3 steps 1–3, §6 consumed events

#### Terms
- **Payment status:** not required, pending, received, failed.
- **Gateway:** Stripe or N-Genius through wl-payments (`FIN.PAY.02`).
- **`PaymentReceived`:** wl-payments event when a payment is confirmed; carries the order as subject and the amount.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `awaiting_payment` | neutral | no | Placed with online prepayment; waiting for the gateway; not visible to fulfilment |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | awaiting_payment, pending_acceptance, credit_hold, confirmed | `sales_orders.create`, or the external party through the portal | (as BL-SAL-01) | target: `awaiting_payment` if paid online and not yet confirmed; else `pending_acceptance` if the channel needs acceptance; otherwise credit check → `credit_hold` or `confirmed`; on `confirmed`: stock reserved, confirmation sent, event `SalesOrderConfirmed` |
| `confirm_payment` | awaiting_payment | pending_acceptance, confirmed | system (`PaymentReceived` from `wl-payments` for the order's payment) | amount equals the order total | as `submit` from acceptance on; prepaid orders skip the credit check |

"Online payment": An order paid online is placed (visible to the outlet or fulfilment) only
after the gateway confirms the payment.

#### Edge cases decided
- **Amount differs from the total** (partial or over-payment): `confirm_payment` does not run; the order stays `awaiting_payment` and times out (BL-SAL-03); `SAL.RUL.06` refunds that payment in full as soon as it arrives (BL-SAL-04). Rounding: compare in AED fils, exact.
- **Paid already confirmed at submit** (gateway answered synchronously): `submit` skips `awaiting_payment` ("not yet confirmed" is false).
- **Visibility:** `awaiting_payment` orders are excluded from outlet, kitchen, dispatch and acceptance lists; no stock is reserved and no kitchen ticket exists.
- **Promo code:** usage is recorded at `submit` (released on failure, BL-SAL-03).
- **Duplicate `PaymentReceived`** for the same payment: idempotent by payment id; the second is ignored.
- **Order already moved** (e.g. cancelled by timeout) when the event arrives: `confirm_payment` is not available; `SAL.RUL.06` handles it.
- **Credit check:** never run for a prepaid order, even on a channel with credit check on.

#### Must not
- Must not show an `awaiting_payment` order to the outlet or fulfilment.
- Must not reserve stock before `confirmed`.
- Must not set the status directly; only through `confirm_payment`.

#### Done when
- [ ] Given a website order AED 85 paid by card, when submitted, it is `awaiting_payment` and absent from the outlet's list.
- [ ] When `PaymentReceived` AED 85 arrives, it becomes `pending_acceptance` (website needs acceptance).
- [ ] Given a channel without acceptance, it becomes `confirmed`, stock is reserved and `SalesOrderConfirmed` is emitted, without a credit check.
- [ ] Given `PaymentReceived` AED 80 for an AED 85 order, the order stays `awaiting_payment`.
- [ ] A second `PaymentReceived` with the same payment id changes nothing.

#### Out of scope
Failure and timeout (BL-SAL-03); late payment refund (BL-SAL-04); gateway integration (Epic wl-payments).

---

### BL-SAL-03 — wl-sales: cancel an order whose online payment failed or timed out

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` `surface:public` `surface:system` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-SAL-02 · Epic wl-payments: `PaymentFailed` · **Blocks:** BL-SAL-04
**Review items:** X-14, 06-N3-a

#### Goal
If the online payment fails, or no confirmation arrives within the payment window (15
minutes), the order is cancelled with reason *payment failed*. The promo code use is released
and the customer is told they can place a new order. Nothing reaches fulfilment.

#### Surfaces
- **Public:** on the website order the customer is told "payment failed, please try again"
  and can place a new order (the failed order cannot be retried).
- **System:** `PaymentFailed`, or a job 15 minutes after `submit`, runs `payment_failed`
  (whichever comes first), releases the promo code use and emits `SalesOrderCancelled`.

#### Files to read
- `docs/modules/wl-sales.md` §4.2 (state `cancelled`, row `payment_failed`, paragraph "Online payment"), §5.3 step 2, §7 setting "Online payment window"

#### Terms
- **Payment window:** time allowed between submit and confirmation; default 15 min.
- **`PaymentFailed`:** wl-payments event for a declined or timed-out gateway payment.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `cancelled` | neutral | yes | Cancelled before anything was fulfilled, or the online payment failed |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `payment_failed` | awaiting_payment | cancelled | system (`PaymentFailed`, including the gateway timeout) | — | reason *payment failed*; promo code use released; the customer is told and may place a new order; nothing reaches fulfilment; event `SalesOrderCancelled` |

Setting: Online payment window before `payment_failed` (gateway timeout): 15 min.

#### Edge cases decided
- **Timer:** starts at `submit`; a scheduled job runs `payment_failed` when 15 min pass without `confirm_payment`, even if wl-payments sends no `PaymentFailed`. Whichever comes first wins; the other is ignored.
- **Customer retries on the same order:** not allowed; a retry is a new order (the cancelled order is terminal).
- **Promo code with a usage limit:** the released use is available again at once.
- **Payment confirmed after the cancel:** refunded by `SAL.RUL.06` (BL-SAL-04), the order stays `cancelled`.
- **Notification channel:** the customer's order channel (website message and SMS/e-mail through wl-notify).

#### Must not
- Must not leave an unpaid order in `awaiting_payment` past the window.
- Must not consume the promo code for a failed order.

#### Done when
- [ ] Given an order submitted 12:00:00 with no confirmation, at 12:15:00 it is `cancelled` with reason *payment failed* and `SalesOrderCancelled` is emitted.
- [ ] Given `PaymentFailed` at 12:02, it is cancelled at 12:02; the timer later does nothing.
- [ ] Given promo code SUMMER (max 1 use per party) used on the failed order, the same party can use it on a new order.
- [ ] The customer receives a "payment failed, please try again" message.

#### Out of scope
Late payment refund (BL-SAL-04).

---

### BL-SAL-04 — wl-sales: refund a payment confirmed after cancel or reject (`SAL.RUL.06`)

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` `surface:system` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-SAL-02 · BL-SAL-03 · Epic wl-payments: refunds (system request of a late payment) · **Blocks:** None.
**Review items:** X-14, X-06, 06-N3-a

#### Goal
A payment can be confirmed after the order was cancelled, rejected or already paid. That
money must go back to the customer in full, without staff action, and the order must not
come back to life.

#### Surfaces
- **System:** `SAL.RUL.06` requests a full refund (reason *late payment*) of a payment
  confirmed for a `cancelled`, `rejected` or already paid order, or of an amount that does
  not match an `awaiting_payment` order, once per payment id; the order state never changes,
  and the customer and the outlet or sales manager are notified.

#### Files to read
- `docs/modules/wl-sales.md` §4.2 rows `confirm_payment`, `reject`, paragraph "Online payment", §6 `SAL.RUL.06`
- `docs/modules/wl-payments.md` §4 *Refund* row `request` (system request of a late payment)

#### Terms
- **Late or unusable payment:** a `PaymentReceived` for an order that is `cancelled`, `rejected` or already fully paid, or for an order in `awaiting_payment` whose amount differs from the order total.
- **`Refunds`:** wl-payments service; a system request for a late payment of the same amount to the same card goes straight to `approved`.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reject` | pending_acceptance | rejected | `sales_orders.accept` or system (acceptance timeout) | reason given | prepayment refunded; party notified; a payment confirmed later is refunded too (`SAL.RUL.06`) |

| ID | When | If | Then |
|---|---|---|---|
| `SAL.RUL.06` | `PaymentReceived` for an order | order is `cancelled` or `rejected`, or already fully paid, or `awaiting_payment` and the amount differs from the order total (partial or over-payment: it cannot confirm the order, which then times out) | request a full refund of that payment (`wl-payments` `Refunds`, reason *late payment*); notify the customer and the outlet or sales manager; the order stays as it is |

#### Edge cases decided
- **"Already fully paid":** the sum of received payments before this one ≥ order total; the new payment is refunded in full (no partial keep).
- **Payment that cannot confirm the order** (BL-SAL-02: amount ≠ total while `awaiting_payment`): refunded in full by this rule when it arrives, with reason *late payment*; the order stays `awaiting_payment` and later times out (BL-SAL-03). Nothing needs to run again after the timeout.
- **Idempotency:** one refund request per payment id.
- **Refund fails in the gateway:** handled by wl-payments (`failed` refund); this rule does not retry.
- **Order state:** never changed by this rule.

#### Must not
- Must not move a `cancelled` or `rejected` order back to any state.
- Must not ask a person to approve this refund (wl-payments routes it).

#### Done when
- [ ] Given an order `cancelled` by timeout, when `PaymentReceived` AED 85 arrives, then a refund request of AED 85 reason *late payment* exists, the customer and outlet manager are notified, and the order stays `cancelled`.
- [ ] Given an order `rejected`, the same.
- [ ] Given an order already paid AED 85, a second payment of AED 85 is refunded in full.
- [ ] Given an AED 85 order `awaiting_payment`, when `PaymentReceived` AED 80 arrives, a refund request of AED 80 exists at once and the order stays `awaiting_payment` (it times out at the window end).
- [ ] The same `PaymentReceived` delivered twice creates one refund request.

#### Out of scope
Refund processing (Epic wl-payments).

---

### BL-SAL-05 — wl-sales: amend an order as a new revision

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-SAL-06, BL-FNB-05, BL-MFG-03, BL-SAL-18
**Review items:** X-14 (07 MEDIUM: order amendment)

#### Goal
Customers change quantities, prices or lines after an order is placed. Staff amend the order:
a new revision is created, the previous one is kept read-only, reservations and vertical
fulfilment follow, and the credit check runs again when the value rises. After fulfilment
started, a manager must approve. Aggregator orders cannot be amended here.

#### Surfaces
- **Back office:** a user with `sales_orders.amend` amends the order with a reason; the new
  revision is shown and the earlier one stays read-only. After fulfilment started a manager
  approves under "Needs your action" (never the requester); `amend` is not offered on
  aggregator orders, and reducing a started line sends the user to `cancel_line`.
- **System:** reservations and vertical fulfilment are adjusted, the credit check re-runs
  when the value rises (fails → `credit_hold`), guards are re-checked at final approval, and
  `SalesOrderAmended` is emitted.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Sales order* (revision), §4.2 row `amend`, paragraph "Cancelling after fulfilment started" (definition of started), §5.7 steps 1–2, §6 (`SAL.ACL.02`, `SAL.ACL.05`, event `SalesOrderAmended`), §7 setting "Amendments allowed per channel", §10 "Audit"

#### Terms
- **Revision:** number starting at 1; each amendment adds 1 and keeps the earlier revision read-only.
- **Fulfilment of a line started:** its goods are picked, a work order or production batch for it has started (07), or the kitchen has started it (06).
- **`SAL.ACL.02`:** a manual discount or price below list beyond a limit (10%; 07, 08: 5%) needs a manager.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `amend` | pending_acceptance, credit_hold, confirmed, partially_fulfilled | same state, or credit_hold | `sales_orders.amend` | reason; no line below its fulfilled quantity; a line whose fulfilment started is not reduced by `amend` (it goes through `cancel_line`, which asks for an outcome); prices and discounts within `SAL.ACL.02`; channel allows amendments (aggregator orders are changed on the aggregator); approval when fulfilment started (`SAL.ACL.05`) | revision + 1 and the previous revision kept; reservations and vertical fulfilment adjusted; credit check re-run when the value rises (fails → `credit_hold`); prepaid: payment link for an increase, refund of a decrease (`wl-payments`); a draft invoice is updated, an issued one is corrected by credit note or an extra invoice (`wl-invoicing`); revised confirmation sent; event `SalesOrderAmended` |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `SAL.ACL.05` | Amending an order or cancelling a line after its fulfilment started needs a manager, as does choosing *finish to stock* or *scrap* or waiving the cancellation fee; the requester cannot approve | `STA`, `SOD` | any value |

Event `SalesOrderAmended`: order, revision, changed lines, value before and after, reason.
Setting: Amendments allowed per channel: on, except aggregator channels.

#### Edge cases decided
- **Credit check:** runs only for orders on account and only when the new total > old total. Exposure = the customer's exposure with this order counted at its new total instead of its old one; it is checked against the customer limit (and the group's combined limit). Fails → `credit_hold` with the state before the hold stored (as `hold`, BL-SAL-10).
- **Amend while in `credit_hold`:** allowed; stays `credit_hold` (release is still needed).
- **Approval needed:** the request stores the new revision as a proposal; the order keeps the current revision (still valid) until approval. Rejected or returned: the proposal is dropped, nothing changes.
- **Order moves while the approval waits** (e.g. a line is picked or fulfilled): the kernel re-checks guards at final approval; "no line below its fulfilled quantity" failing → request `failed`, nothing changes.
- **Reservation adjustment:** a quantity increase reserves more; if stock is short, the extra quantity is reserved as far as available and the rest stays open (normal back-order behaviour of the base). A decrease releases the difference.
- **Concurrency:** two users amending at once: the second save is refused (revision changed); they reload.
- **Removing a line entirely:** an amendment may set a line to its fulfilled quantity (0 if nothing fulfilled); this is an amendment, not `cancel_line`, so no outcome is asked. When fulfilment of that line started, `amend` refuses the reduction and the UI sends the user to `cancel_line`, because a started line needs an outcome.

#### Must not
- Must not edit the current revision in place; always a new revision.
- Must not allow amendment on aggregator channels.
- Must not let the requester approve (`SAL.ACL.05`).
- Must not edit an issued invoice (money effects in BL-SAL-06 go through credit note or extra invoice).

#### Done when
- [ ] Given a `confirmed` on-account order (rev 1, AED 4,000, nothing started) for a customer with limit AED 50,000 and exposure AED 40,000 (this order's 4,000 included), when user A changes line 1 qty 10 → 12 with a reason, then rev 2 exists, rev 1 is read-only, 2 more units are reserved, the credit check runs on exposure 40,800 (40,000 − 4,000 + 4,800) and passes, the order stays `confirmed`, and `SalesOrderAmended` shows 4,000 → 4,800.
- [ ] Given the same amendment with limit AED 40,500, the check fails (40,800 > 40,500): the order is `credit_hold` with state before the hold `confirmed`, and `SAL.RUL.01` notifies.
- [ ] Given a reduction to AED 3,600, no credit check runs.
- [ ] Given line 1 picked, the same change creates an approval request; user A cannot approve; user B approves → rev 2.
- [ ] Given a line with 5 fulfilled, setting it to 4 is refused.
- [ ] Given an aggregator order, `amend` is not offered.
- [ ] Given a discount of 12% at a client with limit 10%, `amend` needs a manager under `SAL.ACL.02`.

#### Out of scope
Payment link, refund and invoice correction (BL-SAL-06).

---

### BL-SAL-06 — wl-sales: settle the money of an amendment (prepaid difference, invoice correction)

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:public` `surface:system` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-SAL-05 · Epic wl-payments: payment links and refunds · Epic wl-invoicing: credit note and extra invoice · **Blocks:** BL-FNB-05
**Review items:** X-14

#### Goal
An amendment changes the money already taken or billed. For a prepaid order, an increase
sends the customer a payment link and a decrease is refunded. A draft invoice is updated; an
issued invoice is never edited: it is corrected by a credit note or an extra invoice.

#### Surfaces
- **Back office:** when the payment link for an increase expires unpaid, the order owner
  (sales rep, or outlet manager for website and portal orders) gets a "payment link expired"
  task with *resend link* or *amend back*.
- **Public:** the customer pays the difference of an increase on the payment link
  (`FIN.PAY.02`).
- **System:** a decrease is refunded through wl-payments (reason *order amended*), a draft
  invoice is updated, an issued one gets a credit note or an extra invoice, and the added
  quantity is not released to fulfilment until the link is paid.

#### Files to read
- `docs/modules/wl-sales.md` §4.2 row `amend` (effects), paragraph "Amendment of a prepaid order", §5.7 step 1, §10 "Amended and cancelled orders"

#### Terms
- **Prepaid order:** payment *prepaid online*; the amount received is on the order.
- **Extra invoice:** a new tax invoice for the added value, linked to the order.
- **Credit note:** a tax credit note against the issued invoice for the reduced value.

#### Spec excerpt (authoritative for this issue)

`amend` effects (money part): prepaid: payment link for an increase, refund of a decrease
(`wl-payments`); a draft invoice is updated, an issued one is corrected by credit note or an
extra invoice (`wl-invoicing`); revised confirmation sent.

§4.2 *Amendment of a prepaid order*: An increase sends the customer a payment link for the
difference; the added quantity is reserved but not fulfilled until the link is paid. When the
link expires unpaid (after the online payment window for website orders, 24 hours for other
channels), the order owner gets a task (the sales rep, or the outlet manager for website and
portal orders without a sales rep): resend the link, or amend the order back. The revision
stays until then.

§10: an amendment after an invoice was issued is corrected by a tax credit note or an
additional invoice, never by editing the invoice.

#### Edge cases decided
- **Payment link not paid:** the added quantity stays reserved but is not fulfilled (not released to the kitchen, picking or dispatch) until the link is paid. The link expires after the online payment window (15 min) for website orders and after 24 h on other channels. On expiry the order owner gets a task "payment link expired": the sales rep, or for website and portal orders without a sales rep the outlet manager. The task offers *resend link* (a new link, same amount) or *amend back* (a new revision with the increase removed, no money effect). The revision stays until one of them is done.
- **Decrease refund:** requested through wl-payments `Refunds` for the difference, reason *order amended*; it follows wl-payments approval rules (`FIN.ACL.04`).
- **Mixed change** (one line up, one down): net the difference; one link or one refund, never both.
- **Issued invoice and value up:** an extra invoice for the added lines; value down: a credit note for the removed value. Draft invoice: updated in place.
- **Rounding:** VAT recalculated per line by wl-invoicing; the difference is the new total minus the old total in fils.
- **Approval rejected:** no money effect runs.

#### Must not
- Must not edit an issued invoice.
- Must not refund a decrease before the amendment is approved (when approval is needed).

#### Done when
- [ ] Given a prepaid order AED 200 amended to AED 260, the customer gets a payment link for AED 60.
- [ ] Given a website order whose AED 60 link is not paid within 15 min, the outlet manager gets a "payment link expired" task, the added quantity is still not released to the kitchen, and the order stays at rev 2; choosing *amend back* creates rev 3 at AED 200 with no refund or link.
- [ ] Given it amended to AED 150, a refund request of AED 50 reason *order amended* exists.
- [ ] Given an issued invoice of AED 4,000 and the order amended to AED 3,600, a credit note of AED 400 (incl. VAT) is requested; the invoice is unchanged.
- [ ] Given a draft invoice, it shows the rev 2 lines.

#### Out of scope
The revision itself (BL-SAL-05).

---

### BL-SAL-07 — wl-sales: cancel an order after fulfilment started, with an outcome per line

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-SAL-08, BL-SAL-09, BL-STK-26, BL-AUT-18, BL-FNB-02, BL-FNB-04, BL-MFG-03, BL-MFG-14
**Review items:** X-14, 07-N2-a

#### Goal
When the customer or business cancels an order whose kitchen, work order or picking has
started, each started line needs an outcome (return to stock, finish to stock, scrap), with or
without a cancellation fee, and a manager's approval. The event carries the outcomes so stock
and the vertical act on them; any prepayment is refunded less the fee.

#### Surfaces
- **Back office:** the user cancelling (for example cashier A) picks an outcome and fee for
  each started line; a manager approves the whole cancellation under "Needs your action" and
  may waive the fee; the requester cannot approve, and `cancel` is not offered once anything
  is fulfilled.
- **System:** guards are re-checked at final approval, one refund less fees is requested,
  and `SalesOrderCancelled` carries the outcome and fee per line.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Sales order* (line cancellation fields), §4.2 row `cancel`, paragraph "Cancelling after fulfilment started", §5.7 step 2, §6 (`SAL.ACL.01`, `SAL.ACL.05`, event `SalesOrderCancelled`), §7 settings, §10 "Amended and cancelled orders"
- `docs/OPEN-ITEMS.md` A-04

#### Terms
- **Fulfilment of a line started:** goods picked, a work order or production batch for it started (07), or the kitchen started it (06).
- **Outcome:** *return to stock*, *finish to stock*, *scrap*, each with or without a *cancellation fee*.
- **Prepaid share:** the part of the payment received for that line.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | draft, pending_acceptance, credit_hold, confirmed | cancelled | `sales_orders.cancel` | nothing fulfilled; paid order above a value, or after vertical fulfilment started, needs approval (`SAL.ACL.01`); outcome per line when fulfilment started | reservations released; outcomes applied; refund if paid, less any cancellation fee; vertical fulfilment cancelled (event `SalesOrderCancelled` with the outcome per line) |

| Outcome | Stock | Money |
|---|---|---|
| *Return to stock* | picked goods put back and the reservation released (`wl-inventory`) | prepaid share refunded |
| *Finish to stock* | the work order runs to the end and its output goes to free stock (vertical) | prepaid share refunded, less any fee |
| *Scrap* | work in progress or prepared goods written off as scrap or wastage (`wl-inventory`, `STK.ACL.02`) | cost recorded as a loss; prepaid share refunded, less any fee |
| *Cancellation fee* (with any of the above) | — | a fee line invoiced, or kept from the prepayment with a tax document (`wl-invoicing`); the manager may waive it |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `SAL.ACL.01` | Cancelling or refunding a paid order above a value, or after fulfilment started, needs a manager | `LIM`, `STA` | AED 50 (F&B default) |
| `SAL.ACL.05` | Amending an order or cancelling a line after its fulfilment started needs a manager, as does choosing *finish to stock* or *scrap* or waiving the cancellation fee; the requester cannot approve | `STA`, `SOD` | any value |

Event `SalesOrderCancelled`: order, reason, responsible party, outcome and fee per line.
Settings: outcomes offered: return to stock, finish to stock, scrap; fee (% of the line or
amount): 0. §10: whether a fee kept from a prepayment carries 5% VAT is `⚠ open: accountant
(A-04)`; the fee line takes a configurable VAT category until this is answered.

#### Edge cases decided
- **Lines not started:** no outcome asked; their reservation is released as in the base cancel.
- **Approval:** one request for the whole cancellation with all outcomes; the requester cannot approve. Rejected or returned: the order stays in its state; nothing is released.
- **Fulfilment starts while the approval waits:** at final approval, lines that started meanwhile have no outcome → the request `fail`s (guard re-check); the user resubmits with outcomes.
- **Fee amount:** % of the line value incl. VAT or a fixed amount, rounded to fils; never above the line's prepaid share when kept from the prepayment.
- **Fee VAT category:** configurable; default 5% standard-rated until A-04 is answered (`blocked:open-item` only for this choice).
- **Waived fee:** needs the manager (`SAL.ACL.05`), recorded on the line.
- **Refund:** total paid − fees kept; one refund request through wl-payments.
- **Outcome *finish to stock* in a vertical without work orders** (06, 08): not offered. Outcomes offered per vertical = the configured list filtered by what the vertical can do (06: return to stock, scrap; 08: return to stock).

#### Must not
- Must not cancel an order with any fulfilled quantity (use `cancel_remaining`).
- Must not post stock or wastage here: wl-inventory and the vertical react to the event (BL-STK-26).
- Must not let the requester approve.

#### Done when
- [ ] Given a `confirmed` prepaid F&B order AED 120 with line 1 (AED 80) started in the kitchen and line 2 (AED 40) not started, when cashier A cancels with outcome *scrap* for line 1 and fee 0, then an approval request is created and the order stays `confirmed`.
- [ ] When kitchen manager B approves, the order is `cancelled`, a refund of AED 120 is requested, and `SalesOrderCancelled` carries line 1 *scrap*, fee 0.
- [ ] Given fee 10% on line 1, the refund is AED 112 and a fee document of AED 8 is requested with the configured VAT category.
- [ ] Cashier A cannot approve.
- [ ] Given a paid order AED 30 with nothing started (below AED 50), no approval is needed.
- [ ] Given a line partially fulfilled, `cancel` is not offered.

#### Out of scope
`cancel_line` (BL-SAL-08), `cancel_remaining` (BL-SAL-09), stock effects (BL-STK-26), finish to stock in work orders (Epic 07 Manufacturing).

---

### BL-SAL-08 — wl-sales: cancel one line of an order (`cancel_line`)

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-SAL-07 · **Blocks:** BL-SAL-13, BL-STK-26, BL-AUT-15, BL-FNB-05, BL-MFG-03, BL-MFG-07, BL-SAL-17
**Review items:** X-14 (06 MEDIUM: item runs out after acceptance)

#### Goal
An item runs out, or the customer drops one line. Staff cancel the open quantity of that line
only, with a reason; the order continues. If the line's fulfilment started, an outcome and a
manager's approval are needed. The prepaid share is refunded less any fee.

#### Surfaces
- **Back office:** staff cancel the open quantity of one line with a reason; when its
  fulfilment started they also pick an outcome and a manager approves under "Needs your
  action" (not the requester).
- **System:** the reservation is released, the prepaid share is refunded less any fee, an
  issued invoice gets a credit note, the order becomes `fulfilled` when nothing is open, and
  `SalesOrderLineCancelled` is emitted.

#### Files to read
- `docs/modules/wl-sales.md` §4.2 row `cancel_line`, paragraph "Cancelling after fulfilment started", §6 event `SalesOrderLineCancelled`

#### Terms
- **Open quantity:** quantity − fulfilled − cancelled.
- (Outcome, fee, started, prepaid share: see BL-SAL-07.)

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel_line` | confirmed, partially_fulfilled | same state, fulfilled | `sales_orders.cancel` | reason; open quantity only; at least one line stays open or fulfilled (otherwise `cancel`); outcome per line and approval when fulfilment of the line started (`SAL.ACL.05`) | quantity cancelled; outcome applied (below); prepaid share refunded less any fee; an invoice already issued for the line is corrected by a credit note for the cancelled value (`wl-invoicing`); `fulfilled` when nothing is open; event `SalesOrderLineCancelled` |

Event `SalesOrderLineCancelled`: order, line, quantity, reason, outcome, fee.

#### Edge cases decided
- **Part of a line:** allowed; the cancelled quantity ≤ open quantity.
- **Last open line with others fulfilled:** goes to `fulfilled`.
- **Every line would end with nothing open and nothing fulfilled:** refused; use `cancel`.
- **Prepaid share:** line total incl. VAT × cancelled qty / line qty, rounded to fils; refunded through wl-payments.
- **Invoice already issued for the line** (invoice on confirmation): a credit note for the cancelled value is requested from wl-invoicing.
- **Approval rejected:** nothing changes.

#### Must not
- Must not cancel fulfilled quantity.
- Must not refund more than the line's prepaid share.

#### Done when
- [ ] Given a `confirmed` order with line A (5 × AED 20, not started) and line B open, when A is cancelled for 2 units reason "out of stock", then line A cancelled qty = 2, 2 units' reservation released, `SalesOrderLineCancelled` emitted, state stays `confirmed`.
- [ ] Given the order prepaid, AED 40 is refunded.
- [ ] Given line A picked, cancelling needs an outcome and approval by someone other than the requester.
- [ ] Given a `partially_fulfilled` order whose only open line is cancelled, it becomes `fulfilled`.
- [ ] Given a `confirmed` order with one line, `cancel_line` for all its quantity is refused.

#### Out of scope
Stock effects of the outcome (BL-STK-26).

---

### BL-SAL-09 — wl-sales: cancel the remaining lines with outcomes (`cancel_remaining`)

**Labels:** `module:wl-sales` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-SAL-07 · **Blocks:** BL-STK-26, BL-AUT-15, BL-FNB-16, BL-MFG-03, BL-SAL-17
**Review items:** X-14

#### Goal
A partially fulfilled order whose rest will never be delivered is closed off with
`cancel_remaining`. Since the change, lines whose fulfilment started need an outcome and an
approval, like `cancel`.

#### Surfaces
- **Back office:** staff run `cancel_remaining` with a reason on a `partially_fulfilled`
  order; started lines need an outcome and a manager's approval.
- **System:** reservations are released, prepaid shares are refunded less fees, and one
  `SalesOrderLineCancelled` is emitted per cancelled line.

#### Files to read
- `docs/modules/wl-sales.md` §4.2 row `cancel_remaining`, paragraph "Cancelling after fulfilment started"

#### Terms
See BL-SAL-07.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel_remaining` | partially_fulfilled | fulfilled | `sales_orders.cancel` | reason given; outcome per line and approval when fulfilment of a line started (`SAL.ACL.05`) | open lines cancelled with their outcomes; reservations released; one event `SalesOrderLineCancelled` per cancelled line |

#### Edge cases decided
- **Event:** emits one `SalesOrderLineCancelled` per cancelled line (so wl-inventory reacts the same way as for `cancel_line`).
- **Prepaid share:** refunded less fees, as `cancel_line`.
- **No started lines:** no approval.

#### Must not
- Must not cancel fulfilled quantities.

#### Done when
- [ ] Given a `partially_fulfilled` order with line A fulfilled and line B (3 open, not started), `cancel_remaining` with a reason makes it `fulfilled`, B cancelled qty 3, reservation released, one `SalesOrderLineCancelled` emitted.
- [ ] Given B picked, outcome and approval are required.

#### Out of scope
Stock effects (BL-STK-26).

---

### BL-SAL-10 — wl-sales: re-check credit at dispatch and hold the order

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle and credit check · **Blocks:** BL-SAL-11, BL-SAL-12, BL-STK-21, BL-AUT-18
**Review items:** X-14, 07-N3-b

#### Goal
A customer's credit can worsen between order confirmation and dispatch. Before goods on
account leave, the dispatch calls `credit_recheck(order)`. If it fails, the order goes to
`credit_hold`, the goods stay reserved, and the accountant and sales rep are told.

#### Surfaces
- **Back office:** the held order shows `credit_hold` with its reason and appears for the
  accountant and the sales rep ("orders on credit hold"); the goods stay reserved.
- **System:** each dispatch of an order on account calls `credit_recheck`, which counts the
  exposure without the order plus its undelivered value and runs `hold` on failure, storing
  the state before the hold and emitting `SalesOrderCreditHeld`.

#### Files to read
- `docs/modules/wl-sales.md` §3 (*Customer account*, *Credit check*), §4.2 (state `credit_hold`, rows `hold`, `fulfil`, paragraph "Credit re-check at dispatch"), §5.2 steps 2 and 4, §6 (`CreditChecker`, `SAL.ACL.03`, `SAL.RUL.01`, event `SalesOrderCreditHeld`), §7 settings

#### Terms
- **Exposure:** open invoices + uncleared post-dated cheques + confirmed orders not yet invoiced.
- **Overdue beyond grace:** invoices overdue more than the grace days past terms (default 30).
- **On-hold flag:** a customer account flag with reason and source (e.g. *bounced cheque*).
- **Order on account:** payment = *on account*.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `credit_hold` | warning | no | Blocked by the credit check at submit, by the re-check at dispatch, or by a bounced cheque (`SAL.CRD.01`) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `hold` | confirmed, partially_fulfilled | credit_hold | system (re-check at dispatch fails, or `SAL.RUL.05` on a bounced cheque) | order on account | state before the hold kept; dispatch blocked, reservations kept; event `SalesOrderCreditHeld` |
| `fulfil` | confirmed, partially_fulfilled | partially_fulfilled, fulfilled | system (`DispatchDelivered` for an order shipped on a dispatch note; `DeliveryDelivered` only for an order delivered without a dispatch note; collection recorded; or vertical fulfilment completed) | dispatch of an order on account passes `credit_recheck`; not yet fulfilled from the same dispatch or delivery (idempotent per source) | accepted quantities fulfilled (refused delivery lines are not); `fulfilled` when nothing is open; invoice drafted when the client invoices on delivery |
| `reverse_fulfilment` | partially_fulfilled, fulfilled | confirmed, partially_fulfilled | system (`DispatchReversed`, or `DispatchLost` of a `delivered` dispatch; for an order delivered without a dispatch note, `DeliveryReversed`, or `DeliveryLost` from `under_review`) | the quantities were fulfilled from that dispatch or delivery | quantity fulfilled reduced by them; `confirmed` when nothing stays fulfilled, otherwise `partially_fulfilled`; a draft invoice is updated, an issued one is left as it is and the accountant gets a task (credit note or keep for re-delivery, `wl-invoicing`); sales rep told; event `SalesOrderFulfilmentReversed` |

§4.2 *Fulfilment from deliveries*: An order shipped on a dispatch note (`wl-inventory`)
follows the dispatch only: `DispatchDelivered` runs `fulfil`, and the `Delivery*` events of the
same delivery are ignored, so one hand-over is never counted twice. Only an order handed to
`wl-delivery` without a dispatch note follows the `Delivery*` events. Then:

- *Returned* (`DispatchReturned`, or `DeliveryReturned` without a dispatch note): no order
  transition. Fulfilment progress of the returned lines goes back to *not dispatched*; the
  goods go to quarantine (`wl-inventory`); the lines keep their reservation request, and a new
  dispatch reserves again from available stock; sales rep and customer told; the lines are
  re-delivered or cancelled (`cancel_line`).
- *Reversed* (`DispatchReversed`, or `DeliveryReversed`): `reverse_fulfilment`; the goods
  follow the delivery again until its next outcome.
- *Lost* (`DispatchLost`, or `DeliveryLost`): the lost quantities are not fulfilled (a delivery
  lost while under review runs `reverse_fulfilment` first) and show as *lost* in the fulfilment
  progress. The sales rep gets a task: a replacement (new dispatch for the open lines) or
  `cancel_line` with a credit or refund. Goods found before that continue on their delivery.
- *After `closed`*: no transition (terminal). The accountant and the sales rep get a task to
  correct the invoice (credit note through a sales return, or re-delivery at zero price).
- *On `credit_hold`*: the quantity fulfilled is reduced and the state kept before the hold is
  set to `confirmed` or `partially_fulfilled` to match.

08 mapping, `return_delivery`: no order transition, on `DispatchReturned` (the dispatch
`return` runs on `DeliveryReturned`); goods back to quarantine through the dispatch; the lines
keep their reservation request and the new dispatch reserves again; sales rep and customer
told; the lines are re-delivered (new dispatch) or cancelled (`cancel_line`).

Event `SalesOrderFulfilmentReversed`: order, lines and quantities, source (dispatch or
delivery), cause (reversed, lost).

#### Edge cases decided
- **What the re-check counts:** the exposure without this order (open invoices + uncleared post-dated cheques + other confirmed orders not yet invoiced), plus this order's undelivered value, so the order is counted once; checked against the customer limit and, for a credit group, the combined limit. It fails when the sum is above the limit.
- **Return value:** `credit_recheck` returns pass/fail with the reason (limit, overdue, on hold) and runs `hold` itself on failure, so every caller behaves the same.
- **Released order dispatched again:** after `release` (BL-SAL-11), the dispatch passes without re-checking for that release: `credit_recheck` passes when the order's latest `release` approval is later than the last change to the account's exposure; otherwise it checks again.
- **Prepaid or cash-on-delivery orders:** not re-checked.
- **Setting off:** `credit_recheck` always passes.
- **Partial dispatch:** each dispatch of an order calls the check.
- **Counter collection:** the collection step in wl-sales calls the same guard.

#### Must not
- Must not release reservations on `hold`.
- Must not let goods leave when the check fails.

#### Done when
- [ ] Given a `confirmed` order on account AED 8,000 (nothing delivered), customer limit AED 50,000, and exposure without this order AED 46,000 at dispatch time (open invoices 38,000 + an uncleared post-dated cheque 8,000), when the dispatch calls `credit_recheck`, the check counts 46,000 + 8,000 = 54,000 > 50,000, then it fails, the order is `credit_hold` with state before hold `confirmed`, reservations stay, `SalesOrderCreditHeld` is emitted, accountant and rep are notified.
- [ ] Given exposure without this order AED 40,000 instead, the check counts 48,000 and passes; the dispatch goes on.
- [ ] Given an invoice 45 days overdue past terms (grace 30), the check fails.
- [ ] Given the account on-hold flag set, the check fails.
- [ ] Given a `partially_fulfilled` order, the state before hold is `partially_fulfilled`.
- [ ] Given a prepaid order, no check runs.

#### Out of scope
`release` (BL-SAL-11); the dispatch guard call in wl-inventory (BL-STK-21); bounced cheque (BL-SAL-12).

---

### BL-SAL-11 — wl-sales: release a held order back to the state before the hold

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** BL-SAL-10 · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-SAL-12, BL-SAL-17
**Review items:** X-14, X-05

#### Goal
A held order goes back to exactly where it was (`confirmed` or `partially_fulfilled`) when a
credit releaser approves. The releaser cannot be the order's creator.

#### Surfaces
- **Back office:** a holder of `credit.release` (never the order's creator) releases the
  held order with approval; the release screen warns "account on hold" and offers to request
  *Clear account hold* in the same action.
- **System:** the order returns to the stored state before the hold, and
  `SalesOrderConfirmed` is emitted only when the order was never confirmed before.

#### Files to read
- `docs/modules/wl-sales.md` §4.2 row `release`, §6 `SAL.ACL.03`

#### Terms
- **State before the hold:** stored on the order by `hold` (and by `submit` → `credit_hold`, where it is `confirmed`).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `release` | credit_hold | confirmed, partially_fulfilled | `credit.release` | approval (`PLT.APR.01`); releaser ≠ creator (`SAL.ACL.03`) | back to the state before the hold (`confirmed` from submit); open back-orders of the order reserved from available stock at the branch (Back-orders below); event `SalesOrderConfirmed` when first confirmed |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `SAL.ACL.03` | An order over the customer's or group's credit limit, or with invoices overdue beyond the grace days, goes on credit hold; the hold is released only by a role with `credit.release`, not by the order's creator | `LIM`, `SOD` | grace 30 days past terms |

#### Edge cases decided
- **`SalesOrderConfirmed`:** emitted only if the order had never been `confirmed` before (held at submit). An order held at dispatch does not emit it again (no second kitchen ticket or work order).
- **Account still on hold** (`SAL.RUL.05`): `release` of the order is allowed, but the next `credit_recheck` fails again while the account flag is set. The release screen warns "account on hold" and offers to request *Clear account hold* in the same action (BL-SAL-12; it still needs a second holder's approval).
- **Approval rejected:** the order stays `credit_hold`.

#### Must not
- Must not let the creator release, even as admin.
- Must not send the order to `confirmed` when it was `partially_fulfilled`.

#### Done when
- [ ] Given an order held at dispatch from `partially_fulfilled`, when user B (not the creator) releases with approval, it is `partially_fulfilled` and no `SalesOrderConfirmed` is emitted.
- [ ] Given an order held at submit, release makes it `confirmed`, reserves stock and emits `SalesOrderConfirmed`.
- [ ] The creator A cannot release.

#### Out of scope
Account hold (BL-SAL-12); reserving the order's back-orders on `release` (BL-SAL-17).

---

### BL-SAL-12 — wl-sales: put the customer account and its orders on hold after a bounced cheque (`SAL.RUL.05`)

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** BL-SAL-10 · BL-SAL-11 · Epic wl-payments: `ChequeBounced` · **Blocks:** BL-AUT-18, BL-MFG-14
**Review items:** X-05, 08-N3-a

#### Goal
When a customer's cheque bounces, the business must stop giving them more credit at once.
The customer account (and its credit group when configured) goes on hold, every open order on
account not yet dispatched is held, and new orders and counter sales on account are refused
until a credit releaser clears the account.

#### Surfaces
- **Back office:** submitting a new order on account, or a counter sale on account, is
  refused with "account on hold" (cash or card still allowed); a holder of `credit.release`
  requests *Clear account hold* on the customer account with a reason and another holder
  approves; roles without `credit.view` do not see the flag reason.
- **System:** `SAL.RUL.05` on `ChequeBounced` sets the account (and the credit group) on
  hold with the cheque as a source, runs `hold` on open orders on account not yet
  dispatched, and notifies the accountant and the sales rep.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Customer account* (on-hold flag), *Credit group*, §4.2 rows `submit`, `hold` and paragraph *Account hold*, §5.7 step 4, §6 `SAL.RUL.05`, `SAL.ACL.03`, §7 setting "Bounced cheque hold"

#### Terms
- **Drawer:** the party who issued the cheque.
- **Credit group:** companies under one owner sharing a combined limit (`PTY.REL.01`).
- **Not yet dispatched:** the order has open quantity whose dispatch is not `dispatched`.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `SAL.RUL.05` | `ChequeBounced` (`wl-payments`) | the drawer has a customer account | set the account on hold (and the credit group when configured); run `hold` on its orders on account not yet dispatched; refuse new orders and counter sales on account until the account hold is cleared (*Clear account hold*, §4.2); notify the accountant and the sales rep | whole group: on |
| `SAL.ACL.03` (part) | … an account hold (`SAL.RUL.05`) is cleared by a holder of `credit.release` with another holder's approval, never the requester | `LIM`, `SOD` | — |

`submit` guard (part): payment on account: the customer account is not on hold (`SAL.RUL.05`;
refused, the order stays `draft`).

Customer account: on-hold flag with reason (e.g. *bounced cheque*) and its open sources
(cleared by *Clear account hold*, §4.2).

§4.2 *Account hold*: `SAL.RUL.05` sets the account's on-hold flag with its source (the bounced
cheque). Each further bounced cheque adds a source, and the flag stays while any source is
open. While it is set, `submit` of a new order on account and a counter sale on account are
refused (the order stays `draft`; cash, card or online payment is still possible), and for
orders already submitted the credit check (`accept`, `amend`) and `credit_recheck` fail
(`credit_hold`). The flag is removed only by
the action *Clear account hold* on the customer account: requested by a holder of
`credit.release` with a reason (cheque replaced, balance paid, other) and approved
(`PLT.APR.01`) by another holder of `credit.release`; the requester cannot approve
(`SAL.ACL.03`). It clears that source on every account that got it (the whole credit group
when configured) and is audited. Clearing does not release held orders: each goes through
`release`, whose screen warns while the account is on hold and offers to request the
clearing in the same action.

§5.7 step 4: A bounced cheque from the customer puts the customer account and its open orders
on hold (`SAL.RUL.05`) until the cheque is replaced or the balance paid. A holder of
`credit.release` then requests *Clear account hold* and another approves (§4.2); the held
orders are released one by one.

#### Edge cases decided
- **Clearing the account flag:** the action *Clear account hold* (§4.2): requested by a holder of `credit.release` with a reason (cheque replaced, balance paid, other), approved by another holder of `credit.release`; the requester cannot approve. It is an action on the customer account, not a state machine; it is audited. Clearing does not release held orders; each is released (BL-SAL-11), whose screen offers to request the clearing too.
- **Group:** with "whole group" on, every member account gets the flag with source = the bounced cheque; clearing clears all flags from that source.
- **Several bounced cheques:** each adds a source; the flag stays while any source is open.
- **New orders vs existing orders:** a `draft` order on account cannot be submitted (refused, stays `draft`, message "account on hold"; it is not created in `credit_hold`). An order already in `pending_acceptance` is not held (no `hold` from there); its `accept` runs the credit check, which fails → `credit_hold`.
- **Counter sale on account:** refused at payment with "account on hold"; cash or card still allowed.
- **Idempotency:** the same `ChequeBounced` twice adds one source.
- **Prepaid orders:** not held.

#### Must not
- Must not hold dispatched goods or cancel orders.
- Must not show the flag reason to roles without `credit.view`.

#### Done when
- [ ] Given customer C (group G with member D, whole group on) with orders O1 `confirmed` on account and O2 `partially_fulfilled` with open lines not dispatched, when `ChequeBounced` for C arrives, then C and D are on hold with reason *bounced cheque*, O1 and O2 are `credit_hold`, accountant and rep are notified.
- [ ] A new order on account for D is refused at `submit` with "account on hold" and stays `draft` (no `credit_hold` order is created); the same order paid by card can be submitted; a counter sale on account for C is refused.
- [ ] Given an order O3 for C in `pending_acceptance` when the cheque bounced, `accept` sends it to `credit_hold`.
- [ ] When user B requests *Clear account hold* for C with reason *cheque replaced* and user E approves, the flags from that cheque are removed from C and D; O1 stays `credit_hold` until released.
- [ ] User B cannot approve their own clearing request, even as admin.

#### Out of scope
Cheque re-presentation and write-off (Epic wl-payments).

---

### BL-SAL-13 — wl-sales: fulfil only accepted quantities, handle a returned delivery and reverse fulfilment

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · BL-SAL-08 · BL-DLV-02 · BL-STK-22 · BL-STK-24 · BL-STK-25 · **Blocks:** BL-MFG-07
**Review items:** X-11, X-14, 07-N4-a (07 and 08 MEDIUM: returned delivery outcome)

#### Goal
The order counts as fulfilled only what the customer accepted. Refused lines stay open and
are re-delivered or cancelled with `cancel_line`. A delivery returned in full moves the lines'
fulfilment progress back to *not dispatched*, keeps the lines' reservation request and tells
the sales rep and the customer. A delivery reversed, or lost while under review, takes back
the fulfilled quantities (`reverse_fulfilment`); a lost delivery gives the sales rep a task.

#### Surfaces
- **Back office:** the order's fulfilment progress shows returned lines as *not dispatched*
  and lost lines as *lost*; the sales rep gets a task for a lost delivery, and the
  accountant and the sales rep get a task when a closed order's delivery is reversed or an
  issued invoice is affected.
- **System:** `fulfil` counts only accepted quantities once per source (dispatch note
  first), `reverse_fulfilment` runs on reversal or loss from review, a draft invoice is
  updated, and `SalesOrderFulfilmentReversed` is emitted.

#### Files to read
- `docs/modules/wl-sales.md` §4.2 rows `fulfil` and `reverse_fulfilment`, paragraph *Fulfilment from deliveries*, vertical mapping rows 08 `return_delivery` and `deliver`, §5.2 step 4, §6 (`OrderFulfilment`, consumed `Dispatch*` and `Delivery*` events, event `SalesOrderFulfilmentReversed`)
- `docs/modules/wl-inventory.md` §4 *Dispatch* rows `confirm_delivery`, `return`, `mark_lost`, `reverse` (events `DispatchDelivered`, `DispatchReturned`, `DispatchLost`, `DispatchReversed`)

#### Terms
- **Fulfilment progress:** a derived view of the order (not a state): not dispatched, picked, out for delivery, delivered.
- **Accepted quantity:** the delivered quantity on the proof of delivery.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `fulfil` | confirmed, partially_fulfilled | partially_fulfilled, fulfilled | system (delivery delivered, collection recorded, or vertical fulfilment completed) | dispatch of an order on account passes `credit_recheck` | accepted quantities fulfilled (refused delivery lines are not); `fulfilled` when nothing is open; invoice drafted when the client invoices on delivery |

08 mapping, `return_delivery`: no order transition: the order stays `confirmed` or
`partially_fulfilled` and its fulfilment progress goes back to *not dispatched* for the
returned lines; goods back to stock or quarantine through the dispatch (`wl-inventory`);
reservations kept; sales rep and customer told; the lines are re-delivered (new dispatch) or
cancelled (`cancel_line`).

§6 consumed: `DeliveryDelivered` (`fulfil` for accepted quantities), `DeliveryReturned`
(fulfilment progress of the returned lines back to *not dispatched*; reservations kept; sales
rep and customer told).

#### Edge cases decided
- **Two events for one hand-over:** an order delivered through a dispatch note gets both `DeliveryDelivered` (wl-delivery) and `DispatchDelivered` (wl-inventory). The order fulfils from `DispatchDelivered` and ignores the `Delivery*` events for the same delivery; `fulfil` is idempotent per source.
- **Refused quantity:** stays open on the line; nothing is cancelled automatically.
- **Invoice on delivery:** drafted for accepted quantities only.
- **Returned delivery and reservations:** the goods come back to quarantine (wl-inventory), so they are not available; the order keeps its reservation request, and wl-inventory re-reserves from available stock when a new dispatch is created.
- **Reversal after `closed`:** no transition; task to the accountant and the sales rep.
- **Reversal while `credit_hold`:** quantity fulfilled reduced; the stored state before the hold follows (`confirmed` when nothing stays fulfilled).
- **Issued invoice on reversal:** not changed automatically; the accountant gets a task.
- **Rounding:** quantities in the line's unit; no rounding.

#### Must not
- Must not fulfil the ordered quantity when lines were refused.
- Must not fulfil twice for one delivery.
- Must not create a replacement or a credit automatically for a lost delivery.

#### Done when
- [ ] Given a `confirmed` order line 10 units, when the delivery is delivered with 8 accepted and 2 refused, then line fulfilled = 8, state `partially_fulfilled`, and a draft invoice (invoice on delivery) bills 8.
- [ ] Given both `DeliveryDelivered` and `DispatchDelivered` for that delivery, fulfilled is 8, not 16.
- [ ] Given `DeliveryReturned` for a delivery of 5 units, the order state is unchanged, those lines show *not dispatched*, and the rep and customer are notified.
- [ ] After that, `cancel_line` (BL-SAL-08) of the 5 units is possible.
- [ ] Given a `fulfilled` order (10 of 10 from one dispatch), on `DispatchReversed` it is `confirmed`, fulfilled = 0, a draft invoice for it is updated, and `SalesOrderFulfilmentReversed` is emitted; `DeliveryReversed` for the same delivery changes nothing more.
- [ ] Given a `closed` order, `DispatchReversed` leaves it `closed` and the accountant and sales rep get a task.
- [ ] Given `DispatchLost` of a `dispatched` dispatch, nothing is fulfilled, the lines show *lost* and the sales rep gets a task; from a `delivered` dispatch, `reverse_fulfilment` runs first.

#### Out of scope
Dispatch side (BL-STK-22, BL-STK-24, BL-STK-25). Credit notes (Epic wl-invoicing).

---

### BL-SAL-14 — wl-sales: refund after completion with nothing returned (`SAL.ACL.06`)

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Sales return lifecycle · Epic wl-ticketing: complaint tickets · Epic wl-payments: refunds · Epic wl-invoicing: credit note · **Blocks:** BL-FNB-07, BL-FNB-08, BL-FNB-09, BL-FNB-10, BL-FNB-16
**Review items:** X-14, 06-N4-b

#### Goal
Food that arrived cold or wrong, a missing item, or a service not delivered cannot come back
as goods. Staff raise a sales return of kind *no goods* from the completed order or a
complaint ticket, with a complaint category and a responsible party, and resolve it directly
with a credit note or refund. Above AED 50 a manager approves, never the person who recorded it.
When an aggregator refunded its customer itself, the vertical resolves the return *refunded by
the channel* (no money leaves), and reverses it when the aggregator takes the refund back.

#### Surfaces
- **Back office:** staff (for example cashier A) raise a *no goods* return from the
  completed order or a complaint ticket with a complaint category, responsible party and
  refund per line; above AED 50 a manager approves under "Needs your action" (not the
  recorder); `returns.handle` can `reverse` a return resolved *refunded by the channel* with
  a reason.
- **System:** the 06 payout statement `match` resolves a return *refunded by the channel*
  and `void` reverses it; the refund cap per order and line is enforced and `ReturnResolved`
  or `ReturnReversed` is emitted.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Sales return*, §4.5 (states `resolved` and `reversed`, rows `resolve` and `reverse`, paragraphs "Refund after completion (kind *no goods*)" and "Refunded by the channel"), §5.3 step 5, §5.6 step 4, §6 (`SAL.ACL.06`, events `ReturnResolved` and `ReturnReversed`), §7 setting "Refund after completion"

#### Terms
- **Kind:** *goods* (something comes back) or *no goods*.
- **Complaint category:** cold, late, missing item, wrong item, quality, service not delivered, other.
- **Responsible party:** business, driver or rider, aggregator, supplier, customer.
- **Refunded by the channel:** a resolution for an aggregator order whose customer the aggregator
  refunded itself, deducting it from the payout; no credit note and no refund here.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `resolved` | success | no | Credited, refunded or replaced; stock restored or written off. Final, except that a refund made by the channel can still be reversed |
| `reversed` | neutral | yes | A refund made by the channel was taken back by the channel or recorded in error; the order's amount refunded is reduced again |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `resolve` | received; requested (kind *no goods*) | resolved | `returns.handle`, or system (vertical: 06 payout statement `match`, resolution *refunded by the channel*) | inspection recorded (kind *goods*); resolution chosen; restocking fee set; value above threshold approved; kind *no goods*: complaint category, responsible party and approval (`SAL.ACL.06`); resolution *refunded by the channel* only for kind *no goods* on an order of an aggregator channel | credit note (`wl-invoicing`) or refund (`wl-payments`) or replacement order at zero price, or, for *refunded by the channel*, no credit note and no refund (the channel refunded the customer and deducts it from its payout); stock restored or written off (kind *goods*); order's amount refunded updated; event `ReturnResolved` |
| `reverse` | resolved | reversed | system (vertical: 06 payout statement `void`, or `resolve` of a dispute in which the aggregator takes the refund back) or `returns.handle` | resolution *refunded by the channel*; reason given | the order's amount refunded reduced by the return's value; refund reports exclude the return; no money and no document moves; event `ReturnReversed` |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `SAL.ACL.06` | A refund or credit after completion with nothing returned needs a complaint category and a responsible party; above a value it needs a manager, and whoever records it cannot approve it (not for resolution *refunded by the channel*, where no money leaves the business) | `LIM`, `SOD` | AED 50 |

"Refund after completion": raised from the completed order or from a complaint ticket and
resolved directly from `requested`. The responsible party drives reports and recovery: a
supplier fault can open a supplier claim, a rider fault goes to the rider's record. The
refund is full or partial per line.

"Refunded by the channel": an aggregator refunds its customer itself and deducts the amount
from its payout. The vertical records it as a *no goods* return resolved *refunded by the
channel* (06: on payout statement `match`): no credit note, no refund payment and no
`SAL.ACL.06` approval, because no money leaves the business; category and responsible party
are still required. The order's amount refunded and the refund reports include it. When the
channel takes the refund back, or the statement it came from is voided, the return runs
`reverse`. Returns resolved by credit note, refund or replacement are never reversed: they
are corrected with a new document.

Event `ReturnResolved`: return, kind, original sale, resolution, value, lot, complaint
category, responsible party. Event `ReturnReversed`: return, original sale, value, reason.

#### Edge cases decided
- **Cap:** the total refunded on an order (all returns) cannot exceed what was paid for it; per line, not above the line value.
- **"Above a value":** the return's total value incl. VAT > AED 50 needs approval; ≤ 50 resolves at once.
- **Order state:** the order may be `fulfilled` or `closed`; it is not changed; its *amount refunded* increases.
- **Responsible party *aggregator*:** recorded for reports. When the aggregator already refunded the customer, the resolution is *refunded by the channel* (only on an order of an aggregator channel; refused on other channels): no refund, no credit note, no approval; the amount refunded still rises and the cap still applies.
- **Reverse:** only a return resolved *refunded by the channel*; any other resolution is refused (corrected with a new document). Reversing twice is refused (`reversed` is terminal).
- **Approval rejected:** the return stays `requested`.
- **Linked ticket:** when raised from a ticket, the ticket is linked; resolution adds a comment.

#### Must not
- Must not move stock for kind *no goods*.
- Must not let the recorder approve.
- Must not issue a refund larger than paid.

#### Done when
- [ ] Given a completed order AED 90 (line pizza AED 60), when cashier A raises a *no goods* return for the pizza, category *cold*, responsible *driver or rider*, refund AED 60, then an approval request is created; A cannot approve.
- [ ] When manager B approves, the return is `resolved`, a refund of AED 60 is requested, the order's amount refunded is AED 60, and `ReturnResolved` carries the category and responsible party.
- [ ] Given a refund of AED 30, it resolves without approval.
- [ ] Without a complaint category, `resolve` is refused.
- [ ] Given Talabat order ORD-1001 (AED 25 line), when the system resolves a *no goods* return of
      AED 25 *refunded by the channel*, category *cold*, party *business*, no approval, refund or
      credit note is created and the amount refunded is AED 25.
- [ ] *Refunded by the channel* on a counter-channel order is refused.
- [ ] `reverse` of that return with reason "statement voided" makes it `reversed`, the amount
      refunded goes back to 0 and `ReturnReversed` is emitted; `reverse` of a return resolved by
      refund is refused.
- [ ] Given AED 60 already refunded on the pizza line, a second *no goods* return of AED 10 on the same line is refused (above the line value of AED 60).

#### Out of scope
Supplier claim and rider record follow-up (verticals); creating and reversing the returns from payout statements (BL-FNB-09, BL-FNB-10).

---

### BL-SAL-15 — wl-sales: accept returns of recalled goods into the recall's hold

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** Base: wl-sales Sales return lifecycle · BL-STK-14 · BL-STK-17 · **Blocks:** None.
**Review items:** X-09

#### Goal
When customers bring back goods from a recalled lot, the return is accepted even outside the
return window, linked to the recall, and the quantity joins the recall's stock hold. The
recall, not the return, decides what happens to the goods.

#### Surfaces
- **Back office:** staff `receive` a return of recalled goods outside the return window
  without approval; the return shows its link to the recall and the goods go to quarantine
  on the recall's hold.
- **System:** `LotTrace` decides whether the lot, or a non-lot item by received date range
  and sale date, is in an open recall, and resolving the return skips the stock part.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Sales return* (recall), §4.5 row `receive`, §5.6 step 2, §6 consumed (`LotTrace`)
- `docs/modules/wl-inventory.md` §4 *Recall* (paragraph after the trace table), *Stock hold*

#### Terms
- **Open recall:** a wl-inventory recall in `open` or `traced`.
- **`LotTrace`:** wl-inventory service that tells whether a lot (or, for items not tracked by lot, an item and receipt date) is in an open recall's scope.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `receive` | requested | received | `returns.handle` | original sale linked; within the return window or approved (a return of recalled goods, including a non-lot item sold in an open recall's trace window, is always accepted) | stock received into quarantine (`wl-inventory`); a lot under an open recall links the return to the recall and adds the qty to its stock hold; an item not tracked by lot is treated the same when it is in an open recall by item and received date range and the original sale is dated in the recall's trace window |

§5.6 step 2: Goods from a lot under an open recall are accepted outside the return window,
linked to the recall and held with it; the recall decides their outcome.

#### Edge cases decided
- **Item not tracked by lot (08 parts):** when the item is in an open recall by item and received date range (BL-STK-17) and the original sale is dated in its trace window (from the first receipt in the range to the trace end date, or today), the return is treated as recalled goods.
- **Resolution:** the customer is still credited or refunded through `resolve`; the return's stock part ("stock restored or written off") is skipped for recalled goods because the recall decides.
- **Return window:** 14 days does not apply; no approval is needed for lateness.

#### Must not
- Must not restock recalled goods through the return.

#### Done when
- [ ] Given recall RCL-1 open on lot L7 and a return of 2 units of L7 sold 60 days ago (window 14), when `receive` runs, it is accepted without approval, linked to RCL-1, and the 2 units are in quarantine and on RCL-1's hold.
- [ ] When the return is resolved with a credit note, no stock is restored.
- [ ] Given recall RCL-2 on part P (not lot-tracked, received 2026-08-01–2026-08-31, first receipt 2026-08-05, no trace end) and a return of 1 P sold 2026-09-10, it is linked to RCL-2 and joins its hold; a P sold 2026-08-01 is a normal return.

#### Out of scope
Recall decisions (BL-STK-16).

---

### BL-SAL-16 — wl-sales: keep a package active when its payment is reversed

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: wl-sales Package lifecycle · Epic wl-invoicing: `InvoiceReopened` · **Blocks:** BL-CLN-16
**Review items:** X-03, X-06

#### Goal
When the payment for a package is reversed or charged back, its invoice reopens. The package
stays `active` and used credits stay used. The staff who sold it get a task to collect, the
holder is told the balance is due, and an optional setting stops further consumption until
the balance is paid.

#### Surfaces
- **Back office:** the seller (05 front desk, 10 counsellor) gets a "collect before next
  use" task, which also lists consumptions refused while suspended.
- **System:** on `InvoiceReopened` the package stays as it is and the holder is told the
  balance is due; with *suspend consumption while unpaid* on, `consume` is refused until the
  invoice is paid.

#### Files to read
- `docs/modules/wl-sales.md` §4.4 (row `consume`, paragraph "Payment reversed after activation"), §6 consumed (`InvoiceReopened`), §7 setting "Package: suspend consumption…"

#### Terms
- **`InvoiceReopened`:** wl-invoicing event when a paid invoice gets an open balance again (payment reversed).
- **Seller:** 05: front desk; 10: counsellor.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `consume` | active | active, completed | system (source event: booking completed, attendance submitted, job card closed) | credits remaining ≥ quantity; not expired (`CLN.ACL.10`); when *suspend consumption while unpaid* is on (§7), no unpaid balance reopened on the package's invoice | movement recorded with its source; `completed` at zero; event `PackageConsumed` |

"Payment reversed after activation": the package does not change state: credits already used
stay used and the package stays `active`. The staff who sold it get a task to collect before
the next use, and the holder is told the balance is due. A setting can suspend further
`consume` until the reopened balance is paid (off by default); the vertical then decides
whether a booked session goes ahead.

Setting: suspend consumption while a reopened invoice balance is unpaid: off (05, 10: task to collect only).

#### Edge cases decided
- **Package `completed`:** same treatment (task and notice); there is nothing left to suspend.
- **Instalment packages:** only a reopened balance on the package's invoice counts; a future instalment not yet due does not.
- **Suspension and a booked session:** `consume` is refused; the source event (e.g. booking completed) records "credit not consumed: unpaid" and the vertical decides; this module only refuses.
- **Balance paid later:** suspension ends with the `paid` invoice; earlier refused consumptions are not replayed automatically; the seller's task lists them for manual `consume`.
- **Idempotency:** one task per `InvoiceReopened` event.

#### Must not
- Must not change the package state or remove used credits.

#### Done when
- [x] Given an `active` package (10 credits, 4 used) whose invoice is reopened, it stays `active` with 6 left, the seller has a "collect before next use" task, and the holder gets a balance-due message.
- [x] With the setting off, the next `consume` succeeds.
- [x] With the setting on, `consume` is refused until the invoice is paid, then succeeds.

#### Out of scope
Payment reversal and invoice reopening (Epic wl-payments, Epic wl-invoicing).

---

### BL-SAL-17 — wl-sales: back-order lines without stock and serve them oldest first

**Labels:** `module:wl-sales` `area:negative-flow` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · BL-SAL-08 (`cancel_line`) · BL-SAL-09 (`cancel_remaining`) · BL-SAL-11 (`release`) · Base: wl-inventory reorder suggestions (`STK.RUL.04`) and goods receipt · **Blocks:** BL-AUT-15
**Review items:** 08 MEDIUM "back-orders"

#### Goal
When a part is not in stock, an order line is back-ordered instead of refused: the available
part is reserved, the rest of the order is picked and fulfilled, the open quantity feeds the
reorder suggestions, and arriving stock is reserved for the oldest back-orders first, with the
customer told. The customer may cancel a back-order. This is the module behaviour; 08 turns it
on (BL-AUT-15).

#### Surfaces
- **Back office:** staff can cancel a back-order with `cancel_line` (no outcome needed) or
  `cancel_remaining`; the order line shows its quantity back-ordered (the 08 settings and
  screens are BL-AUT-15).
- **System:** on `confirmed` the missing quantity is back-ordered, added to the `STK.RUL.04`
  suggestion, and reserved oldest first when stock arrives at the branch or the order is
  released; the customer gets one message per order.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Sales order* (quantity back-ordered), §4.2 (row `submit`,
  paragraph "Back-orders", rows `cancel_line`, `cancel_remaining` and `release`), §6 *Consumes*
  (`wl-inventory`), §7 setting "Back-orders allowed per channel"
- `docs/modules/wl-inventory.md` §6 `STK.RUL.04` and the paragraph on back-ordered quantity
  after the rules table; §8 `ReorderPolicy`

#### Terms
- **Back-order:** the quantity of a confirmed line that could not be reserved; stored as the
  line's quantity back-ordered.
- **Reorder suggestion:** a proposed purchase line per supplier (`STK.RUL.04`, daily 07:00).
- **Order's branch:** the branch whose location stock the order reserves from.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | awaiting_payment, pending_acceptance, credit_hold, confirmed | `sales_orders.create`, or the external party through the portal | lines priced; manual prices and discounts within limits (`SAL.ACL.02`) or approved; the external order number (compared trimmed and case-insensitive) is not already used on this channel (a duplicate import is refused and points to the existing order); when *Unique customer PO number per customer* is on, the customer PO reference with its call-off number is not already used by another order of the same customer, cancelled orders included (refused, pointing to the existing order); payment on account: the customer account is not on hold (`SAL.RUL.05`; refused, the order stays `draft`) | target: `awaiting_payment` if paid online and not yet confirmed; else `pending_acceptance` if the channel needs acceptance; otherwise credit check → `credit_hold` or `confirmed`; on `confirmed`: stock reserved (when back-orders are allowed, quantity without available stock is back-ordered instead of refused), confirmation sent, event `SalesOrderConfirmed` |
| `cancel_line` | confirmed, partially_fulfilled | same state, fulfilled | `sales_orders.cancel` | reason; open quantity only; at least one line stays open or fulfilled (otherwise `cancel`); outcome per line and approval when fulfilment of the line started (`SAL.ACL.05`) | quantity cancelled; outcome applied (below); prepaid share refunded less any fee; an invoice already issued for the line is corrected by a credit note for the cancelled value (`wl-invoicing`); `fulfilled` when nothing is open; event `SalesOrderLineCancelled` |
| `cancel_remaining` | partially_fulfilled | fulfilled | `sales_orders.cancel` | reason given; outcome per line and approval when fulfilment of a line started (`SAL.ACL.05`) | open lines cancelled with their outcomes; reservations released; one event `SalesOrderLineCancelled` per cancelled line |
| `release` | credit_hold | confirmed, partially_fulfilled | `credit.release` | approval (`PLT.APR.01`); releaser ≠ creator (`SAL.ACL.03`) | back to the state before the hold (`confirmed` from submit); open back-orders of the order reserved from available stock at the branch (Back-orders below); event `SalesOrderConfirmed` when first confirmed |

**Back-orders.** When the setting *Back-orders allowed* is on for the channel, a line whose
quantity cannot be reserved from available stock at the order's branch on `confirmed` is not
refused: the available part is reserved and the rest is *back-ordered* (quantity
back-ordered on the line; the order stays `confirmed` or `partially_fulfilled`, and the other
lines are picked and fulfilled). Back-ordered quantity not yet covered by an open purchase
order counts as demand in `STK.RUL.04` (`wl-inventory`). When stock arrives at the branch
(`StockMoved` for a `receive` or `transfer_in` movement, `wl-inventory`), it is reserved for
the open back-orders of the item oldest first (by the time the line was confirmed; on a tie,
by order number, then line number); orders on `credit_hold` are skipped. When such an order
is released (`release`), its open back-orders are reserved at once from the stock then
available at the branch, in the same order; what stays short waits for the next arrival.
The customer gets one message per order when any of its back-orders is reserved. A goods
receipt that is reversed releases the reservations made from it, and the lines are
back-ordered again. The customer may cancel a back-order with `cancel_line` (no
outcome needed: nothing was picked) or `cancel_remaining`; a cancelled back-order leaves the
demand of `STK.RUL.04`.

wl-inventory §6, after the rules table: Back-ordered sales order quantity (`wl-sales`, setting
*Back-orders allowed*) not yet covered by an open PO is added to the `STK.RUL.04` suggestion
for the item: the suggested quantity is the quantity from the item's `ReorderPolicy` (min/max:
up to max; par: up to par; reorder point + lead time in 08), plus that back-ordered quantity.
When the item is above its reorder point, the policy suggests nothing and the suggestion is
the back-ordered quantity alone. A cancelled back-order drops out at the next run.

| Setting | Default |
|---|---|
| Back-orders allowed per channel | off (08: on for counter, phone and sales rep channels) |

#### Edge cases decided
- **Partial stock:** the available quantity is reserved; only the rest is back-ordered.
- **Order of service:** by the time the line was confirmed (oldest first); on a tie, by order
  number, then line number. Orders on `credit_hold` are skipped.
- **Released order:** `release` reserves its open back-orders at once from the stock then
  available at the branch (stock not reserved for other back-orders); the rest waits for the
  next arrival in the normal order.
- **Branch:** only stock arriving at the order's branch serves its back-orders.
- **Arrival larger than the back-orders:** the rest stays available.
- **Customer message:** one per order when any of its back-orders is reserved, not one per line.
- **Receipt reversed later:** reservations made from it are released and the lines are
  back-ordered again (oldest-first order kept).
- **Channel with the setting off:** `submit` keeps today's behaviour (no back-order is created).
- **Idempotency:** one reservation pass per `StockMoved` event.

#### Must not
- Must not reserve arriving stock for a newer back-order before an older one.
- Must not keep a cancelled back-order in the suggestions.
- Must not hold the rest of the order back because one line is back-ordered.

#### Done when
- [ ] Given 2 units of part P in stock and SO-1 for 5 P on a channel with back-orders on, on
      confirm 2 are reserved and 3 back-ordered; the next 07:00 suggestion includes 3 P.
- [ ] Given part Q above its reorder point (min/max policy) and a back-order of 2 Q not covered
      by an open PO, the next suggestion for Q is 2; below its reorder point it is the quantity
      up to max plus 2.
- [ ] Given SO-1 (confirmed 09:00) and SO-2 (10:00) each back-ordering 3 P, a receipt of 4 P
      reserves 3 for SO-1 and 1 for SO-2; each customer gets one message.
- [ ] Given SO-3 on `credit_hold` (confirmed 08:00) with a back-order of 2 P, a receipt of 2 P
      serves SO-1 first; when SO-3 is released with 1 P available, 1 P is reserved for it at
      once and the other 1 P is served from the next receipt.
- [ ] Given SO-4 lines 1 and 2 and SO-5 line 1, each back-ordering 1 P and confirmed in the
      same second, a receipt of 2 P reserves SO-4 line 1 and SO-4 line 2.
- [ ] `cancel_line` on SO-2's back-order needs no outcome and removes it from the next suggestion.
- [ ] Reversing the 4 P receipt releases the reservations and both lines are back-ordered again.
- [ ] With the setting off, a line without stock is not back-ordered.

#### Out of scope
The 08 settings and screens (BL-AUT-15); the reorder suggestion engine (base `STK.RUL.04`).

---

### BL-SAL-18 — wl-sales: refuse a second order with the same customer PO number for a customer

**Labels:** `module:wl-sales` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: wl-sales Sales order lifecycle (`submit`) · BL-SAL-05 (`amend`) · **Blocks:** BL-MFG-06
**Review items:** X-14, 07 MEDIUM "duplicate PO upload"

#### Goal
B2B customers email a PO and sometimes also upload it in the portal. When the setting is on,
the same customer PO number must not create two orders for the same customer: `submit` and
`amend` refuse it and point to the existing order. Call-offs against a blanket PO are told
apart by a call-off number. 07 turns the setting on (BL-MFG-06).

#### Surfaces
- **Back office:** `submit` or `amend` with a customer PO reference (and call-off number)
  already used by the same customer is refused and names the existing order, cancelled
  orders included.
- **Portal:** a buyer's PO upload refused for a duplicate shows only the existing order's
  number and status.
- **System:** a unique key on customer, PO reference and call-off number lets only one of
  two concurrent submits pass.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Sales order* (customer PO reference, call-off number), §4.2
  (rows `submit` and `amend`, paragraph "Customer PO number"), §7 setting "Unique customer PO
  number per customer"

#### Terms
- **Customer PO reference:** the purchase order number printed on the customer's own PO.
- **Blanket PO / call-off:** one framework PO under which the customer calls off deliveries;
  each call-off has its own number.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | awaiting_payment, pending_acceptance, credit_hold, confirmed | `sales_orders.create`, or the external party through the portal | lines priced; manual prices and discounts within limits (`SAL.ACL.02`) or approved; the external order number (compared trimmed and case-insensitive) is not already used on this channel (a duplicate import is refused and points to the existing order); when *Unique customer PO number per customer* is on, the customer PO reference with its call-off number is not already used by another order of the same customer, cancelled orders included (refused, pointing to the existing order); payment on account: the customer account is not on hold (`SAL.RUL.05`; refused, the order stays `draft`) | target: `awaiting_payment` if paid online and not yet confirmed; else `pending_acceptance` if the channel needs acceptance; otherwise credit check → `credit_hold` or `confirmed`; on `confirmed`: stock reserved (when back-orders are allowed, quantity without available stock is back-ordered instead of refused), confirmation sent, event `SalesOrderConfirmed` |
| `amend` | pending_acceptance, credit_hold, confirmed, partially_fulfilled | same state, or credit_hold | `sales_orders.amend` | reason; no line below its fulfilled quantity; a line whose fulfilment started is not reduced by `amend` (it goes through `cancel_line`, which asks for an outcome); prices and discounts within `SAL.ACL.02`; channel allows amendments (aggregator orders are changed on the aggregator); approval when fulfilment started (`SAL.ACL.05`); a changed customer PO reference or call-off number passes the same uniqueness check as `submit` | revision + 1 and the previous revision kept; reservations and vertical fulfilment adjusted; credit check re-run when the value rises (fails → `credit_hold`); prepaid: payment link for an increase, refund of a decrease (`wl-payments`); a draft invoice is updated, an issued one is corrected by credit note or an extra invoice (`wl-invoicing`); revised confirmation sent; event `SalesOrderAmended` |

**Customer PO number.** When *Unique customer PO number per customer* is on, `submit` and
`amend` refuse a customer PO reference that another order of the same customer already uses,
compared trimmed and case-insensitive with its call-off number (a call-off against a blanket
PO carries its own call-off number). Cancelled orders still count, so the customer sends a new
number or a call-off number. The same number for different customers is allowed; orders
without a customer PO reference are not checked. A portal upload refused for this reason
shows the buyer only the existing order's number and status.

| Setting | Default |
|---|---|
| Unique customer PO number per customer (with call-off number) | off (07: on) |

#### Edge cases decided
- **Key:** (customer, customer PO reference, call-off number or empty), trimmed and
  case-insensitive.
- **Same number, different customers:** allowed.
- **Earlier order cancelled or rejected:** still counts as used; staff see the cancelled order
  from the refusal.
- **Empty customer PO reference:** not checked (the account's *customer PO required* setting is
  a separate check).
- **Concurrent submits:** enforced by a unique key so only one passes.
- **Portal message:** names the existing order number and status only (no prices or notes).

#### Must not
- Must not create a second order or reservation for a duplicate.
- Must not show a buyer orders of another customer.

#### Done when
- [ ] Given SO-70 for customer C with PO "PO-8812", submitting a new order for C with
      "po-8812 " is refused and names SO-70.
- [ ] A portal upload of "PO-8812" by buyer B of C is refused and B sees only SO-70's number and
      status.
- [ ] Given "BL-1" / call-off "CO-1" on SO-71, "BL-1" / "CO-2" is accepted and "BL-1" / "CO-1"
      is refused.
- [ ] Given customer D, an order with "PO-8812" is accepted.
- [ ] Amending SO-72 of C to "PO-8812" is refused.
- [ ] With the setting off, the second "PO-8812" order for C is accepted.

#### Out of scope
Duplicate external numbers per channel (BL-SAL-01); 07 settings and portal upload screen
(BL-MFG-06).

---

### BL-SAL-19 — wl-sales: run campaigns with a budget and cost entries

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-kernel state machines and approvals · Base: wl-invoicing supplier bills · **Blocks:** BL-SAL-20, BL-SAL-21, BL-SAL-25, BL-NTF-07, BL-ACC-23, BL-AUT-18, BL-CLN-15, BL-CON-18, BL-EDU-17, BL-FNB-17, BL-LOG-25, BL-MFG-14, BL-MPW-20, BL-PRP-27, BL-SAL-28
**Review items:** G1 (SME problem map 1, 2, 4)

#### Goal
Marketing staff record each campaign (an ad set, a flyer run, an event) with its marketing
channel, period, budget and a code, and enter what it cost, or link the agency's or ad
platform's supplier bill line. The campaign moves through `draft`, `active` and `ended`, on
its dates or by hand. Without costs per campaign the business cannot see what a customer
costs to win (`SAL.CRM.02`).

#### Surfaces
- **Back office:** marketing staff create a campaign (channel, period, budget, code), add
  cost entries or link supplier bill lines, and `activate`, `end` or `reopen` it; an entry
  over budget waits for a manager under "Needs your action" (not the requester); the owner
  gets the 80% and 100% alerts.
- **System:** `activate` and `end` run daily at 00:05 Asia/Dubai, and on
  `SupplierBillApproved` the linked entries enter the budget check under the campaign lock.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Campaign*, *Campaign cost entry*; §4.6 and the text after
  it; §5.8 steps 1–2; §6 `SAL.ACL.07`, `SAL.RUL.10`, consumed `SupplierBillApproved`; §7 rows
  on campaigns
- `docs/modules/wl-invoicing.md` §3 supplier bill (only the line fields)

#### Terms
- **Marketing channel:** where the campaign runs (Google, Meta, print…); not the order
  *Channel* of sales orders.
- **Campaign code:** a unique short code put in web form links and on a linked promo code, so
  leads and sales find their campaign.
- **Purpose:** `acquisition` (win new customers) or `retention` (message existing ones,
  BL-SAL-25).
- `SAL.ACL.07`: shared access rule; here only its budget part.
- `SAL.RUL.10`: budget alert to the campaign owner.

#### Spec excerpt (authoritative for this issue)

| Entity | Description | Important fields |
|---|---|---|
| Campaign | A paid or unpaid marketing action with a period (`SAL.CRM.02`), or a message to a segment (`SAL.LOY.01`) | name, code (unique; used in web form links and linked promo codes), purpose (`acquisition`, `retention`), marketing channel (Google, Meta, TikTok, other online, print, radio or outdoor, event, email, SMS, WhatsApp, partner, other; not the order *Channel*), lead source it implies (e.g. *paid ads*), brand, entity (default: the brand's entity), branch (optional), owner (staff; told of costs, approvals and the end), start, end, budget (AED), promo code (optional, `SAL.DSC.01`), retention only: segment, notification template, message channel, send time (between start and end), retention fields set by (the requester of the broadcast, `NTF.ACL.04`), broadcast and its status, status |
| Campaign cost entry | Money spent on a campaign | campaign, date, amount in AED excluding recoverable VAT (a bill in another currency at the bill's exchange rate), description, supplier bill line (optional, `wl-invoicing`), entered by, approved by, status (`pending_approval`, `approved`, `rejected`, `voided`), reason (rejection or void); an entry linked to a bill takes its amount from the bill line and counts only once the bill is `approved` |

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Being prepared; its code cannot be used yet |
| `active` | info | no | Running; leads and parties can be attributed to it |
| `ended` | neutral | no | Past its end date or stopped; can be reopened |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `activate` | draft | active | `campaigns.manage`, or system (start date reached) | name, code, purpose, marketing channel, owner, start set; retention: segment, approved template, message channel and a send time between start and end set (`SAL.LOY.01`; a send time already past sends at once); the vertical's guards pass (05: `CLN.ACL.13` when the campaign promotes services) | code usable in links and the linked promo code; retention: the broadcast is requested from `wl-notify` for the send time, with the user who set the retention fields as requester (§5.10, `NTF.ACL.04`); event `CampaignActivated` |
| `end` | active | ended | `campaigns.manage`, or system (end date passed, or a vertical guard of `activate` stops passing, e.g. 05: the advertising permit's validity ends) | — | new leads stop taking the campaign after the late-response days (§7); the linked promo code expires with it; retention: a broadcast still `pending_approval` or `scheduled` is cancelled; owner told; event `CampaignEnded` |
| `reopen` | ended | active | `campaigns.manage` | new end date in the future; the vertical's guards of `activate` pass; retention: no second send once a broadcast completed (a new campaign is created for that) | the linked promo code is valid again to the new end date, unless its uses are used up |

A draft campaign is deleted, not cancelled. Cost entries may be added in any state, because
bills often arrive after the end. A campaign with leads, acquisitions or cost entries is never
deleted.

**Cost entries** are `approved` on save, except one that takes the approved costs over the
budget, which is `pending_approval` until a manager other than the requester decides
(`SAL.ACL.07`). A rejected entry becomes `rejected` with the reason, is never counted, and
the requester and the owner are told; a returned entry goes back to the requester to edit
and submit again (wl-kernel §4.1). Only a pending entry is edited. An entry entered in error
is voided with a reason by `campaigns.costs.manage` (audited) and a new one entered; reports
drop it from the period of its date. An entry linked to a supplier bill line counts only
once the bill is `approved` (`SupplierBillApproved`); when the bill is voided, its linked
entries are voided automatically and the owner is told; a debit note on the bill is matched
by a negative entry entered by the accountant. While its bill is not `approved` (for example
`matched` or `exception`), a linked entry is left out of the budget check, the alerts and every
report; on `SupplierBillApproved` it is checked against the budget as a new entry would be
(over budget: `pending_approval`, the user who linked it as requester). A negative entry is
`approved` on save without a budget check and never resets an alert already sent.
`SAL.RUL.10` tells the owner when approved costs reach 80% and 100% of the budget.

| ID | When | If | Then | Parameters (default) | Generalises |
|---|---|---|---|---|---|
| `SAL.RUL.10` | a cost entry of a campaign is approved | approved costs reach a threshold share of the budget for the first time | notify the campaign owner | 80% and 100% | — |

| ID | Rule | Pattern | Parameter (default) | Generalises |
|---|---|---|---|---|
| `SAL.ACL.07` | A manual points adjustment above a number of points, or closing a loyalty account that still has points at staff's initiative (not at the member's request or on erasure, §4.7), needs a manager, and the requester cannot approve; a campaign cost entry that takes the campaign over its budget needs a manager other than the requester | `LIM`, `SOD` | 500 points; any budget overrun | — |

| Setting | Default |
|---|---|
| Campaign cost over budget needs a manager (`SAL.ACL.07`) | on |
| Budget alerts to the campaign owner (`SAL.RUL.10`) | 80% and 100% |
| Late-response days after a campaign ends (leads still attributed) | 30 |

#### Edge cases decided
- **Start date in the past on `activate`:** allowed; the campaign is active from now, the
  start date stays as entered for reports.
- **System `activate` and `end`:** run daily at 00:05 Asia/Dubai; idempotent per campaign
  and date. An end date earlier than the start date is refused on save.
- **Cost linked to a supplier bill line:** amount taken from the line excluding recoverable
  VAT, in AED at the bill's exchange rate; one bill line may be split over several campaigns,
  but the sum may not exceed the line. The entry counts only once the bill is `approved`:
  while the bill is `matched`, `exception` or any state before `approved`, the entry is in no
  budget check, alert or total. On `SupplierBillApproved` the budget check and the alerts run
  for it then, under the campaign lock; over budget, it becomes `pending_approval` with the
  user who linked it as requester. A voided bill voids its linked entries (owner told).
- **Debit note on a linked bill:** the entry does not change; the accountant enters a negative
  entry (manual, not linked). It is `approved` on save, needs no budget check (it only lowers
  costs), and lowering costs below 80% or 100% does not reset an alert already sent.
- **Budget empty:** no over-budget check and no alerts. **Over budget:** the entry is saved as
  `pending_approval` and counts in reports only after approval; the requester cannot approve.
  Rejected: `rejected`, never counted, requester and owner told. Returned: editable, then
  submitted again.
- **Correction:** an `approved` entry is never edited; it is voided with a reason (audited) and
  a new one entered. A pending entry may be edited by its requester.
- **Concurrency:** the budget check and the alert thresholds are evaluated under a lock per
  campaign, so two entries saved at once cannot both pass under the budget.
- **Alerts:** each threshold once per campaign; a void that takes costs back below it does not
  reset it.
- **Vertical guard stops passing** (05: permit validity ends): the system runs `end` that day;
  `reopen` needs the guard to pass again.
- **`reopen`:** the linked promo code is valid again to the new end date unless used up.
- **Retention guards** (segment, template) are checked here as fields only; sending is
  BL-SAL-25.
- **Code reuse:** codes are unique for the client for ever, ended campaigns included.

#### Must not
- Must not delete a campaign that has leads, acquisitions or cost entries.
- Must not count a cost entry pending approval in any report.
- Must not reuse a code of an ended campaign.
- Must not count a cost entry linked to a bill that is not `approved`, or one that is
  `rejected` or `voided`.

#### Done when
- [ ] Given draft campaign C1 (code `SPRING26`, Meta, 1–31 Mar 2026, budget AED 5,000), on
      1 Mar at 00:05 it becomes `active` and `CampaignActivated` is emitted once.
- [ ] Given C1 active, on 1 Apr at 00:05 it becomes `ended`; `reopen` with end 15 Apr makes
      it `active`; `reopen` with an end date in the past is refused.
- [ ] Given C1 with AED 4,800 of costs, user A adds AED 400: the entry waits for approval; B
      (manager) approves it; A cannot approve it.
- [ ] Given a supplier bill line of AED 3,000, two entries of AED 2,000 and AED 1,000 are
      accepted and a third of AED 1 is refused.
- [ ] A second campaign with code `spring26` is refused.
- [ ] Deleting C1 after a cost entry is refused; deleting a draft with nothing linked works.
- [ ] B rejects A's pending AED 400 entry: it is `rejected`, not in any total, and A and the
      owner are told.
- [ ] An approved AED 1,000 entry voided with a reason disappears from the report; the void is
      in the audit log.
- [ ] An entry linked to a bill in `exception` is not counted; it counts when the bill is
      approved; voiding the bill voids the entry and tells the owner.
- [ ] Given C1 with AED 4,000 approved and an AED 2,000 entry linked to a bill in `matched`: no
      alert and no approval yet; when the bill is approved the entry becomes `pending_approval`
      (AED 6,000 > AED 5,000) and, once approved by B, the owner gets the 100% alert.
- [ ] A negative entry of −AED 300 for a debit note is `approved` on save and lowers C1's
      costs; an alert already sent is not sent again when costs rise past it once more.
- [ ] Approved costs reaching AED 4,000 of AED 5,000 tell the owner once (80%); reaching AED
      5,000 tells the owner again (100%).
- [ ] Two entries of AED 300 saved at the same time on C1 with AED 4,500 of costs: one is
      approved, the other waits for approval.
- [ ] `reopen` of C1 to 15 Apr makes its linked promo code valid to 15 Apr again.

#### Out of scope
Attribution of leads and parties (BL-SAL-20); reports (BL-SAL-21); retention sending
(BL-SAL-25); 05's advertising permit guard (`CLN.ACL.13`, 05 epic).

---

### BL-SAL-20 — wl-sales: record the source and campaign of leads and parties and link the first sale

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:public` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-SAL-19 · Base: wl-sales Lead lifecycle · Base: wl-invoicing Invoice lifecycle (`InvoiceIssued`) · Base: wl-parties merge (`PartiesMerged`) · Base: wl-kernel `void` (§5.12) · **Blocks:** BL-SAL-21, BL-ACC-23, BL-AUT-18, BL-CLN-15, BL-CON-18, BL-EDU-17, BL-FNB-17, BL-LOG-25, BL-MFG-14, BL-MPW-20, BL-PRP-27, BL-SAL-27
**Review items:** G1 (SME problem map 1, 2, 4)

#### Goal
Every lead and every new customer gets a source (referral, walk-in, website…) and, when
known, a campaign. The party's first sale is linked with its value, so the business can say
which source and campaign brought which customers and how much they bought. This works with
or without leads (a counter business has no pipeline).

#### Surfaces
- **Back office:** staff must pick a source (and a campaign where required) before a counter
  sale for a new walk-in party completes; staff change an acquisition with a reason, and
  after the first sale another `campaigns.manage` holder approves; sales without a source
  appear on an exceptions report.
- **Public:** a lead sent through a web form link with a campaign code (`?c=SPRING26`) gets
  its source and campaign (screen not specified yet).
- **System:** `record_first_sale` links the first sale and emits `CustomerAcquired` once;
  the nightly run replaces a cancelled or credited first sale and emits
  `CustomerAcquisitionChanged`.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Lead*, *Lead source*, *Party acquisition* and the
  paragraph after the entity table; §4.1 rows `win`, `revert_win`, the text after the
  diagram and *Vertical mapping*; §5.1; §5.8 steps 3–4; §6 `CampaignAttribution`,
  `CustomerAcquired`, `CustomerAcquisitionChanged`, `LeadWinReverted`, `SAL.ACL.09`,
  consumed `PartiesMerged`; §7 rows on campaigns; §10 *Erasure*

#### Terms
- **Party acquisition:** one record per party saying how it became a customer.
- **First sale:** the first confirmed sales order, completed counter sale, issued invoice, or
  the vertical's conversion record (01 lease, 04 engagement, 10 enrolment) of the party.
- **First-touch, single attribution:** the first campaign known for the party gets all the
  credit.
- **Late-response days:** after a campaign ends, leads arriving with its code are still
  attributed for this many days.

#### Spec excerpt (authoritative for this issue)

| Entity | Description | Important fields |
|---|---|---|
| Lead | An enquiry not yet converted | party (may be a new, unverified party), source (from the lead source list), campaign (optional; set from the campaign code of a web form link or promo code, or by staff), branch (the campaign's branch, else the web form's branch, else the owner's branch; its entity is the lead's entity in reports), interest (catalogue items or free text), owner (staff), next action date, lost reason, converted subject, status |
| Lead source | Where a lead or a customer came from (`SAL.CRM.02`) | code, name EN/AR, active. Seeded: referral, walk-in, website, social, paid ads, aggregator, event, partner, other; the client may add more but not delete a source in use |
| Party acquisition | How a party became a customer, kept per party (`SAL.CRM.02`) | party, source, campaign (optional), set from (lead, staff, web form, promo code), first sale (`Subject`: the first confirmed sales order, completed counter sale, issued invoice, or the vertical's conversion record such as a lease, enrolment or engagement), first sale date, first sale value (excluding VAT, net of credit notes), branch (of the first sale, else of the converting lead; its entity is the acquisition's entity in reports), acquired at, change history (source or campaign changed, with reason and approver, `SAL.ACL.09`), superseded by (after a party merge) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `win` | contacted, qualified | won | system (conversion event) | — | converted `Subject` and source recorded; the party acquisition takes the lead's source and campaign when the party has none yet; event `LeadWon` |
| `revert_win` | won | qualified | system (the converted `Subject` is voided as entered in error) | — | converted `Subject` cleared; a party acquisition set from this lead with no other first sale is cleared (`CustomerAcquisitionChanged`); owner told; event `LeadWinReverted` |

A conversion record that is cancelled for a business reason after the lead was won (the
customer backs out of an accepted quotation, a lease ends early) leaves the lead `won`: it
did convert. Only a `void` (entered in error, wl-kernel §5.12) runs `revert_win`.

The source and campaign of a party live in its *party acquisition* record here, not on the
`wl-parties` record, so that a party bought by several brands of the client keeps one record
per client and the attribution stays in the module that reports on it. Source is required
when a party gets its first sale (staff pick it; a web form or a promo code sets it); a party
created before `SAL.CRM.02` was turned on is shown as *unknown*. When two parties are merged
(`PartiesMerged`), the survivor keeps the acquisition acquired first; the other is kept as
history (*superseded by*), so the customer is counted once.

§5.8 steps 3–4:

3. Every lead and every new party gets a source and, when known, a campaign. Attribution is
   single and first-touch: the campaign of the lead that converted the party, otherwise the
   campaign set on the party acquisition (by staff, a form or the promo code of the first
   sale). A lead that arrives with the code after the campaign ended is still attributed
   during the late-response days (§7), then gets the source only. A web form lead for a party
   that already has an open lead on the same campaign is added to that lead as a note, not
   counted again; leads closed as `lost` with the reason *duplicate* or *spam or invalid* are
   left out of every lead count. Changing the source or campaign of an acquisition that has
   a first sale, or of a won lead, needs a reason and a second person (`SAL.ACL.09`).
4. The first sale of the party is linked when it happens (first confirmed sales order,
   completed counter sale, issued invoice, or the vertical's conversion record). Its value
   excluding VAT and net of credit notes on it is the *converted value*; with the setting
   *attribution window*, the value is the party's sales in the first N months instead, net of
   credit notes. When the first sale is voided, cancelled or credited in full, the nightly run
   links the party's next sale as the first sale (the acquired date stays); with none, the
   first sale is cleared and the party is not a new customer until it buys.
   `CustomerAcquisitionChanged` is emitted; reports are recomputed, never stored.

| ID | Rule | Pattern | Parameter (default) | Generalises |
|---|---|---|---|---|
| `SAL.ACL.09` | Changing the source or campaign of a party acquisition that has a first sale, or of a won lead, needs a reason and approval by another holder of `campaigns.manage`; the change is audited and shown on the exceptions report | `STA`, `SOD` | on | — |

| Setting | Default |
|---|---|
| Lead sources, lost reasons, qualification fields | seeded sources (§3 *Lead source*); lost reasons and qualification fields per vertical starter list, always with *duplicate* and *spam or invalid* (left out of lead counts, §5.8) |
| Campaign required for sources (`SAL.CRM.02`) | none (05: *paid ads*, *social*) |
| Late-response days after a campaign ends (leads still attributed) | 30 |
| Attribution window for the converted value | first sale only (alternative: sales in the first N months, N = 12) |
| Changing the attribution of an acquired party or a won lead needs a second person (`SAL.ACL.09`) | on |

#### Edge cases decided
- **Order of the first sale:** the earliest by document date; on a tie, the one recorded
  first. A first sale later voided, cancelled or credited in full is replaced by the next one
  at the next nightly run; the acquisition date stays the first; with no other sale the first
  sale is cleared. A partial credit note lowers the converted value. Each change emits
  `CustomerAcquisitionChanged`; `CustomerAcquired` is never emitted twice for a party.
- **Conversion record voided:** `revert_win` takes the lead back to `qualified`; a business
  cancellation (customer backs out) keeps it `won`.
- **Party with no source at first sale:** staff must pick one before the sale completes; an
  import or an API sale without one is set *other* and listed on an exceptions report.
- **Party created before the capability was on:** source *unknown*; not counted as a new
  customer.
- **Lead and party disagree:** the converting lead wins only when the party has no
  acquisition yet; an existing acquisition is never overwritten automatically. Staff change
  it with a reason; once it has a first sale, another `campaigns.manage` holder approves
  (`SAL.ACL.09`); the requester cannot approve; the change is audited.
- **Duplicates and spam:** a form lead for a party with an open lead on the same campaign is
  a note on that lead; leads `lost` as *duplicate* or *spam or invalid* are not counted.
- **Party merge:** the survivor keeps the acquisition with the earlier acquired date; the
  other is marked *superseded by*; leads move with the party (wl-parties).
- **Erasure:** the party reference is removed from leads and the acquisition; source,
  campaign, dates and values stay.
- **Campaign code of a draft campaign:** ignored (source only). Of an ended campaign: attributed
  within the late-response days, else source only.
- **Several brands:** one acquisition per party per client.
- **Idempotency:** `record_first_sale` called twice for the same party keeps the first.
- **Campaign required (05):** with the setting on for *paid ads* and *social*, a first sale or
  a lead with either source and no campaign is refused until staff pick one.
- **Branch:** a lead takes the campaign's branch, else the web form's, else its owner's; an
  acquisition takes the branch of its first sale, else of the converting lead (BL-SAL-21
  uses it for entity and scope).
- **Conversion records in tests:** verticals call `record_first_sale` and `win` for their own
  records (vertical epics); this issue tests `win` and `revert_win` with a test conversion
  record that supports the kernel `void` (§5.12), standing in for 01's lease.

#### Must not
- Must not overwrite an existing party acquisition from a later lead or promo code.
- Must not store attribution on the `wl-parties` record.

#### Done when
- [ ] Given web form link `?c=SPRING26` while C1 is active, the new lead has source *paid ads*
      and campaign C1.
- [ ] Given lead L1 (C1) won by a quotation accepted, party P gets acquisition source *paid
      ads*, campaign C1; P's first invoice of AED 2,100 incl. VAT (AED 2,000 excl.) is linked as
      first sale with value AED 2,000 and `CustomerAcquired` is emitted once.
- [ ] Given C1 ended on 31 Mar, a lead with `SPRING26` on 20 Apr is attributed to C1 and one on
      5 May gets the source only.
- [ ] Given P already acquired through *referral*, a later lead L2 with C1 won for P leaves P's
      acquisition unchanged.
- [ ] A counter sale for a new walk-in party cannot complete until a source is picked; with
      05's setting (*paid ads*, *social*), picking *social* also needs a campaign.
- [ ] Given P's first sale SO-1 cancelled, the nightly run links P's next sale SO-2.
- [ ] Given P's first invoice of AED 2,000 excl. VAT credited by AED 500, the converted value is
      AED 1,500.
- [ ] Given test conversion record X-1 that won lead L3, voided as entered in error, L3 is
      `qualified` again and `LeadWinReverted` is emitted.
- [ ] Staff A changes P's campaign from C1 to C2 after P's first sale: the change waits for B;
      A cannot approve; after B approves, `CustomerAcquisitionChanged` is emitted and audited.
- [ ] A second web form submission for P on C1 while L1 is open adds a note to L1 and creates
      no lead.
- [ ] Merging P2 (acquired 1 May) into P (acquired 1 Mar) keeps P's acquisition; P2's is
      superseded and the report counts one new customer.

#### Out of scope
Reports (BL-SAL-21); verticals calling `record_first_sale` for their records (vertical
epics); 01 enquiry fields (`PRP.LEA.01`, 01 epic).

---

### BL-SAL-21 — wl-sales: report leads, conversion, cost per lead, CAC and ROAS

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-SAL-19 · BL-SAL-20 · **Blocks:** BL-SAL-25, BL-ACC-23, BL-AUT-18, BL-CLN-15, BL-CON-18, BL-EDU-17, BL-FNB-17, BL-LOG-25, BL-MFG-14, BL-MPW-20, BL-PRP-27, BL-SAL-27
**Review items:** G1 (SME problem map 1, 2, 4)

#### Goal
The owner sees, per source, campaign, marketing channel, brand, branch and period, how many
leads came, how many converted, what they cost, what a new customer cost (CAC) and how much
revenue each dirham of spend brought (ROAS), and exports it.

#### Surfaces
- **Back office:** the owner opens the acquisition report under Reports, filters it by
  source, campaign, marketing channel, brand, branch and month, sees leads, conversion, cost
  per lead, CAC and ROAS ("—" when undefined, costs split *linked to bills* and *manual*)
  within their scope, and exports it to CSV.

#### Files to read
- `docs/modules/wl-sales.md` §5.8 step 5; §3 *Party acquisition*, *Campaign cost entry*

#### Terms
- **Conversion rate:** leads won ÷ leads.
- **Cost per lead:** cost ÷ leads. **CAC:** cost ÷ new customers.
- **ROAS (return on ad spend):** converted value ÷ cost.
- **Converted value:** the first sale value excluding VAT (or sales in the attribution window).

#### Spec excerpt (authoritative for this issue)

5. The acquisition report shows, per source, campaign, marketing channel, brand, branch and
   period: leads, leads won, conversion rate (won ÷ leads), new customers (party
   acquisitions with a first sale in the period), cost, cost per lead (cost ÷ leads),
   customer acquisition cost (cost ÷ new customers) and return on ad spend (converted value
   ÷ cost). Sources without campaigns show leads and conversion with no cost. Costs are
   counted in the period of the entry date; leads and customers in the period they were
   created or won. A ratio with a zero divisor shows "—", never zero. Only `approved` cost
   entries count; cost is shown split into *linked to bills* and *manual*, so the accountant
   can reconcile it with the marketing expense in the accounts. The report filters by entity
   as well: a campaign's leads, customers and costs take the campaign's entity; a lead or
   acquisition without a campaign takes the entity of its branch (§3). Branch scope: a
   campaign with a branch belongs to that branch; a campaign without one is entity-wide, so
   its cost is shown only to users whose scope covers every branch of its entity, and a user
   scoped to some branches sees its leads and customers of those branches with cost, cost per
   lead, CAC and ROAS shown as "—".

#### Edge cases decided
- **Periods:** months in Asia/Dubai; costs by entry date; leads by created date; leads won and
  new customers by win or first sale date. So a campaign's cost and its customers can fall in
  different months; the per-campaign view (all time) shows them together.
- **Cost entries counted:** `approved` only; pending, rejected and voided entries, and entries
  linked to a bill not yet approved, are excluded (BL-SAL-19).
- **Leads counted:** leads `lost` as *duplicate* or *spam or invalid* are left out of leads,
  conversion and cost per lead.
- **Converted value:** net of credit notes; a cleared or replaced first sale (BL-SAL-20)
  moves the customer to the month of the new first sale or out of the report.
- **Reconciliation:** cost is shown as *linked to bills* and *manual*; the bill-linked part
  equals the approved bill lines coded to the campaigns.
- **Entity:** filter by the campaign's entity; a lead or party acquisition without a campaign
  takes the entity of its branch field (BL-SAL-20: lead from the campaign, web form or owner;
  acquisition from the first sale or the converting lead).
- **Zero divisor:** "—", never 0 or infinity.
- **Rounding:** AED to 2 decimals, rates to 1 decimal %, ROAS to 2 decimals.
- **Scope:** a user sees branches in their scope only; totals are of what they can see. A
  campaign with a branch belongs to it. A campaign without a branch is entity-wide: its costs
  are shown only to users whose scope covers every branch of the entity; a user scoped to
  some branches sees that campaign's leads and customers of their branches, with cost, cost
  per lead, CAC and ROAS "—".
- **Attribution window setting changed:** the report recomputes; values are not stored.

#### Must not
- Must not include VAT in converted value or cost.
- Must not show 0 for an undefined ratio.

#### Done when
- [ ] Given C1 with costs AED 5,000, 50 leads, 10 won, 8 new customers with first sales of
      AED 20,000 excl. VAT, the report shows conversion 20.0%, cost per lead AED 100.00, CAC
      AED 625.00 and ROAS 4.00.
- [ ] Source *referral* with 12 leads and no campaign shows leads and conversion and "—" for
      cost per lead, CAC and ROAS.
- [ ] With the attribution window set to 12 months and those 8 customers' sales in their first
      12 months totalling AED 45,000 excl. VAT, ROAS is 9.00 (AED 45,000 ÷ AED 5,000).
- [ ] A user scoped to branch B1 does not see B2's leads or costs; for campaign C9 with no
      branch (AED 1,000), the B1 user sees C9's B1 leads with cost "—", and a user scoped to
      every branch of the entity sees AED 1,000.
- [ ] The report exports to CSV with the same numbers.
- [ ] Of C1's 50 leads, 5 are `lost` as *spam or invalid*: the report shows 45 leads and cost
      per lead AED 111.11.
- [ ] C1's AED 5,000 shows as AED 3,000 linked to bills and AED 2,000 manual.
- [ ] A first sale of AED 2,000 credited by AED 500 adds AED 1,500 to converted value.

#### Out of scope
Retention campaign results (BL-SAL-25); charts (shared report work).

---

### BL-SAL-22 — wl-sales: enrol members and earn points or visits on qualifying sales (`SAL.RUL.07`)

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:portal` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: wl-sales Counter sale, Sales order and Sales return lifecycles · Base: wl-invoicing `InvoicePaid`, `CreditNoteIssued`, `InvoiceReopened`, `InvoiceWrittenOff` · Base: wl-payments `PaymentReversed` · **Blocks:** BL-SAL-23, BL-SAL-24, BL-SAL-25, BL-SAL-26, BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-FNB-17
**Review items:** G4 (SME problem map 32, 35)

#### Goal
A client turns on a loyalty programme; customers enrol and earn points per AED spent or a
visit per sale. A return or credit note, a reversed payment, a write-off or a reversed
fulfilment takes the points back in proportion, and staff cannot collect other customers'
points on their own account. This replaces
the F&B-only `FNB.LOY.01` with a shared capability used by 05, 06, 08 and 10.

#### Surfaces
- **Back office:** at the counter the cashier enrols a member (terms version recorded) and
  attaches the member to a sale before it completes by code or mobile number; attaching to
  the cashier's own party or to a completed sale is refused; the manager sees accounts over
  the daily sales limit listed for review.
- **Portal:** a customer, or a guardian for a minor, enrols in the portal and is attached to
  a sale by portal login (screen not specified yet).
- **Public:** a customer enrols on the ordering site (screen not specified yet).
- **System:** `SAL.RUL.07` earns points or visits once per sale, `SAL.RUL.09` and returns
  reverse them in proportion, and the member is told when a reward becomes available.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Loyalty programme*, *Loyalty account*, *Points movement*;
  §4.7 first paragraph; §5.9 steps 1, 2, 4 and 7; §6 `LoyaltyLedger`, `SAL.RUL.07`,
  `SAL.RUL.09`, `SAL.ACL.08`, `Points*` events; §7 loyalty rows

#### Terms
- **Qualifying sale:** a completed counter sale, a sales order reaching `fulfilled`, or a paid
  invoice not made from an order or counter sale.
- **Earn lot:** the points of one `earn` movement with their own expiry date.
- **Eligible amount:** paid or owed by the member, excluding VAT, fees, tips, excluded items,
  the redeemed value, insurer share (`FIN.TPP.01`) and package credits; the member's share is
  taken excluding its VAT.

#### Spec excerpt (authoritative for this issue)

| Entity | Description | Important fields |
|---|---|---|
| Loyalty programme | The earn and redeem terms of the client (`SAL.LOY.01`) | name, brand (one programme per brand at most), earn kind (`per_aed`: points per AED spent; `per_visit`: one visit per qualifying sale), earn rate (points per AED, or visits per reward), minimum qualifying amount, excluded items or categories, point value (AED per point), minimum points to redeem, maximum share of a sale paid with points, visit reward (fixed discount or a free catalogue item), points validity (months after earning), terms version and terms text (EN/AR, `wl-documents`), change effective date, closing date (when turned off), active |
| Loyalty account | A party's membership; one per programme and party, and one per programme and mobile number | programme, party, accepted by (the party, or a guardian for a minor, `PTY.REL.01`), mobile number for codes, enrolled at, enrolled by (staff, portal, counter), terms version accepted, points balance, points held (redemption in progress), visits since the last reward, reward available until, paused (party blocked), status (`active`, `closed`), closed reason (member request, staff, merged into, erased) |
| Points movement | A change to a loyalty account | account, kind (`earn`, `redeem`, `reverse_earn`, `reverse_redeem`, `expire`, `adjust`, `transfer` (party merge)), points (or visits), AED value, entity of the sale, source `Subject` (counter sale, sales order, invoice, sales return, credit note, payment reversal, write-off), earn lot expiry date (earn), lot consumed (redeem, expire), reason and approval (adjust), user, time |

### 4.7 Loyalty account

Two states, no diagram: `active` (earns and redeems) and `closed` (terminal; the member asked
to leave, staff closed it, the party was merged, or the party's data was erased under
`wl-parties`). A closed account is never reopened; the party enrols again with a zero balance.
Earn lots are consumed oldest expiry first by `redeem` and `expire`. (Closure, merge, block
and programme changes: BL-SAL-26.)

§5.9 steps 1, 2, 4 and 7:

1. A customer enrols at the counter, in the portal or on the ordering site; the terms
   version is recorded. Enrolment is not marketing consent: that is asked for separately
   (`PTY.CNS.01`). A minor (05 dependants, 10 students) is enrolled by a guardian
   (`PTY.REL.01`), who accepts the terms and receives the codes. A mobile number already on
   another account of the programme is refused.
2. A qualifying sale earns points or a visit (`SAL.RUL.07`): a completed counter sale, a
   sales order reaching `fulfilled`, or a paid invoice for a sale not made through an order
   or a counter sale (05 treatments, 10 courses). Points are computed on the amount the
   member paid or owes for the sale, excluding VAT, fees, tips, excluded items and the
   redeemed value; the share paid by an insurer (`FIN.TPP.01`) or with package credits earns
   nothing (the member's share is taken excluding its VAT). A sales order on account or cash
   on delivery earns on `fulfilled` on the member's share of its value; a later write-off or
   reversed payment takes the points back (`SAL.RUL.09`). Each sale earns once. The member
   is attached to the sale before it completes, by their code, a portal login or their
   mobile number; never afterwards, and never by a cashier to their own party
   (`SAL.ACL.08`). An account that earns on more than the daily sales limit (§7) is listed
   for the manager's review. When the payment of an earning sale is reversed, its invoice
   written off, or its fulfilment reversed, the points are reversed in proportion and earned
   again if it is paid or fulfilled again (`SAL.RUL.09`).
4. A sales return or credit note on an earning sale reverses the earned points in proportion
   (`reverse_earn`); the balance may go below zero and later earnings offset it, with no cash
   recovery. A return of a sale that redeemed points gives the points back (`reverse_redeem`)
   with their original expiry, or 30 days when that date has passed. In a per-visit programme
   a returned counted sale takes one visit off; a reward made available by it and not used is
   withdrawn. A reward is available for the points validity.
7. The member sees, in the portal and on each receipt, the balance, the points earned and
   redeemed on the sale, and the next expiry date with its points; the terms in English and
   Arabic are linked.

| ID | When | If | Then | Parameters (default) | Generalises |
|---|---|---|---|---|---|
| `SAL.RUL.07` | a qualifying sale completes (`CounterSaleCompleted`; sales order → `fulfilled`; `InvoicePaid` for an invoice not made from an order or a counter sale), or a sales return or credit note on such a sale is issued | the party has an `active` loyalty account that is not paused (party blocked); the member was attached before completion (`SAL.ACL.08`); eligible amount ≥ the minimum; not yet earned (or reversed) for this source | `earn` points or a visit with the lot's expiry date, or `reverse_earn` in proportion; when visits reach the reward, the reward is made available and the member told | 1 point per AED 1; minimum AED 0; validity 12 months | — |
| `SAL.RUL.09` | `PaymentReversed` on a payment of an earning counter sale; `InvoiceReopened` with reason *payment reversed*, or `InvoiceWrittenOff`, for an earning invoice or the invoice of an earning order or counter sale; `SalesOrderFulfilmentReversed` for an earning order | the source earned points or a visit not yet reversed for this cause | `reverse_earn` in proportion to the amount reversed, written off or no longer fulfilled (the balance may go below zero); notify the member; when the sale is paid or fulfilled again (`InvoicePaid`, `PaymentReceived`, `fulfil`), earn again for that amount | — | — |

| ID | Rule | Pattern | Parameter (default) | Generalises |
|---|---|---|---|---|
| `SAL.ACL.08` | A member is attached to a sale only before the sale completes, by the member's one-time code, portal login or mobile number; a cashier cannot attach a sale they process to a loyalty account of their own party; a mobile number belongs to one account per programme | `STA`, `SOD` | — | — |

| Setting | Default |
|---|---|
| Loyalty programme (`SAL.LOY.01`): earn kind, rate, minimum, exclusions, point value, minimum to redeem, maximum share of a sale | off; when on: per AED, 1 point per AED 1, AED 0, none, AED 0.01 per point, 500 points, 100% (06: 50%) |
| Points validity; expiry reminder (`SAL.RUL.08`) | 12 months; 30 days before |
| Sales per member per day before the account is listed for review (`SAL.ACL.08`) | 5 |

#### Edge cases decided
- **Rounding:** points are whole numbers, rounded down per sale (AED 99.90 → 99 points).
- **Insurer share and VAT:** the eligible amount is the member's share excluding VAT, so the
  insurer's share is taken off excluding VAT too (on a AED 1,000 excl. VAT invoice with an
  insurer share of AED 800 excl. VAT, the member's AED 200 excl. VAT earns).
- **Order not yet paid:** a sales order on account or cash on delivery earns on `fulfilled` on
  its eligible value, unpaid or not; a write-off of its invoice or a reversed payment reverses
  it (`SAL.RUL.09`).
- **One earn per source:** the source `Subject` is the key; an invoice made from an order or a
  counter sale never earns again.
- **Per visit:** one visit per sale at or above the minimum; a reward becomes available when
  visits reach the rate and the counter restarts at 0.
- **Partial return:** reversed points = earned points × returned eligible amount ÷ original
  eligible amount, rounded up; never more than earned.
- **Negative balance:** allowed after a reversal; next earnings offset it; no cash recovery.
- **Enrolment after the sale:** earns nothing for past sales.
- **Closed account:** earns nothing; a return on a sale earned before closing reverses nothing.
- **Programme off:** no earning; balances stay (closing date and expiry: BL-SAL-26).
- **Payment reversed or written off** (`SAL.RUL.09`): reversed points = earned points ×
  reversed (or written-off) amount ÷ the sale's paid amount, rounded up; once per cause; when
  the sale is paid again, the same share is earned again with a new lot. A chargeback that
  the business wins (`PaymentReceived`) earns again.
- **Fulfilment reversed:** `SalesOrderFulfilmentReversed` reverses in proportion to the
  reversed quantity value; a later `fulfil` earns again.
- **Attaching a member** (`SAL.ACL.08`): only before completion, by the member's code, portal
  login or mobile number; the cashier's own party (the party of the staff user's employee
  record) is refused; a mobile number already on another account of the programme is refused
  at enrolment.
- **Daily sales limit:** the 6th earning sale of an account in a day (default 5) still earns,
  and the account is listed for the manager's review.
- **Minor:** enrolled by a guardian (`PTY.REL.01`), who accepts the terms; codes go to the
  guardian's mobile.
- **Per visit reversal:** a returned counted sale takes one visit off; a reward made available
  by it and not yet used is withdrawn; a reward is available for the points validity.
- **Paused (party blocked):** earns nothing; nothing is earned later for sales while paused.

#### Must not
- Must not earn on VAT, on the insurer's share or on the redeemed value.
- Must not earn twice for one sale.
- Must not treat enrolment as marketing consent.
- Must not let points stay earned on a sale whose payment was reversed or written off.

#### Done when
- [ ] Given programme 1 point per AED 1 and member M, a counter sale of AED 105 incl. 5% VAT
      earns 100 points expiring 12 months later.
- [ ] Given an invoice of AED 1,000 excl. VAT (AED 1,050 incl. 5% VAT) with an insurer share of
      AED 800 excl. VAT (AED 840 incl.), M pays AED 210 and earns 200 points.
- [ ] An on-account sales order of AED 300 excl. VAT for M earns 300 points on `fulfilled`,
      before its invoice is paid; writing the invoice off reverses the 300.
- [ ] A sales order invoiced on fulfilment earns once (on `fulfilled`), not again on `InvoicePaid`.
- [ ] Returning half of the AED 100 eligible amount reverses 50 points; with a balance of 20 the
      balance becomes −30.
- [ ] Given per-visit with 10 visits per reward, the 10th sale makes the reward available and
      M is told; visits restart at 0.
- [ ] Enrolment records the terms version and does not create a marketing consent.
- [ ] M earns 100 points on a card counter sale of AED 105; a chargeback of the full payment
      reverses 100 points and M is told; winning the chargeback earns 100 again.
- [ ] An invoice of AED 1,000 that earned 1,000 points is reopened after its cheque bounced:
      1,000 points are reversed; paid again, 1,000 are earned with a new lot.
- [ ] A cashier attaching a sale they ring up to their own loyalty account is refused;
      attaching M to a sale completed yesterday is refused.
- [ ] Enrolling a second account with M's mobile number is refused.
- [ ] Given per-visit, 10 visits per reward, returning the 10th sale before the reward is used
      withdraws the reward and leaves 9 visits.

#### Out of scope
Redemption (BL-SAL-23); expiry, adjustments and the liability report (BL-SAL-24); segments
and campaigns (BL-SAL-25); closure, merge, block and programme changes (BL-SAL-26).

---

### BL-SAL-23 — wl-sales: redeem points as a discount line, confirmed by the member

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:portal` `surface:public` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SAL-22 · Base: wl-sales `DiscountEngine` (`SAL.DSC.01`) · Base: wl-sales Sales order lifecycle (`cancel`, `reject`, `cancel_line`, `payment_failed`) · Base: wl-notify `send_otp` (validity and tries, wl-notify §7) · **Blocks:** BL-SAL-24, BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-FNB-17
**Review items:** G4 (SME problem map 35)

#### Goal
At the counter, on the ordering site or in the portal, a member pays part of a sale with
points or a visit reward. The redemption is a discount line on the sale, confirmed by a code
to the member's phone so staff cannot spend other people's points. A return gives the points
back.

#### Surfaces
- **Back office:** at the counter the cashier adds the *Loyalty redemption* line, the member
  reads out the code sent to their mobile, and staff see at once when the code cannot be
  sent or is blocked after 3 wrong tries; the sale goes on without the line.
- **Portal:** a member logged in to the portal redeems points or a visit reward on a sale
  (screen not specified yet).
- **Public:** on the ordering site the member redeems points, confirmed by the code (screen
  not specified yet).
- **System:** points are held for 30 minutes and checked again at completion; the `redeem`
  is written on completion or `confirmed`, and `cancel`, `reject`, `cancel_line` or a return
  give the points back and tell the member.

#### Files to read
- `docs/modules/wl-sales.md` §5.9 steps 3–4; §10 *Loyalty points*; §6 `DiscountEngine`;
  §7 loyalty rows; `docs/modules/wl-notify.md` §6 `send_otp`, §7 row on one-time codes

#### Terms
- **Loyalty redemption line:** a discount line on the sale worth points × point value.
- **Maximum share:** the largest part of a sale (after other discounts) that points may pay.

#### Spec excerpt (authoritative for this issue)

3. At the next sale the member redeems points as a discount line *Loyalty redemption*
   (`DiscountEngine`), within the minimum points and the maximum share of the sale. When
   *confirm redemption by code* is on, the member confirms with a one-time code to their
   mobile (`Notifier::send_otp`; validity and wrong tries are the `wl-notify` defaults, 5
   minutes and 3: after 3 wrong tries the code is voided and redemption is refused for that
   sale, which goes on without it; an expired code may be sent again) or is logged in to the
   portal. A visit reward is redeemed the same way. The redemption is applied after the
   other discounts and is not limited by *Discount stacking*. The points are held for the
   open sale when confirmed (at most the hold minutes, §7; held points do not expire
   meanwhile) and the balance is checked again at completion, so the same points cannot be
   spent twice. A counter sale writes the `redeem` on completion; a sales order writes it on
   `confirmed` (online-paid: when the payment is confirmed). An order whose payment fails or
   times out releases the held points; one that is cancelled or rejected after the `redeem`
   gives the points back (`reverse_redeem`), and `cancel_line` gives back the share of the
   cancelled lines; the member is told.
4. A sales return or credit note on an earning sale reverses the earned points in proportion
   (`reverse_earn`); the balance may go below zero and later earnings offset it, with no cash
   recovery. A return of a sale that redeemed points gives the points back (`reverse_redeem`)
   with their original expiry, or 30 days when that date has passed. In a per-visit programme
   a returned counted sale takes one visit off; a reward made available by it and not used is
   withdrawn. A reward is available for the points validity.

- **Loyalty points:** a redemption is shown as a discount line on the sale. Whether points
  redeemed reduce the taxable value of the discounted supply only (5% VAT on the price after
  the redemption), and how points given away for free and their expiry are treated for VAT
  and in the accounts (liability or provision), is `⚠ open: accountant (A-15)`; until answered, the
  redemption line reduces the taxable amount of the sale it is on and the liability report
  values points at the point value.

| Setting | Default |
|---|---|
| Loyalty programme (`SAL.LOY.01`): earn kind, rate, minimum, exclusions, point value, minimum to redeem, maximum share of a sale | off; when on: per AED, 1 point per AED 1, AED 0, none, AED 0.01 per point, 500 points, 100% (06: 50%) |
| Confirm redemption by a one-time code to the member | on |
| One-time code (`send_otp`, wl-notify §7): validity; wrong tries before the code is voided | 5 minutes; 3 |
| Points held for an open sale before completion | 30 minutes |
| Points given back on a return after their expiry | 30 days |

#### Edge cases decided
- **VAT:** the redemption line reduces the taxable amount of the sale it is on (the default
  in §10); the treatment is `⚠ open: accountant (A-15)`, so keep the VAT effect of the line in one
  place, where an answer can change it.
- **Order of discounts:** other discounts first, then the redemption, capped by the maximum
  share; with *Discount stacking* off, a promo code and points can still be combined (the
  redemption is not limited by *Discount stacking*, §5.9).
- **Lots:** points are taken from the lots with the earliest expiry first.
- **Code:** the wl-notify one-time code defaults (wl-notify §7): valid 5 minutes, 3 wrong
  tries. After the 3rd wrong code the code is voided and redemption is refused for that sale
  (no new code for it); the sale goes on without redemption. A code that expired unused may
  be sent again. When the code cannot be sent, staff see it at once and the sale goes on
  without redemption.
- **Sale cancelled before completion:** nothing is redeemed (the movement is written on
  completion); held points are released.
- **Hold:** points are held when the member confirms, for at most 30 minutes; held points
  cannot be redeemed elsewhere and do not expire while held; the balance is checked again at
  completion and a shortfall removes the line before the sale completes.
- **Sales order:** the `redeem` is written on `confirmed` (online-paid: when the payment is
  confirmed); `payment_failed` releases the hold; `cancel` or `reject` after the `redeem`
  writes `reverse_redeem` for all of it; `cancel_line` writes it for the share of the
  cancelled lines (redemption × cancelled line value ÷ order value after other discounts,
  rounded down to whole points); the member is told.
- **Return:** points given back with their original lot expiry; a lot already past it gets the
  setting's days.

#### Must not
- Must not redeem without the member's code when the setting is on.
- Must not redeem below the minimum points or above the maximum share.
- Must not give cash for points.

#### Done when
- [ ] Given M with 800 points, value AED 0.01, minimum 500, a sale of AED 40 redeems 800 points as
      an AED 8.00 discount line after M enters the code.
- [ ] With 06's maximum share 50%, a sale of AED 10 accepts at most AED 5.00 (500 points).
- [ ] Three wrong codes block redemption for that sale and no new code can be sent for it; the
      sale completes without the redemption line. A code entered 6 minutes after sending is
      refused as expired, and a new code can be sent.
- [ ] M with 400 points cannot redeem (minimum 500).
- [ ] Returning the sale gives the 800 points back; a lot that expired gets 30 days.
- [ ] M confirms 800 points on till 1; redeeming the same points on till 2 within 30 minutes is
      refused; after the sale on till 1 is abandoned, the hold lapses and they can be used.
- [ ] A lot expiring at midnight while held is not expired until the hold ends.
- [ ] An online order paid with 500 points and rejected by the outlet gives the 500 points back
      and M is told; one whose payment fails releases the hold and writes no movement.
- [ ] Cancelling one of two equal lines of a confirmed order that redeemed 500 points gives back
      250.
- [ ] With the default treatment, the AED 8.00 redemption on a VAT-inclusive AED 40 sale lowers
      the VAT of the sale from AED 1.90 to AED 1.52.

#### Out of scope
Expiry and adjustments (BL-SAL-24); the accountant's answer on VAT (open item).

---

### BL-SAL-24 — wl-sales: expire points, adjust by hand with approval and report the points liability (`SAL.RUL.08`, `SAL.ACL.07`)

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SAL-22 · BL-SAL-23 · Base: wl-kernel approvals (`PLT.APR.01`) · Base: wl-notify `notify` (expiry reminder and its give-up) · **Blocks:** BL-SAL-26, BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-FNB-17
**Review items:** G4 (SME problem map 35)

#### Goal
Points expire per earn lot after their validity, with a reminder to the member before. Staff
can correct a balance by hand, but large corrections and closing an account with points need
a manager. The accountant sees the outstanding points and their value.

#### Surfaces
- **Back office:** staff with `loyalty.adjust` adjust a balance with a reason; above 500
  points, and for a staff closure with points, a manager approves under "Needs your action"
  (not the requester); the account shows *reminder not delivered*; the accountant opens the
  points liability report.
- **System:** `SAL.RUL.08` runs daily at 02:00 Asia/Dubai, sends one reminder per member N
  days before expiry and expires the rest of each lot after its date with `PointsExpired`.

#### Files to read
- `docs/modules/wl-sales.md` §4.7; §5.9 steps 5–6; §6 `SAL.RUL.08`, `SAL.ACL.07`; §7 loyalty rows

#### Terms
- **Points liability:** outstanding points × point value, what the business owes members in
  discounts.

#### Spec excerpt (authoritative for this issue)

5. Points expire per earn lot (`SAL.RUL.08`), with a reminder before; they expire even when
   the reminder could not be delivered (the account shows it). Manual adjustments above the
   limit need a manager (`SAL.ACL.07`); a rejected adjustment writes nothing and the requester
   is told, a returned one is edited and submitted again.
6. The points liability report shows outstanding points and their AED value by age and
   expiry month, per brand and entity, for the accountant. Each movement records the entity
   of its sale, so points earned in one entity and redeemed in another are visible; how that
   is booked follows `⚠ open: accountant (A-15)`.

| ID | When | If | Then | Parameters (default) | Generalises |
|---|---|---|---|---|---|
| `SAL.RUL.08` | N days before an earn lot expires; and when its expiry date has ended (a lot can be redeemed through its expiry date) | points of the lot remain | notify the member (category *reminder*, opt-out respected); after the date, `expire` the rest of the lot in the daily run of the next morning (02:00 Asia/Dubai) | N = 30 | — |

| ID | Rule | Pattern | Parameter (default) | Generalises |
|---|---|---|---|---|
| `SAL.ACL.07` | A manual points adjustment above a number of points, or closing a loyalty account that still has points at staff's initiative (not at the member's request or on erasure, §4.7), needs a manager, and the requester cannot approve; a campaign cost entry that takes the campaign over its budget needs a manager other than the requester | `LIM`, `SOD` | 500 points; any budget overrun | — |

| Setting | Default |
|---|---|
| Points validity; expiry reminder (`SAL.RUL.08`) | 12 months; 30 days before |
| Manual points adjustment needing a manager (`SAL.ACL.07`) | above 500 points |

#### Edge cases decided
- **Schedule:** daily at 02:00 Asia/Dubai. A lot can be redeemed through its expiry date; the
  run at 02:00 the next morning expires what is left (lot expiring 30 Nov: usable all of 30
  Nov, expired at 02:00 on 1 Dec). The reminder goes in the run N days before the expiry
  date.
- **Reminder:** one message per member per day, summing all lots expiring in N days; skipped
  for a member who opted out of reminders.
- **Partly used lot:** only the rest expires. **Negative balance:** nothing to expire.
- **Adjustment at or below 500 points:** `loyalty.adjust` with a reason, no approval; above:
  approval, requester cannot approve. Rejected: no movement, requester told; returned: the
  requester edits and submits again. Closing an account with any points at staff's initiative
  needs approval; closure at the member's request, erasure and merge are BL-SAL-26.
- **Reminder not delivered:** the lot still expires on its date; the account shows that the
  reminder was not delivered (give-up reaction `none`).
- **Liability report:** as at a date, by brand, entity, earn age and expiry month; negative
  balances count as zero.

#### Must not
- Must not expire points of a lot before its date, or twice.
- Must not let the requester approve their own adjustment.

#### Done when
- [ ] Given a lot of 300 points expiring 30 Nov with 100 used, on 31 Oct M gets one reminder and
      on 1 Dec at 02:00 200 points expire with `PointsExpired`.
- [ ] Re-running the job on 1 Dec expires nothing more.
- [ ] A +400 adjustment by A is saved; a −900 adjustment by A waits for B's approval; A cannot
      approve.
- [ ] Staff closing M's account with 50 points needs approval and writes an `adjust` of −50.
- [ ] B rejects A's −900 adjustment: no movement is written and A is told.
- [ ] A reminder that gave up does not stop the lot expiring; the account shows *reminder not
      delivered*.
- [ ] The liability report on 31 Dec with 120,000 points at AED 0.01 shows AED 1,200.00.

#### Out of scope
How the liability is booked in the accounts (`⚠ open: accountant (A-15)`).

---

### BL-SAL-25 — wl-sales: define customer segments and send retention campaigns through wl-notify

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SAL-19 · BL-SAL-21 · BL-SAL-22 · BL-NTF-07 · BL-NTF-08 · BL-NTF-09 · **Blocks:** BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-FNB-17
**Review items:** G4 (SME problem map 32, 35)

#### Goal
Staff pick customers by rules (for example no visit for 60 days and spend over AED 500 last
year) and send them a retention campaign. The send goes through wl-notify, so only customers
with marketing consent and no opt-out get it. The campaign report shows who came back and
what they spent.

#### Surfaces
- **Back office:** staff define a segment in the segment editor (05 hides clinical items by
  default) with a preview count, set a retention campaign's segment, template, channel and
  send time, and see recipients, exclusions by reason, *not sent* with its reason, and
  returned members with their spend; exporting members needs the sensitive export
  permission.
- **System:** activating the campaign requests one `Notifier::broadcast`, which evaluates
  the segment at send time; ending the campaign cancels a pending or scheduled broadcast.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Customer segment*, *Campaign*; §4.6 rows `activate`, `end`;
  §5.10; §6 `Segments`; §7 rows on retention and 05 segment rules; §8 `SegmentRule`
- `docs/modules/wl-notify.md` §4.3, §5.3, §6 `NTF.ACL.04`

#### Terms
- **Segment:** saved rules, evaluated each time they are used (not a stored list).
- **Response days:** a member who buys within this many days after the send counts as
  returned.
- `NTF.ACL.01`: marketing only to recipients with marketing consent for that channel.

#### Spec excerpt (authoritative for this issue)

| Entity | Description | Important fields |
|---|---|---|
| Customer segment | A saved set of rules that selects parties (`SAL.LOY.01`) | name, rules (days since last sale, spend in a period, number of sales, items or categories bought, loyalty balance, package credits remaining, birthday month (parties with a date of birth), branch, party group, enrolled in loyalty, and the vertical's `SegmentRule` kinds, §8), combined with *all* or *any*, evaluated when used, last member count; blocked, archived and merged parties are never members |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `activate` | draft | active | `campaigns.manage`, or system (start date reached) | name, code, purpose, marketing channel, owner, start set; retention: segment, approved template, message channel and a send time between start and end set (`SAL.LOY.01`; a send time already past sends at once); the vertical's guards pass (05: `CLN.ACL.13` when the campaign promotes services) | code usable in links and the linked promo code; retention: the broadcast is requested from `wl-notify` for the send time, with the user who set the retention fields as requester (§5.10, `NTF.ACL.04`); event `CampaignActivated` |
| `end` | active | ended | `campaigns.manage`, or system (end date passed, or a vertical guard of `activate` stops passing, e.g. 05: the advertising permit's validity ends) | — | new leads stop taking the campaign after the late-response days (§7); the linked promo code expires with it; retention: a broadcast still `pending_approval` or `scheduled` is cancelled; owner told; event `CampaignEnded` |

### 5.10 Segments and retention campaigns (`SAL.LOY.01`)

1. Staff define a segment by rules, for example *no sale for 60 days and spend over AED 500
   in the last year*, and preview its member count.
2. A retention campaign names the segment, a marketing template and the message channel,
   with a send time. On `activate` it asks `wl-notify` for a marketing broadcast
   (`wl-notify` §5.3): the segment is evaluated at send time, and only members with marketing
   consent for the channel and no opt-out receive it (`NTF.ACL.01`); send window, frequency
   cap and approval follow `NTF.RUL.02` and `NTF.ACL.03`. Marketing broadcasts come only from
   campaigns (`NTF.ACL.04`); the requester is the user who last set the segment, template,
   channel or send time, so that person cannot approve it. The vertical's guards of
   `activate` are checked again when the broadcast starts.
3. When the broadcast is rejected, cancelled or expires without sending (`BroadcastCancelled`),
   the campaign stays `active` and shows *not sent* with the reason; staff fix it and request
   the broadcast again. Ending the campaign cancels a broadcast not yet sending. A campaign has
   at most one completed broadcast.
4. The campaign report shows recipients, delivered, opted out, recipients (messages `sent` or
   later) who bought within the response days after the send (§7) and their sales value,
   against the campaign's cost; excluded counts are shown by reason.

| Setting | Default |
|---|---|
| Retention campaign response days (bought after the send) | 30 |
| Segment rules on clinical services (05) | off: segments in 05 use visit dates, spend and non-clinical items only |

#### Edge cases decided
- **Evaluation time:** the preview count is an estimate; wl-notify evaluates the segment again at
  send time (`members(segment, at)`).
- **Approval size:** the broadcast's estimate at `request` counts only members who would pass
  consent, opt-out, suppression, contact point and blocked-party checks for the channel
  (wl-notify §4.3 `request`), not the raw segment count; above 200 it needs approval
  (`NTF.ACL.03`, BL-NTF-07).
- **Send once:** `reopen` never sends again; a new campaign is needed.
- **Returned member:** any qualifying sale (BL-SAL-22 definition) within the response days;
  value excluding VAT. A member in two campaigns counts in both.
- **05:** rules on clinical items are hidden unless the setting is on; no clinical data in the
  message (`NTF.ACL.02`); `CLN.ACL.13` is a 05 guard on `activate`.
- **Export of segment members:** follows `NTF.ACL.01` (sensitive export permission, audited).
- **Broadcast cancelled, rejected or expired** (`BroadcastCancelled`): the campaign stays
  `active` and shows *not sent* with the reason; staff fix it and request the broadcast again
  (a new broadcast), or end the campaign. At most one completed broadcast per campaign.
- **Requester** (`NTF.ACL.04`): the user who last set the segment, template, channel or send
  time; also when the campaign activates by itself on its start date. That user cannot
  approve the broadcast.
- **Campaign ended before the send:** a broadcast still `pending_approval` or `scheduled` is
  cancelled.
- **Send time:** must be between start and end; one already past at `activate` sends at once.
- **Members:** blocked, archived and merged parties are never members; loyalty accounts that
  are paused count as not enrolled.
- **Rule kinds:** shared kinds plus birthday month (only parties with a date of birth) and
  package credits remaining; verticals add theirs through `SegmentRule` (05 no booking in N
  days, 08 vehicle make and service due, 10 course completed, trial attended, guardians with N
  students).
- **Returned members:** only recipients whose message reached `sent` or later count.

#### Must not
- Must not send messages directly; only through `Notifier::broadcast`.
- Must not show the requester the names of excluded recipients.

#### Done when
- [ ] Given segment S (no sale for 60 days, spend > AED 500 in 12 months) matching 300 parties of
      which 180 have SMS marketing consent, activating retention campaign R requests one
      broadcast with an estimate of 180, which is `scheduled` without approval (limit 200); the
      result shows 180 recipients and 120 excluded *no consent*.
- [ ] Two members of R buy for AED 150 and AED 250 excl. VAT within 30 days: the report shows 2
      returned and AED 400 against R's cost.
- [ ] `reopen` of R after `end` sends nothing.
- [ ] In 05, the segment editor does not offer clinical service items by default.
- [ ] Exporting S's members without the sensitive export permission is refused.
- [ ] User A sets R's segment and template and R activates on its start date: the broadcast
      names A as requester, and A cannot approve it.
- [ ] B rejects R's broadcast: R shows *not sent*; after A changes the template, R requests a
      new broadcast, which completes; a third request is refused.
- [ ] Ending R before its send time cancels its scheduled broadcast.
- [ ] A blocked party matching S is not in S's members.
- [ ] Of 180 recipients, 20 had messages suppressed by the frequency cap; a buyer among those 20
      is not counted as returned.

#### Out of scope
Broadcast approval, send window and frequency cap (BL-NTF-07, BL-NTF-08); 05 permit guard
(05 epic).

---

### BL-SAL-26 — wl-sales: keep loyalty points through account closure, party merge and block, and programme changes

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:m` `blocked:open-item` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SAL-22 · BL-SAL-24 · Base: wl-parties merge and block (`PartiesMerged`, `PartyBlocked`, `PartyUnblocked`, `BlockEffects`) · Base: wl-notify `notify` · **Blocks:** BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-FNB-17
**Review items:** GR-G4-05, GR-G4-06, GR-G4-07, GR-G4-08 (gap review of G4)

#### Goal
A member's points are not lost by accident: a merge of duplicate parties moves the points to
the surviving party, a blocked party's account is paused rather than closed, and a member who
asks to leave or to be erased is never refused. When the client changes or ends its
programme, points already earned keep their value and expiry, and members are told in time.

#### Surfaces
- **Back office:** at the counter earning and redemption for a blocked member are refused
  with the neutral text *on hold*; staff record a member's request to leave (never refused);
  a staff closure with points needs approval; a programme change with an effective date less
  than 30 days ahead is refused on save.
- **Portal:** a member who asks to leave is shown the points that will be forfeited and
  confirms; the request is never refused and needs no approval.
- **System:** `PartiesMerged` moves lots or the account to the survivor, `PartyBlocked` and
  `PartyUnblocked` pause and resume the account, members are told of programme changes, and
  turning the programme off sets a closing date 90 days ahead after which every open lot
  expires.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Loyalty programme*, *Loyalty account*, *Points movement*;
  §4.7; §6 consumed `PartiesMerged`, `PartyBlocked`, `PartyUnblocked`, `SAL.ACL.07`,
  `PointsTransferred`; §7 row on programme notice and closing; §10 *Loyalty terms changes*,
  *Erasure*
- `docs/modules/wl-parties.md` §5.6 (block effects), §4 `merge`

#### Terms
- **Transfer movement:** moves an earn lot from the merged party's account to the survivor's,
  keeping its expiry date.
- **Block effect *pause*:** the agreement stays; service stops until `unblock`
  (wl-parties §5.6).
- **Closing date:** the last day points can be redeemed after the programme is turned off.

#### Spec excerpt (authoritative for this issue)

### 4.7 Loyalty account

Two states, no diagram: `active` (earns and redeems) and `closed` (terminal; the member asked
to leave, staff closed it, the party was merged, or the party's data was erased under
`wl-parties`). A closed account is never reopened; the party enrols again with a zero balance.
Earn lots are consumed oldest expiry first by `redeem` and `expire`.

- **Member asks to leave, or erasure:** never refused. The member is told how many points will
  be forfeited and confirms (staff record the confirmation for a request at the counter); the
  points go with an `adjust` movement without approval. The movements stay for the liability
  history, without the party after an erasure (§10).
- **Staff close it** (for example abuse of the programme): with points left, a manager other
  than the requester approves (`SAL.ACL.07`); the member is told.
- **Party merge** (`PartiesMerged`): the merged party's lots move to the survivor's account in
  the same programme with their expiry dates (`transfer` movements); when the survivor has no
  account, the account itself moves to the survivor. The merged account closes with reason
  *merged into*; no points are forfeited. A blocked party cannot be merged (wl-parties
  `merge` guard), so a pause never moves; when the survivor is blocked, the moved lots and
  the moved account are paused with the survivor's account.
- **Party blocked** (`PartyBlocked`, wl-parties §5.6): the account is registered as agreement
  kind *loyalty account* with block effect *pause*: no earning, no redemption, out of every
  segment, no loyalty messages; lots keep expiring. `PartyUnblocked` clears the pause; points
  not earned while paused are not earned later.
- **Programme changed or turned off:** a change of earn rate, point value, validity or
  exclusions is a new terms version with an effective date at least the notice days ahead
  (§7); members are told then (category *transactional*). New rates apply to sales from that
  date; lots already earned keep their value and expiry. Turning the programme off stops
  earning at once; members are told and may redeem through the closing date (§7); the rest
  of every lot expires in the expiry run after it (`SAL.RUL.08`). Whether a client may
  shorten validity or devalue points already earned is `⚠ open: lawyer (L-39)`; until
  answered, existing lots are never changed (§10).

| `SAL.ACL.07` | A manual points adjustment above a number of points, or closing a loyalty account that still has points at staff's initiative (not at the member's request or on erasure, §4.7), needs a manager, and the requester cannot approve; a campaign cost entry that takes the campaign over its budget needs a manager other than the requester | `LIM`, `SOD` | 500 points; any budget overrun | — |

| Setting | Default |
|---|---|
| Notice before a programme change takes effect; redemption period after the programme is turned off (§4.7) | 30 days; 90 days |

- **Loyalty terms changes:** whether a client may shorten the validity of points already
  earned, devalue them, or end a programme with a closing date under consumer protection law
  is `⚠ open: lawyer (L-39)`; until answered, a change applies only to points earned from
  its effective date and a programme that ends keeps a redemption period (§4.7).
- **Erasure (PDPL):** when a party's data is erased (`wl-parties` §10), its leads, party
  acquisition and points movements keep source, campaign, dates, values and points without
  the party, so acquisition and liability reports do not change; the loyalty account closes
  with reason *erased*.

#### Edge cases decided
- **Merge, both have accounts:** every open lot of the merged account moves with its expiry
  (`transfer` out and in, same points); a negative balance moves too; visits add up; the
  merged account closes *merged into*; no approval needed.
- **Merge, survivor has none:** the account itself is re-pointed to the survivor; no movement.
- **Merge, different programmes** (two brands): accounts stay separate on the survivor, one per
  programme.
- **Merge and block:** the merged party cannot be `blocked` (wl-parties allows `merge` only
  from `active` or `archived`), so no pause moves with it. When the survivor is blocked, its
  account stays paused and the moved lots, or the moved account, are paused with it until
  `PartyUnblocked`; when the survivor is not blocked, nothing is paused.
- **Member asks to leave:** never refused, no approval; the forfeit `adjust` records *member
  request* and who recorded the confirmation.
- **Erasure:** account closed *erased*; movements keep points, values, dates and entity, without
  the party; the liability report is unchanged until the forfeit is written.
- **Staff closure with points:** `SAL.ACL.07` approval; rejected keeps the account `active`.
- **Blocked:** paused; earn and redeem refused at the counter with the neutral text *on hold*;
  out of segments; lots keep expiring; expiry reminders not sent while paused; `unblock` resumes;
  no back-dated earning.
- **Programme change:** saved with an effective date at least 30 days ahead (shorter refused);
  members with an active account are told on save; a sale before the date uses the old rate.
- **Programme off:** earning stops at once; closing date = today + 90 days; members told;
  redemption works through the closing date, and the expiry run at 02:00 the next morning
  expires every open lot with `PointsExpired`.
- **Open legal question:** whether existing points may be shortened or devalued is
  `⚠ open: lawyer (L-39)`; build so that changes never touch existing lots.

#### Must not
- Must not forfeit points on a party merge.
- Must not refuse or delay closure requested by the member.
- Must not change the value or expiry of lots already earned when the programme changes.
- Must not show the real block reason to counter staff.

#### Done when
- [ ] Given P2 (300 points, lot expiring 2027-03-01) merged into P (200 points), P has 500 points
      and the moved lot keeps 2027-03-01; P2's account is closed *merged into*; no `adjust`.
- [ ] Given P2 with an account and P without one, after the merge the account belongs to P.
- [ ] M asks to leave with 450 points: after M confirms, the account closes with an `adjust` of
      −450 and no approval.
- [ ] M's party is blocked: a counter sale earns nothing and redemption is refused with *on
      hold*; after `unblock`, the next sale earns.
- [ ] Changing the earn rate on 1 Nov with effect 15 Nov is refused; with effect 1 Dec, members
      are told on 1 Nov; a sale on 30 Nov earns at the old rate and one on 1 Dec at the new rate.
- [ ] Turning the programme off on 1 Dec sets the closing date to 1 Mar; redemptions on 1 Mar
      still work; at 02:00 on 2 Mar every open lot expires.
- [ ] Given survivor P blocked, merging P2 (300 points) into P moves the 300 points paused:
      redemption is refused *on hold* until P is unblocked.

#### Out of scope
Accounting treatment of forfeited and expired points (`⚠ open: accountant (A-15)`); the
lawyer's answer on terms changes (open question); party merge and block themselves
(wl-parties).

---

### BL-SAL-27 — wl-sales: count a vertical's own enquiries as campaign leads through a kernel lead-capture contract

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-SAL-20; BL-SAL-21 · **Blocks:** None.
**Review items:** G1 (SME problem map 1, 2, 4); follow-up of BL-PRP-27, Done-when point 9 (a)

#### Goal
01 has no wl-sales leads: its enquiries (`PRP.LEA.01`) stay in apps/property and carry the source and
the campaign. The acquisition report counts wl-sales leads only, so a 01 campaign shows no enquiries,
and cost per lead is "—". This issue adds a kernel contract through which a vertical reports its own
lead records (counted, won, lost as duplicate or spam) per campaign and source, and wl-sales counts them
in the report as leads. 01 implements it for its enquiries.

#### Surfaces
- **Back office:** the campaign report of a 01 campaign shows *Enquiries* (the 01 label for leads), won,
  conversion and cost per lead from the enquiries.
- **System:** apps/property registers its enquiries as a lead provider; wl-sales reads every registered
  provider when it computes the report.

#### Files to read
- `docs/modules/wl-sales.md` §2 (note under the table: 01 keeps its enquiries and uses `SAL.CRM.02` on
  top), §4.1 *Vertical mapping* (01 row), §5.8 (points 3 and 5)
- `docs/verticals/01-property.md` §5 `PRP.LEA.01`, §7.2 *Lead sources used*
- `crates/wl-kernel/src/sales.rs`, `apps/property/src/enquiry.rs`, `crates/wl-sales/src/campaign.rs`

#### Terms
- **Lead provider:** a vertical's implementation of the kernel trait `LeadRecords`: for a period, its
  lead records with source, campaign, created date, won date and whether they are excluded (duplicate,
  spam or invalid).
- **Won:** for 01, the enquiry converted and its tenant's first lease activated.

#### Spec excerpt (authoritative for this issue)
wl-sales §2: "01 keeps its enquiries and viewings (`PRP.LEA.01`, apps/property) and uses `SAL.CRM.02` on
top: source and campaign on the enquiry and on the tenant or buyer party."

| Vertical | Conversion event that runs `win` | Notes |
|---|---|---|
| 01 Property | lease activated | No `SAL.CRM.01`: enquiries stay `PRP.LEA.01` in apps/property and carry source and campaign (`SAL.CRM.02`); the activated lease is the first sale of the tenant's party acquisition |

§5.8 point 5: "The acquisition report shows, per source, campaign, marketing channel, brand, branch and
period: leads, leads won, conversion rate (won ÷ leads), new customers …, cost, cost per lead (cost ÷
leads), customer acquisition cost (cost ÷ new customers) and return on ad spend (converted value ÷ cost)
… leads and customers in the period they were created or won. A ratio with a zero divisor shows "—"."

#### Edge cases decided
- **Both kinds present:** a vertical with wl-sales leads and a provider has both counted; 01 has only the
  provider.
- **Excluded enquiries:** an enquiry closed as duplicate or spam is left out, as leads are.
- **Late response:** the campaign on an enquiry is set at capture (the enquiry form's campaign code or
  staff choice); the late-response days rule applies at capture as for leads.
- **Label:** the vertical names its leads (*Enquiries* in 01); the numbers are the same columns.
- **Dependency direction:** apps/property depends on the kernel contract only, not on wl-sales.

#### Must not
- Copy enquiries into wl-sales lead rows.
- Count an enquiry twice when it converts.

#### Done when
- [ ] Given campaign "Bayut October" with cost AED 3,000 and 12 enquiries in October 2026 (one marked
      duplicate), the report shows 11 enquiries and cost per lead AED 272.73.
- [ ] 2 of them converted with leases activated in October: won 2, conversion 18.2%.
- [ ] A campaign without enquiries shows 0 and cost per lead "—".
- [ ] With apps/property off, the report shows wl-sales leads only.

#### Out of scope
The enquiry and viewing pipeline (`PRP.LEA.01` base); campaign channels (BL-SAL-28).

---

### BL-SAL-28 — wl-sales: keep marketing channels as a configurable list seeded per vertical

**Labels:** `module:wl-sales` `area:sme-gap` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-SAL-19 · **Blocks:** BL-CON-18
**Review items:** G1 (SME problem map 1, 2, 4); follow-up of BL-PRP-27, Done-when point 9 (b)

#### Goal
Each vertical names its own campaign channels: 01 listing portals, social media, search ads, email,
signboards and brokers; 02 tender invitations, exhibitions, LinkedIn, email and company profile
mailings. wl-sales has a fixed `MarketingChannel` enum, so a 01 campaign on a listing portal is recorded
as *other online* and 02 cannot seed its list. This issue turns the marketing channel into a table, like
lead sources, seeded with the module's list and extended or renamed per vertical.

#### Surfaces
- **Back office:** the sales settings list the marketing channels (key, name EN/AR, active); the campaign
  form offers the active ones; the report groups by them.
- **System:** wl-sales seeds the module's list; the vertical seed adds its own and deactivates the ones it
  does not use; existing campaigns are migrated to the matching rows.

#### Files to read
- `docs/modules/wl-sales.md` §3 *Campaign* (marketing channel), §5.8 point 5, §7 (lead sources setting,
  the model for this one)
- `docs/verticals/01-property.md` §7.2 *Campaign channels*; `docs/verticals/02-construction.md` §7.2
  *Campaign channels*
- `crates/wl-sales/src/campaign.rs` (`MarketingChannel`), `lead_source.rs` (the table to copy)

#### Terms
- **Marketing channel:** where a campaign runs (not the order *Channel* of wl-sales).

#### Spec excerpt (authoritative for this issue)
wl-sales §3 *Campaign*: "marketing channel (Google, Meta, TikTok, other online, print, radio or outdoor,
event, email, SMS, WhatsApp, partner, other; not the order *Channel*)".

| Setting (01 §7.2) | 01 value |
|---|---|
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | listing portals, social media, search ads, email, signboards, brokers; converted value: the first lease's contract value; … |

#### Edge cases decided
- **Spec gap:** the module lists fixed channels while every vertical lists its own. Decided: the module's
  list is the seed; a vertical adds channels and deactivates others; keys are stable, names editable.
- **01 seed:** `listing_portals`, `social_media`, `search_ads`, `email`, `signboards`, `brokers` added;
  the module's others deactivated, not deleted.
- **Migration:** existing campaigns keep their channel (the enum key becomes the row key).
- **A channel in use:** cannot be deleted, only deactivated; deactivated channels stay on old campaigns
  and in reports.

#### Must not
- Hard-code a vertical's channels in wl-sales.
- Change the channel of an existing campaign during migration.

#### Done when
- [ ] With the 01 seed, the campaign form offers the six 01 channels; a campaign on *listing portals*
      reports under *Listing portals*, not *Other online*.
- [ ] A campaign created before the change with channel `meta` still shows *Meta* after the migration.
- [ ] Deleting a channel used by a campaign is refused; deactivating it hides it from new campaigns only.
- [ ] The 02 seed (when 02 is enabled) can add *tender invitations* without code in wl-sales.

#### Out of scope
The 02 values themselves (BL-CON-18); lead sources (existing table, BL-SAL-20).
