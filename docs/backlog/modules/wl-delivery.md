# Epic: wl-delivery — negative flows

> Status: draft · Last updated: 2026-10-15 · Spec version: same commit as this file

This epic builds the negative flows of the Delivery and Run machines in
[wl-delivery](../../modules/wl-delivery.md): partial acceptance per line at proof of delivery,
short-paid cash on delivery, review and reversal of a delivery marked by mistake, lost and found
deliveries, the perishable policy, and a run that is aborted or handed over mid-route. It closes
review items X-11, X-12 and the delivery part of X-13, and the HIGH items 03-N3-a, 03-N8-a,
03-N10-a, 03-N11-a, 06-N9-b and 07-N4-a.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-DLV-01 | Block assign and run dispatch on an invalid driver or vehicle document | high | m | Launch 03 | Base: wl-delivery Delivery and Run lifecycle; Epic wl-documents: `document_valid` and `blocks`; Epic wl-hr: `qualification_valid` |
| BL-DLV-02 | Record partial acceptance per line at proof of delivery | high | m | Launch 03 | Base: wl-delivery Delivery lifecycle |
| BL-DLV-03 | Handle cash on delivery collected short (`WRK.ACL.09`) | high | m | Launch 03 | BL-DLV-02; Epic wl-kernel: approval returned; Epic wl-payments: cash custody |
| BL-DLV-04 | Put a disputed delivery under review and close the review | high | m | Launch 03 | Base: wl-delivery Delivery lifecycle; Epic wl-invoicing: invoice dispute flag; Epic wl-ticketing: tickets |
| BL-DLV-05 | Reverse a delivery marked by mistake (`WRK.ACL.10`) | high | m | Launch 03 | BL-DLV-04; Epic wl-kernel: approval returned; Epic wl-payments: cash custody and refunds |
| BL-DLV-06 | Mark a delivery lost and found again (`WRK.RUL.17`) | high | m | Launch 03 | BL-DLV-04; Epic wl-kernel: approval returned; Epic wl-ticketing: tickets |
| BL-DLV-07 | Apply the perishable policy after a failed attempt (`WRK.RUL.16`) | high | m | Launch 06 | Base: wl-delivery Delivery lifecycle; Base: wl-inventory stock movements (`wastage`); BL-DLV-08 |
| BL-DLV-08 | Abort a run mid-route | high | m | Launch 03 | Base: wl-delivery Run lifecycle; BL-DLV-09 |
| BL-DLV-09 | Hand open stops over to another driver's run | high | m | Launch 03 | BL-DLV-01; Epic wl-payments: cash custody |
| BL-DLV-10 | Limit perishable hand-overs to the hand-over window | high | s | Launch 06 | BL-DLV-07; BL-DLV-09 |
| BL-DLV-11 | Close a run with refused and missing goods accounted for | medium | s | Launch 03 | BL-DLV-02; BL-DLV-06 |

---

### BL-DLV-01 — wl-delivery: block assign and run dispatch on an invalid driver or vehicle document

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: wl-delivery Delivery and Run lifecycle · Epic wl-documents: `document_valid` guard and kind `blocks` · Epic wl-hr: `qualification_valid` · **Blocks:** BL-DLV-09, BL-LOG-25
**Review items:** X-13, 03-N8-a

#### Goal
A dispatcher must not put a delivery on a run, and nobody may dispatch a run, when the run's
driver has an expired licence or rider permit, or its vehicle has an expired registration
(Mulkiya) or insurance. Which document kinds block is configured in wl-documents per kind
(`blocks` lists `delivery.assign` and `run.dispatch`). For 03 Logistics these are legal blocks
with no override.

#### Files to read
- `docs/modules/wl-delivery.md` §4.1 *Delivery* (row `assign`), §4.2 *Run* (row `dispatch`), §6 *Consumes*
- `docs/modules/wl-documents.md` §5.6 (the standard guard, when it is checked, override)
- `docs/WORKFLOW.md` §8 (pattern "Expired document blocks")

#### Terms
- **Run:** one driver's trip with ordered stops; has a driver and a vehicle (none for on-foot riders).
- **`document_valid(subject, kind, on_date)`:** wl-documents guard; *yes* when the subject has an `accepted` document of that kind whose expiry is on or after `on_date`. Missing, `rejected` or a renewal still `pending_review` answers *no*.
- **`qualification_valid(employee, kind, on_date)`:** the wl-hr wrapper of the same check for staff licences and permits (`HRM.QUA.01`).
- **`blocks`:** setting on a document kind listing `<entity>.<transition>` it guards, each with a mode (`block` or `flag`) and an override mode (`never`, `with_approval`).
- **`Documents::blocks_for(entity, transition)`:** returns the kinds that guard a transition; call it before `document_valid`.
- **`BlockSubjects` provider:** maps a run to its document subjects: the driver (employee) and the vehicle (asset).

#### Spec excerpt (authoritative for this issue)

Delivery:

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `assign` | pending | assigned | `deliveries.dispatch` | run is `planned`; the driver is available; the run's driver passes `qualification_valid` (licence, permit) and its vehicle `document_valid` on the run date for the kinds whose `blocks` list `delivery.assign` (wl-documents §5.6) | added to the driver's day list |

Run:

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `dispatch` | planned | dispatched | `runs.dispatch` | driver checked in (`HRM.ATT.01`) and vehicle check done (`WRK.CHK.01`) when configured; items scanned out when configured; driver passes `qualification_valid` and vehicle passes `document_valid` today for the kinds whose `blocks` list `run.dispatch` (03: licence, rider permit, Mulkiya, insurance; no override) | deliveries `dispatch` (deliveries handed over to the run while `planned` are already `out_for_delivery` and are skipped); event `RunDispatched` |

wl-documents §5.6 example for 03: driving licence, RTA rider permit (employee); Mulkiya and
insurance (vehicle) → `run.dispatch` (block); `delivery.assign` (block); override: never (legal block).

Must implement in wl-delivery:
1. A `BlockSubjects` provider: run → driver (employee), vehicle (asset; none when vehicle type is *on foot*).
2. In `assign`: for each kind from `blocks_for("delivery", "assign")`, call the guard with `on_date` = the run's date.
3. In run `dispatch`: for each kind from `blocks_for("run", "dispatch")`, call the guard with `on_date` = today.
4. Refusal message names the document, its expiry date and who can renew it (wl-documents provides this).

#### Edge cases decided
- **Run date in the future:** `assign` checks on the run's date, so a licence that expires before the run date refuses `assign` even if valid today. `dispatch` checks today again.
- **Document expires between assign and dispatch:** `dispatch` is refused; the run stays `planned`. The dispatcher changes the driver or vehicle on the planned run, or moves the deliveries (`unassign`).
- **Renewal pending review:** answers *no*; the message shows the pending renewal (wl-documents behaviour).
- **On-foot rider (no vehicle):** only the driver's kinds are checked.
- **Override:** for kinds with override mode `never` (03 legal block) no override is offered. For kinds with `with_approval`, the user may request a one-use override through wl-documents (`DOC.ACL.02`); this module only calls the guard, which answers *yes* while a valid override exists.
- **No kind lists the transition:** the guard passes (no extra check).
- **Time zone:** "today" and the run date are dates in Asia/Dubai.
- **Deliveries already assigned when a document expires:** no automatic unassign (spec §4.2, after the Run table); the next `dispatch` is refused and the dispatcher re-plans. wl-documents `DocumentExpired` notifies the responsible staff.

#### Must not
- Must not hard-code document kinds; always read them from `blocks_for`.
- Must not offer an override button for a kind with override mode `never`.
- Must not check `dispatch` against the run date instead of today.

#### Done when
- [ ] Given driver A with a driving licence expiring 2026-11-10 and a run planned for 2026-11-12, when the dispatcher assigns a delivery, then `assign` is refused with the licence named and its expiry date.
- [ ] Given the same driver and a run planned for 2026-11-09, `assign` succeeds.
- [ ] Given a `planned` run whose vehicle insurance expired yesterday, when `dispatch` is attempted, then it is refused, the run stays `planned`, and no delivery moves to `out_for_delivery`.
- [ ] Given an on-foot rider with a valid rider permit and no vehicle, `dispatch` succeeds.
- [ ] Given a 03 client (override mode `never`), no override request is offered on the refusal.
- [ ] Given no document kind lists `delivery.assign`, `assign` succeeds without calling `document_valid`.

#### Out of scope
Receiving driver checks on `hand_over` (BL-DLV-09). Document expiry reminders and `DocumentExpired` (Epic wl-documents). Midday-break warning `WRK.RUL.15` (not a negative-flow change).

---

### BL-DLV-02 — wl-delivery: record partial acceptance per line at proof of delivery

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: wl-delivery Delivery lifecycle · **Blocks:** BL-DLV-03, BL-DLV-11, BL-STK-22, BL-SAL-13, BL-AUT-18, BL-FNB-16, BL-LOG-07, BL-LOG-26, BL-MFG-14
**Review items:** X-11, 07-N4-a

#### Goal
At the stop, the recipient may accept some lines, or part of a line, and refuse the rest.
The driver records per line the delivered and refused quantity with a refusal reason. The
delivery becomes `delivered` with those quantities, and every source bills and fulfils only
the delivered quantity. Refusing everything is a failed attempt.

#### Files to read
- `docs/modules/wl-delivery.md` §3 (entities *Delivery* (POD methods required), *Delivery line*, *Proof of delivery*, *Refusal reason*), §4.1 (row `deliver`, paragraph "Partial acceptance"), §5.2 steps 2–3 and 5, §6 event `DeliveryDelivered`, §7 settings ("Required proof per source type", partial acceptance, refusal reasons)

#### Terms
- **Delivery line:** one item or parcel on the delivery, copied from the source; fields quantity, delivered quantity, refused quantity, refusal reason.
- **Refusal reason:** configurable list: damaged, wrong item, not ordered, short-dated, temperature, other.
- **Source type:** the kind of `Subject` that requested the delivery (dispatch note, shipment, F&B order, sales order).
- **POD:** proof of delivery: photo, signature or OTP, plus time and location.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `deliver` | out_for_delivery | delivered | `deliveries.complete` (the assigned driver) | the required POD methods captured; per line, delivered + refused = quantity, and every refused quantity has a refusal reason; at least one unit delivered; partial acceptance allowed for the source type (when it is off, no refused quantity: every line is delivered in full, or the driver uses `fail` with reason *refused*); cash collected not above expected (change is given back) and = expected (the amount to collect less the amounts of refused quantities, when the lines carry amounts) within the tolerance, or the difference has a reason and, above the tolerance, approval (`WRK.ACL.09`) | attempt and proof saved; cash collected added to the driver's cash custody (`FIN.CSH.01`); cheque registered (`FIN.CHQ.01`); refused lines come back on the run like undelivered goods; event `DeliveryDelivered` with per-line delivered and refused quantities and the COD difference |
| `fail` | out_for_delivery | attempted | `deliveries.complete` | failure reason chosen; location captured | attempt saved; recipient and source notified; event `DeliveryAttemptFailed` |

State change: `delivered` | success | **no** (was terminal) | Handed over with proof, in full or with refused lines; reversible only through review.

"Partial acceptance": The delivery is `delivered` with the per-line quantities; refusing
everything is a `fail` with the reason *refused*. Sources use the delivered quantities only.
The refused goods come back on the run. At run close each refused quantity is ticked received
back or missing, and `DeliveryRefusedReceived` tells the source, which takes the goods back (a
dispatch note: into quarantine with the refusal reason). Partial acceptance can be turned off
per source type (for example sealed parcels in 03, where the parcel is one line).

**Cash expected after partial acceptance.** When the source gives an amount to collect per
line, the expected cash is the sum of the accepted quantities' amounts (a part of a line pro
rata to its quantity), so refused lines reduce it without a difference. When the source gives
only one amount, the expected cash stays the full amount and a shortfall goes through the
difference flow below.

§5.2 step 3: for each refused line or quantity, enters the refused quantity and a refusal
reason (photo when the reason is damage). The refused goods stay on the vehicle.

Event `DeliveryDelivered` payload: delivery, source, attempt, proof, lines with delivered and
refused quantities and refusal reasons, cash expected, cash collected, COD difference with
reason and absorbed by, cheque.

Settings: partial acceptance per source type: on (03 single parcels: off); refusal reasons
list as above. Required proof per source type: photo, signature, OTP (any / all): default
photo + signature (§7; the required POD methods are copied to the delivery when it is created).

#### Edge cases decided
- **Required POD methods:** taken from the delivery's *POD methods required* field, copied from the §7 setting for its source type when the delivery is created; default photo + signature for every source type; mode *all* (each listed method needed) unless the client sets *any*. OTP is used only where a client configures it.
- **Quantities are decimals** in the line's unit (R18). delivered + refused must equal quantity exactly; no rounding.
- **Partial acceptance off for the source type:** the screen hides the refused fields; every line is delivered in full or the driver uses `fail` with reason *refused*.
- **All lines refused:** the stop screen offers only `fail` with reason *refused*; `deliver` with zero delivered units is refused.
- **Photo for damage:** required when a refused quantity has reason *damaged*; other reasons do not require one.
- **COD with refused lines:** when the source gives line amounts, the expected cash = sum of the accepted quantities' amounts (part of a line pro rata), with no COD difference for the refused part. When the source gives one amount only, the expected cash stays the full amount and a shortfall goes through the COD difference flow (BL-DLV-03).
- **Double tap / retry:** `deliver` is idempotent per attempt: a second submit for a delivery already `delivered` is refused and shows the saved proof.
- **Proof is immutable:** once saved it is never edited (`WRK.ACL.06`); corrections only through BL-DLV-05.

#### Must not
- Must not let the driver edit quantities after `deliver`.
- Must not emit `DeliveryDelivered` with the ordered quantity when lines were refused.
- Must not mark `delivered` as terminal (review and reversal leave it).

#### Done when
- [ ] Given a delivery with line A qty 10 and line B qty 5 (POD required: photo + signature, the default), when the driver captures both and records A delivered 10, B delivered 3 and refused 2 with reason *damaged* and a photo, then the delivery is `delivered` and `DeliveryDelivered` carries A 10/0 and B 3/2 *damaged*.
- [ ] Given the default POD setting and only a photo captured (no signature), `deliver` is refused.
- [ ] Given line B refused 2 without a reason, `deliver` is refused.
- [ ] Given delivered + refused = 4 for a line of 5, `deliver` is refused.
- [ ] Given refused reason *damaged* and no photo, `deliver` is refused.
- [ ] Given every line refused in full, `deliver` is refused and `fail` with reason *refused* moves it to `attempted`.
- [ ] Given a source type with partial acceptance off, refused quantities cannot be entered.
- [ ] Given a `delivered` delivery, the proof and line quantities cannot be edited by any role.
- [ ] Given COD with line amounts A AED 100 (qty 10) and B AED 50 (qty 5), and B delivered 3 / refused 2, the expected cash is AED 130; collecting AED 130 needs no difference reason.
- [ ] Given COD AED 150 without line amounts and the same refusal, the expected cash stays AED 150.

#### Out of scope
COD short payment approval (BL-DLV-03). Receiving refused goods at run close (BL-DLV-11). How sources react: dispatch note (BL-STK-22), sales order (BL-SAL-13), 03 shipment (Epic 03 Logistics).

---

### BL-DLV-03 — wl-delivery: handle cash on delivery collected short (`WRK.ACL.09`)

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-DLV-02 · Epic wl-kernel: approval request lifecycle (rejected or returned sends the record back) · Epic wl-payments: cash custody (`FIN.CSH.01`) and shortage (`FIN.ACL.06`) · **Blocks:** BL-FNB-16, BL-LOG-08, BL-LOG-26
**Review items:** X-11, 03-N3-a

#### Goal
When the recipient pays less cash than the amount to collect, the driver either records the
difference with a reason and gets a second person's approval from the phone, or fails the
attempt. The approver decides who absorbs the difference (customer, client, business or
driver). The driver can never approve their own difference.

#### Files to read
- `docs/modules/wl-delivery.md` §3 (*Proof of delivery*, *COD difference reason*), §4.1 (row `deliver`, paragraph "Cash on delivery short-paid or refused"), §5.2 step 4, §6 (`WRK.ACL.09`, permissions), §7 settings
- `docs/modules/wl-payments.md` cash custody (`FIN.CSH.01`) and `FIN.ACL.06`

#### Terms
- **COD:** cash on delivery; the delivery's *amount to collect*.
- **Tolerance:** the accepted difference without approval; default AED 0.
- **Absorbed by:** who carries the difference: *customer*, *client* (03: the shipper), *business*, *driver*.
- **Cash custody (`FIN.CSH.01`):** the record of cash a driver holds until handed over at the depot (wl-payments).
- **Forced failed attempt:** `fail` with reason *refused to pay* or *no cash*.

#### Spec excerpt (authoritative for this issue)

`deliver` guard (part): cash collected = expected within the tolerance, or the difference has
a reason and, above the tolerance, approval (`WRK.ACL.09`). Effect: cash collected added to
the driver's cash custody (`FIN.CSH.01`); event `DeliveryDelivered` with the COD difference.

| ID | Rule | Pattern |
|---|---|---|
| `WRK.ACL.09` | A COD collected below the expected amount by more than the tolerance is accepted only with a difference reason and approval by `deliveries.approve_cod_difference`, who chooses who absorbs it; the driver cannot approve their own difference; without approval the attempt fails | `LIM`, `SOD` |

Paragraph "Cash on delivery short-paid or refused": When it is less than the amount to collect
and above the tolerance, the driver either:
1. records the difference with a reason and asks for approval from the phone (`WRK.ACL.09`);
   the approver chooses who absorbs it, defaulting to the reason's setting: *customer* (the
   balance stays open on the source invoice or COD statement), *client* (03: deducted on the
   remittance line with the reason, and the client is told on the statement), *business*
   (posted as a COD loss) or *driver* (the full amount stays expected in the driver's cash
   custody, and a shortage follows `FIN.ACL.06`); or
2. fails the attempt with the reason *refused to pay* or *no cash* (a forced failed attempt):
   the goods stay with the driver and follow the reattempt, return or perishable policy.

The driver may withdraw the request and fail the attempt at any time. When no approver
answers within the approval timeout (§7, 10 minutes), the request lapses and the configured
fallback applies: by default the forced failed attempt, which the driver records with the
reason *refused to pay*. A late approval is then refused. A recipient who refuses to pay
anything is always a failed attempt.

Expected cash: the amount to collect, less the refused quantities' amounts when the lines
carry amounts (§4.1 *Cash expected after partial acceptance*).

COD difference reason (configurable): code, label EN/AR (recipient short of cash, disputes the
amount, agreed discount, no change), default absorbed by.

Settings: cash on delivery allowed; difference tolerance: on; AED 0. COD short payment above
the tolerance: allowed with approval, or always a failed attempt; default absorbed by per
difference reason; approval timeout; fallback when no approver answers: allowed with
approval; *customer*; 10 minutes; failed attempt. Permission
`deliveries.approve_cod_difference`.

#### Edge cases decided
- **Rounding:** amounts in AED with 2 decimals (fils); difference = expected − collected, no rounding.
- **Within tolerance:** a difference ≤ tolerance needs a reason only when > 0; no approval. With the default AED 0, every short payment needs approval.
- **Collected more than expected:** refused by the app (`deliver` guard); the driver enters at most the expected amount (change is given back).
- **Setting "always a failed attempt":** the approval path is hidden; a short payment above the tolerance can only end in `fail`.
- **Waiting for approval:** the delivery stays `out_for_delivery` with a pending approval. The driver can at any moment choose *no answer: fail attempt*; this withdraws the approval request and runs `fail` with reason *refused to pay*. After the approval timeout (10 minutes) the request lapses and the phone offers only the fallback (default: the forced failed attempt). A late approval after that is refused by the kernel (`failed`, the record moved).
- **Approver rejects:** the delivery stays `out_for_delivery`; the driver must collect in full or fail the attempt.
- **Driver holds the approve permission:** still cannot approve their own difference (SoD), even as admin.
- **Custody amount:** absorbed by *driver* → custody expects the full amount to collect; any other choice → custody expects the cash collected.
- **Pays nothing:** collected = 0 is not offered as a difference; only `fail`.
- **03 Logistics:** the absorber *customer* is not offered in 03 (only *client*, *business* or
  *driver*, none preselected; wl-delivery §4 vertical mapping row 03).

#### Must not
- Must not complete `deliver` with a difference above the tolerance before approval.
- Must not let the driver of the stop approve.
- Must not post the COD loss, remittance deduction or invoice balance here: consumers do it from `DeliveryDelivered`.

#### Done when
- [ ] Given COD AED 250, tolerance AED 0, when driver A enters AED 200 with reason *recipient short of cash*, then an approval request is created and the delivery stays `out_for_delivery`.
- [ ] When user B (with `deliveries.approve_cod_difference`) approves and picks *business*, then the delivery is `delivered`, custody of A increases by AED 200, and `DeliveryDelivered` carries expected 250, collected 200, difference 50, reason, absorbed by *business*.
- [ ] Same, approver picks *driver*: custody of A expects AED 250.
- [ ] Driver A cannot approve their own request.
- [ ] Given the approval pending, when A chooses *no answer: fail attempt*, then the delivery is `attempted` with reason *refused to pay* and the request is withdrawn.
- [ ] Given no answer within 10 minutes, the request lapses and the driver's phone offers only `fail` with reason *refused to pay*; a later approval is refused.
- [ ] Given the setting "always a failed attempt", entering AED 200 of 250 offers only `fail`.
- [ ] Given collected AED 0, `deliver` is not offered.

#### Out of scope
How the difference is posted: 03 remittance line (Epic 03 Logistics), COD loss and shortages (Epic wl-payments), invoice balance (Epic wl-invoicing).

---

### BL-DLV-04 — wl-delivery: put a disputed delivery under review and close the review

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: wl-delivery Delivery lifecycle · Epic wl-invoicing: invoice dispute flag (`DeliveryDisputed` → `dispute`) · Epic wl-ticketing: tickets · **Blocks:** BL-DLV-05, BL-DLV-06, BL-FNB-07, BL-LOG-09
**Review items:** X-11, 03-N11-a

#### Goal
When a recipient or client denies a delivery, or the driver reports a wrong tap, the delivery
goes to `under_review`. Its cash is held out of remittances and settlements and its billing
is held while the dispatcher checks the proof. If the delivery stands, the review closes
and everything is released.

#### Files to read
- `docs/modules/wl-delivery.md` §3 (*Delivery* frozen flag), §4.1 (states, rows `open_review` and `close_review`, paragraph "Lost and reversed deliveries"), §5.5, §6 events
- `docs/modules/wl-invoicing.md` §4 *Invoice* rows `dispute` and `resolve_dispute`

#### Terms
- **Frozen flag:** a field on the delivery; while set, its COD is not remitted or settled and its source is not billed.
- **Ticket:** a wl-ticketing record that holds the dispute conversation.
- **Outcome *upheld*:** the delivery stands as recorded.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `under_review` | warning | no | The recipient denies the delivery or it was marked by mistake; COD and billing frozen |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `open_review` | delivered | under_review | `deliveries.manage`, or system (dispute ticket from the recipient or the client) | reason; linked ticket (`wl-ticketing`) | frozen flag set: COD of this delivery held out of remittances and settlements, billing of the source held (the issued invoice gets the `disputed` flag in `wl-invoicing`); dispatcher and source notified; event `DeliveryDisputed` |
| `close_review` | under_review | delivered | `deliveries.manage` | outcome *upheld* and a note; ticket resolved (resolved in the same action when still open) | frozen flag cleared; billing and COD released; event `DeliveryDisputeClosed` |

Events: `DeliveryDisputed` (delivery, source, reason, ticket); `DeliveryDisputeClosed`
(delivery, source, outcome, ticket).

wl-invoicing reacts: `DeliveryDisputed` → system `dispute` on the source's issued invoice
(reason *delivery disputed*); `DeliveryDisputeClosed` with *upheld* → `resolve_dispute` `upheld`.

#### Edge cases decided
- **No ticket yet (manual open):** `open_review` by a user opens a new ticket and links it; the guard is met by the new ticket.
- **Second dispute while under review:** `open_review` is not available from `under_review`; the new complaint is linked to the existing ticket.
- **Invoice not issued yet:** billing is held by the frozen flag; sources must not bill a frozen delivery. No `disputed` flag is set (there is no invoice).
- **COD already handed over at the depot:** the cash stays in custody records; the frozen flag only excludes it from remittances and settlements not yet paid.
- **"Ticket resolved" guard:** `close_review` resolves the linked ticket in the same action if it is still open; the guard is met by resolving it as part of `close_review`.
- **Not upheld:** `close_review` offers only *upheld*; any other outcome goes through `reverse_delivery` (BL-DLV-05) or `mark_lost` (BL-DLV-06).
- **Concurrency:** two users closing at once: the second sees the record already moved (state machine version check).

#### Must not
- Must not edit the proof during review.
- Must not release the frozen flag except through `close_review` (or the reversal or lost paths).
- Must not let the source bill while the frozen flag is set.

#### Done when
- [ ] Given a `delivered` COD delivery of AED 300 with an issued invoice, when the dispatcher runs `open_review` with reason "recipient denies", then the state is `under_review`, the frozen flag is set, a ticket is linked, and `DeliveryDisputed` is emitted.
- [ ] Given that delivery, the COD AED 300 is excluded from a remittance prepared afterwards.
- [ ] Given a dispute ticket raised by the recipient for a `delivered` delivery, the system runs `open_review`.
- [ ] When the dispatcher runs `close_review` with outcome *upheld* and a note, then it is `delivered`, the flag is cleared, the ticket is resolved, and `DeliveryDisputeClosed` with *upheld* is emitted.
- [ ] `close_review` without a note is refused.

#### Out of scope
Reversal (BL-DLV-05); lost (BL-DLV-06); the invoice flag itself (Epic wl-invoicing); 03 remittance exclusion (Epic 03 Logistics).

---

### BL-DLV-05 — wl-delivery: reverse a delivery marked by mistake (`WRK.ACL.10`)

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-DLV-04 · Epic wl-kernel: approval request lifecycle · Epic wl-payments: cash custody and refunds · **Blocks:** BL-STK-25, BL-FNB-07, BL-FNB-16, BL-LOG-10, BL-LOG-26
**Review items:** X-11, 03-N11-a

#### Goal
A delivery wrongly marked delivered (wrong tap, wrong recipient, recipient denies it) is
corrected by `reverse_delivery`, never by editing the proof. A manager requests it with a
reason and liability; another manager approves. The original proof is kept and marked
reversed; the cash already recorded is handled; the source reverses its fulfilment.

#### Files to read
- `docs/modules/wl-delivery.md` §4.1 (row `reverse_delivery`, paragraph "Lost and reversed deliveries"), §5.5 step 3, §6 (`WRK.ACL.06`, `WRK.ACL.10`, permissions, event `DeliveryReversed`), §7 failure reasons, §10 "Proof as evidence"

#### Terms
- **Liability:** who is responsible: driver, business, client, none.
- **Open custody:** cash the driver still holds (not yet handed over at the depot).
- **Attempt with reason *reversed*:** a new attempt row that does not count against max attempts.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reverse_delivery` | delivered, under_review | out_for_delivery (run still `dispatched`), attempted (otherwise) | `deliveries.reverse` | reason (marked by mistake, recipient denies, wrong recipient); liability (driver, business, client, none); approval (`WRK.ACL.10`) | original proof kept and marked reversed, never edited; attempt with reason *reversed* that does not count against the maximum; cash recorded as collected: removed from the driver's open custody if not yet handed over, otherwise refunded to the payer or kept for the next attempt (approver's choice, `wl-payments`); driver liability goes to the cash shortage or an HR case; from `under_review`: frozen flag cleared, and the linked ticket resolved with outcome *reversed* (left open for the HR follow-up when liability is *driver*); source reverses its fulfilment; event `DeliveryReversed` |

| ID | Rule | Pattern |
|---|---|---|
| `WRK.ACL.06` | A proof of delivery is never edited after the delivery is `delivered`. A correction is only a `reverse_delivery` that keeps the original proof, marks it reversed and is audited (`WRK.ACL.10`); disputes are tickets | `STA` |
| `WRK.ACL.10` | `reverse_delivery` and a manual `mark_lost` need an operations manager's approval; the requester cannot approve, and the driver of the run can neither request nor approve | `SOD`, `STA` |

Event `DeliveryReversed`: delivery, source, reason, liability, original proof, cash treatment, approver.

#### Edge cases decided
- **Target state is computed at final approval**, not at request: if the run was `dispatched` at request but closed before approval, the target is `attempted`.
- **Frozen flag:** reversing from `under_review` clears the frozen flag (the delivery is no longer delivered) and resolves the linked ticket with outcome *reversed*; the ticket stays open when liability is *driver* (HR follow-up), otherwise it is resolved.
- **Partial reversal:** not supported; the whole delivery is reversed. A wrong quantity on one line is a sales return or credit note at the source.
- **Approval rejected or returned:** the delivery stays in its state (`delivered` or `under_review`); nothing of the effects runs.
- **Driver of the run:** the request button is hidden for the run's driver; an approval by the run's driver is refused even with the permission.
- **No cash on the delivery:** the cash choice is not shown.
- **Approver's cash choice** (*refund to payer* or *keep for next attempt*) is only asked when the cash was already handed over.
- **Repeat:** after a reversal, the delivery can be delivered again (new attempt, new proof) and reversed again; every proof stays.

#### Must not
- Must not edit or delete the original proof; it is marked reversed only.
- Must not let the requester or the run's driver approve.
- Must not count the *reversed* attempt against max attempts.
- Must not post the source's fulfilment reversal here; sources react to `DeliveryReversed`.

#### Done when
- [ ] Given a `delivered` COD delivery (AED 120 collected, not yet handed over) on a `dispatched` run, when manager A requests `reverse_delivery` (reason *marked by mistake*, liability *driver*) and manager B approves, then the delivery is `out_for_delivery`, the original proof is marked reversed, a *reversed* attempt exists, AED 120 leaves driver custody, and `DeliveryReversed` is emitted.
- [ ] Same with the run `closed` and cash handed over: the state becomes `attempted` and B must choose *refund to payer* or *keep for next attempt*.
- [ ] Manager A cannot approve their own request.
- [ ] The driver of the run cannot request or approve.
- [ ] When B rejects, the delivery stays `delivered` and the proof is unchanged.
- [ ] Given a delivery with attempts 2 of max 3, after the reversal the count is still 2.
- [ ] Given an `under_review` delivery, after reversal the frozen flag is cleared.

#### Out of scope
How sources reverse fulfilment: dispatch (BL-STK-25), sales order (BL-SAL-13), 03 shipment (Epic 03 Logistics). Cash shortage and HR case (Epic wl-payments, Epic wl-hr).

---

### BL-DLV-06 — wl-delivery: mark a delivery lost and found again (`WRK.RUL.17`)

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-DLV-04 · Epic wl-kernel: approval request lifecycle · Epic wl-ticketing: tickets · **Blocks:** BL-DLV-11, BL-STK-24, BL-LOG-11, BL-LOG-26
**Review items:** X-11, 03-N10-a

#### Goal
A delivery whose goods are neither delivered nor received back is `lost`. The system marks it
after 7 days untraced; a manager can mark it earlier with a second manager's approval. A claim
ticket opens and the source stops expecting cash and billing. If the goods turn up at a depot,
`found` puts the delivery back to `attempted`.

#### Files to read
- `docs/modules/wl-delivery.md` §4.1 (states, rows `mark_lost` and `found`), §5.4 step 2, §5.5 step 4, §6 (`WRK.ACL.10`, `WRK.RUL.17`, events `DeliveryLost`, `DeliveryFound`), §7 settings

#### Terms
- **Untraced:** no scan, attempt, review action or depot receipt recorded for the delivery.
- **Claim ticket:** a wl-ticketing ticket to recover the loss (insurer, driver, carrier).
- **Sender:** in 03, the client who shipped the parcel.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `lost` | error | no | Goods untraced; excluded from billing; can be found again |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `mark_lost` | out_for_delivery, attempted, under_review | lost | system (untraced for N days, `WRK.RUL.17`) or `deliveries.reverse` | manual: reason, liability, approval (`WRK.ACL.10`), and no scan, attempt, depot receipt or delivery outcome recorded after the request (re-checked at final approval; the request fails otherwise) | claim ticket opened (`wl-ticketing`); the source and the sender (03: the client) notified; COD not expected any more; source excludes it from billing and decides any credit note or claim; event `DeliveryLost` |
| `found` | lost | attempted | `deliveries.manage` | goods scanned or confirmed at a depot | attempt with reason *found* that does not count; source notified; the claim ticket gets a comment "goods found at <depot>" and its owner is notified (not closed); event `DeliveryFound` |

| ID | Rule |
|---|---|
| `WRK.RUL.17` | A delivery in `out_for_delivery`, `attempted` or `under_review` whose goods are not received back or traced for N days (7), counted from the latest of dispatch, last attempt, `open_review`, last scan or note, is marked `lost`; notify the dispatcher and the source, open a claim ticket |
| `WRK.ACL.10` | `reverse_delivery` and a manual `mark_lost` need an operations manager's approval; the requester cannot approve, and the driver of the run can neither request nor approve |

Events: `DeliveryLost` (delivery, source, state before, last known run and location, liability,
ticket); `DeliveryFound` (delivery, source, depot). Setting: untraced days before `lost`; clock
start: 7; latest of dispatch, last attempt, `open_review`, last scan or note.

#### Edge cases decided
- **Clock start:** N days count from the latest of: dispatch, last attempt, `open_review`, last scan or note on the delivery. Any of these resets the clock.
- **Lost from `under_review`:** `DeliveryLost` carries the state before, so the source reverses its fulfilment (dispatch `mark_lost` from `delivered`, BL-STK-24).
- **Time zone and run time:** the rule runs daily at 02:00 Asia/Dubai; "7 days" means 7 × 24 h since the clock start.
- **Idempotency:** a delivery already `lost` is skipped; one claim ticket per `mark_lost`.
- **`under_review` deliveries:** included by the rule; a review note resets the clock, so an actively handled review is not marked lost.
- **System liability:** the system sets liability *none*; a manager may set it later on the ticket.
- **Frozen flag from review:** stays set while `lost` (nothing is billed).
- **After `found`:** the delivery is `attempted`; the normal reattempt/return policy applies (`WRK.RUL.08`, or `WRK.RUL.16` for perishables). The claim ticket is not closed automatically; a comment "goods found at <depot>" is added and the ticket owner is notified.
- **Approval pending while the goods turn up:** a manual `mark_lost` needs no scan, attempt, depot receipt or delivery outcome recorded after the request. The kernel re-checks this at final approval: if any was recorded (e.g. the parcel scanned at the depot, or delivered), the request `fail`s and the delivery stays where it is. The system rule `WRK.RUL.17` has no such guard (its own untraced clock already covers it).
- **03 Logistics:** `DeliveryFound` takes the shipment from `lost` to `at_hub` (the delivery goes
  to `attempted`); the shipment is re-planned on a new delivery (wl-delivery §4 vertical
  mapping row 03).

#### Must not
- Must not delete or cancel the delivery when lost.
- Must not count the *found* attempt against max attempts.
- Must not mark a `delivered` delivery lost (it must go through review first).

#### Done when
- [ ] Given an `attempted` delivery last touched 2026-11-01 10:00, when the rule runs on 2026-11-08 02:00 (less than 7 × 24 h), nothing happens; on 2026-11-09 02:00 it is `lost`, a claim ticket opens, dispatcher and source are notified, `DeliveryLost` is emitted.
- [ ] Given a scan on 2026-11-05, the clock restarts from that time.
- [ ] Given manager A requests `mark_lost` with reason and liability *driver*, manager B approves → `lost`; A cannot approve; the run's driver cannot request.
- [ ] Given A's request pending and the parcel then scanned at the depot, B's approval makes the request `failed`; the delivery stays `attempted` and no claim ticket opens.
- [ ] Given a `lost` delivery, when it is confirmed at a depot, then `found` makes it `attempted` with a non-counting *found* attempt and `DeliveryFound` is emitted.
- [ ] A `delivered` delivery is never picked by the rule.

#### Out of scope
Dispatch reaction (BL-STK-24); 03 shipment lost and client credit note (Epic 03 Logistics); run close with missing goods (BL-DLV-11).

---

### BL-DLV-07 — wl-delivery: apply the perishable policy after a failed attempt (`WRK.RUL.16`)

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 06 · **Size:** m
**Depends on:** Base: wl-delivery Delivery lifecycle · Base: wl-inventory stock movements (`wastage`) · BL-DLV-08 · **Blocks:** BL-DLV-10, BL-STK-23, BL-FNB-01
**Review items:** X-12, 06-N9-b

#### Goal
Food cannot be redelivered tomorrow. A delivery flagged `perishable` gets one attempt only.
After a failed attempt it is returned to the origin or discarded with a photo, according to
the policy of its source type, and the wastage is recorded. The money decision belongs to the
source.

#### Files to read
- `docs/modules/wl-delivery.md` §3 (*Delivery* perishable flag), §4.1 (states, rows `reattempt`, `return`, `discard`, paragraph "Perishable deliveries"), §5.2 step 5, §6 (`WRK.RUL.08`, `WRK.RUL.16`, event `DeliveryDiscarded`), §7 settings, §10 "Food waste"
- `docs/modules/wl-inventory.md` §3 (`wastage` movement), §6 `STK.ACL.02`

#### Terms
- **Perishable:** a delivery flag; set on all F&B orders; any source may set it.
- **Origin:** the outlet or depot the delivery left from.
- **Wastage record:** a wl-inventory `wastage` movement (or, for a dispatch source, the dispatch `discard`).
- **`STK.ACL.02`:** wastage above a value needs approval by someone other than the recorder (AED 1,000; 06: AED 200).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `discarded` | neutral | yes | Perishable goods destroyed after a failed attempt; wastage recorded |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reattempt` | attempted | pending | `deliveries.manage` or system | attempts < max attempts; the reason allows a reattempt; not perishable (`WRK.RUL.16`) | queued for the next run |
| `return` | attempted | returned | `deliveries.manage` or system (attempts ≥ max, or perishable policy *return*) | goods back at the origin, scanned or confirmed | event `DeliveryReturned` |
| `discard` | attempted | discarded | `deliveries.manage` or system (perishable policy *discard*, `WRK.RUL.16`) | perishable; reason and photo; wastage above the limit approved (`STK.ACL.02`) | wastage record through `wl-inventory` with the source as reference; source notified; event `DeliveryDiscarded` |

| ID | Rule |
|---|---|
| `WRK.RUL.08` | After a failed attempt: reattempt while attempts < N, otherwise return (N = 3); not for perishable deliveries |
| `WRK.RUL.16` | After a failed attempt on a `perishable` delivery, or a run `abort` with perishable stops: no reattempt; run `return` or `discard` per the source type's policy; notify the source and the origin's manager |

"Perishable deliveries": max attempts 1 and no reattempt. Policy per source type: *return*
(the goods go back to the origin and the origin decides reuse or wastage) or *discard*
(destroyed with a photo, wastage recorded). For *discard*, the rule puts a "discard with
photo" task on the driver's stop screen; `discard` runs when the driver submits the photo and
reason, and until then the delivery stays `attempted`. What happens to the money is the
source's decision: a prepaid order is refunded or not per the failure reason and the vertical's rule;
a COD order collects nothing.

Setting: perishable source types: F&B orders; policy: *discard* for cooked food, *return* otherwise
(module default). A vertical may replace it: 06 sets *return* to the outlet for own-rider
deliveries, and *discard* with a photo only when the rider cannot return (run aborted)
(`docs/verticals/06-fnb.md` §7.2).
Event `DeliveryDiscarded`: delivery, source, reason, wastage record.

#### Edge cases decided
- **Who runs `discard` when the policy is discard:** `WRK.RUL.16` sets the delivery to "discard pending"; the guard needs a photo and reason, which only the driver can give. The rule creates a task on the driver's stop screen "discard with photo"; the system runs `discard` when the driver submits the photo and reason. Until then the delivery stays `attempted`.
- **Wastage above the limit:** `discard` creates an approval request (`STK.ACL.02`); the delivery stays `attempted` until approved. Rejected: the delivery stays `attempted`, and the dispatcher either has the driver submit `discard` again (new photo or reason, new approval) or runs `return` by hand. The policy decides only what the system runs; `return` is always open to `deliveries.manage` (row `return`, *By*), also for cooked food. The returned food then reaches the origin, whose own wastage record follows `STK.ACL.02` again, so the limit cannot be bypassed.
- **Which policy applies:** the vertical's setting when it sets one, otherwise the module default;
  the rule reads the policy at the failed attempt, so tests must set it explicitly.
- **Policy *return*:** the delivery needs the goods confirmed at the origin (scan or tick at run close); the system runs `return` then. Reuse or wastage is the origin's decision (vertical food-safety rule, not here).
- **Max attempts:** a perishable delivery has max attempts 1 regardless of the client setting (3).
- **Run abort with perishable stops:** the stops fail with *run aborted* (BL-DLV-08); this rule then returns or discards them at once, like any failed perishable attempt.
- **Non-counting reasons** (*not attempted*, *run aborted*): still no reattempt for perishables.
- **Idempotency:** the rule acts once per failed attempt; a second event for the same attempt is ignored.
- **Source a dispatch note:** the wastage is recorded by the dispatch `discard` (BL-STK-23) on `DeliveryDiscarded`; wl-delivery must not post a second wastage movement.

#### Must not
- Must not reattempt a perishable delivery, by hand or by `WRK.RUL.08`.
- Must not discard without a photo and a reason.
- Must not decide any refund here.

#### Done when
- [ ] Given a client on the module default policy (*discard* for cooked food) and a perishable
      cooked-food delivery that fails with reason *not home*, `reattempt` is not offered and the
      driver gets the "discard with photo" task.
- [ ] When the driver submits a photo and reason, the delivery is `discarded`, one wastage record
      exists with the order as reference, source and origin manager are notified,
      `DeliveryDiscarded` is emitted.
- [ ] Given the default policy and wastage AED 250 with a limit of AED 200, the discard waits for
      approval by someone other than the driver.
- [ ] Given that approval rejected, the delivery stays `attempted`; the dispatcher can run
      `return`, and it becomes `returned` once the food is confirmed at the outlet.
- [ ] Given a 06 client (policy *return* for own-rider deliveries) and a cooked-food delivery that
      fails with *customer unreachable*, no discard task is created; the delivery becomes
      `returned` when the food is confirmed at the outlet and `DeliveryReturned` is emitted.
- [ ] Given a 06 client and a run aborted with a perishable stop the rider cannot bring back, the
      stop gets the discard task at once and is never reattempted.
- [ ] Given a non-perishable delivery with 1 of 3 attempts, `WRK.RUL.08` still reattempts.

#### Out of scope
The abort itself (BL-DLV-08) and the hand-over window (BL-DLV-10). F&B order refund decision (Epic 06 F&B). Dispatch `discard` (BL-STK-23).

---

### BL-DLV-08 — wl-delivery: abort a run mid-route

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** Base: wl-delivery Run lifecycle · BL-DLV-09 · Epic wl-payments: cash custody · Epic wl-ticketing: tickets · **Blocks:** BL-DLV-07, BL-FNB-01, BL-LOG-13, BL-LOG-26
**Review items:** X-12

#### Goal
When a run cannot continue (breakdown, accident, driver unwell, weather), the dispatcher or
the driver aborts it. Open stops fail with a reason that does not count, recipients are told
of the delay, perishables follow their policy, and the cash is handed over at the depot as
usual.

#### Files to read
- `docs/modules/wl-delivery.md` §4.2 *Run* (states, row `abort`, paragraph after the diagram), §5.3 step 2, §6 event `RunAborted`, §7 failure reasons

#### Terms
- **Stop not handed over:** a delivery on the run still `out_for_delivery` that was not moved to another run by `hand_over`.
- **Cash handover:** the wl-payments record where the driver's cash is counted by someone else.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `returned_to_depot` | warning | no | Driver back, or the run was aborted; cash and undelivered items to hand over |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `abort` | dispatched | returned_to_depot | `runs.dispatch`, or `runs.drive` with a reason | reason (breakdown, accident, driver unwell, weather, other; *other* needs a text) | stops not handed over `fail` with *run aborted*, which does not count against the maximum; perishable stops follow `WRK.RUL.16`; recipients told of the delay; cash handover opened; the vehicle's breakdown or accident ticket opened when chosen (`wl-ticketing`); dispatcher and depot manager notified; event `RunAborted` |

"When the vehicle cannot move after an abort, the goods are handed over to another run or
collected by the depot; the run closes once each stop is delivered, handed over, received
back or reported missing."

Event `RunAborted`: run, driver, reason, open deliveries.

#### Edge cases decided
- **Order of operations:** do the hand-overs first (BL-DLV-09), then `abort`; only stops still on the run fail.
- **Reason *other*:** needs a free-text note (text required for *other*).
- **No cash collected:** no cash handover is opened.
- **Breakdown/accident ticket:** opened only when the user ticks it; linked to the vehicle asset.
- **Perishable stops:** fail with *run aborted* here like any stop; the perishable reaction (`WRK.RUL.16`, return or discard) is built in BL-DLV-07 (Launch 06), which listens to that failed attempt. 03 has no perishable deliveries.
- **Non-perishable stops:** after *run aborted*, `WRK.RUL.08` reattempts them (the attempt does not count).
- **Run already at `returned_to_depot`:** `abort` not available.
- **03 Logistics:** the abort reason list adds *driver unreachable* (used by `LOG.RUL.13`;
  wl-delivery §4 vertical mapping row 03).

#### Must not
- Must not count *run aborted* against max attempts.
- Must not close the run here; `close` stays a separate system step (BL-DLV-11).

#### Done when
- [ ] Given a `dispatched` run with 6 open stops, of which 2 were handed over, when the dispatcher aborts with reason *breakdown* and ticks the ticket, then 4 deliveries are `attempted` with *run aborted*, their attempt counts are unchanged, recipients are messaged, a breakdown ticket exists for the vehicle, and `RunAborted` lists the 4 deliveries.
- [ ] The run is `returned_to_depot` and a cash handover is open when AED > 0 was collected.
- [ ] The driver can abort only with a reason.

#### Out of scope
`hand_over` (BL-DLV-09); perishable window (BL-DLV-10); run `close` (BL-DLV-11).

---

### BL-DLV-09 — wl-delivery: hand open stops over to another driver's run

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 03 · **Size:** m
**Depends on:** BL-DLV-01 · Epic wl-payments: cash custody (driver-to-driver handover) · **Blocks:** BL-DLV-08, BL-DLV-10, BL-LOG-26
**Review items:** X-12

#### Goal
After a breakdown, accident or for load balancing, the dispatcher moves chosen open stops
from one run to another driver's run. The receiving driver confirms each parcel and counts
the cash, and the cash custody moves with them, so that every parcel and every dirham is
always with one named driver.

#### Files to read
- `docs/modules/wl-delivery.md` §3 *Run* (hand-overs), §4.2 (rows `hand_over`, `return_to_depot`), §5.3 step 1, §6 event `RunHandedOver`, §7 settings
- `docs/modules/wl-payments.md` cash custody (`FIN.CSH.01`)

#### Terms
- **Giving run / receiving run:** the run the stops leave and the run they join.
- **Hand-over record:** on the giving run: to run, deliveries, cash amount, confirmed by the receiving driver, time, location.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `hand_over` | dispatched | dispatched | `runs.dispatch` | receiving run is `dispatched` or `planned` with another driver; the receiving driver passes `qualification_valid` and the receiving vehicle `document_valid` today (same kinds as `run.dispatch`); perishable stops only when they can still arrive within the hand-over window (§7); the receiving driver confirms each parcel (scan or tick) and counts the cash; reason (breakdown, accident, driver unwell, load balancing) | chosen open deliveries move to the receiving run and stay `out_for_delivery` (`DeliveryAssigned`); the giving driver's cash handover is closed, counted by the receiving driver, and the amount moves to the receiving driver's custody (`FIN.CSH.01`); both drivers and the dispatcher see the custody record; event `RunHandedOver` |
| `return_to_depot` | dispatched | returned_to_depot | `runs.drive` (the driver) | every stop has an outcome or was handed over | open deliveries `fail` with *not attempted*; cash handover opened in `wl-payments` if cash was collected |

Settings: hand-over between drivers: receiving driver scans each parcel; counts the cash: on; on.
Event `RunHandedOver`: run, receiving run, both drivers, deliveries, cash amount, reason.

#### Edge cases decided
- **Receiving run `planned`:** the moved deliveries stay `out_for_delivery` (they are physically out). The receiving run cannot be `cancel`led while it holds handed-over deliveries; its `dispatch` skips them (already out).
- **Cash count differs from the giving driver's custody:** the receiving driver enters the counted amount; the custody moves with the counted amount and the difference stays as a shortage on the giving driver (wl-payments shortage flow).
- **Settings off:** scan off → tick per parcel; count off → the custody amount moves as recorded.
- **Partial hand-over:** allowed; stops not chosen stay on the giving run.
- **All stops moved:** the giving driver may then `return_to_depot` (guard counts handed-over stops as done) or the run is aborted (BL-DLV-08).
- **Concurrency:** a stop delivered on the giving driver's phone while the hand-over is being confirmed is skipped; the hand-over includes only stops still open at confirmation.

#### Must not
- Must not move cash custody without the receiving driver's confirmation.
- Must not hand over to the same driver or to a run whose driver fails `qualification_valid`.

#### Done when
- [ ] Given run R1 (driver A, AED 400 collected, 5 open stops) and run R2 (driver B, `dispatched`, documents valid), when the dispatcher hands over 3 stops with reason *breakdown* and B scans 3 parcels and counts AED 400, then the 3 deliveries are on R2 and `out_for_delivery`, B's custody is +400, A's handover is closed, and `RunHandedOver` is emitted.
- [ ] Given B's rider permit expired yesterday, the hand-over is refused.
- [ ] Given B counts AED 380, B's custody is +380 and A has a shortage of AED 20.
- [ ] Given R1 with all stops delivered or handed over, A can `return_to_depot`.

#### Out of scope
The perishable window check (BL-DLV-10). Shortage handling (Epic wl-payments).

---

### BL-DLV-10 — wl-delivery: limit perishable hand-overs to the hand-over window

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-DLV-07 · BL-DLV-09 · **Blocks:** BL-FNB-01, BL-FNB-16
**Review items:** X-12, 06-N9-b

#### Goal
Cooked food handed to another rider must still arrive within the brand's delivery promise.
A perishable stop may be handed over only when the receiving run can still deliver it within
the hand-over window; otherwise it fails with *run aborted* at once and the perishable policy
applies.

#### Files to read
- `docs/modules/wl-delivery.md` §4.2 row `hand_over`, §5.3 step 2, §7 setting "Perishable hand-over window"

#### Terms
- **Hand-over window:** the latest arrival a receiving run may promise; default the source's delivery promise (06: the brand's delivery promise).
- **Expected arrival:** the time the receiving run would reach the stop, entered by the dispatcher per perishable stop at the hand-over (no route optimisation in the MVP; a `RunPlanner` with estimates may fill it Later). The run's estimated duration and sequence are only shown as a hint.

#### Spec excerpt (authoritative for this issue)

`hand_over` guard (part): perishable stops only when they can still arrive within the hand-over window (§7).

§5.3 step 2: Perishable stops are handed over to another run only when they can still reach
the recipient within the perishable hand-over window (§7; default the source's delivery
promise, 06: the brand's promise); otherwise they fail with *run aborted* at once.

§5.3 step 2 (cont.): With no route optimisation in the MVP, the dispatcher enters the
expected arrival of each perishable stop at the hand-over, and a stop whose expected arrival
is after the window end is refused.

Setting: perishable hand-over window after a breakdown or abort: latest arrival a receiving
run may promise: the source's delivery promise (06: the brand's delivery promise).

#### Edge cases decided
- **How the arrival is estimated:** the dispatcher enters the expected arrival per perishable stop when handing over (§5.3 step 2); the app refuses the stop when that time is after the window end (order placed time + promise). Arrival exactly at the window end is accepted.
- **Window end** is computed in Asia/Dubai time from the source's placed time.
- **Refused perishable stop:** it stays on the giving run; on abort it fails with *run aborted* and `WRK.RUL.16` applies.

#### Must not
- Must not let a perishable stop be handed over without an expected arrival.

#### Done when
- [ ] Given an F&B delivery placed 19:00 with a 45-min promise, when it is handed over with expected arrival 19:40, it is accepted.
- [ ] With expected arrival 19:50, the stop is refused and stays on the giving run.
- [ ] Non-perishable stops are handed over without an expected arrival.

#### Out of scope
The hand-over itself (BL-DLV-09); the perishable policy (BL-DLV-07).

---

### BL-DLV-11 — wl-delivery: close a run with refused and missing goods accounted for

**Labels:** `module:wl-delivery` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 03 · **Size:** s
**Depends on:** BL-DLV-02 · BL-DLV-06 · **Blocks:** BL-STK-22, BL-LOG-07
**Review items:** X-11

#### Goal
A run closes only when its cash is counted and every undelivered or refused item is received
back at the depot, or reported missing. A missing item creates a task for the dispatcher and
is then traced by `WRK.RUL.17` until it is found or marked lost.

#### Files to read
- `docs/modules/wl-delivery.md` §3 *Delivery line*, §4.1 paragraph "Partial acceptance", §4.2 (states, row `close`), §5.4, §6 event `DeliveryRefusedReceived`

#### Terms
- **Received back:** the goods are scanned or ticked at the depot at run close. Every refused quantity received back goes to the source with its refusal reason, whatever the reason; for a dispatch note it goes into quarantine (BL-STK-22; the source module posts the stock movement).
- **Reported missing:** the depot user marks an expected item as not on the vehicle.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `closed` | success | yes | Every stop has an outcome, cash counted, undelivered goods back in or reported missing |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `close` | returned_to_depot | closed | system | cash handover counted (or no cash); undelivered and refused goods received back, or reported missing (task to the dispatcher; an undelivered delivery is traced by `WRK.RUL.17`) | received and missing refused quantities recorded on the delivery lines; event `DeliveryRefusedReceived` per delivery with refused lines; event `RunClosed` |

#### Edge cases decided
- **Check list at the depot:** the app lists every undelivered delivery and every refused line on the run; each must be ticked *received* or *missing*.
- **Missing:** the delivery stays `attempted`; its `WRK.RUL.17` clock does not reset from the missing report.
- **Received refused lines:** the receipt is recorded on the delivery line (quantity received back and missing, time and user), and `DeliveryRefusedReceived` is emitted per delivery with refused lines at `close`. The stock movement belongs to the source (BL-STK-22: `return_in` to quarantine, loss for missing).
- **Partial received:** a refused line may be part received and part missing; both quantities are recorded.

#### Must not
- Must not close the run while any item is neither received nor reported missing.

#### Done when
- [ ] Given a run back at the depot with 2 undelivered deliveries and 1 refused line (qty 3), when the depot user ticks all received and the cash handover is counted, the run closes, `DeliveryRefusedReceived` (3 received, 0 missing) and `RunClosed` are emitted.
- [ ] Given the refused line ticked 2 received and 1 missing, `DeliveryRefusedReceived` carries 2 and 1; the delivery stays `delivered`.
- [ ] Given one of the undelivered deliveries is reported missing, the run closes, the dispatcher gets a task, and the delivery stays `attempted`.
- [ ] Given the cash handover not yet counted, the run does not close.

#### Out of scope
Lost marking (BL-DLV-06); quarantine movements and the loss for missing goods (BL-STK-22).
