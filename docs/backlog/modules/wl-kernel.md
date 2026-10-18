# Epic: wl-kernel — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

The platform patterns every other module uses when something goes wrong
([WORKFLOW.md](../../WORKFLOW.md) §8, [wl-kernel](../../modules/wl-kernel.md) §4.1, §4.5,
§5.12–5.15): an approval that ends without a yes, voiding a record entered in error, late entry
after an outage, integration connections that go down and recover, and owner alerts when a
compliance-critical rule fails. It closes review item X-19 and carries the approval part that
every N1 item in the verticals relies on.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-KRN-01 | Send the subject back to its stored state when an approval ends without a yes | high | m | Build 1 | Base: wl-kernel Approval request lifecycle |
| BL-KRN-02 | Return an approval request for changes and link the resubmission | high | s | Build 1 | BL-KRN-01 |
| BL-KRN-03 | Void a record entered in error | high | m | Build 1 | BL-KRN-01; Base: wl-kernel State machine engine, Sequences |
| BL-KRN-04 | Flag and approve late entries | medium | m | Build 1 | Base: wl-kernel Approval request lifecycle, Rule run lifecycle |
| BL-KRN-05 | Track integration health and switch to the manual fallback when a connection is down | high | m | Build 1 | Base: wl-kernel Rule run lifecycle, queue |
| BL-KRN-06 | Drain the queue and reconcile manual work when a connection recovers | high | m | Build 1 | BL-KRN-05, BL-KRN-04 |
| BL-KRN-07 | Monitor site devices by heartbeat | medium | s | Launch 06 | BL-KRN-05, BL-KRN-06 |
| BL-KRN-08 | Alert the business owner when a compliance-critical rule fails or misses its schedule | high | m | Build 1 | Base: wl-kernel Rule run lifecycle |
| BL-KRN-09 | Send managers a daily digest of voids and late entries | medium | s | Build 1 | BL-KRN-03, BL-KRN-04 |

---

### BL-KRN-01 — wl-kernel: send the subject back to its stored state when an approval ends without a yes

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-kernel Approval request lifecycle (`submit`, `approve_step`, `approve`, `reject`, `withdraw`, `expire`, `fail`) · **Blocks:** BL-KRN-02, BL-KRN-03, BL-PTY-04, BL-DOC-06, every module with a waiting state (e.g. `pending_approval`), BL-ACC-14, BL-ACC-19, BL-AUT-01, BL-AUT-12, BL-CLN-08, BL-CLN-09, BL-CON-01, BL-CON-05, BL-CON-11, BL-CON-16, BL-CON-17, BL-EDU-12, BL-LOG-12, BL-LOG-15, BL-LOG-17, BL-MFG-01, BL-MPW-03, BL-MPW-04, BL-MPW-07, BL-PRP-01, BL-PRP-12, BL-PRP-19
**Review items:** X-19 (approval returned), N1 in every vertical

#### Goal
When an approval request ends with `reject`, `withdraw`, `expire` or `fail`, the record the
request was about (the subject) goes back to the state it had before the request, anything the
module reserved at submit time is released, and the requester is told why. Nothing of the
blocked action has run. This makes "approval rejected" behave the same in every module.

#### Surfaces
- **Back office:** the approver rejects with a required comment, or the requester withdraws,
  from the approval card; the requester sees the subject back in its stored state, the
  pending-approval badge gone and the request and decision in the record's Activity tab.
- **System:** on `reject`, `withdraw`, `expire` or `fail` the kernel runs the module's
  `system` back transition and release handlers, records "subject already moved" when needed
  and emits `ApprovalDecided`; the back transition is never offered to a user.

#### Files to read
- `docs/modules/wl-kernel.md` §3 (entity *Approval request*), §4.1 *Approval request* (states,
  transitions, the paragraph "What happens to the record when the answer is not yes"), §6
  (event `ApprovalDecided`)
- `docs/WORKFLOW.md` §3.2 (engine guarantees), §8 (pattern "Approval rejected or returned")
- `docs/ACCESS-CONTROL.md` §5 (approvals)

#### Terms
- **Subject:** the record the approval is about (a refund, a lease, a deduction…), referenced as
  `subject_type` + `subject_id`.
- **Waiting state:** a state some modules show while a request is pending (e.g. `submitted`,
  `pending_approval`). Modules without one only show a pending-approval badge.
- **Back transition:** a `system` transition the module declares from its waiting state to
  each state the wait can start from, one per state, named `back_to_<state>` and guarded by
  `subject_state_before`. The Renox 1.2 engine has a fixed `to` per transition and no run by
  name, so the kernel keeps a registry of transitions by name and runs the one that matches
  ([wl-kernel](../../modules/wl-kernel.md) §4.1).
- **`subject_state_before`:** the subject's state at `submit`, stored on the request.
- **Reservation:** an effect a module ran at submit time (stock hold, reserved slot, reserved
  number) that must be undone if the answer is not yes.
- **`system` transition:** only the platform may run it; never offered to a user.

#### Spec excerpt (authoritative for this issue)

Entity field added to *Approval request*: "state of the subject before the request".

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `rejected` | error | yes | An approver rejected it: the answer is no |
| `withdrawn` | neutral | yes | The requester cancelled it |
| `expired` | neutral | yes | No decision before the expiry time |
| `failed` | error | yes | Approved, but the action could not run because the record changed in the meantime |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `fail` | pending | failed | system | the action's guards no longer pass at final approval | the subject goes back to the state it had before the request (see below); requester and approver notified with the reason; event `ApprovalDecided` |
| `reject` | pending | rejected | approver of the current step | comment required | the subject goes back to the state it had before the request (see below); requester notified with the reason; event `ApprovalDecided` |
| `withdraw` | pending | withdrawn | requester | — | the subject goes back to the state it had before the request (see below); approvers notified; event `ApprovalDecided` |
| `expire` | pending | expired | system | expiry time passed | the subject goes back to the state it had before the request (see below); requester notified; event `ApprovalDecided` |

"What happens to the record when the answer is not yes" (`PLT.WFL.01` guarantee):
1. The request stores the subject's state before the request. A module that shows the wait as a
   state of its own declares a `system` transition back to that stored state. The kernel runs
   it on `reject`, `return`, `withdraw`, `expire` and `fail`, with the comment as the history
   reason. A module that does not use such a state only clears the pending-approval badge: the
   action never ran.
2. If the subject is no longer in its waiting state when the request ends (it was changed by
   another path, e.g. cancelled), the kernel does not move it: it records "subject already moved
   to `<state>`" on the request, still releases what the module reserved at submit, and tells
   the requester.
3. No money, stock or document effect runs before final approval. Effects a module ran at submit
   time (a hold on stock, a reserved slot, a reserved number that is not a document number:
   gapless `Sequence` numbers are taken only at final approval) are undone by the module's release
   handler, which the kernel runs on every end that is not `approve`, whether or not the module
   has a waiting state: in the same transaction as the back transition when there is one,
   otherwise in the same transaction as the request's own end.
4. The requester is notified with the approver's comment. The record's timeline shows the
   request and the decision.

Event:

| Event | When | Payload |
|---|---|---|
| `ApprovalDecided` | every end of a request: approved, rejected, returned, withdrawn, expired or failed | request, decision, approver(s), subject, state the subject went back to |

#### Edge cases decided
- **Same end on two paths at once** (approver rejects while the requester withdraws): the
  engine locks the request row; the first transition commits, the second finds the request no
  longer `pending` and is refused. Only one back transition runs.
- **Multi-step request rejected at step 2:** same behaviour; the step-1 decision stays in the
  request's decisions and in the timeline.
- **Subject already moved** (e.g. the refund was cancelled while waiting): no transition is run;
  the request records "subject already moved to `cancelled`"; reservations are still released;
  `ApprovalDecided.state_returned_to` is empty.
- **Amount changed while waiting:** the request keeps its payload snapshot. If at final approval
  the action's guards no longer pass, `fail` runs and the subject goes back like `reject`.
- **History reason without a comment** (`withdraw`, `expire`, `fail`): the reason is the fixed
  text "approval withdrawn by requester", "approval expired" or
  "approval failed: <guard message>".
- **Release without a waiting state:** the release handler runs on every non-yes end even when
  there is no back transition; reservations never depend on the module having a waiting state.
- **Repeated runs:** release handlers may run twice after a retry. Release handlers are
  idempotent, keyed by request id; a second release is a no-op.
- **Time zone:** `expires at` is stored as an instant; it is shown in Asia/Dubai. `expire` itself
  is triggered by `PLT.RUL.02` (out of scope); this issue only handles its effect.

#### Must not
- Must not run any money, stock or document effect of the action before final approval.
- Must not let a user run the back transition directly (it is `system`).
- Must not move a subject that is no longer in its waiting state.
- Must not leave a reservation in place after a non-yes end.
- Must not write `status` outside the engine.

#### Done when
- [ ] Given a refund moved from `requested` to `pending_approval` by a request, when approver B
      rejects with comment "duplicate", then the refund is `requested`, its history row has
      reason "duplicate", user A (requester) is notified with the comment, and
      `ApprovalDecided { decision: rejected, state_returned_to: requested }` is emitted.
- [ ] Same for `withdraw` by user A (approvers notified), `expire` (expiry time passed) and
      `fail` (the refund amount AED 500 now exceeds the remaining balance AED 300 at final
      approval).
- [ ] Given a subject without a waiting state, `reject` clears the pending-approval badge and the
      subject state is unchanged.
- [ ] Given a booking module **without** a waiting state whose action reserved the 10:00 slot of
      2026-11-03 at submit (booking stays `requested` with the badge), after `reject` the
      booking is still `requested`, the badge is gone and the 10:00 slot is free.
- [ ] Given a material request **with** a waiting state (`draft` → `pending_approval`) that held
      20 bags of cement at submit, after `expire` the request is back in `draft` and the hold
      is released in the same transaction as the back transition.
- [ ] Given the refund was cancelled while the request was pending, when the request is
      rejected, the refund stays `cancelled`, the request shows "subject already moved to
      `cancelled`", the reservation is released and user A is notified.
- [ ] `reject` without a comment is refused.
- [ ] Rejecting and withdrawing the same request at the same time leaves exactly one end state
      and one back transition in history.
- [ ] No user sees the back transition as an available action.

#### Out of scope
`return` and resubmission (BL-KRN-02); reminders and escalation (`PLT.RUL.01`, `PLT.RUL.02`);
each module's declaration of its own back transition and release handlers (module epics).

---

### BL-KRN-02 — wl-kernel: return an approval request for changes and link the resubmission

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-KRN-01 · Base: wl-kernel Approval request lifecycle · **Blocks:** None.
**Review items:** X-19 (approval returned)

#### Goal
An approver can send a request back for changes instead of rejecting it. The subject goes back
to its stored state (as in BL-KRN-01), the requester gets a "fix and resubmit" task, and a
resubmission is a new request linked to the old one that starts again at step 1. A new request
after a `rejected` one shows the earlier rejection, so a "no" is not silently retried.

#### Surfaces
- **Back office:** the approver of the current step chooses Return with a required comment;
  the requester gets a "fix and resubmit" task in "Needs your action" (closed by a
  resubmission or "not resubmitting"); the approver's card of a resubmission shows the
  earlier rejection or return (date, approver, comment).
- **System:** a resubmission is a new request linked by `resubmission of` that starts again
  at step 1; the subject goes back as in BL-KRN-01 and `ApprovalDecided { returned }` is
  emitted.

#### Files to read
- `docs/modules/wl-kernel.md` §3 (*Approval request*: "resubmission of"), §4.1 (states,
  transitions, diagram, last bullet of "What happens to the record…")
- `docs/WORKFLOW.md` §8 (pattern "Approval rejected or returned")

#### Terms
- **Returned:** terminal state meaning "sent back for changes"; the requester may resubmit.
- **Resubmission:** a new approval request whose field `resubmission of` points to the earlier
  returned or rejected request.
- **Task:** a ticket created through the `TaskCreator` capability (`WORKFLOW.md` §4.2 *Create
  task*).
- **Action key:** the identifier of the guarded action on the *Approval policy* entity
  (`wl-kernel.md` §3), e.g. `deposits.refund`; the request's `action` field holds it.
- **Payload snapshot:** the amounts and reason stored on the request at `submit` (§3).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `returned` | warning | yes | An approver sent it back for changes: the requester may fix and resubmit |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `return` | pending | returned | approver of the current step | comment required | as `reject`; the requester gets a task "fix and resubmit"; a resubmission creates a new request linked to this one and starts again at step 1 |

Diagram additions: `pending --> returned: return`, `returned --> [*]`.

Rule: "`rejected` means the same action with the same payload is not resubmitted silently: a new
request shows the earlier rejection to the approver. "The same action with the same payload"
means the same action key (e.g. `deposits.refund`), the same subject and an equal payload
snapshot (amounts compared at 2 decimals, the reason text ignored); such a request is allowed,
and the approver's card shows the earlier rejection (date, approver, comment). `returned`
invites a changed resubmission; an unchanged one is allowed too, and the approver sees the
return comment the same way. The requester's "fix and resubmit" task closes when a linked
resubmission is submitted, or when the requester closes it with "not resubmitting"."

#### Edge cases decided
- **"Same action with the same payload"**: same action key (the request's
  `action`, e.g. `deposits.refund`), same `subject_type` + `subject_id`, and an equal payload
  snapshot (amounts compared at 2 decimals, the reason text ignored). The new request is allowed;
  the approver's card shows the earlier rejection (date, approver, comment).
- **Resubmitting an unchanged payload after `return`:** allowed; the new
  request shows the return comment to the approver like a rejection.
- **Chains:** a resubmission links to its immediate predecessor only; the card shows the whole
  chain by following the links.
- **The "fix and resubmit" task closes** when a linked resubmission is submitted. The requester
  may also close it with "not resubmitting".
- **Subject moved before resubmission:** the resubmission is a normal `submit`; the module's
  guards decide whether the action is still possible.
- **SoD on the new request:** applied afresh from step 1; an approver who returned the old
  request may approve the new one.

#### Must not
- Must not reopen or reuse a `returned` request: resubmission always creates a new request.
- Must not continue a resubmission at the step where the old request stopped.
- Must not block a new request after `rejected`; it only shows the earlier rejection.

#### Done when
- [ ] Given a two-step request for a deduction of AED 6,000 approved at step 1 by user B, when
      user C at step 2 runs `return` with "attach the inspection photos", then the request is
      `returned`, the subject is back in its stored state, user A has a task "fix and resubmit"
      and `ApprovalDecided { decision: returned }` is emitted.
- [ ] `return` without a comment is refused.
- [ ] When user A resubmits, a new request exists with `resubmission of` = the old one, at
      step 1, and the task is closed.
- [ ] Given a request rejected on 2026-11-02 for refund AED 1,200, when user A submits the same
      refund for AED 1,200 again, the approver's card shows "rejected on 2026-11-02 by B:
      <comment>".
- [ ] The returned request offers no transitions.

#### Out of scope
The back transition and release (BL-KRN-01); approval policies per vertical.

---

### BL-KRN-03 — wl-kernel: void a record entered in error

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-KRN-01 · Base: wl-kernel State machine engine (`PLT.WFL.01`), Sequences, Approval request lifecycle · **Blocks:** BL-KRN-09; `void` in every module that opts in, BL-AUT-02, BL-AUT-10, BL-AUT-18, BL-CLN-10, BL-FNB-09, BL-FNB-17, BL-LOG-01, BL-MFG-04, BL-MFG-14, BL-PRP-26
**Review items:** X-19 (void)

#### Goal
Give every module one way to remove a record that should never have existed (a duplicate
receipt, a wrong-party booking): `void` with a reason, which reverses its effects in one
transaction, keeps its number, and needs another user's approval when money or stock was
posted. Documents already given to a third party or an authority are never voided.

#### Surfaces
- **Back office:** a user runs Void with a reason under More on the record page (not offered
  from states the module does not allow or for documents given to a third party); when money
  or stock was posted another user approves it; voided records show struck through in lists
  and reports and the gapless series shows the number as void.
- **System:** in one transaction the kernel runs the module's reverse handlers, moves the
  record to `voided`, writes the Void record and emits `RecordVoided`; a failing handler or
  a locked period rolls everything back.

#### Files to read
- `docs/modules/wl-kernel.md` §3 (*Void record*), §5.12, §6 (service `Voids`, event
  `RecordVoided`, permission `records.void`, rule `PLT.ACL.09`), §7 (setting), §10 (Voids)
- `docs/WORKFLOW.md` §8 (pattern "Void")
- `docs/modules/wl-kernel.md` §5.5 (gapless sequences)

#### Terms
- **Void:** "this record should never have existed". Not cancel (the business changed its mind)
  and not correct (credit note, adjustment).
- **Reverse handler:** a function the module registers per effect type (ledger posting, payment
  allocation, stock move, reserved slot, sent task) that undoes that effect.
- **Given to a third party:** issued tax invoice, filed return, submitted claim, sent WPS file:
  the module declares this per record.
- **Gapless series:** a numbering sequence that never skips numbers (tax invoices, credit notes).

#### Spec excerpt (authoritative for this issue)

Entity *Void record*: subject, state before the void, reason, voided by, approval, reversed
effects (postings, stock moves, allocations), at. The record keeps its number.

§5.12:
1. A module opts in by declaring a `void` transition from the states where it is allowed, to a
   terminal state `voided` (tone neutral), and a reverse handler for each effect the record had.
2. The user runs `void` with a reason. `PLT.ACL.09` decides whether it is allowed and whether it
   needs approval. The approver is any user holding the void permission for that entity in the
   record's scope, except the requester (one step, SoD).
3. In one transaction the kernel runs the module's reverse handlers, moves the record to
   `voided` and writes a Void record. If a reverse handler reports that its posting falls in a
   locked period, the void is refused with that message, and the module's own correction
   document is used instead. The record keeps its number; a gapless series shows the
   number as void and never reuses it.
4. Voided records stay visible (struck through), are excluded from totals and reports, and
   cannot be edited or reopened. Event `RecordVoided`.
5. Not voidable: anything already given to a third party or an authority as a legal or tax
   document. Those are corrected with the module's own document.

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `PLT.ACL.09` | A record can be voided only with a reason, only from the states its module allows, and never after it was given to a third party or an authority as a legal or tax document; when it had posted money or moved stock, the void needs approval by another user | `STA`, `SOD` | approval when money or stock was posted: on |

| Permission | Purpose |
|---|---|
| `records.void` | Void a record of an entity the user may edit (each module may name its own alias, e.g. `receipts.void`); approval per `PLT.ACL.09` |

| Event | When | Payload |
|---|---|---|
| `RecordVoided` | a record voided | subject, state before, reason, user, approval |

Setting: Void approval when money or stock was posted (`PLT.ACL.09`): on; not editable by client.

#### Edge cases decided
- **Who approves:** any user holding the void permission for that entity
  in the record's scope, except the requester (one step, SoD).
- **While the approval waits** the record keeps its state and shows the pending-approval badge;
  if it is changed meanwhile, the final approval re-checks the guards and `fail`s (BL-KRN-01).
- **Money or stock posted** means at least one reverse handler of type posting, allocation or
  stock move has something to reverse; any amount above AED 0.00 counts. A record with only a
  reserved slot or a task is voided at once without approval.
- **A reverse handler fails** (e.g. the stock was already issued onward): the whole transaction
  rolls back; the record keeps its state and the user sees the handler's message.
- **Reversal in a locked period:** if a reverse handler reports that its
  posting falls in a locked period, the void is refused with that message; the module's own
  correction document is used instead.
- **Numbering:** the number stays on the voided record; the next record gets the next number.
- **Time zone:** the Void record's `at` is stored as an instant and shown in Asia/Dubai.

#### Must not
- Must not void a record the module marks as given to a third party or an authority.
- Must not delete the record or free its number.
- Must not let the requester approve their own void.
- Must not include voided records in totals, balances or report aggregates.
- Must not allow edit or any transition out of `voided`.

#### Done when
- [ ] Given receipt RCP-ACME-2026-0042 of AED 3,000 allocated to invoice INV-ACME-2026-0101,
      when user A runs `void` with reason "duplicate", then an approval request is created and
      the receipt is unchanged.
- [ ] User A cannot approve it; when user B approves, in one transaction the allocation is
      reversed (invoice balance +AED 3,000), the receipt is `voided`, a Void record lists the
      reversed allocation, `RecordVoided` is emitted and a history row is written.
- [ ] The next receipt is numbered RCP-ACME-2026-0043; 0042 shows as void in the series.
- [ ] Given a draft booking with a reserved slot and no postings, `void` runs at once without
      approval and the slot is released.
- [ ] `void` without a reason is refused; `void` from a state the module does not allow is not
      offered.
- [ ] Given an issued tax invoice, `void` is not offered and a direct call is refused.
- [ ] A reverse handler that fails leaves the receipt in its previous state with no Void record.
- [ ] The receivables report excludes the voided receipt from totals and lists it struck through.

#### Out of scope
Each module's `void` declaration and reverse handlers (module epics); the daily digest
(BL-KRN-09); credit notes and other corrections (`Epic wl-invoicing`).

---

### BL-KRN-04 — wl-kernel: flag and approve late entries

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:medium` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-kernel Approval request lifecycle, Rule run lifecycle · **Blocks:** BL-KRN-06, BL-KRN-09, BL-ACC-24, BL-AUT-18, BL-CON-18, BL-FNB-10, BL-FNB-14, BL-FNB-15, BL-LOG-25, BL-MFG-14, BL-PRP-27
**Review items:** X-19 (late entry), 06-N7-c

#### Goal
When work done on paper during an outage is entered later, the record keeps its business time
and carries a late-entry flag with a reason. Entries more than N hours late, or in a closed
period that the module allows to reopen, need a manager's approval. Automations use the business
time, so late records do not cause false no-shows or overdue flags.

#### Surfaces
- **Back office:** saving a record whose business time is more than M minutes old asks for a
  late reason and an optional outage reference; beyond N hours or in a closed period the
  save becomes an approval request for a holder of `late_entries.approve`; the late entry
  lists the rule runs "already fired, review".
- **System:** automations evaluate the business time in their conditions, the kernel finds
  the runs already fired between business time and entry time, and `LateEntryRecorded` is
  emitted.

#### Files to read
- `docs/modules/wl-kernel.md` §3 (*Entry timing*), §5.13, §6 (service `LateEntry`, event
  `LateEntryRecorded`, permission `late_entries.approve`, rule `PLT.ACL.10`), §7 (setting)
- `docs/WORKFLOW.md` §8 (pattern "Late entry after an outage")

#### Terms
- **Business time:** when the work really happened (order time, check-in time, POD time); the
  module names this field when it opts in.
- **Entry time:** when the record was typed into the system.
- **Outage reference:** a link to an integration `down` period (BL-KRN-05) or a manual note
  "outage at branch X from–to".
- **Closed period:** a locked timesheet, locked VAT period or closed payroll, as the module
  defines.

#### Spec excerpt (authoritative for this issue)

Entity *Entry timing* (fields on records that opt in): occurred at (business time), entered at,
late entry (yes/no), late reason, outage reference, late-entry approval, rule runs already fired
to review.

§5.13:
1. A module opts in for an entity and names its business-time field and, optionally, one
   parent-subject field: the `Subject` reference of the record the entry belongs to (e.g.
   `booking` on a check-in, `order` on a POD).
2. When the business time is more than M minutes before the entry time, the record is flagged
   late entry. The user gives a reason and may link an outage reference. Changing the
   business-time field of an existing record is checked the same way, against the time of the
   edit.
3. Up to N hours late, the flag and reason are enough. Beyond N hours, or for a business time in
   a closed period, the entry needs approval (`PLT.ACL.10`) or is refused when the module's
   period lock forbids it.
4. Late entries keep the business time for reports, deadlines and SLA calculations, and the
   entry time for the audit. Automations triggered by a late record run at entry time but use
   the business time in their conditions.
5. Automations that already ran before the late record was entered are not undone by the
   kernel. The kernel lists on the late entry the rule runs whose subject is the late record
   itself or the record named in its parent-subject field (point 1) and whose trigger time falls
   between the business time and the entry time, as "already fired, review"; the module that
   owns the affected record corrects its own state, by its own transitions, and the entry
   appears with these runs in the `PLT.RUL.06` digest.
6. `PLT.RUL.06` sends managers a digest (BL-KRN-09).

| ID | Rule | Pattern | Parameter (default) |
|---|---|---|---|
| `PLT.ACL.10` | An entry whose business time is more than N hours before the time it is entered, or falls in a closed period the module allows to reopen, needs a manager's approval; shorter delays above M minutes are flagged with a reason | `TIM`, `LIM` | M = 30 minutes, N = 24 hours |

| Event | When | Payload |
|---|---|---|
| `LateEntryRecorded` | a record entered with the late-entry flag | subject, business time, entry time, reason, approval, rule runs already fired to review |

Permission `late_entries.approve`: approve a late entry beyond N hours. Setting: M / N =
30 minutes / 24 hours, editable per entity.

#### Edge cases decided
- **Clock:** M and N are elapsed wall-clock time between two instants, not business hours.
  Business times are entered in Asia/Dubai local time (no daylight saving) and stored as
  instants.
- **Exactly at the limit:** 30 minutes late is not flagged; 30 minutes and 1 second is. 24 h 00 m
  is flag-only; anything above needs approval.
- **Approval:** the save is an action blocked by a `LIM` rule, so it becomes an approval request
  (kernel SoD on: the requester cannot approve). The record is created only on approval;
  `reject` creates nothing.
- **Edits:** changing the business-time field of an existing record is
  checked the same way, against the time of the edit.
- **Business time in the future:** not a late entry; the module's own validation applies.
- **Automations already fired before the late entry** (e.g. a no-show marked at 09:15, check-in
  entered at 10:30 with business time 09:05): the kernel does not undo runs that already
  happened; it lists them on the late entry as "already fired, review" and the module that owns
  the affected record corrects its own state. The runs are found by `subject` = the check-in
  or `subject` = the value of its parent-subject field (`booking` = B); an entity that names no
  parent-subject field only looks at runs on itself.
- **Closed period the module forbids reopening:** refused with the module's message; no request.

#### Must not
- Must not overwrite the business time with the entry time, or the reverse.
- Must not accept a flagged entry without a reason.
- Must not let the requester approve their own late entry.
- Must not evaluate automation conditions (no-show, overdue, SLA) on entry time for an opted-in
  record.

#### Done when
- [ ] Given an entity opted in with business-time field `checked_in_at`, an entry at
      2026-11-03 10:00 with business time 09:45 is saved without a flag.
- [ ] With business time 09:00, the save asks for a reason; without one it is refused; with
      "router down" it is saved, flagged, and `LateEntryRecorded` is emitted.
- [ ] With business time 2026-11-02 08:00 (26 hours late), an approval request goes to a holder
      of `late_entries.approve`; user A cannot approve; on approval by user B the record exists
      with the flag and the approval id; on reject nothing is created.
- [ ] A business time inside a closed period the module allows to reopen needs approval even
      when only 2 hours late; inside a period the module forbids, the save is refused.
- [ ] Given a ticket entity opted in with business-time field `reported_at` and an SLA of
      4 hours to respond, a ticket entered at 2026-11-03 13:00 with business time 10:00
      (reason "power cut") has response due 14:00 (10:00 + 4 h), not 17:00, and is shown
      overdue from 14:00:01.
- [ ] Changing M to 60 minutes for the entity stops flagging a 45-minute delay.
- [ ] Given a no-show rule run on booking B at 09:15, when its check-in is entered at 10:30
      with business time 09:05, the no-show is not undone by the kernel, the late entry lists
      the 09:15 run as "already fired, review", and `LateEntryRecorded` carries it.

#### Out of scope
The module's choice of business-time field and period locks (module epics); correcting the
records that automations already changed (owning module); the digest (BL-KRN-09).

---

### BL-KRN-05 — wl-kernel: track integration health and switch to the manual fallback when a connection is down

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-kernel Rule run lifecycle, Renox queue · **Blocks:** BL-KRN-06, BL-KRN-07, BL-PTY-10, BL-ACC-23, BL-AUT-18, BL-CLN-15, BL-EDU-17, BL-LOG-25, BL-MFG-14, BL-MPW-20
**Review items:** X-19 (integration down), 09-N7-a, 06-N7-c

#### Goal
Each external system a module calls (payment gateway, ASP for e-invoicing, WPS bank file,
regulator portal, messaging provider) is registered as a connection with a health state. Failed
calls are queued and retried; after N minutes without success the connection goes `down`, the
affected screens show a banner with the manual fallback, and the admin and the connection's
owner role are alerted. Outbound work is never lost.

#### Surfaces
- **Back office:** while a connection is `down`, the screens it registered show a banner
  with the manual fallback text; holders of `integrations.view` open the connection page
  (status, last error, queued items) and holders of `integrations.manage` disable it with a
  reason or enable it.
- **System:** failed calls are queued and retried with backoff without limit; `PLT.RUL.05`
  runs `go_down` after N minutes, alerts the admin and the owner role once and emits
  `IntegrationStatusChanged`.

#### Files to read
- `docs/modules/wl-kernel.md` §3 (*Integration connection*), §4.5 (states, transitions,
  diagram), §5.14 steps 1–3, §6 (service `Integrations`, event `IntegrationStatusChanged`,
  permissions `integrations.view`, `integrations.manage`), `PLT.RUL.05`, §7 (setting)
- `docs/verticals/09-manpower.md` §8 (fallbacks for SIF upload, SMS) as an example

#### Terms
- **Connection:** one external system a module talks to, registered by the owning module with
  the screens it affects and a manual fallback text (EN/AR).
- **Failed since:** the time of the first failed call of the current failure run.
- **Banner:** a message on each affected screen with the fallback instructions.
- **Owner role:** the role responsible for the connection (e.g. accountant for the gateway).

#### Spec excerpt (authoritative for this issue)

Entity *Integration connection*: key, owning module, status, last success, last error, failed
since, queued items, manual fallback instructions EN/AR, owner role, screens affected.

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `up` | success | no | Calls succeed |
| `degraded` | warning | no | Some calls failed; outbound work is queued and retried |
| `down` | error | no | Calls have failed for longer than the limit; banner shown, manual fallback offered |
| `disabled` | neutral | no | Switched off by the admin (e.g. contract with the provider ended); modules use their manual path |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `degrade` | up | degraded | system | a call failed | outbound calls queued; retries with backoff |
| `go_down` | degraded | down | system (`PLT.RUL.05`) | no successful call for N minutes | banner on the screens affected; admin and the connection's owner role alerted; manual fallback enabled; event `IntegrationStatusChanged` |
| `disable` | up, degraded, down | disabled | `integrations.manage` | reason given | queue kept; manual path only |
| `enable` | disabled | up | `integrations.manage` | — | queue drained |

| ID | Rule | When | Then | Parameter (default) |
|---|---|---|---|---|
| `PLT.RUL.05` | Integration down | a connection has had no successful call for N minutes | `go_down`: banner, manual fallback, alert the admin and the connection's owner role; on recovery, drain the queue and open reconciliation tasks | N = 15 |

§5.14: (1) each module registers its connections with the screens they affect and a manual
fallback; (2) "Outbound work is never lost: calls go through the queue and are retried with
backoff (waits of 1, 2, 4 and 8 minutes between attempts, then 15 minutes) with no attempt
limit: a queued item is only removed when it is sent or reported obsolete by its module.
'The limit' is time, not attempts: after N minutes without a successful call the connection
goes `down` (`PLT.RUL.05`); retries continue while it is `down`."; (3) while down, affected
screens show a banner with the fallback instructions; users record work done by hand with
the late-entry flag.

Setting: Integration down after N minutes: 15, editable per connection.

#### Edge cases decided
- **Where N counts from:** from `failed since` (first failure of the run), because `go_down`
  only starts from `degraded` (for devices, `failed since` is the last heartbeat:
  BL-KRN-07). An idle connection with no calls never goes down.
- **Backoff:** a failed item waits 1, 2, 4 and 8 minutes between attempts, then 15 minutes,
  with no attempt limit (spec §5.14 step 2). An item is never dropped.
- **Retries while down** continue every 15 minutes; the first success triggers `recover`
  (BL-KRN-06).
- **Alerts** go once per `go_down`, not per failed call.
- **`disable` while items are queued:** the queue is kept and not sent; `enable` moves to `up`
  and drains it (drain behaviour in BL-KRN-06).
- **Banner text** is shown in the user's language; if the AR text is empty, EN is shown.
- **Concurrent failures** from several workers update one connection row under a lock; only one
  `degrade` is recorded.

#### Must not
- Must not drop or reorder a queued outbound item.
- Must not call the external system while `disabled`.
- Must not show the banner on screens the connection did not register.
- Must not change a connection's status outside these transitions.

#### Done when
- [ ] Given connection `asp.acme` registered by `wl-invoicing` (test fixture) with owner role
      *accountant*, screens affected *invoice list* and *invoice detail*, and N = 15, `up`;
      when a call fails at 10:00, then it is `degraded`, `failed since` = 10:00 and the call
      is queued and retried at 10:01, 10:03, 10:07 and 10:15.
- [ ] With no success until 10:15, `go_down` runs: the invoice list and invoice detail show the
      banner with the fallback text (other screens do not), the company admin and the holders
      of the accountant role are alerted once, and
      `IntegrationStatusChanged { from: degraded, to: down }` is emitted.
- [ ] After `go_down` the queued call is still retried every 15 minutes (10:30, 10:45…) and is
      never dropped.
- [ ] With N set to 5 for the connection, the same happens at 10:05.
- [ ] `disable` without a reason is refused; with a reason, nothing is sent and the queue keeps
      its 12 items.
- [ ] A user without `integrations.view` cannot open the connection page.

#### Out of scope
Recovery, late sync and reconciliation (BL-KRN-06); device heartbeats (BL-KRN-07); each module's
fallback text and screen list (module epics).

---

### BL-KRN-06 — wl-kernel: drain the queue and reconcile manual work when a connection recovers

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-KRN-05, BL-KRN-04 · **Blocks:** BL-KRN-07, BL-FNB-14
**Review items:** X-19 (integration down)

#### Goal
When a degraded or down connection answers again, the queued outbound work is sent in order
(late sync), the banner disappears, and work done by hand while it was down is matched against
the late sync results. Mismatches open a reconciliation task for the owner role. Inbound data
that arrives late is processed with its own business time.

#### Surfaces
- **Back office:** the banner disappears from the affected screens on recovery; the owner
  role gets one reconciliation task in "Needs your action" listing manual-versus-late-sync
  mismatches and obsolete queued items.
- **System:** `recover` commits `up`, then the queue drains oldest first, the natural key
  prevents duplicates, a failure during the drain runs `degrade`, and late inbound data
  follows the late-entry rules.

#### Files to read
- `docs/modules/wl-kernel.md` §4.5 (`recover`, `enable`), §5.14 steps 4–5, §5.13 (late entry)

#### Terms
- **Late sync:** sending the queued items after recovery, oldest first.
- **Manual item:** a record entered by hand while the connection was down, carrying the
  late-entry flag and the outage reference.
- **Natural key:** the module's unique business key, declared when it registers the connection,
  used to avoid duplicates (e.g. external order number; for card payments through a gateway,
  the gateway's transaction reference, stored on the payment as `external reference`).
- **Reconciliation task:** a task (ticket via `TaskCreator`) for the owner role listing items
  whose manual record and late sync result differ.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `recover` | degraded, down | up | system | a call succeeded | queue drained in order (late sync); items handled manually while down listed for reconciliation; banner removed; event `IntegrationStatusChanged` |
| `enable` | disabled | up | `integrations.manage` | — | queue drained |

§5.14:
4. On `recover`, the queue is drained in order (late sync). `recover` commits the move to `up`
   first; the drain then runs as queued work. A call that fails during the drain is a failed
   call of an `up` connection: `degrade` runs and the rest of the queue waits. Before each
   queued item is sent, its module's send handler is asked whether it is still due; an item
   whose record changed while queued (voided, cancelled) is reported obsolete, is not sent and
   is listed on the reconciliation task. Items done manually while down are matched to the late sync results; mismatches open a reconciliation
   task for the owner role. Duplicates are prevented by the module's natural key (e.g. external
   order number, the gateway's transaction reference for a card payment), which the module
   declares when it registers the connection.
5. Inbound data that arrives late (a gateway callback, an aggregator order) is processed with
   its own business time, under the same late-entry rules.

#### Edge cases decided
- **Order:** first in, first out per connection, by enqueue time.
- **Failure during the drain:** `recover` has already committed to `up` before the drain starts
  (spec §5.14 step 4); a failed call during the drain is a failed call of an `up` connection,
  so `degrade` runs (`up` → `degraded`, `failed since` = that failure). The remaining items stay
  queued in the same order; nothing already sent is re-sent.
- **An item whose record changed while queued** (voided, cancelled): the
  owning module's send handler is asked before sending; if it reports the item obsolete, it is
  not sent and is listed on the reconciliation task.
- **Duplicate by natural key:** the late sync result is linked to the existing manual record; no
  second record is created. Different values (amount, status) on the two open a mismatch line.
- **One task per recovery**, listing all mismatches; no task when there are none.
- **Inbound late data** uses BL-KRN-04: flagged when more than M minutes late; beyond N hours it
  needs approval like any late entry.

#### Must not
- Must not create a second record for the same natural key.
- Must not send queued items out of order.
- Must not remove the banner before `recover` commits.

#### Done when
- [ ] Given connection `gateway.acme` `down` with 3 queued refund calls (10:01, 10:02, 10:05),
      when a call succeeds at 10:40, then it is `up`, the 3 calls are sent in that order, the
      banner is gone and `IntegrationStatusChanged { from: down, to: up }` is emitted.
- [ ] Given connection `gateway.acme` registered with natural key = the payment's `external
      reference`, and a card payment of AED 250 entered by hand at 10:20 with external reference
      `TX-889`, and the gateway's late callback for `TX-889` of AED 250, no new payment is
      created and no task is opened.
- [ ] If the callback says AED 205, the owner role gets one reconciliation task listing `TX-889`
      (AED 250 manual vs AED 205 gateway).
- [ ] If the second queued call fails during the drain at 10:41, the connection went `up` at
      10:40 and is now `degraded` with `failed since` = 10:41, the second and third calls are
      still queued in that order and the first is not sent again.
- [ ] `enable` on a `disabled` connection with 2 queued items sends both.

#### Out of scope
Health states and `go_down` (BL-KRN-05); each module's matching rules and natural keys
(module epics).

---

### BL-KRN-07 — wl-kernel: monitor site devices by heartbeat

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 06 · **Size:** s
**Depends on:** BL-KRN-05, BL-KRN-06 · **Blocks:** BL-FNB-14
**Review items:** X-19, 06-N7-c

#### Goal
A device at a site that calls the platform (kitchen display, till, scanner) is a connection
whose health signal is its heartbeat. Missed heartbeats count as failed calls, so the same
`degrade`, `go_down` and `recover` apply. Silence outside the device's active window is ignored,
and devices of one scope can be grouped into one connection. 06 F&B is the first user (its rule
`FNB.RUL.15` pauses an outlet's brands when its kitchen devices go silent).

#### Surfaces
- **Back office:** the company admin sets a device connection's heartbeat interval, active
  window and grouping per scope in the connection settings.
- **System:** missed heartbeats count as failed calls for `degrade`, `go_down` and
  `recover`; silence outside the active window is ignored and a group is `down` only when no
  device answers.

#### Files to read
- `docs/modules/wl-kernel.md` §4.5 (paragraph "Connection kinds"), §7 (setting *Device
  connections*)
- `docs/verticals/06-fnb.md` (`FNB.RUL.15`) for the consumer

#### Terms
- **Heartbeat:** a small call the device sends at a fixed interval to say it is alive.
- **Active window:** the hours when silence counts (e.g. opening hours).
- **Group connection:** one connection for several devices of a scope; it is down only when
  none of them answers.

#### Spec excerpt (authoritative for this issue)

"A connection is either an external system the platform calls or a device at a site that
calls the platform. For a device, the health signal is its heartbeat: each heartbeat counts
as a successful call and a missed heartbeat as a failed one, so `degrade`, `go_down` and
`recover` work the same way. A device connection belongs to a scope (outlet, branch, site)
and may have an active window (e.g. opening hours); outside the window silence is not
counted, and a connection that is `down` when its window closes stays `down` until a heartbeat
arrives but sends no new alerts until the window opens again. A heartbeat is missed when none arrives within one interval plus half an interval
of the previous one (or of the window start); that missed heartbeat is the failed call that
runs `degrade`. For a device, `failed since` is the time of the last heartbeat (or the
window start, if the last heartbeat was before it), so 'down after 5 minutes' means 5
minutes of silence. One connection may group the devices of a scope (down when none of them
answers). Consumers react to `IntegrationStatusChanged`."

| Setting | Default | Editable by client |
|---|---|---|
| Device connections: heartbeat interval, active window, grouping per scope | 1 minute; always; one connection per device | yes, per connection (06: kitchen devices per outlet, opening hours, down after 5 minutes) |

#### Edge cases decided
- **Missed heartbeat:** no heartbeat within one interval plus half an interval of the previous
  one (or of the window start) counts as one failed call and runs `degrade` (spec §4.5).
- **Where N counts from for a device:** `failed since` = the time of the last heartbeat (or the
  window start if that is later), not the time the miss was detected, so N is minutes of
  silence (spec §4.5). This refines BL-KRN-05's "from failed since" for devices.
- **Window opening:** silence counts from the window start, so a device off overnight goes down
  at window start + N minutes if it stays silent.
- **Window closing while down:** the connection stays `down` until a heartbeat arrives; no new
  alerts are sent outside the window.
- **Group:** one device answering keeps the group `up`; `degraded` when at least one device is
  silent; `down` when all have been silent for N minutes.
- **Time zone:** windows are in Asia/Dubai local time.

#### Must not
- Must not count silence outside the active window.
- Must not mark a group `down` while any device in it answers.

#### Done when
- [ ] Given a kitchen device at outlet "Marina" with interval 1 minute, window 10:00–23:00 and
      N = 5, whose last heartbeat arrives at 12:00:00, then the connection is `degraded` at
      12:01:30 with `failed since` = 12:00:00, and `down` at 12:05:00.
- [ ] A heartbeat at 12:07 runs `recover`.
- [ ] Silence from 23:00 to 10:00 produces no transition; silence continuing after 10:00 makes
      it `degraded` at 10:01:30 with `failed since` = 10:00:00 and `down` at 10:05:00.
- [ ] Given a group of 2 devices where one is silent, the group is not `down`.

#### Out of scope
`FNB.RUL.15` and other consumers (`Epic 06 F&B`); base health machine (BL-KRN-05).

---

### BL-KRN-08 — wl-kernel: alert the business owner when a compliance-critical rule fails or misses its schedule

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-kernel Rule run lifecycle, `PLT.RUL.03` digest · **Blocks:** BL-DOC-05, BL-TKT-03, BL-TKT-04, BL-PTY-07, BL-ACC-23, BL-AUT-04, BL-AUT-14, BL-AUT-18, BL-CLN-11, BL-CLN-12, BL-CLN-15, BL-CON-18, BL-EDU-07, BL-EDU-10, BL-EDU-17, BL-FNB-17, BL-LOG-25, BL-MFG-12, BL-MFG-14, BL-MPW-17, BL-MPW-20, BL-PRP-03, BL-PRP-25, BL-PRP-27
**Review items:** X-19 (owner alert)

#### Goal
Some automation rules protect a legal duty (sanctions re-screening, WPS alerts, the VAT due
date, document expiry blocks, fire-safety visit checks). When one of them fails, is stopped, or
does not run on schedule, the business owner role and the company admin are alerted at once, not
only in the next morning's digest, and the owner gets a task that stays open until the duty is
met.

#### Surfaces
- **Back office:** the business owner role and the company admin get an in-app alert at
  once; the owner role gets a task in "Needs your action" that closes on a successful rerun
  or when the owner records a required note (and optional attachment) of how the duty was
  met.
- **System:** `PLT.RUL.04` reacts to a failed or stopped compliance-critical run and to a
  missed occurrence found by the 15-minute scheduler check, once per occurrence, and
  `AutomationFailed` is emitted.

#### Files to read
- `docs/modules/wl-kernel.md` §3 (*Compliance-critical flag*), §4.2 *Rule run*, §5.15, §6
  (event `AutomationFailed`, rule `PLT.RUL.04`), §7 (settings)
- `docs/WORKFLOW.md` §8 (pattern "Compliance-critical rule failure")
- `docs/verticals/01-property.md` §7.2 and `docs/verticals/09-manpower.md` §7.2 (which rules are
  compliance-critical, owner role)

#### Terms
- **Compliance-critical flag:** set on a rule by its owning module; a vertical may add rules.
- **Owner role:** the vertical's business owner role (01: company admin; 09: MD).
- **Missed schedule:** a scheduled or date-reached rule occurrence for which no rule run exists.

#### Spec excerpt (authoritative for this issue)

Entity *Compliance-critical flag*: rule, critical (yes/no), owner alert recipients (role).

§5.15:
1. Rules whose failure can break a legal duty carry the compliance-critical flag.
2. If such a rule run `fail`s or is `stop`ped, or does not run on its schedule, `PLT.RUL.04`
   alerts the business owner role and the company admin at once (in-app and email), not only in
   the daily digest (`PLT.RUL.03`), and opens a task for the owner role.
3. The task closes when the rule has run successfully for the missed occurrence, or when the
   owner records how the duty was met by hand.
4. A run is missed when a scheduler check (every 15 minutes, at :00, :15, :30 and :45
   Asia/Dubai) finds an occurrence of a compliance-critical *Schedule* or *Date reached* rule
   whose planned time is 60 minutes or more in the past with no rule run. "At once" means at
   that check, and each occurrence counts as missed once.

| ID | Rule | When | Then | Parameter (default) |
|---|---|---|---|---|
| `PLT.RUL.04` | Compliance-critical failure alert | a compliance-critical rule run fails or is stopped, or a scheduled run is missed | alert the business owner role and the company admin at once; open a task for the owner role | on |

| Event | When | Payload |
|---|---|---|
| `AutomationFailed` | rule run failed or stopped | rule, subject, error, compliance-critical (yes/no) |

| Setting | Default | Editable by client |
|---|---|---|
| Compliance-critical rules | set by each module and vertical | no (set by us) |
| Owner alert recipients (`PLT.RUL.04`) | the vertical's owner role (e.g. MD, partner) + company admin | yes |

#### Edge cases decided
- **Missed schedule detection** (spec §5.15 step 4): a scheduler check at :00, :15, :30 and :45
  finds occurrences of compliance-critical *Schedule* and *Date reached* rules whose planned
  time is 60 minutes or more ago with no rule run; each counts as missed once. "At once" for a
  missed run means at that check (so at most 75 minutes after the planned time); for a
  `failed` or `stopped` run it means in the same job as the transition.
- **One alert and one task per failed or missed occurrence** (occurrence key), not per retry.
  `retrying` does not alert; only `failed` or `stopped` does.
- **Rerun succeeds** (`rerun` by `automations.retry`): the task closes automatically with "rule
  ran successfully at <time>".
- **Manual closure:** the owner records a note (and optional attachment) of how the duty was met;
  required to close manually.
- **The rule is not compliance-critical:** only the daily digest applies.
- **Time zone:** times in alerts are shown in Asia/Dubai.

#### Must not
- Must not let a client turn the compliance-critical flag off.
- Must not wait for the 07:00 digest before alerting.
- Must not close the task without a successful run or a recorded manual note.

#### Done when
- [ ] Given `HRM.RUL.03` marked compliance-critical in 09, when its run fails after 5 attempts,
      then the MD role and the company admin get an in-app alert and an email at once, the MD
      role has a task, and `AutomationFailed { compliance_critical: true }` is emitted.
- [ ] A stopped run (chain depth > 3) of a compliance-critical rule does the same.
- [ ] Given `DOC.RUL.03` scheduled nightly at 01:00 that has not run, the check at 02:00 finds
      it missed and the same alert and task are created at 02:00; the checks at 02:15 and
      02:30 create nothing more.
- [ ] Given the same rule planned at 01:10 with no run, the check at 02:00 does not find it
      (50 minutes) and the check at 02:15 does (65 minutes).
- [ ] When the failed run is rerun successfully, the task closes.
- [ ] Closing the task by hand without a note is refused.
- [ ] A failed run of a rule that is not compliance-critical produces no alert, only a digest
      line.

#### Out of scope
Marking each rule compliance-critical (module and vertical epics); the daily failed-run digest
`PLT.RUL.03` (base).

---

### BL-KRN-09 — wl-kernel: send managers a daily digest of voids and late entries

**Labels:** `module:wl-kernel` `area:negative-flow` `priority:medium` `size:s` `surface:system` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-KRN-03, BL-KRN-04 · **Blocks:** BL-LOG-25
**Review items:** X-19

#### Goal
Every morning, each manager receives one message listing the records voided and the late
entries made in their scope during the previous day, with the user and the reason, and under
each late entry the rule runs it lists as "already fired, review" (BL-KRN-04), so misuse of
void and late entry is visible and the fired automations get checked.

#### Surfaces
- **System:** `PLT.RUL.06` runs daily at 07:00 and sends each holder of
  `late_entries.approve` one digest per scope of the previous day's voids and late entries
  with the runs "already fired, review"; nothing is sent when empty, and a rerun sends
  nothing new.

#### Files to read
- `docs/modules/wl-kernel.md` §5.13 steps 5–6, §6 (`PLT.RUL.06`)

#### Terms
- **Manager in scope:** a user who holds `late_entries.approve` in a scope unit (building,
  branch, project) of the records, i.e. an approver of `PLT.ACL.10` (spec §5.13 step 6). The
  role names come from the vertical (e.g. property manager, branch manager); the kernel only
  checks the permission.
- **Digest:** one message per recipient summarising many records.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | When | Then | Parameter (default) |
|---|---|---|---|---|
| `PLT.RUL.06` | Voids and late entries digest | daily | send each manager the voids and late entries in their scope, with user and reason, and the rule runs already fired that each late entry lists for review | 07:00 |

§5.13 step 6: "A manager here is a user who holds `late_entries.approve` in a scope unit of the
records (the approvers of `PLT.ACL.10`); the digest also lists, under each late entry, the rule
runs it lists as 'already fired, review'. It covers 07:00 the previous day to 07:00 today
(Asia/Dubai), by the Void record time and the entry time; a manager with nothing in the period
gets no digest."

#### Edge cases decided
- **Period covered:** from 07:00 the previous day to 07:00 today, Asia/Dubai,
  by the Void record time and the entry time.
- **Empty digest:** not sent.
- **Repeated run:** idempotent per day (occurrence key = date); a rerun does not send twice.
- **A manager of several scopes** gets one digest with a section per scope.
- **Content** follows field visibility: a late entry on a confidential record shows only its type
  and number.

#### Must not
- Must not include records outside the manager's scopes.
- Must not send more than one digest per manager per day.

#### Done when
- [ ] Given 2 voids and 3 late entries in building "Tower A" between 2026-11-03 07:00 and
      2026-11-04 07:00, the manager of Tower A gets one digest at 07:00 on 2026-11-04 listing the
      5 records with user and reason.
- [ ] One of those late entries (check-in for booking B, business time 09:05) lists the no-show
      run of 09:15; the digest shows, under that entry, "already fired, review: no-show rule,
      booking B, 09:15".
- [ ] A user with a manager-like role but without `late_entries.approve` in Tower A gets no
      digest.
- [ ] The manager of "Tower B" with no such records gets nothing.
- [ ] Rerunning the rule for 2026-11-04 sends nothing new.

#### Out of scope
The void and late-entry mechanisms (BL-KRN-03, BL-KRN-04).
