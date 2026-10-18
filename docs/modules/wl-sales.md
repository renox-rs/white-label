# wl-sales

> Status: draft · Last updated: 2026-10-16

## 1. Purpose

Everything between "someone is interested" and "goods or services are handed over and
billable": leads and the campaigns that bring them, what the business sells and at what
price, orders from every channel, counter sales, prepaid packages, discounts, loyalty, returns
and credit control. The module owns the
commercial side of a sale. Stock (`wl-inventory`), delivery (`wl-delivery`), invoices
(`wl-invoicing`) and payments (`wl-payments`) do the rest, and each vertical adds its own
fulfilment, for example the F&B kitchen or the workshop job card.

## 2. Capabilities covered

● MVP · ○ Later, per [CAPABILITY-MAP.md](../CAPABILITY-MAP.md) §3.

| ID | Capability | Used by verticals | Scope |
|---|---|---|---|
| `SAL.CRM.01` | Leads and enquiries pipeline | 02 ●, 03 ●, 04 ●, 05 ●, 06 ●, 07 ●, 08 ●, 09 ●, 10 ● | MVP |
| `SAL.CRM.02` | Lead sources and campaigns: cost per campaign, conversion by source and channel, cost per lead, customer acquisition cost and return on ad spend | 01–10 ● | MVP |
| `SAL.CAT.01` | Sellable catalogue and price lists | 05 ●, 06 ●, 07 ●, 08 ●, 10 ● | MVP |
| `SAL.ORD.01` | Sales orders from every channel | 06 ●, 07 ●, 08 ● | MVP |
| `SAL.POS.01` | Counter sale / point of sale with receipt and day closing (absorbs `FNB.CSH.01`) | 06 ● (counter takeaway and daily closing; dine-in Later), 08 ● | MVP |
| `SAL.PKG.01` | Prepaid packages of sessions or units: sell, consume, expire, refund (formerly `EDU.PKG.01`) | 05 ●, 10 ● | MVP |
| `SAL.DSC.01` | Discounts and promo codes (formerly `FNB.PRM.01`) | 06 ●, 10 ● | MVP |
| `SAL.RET.01` | Sales returns with inspection, credit note or refund, restocking (formerly `MFG.RTN.01`) | 07 ○, 08 ● | MVP (08) |
| `SAL.CRD.01` | Customer credit limits, credit check and credit hold | 07 ●, 08 ● | MVP |
| `SAL.LOY.01` | Loyalty and retention: points or visit rewards, customer segments, and retention campaigns sent with marketing consent (formerly `FNB.LOY.01`) | 05 ●, 06 ●, 08 ●, 10 ● | MVP |

`SAL.CRM.01` became MVP in every vertical that lists it (2026-10-16), because `SAL.CRM.02`
needs leads; 02 uses it for tenders and enquiries from main contractors, 06 for corporate
catering and B2B. 01 keeps its enquiries and viewings (`PRP.LEA.01`, apps/property) and uses
`SAL.CRM.02` on top: source and campaign on the enquiry and on the tenant or buyer party.
`SAL.CRM.02` also works without leads: the source and campaign of a party are recorded when
it is created, so a counter or walk-in business still sees acquisition by source. Manual
discounts in 05 and 08 use `SAL.ACL.02` without the `SAL.DSC.01` rule engine.

## 3. Concepts and entities

The customer is always a `Party` from `wl-parties`. This module adds a **customer account**
to the parties that buy on terms.

| Entity | Description | Important fields |
|---|---|---|
| Lead | An enquiry not yet converted | party (may be a new, unverified party), source (from the lead source list), campaign (optional; set from the campaign code of a web form link or promo code, or by staff), branch (the campaign's branch, else the web form's branch, else the owner's branch; its entity is the lead's entity in reports), interest (catalogue items or free text), owner (staff), next action date, lost reason, converted subject, status |
| Catalogue item | Something that can be sold | code, external code (e.g. CPT for a clinic service, OEM number for a part), names EN/AR, type (`product` linked to a stock item, `service`, `bundle`, `package`, `membership`, `course`), brands (multi-brand clients), photo, unit and sales units with fixed conversions (from the wl-inventory item), VAT category, default price, single-unit price (values unused package credits), payment options (upfront, instalment plan through `FIN.BIL.01`), refund policy (per item, applied by the vertical, e.g. EdTech's tiered default), duration and resources (bookable services, wl-scheduling), trial or placement test offered (EdTech), sale restrictions (e.g. *export only*, *regulated*; enforced by the vertical, `MFG.ACL.11`), sellable channels, option groups, active |
| Option group | Choices on an item (F&B modifiers, sizes, add-ons) | item, name, min/max choices, options with price deltas and recipe effect (stock item and quantity, wl-inventory) |
| Price list | A set of prices | name, currency, VAT-inclusive flag, applies to (customer, customer group or tier, channel, branch), validity, lines (item, unit price, minimum quantity) |
| Price tier | A customer price level computed from a rule (08) | name, rule (list − %, cost + %, per item category or grade), special prices per customer, minimum selling price |
| Customer account | Commercial terms of a buying party | party, price list or tier, payment terms (days), credit limit, credit group (optional), grace days, sales rep, customer PO required (none, per month, per order), on-hold flag with reason (e.g. *bounced cheque*) and its open sources (cleared by *Clear account hold*, §4.2), status |
| Credit group | Companies under one owner sharing a combined limit (08 §13 question 4) | the `wl-parties` company group (`member_of_group`, `PTY.REL.01`), combined limit, member accounts |
| Channel | Where orders come from | code (counter, phone, portal, website, aggregator, sales rep; dine-in Later), brand, integration (manual quick entry, or Deliverect middleware for aggregators), commission rate (for margin reports), requires acceptance, credit check on/off, default price list |
| Sales order | A customer's order | number, revision (starts at 1; earlier revisions kept read-only), issuing entity, party, channel, branch, lines (item, options, quantity, unit price, discount, VAT category, quantity fulfilled, quantity invoiced, quantity cancelled, quantity back-ordered, cancellation reason and outcome), delivery method (collect, deliver, none), payment (prepaid online, cash on delivery, on account), payment status (not required, pending, received, failed), amount refunded, requested date, external order number (e.g. aggregator order number; unique per channel), customer PO reference, call-off number and file, source (quotation, reorder of a previous order, uploaded PO), state before credit hold, status |
| Till session | One cash drawer for one shift or day | branch or outlet, register, opened by, opening float, expected totals by payment method, counted totals, variance, status |
| Counter sale | A POS ticket paid on the spot, or charged to a customer account | till session, lines, payments, on account (customer account; credit check runs), receipt number (simplified or full tax invoice), party (optional; required on account), status |
| Package | Prepaid credits bought by a party | party (holder) and payer, catalogue item, credits total and used, unit (session, hour, item), price paid, valid from/to, extension history, status |
| Package movement | A change to the credit balance | package, kind (consume, reverse, extend, adjust, refund), quantity, source `Subject` (booking, attendance, job card), user, time |
| Discount rule | A reusable discount | name, kind (percentage, amount, fixed price), applies to (items, categories, whole order), eligibility (party group, related parties, date window, minimum spend, promo code), stacking allowed, usage limit per code and per party, validity |
| Promo code | A code that activates a discount rule | code, rule, uses, max uses, valid to |
| Sales return | Goods coming back, or a refund after completion with nothing to return | number, kind (*goods*, or *no goods* for food and services), original sale (order or counter sale) and lines, reason, complaint category, responsible party, linked ticket, recall (when the goods are under a `wl-inventory` recall), quantity, condition, lot, resolution (credit note, refund, replacement, refunded by the channel), restocking fee, status |
| Lead source | Where a lead or a customer came from (`SAL.CRM.02`) | code, name EN/AR, active. Seeded: referral, walk-in, website, social, paid ads, aggregator, event, partner, other; the client may add more but not delete a source in use |
| Party acquisition | How a party became a customer, kept per party (`SAL.CRM.02`) | party, source, campaign (optional), set from (lead, staff, web form, promo code), first sale (`Subject`: the first confirmed sales order, completed counter sale, issued invoice, or the vertical's conversion record such as a lease, enrolment or engagement), first sale date, first sale value (excluding VAT, net of credit notes), branch (of the first sale, else of the converting lead; its entity is the acquisition's entity in reports), acquired at, change history (source or campaign changed, with reason and approver, `SAL.ACL.09`), superseded by (after a party merge) |
| Campaign | A paid or unpaid marketing action with a period (`SAL.CRM.02`), or a message to a segment (`SAL.LOY.01`) | name, code (unique; used in web form links and linked promo codes), purpose (`acquisition`, `retention`), marketing channel (Google, Meta, TikTok, other online, print, radio or outdoor, event, email, SMS, WhatsApp, partner, other; not the order *Channel*), lead source it implies (e.g. *paid ads*), brand, entity (default: the brand's entity), branch (optional), owner (staff; told of costs, approvals and the end), start, end, budget (AED), promo code (optional, `SAL.DSC.01`), retention only: segment, notification template, message channel, send time (between start and end), retention fields set by (the requester of the broadcast, `NTF.ACL.04`), broadcast and its status, status |
| Campaign cost entry | Money spent on a campaign | campaign, date, amount in AED excluding recoverable VAT (a bill in another currency at the bill's exchange rate), description, supplier bill line (optional, `wl-invoicing`), entered by, approved by, status (`pending_approval`, `approved`, `rejected`, `voided`), reason (rejection or void); an entry linked to a bill takes its amount from the bill line and counts only once the bill is `approved` |
| Loyalty programme | The earn and redeem terms of the client (`SAL.LOY.01`) | name, brand (one programme per brand at most), earn kind (`per_aed`: points per AED spent; `per_visit`: one visit per qualifying sale), earn rate (points per AED, or visits per reward), minimum qualifying amount, excluded items or categories, point value (AED per point), minimum points to redeem, maximum share of a sale paid with points, visit reward (fixed discount or a free catalogue item), points validity (months after earning), terms version and terms text (EN/AR, `wl-documents`), change effective date, closing date (when turned off), active |
| Loyalty account | A party's membership; one per programme and party, and one per programme and mobile number | programme, party, accepted by (the party, or a guardian for a minor, `PTY.REL.01`), mobile number for codes, enrolled at, enrolled by (staff, portal, counter), terms version accepted, points balance, points held (redemption in progress), visits since the last reward, reward available until, paused (party blocked), status (`active`, `closed`), closed reason (member request, staff, merged into, erased) |
| Points movement | A change to a loyalty account | account, kind (`earn`, `redeem`, `reverse_earn`, `reverse_redeem`, `expire`, `adjust`, `transfer` (party merge)), points (or visits), AED value, entity of the sale, source `Subject` (counter sale, sales order, invoice, sales return, credit note, payment reversal, write-off), earn lot expiry date (earn), lot consumed (redeem, expire), reason and approval (adjust), user, time |
| Customer segment | A saved set of rules that selects parties (`SAL.LOY.01`) | name, rules (days since last sale, spend in a period, number of sales, items or categories bought, loyalty balance, package credits remaining, birthday month (parties with a date of birth), branch, party group, enrolled in loyalty, and the vertical's `SegmentRule` kinds, §8), combined with *all* or *any*, evaluated when used, last member count; blocked, archived and merged parties are never members |
| Credit check | Result of checking an order | order, exposure (open invoices + uncleared post-dated cheques + confirmed orders not yet invoiced), overdue beyond grace, order value, customer limit, group exposure and combined limit, result, time |

Vertical fields (for example a vehicle on an auto parts order, or a recipient's Makani number)
are custom fields or vertical tables that reference the sales order as a `Subject`.

The source and campaign of a party live in its *party acquisition* record here, not on the
`wl-parties` record, so that a party bought by several brands of the client keeps one record
per client and the attribution stays in the module that reports on it. Source is required
when a party gets its first sale (staff pick it; a web form or a promo code sets it); a party
created before `SAL.CRM.02` was turned on is shown as *unknown*. When two parties are merged
(`PartiesMerged`), the survivor keeps the acquisition acquired first; the other is kept as
history (*superseded by*), so the customer is counted once.

## 4. Lifecycles

The machines below are canonical. They replace the definitions in the vertical documents
listed in each *Vertical mapping* table.

### 4.1 Lead

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `new` | neutral | no | Recorded, not yet contacted |
| `contacted` | info | no | First contact made |
| `qualified` | info | no | Need, budget or level and timing known |
| `won` | success | no | Converted to the vertical's conversion record; returns to `qualified` only when that record is voided |
| `lost` | neutral | no | Not converting, with a reason; can be reopened |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `contact` | new | contacted | `leads.manage` | contact note recorded | — |
| `qualify` | contacted | qualified | `leads.manage` | qualification fields set (configurable per vertical) | — |
| `win` | contacted, qualified | won | system (conversion event) | — | converted `Subject` and source recorded; the party acquisition takes the lead's source and campaign when the party has none yet; event `LeadWon` |
| `lose` | new, contacted, qualified | lost | `leads.manage` | reason from the configured list | event `LeadLost` |
| `reopen` | lost | contacted | `leads.manage` | — | — |
| `revert_win` | won | qualified | system (the converted `Subject` is voided as entered in error) | — | converted `Subject` cleared; a party acquisition set from this lead with no other first sale is cleared (`CustomerAcquisitionChanged`); owner told; event `LeadWinReverted` |

```mermaid
stateDiagram-v2
  [*] --> new
  new --> contacted: contact
  contacted --> qualified: qualify
  contacted --> won: win
  qualified --> won: win
  new --> lost: lose
  contacted --> lost: lose
  qualified --> lost: lose
  lost --> contacted: reopen
  won --> qualified: revert_win
```

A conversion record that is cancelled for a business reason after the lead was won (the
customer backs out of an accepted quotation, a lease ends early) leaves the lead `won`: it
did convert. Only a `void` (entered in error, wl-kernel §5.12) runs `revert_win`.

**Vertical mapping**

| Vertical | Conversion event that runs `win` | Notes |
|---|---|---|
| 10 EdTech | enrolment activated | Source of the machine ([10-edtech.md](../verticals/10-edtech.md) §7.1). Trial booking is a `qualified` lead with a booking |
| 05 Clinics | first booking completed | Source required; campaign required when the source is *paid ads* or *social* |
| 04 Accounting | engagement letter signed | — |
| 02 Construction | quotation or tender accepted (`FIN.QUO.01`), or the subcontract signed | Leads are tenders and enquiries from main contractors |
| 03, 07, 08, 09 | quotation accepted (`FIN.QUO.01`) | — |
| 06 F&B | quotation accepted (`FIN.QUO.01`) or first B2B order confirmed | Corporate catering and B2B only; consumers are counted through their party acquisition, not leads |
| 01 Property | lease activated | No `SAL.CRM.01`: enquiries stay `PRP.LEA.01` in apps/property and carry source and campaign (`SAL.CRM.02`); the activated lease is the first sale of the tenant's party acquisition |

### 4.2 Sales order

Merges the machines in [07-manufacturing.md](../verticals/07-manufacturing.md) §7.1 and
[08-auto.md](../verticals/08-auto.md) §7.1. Picking, packing and delivery are **not** order
states: they belong to the dispatch note (`wl-inventory`) and the delivery (`wl-delivery`).
The order shows them as a derived *fulfilment progress*, so there is one source of truth per
fact.

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Being entered; editable |
| `awaiting_payment` | neutral | no | Placed with online prepayment; waiting for the gateway; not visible to fulfilment |
| `pending_acceptance` | neutral | no | Placed through a channel that needs the seller to accept (website, portal) |
| `credit_hold` | warning | no | Blocked by the credit check at submit, by the re-check at dispatch, or by a bounced cheque (`SAL.CRD.01`) |
| `confirmed` | info | no | Accepted; stock reserved; fulfilment can start |
| `partially_fulfilled` | info | no | Some quantities delivered or collected |
| `fulfilled` | success | no | All quantities delivered or collected, or the rest cancelled |
| `closed` | success | yes | Everything fulfilled is invoiced or receipted |
| `rejected` | neutral | yes | Not accepted by the seller; any prepayment refunded |
| `cancelled` | neutral | yes | Cancelled before anything was fulfilled, or the online payment failed |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | awaiting_payment, pending_acceptance, credit_hold, confirmed | `sales_orders.create`, or the external party through the portal | lines priced; manual prices and discounts within limits (`SAL.ACL.02`) or approved; the external order number (compared trimmed and case-insensitive) is not already used on this channel (a duplicate import is refused and points to the existing order); when *Unique customer PO number per customer* is on, the customer PO reference with its call-off number is not already used by another order of the same customer, cancelled orders included (refused, pointing to the existing order); payment on account: the customer account is not on hold (`SAL.RUL.05`; refused, the order stays `draft`) | target: `awaiting_payment` if paid online and not yet confirmed; else `pending_acceptance` if the channel needs acceptance; otherwise credit check → `credit_hold` or `confirmed`; on `confirmed`: stock reserved (when back-orders are allowed, quantity without available stock is back-ordered instead of refused), confirmation sent, event `SalesOrderConfirmed` |
| `confirm_payment` | awaiting_payment | pending_acceptance, confirmed | system (`PaymentReceived` from `wl-payments` for the order's payment) | amount equals the order total | as `submit` from acceptance on; prepaid orders skip the credit check |
| `payment_failed` | awaiting_payment | cancelled | system (`PaymentFailed`, including the gateway timeout) | — | reason *payment failed*; promo code use released; the customer is told and may place a new order; nothing reaches fulfilment; event `SalesOrderCancelled` |
| `accept` | pending_acceptance | credit_hold, confirmed | `sales_orders.accept` | channel open; items available | as `submit` from the credit check on |
| `reject` | pending_acceptance | rejected | `sales_orders.accept` or system (acceptance timeout) | reason given | prepayment refunded; party notified; a payment confirmed later is refunded too (`SAL.RUL.06`) |
| `hold` | confirmed, partially_fulfilled | credit_hold | system (re-check at dispatch fails, or `SAL.RUL.05` on a bounced cheque) | order on account | state before the hold kept; dispatch blocked, reservations kept; event `SalesOrderCreditHeld` |
| `release` | credit_hold | confirmed, partially_fulfilled | `credit.release` | approval (`PLT.APR.01`); releaser ≠ creator (`SAL.ACL.03`) | back to the state before the hold (`confirmed` from submit); open back-orders of the order reserved from available stock at the branch (Back-orders below); event `SalesOrderConfirmed` when first confirmed |
| `amend` | pending_acceptance, credit_hold, confirmed, partially_fulfilled | same state, or credit_hold | `sales_orders.amend` | reason; no line below its fulfilled quantity; a line whose fulfilment started is not reduced by `amend` (it goes through `cancel_line`, which asks for an outcome); prices and discounts within `SAL.ACL.02`; channel allows amendments (aggregator orders are changed on the aggregator); approval when fulfilment started (`SAL.ACL.05`); a changed customer PO reference or call-off number passes the same uniqueness check as `submit` | revision + 1 and the previous revision kept; reservations and vertical fulfilment adjusted; credit check re-run when the value rises (fails → `credit_hold`); prepaid: payment link for an increase, refund of a decrease (`wl-payments`); a draft invoice is updated, an issued one is corrected by credit note or an extra invoice (`wl-invoicing`); revised confirmation sent; event `SalesOrderAmended` |
| `cancel_line` | confirmed, partially_fulfilled | same state, fulfilled | `sales_orders.cancel` | reason; open quantity only; at least one line stays open or fulfilled (otherwise `cancel`); outcome per line and approval when fulfilment of the line started (`SAL.ACL.05`) | quantity cancelled; outcome applied (below); prepaid share refunded less any fee; an invoice already issued for the line is corrected by a credit note for the cancelled value (`wl-invoicing`); `fulfilled` when nothing is open; event `SalesOrderLineCancelled` |
| `fulfil` | confirmed, partially_fulfilled | partially_fulfilled, fulfilled | system (`DispatchDelivered` for an order shipped on a dispatch note; `DeliveryDelivered` only for an order delivered without a dispatch note; collection recorded; or vertical fulfilment completed) | dispatch of an order on account passes `credit_recheck`; not yet fulfilled from the same dispatch or delivery (idempotent per source) | accepted quantities fulfilled (refused delivery lines are not); `fulfilled` when nothing is open; invoice drafted when the client invoices on delivery |
| `reverse_fulfilment` | partially_fulfilled, fulfilled | confirmed, partially_fulfilled | system (`DispatchReversed`, or `DispatchLost` of a `delivered` dispatch; for an order delivered without a dispatch note, `DeliveryReversed`, or `DeliveryLost` from `under_review`) | the quantities were fulfilled from that dispatch or delivery | quantity fulfilled reduced by them; `confirmed` when nothing stays fulfilled, otherwise `partially_fulfilled`; a draft invoice is updated, an issued one is left as it is and the accountant gets a task (credit note or keep for re-delivery, `wl-invoicing`); sales rep told; event `SalesOrderFulfilmentReversed` |
| `cancel_remaining` | partially_fulfilled | fulfilled | `sales_orders.cancel` | reason given; outcome per line and approval when fulfilment of a line started (`SAL.ACL.05`) | open lines cancelled with their outcomes; reservations released; one event `SalesOrderLineCancelled` per cancelled line |
| `close` | fulfilled | closed | system | all fulfilled quantities invoiced or receipted | event `SalesOrderClosed` |
| `cancel` | draft, pending_acceptance, credit_hold, confirmed | cancelled | `sales_orders.cancel` | nothing fulfilled; paid order above a value, or after vertical fulfilment started, needs approval (`SAL.ACL.01`); outcome per line when fulfilment started | reservations released; outcomes applied; refund if paid, less any cancellation fee; vertical fulfilment cancelled (event `SalesOrderCancelled` with the outcome per line) |

```mermaid
stateDiagram-v2
  [*] --> draft
  draft --> awaiting_payment: submit
  draft --> pending_acceptance: submit
  draft --> credit_hold: submit
  draft --> confirmed: submit
  awaiting_payment --> pending_acceptance: confirm_payment
  awaiting_payment --> confirmed: confirm_payment
  awaiting_payment --> cancelled: payment_failed
  pending_acceptance --> credit_hold: accept
  pending_acceptance --> confirmed: accept
  pending_acceptance --> rejected: reject
  confirmed --> credit_hold: hold
  partially_fulfilled --> credit_hold: hold
  credit_hold --> confirmed: release
  credit_hold --> partially_fulfilled: release
  pending_acceptance --> pending_acceptance: amend
  credit_hold --> credit_hold: amend
  confirmed --> confirmed: amend
  confirmed --> credit_hold: amend
  partially_fulfilled --> partially_fulfilled: amend
  partially_fulfilled --> credit_hold: amend
  confirmed --> confirmed: cancel_line
  confirmed --> fulfilled: cancel_line
  partially_fulfilled --> partially_fulfilled: cancel_line
  partially_fulfilled --> fulfilled: cancel_line
  confirmed --> partially_fulfilled: fulfil
  confirmed --> fulfilled: fulfil
  partially_fulfilled --> fulfilled: fulfil
  partially_fulfilled --> confirmed: reverse_fulfilment
  partially_fulfilled --> partially_fulfilled: reverse_fulfilment
  fulfilled --> confirmed: reverse_fulfilment
  fulfilled --> partially_fulfilled: reverse_fulfilment
  partially_fulfilled --> fulfilled: cancel_remaining
  fulfilled --> closed: close
  draft --> cancelled: cancel
  pending_acceptance --> cancelled: cancel
  credit_hold --> cancelled: cancel
  confirmed --> cancelled: cancel
  closed --> [*]
  rejected --> [*]
  cancelled --> [*]
```

**Cancelling after fulfilment started.** Fulfilment of a line has started when its goods are
picked, a work order or production batch for it has started (07), or the kitchen has started
it (06). `cancel`, `cancel_line` and `cancel_remaining` then need an outcome per line and an
approval. The event carries the outcome and the owning module acts on it:

| Outcome | Stock | Money |
|---|---|---|
| *Return to stock* | picked goods put back and the reservation released (`wl-inventory`) | prepaid share refunded |
| *Finish to stock* | the work order runs to the end and its output goes to free stock (vertical) | prepaid share refunded, less any fee |
| *Scrap* | work in progress or prepared goods written off as scrap or wastage (`wl-inventory`, `STK.ACL.02`) | cost recorded as a loss; prepaid share refunded, less any fee |
| *Cancellation fee* (with any of the above) | — | a fee line invoiced, or kept from the prepayment with a tax document (`wl-invoicing`); the manager may waive it |

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

**Customer PO number.** When *Unique customer PO number per customer* is on, `submit` and
`amend` refuse a customer PO reference that another order of the same customer already uses,
compared trimmed and case-insensitive with its call-off number (a call-off against a blanket
PO carries its own call-off number). Cancelled orders still count, so the customer sends a new
number or a call-off number. The same number for different customers is allowed; orders
without a customer PO reference are not checked. A portal upload refused for this reason
shows the buyer only the existing order's number and status.

**Credit re-check at dispatch.** For orders on account, the dispatch note `dispatch` in
`wl-inventory` (and any vertical step that hands goods over) calls the guard
`credit_recheck(order)` through `CreditChecker`. It recomputes the exposure, overdue balance
and the account's on-hold flag as at submit. A failure runs `hold`: the goods stay reserved,
the accountant and sales rep are told (`SAL.RUL.01`), and only `release` lets the dispatch go.
After a `release`, the re-check passes while the release approval is later than the last
change to the account's exposure; once the exposure changes, it checks again.

**Account hold.** `SAL.RUL.05` sets the account's on-hold flag with its source (the bounced
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

**Fulfilment from deliveries.** An order shipped on a dispatch note (`wl-inventory`) follows
the dispatch only: `DispatchDelivered` runs `fulfil`, and the `Delivery*` events of the same
delivery are ignored, so one hand-over is never counted twice. Only an order handed to
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

**Online payment.** An order paid online is placed (visible to the outlet or fulfilment) only
after the gateway confirms the payment. A failed or timed-out payment cancels it. A payment
that is confirmed after the order was cancelled or rejected, or whose amount differs from the
order total while the order is `awaiting_payment`, is refunded in full without staff action
(`SAL.RUL.06`).

**Amendment of a prepaid order.** An increase sends the customer a payment link for the
difference; the added quantity is reserved but not fulfilled until the link is paid. When the
link expires unpaid (after the online payment window for website orders, 24 hours for other
channels), the order owner gets a task (the sales rep, or the outlet manager for website and
portal orders without a sales rep): resend the link, or amend the order back. The revision
stays until then.

**Vertical mapping**

| Vertical | Their state or step | Canonical equivalent |
|---|---|---|
| 07 Manufacturing | `on_credit_hold` | `credit_hold` |
| 07 | `partially_delivered`, `delivered` | `partially_fulfilled`, `fulfilled` (dispatch `confirm_delivery` runs `fulfil`) |
| 07 | `MFG.RUL.01` (work order for shortfall on confirm) | listens to `SalesOrderConfirmed`; unchanged |
| 07 | portal reorder or PO upload, confirmed by sales (`POR.SLF.01`) | `submit` on the portal channel → `pending_acceptance`; sales `accept`. Catalogue cart ordering in the portal is Later |
| 08 Auto | `confirm` | `submit` (channel without acceptance) |
| 08 | `picking`, `ready` | fulfilment progress from the dispatch (`wl-inventory`, `STK.DSP.01`): `ready` is the dispatch `picked`; `SAL.RUL.04` reminds the customer when a *collect* order stays there |
| 08 | `out_for_delivery` | fulfilment progress from the delivery (`wl-delivery`) |
| 08 | `return_delivery` (out for delivery → `confirmed` or `partially_delivered` on `DeliveryReturned`) | no order transition, on `DispatchReturned` (the dispatch `return` runs on `DeliveryReturned`): the order stays `confirmed` or `partially_fulfilled` and its fulfilment progress goes back to *not dispatched* for the returned lines; goods back to quarantine through the dispatch (`wl-inventory`); the lines keep their reservation request and the new dispatch reserves again; sales rep and customer told; the lines are re-delivered (new dispatch) or cancelled (`cancel_line`) |
| 08 | `deliver` with proof of delivery | `fulfil` on `DispatchDelivered` (the dispatch `confirm_delivery` runs on `DeliveryDelivered`); a delivery reversal or loss runs `reverse_fulfilment` |
| 08 | `delivered` → `invoice` → `closed` | `fulfilled` (invoice drafted on fulfil) → `close` |
| 08 | counter collection (`deliver` by counter staff) | `fulfil` from a collection record with signature (`DOC.SIG.01`) |
| 06 F&B | direct order `placed` | `pending_acceptance` (website channel) |
| 06 | aggregator order starts at `accepted` | `submit` on the aggregator channel, no acceptance, no credit check → `confirmed` |
| 06 | kitchen fulfilment (`accepted → preparing → ready → dispatched → completed`) | stays vertical-specific; starts on `SalesOrderConfirmed`; its `complete` runs `fulfil` |
| 06 | `rejected` | `rejected` |
| 06 | direct order with cash on delivery | `submit` with payment *cash on delivery*; the rider's cash goes to cash custody in `wl-payments` |
| 06 | dine-in order starting in `accepted` (Later) | `submit` on the dine-in channel → `confirmed`; completes when the table session is settled |

### 4.3 Till session and counter sale

Generalises the F&B daily closing (`FNB.CSH.01`) into the POS. Any business with a cash
drawer needs it, auto parts counters included.

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `open` | info | no | Drawer in use; counter sales recorded against it |
| `counting` | warning | no | Sales stopped; cash and card totals being counted |
| `closed` | success | no | Counted and submitted; locked |
| `reviewed` | success | yes | Checked by a manager or accountant |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `start_count` | open | counting | `pos.close_till` | — | no new sales on this till |
| `close` | counting | closed | `pos.close_till` | counted totals per payment method entered | variance calculated; Z report PDF (`DOC.GEN.01`); event `TillSessionClosed`; variance above the threshold needs approval (`PLT.APR.01`) |
| `reopen` | closed | open | `pos.reopen_till` | — | reason logged (as `FNB.ACL.05`) |
| `review` | closed | reviewed | `pos.review` | — | — |

```mermaid
stateDiagram-v2
  [*] --> open
  open --> counting: start_count
  counting --> closed: close
  closed --> open: reopen
  closed --> reviewed: review
  reviewed --> [*]
```

**Vertical mapping**

| Vertical | Their state or step | Canonical equivalent |
|---|---|---|
| 06 F&B (daily closing, formerly `FNB.CSH.01`) | `open` (trading day) | `open`; one till session per outlet and day, or per register |
| 06 | `submit` (closing checklist completed, cash counted) → `submitted` | `start_count` + `close` → `closed`; the closing checklist (`WRK.CHK.01`) is a guard added by the vertical; locked (`FNB.ACL.05`) |
| 06 | `approve` → `approved` | `review` → `reviewed` |
| 06 | `reopen` → `reopened` (area manager) → `submit` again | `reopen` → `open`, then `start_count` and `close` again |
| 08 Auto | counter till per branch and shift | unchanged |

A **counter sale** has two states: `open` (cart) and `completed` (paid or charged to the
customer account, receipt or tax invoice issued). An abandoned cart is deleted. A completed
sale is never edited or voided; it is reversed by a sales return with a credit note
(`SAL.ACL.04`).

### 4.4 Package

Merges the clinics *Package* ([05-clinics.md](../verticals/05-clinics.md) §7.1) with EdTech's
session package balance ([10-edtech.md](../verticals/10-edtech.md)).

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `pending_payment` | neutral | no | Sold, not yet paid |
| `active` | info | no | Paid (or first instalment paid); credits usable |
| `completed` | success | yes | All credits used |
| `expired` | warning | no | Validity ended with credits left; can be extended |
| `refunded` | neutral | yes | Unused balance refunded |
| `cancelled` | neutral | yes | Never paid |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `activate` | pending_payment | active | system (payment recorded) | paid in full, or first instalment paid if allowed | validity dates set; event `PackageActivated` |
| `consume` | active | active, completed | system (source event: booking completed, attendance submitted, job card closed) | credits remaining ≥ quantity; not expired (`CLN.ACL.10`); when *suspend consumption while unpaid* is on (§7), no unpaid balance reopened on the package's invoice | movement recorded with its source; `completed` at zero; event `PackageConsumed` |
| `reverse` | active, completed | active | `packages.adjust` or system (source cancelled) | — | credit returned with reason |
| `extend` | active, expired | active | `packages.extend` | new end date; limits per configuration | event `PackageExtended` |
| `expire` | active | expired | system | end date passed | unused value reported (deferred revenue) |
| `refund` | active, expired | refunded | `packages.refund` | refunder ≠ recorder (`FIN.ACL.04`); refund from `expired` only if enabled | unused credits valued by the item's refund policy (default: price paid pro rata; 10: single-unit price without the package discount, minus fees); refund via `wl-payments`; credit note via `wl-invoicing` |
| `cancel` | pending_payment | cancelled | `packages.sell` | nothing paid | — |

```mermaid
stateDiagram-v2
  [*] --> pending_payment
  pending_payment --> active: activate
  pending_payment --> cancelled: cancel
  active --> active: consume
  active --> completed: consume
  active --> active: reverse
  completed --> active: reverse
  active --> active: extend
  expired --> active: extend
  active --> expired: expire
  active --> refunded: refund
  expired --> refunded: refund
  completed --> [*]
  refunded --> [*]
  cancelled --> [*]
```

**Payment reversed after activation.** When the payment for a package is reversed or charged
back, wl-invoicing reopens its invoice (`InvoiceReopened`). The package does not change
state: credits already used stay used and the package stays `active`. The staff who sold it
(05: front desk; 10: counsellor) get a task to collect before the next use, and the holder is
told the balance is due. A setting can suspend further `consume` until the reopened balance
is paid (off by default); the vertical then decides whether a booked session goes ahead.
Refused consumptions are not replayed when the balance is paid: the seller's task lists them
for a manual `consume`.

**Vertical mapping**

| Vertical | Consumed by | Notes |
|---|---|---|
| 05 Clinics | booking `completed` | Expired packages could not be refunded in the clinic machine. Refund from `expired` is now allowed, but off by default (configuration) |
| 10 EdTech | session attendance submitted | EdTech's *Package balance* entity becomes this Package. An enrolment freeze runs `extend` |

### 4.5 Sales return

From [08-auto.md](../verticals/08-auto.md) §7.1, with manufacturing's lot link and complaint.

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `requested` | neutral | no | Customer asked to return goods |
| `received` | info | no | Goods back in a quarantine location; being inspected |
| `resolved` | success | no | Credited, refunded or replaced; stock restored or written off. Final, except that a refund made by the channel can still be reversed |
| `reversed` | neutral | yes | A refund made by the channel was taken back by the channel or recorded in error; the order's amount refunded is reduced again |
| `rejected` | neutral | yes | Not accepted; goods handed back |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `receive` | requested | received | `returns.handle` | original sale linked; within the return window or approved (a return of recalled goods, including a non-lot item sold in an open recall's trace window, is always accepted) | stock received into quarantine (`wl-inventory`); a lot under an open recall links the return to the recall and adds the qty to its stock hold; an item not tracked by lot is treated the same when it is in an open recall by item and received date range and the original sale is dated in the recall's trace window |
| `resolve` | received; requested (kind *no goods*) | resolved | `returns.handle`, or system (vertical: 06 payout statement `match`, resolution *refunded by the channel*) | inspection recorded (kind *goods*); resolution chosen; restocking fee set; value above threshold approved; kind *no goods*: complaint category, responsible party and approval (`SAL.ACL.06`); resolution *refunded by the channel* only for kind *no goods* on an order of an aggregator channel | credit note (`wl-invoicing`) or refund (`wl-payments`) or replacement order at zero price, or, for *refunded by the channel*, no credit note and no refund (the channel refunded the customer and deducts it from its payout); stock restored or written off (kind *goods*); order's amount refunded updated; event `ReturnResolved` |
| `reject` | requested, received | rejected | `returns.handle` | reason given | goods released back to the customer |
| `reverse` | resolved | reversed | system (vertical: 06 payout statement `void`, or `resolve` of a dispute in which the aggregator takes the refund back) or `returns.handle` | resolution *refunded by the channel*; reason given | the order's amount refunded reduced by the return's value; refund reports exclude the return; no money and no document moves; event `ReturnReversed` |

```mermaid
stateDiagram-v2
  [*] --> requested
  requested --> received: receive
  received --> resolved: resolve
  requested --> resolved: resolve
  requested --> rejected: reject
  received --> rejected: reject
  resolved --> reversed: reverse
  reversed --> [*]
  rejected --> [*]
```

**Refund after completion (kind *no goods*).** Food that arrived cold or wrong, a missing
item, or a service not delivered as sold cannot come back as goods. The return is raised from
the completed order or from a complaint ticket and resolved directly from `requested` with a
complaint category (cold, late, missing item, wrong item, quality, service not delivered,
other) and a responsible party (business, driver or rider, aggregator, supplier, customer).
The responsible party drives reports and recovery: a supplier fault can open a supplier
claim, a rider fault goes to the rider's record. The refund is full or partial per line.

**Refunded by the channel.** An aggregator refunds its customer itself and deducts the amount
from its payout. The vertical records it as a *no goods* return resolved *refunded by the
channel* (06: on payout statement `match`): no credit note, no refund payment and no
`SAL.ACL.06` approval, because no money leaves the business; category and responsible party
are still required. The order's amount refunded and the refund reports include it. When the
channel takes the refund back, or the statement it came from is voided, the return runs
`reverse`. Returns resolved by credit note, refund or replacement are never reversed: they
are corrected with a new document.

**Vertical mapping:** 08's `credited` is `resolved` with resolution *credit note* or
*refund*. In 07, a return that comes from a quality complaint links to an NCR
(`MFG.QCL.02`) through `ReturnResolved`. In 06, an aggregator refund line on a payout
statement is a *no goods* return resolved *refunded by the channel*; the statement's `void`,
or a dispute the aggregator resolves by taking the refund back, runs `reverse`.

### 4.6 Campaign

Small on purpose: a campaign is a period with a budget and costs. Attribution and reports
are computed from leads and party acquisitions, never stored as states.

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

```mermaid
stateDiagram-v2
  [*] --> draft
  draft --> active: activate
  active --> ended: end
  ended --> active: reopen
```

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

**Vertical mapping:** no vertical had a campaign machine. 05 records the regulator's
advertising permit on a campaign that promotes services (`CLN.ACL.13`).

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

## 5. Workflows

### 5.1 Lead to customer

1. A lead is captured from a web form, the portal, an import or by staff. Source and
   consent are recorded (`PTY.CNS.01`). A web form link with a campaign code sets the
   campaign and its source.
2. The owner contacts and qualifies it. Next-action dates drive the follow-up list.
3. The vertical's conversion event runs `win`, and the lead's party becomes a customer. Its
   party acquisition takes the lead's source and campaign, and the first sale is linked
   when it happens.

### 5.2 Order to cash (B2B, on account)

1. An order is entered by a sales rep, placed in the portal, or created from an accepted
   quotation. Prices come from the party's price list.
2. On submit, the credit check runs. Exposure counts open invoices, uncleared post-dated
   cheques (`wl-payments`) and confirmed orders not yet invoiced; it is checked against the
   customer's limit and, for a credit group, the combined limit. An overdue balance beyond
   the grace days also fails. A failure puts the order on `credit_hold`, and the accountant
   and sales rep are notified (`SAL.RUL.01`). Release needs an approver who did not create
   the order (`SAL.ACL.03`).
3. `confirmed` reserves stock. `wl-inventory` creates the dispatch note, or the vertical starts
   its own fulfilment.
4. At dispatch the credit is checked again (`credit_recheck`); a failure puts the order on
   `credit_hold` before the goods leave. Each delivery or collection runs `fulfil` for the
   quantities the customer accepted (once per hand-over: from the dispatch note when there
   is one); refused lines come back and stay open or are cancelled with `cancel_line`. A
   delivery reversed or lost after it was delivered runs `reverse_fulfilment`. A *collect*
   order whose goods are picked but not collected after N days triggers a reminder
   (`SAL.RUL.04`). The invoice is drafted on delivery or periodically, depending on
   configuration.
5. When everything fulfilled is invoiced, the order closes.

### 5.3 Direct order (website or portal, prepaid)

1. The customer builds a cart from the channel's catalogue, applies a promo code and pays
   online (`FIN.PAY.02`, Stripe or N-Genius) or chooses cash on delivery where the channel
   allows it.
2. An online-paid order waits in `awaiting_payment` until the gateway confirms. If the
   payment fails or times out, the order is cancelled and the customer may try again.
3. The order arrives as `pending_acceptance`. The outlet accepts it, or rejects it and the
   payment is refunded. A payment confirmed after a rejection or cancellation is refunded
   automatically (`SAL.RUL.06`).
4. The vertical fulfils it (the F&B kitchen); when delivered, its fulfilment completes and
   runs `fulfil`.
5. A complaint after delivery (cold, missing, wrong) is refunded through a sales return of
   kind *no goods* with the category and the responsible party (`SAL.ACL.06`).

### 5.4 Counter sale and day closing

1. Staff open a till with a float.
2. Each sale: scan or search items, apply allowed discounts, take payment in one or more
   methods or charge it to a customer account (credit check runs), and print or send the
   receipt (simplified or full tax invoice).
3. At the end of the shift: count, enter totals, submit. A variance above the threshold goes
   to approval. The Z report is generated and the session is locked.
4. A manager reviews the session. Cash goes to the cash custody of `wl-payments`.

### 5.5 Prepaid package

1. A package is sold as a catalogue item of type `package` and paid in full or by instalment.
2. Each qualifying event consumes credits. Cancelling the event reverses the consumption.
3. When credits run low, the holder is offered a renewal (`SAL.RUL.03`). Before expiry, the
   holder is reminded (`SAL.RUL.02`). On expiry the unused value is reported. An extension
   or refund follows the approval rules.

### 5.6 Return

1. A return is requested at the counter, through the portal, or from a complaint.
2. The goods are received into quarantine and inspected. Goods from a lot under an open
   recall are accepted outside the return window, linked to the recall and held with it;
   the recall decides their outcome.
3. The return is resolved: credit note, refund or replacement. Stock is restored or written
   off.
4. With nothing to return (food, services), the return is resolved directly with a complaint
   category and the responsible party.

### 5.7 Changing or cancelling an order

1. Before fulfilment starts, staff amend the order (new revision) or cancel lines; the
   customer gets the revised confirmation. Prepaid differences are charged or refunded.
2. After fulfilment started, the change or cancellation needs a manager (`SAL.ACL.01`,
   `SAL.ACL.05`) and an outcome per line: return to stock, finish to stock, scrap, with or
   without a cancellation fee.
3. At dispatch, an order on account is credit-checked again; a failure puts it on hold.
4. A bounced cheque from the customer puts the customer account and its open orders on hold
   (`SAL.RUL.05`) until the cheque is replaced or the balance paid. A holder of
   `credit.release` then requests *Clear account hold* and another approves (§4.2); the held
   orders are released one by one.

### 5.8 Campaigns and acquisition reports (`SAL.CRM.02`)

1. Marketing staff create a campaign with its marketing channel, period, budget and code,
   and activate it (or it activates on its start date). Links to web forms and the linked
   promo code carry the code.
2. Costs are entered as they are spent, or linked to the supplier bill line of the agency or
   the ad platform (`wl-invoicing`). An entry beyond the budget needs a manager; the owner is
   told at 80% and 100% of the budget (`SAL.RUL.10`). Corrections are voids, never edits of an
   approved entry (§4.6).
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

### 5.9 Loyalty (`SAL.LOY.01`)

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
5. Points expire per earn lot (`SAL.RUL.08`), with a reminder before; they expire even when
   the reminder could not be delivered (the account shows it). Manual adjustments above the
   limit need a manager (`SAL.ACL.07`); a rejected adjustment writes nothing and the requester
   is told, a returned one is edited and submitted again.
6. The points liability report shows outstanding points and their AED value by age and
   expiry month, per brand and entity, for the accountant. Each movement records the entity
   of its sale, so points earned in one entity and redeemed in another are visible; how that
   is booked follows `⚠ open: accountant (A-15)`.
7. The member sees, in the portal and on each receipt, the balance, the points earned and
   redeemed on the sale, and the next expiry date with its points; the terms in English and
   Arabic are linked.

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

## 6. Interfaces to other modules

**Provides**

| Service | Purpose |
|---|---|
| `Pricer` | Price for a party, item, options, quantity, channel and date, with the applied price list and discounts |
| `CreditChecker` | Check an amount against a party's limit, overdue balance and on-hold flag; used by sales orders and by verticals (e.g. a fleet job card); `credit_recheck(order)` is the dispatch guard for orders on account |
| `OrderFulfilment` | `fulfil(order, lines, quantities, source)` and `reverse(order, lines, quantities, source)`, called from `wl-inventory` and `wl-delivery` events and by vertical fulfilment; idempotent per source |
| `PackageLedger` | `consume`, `reverse` and `balance` for a party and item, called by `wl-scheduling` and verticals |
| `LeadConversion` | Mark a lead won from any vertical event |
| `DiscountEngine` | Evaluate discount rules and promo codes for a cart, and a loyalty redemption line; record usage |
| `CampaignAttribution` | `attribute(party or lead, source, campaign, set_from)` and `record_first_sale(party, subject, value)`, called by verticals whose first sale is their own record (01 lease, 04 engagement, 10 enrolment) |
| `LoyaltyLedger` | `earn`, `redeem`, `reverse` and `balance` for a party, called by sales returns, `wl-invoicing` credit notes and verticals |
| `Segments` | `members(segment, at)` for a retention campaign and for `wl-notify` broadcasts |

**Consumes**

- `wl-kernel`: `Subject`, `Sequences` (order, receipt and return numbers), `Settings`,
  `Approvals`, `CustomFields`, `Automations`, state machines.
- `wl-parties`: parties, relationships (`PTY.REL.01` for sibling discounts and payer vs
  holder), consents. Events consumed: `PartiesMerged` (party acquisition kept once; loyalty
  lots moved, §4.7), `PartyBlocked` and `PartyUnblocked` (loyalty account paused and resumed;
  agreement kind *loyalty account* registered in `BlockEffects` with effect *pause*).
- `wl-invoicing`: `Invoicer` (invoices, simplified tax invoices, credit notes), receivables
  balance for the credit check, quotations as order source.
- `wl-payments`: `PaymentRecorder`, `PaymentLinks`, `Refunds`, `CashCustody` for till
  cash, `Cheques` (uncleared post-dated cheques in the credit exposure). Events consumed:
  `PaymentReceived` (`confirm_payment`, `SAL.RUL.06`), `PaymentFailed` (`payment_failed`),
  `ChequeBounced` (`SAL.RUL.05`). From `wl-invoicing`: `InvoiceReopened` for a package's
  invoice (task to collect, holder told; consumption suspended only when the setting is on,
  §4.4).
- `wl-inventory`: availability, reservations, dispatch notes (which call `credit_recheck`).
  `StockMoved` for `receive` and `transfer_in` movements reserves arriving stock for open
  back-orders.
  Events consumed for orders shipped on a dispatch note: `DispatchDelivered` (`fulfil` for
  delivered quantities), `DispatchReturned` (as `DeliveryReturned` below), `DispatchReversed`
  and `DispatchLost` (`reverse_fulfilment` when fulfilled; lost lines to the sales rep's
  task). Receipt of returned goods (recalled lots linked to the recall and its stock hold),
  scrap and wastage for cancelled lines; `LotTrace` to find whether a returned lot is
  recalled.
- `wl-ticketing`: complaint tickets linked to a refund after completion.
- `wl-invoicing` for `SAL.CRM.02` and `SAL.LOY.01`: supplier bill lines linked to campaign
  cost entries, `SupplierBillApproved` (a linked entry starts to count) and the bill's `void`
  (linked entries voided); `InvoiceIssued` (first sale), `InvoicePaid` (earning for invoices
  not made from an order or a counter sale), `CreditNoteIssued` (points reversal; converted
  value), `InvoiceReopened` with reason *payment reversed* and `InvoiceWrittenOff` (points
  reversal, `SAL.RUL.09`). From `wl-payments`: `PaymentReversed` on a counter sale payment
  (`SAL.RUL.09`).
- `wl-delivery`: delivery requests for orders with delivery method *deliver*. Events
  consumed only for orders delivered without a dispatch note (§4.2 *Fulfilment from
  deliveries*): `DeliveryDelivered` (`fulfil` for accepted quantities), `DeliveryReturned`
  (fulfilment progress of the returned lines back to *not dispatched*; reservation request
  kept; sales rep and customer told), `DeliveryReversed` (`reverse_fulfilment`),
  `DeliveryLost` (`reverse_fulfilment` when it was under review; lost lines to the sales
  rep's task).
- `wl-notify`: confirmations, receipts, reminders; `send_otp` for redemption by code;
  marketing broadcasts for retention campaigns, with their results (`BroadcastCompleted`,
  `BroadcastCancelled`, `BroadcastStopped`).
  `wl-documents`: order confirmations, receipts, Z reports.

**Events emitted**

| Event | When | Payload |
|---|---|---|
| `LeadCreated`, `LeadWon`, `LeadLost` | lead created or closed | lead, party, source, owner, converted subject or reason |
| `LeadWinReverted` | `revert_win` | lead, party, voided converted subject |
| `SalesOrderConfirmed` | order reaches `confirmed` | order, party, channel, lines, delivery method |
| `SalesOrderCreditHeld` | order goes to `credit_hold` | order, party, open balance, overdue, limit |
| `SalesOrderFulfilled` | quantities fulfilled | order, lines and quantities, source |
| `SalesOrderFulfilmentReversed` | `reverse_fulfilment` | order, lines and quantities, source (dispatch or delivery), cause (reversed, lost) |
| `SalesOrderCancelled`, `SalesOrderRejected` | order cancelled or rejected | order, reason, responsible party, outcome and fee per line |
| `SalesOrderAmended` | `amend` | order, revision, changed lines, value before and after, reason |
| `SalesOrderLineCancelled` | `cancel_line` | order, line, quantity, reason, outcome, fee |
| `SalesOrderClosed` | order closed | order |
| `CounterSaleCompleted` | POS sale paid | sale, till session, totals by payment method |
| `TillSessionClosed` | till counted and submitted | session, expected, counted, variance |
| `PackageActivated`, `PackageConsumed`, `PackageExtended`, `PackageExpired`, `PackageRefunded` | package changes | package, party, credits, source |
| `ReturnResolved` | return resolved | return, kind, original sale, resolution, value, lot, complaint category, responsible party |
| `ReturnReversed` | `reverse` | return, original sale, value, reason |
| `CreditLimitChanged` | limit or terms approved | party, old and new values, approver |
| `CampaignActivated`, `CampaignEnded` | campaign changes | campaign, purpose, marketing channel, period, budget |
| `CustomerAcquired` | a party's first sale is linked | party, source, campaign, first sale, value |
| `CustomerAcquisitionChanged` | first sale replaced or cleared, source or campaign changed (`SAL.ACL.09`), or acquisition superseded by a merge | party, old and new source, campaign and first sale, reason |
| `PointsEarned`, `PointsRedeemed`, `PointsReversed`, `PointsExpired`, `PointsAdjusted`, `PointsTransferred` | points movement | account, party, points, AED value, source |

**Attaches to:** a sales order and a lead may reference any `Subject` as their source
(quotation, booking, campaign). A campaign cost entry may reference a supplier bill line. Package movements reference the `Subject` that consumed
them.

**Permissions**

`leads.view`, `leads.manage`, `leads.assign` · `catalogue.view`, `catalogue.manage`,
`price_lists.manage` · `sales_orders.view`, `sales_orders.create`, `sales_orders.accept`,
`sales_orders.amend`, `sales_orders.cancel`, `prices.override` · `credit.view`, `credit.release`,
`credit.limits.propose`, `credit.limits.approve` · `pos.sell`, `pos.open_till`,
`pos.close_till`, `pos.reopen_till`, `pos.review` · `packages.view`, `packages.sell`,
`packages.extend`, `packages.adjust`, `packages.refund` · `discounts.manage`,
`discounts.apply_manual` · `returns.request`, `returns.handle` · `campaigns.view`,
`campaigns.manage`, `campaigns.costs.manage`, `acquisition.reports.view` · `loyalty.view`,
`loyalty.enrol`, `loyalty.redeem`, `loyalty.adjust`, `loyalty.manage`, `segments.manage`.

**Shared access rules** (registered in [ACCESS-CONTROL.md](../ACCESS-CONTROL.md) §9, all
`in use`; `SAL.ACL.07` to `SAL.ACL.09` added 2026-10-16)

| ID | Rule | Pattern | Parameter (default) | Generalises |
|---|---|---|---|---|
| `SAL.ACL.01` | Cancelling or refunding a paid order above a value, or after fulfilment started, needs a manager | `LIM`, `STA` | AED 50 (F&B default) | `FNB.ACL.11` (void after fire, proposed, Later) |
| `SAL.ACL.02` | A manual discount or a price below list beyond a limit needs a manager | `LIM` | 10% (07, 08: 5%) | `CLN.ACL.07`, `EDU.ACL.07`, `AUT.ACL.03`, `MFG.ACL.05` |
| `SAL.ACL.03` | An order over the customer's or group's credit limit, or with invoices overdue beyond the grace days, goes on credit hold; the hold is released only by a role with `credit.release`, not by the order's creator; an account hold (`SAL.RUL.05`) is cleared by a holder of `credit.release` with another holder's approval, never the requester | `LIM`, `SOD` | grace 30 days past terms | `MFG.ACL.04`, `AUT.ACL.04`, `AUT.ACL.05` |
| `SAL.ACL.04` | A completed counter sale is never edited or voided; it is reversed by a sales return with a credit note | `STA` | — | `AUT.ACL.06` |
| `SAL.ACL.05` | Amending an order or cancelling a line after its fulfilment started needs a manager, as does choosing *finish to stock* or *scrap* or waiving the cancellation fee; the requester cannot approve | `STA`, `SOD` | any value | — |
| `SAL.ACL.06` | A refund or credit after completion with nothing returned needs a complaint category and a responsible party; above a value it needs a manager, and whoever records it cannot approve it (not for resolution *refunded by the channel*, where no money leaves the business) | `LIM`, `SOD` | AED 50 | — |
| `SAL.ACL.07` | A manual points adjustment above a number of points, or closing a loyalty account that still has points at staff's initiative (not at the member's request or on erasure, §4.7), needs a manager, and the requester cannot approve; a campaign cost entry that takes the campaign over its budget needs a manager other than the requester | `LIM`, `SOD` | 500 points; any budget overrun | — |
| `SAL.ACL.08` | A member is attached to a sale only before the sale completes, by the member's one-time code, portal login or mobile number; a cashier cannot attach a sale they process to a loyalty account of their own party; a mobile number belongs to one account per programme | `STA`, `SOD` | — | — |
| `SAL.ACL.09` | Changing the source or campaign of a party acquisition that has a first sale, or of a won lead, needs a reason and approval by another holder of `campaigns.manage`; the change is audited and shown on the exceptions report | `STA`, `SOD` | on | — |

**Shared automations** (registered in [WORKFLOW.md](../WORKFLOW.md) §7, all `in use`;
`SAL.RUL.07` to `SAL.RUL.10` added 2026-10-16)

| ID | When | If | Then | Parameters (default) | Generalises |
|---|---|---|---|---|---|
| `SAL.RUL.01` | sales order → `credit_hold` | — | notify accountant and the sales rep | — | — |
| `SAL.RUL.02` | N days before a package's end date | credits remaining > 0 | notify the holder (and the payer, e.g. a guardian) | N = 14 | `CLN.RUL.06`, `EDU.RUL.04` |
| `SAL.RUL.03` | package credit consumed | credits remaining ≤ N | notify the holder or payer with a renewal link | N = 2 | `EDU.RUL.03` |
| `SAL.RUL.04` | N days after the dispatch of a *collect* order reaches `picked` (ready for collection) | not collected | notify the customer and the sales rep | N = 3 | `AUT.RUL.06` |
| `SAL.RUL.05` | `ChequeBounced` (`wl-payments`) | the drawer has a customer account | set the account on hold (and the credit group when configured); run `hold` on its orders on account not yet dispatched; refuse new orders and counter sales on account until the account hold is cleared (*Clear account hold*, §4.2); notify the accountant and the sales rep | whole group: on | — |
| `SAL.RUL.06` | `PaymentReceived` for an order | order is `cancelled` or `rejected`, or already fully paid, or `awaiting_payment` and the amount differs from the order total (partial or over-payment: it cannot confirm the order, which then times out) | request a full refund of that payment (`wl-payments` `Refunds`, reason *late payment*); notify the customer and the outlet or sales manager; the order stays as it is | — | — |
| `SAL.RUL.07` | a qualifying sale completes (`CounterSaleCompleted`; sales order → `fulfilled`; `InvoicePaid` for an invoice not made from an order or a counter sale), or a sales return or credit note on such a sale is issued | the party has an `active` loyalty account that is not paused (party blocked); the member was attached before completion (`SAL.ACL.08`); eligible amount ≥ the minimum; not yet earned (or reversed) for this source | `earn` points or a visit with the lot's expiry date, or `reverse_earn` in proportion; when visits reach the reward, the reward is made available and the member told | 1 point per AED 1; minimum AED 0; validity 12 months | — |
| `SAL.RUL.08` | N days before an earn lot expires; and when its expiry date has ended (a lot can be redeemed through its expiry date) | points of the lot remain | notify the member (category *reminder*, opt-out respected); after the date, `expire` the rest of the lot in the daily run of the next morning (02:00 Asia/Dubai) | N = 30 | — |
| `SAL.RUL.09` | `PaymentReversed` on a payment of an earning counter sale; `InvoiceReopened` with reason *payment reversed*, or `InvoiceWrittenOff`, for an earning invoice or the invoice of an earning order or counter sale; `SalesOrderFulfilmentReversed` for an earning order | the source earned points or a visit not yet reversed for this cause | `reverse_earn` in proportion to the amount reversed, written off or no longer fulfilled (the balance may go below zero); notify the member; when the sale is paid or fulfilled again (`InvoicePaid`, `PaymentReceived`, `fulfil`), earn again for that amount | — | — |
| `SAL.RUL.10` | a cost entry of a campaign is approved | approved costs reach a threshold share of the budget for the first time | notify the campaign owner | 80% and 100% | — |

## 7. Configuration per client

| Setting | Default |
|---|---|
| Channels and, per channel: acceptance required, acceptance timeout, credit check, default price list | counter and phone: no acceptance, credit check on; website and portal: acceptance on, 10 min timeout |
| Prices entered VAT-inclusive | on for consumer channels, off for B2B |
| Credit: grace days past terms; hold on overdue; hold on limit | 30 days; on; on |
| Invoice timing for orders | on fulfilment (alternatives: on confirmation, periodic) |
| Manual discount limit (`SAL.ACL.02`) | 10% |
| Paid-order cancellation limit (`SAL.ACL.01`) | AED 50 |
| Till variance needing approval | AED 20 |
| Return window and restocking fee | 14 days; 0% |
| Online payment window before `payment_failed` (gateway timeout) | 15 min |
| Unique external order number per channel | on |
| Unique customer PO number per customer (with call-off number) | off (07: on) |
| Back-orders allowed per channel | off (08: on for counter, phone and sales rep channels) |
| Amendments allowed per channel | on, except aggregator channels |
| Cancellation after fulfilment started: outcomes offered; fee (% of the line or amount) | return to stock, finish to stock, scrap, limited to what the vertical can do (06: return to stock, scrap; 08: return to stock); 0 |
| Credit re-check at dispatch for orders on account | on |
| Bounced cheque hold (`SAL.RUL.05`): hold the whole credit group | on |
| Refund after completion: approval above (`SAL.ACL.06`); complaint categories; responsible parties | AED 50; cold, late, missing item, wrong item, quality, service not delivered, other; business, driver or rider, aggregator, supplier, customer |
| Package defaults: validity, refund from expired, extension limit | 6 months; off; one extension |
| Package: suspend consumption while a reopened invoice balance is unpaid (§4.4) | off (05, 10: task to collect only) |
| Package reminders: days before expiry (`SAL.RUL.02`); low-credit threshold (`SAL.RUL.03`) | 14 days; 2 credits |
| Uncollected order reminder (`SAL.RUL.04`) | 3 days |
| Credit groups with a combined limit | off |
| Counter sales on account | off (08: on) |
| Discount stacking | off |
| Lead sources, lost reasons, qualification fields | seeded sources (§3 *Lead source*); lost reasons and qualification fields per vertical starter list, always with *duplicate* and *spam or invalid* (left out of lead counts, §5.8) |
| Campaign required for sources (`SAL.CRM.02`) | none (05: *paid ads*, *social*) |
| Late-response days after a campaign ends (leads still attributed) | 30 |
| Attribution window for the converted value | first sale only (alternative: sales in the first N months, N = 12) |
| Campaign cost over budget needs a manager (`SAL.ACL.07`) | on |
| Budget alerts to the campaign owner (`SAL.RUL.10`) | 80% and 100% |
| Changing the attribution of an acquired party or a won lead needs a second person (`SAL.ACL.09`) | on |
| Loyalty programme (`SAL.LOY.01`): earn kind, rate, minimum, exclusions, point value, minimum to redeem, maximum share of a sale | off; when on: per AED, 1 point per AED 1, AED 0, none, AED 0.01 per point, 500 points, 100% (06: 50%) |
| Points validity; expiry reminder (`SAL.RUL.08`) | 12 months; 30 days before |
| Confirm redemption by a one-time code to the member | on |
| Points held for an open sale before completion | 30 minutes |
| Sales per member per day before the account is listed for review (`SAL.ACL.08`) | 5 |
| Notice before a programme change takes effect; redemption period after the programme is turned off (§4.7) | 30 days; 90 days |
| Points given back on a return after their expiry | 30 days |
| Manual points adjustment needing a manager (`SAL.ACL.07`) | above 500 points |
| Retention campaign response days (bought after the send) | 30 |
| Segment rules on clinical services (05) | off: segments in 05 use visit dates, spend and non-clinical items only |
| Numbering | `SO-`, `RCPT-`, `RET-` + year + sequence |

## 8. Extension points

- **Strategy traits:** `PriceRule` (e.g. auto parts tier pricing, F&B channel prices),
  `CreditPolicy` (e.g. a fleet customer checked per contract), `ConversionRule` (which
  event wins a lead), `PackageConsumptionRule` (what consumes how many credits),
  `ReturnPolicy`, `FirstSaleRule` (which record is a party's first sale, e.g. the 01 lease),
  `EarnRule` (what earns how many points or visits), `SegmentRule` (add a rule kind: 05 no
  booking in N days; 08 vehicle make and next service due by date or mileage; 10 course
  completed without re-enrolment, trial attended and not enrolled, guardians with N or more
  students).
- **Events:** every event in §6. Vertical fulfilment (F&B kitchen, workshop) starts from
  `SalesOrderConfirmed` and reports back through `OrderFulfilment`.
- **Custom fields:** on lead, catalogue item, customer account, sales order, order line and
  campaign.
- **Overridable views:** order confirmation, receipt (80 mm and A4), Z report, return note,
  portal order pages, catalogue cards.

## 9. Renox fit

| Need | Renox provides | Gap |
|---|---|---|
| Catalogue, price lists, orders in the back office | grid, forms, repeater, search, import | — |
| POS screen: fast item entry, cart updates without reloads | live components, `keypad` and `quantity` blocks | Barcode scanning with the camera or a keyboard-wedge scanner: R5 (`rx-barcode-scan`) |
| Receipts and Z reports | `Download::bytes` | PDF (R3), Arabic (R1); 80 mm thermal receipt layout (R21) |
| Catalogue and price import (08 part master with cross-references from Excel) | import (CSV, row validation, templates) | R23 (Excel files, dry run, rollback window) |
| Lead pipeline board | `kanban` block | — |
| Acquisition and loyalty reports (by source, campaign, period) | grid with grouping, export | Charts on report pages: as for other module reports |
| Approvals, settings, state machines | — | wl-kernel; R2; R12 |
| Online payment for direct orders | renox-billing gateways (Stripe, Xendit) | Stripe and N-Genius through `wl-payments` (R17; N-Genius adapter is a gap) |
| POS during a network outage | — | Offline queue: R25, delivered in Renox 1.2.0 (no conflict merge); adoption Later. The MVP is online only ([ARCHITECTURE.md](../ARCHITECTURE.md) §1 decision 4) |

## 10. Compliance

- **Tax invoices:** a full tax invoice is required for B2B sales to VAT-registered buyers. ✔ A
  simplified tax invoice may be issued to buyers that are not VAT-registered, or to
  registered buyers when the consideration is at most AED 10,000 (Executive Regulation
  Art. 59(5)) [S1]; once e-invoicing applies to the client (1 January or 1 July 2027), B2B
  sales need full e-invoices (Cabinet Decision 100 of 2025) [S2]. Credit notes for returns
  are tax credit notes.
- **VAT-inclusive prices:** ✔ consumer prices must be displayed VAT-inclusive (fine AED
  5,000) [S3].
- **Advance payments and packages:** VAT on prepaid packages is due when payment is
  received, because the date of supply is the earliest of payment, invoice or delivery
  (Decree-Law Art. 25) ✔ [S4]. Unused balances are deferred revenue until consumed or
  expired.
- **Consumer protection** (Federal Law No. 15 of 2020): return, refund and warranty terms
  shown before purchase. No federal rule on the expiry of prepaid services or gift vouchers
  was found in Cabinet Resolution 66 of 2023 [S5]; whether an expiry term is enforceable is
  `⚠ open: lawyer (L-08)`.
- **Marketing:** leads and promo campaigns need a recorded consent (`PTY.CNS.01`, PDPL).
  Retention campaigns go only to members with marketing consent for the channel and offer
  an opt-out (`NTF.ACL.01`, [wl-notify](wl-notify.md) §10); for a minor, the guardian enrols,
  consents and receives the messages. ✔ The client registers its alphanumeric SMS sender ID
  (one per brand) with e& and du through its SMS provider under TDRA rules; promotional SMS
  use the `AD-` prefix [S6].
- **Loyalty points:** a redemption is shown as a discount line on the sale. Whether points
  redeemed reduce the taxable value of the discounted supply only (5% VAT on the price after
  the redemption), and how points given away for free and their expiry are treated for VAT
  and in the accounts (liability or provision), is `⚠ open: accountant (A-15)`; until answered, the
  redemption line reduces the taxable amount of the sale it is on and the liability report
  values points at the point value.
- **Loyalty terms changes:** whether a client may shorten the validity of points already
  earned, devalue them, or end a programme with a closing date under consumer protection law
  is `⚠ open: lawyer (L-39)`; until answered, a change applies only to points earned from
  its effective date and a programme that ends keeps a redemption period (§4.7).
- **Erasure (PDPL):** when a party's data is erased (`wl-parties` §10), its leads, party
  acquisition and points movements keep source, campaign, dates, values and points without
  the party, so acquisition and liability reports do not change; the loyalty account closes
  with reason *erased*.
- **Advertising by clinics:** a campaign that promotes health services needs the regulator's
  permit before sending; 05 records it on the campaign (`CLN.ACL.13`, 05 §10).
- **Amended and cancelled orders:** an amendment after an invoice was issued is corrected by
  a tax credit note or an additional invoice, never by editing the invoice. Whether a
  cancellation fee kept from a prepayment is consideration for a supply (5% VAT) or
  compensation outside the scope of VAT is `⚠ open: accountant (A-04)`; the fee line takes a
  configurable VAT category until this is answered.
- **Audit:** price overrides, discounts above the limit, credit releases, order amendments
  and revisions, cancellation outcomes, till variances, refunds, campaign costs and their
  approvals and voids, attribution changes (`SAL.ACL.09`), points adjustments, account
  closures and transfers are audited and visible in reports.

## 11. Out of scope

| Not here | Lives in |
|---|---|
| Quotations | `wl-invoicing` (`FIN.QUO.01`) |
| Invoices, credit notes, receivables | `wl-invoicing` |
| Payments, refunds, cash custody | `wl-payments` |
| Stock levels, reservations, picking and dispatch notes | `wl-inventory` |
| Delivery to the customer and proof of delivery | `wl-delivery` |
| F&B kitchen fulfilment, aggregator integration, branded ordering sites | `apps/fnb` (`FNB.KDS.*`, `FNB.AGG.*`, `FNB.DTC.*`) |
| Workshop job cards and supplier warranty recovery | `apps/auto` (`AUT.WAR.02`) |
| Customer warranty claims | `wl-ticketing`, category *warranty* (formerly `AUT.WAR.01`) |
| Third-party payer split (insurer and customer) | `wl-invoicing` (`FIN.TPP.01`) |
| Multi-touch attribution, ad platform imports (spend and clicks from Google or Meta), A/B tests, journeys | not planned; costs are entered or linked to supplier bills |
| Sending messages, consent records | `wl-notify`, `wl-parties` (`PTY.CNS.01`) |
| Sales commissions, CPQ, EDI | not planned |

## 12. Open questions

1. ~~Is a separate *customer account* needed, or are terms and limits enough as party fields?~~
   Decided (2026-10-15): a customer account in wl-sales (08 §6), with an optional credit
   group and a combined limit (08 §13 question 4). *(us)*
2. Do B2B clients invoice per delivery, or once a month per customer? Both are supported;
   which is the default? *(clients 07, 08)*
3. Can a package be shared within a family? *(client 05)* Instalments are allowed: a
   package activates on the first instalment (05 §7.1).
4. ~~Should POS work offline for short outages, or is a mobile-data fallback enough?~~ Decided
   (2026-10-15): online only in the MVP, as on the rest of the platform; the offline queue is
   delivered in Renox 1.2.0 (R25); adoption Later. *(clients 06, 08)*
5. Are sales rep commissions needed in any vertical? *(clients 07, 08)*
6. Thermal receipt printers: browser printing only, or a local print bridge? *(us)*

**Added after the gap analysis (2026-10-14):** a **dine-in** channel (06 F&B, Later; decided
2026-10-15 to stay Later, built when a client asks, 06 §13 question 9): a sales
order opened from a table session, several payments per bill and split bills on the till
session, service charge and fee lines ([06-fnb.md](../verticals/06-fnb.md) §4.7). ✔ A service
charge (up to 10%) is allowed only for hotel- or tourism-licensed outlets, and Dubai adds a
7% municipality fee for them [S7]; a mandatory service charge carries 5% VAT, voluntary tips
passed to staff do not [S8].

**Added for the SME problem map (2026-10-16):** `SAL.CRM.02` and `SAL.LOY.01` (gaps G1 and
G4 of [the problem map](../reviews/2026-10-16-sme-problem-map.md) §3).

7. Is a free points balance on a cash-paid sale a separate supply, and how is the points
   liability booked? `⚠ open: accountant (A-15)` (§10). *(accountant)*
8. Should a customer's points be shared across the brands of one client? Default: one
   programme per brand, no sharing. *(clients 06, 08)*
9. May a client shorten the validity of points already earned, devalue them, or end the
   programme, and with how much notice? `⚠ open: lawyer (L-39)` (§4.7, §10). *(lawyer)*

## Sources

Checked 2026-10-15. *Official* = government or legislation; others are secondary.

| # | Source | Type | Used for |
|---|---|---|---|
| S1 | FTA, Cabinet Decision 52 of 2017 on the VAT Executive Regulation (unofficial translation, 2017 text), https://tax.gov.ae/-/media/Files/FTA/links/Legislation/VAT/03-Cabinet-Decision-52-of-2017.pdf | Official | Simplified tax invoice conditions (Art. 59) |
| S2 | Ministerial Decision 244 of 2025 (e-invoicing) on mof.gov.ae; Cabinet Decision 100 of 2025 (simplified invoices under e-invoicing), via Grant Thornton and Crowe, https://www.crowe.com/ae/news/what-is-the-recent-amendment-to-uae-vat | Official / secondary | Simplified invoices end under e-invoicing |
| S3 | WAM, "FTA: businesses must display VAT-inclusive prices", https://wam.ae/en/article/hszr6ptm-fta-businesses-must-display-vat-inclusive-prices | Official | VAT-inclusive prices |
| S4 | FTA, Federal Decree-Law 8 of 2017 on VAT (unofficial translation), https://tax.gov.ae/DataFolder/Files/Pdf/VAT-Decree-Law-No-8-of-2017.pdf | Official | Date of supply for advance payments (Art. 25) |
| S5 | Baker McKenzie, "Exploring the UAE's latest Consumer Protection Regulations", https://insightplus.bakermckenzie.com/bm/consumer-goods-retail_1/uae-enhancing-consumer-rights-exploring-the-uaes-latest-consumer-protection-regulations; Cabinet Resolution 66 of 2023 on moet.gov.ae | Secondary / official | No prepaid expiry rule found |
| S6 | Telerivet, "UAE SMS compliance: TDRA and the AD- prefix", https://www.telerivet.com/blog/uae-sms-compliance-tdra-ad-prefix; D7 Networks, https://d7networks.com/sms/uae/sms-regulations/ | Secondary | SMS sender ID registration |
| S7 | Service charge and Dubai municipality fee (Gulf News, Khaleej Times, Arabian Business); Abu Dhabi fees removed in 2023 (Gulf News) | Secondary | Service charge and fees |
| S8 | VAT on service charges and tips, via Ripple and Tally | Secondary | VAT on service charge and tips |
