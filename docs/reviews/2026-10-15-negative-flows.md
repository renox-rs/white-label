# Review: Negative-Flow Coverage

> Status: closed · Date: 2026-10-15 · Closed: 2026-10-15 · Scope: the ten accepted verticals and the shared modules
> they use

The documents describe the happy path well. This review asks the other question for every
value stream: **what happens when it goes wrong?** Each finding is a gap to close in the
vertical or module document; nothing here is a decision yet. **All items were fixed on 2026-10-15; see §8.**

## 1. Method

For each vertical, every value stream (§4) and state machine (§7) was checked against 13 kinds
of negative flow. A flow is **covered** when the documents state the trigger, the transition or
state, who is told, and what happens to money, stock and documents; **partial** when it is
mentioned but the outcome is unclear; **missing** when it is absent.

| Code | Negative flow |
|---|---|
| N1 | Approval rejected or returned |
| N2 | Cancellation or withdrawal, before or after work started |
| N3 | Payment failure: bounced cheque, failed card, partial, over- or under-payment, refund, chargeback, bad debt |
| N4 | Dispute or complaint |
| N5 | Missed deadline, no-show, SLA breach, expiry |
| N6 | Data correction: void, reversal, amendment, duplicates, import errors |
| N7 | External or integration failure, including no connectivity |
| N8 | Compliance block: expired licence or permit, sanctions hit, locked period |
| N9 | People or partner failure: absconding, no-show, vendor failure, unreachable customer |
| N10 | Physical goods: shortage, damage, loss, recall, return |
| N11 | Fraud, abuse and security |
| N12 | Early or exceptional end: termination, insolvency, death, offboarding |
| N13 | Concurrency and edge cases: double booking, overlaps, period boundaries, partial quantities |

Impact: **HIGH** = money, legal or regulatory exposure, data loss or customer harm in an MVP
flow; **MEDIUM** = operational confusion in an MVP flow; **LOW** = Later scope or rare.

## 2. Results

| Vertical | Covered | Partial | Missing | HIGH |
|---|---|---|---|---|
| 01 Property | 40 | 19 | 17 | 14 |
| 02 Construction | 28 | 22 | 9 | 10 |
| 03 Logistics | 26 | 17 | 14 | 10 |
| 04 Accounting | 19 | 25 | 10 | 9 |
| 05 Clinics | 22 | 12 | 10 | 8 |
| 06 F&B | 30 | 19 | 12 | 8 |
| 07 Manufacturing | 30 | 19 | 8 | 6 |
| 08 Auto | 24 | 17 | 12 | 9 |
| 09 Manpower | 38 | 16 | 18 | 17 |
| 10 EdTech | 18 | 22 | 7 | 4 |
| **Total** | **275** | **188** | **117** | **95** |

About half of the negative flows found (275 of 580) are fully specified. Approvals, simple
cancellations, reminders, bounced-cheque notices, failed delivery attempts, message fallback,
import validation and locked VAT periods are well covered everywhere.

The gaps cluster in five places:

1. **Unwinding money:** bad debt, disputes, payment reversals, failed refunds, chargebacks.
2. **Terminal states with no way out:** records that end too early or cannot be corrected once
   money or a regulator is involved.
3. **Expiry that only reminds:** expired permits, licences and approvals send a reminder but do
   not block the action they should block.
4. **Physical exceptions:** rejection at receipt, returns to suppliers, recalls, loss, partial
   acceptance.
5. **Stopping midway:** cancelling or terminating after work has started.

## 3. Cross-cutting gaps in shared modules

Fixing these once closes gaps in several verticals. Ordered by reach.

| # | Module | Gap | Verticals | Impact |
|---|---|---|---|---|
| X-01 | wl-invoicing | Invoice has no `write_off` (bad debt) transition with approval; the VAT bad-debt relief adjustment (Art. 64) has nothing that produces it | all | HIGH |
| X-02 | wl-invoicing | No dispute flag that pauses reminders (`FIN.RUL.01`) and links a ticket; resolution by credit note or upheld | all | MEDIUM |
| X-03 | wl-invoicing / wl-payments | Payment `reverse` reopens invoices, but Invoice `paid` is terminal: no `unallocate` transition | all | HIGH |
| X-04 | wl-invoicing | No e-invoicing transmission lifecycle (submitted, accepted, rejected by the ASP, resubmitted), mandatory for B2B from 2027 | all B2B | HIGH |
| X-05 | wl-payments | Cheque `bounced` leads only to `replaced`: no re-presentation, partial honour, legal case or write-off; no bounce fee; no link to credit hold | 01, 02, 04, 07, 08 | HIGH |
| X-06 | wl-payments | Refund has no `failed` state; `PaymentFailed`, pending timeouts and `PaymentReversed` (chargebacks) have no consumers outside subscriptions | 05, 06, 10 | HIGH |
| X-07 | wl-payments | Cash custody does not cover clinic front desks, EdTech desks, cashier till shortages or cheques collected in the field | 05, 06, 08, 10 | MEDIUM |
| X-08 | wl-inventory / wl-invoicing | No rejected quantity at goods receipt, no return-to-supplier, no supplier debit note against the bill; PO cannot be revised after issue | 02, 06, 07, 08 | HIGH |
| X-09 | wl-inventory | No recall workflow: lots have no forward trace to customers, patients or orders; no mass hold | 05, 06, 07, 08 | HIGH |
| X-10 | wl-inventory | Negative stock policy is still open; landed cost cannot be adjusted after allocation | 05, 06, 08 | MEDIUM |
| X-11 | wl-delivery | Delivery is all or nothing: no partial acceptance per line at proof of delivery; no `lost` state; no reversal of a delivery marked by mistake (and `WRK.ACL.06` contradicts §10 "corrections are audited") | 03, 06, 07, 08 | HIGH |
| X-12 | wl-delivery | No perishable policy (no reattempt, discard) and the 06 mapping ignores failed attempts; a run cannot be aborted or handed over mid-route | 03, 06 | HIGH |
| X-13 | wl-documents / wl-hr | Expired documents only remind. There is no standard "expired document blocks transition X" policy: driver permits, tutor clearance, course approvals, practitioner licences with future bookings, worker visas during deployment, vendor approvals, KYC documents | 01, 03, 04, 05, 09, 10 | HIGH |
| X-14 | wl-sales | No order `amend`; no cancel after fulfilment started; no credit re-check at dispatch; no line-level cancel; no refund after fulfilment for food or services; external order number not unique per channel | 06, 07, 08 | HIGH |
| X-15 | wl-hr | Payroll run: no WPS rejection or partial failure, no correction run after `paid`, no `return` by the approver, held lines never released. Employee: no exit from `onboarding`, no notice withdrawal, no absconding or death path. Settlement: no dispute, no negative net, 14-day rule not registered. No change control on pay accounts | 02, 09 (and 10 employed tutors) | HIGH |
| X-16 | wl-attendance | Locked timesheets cannot be corrected; no flags for overtime above the cap, hours inside the midday break when a supervisor marks crews, or an expired work permit; no overlap guard across projects | 02, 09 | HIGH |
| X-17 | wl-scheduling | Bookings in `checked_in` or `in_progress` cannot be cancelled; a staff-initiated cancellation has no deposit refund rule; licence expiry does not raise `BookingConflict` for future bookings | 05, 08, 10 | HIGH |
| X-18 | wl-parties | Party `block` does not say what happens to active agreements (engagement, subscription, enrolment), and the blocked status is visible to all staff (tipping-off risk) | 04, all | HIGH |
| X-19 | wl-kernel / WORKFLOW | No platform pattern for: voiding a record entered in error; what state a record keeps after its approval is rejected; an integration being down (Deliverect, gateway, regulator portal); late entry after an outage on online-only screens; compliance-critical rule failures alerting the business owner, not only the admin | all | MEDIUM |
| X-20 | wl-notify | `give_up` notifies the requester module, but no module says how it reacts | 04, all | MEDIUM |

## 4. Contradictions and defects found

These are errors in the current text, not only missing coverage.

| # | Where | Problem | Fix |
|---|---|---|---|
| D-01 | 01 Lease | `expired` is terminal, but the doc cites Law 26 of 2007 Art. 6: a tenant who stays renews automatically on the same terms | Make `expired` non-terminal or add automatic renewal on holding over; keep billing |
| D-02 | 01 Lease | `submit` requires the unit to be available or reserved, so a renewal version for the same tenant cannot be submitted | Exempt renewal versions |
| D-03 | 01 `PRP.RUL.01` | Flags the renewal at 90 days, which leaves no lead time for the 90-day notice of changed terms | Flag earlier (e.g. 120 days) and guard rent changes without timely notice |
| D-04 | 02 §9 | Retention kept by the client is "reversed by credit note", but retention is not invoiced until release | Define retention states and the VAT document for each |
| D-05 | 02 `CON.RUL.03` | A new draft IPA is created monthly even when the previous one is not certified; "previous certified" is then undefined | Use the last certified IPA, or warn while one is pending |
| D-06 | 08 `AUT.RUL.08` | Uses state `supplier_claimed`; the machine names it `raised` | Rename |
| D-07 | 08 `AUT.RUL.07` | Fires on ticket `resolved`, but supplier claim `raise` requires the ticket `closed` | Align trigger and guard |
| D-08 | 08 Insurance claim | The police report is a guard only on `book_survey`; `approve` and `submit_estimate` skip it (repair permit rule) | Guard on `approve` or job start |
| D-09 | 04 `ACC.RUL.04` | Names the removed ticketing transition `miss` | Use `mark_overdue` |
| D-10 | 04 Return `correct` | Always demands a voluntary disclosure reference; errors of AED 10,000 or less go into the next return | Require it only above the threshold |
| D-11 | 03 `LOG.ACL.09` | Lets a dispatcher cancel after pickup, but `cancel` runs only from `created` | Allow cancel to `returning`, or fix the rule text |
| D-12 | 06 vs wl-payments | 06 lists `FIN.CSH.01` (rider cash) as MVP; the wl-payments cash handover mapping still treats 06 as a candidate | Align the mapping |
| D-13 | wl-ticketing vs 01 | On service contract end, 01 cancels or reassigns open visits and flags the building; the module keeps them open with no flag | Align, and move the compliance flag into the module |
| D-14 | 01 Ticket | A chargeable job's invoice is drafted on `resolve`, before the tenant confirms | Invoice on `confirm`, or hold the draft |

## 5. HIGH items per vertical

IDs follow `NN-Nx-letter`. Cross-cutting items (§3) are not repeated here unless the vertical
needs a specific rule on top.

### 01 Property

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 01-N2-a | Prospect backs out after paying the deposit or handing over PDCs | Cancel-after-payment path: deposit refunded or forfeited per policy, PDCs returned, approval |
| 01-N3-a | Bounced cheque never replaced | X-05 plus bounce fee from the charge catalogue |
| 01-N3-b | Arrears grow after the last reminder | Escalation: legal notice, case task, termination for non-payment |
| 01-N3-e | Deductions exceed the deposit | Balance invoiced to the tenant, then the arrears flow |
| 01-N3-g | Owner balance negative (vacancy, AMC) | Carry forward or invoice the owner; payout blocked |
| 01-N4-c | Tenant disputes deductions shown in the portal | Dispute step, response window, escalation with evidence |
| 01-N5-b | 90-day notice for changed terms | D-03 |
| 01-N6-a | Mid-term change to an active lease | `amend` transition: new version, charges reissued, cheques relinked, Ejari update |
| 01-N8-b | Vendor's Civil Defence approval expires mid-contract | X-13: flag contract and building, block visit completion |
| 01-N9-a | Tenant absconds or abandons the unit | Forced termination with approval, legal task, deposit forfeit, arrears |
| 01-N12-a | Early termination by the tenant | Penalty charge type; credit notes or refunds for periods after move-out |
| 01-N12-c | Management agreement ends or unit sold mid-lease | Agreement lifecycle with handover of deposits, charges, cheques and Ejari |
| 01-N13-a | Tenant holds over | D-01 |

### 02 Construction

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 02-N1-a | Client rejects a variation already executed on site instruction | Disputed or claim path; cost kept on an unrecovered code; resubmit as a claim |
| 02-N2-a | Project terminated mid-way (termination, main contractor insolvency) | `terminate`: final account, retention and guarantees, materials on site, open orders, crews, portal |
| 02-N4-a | Back-charges, contra-charges or liquidated damages deducted from a certificate | Typed deduction lines on certification; VAT via credit note |
| 02-N4-b | Client keeps or disputes retention | D-04 |
| 02-N5-d | Worker with an expired visa or permit marked present | X-13 / X-16 |
| 02-N8-a | Overtime above 2 h/day or work in the midday break when crews are marked | X-16 flags on timesheet hours |
| 02-N10-a | Damaged or wrong materials delivered | X-08 |
| 02-N12-b | Performance bond or advance guarantee called | Guarantee lifecycle with `called`: amount, project cost, notices |
| 02-N13-a | Overlapping draft IPAs | D-05 |

### 03 Logistics

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 03-N10-a | Shipment lost | `lost` state set after N days; client notified; claim ticket; excluded from billing; credit note |
| 03-N11-a | Delivery marked by mistake or fraud; recipient denies it | X-11 reversal with approval; COD and billing frozen; driver liability |
| 03-N8-a | Run dispatched with an expired licence or rider permit | X-13 guard on run `assign`/`dispatch` |
| 03-N3-a | Customer pays less than the COD amount or refuses to pay | Difference on the remittance line, who absorbs it, client notice; or a forced failed attempt |
| 03-N9-a | Driver absconds with cash, or never hands it over | Escalation: full shortage, HR case, write-off approval; client still remitted |
| 03-N6-c | Remittance wrong after payment, transfer returned, statement disputed | Reversal or adjustment line in the next remittance, with approval and notice |
| 03-N7-c | Customs hold, inspection, declaration rejected, platform down | Freight job `on_hold` with a reason; notify client and documentation officer |
| 03-N5-a | Demurrage, detention or port storage | Daily charges on the job, liability, recharge, free-time alert |

### 04 Accounting

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 04-N5-c | Statutory deadlines inside a suspension | On `suspend`, send the client the deadlines in the window, with acknowledgement of responsibility |
| 04-N6-a | Tax profile changes (stagger, financial year, registration) | Regenerate future calendar tasks with manager approval |
| 04-N7-b | Sanctions list download fails; screening runs on stale lists | List freshness check; alert the MLRO after N hours; last update shown |
| 04-N8-a | Confirmed sanctions match on an active client | Engagement suspended with a neutral reason, retainer paused, AML case with 24-hour freeze and report tasks |
| 04-N8-b | MLRO exits an approved client | CDD `exit` → engagement `end` with a neutral reason |
| 04-N8-d | Return filed by someone who is not a valid registered tax agent | Tax agent registration on staff (`HRM.QUA.01`); `file` guard |
| 04-N11-a | Tipping off through visible side effects (blocked party, refused invoices) | X-18: neutral reason codes and restricted fields |

### 05 Clinics

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 05-N3-c | Payer overpays or recoups in a later remittance | Payer bill `record_recoupment` and payer credit balance, with approval |
| 05-N5-a | First-submission window missed | Write-off or late-close from draft/ready with `FIN.ACL.14`, or transfer to the patient where allowed |
| 05-N5-b | Prior authorisation expires or is used up before the visit | Re-flag appointments; check at check-in; self-pay consent |
| 05-N5-c | Insurance card expired or eligibility negative at check-in | Outcomes: self-pay with consent, re-split, or reschedule |
| 05-N7-a | Claim rejected technically at submission | `submission_rejected` back to draft with error codes; not a resubmission |
| 05-N8-a | Practitioner licence expires with future appointments booked | X-17: `BookingConflict`, reassign or reschedule, notify patients |
| 05-N10-a | Recall of a consumable or injectable lot | X-09 with lot required on issue to a visit |
| 05-N11-a | Front-desk cash shortage, no daily reconciliation | X-07 till session or cash handover per receptionist |

### 06 F&B

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 06-N9-b | Own-rider delivery fails (unreachable, wrong address, refuses COD) | X-12: order `fail_delivery`, one attempt for food, refund or COD rule, wastage |
| 06-N3-a | Online payment pending, failed or late | Order placed only after payment confirmation; late confirmation on a rejected order refunds automatically |
| 06-N4-b | Complaint after delivery: cold, missing, wrong | X-14 refund after completion with reason and responsible party; complaint category |
| 06-N4-a | Aggregator refunds the customer and deducts it from the payout | Payout refund line updates the order and the refund KPI |
| 06-N10-b | Goods fail the temperature or quality check at receiving | X-08 |
| 06-N10-c | Supplier recall or contaminated ingredient lot | X-09 through production batches, transfers and orders |
| 06-N10-e | Fridge reads out of range: what happens to the food | Hold the unit's lots; chef decides discard or release |
| 06-N7-c | Internet or power outage at the outlet | X-19 outage procedure: pause channels, paper tickets, late entry |

### 07 Manufacturing

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 07-N2-a | Customer cancels after the work order started | Finish to stock, scrap WIP or cancellation fee, with approval |
| 07-N3-b | Customer credit worsens between confirmation and dispatch | X-14 credit re-check at dispatch |
| 07-N4-a | Customer accepts only part of a delivery | X-11 partial acceptance; invoice only accepted quantities |
| 07-N4-c | Product recall | X-09 recall action using lot genealogy |
| 07-N10-a | Return to supplier after failed incoming inspection | X-08 (the NCR "supplier return note" is undefined today) |

### 08 Auto

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 08-N2-a | Customer stops the job after work started | Job card `cancel` with reason: bill work done, return unused parts |
| 08-N2-b | Customer withdraws the insurance claim after approval | Claim `cancel` from approved; bill work done; job card follows |
| 08-N3-a | Post-dated cheque bounces | X-05 with credit hold for the customer or group |
| 08-N5-a | Parts not supplied within the legal days; substitute vehicle needed | Timer in `awaiting_parts`, customer notice, substitute record |
| 08-N5-c | Export exit evidence missing at 90 days | Correction: credit note and 5% invoice, recharge |
| 08-N8-a | Repair without a police report | D-08 |
| 08-N10-a | Customer vehicle damaged in the workshop or on a test drive | Incident ticket linked to the job; liability decision |
| 08-N12-a | Total loss declared after approval | Total-loss end from approved; bill strip-down, labour and storage |

### 09 Manpower

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 09-N3-a | Client stops paying while workers stay deployed | Credit hold blocks new deployments; notify account manager and MD |
| 09-N12-a | Client contract expires or ends early with workers on site | Contract lifecycle; deployments end; workers to the bench |
| 09-N12-b | New visa case cancelled while the employee is onboarding | X-15 exit from `onboarding` |
| 09-N4-a | Client disputes hours after invoicing | X-16 correction: credit note and adjustment timesheet, payroll effect |
| 09-N5-b | Client never approves the timesheet | Escalation ladder; deemed approval per contract |
| 09-N5-c | Timesheets not approved at payroll cut-off | Salary paid from attendance regardless; later reconciliation |
| 09-N6-b | Payroll error after `paid` | X-15 supplementary run; recovery within Art. 25 limits |
| 09-N7-b | WPS file rejected, fully or partly | X-15 rejection path; regenerate the SIF for failed lines |
| 09-N3-d | Held salary lines released later | Off-cycle run or carry-over with the WPS deadline check |
| 09-N11-b | Pay account changed just before payroll | X-15 change control and worker SMS notice |
| 09-N8-b | Visa, insurance, SIRA or health card expires during deployment | X-13 deployment `suspend`: no attendance, no billing |
| 09-N8-c | Renewal visa rejected or cancelled | Employee notice or cancellation case with a deadline |
| 09-N4-b | Worker disputes the settlement or files a MoHRE complaint | Settlement `dispute`; labour case ticket |
| 09-N5-d | Settlement not paid within 14 days | Register the rule in 09 §7.2 |
| 09-N12-c | Absconding worker cannot exit | Exit through the MoHRE report without the standard guards |
| 09-N7-a | E-invoice rejected by the ASP | X-04 |

### 10 EdTech

| ID | Negative flow | What the documents should specify |
|---|---|---|
| 10-N3-f | Withdrawal refund issues no credit note; future charges not cancelled | `withdraw` effects: credit note, cancel scheduled charges and subscription, invoice amount owed |
| 10-N8-a | Course runs or certificate issued with an expired regulator approval or permit | X-13 guard on batch `open` and certificate `issue` |
| 10-N8-c | Tutor with expired permit or police clearance assigned to minors | X-13 `HRM.QUA.01` guard on batch and session assignment |
| 10-N12-b | A running batch must stop | Batch `terminate`: pro-rata refunds or transfers, notices |

## 6. MEDIUM and LOW items

Listed in short form. Each is re-checked against the source document when its vertical is
fixed. These are the MEDIUM items worth doing together with the HIGH ones, because they sit
on the same transitions:

- **01:** pending-signature timeout; tenant withdraws notice; Ejari cancellation at move-out;
  owner statement lifecycle and disputes; vendor no-show reassignment; duplicate vendor bills.
- **02:** omission variations; variation response reminder; extension-of-time notices;
  guarantee lifecycle; worker absconding; PO revision; site stock counts.
- **03:** cancel after pickup (D-11); damaged parcel outcome; address correction; duplicate
  bookings; partial pickup; COD netting against invoices; re-weigh surcharge; late costs on
  closed jobs.
- **04:** engagement letter never signed; ending mid-period; suspension and retainer; e-mail
  bounces; KPI pack lifecycle; withdraw approval before filing.
- **05:** walk-out after check-in; clinic-side cancellation and deposit refund; billing
  disputes; void a claim; practitioner sick on the day; negative stock.
- **06:** customer cancel before acceptance; aggregator rider never arrives; item runs out
  after acceptance; duplicate external orders; payout file revert; business-day cut-off.
- **07:** order amendment; work order aborted after issue; returned delivery outcome;
  duplicate PO upload; export 90-day alert.
- **08:** returned delivery outcome; fleet contract early end; estimate expiry and abandoned
  vehicles; landed cost adjustment; back-orders; cheques in the field handover.
- **09:** client rejects a deployed worker; rate card versions; overlap guards; visa step
  deadlines; overtime cap flags; notice withdrawal; no cover available.
- **10:** enrolment payment timeout; freeze linked to membership pause; absence consumption
  policy; attendance correction after a session; certificate reissue; capacity under
  concurrency; tutor reassignment.

## 7. Next step

Fix in this order, so each change lands once:

1. **Cross-cutting module patterns** X-01 to X-20, in the module documents and
   [WORKFLOW.md](../WORKFLOW.md) (a "negative flows" section with the platform patterns).
2. **Contradictions** D-01 to D-14.
3. **HIGH items** per vertical that remain after step 1.
4. **MEDIUM items** listed in §6.

Each fix updates the state machine, rule or automation in the source document and marks the
item here as closed.

## 8. Closure (2026-10-15)

Every X-, D-, HIGH and MEDIUM item above is fixed in the source documents. Where a fix rests on a
legal or tax fact that public sources do not settle, the document says so with `⚠ open:` and
the question is tracked in [OPEN-ITEMS.md](../OPEN-ITEMS.md).

**Platform patterns.** [WORKFLOW.md](../WORKFLOW.md) §8 now states the negative-flow patterns
every state machine follows (void, approval returned, integration down, late entry, expired
document blocks, cancel after work started, dispute flag, party blocked, message gave up, owner
alert) and a checklist. Each vertical answers the checklist in §7.1.

**Cross-cutting items, where they were fixed**

| # | Fixed in | How |
|---|---|---|
| X-01 | wl-invoicing | Invoice `write_off` → `written_off` with `FIN.ACL.15`; bad-debt relief proposed by `FIN.RUL.14`; `recover` |
| X-02 | wl-invoicing | `disputed` flag with `dispute` / `resolve_dispute`; reminders paused; `FIN.RUL.15` |
| X-03 | wl-payments, wl-invoicing | `PaymentReversed` with allocations; Invoice and supplier bill `unallocate`; `paid` no longer terminal; credit note `reopen` on `RefundFailed` |
| X-04 | wl-invoicing | E-invoice transmission lifecycle; `FIN.RUL.16` |
| X-05 | wl-payments, wl-sales | Cheque `represent`, `settle_part`, `refer_to_legal`, `write_off`; bounce fee charge type; `FIN.RUL.18`; credit hold `SAL.RUL.05` |
| X-06 | wl-payments | Refund `failed`; pending timeout `FIN.RUL.19`; `late_confirm`; chargebacks with `FIN.ACL.18`, `FIN.RUL.20`; `FIN.RUL.21` |
| X-07 | wl-payments | Cash handover per person or till session; field cheques (`FIN.ACL.20`); shortage recovery (`FIN.ACL.19`) |
| X-08 | wl-inventory, wl-invoicing | Accepted and rejected quantity at receipt; Return to supplier; supplier debit note (`FIN.RUL.17`); PO `revise` |
| X-09 | wl-inventory | Recall with holds, forward trace (`TracePartyResolver`), notices and outcomes |
| X-10 | wl-inventory | Negative stock policy per location (default `block` with pending issues); landed cost `adjust` |
| X-11 | wl-delivery | Proof of delivery per line; `under_review`, `lost`, `reverse_delivery`; `WRK.ACL.06` corrected; `WRK.ACL.09`, `WRK.ACL.10`, `WRK.RUL.17` |
| X-12 | wl-delivery | `perishable` policy (`WRK.RUL.16`), `discarded`; run `abort` and `hand_over` |
| X-13 | wl-documents, wl-hr | `document_valid` and `qualification_valid` guards; document kind `blocks`; `DOC.RUL.02`, `DOC.RUL.03`, `DOC.ACL.02`, `HRM.RUL.10` |
| X-14 | wl-sales | Order `amend`, `cancel_line`, cancel with outcome per line, `credit_recheck`, *no goods* return, unique external number, `awaiting_payment`; `SAL.ACL.05`, `SAL.ACL.06`, `SAL.RUL.06` |
| X-15 | wl-hr | Payroll `return`, WPS rejection, supplementary and off-cycle runs; employee exits; settlement `dispute`; pay account control; `HRM.RUL.09`, `HRM.RUL.11`, `HRM.RUL.12`, `HRM.ACL.05`–`07` |
| X-16 | wl-attendance | Adjustment timesheets; flags for overtime cap, midday break, invalid permit, overlap; deemed approval; payroll from attendance at cut-off; `HRM.RUL.13`–`15`, `HRM.ACL.08`, `HRM.ACL.09` |
| X-17 | wl-scheduling | Cancel after check-in; business-initiated cancel refunds the deposit; bulk move; eligibility conflicts; `SCH.RUL.05`, `SCH.RUL.06`, `SCH.ACL.05`, `SCH.ACL.06` |
| X-18 | wl-parties | Block effects per agreement; neutral reason; `PTY.ACL.02`, `PTY.ACL.03`; freeze and report tasks; `PTY.RUL.02` |
| X-19 | wl-kernel, WORKFLOW §8 | Void, approval `returned`, integration connections, late entry, owner alert; `PLT.ACL.09`, `PLT.ACL.10`, `PLT.RUL.04`–`06` |
| X-20 | wl-notify | `MessageGaveUp` reactions per notification type; `NTF.RUL.01` |

**Contradictions:** D-01, D-02, D-03, D-13 (with wl-ticketing `WRK.RUL.18`) and D-14 in 01;
D-04 and D-05 in 02; D-11 in 03; D-09 and D-10 in 04; D-12 in 06 and wl-payments; D-06, D-07
and D-08 in 08.

**HIGH and MEDIUM items:** fixed in each vertical's §3.3, §4 ("When it goes wrong" tables),
§7.1 and §7.2. New vertical rules are numbered in each vertical. These items are only partly
closed, pending an answer tracked in OPEN-ITEMS:

| Item | What is still open |
|---|---|
| 07-N4-c | Whether a recall must be notified to an authority |
| 08-N5-a | Which legal parts supply period applies to which part |
| 08-N5-c | Whether the zero rate can be restored when export evidence arrives late |
| 08 (MEDIUM) abandoned vehicles | What a workshop may lawfully do with an uncollected vehicle |
