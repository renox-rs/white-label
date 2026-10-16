# Epic: wl-sales — negative flows

> Status: draft · Last updated: 2026-10-15 · Spec version: same commit as this file

This epic builds the negative flows of the Sales order, Sales return and Package machines in
[wl-sales](../../modules/wl-sales.md): payment-first online orders and late payments, duplicate
external orders, order amendment, cancellation of an order or a line after fulfilment started
with an outcome per line, the credit re-check at dispatch and the bounced-cheque hold, partial
fulfilment from refused deliveries, refunds after completion with nothing returned, returns of
recalled goods, a package whose payment is reversed, back-orders for lines without stock and
duplicate customer PO numbers. It closes X-14 and the sales parts of X-03, X-05 and X-09,
and the HIGH items 06-N3-a, 06-N4-b, 07-N2-a, 07-N3-b and 08-N3-a.

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

---

### BL-SAL-01 — wl-sales: refuse a duplicate external order number per channel

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** Base: wl-sales Sales order lifecycle · **Blocks:** BL-FNB-16, BL-FNB-17
**Review items:** X-14 (06 MEDIUM: duplicate external orders)

#### Goal
An aggregator or website may send the same order twice. The order's external order number
must be unique per channel, so a second import is refused and points to the existing order
instead of cooking and charging twice.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · Epic wl-payments: `PaymentReceived` · **Blocks:** BL-SAL-03, BL-SAL-04
**Review items:** X-14, 06-N3-a

#### Goal
An order paid online must not reach the kitchen or warehouse before the gateway confirms the
payment. The order waits in `awaiting_payment`, invisible to fulfilment. When the payment is
confirmed for the full amount, the order continues to acceptance or confirmation; prepaid
orders skip the credit check.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-SAL-02 · Epic wl-payments: `PaymentFailed` · **Blocks:** BL-SAL-04
**Review items:** X-14, 06-N3-a

#### Goal
If the online payment fails, or no confirmation arrives within the payment window (15
minutes), the order is cancelled with reason *payment failed*. The promo code use is released
and the customer is told they can place a new order. Nothing reaches fulfilment.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-SAL-02 · BL-SAL-03 · Epic wl-payments: refunds (system request of a late payment) · **Blocks:** None.
**Review items:** X-14, X-06, 06-N3-a

#### Goal
A payment can be confirmed after the order was cancelled, rejected or already paid. That
money must go back to the customer in full, without staff action, and the order must not
come back to life.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-SAL-06, BL-FNB-05, BL-MFG-03, BL-SAL-18
**Review items:** X-14 (07 MEDIUM: order amendment)

#### Goal
Customers change quantities, prices or lines after an order is placed. Staff amend the order:
a new revision is created, the previous one is kept read-only, reservations and vertical
fulfilment follow, and the credit check runs again when the value rises. After fulfilment
started, a manager must approve. Aggregator orders cannot be amended here.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-SAL-05 · Epic wl-payments: payment links and refunds · Epic wl-invoicing: credit note and extra invoice · **Blocks:** BL-FNB-05
**Review items:** X-14

#### Goal
An amendment changes the money already taken or billed. For a prepaid order, an increase
sends the customer a payment link and a decrease is refunded. A draft invoice is updated; an
issued invoice is never edited: it is corrected by a credit note or an extra invoice.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-SAL-08, BL-SAL-09, BL-STK-26, BL-AUT-18, BL-FNB-02, BL-FNB-04, BL-MFG-03, BL-MFG-14
**Review items:** X-14, 07-N2-a

#### Goal
When the customer or business cancels an order whose kitchen, work order or picking has
started, each started line needs an outcome (return to stock, finish to stock, scrap), with or
without a cancellation fee, and a manager's approval. The event carries the outcomes so stock
and the vertical act on them; any prepayment is refunded less the fee.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** BL-SAL-07 · **Blocks:** BL-SAL-13, BL-STK-26, BL-AUT-15, BL-FNB-05, BL-MFG-03, BL-MFG-07, BL-SAL-17
**Review items:** X-14 (06 MEDIUM: item runs out after acceptance)

#### Goal
An item runs out, or the customer drops one line. Staff cancel the open quantity of that line
only, with a reason; the order continues. If the line's fulfilment started, an outcome and a
manager's approval are needed. The prepaid share is refunded less any fee.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-SAL-07 · **Blocks:** BL-STK-26, BL-AUT-15, BL-FNB-16, BL-MFG-03, BL-SAL-17
**Review items:** X-14

#### Goal
A partially fulfilled order whose rest will never be delivered is closed off with
`cancel_remaining`. Since the change, lines whose fulfilment started need an outcome and an
approval, like `cancel`.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle and credit check · **Blocks:** BL-SAL-11, BL-SAL-12, BL-STK-21, BL-AUT-18
**Review items:** X-14, 07-N3-b

#### Goal
A customer's credit can worsen between order confirmation and dispatch. Before goods on
account leave, the dispatch calls `credit_recheck(order)`. If it fails, the order goes to
`credit_hold`, the goods stay reserved, and the accountant and sales rep are told.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** BL-SAL-10 · Epic wl-kernel: approval request lifecycle · **Blocks:** BL-SAL-12, BL-SAL-17
**Review items:** X-14, X-05

#### Goal
A held order goes back to exactly where it was (`confirmed` or `partially_fulfilled`) when a
credit releaser approves. The releaser cannot be the order's creator.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** BL-SAL-10 · BL-SAL-11 · Epic wl-payments: `ChequeBounced` · **Blocks:** BL-AUT-18, BL-MFG-14
**Review items:** X-05, 08-N3-a

#### Goal
When a customer's cheque bounces, the business must stop giving them more credit at once.
The customer account (and its credit group when configured) goes on hold, every open order on
account not yet dispatched is held, and new orders and counter sales on account are refused
until a credit releaser clears the account.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 07 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · BL-SAL-08 · BL-DLV-02 · BL-STK-22 · BL-STK-24 · BL-STK-25 · **Blocks:** BL-MFG-07
**Review items:** X-11, X-14, 07-N4-a (07 and 08 MEDIUM: returned delivery outcome)

#### Goal
The order counts as fulfilled only what the customer accepted. Refused lines stay open and
are re-delivered or cancelled with `cancel_line`. A delivery returned in full moves the lines'
fulfilment progress back to *not dispatched*, keeps the lines' reservation request and tells
the sales rep and the customer. A delivery reversed, or lost while under review, takes back
the fulfilled quantities (`reverse_fulfilment`); a lost delivery gives the sales rep a task.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-sales Sales return lifecycle · Epic wl-ticketing: complaint tickets · Epic wl-payments: refunds · Epic wl-invoicing: credit note · **Blocks:** BL-FNB-07, BL-FNB-08, BL-FNB-09, BL-FNB-10, BL-FNB-16
**Review items:** X-14, 06-N4-b

#### Goal
Food that arrived cold or wrong, a missing item, or a service not delivered cannot come back
as goods. Staff raise a sales return of kind *no goods* from the completed order or a
complaint ticket, with a complaint category and a responsible party, and resolve it directly
with a credit note or refund. Above AED 50 a manager approves, never the person who recorded it.
When an aggregator refunded its customer itself, the vertical resolves the return *refunded by
the channel* (no money leaves), and reverses it when the aggregator takes the refund back.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** Base: wl-sales Sales return lifecycle · BL-STK-14 · BL-STK-17 · **Blocks:** None.
**Review items:** X-09

#### Goal
When customers bring back goods from a recalled lot, the return is accepted even outside the
return window, linked to the recall, and the quantity joins the recall's stock hold. The
recall, not the return, decides what happens to the goods.

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: wl-sales Package lifecycle · Epic wl-invoicing: `InvoiceReopened` · **Blocks:** BL-CLN-16
**Review items:** X-03, X-06

#### Goal
When the payment for a package is reversed or charged back, its invoice reopens. The package
stays `active` and used credits stay used. The staff who sold it get a task to collect, the
holder is told the balance is due, and an optional setting stops further consumption until
the balance is paid.

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
- [ ] Given an `active` package (10 credits, 4 used) whose invoice is reopened, it stays `active` with 6 left, the seller has a "collect before next use" task, and the holder gets a balance-due message.
- [ ] With the setting off, the next `consume` succeeds.
- [ ] With the setting on, `consume` is refused until the invoice is paid, then succeeds.

#### Out of scope
Payment reversal and invoice reopening (Epic wl-payments, Epic wl-invoicing).

---

### BL-SAL-17 — wl-sales: back-order lines without stock and serve them oldest first

**Labels:** `module:wl-sales` `area:negative-flow` `priority:medium` `size:m` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** Base: wl-sales Sales order lifecycle · BL-SAL-08 (`cancel_line`) · BL-SAL-09 (`cancel_remaining`) · BL-SAL-11 (`release`) · Base: wl-inventory reorder suggestions (`STK.RUL.04`) and goods receipt · **Blocks:** BL-AUT-15
**Review items:** 08 MEDIUM "back-orders"

#### Goal
When a part is not in stock, an order line is back-ordered instead of refused: the available
part is reserved, the rest of the order is picked and fulfilled, the open quantity feeds the
reorder suggestions, and arriving stock is reserved for the oldest back-orders first, with the
customer told. The customer may cancel a back-order. This is the module behaviour; 08 turns it
on (BL-AUT-15).

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

**Labels:** `module:wl-sales` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 07 · **Size:** s
**Depends on:** Base: wl-sales Sales order lifecycle (`submit`) · BL-SAL-05 (`amend`) · **Blocks:** BL-MFG-06
**Review items:** X-14, 07 MEDIUM "duplicate PO upload"

#### Goal
B2B customers email a PO and sometimes also upload it in the portal. When the setting is on,
the same customer PO number must not create two orders for the same customer: `submit` and
`amend` refuse it and point to the existing order. Call-offs against a blanket PO are told
apart by a call-off number. 07 turns the setting on (BL-MFG-06).

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
