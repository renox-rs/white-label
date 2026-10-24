# Epic: 08 Auto — negative flows

> Status: in use (F1–F10 built; open items in [README](../README.md) §7) · Last updated: 2026-10-24

This epic builds the negative flows that belong to the [08 Auto](../../verticals/08-auto.md)
vertical itself: job card cancellation after work started, void, estimate expiry, the parts-wait
timer with the substitute vehicle, uncollected vehicles, vehicle incidents, the police-report
legal block, insurance claim withdrawal, total loss and void, supplier warranty recovery (D-06,
D-07), fleet contract suspension, end and early termination, export evidence, back-orders, the
recall resolver, party-block reactions, and the vertical's values for every shared negative-flow
mechanism. Shared behaviour (sales order amend, cancel and credit re-check, partial delivery,
cheques and handovers, returns to suppliers, landed cost, booking cancel after check-in,
write-off, disputes) is built in the module epics and linked by ID. It closes 08-N2-a, 08-N2-b,
08-N5-a, 08-N5-c, 08-N8-a, 08-N10-a, 08-N12-a, D-06, D-07 and D-08, the 08 MEDIUM items of review
§6 (vertical parts), and the 08 parts of X-05, X-07, X-08, X-09, X-11, X-13, X-14, X-17, X-18
and X-19.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-AUT-01 | Cancel a job card after work started and bill the work done (`AUT.ACL.21`) | high | m | Launch 08 | Base: 08 Job card lifecycle; BL-SCH-03; BL-KRN-01 |
| BL-AUT-02 | Void a job card opened in error | medium | s | Launch 08 | Base: 08 Job card lifecycle; BL-KRN-03; BL-SCH-03 |
| BL-AUT-03 | Expire an unanswered estimate and decline the job card (`AUT.RUL.17`) | medium | s | Launch 08 | Base: 08 Job card lifecycle |
| BL-AUT-04 | Track the legal parts supply period and the substitute vehicle (`AUT.RUL.16`) | high | m | Launch 08 | Base: 08 Job card lifecycle; BL-KRN-08 |
| BL-AUT-05 | Remind and escalate vehicles that are not collected (`AUT.RUL.18`) | medium | s | Launch 08 | BL-AUT-01; BL-AUT-03 |
| BL-AUT-06 | Record test drives and decide liability for a vehicle incident (`AUT.ACL.22`) | high | m | Launch 08 | Base: wl-ticketing Ticket lifecycle; BL-TKT-11; BL-AUT-07 |
| BL-AUT-07 | Refuse accident repair without a police report (`AUT.ACL.20`, D-08) | high | m | Launch 08 | BL-DOC-01; Base: 08 Job card and Insurance claim lifecycles |
| BL-AUT-08 | Withdraw an insurance claim, also after approval | high | m | Launch 08 | BL-AUT-01; BL-AUT-03; Base: 08 Insurance claim lifecycle |
| BL-AUT-09 | Declare a total loss and bill strip-down, labour and storage | high | m | Launch 08 | BL-AUT-01; Base: wl-invoicing Payer bill lifecycle |
| BL-AUT-10 | Void an insurance claim registered in error | medium | s | Launch 08 | Base: 08 Insurance claim lifecycle; BL-KRN-03 |
| BL-AUT-11 | Raise and recover a supplier warranty claim (D-06, D-07) | high | m | Launch 08 | Base: wl-ticketing Ticket lifecycle; BL-STK-03; BL-STK-05; BL-STK-06; BL-INV-14 |
| BL-AUT-12 | Terminate a fleet contract early | high | m | Launch 08 | Base: 08 Fleet contract lifecycle; BL-KRN-01; BL-AUT-13 |
| BL-AUT-13 | Suspend and end a fleet contract with its bookings and invoice | medium | s | Launch 08 | Base: 08 Fleet contract lifecycle |
| BL-AUT-14 | Alert on missing export evidence and open the VAT correction (`AUT.RUL.15`) | high | s | Launch 08 | BL-KRN-08; Base: wl-invoicing Invoice and Credit note lifecycle |
| BL-AUT-15 | Back-order lines without stock and serve them first come, first served | medium | m | Launch 08 | BL-SAL-17; BL-SAL-08; BL-SAL-09; Base: wl-inventory `STK.RUL.04` |
| BL-AUT-16 | Resolve recall trace lines to vehicle owners, fleets and customers (`TracePartyResolver` for 08) | high | m | Launch 08 | BL-STK-14; BL-STK-15; BL-STK-17 |
| BL-AUT-17 | React to a blocked party: fleet contracts, vehicles in the workshop, insurers | high | s | Launch 08 | BL-PTY-03; BL-AUT-12; BL-AUT-13 |
| BL-AUT-18 | 08 Auto: configure shared negative-flow settings | high | m | Launch 08 | BL-KRN-03; BL-KRN-04; BL-KRN-05; BL-KRN-08; BL-DOC-01; BL-INV-01; BL-INV-02; BL-INV-04; BL-PAY-09; BL-PAY-10; BL-PAY-14; BL-PAY-18; BL-SAL-07; BL-SAL-10; BL-SAL-12; BL-STK-03; BL-STK-09; BL-DLV-02; BL-SCH-03; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-STK-28; BL-STK-29; BL-STK-30; BL-STK-31; BL-SAL-22; BL-SAL-23; BL-SAL-24; BL-SAL-25; BL-SAL-26; BL-NTF-07; BL-NTF-08; BL-NTF-09; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |

---

### BL-AUT-01 — 08 Auto: cancel a job card after work started and bill the work done (`AUT.ACL.21`)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:field-pwa` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** Base: 08 Job card lifecycle · Base: wl-invoicing Invoice lifecycle · Base: wl-inventory stock movements · BL-SCH-03 (booking cancel after check-in, `SCH.ACL.06`) · BL-KRN-01 (approval ends without a yes) · **Blocks:** BL-AUT-05, BL-AUT-08, BL-AUT-09
**Review items:** 08-N2-a, X-17

#### Goal
A customer may stop a job after the workshop started it. The advisor cancels the job card with a
reason, records the work done per line, and gets the customer's acknowledgement. The customer is
billed only for work done (labour done, parts fitted, agreed special-order parts); parts issued
but not fitted go back to stock; the booking is closed; the vehicle is released safe or "as is"
with a signature. Billing less than the work done needs the branch manager.

#### Surfaces
- **Back office:** the service advisor runs *cancel* on the job card with a reason, records
  the work done per line and takes the customer's signature (or *refused to sign* with a
  witness); the branch manager sees a lowered or waived bill, or a refused signature, under
  "Needs your action" and approves; the advisor records *release vehicle* as safe or as is.
- **Field PWA:** while the cancel waits for approval, the technician's job card refuses new
  labour and part issues.
- **System:** the work-done invoice is issued, unfitted parts go back to stock, parts
  requests and the booking are cancelled, the purchaser gets a task to revise an open
  special-order PO, and `JobCardCancelled` is emitted.

#### Files to read
- `docs/verticals/08-auto.md` §3.3 `AUT.ACL.17`, `AUT.ACL.21`, §4.4 "When it goes wrong" (row "The customer stops the job…"), §6 *Job card* fields, §7.1 *Job card* (states, row `cancel`), §7.1 *Booking* (second `cancel` row), §9 Consumer protection
- `docs/modules/wl-scheduling.md` `SCH.ACL.06`

#### Terms
- **Job card:** the workshop work order for one vehicle visit.
- **Work done per line:** for each labour line: done (hours or flat rate), part done (hours done), not done; for each part: fitted, issued not fitted, special-order agreed.
- **Part done value:** hours done × the line's hourly rate in the estimate; for a flat-rate line, flat price × hours done ÷ flat hours, never above the flat price.
- **Invoiced:** the invoice is issued (not left in draft) when the transition runs (at final approval when an approval is needed).
- **Special-order part:** a part ordered for this job that the supplier will not take back; billable only when the customer agreed to it in the estimate.
- **Acknowledgement:** the customer's signature (`DOC.SIG.01`) on the work-done summary.
- **Released as is:** the vehicle leaves not drivable or unsafe, with the customer's signature.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `cancelled` | neutral | yes | Stopped after work started (customer, claim withdrawn, total loss); work done invoiced; unused parts returned |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | inspecting, awaiting_approval, in_progress, awaiting_parts, quality_check | cancelled | `job_cards.advise`, or system (claim `cancel` or `declare_total_loss` when the customer does not continue privately; `AUT.RUL.17` when the private estimate after a claim `cancel` is not signed in 7 days) | reason (customer stopped, claim withdrawn, total loss); work done recorded per line; customer acknowledgement (`DOC.SIG.01`), or *refused to sign* with a staff witness and branch manager approval, or the insurer's notice or the customer's written withdrawal; `AUT.ACL.21`: approval whenever the invoice is below the work-done value (labour done at the estimate's prices, parts fitted, agreed special-order parts) | invoice for work done (labour done, parts fitted, agreed special-order parts) to the customer; for a total loss no job card invoice: the work-done lines go to the claim, which invoices; billed special-order parts handed over with the vehicle; issued parts not fitted returned to stock; open parts requests cancelled; an open supplier PO for a special-order part kept, with a task to the purchaser to revise it; booking `cancel` (`SCH.ACL.06`); vehicle released when paid or on account, or under `AUT.ACL.17` for insurance jobs; event `JobCardCancelled` |
| `cancel` (booking) | checked_in | cancelled | `bookings.manage` only (`SCH.ACL.06`) | reason code; the job card is `cancel`led with its work done, `declined`, or `void`ed | slot released for the remaining time; work done billed on the job card |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `AUT.ACL.21` | A job card is cancelled after work started only by the service advisor with a reason, the work done recorded per line and the customer's acknowledgement; billing less than the work done, or waiving it, needs branch manager approval | `STA`, `SOD` | service advisor, branch manager | on |
| `AUT.ACL.17` | An insurance vehicle is released only after the excess and private items are paid, or the insurer confirms the excess is waived | `STA` | service advisor | — |

§4.4: … billed special-order parts are handed to the customer with the vehicle; an open supplier
PO for a special-order part is not cancelled by itself (the purchaser gets a task to revise it).
The vehicle is put back in a safe, drivable state where possible, or released as is with the
customer's signature. A customer who refuses to sign is recorded as *refused to sign* with a staff
witness, and the branch manager approves the cancel. The booking is cancelled by staff with the
work done (`SCH.ACL.06`).

§7.1 *Job card*: Labour *part done* on a cancelled job card is valued at the hours done × the
line's hourly rate in the estimate; a flat-rate line at its flat price × hours done ÷ its flat
hours, never above the flat price. Where a job card transition says an invoice is *invoiced*,
the invoice is issued (not left in draft) when the transition runs.

#### Edge cases decided
- **Billing less than work done:** when the advisor lowers or waives any work-done amount, the cancel becomes an approval request to the branch manager (not the advisor). Rejected or returned → the job card stays in its state (BL-KRN-01); nothing is invoiced or returned.
- **Amount check:** "work done" value = labour done at the estimate's prices (done lines at their price, part done lines at their part done value) + fitted parts + agreed special-order parts; any invoice below it, by even AED 0.01, needs approval.
- **Special-order parts billed:** handed to the customer with the vehicle and recorded on the job card.
- **Open supplier PO for a special part:** not cancelled automatically; the purchaser gets a task (PO `revise`, BL-STK-08).
- **Total loss (system):** the job card does not issue its own invoice; it hands its work-done lines to the claim, which invoices (BL-AUT-09). For claim withdrawal the invoice goes to the customer.
- **System cancel:** reason and insurer's notice (or customer's written withdrawal) replace the acknowledgement; `AUT.ACL.21` approval still applies if billing is lowered.
- **Nothing done yet (e.g. `inspecting`):** allowed; work done = none; an inspection fee is invoiced only if agreed at check-in.
- **Vehicle release:** recorded with time (Asia/Dubai) and condition *safe* or *as is* (with signature); allowed only when the invoice is paid or the customer is on account; for insurance jobs `AUT.ACL.17`.
- **Customer refuses to sign:** the advisor records "refused to sign" with a witness staff name; the branch manager approves the cancel.
- **Concurrency:** the technician cannot add labour or issue parts once the cancel is requested (pending approval).

#### Must not
- Must not bill labour not done or parts not fitted.
- Must not let the advisor approve their own reduced billing.
- Must not release an insurance vehicle without `AUT.ACL.17`.
- Must not cancel from `checked_in` (use `void` for an error or `decline` when the customer leaves), `ready` or `delivered`.

#### Done when
- [ ] Given JC-10 `in_progress` with labour "brake pads" done (AED 200), labour "discs" not done, pads fitted (AED 300), and a filter issued not fitted, when advisor A cancels with reason "customer stopped" and the customer signs, then JC-10 is `cancelled`, an invoice for AED 500 + VAT exists, the filter is back in stock, the booking is `cancelled` and `JobCardCancelled` is emitted.
- [ ] Given JC-12 with labour "service" at AED 150/hour, 3 hours estimated, 2 hours done, and flat-rate labour "alignment" AED 120 for 1 flat hour with 0.5 hours done, the cancel invoice is AED 300 + 60 = AED 360 + VAT.
- [ ] Given A lowers the invoice to AED 300, an approval request goes to branch manager B and JC-10 stays `in_progress`; on B's approval it completes as above with AED 300.
- [ ] A cannot approve that request.
- [ ] Given the customer on cash terms with the invoice unpaid, vehicle release is refused; after payment it is allowed.
- [ ] Given release *as is* without a signature, it is refused.
- [ ] Given JC-11 `checked_in`, `cancel` is not offered.
- [ ] Given the customer refuses to sign, A records *refused to sign* with witness W; the cancel waits for branch manager B's approval.
- [ ] Given an open supplier PO for a special-order part on JC-10, the PO is unchanged after the cancel and the purchaser has a task to revise it.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Booking cancel guard (BL-SCH-03); claim-driven cancels (BL-AUT-08, BL-AUT-09); uncollected vehicles (BL-AUT-05).

---

### BL-AUT-02 — 08 Auto: void a job card opened in error

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** Base: 08 Job card lifecycle · BL-KRN-03 (void) · BL-SCH-03 · **Blocks:** None
**Review items:** X-19 (void)

#### Goal
A duplicate check-in creates a second job card. Before any labour or part is recorded, the advisor
voids it with a reason, and its booking is cancelled with work done *none*.

#### Surfaces
- **Back office:** the service advisor runs *void* on a `checked_in` job card with a reason;
  it is refused when a labour or part line exists or a claim is linked, and no "unlink
  claim" action is offered.
- **System:** the booking is cancelled with work done *none*, the number is kept and
  `RecordVoided` is emitted.

#### Files to read
- `docs/verticals/08-auto.md` §4.4 "When it goes wrong" (last row), §7.1 *Job card* (row `void`), §7.1 *Insurance repair claim* (rows `void`, `cancel`), §7.1 settings *Void*

#### Terms
- **Void:** terminal `voided`; the record should never have existed; the number is kept.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `voided` | neutral | yes | Opened in error; no work or parts recorded |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `void` | checked_in | voided | `job_cards.advise` | reason given; no labour or parts recorded (`PLT.ACL.09`); no insurance claim linked | booking cancelled with work done *none* |

Settings, *Void*: Job cards from `checked_in` and insurance claims from `registered`; draft sales
orders. Never counter sales (`SAL.ACL.04`), invoices or anything sent to an insurer.

§4.4: `void` from `checked_in` before any work or parts (`PLT.ACL.09`), not while an insurance
claim is linked (first `void` the claim when nothing was sent to the insurer, which unlinks it,
or `cancel` it, which declines the job card); its booking is cancelled by staff with work done
*none*.

#### Edge cases decided
- **Approval:** none: no money or stock was posted (`PLT.ACL.09`).
- **Linked to an insurance claim:** void is refused while a claim is linked. There is no separate unlink action: when nothing was sent to the insurer, `void` the claim (BL-AUT-10), which unlinks it, then void the job card; otherwise `cancel` the claim (BL-AUT-08), which `decline`s the job card (no work recorded) instead.
- **Check-in photos and documents:** stay attached to the voided record (audit).
- **Vehicle record:** unchanged; mileage from the voided job card is not used as the vehicle's latest mileage.

#### Must not
- Must not void a job card with any labour or part line.
- Must not reuse the number.

#### Done when
- [ ] Given JC-20 `checked_in` with no lines, when advisor A voids it with reason "duplicate of JC-19", then JC-20 is `voided`, its booking is `cancelled` with work done *none*, and `RecordVoided` is emitted.
- [ ] Given JC-21 `checked_in` with a part line, void is refused.
- [ ] Given JC-22 `inspecting`, void is not offered.
- [ ] Given JC-23 `checked_in` linked to claim CL-20 `registered` with nothing sent, void of JC-23 is refused; after CL-20 is voided (JC-23 unlinked), void of JC-23 succeeds.
- [ ] No "unlink claim" action is offered on a job card.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Claim void (BL-AUT-10).

---

### BL-AUT-03 — 08 Auto: expire an unanswered estimate and decline the job card (`AUT.RUL.17`)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** Base: 08 Job card lifecycle (`send_estimate`, `decline`) · Base: wl-scheduling Booking lifecycle · **Blocks:** BL-AUT-05, BL-AUT-08
**Review items:** 08 MEDIUM "estimate expiry"

#### Goal
A customer who never answers an estimate leaves a vehicle in a bay. After 7 days with nothing
approved, the system declines the job card, asks the customer to collect the vehicle, and
invoices the inspection fee when one was agreed at check-in.

#### Surfaces
- **Back office:** the service advisor runs *decline* on a job card when the customer leaves
  before any work, and sees "message not delivered" on the job card when the collect SMS
  gives up.
- **System:** the hourly `AUT.RUL.17` check declines a job card 7 days after
  `awaiting_approval` with nothing approved, cancels the booking, issues an agreed
  inspection fee invoice and sends the `auto.collect_vehicle` SMS.

#### Files to read
- `docs/verticals/08-auto.md` §4.4 "When it goes wrong" (row 1), §7.1 *Job card* (state `declined`, row `decline`), §7.2 `AUT.RUL.03`, `AUT.RUL.17`

#### Terms
- **Nothing approved yet:** no estimate line of this job card has been approved by the customer.
- **Inspection fee:** a fee for diagnosis the customer agreed to at check-in.
- **Invoiced:** issued (not left in draft) when the transition runs (§7.1 *Job card*).
- **Collect message:** notification type `auto.collect_vehicle` (wl-notify, group *Booking confirmations and reminders*), sent by SMS to the customer's mobile (`NTF.MSG.03`); EN/AR text names the plate, the job card number, the reason (*estimate expired* or *estimate declined*), the inspection fee amount when one is invoiced, and the workshop's address and phone. Give-up: contact marked invalid and the job card shows "message not delivered" to the advisor.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `declined` | neutral | yes | Customer declined the estimate or left before one was sent, or it expired; inspection fee invoiced if any |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `decline` | checked_in, inspecting, awaiting_approval | declined, in_progress | customer, `job_cards.advise`, or system (estimate expired, `AUT.RUL.17`; claim `cancel` or `declare_total_loss` with no work started) | from `checked_in` or `inspecting`: no labour or parts recorded (target `declined`) | `declined` if nothing was approved yet: booking `cancel` (`SCH.ACL.06`, reason *estimate declined*), inspection fee invoiced when agreed at check-in, customer asked to collect the vehicle; otherwise work continues without the added items |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `AUT.RUL.17` | N days after job card → `awaiting_approval` | nothing approved yet and no open insurance claim linked; or the estimate is the private estimate sent after a claim `cancel` (*continue privately*) and is not signed | estimate expires. Nothing approved: job card `decline` (system, reason *no response*); customer asked to collect the vehicle; inspection fee invoiced if agreed at check-in. Private estimate after a claim withdrawal: job card `cancel` (system, reason *claim withdrawn*), work done invoiced to the customer | N = 7 | on | MVP |

#### Edge cases decided
- **Clock:** from the latest entry into `awaiting_approval`, 7 × 24 hours, checked hourly (Asia/Dubai). Revising the estimate while waiting does not restart it.
- **Added work on a job already approved:** "nothing approved yet" is false, so the rule does nothing; `AUT.RUL.03` reminders continue.
- **Job card linked to an open insurance claim:** skipped (`AUT.RUL.17` If); the insurer's answer is chased by `AUT.RUL.10` and `AUT.RUL.14`. An open claim is one not in a terminal state.
- **Booking:** on `declined`, the system cancels the booking with reason *estimate declined* (`decline` effects; allowed by `SCH.ACL.06`).
- **Customer approves after expiry:** the job card is terminal; the advisor opens a new job card.
- **Idempotent:** one decline per job card.
- **Second branch of the rule** (private estimate after a claim withdrawal → `cancel`): built in BL-AUT-08 on this rule's clock; this issue builds the rule and its *nothing approved* branch.

#### Must not
- Must not decline a job card where any line was approved.
- Must not invoice an inspection fee that was not agreed at check-in.

#### Done when
- [ ] Given JC-30 entered `awaiting_approval` on 2026-11-01 10:00 with nothing approved, on the 2026-11-08 10:00 hourly check it is `declined` with reason *no response*, an `auto.collect_vehicle` SMS naming the plate and JC-30 is queued to the customer, and the booking is `cancelled`.
- [ ] Given an inspection fee of AED 150 agreed at check-in, an invoice for AED 150 + VAT is issued to the customer at the decline and the collect SMS names AED 157.50.
- [ ] Given JC-31 `awaiting_approval` for added work after an approved estimate, nothing happens on day 7.
- [ ] Given JC-32 linked to an open claim, nothing happens on day 7.
- [ ] Given JC-33 `inspecting` with no labour or parts, when the customer leaves and advisor A runs `decline`, JC-33 is `declined` and its booking `cancelled`.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Collection reminders (BL-AUT-05).

---

### BL-AUT-04 — 08 Auto: track the legal parts supply period and the substitute vehicle (`AUT.RUL.16`)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** Base: 08 Job card lifecycle (`wait_parts`, `parts_arrived`) · Base: wl-notify SMS · BL-KRN-08 (owner alert) · **Blocks:** None
**Review items:** 08-N5-a

#### Goal
Consumer protection law requires spare parts within 7, 14 or 30 days depending on demand and
source, and a substitute vehicle when a warranted repair takes longer than 7 days. The job card
records when it started waiting for parts and the legal supply date; a daily rule warns the
advisor and the customer before the date, the branch manager on the date, and asks for a
substitute vehicle on warranty jobs after 7 days. The rule is compliance-critical.

#### Surfaces
- **Back office:** the job card shows the parts wait start and the legal supply date, not
  editable; the service advisor gets the date notices, a call task when the customer has no
  mobile, and a substitute vehicle task closed by recording plate, from and to (or the
  customer's refusal with a reason); the branch manager is told on the legal date; `deliver`
  is refused while a substitute is open.
- **System:** `AUT.RUL.16` runs daily at 08:00, fires each missed step once in order,
  notifies the customer by SMS, and alerts the Owner / GM and the company admin with an
  owner task when a run fails or is missing.

#### Files to read
- `docs/verticals/08-auto.md` §2 (consumer protection), §4.4 "When it goes wrong" (row "Parts do not arrive"), §6 *Job card* fields, §7.1 *Job card* (row `wait_parts`), §7.2 `AUT.RUL.16`, §9 Consumer protection
- `docs/OPEN-ITEMS.md` L-35

#### Terms
- **Part source:** where a part comes from (stock, another branch, local supplier, import); each has a supply period set at onboarding.
- **Legal supply date:** parts wait start + the supply period (7, 14 or 30 days).
- **Warranty repair:** a job card linked to a ticket of category *warranty*.
- **Substitute vehicle:** a car lent to the customer; recorded on the job card (plate, from, to).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `wait_parts` | in_progress | awaiting_parts | `job_cards.work` | parts request raised | parts wait start and legal supply date recorded: wait start + the shortest supply period among the requested parts' sources (`AUT.RUL.16`); not editable by hand |
| `parts_arrived` | awaiting_parts | in_progress | system (stock issued) | — | — |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `AUT.RUL.16` | daily at 08:00, for open job cards | in `awaiting_parts`: N days before the legal supply date, or the date itself; any state before `ready`: a warranty repair (linked *warranty* ticket) 7 days after check-in | parts: notify the service advisor and the customer with the new date (the expected arrival date on the parts request; when none, "we will confirm the date" and a task to the advisor); at the legal date notify the branch manager. Warranty repair: task to provide a substitute vehicle, recorded on the job card, closed only by the substitute record or the customer's refusal with a reason ✔ [S6]. Each step fires once (per wait for parts; per job card for the substitute); a late run fires every step whose day has passed and that has not fired, in order. Compliance-critical | N = 2; supply period per part source set at onboarding (7, 14 or 30 days) | on | MVP |

§9: which period applies to which part (by demand and source) is set per part source at
onboarding `⚠ open: lawyer (L-35)`.

#### Edge cases decided
- **Several parts, several sources:** the legal supply date is the earliest of the dates of the requested parts (the strictest).
- **Days:** calendar days, Asia/Dubai; the rule runs daily at 08:00; each step (N days before, the date) fires once per wait.
- **Missed or late run:** the next run fires every step whose day has passed and that has not fired for this wait, in order (N days before, then the date; the warranty task once). A step whose wait already ended (`parts_arrived`) does not fire.
- **Wait starts late:** the legal date is at least 7 days after the wait start and N = 2, so both steps are always in the future at `wait_parts`; nothing is skipped.
- **New date for the customer:** the expected arrival date entered by the purchaser on the parts request; when none is entered, the message says "we will confirm the date" and the advisor gets a task.
- **Wait ends and starts again:** each `wait_parts` records a new wait start and legal date; earlier waits stay in history.
- **Warranty 7 days:** counted from the job card's check-in date, in any state before `ready` (not only `awaiting_parts`); the task is created once per job card.
- **Substitute record:** plate, from date, to date; "to" is filled when the vehicle is returned. A job card cannot be `deliver`ed with an open substitute record.
- **Compliance-critical:** a run that ends `failed` (the job raises an error) or is `stop`ped, or a day with no run 60 minutes after 08:00, alerts the Owner / GM role and the company admin and opens an owner task (BL-KRN-08).
- **Customer without a mobile number:** the advisor gets a task to call.

#### Must not
- Must not let the legal date be edited by hand.
- Must not skip the warranty substitute task because a substitute was offered verbally; it is closed only by recording the substitute or a refusal by the customer with a reason.

#### Done when
- [ ] Given JC-40 entering `awaiting_parts` on 2026-11-01 for an imported part with a 14-day period, the legal date is 2026-11-15; on 2026-11-13 the advisor and the customer are notified; on 2026-11-15 the branch manager is notified.
- [ ] Given JC-41 (warranty ticket linked) checked in on 2026-11-01 and `awaiting_parts`, on 2026-11-08 the advisor has a substitute vehicle task.
- [ ] Given JC-42 (warranty ticket linked) checked in on 2026-11-01 and still `in_progress` (no parts wait), on 2026-11-08 the advisor has a substitute vehicle task.
- [ ] Recording substitute plate "Dubai A 12345" from 2026-11-08 closes the task; `deliver` is refused until "to" is filled.
- [ ] Given parts from a 7-day source and a 30-day source, the legal date uses 7 days.
- [ ] Given the 2026-11-13 08:00 run of `AUT.RUL.16` ends `failed` (test: the job raises an error), the Owner / GM role and the company admin are alerted and the owner has a task; given no run exists for 2026-11-14, the alert comes at the 09:00 or 09:15 check.
- [ ] Given JC-40 and no run on 2026-11-13 and 2026-11-14, the 2026-11-15 run sends the N-day notice and then the legal-date notice to the branch manager.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Which period applies to which part (L-35, onboarding).

---

### BL-AUT-05 — 08 Auto: remind and escalate vehicles that are not collected (`AUT.RUL.18`)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:medium` `size:s` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** BL-AUT-01 · BL-AUT-03 · Base: 08 Job card lifecycle (`pass_qc`, `deliver`) · **Blocks:** None
**Review items:** 08 MEDIUM "abandoned vehicles"

#### Goal
Vehicles that are ready, declined or cancelled but not taken away block bays. The customer is
reminded after 3 and 7 days, and the branch manager gets a task after 30 days. A storage fee is
charged only when the client turned it on and the customer agreed at check-in. What the workshop
may finally do with an abandoned vehicle is a legal open item.

#### Surfaces
- **Back office:** the service advisor records *release vehicle* on a `declined` or
  `cancelled` job card (time, recipient, safe or as is, signature), refused while unpaid on
  cash terms; the branch manager gets a task at day 30 saying "legal next steps open
  (L-36)".
- **System:** `AUT.RUL.18` sends the `auto.collect_vehicle` SMS at 3 and 7 days (a call task
  to the advisor when there is no mobile) and invoices the agreed storage fee at release,
  never after a total loss.

#### Files to read
- `docs/verticals/08-auto.md` §4.4 "When it goes wrong" (row "The vehicle is not collected"), §6 *Job card* (vehicle released at), §7.2 `AUT.RUL.18`, §9 Consumer protection
- `docs/OPEN-ITEMS.md` L-36

#### Terms
- **Released:** the vehicle left the workshop: job card `delivered`, or the *release vehicle* action on a `declined` or `cancelled` job card.
- **Storage fee:** a daily charge for keeping the vehicle, agreed by the customer at check-in.
- **Reminder:** notification type `auto.collect_vehicle`, SMS to the customer's mobile (`NTF.MSG.03`); EN/AR text names the plate, the job card number, days since ready (or declined, cancelled), the storage fee per day when it applies, and the workshop's address and phone.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `AUT.RUL.18` | N days after job card → `ready`, `declined` or `cancelled` | vehicle not released | remind the customer at each N; at M create a task for the branch manager; storage fee charged only when turned on and agreed at check-in: one day's fee at the agreed daily rate for each calendar day from the first reminder to the day before *release vehicle*, invoiced at release; no storage fee on a job card cancelled for a total loss (storage is billed through the claim, `declare_total_loss`) | N = 3, 7; M = 30; storage fee off | on | MVP |

§9: What a workshop may do with a vehicle that is never collected, and whether a storage fee
needs more than the customer's agreement at check-in, is `⚠ open: lawyer (L-36)`.

§7.1 *Job card*: **Release vehicle** is an action on a `declined` or `cancelled` job card (on
`delivered` it is part of `deliver`), not a transition: it records released at (Asia/Dubai), the
recipient, the condition *safe* or *as is* (as is needs the customer's signature) and the
signature. It is allowed when the invoices are paid or the customer is on account; for insurance
jobs `AUT.ACL.17` applies, and after a total loss the insurer's written instruction is required
and is enough, paid or not. It stops `AUT.RUL.18` and closes the storage fee, invoiced at
release.

#### Edge cases decided
- **Days:** calendar days from the state entry, Asia/Dubai, daily run; each step once.
- **Storage fee start:** one day's fee for each calendar day from the day of the first reminder (day 3) up to the day before release, at the daily rate agreed at check-in; invoiced at release.
- **Released between runs:** no further reminders.
- **Release on `declined`/`cancelled`:** the *release vehicle* action, recorded by the advisor with time, recipient, condition and signature; for insurance vehicles `AUT.ACL.17` applies.
- **Day 30 task:** says "legal next steps open (L-36)"; no disposal action exists in the app.
- **Total loss:** a job card `cancelled` with reason *total loss* still gets the reminders, but no `AUT.RUL.18` storage fee: storage is billed only through the claim at the insurer's rate (BL-AUT-09), so it is never charged twice.
- **Customer without a mobile number:** the advisor gets a task to call instead of the SMS.

#### Must not
- Must not charge storage when the setting is off or the customer did not agree at check-in.
- Must not offer any sale, disposal or lien action on the vehicle.

#### Done when
- [ ] Given JC-50 `ready` on 2026-11-01 and not released, an `auto.collect_vehicle` SMS is sent to the customer on 2026-11-04 and 2026-11-08, and the branch manager has a task on 2026-12-01.
- [ ] Given storage on at AED 50/day and agreed at check-in, release on 2026-11-10 invoices 6 days (2026-11-04 to 2026-11-09) = AED 300 + VAT.
- [ ] Given storage off, nothing is charged.
- [ ] Given storage on and agreed, and JC-52 `cancelled` with reason *total loss*, release on 2026-11-10 invoices no `AUT.RUL.18` storage fee.
- [ ] Given release recorded on 2026-11-02, no reminder is sent.
- [ ] Given JC-51 `declined` with the invoice unpaid and the customer on cash terms, *release vehicle* is refused; *as is* without a signature is refused.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Abandoned vehicle disposal (L-36).

---

### BL-AUT-06 — 08 Auto: record test drives and decide liability for a vehicle incident (`AUT.ACL.22`)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:field-pwa` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** Base: wl-ticketing Ticket lifecycle · BL-TKT-11 (ticket categories) · BL-AUT-07 (police report guard) · **Blocks:** None
**Review items:** 08-N10-a

#### Goal
A customer's vehicle can be damaged in the bay or on a test drive. Test drives are logged on the
job card. Damage is recorded as a *vehicle incident* ticket linked to the job card, with photos
compared to the check-in photos. Only the branch manager, not the people involved, decides
liability. Damage the workshop caused is repaired at no charge on an internal job card.

#### Surfaces
- **Back office:** the job card lists test drives (driver, times, mileage out and in); the
  branch manager opens the *vehicle incident* ticket, sees the check-in photos next to the
  incident photos and resolves it with *workshop* or *not workshop* and a reason; involved
  people are refused.
- **Field PWA:** the technician records a test drive on the job card and raises the incident
  ticket with photos and the road accident field (screen not specified yet).
- **System:** liability *workshop* creates an internal job card (payer = workshop) with the
  accident damage flag from the ticket; `pass_qc` is refused while a test drive has no
  mileage in.

#### Files to read
- `docs/verticals/08-auto.md` §3.3 `AUT.ACL.20`, `AUT.ACL.22`, §4.4 "When it goes wrong" (row "The vehicle is damaged…"), §6 *Job card* (test drives)

#### Terms
- **Vehicle incident:** a ticket category for damage while the vehicle is in the workshop's care or on a test drive.
- **Liability decision:** *workshop* (the workshop caused it), *not workshop* (pre-existing or customer), recorded with a reason.
- **Internal job card:** a job card for the same vehicle whose payer is the workshop itself; nothing on it is billed to the customer (its cost goes to the workshop's own expense).
- **Road accident:** a required yes/no field of the incident ticket: the damage came from a road accident (on a test drive or while moving the vehicle).
- **Involved person:** the ticket's reporter, the job card's technicians and advisor, and the test-drive driver.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `AUT.ACL.22` | A vehicle incident (damage while in the workshop's care or on a test drive) is closed only with a liability decision by the branch manager, not by the technician or advisor involved; when the branch manager was involved, another branch manager or the owner decides | `SOD` | branch manager | — |

§4.4: An incident ticket (category *vehicle incident*) linked to the job card, with photos
compared to the check-in photos; the branch manager decides liability (`AUT.ACL.22`). The ticket
records whether the damage came from a road accident (required field). Damage the workshop
caused is repaired on an internal job card at no charge to the customer; a road accident sets
that job card's accident damage flag, so it needs a police report first (`AUT.ACL.20`). With
liability *not workshop* no internal job card is created: a repair is offered to the customer
as an estimate on the job card, at the customer's cost. Test drives are recorded on the job
card: driver, time, mileage out and in. When the branch manager was involved, another branch
manager or the owner decides; the advisor informs the customer.

#### Edge cases decided
- **Category seeding:** this vertical's starter data adds the ticket category *vehicle incident* (BL-TKT-11 seeds only platform categories) with the required fields: job card, photos, road accident (yes/no), liability decision with reason (on `resolve`).
- **Photo comparison:** the ticket shows the check-in photos next to the incident photos; no automatic comparison.
- **Branch manager involved:** another branch manager or the owner decides.
- **Workshop liability:** creates an internal job card for the same vehicle (payer = workshop), with its accident damage flag set from the ticket's *road accident* field (so `AUT.ACL.20` requires a police report before repair).
- **Not workshop liability:** no internal job card; the ticket is resolved with the reason (pre-existing per check-in photos, or customer); the advisor informs the customer and may add the repair to the customer's job card as a normal estimate (`send_estimate`), billed to the customer only if approved.
- **Test drive open:** a test drive without mileage in blocks `pass_qc`; mileage in must be ≥ mileage out.
- **Customer notice:** the advisor informs the customer; no automatic message.

#### Must not
- Must not let an involved person resolve or close the ticket.
- Must not bill the customer for repairs of damage with liability *workshop*.

#### Done when
- [ ] Given JC-60 with test drive by technician T (out 10:00, 45,210 km; in 10:25, 45,222 km), the drive is listed on the job card.
- [ ] Given an incident ticket on JC-60 raised by T, when T or advisor A tries to resolve it, it is refused; branch manager B resolves with *workshop*, and an internal job card for the vehicle with payer = workshop is created; nothing from it appears on any customer invoice.
- [ ] Given B resolves an incident with *not workshop* and reason "scratch visible on check-in photo 3", no internal job card is created.
- [ ] Given the incident was a road accident, the internal job card is flagged accident damage and cannot be approved without a police report.
- [ ] Given a test drive without mileage in, `pass_qc` is refused.
- [ ] Given branch manager B drove the test drive, B cannot resolve the incident; branch manager C or the owner can.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Police report guard (BL-AUT-07).

---

### BL-AUT-07 — 08 Auto: refuse accident repair without a police report (`AUT.ACL.20`, D-08)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** BL-DOC-01 (`document_valid` guard) · Base: 08 Job card and Insurance claim lifecycles · **Blocks:** BL-AUT-06
**Review items:** 08-N8-a, D-08, X-13

#### Goal
The law forbids a garage to repair accident damage without a repair permit, which is the police
accident report. The report must be attached before the insurer estimate is submitted or the
claim approved, and before any accident repair is approved on the job card, also for private
repairs and rejected claims. There is no override. Earlier only `book_survey` checked it (D-08).

#### Surfaces
- **Back office:** the service advisor or insurance coordinator attaches the police accident
  report and a second user accepts it; job card `approve`, `book_survey`, `submit_estimate`
  and claim `approve` are refused with "police report required" until then; clearing the
  accident flag goes to the workshop manager for approval.
- **System:** the `document_valid` guard is a legal block with no override for any role or
  client setting, and a report on the claim counts for its job card and the other way round.

#### Files to read
- `docs/verticals/08-auto.md` §2 (insurance accident repairs), §3.3 `AUT.ACL.20`, §4.7 "When it goes wrong" (row "No police report"), §7.1 *Job card* (row `approve`), §7.1 *Insurance repair claim* (rows `book_survey`, `submit_estimate`, `approve`), §7.1 settings *Document blocks*, §9 Insurance repairs

#### Terms
- **Accident damage flag:** a job card field set at check-in when the vehicle has accident damage.
- **Police accident report:** document kind (Dubai Police app, Saaed, Rafid) with report number and date; subject = job card or insurance claim.
- **Legal block:** override mode `never`, locked by the platform.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `AUT.ACL.20` | Accident damage is repaired only when the police accident report (the repair permit) is attached to the job card or its insurance claim; no override, also for private repairs ✔ [S1] | `STA` | service advisor, insurance coordinator, workshop manager | legal block |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `approve` (job card) | awaiting_approval | in_progress | customer or `job_cards.advise` (recorded approval) | accident damage flagged: police report attached (`AUT.ACL.20`, legal block) | parts reserved |
| `book_survey` | registered | awaiting_survey | `insurance.manage` | insurer, policy and claim number recorded; police report (repair permit) attached ✔ [S1] | surveyor contacted by email |
| `submit_estimate` | registered, awaiting_survey | estimate_submitted | `insurance.manage` | estimate lines priced; damage photos attached; police report attached (`AUT.ACL.20`) | estimate PDF sent to the insurer (`DOC.GEN.01`) |
| `approve` (claim) | estimate_submitted | approved | `insurance.manage` | approval document attached; approved amount per line recorded (`AUT.ACL.16`); police report attached (`AUT.ACL.20`, legal block) | job card `approve` (system) for approved lines; excess amount set; customer notified |

Settings, *Document blocks*: Police accident report (job card or claim): `job_card.approve` and
`insurance_claim.approve` (block), override `never` (legal block ✔ [S1]).
§4.7: The claim cannot be submitted or approved and the job card cannot start accident repair
(`AUT.ACL.20`, legal block). The vehicle may stay checked in for inspection only.
§7.1 *Job card*: The accident damage flag is cleared only with a reason and the workshop
manager's approval (not the requester), and never while a claim is linked.

#### Edge cases decided
- **Where the report sits:** a report on the claim counts for its job card and vice versa (`BlockSubjects`: job card → itself and its claim; claim → itself and its job card).
- **No expiry:** the kind has no expiry; `document_valid` answers yes when an `accepted` report exists.
- **Removed report:** a report removed from the record is `archived` (wl-documents, `documents.manage`); an `archived` document is not `accepted`, so `document_valid` answers no and the guard refuses again until another report is accepted.
- **Review:** the kind has `blocks`, so the report is submitted and accepted by a second user (`DOC.ACL.03`).
- **Inspection allowed:** `start_inspection` and `send_estimate` are not blocked; only `approve` (job card), `book_survey`, `submit_estimate` and `approve` (claim).
- **Clearing the accident flag:** needs a reason and approval by the workshop manager (not the requester), and is refused while a claim is linked.
- **Rejected claim, private repair:** the job card's own `approve` still needs the report.
- **Added work on an accident job:** each later `approve` checks again (the report stays attached, so it passes).

#### Must not
- Must not offer an override, for any role or client setting.
- Must not let a private estimate bypass the guard.

#### Done when
- [ ] Given JC-70 flagged accident damage with no report, when the customer approves the estimate, it is refused with "police report required".
- [ ] Given claim CL-7 `registered` without a report, `book_survey` and `submit_estimate` are refused.
- [ ] Given CL-7 `estimate_submitted` and the report removed (archived), claim `approve` is refused.
- [ ] Given a report accepted on CL-7, JC-70's `approve` passes.
- [ ] No override request can be created for this kind.
- [ ] Given JC-71 without the flag, `approve` needs no report.
- [ ] Given JC-70 linked to CL-7, clearing its accident flag is refused; on JC-72 (no claim) it needs the workshop manager's approval.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Generic guard engine (BL-DOC-01).

---

### BL-AUT-08 — 08 Auto: withdraw an insurance claim, also after approval

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:field-pwa` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** BL-AUT-01 · BL-AUT-03 (`AUT.RUL.17` clock, extended here) · Base: 08 Insurance claim lifecycle · **Blocks:** None
**Review items:** 08-N2-b

#### Goal
A customer may withdraw the insurance claim, even after the insurer approved it. The coordinator
cancels the claim with the customer's written withdrawal and tells the insurer in writing. The
job card then follows: declined if no work started; otherwise the customer either continues
privately on a new private estimate or stops, and the work done is billed to the customer.

#### Surfaces
- **Back office:** the insurance coordinator cancels the claim with a reason, the customer's
  written withdrawal (from `approved` or `supplementary_pending`) and the choice *continue
  privately* or *stop*; with *continue privately* the advisor sends the private estimate.
- **Field PWA:** until the private estimate is signed, the technician's job card refuses
  labour and part entries.
- **System:** the withdrawal letter PDF is emailed to the insurer (a coordinator task when
  email gives up); the job card is declined or cancelled with the work done and fees
  invoiced to the customer; `AUT.RUL.17` cancels it when the private estimate is unsigned
  after 7 days.

#### Files to read
- `docs/verticals/08-auto.md` §3.3 `AUT.ACL.15`, §4.7 "When it goes wrong" (row "The customer withdraws the claim…"), §7.1 *Insurance repair claim* (state `cancelled`, row `cancel`), §7.1 *Job card* (rows `decline`, `cancel`)

#### Terms
- **Written withdrawal:** a document signed by the customer withdrawing the claim.
- **Continue privately:** the customer pays the repair; a private estimate is approved on the same job card.
- **Work started:** labour recorded as done or part done, or parts issued, on the job card.
- **Private estimate:** the estimate sent with `send_estimate` after the claim `cancel` with choice *continue privately*; the job card goes `awaiting_approval`.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `cancelled` | neutral | yes | Withdrawn by the customer |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | registered, awaiting_survey, estimate_submitted, approved, supplementary_pending | cancelled | `insurance.manage` | reason given (withdrawn by the customer); from `approved` or `supplementary_pending`: the customer's written withdrawal | insurer told in writing; job card: `decline` when no work started; otherwise the customer chooses to continue privately (a private estimate sent with `send_estimate` and approved on the job card; no labour or parts recorded until it is signed, and unsigned after 7 days counts as *stop*, `AUT.RUL.17`) or to stop (job card `cancel`, work done invoiced to the customer); inspection or storage fee invoiced to the customer if any (never to the insurer) |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `AUT.ACL.15` | Work on an insurance job is limited to the insurer's approved lines; anything beyond needs a recorded supplementary approval or the customer's own approval as a private item | `STA`, `LIM` | service advisor, technician | tolerance AED 0 |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `AUT.RUL.17` | N days after job card → `awaiting_approval` | nothing approved yet and no open insurance claim linked; or the estimate is the private estimate sent after a claim `cancel` (*continue privately*) and is not signed | estimate expires. Nothing approved: job card `decline` (system, reason *no response*); customer asked to collect the vehicle; inspection fee invoiced if agreed at check-in. Private estimate after a claim withdrawal: job card `cancel` (system, reason *claim withdrawn*), work done invoiced to the customer | N = 7 | on | MVP |

#### Edge cases decided
- **Choice recorded:** the cancel form asks *continue privately* or *stop* when work started; required.
- **No work started, job card in `awaiting_approval`:** system `decline`.
- **No work started, job card in `checked_in` or `inspecting`:** system `decline` as well (allowed from those states when no labour or parts are recorded).
- **Continue privately:** all job card lines become private (payer = customer); a private estimate (done + remaining lines, at the private price list) is sent with `send_estimate`, so the job card goes `awaiting_approval`; until signed, no new labour or parts may be recorded (`AUT.ACL.15`, AED 0 tolerance). Unsigned after 7 × 24 hours (hourly check, as in BL-AUT-03) → `AUT.RUL.17` runs the system `cancel` with reason *claim withdrawn* and the work done is invoiced to the customer (BL-AUT-01).
- **Inspection or storage fee:** when one applies, it is invoiced to the customer, never to the insurer.
- **Stop:** job card `cancel` (system) with reason *claim withdrawn*; BL-AUT-01 bills the work done to the customer.
- **Insurer letter:** generated PDF emailed to the insurer's claim email; when email gives up, the coordinator gets a task.
- **Pending supplementary:** cancelled with the claim; no answer is expected.
- **Built here:** the *private estimate* branch of `AUT.RUL.17` (BL-AUT-03 builds the rule).
- **Police report:** a private repair still needs it (BL-AUT-07).

#### Must not
- Must not invoice the insurer after withdrawal.
- Must not cancel from `approved` without the written withdrawal.

#### Done when
- [ ] Given CL-8 `approved` with JC-80 `in_progress` (AED 1,200 labour done), when coordinator A cancels with the customer's written withdrawal and choice *stop*, then CL-8 is `cancelled`, the insurer letter is sent, JC-80 is `cancelled` and an invoice to the customer for AED 1,200 + VAT exists.
- [ ] Given choice *continue privately*, CL-8 is `cancelled`, a private estimate is sent, JC-80 is `awaiting_approval` and labour entry is refused until it is signed.
- [ ] Given CL-9 `estimate_submitted` with JC-90 `awaiting_approval` and no work, JC-90 is `declined`.
- [ ] Given CL-14 `awaiting_survey` with JC-91 `checked_in`, cancelling CL-14 makes JC-91 `declined` and cancels its booking.
- [ ] Given choice *continue privately* on 2026-11-01 10:00 and the private estimate unsigned, at the 2026-11-08 10:00 check `AUT.RUL.17` makes JC-80 `cancelled` with reason *claim withdrawn* and the work done is invoiced to the customer.
- [ ] Given CL-9 cancelled with an inspection fee of AED 150 agreed at check-in, the fee invoice is addressed to the customer, not to the insurer.
- [ ] Given CL-8 `approved` without a written withdrawal, `cancel` is refused.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Total loss (BL-AUT-09); job card billing (BL-AUT-01).

---

### BL-AUT-09 — 08 Auto: declare a total loss and bill strip-down, labour and storage

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** BL-AUT-01 · Base: 08 Insurance claim lifecycle · Base: wl-invoicing Payer bill lifecycle (`FIN.TPP.01`) · **Blocks:** None
**Review items:** 08-N12-a

#### Goal
The insurer may declare the vehicle a total loss after survey, approval or strip-down. Work
stops, the job card is cancelled with reason *total loss*, and the strip-down, labour done and
storage are billed to the insurer as far as it approves them, the rest to the customer. The
vehicle leaves only on the insurer's written instruction, with a signature.

#### Surfaces
- **Back office:** the insurance coordinator declares the total loss with the insurer's
  notice, records the insurer's approval of the strip-down, labour and storage lines, and
  runs `invoice`; the advisor records *release vehicle* with the insurer's written
  instruction and a signature.
- **System:** the job card is cancelled (or declined) with no invoice of its own, the charge
  lines are listed on the claim, the customer is notified, the insurer and customer invoices
  are split by the approval, and `settle` runs when every invoice is paid.

#### Files to read
- `docs/verticals/08-auto.md` §2 (total loss), §4.7 "When it goes wrong" (row "total loss"), §7.1 *Insurance repair claim* (state `total_loss`, rows `declare_total_loss`, `invoice`), §7.1 *Job card* row `cancel`, §9 Insurance repairs
- `docs/modules/wl-invoicing.md` §4 *Payer bill* 08 mapping

#### Terms
- **Total loss:** the insurer decides repair costs more than 50% of the vehicle's value; the repair claim ends.
- **Insurer's total-loss notice:** the insurer's written decision, attached.
- **Total-loss charges:** strip-down, labour done and storage, listed per line for the insurer's approval.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `total_loss` | warning | no | Insurer declared the vehicle a total loss; work stopped; strip-down, labour and storage to bill |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `declare_total_loss` | awaiting_survey, estimate_submitted, approved, supplementary_pending | total_loss | `insurance.manage` | insurer's total-loss notice attached ✔ [S3] | work stops; job card `cancel` (reason *total loss*), or `decline` when no work started; charges for strip-down, labour done and storage (from check-in to release at the storage rate of the insurer's rate card, 0 when it has none) listed for the insurer's approval; vehicle released to the insurer or the owner on the insurer's written instruction, with a signature (`DOC.SIG.01`); customer notified |
| `invoice` | repair_complete, total_loss | invoiced | `invoices.issue` | from `total_loss`: the insurer's approval of the charges attached (reference, lines, amount, document) | from `repair_complete`: insurer invoice for the approved amount, customer invoice for excess and private items (`AUT.INS.04`); from `total_loss`: insurer invoice for strip-down, labour done and storage as the insurer approved, customer invoice for the rest (none when the insurer approved every line) |
| `settle` | invoiced | settled | system (every invoice of the claim paid) | — | claim closed in the insurer statement |

§7.1 *Job card*, *release vehicle*: after a total loss the insurer's written instruction is
required and is enough, paid or not.

#### Edge cases decided
- **One invoice path:** the job card `cancel` (system) records work done and hands the lines to the claim; it does not invoice itself; the claim `invoice` issues both invoices (avoids double billing).
- **Insurer approval of charges:** recorded as an insurer approval (reference, lines, amount, document) on the claim; `invoice` from `total_loss` is refused until it exists. Lines the insurer did not approve go to the customer invoice.
- **Customer share zero:** when the insurer approves every line, no customer invoice is issued and `settle` needs only the insurer invoice paid.
- **Storage:** from check-in to the release date at the storage rate on the insurer's rate card; 0 when the card has none. This is the only storage charge after a total loss: the customer storage fee of `AUT.RUL.18` (BL-AUT-05) is not charged on a job card cancelled for a total loss, so storage is never billed twice. Storage lines the insurer does not approve go to the customer invoice like any other unapproved line.
- **Job card with no work started (`checked_in`, `inspecting`, `awaiting_approval`):** system `decline` instead of `cancel`; the claim still invoices storage as approved.
- **Release:** the *release vehicle* action, with recipient (insurer or owner, per the instruction document), time and signature; release before the invoices are paid is allowed for a total loss (the insurer's written instruction prevails over `AUT.ACL.17`).
- **Parts issued not fitted:** returned to stock (BL-AUT-01).

#### Must not
- Must not continue any labour on the job card after the declaration.
- Must not release the vehicle without the insurer's written instruction and a signature.

#### Done when
- [ ] Given CL-10 `approved` with JC-100 `in_progress` (strip-down AED 800, labour done AED 600), when coordinator A declares total loss with the notice, then CL-10 is `total_loss`, JC-100 is `cancelled` with reason *total loss* and no invoice of its own, and the customer is notified.
- [ ] Given the insurer approves AED 800 strip-down and AED 400 labour, `invoice` creates an insurer invoice of AED 1,200 + VAT and a customer invoice of AED 200 + VAT.
- [ ] `invoice` is refused while no insurer approval of the charges is attached.
- [ ] Vehicle release without the instruction document is refused.
- [ ] Given CL-11 `registered`, `declare_total_loss` is not offered.
- [ ] Given the insurer approves every charge line, only the insurer invoice is issued and CL-10 `settle`s when it is paid.
- [ ] Given CL-12 `awaiting_survey` with JC-102 `checked_in`, `declare_total_loss` makes JC-102 `declined`.
- [ ] Given the client's `AUT.RUL.18` storage fee on and agreed at check-in for JC-100, the release after the total loss creates no `AUT.RUL.18` storage invoice; storage appears only on the claim's charges.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Salvage handling (excluded from the MVP); insurer short payment (`FIN.TPP.01`, base).

---

### BL-AUT-10 — 08 Auto: void an insurance claim registered in error

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** Base: 08 Insurance claim lifecycle · BL-KRN-03 · **Blocks:** None
**Review items:** X-19 (void)

#### Goal
A claim registered for the wrong vehicle or twice can be voided while nothing was sent to the
insurer; its job card is unlinked and carries on as a private job or is voided separately.

#### Surfaces
- **Back office:** the insurance coordinator runs *void* on a `registered` claim with a
  reason; it is refused once anything was sent to the insurer.
- **System:** the job card is unlinked, its estimate lines become private (payer = customer,
  not approved) and `RecordVoided` is emitted.

#### Files to read
- `docs/verticals/08-auto.md` §4.7 "When it goes wrong" (last row), §7.1 *Insurance repair claim* (state `voided`, row `void`)

#### Terms
- **Sent to the insurer:** any email, estimate, survey request or document sent to the insurer for this claim.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `voided` | neutral | yes | Registered in error; nothing sent to the insurer |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `void` | registered | voided | `insurance.manage` | reason given; nothing sent to the insurer (`PLT.ACL.09`) | job card unlinked from the claim |

#### Edge cases decided
- **Sent check:** any outbound message to the insurer's addresses linked to the claim makes void refused; use `cancel`.
- **Approval:** none (no money or stock).
- **Claim documents (Emirates ID, licence):** stay attached to the voided claim, still under `AUT.ACL.18`.
- **Job card after unlink:** keeps its accident flag; the police report attached to the claim no longer counts for it (BL-AUT-07).
- **Job card lines after unlink:** a claim can be voided only from `registered`, before any estimate was sent or approved, so the job card has no insurer-approved lines. Its estimate lines (if any) become private (payer = customer) and need the customer's approval through the normal job card `send_estimate` / `approve`.

#### Must not
- Must not void after anything was sent to the insurer.

#### Done when
- [ ] Given CL-12 `registered` and nothing sent, when A voids with reason "duplicate", CL-12 is `voided`, its job card is unlinked and `RecordVoided` is emitted.
- [ ] Given CL-13 `registered` with a survey email sent, void is refused.
- [ ] Given CL-12's job card JC-121 `inspecting` with two estimate lines, after the void both lines have payer = customer and none is approved.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Job card void (BL-AUT-02).

---

### BL-AUT-11 — 08 Auto: raise and recover a supplier warranty claim (D-06, D-07)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** Base: wl-ticketing Ticket lifecycle (category *warranty*) · BL-STK-03 (dispatch a return to supplier) · BL-STK-05 (credit a return) · BL-STK-06 (free replacement) · BL-INV-14 (settle a supplier debit note) · **Blocks:** None
**Review items:** D-06, D-07, X-08

#### Goal
When a customer's warranty claim is accepted and the part carries a supplier warranty, the
purchaser raises a claim to the supplier and sends the failed part back on a Return to supplier.
The claim is recovered when the supplier's credit or a replacement arrives, chased after 60
days, or written off with approval. The rules used the wrong trigger (`resolved` instead of
`closed`, D-07) and a state that does not exist (`supplier_claimed`, D-06).

#### Surfaces
- **Back office:** the purchaser gets a task when a warranty ticket closes as accepted,
  raises the supplier claim (refused with the term that failed when out of terms), and
  requests a write-off that the branch manager approves; purchaser and branch manager are
  told at 60 days.
- **System:** `AUT.RUL.07` creates the task on ticket `closed` (never on `resolved`),
  `raise` emails the claim PDF and drafts the Return to supplier, `SupplierReturnCredited`
  with a settled debit note recovers the claim, and `AUT.RUL.08` fires once at 60 days.

#### Files to read
- `docs/verticals/08-auto.md` §4.6, §7.1 *Warranty claim* mapping table, §7.1 *Supplier warranty claim* (states, transitions), §7.2 `AUT.RUL.07`, `AUT.RUL.08`
- `docs/modules/wl-inventory.md` §4 *Return to supplier*, event `SupplierReturnCredited`

#### Terms
- **Warranty ticket:** a ticket of category *warranty* with the field *warranty decision* (accepted or rejected).
- **Supplier warranty:** terms per brand or supplier (period, mileage, labour included) stored in the catalogue.
- **Return to supplier (RTS):** wl-inventory record sending the part back; outcome *credit*.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `raised` | warning | no | Claim document sent to the supplier |
| `recovered` | success | yes | Supplier credited or replaced |
| `written_off` | neutral | yes | Supplier refused; cost absorbed as warranty expense |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `raise` | — | raised | `supplier_claims.manage` | warranty ticket closed as *accepted*; supplier warranty applies | claim document sent to supplier; the failed part, when kept, goes on a Return to supplier (`wl-inventory`, outcome *credit*) |
| `recover` | raised | recovered | `supplier_claims.manage`, or system (`SupplierReturnCredited`) | supplier credit note (supplier debit note `settled` in `wl-invoicing`) or replacement received | — |
| `write_off` | raised | written_off | `supplier_claims.manage` + branch manager approval | reason recorded | cost booked as warranty expense |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `AUT.RUL.07` | warranty ticket → `closed` | *warranty decision* is accepted and the part has a supplier warranty | create a task for the purchaser to raise the supplier claim | — | on | MVP |
| `AUT.RUL.08` | N days after supplier warranty claim → `raised` | not `recovered` | notify purchaser and branch manager, once (no repeat) | N = 60 | on | MVP |

§4.6 step 4: A replacement part sent free of charge is received against the claim at zero cost.

§7.1 *Supplier warranty claim*: The supplier warranty *applies* when the failure date is within
the period from the part's purchase date from the supplier and, when the terms set a mileage and
the part was fitted on a job card, the vehicle's mileage at the failure less its mileage at
fitting is within it (a part sold over the counter is checked on the period only). *Labour
included* does not decide whether it applies: it adds the job card's labour for the replacement
to the claim amount.

#### Edge cases decided
- **`SupplierReturnCredited` with a manual "supplier refused" reason:** the guard (debit note settled or replacement received) fails; the claim stays `raised` and the purchaser gets a task to `write_off`.
- **Part not kept** (discarded by the customer): no RTS; `recover` is manual on the supplier's credit note.
- **One claim per ticket:** a second `raise` for the same ticket is refused.
- **`AUT.RUL.08`:** calendar days from `raised`, Asia/Dubai; once at day 60; no repeat.
- **Write-off approval:** branch manager, not the requester; rejected → stays `raised`.
- **Warranty expired at supplier level:** "supplier warranty applies" when the failure date (the warranty ticket's opening date) is within the period from the part's purchase date from the supplier and, when the terms set a mileage and the part was fitted on a job card, the vehicle's mileage at the failure less its mileage at fitting is within the limit; a part sold over the counter is checked on the period only. Out of terms → `raise` refused with the term that failed.
- **Labour included:** does not decide whether the warranty applies; when on, the labour of the replacement job card is added to the claim amount; when off, only the part is claimed.

#### Must not
- Must not trigger on ticket `resolved`.
- Must not reference a state `supplier_claimed`.
- Must not recover on a refused credit.

#### Done when
- [ ] Given warranty ticket T-1 *accepted* for a part with a 12-month supplier warranty bought 2026-05-01, when T-1 is `closed` on 2026-11-02, the purchaser gets a task; on T-1 `resolved` nothing happens.
- [ ] When purchaser P raises the claim, it is `raised`, the claim PDF is emailed and a draft RTS (outcome *credit*) exists for the kept part.
- [ ] When the RTS is credited by a settled debit note, the claim is `recovered` automatically.
- [ ] Given the claim `raised` on 2026-11-02 and not recovered, on 2027-01-01 purchaser and branch manager are notified.
- [ ] Given an RTS credited manually with reason "supplier refused", the claim stays `raised` and P has a write-off task.
- [ ] Given terms of 12 months / 20,000 km and a part fitted at 40,000 km that fails at 65,000 km within 12 months, `raise` is refused with "mileage out of terms"; at 58,000 km it is allowed.
- [ ] Given *labour included* on and AED 180 labour on the replacement job card, the claim amount is part cost + AED 180.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
RTS dispatch, credit and replacement receipt (BL-STK-03, BL-STK-05, BL-STK-06).

---

### BL-AUT-12 — 08 Auto: terminate a fleet contract early

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** Base: 08 Fleet contract lifecycle · Base: wl-invoicing usage billing (`FIN.USG.01`) · BL-KRN-01 · BL-AUT-13 · **Blocks:** BL-AUT-17
**Review items:** 08 MEDIUM "fleet contract early end"

#### Goal
A fleet contract may end before its term (fleet sold, non-payment, breach, agreement). The
contract is terminated with a reason, a notice date and an effective date, approved by the
owner. The effective date is the contract's last day of service; after it future services stop, open job cards are finished and billed at
contract rates, a final consolidated invoice covers the period to the effective date, and the
early termination fee is invoiced when the contract has one.

#### Surfaces
- **Back office:** the fleet manager requests termination with reason, notice date and
  effective date; the owner approves it under "Needs your action" (and any fee waiver); the
  contract shows "termination scheduled" until the effective date.
- **System:** the 00:30 daily job ends the contract after the effective date: later bookings
  cancelled, open job cards finished at contract rates, final consolidated invoice drafted,
  fee invoiced, fleet contact notified and `FleetContractTerminated` emitted.

#### Files to read
- `docs/verticals/08-auto.md` §4.5 step 6, §6 *Fleet contract* fields, §7.1 *Fleet contract* (row `terminate`), §7.2 `AUT.RUL.04`, `AUT.RUL.05`

#### Terms
- **Notice date:** when notice was given; **effective date:** the last day of the contract, still served (valid through it, Asia/Dubai), as for the end date of a contract that ends by itself (BL-AUT-13).
- **Consolidated invoice:** the monthly fleet invoice with a line per vehicle and job (`FIN.USG.01`).
- **Early termination fee:** a fee set in the contract.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `terminate` | active, suspended, renewal_due | ended | requested by `fleet_contracts.manage`, approved by the owner (`PLT.APR.01`), not the requester; runs at approval when the effective date is before today, otherwise by the system (daily job, 00:30 Asia/Dubai) on the first run after the effective date (the contract is valid through its effective date, its last day) | reason (fleet sold, non-payment, breach, agreement); notice date and effective date; effective date not before the notice date nor inside an already invoiced period | at approval: termination recorded and the contract marked *termination scheduled* (it keeps its state; `AUT.RUL.04` requests no booking after the effective date). On the effective date: future schedules and bookings cancelled; open job cards finished and billed at contract rates; final consolidated invoice for the period up to the effective date; early termination fee invoiced when the contract has one (waived only with the owner's approval); fleet contact notified; event `FleetContractTerminated` |

#### Edge cases decided
- **Effective date today or in the future:** after approval the contract keeps its state with "termination scheduled" shown; the daily job (00:30 Asia/Dubai) of the day after the effective date runs the transition, so the effective date itself is still served at contract rates. Effective date before today → runs at approval.
- **Effective date before the notice date:** refused. Before the last consolidated invoice's period end: refused (that period is already billed).
- **Between approval and effective date:** `AUT.RUL.04` creates no booking requests dated after the effective date; bookings already dated after it are cancelled when the transition runs. Bookings on the effective date stay.
- **Final invoice:** from the first day after the last invoiced period to the effective date; `AUT.RUL.05` drafts no further invoices.
- **Fee:** only when the contract has one; the fee is waived only with owner approval.
- **Rejected:** contract unchanged (BL-KRN-01).
- **Job cards opened after the effective date:** billed at the normal price tier.

#### Must not
- Must not run the effects before approval.
- Must not stop open job cards; they finish.

#### Done when
- [ ] Given contract FC-1 `active` with monthly invoices to 2026-10-31, when manager A requests termination reason *fleet sold*, notice 2026-11-01, effective 2026-11-20, and owner O approves on 2026-11-05, then FC-1 is still `active` on 2026-11-20 and on the 2026-11-21 00:30 run it is `ended`, bookings dated after 2026-11-20 are cancelled (a booking on 2026-11-20 stays), a final invoice for 2026-11-01–2026-11-20 is drafted, the AED 2,000 fee is invoiced, the fleet contact is notified and `FleetContractTerminated` is emitted.
- [ ] JC-110 open under FC-1 on 2026-11-20 finishes and is billed at contract rates.
- [ ] A cannot approve their own request.
- [ ] Given an effective date of 2026-10-15, the request is refused.
- [ ] Between O's approval and the 2026-11-21 00:30 run, FC-1 stays `active` with "termination scheduled" shown and `AUT.RUL.04` creates no booking request dated after 2026-11-20.
- [ ] Given FC-5 (invoiced to 2026-10-31) with notice 2026-11-20 and effective date 2026-11-24, approved on 2026-11-25, FC-5 is `ended` at approval; with effective date 2026-11-25 approved the same day, FC-5 ends on the 2026-11-26 00:30 run.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Suspend and natural end (BL-AUT-13); party block review (BL-AUT-17).

---

### BL-AUT-13 — 08 Auto: suspend and end a fleet contract with its bookings and invoice

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** Base: 08 Fleet contract lifecycle · **Blocks:** BL-AUT-12, BL-AUT-17
**Review items:** 08 MEDIUM "fleet contract early end"

#### Goal
A suspended contract (e.g. unpaid invoices) blocks new bookings while open job cards finish. A
contract that reaches its end date ends by itself: future schedules and bookings are cancelled,
the last consolidated invoice is drafted and the fleet contact told.

#### Surfaces
- **Back office:** the fleet manager suspends a contract with a reason and resumes it; new
  bookings for its vehicles are refused, and the advisor sees the bookings already made
  listed to confirm or cancel; `end` is not offered to users.
- **System:** `AUT.RUL.04` creates no booking requests while suspended; the 00:30 daily job
  ends a contract after its end date, cancels future bookings, drafts the last invoice and
  notifies the fleet contact.

#### Files to read
- `docs/verticals/08-auto.md` §7.1 *Fleet contract* (rows `suspend`, `resume`, `end`)

#### Terms
- **Service schedule:** planned services by time or mileage (`WRK.PPM.01`) that create bookings.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `suspend` | active, renewal_due | suspended | `fleet_contracts.manage` | reason given (unpaid invoices, fleet put on hold) | new bookings blocked; bookings already made are kept and listed for the advisor to confirm or cancel; open job cards finish |
| `resume` | suspended | active | `fleet_contracts.manage` | — | — |
| `end` | active, suspended, renewal_due | ended | system (daily job, 00:30 Asia/Dubai, on the first run after the end date; the contract is valid through its end date) | — | future schedules and bookings cancelled; last consolidated invoice drafted; fleet contact notified |

#### Edge cases decided
- **Bookings already made when suspended:** kept but listed for the advisor to confirm or cancel; only new bookings are blocked.
- **`AUT.RUL.04` while suspended:** creates no booking requests.
- **`end` is system only:** no user may run it; users terminate (BL-AUT-12).
- **End date:** the last day of service; the contract ends on the first daily run (00:30 Asia/Dubai) after the end date (valid through it). The same rule applies to a terminated contract's effective date (BL-AUT-12).
- **Resume from suspended after end date passed:** `end` runs first (system), so resume is not possible.

#### Must not
- Must not let a user run `end` by hand.

#### Done when
- [ ] Given FC-2 `active` suspended for unpaid invoices, a new booking for its vehicle is refused, and JC-120 under FC-2 continues to `delivered`.
- [ ] Given FC-3 ending 2026-12-31, it is still `active` on 2026-12-31 and on the 2027-01-01 00:30 run it is `ended`, its future bookings are cancelled, the last invoice is drafted and the fleet contact is notified.
- [ ] `end` is not offered to users.
- [ ] Given FC-2 has a booking for 2026-11-10 when suspended, it is kept and listed for the advisor to confirm or cancel.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Termination (BL-AUT-12).

---

### BL-AUT-14 — 08 Auto: alert on missing export evidence and open the VAT correction (`AUT.RUL.15`)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:s` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** BL-KRN-08 · Base: wl-invoicing Invoice and Credit note lifecycle · **Blocks:** None
**Review items:** 08-N5-c

#### Goal
Re-exported parts are zero-rated only if they leave within 90 days with an exit certificate, a
bill of lading or an airway bill. At 60 days without evidence the accountant is warned; at 90 days the sale is
flagged "5% VAT due" and the accountant gets a task to issue a credit note and a new 5% invoice,
recharging the VAT where the sale terms allow. The rule is compliance-critical.

#### Surfaces
- **Back office:** the accountant is told at 60 days, sees the invoice flagged "5% VAT due"
  at 90 days with a task linking it, and issues the credit note and new invoice by hand;
  attaching an exit certificate, bill of lading or airway bill stops the rule.
- **System:** `AUT.RUL.15` runs daily at 08:00, fires missed steps late, and alerts the
  Owner / GM and the company admin with an owner task when a run fails or is missing.

#### Files to read
- `docs/verticals/08-auto.md` §2 (re-export), §4.1 "When it goes wrong" (last row), §7.2 `AUT.RUL.15`, §9 "Exports not leaving in time"
- `docs/OPEN-ITEMS.md` A-12

#### Terms
- **Zero-rated export sale:** an invoice with lines in the VAT category *export, zero-rated* (`AUT.EXP.01`).
- **Exit evidence:** customs exit certificate or bill of lading (or airway bill), attached to the invoice.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `AUT.RUL.15` | N days after the supply date (delivery or collection) of a zero-rated export sale (checked daily at 08:00 Asia/Dubai) | exit evidence (customs exit certificate, bill of lading or airway bill) missing | notify the accountant; at 90 days, flag that 5% VAT is due and create a task to correct the invoice (credit note and a new invoice at 5%; recharge the VAT to the customer where the sale terms allow, otherwise absorb it). Compliance-critical | N = 60, 90 | on | MVP |

§9: Whether a zero rate can be restored when evidence arrives later is `⚠ open: accountant (A-12)`.

#### Edge cases decided
- **Start date:** the invoice's supply date (delivery or collection date); calendar days, Asia/Dubai; the rule runs daily at 08:00; each step once.
- **Evidence:** any one of the three kinds (customs exit certificate, bill of lading, airway bill) attached to the invoice stops the rule for it.
- **Failed or missed run (BL-KRN-08):** a run that ends `failed` (the job raises an error) or is `stop`ped, or a day with no run 60 minutes after 08:00, alerts the Owner / GM role and the company admin and opens an owner task.
- **Evidence after day 90:** flag stays; the accountant is told; nothing reversed (A-12).
- **Missed run:** late run fires all due steps.
- **No automatic documents:** the task links the invoice; the accountant issues the credit note and new invoice.

#### Must not
- Must not issue the correction documents automatically.

#### Done when
- [ ] Given INV-140 zero-rated AED 25,000 supplied 2026-08-01 with no evidence, the accountant is notified on 2026-09-30 and on 2026-10-30 the invoice is flagged and a correction task exists.
- [ ] Given a bill of lading attached on 2026-08-20, nothing fires; the same with an airway bill.
- [ ] Given the 2026-10-30 08:00 run ends `failed` (test: the job raises an error), the Owner / GM role and the company admin are alerted and the owner has a task; a successful rerun closes it.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Restoring the zero rate (A-12).

---

### BL-AUT-15 — 08 Auto: back-order lines without stock and serve them first come, first served

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** BL-SAL-17 (back-orders in wl-sales) · BL-SAL-08 (`cancel_line`) · BL-SAL-09 (`cancel_remaining`) · Base: wl-inventory reorder suggestions (`STK.RUL.04`) and goods receipt · **Blocks:** None
**Review items:** 08 MEDIUM "back-orders"

#### Goal
When a part is not in stock, the order line is back-ordered instead of refused: the rest of the
order is picked and delivered, the open line feeds the reorder suggestions, and when the parts
arrive they are reserved for the oldest back-orders first and the customer is told. The customer
may cancel the open line. The mechanism is built in wl-sales (BL-SAL-17, setting *Back-orders
allowed* per channel, off by default); this issue turns the setting on in the 08 deployment
template for the counter, phone and sales rep channels and checks the 08 order flow (`pack`
with back-ordered lines, branch, credit hold) end to end.

#### Surfaces
- **Back office:** counter, phone and sales rep staff confirm orders with back-ordered
  lines, `pack` with back-ordered lines open and run `cancel_line` on a back-order; the
  reorder suggestion shows the back-ordered quantity.
- **System:** the 08 template turns *Back-orders allowed* on for those channels; a goods
  receipt reserves for the oldest back-orders first (skipping `credit_hold` until
  `release`), the customer is told once per order, and a reversed receipt back-orders the
  lines again.

#### Files to read
- `docs/verticals/08-auto.md` §4.1 "When it goes wrong" (row "A part is not in stock"), §4.3 step 1, §7.1 *Sales order* (rows `confirm`, `pack`, `release`, `cancel_line`, `cancel_remaining`, paragraph "Back-orders"), §7.2 `STK.RUL.04`
- `docs/modules/wl-sales.md` §4.2 paragraph "Back-orders", §7 setting "Back-orders allowed per channel"; `docs/modules/wl-inventory.md` §7 `STK.RUL.04`

#### Terms
- **Back-order:** the open quantity of a confirmed line that could not be reserved.
- **Reorder suggestion:** a proposed purchase line per supplier (`STK.RUL.04`, daily 07:00).
- **Reorder quantity:** what `STK.RUL.04` would suggest for the item from its `ReorderPolicy` without back-orders (0 when the item has no reorder point or is above it); back-ordered quantity is added on top of it (wl-inventory §7).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `confirm` | draft | confirmed, credit_hold | `sales_orders.create` | customer and lines set; prices within `SAL.ACL.02` or approved | stock reserved; lines without stock back-ordered; `credit_hold` if the credit check fails |
| `pack` | picking | ready | `stock.pick` | all lines picked or back-ordered | delivery note generated |
| `cancel_line` | confirmed, picking, ready, partially_delivered | same state, delivered | `sales_orders.create` | reason; open quantity only; picked lines: outcome *return to stock* and `SAL.ACL.05` | quantity cancelled; reservation released; back-order removed from reorder suggestions |
| `cancel_remaining` | partially_delivered | delivered | `sales_orders.create` | reason given | open lines and back-orders cancelled; customer told |

Back-orders: a line without stock stays open and feeds reorder suggestions (`STK.RUL.04`). When
the parts arrive they are reserved for open back-orders first come, first served by the line's
confirmation time (lines confirmed at the same time by order number, then line number), and the
customer is told; orders on `credit_hold` are skipped and served when released: at `release`
the branch's available stock is reserved for their back-orders, the rest waits for the next
receipt. This is the `wl-sales` setting *Back-orders allowed*, on in this vertical for the
counter, phone and sales rep channels.

| Setting (wl-sales) | Default | 08 value |
|---|---|---|
| Back-orders allowed per channel | off | on for counter, phone and sales rep channels |

#### Edge cases decided
- **Partial stock:** the available quantity is reserved; only the rest is back-ordered.
- **Order of service:** by the line's confirmation time (oldest first); same time → lower order number, then lower line number. Orders on `credit_hold` are skipped; at `release` the branch's available (unreserved) stock is reserved for their back-orders at once, and what is still missing waits for the next receipt, where the order takes its place by its original confirmation time.
- **Other channels** (portal B2B): the setting stays off unless the client turns it on; a line without stock is refused there as today.
- **Branch:** back-orders are served from receipts into the order's branch only (`STK.ACL.04`).
- **Reorder suggestion:** quantity = back-ordered quantity not yet covered by open POs + the item's reorder quantity (wl-inventory §7).
- **Customer message:** one message per order when any of its back-orders is reserved.
- **Receipt reverses later:** reservations made from it are released and the lines are back-ordered again.

#### Must not
- Must not reserve arriving stock for a newer back-order before an older one.
- Must not keep a cancelled back-order in suggestions.

#### Done when
- [ ] In a new 08 deployment *Back-orders allowed* is on for the counter, phone and sales rep channels and off for the portal.
- [ ] Given 2 units of part P in stock, P with no reorder point set (min and max 0, reorder quantity 0), no open PO, and SO-1 (counter channel) for 5 P, on confirm 2 are reserved and 3 back-ordered; the next 07:00 suggestion for P is 3. Given an open PO for 1 P, it is 2.
- [ ] Given SO-1 (confirmed 09:00) and SO-2 (10:00) each back-ordering 3 P, a receipt of 4 P reserves 3 for SO-1 and 1 for SO-2; both customers are told.
- [ ] `cancel_line` on SO-2's back-order removes it from the next suggestion.
- [ ] `pack` succeeds with back-ordered lines open.
- [ ] Given SO-1 on `credit_hold`, a receipt of 3 P reserves for SO-2 first; SO-1 is served from the next receipt after its release.
- [ ] Given SO-1 on `credit_hold` with 3 P back-ordered and 4 P unreserved in the branch when it is released, 3 P are reserved for SO-1 at `release`.
- [ ] Given SO-3 and SO-4 lines confirmed at the same second, a receipt of 1 P reserves it for the lower order number.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
The back-order mechanism in wl-sales (BL-SAL-17); reorder suggestion engine (base `STK.RUL.04`).

---

### BL-AUT-16 — 08 Auto: resolve recall trace lines to vehicle owners, fleets and customers (`TracePartyResolver` for 08)

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** BL-STK-14 · BL-STK-15 · BL-STK-17 (items not tracked by lot) · **Blocks:** None
**Review items:** X-09

#### Goal
When a supplier recalls a part, the workshop must find who got it. Parts are not lot-tracked in
the MVP, so the client gives the received date range from the supplier's notice and the trace
covers parts sold or fitted on or after the first receipt of the part in that range. This vertical
resolves each subject to a party: job card → vehicle owner or fleet; sales order and account sale
→ customer; walk-in counter sales → a count per branch and day.

#### Surfaces
- **Back office:** the Owner / GM or a branch manager with `recalls.manage` opens a recall
  with the received date range and sees trace lines per vehicle owner, fleet or customer,
  all *possible*, plus one count line per branch and day for walk-in sales.
- **System:** the 08 `TracePartyResolver` maps job card → vehicle owner or fleet (the owner
  when the contract has ended) and sales → customer.

#### Files to read
- `docs/verticals/08-auto.md` §7.1 settings *Recall*
- `docs/modules/wl-inventory.md` §4 *Recall* (trace table, items not tracked by lot), §6 `TracePartyResolver`, §4 vertical mapping row *Recall*

#### Terms
- **Vehicle owner or fleet:** the vehicle's owner party, or the fleet when the vehicle is under a fleet contract.
- **Possible line:** an affected line where the unit cannot be proven to come from the recalled receipts.

#### Spec excerpt (authoritative for this issue)

Settings, *Recall*: A supplier recall of a part is traced through job cards and sales
(`TracePartyResolver`: job card → vehicle owner or fleet, the owner when the fleet contract has
ended; sales order and account sale → customer; walk-in counter sales → a count per branch and
day). Parts are not lot-tracked in the MVP, so the client gives the received date range from the
supplier's notice and the trace covers parts sold or fitted on or after the first receipt of the
part in that range (`wl-inventory`, items not tracked by lot), every line *possible*. Recalls are
opened by the Owner / GM and branch managers (`recalls.manage`).

wl-inventory mapping: 08: job card → vehicle owner or fleet, sale → customer. 08 parts are not
lot-tracked in the MVP: recall by item and received date range, lines marked *possible*.

wl-inventory *Recall*, items not tracked by lot: the trace lists every `issue` / `consume` /
`dispatch` of the item dated on or after the first receipt in the scope's date range, up to the
scope's trace end date when one is given, otherwise today (the received date range only
chooses the receipts that start the trace).

#### Edge cases decided
- **Fleet vehicle:** party = the fleet (contract party) and the fleet contact gets the notice; when the contract has ended, the vehicle owner.
- **Vehicle sold since:** the owner on the job card date (no ownership history in the MVP).
- **Counter sale on account:** customer; walk-in (no party): count per branch and day.
- **Job card parts issued not fitted (returned):** not listed.
- **Who opens recalls:** owner / GM and branch manager with `recalls.manage`.
- **Which date filters:** the client enters the received date range (and supplier) from the notice; there is no separate sale or fit date filter. A sale or fit is listed when it is on or after the first receipt in the range (and on or before the trace end date when given). The fit date of a job card part is the date its `issue` movement was posted.

#### Must not
- Must not expose another customer's vehicle data in a notice.

#### Done when
- [ ] Given part P fitted on JC-150 (vehicle of owner O1), JC-151 (vehicle under fleet F1), sold on SO-15 to workshop W1 and 4 walk-in sales at Branch 1 on 2026-09-03, then lines exist for O1, F1, W1 (all *possible*) and one line "4 sales, Branch 1, 2026-09-03".
- [ ] Given receipts of P on 2026-08-20 and 2026-09-01 and a scope received 2026-09-01–2026-09-30, a sale on 2026-08-30 is not listed and a sale on 2026-09-02 is; with a scope received 2026-08-01–2026-08-31 the 2026-08-30 sale is listed.
- [ ] Given JC-152 for a vehicle whose fleet contract has `ended`, the line names the vehicle owner.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Recall open, hold, notices and close (BL-STK-14–BL-STK-17).

---

### BL-AUT-17 — 08 Auto: react to a blocked party: fleet contracts, vehicles in the workshop, insurers

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 08 · **Size:** s
**Depends on:** BL-PTY-03 (block effect per agreement kind) · BL-AUT-12 · BL-AUT-13 · **Blocks:** None
**Review items:** X-18

#### Goal
When a party is blocked, each agreement reacts by its kind. In this vertical the owner reviews
fleet contracts (suspend or terminate), vehicles already in the workshop get their safety work
finished and are then released, and insurance claims with a blocked insurer continue because
money comes in.

#### Surfaces
- **Back office:** the owner gets one review task per blocked fleet with *suspend*,
  *terminate* or *continue*; the branch manager gets a review task listing the party's open
  job cards with *finish safety lines only* or *continue*; new estimate approvals on those
  job cards are refused.
- **System:** the agreement kinds are registered with their block effect; removed lines get
  reason *party blocked* and are not billed; claims with a blocked insurer keep their state
  and remittances are still recorded; texts use the neutral `on_hold` reason.

#### Files to read
- `docs/verticals/08-auto.md` §7.1 settings *Party blocked*
- `docs/modules/wl-parties.md` §5.6

#### Terms
- **Review:** nothing changes on its own; the decider gets a task to pause, continue or end.
- **Safety work:** labour lines the advisor marks as needed to make the vehicle safe to drive.
- **Removed line:** a job card line not yet started that the review takes off the job card with reason *party blocked*; it is not billed and stays visible in history (job card lines have no hold state).

#### Spec excerpt (authoritative for this issue)

Settings, *Party blocked*: Sales orders not yet dispatched held (module default); fleet contract:
review (the owner decides `suspend` or `terminate`); vehicles in the workshop: review (finish
safety work, then release; no new estimate lines approved while blocked); insurance claims with a blocked insurer continue (money in continues).

§7.1 *Job card*: While the customer of a job card is blocked (`wl-parties`), no new estimate
lines are approved on it (settings, *Party blocked*); the branch manager's review finishes the
lines marked *safety work* and removes the other lines not yet started from the job card with
reason *party blocked* (not billed), so the job card can go on to `finish_work`.

#### Edge cases decided
- **Agreement kinds registered:** *fleet contract* (review, decider owner / GM); *job card* (review, decider branch manager); *insurance claim* (continue, when the blocked party is the insurer).
- **Job card review task:** lists open job cards of the party; actions: *finish safety lines only*: the lines marked *safety work* continue, and every other line not yet started is removed with reason *party blocked* (lines already done or part done stay and are billed); then the job card follows `finish_work`, `pass_qc` and `deliver` (or *release vehicle*) as usual. *Continue*: nothing is removed. No new estimate lines may be approved while the party is blocked.
- **Block lifted:** removed lines stay removed; the advisor may send a new estimate for them.
- **Customer blocked on a claim:** the claim continues as for insurer; the job card review applies.
- **Neutral text:** tasks and messages use the neutral `on_hold` reason (BL-PTY-03).

#### Must not
- Must not suspend or terminate a fleet contract automatically.
- Must not stop recording insurer payments.

#### Done when
- [ ] Given fleet F2 blocked with contract FC-4 `active`, the owner gets one review task with *suspend* / *terminate* / *continue*; FC-4 stays `active`.
- [ ] Given customer C blocked with JC-170 `in_progress`, the branch manager gets a review task and new estimate approvals on JC-170 are refused.
- [ ] Given JC-170 with "brake pads" (safety work, not started) and "paint touch-up" (not started), when the branch manager chooses *finish safety lines only*, "paint touch-up" is removed with reason *party blocked* and not invoiced, and `finish_work` is allowed once "brake pads" is done.
- [ ] Given insurer I blocked, claims of I keep their state and a remittance from I is recorded.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
Sales order hold (Epic wl-sales: order hold on block).

---

### BL-AUT-18 — 08 Auto: configure shared negative-flow settings

**Labels:** `vertical:08-auto` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` `surface:back-office` `surface:field-pwa` `surface:portal` `surface:system` · **Milestone:** Launch 08 · **Size:** m
**Depends on:** BL-KRN-03 · BL-KRN-04 · BL-KRN-05 · BL-KRN-08 · BL-DOC-01 · BL-INV-01 · BL-INV-02 · BL-INV-04 · BL-PAY-09 · BL-PAY-10 · BL-PAY-14 · BL-PAY-18 · BL-SAL-07 · BL-SAL-10 · BL-SAL-12 · BL-STK-03 · BL-STK-09 · BL-DLV-02 · BL-SCH-03 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-STK-28 · BL-STK-29 · BL-STK-30 · BL-STK-31 · BL-SAL-22 · BL-SAL-23 · BL-SAL-24 · BL-SAL-25 · BL-SAL-26 · BL-NTF-07 · BL-NTF-08 · BL-NTF-09 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None
**Review items:** X-01, X-02, X-05, X-07, X-08, X-10, X-11, X-13, X-14, X-17, X-19

#### Goal
Seed the 08 deployment template with the vertical's values for every shared negative-flow
mechanism, so a new parts and workshop client starts with the right limits, blocks, roles and
fallbacks.
It also seeds the 08 values for the cash-flow forecast alert (`FIN.CFL.01`), the budget with
owner drawings (`FIN.BUD.01`), lead sources and campaigns (`SAL.CRM.02`), the supplier scorecard
(`STK.SUP.01`), loyalty (`SAL.LOY.01`), onboarding and SOPs (`HRM.ONB.01`) and registration
kinds (`DOC.EXP.01`), added on 2026-10-16: `FIN.RUL.23`, `FIN.RUL.24`, `FIN.ACL.21`,
`SAL.ACL.07`, `STK.RUL.09`, `STK.ACL.07`, `SAL.RUL.07`, `SAL.RUL.08`, `NTF.RUL.02`, `NTF.ACL.03`,
`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`. Onboarding runs on the
minimal staff records (`HRM.EMP.01`, MVP since 2026-10-16).

#### Surfaces
- **Back office:** a new 08 client starts with these values in Settings, and staff see their
  effects: void offered only where allowed, approvals for discounts, write-offs, stock
  adjustments, late entries and drawings, tasks to the responsible roles, and the forecast
  and budget alerts.
- **Field PWA:** drivers get partial acceptance at proof of delivery for parts deliveries,
  and a driver's collected cheques reach the deposit list only after another person counts
  the handover.
- **Portal:** the B2B buyer can flag an invoice as disputed.
- **System:** compliance-critical rules, notification give-up reactions, owner alerts,
  loyalty, campaign, onboarding and SOP rules run with the 08 values; the police report
  override stays `never`.

#### Files to read
- `docs/verticals/08-auto.md` §3.3, §4 "When it goes wrong" tables, §7.1 *Booking* paragraph, §7.1 "Negative-flow settings", §7.2 (reused rules)
- `docs/modules/wl-invoicing.md` §5.14–§5.16 and §7 (cash-flow forecast, company budget, owner
  drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments)
- `docs/modules/wl-sales.md` §4.6, §4.7, §5.8, §5.10 and §7 (campaigns, loyalty, segments);
  `docs/modules/wl-notify.md` §4.3, §5.3 and §7 (marketing broadcasts)
- `docs/modules/wl-inventory.md` §5.9 and §7 (supplier scorecard); `docs/modules/wl-hr.md` §5.8
  and §7 (onboarding); `docs/modules/wl-documents.md` §3, §5.7 and §7 (SOPs, registration kinds)
- `docs/WORKFLOW.md` §8

#### Terms
- **Deployment template:** the seed configuration applied when a client of a vertical is created.
- **Credit group:** companies under one owner sharing a combined credit limit (`PTY.REL.01`).

#### Spec excerpt (authoritative for this issue)

| Setting | Value (08) | Spec reference |
|---|---|---|
| **Void** — allowed | job cards from `checked_in`; insurance claims from `registered`; draft sales orders | §7.1 settings *Void* |
| **Void** — never | counter sales (`SAL.ACL.04`), invoices, anything sent to an insurer (sales return, credit note or supplementary instead) | §7.1 settings *Void* |
| Sales order cancel after picking | outcome *return to stock* with `SAL.ACL.05`, branch manager approval; no cancellation fee by default | §3.3, §7.1 settings |
| Booking cancel after check-in | staff only (`SCH.ACL.06`); no booking deposits in the MVP (`SCH.ACL.05` applies when a client takes deposits) | §7.1 *Booking* |
| `SAL.ACL.02` discount limit | above 5%; branch manager | §3.3 |
| `SAL.ACL.03` grace | 30 days past terms; per customer and credit group | §3.3 |
| Credit re-check | at dispatch and at counter collection for orders on account | §7.1 *Sales order* `hold`, `deliver` |
| `SAL.RUL.05` bounced cheque hold | on; whole credit group | §4.1, §7.1 settings |
| Partial acceptance at proof of delivery | on for parts deliveries | §4.1 |
| Document kind police accident report (subject job card or claim) | `blocks`: `job_card.approve`, `insurance_claim.approve` (block); override `never` (legal block) | §7.1 settings *Document blocks* |
| Customer trade licences, fleet contracts | reminders only (`DOC.RUL.01`), no `blocks` | §7.1 settings |
| Integration: SMS gateway | fallback: advisors call the customer; messages queued (`PLT.RUL.05`) | §7.1 settings *Integrations* |
| Integration: e-invoicing ASP (from 2027) | invoices queued, sent on recovery | §7.1 settings *Integrations* |
| Insurer portals, police reports | manual already (no connection) | §7.1 settings, §8 |
| Late entry (`PLT.ACL.10`) | job card times, test drives, deliveries keep business time; > 24 h late needs the branch or workshop manager | §7.1 settings *Late entry* |
| Invoice disputes | by accountant and B2B buyer in the portal: on; insurer short payments through the payer bill (`FIN.TPP.01`) | §7.1 settings *Disputes* |
| Compliance-critical rules | `AUT.RUL.15` (daily 08:00), `AUT.RUL.16` (daily 08:00) | §7.1 settings, §7.2 |
| Owner role (`PLT.RUL.04`) | Owner / GM | §7.1 settings |
| Write-off limit (`FIN.ACL.15`) | AED 0 (always approval); accountant, owner | §3.3 |
| Bad-debt relief proposal (`FIN.RUL.14`) | after 6 months | §4.1 |
| Bounced cheque | bounce fee off unless the client sets a charge type and amount; reminders 3 and 14 days (`FIN.RUL.18`); maximum two presentations | §4.1, §7.1 settings |
| Cash custody (`FIN.CSH.01`) | counter till sessions; cheques collected by drivers and sales reps handed over per person per day, cheques only; `FIN.ACL.20` on | §3.3, §4.1 |
| `STK.ACL.02` | AED 1,000 | §3.3 |
| `STK.ACL.06` | AED 5,000; purchaser | §3.3 |
| `STK.RUL.08` return chase | 30 days | §4.3 |
| `STK.RUL.02` low stock | on | §7.2 |
| `FIN.RUL.13` insurer follow-up | 30 days | §7.2 |
| Recall trace start | first receipt of the part in the received date range the client gives per recall (parts not lot-tracked; wl-inventory) | §7.1 settings *Recall* |
| Recall openers | `recalls.manage`: Owner / GM, branch managers | §7.1 settings *Recall* |
| `AUT.RUL.18` storage fee | off; when on, the daily rate agreed at check-in, from the first reminder to the day before *release vehicle*; never on a job card cancelled for a total loss | §7.2, §7.1 *Job card* |
| Back-orders allowed (wl-sales) | on for counter, phone and sales rep channels (set by BL-AUT-15) | §7.1 *Sales order* |
| Responsible role per document kind (wl-documents §5.2) | customer trade licences, export exit evidence → accountant; fleet contracts → Owner / GM; police accident report, driving licence, Mulkiya, Emirates ID on a claim → insurance coordinator of the workshop | §7.1 settings |
| Notification types (wl-notify §5.4 group) | statutory: `auto.claim_withdrawal_letter` (insurance coordinator), `auto.fleet_termination_notice` (Owner / GM); *Booking confirmations and reminders*: `auto.vehicle_ready`, `auto.claim_approved`, `auto.collect_vehicle`, `auto.fleet_booking_request`, `auto.fleet_contract_ended`, `auto.parts_delay` (`urgent`); *Portal invitations, document requests, signature requests*: `auto.insurer_request`, `auto.supplier_warranty_claim`; *Invoices, statements, payment reminders*: `auto.insurer_statement`; *Marketing*: `auto.service_reminder`; no group, `urgent`: `auto.recall_notice`; staff-only alerts: no group, give-up `none` | §7.1 *Notification types registered by 08* |
| Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 100,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the Owner / GM and the accountant, checked daily at 06:00; insurer receivables by their due dates (`FIN.TPP.01`); foreign-currency supplier bills at the latest rate (open POs not yet billed are covered by the budget fill); due dates adjusted by each customer's average days late: on; disputed amounts counted: off | §5; §7.1 settings |
| Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories owned by the accountant and mapped to the export accounts; one budget per legal entity and fiscal year, by month, approved by the Owner / GM; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.1 settings |
| Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of operating cost and profit; every drawing approved, whatever its amount (`FIN.ACL.21` limit AED 0), by the Owner / GM, never by the requester (`FIN.ACL.21`) | §5; §7.1 settings |
| Lead sources used (`SAL.CRM.02`) | referral, walk-in, website, social, paid ads, partner (insurers, fleet brokers), event, other; fleet contract enquiries are leads (`SAL.CRM.01`); walk-in customers get the source on their party; no source requires a campaign | §5; §7.1 settings |
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | search ads, Instagram, SMS, email, trade events; converted value: the first sales order, job card or fleet contract invoice; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.1 settings |
| Supplier scorecard review (`STK.SUP.01`, `STK.RUL.09`, `STK.ACL.07`) | score 0–100 over a 3-month window (minimum 3 receipts, 1 grace day); below 70: `STK.RUL.09` gives a review task to the purchaser / importer and tells the branch manager; preferred and alternative suppliers per part used by reorder suggestions (capability `STK.ROP.01`, reorder points; built by `STK.RUL.04` with BL-STK-28); changing a part's preferred supplier needs a second person (`STK.ACL.07`: on) | §5; §7.1 settings |
| Loyalty earn and redeem (`SAL.LOY.01`, `SAL.RUL.07`) | individual car owners only: `SAL.RUL.07` earns 1 point per AED 10 on workshop invoices and counter sales, reversed on returns and credit notes; none on B2B trade, fleet or insurer-paid jobs; 100 points = an AED 10 discount line on a later workshop invoice, confirmed by a one-time code to the member (VAT on the discounted supply: `⚠ open: accountant (A-15)`) | §5; §7.1 settings |
| Loyalty expiry, adjustments and segments (`SAL.LOY.01`, `SAL.RUL.08`, `SAL.ACL.07`, `NTF.RUL.02`, `NTF.ACL.03`) | points expire 24 months after they are earned, the member reminded 30 days before (`SAL.RUL.08`); a manual adjustment above 500 points needs a manager (`SAL.ACL.07`); segments: service due by date or mileage, no visit in 12 months, vehicle make, spend over AED 5,000 in 12 months; campaigns only to opted-in contacts (`NTF.ACL.01`), with opt-out, sent 07:00–21:00 and at most 2 marketing messages per contact in 7 days (`NTF.RUL.02`); a broadcast to more than 200 recipients needs a second person (`NTF.ACL.03`); `AUT.RUL.01` service reminders unchanged | §5; §7.1 settings |
| Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for technician, service advisor, counter sales, storekeeper and delivery driver, started by `HRM.RUL.16` when a staff record (`HRM.EMP.01`) is created in `onboarding` (none for a record created directly `active`, e.g. by a data import); checklist steps use `WRK.CHK.01`; tasks to the branch manager (workshop manager for workshop roles), the line manager and the employee; the employee's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds the assignees of an overdue step on the first run after its due date, then every 2 days, and tells the branch manager and the line manager after 5 days; completion blocks nothing (role activation not blocked) | §5; §7.1 settings |
| SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: workshop manager (check-in, QC, workshop safety) and branch manager (counter, cash, returns); required readers by role per branch; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP's owner role (the SOP owner named above) after 7 days overdue; acknowledged only by the reader (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.1 settings |
| Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Trademark registration* and *Other certification* (subject: legal entity); responsible role Owner / GM; `DOC.RUL.01` at 180, 90 and 30 days for trademarks and 90, 30 and 7 days for the others; no `blocks` | §5; §7.1 settings |

#### Edge cases decided
- **Loyalty VAT:** a redeemed points discount reduces the VAT of the discounted supply only
  (`⚠ open: accountant (A-15)`); the rest of the loyalty settings can be built.
- **Client changes:** every value may be changed per client by the company admin, except the police report override mode `never` and `PLT.ACL.09`.
- **Existing clients:** migration sets only values still at the platform default.
- **Credit group off for a customer** (no group): the hold applies to the customer only.

#### Must not
- Must not allow an override on the police report kind.
- Must not hard-code any 08 value in a module.

#### Done when
- [ ] **Void:** `void` is offered on a `checked_in` job card and a `registered` claim, never on a counter sale.
- [ ] **Sales:** cancelling a picked line proposes *return to stock*, needs branch manager approval and adds no fee; a 6% discount needs approval.
- [ ] **Credit and cheques:** a bounced cheque of company A in group G holds the open orders on account of every member of G; a third presentation is refused; no bounce fee is invoiced.
- [ ] **Documents:** `Documents::blocks_for("job_card", "approve")` returns the police report kind with mode `block` and override `never`.
- [ ] **Integrations and late entry:** a test drive entered 30 hours late creates an approval request to the workshop manager.
- [ ] **Compliance:** an `AUT.RUL.16` run that ends `failed` (test: the job raises an error) alerts the Owner / GM role and the company admin and opens an owner task (BL-KRN-08); so does a day with no `AUT.RUL.15` run 60 minutes after 08:00.
- [ ] **Money:** every write-off needs approval; a driver's cheque is not on the deposit list before another person counts the handover.
- [ ] **Stock:** a stock adjustment of AED 1,100 needs approval; an RTS of AED 5,100 needs approval.
- [ ] **Disputes:** a B2B buyer can flag an invoice as disputed in the portal.
- [ ] **Responsible roles:** an expiring customer trade licence notifies the accountant; a police report submitted for review on a claim goes to the insurance coordinators of the workshop.
- [ ] **Notifications:** when `auto.claim_withdrawal_letter` gives up, the contact point is marked invalid and the insurance coordinator gets a `statutory_notice` task; when `auto.vehicle_ready` gives up, the job card shows "message not delivered" and no task is created; `auto.parts_delay` giving up also creates a task to the service advisor.
- [ ] **Cash-flow forecast:** in a fresh 08 deployment, a projected balance of AED 99,999 in week
      5 alerts the Owner / GM and the accountant once (`FIN.RUL.23`, 06:00 run); AED 100,000 or
      more in every week alerts no one; the forecast shows 13 weekly buckets from Monday.
- [ ] **Budget and drawings:** a monthly budget for a legal entity prepared by the accountant
      needs approval by the Owner / GM; an expense with category *owner drawings* is left out of
      operating cost and profit reports, and the user who records it cannot approve it
      (`FIN.ACL.21`); an expense line reaching 100% of its month's budget line warns the
      category's owner role and the accountant once (`FIN.RUL.24`).
- [ ] **Lead sources:** a new lead or party offers the sources referral, walk-in, website, social,
      paid ads, partner, event and other; a campaign with channel *search ads*, a budget and cost
      entries reports leads, conversion, cost per lead, CAC and ROAS; a cost entry over the budget
      waits for a manager (`SAL.ACL.07`).
- [ ] **Supplier scorecard:** a supplier whose monthly snapshot over the 3-month window scores 69
      of 100 gets one review task for the purchaser / importer (`STK.RUL.09`); 70 creates none; an
      item's alternative supplier is offered on the reorder suggestion; making it the preferred
      supplier waits for a second person (`STK.ACL.07`).
- [ ] **Loyalty:** a workshop invoice of AED 500 to an individual car owner earns 50 points and a
      fleet job earns none; redeeming 100 points adds an AED 10 discount line; points earned on
      2026-11-01 expire on 2028-11-01, with a reminder on 2028-10-02 (`SAL.RUL.08`); a manual
      adjustment of 600 points waits for a manager (`SAL.ACL.07`); a contact who has not opted in
      is left out of a campaign, a campaign message queued at 22:00 is deferred to 07:00 and a
      third marketing message in 7 days is suppressed (`NTF.RUL.02`); a broadcast to 201
      recipients waits for a second person (`NTF.ACL.03`).
- [ ] **Onboarding and SOPs:** hiring a technician creates the technician checklist with tasks to
      the workshop manager, the line manager and the employee, and the role is active before it is
      complete; a step still open on the first run after its due date reminds its assignees, and
      again 2 days later (`HRM.RUL.17`); a new version of an SOP owned by the workshop manager asks
      every required reader to acknowledge again within 7 days (`DOC.RUL.04`), and no one else can
      acknowledge for them (`HRM.ACL.10`).
- [ ] **Registrations:** a fresh 08 deployment lists *Trademark registration* and *Other
      certification* with responsible role Owner / GM; a trademark registration expiring in 90
      days notifies the Owner / GM.
- [ ] Given a parts delivery with 3 lines, when the driver records 1 line refused with a reason at
      proof of delivery on the field PWA, then the delivery is accepted for 2 lines only and the
      refused line goes back to quarantine at run close.

_Deferred points of this issue: see [docs/progress/f8-auto-deferred.md](../../progress/f8-auto-deferred.md) (what is missing, who provides it, why)._

#### Out of scope
The mechanisms themselves (module epics listed in Depends on).
