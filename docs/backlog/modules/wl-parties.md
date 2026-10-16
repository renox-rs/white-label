# Epic: wl-parties — negative flows

> Status: draft · Last updated: 2026-10-15 · Spec version: same commit as this file

What happens when a party is blocked (sanctions, fraud, non-payment, legal dispute): restricted
reason codes that only the compliance role can see, the block effect on each kind of active
agreement, no new business and no money out without an exception, and unblocking. For
deployments with screening on (04), it adds the compliance-only review task, the freeze and
report tasks on a confirmed match, related companies, sanctions list freshness and the manual
list upload. Review items X-18, 04-N7-b, 04-N8-a, 04-N11-a, and the e-invoicing recipient
identifier needed by X-04.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-PTY-01 | Block a party with a reason code and hide restricted reasons | high | m | Build 1 | Base: wl-parties Party lifecycle |
| BL-PTY-02 | Unblock a party and release what the block paused | high | s | Build 1 | BL-PTY-01, BL-PTY-03 |
| BL-PTY-03 | Apply the block effect to each kind of active agreement | high | m | Build 1 | BL-PTY-01 |
| BL-PTY-04 | Refuse new agreements and money out to a blocked party unless an exception is approved | high | m | Build 1 | BL-PTY-01, BL-KRN-01 |
| BL-PTY-05 | Send a possible sanctions match to the compliance role only | high | s | Launch 04 | BL-PTY-01; Base: wl-parties Screening lifecycle |
| BL-PTY-06 | Block, freeze and report on a confirmed sanctions match | high | m | Launch 04 | BL-PTY-01, BL-PTY-04, BL-PTY-05 |
| BL-PTY-07 | Alert compliance and re-run screenings when a sanctions list is stale | high | m | Launch 04 | Base: wl-parties Screening lifecycle; BL-KRN-08 |
| BL-PTY-08 | Warn or block onboarding while a sanctions list is stale | medium | s | Launch 04 | BL-PTY-07 |
| BL-PTY-09 | Upload an official sanctions list by hand when the download fails | high | s | Launch 04 | BL-PTY-07 |
| BL-PTY-10 | Record a company's e-invoicing recipient identifier | medium | s | Build 1 | BL-KRN-05; Base: wl-parties Party lifecycle |

---

### BL-PTY-01 — wl-parties: block a party with a reason code and hide restricted reasons

**Labels:** `module:wl-parties` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-parties Party lifecycle · **Blocks:** BL-PTY-02, BL-PTY-03, BL-PTY-04, BL-PTY-05, BL-PTY-06, BL-POR-01, BL-ACC-07, BL-ACC-23, BL-EDU-04, BL-LOG-19, BL-MPW-15, BL-STK-30
**Review items:** X-18, 04-N11-a

#### Goal
A user with `parties.block` (or the system on a confirmed sanctions match) blocks a party with a
reason code and a note. Every block keeps a Block record. Reason codes marked *restricted*
(`sanctions`, `aml_concern`) are visible only to users with `parties.block_reason.view`;
everyone else, every event and every agreement sees only the neutral code `on_hold`, shown as
**On hold**. No automatic message tells the party. This prevents tipping off.

#### Files to read
- `docs/modules/wl-parties.md` §3 (*Block record*), §4 *Party* (states, `block`), §5.6 steps 1
  and 3, §6 (event `PartyBlocked`, permissions, `PTY.ACL.02`), §7 (block reason codes), §10 (No
  tipping off)
- `docs/ACCESS-CONTROL.md` §3 (patterns `SEN`, `FLD`)

#### Terms
- **Reason code:** a configured code for why a party is blocked; each has a *restricted* flag.
- **Restricted reason:** `sanctions`, `aml_concern` by default; a vertical may add more.
- **Neutral code `on_hold`:** always present; what everyone without `PTY.ACL.02` sees.
- **Block record:** the trace of one `block`, with the real reason.
- **Tipping off:** revealing to the party (or to staff who might tell them) that they are
  suspected; forbidden under the AML law.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `blocked` | error | no | New business is not allowed (confirmed sanctions match, fraud, compliance hold). Existing records stay visible. Shown as **On hold** with the neutral reason to users without `PTY.ACL.02`; never shown in the portal |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `block` | active | blocked | `parties.block`, or system (screening `confirm_match`) | reason code and note given; a restricted reason code needs `parties.block_reason.view` (`PTY.ACL.02`) | Block record kept; event `PartyBlocked` with the display reason (the neutral code `on_hold` when the real code is restricted); modules refuse new orders, leases, bookings and invoices for the party and apply the block effect per active agreement (§5.6); money out to the party stopped (`PTY.ACL.03`); no automatic message to the party |

Entity *Block record*: one per `block`, and one per confirmed screening match on a party that is
already `blocked` or `archived` (added without a transition); party, reason code, restricted
(from the reason code), note, blocked by, blocked at, unblocked by, unblocked at, unblock note.
Restricted records are readable only under `PTY.ACL.02`.

§5.6 step 4: "`block` runs only from `active`. When `confirm_match` hits a party (or a related
company) that is already `blocked`, the system adds a new restricted Block record `sanctions`
without a transition; the party stays `blocked`, and the freeze and report tasks name it. When
the party is `archived`, it stays `archived`; a restricted Block record is still created and
named in the freeze task, and a later `restore` brings the party back as `blocked`, never
`active`. `unblock` closes every open Block record of the party, so its guards apply to each of
them (restricted reason, sanctions evidence, SoD)."

| Event | When | Payload |
|---|---|---|
| `PartyBlocked` / `PartyUnblocked` | status change | party, display reason code (`on_hold` when the real code is restricted; the real code stays in the Block record) |

| Permission | Allows |
|---|---|
| `parties.block` | Block and unblock |
| `parties.block_reason.view` | See and use restricted block reason codes and Block records; see compliance-only tasks (MLRO and deputy by default; `PTY.ACL.02`) |

`PTY.ACL.02`: restricted block reasons (sanctions, AML) are visible only with
`parties.block_reason.view`; everyone else sees **On hold** and the neutral code, and the portal
sees no block at all (`SEN`, `FLD`; no tipping off).

Setting: Block reason codes (restricted flag): `on_hold` (neutral, always present),
`non_payment`, `legal_dispute`, `fraud_suspected`, `other`; restricted: `sanctions`,
`aml_concern`. A vertical can add codes and mark them restricted.

§5.6 step 3: the pause or hold reason recorded on each agreement is the neutral `on_hold`, never
the real reason. Automatic messages to the party that would announce the pause are suppressed.

#### Edge cases decided
- **History row of the `block` transition:** for a restricted code the
  history reason stores `on_hold`; the real code and note live only in the Block record.
- **Non-restricted codes** (`non_payment`, `legal_dispute`, `fraud_suspected`, `other`) are shown
  to every user who can see the party; the event still carries the code itself.
- **Search, lists, exports and the party card** show "On hold" for a restricted block to users
  without the permission; filters by reason code do not offer restricted codes to them.
- **Already blocked:** `block` runs only from `active`. A second reason on a blocked party (a
  sanctions match on a party already on hold for non-payment) is added as a new Block record by
  the system without a transition; the party stays `blocked`; see BL-PTY-02 for unblocking.
  Users cannot add a second Block record by hand.
- **Archived party:** `block` is not offered; the system case (restricted Block record, `restore`
  lands in `blocked`) is handled in BL-PTY-06.
- **`on_hold` cannot be disabled or marked restricted** by a client.

#### Must not
- Must not put a restricted code in an event, a history row, an agreement reason, a document or
  any portal page.
- Must not send any automatic message to the party because of the block.
- Must not let a user without `parties.block_reason.view` choose a restricted code.

#### Done when
- [ ] Given party "Al Noor Trading" `active`, when user A (with `parties.block`) blocks it with
      `non_payment` and note "3 bounced cheques", then it is `blocked`, a Block record exists,
      `PartyBlocked { display_reason: non_payment }` is emitted and staff see "On hold —
      non_payment".
- [ ] When the MLRO blocks a party with `aml_concern`, the event carries `on_hold`, the history
      row says `on_hold`, and user B without the permission sees only "On hold".
- [ ] User B cannot see the Block record or pick `sanctions` / `aml_concern` in the block dialog.
- [ ] `block` without a note is refused.
- [ ] No message is queued to the party's contacts by the block.
- [ ] A party export by user B shows "On hold" for the restricted block.

#### Out of scope
Effects on agreements (BL-PTY-03); money out and new agreements (BL-PTY-04); unblock
(BL-PTY-02); the portal (BL-POR-01); screening (BL-PTY-05, BL-PTY-06).

---

### BL-PTY-02 — wl-parties: unblock a party and release what the block paused

**Labels:** `module:wl-parties` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PTY-01, BL-PTY-03 · **Blocks:** BL-ACC-05
**Review items:** X-18

#### Goal
A second user unblocks a party. A restricted block (sanctions, AML) can only be lifted by a user
who may see restricted reasons, and a sanctions block only when the party is no longer listed or
the match was found false, with evidence. On unblock, agreements paused by the block resume and
agreements under review go back to their deciders.

#### Files to read
- `docs/modules/wl-parties.md` §4 *Party* (`unblock`), §5.6 steps 2 and 4, §6 (event
  `PartyUnblocked`)

#### Terms
- **SoD:** segregation of duties: the user who blocked cannot unblock.
- **Paused agreement:** an agreement whose block effect was *pause*.
- **Agreement under review:** an agreement whose block effect was *review*, with a decider task.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `unblock` | blocked | active | `parties.block`; for a restricted reason code also `parties.block_reason.view` | reason given; approver ≠ the user who blocked each open Block record (`SOD`; for a block made by `confirm_match`, the reviewer who ran `confirm_match`); for a sanctions block, the party is no longer listed or the match was found false (note and evidence recorded) | every open Block record closed; event `PartyUnblocked`; agreements paused by the block resume and agreements under review are returned to their deciders (§5.6) |

#### Edge cases decided
- **Several open Block records** (BL-PTY-01 edge case, §5.6 step 4): `unblock` closes all of
  them; it needs `parties.block_reason.view` if any is restricted, and the sanctions evidence if
  any is `sanctions`. SoD applies against every "blocked by" user.
- **System block** (`confirm_match`): "blocked by" is the reviewer who ran `confirm_match` (the
  approved action runs as the requester); that user cannot unblock. The second reviewer who
  approved the match may unblock, so a compliance team of MLRO and deputy can still lift it.
- **Evidence:** a note plus at least one attached file (e.g. the delisting notice or the review
  note).
- **Agreements already ended by their decider** while blocked stay ended.
- **Under-review tasks still open** are updated with "party unblocked" and reassigned to the
  decider; they are not closed automatically.
- **Pending money-out exception requests** (BL-PTY-04) are left as they are; once unblocked the
  normal path is available.

#### Must not
- Must not let the user who blocked the party unblock it.
- Must not resume an agreement that its decider ended.
- Must not reveal the restricted reason in `PartyUnblocked`.

#### Done when
- [ ] Given a party blocked by user A with `non_payment`, user A cannot unblock it; user B
      unblocks with reason "paid in full"; the party is `active`, the Block record is closed with
      B and the note, and `PartyUnblocked { display_reason: non_payment }` is emitted.
- [ ] Given a `sanctions` block, user C with `parties.block` but not
      `parties.block_reason.view` cannot unblock; the deputy MLRO can only after recording
      "match found false" with an attached file.
- [ ] After unblock, a subscription paused by the block resumes (its owner module receives
      `PartyUnblocked`), and a review task for a booking list is returned to the clinic manager.
- [ ] `unblock` without a reason is refused.
- [ ] Given a `sanctions` block made by `confirm_match` run by the MLRO and approved by the
      deputy, the MLRO cannot unblock; the deputy can, with the evidence.
- [ ] Given a party restored from `archived` into `blocked` (BL-PTY-06), `unblock` by user B
      closes the open Block record and the party is `active`.

#### Out of scope
How each module resumes its agreements (module epics, e.g. `Epic wl-invoicing: subscription
resume after unblock`).

---

### BL-PTY-03 — wl-parties: apply the block effect to each kind of active agreement

**Labels:** `module:wl-parties` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PTY-01 · **Blocks:** BL-PTY-02, BL-TKT-08, BL-ACC-03, BL-ACC-23, BL-AUT-17, BL-CON-03, BL-CON-17, BL-EDU-04, BL-EDU-17, BL-MFG-14, BL-MPW-15, BL-MPW-20, BL-PRP-26
**Review items:** X-18, 04-N8-a

#### Goal
Blocking a party does not end anything by itself. Each kind of agreement has a configured block
effect (*pause*, *continue* or *review*) and a decider. wl-parties holds this configuration,
exposes it through the `BlockEffects` service (the `PartyBlocked` payload is unchanged), and for
*review* gives the decider a task. Modules that own the
agreements apply the effect. Any ending is a decision of the decider through the agreement's own
transition.

#### Files to read
- `docs/modules/wl-parties.md` §5.6 (steps 2–3 and the table), §6 (service `BlockEffects`), §7
  (setting "Block effect and decider per agreement kind")
- `docs/WORKFLOW.md` §8 (pattern "Party blocked")

#### Terms
- **Agreement kind:** a type of ongoing relationship owned by a module or vertical (lease,
  engagement, subscription, enrolment, deployment, project, sales orders, bookings, service
  contract, money out, money in).
- **pause:** the agreement stays, service and billing stop; it resumes on `unblock`.
- **continue:** it runs to its normal end; renewal and extension are refused.
- **review:** nothing changes on its own; the decider gets a task to pause, continue or end.
- **Decider:** the role that may end the agreement while the party is blocked.

#### Spec excerpt (authoritative for this issue)

§5.6 step 2: every module that holds agreements with the party reacts to `PartyBlocked` with the
block effect configured for that kind of agreement (pause / continue / review). No agreement is
ended automatically. Stopping (ending) an agreement is always a decision of the decider listed
below, through the agreement's own `end`, `cancel` or `withdraw` transition, with its normal
money effects, except that money out to the party waits for unblock or an approved exception
(`PTY.ACL.03`).
Step 3: the pause or hold reason recorded on each agreement is the neutral `on_hold`. Automatic
messages to the party that would announce the pause are suppressed; the decider chooses what to
tell the party, with a neutral template.

| Agreement kind | Owner | Default effect | Stop (end) decided by | Verticals |
|---|---|---|---|---|
| Engagement | 04 vertical | pause (engagement `suspend`; tasks frozen) | compliance role (MLRO), then the engagement's normal `end` approver | 04 |
| Subscription, retainer | `wl-invoicing` | pause (system `pause`, pause allowance ignored; no cycle invoices, no automatic charge) | `subscriptions.manage` holder named by the vertical | 04, 06, 10, others |
| Enrolment | 10 vertical | pause (`freeze` without the freeze-day limit; seat kept) | centre manager | 10 |
| Lease | 01 vertical | continue (the tenant's rights under the lease do not end because of a block); renewal and new leases refused | property manager; ending a lease follows the tenancy law and the 01 process | 01 |
| Deployment | 09 vertical | continue for `active`; `planned` ones are not activated | the vertical's decider (09: MD), per the client contract | 09 |
| Project | 02 vertical | continue: work, IPAs and variations go on; no new project is started for the party; money out stops (`PTY.ACL.03`) | the vertical's decider (02: MD), through the project's `terminate` | 02 |
| Sales orders | `wl-sales` | pause: orders not yet dispatched are held (no dispatch, no new invoice); dispatched orders complete | sales manager (`cancel` or release after unblock) | 03, 06, 07, 08 |
| Bookings | `wl-scheduling` | review: future bookings listed for the decider; no new bookings | branch or clinic manager | 05, 10 |
| Purchase orders (party as supplier) | `wl-inventory` | continue: issued POs may still be received; no new PO and no revision that raises the total or extends the delivery date; supplier payments stop (`PTY.ACL.03`) | procurement (`cancel` or `short_close`) | 02, 06, 07, 08 |
| Service contracts (party as vendor) | `wl-ticketing` | review: no new visits assigned; planned visits listed for reassignment | facility manager | 01, 02 |
| Money out (refunds, payouts, deposit returns, supplier payments) | `wl-payments` | stop until unblock (`PTY.ACL.03`) | compliance role, through an approved exception | all |
| Money in | `wl-payments` | continue: receipts are recorded; for a restricted reason they are not refunded or paid on, and the compliance role decides | compliance role | all |

"The verticals set the effect and the decider for their own agreement kinds; the defaults above
apply when they do not."

`BlockEffects` service (§6): "Owning modules register each agreement kind with its default
block effect and decider role; `effect_for(kind, scope)` returns the effect (pause / continue /
review) and the decider role after the vertical's override (§5.6); modules call it when they
handle `PartyBlocked`."

#### Edge cases decided
- **Registration:** each owning module registers its agreement kind with a default effect and
  decider role; the vertical's configuration overrides them. Unknown kinds default to *review*
  (the safest of the three).
- **Review task:** wl-parties opens one task per agreement kind per party
  for the decider role in the agreement's scope, listing the open agreements (from the owning
  module's list provider) with the actions pause / continue / end. For a restricted reason the
  task text shows only "On hold".
- **Neutral template:** wl-parties provides one notification type `party.on_hold.neutral` (EN/AR)
  the decider may send; it says the account is on hold and asks the party to contact the
  business, with no reason.
- **Suppressing messages:** owning modules pass the `on_hold` reason to their own pause
  transitions; notification types they mark "announces pause" are not sent while the party is
  blocked.
- **Repeated `PartyBlocked`** (a second Block record): no second review task if one is open.
- **Agreements created after the block** cannot exist (BL-PTY-04).

#### Must not
- Must not end any agreement automatically.
- Must not record the real reason on an agreement.
- Must not send an automatic message announcing the pause.

#### Done when
- [ ] With defaults, the configuration lists the 12 agreement kinds above with their effect and
      decider; 09's override "deployment: continue; decider MD" is shown for a 09 deployment.
- [ ] Given a blocked patient with 3 future bookings, the clinic manager gets one task listing
      the 3 bookings with pause / continue / end; for a `sanctions` block the task says "On hold".
- [ ] Given `wl-invoicing` registered kind `subscription` (default pause, decider
      `subscriptions.manage` holder), when it handles `PartyBlocked` for a party in branch
      "Marina", `BlockEffects::effect_for("subscription", Marina)` returns
      `{ effect: pause, decider_role: <subscriptions.manage holder named by the vertical> }`;
      in a 09 deployment `effect_for("deployment", scope)` returns `{ continue, MD }`.
- [ ] `effect_for` for a kind nobody registered returns `{ effect: review }` (Default above).
- [ ] No message is sent to the party; the neutral template is available to the decider.

#### Out of scope
Each owner's reaction: `Epic wl-invoicing: subscription pause on block`, `Epic wl-sales: order
hold on block`, `Epic wl-scheduling: booking review on block`, `Epic wl-payments: money in for
restricted blocks`, the vertical epics (engagement, enrolment, lease, deployment, project);
service contracts (BL-TKT-08); money out (BL-PTY-04).

---

### BL-PTY-04 — wl-parties: refuse new agreements and money out to a blocked party unless an exception is approved

**Labels:** `module:wl-parties` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-PTY-01, BL-KRN-01 · Base: wl-kernel Approval request lifecycle · **Blocks:** BL-PTY-06, BL-POR-01, BL-TKT-08, BL-ACC-01, BL-ACC-06, BL-ACC-23, BL-CON-03, BL-EDU-04, BL-LOG-19, BL-LOG-26, BL-PAY-24, BL-PRP-15, BL-PRP-20, BL-PRP-26, BL-STK-27
**Review items:** X-18, 04-N8-a

#### Goal
While a party is blocked, no module starts, renews or extends an agreement with it, and no money
goes out to it (refund, payout, deposit return, supplier payment). wl-parties provides one guard
that every module calls, and an exception path: the compliance role (or the finance manager when
screening is off) approves one specific action.

#### Files to read
- `docs/modules/wl-parties.md` §4 *Party* (`block` effects), §5.6, §6 (`PTY.ACL.03`, permission
  `parties.block_exception`), §7 (exception approver)
- `docs/modules/wl-kernel.md` §4.1, §5.2 (approval requests)

#### Terms
- **Money out:** refund, payout, deposit return, supplier payment to the party.
- **Exception:** an approval request that lets one blocked action run once.
- **Compliance role:** MLRO and deputy by default. It holds `parties.block_reason.view`,
  `parties.block_exception` and `screening.review`; approving an exception checks
  `parties.block_exception` (spec §6, paragraph "Compliance role").

#### Spec excerpt (authoritative for this issue)

`PTY.ACL.03`: while a party is blocked, no module starts, renews or extends an agreement with it
or pays money out to it (refund, payout, deposit return, supplier payment) unless the compliance
role approves an exception (`STA`, `LIM`).

| Permission | Allows |
|---|---|
| `parties.block_exception` | Approve an exception to `PTY.ACL.03` (compliance role) |

| Setting | Default |
|---|---|
| Exception approver for money out to a blocked party (`PTY.ACL.03`) | compliance role; finance manager when screening is off |

`block` effect: "modules refuse new orders, leases, bookings and invoices for the party".
§5.6: ending an agreement keeps its normal money effects, "except that money out to the party
waits for unblock or an approved exception".

#### Edge cases decided
- **One guard:** `BlockGuard::check(party, action_kind)` where action_kind is `start`, `renew`,
  `extend` or `money_out`; modules call it in their transitions. It returns allowed, or blocked.
  The refusal shown to a user without `parties.block_reason.view` says only that the party is
  on hold (neutral code `on_hold`); a user with it sees the real reason code (e.g. "Blocked:
  sanctions") and a link to the open Block record(s), as `PTY.ACL.02` allows. Both see how to
  request an exception.
- **Exception = approval request** for that one action and payload (kernel); on approval the
  action runs once as the requester. Amount changes after approval need a new request (the
  kernel re-checks guards and `fail`s).
- **Blocked while a money-out request is already pending approval:** the final approval re-checks
  the guard and fails (BL-KRN-01); the refund goes back to its stored state.
- **Money out already sent to the bank** before the block is not recalled by the platform.
- **Money in** (receipts) is never refused by this guard.
- **Scheduled money out** (e.g. owner payouts drafted monthly): the guard runs at the step that
  releases money, not at drafting.
- **Exception approver** cannot be the requester (SoD on).

#### Must not
- Must not let any module skip the guard for start, renew, extend or money out.
- Must not refuse receipts from the party.
- Must not show a restricted reason in the refusal message.

#### Done when
- [ ] Given party "Gulf Spare Parts" blocked with `fraud_suspected` and screening off, when the
      accountant runs a supplier payment of AED 8,400, then an approval request goes to the
      finance manager and no payment exists.
- [ ] The finance manager approves; the payment of AED 8,400 runs once; a second payment needs a
      new request.
- [ ] Given screening on and a `sanctions` block, the refusal shown to a user without
      `PTY.ACL.02` says only that the party is on hold; the approver is the compliance role.
      The same refusal shown to the MLRO says "Blocked: sanctions" with a link to the Block
      record.
- [ ] Renewing a lease and creating a new booking for the blocked party are refused.
- [ ] A receipt of AED 2,000 from the blocked party is recorded normally.
- [ ] A refund pending approval when the party is blocked fails at final approval.

#### Out of scope
Each module's call to the guard (module epics); what happens to money in for restricted reasons
(`Epic wl-payments: money in from a blocked party`).

---

### BL-PTY-05 — wl-parties: send a possible sanctions match to the compliance role only

**Labels:** `module:wl-parties` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** BL-PTY-01 · Base: wl-parties Screening lifecycle · **Blocks:** BL-PTY-06
**Review items:** 04-N8-a, 04-N11-a

#### Goal
When screening finds a candidate at or above the match threshold, the compliance role gets a
review task visible only under `PTY.ACL.02`, due 24 hours after the flag. Until it is decided,
nothing visible changes for other staff, so a possible match is never revealed.

#### Files to read
- `docs/modules/wl-parties.md` §4 *Screening* (`flag`), §5.4 step 5, §6 (`PTY.RUL.01`)

#### Terms
- **Possible match:** a candidate with similarity ≥ the threshold (default 0.85).
- **Compliance-only task:** a task visible only to holders of `parties.block_reason.view`.
- **Compliance role / compliance officer:** the same role, MLRO and deputy by default; the task
  and the alert go to every holder of `parties.block_reason.view`.
- **Alert at once:** an in-app alert and an email sent when `flag` commits.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `flag` | pending | possible_match | system; for a manual check, the result recorded by `screening.review` | candidate ≥ threshold (manual: the reviewer records a candidate with the list entry) | task for the compliance role, visible only under `PTY.ACL.02`, due 24 hours after the flag (`PTY.RUL.01`); when a task for the party is already open, no second task: the new run is linked to it; event `ScreeningCompleted`, delivered only to compliance listeners for `possible_match` (other listeners get nothing until the decision) |

§5.4 step 5: "The compliance role gets a task visible only under `PTY.ACL.02`, due 24 hours after
the flag. Until it is decided, nothing visible changes for other staff."

`PTY.RUL.01`: possible hits alert the compliance officer at once (24-hour freeze duty under
Cabinet Decision 74 of 2020).

**Compliance role** (`wl-parties.md` §6, paragraph "Compliance role"): "'Compliance role' and
'compliance officer' in this document mean the same role: the MLRO and the deputy by default
(the vertical may name it otherwise). It holds `parties.block_reason.view`,
`parties.block_exception` and `screening.review`, and each action checks its own permission
[…]. Compliance tasks and alerts go to every user holding `parties.block_reason.view`; an alert
'at once' is an in-app alert and an email sent when the event happens."


#### Edge cases decided
- **Due time:** flag time + 24 elapsed hours (not working hours), shown in Asia/Dubai.
- **`ScreeningCompleted`** for `possible_match` is delivered only to compliance listeners;
  other listeners receive nothing until the decision.
- **The party's page** for other staff shows no badge, count or status change.
- **Several candidates** in one run: one task listing all.
- **Re-screening while a task is open:** no second task; the new run is
  linked to the open one.

#### Must not
- Must not show the task, its count or its due date to users without the permission.
- Must not block the party at `flag`.

#### Done when
- [ ] Given a screening of "Omar K." flagged at 2026-11-03 14:00 with a candidate at 0.91, the
      MLRO and deputy (the holders of `parties.block_reason.view`) have one task due
      2026-11-04 14:00, and each gets an in-app alert and an email at 14:00.
- [ ] A partner without `parties.block_reason.view` sees no task, no badge and the party
      `active`.
- [ ] A re-screen at 18:00 the same day adds no second task.

#### Out of scope
`dismiss` (base); confirmed match (BL-PTY-06).

---

### BL-PTY-06 — wl-parties: block, freeze and report on a confirmed sanctions match

**Labels:** `module:wl-parties` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-PTY-01, BL-PTY-04, BL-PTY-05 · **Blocks:** BL-ACC-10, BL-ACC-23
**Review items:** 04-N8-a, X-18

#### Goal
When two reviewers confirm a sanctions match, the party is blocked with the restricted reason
`sanctions`; a company whose UBO, director or signatory matched is blocked too; and the
compliance role gets a freeze task and a report task, both restricted. The law requires freezing
within 24 hours and notifying the supervisor; the exact start of the 24 hours and the report form
are open (L-30), so the tasks use the safest default.

#### Files to read
- `docs/modules/wl-parties.md` §4 *Screening* (`confirm_match`), §4 *Party* (`restore`), §5.4
  steps 6 and 9, §5.6 step 4, §7
  (settings for related persons, freeze and report task due), §10 (Freeze and report)
- `docs/OPEN-ITEMS.md` L-30

#### Terms
- **UBO:** ultimate beneficial owner (ownership record marked UBO).
- **Related company:** a company party where the matched person is a UBO, director or signatory
  (ownership or relationship records).
- **Freeze task / report task:** compliance-only tasks (visible under `PTY.ACL.02`).
- **goAML:** the UAE channel for suspicious transaction reports.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `confirm_match` | possible_match | confirmed_match | `screening.review` | review note given; second reviewer approves (`SOD`, `PLT.APR.01`) | party `block` with reason code `sanctions` (restricted); when the screened party is a UBO, director or signatory of a company party (ownership or relationship records), that company is blocked too with the same restricted reason (setting, on); a party or company already `blocked` or `archived` gets a new restricted Block record `sanctions` without a transition (§5.6 step 4); freeze task and report task for the compliance role (§5.4); event `ScreeningMatchConfirmed`; never shown to portal users or client-facing staff |

§5.4 step 6:
- **Freeze task**, due 24 hours after the possible match was flagged: confirm that the block took
  effect, list the funds and assets the business holds for the party and its agreements (client
  money, deposits, retainers, prepaid packages, open refunds; pre-filled from the modules' open
  balances, confirmed by hand by the compliance role), and record the freeze time. Money
  out is already refused (`PTY.ACL.03`). Whether the 24 hours run from the list update or from
  the match is `⚠ open: lawyer (L-30)`, so the task uses the earlier start (the flag) by default.
- **Report task**: notify the supervisor of the freeze and record the reference and date. The
  exact report form and its deadline are `⚠ open: lawyer (L-30)`, so the task's due date is a
  parameter (default: same 24 hours).
Step 9: a confirmed match on a UBO, director or signatory blocks the company they belong to as
well (freeze and report tasks name both).

§5.6 step 4: "`block` runs only from `active`. When `confirm_match` hits a party (or a related
company) that is already `blocked`, the system adds a new restricted Block record `sanctions`
without a transition; the party stays `blocked`, and the freeze and report tasks name it. When
the party is `archived`, it stays `archived`; a restricted Block record is still created and
named in the freeze task, and a later `restore` brings the party back as `blocked`, never
`active`. `unblock` closes every open Block record of the party, so its guards apply to each of
them (restricted reason, sanctions evidence, SoD)."

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `restore` | archived | active, or blocked when the party has an open Block record | `parties.archive` | — | event `PartyRestored`; when it lands in `blocked`, also `PartyBlocked` with the display reason, and only `unblock` (with its guards) makes the party `active` |

| Setting | Default |
|---|---|
| Second reviewer for confirmed match | on |
| Confirmed match on a UBO, director or signatory also blocks the company party | on |
| Freeze task due | 24 hours after the possible match was flagged |
| Report task due | 24 hours after the possible match was flagged (`⚠ open: lawyer (L-30)` on the real deadline) |

Event `ScreeningMatchConfirmed`: party, screening (delivered only to compliance listeners).

#### Edge cases decided
- **Party already blocked** for another reason: a new restricted Block record `sanctions` is
  added without a transition (BL-PTY-01); the party stays `blocked`.
- **Party or related company archived:** no state change; a restricted Block record is still
  created and named in the freeze task; a later `restore` lands in `blocked` (not `active`) and
  emits `PartyBlocked` with `on_hold`; only `unblock`, with the sanctions evidence, makes it
  `active`.
- **Several related companies:** each is blocked; one freeze task and one report task name all
  of them.
- **Due times** are elapsed hours from the flag (Asia/Dubai display); a confirmation made after
  the 24 hours creates tasks already overdue.
- **Listing funds:** the freeze task is pre-filled from modules' open balances for the party
  (deposits, prepaid credits, open refunds); the compliance role confirms
  the list by hand.
- **Second reviewer rejects:** the screening stays `possible_match` (BL-KRN-01).

#### Must not
- Must not show the match, the tasks or the reason to client-facing staff or the portal.
- Must not send any message to the party.
- Must not hard-code the report form or deadline: it is a parameter until L-30 is answered.

#### Done when
- [ ] Given a screening flagged at 2026-11-03 14:00, when reviewer A confirms at 2026-11-04 09:00
      and reviewer B approves, then the party is `blocked` with a restricted `sanctions` Block
      record, a freeze task and a report task are due 2026-11-04 14:00, and
      `ScreeningMatchConfirmed` reaches compliance listeners only.
- [ ] Given the matched person is UBO of "Falcon Holdings LLC", that company is blocked with the
      same restricted reason and both tasks name both parties.
- [ ] With the related-company setting off, only the person is blocked.
- [ ] A partner without the permission sees both parties "On hold" and no tasks.
- [ ] Reviewer A cannot be the second reviewer.
- [ ] Changing the report task due parameter to 48 hours changes the due time of new tasks.
- [ ] Given the matched person's party is already `blocked` for `non_payment`, a second,
      restricted Block record `sanctions` is added, the party stays `blocked` and no second
      `block` transition appears in its history.
- [ ] Given "Falcon Holdings LLC" is `archived`, it stays `archived` with an open restricted Block
      record named in the freeze task; when it is later restored, it becomes `blocked`, not
      `active`.

#### Out of scope
The vertical's AML case (`Epic 04 Accounting: ACC.RUL.12`); goAML filing itself.

---

### BL-PTY-07 — wl-parties: alert compliance and re-run screenings when a sanctions list is stale

**Labels:** `module:wl-parties` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** Base: wl-parties Screening lifecycle; BL-KRN-08 · **Blocks:** BL-PTY-08, BL-PTY-09, BL-ACC-08, BL-ACC-23
**Review items:** 04-N7-b

#### Goal
Screening is only as good as its lists. Each sanctions list records its last successful update
and version. If a download fails, or a list has not been updated for 24 hours, the compliance
officer is alerted at once, the download is retried hourly, screenings run meanwhile are marked
as made on a stale list, and they are re-run after the next successful update.

#### Files to read
- `docs/modules/wl-parties.md` §3 (*Sanctions list*, *Screening* stale list flag), §5.4 step 7,
  §6 (events `SanctionsListStale`, `SanctionsListUpdated`, `PTY.RUL.02`), §7 (freshness limit),
  §10 (List freshness)

#### Terms
- **Sanctions list:** one source (UN Consolidated List, UAE Local Terrorist List).
- **Stale:** a list whose last download failed or whose last successful update is older than the
  freshness limit.
- **Stale list flag:** on a Screening, set when any list used was stale.
- **Compliance officer:** the compliance role (MLRO and deputy by default); the alert goes to
  every holder of `parties.block_reason.view` (spec §6, paragraph "Compliance role").

#### Spec excerpt (authoritative for this issue)

Entity *Sanctions list*: name, source, format, last download attempt, last successful update,
list version or date, update source (download / manual upload, uploaded by), status (`current` /
`stale`), last error.
Screening field: stale list flag (a list used was older than the freshness limit, `PTY.RUL.02`).

`PTY.RUL.02`: a sanctions list download fails, or a list has had no successful update for N hours
(default 24): alert the compliance officer at once (in-app and email), retry hourly, mark
screenings run on the stale list and re-run them after the next update. Compliance-critical, like
`PTY.RUL.01` (a failed run alerts the owner, wl-kernel §5.15). Enabled when screening is on.

§5.4 step 7: each Sanctions list shows its last successful update and version on the screening
page and on every Screening record. A download that returns the version already held counts as a
successful update (it refreshes the last successful update and clears the stale status) but emits
no `SanctionsListUpdated`, so nothing is re-screened.

| Event | When | Payload |
|---|---|---|
| `SanctionsListStale` | a list download failed, or a list is older than the freshness limit (`PTY.RUL.02`) | list, last successful update, last error (delivered only to compliance listeners) |
| `SanctionsListUpdated` | a list downloaded successfully, or uploaded by hand, with a new version | list, version; triggers `PTY.RUL.01` |

Setting: Sanctions list freshness limit (`PTY.RUL.02`): 24 hours.

**Compliance role** (`wl-parties.md` §6, paragraph "Compliance role"): "'Compliance role' and
'compliance officer' in this document mean the same role: the MLRO and the deputy by default
(the vertical may name it otherwise). It holds `parties.block_reason.view`,
`parties.block_exception` and `screening.review`, and each action checks its own permission
[…]. Compliance tasks and alerts go to every user holding `parties.block_reason.view`; an alert
'at once' is an in-app alert and an email sent when the event happens."


#### Edge cases decided
- **Download succeeds with the same version:** counts as a successful
  update (refreshes "last successful update", clears `stale`) but emits no
  `SanctionsListUpdated`, so no mass re-screening.
- **Alert once per stale period**, not per hourly retry; a new alert after the list was current
  again and went stale again.
- **Re-run:** after the next `SanctionsListUpdated`, every Screening with the stale flag since
  the last current version is re-run as a new Screening (trigger `list update`).
- **Compliance-critical:** `PTY.RUL.02` is marked compliance-critical like
  `PTY.RUL.01`, so a failed run alerts the owner (BL-KRN-08).
- **Freshness check** runs hourly; 24 hours is elapsed time.

#### Must not
- Must not run screening silently on a stale list without the flag.
- Must not deliver `SanctionsListStale` to non-compliance listeners.

#### Done when
- [ ] Given the UN list last updated 2026-11-02 06:00, at 2026-11-03 06:00 it becomes `stale`,
      the MLRO and the deputy (holders of `parties.block_reason.view`) each get an in-app
      alert and an email, and `SanctionsListStale` is emitted.
- [ ] A download failure at 2026-11-03 09:00 records the error and retries at 10:00, 11:00…
      without new alerts.
- [ ] A screening at 2026-11-03 12:00 carries the stale list flag and shows the list date.
- [ ] When the download succeeds with a new version at 15:00, the list is `current`,
      `SanctionsListUpdated` is emitted and the 12:00 screening is re-run.

#### Out of scope
Manual upload (BL-PTY-09); onboarding guard (BL-PTY-08); `PTY.RUL.01` itself (base).

---

### BL-PTY-08 — wl-parties: warn or block onboarding while a sanctions list is stale

**Labels:** `module:wl-parties` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** BL-PTY-07 · **Blocks:** BL-ACC-01, BL-ACC-23
**Review items:** 04-N7-b

#### Goal
When a new client is onboarded while a list is stale, the user is warned by default. A vertical
may make the stale list block the guarded transition (e.g. engagement `activate`) until the list
is current.

#### Files to read
- `docs/modules/wl-parties.md` §5.4 step 7 (last sentence), §6 (service `Screening` as a guard),
  §7 (setting "Stale list at onboarding")

#### Terms
- **Guarded transition:** a vertical transition that calls the `Screening` guard (e.g. 04
  engagement `activate`).
- **`clear`:** the Screening state "no candidate above the match threshold" (Screening
  lifecycle, transition `complete_clear`); `possible_match` is not `clear`.

#### Spec excerpt (authoritative for this issue)

"A stale list warns at onboarding by default; a vertical can make it block the guarded
transition (e.g. engagement `activate`)."

| Setting | Default |
|---|---|
| Stale list at onboarding | warn (option: block the guarded transition) |

Service `Screening`: screen a party now; latest result; used as a guard (e.g. engagement
activation).

#### Edge cases decided
- **What "stale" means here:** the party's latest Screening carries the stale list flag, or any
  configured list is `stale` now.
- **Warn mode:** the transition runs; the record shows "screened on a stale list (UN list of
  2026-11-02)" until the re-run clears it.
- **Block mode:** the transition is refused with the list name and date; it runs once the list
  is current and the re-run is `clear`.

#### Must not
- Must not block in warn mode.

#### Done when
- [ ] With "warn" and the UN list stale, activating an engagement succeeds and shows the warning.
- [ ] With "block", the same activation is refused naming the UN list and its date; after the
      list update and a `clear` re-run it succeeds.

#### Out of scope
The 04 engagement transition itself (`Epic 04 Accounting`).

---

### BL-PTY-09 — wl-parties: upload an official sanctions list by hand when the download fails

**Labels:** `module:wl-parties` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** BL-PTY-07 · **Blocks:** BL-ACC-23
**Review items:** 04-N7-b

#### Goal
While a download fails, the compliance role can upload the official list file downloaded by
hand from the issuing authority. It becomes the current version, clears the stale status and
triggers re-screening exactly like a download. A list published only as PDF is checked by hand
and the check is recorded on the Screening as *manual*.

#### Files to read
- `docs/modules/wl-parties.md` §5.4 step 8, §7 (setting "Manual upload")

#### Terms
- **Manual upload:** a file uploaded by a user, with source *manual upload*.
- **Manual check:** a Screening with method *manual*, recorded by a holder of
  `screening.review` through *Record manual check* against a PDF-only list.
- **Permission:** every action in this issue (upload, manual check) checks `screening.review`
  only. The compliance role holds it with `parties.block_reason.view` and
  `parties.block_exception`, but those do not gate the upload (spec §6, "Compliance role").

#### Spec excerpt (authoritative for this issue)

§5.4 step 8: "Only while the list is stale (a download fails or the freshness limit has passed),
the compliance role may upload the official file downloaded
by hand from the issuing authority's site (the format the list uses: XML, Excel; a PDF-only list
is checked by hand and the check recorded on the Screening as *manual*: a holder of
`screening.review` uses *Record manual check* on the party, names the list and its date, and
records 'no match' (`complete_clear`) or a candidate with the list entry (`flag`); the Screening
has method *manual* and the reviewer as the person who ran it). The upload records who
uploaded it, the source URL and the list date; a file whose list date is older than the current
version is refused. It becomes the current version with source *manual upload*, clears the stale status and triggers the re-screening of `PTY.RUL.01` like a
download."

| Setting | Default |
|---|---|
| Manual upload of an official list file when the download fails | allowed, compliance role (`screening.review`) |

#### Edge cases decided
- **File checks:** the file must parse in the list's configured format; a file that does not
  parse is refused and nothing changes.
- **Older list date than the current version:** refused ("older than the
  current version").
- **Upload while the download works:** allowed only while the list is `stale`.
- **A later successful download** with a newer version replaces the manual one as normal.

#### Must not
- Must not accept an upload without source URL and list date.
- Must not let a user without `screening.review` upload.

#### Done when
- [ ] Given the UAE Local Terrorist List `stale`, the MLRO uploads an XML file with source URL
      and list date 2026-11-03; the list is `current` with source *manual upload* and uploader,
      `SanctionsListUpdated` is emitted and flagged screenings are re-run.
- [ ] An unparsable file is refused and the list stays `stale`.
- [ ] A user without `screening.review` cannot see the upload action.
- [ ] Given a PDF-only list "Local list (PDF) of 2026-11-03", the MLRO uses *Record manual
      check* on party "Omar K." and records "no match": a Screening exists with method
      *manual*, that list and date, reviewer = MLRO, state `clear`.
- [ ] Recording a candidate instead gives state `possible_match` and the BL-PTY-05 task.
- [ ] A user without `screening.review` does not see *Record manual check*.

#### Out of scope
Freshness and alerts (BL-PTY-07).

---

### BL-PTY-10 — wl-parties: record a company's e-invoicing recipient identifier

**Labels:** `module:wl-parties` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-KRN-05 · Base: wl-parties Party lifecycle · **Blocks:** None.
**Review items:** X-04, 09-N7-a

#### Goal
A company party carries its e-invoicing recipient identifier (Peppol participant ID). wl-invoicing
needs it before issuing an invoice to a recipient in e-invoicing scope, and a change emits
`PartyUpdated`, so an e-invoice rejected for a wrong identifier can be resubmitted after the fix.

#### Files to read
- `docs/modules/wl-parties.md` §3 (*Company details*), §6 (`PartyDirectory`, `PartyUpdated`)

#### Terms
- **Peppol participant ID:** the identifier of the recipient on the e-invoicing network.
- **ASP:** accredited service provider that transmits e-invoices for the legal entity.

#### Spec excerpt (authoritative for this issue)

Company details field: "e-invoicing recipient identifier (Peppol participant ID, entered or
looked up by the company's TRN through the ASP adapter of the legal entity linked to the
user's scope: a found ID is filled in for the user to confirm before saving; 'not found', an
error, or an ASP connection that is not `up` leaves the field empty for entry by hand;
required by `wl-invoicing` before an invoice to a recipient in e-invoicing scope is issued;
a change emits `PartyUpdated`, so rejected transmissions can be resubmitted)".

`PartyDirectory`: billing identity for `wl-invoicing` (legal names, TRN, address, e-invoicing
recipient identifier).

`PartyUpdated`: party, changed fields (names only, never values of sensitive fields).

#### Edge cases decided
- **Lookup through the ASP:** input = the company's TRN; sent through the ASP adapter of the
  legal entity linked to the user's scope. Found: the ID is filled in and the user confirms
  before saving. "Not found" or an error: message "No e-invoicing ID found for this TRN; enter
  it by hand", field left empty. Connection not `up` (BL-KRN-05): the lookup button is
  disabled with the banner, and the field is entered by hand. A company without a TRN cannot
  use the lookup.
- **Format:** stored as entered (scheme + identifier), trimmed; no format
  validation until the ASP's format is confirmed.
- **Person parties** do not have the field.
- **Change:** `PartyUpdated { changed_fields: [einvoice_recipient_id] }` is emitted once per save.

#### Must not
- Must not put the identifier value in the event payload.

#### Done when
- [ ] A company party saved with recipient identifier `0235:1234567890` exposes it through
      `PartyDirectory`.
- [ ] Changing it emits `PartyUpdated` with the field name only.
- [ ] Given company "Delta Trading LLC" with TRN 100123456700003 and the ASP returning
      `0235:100123456700003`, *Look up* fills the field and nothing is saved until the user
      confirms.
- [ ] When the ASP answers "not found", the field stays empty and the message asks for entry by
      hand.
- [ ] With the ASP connection `down`, the lookup is disabled and the user can still enter the
      identifier by hand.

#### Out of scope
The issue guard and resubmission (`Epic wl-invoicing: e-invoice transmission lifecycle`).
