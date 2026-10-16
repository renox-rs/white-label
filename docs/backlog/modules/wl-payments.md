# Epic: wl-payments — negative flows

> Status: draft · Last updated: 2026-10-15 · Spec version: same commit as this file

This epic builds the negative flows that the negative-flow review added to
[wl-payments](../../modules/wl-payments.md): payment reversal with its allocations (X-03), failed
and late online payments and chargebacks (X-06, 06-N3-a), failed refunds (X-06), the bounced
cheque follow-up (X-05, 01-N3-a, 08-N3-a), the deposit deductions dispute, transfer and
adjustment while held (01-N4-c, 01-N12-c), set-off of an invoice against a supplier bill, and
cash custody for receptionists, tills, field cheques, run hand-overs, holders who do not hand
over and shortages found later (X-07, D-12, 03-N9-a, 05-N11-a).

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-PAY-01 | Reverse a payment and remove its allocations | high | s | Build 1 | Base: wl-payments Payment lifecycle |
| BL-PAY-02 | Fail an online payment and time out a pending one (`FIN.RUL.19`) | high | m | Launch 05 | Base: wl-payments Payment and Payment link lifecycle |
| BL-PAY-03 | Accept a late gateway confirmation and refund it automatically | high | m | Launch 05 | BL-PAY-02; Base: wl-payments Refund lifecycle |
| BL-PAY-04 | Record a chargeback and open the evidence task (`FIN.RUL.20`) | high | m | Launch 05 | BL-PAY-01; BL-INV-07 |
| BL-PAY-05 | Close a chargeback: won, lost or accepted (`FIN.ACL.18`) | high | m | Launch 05 | BL-PAY-04 |
| BL-PAY-06 | Fail a refund and restore its source | high | s | Build 1 | Base: wl-payments Refund lifecycle; BL-PAY-01 |
| BL-PAY-07 | Retry, change the method of, or cancel a failed refund (`FIN.RUL.21`) | high | m | Build 1 | BL-PAY-06 |
| BL-PAY-08 | Refund a payer credit balance | medium | s | Launch 05 | BL-INV-17; Base: wl-payments Refund lifecycle |
| BL-PAY-09 | Record a bounced cheque with partial honour, return certificate and bounce fee | high | m | Build 1 | Base: wl-payments Cheque lifecycle; Epic wl-sales: credit hold `SAL.RUL.05` |
| BL-PAY-10 | Re-present a bounced cheque | high | s | Build 1 | BL-PAY-09 |
| BL-PAY-11 | Settle part of a bounced cheque and replace it | high | s | Build 1 | BL-PAY-09 |
| BL-PAY-12 | Refer a bounced cheque to a legal case | high | s | Build 1 | BL-PAY-09 |
| BL-PAY-13 | Write off a bounced cheque through the invoice write-off | high | s | Build 1 | BL-PAY-09; BL-INV-01 |
| BL-PAY-14 | Chase a bounced cheque (`FIN.RUL.18`) | high | s | Build 1 | BL-PAY-09 |
| BL-PAY-15 | Dispute deposit deductions before the refund | high | m | Build 1 | Base: wl-payments Deposit lifecycle |
| BL-PAY-16 | Transfer a deposit to a new holder | medium | s | Build 1 | Base: wl-payments Deposit lifecycle |
| BL-PAY-17 | Count, recount, recover and write off a cash shortage (`FIN.ACL.19`) | medium | m | Launch 03 | Base: wl-payments Cash handover lifecycle; Epic wl-hr: cash shortage recovery adjustment |
| BL-PAY-18 | Confirm cheques collected in the field in a handover (`FIN.ACL.20`) | medium | m | Launch 08 | BL-PAY-17 |
| BL-PAY-19 | Create a till's handover from a closed till session | medium | s | Launch 06 | BL-PAY-17; Epic wl-sales: till session `close` |
| BL-PAY-20 | Move cash between drivers on a run hand-over | medium | s | Launch 03 | BL-PAY-17; Epic wl-delivery: run `hand_over` |
| BL-PAY-21 | Escalate a handover that is not handed over (`FIN.RUL.22`) | medium | s | Launch 03 | BL-PAY-17 |
| BL-PAY-22 | Top up or part-refund a held deposit when its agreement changes | high | s | Build 1 | Base: wl-payments Deposit and Refund lifecycle; BL-PAY-06 |
| BL-PAY-23 | Raise a later cash shortage against a holder | medium | s | Launch 03 | BL-PAY-17 |
| BL-PAY-24 | Set off a party's invoice against its supplier bill | medium | s | Launch 02 | Base: wl-payments Payment lifecycle; BL-PAY-01; BL-PTY-04 |

---

### BL-PAY-01 — wl-payments: reverse a payment and remove its allocations

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-payments Payment lifecycle (`record`, allocation) · **Blocks:** BL-INV-07, BL-INV-12, BL-INV-17, BL-PAY-04, BL-PAY-06, BL-PAY-24
**Review items:** X-03

#### Goal
A payment recorded in error or returned by the bank is reversed. Every allocation is removed and
`PaymentReversed` carries them, so wl-invoicing reopens each target (`unallocate`). The party is
told about a bank return. A gateway payment with an open dispute cannot be reversed this way.

#### Files to read
- `docs/modules/wl-payments.md` §3 (Payment, Allocation), §4 *Payment* (`reversed` state,
  `reverse`, the paragraph after the diagram, table "Who reacts …" rows for `PaymentReversed`),
  §6 (`PaymentReversed`, `payments.reverse`)
- `docs/modules/wl-invoicing.md` §5.11

#### Terms
- **Allocation:** part of a payment applied to a target: invoice, supplier bill, credit note,
  deposit, supplier debit note, payer credit balance.
- **Party credit balance:** unallocated money of the party.
- **Prepaid subject:** a booking, reservation or order the payment was a prepayment for.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `reversed` | neutral | yes | Recorded in error, returned by the bank, or a chargeback lost or accepted |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reverse` | received | reversed | `payments.reverse` | reason (entered in error, bank return); the payment is not a gateway payment with an open dispute | every allocation removed; event `PaymentReversed` with the allocations, so wl-invoicing runs `unallocate` on each target (the invoice goes back to `partially_paid`, `issued` or `overdue` by balance and due date); party notified for a bank return |

Event `PaymentReversed`: payment, party, reason (entered in error, bank return, chargeback),
allocations removed (target, amount), prepaid `Subject` (if any); wl-invoicing runs `unallocate`
on each target.

After the diagram: "Unallocated amounts show as the party's credit balance. A reversed payment
whose credit balance was already refunded leaves the party owing that amount; the platform gives
the accountant a task "raise a charge for AED X" with the payment and the refund linked, and the
accountant raises the charge (the platform does not create the charge itself)."

Who reacts: "`PaymentReversed` (bank return or entered in error) | wl-invoicing `unallocate`; the
subject that the payment unblocked (prepaid booking, order, enrolment) gets the event and the
vertical decides (notify staff by default)".

#### Edge cases decided
- **Part of the payment is still unallocated credit:** the credit is removed with the payment.
- **The credit was already refunded:** the reversal still runs; the platform creates a task for
  the accountant "raise a charge for AED X" (spec). The platform creates the task only, never the
  charge; the accountant raises the charge by hand.
- **Payment created by a bank statement import:** reversing it marks the statement line unmatched again.
- **Double click:** the second call finds `reversed` and is refused.
- **Allocation to a written-off invoice:** still removed and sent in the event; wl-invoicing
  handles it (BL-INV-07).
- **Date:** the reversal date is today in Asia/Dubai; the original payment date is kept.

#### Must not
- Must not delete the payment or its allocations' history.
- Must not reverse a gateway payment with an open dispute (use the chargeback flow).
- Must not change the state of any target directly; only through the event.

#### Done when
- [ ] Given a `received` payment of AED 3,000.00 allocated 2,000 to INV-1 and 1,000 to INV-2, when
      user A reverses it with reason "bank return", it is `reversed`, both allocations are removed,
      `PaymentReversed` lists both, the party is notified.
- [ ] With reason "entered in error", the party is not notified.
- [ ] Given AED 500.00 of its credit already refunded, the accountant gets a task "raise a charge
      for AED 500.00" linking the payment and the refund, and no charge is created.
- [ ] A reversal without a reason is refused; reversing a `reversed` payment is refused.

#### Out of scope
Invoice reopening (BL-INV-07); chargebacks (BL-PAY-04); outgoing payment reversal effects on
supplier bills (BL-INV-12).

---

### BL-PAY-02 — wl-payments: fail an online payment and time out a pending one (`FIN.RUL.19`)

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: wl-payments Payment and Payment link lifecycle (`start_online`, `confirm`) · Epic wl-kernel: integration down · **Blocks:** BL-PAY-03, BL-CLN-16, BL-EDU-01, BL-EDU-18, BL-FNB-16
**Review items:** X-06, 06-N3-a

#### Goal
An online payment that the gateway declines, that the payer abandons, or that gets no final answer
within the timeout ends in `failed`. The link stays open so the party can retry, and consumers
(subscriptions, bookings, orders) get `PaymentFailed`. When the link expires, `PaymentLinkExpired`
lets wl-scheduling release an unpaid hold.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Payment* (`pending`, `failed` states; `start_online`, `fail`;
  table "Who reacts to `PaymentFailed` …"), §4 *Payment link* (`expire`), §5.2 step 4, §6
  (`FIN.RUL.19`, events `PaymentFailed`, `PaymentLinkExpired`), §7 (pending timeout, link expiry)

#### Terms
- **Gateway:** Stripe or Network International N-Genius, through `GatewayAdapter`.
- **Status query:** a call to the gateway asking the payment's final state.
- **Payment link:** a request to pay online with an expiry.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `pending` | info | no | Online payment started; waiting for the gateway |
| `failed` | error | no | Gateway declined, the payer abandoned, or no answer before the timeout; only a late confirmation from the gateway moves it on |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `fail` | pending | failed | system (gateway webhook: declined, reason from the gateway; or the hosted session cancelled or expired by the payer, reason *abandoned*; or `FIN.RUL.19` when the gateway gives no final answer before the timeout, reason timeout, which also covers a payer who leaves without the gateway telling the platform) | — | payment link stays `open` until its expiry; the party sees the reason and a retry button; event `PaymentFailed` (consumers below) |
| `expire` (Payment link) | open | expired | system | expiry passed | event `PaymentLinkExpired` (wl-scheduling releases an unpaid booking hold) |

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `FIN.RUL.19` | M minutes after payment → `pending` | still `pending` | ask the gateway for the status; run `confirm` or `fail` (reason timeout); a status query that fails, or an answer that is not final (still processing), also gives `fail` (reason timeout), and a later webhook can still `late_confirm` | M = 30 |

Who reacts (§4):

| Situation | Reaction |
|---|---|
| Subscription automatic charge failed | wl-invoicing retries (`FIN.RUL.06`) |
| Invoice or payment link paid online failed | invoice unchanged; the link stays `open`; no task (the party retries) |
| Prepayment for a booking (wl-scheduling) or a direct order (wl-sales) failed or timed out | the subject is not confirmed or placed: wl-scheduling keeps the slot on hold until the link expiry, then releases it; wl-sales does not place a prepaid direct order (06: the order is only sent to the kitchen after `confirm`) |
| Payment still `pending` after the timeout | `FIN.RUL.19` asks the gateway, then runs `confirm` or `fail` |

Events: `PaymentFailed` — payment, party, gateway reason (or timeout), subscription (if any),
payment link and prepaid `Subject` (if any). `PaymentLinkExpired` — payment link, party, amount,
prepaid `Subject` (if any).

#### Edge cases decided
- **Payer abandons:** a gateway webhook that reports the hosted session cancelled or expired runs
  `fail` with reason *abandoned*; when the gateway sends nothing, `FIN.RUL.19` runs `fail` with
  reason timeout after M minutes (spec).
- **Gateway status query fails (gateway down):** `fail` with reason timeout (spec); a later
  webhook can still `late_confirm` (BL-PAY-03).
- **Gateway says "still processing":** not a final answer → `fail` (timeout) (spec).
- **Webhook `fail` and timeout at the same moment:** the first transition wins; the second finds
  `failed` and does nothing.
- **Party retries:** a new payment (`start_online`) on the same link; the failed one stays `failed`.
- **Timeout clock:** from `pending` time (UTC stored, shown in Asia/Dubai).

#### Must not
- Must not close the payment link on a failure.
- Must not confirm a booking or place a prepaid order on `fail`.
- Must not create a task for an ordinary failed invoice payment.

#### Done when
- [ ] Given a `pending` payment and a decline webhook, it is `failed`, the link stays `open`, the
      party sees the reason and a retry button, `PaymentFailed` is emitted with the link and subject.
- [ ] Given a payment `pending` for 30 minutes and the gateway reporting it paid, it is `received` and allocated.
- [ ] Given the gateway giving no final answer, it is `failed` with reason timeout.
- [ ] Given a `pending` payment and a gateway webhook "session cancelled by the payer", it is
      `failed` with reason *abandoned* and the link stays `open`.
- [ ] Given a payer who closes the page and no webhook, the payment is `failed` with reason
      timeout 30 minutes after it became `pending`.
- [ ] Given the status query erroring (gateway down), it is `failed` with reason timeout.
- [ ] Given a link with expiry 2027-01-07 18:00 Dubai time, at that time it is `expired` and
      `PaymentLinkExpired` carries the booking.

#### Out of scope
Late confirmation (BL-PAY-03); subscription retries (`FIN.RUL.06`, wl-invoicing base); the
reactions inside wl-scheduling and wl-sales (Epic wl-scheduling, Epic wl-sales).

---

### BL-PAY-03 — wl-payments: accept a late gateway confirmation and refund it automatically

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-PAY-02 · Base: wl-payments Refund lifecycle · **Blocks:** BL-EDU-01
**Review items:** X-06, 06-N3-a

#### Goal
A gateway may confirm a payment after it was marked failed. The money is then received: allocated
if the targets are still open and the prepaid subject is still active, otherwise kept as the
party's credit and refunded automatically. System refunds for late payments go straight to
`approved` without a second person when they meet the strict conditions.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Payment* (`late_confirm`, "Who reacts" row "Late confirmation …"),
  §4 *Refund* (`request`), §5.2 step 4, §7 (late payment refund setting)
- `docs/modules/wl-sales.md` (`SAL.RUL.06`), `docs/modules/wl-scheduling.md` (payment for an expired hold)

#### Terms
- **Subject no longer active** (spec, one list for both rows): the prepaid booking or order is
  cancelled, rejected, already paid by another payment, or its slot gone (the booking hold was
  released, e.g. on `PaymentLinkExpired`). Otherwise it is still active.
- **`FIN.ACL.04`:** whoever requests or records a refund cannot approve it.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `late_confirm` | failed | received | system (verified gateway webhook after `fail`) | webhook signature and amount match | if the targets are still open and the prepaid `Subject` is still active: allocated as in `confirm` (targets only partly open: allocated up to their open balances, the rest kept as credit with a refund requested for it, `requested` because it does not equal the payment); otherwise the amount stays as the party's credit and a refund is requested automatically with reason "late confirmation" (approved without a second person when the `request` guard below holds: it equals the payment, goes back to the same card or account, and the subject is no longer active, i.e. cancelled, rejected, already paid by another payment, or its slot gone; setting); accountant notified; event `PaymentReceived` |
| `request` (Refund) | — | requested, approved | `refunds.request` or system (`late_confirm`; wl-sales `SAL.RUL.06`; wl-scheduling payment for an expired hold) | source with refundable balance; reason; target `approved` only for a system request of a late payment whose amount equals that payment, back to the same card or account, while the subject is cancelled, rejected, already paid by another payment, or its slot gone (the booking hold was released, e.g. on `PaymentLinkExpired`) (setting, on) | approvers notified; a system request that goes straight to `approved` is not subject to `FIN.ACL.04` (no person requested it), the accountant is notified, and it is paid by the original method |

Setting (§7): *Late payment refunds requested by the system (`late_confirm`, wl-sales
`SAL.RUL.06`, wl-scheduling payment for an expired hold) approved automatically when equal to the
payment and returned to the same card or account* — on.

#### Edge cases decided
- **Targets partly open (one invoice paid meanwhile):** allocate to the open
  targets, keep the rest as credit, and request a refund of the rest (status `requested`, normal
  approval, because it does not equal the payment).
- **Subject active but targets paid by another payment:** credit + refund request (`requested` unless
  the auto-approve conditions hold).
- **Setting off:** the refund is `requested` and follows `FIN.ACL.04`.
- **Blocked party:** the refund waits (`PTY.ACL.03`, Epic wl-parties).
- **Webhook replayed:** idempotent by gateway transaction id.
- **Amount mismatch:** guard fails; the webhook is logged and the accountant gets a task.

#### Must not
- Must not allocate to a cancelled booking's or rejected order's invoice.
- Must not auto-approve a refund that differs from the payment amount or goes to other details.

#### Done when
- [ ] Given a `failed` AED 250.00 payment for a booking still on hold with its invoice open, a
      verified late webhook makes it `received` and allocates it.
- [ ] Given the booking hold released on `PaymentLinkExpired` (slot gone), the AED 250.00 stays as
      credit, a refund is `approved` at once with reason "late confirmation", paid to the same
      card, the accountant is notified.
- [ ] The same holds when the booking was cancelled, or its invoice was already paid by another
      payment: refund `approved` at once.
- [ ] With the setting off, the refund is `requested`.
- [ ] A replayed webhook does nothing.

#### Out of scope
Failure and timeout (BL-PAY-02); refund failures (BL-PAY-06).

---

### BL-PAY-04 — wl-payments: record a chargeback and open the evidence task (`FIN.RUL.20`)

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-PAY-01 · BL-INV-07 · **Blocks:** BL-PAY-05, BL-EDU-06, BL-EDU-18
**Review items:** X-06

#### Goal
When the card holder disputes a gateway payment and the gateway takes the funds back, the payment
goes to `charged_back`. Its allocations are removed (invoices reopen), the gateway's dispute fee is
recorded, and the accountant gets an evidence task with the gateway's deadline.

#### Files to read
- `docs/modules/wl-payments.md` §3 (Payment field chargeback), §4 *Payment* (`charged_back`,
  `charge_back`, "Who reacts" chargeback row), §5.2a steps 1–2, §6 (`FIN.RUL.20`), §7, §10 (Chargebacks)

#### Terms
- **Chargeback:** the card holder's dispute; the gateway debits the merchant.
- **Evidence:** receipt, invoice, proof of delivery or attendance, signed documents, messages sent.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `charged_back` | error | no | The card holder disputed the payment and the gateway took the funds back; the dispute is still open |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `charge_back` | received | charged_back | system (verified gateway dispute webhook) | — | every allocation removed; event `PaymentReversed` with reason `chargeback` and the allocations (wl-invoicing `unallocate`); gateway dispute fee recorded as an outgoing payment to the gateway provider (a supplier party; category bank charges; allocated to no document); evidence task created (`FIN.RUL.20`); the staff who took the payment and the accountant notified |

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `FIN.RUL.20` | payment → `charged_back` | — | create an evidence task for the accountant with the gateway's deadline and the linked evidence; remind K days before the deadline | K = 2 |

Who reacts: "`PaymentReversed` (chargeback) | as above, plus the evidence task; a completed service
or order is not undone, the loss is decided through `accept_chargeback` or `lose_chargeback`".

#### Edge cases decided
- **Dispute fee party:** outgoing payment to the gateway provider as a
  supplier party, category bank charges, no document allocation.
- **Fee in another currency:** recorded in the settlement currency with the rate of the day.
- **Deadline:** stored as the gateway gives it; reminder at deadline − 2 days, 09:00 Asia/Dubai.
- **Webhook replayed:** idempotent by gateway dispute reference.
- **Evidence list:** links only to records the accountant may already see (same access rules).

#### Must not
- Must not undo the service, booking or order.
- Must not send evidence to the gateway automatically.

#### Done when
- [ ] Given a `received` AED 800.00 gateway payment allocated to INV-9, a dispute webhook received
      on 2027-02-01 with evidence deadline 2027-02-10 and fee AED 60.00 makes it `charged_back`,
      INV-9 reopens (`PaymentReversed` reason `chargeback`), a fee of AED 60.00 is recorded as an
      outgoing payment, the accountant gets a task due 2027-02-10 with the evidence links, the
      staff member who took it is notified.
- [ ] With that deadline (2027-02-10) and K = 2, the reminder is sent on 2027-02-08 at 09:00
      Asia/Dubai, once, and not when evidence was already submitted.
- [ ] A replayed webhook does nothing.

#### Out of scope
Outcomes (BL-PAY-05); invoice reopening (BL-INV-07).

---

### BL-PAY-05 — wl-payments: close a chargeback: won, lost or accepted (`FIN.ACL.18`)

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-PAY-04 · **Blocks:** None.
**Review items:** X-06

#### Goal
A chargeback ends in one of three ways. Won: the funds come back and the allocations are
re-applied where possible. Lost (or deadline passed without evidence): the payment is reversed and
the invoices stay open. Accepted: the accountant decides not to contest, which needs another
person's approval.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Payment* (`win_chargeback`, `lose_chargeback`,
  `accept_chargeback`), §5.2a steps 3–4, §6 (`FIN.ACL.18`, `ChargebackClosed`), §7

#### Terms
- **Approval request:** wl-kernel `PLT.APR.01`; runs the action at final approval.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `win_chargeback` | charged_back | received | system (gateway dispute won) | — | funds back; the same allocations re-applied where the targets are still open (a target written off meanwhile counts as not open), the rest kept as the party's credit, and the accountant notified (allocating it to a written-off invoice runs its `recover`); event `PaymentReceived` |
| `lose_chargeback` | charged_back | reversed | system (gateway dispute lost, or evidence deadline passed without a submission) | — | the reopened invoices stay open: collect from the party again, or write off through invoice `write_off` (wl-invoicing); accountant notified; event `ChargebackClosed` (outcome lost) |
| `accept_chargeback` | charged_back | reversed | `payments.chargeback` | reason; approval (`FIN.ACL.18`); accepting is sent to the gateway, and when the gateway rejects the call the approval request `fail`s, the payment stays `charged_back` and the accountant is notified | as `lose_chargeback`; event `ChargebackClosed` (outcome accepted) |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `FIN.ACL.18` | Accepting a chargeback (not contesting it) needs approval by a role with `payments.chargeback` other than the requester | `LIM`, `SOD` | AED 0 (always approval) |

#### Edge cases decided
- **Target written off meanwhile:** treated as not open; the amount stays as
  credit and the accountant is notified (they may allocate it, which runs invoice `recover`).
- **Target partly paid by another payment:** re-apply up to its open balance; rest as credit.
- **"Evidence submitted" recorded:** the deadline rule does not fire; only a gateway outcome closes it.
- **Gateway rejects the acceptance call:** the approval request `fail`s and
  the accountant is notified; the payment stays `charged_back`.
- **Dispute fee:** not refunded by a win unless the gateway refunds it (then recorded as incoming).

#### Must not
- Must not let the requester approve the acceptance.
- Must not close the invoices automatically after a loss.

#### Done when
- [ ] Given a `charged_back` AED 800.00 payment and INV-9 still open, a won webhook makes it
      `received`, re-allocates AED 800.00 to INV-9, emits `PaymentReceived`.
- [ ] Given INV-9 paid by another payment meanwhile, AED 800.00 stays as credit.
- [ ] Given the deadline passed with no evidence submitted, it is `reversed`, INV-9 stays open, the
      accountant is notified, `ChargebackClosed` (lost).
- [ ] User A requests `accept_chargeback`; user A cannot approve; user B approves; the gateway is
      told; `ChargebackClosed` (accepted).

#### Out of scope
Recording the chargeback (BL-PAY-04); invoice write-off (BL-INV-01).

---

### BL-PAY-06 — wl-payments: fail a refund and restore its source

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-payments Refund lifecycle · BL-PAY-01 · **Blocks:** BL-PAY-07, BL-INV-08, BL-EDU-03, BL-FNB-02, BL-PAY-22, BL-PRP-01
**Review items:** X-06

#### Goal
A refund can fail: the gateway refuses it, the card cannot receive it, or the bank returns the
transfer after it was paid. The refund goes to `failed`, the outgoing payment (if any) is
reversed, the source balance is restored, people are told and `RefundFailed` lets the source owner
restore its side (credit note reopens, payer credit balance, order or booking shows "not yet refunded").

#### Files to read
- `docs/modules/wl-payments.md` §3 (Refund: attempts), §4 *Refund* (`paid`, `failed` states,
  `fail`), §5.5a steps 1 and 4, §6 (`RefundFailed`)

#### Terms
- **Source:** what the refund comes from: payment, credit note, deposit, party credit balance,
  payer credit balance, order, booking.
- **Attempt:** one try to pay the refund (method, date, failure reason).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `paid` | success | no | Paid out (outgoing payment recorded or gateway refund confirmed); leaves only if the bank returns the transfer |
| `failed` | error | no | The gateway refused the refund, the card can no longer receive it, or the bank returned the transfer |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `fail` | approved, paid | failed | system (gateway refund declined) or `refunds.pay` (transfer returned by the bank) | failure reason | from `paid`: the outgoing payment is reversed; source balance restored; requester and accountant notified; the party told the refund is delayed; event `RefundFailed`; `FIN.RUL.21` |

Event `RefundFailed`: refund, party, amount, source (type and id: payment, credit note, deposit,
party or payer credit balance, order, booking), method, failure reason, attempt.

§5.5a step 4: "`RefundFailed` names the source, so the owner restores it: a credit note reopens
(wl-invoicing `reopen`), a payer credit balance gets the amount back, an order or booking refund is
shown on that record as not yet refunded. A deposit stays `refunded` (terminal) and is marked
*refund failed* with a link to the refund; the amount goes to the party's credit balance until the
refund is retried, paid by another method or cancelled."

#### Edge cases decided
- **Source is a deposit already `refunded` (terminal):** the deposit stays `refunded`, shows
  "refund failed" with the refund link, and the amount sits on the party's credit balance until the
  refund is retried, paid by another method or cancelled (spec, §5.5a step 4).
- **Bank return weeks later:** allowed from `paid`; the reversal is dated today (Asia/Dubai).
- **Party blocked:** no message to the party (wl-parties suppresses automatic messages).
- **Duplicate gateway webhook:** ignored per refund attempt.

#### Must not
- Must not lose the amount: it is always back on the source or the party's credit.
- Must not delete the failed attempt.

#### Done when
- [ ] Given an `approved` gateway refund of AED 500.00 from a credit note, when the gateway declines
      it, it is `failed`, the attempt stores the reason, requester and accountant are notified, the
      party is told it is delayed, `RefundFailed` names the credit note.
- [ ] Given a `paid` transfer refund returned by the bank, user A runs `fail`: the outgoing payment
      is `reversed` and the source balance restored.
- [ ] `fail` without a reason is refused.
- [ ] Given a `paid` deposit refund of AED 8,000.00 returned by the bank, the deposit stays
      `refunded` marked "refund failed" with the refund link, and the party's credit balance
      shows AED 8,000.00.

#### Out of scope
Retry, change of method, cancel (BL-PAY-07); credit note reopen (BL-INV-08).

---

### BL-PAY-07 — wl-payments: retry, change the method of, or cancel a failed refund (`FIN.RUL.21`)

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PAY-06 · **Blocks:** BL-CLN-16, BL-EDU-03, BL-EDU-18, BL-FNB-02, BL-FNB-16
**Review items:** X-06

#### Goal
A failed gateway refund is retried once after 24 hours. Otherwise the accountant pays by another
method (new bank details need a new approval) or, if the party cannot be reached or wants to keep
the credit, cancels the refund and leaves the amount on the party's credit balance.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Refund* (`cancelled` state, `retry`, `change_method`,
  `cancel`, diagram), §5.5a steps 2–3, §6 (`FIN.RUL.21`, `FIN.ACL.04`), §7

#### Terms
- **Original method:** the card or account the money came from.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `cancelled` | neutral | yes | Given up after a failure; the amount is back on the party's credit balance |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `retry` | failed | approved | `refunds.pay` or system (`FIN.RUL.21`) | same method and details; retries below the maximum | — |
| `change_method` | failed | approved, requested | `refunds.pay` | new method set: original card through the gateway, transfer, cheque or cash; target `approved` when paid to the same party by cash, cheque in the party's name or the original card, otherwise `requested` (new bank details need a new approval, `FIN.ACL.04`) | party notified of the new method |
| `cancel` | failed | cancelled | `refunds.approve` | reason (party unreachable, party asks to keep the credit) | amount stays on the party's credit balance; party notified where reachable |

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `FIN.RUL.21` | refund → `failed` | gateway refund, retries below the maximum | retry after N hours; otherwise create a task for the accountant to change the method | N = 24, maximum 1 retry |

#### Edge cases decided
- **Manual retries count:** yes, toward the same maximum (the guard applies to the transition).
- **Transfer refund failed:** no automatic retry (gateway only); the accountant gets the task at once.
- **Cash refund:** paid at the counter; the cash payment records the collector/holder (cash custody).
- **Cancel for a credit-note source:** the credit note already got the amount back (BL-INV-08).
- **Retry scheduled and the accountant changes the method first:** the scheduled retry finds the
  refund no longer `failed` and does nothing.

#### Must not
- Must not pay to new bank details without a new approval by someone other than the requester.
- Must not retry more than the maximum.

#### Done when
- [ ] Given a gateway refund `failed` at 2027-01-10 10:00, it is retried at 2027-01-11 10:00; if it
      fails again, the accountant gets a "change method" task.
- [ ] `change_method` to cheque in the party's name goes to `approved`; to a new IBAN goes to `requested`.
- [ ] `cancel` with reason "party asks to keep the credit" leaves AED 500.00 on the party's credit
      and notifies the party.
- [ ] A third retry is refused.

#### Out of scope
Failure (BL-PAY-06).

---

### BL-PAY-08 — wl-payments: refund a payer credit balance

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-INV-17 · Base: wl-payments Refund lifecycle · **Blocks:** None.
**Review items:** 05-N3-c

#### Goal
Money an insurer or payer paid in excess and that was not set off against recoupments is paid back
with a refund whose source is the payer credit balance. Its approval is the movement's approval
(`FIN.ACL.16`), and `FIN.ACL.04` also applies.

#### Files to read
- `docs/modules/wl-payments.md` §3 (Refund source), §5.5a paragraph "Balances owed between the
  business and a supplier or payer"
- `docs/modules/wl-invoicing.md` §3 (Payer credit balance), §6 (`FIN.ACL.16`)

#### Terms
- **Payer credit balance:** see BL-INV-17.

#### Spec excerpt (authoritative for this issue)
§5.5a: "A *payer credit balance* (wl-invoicing `PayerBilling`, money an insurer or payer paid in
excess) not set off against recoupments is paid back with a refund whose source is the payer credit
balance; the refund's approval is the movement's approval (`FIN.ACL.16`), and `FIN.ACL.04` applies."

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `FIN.ACL.16` | Moving a payer overpayment to the payer credit balance, or recording a payer recoupment, needs approval by a second person (a reversal from wl-payments is exempt) | `SOD`, `LIM` | any amount |
| `FIN.ACL.04` | Whoever requests or records a refund cannot approve it | `SOD` | on |

#### Edge cases decided
- **One approval:** the refund `approve` is the movement approval; no second approval request.
- **Refund above the balance:** refused (refundable balance guard).
- **Refund fails:** the payer credit balance gets the amount back (BL-PAY-06).

#### Must not
- Must not let the requester approve.

#### Done when
- [ ] Given a payer credit balance of AED 1,200.00, user A requests a refund of AED 1,200.00; user B
      approves; a payer credit balance movement "refund" of −1,200.00 is recorded with the approval.
- [ ] A refund of AED 1,300.00 is refused.

#### Out of scope
Overpayment and recoupment (BL-INV-17).

---

### BL-PAY-09 — wl-payments: record a bounced cheque with partial honour, return certificate and bounce fee

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-payments Cheque lifecycle · Epic wl-sales: credit hold (`SAL.RUL.05`) · **Blocks:** BL-PAY-10, BL-PAY-11, BL-PAY-12, BL-PAY-13, BL-PAY-14, BL-ACC-24, BL-AUT-18, BL-CON-17, BL-LOG-26, BL-MFG-14, BL-MPW-01, BL-MPW-21, BL-PRP-26
**Review items:** X-05, 01-N3-a, 08-N3-a

#### Goal
When the bank returns a received cheque, the accountant records the return reason, attaches the
bank's return certificate and enters any part the bank paid from the available balance. The
honoured part becomes a payment; the rest is the open amount still owed. A bounce fee charge is
added when configured, and `ChequeBounced` lets wl-sales put the customer (and its credit group) on
credit hold.

#### Files to read
- `docs/modules/wl-payments.md` §3 (Cheque fields), §4 *Cheque* (intro, `bounced` state,
  `deposit`, `bounce`), §5.3 step 3, §6 (`ChequeBounced`, `FIN.RUL.04`), §7 (bounce fee, credit
  hold), §10 (Cheques, Bounce fee)
- `docs/OPEN-ITEMS.md` A-05, L-21

#### Terms
- **Honoured amount:** the part the bank paid from the drawer's available balance (UAE banks must
  pay partially).
- **Open amount:** cheque amount − honoured amount − later part settlements.
- **Bank return certificate:** the bank's document; it makes the cheque an executive instrument.
- **Bounce fee charge:** a charge from the client's charge catalogue (wl-invoicing) added to the party.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `bounced` | error | no | Returned unpaid, in full or in part; the open amount is still owed |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `deposit` | received, due_for_deposit | deposited | `cheques.deposit` | cheque date reached; a cheque collected in the field is confirmed in a cash handover (`FIN.ACL.20`) | presentations + 1 |
| `bounce` | deposited | bounced | `cheques.deposit` | bank return reason recorded; bank return certificate attached (received direction); honoured amount entered when the bank paid part from the available balance (default 0) | honoured part recorded as a payment and allocated; open amount = amount − honoured; received direction: bounce fee charge added to the party when a fee charge type is set (§7), due on the bounce date on the agreement linked to the cheque, or as a standalone charge for the party when none is linked, and event `ChequeBounced` (`FIN.RUL.04`, `FIN.RUL.18`; wl-sales puts the customer account on credit hold); issued direction: none of these, the accountant gets a task to replace the cheque |

Intro: "issued cheques skip `flag_due` and the bounce follow-up (`represent`, `settle_part`,
`refer_to_legal`, `write_off`), and use `return` only for an issued cheque cancelled and recovered
before it is presented; an issued cheque that bounces is replaced. Its path is `deposited` →
`bounce` → `bounced` → `replace` → `replaced`: `bounce` records the bank return reason and any
honoured part (an outgoing payment allocated to the linked supplier bill), with no certificate,
no bounce fee and no `ChequeBounced` (so no `FIN.RUL.04`, `FIN.RUL.18` or credit hold); the
accountant gets a task to replace it. `replace` needs a new issued cheque or an outgoing payment
linked for the open amount. The linked supplier bill keeps that amount open meanwhile, since an
issued cheque records its payment only at `clear`."

Event `ChequeBounced`: cheque, party, credit group, amount, honoured amount, open amount, bank
return reason, presentation number, linked documents.

Settings (§7): *Bounce fee: charge type … and amount* — none (off); the vertical sets it (e.g. 01).
*Bounced cheque puts the customer account on credit hold (wl-sales)* — on, including the credit group.
§10: whether the fee is standard rated or out of scope is `⚠ open: accountant (A-05)`; the default
charge type uses the standard rate.

#### Edge cases decided
- **Honoured amount ≥ cheque amount:** refused (that is `clear`).
- **Honoured payment allocation:** to the cheque's linked documents, oldest first.
- **Bounce fee charge:** a scheduled charge due on the bounce date (Asia/Dubai) on the party's
  agreement linked to the cheque; when no agreement is linked, a standalone
  charge for the party.
- **Issued cheque bounces:** `bounce` with the return reason only; no certificate guard, no fee,
  no `ChequeBounced`, no credit hold; the accountant gets a task to replace it, and it ends
  `replaced` through `replace` (BL-PAY-11 transition) when a new issued cheque or outgoing payment
  covers the open amount (spec).
- **Second bounce after re-presentation:** a new fee is added each bounce (Default, spec silent).
- **Credit hold:** the event only; the hold itself is wl-sales.

#### Must not
- Must not record a received-cheque bounce without the certificate.
- Must not record the honoured part as anything but a payment.

#### Done when
- [ ] Given a deposited received cheque of AED 20,000.00 for rent, when user A records a bounce with
      reason "insufficient funds", the certificate attached and honoured AED 6,000.00, it is
      `bounced`, a payment of AED 6,000.00 is allocated to the rent invoice, open amount AED 14,000.00,
      `ChequeBounced` is emitted.
- [ ] With a 01 fee charge type of AED 500.00, a charge of AED 500.00 + VAT (standard default) is added.
- [ ] Without the certificate, `bounce` is refused for a received cheque.
- [ ] A field cheque not confirmed in a handover cannot be deposited (see BL-PAY-18).
- [ ] Given a deposited issued cheque of AED 10,500.00 for an `approved` supplier bill, `bounce`
      with reason "insufficient funds" and no certificate gives `bounced`, open amount
      AED 10,500.00, no fee, no `ChequeBounced`, a replace task for the accountant; the bill stays
      `approved`. (Its `replace` is checked in BL-PAY-11.)

#### Out of scope
Follow-up outcomes (BL-PAY-10 to BL-PAY-13); reminders (BL-PAY-14); credit hold (Epic wl-sales).

---

### BL-PAY-10 — wl-payments: re-present a bounced cheque

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:s` `blocked:open-item` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PAY-09 · **Blocks:** BL-AUT-18, BL-CON-17, BL-MFG-14, BL-MPW-03, BL-PRP-26
**Review items:** X-05

#### Goal
When the party confirms there are funds, a bounced received cheque can be deposited again, at most
the configured number of presentations, and only if the bank honoured nothing before. A cheque that
clears after a re-presentation emits `ChequeBounceResolved` (cleared).

#### Files to read
- `docs/modules/wl-payments.md` §4 *Cheque* (`represent`, `clear`), §5.3 step 4, §6
  (`ChequeBounceResolved`), §7 (maximum presentations), §10
- `docs/OPEN-ITEMS.md` L-21

#### Terms
- **Presentations:** number of deposits of the cheque.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `represent` | bounced | deposited | `cheques.deposit` | received direction; nothing honoured by the bank on an earlier presentation; presentations below the maximum (default 2), checked before the count goes up; new deposit date and the party's confirmation of funds, or a note; a part settled meanwhile (`settle_part`) does not prevent it (`clear` then records the open amount only) | presentations + 1 (with the first `deposit`, a maximum of 2 allows one re-presentation); party notified of the new presentation date |
| `clear` | deposited | cleared | `cheques.deposit` | — | payment recorded (direction in or out) for the open amount and allocated to the linked document; after a re-presentation: event `ChequeBounceResolved` (outcome cleared) |

Event `ChequeBounceResolved`: cheque, party, outcome (cleared, replaced, written off), other bounced
cheques of the party still open; wl-sales lifts the hold for cleared or replaced when none is left.
§10: whether a partly honoured cheque may be presented again is `⚠ open: lawyer (L-21)`; the
default forbids it.

#### Edge cases decided
- **Presentations count:** `represent` adds 1, checked before the increment (spec). First deposit
  → 1; `represent` (1 < 2) → 2; a second `represent` (2 < 2) is refused. Maximum 2 therefore allows
  one re-presentation.
- **Part settled (`settle_part`) before re-presenting:** allowed; `clear`
  records a payment for the open amount only.
- **Other bounced cheques still open:** the event lists them; wl-sales keeps the hold.

#### Must not
- Must not re-present a partly honoured cheque or an issued cheque.

#### Done when
- [ ] Given a bounced cheque (presentations 1, honoured 0), `represent` with date 2027-02-05 and a
      note "funds confirmed by phone" makes it `deposited`, presentations 2, the party is notified.
- [ ] A second `represent` is refused (maximum 2).
- [ ] With honoured AED 6,000.00, `represent` is refused.
- [ ] `clear` after re-presentation records a payment for the open amount and emits `ChequeBounceResolved` (cleared).

#### Out of scope
Recording the bounce (BL-PAY-09).

---

### BL-PAY-11 — wl-payments: settle part of a bounced cheque and replace it

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PAY-09 · **Blocks:** BL-MPW-03
**Review items:** X-05

#### Goal
The party may pay part of a bounced cheque's open amount in cash, by transfer or card; the open
amount goes down and the cheque stays where it is. When replacement cheques or payments cover the
open amount, the cheque is `replaced`, any legal case task is closed, and the original is handed
back once the replacement clears.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Cheque* (`settle_part`, `replace`), §4 *Payment* (`record`
  By note), §5.3 step 4, §6 (`FIN.ACL.02`, `ChequeBounceResolved`)

#### Terms
- **Replacement:** new cheques or payments linked to the bounced cheque.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `replaced` | neutral | yes | Open amount settled by other cheques or payments (`FIN.ACL.02`) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `settle_part` | bounced, legal_case | bounced, legal_case (unchanged) | `cheques.manage` | received direction; payment (cash, transfer, card) below the open amount | payment recorded and allocated to the cheque's linked documents; open amount reduced |
| `replace` | bounced, legal_case | replaced | `cheques.manage` | replacement cheques or payments linked and covering the open amount (`FIN.ACL.02`) | legal case task closed; original cheque kept until the replacement clears, then handed back with a receipt (if a replacement cheque bounces, the original stays `replaced` and is kept with a note; the new cheque follows its own bounce flow); event `ChequeBounceResolved` (outcome replaced) |

Payment `record` guard note: "cheque payments are created by the cheque `clear` and `settle_part` transitions".

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `FIN.ACL.02` | A bounced cheque can be marked replaced only by linking the replacement cheque or payment | `STA` | — |

#### Edge cases decided
- **Payment equal to the open amount:** use `replace` with that payment (settle_part needs "below").
- **Replacement cheque later bounces:** the original is not handed back; the new cheque follows its
  own bounce flow. the original stays `replaced` and is kept with a note.
- **Hand-back:** a task when the replacement clears; the receipt is generated then.
- **Issued cheque:** `replace` is the only way out of `bounced` (spec §4 *Cheque* intro); the
  replacement is a new issued cheque or an outgoing payment to the payee.

#### Must not
- Must not mark `replaced` without linked replacements covering the open amount.

#### Done when
- [ ] Given a `bounced` cheque with open AED 14,000.00, a cash `settle_part` of AED 4,000.00 records
      and allocates a payment; open AED 10,000.00; state stays `bounced`.
- [ ] On that cheque (open AED 10,000.00), `settle_part` of AED 10,000.00 is refused.
- [ ] Given a cheque in `legal_case` with open AED 10,000.00 and an open case-owner task, linking a
      new cheque of AED 10,000.00 and running `replace` gives `replaced`, closes the legal task,
      emits `ChequeBounceResolved` (replaced); when the new cheque clears, a hand-back task and
      receipt follow.
- [ ] Given open AED 10,000.00, linking a new cheque of AED 9,000.00 only, `replace` is refused.
- [ ] Given a `bounced` issued cheque of AED 10,500.00 (BL-PAY-09), linking an outgoing transfer of
      AED 10,500.00 and running `replace` gives `replaced`; `settle_part` on it is refused
      (received direction only).

#### Out of scope
Legal case (BL-PAY-12); write-off (BL-PAY-13).

---

### BL-PAY-12 — wl-payments: refer a bounced cheque to a legal case

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:s` `blocked:open-item` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PAY-09 · **Blocks:** BL-PRP-13
**Review items:** X-05, 01-N3-b (cheque side)

#### Goal
Usually after the `FIN.RUL.18` reminders (practice, not a guard: spec §5.3 step 4), the
accountant hands the open amount to a legal case: civil execution by default,
or a criminal complaint only for return reasons on the configured criminal-grounds list. The case
owner gets a task with an evidence pack; the party gets a formal notice. The platform records the
case; it files nothing.

#### Files to read
- `docs/modules/wl-payments.md` §3 (Cheque: legal case), §4 *Cheque* (`legal_case` state,
  `refer_to_legal`), §5.3 step 4, §6 (`cheques.legal`), §7 (criminal grounds, minimum amount), §10 (Cheques)
- `docs/OPEN-ITEMS.md` L-21

#### Terms
- **Civil execution:** enforcement of the returned cheque as an executive instrument.
- **Criminal grounds list:** return reasons that still allow a criminal complaint (setting). Until
  L-21 is answered the default is: account closed, payment stopped by the drawer, signature or
  alteration irregular (§7, §10).
- **Case owner:** the user named at referral (default: the user who refers), stored on the
  cheque's legal case; gets the follow-up task and the evidence pack (spec §3, §5.3).
- **Formal notice (default template, §10, until L-21):** drawer's name; cheque number, bank,
  date and amount; return reason and date; honoured and open amounts; bounce fee if any; a demand
  to pay the open amount within 7 days; the business's contact. The client may edit it.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `legal_case` | error | no | Open amount handed to a legal case (civil execution or criminal complaint) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `refer_to_legal` | bounced | legal_case | `cheques.legal` | received direction; bank return certificate attached; case type set: `civil_execution` (default) or `criminal_complaint`, which is allowed only when the bank return reason is in the criminal-grounds list (§7, §10); open amount above the minimum for legal action; case owner named (default: the user who refers) | `FIN.RUL.18` reminders stop; follow-up task for the case owner with the evidence pack (cheque image, return certificate, linked agreement and invoices, notices sent); formal notice to the party (template); manager notified |

Settings (§7): criminal grounds — account closed, payment stopped by the drawer, signature or
alteration irregular `⚠ open: lawyer (L-21)`; minimum open amount for a legal case — AED 0 (any).

#### Edge cases decided
- **Minimum AED 0:** "above the minimum" means open amount > 0.
- **Reminders not yet sent:** referral is still allowed (no guard on reminders, spec).
- **Formal notice template:** the §10 default above, marked "content pending L-21"; the client edits it.
- **Return reason edited after referral:** not allowed (reason is part of the bounce record).

#### Must not
- Must not allow `criminal_complaint` for "insufficient funds".
- Must not file or send anything to a court or the police.

#### Done when
- [ ] Given a bounced cheque (reason "insufficient funds", open AED 10,000.00), `refer_to_legal` by
      user A with `civil_execution` and no case owner named makes it `legal_case` with case owner
      A, stops reminders, creates A's task with the evidence pack, sends the notice carrying the
      cheque number, return reason, open amount AED 10,000.00 and the 7-day demand, and notifies
      the manager.
- [ ] Naming user C as case owner gives the task to C.
- [ ] Referral one day after the bounce (before the 3-day reminder) is allowed.
- [ ] With the default criminal-grounds list, `criminal_complaint` for that cheque is refused; for
      reason "account closed" it is allowed.
- [ ] A user without `cheques.legal` cannot refer.

#### Out of scope
Settlement or write-off from `legal_case` (BL-PAY-11, BL-PAY-13).

---

### BL-PAY-13 — wl-payments: write off a bounced cheque through the invoice write-off

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PAY-09 · BL-INV-01 · **Blocks:** BL-MPW-03
**Review items:** X-05

#### Goal
When the open amount cannot be collected, the cheque is written off by writing off the linked
invoices through wl-invoicing `write_off` (with its approval). The cheque moves to `written_off` when
`InvoiceWrittenOff` arrives; records are kept as evidence and the credit hold stays until wl-sales releases it.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Cheque* (`written_off` state, `write_off`), §5.3 step 4, §6
  (Events consumed `InvoiceWrittenOff`, `ChequeBounceResolved`)
- `docs/modules/wl-invoicing.md` §4 *Invoice* (`write_off`)

#### Terms
- **Linked charge not yet invoiced:** a scheduled charge the cheque covered that has no invoice yet.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `written_off` | neutral | yes | Open amount written off as bad debt on the linked invoices |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `write_off` | bounced, legal_case | written_off | `cheques.manage` | reason; a linked charge not yet invoiced is invoiced first; the open balances of the linked invoices together equal the cheque's open amount (invoice `write_off` always takes an invoice's whole open balance; otherwise staff write off the invoices directly and settle the cheque another way); the open amount written off on the linked invoices through invoice `write_off` in wl-invoicing, one request per invoice (its approval limit applies) | the cheque moves to `written_off` when `InvoiceWrittenOff` has arrived for every linked invoice; a request rejected or failed leaves the cheque in its state (write-offs already approved stay) and the accountant is notified; cheque, certificate and case records kept as evidence; credit hold stays until released in wl-sales; event `ChequeBounceResolved` (outcome written off) |

Events consumed: "`InvoiceWrittenOff` | wl-invoicing | completes a cheque `write_off` waiting for
the invoice write-off approvals, once every linked invoice is written off".

§5.3 step 4 (Write-off): "This is offered only when the linked invoices' open balances equal the
cheque's open amount; otherwise the invoices are written off directly."

#### Edge cases decided
- **Invoice balance differs from the cheque's open amount:** cheque `write_off` is refused (spec)
  unless the linked invoices' open balances together equal the cheque's open amount, compared
  after any linked charge not yet invoiced has been invoiced. The message tells the user to write
  off the invoices directly instead.
- **Several linked invoices:** one invoice write-off request each; the cheque moves to
  `written_off` when every linked invoice is written off; if one request is rejected or fails, the
  cheque stays and the accountant is notified (approved write-offs stay) (spec).
- **Linked invoice disputed:** invoice write-off refused → cheque write-off cannot start.
- **Party pays while waiting:** the invoice request fails (balance changed); cheque stays.

#### Must not
- Must not mark the cheque `written_off` before the invoice write-offs are approved.
- Must not lift the credit hold.

#### Done when
- [ ] Given a bounced cheque open AED 10,000.00 linked to INV-5 balance AED 10,000.00, `write_off`
      with reason "untraceable" creates an invoice write-off request; when user B approves it and
      `InvoiceWrittenOff` arrives, the cheque is `written_off` and `ChequeBounceResolved` (written off) is emitted.
- [ ] Given a linked charge not yet invoiced, the charge is invoiced first, then written off.
- [ ] When the invoice request is rejected, the cheque stays `bounced`.
- [ ] Given open AED 10,000.00 linked to INV-5 with balance AED 12,500.00, cheque `write_off` is
      refused with a message to write off the invoice directly.
- [ ] Given open AED 10,000.00 linked to INV-5 (AED 6,000.00) and INV-6 (AED 4,000.00), two requests
      are created; after only INV-5 is approved the cheque is still `bounced`; after INV-6 it is `written_off`.

#### Out of scope
The invoice write-off (BL-INV-01).

---

### BL-PAY-14 — wl-payments: chase a bounced cheque (`FIN.RUL.18`)

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PAY-09 · **Blocks:** BL-ACC-24, BL-AUT-18, BL-CON-17, BL-MFG-14, BL-PRP-26
**Review items:** X-05, 01-N3-a

#### Goal
A bounced cheque must not be forgotten. After N days the party (with the open amount and bounce
fee) and the responsible staff are reminded; after M days the accountant and manager get a task to
re-present, refer to legal or write off.

#### Files to read
- `docs/modules/wl-payments.md` §6 (`FIN.RUL.18`, `FIN.RUL.04`), §7

#### Terms
- **Responsible staff:** the user who owns the cheque's linked agreement or collected the cheque.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `FIN.RUL.18` | N and M days after cheque → `bounced` | still `bounced` | at N: remind the party (notice with the open amount and the bounce fee; not sent while the party is blocked, wl-parties §5.6) and the responsible staff; at M: task to the accountant and manager to re-present, refer to legal or write off | N = 3, M = 14 |

Setting (§7): *Bounced cheque reminders (`FIN.RUL.18`)* — party and staff at 3 days; decision task at 14 days.

#### Edge cases decided
- **Days counted from the bounce date** in Asia/Dubai. Bounce 2027-02-01 → reminder 2027-02-04, task 2027-02-15.
- **Re-presented and bounced again:** counting restarts from the new bounce.
- **`settle_part` meanwhile:** still `bounced`, reminders continue with the new open amount.
- **Party blocked:** the party reminder is suppressed (no automatic messages
  to a blocked party, wl-parties §5.6); staff still get theirs.

#### Must not
- Must not remind once the cheque is `legal_case`, `replaced`, `written_off` or cleared.

#### Done when
- [ ] Given a bounce on 2027-02-01 with open AED 14,000.00 and fee AED 500.00, the party notice on
      2027-02-04 shows both; the task is created on 2027-02-15.
- [ ] Given `refer_to_legal` on 2027-02-10, no task on 2027-02-15.

#### Out of scope
`FIN.RUL.04` (unchanged base).

---

### BL-PAY-15 — wl-payments: dispute deposit deductions before the refund

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-payments Deposit lifecycle · Epic wl-portal: deductions shown to the party · **Blocks:** BL-PRP-12, BL-PRP-13, BL-PRP-16, BL-PRP-17, BL-PRP-26
**Review items:** 01-N4-c

#### Goal
After deposit deductions are published in the portal, the party may dispute lines within the
dispute window (setting per vertical, off by default, on for 01). While disputed, `refund` and
`forfeit` wait. Staff answer with evidence and either revise the deductions (new version, window
restarts) or uphold them.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Deposit* (`refund`, `forfeit`, "Deductions dispute flag"),
  §5.4 steps 2–4, §7 (deductions dispute setting)
- `docs/verticals/01-property.md` (`PRP.RUL.11`)

#### Terms
- **Dispute window:** days after publication in which the party may dispute (default 14).
- **Response window:** days staff have to answer (default 7).
- **Published version:** a numbered version of the deductions shown to the party.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `refund` | settlement_pending | refunded | `deposits.refund` | deductions itemised with evidence; deductions published to the party in the portal before payment (01 §13 question 5); when the deductions dispute is on for the vertical: the dispute window of the latest published version has passed and no deductions dispute is open (every disputed line revised or upheld); the vertical's limit and SoD rules pass (e.g. `PRP.ACL.05`, `FIN.ACL.04`), otherwise an approval request | refund paid by transfer or issued cheque; deductions invoiced if billable; settlement statement generated |
| `forfeit` | settlement_pending | forfeited | `deposits.refund` | reason; approval; no deductions dispute open; when the deductions dispute is on for the vertical: the forfeiture published to the party as the deductions (the itemised deductions it is applied to, e.g. 01 *applied to deductions*, or one line for the whole deposit with the reason) and the dispute window of the latest published version passed, as for `refund` | settlement statement generated |

"**Deductions dispute flag** (not a state, setting per vertical, off by default). After the
deductions are published, the party may dispute one or more lines in the portal within the dispute
window. While the flag is set, `refund` and `forfeit` wait. The staff member responsible answers
within the response window with evidence, revises the deductions (a new published version; the
window starts again) or upholds them, which clears the flag without restarting the window.
`refund` and `forfeit` run only after the window has passed, even when no line was disputed, so
the party always has the whole window. A legal case opened on the source agreement keeps the flag until the
decision. The vertical's rule escalates an unanswered dispute (01: `PRP.RUL.11`)."

Setting (§7): *Deductions dispute by the party in the portal: on/off; dispute window; response
window* — off; 14 days; 7 days (01: on).

#### Edge cases decided
- **Window and dispute both checked:** with the setting on, `refund` and `forfeit` need the window
  of the latest published version to have passed and no dispute open (spec); neither pre-empts the
  window. A forfeit is first published as the deductions (the itemised deductions it is applied
  to, or one line for the whole deposit with the reason), so the party can dispute it. With the
  setting off, there is no window.
- **Window end:** 23:59 Asia/Dubai of the last day. Published 2027-03-01 → window ends 2027-03-15 23:59.
- **Revision:** new version; the old version kept read-only; the window starts again from the new
  publication.
- **Uphold:** clears the flag; no new window (spec).
- **Dispute after the window:** refused in the portal.
- **Legal case on the agreement:** flag stays until a staff member records the decision.

#### Must not
- Must not refund or forfeit while the flag is set.
- Must not change a published version; revise into a new one.

#### Done when
- [ ] Given deductions AED 2,500.00 published 2027-03-01 (01, setting on), when the tenant disputes
      the "repainting" line on 2027-03-05, the flag is set and `refund` is refused.
- [ ] Staff revise repainting to AED 800.00 on 2027-03-10: version 2 is published on 2027-03-10,
      the window restarts and ends 2027-03-24 23:59; `refund` is refused on 2027-03-24 and
      allowed on 2027-03-25.
- [ ] Staff uphold instead: the flag clears and `refund` is allowed once the window has passed.
- [ ] With the setting off (other verticals), no dispute button appears and `refund` needs no window.
- [ ] With the setting on and no line disputed, `refund` is refused on 2027-03-05 and allowed on
      2027-03-16; after it the portal shows no dispute button.
- [ ] Staff uphold on 2027-03-08 (inside the window): the flag clears, the window still ends
      2027-03-15 and `refund` is allowed from 2027-03-16.
- [ ] With the setting on, a forfeiture of the whole deposit published 2027-03-01 is refused
      (`forfeit`) on 2027-03-10 and allowed on 2027-03-16 when no dispute is open; a forfeit not
      published to the party is refused.

#### Out of scope
Escalation of unanswered disputes (`PRP.RUL.11`, Epic 01 Property); deposit transfer (BL-PAY-16).

---

### BL-PAY-16 — wl-payments: transfer a deposit to a new holder

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-payments Deposit lifecycle · **Blocks:** BL-PRP-22, BL-PRP-23
**Review items:** 01-N12-c

#### Goal
When the source agreement passes to another holder before it ends (01 management agreement `end`
or `transfer_unit`), the deposit is handed over against a receipt from the new holder; the liability
leaves the client and the party is told who now holds it.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Deposit* (`transferred` state, `transfer`), §5.4 step 5, §6 (`DepositSettled`)
- `docs/verticals/01-property.md` (management agreement)

#### Terms
- **Holder:** the business that holds the deposit (the client, a new manager or owner).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `transferred` | neutral | yes | Handed over with the source agreement to another holder (new manager or owner) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `transfer` | held | transferred | `deposits.manage` | the source agreement is handed to another holder (e.g. 01 management agreement `end` or `transfer_unit`); receipt from the new holder attached | deposit liability leaves the client; the money passed to the new holder is recorded as an outgoing payment to the new holder with the receipt as reference (not a refund); the party told who now holds the deposit; event `DepositSettled` (outcome transferred) |

#### Edge cases decided
- **Deposit in `settlement_pending`:** not transferable (spec: from `held` only); the settlement completes first.
- **Payment of the deposit to the new holder:** recorded as an outgoing
  payment to the new holder with reference to the receipt; not a refund.
- **Party blocked:** the transfer is not money out to the party, so it is allowed.

#### Must not
- Must not transfer without the new holder's receipt.

#### Done when
- [ ] Given a `held` deposit AED 10,000.00 and a management agreement ended, `transfer` with the new
      holder's receipt makes it `transferred`, notifies the tenant, emits `DepositSettled` (transferred).
- [ ] Without a receipt, refused.

#### Out of scope
Management agreement lifecycle (Epic 01 Property).

---

### BL-PAY-17 — wl-payments: count, recount, recover and write off a cash shortage (`FIN.ACL.19`)

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:m` `blocked:open-item` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: wl-payments Cash handover lifecycle · Epic wl-hr: cash shortage recovery adjustment · **Blocks:** BL-PAY-18, BL-PAY-19, BL-PAY-20, BL-PAY-21, BL-CLN-16, BL-EDU-18, BL-FNB-16, BL-LOG-10, BL-LOG-13, BL-LOG-26, BL-PAY-23
**Review items:** X-07, D-12, 03-N9-a, 05-N11-a

#### Goal
Cash custody now covers any holder in per-person mode (driver, rider, receptionist, counsellor,
sales rep) with a handover per run, shift or day. Another user counts. A short count can be
recounted by a third user, then recovered from the holder (cash, or salary deduction only with
`FIN.ACL.19`, off by default) or written off with approval as a loss of the holder's branch.

#### Files to read
- `docs/modules/wl-payments.md` §2 (`FIN.CSH.01`), §3 (Cash holder, Cash handover), §4 *Cash
  handover* (modes, states, `count`, `recount`, `recover`, `write_off`, vertical mapping), §5.5
  steps 1–3, §6 (`FIN.ACL.05`, `FIN.ACL.06`, `FIN.ACL.19`, `CashShortage`), §7, §10 (Cash shortages from wages)
- `docs/OPEN-ITEMS.md` L-20

#### Terms
- **Holder:** a staff user or till holding cash.
- **Mode:** per person or till session (setting per holder).
- **Expected amount:** sum of cash payments the holder recorded in the period.
- **Overage:** counted more than expected; recorded as a separate line, and the state is
  `counted` (spec: "equals or exceeds expected").
- **Responsible person:** the holder, or for a till session its cashier; recoveries are taken
  from this person.
- **Part recovery:** a recovery recorded on a `short` handover without a state change (spec
  §4 *Cash handover*, "Part recoveries").

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `open` | warning | no | The holder has cash or cheques collected in the period |
| `counted` | success | yes | Counted amount equals or exceeds expected (an overage is recorded as a separate line) and every expected cheque is present |
| `short` | error | no | Counted less than expected, or an expected cheque is missing |
| `recovered` | success | yes | Shortage recovered from the holder |
| `written_off` | neutral | yes | Shortage written off with approval |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `count` | open | counted, short | `cash.count`, or system (`TillSessionClosed`, till session mode) | counter ≠ holder (`FIN.ACL.05`; for a till, the session's close and review in wl-sales); each expected cheque ticked present or missing | cash moved to the safe or bank deposit list; overages recorded as a separate line and held as a liability, not income, until the accountant decides (§12 question 4); cheques ticked present get custody confirmed and can be deposited; on `short`: event `CashShortage`, holder and their manager notified |
| `recount` | short | counted, short | `cash.count` | counter ≠ holder and ≠ the first counter; the holder present or notified | difference updated; a cheque found is confirmed |
| `recover` | short | recovered | `cash.count` | missing cheques found (confirmed) or returned as lost; recoveries recorded on the handover cover the whole difference: cash from the responsible person (the holder, or a till session's cashier), or a deduction passed to wl-hr as an adjustment of type cash shortage recovery (`FIN.ACL.19`) | — |
| `write_off` | short | written_off | `cash.write_off` | missing cheques found or returned as lost; the rest (difference − recoveries recorded) below the write-off limit (`FIN.ACL.06`, from `LOG.ACL.06`), otherwise approval | the rest posted as a loss of the holder's outlet or branch; recoveries recorded stay shown on the handover |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `FIN.ACL.19` | Recovering a cash shortage through a salary deduction needs approval by someone other than the counter and the holder's acknowledgement, and stays within the wl-hr deduction caps | `SOD`, `LIM` | off: salary deduction is not offered until the client turns it on (§7, `⚠ open: lawyer (L-20)`); when on, the approval and acknowledgement always apply |
| `FIN.ACL.05` | Cash is counted by someone other than its holder | `SOD` | on |
| `FIN.ACL.06` | A cash shortage above a limit is written off only with approval | `LIM` | AED 0 (03: AED 50) |

Vertical mapping (per-person rows): 03 driver per run; 05 receptionist per shift or day at each
front desk (MVP); 06 own rider per shift for cash on delivery of direct orders (`FIN.CSH.01`, MVP);
08 delivery drivers and sales reps per day, cheques only (MVP); 10 admissions counsellor per day,
cash and cheques (MVP).

Part recoveries (§4): "While a handover is `short`, each part recovered (cash from the
responsible person, or a wl-hr deduction under `FIN.ACL.19`) is recorded on the handover without
a state change. `recover` runs when the recoveries cover the whole difference; `write_off` writes
off only what is left after them."

Settings (§7): holders — staff: per person, per day; tills: till session. *Shortage recovery by
salary deduction (`FIN.ACL.19`)* — off (recovery in cash only) `⚠ open: lawyer (L-20)`. *Write-off
limit for cash shortages* — AED 0 (always approval).

#### Edge cases decided
- **Overage:** state `counted` (spec: equals or exceeds expected); the overage line is kept and
  held as a liability until the accountant decides (spec; treatment is §12 question 4, accountant).
- **Partial recovery:** each part is recorded on the `short` handover with no transition (spec);
  `recover` needs the recoveries to cover the whole difference; otherwise `write_off` writes off
  the rest (difference − recoveries), its limit applying to that rest, and the recoveries stay
  shown.
- **`FIN.ACL.19` default:** off (spec §6 and §7 agree): salary deduction is not offered until the
  client turns it on; when on, approval by someone other than the counter and the holder's
  acknowledgement always apply.
- **Salary deduction while the setting is off:** the option is hidden and refused.
- **"Holder present or notified":** the recount records which.
- **Period boundaries:** a cash payment belongs to the handover of the period in which it was
  recorded (Asia/Dubai business time; late entries keep their business time).

#### Must not
- Must not let the holder count or recount their own cash.
- Must not let the first counter recount.
- Must not deduct from salary when `FIN.ACL.19` is off.

#### Done when
- [ ] Given receptionist R with AED 2,300.00 expected on 2027-04-01, counter C counts AED 2,250.00:
      `short`, `CashShortage` (AED 50.00), R and R's manager notified.
- [ ] R cannot count; C cannot recount; counter D recounts AED 2,300.00 → `counted`.
- [ ] With AED 50.00 short, `recover` with cash AED 50.00 → `recovered`.
- [ ] `write_off` of AED 50.00 (limit AED 0) needs approval; after approval the loss is posted to R's branch.
- [ ] With AED 50.00 short, a part recovery of AED 20.00 cash is recorded and the handover stays
      `short`; `recover` is refused; `write_off` of the rest asks approval for AED 30.00 and, after
      approval, is `written_off` with AED 30.00 posted as a loss and AED 20.00 shown recovered.
- [ ] Given counted AED 2,320.00 against AED 2,300.00 expected, the handover is `counted` with an
      overage line of AED 20.00 held as a liability.
- [ ] With the default setting (salary deduction off), the deduction option is not offered and an
      API call is refused.
- [ ] After the client turns salary deduction on, a deduction needs approval by someone other
      than C and R's acknowledgement, and is passed to wl-hr.

#### Out of scope
Cheques in handovers (BL-PAY-18); till sessions (BL-PAY-19); run hand-over (BL-PAY-20); escalation
(BL-PAY-21); the wl-hr deduction (Epic wl-hr).

---

### BL-PAY-18 — wl-payments: confirm cheques collected in the field in a handover (`FIN.ACL.20`)

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:m` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** BL-PAY-17 · **Blocks:** BL-AUT-18
**Review items:** X-07, 08 MEDIUM "cheques in the field handover"

#### Goal
Cheques collected by drivers, sales reps or desk staff are expected in the holder's handover. The
counter ticks each present or missing; present cheques get custody confirmed and only then can be
deposited. A missing cheque is never written off as cash: it is searched for, or returned as lost
and the party asked for a replacement.

#### Files to read
- `docs/modules/wl-payments.md` §3 (Cheque: collected by, custody confirmed), §4 *Cheque*
  (`deposit`, `return`), §4 *Cash handover* (`count`, `recover`, `write_off` cheque guards, paragraph
  "A missing cheque is not a cash shortage"), §5.3 step 6, §6 (`FIN.ACL.20`), §7

#### Terms
- **Expected cheques:** received cheques the holder recorded in the period.
- **Custody confirmed:** the handover that confirmed the cheque.

#### Spec excerpt (authoritative for this issue)
From the rows in BL-PAY-17: `count` guard "each expected cheque ticked present or missing"; effect
"cheques ticked present get custody confirmed and can be deposited"; `recount` effect "a cheque
found is confirmed"; `recover` and `write_off` guard "missing cheques found (confirmed) or returned as lost".

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `deposit` | received, due_for_deposit | deposited | `cheques.deposit` | cheque date reached; a cheque collected in the field is confirmed in a cash handover (`FIN.ACL.20`) | presentations + 1 |
| `return` | received, due_for_deposit | returned | `cheques.manage` or system (agreement ended) | not deposited | handover receipt generated; a cheque lost in custody is returned with reason "lost" and the party asked for a replacement |

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `FIN.ACL.20` | A cheque collected in the field (its collector is a cash holder in per-person mode; cheques entered at the office are not) is deposited only after a cash handover, counted by someone other than the collector, confirms it | `STA`, `SOD` | on |

"A missing cheque is not a cash shortage: it is never written off as cash. The accountant asks the
party whether the cheque was handed over; if it is lost, it is `return`ed with reason "lost" and the
party is asked for a replacement (a stop-payment on the lost cheque is the party's decision)."

#### Edge cases decided
- **Cheque-only holder (08):** expected cash 0; the handover is `counted` when every cheque is present.
- **"Collected in the field":** any cheque with a "collected by" user who is
  a cash holder in per-person mode; cheques entered at the office by the accountant are not.
- **Missing cheque found later:** `recount` confirms it.
- **Setting off:** cheques can be deposited without confirmation.

#### Must not
- Must not deposit an unconfirmed field cheque.
- Must not write off a missing cheque as cash.

#### Done when
- [ ] Given sales rep S collected cheques #101 (AED 5,000.00) and #102 (AED 3,000.00), counter C ticks
      #101 present, #102 missing: `short`; #101 can be deposited, #102 cannot.
- [ ] `write_off` is refused until #102 is found or returned as lost.
- [ ] #102 returned with reason "lost": the party is asked for a replacement; then `recover` or `write_off` is allowed.

#### Out of scope
Cash counting (BL-PAY-17); cheque bounce (BL-PAY-09).

---

### BL-PAY-19 — wl-payments: create a till's handover from a closed till session

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-PAY-17 · Epic wl-sales: till session `close` (`SAL.POS.01`) · **Blocks:** BL-FNB-15, BL-FNB-16
**Review items:** X-07, D-12

#### Goal
A till's cash is counted in the wl-sales till session. When the session closes, the system creates
the till's handover with the session's expected and counted cash and the session's cashier as the
responsible person, and runs `count`. A shortage is then recovered or written off here.

#### Files to read
- `docs/modules/wl-payments.md` §2 (paragraph under the capability table), §4 *Cash handover*
  (mode "Till session", `count`, vertical mapping rows 06, 08), §5.5 step 1, §6 (Events consumed
  `TillSessionClosed`, `CashShortage`)

#### Terms
- **Till session:** wl-sales period of a till with expected and counted cash, closed and reviewed there.
- **Holder:** the till (spec: a cash holder is a staff user or a till); the handover's holder is
  the till, not the cashier.
- **Responsible person:** the session's cashier (spec field "responsible person: the holder, or
  the cashier of a till session"); recoveries are taken from this person.

#### Spec excerpt (authoritative for this issue)
"**Till session:** a till counted in the wl-sales till session (`SAL.POS.01`). The session `close`
already records expected and counted cash and approves the variance there; the handover is created
by the system from `TillSessionClosed` with those amounts and the session's cashier as the
responsible person. The shortage is then treated here."

Events consumed: "`TillSessionClosed` | wl-sales | create the till's cash handover (till session
mode) and run `count` with the session's expected and counted cash".

`count` By: "`cash.count`, or system (`TillSessionClosed`, till session mode)"; guard "counter ≠
holder (`FIN.ACL.05`; for a till, the session's close and review in wl-sales)".

Event `CashShortage`: holder, responsible person, period, till session (if any), difference, missing cheques.

#### Edge cases decided
- **Event received twice:** one handover per till session.
- **Overage:** recorded as a separate line (BL-PAY-17 default).
- **Recovery from the cashier:** `recover`/`write_off` as in BL-PAY-17; the cash recovery or
  salary deduction is taken from the responsible person (cashier K), while the holder stays the
  till (spec).
- **`count` guard "counter ≠ holder":** for a till it is met by the session's close and review in
  wl-sales (spec); no user counts here.

#### Must not
- Must not ask a user to count the till again here.

#### Done when
- [ ] Given till session TS-7 of till T1 closed with expected AED 4,000.00, counted AED 3,950.00,
      cashier K, a handover is created with holder T1 and responsible person K, and is `short`;
      `CashShortage` names T1 as holder, K as responsible person, TS-7 and AED 50.00.
- [ ] `recover` with AED 50.00 cash records it as received from K.
- [ ] Given counted = expected, the handover is `counted`.
- [ ] A duplicate event creates nothing.

#### Out of scope
Till session in wl-sales (Epic wl-sales).

---

### BL-PAY-20 — wl-payments: move cash between drivers on a run hand-over

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-PAY-17 · Epic wl-delivery: run `hand_over` (`RunHandedOver`) · **Blocks:** None.
**Review items:** X-12, X-07

#### Goal
When a run is handed over mid-route, the receiving driver confirms in the app the cash counted and
each cheque received. That cash and those cheques leave the giver's expected amount and join the
receiver's open handover; any difference stays on the giver's handover and shows as a shortage at its count.

#### Files to read
- `docs/modules/wl-payments.md` §4 *Cash handover* (`hand_over`), §6 (Events consumed `RunHandedOver`)
- `docs/modules/wl-delivery.md` (run `hand_over`)

#### Terms
- **Giver / receiver:** the driver handing the run over and the one taking it.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `hand_over` | open | open | system (`RunHandedOver` from wl-delivery) | the receiving holder (≠ the giving holder) confirmed in the driver app the cash counted and each cheque received | counted cash and cheques leave the giver's expected amount and join the receiving holder's open handover (one is opened for the receiver's current period when none is open), each payment keeping its collector; any difference between what the giver should hand over and what was counted stays on the giver's handover and shows at its `count` as a shortage; when the receiver confirms more than the giver's expected amount, the giver's whole expected amount moves and the excess is recorded on the receiver's handover as an overage line from the hand-over (added to its expected cash, held as a liability like a counting overage, §12 question 4), so the giver's handover keeps nothing for it; both drivers and the dispatcher notified |

#### Edge cases decided
- **Receiver has no open handover:** one is opened for the receiver's current period.
- **Event twice:** idempotent per run hand-over id.
- **Cheques not confirmed by the receiver:** they stay expected on the giver's handover.
- **Receiver confirms more than the giver's expected:** the giver's whole expected amount moves;
  the excess goes on the receiver's handover as an overage line from the hand-over, added to its
  expected cash and held as a liability; the giver's handover keeps nothing (spec).

#### Must not
- Must not change the collector of a payment.
- Must not let a driver hand over to themselves.

#### Done when
- [ ] Given driver G with AED 1,200.00 expected and receiver H confirming AED 1,150.00, H's handover
      expects AED 1,150.00, G's keeps AED 50.00 expected, both drivers and the dispatcher are notified.
- [ ] At G's `count` with AED 0 counted, G is `short` AED 50.00.
- [ ] Given G with AED 1,200.00 expected and H confirming AED 1,230.00, H's handover expects
      AED 1,230.00 including an overage line of AED 30.00 from the hand-over; G's handover expects
      AED 0.00.

#### Out of scope
Run abort and hand-over in wl-delivery.

---

### BL-PAY-21 — wl-payments: escalate a handover that is not handed over (`FIN.RUL.22`)

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-PAY-17 · **Blocks:** BL-LOG-13, BL-LOG-26
**Review items:** 03-N9-a, X-07

#### Goal
A holder who does not come back at the end of the run, shift or day must not leave cash unaccounted
for. After N hours the manager gets a task to reach the holder; after M hours the company admin is
notified and the manager counts the handover at what was actually received, so the full difference
becomes a shortage. Money owed to third parties (03 COD remittances) is not reduced.

#### Files to read
- `docs/modules/wl-payments.md` §5.5 step 4, §6 (`FIN.RUL.22`), §7 (escalation setting)
- `docs/verticals/03-logistics.md` (`LOG.RUL.13`)

#### Terms
- **End of the period** (spec `FIN.RUL.22`): run: the earlier of its `return_to_depot` or `abort`
  time and its planned end (planned start + estimated duration, wl-delivery); shift: its planned
  end; day: 23:59 Asia/Dubai. Run `close` is not used: it needs the handover counted first.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `FIN.RUL.22` | N hours after the end of a holder's period (run: the earlier of its `return_to_depot` or `abort` time and its planned end, i.e. planned start + estimated duration in wl-delivery; shift: its planned end; day: 23:59 Asia/Dubai) | cash handover still `open` with cash or cheques expected | task for the holder's manager to reach the holder; after M hours notify the company admin and ask the manager to `count` at what was received (§5.5 step 4) | N = 12, M = 48; does not run for a vertical rule that replaces it (03: `LOG.RUL.13` replaces it for driver handovers, so a driver gets one escalation, not two) |

§5.5 step 4: "… After M hours the company admin is notified, and the manager counts the handover at
what was actually received (often nothing; the holder's presence is not needed). The full
difference is a shortage: missing cheques are returned as lost and replacements asked for, the rest
is recovered (`FIN.ACL.19`, including from final dues through wl-hr) or written off (`FIN.ACL.06`).
HR may follow the absconding path (`report_absconding` in wl-hr). Money owed to third parties (e.g.
a 03 client's COD remittance) is not reduced."

#### Edge cases decided
- **M counted from:** the end of the period (not from the N task). Period end 2027-05-01 23:59 →
  task 2027-05-02 11:59, admin notice 2027-05-03 23:59.
- **Run that returns late:** the clock started at the planned end, so a driver still out 12 hours
  after it triggers the task (spec).
- **Holder's presence:** not needed for this count; the count records "holder absent".
- **Holder returns before M:** the normal count runs; the task is closed.
- **03 drivers:** `FIN.RUL.22` does not run for driver handovers of a 03 client; `LOG.RUL.13`
  (Epic 03 Logistics) escalates them instead. Other 03 holders (e.g. counter staff) still use
  `FIN.RUL.22`.

#### Must not
- Must not reduce a client's COD remittance because of the shortage.
- Must not count with the holder as the counter.

#### Done when
- [ ] Given a driver's handover `open` with AED 900.00 expected and the day ending 2027-05-01 23:59,
      the manager gets a task at 2027-05-02 11:59.
- [ ] At 2027-05-03 23:59 the admin is notified and the manager is asked to count; counting AED 0
      gives `short` AED 900.00.
- [ ] A handover counted before 12 hours triggers nothing.
- [ ] Given a run planned 08:00 with estimated duration 10 h (planned end 18:00) that never runs
      `return_to_depot`, the task is created at 06:00 next day.
- [ ] Given the same run returning to the depot at 15:00 with its handover left `open`, the task
      is created at 03:00 next day.
- [ ] Given a 03 client and a driver's handover left `open`, `FIN.RUL.22` creates no task (only
      `LOG.RUL.13` runs).

#### Out of scope
Absconding in wl-hr; COD remittance (apps/logistics).

---

### BL-PAY-22 — wl-payments: top up or part-refund a held deposit when its agreement changes

**Labels:** `module:wl-payments` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-payments Deposit and Refund lifecycle · BL-PAY-06 (refund `fail`) · **Blocks:** BL-PRP-08
**Review items:** 01 MEDIUM (lease amendment)

#### Goal
A lease amendment or renewal can change the deposit while it is held. The deposit does not leave
`held`: an increase raises a top-up charge, and a decrease becomes a part refund through the
normal Refund flow with its approval. The held amount follows the money actually received or
paid back, so the final settlement starts from the right figure.

#### Files to read
- `docs/modules/wl-payments.md` §3 *Deposit* (amount, held amount, adjustments), §4 *Deposit*
  (row `adjust`), §4 *Refund*, §5.4 step 1, §6 event `DepositAdjusted`
- `docs/verticals/01-property.md` §7.1 *Lease* (`amend`, `activate`)

#### Terms
- **Amount:** what the source agreement requires as deposit.
- **Held amount:** what is actually held: received less part refunds paid.
- **Top-up charge:** a charge for the increase, requested like the original deposit.
- **`FIN.ACL.04`:** a refund is approved by someone other than the requester.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `held` | info | no | Received and held for the source agreement |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `adjust` | held | held | system (the source agreement changes the deposit amount, e.g. 01 lease amendment or renewal version `activate`) or `deposits.manage` | new amount and the source version given; no adjustment of the deposit still open (top-up unpaid or part refund not paid, cancelled or rejected) | adjustment recorded; increase: a top-up charge for the difference on the effective date (requested like the original deposit), and the held amount rises when its payment is allocated; decrease: a refund `request` for the difference with the deposit as source (Refund flow, approval `FIN.ACL.04`), and the held amount falls when the refund is `paid` (back up if it `fail`s; on refund `cancel` the amount moves to the party's credit balance and the held amount stays reduced; a `rejected` refund leaves the held amount, and the difference is paid back at settlement); the deposit stays `held`; event `DepositAdjusted` |

| Event | When | Payload |
|---|---|---|
| `DepositAdjusted` | deposit `adjust` | deposit, source version, old and new amount, top-up charge or part refund |

#### Edge cases decided
- **Same amount:** no adjustment is recorded.
- **Second change while one is open:** refused until the top-up is paid or the part refund is
  settled (`paid`, `rejected` or `cancelled`).
- **Part refund rejected:** the held amount stays; the amount stays reduced, so the difference
  is due back at settlement.
- **Settlement before the top-up is paid:** settlement uses the held amount; the unpaid top-up
  charge stays on the tenant's balance.
- **Blocked party:** the part refund is money out and waits under `PTY.ACL.03`.

#### Must not
- Must not move the deposit out of `held`.
- Must not reduce the held amount before the part refund is paid.
- Must not pay a part refund without the `FIN.ACL.04` approval.

#### Done when
- [ ] Given a held deposit of AED 5,000 and a lease amendment to AED 6,000 effective 2027-05-01,
      `adjust` creates a top-up charge of AED 1,000 dated 2027-05-01; after its payment is
      allocated, the held amount is AED 6,000 and `DepositAdjusted` was emitted.
- [ ] Given a held deposit of AED 6,000 changed to AED 5,500, a refund request of AED 500 is
      created; user A requests, user B approves, it is paid, and the held amount is AED 5,500.
- [ ] When that refund `fail`s after payment, the held amount goes back to AED 6,000.
- [ ] A second `adjust` while the AED 500 refund is `requested` is refused.

#### Out of scope
The lease amendment itself (BL-PRP-08); refund failure handling (BL-PAY-06, BL-PAY-07).

---

### BL-PAY-23 — wl-payments: raise a later cash shortage against a holder

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-PAY-17 (shortage recovery and write-off) · **Blocks:** BL-LOG-10
**Review items:** X-07, 03-N9-a

#### Goal
Some shortages show up after the holder's handover was already counted: in 03 a delivery is
reversed days later with liability *driver*, so the COD the driver handed over was never really
collected. The operations manager raises the amount as a later shortage against the driver: a
new handover, `short` from the start and linked to its source, that is recovered or written off
through the normal shortage flow.

#### Files to read
- `docs/modules/wl-payments.md` §3 *Cash handover* (kind), §4 *Cash handover* (rows
  `raise_shortage`, `recover`, `write_off`), §5.5 step 5, §6 event `CashShortage`
- `docs/verticals/03-logistics.md` §7.1 *Shipment* (row `reverse_delivery`, liability)

#### Terms
- **Later shortage:** a handover of kind *later shortage*: expected = the amount, counted = 0.
- **Source:** the record that caused it (e.g. the reversed shipment), as a `Subject`.
- **`FIN.ACL.05`:** the counter (here the raiser) is not the holder.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `short` | error | no | Counted less than expected, or an expected cheque is missing |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `raise_shortage` | — | short | `cash.count` (e.g. from the 03 operations manager's task after shipment `reverse_delivery` with liability *driver*) | holder, amount, reason and source (`Subject`) given; raiser ≠ holder (`FIN.ACL.05`); the holder's handover for that cash is no longer `open`; no other later shortage open for the same source | new handover of kind *later shortage* with expected = amount and counted = 0, linked to the source; holder and their manager notified; event `CashShortage` with the source |
| `recover` | short | recovered | `cash.count` | missing cheques found (confirmed) or returned as lost; recoveries recorded on the handover cover the whole difference: cash from the responsible person (the holder, or a till session's cashier), or a deduction passed to wl-hr as an adjustment of type cash shortage recovery (`FIN.ACL.19`) | — |
| `write_off` | short | written_off | `cash.write_off` | missing cheques found or returned as lost; the rest (difference − recoveries recorded) below the write-off limit (`FIN.ACL.06`, from `LOG.ACL.06`), otherwise approval | the rest posted as a loss of the holder's outlet or branch; recoveries recorded stay shown on the handover |

5. A shortage found later (03: a delivery reversed with liability *driver* after the COD was
   counted) is raised against the holder with `raise_shortage`: a new handover of kind
   *later shortage*, `short` from the start, linked to its source, and recovered or written
   off like any other. While the holder's handover for that cash is still `open`, the amount
   is added to its expected total instead and shows at its `count`.

| Event | When | Payload |
|---|---|---|
| `CashShortage` | handover `short` | holder, responsible person, period, till session (if any), difference, missing cheques, source (later shortage) |

#### Edge cases decided
- **Same source twice:** refused while a later shortage for it is open.
- **Holder's handover still `open`:** no later shortage; the amount is added to its expected
  total and shows at its count.
- **Client remittance:** never reduced by the shortage (03 §9).
- **Write-off limit:** `FIN.ACL.06` applies as for any shortage.

#### Must not
- Must not change the counted handover the cash first came in on.
- Must not let the holder raise or count their own shortage.

#### Done when
- [ ] Given driver D's handover of 2027-05-01 `counted` and shipment S (COD AED 300) reversed on
      2027-05-04 with liability *driver*, operations manager M runs `raise_shortage` for AED 300
      with S as source; a new handover of kind *later shortage* is `short` AED 300, D and their
      manager are notified, `CashShortage` carries S.
- [ ] A second `raise_shortage` for S is refused while the first is `short`.
- [ ] D running `raise_shortage` on themselves is refused.
- [ ] AED 300 recovered in cash moves it to `recovered`; a AED 40 one can be written off by the
      cashier within the AED 50 limit.

#### Out of scope
The 03 reversal and its task (BL-LOG-10); salary deductions (Epic wl-hr).

---

### BL-PAY-24 — wl-payments: set off a party's invoice against its supplier bill

**Labels:** `module:wl-payments` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** Base: wl-payments Payment lifecycle · BL-PAY-01 (`reverse`) · BL-PTY-04 (`BlockGuard`) · **Blocks:** BL-CON-03, BL-CON-06, BL-CON-17
**Review items:** 02 MEDIUM (certificate deductions)

#### Goal
When a party is both our customer and our supplier and the two sides agree to settle against
each other (02: the client's back- and contra-charges recorded as a supplier bill from the client
against our IPA invoice), the accountant records a set-off: one payment of method `set_off`
that moves no money and is allocated at once to the invoice and to the supplier bill.

#### Files to read
- `docs/modules/wl-payments.md` §3 *Payment* (method), §4 *Payment* (rows `record`, `reverse`,
  paragraph "Set-off"), §7 setting "Payment methods enabled"
- `docs/modules/wl-invoicing.md` §4 *Invoice* (`allocate`, `unallocate`), *Supplier bill*
  (`pay`, `unallocate`)
- `docs/modules/wl-parties.md` §6 `BlockGuard`, `PTY.ACL.03`

#### Terms
- **Set-off:** settling a receivable and a payable of the same party against each other.
- **Open balance:** what is still unpaid on the invoice or the supplier bill.

#### Spec excerpt (authoritative for this issue)

**Set-off.** When a party is both customer and supplier and the two sides agree to settle
against each other (02: the client's back- and contra-charges recorded as a supplier bill from
the client against our IPA invoice), the accountant records a payment of method `set_off`. It
moves no money and has no bank account: one record with the amount, allocated at once to the
party's open invoice (receivable) and to its `approved` supplier bill (payable) of the same
entity and currency, for the same amount, not above either open balance. The invoice runs
`allocate` and the supplier bill `pay` as for any payment. Because it settles a payable, a
set-off is money out to the party and needs `BlockGuard::check(party, money_out)`
(`PTY.ACL.03`). It is corrected only with `reverse` (reason *entered in error*), which removes
both allocations (`PaymentReversed`: the invoice and the supplier bill run `unallocate`).

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reverse` | received | reversed | `payments.reverse` | reason (entered in error, bank return); the payment is not a gateway payment with an open dispute | every allocation removed; event `PaymentReversed` with the allocations, so wl-invoicing runs `unallocate` on each target (the invoice goes back to `partially_paid`, `issued` or `overdue` by balance and due date); party notified for a bank return |

| Setting | Default |
|---|---|
| Payment methods enabled | cash, transfer, cheque, card at counter (set-off: off; 02: on) |

#### Edge cases decided
- **Amount:** not above the invoice's or the bill's open balance; the rest of either stays open.
- **Different entity or currency:** refused.
- **Supplier bill not `approved`:** refused (a bill in `exception` must be approved first).
- **Blocked party:** refused unless an exception is approved (`PTY.ACL.03`).
- **Reports:** a set-off is shown in receipts and payments with method *set-off*, never in a
  bank reconciliation.

#### Must not
- Must not create a bank movement or need a bank account.
- Must not allocate to only one side.

#### Done when
- [ ] Given IPA invoice INV-0410 (AED 105,000 open) and the client's approved supplier bill
      SB-77 (AED 5,250), a set-off of AED 5,250 leaves INV-0410 `partially_paid` with AED 99,750
      open and SB-77 `paid`; no bank account is set.
- [ ] A set-off of AED 6,000 against SB-77 is refused.
- [ ] `reverse` of the set-off with reason *entered in error* reopens both documents.
- [ ] With the client blocked, the set-off is refused until the compliance role approves an
      exception.
- [ ] With the setting off, method *set-off* is not offered.

#### Out of scope
Creating the supplier bill from the certificate (BL-CON-06).
