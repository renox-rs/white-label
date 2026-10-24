# Workflow

> Status: specification; the engine is built (Renox 1.2.0 state machines, used by the F1-F10 modules) · Last updated: 2026-10-24

How work moves through the system: statuses, approvals, automations and integrations. Each
vertical lists only its own state machines and automations
([templates/vertical.md](templates/vertical.md) §7).

## 1. Decision: no BPM engine, no embedded n8n

| Option | Why not |
|---|---|
| BPM engine (Camunda/BPMN, Temporal, Flowable) | A separate runtime per client breaks "one binary per client". SME clients will not model BPMN themselves, so we would end up configuring it for them, which costs more than writing it in code |
| n8n (or similar) embedded | A separate Node service and database per client, under the Sustainable Use License, which allows use only for the user's own internal business purposes, so offering it to clients as part of our service needs a separate commercial licence from n8n ✔ [S1]. Business logic placed there escapes our tests, audit and access rules |
| Fully configurable workflows | Every combination a client can configure is one we must test. Invoice statuses that can be freely changed can break VAT and financial reports |

Instead, the platform has **four small building blocks**. Business logic that touches money,
tax or contracts stays in tested code. Per-client variation is configuration. Client-specific
integrations live **outside** the platform, connected by outbound webhooks and the API (n8n,
Zapier and Make are fine there).

**Revisit when** three or more clients ask for cross-department approval flows that they want
to design themselves.

## 2. The four building blocks

| # | Block | What it does | Defined by | Client can configure |
|---|---|---|---|---|
| 1 | **State machine** | The statuses of an entity, the allowed transitions, who may run them, their guards and effects | module code | state labels; extra sub-states where the module allows it (e.g. ticket *on hold* reasons) |
| 2 | **Approvals** | Turns an action blocked by a `LIM`/`SOD` rule into an approval request ([ACCESS-CONTROL.md](ACCESS-CONTROL.md) §5) | `wl-kernel` | thresholds, approving role |
| 3 | **Automation rules** | *When* an event or date, *if* a condition, *then* actions from a fixed catalogue | defaults per vertical | turn rules on/off, change parameters; MVP ships defaults only |
| 4 | **Outbound webhooks + API** | Sends events to external URLs, signed and retried; API tokens for pulling data | Renox (R11) | subscriptions per event |

## 3. State machine

### 3.1 What it is

A state machine is a **declaration attached to a model**, not a separate service. For one
entity it declares:

- **States:** name, UI tone ([DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) §6), whether it is terminal.
- **Transitions:** a verb (`issue`, `activate`, `resolve`), the states it may start from, the
  state it leads to, and:
  - **by:** the permission needed, or `system` for transitions only automations or other
    transitions may run;
  - **guards:** conditions that must hold (required fields or documents) and access rules
    (`PRP.ACL.04`…);
  - **effects:** what happens after it succeeds (events, documents, other transitions).

### 3.2 What the engine guarantees

One small engine runs every state machine, so every entity behaves the same way:

1. **The status column is changed only by a transition.** No form, import or admin page
   writes it directly.
2. **One check path.** A transition runs authentication, permission in scope, guards and
   access rules in the order of [ACCESS-CONTROL.md](ACCESS-CONTROL.md) §4. A `LIM`/`SOD` failure
   becomes an approval request.
3. **History.** Every transition writes a history row: entity, from, to, transition, actor,
   time, reason, approval. The UI shows it with the Renox `history` block.
4. **Events.** Every transition emits `<Entity>Transitioned { from, to, transition }`, which
   automations and outbound webhooks listen to.
5. **Available actions.** For a given user and record, the engine lists the transitions they
   may run now. The UI renders exactly those as buttons, so pages never repeat the rules.
6. **Atomic.** The status change, the history row and the effects that write data commit
   together or not at all. Notifications and webhooks are queued after the commit.

### 3.3 What it deliberately does not do

- No visual designer and no states defined at runtime.
- No parallel states and no nested sub-processes. An entity has exactly one status. Parallel
  work is modelled as separate entities, e.g. a lease and its inspection ticket.
- No timers inside the machine. Time-based transitions are automation rules (§4) that call a
  `system` transition.

### 3.4 Where it lives

The engine is generic, so it was proposed for **Renox** as **R12** and ships in Renox 1.2.0
(`StateMachine`, `Machine`, `Transition`, `state_transitions`, `rnx state:diagram`). Modules
only declare their machines.

Two limits of the Renox engine shape how our tables are built (checked in the source,
2026-10-18):

- **One fixed target per transition.** A row whose "To" lists several states and means "back
  to the stored state" (a release after a hold, a back transition after an approval) is
  declared as one transition per target, `<verb>_to_<state>` or `back_to_<state>`, each
  guarded by the stored state. The UI still shows one button, since only the matching one is
  available.
- **No run by name and no replay.** Running a transition chosen at run time (by the approval
  kernel, by an automation rule) goes through the kernel's registry of transitions by name
  ([wl-kernel](modules/wl-kernel.md) §4.1, built in `crates/wl-kernel/src/machines.rs`). The
  transition the kernel runs after the last approval is a `system` transition named
  `<action>_approved` (e.g. `cancel_approved`, `force_terminate_approved`, `pay_out_approved`),
  next to the user's `<action>`, which only opens the request.
- **No transition that stays in its state.** A row whose "From" and "To" are the same state
  (deposit `adjust` held → held, management agreement `transfer_unit` active → active, handover
  `hand_over` open → open) is not a machine transition: it is built as its own record with
  its own guards and approval (e.g. `DepositAdjustment` in crates/wl-payments/src/deposit_adjust.rs,
  `UnitTransfer` in apps/property/src/unit_transfer.rs). The tables keep the row, since it is
  what the user sees as an action on the record.

### 3.5 How a state machine is documented

Two tables per entity, in the vertical or module document:

**States**

| State | Tone | Terminal | Meaning |
|---|---|---|---|

**Transitions**

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|

**Diagram:** a Mermaid `stateDiagram-v2` drawn from the transitions table, one arrow per
`from → to` pair, labelled with the transition. The table is authoritative.

The engine exists (Renox 1.2.0), so the same diagram can be **generated** from the declaration with
`rnx state:diagram`; a hand-drawn diagram in a document must match it, and the table stays authoritative.

## 4. Automation rules

### 4.1 Shape

**When** a trigger · **if** conditions · **then** one or more actions. Each rule has an ID
(`<DOMAIN>.RUL.NN`), parameters with defaults, and a default on/off state.

### 4.2 Catalogue

Rules are built only from this catalogue. A new trigger, condition or action is a platform
change, not a client configuration.

| Kind | Item | Example |
|---|---|---|
| Trigger | **Transitioned**: entity moved to / from a state | invoice → *overdue* |
| Trigger | **Date reached**: N days before/after a date field | 90 days before `lease.end_date` |
| Trigger | **Created**: a record of a type was created | a ticket was created |
| Trigger | **Schedule**: daily / weekly / monthly at a time | 5th of each month, 06:00 |
| Trigger | **Approval decided** | refund approved |
| Condition | Field comparison: `=`, `≠`, `>`, `<`, `in` | `amount > 5000`, `category in [plumbing, ac]` |
| Condition | Scope: record is in given buildings / projects / branches | building in [Tower A] |
| Condition | Exists / missing: a related record or document | no Ejari number recorded |
| Condition | Current state is / is not | lease is not *renewed* |
| Action | **Notify**: template, recipients (role in scope, party, assignee), channel (email, WhatsApp, in-app) | remind the tenant |
| Action | **Transition**: run a `system` transition | mark invoice *overdue* |
| Action | **Create task**: a ticket from a template, assigned to a role in scope | follow up a bounced cheque |
| Action | **Generate document**: from a template, attached to the record | draft owner statement |
| Action | **Request approval** | — |
| Action | **Send webhook**: to a subscribed URL | push to the client's n8n |

### 4.3 Execution guarantees

- Runs in the Renox queue as the **system actor**, with only the rights the rule needs, and
  logged.
- **Idempotent:** a rule fires at most once per record per occurrence (e.g. once per invoice
  for "overdue by 7 days").
- **No loops:** a chain of rules triggering rules stops at depth 3 and is reported.
- **Visible:** every run is logged (rule, record, result). Failures retry with backoff and
  show on an admin page.

## 5. Outbound webhooks and API

- A client admin subscribes a URL to events (`InvoiceIssued`, `LeaseActivated`, …).
- Payloads are signed (HMAC), delivered through the queue, retried with backoff, and logged.
- API tokens with abilities (Renox) let external tools read data within the same access rules.
- This is where client-specific integrations go: n8n, Zapier, Make, or a small script.

Renox has inbound webhooks only. Outbound webhooks are proposed as **R11**.

## 6. Shared capabilities

| ID | Capability | Module | Scope |
|---|---|---|---|
| `PLT.WFL.01` | State machines with transition history and available actions | Renox (R12) + wl-kernel | MVP |
| `PLT.APR.01` | Approval requests | wl-kernel | MVP |
| `PLT.ATM.01` | Automation rules: defaults per vertical | wl-kernel | MVP |
| `PLT.ATM.02` | Automation rules: client can toggle and set parameters in the admin | wl-kernel | Later |
| `PLT.HOK.01` | Outbound webhooks and event subscriptions | Renox (R11) | Later |

## 7. Shared automation register

Every shared automation, with its owner and the vertical rules it generalises. Statuses as in
[ACCESS-CONTROL.md](ACCESS-CONTROL.md) §9.

| ID | Rule | Owner | Generalises | Status |
|---|---|---|---|---|
| `PLT.RUL.01` | Remind approvers of a pending approval after N hours | wl-kernel | — | module |
| `PLT.RUL.02` | Escalate or expire an approval pending for M hours | wl-kernel | — | module |
| `PLT.RUL.03` | Daily digest of failed automation runs to the company admin | wl-kernel | — | module |
| `PLT.RUL.04` | Alert the owner role and admin when a compliance-critical rule fails, stops or misses its schedule | wl-kernel | — | module |
| `PLT.RUL.05` | Mark an integration down after N minutes without a successful call; banner, manual fallback, alert; replay and reconcile on recovery | wl-kernel | — | module |
| `PLT.RUL.06` | Daily digest of voids and late entries to each manager | wl-kernel | — | module |
| `PTY.RUL.01` | Re-screen parties against sanctions lists periodically and on list updates | wl-parties | `ACC.RUL.08` | in use |
| `PTY.RUL.02` | Alert compliance when a sanctions list is stale; retry; flag and re-run screenings made on the stale list | wl-parties | — | module |
| `POR.RUL.01` | Remind the party of open document requests; escalate overdue ones to staff | wl-portal | `ACC.RUL.03` (`ACC.RUL.02`, which creates the request list, stays in the vertical) | in use |
| `DOC.RUL.01` | Tiered reminders before a document expires | wl-documents | — | in use |
| `DOC.RUL.02` | Before a blocking document expires, list the planned work it affects and tell the responsible staff | wl-documents | — | module |
| `DOC.RUL.03` | When a blocking document expires, emit the blocked records, tell staff (and the owner if compliance-critical), open a renewal task | wl-documents | — | module |
| `DOC.RUL.04` | Request, remind and escalate SOP acknowledgements for each new version | wl-documents | — | module |
| `NTF.RUL.01` | On give-up, run the notification type's give-up reaction | wl-notify | — | module |
| `NTF.RUL.02` | Send marketing only 07:00–21:00 and at most twice per recipient per 7 days | wl-notify | — | module |
| `NTF.RUL.03` | Expire unapproved broadcasts and drop marketing messages stuck more than 48 hours | wl-notify | — | module |
| `FIN.RUL.01` | Invoice reminders before and after the due date | wl-invoicing | — | in use |
| `FIN.RUL.02` | Mark invoices overdue | wl-invoicing | — | in use |
| `FIN.RUL.03` | Put cheques on the deposit list before their date | wl-payments | — | in use |
| `FIN.RUL.04` | On a bounced cheque, notify and create a follow-up task | wl-payments | — | in use |
| `FIN.RUL.05` | Warn when committed + actual cost reaches P% of a budget line | wl-invoicing | — | in use |
| `FIN.RUL.06` | Retry failed subscription payments, then suspend | wl-invoicing | — | in use |
| `FIN.RUL.07` | Remind the owner before a sent quotation expires | wl-invoicing | `CON.RUL.06` | in use |
| `FIN.RUL.08` | Prepare the VAT summary N days after a tax period ends | wl-invoicing | — | module |
| `FIN.RUL.09` | Warn N days before the VAT return due date if the summary is not locked | wl-invoicing | — | module |
| `FIN.RUL.10` | Notify holder and accountant when a petty cash float runs low | wl-payments | — | module |
| `FIN.RUL.11` | Remind the approver of an undecided expense claim | wl-payments | — | module |
| `FIN.RUL.12` | Create a task for bank statement lines still unmatched after N days | wl-payments | `PRP.RUL.08` | in use |
| `FIN.RUL.13` | Create a follow-up task when a third-party payer has not remitted N days after the bill was issued | wl-invoicing | `CLN.RUL.04` | in use |
| `FIN.RUL.14` | Propose the VAT bad-debt relief adjustment when a written-off invoice meets the conditions | wl-invoicing | — | module |
| `FIN.RUL.15` | Escalate an invoice still disputed after N days | wl-invoicing | — | module |
| `FIN.RUL.16` | Create a task when an e-invoice is rejected or not accepted in time | wl-invoicing | — | module |
| `FIN.RUL.17` | Draft a supplier debit note when a return to supplier is dispatched | wl-invoicing | — | module |
| `FIN.RUL.18` | Remind about a cheque still bounced after 3 and 14 days, then ask for a decision (re-present, legal case, write-off) | wl-payments | — | module |
| `FIN.RUL.19` | Query the gateway for a payment pending N minutes, then confirm or fail it | wl-payments | — | module |
| `FIN.RUL.20` | Create a chargeback evidence task with the gateway deadline | wl-payments | — | module |
| `FIN.RUL.21` | Retry a failed refund once, then ask the accountant to change the method | wl-payments | — | module |
| `FIN.RUL.22` | Escalate a cash handover still open N hours after the holder's period ends; the manager counts what was received after M hours | wl-payments | `LOG.RUL.13` (values) | in use |
| `FIN.RUL.23` | Warn when a projected weekly cash balance falls below the entity's threshold | wl-invoicing | — | module |
| `FIN.RUL.24` | Warn when a month's actual expense or drawings reach the budget line | wl-invoicing | — | module |
| `FIN.RUL.25` | Task for the accountant when a bank balance for the forecast is missing or older than 7 days | wl-invoicing | — | module |
| `FIN.RUL.26` | Remind 30 days before a fiscal year that has no approved budget | wl-invoicing | — | module |
| `SAL.RUL.01` | Notify accountant and sales rep when an order goes on credit hold | wl-sales | — | in use |
| `SAL.RUL.02` | Remind the party before a package expires | wl-sales | `CLN.RUL.06`, `EDU.RUL.04` | in use |
| `SAL.RUL.03` | Notify the party when package credits run low | wl-sales | `EDU.RUL.03` | in use |
| `SAL.RUL.04` | Remind the customer of an order ready but not collected | wl-sales | `AUT.RUL.06` | in use |
| `SAL.RUL.05` | Put the customer account and its credit group on hold when a cheque bounces | wl-sales | — | module |
| `SAL.RUL.06` | Refund a payment that arrives for a cancelled, rejected or fully paid order | wl-sales | — | module |
| `SAL.RUL.07` | Earn loyalty points or visits on a qualifying sale; reverse them on a return or credit note | wl-sales | `FNB.LOY.01` (06) | module |
| `SAL.RUL.08` | Remind members before points expire, then expire them | wl-sales | — | module |
| `SAL.RUL.09` | Take back loyalty points when the payment is reversed, the invoice written off or the fulfilment reversed | wl-sales | — | module |
| `SAL.RUL.10` | Tell the campaign owner when costs reach 80% and 100% of the budget | wl-sales | — | module |
| `WRK.RUL.01` | SLA warning and breach notifications | wl-ticketing | — | in use |
| `WRK.RUL.02` | Auto-close resolved tickets not confirmed in N days | wl-ticketing | — | in use |
| `WRK.RUL.03` | Create a planned maintenance ticket when a meter reading reaches the threshold | wl-ticketing | — | in use |
| `WRK.RUL.04` | Create planned maintenance tickets ahead of the due date | wl-ticketing | — | module |
| `WRK.RUL.05` | Mark tasks overdue | wl-ticketing | — | module |
| `WRK.RUL.06` | Notify the owner when a task's start date arrives | wl-ticketing | — | module |
| `WRK.RUL.07` | Notify the recipient when a delivery is out for delivery | wl-delivery | `LOG.RUL.01` | in use |
| `WRK.RUL.08` | After a failed attempt, schedule a reattempt or start the return, per policy | wl-delivery | `LOG.RUL.02`, `LOG.RUL.03` | in use |
| `WRK.RUL.09` | Send a feedback request when a ticket, booking, order or delivery completes | wl-ticketing | — | module |
| `WRK.RUL.10` | A rating below the threshold opens a ticket | wl-ticketing | — | module |
| `WRK.RUL.11` | Expire unanswered feedback requests | wl-ticketing | — | module |
| `WRK.RUL.12` | Mark a service contract as expiring N days before its end | wl-ticketing | `PRP.RUL.06` | in use |
| `WRK.RUL.13` | Expire a service contract at its end date | wl-ticketing | — | module |
| `WRK.RUL.14` | Flag a missed planned visit; for compliance contracts, flag non-compliance | wl-ticketing | `PRP.RUL.07` | in use |
| `WRK.RUL.15` | Warn the dispatcher about rider runs in the midday break window beyond the stop and time limits; record overrides; the rider may decline | wl-delivery | `LOG.RUL.11` | in use |
| `WRK.RUL.16` | No reattempt for perishable deliveries: return or discard per policy | wl-delivery | — | module |
| `WRK.RUL.17` | Mark a delivery lost after N days untraced and open a claim ticket | wl-delivery | — | module |
| `WRK.RUL.18` | When a service contract ends, cancel or hand over later visits; flag compliance contracts without a replacement | wl-ticketing | `PRP.RUL.06` (end part) | in use |
| `WRK.RUL.19` | Record a vendor no-show M hours after the visit time and offer reassignment | wl-ticketing | — | module |
| `SCH.RUL.01` | Booking reminders N hours before | wl-scheduling | — | in use |
| `SCH.RUL.02` | Mark no-shows M minutes after start; offer the slot to the waiting list | wl-scheduling | — | in use |
| `SCH.RUL.03` | Offer a released slot to a matching waiting-list entry | wl-scheduling | — | module |
| `SCH.RUL.04` | Cancel requested bookings whose prepayment hold expired | wl-scheduling | — | module |
| `SCH.RUL.05` | Raise a booking conflict for future bookings whose resource will be ineligible at the start | wl-scheduling | — | module |
| `SCH.RUL.06` | Escalate a booking conflict still open H hours before the start | wl-scheduling | — | module |
| `HRM.RUL.01` | Flag check-ins during the midday break at outdoor sites | wl-attendance | — | in use |
| `HRM.RUL.02` | Draft the payroll run on the monthly cut-off | wl-hr | — | in use |
| `HRM.RUL.03` | Alert before and after the WPS due date when salaries are unpaid | wl-hr | — | in use |
| `HRM.RUL.04` | Open a visa renewal case N days before a residence visa expires | wl-hr | `MPW.RUL.01` | in use |
| `HRM.RUL.05` | On notice, draft the final settlement and the visa cancellation case | wl-hr | `MPW.RUL.07` | in use |
| `HRM.RUL.06` | Remind HR before probation ends | wl-hr | `MPW.RUL.08` | in use |
| `HRM.RUL.07` | Notify the supervisor when no attendance is recorded M minutes after shift start | wl-attendance | `MPW.RUL.02`, `CON.RUL.01`, `LOG.RUL.08` | in use |
| `HRM.RUL.08` | Submit timesheets to their approver at period end; remind if not approved | wl-attendance | `MPW.RUL.03`, `MPW.RUL.04` | in use |
| `HRM.RUL.09` | Alert before and on the 14-day deadline for paying a final settlement | wl-hr | — | module |
| `HRM.RUL.10` | When a blocking qualification expires, list the affected work and tell the manager and HR | wl-hr | — | module |
| `HRM.RUL.11` | Create tasks for salary lines rejected by the bank or WPS; escalate before the WPS due date | wl-hr | — | module |
| `HRM.RUL.12` | Propose an off-cycle run for released held lines; warn on carry-over past the WPS due date | wl-hr | — | module |
| `HRM.RUL.13` | Flag hours above the overtime cap or inside the midday break, including supervisor-marked hours | wl-attendance | — | module |
| `HRM.RUL.14` | Flag attendance recorded with an invalid work permit, visa or qualification | wl-attendance | — | module |
| `HRM.RUL.15` | Escalate an unapproved external timesheet; deemed approval after N days where the contract allows | wl-attendance | — | module |
| `HRM.RUL.16` | Start the onboarding checklist for the role when an employee is hired | wl-hr | — | module |
| `HRM.RUL.17` | Remind about overdue onboarding steps, then tell HR and the manager | wl-hr | — | module |
| `OPS.RUL.01`–`06` | Operator-side automations (health checks, suspension after non-payment, restore drills, usage collection, demo reset, replication and backup alerts) | wl-operator | — | module |
| `STK.RUL.01` | Notify procurement when a PO delivery is overdue | wl-inventory | — | in use |
| `STK.RUL.02` | Notify when stock falls below its minimum | wl-inventory | — | in use |
| `STK.RUL.03` | Warn N days before a lot expires | wl-inventory | — | in use |
| `STK.RUL.04` | Build reorder suggestions or draft POs daily | wl-inventory | — | in use |
| `STK.RUL.05` | Monthly dead-stock report | wl-inventory | `AUT.RUL.09` | in use |
| `STK.RUL.06` | Notify on a pending issue or negative balance; task after H hours | wl-inventory | — | module |
| `STK.RUL.07` | Remind the deciders of an open stock hold; notify the owner after 2×H | wl-inventory | — | module |
| `STK.RUL.08` | Chase a return to supplier not credited after N days | wl-inventory | — | module |
| `STK.RUL.09` | Give procurement a review task when a supplier's monthly score is below the threshold | wl-inventory | — | module |

## 8. Negative-flow patterns

Every module and vertical handles the same kinds of "something went wrong" in the same way.
The mechanisms live in [wl-kernel](modules/wl-kernel.md) (§4.1, §4.5, §5.12–5.15) and in the
modules named below; a state machine uses them instead of inventing its own.

| Pattern | When | What happens | Where |
|---|---|---|---|
| **Void** | A record was entered in error (duplicate, wrong party, wrong order) | `void` with a reason → terminal `voided`; effects reversed (money, stock, allocations, reserved slots); the number is kept and never reused; approval by another user when money or stock was posted. Never for documents already given to a third party or an authority: those are corrected (credit note, amendment, resubmission) | wl-kernel §5.12, `PLT.ACL.09` |
| **Approval rejected or returned** | An approver says no, or sends the request back | The record goes back to the state it had before the request; the reason is recorded in its history; the requester is told; nothing of the action ran. After `return`, the requester fixes and resubmits (a new linked request from step 1); after `reject`, a new request shows the earlier rejection | wl-kernel §4.1 |
| **Integration down** | A payment gateway, aggregator, e-invoicing provider, regulator portal or messaging provider does not answer | Outbound work is queued and retried; after N minutes the connection is `down`; affected screens show a banner and the manual fallback; work done by hand is entered with the late-entry flag; on recovery the queue is drained (late sync) and mismatches open a reconciliation task | wl-kernel §4.5, §5.14, `PLT.RUL.05` |
| **Late entry after an outage** | Work done on paper while the site was offline is entered later | The record keeps its business time and gets a late-entry flag with a reason; beyond N hours, or in a closed period, a manager approves; automations use the business time, so no false no-shows or overdue flags; managers get a daily digest | wl-kernel §5.13, `PLT.ACL.10`, `PLT.RUL.06` |
| **Expired document blocks** | A licence, certificate, permit or qualification needed for a step has expired | The step's transition has the guard `document_valid(subject, kind)` (or `qualification_valid(employee, kind)` for staff); each document kind lists the `<entity>.<transition>` it blocks; `DocumentExpired` lets modules flag future work that will be blocked | [wl-documents](modules/wl-documents.md), [wl-hr](modules/wl-hr.md) |
| **Cancel after work started** | The customer or the business cancels when part of the work is done | Cancel is allowed from the in-progress states with a reason; the work done is billed (or the deposit kept) per the module's policy; reserved resources (slots, stock, staff, vehicles) are released; materials used stay consumed; a refund or credit note covers what was paid and not delivered | each module's lifecycle (e.g. booking, order, work order, ticket) |
| **Dispute** | The party contests an invoice, a delivery, a charge or a decision | A `disputed` flag (not a state) pauses reminders, escalations and other automations on the record and links a ticket; the dispute is closed with an outcome (`upheld`, `credited`, or the module's own), which clears the flag and resumes or ends the automation | [wl-invoicing](modules/wl-invoicing.md) (invoice), [wl-ticketing](modules/wl-ticketing.md) (the ticket) |
| **Compliance-critical rule failure** | A rule that protects a legal duty fails, stops or misses its schedule | The business owner role and the admin are alerted at once, not only in the daily digest, and the owner gets a task until the duty is met | wl-kernel §5.15, `PLT.RUL.04` |
| **Party blocked** | A party is put on hold (sanctions, fraud, non-payment, legal dispute) | Each active agreement is paused, continues, or goes to review per its kind; no new business; no money out; the real reason is visible only to the compliance role | [wl-parties](modules/wl-parties.md) §5.6 |
| **Message gave up** | A message could not be delivered after all retries | `MessageGaveUp` reaches the requester, which reacts per its default (task for staff, alternative channel, contact marked invalid) | [wl-notify](modules/wl-notify.md) |

**Checklist: every state machine answers**

1. How is a record entered in error removed (`void`), and which effects are reversed?
2. Which transitions need approval, and which state does the record return to when the
   approval is rejected, returned, withdrawn or expires?
3. Can the record be cancelled after work started? What is billed, refunded and released?
4. Which documents or qualifications guard which transitions (`document_valid`,
   `qualification_valid`), and what happens to future work when one expires?
5. What does the record do while a party involved is blocked?
6. Which integration does a transition depend on, and what is the manual fallback?
7. Can it be entered late, and which field is its business time?
8. Can it be disputed, and which automations does the dispute pause?
9. Which of its rules are compliance-critical, and who is alerted when they fail?
10. Who is told, and what happens to money, stock and documents, on every negative path?

## Sources

Checked 2026-10-15. *Official* = government or legislation; others are secondary.

| # | Source | Type | Used for |
|---|---|---|---|
| S1 | n8n, "Sustainable Use License" (LICENSE.md), https://github.com/n8n-io/n8n/blob/master/LICENSE.md | Secondary (vendor licence text) | n8n use limited to internal business purposes |
