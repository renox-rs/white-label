# Epic: 01 Property — negative flows

> Status: in use (F1–F10 built; open items in [README](../README.md) §7) · Last updated: 2026-10-24

This epic builds the negative flows that the negative-flow review added to
[01 Property](../../verticals/01-property.md): the lease machine (cancel after payment, the
pending-signature chase, the 120-day renewal window and the 90-day notice guard, holding over,
renewal and amendment versions, notice withdrawal, termination, forced termination, the legal
case flag and arrears escalation), the deposit, write-off and owner-statement reactions, the new
management agreement and owner statement machines, and 01's values for the shared mechanisms.
Review items: D-01, D-02, D-03, D-13, D-14, 01-N2-a, 01-N3-a, 01-N3-b, 01-N3-e, 01-N3-g,
01-N4-c, 01-N5-b, 01-N6-a, 01-N8-b, 01-N9-a, 01-N12-a, 01-N12-c, 01-N13-a and the 01 MEDIUM
items of review §6.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-PRP-01 | Cancel a lease after money or cheques were received (`PRP.ACL.18`) | high | m | Build 1 | Base: 01 Lease lifecycle; BL-KRN-01; BL-PAY-06 |
| BL-PRP-02 | Chase a lease waiting for signature (`PRP.RUL.12`) | medium | s | Build 1 | Base: 01 Lease lifecycle; BL-PRP-01 |
| BL-PRP-03 | Open the renewal window at 120 days and guard the notice deadline (`PRP.RUL.01`, `PRP.RUL.09`) | high | m | Build 1 | Base: 01 Lease lifecycle; BL-NTF-05; BL-KRN-08 |
| BL-PRP-04 | Refuse a version with changed terms without timely notice or the tenant's acceptance (`PRP.ACL.17`) | high | s | Build 1 | BL-PRP-03; BL-PRP-05 |
| BL-PRP-05 | Submit and activate renewal and amendment versions for the sitting tenant | high | s | Build 1 | Base: 01 Lease lifecycle |
| BL-PRP-06 | Hold over after the end date (`expire`, `PRP.RUL.03`, `PRP.RUL.14`) | high | m | Build 1 | Base: 01 Lease lifecycle; Base: wl-invoicing Charge schedule; BL-PRP-03 |
| BL-PRP-07 | Renew from `renewal_due` or `expired` without billing twice | high | s | Build 1 | BL-PRP-05; BL-PRP-06 |
| BL-PRP-08 | Amend an active lease mid-term (`amend`) | high | m | Build 1 | BL-PRP-04; BL-PRP-05; BL-PRP-13; BL-INV-07; BL-PAY-22 |
| BL-PRP-09 | Record a notice from any running state, with the landlord's statutory notice and the early termination penalty | high | s | Build 1 | Base: 01 Lease lifecycle; BL-PRP-06; BL-NTF-05 |
| BL-PRP-10 | Withdraw a tenant's notice (`withdraw_notice`, `PRP.ACL.20`) | medium | s | Build 1 | BL-PRP-09 |
| BL-PRP-11 | Terminate a lease and settle charges, cheques and Ejari after move-out | high | m | Build 1 | BL-PRP-09; Base: wl-payments Cheque lifecycle |
| BL-PRP-12 | Force-terminate a lease for an abandoned unit or an executed eviction (`PRP.ACL.19`) | high | m | Build 1 | BL-PRP-11; BL-PRP-13; BL-KRN-01 |
| BL-PRP-13 | Set and clear the legal case flag on a lease | high | m | Build 1 | Base: 01 Lease lifecycle; BL-INV-04; BL-PAY-12; BL-PAY-15 |
| BL-PRP-14 | Escalate arrears to a formal demand and a legal case (`PRP.RUL.10`) | high | m | Build 1 | BL-PRP-13; BL-NTF-05; BL-INV-04 |
| BL-PRP-15 | Refuse new lease versions and hold money out for a blocked tenant | high | s | Build 1 | BL-PTY-04; BL-PRP-05 |
| BL-PRP-16 | Escalate an unanswered deposit deductions dispute (`PRP.RUL.11`) | medium | s | Build 1 | BL-PAY-15 |
| BL-PRP-17 | Apply the deposit in full and invoice deductions that exceed it | high | s | Build 1 | BL-PAY-15; BL-PRP-11 |
| BL-PRP-18 | Record the owner's consent to a write-off and show write-offs and recoveries on the owner statement | high | s | Build 1 | BL-INV-01; BL-INV-03; BL-PRP-19 |
| BL-PRP-19 | Approve, return, publish and revise an owner statement | medium | m | Build 1 | Base: 01 Owner statement generation (`PRP.RUL.05`); BL-KRN-01 |
| BL-PRP-20 | Pay out only a positive, undisputed balance; carry forward or invoice a negative one (`PRP.ACL.21`) | high | m | Build 1 | BL-PRP-19; BL-PRP-21; BL-PTY-04 |
| BL-PRP-21 | Dispute an owner statement line and correct it by revision or adjustment lines | medium | s | Build 1 | BL-PRP-19 |
| BL-PRP-22 | Activate a management agreement, give notice and end it with a handover pack | high | m | Build 1 | BL-PRP-19; BL-PAY-16; Base: wl-payments Cheque lifecycle |
| BL-PRP-23 | Transfer a unit out of a management agreement when it is sold or no longer managed (`transfer_unit`) | high | m | Build 1 | BL-PRP-22 |
| BL-PRP-24 | Cancel a maintenance job after work started and guard compliance visits (`PRP.ACL.15`) | medium | s | Build 1 | Base: wl-ticketing Ticket lifecycle; BL-TKT-02; BL-TKT-04 |
| BL-PRP-25 | Show building compliance flags and expiring certificates without blocking leasing | high | s | Build 1 | BL-TKT-03; BL-TKT-04; BL-TKT-07; BL-KRN-08 |
| BL-PRP-26 | 01: configure shared negative-flow settings — money | high | m | Build 1 | BL-INV-01; BL-INV-04; BL-INV-11; BL-INV-24; BL-PAY-09; BL-PAY-15; BL-PTY-03; BL-KRN-03; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26 |
| BL-PRP-27 | 01: configure shared negative-flow settings — compliance, documents and notices | high | m | Build 1 | BL-DOC-01; BL-DOC-04; BL-TKT-03 to BL-TKT-08; BL-NTF-05; BL-KRN-08; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10; Base: 01 Enquiries and viewings (`PRP.LEA.01`) |
| BL-PRP-28 | Release a reserved unit and settle its booking deposit | medium | s | Later | Base: 01 Unit reservation (`PRP.LEA.02`, Later); BL-PRP-01 |
| BL-PRP-29 | Owner portal: dispute a statement line and keep read-only access after the agreement ends | medium | s | Later | BL-PRP-21; BL-PRP-22; Base: wl-portal owner portal (`POR.SLF.02`, Later) |
| BL-PRP-30 | Tenant portal: show the lease, its cheques and documents (`POR.SLF.01`, `PRP.ACL.01`) | high | s | Build 1 | BL-POR-02; BL-POR-01; Base: 01 Lease lifecycle |
| BL-PRP-31 | Generate the tenancy contract and the owner statement PDF and send them | high | s | Build 1 | BL-DOC-11; BL-NTF-10; BL-PRP-19 |
| BL-PRP-32 | Apply the termination credit to the deposit deductions first | high | s | Build 1 | BL-PRP-11; BL-PRP-17; BL-PAY-15 |
| BL-PRP-33 | Run `terminate` and its money effects in one transaction | high | s | Build 1 | BL-KRN-11; BL-PRP-11 |
| BL-PRP-34 | Put approved vendor bills on the owner statement (ticket jobs and AMC shares) | high | m | Build 1 | BL-PRP-19; BL-PRP-24; BL-INV-24; Base: wl-invoicing Supplier bill lifecycle |

---

### BL-PRP-01 — 01 Property: cancel a lease after money or cheques were received

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 01 Lease lifecycle · Base: wl-invoicing Credit note · Base: wl-payments Refund lifecycle · BL-KRN-01 · BL-PAY-06 · **Blocks:** BL-PRP-02, BL-PRP-28
**Review items:** 01-N2-a

#### Goal
A prospect may back out after paying rent or a deposit or handing over post-dated cheques,
before the lease is active. The leasing agent or accountant requests the cancellation with a
reason; a property manager approves it. On approval the lease is `cancelled`, undeposited
cheques are returned against a receipt, issued invoices are credited, rent paid is refunded and
the deposit is refunded or partly forfeited only where the signed terms allow it.

#### Surfaces
- **Back office:** when the prospect backs out, the leasing agent or accountant runs `cancel`
  on the lease record with a reason and, where the signed terms allow, a forfeit amount and clause reference; a
  property manager approves or rejects it under "Needs your action", and the accountant gets
  a task listing any cheque already deposited.
- **Portal:** the tenant sees the published deposit settlement, with any forfeit as a
  deduction line with its clause reference, and may dispute it within the 14-day window.
- **System:** at final approval the lease is cancelled and the cheque returns, credit notes,
  refund requests and deposit settlement run on the money received by then.

#### Files to read
- `docs/verticals/01-property.md` §4.1 (table "When it goes wrong", row "The prospect backs out
  after paying the deposit or handing over PDCs"), §3.3 `PRP.ACL.18`, `FIN.ACL.04`, §7.1
  *Lease* (`cancel`), *Unit* (`release`), *Cheque* (`return`), *Deposit*
- `docs/modules/wl-kernel.md` §4.1 (approval request; what happens when the answer is not yes)
- `docs/modules/wl-invoicing.md` §4 *Invoice* (`credit`) and *Credit note*
- `docs/modules/wl-payments.md` §4 *Refund*, *Cheque* (`return`), *Deposit*

#### Terms
- **Money received:** any payment allocated to the lease's invoices, a deposit in `held`, or a
  cheque linked to the lease in `received` or `due_for_deposit` (or already `cleared`). Once
  reservations exist (Later, BL-PRP-28), a booking deposit on the unit reserved for this
  prospect counts too.
- **Forfeit:** keeping part or all of the deposit; allowed only where the signed reservation or
  contract terms allow it.
- **Handover receipt:** the PDF generated by cheque `return`, signed by the person collecting
  the cheques.
- **`SOD` rule:** segregation of duties; the requester cannot approve.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | draft, pending_signature | cancelled | `leases.create` | reason given; when money or cheques were received, `PRP.ACL.18` | unit `release` if reserved; cheques not deposited `return` against a receipt, a deposited one waits for `clear` or `bounce`; issued invoices credited (a paid one unallocated first, its payment moved to the tenant's credit balance), drafts discarded; rent paid refunded from the credit balance through the Refund flow; deposit refunded, or part forfeited with the clause reference where the signed terms allow it |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PRP.ACL.18` | Cancelling a lease after money or cheques were received (a booking deposit on the reserved unit counts), and forfeiting any of it, is requested by the leasing agent or accountant and approved by a property manager | `SOD` | leasing agent, accountant | on |
| `FIN.ACL.04` *(shared)* | The user who records a deposit refund cannot approve it | `SOD` | all staff | on |

Row from §4.1: "Lease `cancel` with a reason, approved by a property manager (`PRP.ACL.18`); one
approval covers the cancellation, the refunds and any forfeit, and the money effects run only at
final approval, on the money received by then. Cheques not yet deposited are returned against a
handover receipt; a cheque already deposited waits for `clear` (its amount joins the refund) or
`bounce` (nothing to refund), and the accountant gets a task listing it. Issued invoices are
credited: a paid one is first unallocated (the payment moves to the tenant's credit balance) and
then credited in full; draft invoices are discarded. Rent already paid is refunded from the credit
balance through the Refund flow (`FIN.ACL.04`). The security deposit is refunded, or part of it
forfeited only where the signed reservation or contract terms allow it (the clause reference is
recorded), with the property manager's approval; the forfeit is published to the tenant as a
deduction and follows the dispute window (§7.1 *Deposit*). The unit is released."

Deposit note (§7.1): "A part forfeit on a cancelled lease (`PRP.ACL.18`) is a deduction line, with
the clause reference, on `refund`; `forfeit` keeps the whole deposit."

Deposit transitions used: `start_settlement` (held → settlement_pending, system on lease
`cancel`), `refund`, `forfeit`.

Negative-flow settings (§7.2): Lease — approval rejected or returned: "the version stays in its
state; requester told".

#### Edge cases decided
- **No money received:** `cancel` runs at once with a reason; no approval request (the rule only
  applies when money or cheques were received).
- **Approval pending:** the lease stays in `draft` or `pending_signature` with a pending-approval
  badge; `activate` is refused while the cancel request is pending. Rejected or returned: the lease
  stays in its state and the requester is told (BL-KRN-01).
- **Payment received while the request waits:** the kernel re-checks at final approval; the new
  payment is included in the refund list built at approval time, not at request time.
- **Cheque received while the request waits:** the cheque list is also built at final approval
  ("on the money received by then", §4.1). A cheque linked to the lease after the request and
  still `received` or `due_for_deposit` at approval is returned with the others on the same
  handover receipt; one already `deposited` by then waits for `clear` or `bounce` like any
  other (task to the accountant).
- **Cleared cheques:** they are payments; their amount is refunded like rent paid. Only cheques in
  `received` or `due_for_deposit` are returned. A cheque already `deposited` at approval waits
  for `clear` or `bounce`; on `clear` its amount joins the refund, on `bounce` nothing is
  refunded for it. The accountant gets a task listing it.
- **Invoices:** an issued invoice with no payment allocated is credited in full (`credit`). An
  invoice with payments allocated cannot be fully credited (`credit` guard), so the allocations
  are removed first (system `unallocate`; the payment moves to the tenant's credit balance), the
  invoice is credited in full and the credit balance is refunded through the Refund flow. Draft
  invoices are discarded.
- **Forfeit:** the requester enters the forfeited amount and the clause reference (signed
  reservation or contract). Without a clause reference, forfeit is refused and the full deposit is
  refunded. The forfeited amount cannot exceed the deposit. A part forfeit is a deduction line on
  the deposit `refund`, published to the tenant; the refund of the rest waits for the 14-day
  dispute window like any deposit settlement (`PRP.RUL.11`). Whether a deposit can be forfeited
  before signing at all is `⚠ open: lawyer (L-22)`; the default built here only allows forfeit
  with a clause reference.
- **One approval:** one approval request covers the cancel, the refunds and the forfeit amount
  (`PRP.ACL.18`); the individual refunds still follow `FIN.ACL.04` (the approver of the refund is
  not the user who recorded it).
- **Rounding:** amounts in AED with 2 decimals; refunds equal the allocated amounts exactly.
- **Repeated run:** a second `cancel` on a `cancelled` lease is refused (terminal).
- **Reserved unit:** reservations are Later; in the MVP the unit is `available` and nothing
  changes on it. The `release` effect is BL-PRP-28.

#### Must not
- Must not delete invoices that were issued; credit them.
- Must not forfeit anything without a clause reference and approval.
- Must not return cheques that were deposited or cleared.
- Must not let the requester approve their own cancellation.
- Must not run any money effect before final approval.

#### Done when
- [ ] Given a `pending_signature` lease with no payment and no cheque, when the leasing agent A
      cancels with reason "prospect withdrew", then the lease is `cancelled` at once.
- [ ] Given a `pending_signature` lease with deposit AED 5,000.00 `held`, first rent AED
      25,000.00 paid against an issued invoice, and 3 cheques of AED 25,000.00 `received`, when
      A requests `cancel`, then an approval request is created for a property manager and the
      lease stays `pending_signature`.
- [ ] When A tries to approve, it is refused (SoD).
- [ ] When property manager B approves with no forfeit, then the lease is `cancelled`; the 3
      cheques are `returned` with a handover receipt; the rent invoice is unallocated (AED
      25,000.00 to the tenant's credit balance) and credited in full; a refund of AED 25,000.00
      from the credit balance is requested; the deposit is `settlement_pending` with its
      settlement published to the tenant, and its refund of AED 5,000.00 is requested once the
      14-day dispute window has passed with no dispute.
- [ ] With forfeit AED 2,000.00 and clause "Reservation form §4", the published settlement shows a
      deduction line of AED 2,000.00 with the clause and, after the window, the deposit refund is
      AED 3,000.00; without a clause the forfeit field is refused.
- [ ] When B rejects, the lease stays `pending_signature`, nothing is credited or returned and A
      is notified with the comment.
- [ ] A cheque `deposited` at approval time is listed in a task; when it clears, AED 25,000.00 is
      added to the refunds.
- [ ] Given the request above and a 4th cheque of AED 25,000.00 `received` after the request,
      when B approves, the 4 cheques are `returned` on one handover receipt.

#### Out of scope
Unit `release` and booking deposits (BL-PRP-28, Later); refund failure (BL-PAY-06, BL-PAY-07);
the pending-signature chase (BL-PRP-02).

---

### BL-PRP-02 — 01 Property: chase a lease waiting for signature (`PRP.RUL.12`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: 01 Lease lifecycle · BL-PRP-01 · **Blocks:** None.
**Review items:** 01 MEDIUM "pending-signature timeout" (review §6)

#### Goal
A contract that was generated but not signed, or whose deposit was not paid, must not wait
forever. After 14 days in `pending_signature` the leasing agent and property manager are reminded;
after 30 days the property manager gets a task to extend the date or cancel the lease.

#### Surfaces
- **Back office:** the leasing agent and property manager are reminded after 14 days; after
  30 days the property manager has an "extend or cancel" task with "extend to date" and
  "cancel".
- **System:** the daily `PRP.RUL.12` run at 02:00 creates the reminder and the task, and a
  new task when an extended date passes; it never cancels.

#### Files to read
- `docs/verticals/01-property.md` §4.1 (row "The contract is generated but not signed, or the
  deposit not paid"), §7.2 `PRP.RUL.12`, §7.1 *Lease* (`cancel`)

#### Terms
- **Days in `pending_signature`:** calendar days since the lease entered `pending_signature`
  (history timestamp, Asia/Dubai date).
- **Extend:** the property manager sets a new follow-up date; the lease stays `pending_signature`.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `PRP.RUL.12` | N and M days after lease → `pending_signature` | still `pending_signature` | at N: remind the leasing agent and property manager; at M: task to the property manager to extend to a date at most 30 days ahead, or `cancel`. Extending closes the task; a lease still `pending_signature` on that date gets a new task. Never cancels automatically; nothing is sent to the tenant | N = 14; M = 30 | on | MVP |

#### Edge cases decided
- **Run time:** daily at 02:00 Asia/Dubai (§7.2 intro). A lease
  that entered `pending_signature` on 2027-03-01 is reminded on 2027-03-15 and gets the task on
  2027-03-31.
- **Extend:** the task offers "extend to date" (at most 30 days ahead) and "cancel". Extending records the date on the lease and closes the task; if the lease is still
  `pending_signature` on that date, a new task is created. There is no automatic cancellation.
- **Cancel from the task:** opens `cancel` (BL-PRP-01), including its approval when money was
  received.
- **Left `pending_signature` and came back:** the clock restarts from the latest entry.
- **Idempotency:** one reminder per lease at N and one open task at a time.

#### Must not
- Must not cancel the lease automatically.
- Must not send the tenant any message from this rule.

#### Done when
- [ ] Given a lease `pending_signature` since 2027-03-01, on 2027-03-15 the leasing agent and the
      property manager are notified once.
- [ ] On 2027-03-31 the property manager has one task "extend or cancel".
- [ ] Extending to 2027-04-15 closes the task; on 2027-04-15, still unsigned, a new task appears.
- [ ] A lease activated on 2027-03-10 gets no reminder.

#### Out of scope
The cancellation itself (BL-PRP-01).

---

### BL-PRP-03 — 01 Property: open the renewal window at 120 days and guard the notice deadline (`PRP.RUL.01`, `PRP.RUL.09`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 01 Lease lifecycle · BL-NTF-05 · BL-KRN-08 · **Blocks:** BL-PRP-04, BL-PRP-06
**Review items:** D-03, 01-N5-b

#### Goal
In Dubai a change of rent or terms at renewal needs written notice at least 90 days before
expiry. The lease enters the renewal window 120 days before its end so the property manager has a
month to decide. The lease shows the notice deadline; a compliance-critical rule warns 14 and 3
days before it, and at the deadline records that only the current terms can be renewed unless the
tenant signs the new ones.

#### Surfaces
- **Back office:** the property manager sets the renewal decision on the lease, which shows
  "notice deadline" and, once it has passed, "current terms only"; warnings arrive 14 and 3
  days before the deadline.
- **System:** the daily runs of `PRP.RUL.01` and `PRP.RUL.09` flag the lease `renewal_due`
  and set the marker; a failed `PRP.RUL.09` run alerts the company admin.

#### Files to read
- `docs/verticals/01-property.md` §2 (90-day notice, Art. 13–14), §4.3 steps 1–2 and row "The
  notice of changed terms was late or not delivered", §6 *Lease* (notice of changed terms
  fields), §7.1 *Lease* (`flag_renewal`), §7.2 `PRP.RUL.01`, `PRP.RUL.09`, "Compliance-critical
  rules and owner alerts", "Statutory notices"
- `docs/modules/wl-notify.md` §5.4 (`statutory_notice`)

#### Terms
- **Notice period:** 90 days, or the period the contract sets (lease field).
- **Notice deadline:** lease end date − notice period.
- **Notice of changed terms:** a statutory notice to the tenant announcing new rent or terms;
  fields on the lease: sent, delivered, proof.
- **Proof of delivery:** the delivery record of the message, or the proof a staff member records
  when the message gave up (courier receipt, signed hand-delivery note…).
- **Current terms only:** a marker on the lease meaning a renewal with changed terms needs the
  tenant's signed acceptance (`PRP.ACL.17`, BL-PRP-04).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `flag_renewal` | active | renewal_due | system (`PRP.RUL.01`) | — | notify manager and tenant; the notice-of-changed-terms deadline (end date − 90 days, or the contract's period) is shown on the lease |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `PRP.RUL.01` | max(N, contract notice period + 30) days before `lease.end_date` | lease is `active` | lease `flag_renewal`; notify property manager and tenant (a normal message, not a statutory notice) | N = 120 (leaves a month to decide before the 90-day notice deadline; a 120-day contract notice period flags at 150 days) | on | MVP |
| `PRP.RUL.09` · compliance-critical | N days before the notice-of-changed-terms deadline (end date − 90 days, or the contract's period) | lease `renewal_due`; renewal decision `undecided`, or `changed_terms` and no proof of delivery | notify the property manager; on the day after the deadline, set the current-terms-only marker: only the current terms can be renewed without the tenant's signed acceptance (`PRP.ACL.17`). Proof recorded later is stored but does not clear the marker. A lease that reaches `renewal_due` with its deadline already passed (activated late, or a term shorter than the notice period) gets the marker on the first run, with one notification to the property manager; warnings for offsets already passed are not sent | N = 14, 3 | on | MVP |

§4.3 step 2: "If rent or terms change, the new rent is checked against the rental index
(`PRP.LSE.03`) and the notice of changed terms is sent as a statutory notice; its proof of
delivery is recorded on the lease (`PRP.RUL.09`). Without timely proof, the renewal keeps the
current terms unless the tenant signs the new ones (`PRP.ACL.17`)."

"Compliance-critical rules": if `PRP.RUL.09` fails or does not run, `PLT.RUL.04` alerts the owner
role (01: company admin by default) and the company admin; with the default they are the same
role, alerted once.

#### Edge cases decided
- **Renewal decision:** the lease field `renewal_decision` with values `undecided` (default),
  `same_terms`, `changed_terms`, `not_renewing`, set by the property manager (§6). `PRP.RUL.09`
  fires when the decision is `undecided`, or `changed_terms` without proof of delivery.
- **Dates:** all in Asia/Dubai. End 2027-12-31: flag on 2027-09-02 (120 days before), deadline
  2027-10-02 (90 days before), warnings on 2027-09-18 and 2027-09-29, marker set on 2027-10-03
  (the day after the deadline, once the deadline day is over).
- **Deadline inclusive:** a notice delivered on the deadline day counts.
- **Contract notice period longer than 90 days:** `PRP.RUL.01` fires at max(N, contract notice
  period + 30) days before the end, so the month to decide is kept. Example: 120-day contract
  notice → flag at 150 days.
- **Lease shorter than N days when activated:** the daily run flags it on the first run after
  activation.
- **Deadline already passed when flagged** (lease activated late, or a term shorter than the
  notice period): on the first run the lease is flagged and, if the decision is `undecided` or
  `changed_terms` without proof, the marker is set at once with one notification to the property
  manager. Warnings whose offsets (14, 3) have already passed are not sent (`PRP.RUL.09`).
- **Message gave up:** the notice does not count as delivered until a staff member records proof
  (BL-NTF-05); the warning at 3 days says "notice not delivered".
- **Proof recorded after the deadline:** it is stored, but the marker stays (the notice was not
  timely).
- **Tenant notification on flag:** a normal (not statutory) message.
- **Run time:** daily at 02:00 Asia/Dubai (§7.2 intro). A missed run triggers
  `PLT.RUL.04` (BL-KRN-08) because the rule is compliance-critical.
- **Idempotency:** one warning per lease per offset (14, 3); the marker is set once.

#### Must not
- Must not send the notice of changed terms automatically; the property manager decides.
- Must not count a sent but undelivered notice as given.
- Must not block a renewal on the current terms because of the marker.

#### Done when
- [ ] Given an `active` lease ending 2027-12-31, on 2027-09-02 it is `renewal_due`, the manager
      and the tenant are notified, and the lease shows "notice deadline 2027-10-02".
- [ ] With `renewal_decision = undecided`, the property manager is notified on 2027-09-18 and
      2027-09-29.
- [ ] With `changed_terms` and a notice delivered on 2027-09-20 (proof recorded), no warning is
      sent and no marker is set.
- [ ] With `changed_terms` and the notice message given up and no proof by 2027-10-02, the lease
      shows "current terms only" from 2027-10-03.
- [ ] With `same_terms`, no warning and no marker.
- [ ] A lease with a contract notice period of 60 days ending 2027-12-31 shows deadline
      2027-11-01.
- [ ] Given a 60-day lease 2027-06-01 to 2027-07-30 (contract notice period 90 days, deadline
      2027-05-01) activated on 2027-06-01, on 2027-06-02 it is `renewal_due`, shows "current
      terms only" and the property manager is notified once; no 14- or 3-day warning is sent.
- [ ] If the `PRP.RUL.09` run fails, the company admin gets the owner alert.

#### Out of scope
The guard on the renewal version (BL-PRP-04); the rental index check (`PRP.LSE.03`, Later);
statutory notice delivery mechanics (BL-NTF-05).

---

### BL-PRP-04 — 01 Property: refuse a version with changed terms without timely notice or the tenant's acceptance (`PRP.ACL.17`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PRP-03 · BL-PRP-05 · **Blocks:** BL-PRP-08
**Review items:** D-03, 01-N5-b

#### Goal
A renewal or amendment version that raises the rent or changes other terms against the tenant
can be submitted only when the notice of changed terms was delivered, with proof, at least the
notice period before the current end date; otherwise only with the tenant's signed acceptance
attached. An amendment version (mid-term) against the tenant always needs the acceptance. This
keeps the client inside the 90-day rule.

#### Surfaces
- **Back office:** the leasing agent or property manager submits a renewal or amendment
  version; `submit` is refused with the deadline, the proof status and the two ways forward,
  or asks "changes terms against the tenant?" with a reason for *no*; the tenant's signed
  acceptance is attached to the draft.
- **System:** the `submit` guard compares rent, deposit and number of cheques with the
  current version.

#### Files to read
- `docs/verticals/01-property.md` §3.3 `PRP.ACL.17`, §4.3 row "The notice of changed terms was
  late or not delivered", §7.1 *Lease* (`submit`)

#### Terms
- **Version:** a new lease record of kind `renewal` or `amendment` that replaces the current
  version when activated.
- **Against the tenant:** a change that makes the lease worse for the tenant (see edge cases).
- **Tenant's signed acceptance:** a document attached to the version in which the tenant accepts
  the new terms.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PRP.ACL.17` | A renewal or amendment version that raises the rent or changes other terms against the tenant is submitted only when a notice of changed terms was delivered, with proof recorded, at least the notice period before the current end date (the deadline day counts; the proof is read from the version being replaced); otherwise only with the tenant's signed acceptance (document kind *tenant acceptance of changed terms*) attached to the draft version. An amendment version (mid-term, by agreement) that changes terms against the tenant always needs that acceptance; a notice alone is not enough. Against the tenant: a higher annual rent or deposit, or fewer cheques, than the current version, detected automatically; any other changed term (clauses, payment dates, added charges) unless the submitter answers *no* with a reason kept in history. A rent decrease alone is not against the tenant | `STA` | leasing agent, property manager | 90 days, or the contract's period |

`submit` guard (excerpt): "changed terms against the tenant: `PRP.ACL.17`".

§4.3 row: "The renewal version is refused with changed terms (`PRP.ACL.17`); the property
manager renews on the current terms or gets the tenant's signed acceptance. A notice whose
delivery failed is not counted until proof is recorded (`statutory_notice`, wl-notify)."

#### Edge cases decided
- **What counts as against the tenant** (`PRP.ACL.17`): detected automatically when the annual
  rent is higher, the deposit is higher, or the number of cheques is lower than in the current
  version. Any other changed field (clauses, payment dates, added charges) makes the
  submitter answer "changes terms against the tenant?" with default *yes*; *no* needs a reason
  that is kept in history. A rent decrease alone is not against the tenant.
- **Timely:** proof-of-delivery date ≤ current end date − notice period (inclusive).
- **Amendment mid-term:** a mid-term change is by agreement, so an amendment version against the
  tenant always needs the tenant's signed acceptance; a timely notice alone does not let it be
  submitted (`PRP.ACL.17`). The notice test applies only to renewal versions. An amendment that
  is not against the tenant (e.g. a rent decrease, or *no* with a reason) needs neither.
- **Acceptance document:** a document of kind *tenant acceptance of changed terms* attached to
  the draft version before `submit` (`PRP.ACL.17`; the signed contract comes later at `activate`
  and is not enough at submit).
- **Notice for another version:** the proof is read from the current version (the one being
  replaced).
- **Error message:** names the deadline, the proof status and the two ways forward.

#### Must not
- Must not accept a sent but undelivered notice.
- Must not apply the guard to a version on the current terms.

#### Done when
- [ ] Given current end 2027-12-31 (deadline 2027-10-02), a renewal draft raising rent from AED
      100,000 to AED 105,000, and proof of delivery dated 2027-09-25, `submit` succeeds.
- [ ] With proof dated 2027-10-10, `submit` is refused with "notice not timely (deadline
      2027-10-02)"; after attaching the tenant's signed acceptance, `submit` succeeds.
- [ ] With the notice message given up and no proof, `submit` is refused.
- [ ] A renewal on the same rent and terms is submitted without notice or acceptance.
- [ ] A renewal changing only a clause, with the answer *yes*, is refused without proof or
      acceptance; with *no* and reason "typo fix", it is submitted and the reason is in history.
- [ ] Given an `active` lease ending 2027-12-31 and proof of a notice delivered on 2027-03-01, an
      amendment draft raising rent from 2027-06-01 is refused without the tenant's acceptance;
      with the acceptance attached, `submit` succeeds.

#### Out of scope
The deadline warnings (BL-PRP-03); the rest of `submit` (BL-PRP-05).

---

### BL-PRP-05 — 01 Property: submit and activate renewal and amendment versions for the sitting tenant

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: 01 Lease lifecycle · **Blocks:** BL-PRP-04, BL-PRP-07, BL-PRP-08, BL-PRP-15
**Review items:** D-02

#### Goal
A renewal or amendment version for the tenant already in the unit could not be submitted,
because `submit` required the unit to be available or reserved. The guard now exempts such
versions, and `activate` acts per version kind: a new lease occupies the unit; a renewal or
amendment replaces the previous version and leaves the unit's state unchanged.

#### Surfaces
- **Back office:** the leasing agent submits and activates a renewal or amendment version
  for the sitting tenant from the lease record; a refused `submit` or `activate` shows its
  reason ("unit not available").
- **System:** `activate` ends the previous version as `renewed` or `amended` in one
  transaction, creates the charge schedule and the Ejari task and emits `LeaseActivated`.

#### Files to read
- `docs/verticals/01-property.md` §7.1 *Lease* (intro, `submit`, `activate`), *Unit* (`occupy`
  and the line "A renewal or amendment version does not change the unit's state"), §6 *Lease*
  (version kind, effective date)

#### Terms
- **Version kind:** `new`, `renewal` or `amendment` (lease field).
- **Effective date:** the date from which an amendment applies; a renewal starts on its start
  date.
- **Previous version:** the version the new one replaces (same lease, same unit).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | pending_signature | `leases.create` | the unit is available or reserved for this tenant, or this is a renewal or amendment version of the lease on the unit for the same tenant; tenant KYC documents present; payment plan adds up to the annual rent; a renewal version needs the previous version in `renewal_due` or `expired`, an amendment version needs it in `active`, `renewal_due` or `expired`; an amendment's effective date within the current term and not before the first day of the current month; changed terms against the tenant: `PRP.ACL.17`; tenant not blocked (`PTY.ACL.03`) | contract PDF generated in EN/AR (`DOC.GEN.01`) |
| `activate` | pending_signature | active | `leases.activate` | signed contract attached; deposit received (for a renewal or amendment, the carried-over `held` deposit; a different deposit amount is handled by deposit `adjust` on activation, so an unpaid top-up does not block `activate`) or waiver approved; tenant not blocked; a renewal or amendment version: the previous version still in a state that its `renew` or `amend` accepts (checked again; `activate` and that transition run in one transaction) | new lease: unit `occupy`; renewal version: previous version `renew`; amendment version: previous version `amend`; charge schedule created from the start or effective date (`FIN.BIL.01`); a renewal or amendment whose deposit differs from the held amount: deposit `adjust` (system); Ejari task (`PRP.RUL.02`); event `LeaseActivated` |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `occupy` | available, reserved | occupied | system (lease `activate` of a new lease) | — | — |

"A renewal or amendment version does not change the unit's state."

#### Edge cases decided
- **Same tenant:** the billed tenant party of the version equals the billed tenant of the previous
  version. A version that changes the billed tenant is a `new` lease (the unit must be available).
  Adding a co-tenant is an amendment (spec §4.3 "added tenant").
- **Previous version state:** a renewal version needs the previous version in `renewal_due` or
  `expired`; an amendment needs `active`, `renewal_due` or `expired` (the `renew` and `amend` From
  states). Otherwise `submit` is refused. Checked at `submit` and again at `activate`.
- **Two versions in progress:** only one renewal or amendment version may be in `draft` or
  `pending_signature` per lease at a time (§7.1 *Lease* intro).
- **Previous version moved meanwhile** (e.g. `notice_given` or `terminated`): `activate` is
  refused with the reason; the version can be cancelled.
- **Lease in `notice_given`:** `submit` of a renewal version is refused here by the
  previous-version state guard (`notice_given` is not a `renew` From state). Withdrawing the
  notice is BL-PRP-10, which builds on this issue; it is not needed to build or test this one.
- **Atomicity:** `activate` and the previous version's `renew` or `amend` run in one transaction;
  if one fails, neither happens.
- **Deposit:** a renewal or amendment carries the held deposit over; "deposit received" is met by
  the carried-over `held` deposit (an unpaid top-up never blocks `activate`). A changed amount
  runs deposit `adjust` (BL-PAY-22) on activation; its effects on an amendment are tested in
  BL-PRP-08, which builds on this issue. This issue tests only an unchanged deposit.

#### Must not
- Must not change the unit's state on a renewal or amendment.
- Must not leave two versions `active` at once.

#### Done when
- [ ] Given unit 1204 `occupied` by tenant T with lease version 1 `renewal_due`, a renewal draft
      for T passes the unit guard on `submit`.
- [ ] A new lease draft for tenant U on unit 1204 is refused ("unit not available").
- [ ] Activating the renewal makes version 1 `renewed`, version 2 `active`, unit 1204 stays
      `occupied`, the charge schedule starts on version 2's start date, an Ejari task is created
      and `LeaseActivated` is emitted.
- [ ] Activating an amendment makes the previous version `amended`; the unit is unchanged.
- [ ] A second renewal draft for the same lease is refused while one is `pending_signature`.
- [ ] If version 1 was terminated meanwhile, activating version 2 is refused.

#### Out of scope
`PRP.ACL.17` (BL-PRP-04); the `renew` effects (BL-PRP-07); the `amend` effects (BL-PRP-08);
the blocked-tenant guard (BL-PRP-15).

---

### BL-PRP-06 — 01 Property: hold over after the end date (`expire`, `PRP.RUL.03`, `PRP.RUL.14`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 01 Lease lifecycle · Base: wl-invoicing Charge schedule (`FIN.BIL.01`) · BL-PRP-03 · **Blocks:** BL-PRP-07, BL-PRP-09
**Review items:** D-01, 01-N13-a

#### Goal
When a lease passes its end date with no active renewal and no notice, it moves to `expired`,
which is not an end: in Dubai the tenant who stays is renewed on the same terms (Law 26 of 2007,
Art. 6). Billing continues for the statutory renewal period and the property manager gets a task
to issue the renewal version and renew Ejari, or record a notice. If the tenant still holds over
when that period ends, billing is extended again and the task repeats.

#### Surfaces
- **Back office:** the property manager is notified when the lease becomes `expired` and
  gets a holding-over task to issue the renewal version or record a notice; the lease shows
  "holding over until" and offers `record_notice`, `renew`, `amend` and `force_terminate`.
- **System:** `PRP.RUL.03` expires the lease the day after its end date and extends the
  charge schedule per statutory period; `PRP.RUL.14` creates the task.

#### Files to read
- `docs/verticals/01-property.md` §2 (Art. 6), §4.3 row "The tenant stays after the end date
  (holding over)", §7.1 *Lease* (states, `expire`, diagram), §7.2 `PRP.RUL.03`, `PRP.RUL.14`,
  §9 "Tenancy law on the negative paths"
- `docs/modules/wl-invoicing.md` §4 *Charge* (scheduled charges)
- `docs/OPEN-ITEMS.md` L-22

#### Terms
- **Holding over:** the tenant stays after the end date without a new contract.
- **Statutory renewal period:** the lease's own period or one year, whichever is shorter.
- **Renewal version:** a new lease version for the same tenant and unit (BL-PRP-05).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `expired` | warning | no | End date passed without renewal or notice; the tenant holds over and, in Dubai, the lease renews on the same terms (Art. 6); billing continues until a renewal version or a notice |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `expire` | renewal_due | expired | system (`PRP.RUL.03`) | end date passed; no notice; no renewal version `active` | charge schedule extended on the same terms for the statutory renewal period (same period or one year, whichever is shorter); if the tenant still holds over when that period ends, the schedule is extended again for another period and the task repeats; property manager notified; holding-over task (`PRP.RUL.14`). A landlord who objects records a notice (`record_notice` with the landlord's ground); there is no separate objection guard |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `PRP.RUL.03` | the day after `lease.end_date` (Asia/Dubai) | lease is `renewal_due`; no renewal version `active` | lease `expire` (holding over, billing continues); notify property manager | — | on | MVP |
| `PRP.RUL.14` | N days after lease → `expired`, and again at the end of each statutory renewal period | no renewal version submitted (a draft or submitted version is enough to stop a new task from being created; `PRP.RUL.03` instead needs an `active` one to prevent expiry); no notice recorded | task to the property manager to issue the renewal version on the same terms and renew Ejari, or record a notice; a task already open closes only when a renewal version is submitted (a draft created after it opened does not close it) | N = 7 | on | MVP |

Transitions leaving `expired`: `record_notice`, `renew`, `amend`, `force_terminate` (other
issues). The rule in Abu Dhabi is `⚠ open: lawyer (L-22)`.

#### Edge cases decided
- **Same terms:** the extension repeats the current version's payment plan (frequency, amount per
  instalment, charge types and VAT categories) for the statutory period. A 1-year lease with 12
  monthly charges of AED 10,000 gets 12 more; a 2-year lease gets 1 year; a 6-month lease gets 6
  months.
- **The lease's end date is not changed**; the extension end is stored as "holding over until".
- **Cheques:** none are linked to the extended charges; invoices are issued on their due dates as
  usual.
- **Abu Dhabi leases:** same behaviour in every emirate until L-22 is answered; no emirate-specific
  logic (`blocked:open-item` for the Abu Dhabi part only).
- **Lease in `notice_given` or `active` at the end date:** not expired (only `renewal_due`).
- **Renewal version `active` on the end date:** `expire` does not run (guard).
- **Second period:** at the end of each extension, if the lease is still `expired`, the schedule
  is extended again for the same length and `PRP.RUL.14` creates a new task.
- **Task stop condition:** two different moments. *Creating* a task: a renewal version in
  `draft` or `pending_signature`, or a recorded notice, means no new task is created.
  *Closing* a task already open: only a submitted version (`pending_signature`) or a recorded
  notice closes it; a draft created after the task was opened leaves it open (`PRP.RUL.14`).
  There is no contradiction: a draft stops the next task, not the current one.
- **Run time:** `PRP.RUL.03` runs at 02:00 Asia/Dubai (§7.2 intro) on the day after the
  end date; a lease ending 2027-12-31 expires on 2028-01-01.
- **Idempotency:** a re-run on the same day does nothing; the extension is created once per
  period.

#### Must not
- Must not treat `expired` as terminal or read-only.
- Must not stop, cancel or reprice charges on expiry.
- Must not create the renewal version automatically.
- Must not add a landlord-objection guard; objection is a `record_notice`.

#### Done when
- [ ] Given a `renewal_due` lease 2027-01-01 to 2027-12-31 at AED 10,000 per month and no notice,
      on 2028-01-01 it is `expired`, 12 monthly charges of AED 10,000 exist from 2028-01-01 to
      2028-12-01, and the property manager is notified.
- [ ] On 2028-01-08, with no renewal version and no notice, the property manager has the task.
- [ ] With a renewal draft created on 2028-01-05, no task is created on 2028-01-08.
- [ ] With the task open from 2028-01-08, a renewal draft created on 2028-01-10 leaves it open;
      submitting that version on 2028-01-12 closes it.
- [ ] Given still `expired` on 2029-01-01, 12 more charges are created and a new task appears.
- [ ] A 2-year lease ending 2027-12-31 gets a 1-year extension; a 6-month lease gets 6 months.
- [ ] A lease with a notice recorded on 2027-11-01 does not expire.
- [ ] From `expired`, `record_notice`, `renew`, `amend` and `force_terminate` are offered.

#### Out of scope
`renew` from `expired` (BL-PRP-07); `record_notice` from `expired` (BL-PRP-09); the notice
deadline (BL-PRP-03).

---

### BL-PRP-07 — 01 Property: renew from `renewal_due` or `expired` without billing twice

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PRP-05 · BL-PRP-06 · **Blocks:** None.
**Review items:** D-01, D-02, 01-N13-a

#### Goal
When a renewal version is activated, the previous version ends as `renewed`, also from `expired`.
Scheduled charges of the old version that the new version covers are cancelled; invoices already
issued for a holding-over period stay, and the new version does not bill those periods again.

#### Surfaces
- **Back office:** on activation the property manager is shown the rent difference per
  period already invoiced and may add a charge or credit by hand; the accountant gets a task
  to relink cheques.
- **System:** `renew` links the versions and cancels the old version's scheduled charges
  that the new one covers, without billing any period twice.

#### Files to read
- `docs/verticals/01-property.md` §7.1 *Lease* (`renew`, `activate`), §4.3 row "holding over"
- `docs/modules/wl-invoicing.md` §4 *Charge* (cancel a scheduled charge)

#### Terms
- **Covered period:** the dates from the renewal version's start date onward.
- **Holding-over invoice:** an invoice issued from a charge created by `expire` (BL-PRP-06).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `renew` | renewal_due, expired | renewed | system (renewal version `activate`) | the renewal version is `active` | links old and new versions; scheduled charges of the old version that the new one covers (due date on or after the new start date) are cancelled; invoices already issued for a holding-over period stay, and the new version does not bill those periods again; a different rent for a period already invoiced is not adjusted automatically: the property manager is shown the difference per period and may add a charge or credit by hand; cheques linked to cancelled charges stay `received` and the accountant gets a task to relink them to the new version's charges |

#### Edge cases decided
- **Back-dated renewal:** a renewal version activated on 2028-03-10 with start date 2028-01-01:
  charges of the old version for January and February that were already issued stay; scheduled
  (not yet issued) old charges from the start date are cancelled; the new version's charge
  schedule skips the periods already invoiced by the old version.
- **Different rent for a period already invoiced:** no automatic adjustment (`renew`
  effects). The property manager is shown the difference per period on activation and may add a
  charge or credit by hand. Changed terms are only possible with timely notice or acceptance
  (BL-PRP-04).
- **Partial period:** a charge whose period straddles the start date is treated as covered if its
  due date is on or after the start date (`renew` effects; avoids prorating).
- **Cheques:** cheques linked to cancelled scheduled charges stay `received` and are relinked by
  the accountant to the new version's charges (task, `renew` effects).
- **Atomicity:** with `activate` (BL-PRP-05).

#### Must not
- Must not credit or void invoices already issued for a holding-over period.
- Must not bill a period twice.

#### Done when
- [ ] Given lease v1 `expired` with holding-over invoices for January and February 2028 issued and
      charges from March scheduled, when renewal v2 starting 2028-01-01 at AED 10,000 per month
      activates on 2028-03-10, then v1 is `renewed`, v1's March–December charges are cancelled,
      the January and February invoices stay, and v2 schedules charges from March only.
- [ ] Given v1 `renewal_due` and v2 starting the day after v1's end, v1 is `renewed` and no charge
      is cancelled or skipped.
- [ ] v1 and v2 are linked both ways.

#### Out of scope
Version submission and activation (BL-PRP-05); holding over (BL-PRP-06).

---

### BL-PRP-08 — 01 Property: amend an active lease mid-term (`amend`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PRP-04 · BL-PRP-05 · BL-PRP-13 · BL-INV-07 · BL-PAY-22 · Base: wl-invoicing Credit note · Base: wl-payments Cheque lifecycle · **Blocks:** None.
**Review items:** 01-N6-a

#### Goal
A mid-term change (rent, payment plan, an added tenant, parking) is made by an amendment version
with an effective date. When the amendment activates, the previous version ends as `amended`;
charges after the effective date are recreated from the new plan; invoices already issued for
those periods are credited and reissued; cheques are relinked, or returned and replaced where the
amounts change; the deposit is carried over with a top-up charge or a part refund; Ejari is
updated.

#### Surfaces
- **Back office:** the leasing agent drafts an amendment version with an effective date on
  the lease and activates it; `submit` and `activate` are refused with "legal case open" or
  an effective date in a past month; the accountant gets a task to collect replacement
  cheques.
- **System:** `amend` recreates the charges, credits and reissues invoices pro rata, relinks
  or returns cheques, runs deposit `adjust`, emits `LeaseAmended`; `PRP.RUL.02` chases the
  Ejari number after 14 days.

#### Files to read
- `docs/verticals/01-property.md` §4.3 row "Mid-term change", §7.1 *Lease* (`amend`, `activate`),
  §7.2 `PRP.RUL.02`, §3.3 `PRP.ACL.04`, `FIN.ACL.01`
- `docs/modules/wl-invoicing.md` §4 *Invoice* (`credit`, `unallocate`), *Credit note*
- `docs/modules/wl-payments.md` §4 *Cheque* (`return`), *Deposit* (row `adjust`)

#### Terms
- **Amendment version:** a lease version of kind `amendment` with an effective date.
- **Relink:** move a cheque's linked charge to the matching new charge.
- **Top-up charge:** a charge for the deposit increase.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `amend` | active, renewal_due, expired | amended | system (amendment version `activate`) | the amendment version is `active` | scheduled charges after the effective date cancelled and recreated by the new version; invoices issued for periods after it credited and reissued (`FIN.ACL.01`): a paid invoice is first unallocated (system `unallocate`, the payment moves to the tenant's credit balance), then credited in full, and the credit is allocated to the reissued invoice; an invoice straddling the effective date is reissued as two pro rata lines; cheques in `received` relinked to new charges of the same amount, or returned with a receipt and a replacement task to the accountant where the amount changes (deposited or cleared cheques are not touched); deposit carried over and stays `held`; a changed amount runs deposit `adjust` (wl-payments): a top-up charge on the effective date, or a partial refund of the held deposit through the Refund flow (`FIN.ACL.04`), the held amount reduced when the refund is paid; event `LeaseAmended` |

§4.3 row: "Lease `amend`: a new version with an effective date is drafted, signed and activated.
The effective date is within the current version's term and not before the first day of the
current month, so closed VAT periods are not touched. Charges after the effective date are
reissued from the new plan; invoices already issued for those periods are corrected by credit
note and a new invoice (a paid one is first unallocated to the tenant's credit balance, credited
in full, and the credit is allocated to the new invoice, any excess kept as credit or refunded on
request through the Refund flow); an invoice whose period straddles the effective date is
reissued as two lines, old terms to the day before and new terms from it, pro rata; cheques are
relinked to the new charges, or returned and replaced where the amounts change; a deposit
increase becomes a top-up charge on the effective date, and a decrease a partial refund of the
held deposit through the Refund flow (wl-payments, `FIN.ACL.04`); an Ejari update task is created
(`PRP.RUL.02`)."

§7.1 *Lease* intro: "Pro rata amounts are daily: amount × days ÷ days in the invoice's period,
rounded half up to 0.01 AED."

`PRP.RUL.02`: N days after lease `activate`, no Ejari number recorded → notify property manager
(N = 14).

Legal case flag (§7.1 *Lease*): "While set, … renewal and amendment versions are refused".

Deposit `adjust` (§7.1 *Deposit*, canonical in wl-payments): held → held; increase: top-up
charge for the difference on the effective date; decrease: refund `request` for the difference
(`FIN.ACL.04`), the held amount falls when it is `paid`; event `DepositAdjusted`.

#### Edge cases decided
- **Effective date:** on or after the current version's start date and before its end date, and
  not before the first day of the current month (`submit` guard), so closed VAT periods are not
  touched without the accountant.
- **Paid invoice after the effective date:** `credit` requires no payments allocated, so the
  allocation is removed first (system `unallocate`; the payment moves to the tenant's credit
  balance), the invoice is credited in full, the reissued invoice is created and the credit is
  allocated to it; any excess stays as the tenant's credit or is refunded on request through the
  Refund flow.
- **Invoice period straddling the effective date:** credited in full and reissued as two lines: old terms up to the day before the effective date (daily pro rata) and
  new terms from it. Daily pro rata = amount × days ÷ days in the invoice's period, rounded half
  up to 0.01 AED.
- **Cheques:** a `received` cheque whose new charge has the same amount is relinked; a different
  amount → the cheque is `return`ed with a receipt and a task asks the accountant to collect a
  replacement. Deposited or cleared cheques are payments and are not touched.
- **Deposit:** a changed amount runs deposit `adjust` (BL-PAY-22) as a system effect of the
  amendment's `activate`; the deposit stays `held`. Increase → top-up charge of the difference
  on the effective date; the held amount rises when it is paid. Decrease → a refund `request`
  for the difference with the deposit as source (Refund flow, `FIN.ACL.04`); the held amount
  falls when the refund is `paid`. Same amount → no adjustment. An unpaid top-up does not block
  `activate`. A second deposit change while one is open is refused by `adjust` (BL-PAY-22).
- **Amending from `expired`:** allowed; the holding-over charges after the effective date are
  replaced like any other.
- **Legal case flag set:** amendment refused (BL-PRP-13).
- **Ejari task:** `PRP.RUL.02` runs from the amendment's activation.

#### Must not
- Must not edit an issued invoice (`FIN.ACL.01`); credit and reissue.
- Must not change the active version in place (`PRP.ACL.04`).
- Must not return a deposited or cleared cheque.

#### Done when
- [ ] Given v1 `active` from 2027-01-01 at AED 120,000 a year in 12 monthly charges, with
      invoices issued to April and cheques `received` for May–December, when an amendment v2
      with effective date 2027-05-01 and rent AED 132,000 activates on 2027-04-20, then v1 is
      `amended`, the May–December charges of v1 are cancelled, v2 creates May–December charges of
      AED 11,000, and `LeaseAmended` is emitted.
- [ ] The 8 cheques of AED 10,000 are returned with a receipt and the accountant has a task to
      collect replacements.
- [ ] With an effective date 2027-04-15, the April invoice (AED 10,000, paid) is unallocated,
      credited and reissued as AED 4,666.67 (14 of 30 days at the old rate) plus AED 5,866.67
      (16 days at the new rate); the payment moves to the tenant's credit balance and is
      allocated to the new invoice.
- [ ] A deposit raised from AED 5,000 to AED 6,000 runs `adjust` and creates a top-up charge of
      AED 1,000 dated 2027-05-01 (`DepositAdjusted` emitted); one lowered to AED 4,000 creates a
      refund request of AED 1,000 from the held deposit, and the held amount is AED 4,000 once it
      is paid; in both cases the deposit stays `held`.
- [ ] With a legal case flag open on v1, `submit` and `activate` of v2 are refused with "legal
      case open".
- [ ] An amendment draft with an effective date in the previous month is refused at `submit`.
- [ ] An Ejari reminder reaches the property manager 14 days after activation if no number was
      recorded.

#### Out of scope
`PRP.ACL.17` (BL-PRP-04); version submission (BL-PRP-05); payment reallocation mechanics
(BL-INV-07).

---

### BL-PRP-09 — 01 Property: record a notice from any running state, with the landlord's statutory notice and the early termination penalty

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: 01 Lease lifecycle · BL-PRP-06 · BL-NTF-05 · **Blocks:** BL-PRP-10, BL-PRP-11
**Review items:** 01-N12-a, D-01

#### Goal
A notice to end the lease can now be recorded from `active`, `renewal_due` and `expired`. It
records the date, the party and the reason. A landlord's notice is a statutory notice that needs
proof of delivery. When the tenant terminates early, the penalty set by the contract is added as
a charge from the catalogue.

#### Surfaces
- **Back office:** the property manager records a notice on the lease with date, party,
  reason, the planned move-out date and an early termination penalty; a landlord's notice
  shows "notice not delivered" until its proof is recorded.
- **System:** the unit moves to `notice_given`, the move-out inspection ticket and the
  penalty charge are created, and the penalty's due date follows the planned move-out date
  while it is not invoiced.

#### Files to read
- `docs/verticals/01-property.md` §4.3 rows "Early termination by the tenant" and "holding over",
  §7.1 *Lease* (`record_notice`), *Unit* (`mark_notice`), §7.2 `PRP.RUL.04`, "Statutory notices",
  §9 (early termination penalty VAT)
- `docs/OPEN-ITEMS.md` L-22, A-06

#### Terms
- **Notice party:** `tenant` or `landlord`.
- **Notice reason:** `end_of_term`, `early_termination`, `landlords_ground`, `non_payment`.
- **Early termination fee:** a charge type in the client's catalogue; its amount comes from the
  contract.
- **Planned move-out date:** the date the tenant will leave, recorded on the notice: entered at
  `record_notice` for a tenant's notice, computed from the proof-of-delivery date for a
  landlord's notice. It is not the actual move-out date (keys returned), which is recorded at
  `terminate` (BL-PRP-11).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `record_notice` | active, renewal_due, expired | notice_given | `leases.manage` | notice date, party and reason (end of term, early termination, landlord's ground, non-payment) recorded; for a tenant's notice, the planned move-out date entered; a landlord's notice is a statutory notice: allowed once sent, and the lease shows "notice not delivered" until its proof of delivery is recorded | unit `mark_notice`; move-out inspection ticket created (`PRP.RUL.04`); early termination: penalty charge per the contract, due on the planned move-out date (moved with it while not invoiced; the actual move-out date is recorded at `terminate` and does not change it); for a landlord's notice, the planned move-out date and the deadlines that depend on the notice are computed from the proof-of-delivery date |

§4.3 row: "Notice reason *early termination*. The penalty set by the contract is added as a
charge from the catalogue (charge type *early termination fee*; its VAT treatment
`⚠ open: accountant (A-06)`)."

Statutory notices (§7.2): "a landlord's notice to end a lease … when a message gives up, the
property manager gets a task to deliver it another way and record the proof … the deadlines that
depend on the notice do not start until then."

#### Edge cases decided
- **Landlord's notice not yet delivered:** `record_notice` is allowed once the notice is sent; the lease shows "notice not delivered" until proof is recorded (BL-NTF-05),
  and the move-out date shown is computed only from the proof date.
- **Penalty amount:** entered by the user from the contract (no formula); due date = the planned
  move-out date. If the planned move-out date is edited while the penalty is not invoiced, the
  due date moves with it; once invoiced it stays. The actual move-out date at `terminate` never
  changes it.
- **Tenant's notice without a planned move-out date:** refused. The charge's VAT category comes from the charge type the client configures;
  there is no platform default while A-06 is open (`blocked:open-item`).
- **No early termination fee charge type configured:** the penalty field is refused with
  "configure the charge type first".
- **From `expired`:** this is how the landlord objects to holding over (reason
  `landlords_ground`).
- **Legal grounds:** which landlord grounds are valid is `⚠ open: lawyer (L-22)`; the platform
  records the ground as text and does not judge it.
- **Concurrent renewal version:** a renewal version in `draft` or `pending_signature` stays; it
  cannot activate while the lease is `notice_given` (BL-PRP-05).

#### Must not
- Must not compute a penalty automatically.
- Must not start a notice-dependent deadline before proof of delivery for a landlord's notice.

#### Done when
- [ ] Given an `expired` lease, `record_notice` (party landlord, reason `landlords_ground`, date
      2028-02-01) makes it `notice_given`, the unit `notice_given` and creates a move-out
      inspection ticket.
- [ ] Given an `active` lease, a tenant notice with reason `early_termination`, planned move-out
      date 2027-08-31 and penalty AED 20,000 adds one *early termination fee* charge of AED
      20,000 due 2027-08-31; editing the planned date to 2027-09-15 before it is invoiced moves
      the due date to 2027-09-15.
- [ ] A tenant notice without a planned move-out date is refused.
- [ ] A landlord's notice whose message gave up shows "notice not delivered" until proof is
      recorded.
- [ ] `record_notice` without party or reason is refused.

#### Out of scope
Withdrawing a notice (BL-PRP-10); termination (BL-PRP-11).

---

### BL-PRP-10 — 01 Property: withdraw a tenant's notice (`withdraw_notice`, `PRP.ACL.20`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PRP-09 · **Blocks:** None.
**Review items:** 01 MEDIUM "tenant withdraws notice" (review §6)

#### Goal
A tenant who gave notice may change their mind. With the landlord side's written agreement, the
property manager withdraws the notice: the lease goes back to the state its dates give, the unit
back to `occupied`, the move-out inspection is cancelled and an uninvoiced penalty is cancelled.
It is refused once a lease for another tenant on the unit was submitted.

#### Surfaces
- **Back office:** the property manager runs `withdraw_notice` on the lease with the written
  agreement attached; it is refused without it, after the move-out inspection, or once
  another tenant's lease was submitted.
- **System:** the lease returns to the state its dates give, the unit to `occupied`, the
  inspection ticket and an uninvoiced penalty are cancelled, and `LeaseNoticeWithdrawn` is
  emitted.

#### Files to read
- `docs/verticals/01-property.md` §3.3 `PRP.ACL.20`, §4.3 row "The tenant withdraws the notice",
  §7.1 *Lease* (`withdraw_notice`), *Unit* (`cancel_notice`)

#### Terms
- **State the dates give:** `active` if today is before the renewal window, `renewal_due` if
  inside it and before the end date, `expired` if after the end date.
- **Landlord side's written agreement:** a document attached to the lease (owner's or property
  manager's signed agreement).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `withdraw_notice` | notice_given | active, renewal_due, expired | `leases.manage` | `PRP.ACL.20`; move-out inspection not completed | back to the state the dates give; when that is `expired` and no holding-over extension exists yet, `expire`'s effects run as if it expired that day; unit `cancel_notice`; move-out inspection ticket cancelled; penalty charge cancelled while not invoiced (an invoiced penalty stays and is corrected only by a credit note); event `LeaseNoticeWithdrawn` |
| `cancel_notice` (Unit) | notice_given | occupied | system (lease `withdraw_notice`) | — | marketing of the unit stops |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PRP.ACL.20` | Withdrawing a tenant's notice needs the landlord side's written agreement; it is refused once a lease for another tenant on the unit has been submitted (`pending_signature` or later; a draft does not count). Withdrawing a landlord's notice uses the same transition and also needs the written agreement document | `STA` | property manager | — |

#### Edge cases decided
- **Renewal window:** computed with `PRP.RUL.01` (BL-PRP-03): end date − 120 days (or the
  longer contract offset).
- **Back to `expired`:** the holding-over extension created earlier is still there; if none was
  created (the notice was given before expiry), `expire`'s effects run (extension and
  `PRP.RUL.14` task) as if it had expired that day (`withdraw_notice` effects).
- **Penalty already invoiced:** stays; corrected only by a credit note, by hand (`FIN.ACL.01`).
- **Another tenant's lease submitted:** any lease on the unit for another tenant in
  `pending_signature` or later; a `draft` does not block (spec: "submitted").
- **Landlord's notice:** withdrawing a landlord's notice uses the same transition and also needs
  the written agreement document (`PRP.ACL.20`).
- **Unit marketing:** listings are Later; in the MVP the unit state change is enough.

#### Must not
- Must not withdraw after the move-out inspection was completed.
- Must not cancel an invoiced penalty.

#### Done when
- [ ] Given a lease ending 2027-12-31 with a tenant notice and today 2027-06-01, with the agreement
      attached, `withdraw_notice` makes it `active`, the unit `occupied`, cancels the inspection
      ticket and the uninvoiced penalty charge, and emits `LeaseNoticeWithdrawn`.
- [ ] With today 2027-10-01 it becomes `renewal_due`.
- [ ] Without the agreement document, refused.
- [ ] With a new lease for tenant U on the unit in `pending_signature`, refused.
- [ ] With the move-out inspection completed, refused.

#### Out of scope
Recording the notice (BL-PRP-09).

---

### BL-PRP-11 — 01 Property: terminate a lease and settle charges, cheques and Ejari after move-out

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PRP-09 · Base: wl-payments Cheque lifecycle · Base: wl-invoicing Credit note · **Blocks:** BL-PRP-12, BL-PRP-17, BL-PRP-32, BL-PRP-33
**Review items:** 01-N12-a, 01 MEDIUM "Ejari cancellation at move-out" (review §6)

#### Goal
When a tenant moves out (end of term or early), `terminate` now also settles the money:
scheduled charges after the move-out date are cancelled, invoices for periods after it are
credited pro rata unless the contract keeps rent due, unused paid amounts are refunded or kept as
credit, later cheques are returned, the tenant portal turns read-only, and the property manager is
chased if the Ejari or Tawtheeq registration is not cancelled within 7 days.

#### Surfaces
- **Back office:** the property manager runs `terminate` with the move-out date and chooses
  whether the tenant's credit is refunded or kept; 7 days later the property manager is
  chased for the Ejari or Tawtheeq cancellation.
- **Portal:** the tenant's portal turns read-only after move-out.
- **System:** in one transaction, later charges are cancelled, invoices are credited pro
  rata, later cheques are returned and the deposit settlement starts; `PRP.RUL.13` runs 7
  days later.

#### Files to read
- `docs/verticals/01-property.md` §4.3 step 4 and row "Early termination by the tenant", §7.1
  *Lease* (`terminate`), *Unit* (`vacate`), *Cheque* (`return`), §7.2 `PRP.RUL.13`, §6 *Lease*
  (Ejari no. and cancellation date)
- `docs/modules/wl-invoicing.md` §4 *Credit note*

#### Terms
- **Move-out date:** the date the keys were returned (recorded at `terminate`).
- **Pro rata credit:** a partial credit note for the days of an invoice's period after the
  move-out date.
- **Rent due until:** an optional date from the contract until which rent stays due even after
  move-out.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `terminate` | notice_given | terminated | `leases.manage` | move-out inspection completed; all keys returned | unit `vacate`; deposit `start_settlement`; scheduled charges after the move-out date (or after `rent_due_until`, when set) cancelled; rent and recurring lines of invoices for periods after it credited pro rata by partial credit note; a credit on a paid invoice goes to the tenant's credit balance, applied to the deposit settlement's deductions first, the rest refunded through the Refund flow or kept as credit (property manager's choice); cheques in `received` or `due_for_deposit` dated after that date `return`; all in one transaction; Ejari cancellation task (`PRP.RUL.13`); tenant portal read-only (`POR.ACL.01`) |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `PRP.RUL.13` | N days after lease → `terminated` | Ejari or Tawtheeq registration recorded; no cancellation date recorded | notify the property manager | N = 7 | on | MVP |

§4.3 row: "On `terminate`, scheduled charges after the move-out date are cancelled; invoices for
periods after it are credited pro rata (rent and recurring charges only; one-off charges stay),
unless the contract keeps rent due until a fixed date (lease field `rent_due_until`); a partial
credit note on a paid invoice adds its amount to the tenant's credit balance, which is applied to
the deposit settlement's deductions first and the rest refunded through the Refund flow, or kept
as credit when the property manager chooses; later cheques are returned."

§7.1 *Lease* intro: "Pro rata amounts are daily: amount × days ÷ days in the invoice's period,
rounded half up to 0.01 AED."

`POR.ACL.01`: after move-out, the tenant portal is read-only, then closes after 90 days.

#### Edge cases decided
- **Rent due until:** the optional lease field `rent_due_until` (§6); when set, charges and invoices up to that date are kept and only
  later ones are cancelled or credited.
- **Pro rata:** daily = invoice line amount × days after the move-out date ÷ days in the period,
  rounded half up to 0.01 AED (§7.1 *Lease* intro). Only rent and recurring charges are
  prorated; one-off charges (penalty, fines) stay.
- **Paid invoice credited:** a partial credit note on a paid invoice puts its amount on the
  tenant's credit balance (no `unallocate` is needed for a partial credit). Default: applied to
  the deposit settlement's deductions first, then the remainder refunded through the wl-payments
  Refund flow (`FIN.ACL.04`); the property manager may instead keep it as the tenant's credit.
- **Cheques:** cheques in `received` or `due_for_deposit` dated after the move-out date (or after
  `rent_due_until`) are `return`ed; cheques for kept periods stay.
- **Unit:** `vacate` to `under_maintenance` if the move-out inspection opened tickets, else
  `available` (existing rule).
- **`PRP.RUL.13`:** fires once, 7 days after `terminated` (02:00 Asia/Dubai, §7.2 intro),
  only if a registration number exists and no cancellation date.
- **Retry:** if crediting fails for one invoice, the whole `terminate` rolls back (one
  transaction).

#### Must not
- Must not credit periods kept due by the contract.
- Must not return cheques for periods that stay due.
- Must not delete the tenant's portal access at once; read-only first.

#### Done when
- [ ] Given a lease with monthly invoices of AED 9,000 issued to July 2027 (all paid), charges
      scheduled from August, and 5 cheques `received` for August–December, when it terminates
      with move-out date 2027-07-20, then August–December charges are cancelled, the July invoice
      gets a credit note of AED 3,193.55 (11 of 31 days, 21–31 July) that goes to the tenant's
      credit balance for the deposit settlement, the 5 cheques are `returned` with a receipt, the deposit is `settlement_pending`, the unit is vacated and the tenant portal is
      read-only.
- [ ] With `rent_due_until = 2027-09-30`, August and September charges and cheques stay; only
      October–December are cancelled and returned; July is not credited.
- [ ] With an Ejari number and no cancellation date, the property manager is notified on
      2027-07-27; with a cancellation date recorded on 2027-07-25, nothing is sent.

#### Out of scope
Forced termination (BL-PRP-12); deposit settlement and disputes (BL-PAY-15, BL-PRP-16,
BL-PRP-17); the early termination penalty (BL-PRP-09).

---

### BL-PRP-12 — 01 Property: force-terminate a lease for an abandoned unit or an executed eviction (`PRP.ACL.19`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PRP-11 · BL-PRP-13 · BL-KRN-01 · BL-PAY-15 · **Blocks:** None.
**Review items:** 01-N9-a, 01-N3-b

#### Goal
When a tenant abandons the unit or is evicted by a court, the lease cannot end through the normal
notice and key return. The property manager requests a forced termination with evidence; another
user with `leases.force_terminate` (default company admin) approves. The lease ends as
`terminated` from any running state, the unit is vacated, the deposit is applied to arrears and
damages, and the legal case flag is closed with the outcome.

#### Surfaces
- **Back office:** the property manager requests `force_terminate` with reason *abandoned*
  or *eviction executed*, the required attachments and a "keep as evidence" or return choice
  per later cheque; the company admin approves under "Needs your action"; a missing
  attachment is named in the refusal.
- **System:** on approval the lease is `terminated` as in `terminate`, the unit vacated, the
  deposit sent to settlement, the legal case flag closed with its outcome and
  `LeaseForceTerminated` emitted.

#### Files to read
- `docs/verticals/01-property.md` §3.3 `PRP.ACL.19`, §4.3 row "The tenant abandons the unit or is
  evicted", §4.1 row "The tenant abandons the unit", §7.1 *Lease* (`force_terminate`, legal case
  flag), *Unit* (`vacate`), §9 "Tenancy law on the negative paths"
- `docs/OPEN-ITEMS.md` L-22

#### Terms
- **Abandonment inspection:** an inspection done without the tenant, with photos.
- **Legal basis for repossession:** the document or reference that allows repossession of an
  abandoned unit (what counts is open, L-22).
- **Judgment and execution record:** the court judgment and the execution report of an eviction.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `force_terminate` | active, renewal_due, expired, notice_given | terminated | `leases.force_terminate` | `PRP.ACL.19`; reason `abandoned` (abandonment inspection with photos and the legal basis for repossession, `⚠ open: lawyer (L-22)`) or `eviction_executed` (judgment and execution record) | as `terminate`, with the inspection done without the tenant; the deposit settlement applies the deposit to arrears and damages; remaining arrears stay on the invoices; the legal case flag is closed with the outcome; event `LeaseForceTerminated` |
| `vacate` (Unit) | notice_given, occupied | available, under_maintenance | system (lease `terminate` or `force_terminate`) | — | `under_maintenance` if the move-out inspection opened tickets |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PRP.ACL.19` | A forced termination (unit abandoned, or eviction executed) is requested by the property manager with evidence and approved by another user with `leases.force_terminate` | `SOD` | property manager | approver: company admin |

§4.3 row: "The unit is vacated; the deposit settlement applies the deposit to arrears and damages;
later cheques are kept as evidence or returned; remaining arrears follow the arrears flow or
write-off."

#### Edge cases decided
- **Move-out date:** the abandonment inspection date, or the execution date for an eviction.
- **Later cheques:** the requester chooses per cheque "keep as evidence" (stays `received`, marked
  evidence, never deposited) or `return`. Default: keep as evidence while a legal case flag is
  open.
- **Deposit:** the deposit goes to `settlement_pending`; the settlement uses the deductions flow
  with the arrears and damages; applying it is BL-PRP-17. The deductions are still published; the
  tenant's portal is read-only, so a dispute can only come through staff or a Rental Dispute
  Centre case.
- **Evidence (eviction):** two required attachments, document kinds *judgment* and *execution
  record*; either one missing refuses the request, naming the missing one.
- **Legal basis (abandoned):** a required attachment of kind *repossession basis* plus a text;
  the platform does not judge it (L-22 open, `blocked:open-item`).
- **No legal case flag:** allowed; there is nothing to close.
- **Approval pending:** the lease keeps its state; billing continues; on reject or return the
  lease stays and the requester is told (BL-KRN-01).
- **Arrears:** invoices for periods up to the move-out date stay open and follow `PRP.RUL.10` or
  write-off; invoices after it are credited pro rata as in `terminate`.

#### Must not
- Must not let the requester approve.
- Must not run without the evidence for the chosen reason.
- Must not write off arrears automatically.

#### Done when
- [ ] Given an `active` lease with arrears of AED 30,000 and a deposit of AED 8,000, when property
      manager A requests `force_terminate` reason `abandoned` with inspection photos and a
      repossession basis, then an approval request goes to the company admin and the lease stays
      `active`.
- [ ] A cannot approve. When company admin B approves, the lease is `terminated`, the unit is
      vacated, the deposit is `settlement_pending`, the invoices to the move-out date stay open,
      and `LeaseForceTerminated` is emitted.
- [ ] Reason `eviction_executed` needs two attachments: a judgment and an execution record.
      With only the judgment attached it is refused ("execution record missing"); with only the
      execution record it is refused ("judgment missing"); with both, the request is created.
- [ ] An open legal case flag of type eviction is closed with outcome "eviction executed".
- [ ] Cheques marked "keep as evidence" stay `received` and never appear on the deposit list.

#### Out of scope
Applying the deposit and invoicing excess damages (BL-PRP-17); the legal case flag (BL-PRP-13);
arrears escalation (BL-PRP-14).

---

### BL-PRP-13 — 01 Property: set and clear the legal case flag on a lease

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 01 Lease lifecycle · BL-INV-04 · BL-PAY-12 · BL-PAY-15 · BL-TKT-11 · **Blocks:** BL-PRP-08, BL-PRP-12, BL-PRP-14
**Review items:** 01-N3-b, 01-N4-c

#### Goal
When a tenant does not pay after the formal demand, or a bounced cheque or a deposit dispute goes
to the Rental Dispute Centre or a court, the lease carries a legal case flag. While set, reminders
on its invoices pause, renewal and amendment versions are refused and the tenant gets no
automatic messages about the case. It is cleared with an outcome.

#### Surfaces
- **Back office:** the property manager sets a legal case flag on the lease (type,
  reference, new or picked *legal case* ticket) and clears it with an outcome; renewal and
  amendment `submit` show "legal case open".
- **System:** while a flag is open, invoice reminders, `PRP.RUL.10` tasks and automatic
  tenant messages are suppressed; cheque `refer_to_legal` sets a flag and clearing a flag
  resolves its ticket.

#### Files to read
- `docs/verticals/01-property.md` §7.1 *Lease* (paragraph "Legal case flag"), *Cheque*
  (`refer_to_legal`), *Deposit* ("Deductions dispute flag"), §4.3 rows "The tenant does not pay
  after the formal demand" and "The tenant disputes the deductions", §6 *Lease* (legal case flag)

#### Terms
- **Case type:** `arrears`, `eviction`, `deposit_dispute`, `other`.
- **Reference:** the Rental Dispute Centre or court case number.
- **Legal case ticket:** a wl-ticketing ticket of category *legal case* linked to the flag.
- **Outcome:** `paid`, `settled`, `judgment`, `eviction_executed`.

#### Spec excerpt (authoritative for this issue)

"**Legal case flag** (not a state). The property manager sets it on a lease with a case type
(arrears, eviction, deposit dispute, other), the Rental Dispute Centre or court reference, and a
linked ticket of category *legal case*. While set, reminders on the lease's invoices are paused
(as for an invoice `dispute`), renewal and amendment versions are refused, and the tenant is not
sent automatic messages about the case; invoices still become `overdue`, and on clearing
reminders resume from the next scheduled offset. The reference may be empty while the case is
prepared, but is required to clear with outcome *judgment*. A lease may carry several flags; the
effects apply while any is open, and each is cleared separately, with an outcome (paid, settled,
judgment, eviction executed). Every flag has its ticket: setting the flag creates the linked
*legal case* ticket (assigned to the property manager, subject the lease), unless the property
manager picks an open *legal case* ticket of the same lease that has no flag yet; a cheque
`refer_to_legal` creates the flag and its ticket the same way. Clearing the flag resolves its
ticket if still open; resolving the ticket by hand does not clear the flag."

| Transition (Cheque) | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `refer_to_legal` | bounced | legal_case | `cheques.legal` | return certificate attached | case task for the property manager; legal case flag on the lease |

Deposit: "an open case at the Rental Dispute Centre keeps the flag until the decision."
§4.3: "The lease carries a legal case flag (arrears or eviction), which pauses reminders on its
invoices and blocks renewal; the case ends with payment, a settlement, or eviction
(`force_terminate`)."
`PRP.RUL.10` condition: "no legal case flag on the lease".

#### Edge cases decided
- **Ticket:** never a flag without a ticket. Setting the flag creates the *legal case* ticket
  (category seeded by BL-TKT-11) in the same transaction, or links an open *legal case* ticket
  of the same lease that has no flag yet, if the property manager picks one. Clearing the flag
  resolves its ticket if still open; resolving the ticket by hand does not clear the flag.
- **Who sets it:** a property manager (`leases.manage` in their building). Reference may be empty
  at first (case being prepared); the reference is required to clear with outcome `judgment`.
- **Cheque `refer_to_legal`:** sets the flag with type `arrears` and links the cheque; the property
  manager completes the reference.
- **Deposit dispute at the Rental Dispute Centre:** sets the flag with type `deposit_dispute`; the
  deposit's deductions dispute flag stays until this case is cleared.
- **Several cases:** a lease may carry several flags; the effects apply while any is open; each
  is cleared separately.
- **Reminders:** `FIN.RUL.01` and `PRP.RUL.10` skip every invoice of the lease; `FIN.RUL.02` still
  marks invoices overdue (the due date is a fact). On clearing, reminders resume from the next
  scheduled offset; missed offsets are not sent.
- **Automatic messages:** reminders, overdue notices and bounce reminders (`FIN.RUL.18`) to the
  tenant are suppressed for the lease; staff may still send a message by hand.
- **Renewal or amendment in progress:** a version in `draft` stays; `submit` and `activate` are
  refused while a flag is open.
- **Terminated lease:** the flag can stay open on a terminated lease (arrears after move-out).
- **Clearing with `eviction_executed`:** done by `force_terminate` (BL-PRP-12).

#### Must not
- Must not send automatic reminders while a flag is open.
- Must not show the case to the tenant in the portal beyond what staff publish.
- Must not clear a flag without an outcome.

#### Done when
- [ ] Given a lease with 2 overdue invoices, when property manager A sets a flag (type arrears,
      reference "RDC-2027-1234") with a legal case ticket, then no reminder or `PRP.RUL.10` task
      is created for the lease's invoices, and the invoices still become `overdue`.
- [ ] Setting that flag created one *legal case* ticket linked to it; with an existing open
      *legal case* ticket of the lease picked instead, no new ticket is created.
- [ ] Resolving the ticket by hand leaves the flag open; clearing the flag with outcome `paid`
      resolves its ticket if still open.
- [ ] A renewal version `submit` is refused with "legal case open".
- [ ] A cheque `refer_to_legal` sets a flag of type arrears linked to the cheque and the lease.
- [ ] Clearing with outcome `settled` resumes reminders; clearing without an outcome is refused.
- [ ] With a deposit dispute case open, the deposit's dispute flag stays until the case is cleared.

#### Out of scope
Cheque legal case state (BL-PAY-12); the deposit dispute flag (BL-PAY-15); arrears escalation
(BL-PRP-14); forced termination (BL-PRP-12).

---

### BL-PRP-14 — 01 Property: escalate arrears to a formal demand and a legal case (`PRP.RUL.10`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PRP-13 · BL-NTF-05 · BL-INV-04 · **Blocks:** None.
**Review items:** 01-N3-b

#### Goal
Arrears that grow after the last reminder are escalated: 30 days after the due date the property
manager gets a task to send a formal payment demand (a statutory notice with proof of delivery).
When the demand period lapses unpaid, a second task asks to open a legal case on the lease or to
agree a payment plan.

#### Surfaces
- **Back office:** the property manager gets one "send formal payment demand" task per lease
  listing the overdue invoices and sends the statutory notice from it; when the demand
  period lapses unpaid, a task "open a legal case or agree a payment plan".
- **System:** the daily `PRP.RUL.10` run builds and updates the demand list and starts the
  M-day clock only at the proof-of-delivery date.

#### Files to read
- `docs/verticals/01-property.md` §4.1 row "Arrears grow after the last reminder", §7.2
  `PRP.RUL.10`, "Statutory notices", §7.1 *Invoice* ("Dispute flag"), §13 question 12
- `docs/modules/wl-notify.md` §5.4 (`statutory_notice`)
- `docs/OPEN-ITEMS.md` L-22

#### Terms
- **Formal payment demand:** a statutory notice to the tenant listing the overdue amounts and the
  demand period.
- **Demand period (M):** days after the demand's proof of delivery before the legal-case task.
- **Payment plan:** an agreement with the tenant to pay the arrears on new dates.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `PRP.RUL.10` | N days after an invoice's due date | balance > 0; not disputed; no legal case flag on the lease | task to the property manager to send a formal payment demand (statutory notice), one open task per lease listing every invoice of the lease overdue by N days or more (later ones are added to it until the demand is sent; disputed or written-off ones removed); sending the demand closes the task and freezes its list: the demand and its M-day clock cover only the invoices it lists, and an invoice reaching N days afterwards opens a new demand task (invoices on a sent demand are not listed again); M days after its proof of delivery, any listed invoice still unpaid: task to open a legal case or agree a payment plan. An agreed payment plan is recorded as a note with the agreed dates on the lease; issued invoices are not changed and the rule stays quiet for them until the latest agreed date | N = 30; M = 30 (`⚠ open: lawyer (L-22)`) | on | MVP |

§4.1 row: "the property manager gets a task to send a formal payment demand (statutory notice;
proof of delivery recorded). When the demand period lapses unpaid, a task to open a legal case
(Rental Dispute Centre claim for arrears or eviction) on the lease, or to agree a payment plan. The
demand period and the grounds for eviction are `⚠ open: lawyer (L-22)`."

Invoice dispute flag: "reminders and the arrears escalation `PRP.RUL.10` pause for it."

#### Edge cases decided
- **Per lease, not per invoice:** one open demand task per lease (`PRP.RUL.10`); a second
  invoice reaching N days while the task is open (demand not yet sent) is added to it. The
  demand lists all invoices of the lease overdue by N days or more when it is sent.
- **Invoice reaching N days after the demand was sent:** sending closes the task and freezes the
  list; the sent demand's M-day clock covers only the invoices it lists. The later invoice opens
  a new demand task (a new statutory notice with its own proof and its own M-day clock);
  invoices already on a sent demand are not listed again.
- **Demand sent:** the property manager sends the demand from the task (template, statutory type);
  the M-day clock starts at the proof-of-delivery date, never at sending (BL-NTF-05).
- **Still unpaid:** any of the listed invoices still has a balance > 0 on day M. Partly paid still
  counts.
- **Payment plan:** no entity exists; recorded as a note with the agreed
  dates on the lease and the task closes; issued invoices are not changed; `PRP.RUL.10` stays
  quiet for those invoices until the latest agreed date, then runs again if unpaid.
- **Invoice disputed or written off meanwhile:** removed from the demand list; if nothing is
  left, the task closes.
- **Legal case opened:** from the second task the property manager sets the legal case flag
  (BL-PRP-13); that stops further `PRP.RUL.10` tasks.
- **Run time:** daily at 02:00 Asia/Dubai (§7.2 intro). An invoice due 2027-03-01 gets
  the task on 2027-03-31.
- **L-22:** N and M are parameters; the defaults are built and flagged `blocked:open-item`.

#### Must not
- Must not send the demand automatically.
- Must not start the demand period before proof of delivery.
- Must not escalate a disputed invoice or a lease with a legal case flag.

#### Done when
- [ ] Given an invoice of AED 12,000 due 2027-03-01, unpaid and not disputed, on 2027-03-31 the
      property manager has one task "send formal payment demand".
- [ ] With that demand not yet sent, a second invoice of the same lease due 2027-04-01 is added
      to the open task on 2027-05-01, not a new task.
- [ ] Demand delivered with proof on 2027-04-05; still unpaid on 2027-05-05: task "open a legal
      case or agree a payment plan".
- [ ] Demand message given up and no proof recorded: no second task.
- [ ] Given the demand for the 2027-03-01 invoice sent on 2027-04-03 (task closed), an invoice
      due 2027-04-10 reaching 30 days on 2027-05-10 opens a new demand task listing only that
      invoice; the first demand's legal-case task still comes on day 30 after its own proof.
- [ ] With the invoice disputed on 2027-03-20, no task on 2027-03-31.
- [ ] With a legal case flag on the lease, no task.

#### Out of scope
The legal case flag (BL-PRP-13); the statutory notice mechanics (BL-NTF-05); reminders
(`FIN.RUL.01`, base).

---

### BL-PRP-15 — 01 Property: refuse new lease versions and hold money out for a blocked tenant

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PTY-04 · BL-PRP-05 · **Blocks:** None.
**Review items:** X-18

#### Goal
While a tenant is blocked, its active lease continues (block effect `continue`, decided by the
property manager), but no new lease, renewal or amendment is submitted or activated for it, and
deposit refunds wait for unblock or an approved exception. 01 calls the shared guard in its own
transitions.

#### Surfaces
- **Back office:** for a blocked tenant, `submit` and `activate` of a new lease, renewal or
  amendment are refused with the neutral text "party on hold", and a deposit `refund`
  becomes an exception request to the compliance role.
- **System:** the lease transitions and the deposit `refund` call `BlockGuard` with the
  matching action kind; the active lease keeps billing and accepting payments.

#### Files to read
- `docs/verticals/01-property.md` §3.3 `PTY.ACL.03`, §7.1 *Lease* (`submit`, `activate`, paragraph
  "Party blocked"), *Deposit* (`refund` guard), negative-flow settings table (row Lease, Deposit)
- `docs/modules/wl-parties.md` §5.6

#### Terms
- **Blocked party:** a party put on hold in wl-parties; the reason may be restricted.
- **`BlockGuard`:** the shared check `BlockGuard::check(party, action_kind)` with action kinds
  `start`, `renew`, `extend`, `money_out` (BL-PTY-04).

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PTY.ACL.03` *(shared)* | While a party is blocked, no new lease, renewal or amendment is activated for it, and no money goes out to it (deposit refund, owner payout, vendor payment) unless the compliance role approves an exception. Active leases continue (block effect `continue`) | `STA` | all staff | — |

"**Party blocked.** A blocked tenant's active lease continues (block effect `continue`, decided by
the property manager); new leases, renewals and amendments for it are refused; deposit refunds wait
for unblock or an approved exception (`PTY.ACL.03`)."

Lease `submit` and `activate` guards: "tenant not blocked". Deposit `refund` guard: "tenant not
blocked".

#### Edge cases decided
- **Action kinds:** new lease `submit`/`activate` → `start`; renewal → `renew`; amendment →
  `extend`; deposit `refund` → `money_out`.
- **Blocked between submit and activate:** `activate` re-checks and is refused; the version stays
  `pending_signature`.
- **Holding over:** `expire` and its charge extension still run (the lease continues).
- **Refusal text:** neutral ("party on hold") for users without `PTY.ACL.02`.
- **Money in:** rent payments are recorded as usual.
- **Owner payouts:** handled in BL-PRP-20.

#### Must not
- Must not end or pause the active lease automatically.
- Must not show a restricted block reason.

#### Done when
- [ ] Given tenant T blocked, a renewal version `submit` for T is refused with the neutral text.
- [ ] A version in `pending_signature` before the block cannot be activated after it.
- [ ] T's active lease keeps issuing invoices and accepts payments.
- [ ] A deposit `refund` for T creates an exception request to the compliance role and no refund.

#### Out of scope
The guard and exception (BL-PTY-04); owner payouts (BL-PRP-20).

---

### BL-PRP-16 — 01 Property: escalate an unanswered deposit deductions dispute (`PRP.RUL.11`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PAY-15 · **Blocks:** None.
**Review items:** 01-N4-c

#### Goal
After deposit deductions are published, the tenant has 14 days to dispute. When the window ends
with no dispute, the accountant is told the settlement may proceed. A dispute left 14 days without
an answer or agreement is escalated to the property manager's manager.

#### Surfaces
- **Back office:** the accountant is told that the deposit settlement may proceed once the
  window ends with no dispute; the property manager's manager is told about a dispute left
  unanswered for 14 days.
- **System:** the daily `PRP.RUL.11` run counts the window from the latest published version
  and the escalation from the day the dispute was opened.

#### Files to read
- `docs/verticals/01-property.md` §4.3 step 4 and row "The tenant disputes the deductions", §7.2
  `PRP.RUL.11`, §7.1 *Deposit* ("Deductions dispute flag")

#### Terms
- **Published:** a numbered version of the itemised deductions shown to the tenant in the portal.
- **Answer:** the property manager revises (new version) or upholds the disputed lines.
- **Property manager's manager:** the user set as manager of the property manager in the staff
  profile; if none, the company admin (`PRP.RUL.11`).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `PRP.RUL.11` | N days after deposit deductions are published | deposit `settlement_pending` | no line disputed: notify the accountant that the settlement may proceed (once per published version; a new version restarts N); a dispute open for M days without an answer or agreement, counted from the day it was opened and while no `deposit_dispute` legal case flag is open: notify the property manager's manager (the manager set on the property manager's staff profile, otherwise the company admin) | N = 14; M = 14 | on | MVP |

§4.3 row: "the property manager answers within 7 days with evidence (inspection photos, quotes),
revises the deductions (a new published version) or upholds them. There is no separate reminder
at 7 days; unresolved after 14 days, `PRP.RUL.11` escalates to the property manager's
manager".

#### Edge cases decided
- **Days:** calendar days in Asia/Dubai from the publication date of the latest version; run at
  02:00 (§7.2 intro). Published 2027-03-01 → accountant told on 2027-03-15.
- **New version:** restarts N (the window restarts, BL-PAY-15).
- **M counted from:** the date the dispute was opened.
- **Response window of 7 days:** no separate reminder at 7 days; the escalation at M = 14 covers
  it (§4.3 row).
- **Legal case on the lease:** no escalation while a `deposit_dispute` legal case flag is open.
- **Idempotency:** one notice per published version and one escalation per dispute.

#### Must not
- Must not refund or forfeit from this rule.

#### Done when
- [ ] Given deductions published 2027-03-01 and no dispute, the accountant is notified on
      2027-03-15.
- [ ] Given a dispute opened 2027-03-05 and not answered, the manager is notified on 2027-03-19.
- [ ] Given a revision published 2027-03-10, the accountant notice moves to 2027-03-24.

#### Out of scope
The dispute flag, revise and uphold (BL-PAY-15).

---

### BL-PRP-17 — 01 Property: apply the deposit in full and invoice deductions that exceed it

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PAY-15 · BL-PRP-11 · Base: wl-invoicing Invoice lifecycle · **Blocks:** BL-PRP-32
**Review items:** 01-N3-e

#### Goal
When the approved deductions (damages, unpaid rent) exceed the deposit, the deposit is applied in
full (`forfeit` with reason *applied to deductions*) and the excess is invoiced to the tenant as a
damages charge, which then follows the normal arrears flow.

#### Surfaces
- **Back office:** the accountant runs deposit `forfeit` with reason *applied to
  deductions*; a property manager approves, plus finance or the owner above AED 5,000; the
  settlement statement shows deposit, deductions and the excess invoice number.
- **System:** on approval the deposit is applied to unpaid rent oldest first, then to
  damages, and the rest of the damages is issued as one damages invoice.

#### Files to read
- `docs/verticals/01-property.md` §4.3 row "Deductions exceed the deposit", §7.1 *Deposit*
  (`forfeit`), §6 *Deposit* (excess invoiced), §9 "Deposits and damages"
- `docs/OPEN-ITEMS.md` A-06

#### Terms
- **Deductions:** the itemised, published amounts the client keeps from the deposit.
- **Total deductions:** unpaid rent deducted plus damage lines; both count toward "exceed the
  deposit" and toward the `PRP.ACL.05` threshold.
- **Excess:** total deductions − deposit amount. Only the damage part of it is newly invoiced:
  unpaid rent is already on invoices, which simply stay open for what the deposit did not
  cover.
- **Damages charge:** a charge type in the catalogue for damage costs invoiced to the tenant.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `forfeit` | settlement_pending | forfeited | `deposits.refund` | reason given; approval by a property manager (above `PRP.ACL.05`, also finance or the owner); the forfeiture published to the tenant as the deductions it is applied to (*applied to deductions*), or as one line for the whole deposit with the reason; the 14-day dispute window of the latest published version has passed and no line is disputed (`PRP.RUL.11`), as for `refund` | settlement statement generated (deposit, deductions, excess invoice number); when the deductions exceed the deposit, the deposit is applied first to unpaid rent and then to damages, and the excess is invoiced to the tenant (§4.3) |

§4.3 row: "The deposit is applied in full (`forfeit`, reason *applied to deductions*): first to
unpaid rent invoices, oldest first, as a payment of method *deposit applied*, then to the damage
lines. What is left of the damage lines is invoiced to the tenant as one damages invoice (charge
type *damages*), which then follows the arrears flow (§4.1). VAT on damages kept as compensation
is `⚠ open: accountant (A-06)`."

`PRP.ACL.05`: "Deposit deductions are approved by the property manager; above a threshold,
checked on the total of the deductions (a forfeit included), finance or the owner also approves"
(AED 5,000).

§9: "a deposit applied to unpaid rent takes the VAT of the rent; an amount kept as compensation
for damage is generally outside the scope of VAT, depending on the contract."

#### Edge cases decided
- **Order of application:** the deposit is applied first to unpaid rent
  invoices (oldest first, as a payment of method *deposit applied*, so the rent's VAT is already
  on its invoice), then to damage lines; the excess is what is left of the damage lines.
- **Excess invoice:** one invoice with one line per remaining damage deduction, charge type
  *damages*, VAT category from the charge type as configured by the client (no platform default
  while A-06 is open, `blocked:open-item`); due date = issue date + the client's standard terms.
- **Approval:** the `forfeit` approval by a property manager covers the excess invoice; above
  `PRP.ACL.05` (AED 5,000, checked on the total deductions) the second approver also approves.
- **Dispute window:** the forfeiture is published to the tenant as the deductions it is applied
  to; `forfeit` runs only after the 14-day window of the latest published version (§7.1
  *Deposit* `forfeit` guard).
- **Dispute:** `forfeit` waits while any line is disputed (BL-PAY-15).
- **Blocked tenant:** forfeit is not money out; allowed.
- **Rounding:** 0.01 AED; the new damages invoice equals the sum of unapplied damage amounts
  exactly.
- **Unpaid rent alone exceeds the deposit:** the deposit is applied to the rent invoices oldest
  first; no damages invoice; the rest of the rent stays open on its invoices.
- **Deductions below the deposit:** not this issue's path: `forfeit` is not used; the deductions
  are kept on `refund` and the rest is paid back to the tenant (BL-PAY-15). Deductions exactly
  equal to the deposit: `forfeit` with *applied to deductions*, no excess invoice. Reason
  *applied to deductions* is refused when the total deductions are below the deposit.

#### Must not
- Must not issue a credit note or refund when deductions exceed the deposit.
- Must not apply VAT to the deposit itself.

#### Done when
- [ ] Given a deposit of AED 5,000.00, an unpaid rent invoice of AED 2,000.00 and damage
      deductions of AED 4,500.00 (window passed, no dispute), when `forfeit` with reason *applied
      to deductions* is approved, then AED 2,000.00 is allocated to the rent invoice, AED 3,000.00
      covers damages, and a damages invoice of AED 1,500.00 is issued to the tenant.
- [ ] The deposit is `forfeited` and the settlement statement shows deposit, deductions and the
      excess invoice number.
- [ ] With a line disputed, `forfeit` is refused; before the 14-day window after publication has
      passed, it is refused too.
- [ ] The excess invoice becomes overdue and enters `PRP.RUL.10` like any rent invoice.
- [ ] In the first case the total deductions are AED 6,500.00, above AED 5,000, so a second
      approver (finance or the owner) is required.
- [ ] Given a deposit of AED 5,000.00 and unpaid rent of AED 7,000.00 (two invoices of AED
      3,500.00) and no damages, `forfeit` pays the older invoice in full and AED 1,500.00 of the
      newer; AED 2,000.00 stays open on it and no damages invoice is issued.
- [ ] Given a deposit of AED 5,000.00 and damages of AED 1,200.00, `forfeit` with reason
      *applied to deductions* is refused; the settlement goes through `refund` with AED
      3,800.00 paid back.

#### Out of scope
Deductions dispute (BL-PAY-15); escalation (BL-PRP-16).

---

### BL-PRP-18 — 01 Property: record the owner's consent to a write-off and show write-offs and recoveries on the owner statement

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-INV-01 · BL-INV-03 · BL-PRP-19 · **Blocks:** None.
**Review items:** X-01 (01 values)

#### Goal
In a managed building the loss from an unpaid rent invoice is the owner's. A write-off in 01 is
always approved by a property manager who did not request it, and for a managed building the
owner's consent is recorded on the request. The written-off amount appears on the owner's
statement, and a later recovery is credited there.

#### Surfaces
- **Back office:** the accountant attaches the owner's consent to a write-off request for a
  managed building; a property manager other than the requester approves it; the owner
  statement shows "rent written off" and later "recovered after write-off".
- **System:** the lines go to the statement of the approval or recovery period, or to the
  next statement when that one is already published.

#### Files to read
- `docs/verticals/01-property.md` §3.3 `FIN.ACL.15`, §4.1 row "A debt is uncollectable", §7.1
  *Invoice* (`write_off`, `recover`)
- `docs/modules/wl-invoicing.md` §4 *Invoice* (`write_off`, `recover`)

#### Terms
- **Managed building:** a building whose owner has a management agreement with the client (the
  client is not the owner).
- **Owner's consent:** a document (e-mail, letter) or a recorded note with date and the owner
  contact who gave it.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `FIN.ACL.15` *(shared)* | Writing off a tenant's or owner's unpaid balance always needs approval by a property manager who did not request it; for a managed building the owner's consent is recorded, because the loss is the owner's | `LIM`, `SOD` | accountant | AED 0 (always) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `write_off` | issued, partially_paid, overdue | written_off | `invoices.write_off` | reason; not disputed; `FIN.ACL.15` (always approval in 01; owner's consent for a managed building) | reminders stop; write-off notice to the tenant; bad-debt relief proposed for commercial VAT (`FIN.RUL.14`); shown on the owner statement |
| `recover` | written_off | written_off, paid | system (payment allocated) | — | recovered amount recorded; owner statement credited |

#### Edge cases decided
- **Consent required when:** the invoice's issuing entity is the owner of a managed building (not
  the management company's own fee invoice). Missing consent → the approval request cannot be
  submitted.
- **Statement line:** a line "rent written off" (negative to the owner's collected amount) on the
  statement of the period in which the write-off was approved; a recovery adds a line "recovered
  after write-off" in the period of the recovery.
- **Statement already published for that period:** the line goes on the next statement (published
  statements are never edited, `PRP.ACL.21`).
- **Owner-invoice write-off** (an owner's negative balance invoiced and unpaid): no consent needed
  (the owner is the debtor); not shown as a rent write-off.

#### Must not
- Must not let the requester approve the write-off.
- Must not edit a published owner statement.

#### Done when
- [ ] Given a managed building and an overdue rent invoice of AED 6,000, a write-off request
      without owner's consent is refused; with consent attached it goes to a property manager
      other than the requester.
- [ ] On approval, the next draft owner statement shows "rent written off −6,000.00".
- [ ] A later recovery of AED 2,000 shows "recovered after write-off +2,000.00" on the statement
      of that period.
- [ ] A write-off of the management company's own fee invoice needs no owner consent.

#### Out of scope
Write-off and recovery mechanics (BL-INV-01, BL-INV-03); bad-debt relief (BL-INV-02).

---

### BL-PRP-19 — 01 Property: approve, return, publish and revise an owner statement

**Labels:** `vertical:01-property` `area:negative-flow` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 01 Owner statement generation (`PRP.RUL.05`) · BL-KRN-01 · **Blocks:** BL-PRP-18, BL-PRP-20, BL-PRP-21, BL-PRP-22, BL-PRP-31, BL-PRP-34
**Review items:** 01 MEDIUM "owner statement lifecycle and disputes" (review §6)

#### Goal
Owner statements get a lifecycle: the accountant prepares a draft, a property manager approves it
(not the preparer) or returns it, the accountant publishes it to the owner as a PDF, and a
published statement with no payout can be revised into a new draft revision, the old one becoming
`superseded`.

#### Surfaces
- **Back office:** the accountant prepares the draft owner statement; a property manager who
  is not the preparer approves it or returns it with a reason; the accountant publishes it
  and may revise a published one with no payout.
- **System:** `publish` generates the PDF and e-mails it to the owner; `revise` creates the
  new draft revision and supersedes the old one.

#### Files to read
- `docs/verticals/01-property.md` §4.4 steps 1–3, §3.3 `PRP.ACL.08`, `PRP.ACL.21`, §6 *Owner
  statement*, §7.1 *Owner statement*, §7.2 `PRP.RUL.05`

#### Terms
- **Preparer:** the user who last edited the draft.
- **Revision:** a new statement record for the same owner, agreement and period, numbered 2, 3…
- **Payout:** the money paid to the owner for a positive balance.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Generated (`PRP.RUL.05`); being checked |
| `approved` | info | no | Approved by a property manager (`PRP.ACL.08`) |
| `published` | info | no | Sent to the owner |
| `settled` | success | yes | Payout paid, nothing due, or the negative balance carried forward or invoiced |
| `superseded` | neutral | yes | Replaced by a revision before payout |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `approve` | draft | approved | `owner_statements.approve` | approver ≠ preparer (`PRP.ACL.08`) | — |
| `return` | approved | draft | `owner_statements.approve` | reason | preparer notified |
| `publish` | approved | published | `owner_statements.prepare` | — | PDF generated and sent to the owner |
| `revise` | published | superseded | `owner_statements.prepare` | no payout recorded; reason | new `draft` revision created; owner told a corrected statement follows |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PRP.ACL.08` | Owner payouts are prepared by one user and approved by another | `SOD` | accountant, property manager | on |

#### Edge cases decided
- **Preparer = approver check:** the user who generated or last edited the draft cannot approve.
  A statement generated by `PRP.RUL.05` (system) and not edited can be approved by any property
  manager.
- **Publishing:** by e-mail in the MVP (owner portal is Later, BL-PRP-29).
- **Revision content:** copies the superseded statement's lines; the accountant corrects them.
  Balance brought forward is the same as the original.
- **One open revision:** a statement can be revised once; the new revision is the one that can
  be revised again after publishing.
- **Edits after approve:** refused; `return` first.
- **Next month's statement:** brings forward the balance of the latest non-superseded statement.

#### Must not
- Must not edit a published statement.
- Must not let the preparer approve.
- Must not revise after a payout was recorded.

#### Done when
- [ ] Given a draft statement edited by accountant A, A cannot approve; property manager B
      approves it.
- [ ] B returns an approved statement with reason "missing AMC bill"; it is `draft` and A is
      notified.
- [ ] Publishing sends the PDF to the owner's e-mail.
- [ ] Revising a published statement with no payout makes it `superseded`, creates revision 2 in
      `draft` and tells the owner a corrected statement follows.
- [ ] Revising after a payout is refused.

#### Out of scope
Payout, carry forward and owner invoice (BL-PRP-20); disputes (BL-PRP-21).

---

### BL-PRP-20 — 01 Property: pay out only a positive, undisputed balance; carry forward or invoice a negative one (`PRP.ACL.21`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PRP-19 · BL-PRP-21 · BL-PTY-04 · Base: wl-payments Payment lifecycle · **Blocks:** None.
**Review items:** 01-N3-g

#### Goal
A published statement is settled one of three ways. A positive balance is paid out, but only when
the statement is not disputed, the owner is not blocked and a second user approved the payout. A
balance of zero or less is carried forward to the next statement. When the agreement allows it, or
the balance has stayed negative for 3 statements, the property manager may invoice the negative
balance to the owner from the management company's entity.

#### Surfaces
- **Back office:** on a published statement the accountant runs `pay_out`, `carry_forward`
  or `invoice_owner`; the payout or owner invoice is approved by another user under "Needs
  your action"; disputed or blocked cases are refused with their reason.
- **System:** final approval checks the guards again; the payout is recorded, or the
  negative balance is brought forward or invoiced from the management company's entity.

#### Files to read
- `docs/verticals/01-property.md` §3.3 `PRP.ACL.08`, `PRP.ACL.21`, `PTY.ACL.03`, §4.4 step 3 and
  rows "The owner's balance is negative" and "The owner is blocked", §7.1 *Owner statement*
- `docs/OPEN-ITEMS.md` A-08

#### Terms
- **Balance:** brought forward + collected − expenses − fee ± adjustments.
- **Management company's entity:** the legal entity that issues management fees (`PLT.ENT.01`).
- **Negative for 3 statements:** the latest 3 non-superseded statements of the agreement each
  ended with a negative balance (including this one).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `pay_out` | published | settled | `owner_statements.prepare` | balance > 0; not disputed; owner not blocked (`PRP.ACL.21`); payout approved (`PRP.ACL.08`; guards checked again at final approval) | payout of the full positive balance recorded (`FIN.PAY.01`); no partial payouts |
| `carry_forward` | published | settled | `owner_statements.prepare` | balance ≤ 0 | balance brought forward on the next statement |
| `invoice_owner` | published | settled | `owner_statements.approve` | balance < 0; the agreement's `may_invoice_negative_balance` is set, or the latest 3 non-superseded statements of the agreement, this one included, ended negative; approval by a property manager other than the requester | invoice to the owner for the negative balance from the management company's entity (`FIN.INV.01`) |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PRP.ACL.21` | An owner payout is refused while the owner's statement balance is negative, the statement is disputed, or the owner is blocked; a published statement is corrected only by a revision before payout, or by adjustment lines on the next statement | `STA` | accountant, property manager | — |

§4.4 row: "No payout (`PRP.ACL.21`). By default the negative balance is carried forward to the
next statement; when the agreement allows it (field `may_invoice_negative_balance`), or the latest
3 non-superseded statements of the agreement all ended negative, the property manager may invoice
it to the owner from the management company's entity (VAT on
recharged costs `⚠ open: accountant (A-08)`)." Row "The owner is blocked": "Payouts stop
(`PTY.ACL.03`); the owner's leases continue; statements are still prepared."

#### Edge cases decided
- **"Agreement allows":** the boolean field `may_invoice_negative_balance` on the agreement
  (default false, §6).
- **`invoice_owner` guard in full:** balance < 0; and (agreement allows, or negative for 3
  statements); requested by `owner_statements.approve` holder; approved by a property manager
  other than the requester (`invoice_owner` guard; same SoD as `PRP.ACL.08`).
- **Invoice VAT:** the line's VAT category is chosen by the accountant from the catalogue charge
  type *recharged costs*; no platform default while A-08 is open (`blocked:open-item`).
- **Payout approval pending and the owner gets blocked or the statement disputed:** final approval
  re-checks the guards and fails (BL-KRN-01).
- **Balance exactly 0:** `carry_forward` (settled, nothing brought forward).
- **Amount of payout:** the full positive balance; partial payouts are not supported (`pay_out`
  effects).
- **Blocked owner exception:** an approved `PTY.ACL.03` exception lets one payout run.

#### Must not
- Must not pay out a negative, disputed or blocked owner's balance.
- Must not invoice a negative balance without the agreement clause or 3 negative statements.

#### Done when
- [ ] Given a published statement with balance AED 42,000.00, not disputed, owner not blocked,
      prepared by accountant A, `pay_out` creates an approval request; A cannot approve; property
      manager B approves; a payout of AED 42,000.00 is recorded and the statement is `settled`.
- [ ] With the dispute flag set, `pay_out` is refused.
- [ ] With the owner blocked, `pay_out` is refused with the neutral text.
- [ ] A balance of AED −3,500.00 is carried forward: the next draft statement shows brought
      forward −3,500.00.
- [ ] With balances −1,000, −2,000 and −3,500 on 3 consecutive statements, `invoice_owner` issues
      an invoice of AED 3,500.00 from the management company's entity; with only 2 negative
      statements and the agreement flag off, it is refused.

#### Out of scope
Statement approval and revision (BL-PRP-19); disputes (BL-PRP-21).

---

### BL-PRP-21 — 01 Property: dispute an owner statement line and correct it by revision or adjustment lines

**Labels:** `vertical:01-property` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PRP-19 · BL-TKT-11 · **Blocks:** BL-PRP-20, BL-PRP-29
**Review items:** 01 MEDIUM "owner statement lifecycle and disputes" (review §6)

#### Goal
An owner may dispute a line of a published statement. The statement gets a dispute flag and a
ticket; the payout waits. The flag is cleared by a revision before payout, or with an outcome:
upheld, or an adjustment line on the next statement. A cost allocated to the wrong owner is
corrected by adjustment lines on the next statements of both owners.

#### Surfaces
- **Back office:** staff flag a disputed line on a published or settled statement on the
  owner's behalf and clear the flag by `revise` or with outcome *upheld* or *adjustment
  line*; `pay_out` is blocked while the flag is set.
- **System:** setting the flag opens an *owner statement dispute* ticket; an adjustment
  outcome creates the lines picked up by the next draft statements, for both owners when the
  cost was allocated to the wrong one.

#### Files to read
- `docs/verticals/01-property.md` §4.4 rows "The owner disputes a statement line" and "A cost was
  allocated to the wrong owner", §7.1 *Owner statement* ("Dispute flag"), §3.3 `PRP.ACL.21`

#### Terms
- **Dispute flag:** a marker on the statement (not a state) with the disputed line(s) and a
  linked ticket.
- **Adjustment line:** a line on a later statement that corrects an amount from an earlier one,
  with a reference to it.

#### Spec excerpt (authoritative for this issue)

"**Dispute flag** (not a state). An owner disputes a line of a `published` statement; the flag
opens a ticket and blocks `pay_out` until it is cleared by `revise` or with an outcome (upheld, or
an adjustment line on the next statement). The flag can also be set on a `settled` statement
after payout; it is then cleared only as upheld or by an adjustment line."

§4.4 rows: "The statement is flagged disputed and a ticket opened; the payout waits. The line is
corrected by a revision before payout, or by an adjustment line on the next statement after
payout; the owner is told the outcome." "Corrected on the next statements of both owners by
adjustment lines; a published statement is never edited."

#### Edge cases decided
- **Who records it in the MVP:** staff, on the owner's behalf (owner portal is Later,
  BL-PRP-29).
- **Disputed after payout:** the statement is `settled`; the flag can still be set (ticket opened) and is cleared only with *upheld* or *adjustment line*; no payout is
  affected.
- **Adjustment outcome:** creates a pending adjustment line that the next draft statement picks
  up; for a wrong-owner cost, two lines (one negative, one positive) on the two owners' next
  statements, linked.
- **Ticket category:** *owner statement dispute* (seeded, BL-TKT-11); closing the flag resolves
  the ticket.
- **Owner told:** the outcome message is sent by e-mail.

#### Must not
- Must not edit a published statement.
- Must not pay out while the flag is set.

#### Done when
- [ ] Given a published statement, flagging the line "AMC chiller Q1 AED 4,200" sets the flag,
      opens a ticket and blocks `pay_out`.
- [ ] Clearing with outcome *adjustment line* (−4,200) puts the line on the next draft statement
      and allows `pay_out`.
- [ ] Clearing by `revise` supersedes the statement and creates revision 2.
- [ ] A cost of AED 1,800 allocated to owner X instead of owner Y produces −1,800 on X's next
      statement and +1,800 on Y's, linked.

#### Out of scope
Owner portal disputes (BL-PRP-29); payout (BL-PRP-20).

---

### BL-PRP-22 — 01 Property: activate a management agreement, give notice and end it with a handover pack

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PRP-19 · BL-PAY-16 · Base: wl-payments Cheque lifecycle · **Blocks:** BL-PRP-23, BL-PRP-29
**Review items:** 01-N12-c

#### Goal
The management agreement with a landlord gets a lifecycle. When either side gives notice, the
agreement moves to `ending` and a handover pack is prepared: deposits held and cheques not yet
deposited are handed over against a receipt, open charges and arrears are listed, and the Ejari
details are given. On `end`, charges after the handover date are cancelled, cheques are returned
as *handed over*, deposits are transferred, tenants are told the new payee and a final owner
statement is approved.

#### Surfaces
- **Back office:** the property manager activates the management agreement, records
  `give_notice` with the handover date, completes the handover pack with a receipt per
  deposit and cheque, and runs `end`; a missing receipt or an unapproved final statement
  refuses `end`.
- **System:** on `end` later charges are cancelled, cheques returned as *handed over*,
  deposits transferred, tenants told the new payee and the leases marked "handed over on
  <date>".

#### Files to read
- `docs/verticals/01-property.md` §4.4 row "The management agreement ends, or a unit is sold
  mid-lease", §6 *Management agreement*, §7.1 *Management agreement*, *Cheque* (`return`),
  *Deposit* (`transfer`), §13 question 13
- `docs/OPEN-ITEMS.md` AU-24

#### Terms
- **Handover date:** the date from which the units are no longer managed by the client.
- **Handover pack:** a generated document listing deposits held, cheques not deposited, open
  charges and arrears per lease, and Ejari details; with a receipt section signed by the new
  holder.
- **New holder:** the owner, or the new manager, who takes over.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Being prepared |
| `active` | success | no | The company manages the owner's units |
| `ending` | warning | no | Notice given by either side; handover being prepared |
| `ended` | neutral | yes | Handed over; no more charges or statements, except the final statement |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `activate` | draft | active | `agreements.manage` | owner, units, fee and payout frequency set; signed agreement attached | units' issuing entity and owner set from the start date |
| `give_notice` | active | ending | `agreements.manage` | notice date, party and handover date recorded | property manager and accountant notified; handover pack started |
| `end` | ending | ended | `agreements.manage` | handover pack complete: deposits held and cheques not deposited handed over against a receipt, open charges and arrears listed, Ejari details given; no deposit in `settlement_pending`; final owner statement (prepared by hand from the agreement, up to the handover date) approved | scheduled charges after the handover date cancelled; issued invoices stay with the owner, not credited, and their reminders stop; future cheques `return` with reason *handed over*; deposits handed over (deposit `transfer`); tenants told the new payee; leases stay with the new manager or owner and are marked "handed over on <date>", with no further automation; owner portal access read-only for 90 days |

Whether a change of manager or owner needs a new Ejari registration is `⚠ open: authority
(AU-24)`.

#### Edge cases decided
- **Final owner statement:** the statement for the period up to the handover date, generated by
  hand from the agreement (`end` guard). `end` waits only for `approved` (BL-PRP-19), not for
  publishing or the payout: after `end` the final statement is published, disputed, paid out or
  carried as a negative balance like any statement (`ended` keeps "except the final
  statement"); the agreement stays `ended` throughout.
- **Arrears:** invoices already issued stay with the issuing entity (the owner) and are listed in
  the pack; the client stops chasing them after `end` (reminders stop for invoices of
  handed-over leases; they are not credited; `end` effects).
- **Leases:** stay `active` in the records as read-only history for this client; they get the
  marker "handed over on <date>" (§6) and no further automation runs on them.
- **Deposits in `settlement_pending`:** cannot be transferred (BL-PAY-16); `end` is refused until
  they are refunded or forfeited.
- **Pack completeness:** each deposit and cheque line has a receipt reference; missing ones block
  `end`.
- **Ejari:** the pack records the Ejari numbers; no Ejari action is automated (AU-24,
  `blocked:open-item`).
- **Owner portal read-only:** Later (BL-PRP-29).

#### Must not
- Must not cancel or credit issued invoices on `end`.
- Must not end without receipts for every deposit and undeposited cheque.

#### Done when
- [ ] Given an `active` agreement for owner O with 3 units, `give_notice` (party owner, handover
      date 2027-06-30) makes it `ending`, notifies the property manager and accountant and starts
      the pack.
- [ ] The pack lists 3 deposits held (AED 15,000 total), 9 cheques `received`, open invoices and
      Ejari numbers.
- [ ] `end` without a receipt for one cheque is refused.
- [ ] `end` with all receipts and an approved final statement: charges after 2027-06-30 are
      cancelled, the 9 cheques are `returned` with reason *handed over*, the 3 deposits are
      `transferred`, each tenant is told the new payee, and the agreement is `ended`.
- [ ] With the final statement `approved` but not yet published, `end` succeeds; the statement
      is then published and paid out to O with the agreement already `ended`.
- [ ] With the final statement still `draft`, `end` is refused.
- [ ] `activate` without a signed agreement is refused.

#### Out of scope
One unit leaving the agreement (BL-PRP-23); deposit transfer mechanics (BL-PAY-16); owner portal
(BL-PRP-29).

---

### BL-PRP-23 — 01 Property: transfer a unit out of a management agreement when it is sold or no longer managed (`transfer_unit`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PRP-22 · BL-PAY-16 · **Blocks:** None.
**Review items:** 01-N12-c

#### Goal
A unit can leave an agreement mid-lease: sold to a new owner, or no longer managed. The property
manager approves the transfer; from the transfer date the unit's charges are issued by the new
landlord of record (when the client still manages it for the buyer) or stop being managed here,
with the same handover as `end` for that unit only.

#### Surfaces
- **Back office:** the property manager requests `transfer_unit` with the transfer date and
  the new owner or "no longer managed"; another property manager approves; a managed sale is
  refused with "new owner's agreement not active" until it is activated.
- **System:** on approval the unit's charges from the transfer date are recreated with the
  new issuer or the handover runs for that unit; a task suggests `give_notice` when the last
  unit leaves.

#### Files to read
- `docs/verticals/01-property.md` §4.4 row "The management agreement ends, or a unit is sold
  mid-lease", §7.1 *Management agreement* (`transfer_unit`, `end`), §6 *Management agreement*
  (units with dates in and out), §13 question 13
- `docs/OPEN-ITEMS.md` AU-24

#### Terms
- **Transfer date:** the date the sale completes or management stops for the unit.
- **Landlord of record:** the owner whose entity issues the unit's invoices.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `transfer_unit` | active, ending | active, ending (unchanged) | `agreements.manage` | unit, transfer date and new owner (unit sold) or "no longer managed" recorded; approval by a property manager other than the requester | as `end`, for that unit only; for a sale still managed by us, the unit moves to the new owner's agreement (which must be `active`: `transfer_unit` is refused otherwise; the property manager creates and activates it first through its own `activate`, signed agreement attached, listing the unit with date in = transfer date, and the unit's issuer changes only at `transfer_unit`; nothing is activated automatically) and scheduled charges from the transfer date are recreated with the new landlord of record as issuer (issued invoices stay with the old owner); the deposit stays `held`, re-assigned to the new owner against the new owner's receipt, and cheques stay linked to the recreated charges |

#### Edge cases decided
- **Sale still managed by us:** the new owner needs an `active` agreement. Nothing is activated
  automatically: a user with `agreements.manage` creates the new owner's agreement listing the
  unit (date in = transfer date) and activates it through BL-PRP-22's `activate`, which needs
  the signed agreement; the unit's issuer still changes only at `transfer_unit`. If the new owner
  has no signed agreement yet, `transfer_unit` as a managed sale is refused ("new owner's
  agreement not active"); the property manager waits for it or records "no longer managed"
  instead. The unit's date out on the old agreement = transfer date − 1, date in on the new one =
  transfer date. Scheduled charges from the transfer date are recreated with the new owner's
  entity as issuer; issued invoices stay with the old owner.
- **Deposit and cheques on a sale still managed:** the deposit stays `held`
  (the client still holds it) and is re-assigned to the new owner's account; cheques stay linked
  to the recreated charges. A receipt from the new owner records the liability change.
- **No longer managed:** as `end` for that unit: charges after the date cancelled, cheques
  `return`ed (*handed over*), deposit `transfer`red, tenant told the new payee.
- **Approval:** a property manager other than the requester approves (`transfer_unit` guard).
- **Owner statements:** the old owner's statement includes the unit up to the transfer date; the
  new owner's from it.
- **Ejari:** recorded only (AU-24, `blocked:open-item`).
- **Last unit of the agreement:** the agreement stays `active` with no units; a task suggests
  `give_notice`.

#### Must not
- Must not change the issuer on invoices already issued.
- Must not move a unit without approval.

#### Done when
- [ ] Given unit 803 under owner O's agreement with a lease billing AED 8,000 monthly, a
      `transfer_unit` to new owner P (still managed) dated 2027-05-01, once approved, recreates
      charges from May with P's entity as issuer; April's invoice stays with O.
- [ ] O's May statement excludes unit 803; P's includes it from May.
- [ ] A "no longer managed" transfer returns the unit's undeposited cheques and transfers its
      deposit against a receipt.
- [ ] Without approval, nothing changes.
- [ ] With P's agreement still `draft` (no signed agreement), the managed-sale `transfer_unit` is
      refused with "new owner's agreement not active"; after P's agreement is activated, it is
      accepted.

#### Out of scope
Ending the whole agreement (BL-PRP-22).

---

### BL-PRP-24 — 01 Property: cancel a maintenance job after work started and guard compliance visits (`PRP.ACL.15`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:field-pwa` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-ticketing Ticket lifecycle · BL-TKT-02 · BL-TKT-04 · **Blocks:** BL-PRP-34
**Review items:** D-14, 01-N8-b

#### Goal
In 01 a job cancelled after work started (from `on_hold`) is not charged to the tenant unless the
coordinator marks it chargeable with a reason; work a vendor already did is billed by the vendor
and allocated to the owner. A planned visit of a compliance-critical contract (fire and life
safety, lifts) can be resolved only with the vendor's service report attached, and cannot be
skipped by a user: only rescheduled with a reason. The only cancel is the system's when the
contract ends (`WRK.RUL.18`), for visits due after the end date; that is not a skip, because
the visit is no longer owed under the ended contract and the building shows `no_contract`.

#### Surfaces
- **Back office:** the maintenance coordinator cancels a job from `on_hold` with a reason,
  optionally ticking "charge tenant for work done" with charge lines; a compliance-critical
  visit can only be rescheduled with a reason, and the `WRK.RUL.18` task offers only
  "reassign" for open visits due before the contract end.
- **Field PWA:** the technician cannot resolve a compliance-critical visit until the
  vendor's service report is attached, or a chargeable job until it has a charge line.
- **System:** when the contract ends, `WRK.RUL.18` cancels only the visits due after the end
  date and the building shows `no_contract`.

#### Files to read
- `docs/verticals/01-property.md` §4.2 rows "The job is cancelled after work started" and "The
  tenant says the job is not done", §3.3 `PRP.ACL.15`, §7.1 *Ticket* (`resolve`, `cancel`)

#### Terms
- **Chargeable job:** a ticket the tenant pays for; invoiced on `confirm` or `close` in 01.
- **Compliance-critical planned visit:** a ticket generated from a fire and life safety or lifts
  service contract.
- **Service report:** the vendor's report document attached to the visit.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `resolve` | in_progress | resolved | `tickets.work` | after-photo attached when the category requires it; fire and life safety: the vendor's Civil Defence approval valid (`WRK.ACL.11`); compliance-critical planned visit: service report attached (`PRP.ACL.15`); a chargeable job has at least one charge line | requester asked to confirm; a chargeable job shows the tenant the amount (the charge lines' total including VAT), with no invoice yet |
| `cancel` | open, assigned, on_hold | cancelled | `tickets.assign`, or system (`WRK.RUL.18`) | reason given; a compliance-critical planned visit only by the system (`PRP.ACL.15`: it is rescheduled, not skipped) | requester and assignee notified; vendor work already done is billed by the vendor to the owner; charged to the tenant only when the coordinator marks it chargeable with a reason and charge lines, then invoiced as for `confirm` |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `PRP.ACL.15` | A planned visit of a compliance-critical contract (fire and life safety, lifts) can be completed only with the vendor's service report attached; it cannot be skipped by a user, only rescheduled with a reason. The one cancel allowed is the system's when the contract itself ends (`WRK.RUL.18`): it cancels only visits due after the end date, which are no longer owed under that contract, and the building gets the compliance flag `no_contract` until a replacement is active. Open visits due before the end date stay: the `WRK.RUL.18` task may only reassign them, not cancel them | `STA` | maintenance coordinator, vendor | — |

§4.2 row: "Ticket `cancel` from `on_hold` with a reason; work already done by a vendor is billed by
the vendor and allocated to the owner; nothing is charged to the tenant unless the coordinator
marks it chargeable with a reason and charge lines, in which case the invoice is drafted and
issued at once (there is no confirmation step for a cancelled job)."

#### Edge cases decided
- **Chargeable on cancel:** the coordinator may tick "charge tenant for work done" with a reason
  and charge lines; an invoice is drafted through `Invoicer` with those lines and issued at once
  (there is no confirmation step for a cancelled job). Without the tick nothing is invoiced.
- **Vendor bill:** the vendor's bill references the cancelled ticket and is allocated to the
  unit's owner like any vendor job.
- **Skip refused:** a user `cancel` of a compliance-critical planned visit is refused; only the
  system cancel by `WRK.RUL.18` (contract ended) is allowed, and only for visits due after the
  end date. Open compliance-critical visits due before the end date stay open: the
  coordinator's `WRK.RUL.18` task offers only "reassign" for them, not "cancel". Rescheduling changes the due date with
  a reason kept in history.
- **Report uploaded by the coordinator:** allowed on the vendor's behalf (§4.5 step 4).

#### Must not
- Must not invoice the tenant for a cancelled job without the coordinator's reason.
- Must not resolve a compliance-critical visit without a service report.

#### Done when
- [ ] Given a vendor job `on_hold` (parts), `cancel` with reason "tenant fixed it" notifies the
      tenant and vendor and creates no tenant invoice; the vendor's bill of AED 350 is allocated
      to the owner.
- [ ] With "charge tenant" ticked, reason "tenant damage" and one charge line of AED 200, an
      invoice of AED 200 is issued to the tenant.
- [ ] A chargeable ticket with no charge line cannot be resolved.
- [ ] A lifts planned visit without a service report cannot be resolved; with it, it can.
- [ ] A user cancel of a fire and life safety planned visit is refused; rescheduling it to
      2027-04-10 with a reason is allowed.
- [ ] Given that contract ending 2027-06-30 with visits due 2027-06-20 (open) and 2027-07-20, on
      expiry the 2027-07-20 visit is cancelled by the system, the 2027-06-20 visit stays open and
      the coordinator's task offers only "reassign" for it; the building shows `no_contract`.

#### Out of scope
Invoice on confirm or close (BL-TKT-02; 01 value in BL-PRP-26); Civil Defence approval guard
(BL-TKT-06); contract end (BL-TKT-04).

---

### BL-PRP-25 — 01 Property: show building compliance flags and expiring certificates without blocking leasing

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-TKT-03 · BL-TKT-04 · BL-TKT-07 · BL-KRN-08 · Base: wl-documents Document expiry (`DOC.RUL.01`) · **Blocks:** None.
**Review items:** 01-N8-b, D-13

#### Goal
The property manager and company admin must see which buildings are non-compliant: compliance
flags (`visit_missed`, `no_contract`, `vendor_document_expired`) and expired compliance
certificates (building safety certificate, lift certificate). On certificate expiry the building is
flagged non-compliant on the dashboard and the company admin is told; leasing is not blocked.

#### Surfaces
- **Back office:** the property manager, maintenance coordinator and company admin see the
  "Buildings non-compliant" dashboard tile with count and a list (building, cause, since).
- **System:** `DOC.RUL.01` warns at 90, 30 and 7 days, raises `certificate_expired` the day
  after expiry and tells the company admin; no lease transition checks the flags.

#### Files to read
- `docs/verticals/01-property.md` §4.5 rows "A planned visit is missed", "The contract expires or
  is terminated without a replacement", "A compliance certificate … expires", §7.2 `DOC.RUL.01`,
  `WRK.RUL.14`, §10 (PPM compliance, certificates, buildings flagged non-compliant)

#### Terms
- **Compliance flag:** a marker on a building raised by wl-ticketing with a cause; cleared by the
  module.
- **Compliance certificate:** a document of kind *building safety certificate* or *lift
  certificate* on the building, with an expiry date.

#### Spec excerpt (authoritative for this issue)

§4.5 row: "`DOC.RUL.01` warns at 90, 30 and 7 days; on the day after the expiry date the building
gets the compliance flag `certificate_expired`, shows as non-compliant on the dashboard, and the
company admin is told (the kind is compliance-critical). The flag clears when a renewed
certificate is accepted. The platform does not block leasing because of it."

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `DOC.RUL.01` *(shared)* · compliance-critical for compliance certificates and the Civil Defence approval | N days before `document.expires_on` | — | notify the record's responsible staff and, for KYC documents, the party; for compliance certificates, the coordinator and property manager | N = 90, 30, 7 | on | MVP |

§10: "PPM compliance: planned visits done on time %, per building and category"; "Compliance
certificates expiring in 30/60/90 days; buildings flagged non-compliant".

#### Edge cases decided
- **Non-compliant building:** any open compliance flag or any expired compliance certificate.
- **Certificate expiry flag:** a compliance flag with cause
  `certificate_expired` raised on the building on the day after the expiry date (Asia/Dubai) and
  cleared when a renewed certificate is accepted.
- **Dashboard tile:** "Buildings non-compliant" with count and a list (building, cause, since);
  visible to property manager (own buildings), maintenance coordinator and company admin.
- **Leasing:** no lease transition checks compliance flags.

#### Must not
- Must not block leases, renewals or invoices because of a compliance flag.

#### Done when
- [ ] Given Tower A's building safety certificate expiring 2027-05-31, warnings go to the
      coordinator and property manager on 2027-03-02, 2027-05-01 and 2027-05-24.
- [ ] On 2027-06-01 Tower A shows on the "non-compliant" tile with cause `certificate_expired` and
      the company admin is notified; a new lease on Tower A is still activated.
- [ ] A `visit_missed` flag on Tower B shows on the tile and disappears when the flag clears.

#### Out of scope
Raising and clearing the ticketing flags (BL-TKT-03, BL-TKT-04, BL-TKT-07).

---

### BL-PRP-26 — 01 Property: configure shared negative-flow settings — money

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-INV-01 · BL-INV-04 · BL-INV-05 · BL-INV-09 · BL-INV-11 · BL-INV-24 · BL-PAY-09 · BL-PAY-10 · BL-PAY-14 · BL-PAY-15 · BL-PTY-03 · BL-PTY-04 · BL-KRN-03 · BL-TKT-02 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · **Blocks:** None.
**Review items:** X-01, X-02, X-03, X-04, X-05, X-18, X-19, D-14, 01-N3-a, 01 MEDIUM "duplicate vendor bills" (review §6)

#### Goal
01 seeds its values for the shared money mechanisms built in the module epics: write-off,
invoice disputes, e-invoicing, bounced cheques, voids and bank match reversal, deposit disputes,
duplicate vendor bills, chargeable tickets and party block effects. Nothing new is built here;
each value is configured in the 01 seed and checked by a test.
It also seeds the 01 values for the cash-flow forecast alert (`FIN.CFL.01`) and the
company budget with owner drawings (`FIN.BUD.01`), added on 2026-10-16: `FIN.RUL.23`,
`FIN.RUL.24` and `FIN.ACL.21`.

#### Surfaces
- **Back office:** the seeded money values show up in staff screens: write-off and payment
  void approvals, held duplicate bills, bounce tasks, the bank match reversal limited to a
  property manager, the cash-flow alert and the budget approval.
- **Portal:** the tenant sees "dispute" on an issued invoice and a dispute button on
  published deposit deductions for 14 days.
- **System:** the 01 seed loads every value as configuration; the rules (`FIN.RUL.15`,
  `FIN.RUL.18`, `FIN.RUL.23`, `FIN.RUL.24`, e-invoicing transmission) run with these
  defaults.

#### Files to read
- `docs/verticals/01-property.md` §3.3, §4.1, §4.2, §4.4 (tables "When it goes wrong"), §7.1
  (*Invoice*, *Cheque*, *Deposit*, *Ticket* notes), §7.2 (rules marked *shared*, "Negative-flow
  settings" table), §9 (bounced cheques, e-invoicing)
- The module settings sections (§7) of `docs/modules/wl-invoicing.md`, `wl-payments.md`,
  `wl-parties.md`, `wl-ticketing.md`, `wl-kernel.md`
- `docs/modules/wl-invoicing.md` §5.14–§5.16 and §7 (cash-flow forecast, company budget, owner
  drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments)
- `docs/OPEN-ITEMS.md` L-22, A-05

#### Terms
- **Seed:** the vertical's default configuration loaded for every new 01 deployment; a client may
  change values later where the module allows it.
- **Owner role:** the business owner role alerted by `PLT.RUL.04` (set in BL-PRP-27).

#### Spec excerpt (authoritative for this issue)

| # | Setting | 01 value | Spec reference |
|---|---|---|---|
| **Invoices** | | | |
| 1 | Write-off limit and approver (`FIN.ACL.15`) | AED 0 (always approval); approver a property manager who did not request it; owner's consent recorded for a managed building (BL-PRP-18) | §3.3 `FIN.ACL.15`; §7.1 *Invoice* `write_off` |
| 2 | Invoice dispute by the party in the portal | on (staff can also dispute) | §7.1 *Invoice* "Dispute flag" |
| 3 | `FIN.RUL.15` dispute escalation | N = 14 days; remind the invoice owner, then escalate to the property manager | §7.2 `FIN.RUL.15` |
| 4 | Automations paused by an invoice dispute | `FIN.RUL.01` reminders and `PRP.RUL.10` | §7.1 *Invoice* "Dispute flag" |
| 5 | Bad-debt relief (`FIN.RUL.14`) | on; for commercial-rent invoices (residential rent is exempt) | §7.2 `FIN.RUL.14`; §9 "Bad debts" |
| 6 | E-invoicing | commercial-lease invoices transmitted through the ASP from the client's go-live date (1 January 2027 for revenue ≥ AED 50 million, 1 July 2027 for the others); only recipients in scope, so exempt residential invoices to individuals (B2C) are not transmitted; while the ASP is down, invoices queue | §7.2 settings table row Invoice; §9 "E-invoicing" |
| 7 | Chargeable ticket invoiced on | *close* (`confirm` or `close`), not `resolve`; drafted through `Invoicer` with the ticket's charge lines; `resolve` needs at least one charge line | §7.1 *Ticket* (`resolve`, `confirm`); §4.2 steps 2, 5–6 |
| 7a | Invoice drafts issued automatically (wl-invoicing) | on for the ticket source; off (reviewed) for the others | §4.2 step 6; §7.2 settings table row Invoice |
| **Cheques** | | | |
| 8 | Bounce fee | charge type *returned cheque fee*; amount set per client from its tenancy contract; none until set (`⚠ open: lawyer (L-22)` whether it may be charged without a contract clause; VAT `⚠ open: accountant (A-05)`) | §7.2 `FIN.RUL.18`; §9 "Bounced cheques" |
| 9 | Maximum presentations of one cheque | 2 | §7.2 `FIN.RUL.18`; §7.1 *Cheque* `represent` |
| 10 | `FIN.RUL.18` chase | N = 3 days: remind the tenant (open amount and bounce fee) and the property manager; M = 14 days: task to the accountant and property manager to re-present, refer to legal or write off | §7.2 `FIN.RUL.18` |
| 11 | `FIN.RUL.04` on bounce | notify the tenant and the property manager; follow-up task to the accountant | §7.2 `FIN.RUL.04` |
| 12 | Default legal follow-up for a bounced cheque | civil case; `refer_to_legal` sets the lease's legal case flag (BL-PRP-13) | §7.1 *Cheque* `refer_to_legal`; §9 |
| 13 | Cheque void | allowed while `received` (entered in error) | §7.2 settings table row Cheque |
| **Payments and bank matching** | | | |
| 14 | Payment void | with a reason, approved by another user (`PLT.ACL.09`); allocations reversed | §4.1 row "A payment was entered twice…" |
| 15 | Reversing a confirmed bank match | only a property manager (`PRP.ACL.16`); allocations removed and invoices reopened (`unallocate`), reminders resume | §3.3 `PRP.ACL.16`; §4.1 row "A bank match was wrong…" |
| 16 | Late entry of cheque deposits | allowed; business time = the deposit date | §7.2 settings table row Cheque |
| **Deposits** | | | |
| 17 | Deductions dispute by the tenant in the portal | on; dispute window 14 days; response window 7 days | §4.3 step 4; §7.1 *Deposit* |
| 18 | Deduction approval (`PRP.ACL.05`) | property manager; above AED 5,000 finance or the owner also approves | §3.3 `PRP.ACL.05` |
| 19 | Refund SoD (`FIN.ACL.04`) | on | §3.3 `FIN.ACL.04` |
| **Supplier bills** | | | |
| 20 | Duplicate check keys (`PRP.ACL.22`) | same vendor and vendor invoice number as a bill not `void`, or the same contract, period and amount; held bill approved only by another user with a reason | §3.3 `PRP.ACL.22`; §4.2 and §4.5 rows "The vendor bills…" |
| 20a | Vendor contract bill after the contract end (BL-INV-24) | each line needs a service date; a line after the contract's end or termination date sends the bill to `exception` with reason *work after contract end*, approved by another user with a reason | §4.5 row "The vendor bills twice, or for work after the end date" |
| **Party blocked** | | | |
| 21 | Block effect on leases | `continue`; decider property manager | §7.1 *Lease* "Party blocked"; §3.3 `PTY.ACL.03` |
| 22 | Block effect on service contracts (vendor) | `review`; decider property manager | §7.1 *Service contract* note |
| 23 | Money out stopped for a blocked party | deposit refund, owner payout, vendor payment; exception through `PTY.ACL.03` | §3.3 `PTY.ACL.03`; §4.4 row "The owner is blocked" |
| 24 | Money in from a blocked party | continues | §7.2 settings table rows Invoice, Payment |
| **Cash-flow forecast and budget** | | | |
| 25 | Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 50,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the company admin and the accountant, checked daily at 06:00; post-dated rent cheques by cheque date; owner payouts (`PRP.OWN.01`) counted as payments (a `CashForecastSource` of 01); due dates adjusted by each tenant's average days late: on; disputed amounts counted: off | §5; §7.2 settings |
| 26 | Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories owned by the accountant and mapped to the export accounts; one budget per legal entity and fiscal year, by month, approved by the company admin; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.2 settings |
| 27 | Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of operating cost and profit; every drawing approved, whatever its amount (`FIN.ACL.21` limit AED 0), by the company admin, never by the requester (`FIN.ACL.21`); payouts to landlords (`PRP.OWN.01`) are not drawings | §5; §7.2 settings |

#### Edge cases decided
- **Bounce fee unset:** no fee charge is created and the `FIN.RUL.18` reminder shows only the open
  amount (spec: "none until set"). Item 8 is `blocked:open-item` (L-22, A-05); the rest can be
  built.
- **E-invoicing go-live:** a per-client date setting, not computed by the platform; residential
  (exempt) invoices to individuals are B2C and not transmitted (transmission follows the
  module's recipient-in-scope test, §7.2 settings table row Invoice).
- **Client changes:** values 1, 9, 17, 18 and 20 are changeable per client within the module's
  limits; the seed only sets the defaults.

#### Must not
- Must not hard-code any of these values in 01 code; they are configuration.
- Must not set a bounce fee amount in the seed.

#### Done when
- [ ] **Invoices:** in a fresh 01 deployment, a write-off of AED 1 creates an approval request to a
      property manager; the tenant portal shows "dispute" on an issued invoice; a dispute open 14
      days reminds the invoice owner; a chargeable ticket creates no invoice on `resolve` and, on
      `confirm`, an invoice with its charge lines that is issued without review.
- [ ] **Invoices (items 4–6):** with an issued invoice disputed, no `FIN.RUL.01` reminder and no
      `PRP.RUL.10` task are created for it (4); a written-off commercial-rent invoice that meets
      the relief conditions gets a `FIN.RUL.14` VAT adjustment proposal and the accountant is
      notified, a written-off residential-rent invoice gets none (5); with e-invoicing go-live
      2027-07-01, a commercial-lease invoice to a company issued 2027-07-02 is transmitted to the
      ASP, one issued 2027-06-30 is not, a residential invoice to an individual is never
      transmitted, and with the ASP down the commercial invoice is queued (6).
- [ ] **Cheques:** a bounced cheque creates no fee charge until the client sets one; a third
      presentation is refused; reminders go out at 3 days and the decision task at 14 days.
- [ ] **Cheques (items 11–13):** on `bounce`, the tenant and the property manager are notified and
      the accountant has a follow-up task (11); `refer_to_legal` offers *civil case* by default and
      sets a legal case flag on the lease (12); `void` of a `received` cheque with reason
      *entered in error* is allowed, `void` of a `deposited` cheque is refused (13).
- [ ] **Payments:** an accountant cannot reverse a confirmed bank match; a property manager can,
      and the invoices reopen.
- [ ] **Payments (items 14, 16):** a payment void requested by accountant A cannot be approved
      by A; when B approves, its allocations are reversed (14); a cheque deposit entered on
      2027-05-10 with deposit date 2027-05-03 is accepted with business time 2027-05-03 (16).
- [ ] **Deposits:** deductions published to the tenant show a dispute button for 14 days; a
      deduction total of AED 6,000 needs a second approver; a deposit refund recorded by user A
      cannot be approved by A (19).
- [ ] **Supplier bills:** a second bill for the same AMC contract, period and amount is held as a
      possible duplicate; a bill against an AMC contract ending 2027-09-30 with a line dated
      2027-10-05 is held with reason *work after contract end* (20a).
- [ ] **Party blocked:** blocking a tenant leaves its lease active and creates no review task for
      the lease; blocking a vendor puts its service contracts in review for the property manager.
- [ ] **Party blocked (items 23–24):** with tenant T blocked, T's deposit refund waits; with owner
      O blocked, O's payout waits; with vendor V blocked, a payment to V waits; each runs once the
      compliance role approves an exception (23). A rent payment from blocked T is recorded and
      allocated as usual (24).
- [ ] **Cash-flow forecast:** in a fresh 01 deployment, a projected balance of AED 49,999 in week
      5 alerts the company admin and the accountant once (`FIN.RUL.23`, 06:00 run); AED 50,000
      or more in every week alerts no one; the forecast shows 13 weekly buckets from Monday.
- [ ] **Budget and drawings:** a monthly budget for a legal entity prepared by the accountant
      needs approval by the company admin; an expense with category *owner drawings* is left out
      of operating cost and profit reports, and the user who records it cannot approve it
      (`FIN.ACL.21`); an expense line reaching 100% of its month's budget line warns the
      category's owner role and the accountant once (`FIN.RUL.24`).

#### Out of scope
The mechanisms themselves (module epics named in Depends on); compliance, documents and notices
(BL-PRP-27).

---

### BL-PRP-27 — 01 Property: configure shared negative-flow settings — compliance, documents and notices

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` `surface:back-office` `surface:field-pwa` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-DOC-01 · BL-DOC-04 · BL-DOC-05 · BL-TKT-01 · BL-TKT-03 · BL-TKT-04 · BL-TKT-05 · BL-TKT-06 · BL-TKT-07 · BL-TKT-08 · BL-TKT-11 · BL-NTF-05 · BL-KRN-04 · BL-KRN-08 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · Base: 01 Enquiries and viewings (`PRP.LEA.01`) · **Blocks:** None.
**Review items:** X-13, X-19, X-20, D-13, 01-N8-b, 01 MEDIUM "vendor no-show reassignment" (review §6)

#### Goal
01 seeds its values for the shared compliance mechanisms: the owner role and the
compliance-critical rules, the vendor's Civil Defence approval as a blocking document kind, the
compliance-critical ticket categories and contract-end and no-show rules, the statutory notice
types, and late entry of field updates.
It also seeds the 01 values for lead sources and campaigns (`SAL.CRM.02`), onboarding and SOPs
(`HRM.ONB.01`) and registration kinds (`DOC.EXP.01`), added on 2026-10-16: `SAL.ACL.07`,
`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`. Lead sources are
recorded on the enquiry (`PRP.LEA.01`, MVP since 2026-10-16).

#### Surfaces
- **Back office:** the coordinator sees the list of a vendor's visits after the approval
  expiry; staff get the seeded tasks (statutory-notice delivery, onboarding steps, SOP
  acknowledgements), the lead sources on the enquiry and the campaign figures.
- **Field PWA:** the technician's resolve of a fire and life safety ticket is refused with
  no override when the vendor's Civil Defence approval has expired; a ticket update entered
  late keeps the work time with a late-entry flag.
- **System:** the 01 seed loads the owner role, compliance-critical rules, document kinds,
  categories, notice types and notification groups; `PLT.RUL.04` alerts the owner role when
  a critical rule fails.

#### Files to read
- `docs/verticals/01-property.md` §4.5 (table "When it goes wrong"), §7.2 (rules marked shared and
  compliance-critical; paragraphs "Compliance-critical rules and owner alerts", "Statutory
  notices", "Document guard hooks", "Negative-flow settings"), §9 "Vendor approvals"
- `docs/modules/wl-documents.md` §5.6, §7; `docs/modules/wl-ticketing.md` §7;
  `docs/modules/wl-notify.md` §5.4; `docs/modules/wl-kernel.md` §5.13, §5.15
- `docs/modules/wl-sales.md` §4.6, §5.8 and §7 (campaigns, sources); `docs/modules/wl-hr.md` §5.8 and
  §7 (onboarding); `docs/modules/wl-documents.md` §3, §5.7 and §7 (SOPs, registration kinds)
- `docs/OPEN-ITEMS.md` L-19, AU-08

#### Terms
- **Compliance-critical rule:** an automation that protects a legal duty; if it fails or misses
  its schedule, `PLT.RUL.04` alerts the owner role and the company admin at once (the same role
  in 01 by default; there is no separate "admin" role).
- **Legal block:** a document kind whose `blocks` can never be overridden.
- **`BlockSubjects` / `FutureWork`:** providers 01 registers so wl-documents can map a ticket or
  contract to its vendor, and list the vendor's future visits.

#### Spec excerpt (authoritative for this issue)

| # | Setting | 01 value | Spec reference |
|---|---|---|---|
| **Owner alerts** | | | |
| 1 | Owner role (`PLT.RUL.04`) | company admin by default (the client may name its managing director); the company admin is also alerted (with the default this is the same role, alerted once) | §7.2 "Compliance-critical rules and owner alerts" |
| 2 | Compliance-critical rules | `PRP.RUL.09`, `WRK.RUL.14`, `WRK.RUL.18`, `DOC.RUL.01` (for compliance certificates and the Civil Defence approval), `DOC.RUL.02`, `DOC.RUL.03` | §7.2 rules table |
| **Documents** | | | |
| 3 | Document kind *Civil Defence approval* (subject: vendor party) | `blocks`: `service_contract.activate` (condition: the contract covers a fire and life safety category) and `ticket.resolve` (condition: category is fire and life safety); mode `block`; override `never` (legal block); compliance-critical; look-ahead 30 days | §7.2 "Document guard hooks"; §4.5 row "The vendor's Civil Defence approval expires" |
| 4 | Providers registered by 01 | `BlockSubjects`: ticket or service contract → its vendor; `FutureWork`: the vendor's planned visits after a date | §7.2 "Document guard hooks" |
| 5 | Who is told on Civil Defence approval expiry (`DOC.RUL.03`) | coordinator, property manager, company admin; the contract gets the vendor document flag, open visits are flagged, buildings get `vendor_document_expired` | §4.5 row "The vendor's Civil Defence approval expires" |
| 6 | Compliance certificate kinds | *building safety certificate*, *lift certificate* (subject: building); `DOC.RUL.01` at 90, 30, 7 days to the coordinator and property manager; compliance-critical; no `blocks`; on expiry flag `certificate_expired` | §4.5 row "A compliance certificate … expires"; §7.2 `DOC.RUL.01` |
| 6a | Responsible role per document kind | maintenance coordinator: Civil Defence approval, building safety and lift certificates, service contract and service report; property manager: KYC kinds (Emirates ID, passport, residence visa, trade licence, TRN certificate), signed tenancy contract, Ejari / Tawtheeq certificate, tenant acceptance of changed terms, repossession basis, judgment, execution record, signed management agreement | §7.2 "Document kinds and responsible roles" |
| **Ticketing** | | | |
| 7 | Compliance-critical categories | fire and life safety, lifts | §7.2 `WRK.RUL.14`; wl-ticketing §7 |
| 8 | `WRK.RUL.12` contract expiring | N = 60 days; notify coordinator and property manager | §7.2 |
| 9 | `WRK.RUL.14` planned visit missed | N = 3 days; for fire and life safety and lifts also notify the property manager and flag `visit_missed` | §7.2; §4.5 row "A planned visit is missed" |
| 10 | `WRK.RUL.18` contract ended | task due 1 working day; flag `no_contract` for fire and life safety and lifts; notify the property manager and the company admin | §7.2; §4.5 row "The contract expires…" |
| 11 | `WRK.RUL.19` vendor no-show | M = 2 hours; reassignment offered to the coordinator; for fire and life safety and lifts also notify the property manager; repeated no-shows shown on the contract page | §7.2; §4.2 row "The vendor does not turn up" |
| 12 | Reassignment reasons | `vendor_no_show`, `contract_ended`, `vendor_document_expired` | §7.1 *Ticket* `assign` |
| 13 | Service contract approval (`PRP.ACL.14`) | above AED 50,000 a year property manager approval; above AED 200,000 the owner's approval recorded; rejected or returned → back to `draft` with the reason | §3.3 `PRP.ACL.14`; §7.1 *Service contract* note |
| 14 | Ticket categories seeded by 01 for negative flows | *legal case*, *billing dispute*, *owner statement dispute* | §7.1 *Lease* "Legal case flag"; §4.1; §4.4 |
| **Notices** | | | |
| 15 | Statutory notice types | notice of changed terms; landlord's notice to end a lease; formal payment demand; formal notice on a bounced cheque. Reaction: `mark_contact_invalid` + `statutory_notice`; proof: courier receipt, registered mail receipt, signed hand-delivery note, or a call note | §7.2 "Statutory notices"; wl-notify §5.4 |
| 15a | Notification types and their group (`register_type`) | `lease.contract_for_signature`, `lease.renewal_window`, `lease.notice_withdrawn` → *Portal invitations, document requests, signature requests*; `lease.notice_of_changed_terms`, `lease.landlord_notice`, `lease.payment_demand` → *Statutory notices* (statutory); `lease.new_payee`, `owner_statement.published`, `owner_statement.revision_follows`, `owner_statement.dispute_outcome` → *Invoices, statements, payment reminders*; staff-only types: reaction `none`. Each type takes its group's give-up reaction from wl-notify §5.4: *Portal invitations…* → `mark_contact_invalid` + `staff_task` to the request's owner (the lease's property manager); *Statutory notices* → `mark_contact_invalid` + `statutory_notice`; *Invoices, statements, payment reminders* → `mark_contact_invalid` + `alternative_recipient`, then `staff_task` to the collector role | §7.2 "Notification types"; wl-notify §5.4 |
| **Late entry and integrations** | | | |
| 16 | Late field updates on tickets | keep the work time as business time (`PLT.ACL.10`: flagged above 30 minutes, manager approval above 24 hours) | §7.2 settings table row Ticket |
| 17 | Ejari / Tawtheeq | manual in the MVP; `PRP.RUL.02` (N = 14) and `PRP.RUL.13` (N = 7) chase the number and the cancellation; no integration connection to monitor | §7.2 settings table row Lease; §8 |
| **Sales, finance, staff and registrations** | | | |
| 18 | Lead sources used (`SAL.CRM.02`) | referral, walk-in, website, social, paid ads, partner (brokers), event, other; recorded on the enquiry (`PRP.LEA.01`) and carried to the tenant party when the enquiry converts; a tenant without an enquiry gets the source on its party; no source requires a campaign | §5; §7.2 settings |
| 19 | Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | listing portals, social media, search ads, email, signboards, brokers; converted value: the first lease's contract value; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.2 settings |
| 20 | Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for property manager, leasing agent, accountant, maintenance coordinator and technician, started by `HRM.RUL.16` when the employee record (`HRM.EMP.01`) is created in `onboarding` (none for a record created directly `active`, e.g. by a data import); checklist steps use `WRK.CHK.01`; tasks to the company admin, the line manager and the new staff member; the new staff member's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds the assignees of an overdue step on the first run after its due date, then every 2 days, and tells the company admin and the line manager after 5 days; completion blocks nothing (role activation not blocked) | §5; §7.2 settings |
| 21 | SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: property manager (leasing, move-in, move-out, maintenance) and accountant (cheques, deposits, owner statements); required readers by role; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP's owner role (the SOP owner named above) after 7 days overdue; acknowledged only by the reader (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.2 settings |
| 22 | Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Trademark registration* and *Other certification* (subject: legal entity); responsible role company admin; `DOC.RUL.01` at 180, 90 and 30 days for trademarks and 90, 30 and 7 days for the others; no `blocks` | §5; §7.2 settings |

#### Edge cases decided
- **Legal block:** whether the law forbids the work outright is `⚠ open: lawyer (L-19)`; the seed
  uses override `never` as the spec says (`blocked:open-item` for the legal basis only).
- **Other emirates:** fire and life safety rules outside Dubai are `⚠ open: authority (AU-08)`;
  the same kind and categories are seeded for every emirate.
- **Company admin told on `WRK.RUL.18`:** the rule row and §4.5 both name the property manager
  and the company admin; both are seeded.
- **Client changes:** a client may add `blocks` entries but not remove the Civil Defence legal
  block (wl-documents §7).

#### Must not
- Must not allow an override of the Civil Defence approval block.
- Must not block leasing on a compliance certificate.

#### Done when
- [ ] **Owner alerts:** when the `PRP.RUL.09` run fails, each company admin gets the owner alert
      once (owner role and company admin are the same role by default); with the managing
      director named as owner role, the managing director and the company admin are both
      alerted; a non-critical rule failure (e.g. `PRP.RUL.12`) does not alert the owner.
- [ ] **Documents:** with vendor V's Civil Defence approval expired on 2027-04-30, resolving V's
      fire and life safety ticket on 2027-05-02 is refused with no override option; activating a
      new fire and life safety contract for V is refused; a cleaning ticket of V resolves.
- [ ] On 2027-04-01 the coordinator has the list of V's visits after 2027-04-30.
- [ ] **Ticketing:** a lifts visit missed by 3 days notifies the property manager and flags the
      building; a vendor not started 2 hours after the scheduled time is recorded as a no-show.
- [ ] **Notices:** a formal payment demand whose e-mail gave up creates a 1-working-day task to the
      property manager to deliver it another way.
- [ ] **Responsible roles:** a Civil Defence approval expiring in 30 days notifies the vendor's
      buildings' maintenance coordinators and opens their renewal task on expiry; a tenant's
      Emirates ID expiring notifies the property manager and the tenant.
- [ ] **Notification types:** a `lease.renewal_window` e-mail that hard-bounces takes its
      group's reaction (`mark_contact_invalid` + `staff_task` to the request's owner): the
      contact is marked invalid and the lease's property manager gets a task; a
      `lease.payment_demand` give-up creates the statutory-notice task; a staff reminder that
      fails creates no task.
- [ ] **Late entry:** a ticket update entered 3 hours late keeps the work time and carries a
      late-entry flag with a reason.
- [ ] **Lead sources:** a new enquiry offers the sources referral, walk-in, website, social, paid
      ads, partner, event and other, and its source and campaign reach the tenant party when it
      converts; a campaign with channel *listing portals*, a budget and cost entries reports
      enquiries, conversion, cost per lead, CAC and ROAS; a cost entry over the budget waits for a
      manager (`SAL.ACL.07`).
- [ ] **Onboarding and SOPs:** hiring a leasing agent creates the leasing agent checklist with tasks
      to the company admin, the line manager and the employee, and the role is active before it is
      complete; a step still open on the first run after its due date reminds its assignees, and
      again 2 days later (`HRM.RUL.17`); a new version of an SOP owned by the property manager asks
      every required reader to acknowledge again within 7 days (`DOC.RUL.04`), and no one else can
      acknowledge for them (`HRM.ACL.10`).
- [ ] **Registrations:** a fresh 01 deployment lists *Trademark registration* and *Other
      certification* with responsible role company admin; a trademark registration expiring in 90
      days notifies the company admin.

#### Out of scope
The mechanisms themselves (module epics named in Depends on); money settings (BL-PRP-26); the
dashboard tile (BL-PRP-25).

---

### BL-PRP-28 — 01 Property: release a reserved unit and settle its booking deposit

**Labels:** `vertical:01-property` `area:negative-flow` `priority:medium` `blocked:open-item` `size:s` `surface:back-office` `surface:system` · **Milestone:** Later · **Size:** s
**Depends on:** Base: 01 Unit reservation (`PRP.LEA.02`, Later) · BL-PRP-01 · **Blocks:** None.
**Review items:** 01-N2-a

#### Goal
When reservations exist (Later), a reserved unit is released when the prospect backs out or the
lease is cancelled. The booking deposit is refunded, or forfeited only with the property manager's
approval where the signed reservation allows it.

#### Surfaces
- **Back office:** the leasing agent or accountant cancels the lease or releases the
  reserved unit, entering any forfeit with a clause reference; a property manager approves
  under "Needs your action".
- **System:** on approval the unit becomes `available` and the booking deposit refund (or
  the rest after the forfeit) is requested.

#### Files to read
- `docs/verticals/01-property.md` §7.1 *Unit* (`reserve`, `release`), §3.3 `PRP.ACL.18`, §5
  `PRP.LEA.02` (Later), §13 question 12
- `docs/OPEN-ITEMS.md` L-22

#### Terms
- **Booking deposit:** a payment recorded when the unit is reserved.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `release` | reserved | available | `units.reserve` or system (lease `cancel`) | — | booking deposit refunded or forfeited (`PRP.ACL.18`) |

`PRP.ACL.18`: "Cancelling a lease after money or cheques were received (a booking deposit on the
reserved unit counts), and forfeiting any of it,
is requested by the leasing agent or accountant and approved by a property manager." Whether a
deposit can be forfeited when a prospect backs out before signing is `⚠ open: lawyer (L-22)`.

#### Edge cases decided
- **Two transitions on main:** the spec's single `release` is built as `release` (a user with
  `units.reserve`) and `release_by_system` (the lease `cancel`), as `vacate` is split by target
  (`apps/property/src/unit_machine.rs`). `reserve` is declared and refuses until booking deposits exist
  (`PRP.LEA.02`, Later). This issue adds the booking deposit behaviour to both release
  transitions; it does not build `reserve`.
- **Booking deposit is money received:** a lease `cancel` for the prospect holding the
  reservation needs the `PRP.ACL.18` approval (BL-PRP-01), even for a full refund; the unit is
  released and the booking deposit refunded or part forfeited at final approval.
- **Manual release with a booking deposit:** the same approval as `PRP.ACL.18` (requester ≠
  approver) when any of the deposit is forfeited; a full refund needs only `FIN.ACL.04` on the
  refund.
- **Forfeit:** only with a clause reference from the signed reservation (as BL-PRP-01).

#### Must not
- Must not forfeit without approval and a clause reference.

#### Done when
- [ ] Given unit 504 `reserved` with a booking deposit of AED 3,000 and nothing else received, a
      lease `cancel` for that prospect by leasing agent A creates a `PRP.ACL.18` approval request;
      when property manager B approves, the unit is `available` and a refund of AED 3,000 is
      requested.
- [ ] A manual `release` with a full refund of AED 3,000 needs no `PRP.ACL.18` approval; the
      refund follows `FIN.ACL.04`.
- [ ] A manual `release` forfeiting AED 1,000 with clause "Reservation §2" creates an approval
      request; on approval the refund is AED 2,000.

#### Out of scope
Reservations themselves (`PRP.LEA.02`, Later).

---

### BL-PRP-29 — 01 Property: owner portal — dispute a statement line and keep read-only access after the agreement ends

**Labels:** `vertical:01-property` `area:negative-flow` `priority:medium` `size:s` `surface:portal` `surface:system` · **Milestone:** Later · **Size:** s
**Depends on:** BL-PRP-21 · BL-PRP-22 · Base: wl-portal owner portal (`POR.SLF.02`, Later) · **Blocks:** None.
**Review items:** 01 MEDIUM "owner statement lifecycle and disputes" (review §6), 01-N12-c

#### Goal
When the owner portal exists (Later), an owner disputes a statement line from the portal instead
of through staff, and after the management agreement ends the owner keeps read-only access for 90
days.

#### Surfaces
- **Portal:** the owner disputes a line of a published statement from the owner portal;
  after the agreement ends the owner can only read statements and documents for 90 days.
- **System:** the dispute sets the flag and opens the ticket as in BL-PRP-21; access closes
  at 23:59 of day 90 after `end`.

#### Files to read
- `docs/verticals/01-property.md` §7.1 *Management agreement* (`end`), *Owner statement*
  ("Dispute flag"), §3.3 `PRP.ACL.02`, §5 `POR.SLF.02`

#### Terms
- **Read-only access:** the owner sees statements and documents but cannot dispute or change
  contact details.

#### Spec excerpt (authoritative for this issue)

`end` effect (Management agreement): "owner portal access read-only for 90 days".
Owner statement dispute flag: "An owner disputes a line of a `published` statement; the flag opens
a ticket and blocks `pay_out` until it is cleared".
`PRP.ACL.02`: "An owner sees only units under their management agreements, and the statements for
them" (`OWN`).

#### Edge cases decided
- **Days:** 90 calendar days from the `end` date (Asia/Dubai); access closes at 23:59 of day 90
  (test boundary).
- **Dispute after `end`:** refused in the portal (read-only); staff may still record it.

#### Must not
- Must not let an owner dispute a statement of another owner.

#### Done when
- [ ] Given owner O in the portal, disputing the line "Lift AMC Q2" sets the dispute flag and
      opens the ticket (as BL-PRP-21).
- [ ] After the agreement ended on 2028-06-30, O can read statements until 2028-09-28 and cannot
      dispute; on 2028-09-29 the portal is closed for O.

#### Out of scope
The dispute flag itself (BL-PRP-21); the owner portal (`POR.SLF.02`).

---

### BL-PRP-30 — 01 Property: tenant portal: show the lease, its cheques and documents (`POR.SLF.01`, `PRP.ACL.01`)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:portal` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-POR-02; BL-POR-01; Base: 01 Lease lifecycle · **Blocks:** None.
**Review items:** X-18; follow-up of BL-POR-01, Done-when point 1

#### Goal
The tenant portal (`POR.SLF.01`, MVP in 01) shows the tenant their lease, the post-dated cheques they
handed over and the lease documents. apps/property registers three portal sections for the audience
`tenant`, filtered to the leases where the user's party is a tenant (`PRP.ACL.01`), and seeds the
audience's roles. Invoices, the deposit and requests come from their modules (BL-INV-31, BL-PAY-27,
BL-TKT-14).

#### Surfaces
- **Portal:** *Lease* shows each lease of the party (unit, building, term, annual rent, payment plan,
  status, Ejari number) and its versions; *Cheques* lists the cheques of those leases (number, bank,
  date, amount, status as wl-payments stored it); *Documents* lists the lease's documents the tenant may
  see (signed contract, Ejari certificate, notices, receipts) with downloads.
- **System:** the sections are registered at start-up; the 01 seed creates the `tenant` audience and
  its portal roles (BL-POR-02 edge cases).

#### Files to read
- `docs/verticals/01-property.md` §3.1 (tenant roles), §3.3 `PRP.ACL.01`, `PRP.ACL.11`, §5 `POR.SLF.01`
- `docs/modules/wl-portal.md` §2 (01 audiences), §6 (`PortalSections`, blocked parties)
- `apps/property/src/lease.rs`, `cheques.rs`, `documents.rs`

#### Terms
- **Lease version:** a renewal or amendment version of the lease on the unit.
- **Tenant-visible document:** a document of the lease whose kind is marked visible to the party (not
  internal notes, not KYC documents of other parties).

#### Spec excerpt (authoritative for this issue)
| Role | Actor type | What they do | Permissions (summary) |
|---|---|---|---|
| Tenant — primary contact | external | Sees the lease, invoices, cheques, documents; pays; raises requests; manages the company's other portal users | portal: lease, finance, requests, users |
| Tenant — finance contact | external | Sees and pays invoices only | portal: finance |

| ID | Rule | Pattern | Applies to |
|---|---|---|---|
| `PRP.ACL.01` | A tenant user sees only leases where their party is a tenant, and the invoices, cheques, documents and tickets of those leases | `OWN` | tenant |

| ID | Capability | Type | Module | Scope |
|---|---|---|---|---|
| `POR.SLF.01` | Tenant self-service portal: lease, invoices, cheques, documents, requests | shared | wl-portal | MVP |

#### Edge cases decided
- **Blocked tenant:** everything shows with the stored statuses and no block text (BL-POR-01).
- **Read-only grant after move-out:** the sections show (history and downloads are kept) (BL-POR-03).
- **Draft versions:** not shown; `pending_signature` is shown as *Waiting for signature*.
- **Legal case flag:** not shown in the portal (internal).
- **Cheque statuses:** the stored ones (*Received*, *Deposited*, *Cleared*, *Bounced*, *Returned*).

#### Must not
- Show another party's lease, cheque or document, or internal notes and the legal case flag.
- Show the finance contact the lease or cheques sections.

#### Done when
- [ ] Given tenant "Al Noor Trading LLC" with lease L-0042 on unit 1204 (AED 120,000, 4 cheques), Sara
      (primary contact) sees L-0042 with its term and rent, the 4 cheques with their statuses and the
      signed contract and Ejari certificate.
- [ ] She does not see lease L-0043 of another tenant on the same floor, even by URL.
- [ ] Omar (finance contact) has no Lease or Cheques entry.
- [ ] With the party blocked (`non_payment`), the same pages show with no block text.
- [ ] After the lease is terminated, she still sees and downloads them (read-only).

#### Out of scope
Invoices and statement of account (BL-INV-31), deposit and deductions (BL-PAY-27), requests
(BL-TKT-14), portal users management (base wl-portal), the owner portal (`POR.SLF.02`, Later).

---

### BL-PRP-31 — 01 Property: generate the tenancy contract and the owner statement PDF and send them

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-DOC-11; BL-NTF-10; BL-PRP-19 · **Blocks:** None.
**Review items:** 01 MEDIUM "owner statement lifecycle and disputes" (review §6); follow-up of BL-PRP-19, Done-when point 3

#### Goal
Two 01 transitions must produce a document: lease `submit` generates the contract PDF in English and
Arabic, and owner statement `publish` generates the statement PDF and sends it to the owner. Today
`publish` e-mails the statement as text and `submit` generates nothing. With document generation
(BL-DOC-11) and attachments (BL-NTF-10), apps/property registers the two templates and their data
providers and calls them.

#### Surfaces
- **Back office:** the lease shows the generated contract (*Tenancy contract L-0042 v1*) under its
  documents; the statement shows its PDF; both download.
- **System:** `submit` generates the contract; `publish` generates the statement PDF and sends
  `owner_statement.published` with the PDF attached; `lease.contract_for_signature` carries the contract.

#### Files to read
- `docs/verticals/01-property.md` §4.1 step 6, §4.4 step 2, §7.1 *Lease* (`submit`), §7.1 *Owner
  statement* (`publish`), §7.2 *Notification types*
- `docs/modules/wl-documents.md` §5.3
- `apps/property/src/notices.rs`, `statement_events.rs`, `lease_machine.rs`

#### Terms
- **Tenancy contract:** the bilingual contract of one lease version: parties, unit, term, rent, payment
  plan, deposit, clauses, signatory block.
- **Owner statement PDF:** collected, expenses, fee, adjustments and balance of one statement, with its
  lines.

#### Spec excerpt (authoritative for this issue)
| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` (Lease) | draft | pending_signature | `leases.create` | … | contract PDF generated in EN/AR (`DOC.GEN.01`) |
| `publish` (Owner statement) | approved | published | `owner_statements.prepare` | — | PDF generated and sent to the owner |

01 §4.4 step 2: "The owner statement is generated as a PDF and published to the owner by email, and in
the owner portal once it exists (`DOC.GEN.01`, `POR.SLF.02`)."

#### Edge cases decided
- **Renewal and amendment versions:** each version gets its own contract; the earlier one is kept.
- **A revision of a statement:** the superseded statement's PDF stays; the new revision gets its own PDF
  at its own `publish`.
- **Generation fails:** the transition is refused with the error (nothing published, nothing submitted),
  so no statement is published without its PDF.
- **Statement language:** bilingual; contract: bilingual (§4.1 step 6).
- **E-signature:** the contract is sent for signature by e-mail as an attachment; the signed copy is
  attached by staff (`activate` guard). Online e-signature (`DOC.SIG.01`) is later.

#### Must not
- Regenerate the contract of a submitted version or the PDF of a published statement.
- E-mail the owner outside wl-notify.

#### Done when
- [ ] Given lease L-0042 in draft, `submit` stores *Tenancy contract L-0042 v1* (source generated,
      bilingual) and the tenant gets it attached to `lease.contract_for_signature`.
- [ ] Given an approved statement for October 2026 with balance AED 18,450, `publish` stores its PDF and
      the owner's e-mail carries it; the message log lists the attachment.
- [ ] When the PDF cannot be generated, `publish` is refused and the statement stays `approved`.
- [ ] A revision published later has its own PDF; the first one is still there.

#### Out of scope
Generation and attachments (BL-DOC-11, BL-NTF-10); the cheque handover receipt (BL-PAY-30); the owner
portal (`POR.SLF.02`, Later); online e-signature (`DOC.SIG.01`).

---

### BL-PRP-32 — 01 Property: apply the termination credit to the deposit deductions first

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PRP-11; BL-PRP-17; BL-PAY-15 · **Blocks:** None.
**Review items:** 01-N12-a, 01-N3-e; follow-up of BL-PRP-11, Done-when point 1 (a)

#### Goal
When a lease ends early, invoices for periods after the move-out are credited pro rata; on a paid invoice
the credit goes to the tenant's credit balance. The spec applies that credit to the deposit settlement's
deductions first, and only the rest is refunded or kept. Today `terminate` asks the refund at once (or
keeps the credit), because the deductions do not exist yet at `terminate`. This issue holds the
termination credit for the settlement and uses it before the deposit when the deductions are settled.

#### Surfaces
- **Back office:** the lease and the deposit settlement show *Termination credit held for the settlement:
  AED 3,193.55*; the settlement statement lists the credit applied, the deposit applied, the deposit
  refunded and the credit refunded or kept.
- **System:** at deposit `refund` or `forfeit` the credit is allocated first, then the deposit; the rest
  of the credit follows the property manager's choice recorded at `terminate`.

#### Files to read
- `docs/verticals/01-property.md` §4.3 rows "Early termination by the tenant" and "Deductions exceed the
  deposit", §7.1 *Lease* (`terminate`), §7.1 *Deposit*
- `apps/property/src/termination.rs` (`settle_credit`), `deposit_forfeit.rs`

#### Terms
- **Termination credit:** the part of the pro rata credit notes of `terminate` that no balance of their
  invoices uses (the tenant's credit balance).
- **Deductions:** unpaid rent invoices plus the published damage lines (BL-PRP-17).

#### Spec excerpt (authoritative for this issue)
`terminate` effects: "… a credit on a paid invoice goes to the tenant's credit balance, applied to the
deposit settlement's deductions first, the rest refunded through the Refund flow or kept as credit
(property manager's choice) …"
§4.3 "Early termination by the tenant": "a partial credit note on a paid invoice adds its amount to the
tenant's credit balance, which is applied to the deposit settlement's deductions first and the rest
refunded through the Refund flow, or kept as credit when the property manager chooses".

#### Edge cases decided
- **Order:** the credit covers the deductions before the deposit: first the unpaid rent invoices, oldest
  first (credit note allocated to them), then the damage lines; the deposit covers what is left, and its
  remainder is refunded as usual. Decided where the spec is silent: the credit goes first because it is
  already the tenant's money on account, so more of the deposit is returned.
- **No deposit held, or a deposit already settled:** the credit follows the choice at once, as today.
- **Credit larger than the deductions:** the rest follows the choice (`refund`: Refund flow with the
  credit note as source, `FIN.ACL.04`; `keep`: stays on account).
- **Deductions disputed:** nothing is applied while the dispute flag is set (the settlement waits).
- **A damages invoice:** only what neither the credit nor the deposit covered is invoiced.

#### Must not
- Refund the termination credit before the deductions are settled when a deposit is held.
- Apply the credit twice (once to the deductions and again by refund).

#### Done when
- [ ] Given a termination credit of AED 3,193.55, deposit AED 10,000 and deductions of AED 2,000, at
      `refund` the credit covers the AED 2,000, AED 1,193.55 is refunded (choice `refund`), and the full
      deposit of AED 10,000 is refunded.
- [ ] With choice `keep`, the AED 1,193.55 stays on the tenant's account.
- [ ] With deductions of AED 14,000 (rent AED 4,500 + damages AED 9,500), the credit pays AED 3,193.55
      of the rent invoice, the deposit covers the rest of the rent and AED 8,693.55 of damages, and a
      damages invoice of AED 806.45 is raised.
- [ ] With no deposit held, the credit is refunded or kept at `terminate`, as today.
- [ ] While a deduction line is disputed, nothing is applied.

#### Out of scope
`terminate` in one transaction (BL-PRP-33); the deductions dispute (BL-PAY-15, BL-PRP-16).

---

### BL-PRP-33 — 01 Property: run `terminate` and its money effects in one transaction

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-KRN-11; BL-PRP-11 · **Blocks:** None.
**Review items:** 01-N12-a; follow-up of BL-PRP-11, Done-when point 1 (b)

#### Goal
The spec says the effects of lease `terminate` run "all in one transaction" and that "if crediting fails
for one invoice, the whole `terminate` rolls back". Today the money effects (credit notes, cheque returns,
deposit settlement, credit handling) run after the commit and a failed step becomes a task for the
accountant. With the transaction-aware money contracts (BL-KRN-11), `terminate` runs them inside its own
transaction, so the lease, the unit, the charges, the credit notes, the cheques and the deposit change
together or not at all.

#### Surfaces
- **Back office:** `terminate` either succeeds with every effect, or is refused with the reason of the
  failing step and nothing changed; the "finish by hand" task is no longer created for `terminate`.
- **System:** the effects run in the transition's transaction; events and messages go out after the
  commit.

#### Files to read
- `docs/verticals/01-property.md` §7.1 *Lease* (`terminate`); BL-PRP-11 *Edge cases decided* (Retry)
- `apps/property/src/termination.rs` (`terminate_effects`, `settle`)

#### Terms
- **Money effects of `terminate`:** pro rata credit notes, draft invoices discarded, cheques returned
  against one receipt, deposit `start_settlement`, the termination credit held or handled (BL-PRP-32).

#### Spec excerpt (authoritative for this issue)
| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `terminate` | notice_given | terminated | `leases.manage` | move-out inspection completed; all keys returned | unit `vacate`; deposit `start_settlement`; scheduled charges after the move-out date (or after `rent_due_until`, when set) cancelled; rent and recurring lines of invoices for periods after it credited pro rata by partial credit note; …; cheques in `received` or `due_for_deposit` dated after that date `return`; all in one transaction; Ejari cancellation task (`PRP.RUL.13`); tenant portal read-only (`POR.ACL.01`) |

BL-PRP-11 edge case: "**Retry:** if crediting fails for one invoice, the whole `terminate` rolls back
(one transaction)."

#### Edge cases decided
- **Ejari task and `LeaseTerminated`:** after the commit only (no task or event for a rolled-back run).
- **A cheque that changed meanwhile** (deposited between the screen and the click): the return skips it
  as today (it is no longer `received`), which is not a failure.
- **Refund of the credit** (choice `refund`, no deposit held): the refund request is part of the
  transaction; its approval runs later as usual.
- **Retry:** the user runs `terminate` again after fixing the cause; nothing is half done.

#### Must not
- Leave a terminated lease with uncredited invoices or unreturned cheques after a failure.
- Send the tenant or owner a message for a rolled-back termination.

#### Done when
- [ ] Given lease L-0042 with 2 invoices to credit and 5 cheques to return, `terminate` commits the lease,
      the unit, both credit notes, the 5 returns and the deposit `settlement_pending` together.
- [ ] When crediting the second invoice fails, nothing changed: the lease is `notice_given`, the first
      credit note does not exist, the cheques are `received`, the deposit is `held`, and the user sees the
      reason.
- [ ] No "finish by hand" task is created for a failed `terminate`; no `LeaseTerminated` is emitted.
- [ ] After the cause is fixed, `terminate` succeeds once.

#### Out of scope
The contracts themselves (BL-KRN-11); the credit applied to the deductions (BL-PRP-32); lease `cancel`
and `amend`, which keep their after-commit effects.

---

### BL-PRP-34 — 01 Property: put approved vendor bills on the owner statement (ticket jobs and AMC shares)

**Labels:** `vertical:01-property` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PRP-19; BL-PRP-24; BL-INV-24; Base: wl-invoicing Supplier bill lifecycle · **Blocks:** None.
**Review items:** D-14, 01-N8-b; follow-up of BL-PRP-24, Done-when point 1

#### Goal
An owner statement is *collected − expenses − fee*. Expenses are the vendor bills of the owner's units:
the bill of a maintenance job (also of a job cancelled after work started) and each building's share of
an AMC bill. Today the accountant types expense lines by hand, so a cancelled job's vendor bill never
reaches the owner. This issue reads the approved supplier bills coded to a ticket on the owner's units or
to a service contract covering the owner's buildings, splits contract bills by the allocation basis, and
adds them as expense lines of the draft statement (`PRP.RUL.05`).

#### Surfaces
- **Back office:** the draft statement shows expense lines *Vendor bill B-101 CoolAir — TKT-2026-0042
  (unit 1204)* and *AMC Q4 Lifts — Tower B, owner's share*, each linked to its bill; the accountant can still
  add or remove lines in the draft.
- **System:** `PRP.RUL.05` adds the bills approved in or before the period that are on no earlier
  statement of the agreement.

#### Files to read
- `docs/verticals/01-property.md` §4.2 step 6 and row "The job is cancelled after work started", §4.4
  step 1, §4.5 steps 7–8, §7.1 *Owner statement*, §7.2 `PRP.RUL.05`
- `docs/modules/wl-invoicing.md` §4 *Supplier bill*, §5.4 point 4 (allocation basis), §6 (`SupplierBills`)
- `apps/property/src/statement.rs` (module comment: expenses entered by hand today)

#### Terms
- **Allocation basis:** how a contract bill is spread: area (default), number of units, asset, or manual %.
- **Expense line:** a statement line with negative effect on the owner's balance, its source the bill.

#### Spec excerpt (authoritative for this issue)
01 §4.2 step 6: "A vendor job produces a vendor bill, allocated to the unit's owner (`FIN.PAB.01`)."
01 §4.2 row "The job is cancelled after work started": "work already done by a vendor is billed by the
vendor and allocated to the owner".
01 §4.4 step 1: "Rent collected, expenses (vendor bills, including each building's share of AMC costs)
and the management fee are collected per owner for the period (`PRP.OWN.01`, `FIN.PAB.01`, `WRK.SVC.01`)."
wl-invoicing §5.4 point 4: "A contract bill is spread over buildings, units or assets by the contract's
allocation basis (area by default, number of units, asset, or manual %), and the shares flow into the
owner statements of 01 (`PRP.OWN.01`)."

#### Edge cases decided
- **Which bills:** `approved`, `partially_paid` or `paid` bills; drafts, exceptions (possible duplicates)
  and void bills never.
- **Amount:** the bill's total including VAT (the owner bears what the company paid); VAT on
  recharged costs (`⚠ open: accountant (A-08)`) concerns only an invoice to the owner, not this line.
- **A chargeable job:** its bill is listed as for any job; crediting the owner with what the tenant
  pays on the ticket's invoice is not in this issue (collected lines read lease charges only).
- **Share rounding:** half up to a fils; the last share takes the difference so the shares add up.
- **A building of several owners:** the building's share is split again by the owners' units in it, by
  the same basis.
- **A bill reduced after it was on a published statement** (a supplier debit note): an adjustment line
  on the next statement; a published statement is never edited.
- **Repeated runs:** a bill (or share) is on one statement of the agreement only.

#### Must not
- List a bill that is not approved, or list a bill or a share twice.
- Edit a published statement.

#### Done when
- [ ] Given vendor job TKT-2026-0042 on unit 1204 cancelled from `on_hold` and CoolAir's bill of AED 350
      approved on 2026-10-20, the November draft statement for October lists it as an expense of AED 350.
- [ ] An AMC bill of AED 8,000 for Tower B and Tower C split by area 60/40, with the owner holding 25% of
      Tower B's area, gives the owner a line of AED 1,200.
- [ ] A bill held as a possible duplicate is not listed.
- [ ] Generating the draft again does not list the bill twice.
- [ ] A bill approved on 2026-11-03 is not on the October statement; it is on the November one.

#### Out of scope
Supplier bill approval and duplicates (BL-INV-11); paying the vendor (BL-PAY-28); AMC cost reports
(`RPT.LIB.01`).
