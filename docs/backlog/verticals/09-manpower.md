# Epic: 09 Manpower — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows that belong to the Manpower vertical itself: the new client
contract lifecycle (credit hold, release, early termination, renewal and end with its
deployments), rate card versions, the deployment guards (documents and site licences, overlap,
blocked client), deployment suspension and resume, the deployment reactions to non-standard
employee exits, client rejection of a worker, missing cover, the absence case, late visa steps,
the client-contract values the timesheet escalation reads, and 09's values for the shared
negative-flow mechanisms. It closes 09-N3-a, 09-N12-a, 09-N8-b, the 09 parts of X-13, X-15,
X-16 and X-18, and the 09 MEDIUM items (client rejects a worker, rate card versions, overlap
guards, visa step deadlines, no cover available). Payroll, settlement, pay account, timesheet,
invoice and cheque behaviour lives in the module epics and is only configured here.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-MPW-01 | Put a client contract on credit hold (`hold`, `MPW.RUL.10`) | high | m | Build 1 | Base: 09 Client contract lifecycle; BL-INV-04; BL-PAY-09 |
| BL-MPW-02 | Refuse requests, activations and extensions while a contract is on hold (`MPW.ACL.14`) | high | s | Build 1 | BL-MPW-01; Base: 09 Manpower request and Deployment lifecycle |
| BL-MPW-03 | Release a credit hold (`release`) | high | m | Build 1 | BL-MPW-01; BL-KRN-01; BL-PAY-10; BL-PAY-11; BL-PAY-13 |
| BL-MPW-04 | Terminate a client contract early (`terminate`, `MPW.ACL.17`) | high | m | Build 1 | Base: 09 Client contract lifecycle; Base: 09 Deployment lifecycle; BL-KRN-01 |
| BL-MPW-05 | Warn before a contract ends and renew it (`MPW.RUL.11`, `renew`) | high | m | Build 1 | Base: 09 Client contract lifecycle; BL-MPW-07 |
| BL-MPW-06 | End a contract with its deployments, requests and last timesheets (`end`) | high | m | Build 1 | BL-MPW-04; BL-MPW-05; Base: 09 Deployment lifecycle; Base: wl-attendance Timesheet lifecycle |
| BL-MPW-07 | Price hours by rate card version (`MPW.ACL.16`) | high | m | Build 1 | Base: wl-invoicing Rate cards and usage billing; BL-KRN-01 |
| BL-MPW-08 | Guard deployment `activate`, `resume` and `extend` on worker documents and site licences (`MPW.ACL.07`) | high | m | Build 1 | BL-DOC-01; BL-DOC-06; BL-HRM-01; BL-MPW-09 |
| BL-MPW-09 | Register the deployment `BlockSubjects` and `FutureWork` providers and flag deployments ahead | high | s | Build 1 | BL-DOC-01; BL-DOC-04 |
| BL-MPW-10 | Refuse overlapping deployments of one worker (`MPW.ACL.15`) | medium | s | Build 1 | Base: 09 Deployment lifecycle |
| BL-MPW-11 | Suspend a deployment when a needed document or licence expires (`suspend`) | high | m | Build 1 | BL-MPW-09; BL-DOC-05; BL-HRM-01; Base: 09 Replacements (`MPW.DEP.02`) |
| BL-MPW-12 | Resume a suspended deployment (`resume`) | high | s | Build 1 | BL-MPW-11; BL-MPW-08 |
| BL-MPW-13 | End or cancel deployments on non-standard employee exits (`MPW.RUL.12`) | high | m | Build 1 | BL-HRM-03; BL-HRM-05; BL-HRM-06; BL-HRM-07; BL-HRM-08; BL-HRM-09 |
| BL-MPW-14 | End a deployment rejected by the client and open the replacement | medium | m | Build 1 | Base: 09 Deployment lifecycle; Base: 09 Replacements (`MPW.DEP.02`); Base: wl-ticketing Ticket lifecycle |
| BL-MPW-15 | Apply a client block to deployments and the contract (`continue`) | high | s | Build 1 | BL-PTY-01; BL-PTY-03 |
| BL-MPW-16 | Warn when cover is not found in time (`MPW.RUL.13`) | medium | s | Build 1 | Base: 09 Replacements (`MPW.DEP.02`, `MPW.RUL.06`) |
| BL-MPW-17 | Open and close an absence case (`MPW.RUL.09`) | high | m | Build 1 | Base: wl-ticketing Ticket lifecycle; BL-HRM-07; BL-HRM-18; BL-KRN-08 |
| BL-MPW-18 | Escalate late visa case steps (`MPW.RUL.14`) | medium | s | Build 1 | Base: wl-hr Visa case lifecycle (step templates) |
| BL-MPW-19 | Give wl-attendance the client approver, escalation contact and deemed-approval days | medium | s | Build 1 | BL-ATT-10; BL-ATT-11 |
| BL-MPW-20 | 09: configure shared negative-flow settings (documents, parties, notifications, alerts, integrations) | high | m | Build 1 | BL-DOC-01; BL-DOC-02; BL-DOC-06; BL-PTY-03; BL-NTF-06; BL-KRN-05; BL-KRN-08; BL-TKT-11; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |
| BL-MPW-21 | 09: configure shared negative-flow settings (finance, attendance, payroll and HR) | high | m | Build 1 | BL-INV-01; BL-INV-04; BL-INV-10; BL-PAY-09; BL-ATT-02; BL-ATT-04; BL-ATT-10; BL-ATT-12; BL-HRM-04; BL-HRM-14; BL-HRM-15; BL-HRM-19; BL-HRM-25; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26 |

---

### BL-MPW-01 — 09 Manpower: put a client contract on credit hold (`hold`, `MPW.RUL.10`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 09 Client contract lifecycle (`activate`) · BL-INV-04 (invoice `disputed` flag) · BL-PAY-09 (`ChequeBounced`) · **Blocks:** BL-MPW-02, BL-MPW-03
**Review items:** 09-N3-a, X-05

#### Goal
When a client stops paying while workers stay on site, the agency must stop taking on more
exposure. A daily rule puts the client contract `on_hold` when an invoice is overdue more than
30 days or a client cheque bounces; account managers can also hold a contract by hand. The
contract remembers its state before the hold, the account manager and the MD are told with the
amount and the workers on site, and the client only gets the normal overdue reminders.

#### Files to read
- `docs/verticals/09-manpower.md` §3.3 (`MPW.ACL.14`), §4.1 "When it goes wrong" row "The client
  stops paying", §4.3 rows "The invoice is never paid" and "A client cheque bounces", §6 (Client
  contract fields), §7.1 *Client contract*, §7.2 (`MPW.RUL.10`)
- `docs/modules/wl-invoicing.md` §4 *Invoice* (`disputed` flag)
- `docs/modules/wl-payments.md` §4 *Cheque*, §6 events (`ChequeBounced`, `ChequeBounceResolved`)

#### Terms
- **Client contract:** the 09 agreement with a client company (rate card, billing cycle, payment
  terms, cover obligation, notice period, hold reason, status).
- **Overdue more than N days:** today (Asia/Dubai) minus the invoice due date > N, with an open
  balance > 0.
- **`disputed`:** wl-invoicing flag on an invoice under a billing dispute; reminders pause.
- **State before the hold:** the contract state (`active` or `terminating`) stored on `hold` so
  `release` can go back to it.
- **Workers on site:** deployments of the contract in `active` or `suspended`.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `on_hold` | warning | no | Credit hold: no new requests or deployments; workers on site stay |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `hold` | active, terminating | on_hold | system (`MPW.RUL.10`) or `contracts.manage` | reason (overdue, bounced cheque, other with text) | state before the hold kept, with the reason, the triggering invoices or cheque, and who held it; no request `submit` and no deployment `activate` or `extend` (`MPW.ACL.14`); account manager and MD notified once with the overdue amount and the workers on site; the client gets the normal overdue reminders, not a hold notice. On a contract already `on_hold`, a new cause is added to its open hold reasons instead (no second `hold`) |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MPW.RUL.10` | daily; `ChequeBounced` and `ChequeBounceResolved` for the client; a payment allocation that changes an invoice balance of the client | client contract `active` or `terminating`; an invoice issued under the contract, not `disputed` nor `written_off`, with an open balance overdue more than N days (today minus the due date > N), or a bounced cheque of the client not resolved (every `active` or `terminating` contract of that client) | contract `hold`, or a new reason added to an `on_hold` contract; notify the account manager and the MD once with the overdue amount and the workers on site; when no hold condition remains, `release`. Invoices and cheques already covered by an MD-approved release do not hold again | N = 30 | on | MVP |

§4.1: "`MPW.RUL.10` puts the client contract `on_hold` when an invoice is overdue more than N
days (default 30) or a client cheque bounces. No new request or deployment starts
(`MPW.ACL.14`); workers on site stay and are billed. The account manager and the MD are told;
the MD decides whether to withdraw workers (deployment `end`, reason *client non-payment*,
notice per the contract)."

Effects in detail:
1. Store on the contract: hold reason (`overdue`, `bounced_cheque`, `other` + text), the state
   before the hold, the triggering invoice(s) or cheque, held at, held by (user or rule).
2. Notify the account manager of the client and the MD with: overdue amount (sum of open
   balances of the invoices that triggered the hold), bounced cheque number and amount, and the
   list of workers on site (name, site, role).
3. Send nothing new to the client: the wl-invoicing reminders (`FIN.RUL.01`) continue as usual.
4. History row with the reason.

#### Edge cases decided
- **Run time:** the daily part of `MPW.RUL.10` runs at 02:00 Asia/Dubai.
- **Boundary:** an invoice due 2026-10-01 is overdue 30 days on 2026-10-31 (no hold) and 31 days
  on 2026-11-01 (hold).
- **Which invoices count:** invoices of the contract's client issued under this contract, not
  `disputed`, with an open balance > 0 (partly paid counts). `written_off` invoices do not
  count.
- **Dispute raised after the hold:** the hold stays; it is lifted by payment, by
  `ChequeBounceResolved`, or by a manual `release` (BL-MPW-03).
- **Already `on_hold`:** no second `hold`; a new reason (e.g. a cheque bounces while held for
  overdue) is added to the contract's open hold reasons and both managers are notified once.
- **Contract `draft`, `ended`, `cancelled`:** the rule does nothing.
- **`ChequeBounced` for a client with several contracts:** hold every `active` or `terminating`
  contract of that client.
- **Idempotency:** a second daily run on the same day with the same facts changes nothing and
  sends nothing.
- **Workers on site:** nothing happens to deployments; they keep working and are billed.

#### Must not
- Must not end, suspend or cancel any deployment automatically.
- Must not send the client a hold notice or mention the hold in the portal.
- Must not count a `disputed` invoice.
- Must not write the contract status directly; use the transition.

#### Done when
- [ ] Given an `active` contract for client C with invoice INV-1 of AED 42,000 due 2026-10-01
      and unpaid, when the rule runs on 2026-11-01 02:00, then the contract is `on_hold`, reason
      `overdue`, state before = `active`, and the account manager and MD get one message with
      AED 42,000 and the 12 workers on site.
- [ ] On 2026-10-31 the same contract stays `active`.
- [ ] Given INV-1 is `disputed`, the rule does not hold the contract.
- [ ] Given a `terminating` contract and `ChequeBounced` for C's cheque of AED 15,000, the
      contract is `on_hold`, reason `bounced_cheque`, state before = `terminating`.
- [ ] User A with `contracts.manage` holds an `active` contract with reason `other` and text
      "client in liquidation"; the contract is `on_hold`.
- [ ] `hold` without a reason is refused.
- [ ] After the hold, the client receives the normal `FIN.RUL.01` reminder for INV-1 and no other
      message; deployments are unchanged.
- [ ] Running the rule twice on 2026-11-01 sends one notification.

#### Out of scope
The guards that the hold enables (BL-MPW-02); `release` (BL-MPW-03); MD withdrawing workers
(deployment `end` with reason *client non-payment*, BL-MPW-14); cheque re-presentation and
write-off (BL-PAY-10 to BL-PAY-14); invoice write-off (BL-INV-01).

---

### BL-MPW-02 — 09 Manpower: refuse requests, activations and extensions while a contract is on hold (`MPW.ACL.14`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-MPW-01 · Base: 09 Manpower request lifecycle · Base: 09 Deployment lifecycle · **Blocks:** None
**Review items:** 09-N3-a

#### Goal
While a client contract is `on_hold`, no new exposure may start: a manpower request cannot be
submitted, and no deployment of that client can be activated or extended. Workers already on
site keep working. The guard must apply to staff and to the client approver in the portal.

#### Files to read
- `docs/verticals/09-manpower.md` §3.3 (`MPW.ACL.14`), §7.1 *Manpower request* (`submit`),
  *Deployment* (`activate`, `extend`)

#### Terms
- **Manpower request:** a client's order for workers (role, headcount, site, shift pattern,
  start, duration).
- **`extend`:** a deployment transition that sets a later end date; the state stays the same.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `MPW.ACL.14` | While a client contract is `on_hold`, no manpower request is submitted and no deployment of that client is activated or extended (`resume` stays allowed; drafts and planned deployments may still be created); workers already on site stay unless the MD decides to withdraw them. Releasing the hold while a hold condition remains (`MPW.RUL.10`) needs MD approval, and the requester cannot approve; a release when no hold condition remains needs no approval | `STA`, `SOD` | account manager, accountant | approver: MD |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` (request) | draft | open | `requests.create` (staff or client approver) | the contract's rate card covers the role and shift; contract `active` (not `on_hold`, `MPW.ACL.14`, nor `terminating`); client not blocked. The portal refusal does not mention a hold | account manager notified |
| `activate` (deployment) | planned | active | `deployments.manage` | employee `active`; `MPW.ACL.07` documents and site licences valid; client contract `active` or `terminating` (`MPW.ACL.14`); client not blocked; no overlapping deployment (`MPW.ACL.15`) | request `fill`; attendance enabled at the site; client notified |
| `extend` (deployment) | active, suspended | same state | `deployments.manage` | new end date within the contract; `MPW.ACL.07` on its check date; contract not `on_hold`; client not blocked; no overlap | client notified |

#### Edge cases decided
- **Request submit while `terminating`:** refused; the guard requires `active`. Requests are for
  new work and a terminating contract is ending.
- **Refusal message (staff):** "Contract on credit hold since <date>: <reason>". **Portal:**
  "This request cannot be submitted now. Please contact your account manager." (no mention of
  the hold, consistent with no hold notice).
- **Drafts:** requests can still be created and edited as `draft`; only `submit` is refused.
- **Concurrency:** the guard reads the contract state in the same transaction as the
  transition; a `hold` committed first wins.
- **`resume` of a suspended deployment:** not covered by `MPW.ACL.14`; allowed while on hold.
- **Planned deployments already created:** stay `planned`; they can be activated after
  `release`.

#### Must not
- Must not end or suspend workers on site.
- Must not reveal the hold reason to the client in the portal.

#### Done when
- [ ] Given contract K `on_hold`, when the client approver submits request R in the portal, it is
      refused with the neutral message and R stays `draft`.
- [ ] Given K `on_hold`, staff user A's `activate` of planned deployment D (start 2026-11-05) is
      refused with the hold reason and date.
- [ ] Given K `on_hold` and active deployment D2 ending 2026-11-30, `extend` to 2026-12-31 is
      refused.
- [ ] Given K `terminating`, `activate` of D is allowed (if the other guards pass) and request
      `submit` is refused.
- [ ] Given K `active`, all three are allowed.
- [ ] A suspended deployment of K can `resume` while K is `on_hold`.

#### Out of scope
`release` and its MD approval (BL-MPW-03); other deployment guards (BL-MPW-08, BL-MPW-10,
BL-MPW-15).

---

### BL-MPW-03 — 09 Manpower: release a credit hold (`release`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-MPW-01 · BL-KRN-01 (approval ends without a yes) · BL-PAY-10, BL-PAY-11, BL-PAY-13 (`ChequeBounceResolved`) · **Blocks:** None
**Review items:** 09-N3-a

#### Goal
A hold ends when the client pays or the bounced cheque is resolved; the system then releases
the contract back to the state it had before. An account manager may also release earlier
(for example after a payment plan), but releasing before the overdue amount is paid needs the
MD's approval, and the requester cannot approve.

#### Files to read
- `docs/verticals/09-manpower.md` §3.3 (`MPW.ACL.14`), §7.1 *Client contract* (`release`),
  §7.2 (`MPW.RUL.10`)
- `docs/modules/wl-payments.md` §6 (`ChequeBounceResolved`)
- `docs/modules/wl-kernel.md` §4 *Approval request*

#### Terms
- **`ChequeBounceResolved`:** wl-payments event when a bounced cheque is cleared after
  re-presentation, replaced or written off.
- **Hold condition:** the facts that `MPW.RUL.10` checks (an invoice issued under the contract,
  not `disputed` nor `written_off`, with an open balance overdue more than 30 days, or a bounced
  cheque of the client not resolved). The same test is used to hold and to release.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `release` | on_hold | active, terminating | system (no hold condition left: every invoice that caused an *overdue* reason paid, no other invoice under the contract meets the `MPW.RUL.10` invoice condition (not `disputed` nor `written_off`, open balance overdue more than N days), every bounced cheque that caused a reason resolved) or `contracts.manage` (with MD approval while a hold condition remains, `MPW.ACL.14`) | — | back to the state before the hold; account manager notified; the client is not told. A dispute raised after the hold does not release it. After an MD-approved release, `MPW.RUL.10` does not hold the contract again for the same invoices or cheques; a system release withdraws a pending manual request |

`MPW.ACL.14` (excerpt): "Releasing the hold before the overdue amount is paid needs MD approval,
and the requester cannot approve" (`STA`, `SOD`; approver: MD).

`MPW.RUL.10` (excerpt): "on full payment or `ChequeBounceResolved`, `release`".

#### Edge cases decided
- **System release** runs when no hold condition remains: every invoice that caused an
  `overdue` hold is paid (open balance 0), no other invoice under the contract meets the hold
  condition (a `disputed` or `written_off` invoice never counts, as in BL-MPW-01), and every
  bounced cheque that caused a hold has `ChequeBounceResolved`. With two open hold reasons
  (BL-MPW-01), clearing one does not release.
- **Check timing:** when a payment allocation changes an invoice balance of the client and on `ChequeBounceResolved`
  immediately, and again in the 02:00 daily run.
- **Manual release when no hold condition remains:** no approval needed (the overdue amount is
  paid).
- **Manual release with a hold condition:** creates an approval request to the MD; the
  requester (account manager or accountant) cannot approve, even if they hold the MD role. On
  approval the contract returns to the state before; on reject or return it stays `on_hold`
  (BL-KRN-01).
- **No immediate re-hold:** after an MD-approved release, `MPW.RUL.10` does not hold the
  contract again for the same invoices or cheques; a new invoice becoming overdue more than 30
  days, or a new bounce, holds it again.
- **State before = `terminating` and the end date passed while on hold:** the contract has
  already `end`ed (BL-MPW-06); nothing to release.
- **Concurrency:** a system release and a pending manual request: the system release wins and
  the approval request is withdrawn with the reason "released by payment".

#### Must not
- Must not let the requester approve their own release.
- Must not release to `active` a contract that was `terminating` before the hold.
- Must not message the client about the release (they were never told of the hold).

#### Done when
- [ ] Given contract K `on_hold` (before: `active`, reason `overdue` for INV-1 AED 42,000), when
      a payment of AED 42,000 is allocated to INV-1, K is `active` and the account manager is
      notified.
- [ ] Given K held for INV-1 and a bounced cheque CHQ-9, paying INV-1 alone leaves K `on_hold`;
      after `ChequeBounceResolved` for CHQ-9, K is released.
- [ ] Given K held for INV-1 and INV-2 of the same contract `disputed` and 45 days overdue,
      paying INV-1 releases K (INV-2 does not count).
- [ ] Given K `on_hold` (before: `terminating`), after release K is `terminating`.
- [ ] Account manager A requests release while INV-1 is unpaid: an approval request goes to the
      MD; A cannot approve it; when MD user B approves, K returns to its state before.
- [ ] When B rejects with a comment, K stays `on_hold` and A is notified.
- [ ] After B's approval, the 02:00 run the next day does not hold K again for INV-1; when INV-2
      becomes 31 days overdue, K is held again.

#### Out of scope
Putting on hold (BL-MPW-01); cheque resolution itself (BL-PAY-10 to BL-PAY-13).

---

### BL-MPW-04 — 09 Manpower: terminate a client contract early (`terminate`, `MPW.ACL.17`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 09 Client contract lifecycle · Base: 09 Deployment lifecycle · BL-KRN-01 · **Blocks:** BL-MPW-06
**Review items:** 09-N12-a

#### Goal
Either side may end a client contract before its end date. The account manager records who
initiated it, the reason, the notice date and the new end date; the MD approves. The contract
becomes `terminating`, deployments are cut to the new end date, deployments that would start
after it are cancelled, and the client receives a confirmation. The contract then ends on its
end date through `end` (BL-MPW-06).

#### Files to read
- `docs/verticals/09-manpower.md` §3.3 (`MPW.ACL.17`), §4.1 row "The contract expires, or ends
  early", §6 (Client contract: notice period), §7.1 *Client contract*, *Manpower request*
  (`cancel`), *Deployment* (`cancel`)

#### Terms
- **`terminating`:** notice of early termination given; the contract still runs until its end
  date and can be held for non-payment.
- **Notice period:** a client contract field (days).
- **Waiver:** a document (e.g. a signed letter) in which the other side accepts a shorter notice.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `terminating` | warning | no | Notice of early termination given; ends on the end date |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `terminate` | active, on_hold | terminating | `contracts.manage`, approved by the MD (`MPW.ACL.17`) | initiated by (client or agency), reason, notice date and end date recorded; end date not later than the current one; end date ≥ notice date + the contract's notice period in calendar days, or the waiver attached | nothing changes before the approval (a `hold` meanwhile does not stop the request: `terminate` is allowed from `on_hold`; an `end` or a `renew` meanwhile withdraws it); then: any hold cleared; end dates of active and suspended deployments set to the contract end date when theirs is later or empty; planned deployments starting after it cancelled (reason *contract terminated*), the others capped at it; open requests starting after it cancelled, the others at `end`; client sent the confirmation letter (`DOC.GEN.01`) |
| `cancel` (request) | draft, open, partially_filled | cancelled | `requests.manage`, client approver, or system (contract `terminate` or `end`) | reason given | planned deployments for it cancelled |
| `cancel` (deployment) | planned | cancelled | `deployments.manage` or system (`EmployeeOnboardingCancelled`, `EmployeeAbsconded`, `EmployeeDeceased`, contract `terminate` or `end`, request `cancel`) | reason given | client told when the start date was already communicated |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `MPW.ACL.17` | Early termination of a client contract (`terminate`), and ending active deployments because of it, is approved by the MD | `SOD` | account manager | approver: MD |

#### Edge cases decided
- **Notice respected:** new end date ≥ notice date + notice period (calendar days, Asia/Dubai).
  Otherwise a waiver file is required.
- **New end date later than the current end date:** refused (that is `renew`).
- **From `on_hold`:** the hold ends with the transition (the stored state before is cleared);
  `MPW.RUL.10` can hold the `terminating` contract again (BL-MPW-01).
- **Active and suspended deployments:** end date set to the contract's new end date only when
  their end date is later or empty; earlier end dates are kept.
- **Planned deployments:** those starting after the new end date are cancelled (reason
  *contract terminated*); those starting on or before it are kept with their end date capped.
- **Open requests:** cancelled (reason *contract terminated*) only when their start date is
  after the new end date; the others are cancelled at `end` (BL-MPW-06).
- **Approval pending:** the contract stays in its state; nothing is cut until the MD approves.
  If the contract is put `on_hold` meanwhile, the request stays valid (`terminate` is allowed
  from `on_hold`) and the approval runs it from `on_hold`. If the contract `end`s or is
  `renew`ed meanwhile, the approval request is withdrawn (BL-KRN-01) and the account manager
  is told.
- **Client confirmation:** generated as a letter (`DOC.GEN.01`) and e-mailed to the client's
  contacts on approval.

#### Must not
- Must not end any deployment immediately; `end` happens on the end date.
- Must not run any effect before the MD approves.
- Must not let the account manager approve their own request.

#### Done when
- [ ] Given `active` contract K ending 2027-06-30 with a 30-day notice period, when account
      manager A requests `terminate` (initiated by client, reason "budget cut", notice
      2026-11-01, end 2026-12-01), an approval request goes to the MD and K is unchanged.
- [ ] When MD user B approves, K is `terminating` with end date 2026-12-01; active deployment D1
      (end 2027-06-30) now ends 2026-12-01; D2 (end 2026-11-20) keeps 2026-11-20; planned D3
      starting 2026-12-10 is `cancelled`; open request R starting 2026-12-15 is `cancelled`; the
      client gets the confirmation.
- [ ] With end 2026-11-15 (14 days) and no waiver, the request is refused; with a waiver file
      it is allowed.
- [ ] A cannot approve the request.
- [ ] From `on_hold`, after approval K is `terminating`.
- [ ] Given A's request pending and K put `on_hold` by `MPW.RUL.10` before B approves, B's
      approval still runs: K is `terminating` and the hold is cleared.

#### Out of scope
The contract `end` on the end date (BL-MPW-06); `renew` (BL-MPW-05).

---

### BL-MPW-05 — 09 Manpower: warn before a contract ends and renew it (`MPW.RUL.11`, `renew`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 09 Client contract lifecycle · BL-MPW-07 (new rate card versions) · **Blocks:** BL-MPW-06
**Review items:** 09-N12-a

#### Goal
A contract must not run out unnoticed with workers on site. 60 and 30 days before the end
date the account manager is told which deployments will end. The account manager renews the
contract with a new end date and a signed renewal; changed rates come as a new rate card
version. After renewal, deployment end dates may be extended and the warnings restart.

#### Files to read
- `docs/verticals/09-manpower.md` §4.1 row "The contract expires, or ends early", §7.1 *Client
  contract* (`renew`), §7.2 (`MPW.RUL.11`)

#### Terms
- **Renewal signed:** the renewal document accepted by the client through `DOC.SIG.01`.
- **Rate card version:** see BL-MPW-07.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `renew` | active, on_hold | same state | `contracts.manage` | new end date later than the current one; renewal signed; changed rates as a new rate card version (`MPW.ACL.16`); client not blocked (`PartyBlocked`) | deployment end dates are not changed: the next `MPW.RUL.11` notice lists them for `extend`; `MPW.RUL.11` restarts from the new end date |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MPW.RUL.11` | daily: N and M days before a client contract's end date | contract `active` or `on_hold`; not renewed | notify the account manager with the deployments that will end; second notice at M days; one notice per threshold per end date (a contract created or renewed inside the window gets the missed notice at the next run); on the end date the contract `end`s | N = 60; M = 30 | on | MVP |

#### Edge cases decided
- **New end date:** must be later than the current end date.
- **Renewal while `on_hold`:** allowed (spec); the contract stays `on_hold`.
- **Renewal while `terminating`:** not offered (spec lists `active`, `on_hold` only).
- **Deployments:** not extended automatically; the notice after renewal lists them so the
  account manager runs `extend` (guards in BL-MPW-02, BL-MPW-08, BL-MPW-10).
- **Restart:** the 60- and 30-day notices are computed from the new end date; notices already
  sent for the old date are not resent.
- **Contract created or renewed inside the window** (e.g. 40 days left): the 60-day notice is
  sent at the next daily run, then the 30-day one on time.
- **Run time and idempotency:** daily at 02:00 Asia/Dubai, one notice per threshold per end
  date.
- **Blocked client:** `renew` refused (BL-MPW-15).

#### Must not
- Must not change rates on the existing rate card version (always a new version).
- Must not end the contract from this rule before the end date.

#### Done when
- [ ] Given `active` contract K ending 2026-12-31 with 8 active deployments, on 2026-11-01 the
      account manager gets a notice listing the 8 deployments; on 2026-12-01 a second notice.
- [ ] When the account manager renews K to 2027-12-31 with a signed renewal, K stays `active`
      and the next notice is due 2027-11-01.
- [ ] Renew without a signed renewal, or with end date 2026-12-15 (earlier), is refused.
- [ ] Renew with changed rates requires a new rate card version effective from 2027-01-01.
- [ ] Given K `on_hold`, renew works and K stays `on_hold`.

#### Out of scope
The `end` itself (BL-MPW-06); rate card versions (BL-MPW-07).

---

### BL-MPW-06 — 09 Manpower: end a contract with its deployments, requests and last timesheets (`end`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-MPW-04 · BL-MPW-05 · Base: 09 Deployment lifecycle (`end`, `cancel`) · Base: wl-attendance Timesheet lifecycle · Base: 09 billing (`MPW.RUL.05`) · **Blocks:** None
**Review items:** 09-N12-a

#### Goal
On the contract end date (normal expiry or after early termination), the contract ends and
everything under it closes cleanly: active and suspended deployments end, planned ones and open
requests are cancelled, the last timesheets are submitted and still billed, and the workers go
back to the bench.

#### Files to read
- `docs/verticals/09-manpower.md` §4.1 row "The contract expires, or ends early", §7.1 *Client
  contract* (`end`), *Manpower request* (`cancel`), *Deployment* (`end`, `cancel`), §7.2
  (`MPW.RUL.05`, `MPW.RUL.11`)

#### Terms
- **Bench:** employees who are `active` but have no active deployment.
- **Last timesheets:** the open timesheet of each deployment for the period up to the end date.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `ended` | neutral | yes | End date reached; deployments ended |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `end` (contract) | active, on_hold, terminating | ended | system (first daily run after the end date) | not renewed | active and suspended deployments `end` (reason *contract end*, end date = the contract end date); planned ones cancelled; open requests cancelled; hold reasons closed (overdue invoices stay in receivables); each open timesheet closed at the end date and submitted to the client at once, and billed after approval by `MPW.RUL.05` though the contract is `ended`; workers still `active` back on the bench; account manager notified |
| `end` (deployment) | active, suspended | ended | `deployments.manage` or system (end date; contract `end`; employee `exit`, `report_absconding`, `record_death`) | end reason recorded (end date, contract end, client rejected, client non-payment, replaced, exit, absconded, deceased, entered in error); *client rejected* needs the client's note; *client non-payment* only by the MD, with an end date at least the contract's notice period away unless a waiver is attached | open timesheet closed for the period; client notified; *client rejected*: replacement request if the contract requires cover; *entered in error*: its attendance rejected (hours already in a `locked` timesheet need an adjustment timesheet, `HRM.ACL.08`); worker back on the bench when still `active`; a request filled by it does not reopen (the account manager may create a replacement) |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MPW.RUL.05` | end of the contract's billing cycle | approved timesheets not yet billed | create a draft invoice per client from the rate card; notify the accountant | — | on | MVP |

#### Edge cases decided
- **When:** the end date is the last working day; `end` runs at the first daily run after it
  (00:05 Asia/Dubai the next day).
- **Last timesheets:** each open timesheet is closed at the end date and submitted to the client
  at once (not at the period end). After approval, `MPW.RUL.05` bills it in the next billing
  run even though the contract is `ended`.
- **`on_hold` contract:** ends too; the hold reasons are closed with the contract; overdue
  invoices stay in receivables.
- **Suspended deployments:** ended with reason *contract end*; suspended days stay unbilled.
- **Worker no longer `active`** (e.g. `notice`): deployment ends but the worker does not join
  the bench.
- **Idempotency:** a second run finds the contract `ended` and does nothing.
- **Renewed before the run:** guard "not renewed" fails; nothing happens.

#### Must not
- Must not drop unbilled approved hours of the ended contract.
- Must not end the contract before the end date has passed.

#### Done when
- [ ] Given `terminating` contract K with end date 2026-12-01, active D1, suspended D2,
      planned D3 and open request R, at 00:05 on 2026-12-02 K is `ended`; D1 and D2 are `ended`
      with reason *contract end* and end date 2026-12-01; D3 and R are `cancelled`; the client
      is notified per deployment; the account manager is notified once.
- [ ] D1's open timesheet for 2026-11-25 to 2026-12-01 is closed and submitted; after the
      client approves it on 2026-12-04, the next `MPW.RUL.05` run creates a draft invoice with
      those hours.
- [ ] D1's worker (`active`) is on the bench; D2's worker in `notice` is not.
- [ ] Given K renewed on 2026-11-30 to 2027-11-30, nothing ends on 2026-12-02.

#### Out of scope
Early termination (BL-MPW-04); renewal and warnings (BL-MPW-05).

---

### BL-MPW-07 — 09 Manpower: price hours by rate card version (`MPW.ACL.16`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-invoicing Rate cards and usage billing (`FIN.RAT.01`, `FIN.USG.01`) · BL-KRN-01 · **Blocks:** BL-MPW-05
**Review items:** 09 MEDIUM "rate card versions"

#### Goal
Rates change over a contract's life. A change is never an edit: it is a new rate card version
with an effective date, accepted by the client. Hours are priced by the version in force on the
work date, and nothing already invoiced or in a locked timesheet is repriced. A version dated in
the past needs the MD's approval.

#### Files to read
- `docs/verticals/09-manpower.md` §3.3 (`MPW.ACL.16`), §4.3 row "The rate card changes", §6
  (Rate card line: rate card version, effective from)
- `docs/modules/wl-invoicing.md` §3 (Rate card, Rate card line, Usage record), §5 usage billing

#### Terms
- **Rate card version:** a set of rate card lines for one client contract with an *effective
  from* date; the previous version applies until the day before.
- **Work date:** the date of the attendance hours being priced.
- **`locked` timesheet:** a timesheet read by billing or payroll; never edited (`HRM.ACL.02`,
  `HRM.ACL.08`).

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `MPW.ACL.16` | A rate card change is a new rate card version with an effective date, accepted by the client (`DOC.SIG.01`); it never changes hours already invoiced or timesheets already `locked`. A version is *pending* until the client accepts it and prices nothing before; lines of an accepted version are never edited; two versions of one contract never share an effective date. A change dated in the past (effective before the day it is submitted) needs MD approval after the client's acceptance, and reprices only hours not yet invoiced or `locked` | `STA`, `LIM` | account manager | — |

§4.3: "A new rate card version from its effective date (`MPW.ACL.16`); hours are priced by the
version in force on the work date."

§6 Rate card line fields: "rate card version, effective from, role, shift type, bill rate per
hour/day/month, OT and holiday multipliers, minimum hours".

#### Edge cases decided
- **Version states:** a new version is *pending* until the client accepts it (`DOC.SIG.01`);
  only accepted versions price hours.
- **"Dated in the past":** effective from earlier than today (Asia/Dubai) at submission. The MD
  approval request is created after client acceptance; on reject the version is not used.
- **Two versions with the same effective date:** refused.
- **Repricing scope of a past-dated version:** only hours whose timesheet is not `locked` and
  not invoiced; locked or invoiced hours keep the old price (any difference is the client's and
  agency's commercial matter, outside the system). 
- **Shift crossing midnight:** the work date is the shift start date.
- **Multipliers and minimum hours:** taken from the version in force on the work date. The
  pricing itself is wl-invoicing usage billing (base × quantity × multipliers, then minimums,
  wl-invoicing §5.2): OT hours × bill rate × OT multiplier; hours on a public holiday × bill
  rate × holiday multiplier; the minimum hours apply per shift (a 3-hour shift with a
  4-hour minimum bills 4 hours). This issue only selects the version.
- **Monthly rates with a version change mid-month:** pro rata by calendar days per version:
  month rate × days of the version in the month ÷ calendar days of the month. One line per
  version.
- **Rounding:** each priced line is rounded once, half-up to fils (2 decimals, AED), after all
  multiplying, including the pro-rata fraction; no intermediate rounding.

#### Must not
- Must not edit lines of an accepted version.
- Must not reprice invoiced hours or `locked` timesheets.
- Must not use a version the client has not accepted.

#### Done when
- [ ] Given version V1 (guard AED 25/h) and V2 (AED 27/h) effective 2026-12-01, hours worked
      2026-11-30 are priced at 25 and hours on 2026-12-01 at 27.
- [ ] Editing a line of accepted V1 is refused; the user is offered "new version".
- [ ] V2 created on 2026-12-10 with effective date 2026-12-01 needs MD approval after client
      acceptance; before approval, December hours price at V1.
- [ ] After approval, unlocked December hours are priced at 27; a timesheet locked on
      2026-12-08 for 2026-12-01 to 2026-12-07 keeps AED 25.
- [ ] A version pending client acceptance prices nothing.
- [ ] Given V1 month rate AED 3,100 and V2 AED 3,410 effective 2026-12-11, December bills
      3,100 × 10 ÷ 31 = AED 1,000.00 and 3,410 × 21 ÷ 31 = AED 2,310.00.
- [ ] Given V2 with OT multiplier 1.25 and minimum 4 hours per shift, 2 OT hours on 2026-12-02
      bill 2 × 27 × 1.25 = AED 67.50, and a 3-hour shift on 2026-12-03 bills 4 × 27 = AED 108.00.

#### Out of scope
Contract `renew` (BL-MPW-05); usage billing itself (base).

---

### BL-MPW-08 — 09 Manpower: guard deployment `activate`, `resume` and `extend` on worker documents and site licences (`MPW.ACL.07`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-DOC-01 (`document_valid` and `blocks`) · BL-DOC-06 (override) · BL-HRM-01 (`qualification_valid`) · BL-MPW-09 · **Blocks:** BL-MPW-12
**Review items:** X-13, 09-N8-b

#### Goal
A worker may not start, restart or continue at a client site without a valid residence visa,
work permit, Emirates ID and health insurance, and the licences the site requires (SIRA guard
card, health card, site pass). The guard also looks ahead 30 days so a worker is not deployed
on a visa that expires next week. Overrides are possible only for kinds that allow them, with
the MD's approval.

#### Files to read
- `docs/verticals/09-manpower.md` §3.3 (`MPW.ACL.07`), §4.1 step 4, §7.1 *Deployment*
  (`activate`, `extend`, `resume`), §9 "Documents that block work"
- `docs/modules/wl-documents.md` §5.6 (standard guard, override `DOC.ACL.02`)
- `docs/modules/wl-hr.md` §2 (`HRM.QUA.01`, standard guard)
- `docs/OPEN-ITEMS.md` L-19

#### Terms
- **`document_valid(subject, kind, on_date)`:** yes when an `accepted` document of that kind
  expires on or after `on_date`.
- **`qualification_valid(employee, kind, on_date)`:** the same with role and scope (site).
- **Site requirements (`MPW.CMP.01`):** the licences, passes and inductions a site requires.
- **Override:** one-use permission for one record and one transition, approved by a second user
  (`DOC.ACL.02`), valid 72 hours.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `MPW.ACL.07` | A deployment can start (`activate`), `resume` or be `extend`ed only if the worker's residence visa, work permit, Emirates ID and health insurance pass `document_valid`, and the site's required licences for the deployment's role (SIRA for security roles, health card, site pass) pass `qualification_valid` (`HRM.QUA.01`), on the check date: the transition date + N days, or the deployment's end date if earlier (for `extend`, the new end date if earlier). A deployment shorter than N days needs its documents valid to its end only. This is a hard guard (mode `block`, §9) at the moment of the transition. Creating or rescheduling a `planned` deployment only warns (wl-documents §5.6 point 2), and `DOC.RUL.02` lists it again inside the look-ahead window. A renewal still in review does not count. ILOE and passport do not block. Override only as the kind allows (§9): never for visa, work permit and SIRA licence; with MD approval for the others (`DOC.ACL.02`, one use for this deployment and this transition) | `STA` | account manager | N = 30 |

| Document kind | `blocks` (mode) | Override | Responsible role |
|---|---|---|---|
| Residence visa, work permit | `deployment.activate`, `deployment.extend`, `deployment.resume` (block); `attendance_record.accept` (flag) | never | PRO / visa officer |
| SIRA licence (security roles) | `deployment.activate`, `deployment.extend`, `deployment.resume` (block); `attendance_record.accept` (flag) | never | PRO / visa officer |
| Emirates ID, health insurance, health card | `deployment.activate`, `deployment.extend`, `deployment.resume` (block); `attendance_record.accept` (flag) | with MD approval (`DOC.ACL.02`) | PRO / visa officer |
| Site pass | as above | with MD approval (`DOC.ACL.02`) | account manager |
| Passport, labour contract, ILOE | none | — | PRO / visa officer (labour contract: HR / payroll officer) |
| Client contract, rate card acceptance, signed timesheet | none | — | account manager |

| Transition | From | To | By | Guards |
|---|---|---|---|---|
| `activate` | planned | active | `deployments.manage` | employee `active`; `MPW.ACL.07` documents and site licences valid; client contract `active` or `terminating` (`MPW.ACL.14`); client not blocked; no overlapping deployment (`MPW.ACL.15`) |
| `extend` | active, suspended | same state | `deployments.manage` | new end date within the contract; `MPW.ACL.07` on its check date; contract not `on_hold`; client not blocked; no overlap |
| `resume` | suspended | active | system (`DocumentAccepted` for the expired kind, when no other reason remains; document reasons only) or `deployments.manage` | `MPW.ACL.07` checks pass again; end date not passed; contract not `ended` |

§9: "Override mode `never` is proposed as a legal block `⚠ open: lawyer (L-19)`."

#### Edge cases decided
- **`on_date` for `activate` and `resume`:** the check passes when each kind is valid on
  min(transition date + 30 days, deployment end date). A 10-day deployment needs documents valid
  to its end only.
- **`extend` ("at the new date"):** the same check with on_date = min(today + 30 days, new end
  date).
- **Which site kinds:** the site's required kinds (`MPW.CMP.01`) for the deployment's role;
  SIRA only for security roles.
- **ILOE, passport:** do not block (not listed).
- **Refusal message:** names each failing kind, its expiry date, and whether an override is
  possible and who approves.
- **Override:** only for Emirates ID, health insurance, health card, site pass; one use for this
  deployment and this transition, MD approves, requester ≠ approver, 72 h. Never for visa, work
  permit, SIRA (no override button).
- **What to build while L-19 is open:** build the spec values above (visa, work permit, SIRA:
  `never`; the others: `with_approval`, MD). The guard reads each kind's override mode from the
  wl-documents kind setting seeded by BL-MPW-20 and never from code, so an L-19 answer changes
  the seed only. The label `blocked:open-item` does not stop the build.
- **Renewal in `pending_review`:** answers *no*; the message shows the pending renewal.

#### Must not
- Must not offer an override for residence visa, work permit or SIRA licence.
- Must not keep a separate expiry check; always call `document_valid` / `qualification_valid`.

#### Done when
- [ ] Given worker W with a visa expiring 2026-11-20 and planned deployment D (start 2026-11-05,
      end 2027-05-31), `activate` on 2026-11-05 is refused naming the visa (needs validity to
      2026-12-05) with no override offered.
- [ ] Given the same visa and D ending 2026-11-15, `activate` on 2026-11-05 is allowed.
- [ ] Given W's health insurance expired and all else valid, `activate` is refused; account
      manager A asks an override; MD user B approves; A activates within 72 h; a second
      `activate` on another deployment needs a new override.
- [ ] Given a site requiring a health card and W without one, `activate` is refused.
- [ ] `extend` of active D2 to 2027-03-31 on 2026-11-01 with W's Emirates ID expiring
      2026-11-20 is refused.
- [ ] Manual `resume` with an expired work permit is refused.
- [ ] Creating planned deployment D7 (start 2026-12-01) for W, whose visa expires 2026-11-20,
      shows a warning and saves D7; its `activate` is refused later.
- [ ] In a test seed with the Emirates ID override mode set to `never`, the health insurance
      case above still offers the override and the Emirates ID case offers none, with no code
      change (the mode is read from the kind setting).

#### Out of scope
Providers and look-ahead flags (BL-MPW-09); suspension on expiry (BL-MPW-11); attendance flag
`permit_invalid` (BL-ATT-04).

---

### BL-MPW-09 — 09 Manpower: register the deployment `BlockSubjects` and `FutureWork` providers and flag deployments ahead

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-DOC-01 · BL-DOC-04 (`DOC.RUL.02` look-ahead with `affected`) · **Blocks:** BL-MPW-08, BL-MPW-11
**Review items:** X-13, 09-N8-b

#### Goal
wl-documents needs to know which documents a deployment depends on and which deployments an
expiring document affects. The deployment module registers both providers, so the guard works
and so the account manager sees, 30 days ahead, the deployments that a document will stop.

#### Files to read
- `docs/verticals/09-manpower.md` §4.5 step 3, §9 (last paragraph of "Documents that block work")
- `docs/modules/wl-documents.md` §5.6 "When it is checked" points 2–4, §6 (providers)

#### Terms
- **`BlockSubjects`:** provider that maps a record (a deployment) to the document subjects that
  guard it.
- **`FutureWork`:** provider that lists the records that will need the document after an
  expiry date (here: planned, active and suspended deployments, see Edge cases).
- **`affected`:** the list carried by `DocumentExpiring` and `DocumentExpired`.

#### Spec excerpt (authoritative for this issue)

§9: "Expiry of any of these during a deployment suspends it (`MPW.RUL.12`). The deployment
module registers `BlockSubjects` (deployment → the employee for visa, work permit, Emirates ID
and health insurance; the employee with the deployment's role and site scope for the site's
kinds) and `FutureWork` (deployments `planned` with a start after the expiry date, and `active`
or `suspended` with an end date after it or none)."

§4.5 step 3: "Workers whose documents will expire during a deployment are flagged to the
account manager (`MPW.ACL.07`, `DOC.RUL.02` look-ahead of 30 days)."

wl-documents §5.6 point 3: "every night, for documents of blocking kinds that expire within the
kind's look-ahead window (default 30 days), each guarded module lists, through its `FutureWork`
provider, the planned items dated after the expiry date. They are added to `DocumentExpiring`
as `affected`, and the record's responsible staff get one message with the list."

#### Edge cases decided
- **`BlockSubjects(deployment)`:** the employee (for visa, work permit, Emirates ID, health
  insurance) and, for site kinds, the employee with role and site scope.
- **`FutureWork(employee, expiry date)`:** deployments in `planned` with start after the expiry
  date, and in `active` or `suspended` with end date after it (or no end date).
- **Responsible staff:** the account manager of the deployment's client company.
- **Flag:** the deployment shows "document expiring <kind> <date>" in lists until the renewed
  document is accepted.
- **Renewal visa rejected:** wl-hr re-emits `DocumentExpiring` (BL-HRM-03); the same provider
  answers, so the account manager and MD see the affected deployments.

#### Must not
- Must not send one message per deployment; one message per document per night.

#### Done when
- [ ] Given W's visa expiring 2026-12-10 and W's active deployment D ending 2027-03-31, the
      nightly run on 2026-11-10 lists D in `DocumentExpiring.affected`, D is flagged, and the
      account manager gets one message.
- [ ] A deployment ending 2026-12-05 is not listed.
- [ ] After a renewed visa is accepted, the flag disappears.
- [ ] `BlockSubjects` for a security guard deployment at site S returns W with the SIRA kind
      scoped to S.

#### Out of scope
The guard (BL-MPW-08); suspension (BL-MPW-11).

---

### BL-MPW-10 — 09 Manpower: refuse overlapping deployments of one worker (`MPW.ACL.15`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:medium` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: 09 Deployment lifecycle · **Blocks:** None
**Review items:** 09 MEDIUM "overlap guards", X-16

#### Goal
A worker cannot be at two client sites at the same time. Activating or extending a deployment
is refused when the worker already has an active or suspended deployment whose shift times
overlap it. (Overlapping attendance is flagged separately by wl-attendance.)

#### Files to read
- `docs/verticals/09-manpower.md` §3.3 (`MPW.ACL.15`), §4.1 row "The worker is already deployed
  for the same hours", §7.1 *Deployment* (`activate`, `extend`)

#### Terms
- **Shift times:** the deployment's daily shift window (start and end time) on its working
  days, between its start and end dates.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `MPW.ACL.15` | A deployment cannot be activated or extended for an employee who already has an `active` or `suspended` deployment whose shift times overlap it | `STA` | account manager | — |

§4.1: "`activate` is refused (`MPW.ACL.15`); overlapping attendance is flagged `overlap` when it
is recorded (an effect of `record` in wl-attendance §4, across sites and projects);
`HRM.RUL.13` checks only the overtime cap and the midday break."

#### Edge cases decided
- **Overlap test:** date ranges intersect and, on at least one common working day, the shift
  windows intersect. Windows that touch (one ends 14:00, the other starts 14:00) do not
  overlap. Overnight shifts (22:00–06:00) are handled as windows crossing midnight.
- **Planned deployments:** not compared (only `active` and `suspended`, per the rule); they are
  checked when they activate.
- **No end date:** treated as open-ended.
- **Cover deployments** for the same worker's own leave cannot exist (the worker is not the
  cover). A cover worker who is already active elsewhere at those hours is refused.
- **Concurrency:** two activations of the same worker at once: the check and the state change
  run under a lock on the employee; the second is refused.

#### Must not
- Must not block on deployments in `ended` or `cancelled`.

#### Done when
- [ ] Given W active at site A Mon–Sat 07:00–15:00 to 2027-03-31, activating W at site B
      14:00–22:00 from 2026-12-01 is refused naming the site A deployment.
- [ ] Activating W at site B 15:00–23:00 is allowed.
- [ ] Extending W's site B deployment (Fridays 08:00–12:00) into dates where A also runs Fridays
      07:00–15:00 is refused.
- [ ] Given W's site A deployment `suspended`, the site B activation with overlapping hours is
      still refused.

#### Out of scope
The attendance `overlap` flag (BL-ATT-01).

---

### BL-MPW-11 — 09 Manpower: suspend a deployment when a needed document or licence expires (`suspend`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-MPW-09 · BL-DOC-05 (`DocumentExpired` with blocked records) · BL-HRM-01 (`QualificationExpired`) · Base: 09 Replacements (`MPW.DEP.02`) · **Blocks:** BL-MPW-12
**Review items:** X-13, 09-N8-b

#### Goal
When a worker's visa, work permit, insurance, SIRA licence, health card or site pass expires
during a deployment, the worker must stop working at the site and the agency must stop billing.
The deployment is suspended: attendance is disabled, suspended days are not billed, the client
and the account manager are told with a neutral reason, and cover is requested when the
contract requires it. Staff can also suspend by hand (site access withdrawn, investigation).

#### Files to read
- `docs/verticals/09-manpower.md` §4.5 "When it goes wrong" row 1, §7.1 *Deployment* (states,
  `suspend`, paragraph after the diagram), §7.2 (`MPW.RUL.12`), §9
- `docs/modules/wl-documents.md` §5.6 point 4

#### Terms
- **Kind the deployment needs:** residence visa, work permit, Emirates ID, health insurance, and
  the site's required kinds for the role (see BL-MPW-08).
- **Cover:** a replacement deployment for the period, created through a replacement request
  (`MPW.DEP.02`) when the contract's cover obligation is on.
- **Neutral reason:** a reason text that does not disclose the worker's documents.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `suspended` | warning | no | Worker may not work at the site for now (expired document or licence, site access withdrawn); no attendance, no billing |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `suspend` | active | suspended | system (`MPW.RUL.12` on `DocumentExpired` or `QualificationExpired` for a kind the deployment needs) or `deployments.manage` | reason given (document kind, site access withdrawn, investigation) | suspension date = the day after the expiry date for a document reason; attendance at the site disabled; suspended days not billed (no attendance, so hour and day rates bill nothing for them; a month rate is reduced by the suspended calendar days over the calendar days of the month; no minimum hours for suspended days); client and account manager told with a neutral reason ("temporarily unavailable"); cover request if the contract requires it (`MPW.DEP.02`; 1 worker, same role, site and shift, from the suspension date, open-ended). On a deployment already `suspended`, a further expired kind is added to its suspension reasons (no second transition, no second client message) |

`MPW.RUL.12` (the expiry part): "When: `DocumentExpired` or `QualificationExpired` for a worker
… If: the worker has an `active`, `suspended` or `planned` deployment (the If covers all the
rule's events). Then: expired kind needed by an `active` deployment → `suspend`, tell the client
and the account manager, cover request if the contract requires it; by a `suspended` one → the
kind added to its suspension reasons (no second message); `planned` ones are not changed (their
`activate` is refused by `MPW.ACL.07`)." Compliance-critical (`PLT.RUL.04`).

§7.1: "Billing for a suspended deployment stops on the suspension date; a deployment that stays
`suspended` beyond the contract's cover terms is decided by the account manager (`end`, or keep
waiting)."

#### Edge cases decided
- **Timing:** the document is valid through its expiry date; `DocumentExpired` comes at the
  first nightly run after it, so the suspension date is the day after expiry.
- **Already suspended** for another kind: no second transition; the new kind is added to the
  deployment's suspension reasons.
- **Planned deployments:** not suspended (only `active` can be); they stay `planned` and
  `activate` will be refused (BL-MPW-08).
- **Billing:** no hours are recorded (attendance disabled), so hour and day rates bill nothing
  for suspended days (a day rate is billed per day with attendance). A month rate is reduced
  by the suspended calendar days: month rate × days not suspended ÷ calendar days of the month.
  Minimum hours are not charged for suspended days.
- **Client message (neutral):** "<worker> is temporarily unavailable at <site> from <date>.
  We will confirm the return date."
- **Cover request:** headcount 1, same role, site and shift, start = suspension date, open-ended;
  it ends on its own end date after resume (see BL-MPW-12).
- **Long suspension:** no automatic end; the deployment list shows days suspended; the account
  manager decides.
- **Idempotency:** the same `DocumentExpired` twice suspends once.

#### Must not
- Must not end the deployment automatically.
- Must not bill any suspended day.
- Must not tell the client which document expired.

#### Done when
- [ ] Given W's work permit expiring 2026-11-20 and active D at site S (contract requires
      cover), on the nightly run of 2026-11-21 D is `suspended` with reason "work permit",
      W's check-in at S is refused, the client and account manager get the neutral message, and
      a replacement request (1 × same role, from 2026-11-21) is `open`.
- [ ] November's invoice bills D only to 2026-11-20; for a monthly rate of AED 3,000, November
      bills 3,000 × 20/30 = AED 2,000; for a day rate of AED 120, only the days with attendance
      up to 2026-11-20 are billed.
- [ ] Given W's planned deployment D8 when the permit expires, D8 stays `planned` and nothing
      is sent for it.
- [ ] Given D already suspended for the work permit, the health insurance expiring adds a
      second reason and no second message to the client.
- [ ] Account manager A suspends D2 with reason "site access withdrawn"; without a reason it is
      refused.
- [ ] A failure of the `MPW.RUL.12` run alerts the MD and the admin (`PLT.RUL.04`).

#### Out of scope
Resume (BL-MPW-12); the `permit_invalid` attendance flag (BL-ATT-04); HR reactions (BL-MPW-13).

---

### BL-MPW-12 — 09 Manpower: resume a suspended deployment (`resume`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-MPW-11 · BL-MPW-08 · **Blocks:** None
**Review items:** X-13, 09-N8-b

#### Goal
When the renewed document is accepted, the deployment resumes on its own if every document and
licence check passes again; staff can also resume by hand (e.g. site access restored). The
client is told, attendance is enabled again, and any cover deployment runs to its own end date.

#### Files to read
- `docs/verticals/09-manpower.md` §4.5 row 1, §7.1 *Deployment* (`resume`), §7.2 (`MPW.RUL.12`)
- `docs/modules/wl-documents.md` §5.6 "Lifting the block"

#### Terms
- **`DocumentAccepted`:** wl-documents event when a (renewed) document becomes accepted.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `resume` | suspended | active | system (`DocumentAccepted` for the expired kind, when no other reason remains; document reasons only) or `deployments.manage` | `MPW.ACL.07` checks pass again; end date not passed; contract not `ended` | attendance enabled; client told; the cover deployment ends on its own end date; a cover request still `open` or `partially_filled` is cancelled (reason *worker returned*). When a check still fails, the deployment stays `suspended` with the reason removed for the accepted kind only, and the account manager is told the kind still failing |

`MPW.RUL.12` (excerpt): "renewed document accepted and all `MPW.ACL.07` checks pass → `resume`".

#### Edge cases decided
- **Several suspension reasons:** resume only when all checks pass; accepting one renewal while
  another kind is still expired removes that reason only.
- **Manual suspension reasons** (site access withdrawn, investigation): resumed only by staff,
  not by `DocumentAccepted`.
- **End date passed while suspended:** no resume; the deployment `end`s with reason *end date*
  on its end date through the normal end-date run.
- **Contract `on_hold`:** resume allowed (`MPW.ACL.14` covers activate and extend only).
- **Check failing on resume:** the deployment stays `suspended`; the account manager gets a
  message naming the kind still failing.
- **Cover request still `open` or `partially_filled`:** cancelled with reason "worker
  returned"; its `planned` cover deployments are cancelled by the request `cancel`; an `active`
  cover deployment runs to its own end date. A `filled` cover request is left as it is.

#### Must not
- Must not resume on `DocumentAccepted` for a kind that is not the expired one while it is
  still expired.
- Must not end the cover deployment early.

#### Done when
- [ ] Given D suspended for the work permit, when the renewed permit (expiry 2028-11-20) is
      accepted on 2026-11-28, D is `active`, W can check in at S, and the client is told.
- [ ] Given D suspended for work permit and health insurance, accepting the permit leaves D
      `suspended` with one reason left.
- [ ] Given D suspended for "site access withdrawn", `DocumentAccepted` does not resume it;
      account manager A can resume it.
- [ ] Cover deployment C ending 2026-12-15 keeps running after D resumes.
- [ ] Given the cover request `open` (nobody found) when D resumes, it is `cancelled` with
      reason "worker returned"; given it `partially_filled` (headcount 2, one active cover), it
      is `cancelled` and the active cover runs to its end date.

#### Out of scope
The guard itself (BL-MPW-08).

---

### BL-MPW-13 — 09 Manpower: end or cancel deployments on non-standard employee exits (`MPW.RUL.12`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-HRM-03 (`VisaCaseRejected`) · BL-HRM-05 (`EmployeeOnboardingCancelled`) · BL-HRM-06 (`EmployeeNoticeWithdrawn`) · BL-HRM-07 (`EmployeeAbsconded`) · BL-HRM-08 (`EmployeeReinstated`, only for the Must not) · BL-HRM-09 (`EmployeeDeceased`) · **Blocks:** None
**Review items:** X-15, 09-N12-b, 09-N12-c, 09 MEDIUM "notice withdrawal"

#### Goal
wl-hr changes the employee; the deployments must follow. When a worker absconds or dies, their
deployments end; when onboarding is cancelled, their planned deployments are cancelled and the
requests stay unfilled; when a notice is withdrawn, the end date set by `give_notice` goes back
to the one it replaced. The
client and the account manager are told each time. When a renewal visa is rejected, the
account managers and the MD are told which deployments run past the current visa's expiry.

#### Files to read
- `docs/verticals/09-manpower.md` §4.2 row "The *new* visa case is rejected or cancelled", §4.7
  rows "The notice is withdrawn", "The worker absconds", "The worker dies in service", §7.1
  *Deployment* (`end`, `cancel`, paragraph after the diagram), §7.2 (`MPW.RUL.12`)
- `docs/modules/wl-hr.md` §6 events

#### Terms
- **Scheduled end date:** the end date set on the deployment by `give_notice` (the last working
  day). `give_notice` stores the end date it replaces (or none).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Default | Scope |
|---|---|---|---|---|---|
| `MPW.RUL.12` | `DocumentExpired` or `QualificationExpired` for a worker; `DocumentAccepted`; `VisaCaseRejected` (*renewal*); `EmployeeAbsconded`, `EmployeeDeceased`; `EmployeeOnboardingCancelled`; `EmployeeNoticeWithdrawn` | the worker has an `active`, `suspended` or `planned` deployment | expired kind needed by an `active` deployment → `suspend`, tell the client and the account manager, cover request if the contract requires it; by a `suspended` one → the kind added to its suspension reasons (no second message); `planned` ones are not changed (their `activate` is refused by `MPW.ACL.07`); renewed document accepted and all `MPW.ACL.07` checks pass → `resume`; renewal visa rejected → tell the account managers and the MD which deployments run past the current visa's expiry date (no state change); absconded or deceased → `end` active and suspended deployments, `cancel` planned ones; onboarding cancelled → `cancel` planned deployments, their requests stay unfilled and the account manager is told; notice withdrawn → restore the end date stored by `give_notice`. A repeated event changes nothing | on | MVP |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `end` | active, suspended | ended | `deployments.manage` or system (end date; contract `end`; employee `exit`, `report_absconding`, `record_death`) | end reason recorded (end date, contract end, client rejected, client non-payment, replaced, exit, absconded, deceased, entered in error); *client rejected* needs the client's note; *client non-payment* only by the MD, with an end date at least the contract's notice period away unless a waiver is attached | open timesheet closed for the period; client notified; *client rejected*: replacement request if the contract requires cover; *entered in error*: its attendance rejected (hours already in a `locked` timesheet need an adjustment timesheet, `HRM.ACL.08`); worker back on the bench when still `active`; a request filled by it does not reopen (the account manager may create a replacement) |
| `cancel` | planned | cancelled | `deployments.manage` or system (`EmployeeOnboardingCancelled`, `EmployeeAbsconded`, `EmployeeDeceased`, contract `terminate` or `end`, request `cancel`) | reason given | client told when the start date was already communicated |

§7.1: "`give_notice` stores the end date it replaces; on `EmployeeNoticeWithdrawn` that end
date is restored (empty if there was none)."
§4.2: "Planned deployments of the worker are cancelled (`EmployeeOnboardingCancelled`); their
requests stay unfilled and the account manager is told."
§4.7: absconding — "deployments ended"; client "told the worker is no longer available".

#### Edge cases decided
- **End date for absconded/deceased:** the date in the event (report date; date of death);
  the absence days before it have no attendance and are not billed.
- **Planned deployments of an absconded or deceased worker:** cancelled (reason absconded or
  deceased).
- **Requests:** a request partly filled by the ended deployment does not reopen automatically;
  the account manager is told and may create a replacement.
- **Withdrawn notice:** restore the end date the deployment had before `give_notice` (stored
  at that time); if none, the end date becomes empty.
- **Client message:** absconded → "no longer available" (no reason); deceased → same neutral
  text; the account manager may add a personal note.
- **Idempotency:** a repeated event changes nothing the second time.
- **`EmployeeReinstated`:** no reaction here; a cancelled or ended deployment stays so and the
  account manager creates a new one (the event is only needed to test the Must not).
- **`VisaCaseRejected` (*renewal*):** the event carries the current visa's expiry date; the
  active, suspended and planned deployments ending after it (or with no end date) are listed
  to the account manager of each client and to the MD in one message; no deployment changes
  state, and the client is not told. A *new* or *cancellation* case rejection is ignored here.

#### Must not
- Must not tell the client that a worker absconded or died through an automatic message.
- Must not reopen a cancelled deployment on `EmployeeReinstated`; a new deployment is needed.

#### Done when
- [ ] Given W with active D1 and planned D2, when `EmployeeAbsconded` (report 2026-11-09) arrives,
      D1 is `ended` with reason *absconded* and end date 2026-11-09, D2 is `cancelled`, the client
      gets "no longer available", the account manager is notified.
- [ ] `EmployeeDeceased` for W ends D1 with reason *deceased*.
- [ ] `EmployeeOnboardingCancelled` for W with planned D3 (start communicated) cancels D3, tells
      the client, and tells the account manager that request R stays unfilled.
- [ ] Given D4 whose end date 2027-03-31 was changed to 2026-12-15 by `give_notice`,
      `EmployeeNoticeWithdrawn` restores 2027-03-31; for D4' with no end date before
      `give_notice`, the end date becomes empty.
- [ ] After `EmployeeReinstated` for W, D1 stays `ended` and D2 stays `cancelled`.
- [ ] Given W's residence visa expiring 2027-01-31, active D5 ending 2027-06-30 and D6 ending
      2027-01-15, `VisaCaseRejected` (*renewal*) tells D5's account manager and the MD about D5
      only; D5 stays `active`.

#### Out of scope
The employee transitions themselves (BL-HRM-05 to BL-HRM-10); suspension and resume (BL-MPW-11,
BL-MPW-12).

---

### BL-MPW-14 — 09 Manpower: end a deployment rejected by the client and open the replacement

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:medium` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: 09 Deployment lifecycle · Base: 09 Replacements (`MPW.DEP.02`) · Base: wl-ticketing Ticket lifecycle (`WRK.TKT.01`) · **Blocks:** None
**Review items:** 09 MEDIUM "client rejects a deployed worker", 09-N3-a (withdrawal for non-payment)

#### Goal
Deployments end for many reasons, and each needs a recorded reason. When a client rejects a
worker, the deployment ends with the client's note, a replacement request opens at once if the
contract requires cover, the worker returns to the bench, and a complaint ticket records conduct
issues. The same transition covers withdrawal for client non-payment (MD's decision) and a
deployment entered in error.

#### Files to read
- `docs/verticals/09-manpower.md` §4.1 rows "The client stops paying", "The client rejects a
  deployed worker", §7.1 *Deployment* (`end`, paragraph after the diagram)

#### Terms
- **End reason:** one of: end date, contract end, client rejected, client non-payment, replaced,
  exit, absconded, deceased, entered in error.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `end` | active, suspended | ended | `deployments.manage` or system (end date; contract `end`; employee `exit`, `report_absconding`, `record_death`) | end reason recorded (end date, contract end, client rejected, client non-payment, replaced, exit, absconded, deceased, entered in error); *client rejected* needs the client's note; *client non-payment* only by the MD, with an end date at least the contract's notice period away unless a waiver is attached | open timesheet closed for the period; client notified; *client rejected*: replacement request if the contract requires cover; *entered in error*: its attendance rejected (hours already in a `locked` timesheet need an adjustment timesheet, `HRM.ACL.08`); worker back on the bench when still `active`; a request filled by it does not reopen (the account manager may create a replacement) |

§4.1: "Deployment `end` with reason *client rejected* and the client's note. If the contract
requires cover, a replacement request opens at once (`MPW.DEP.02`); the worker returns to the
bench. A complaint ticket records the reason when conduct is involved (`WRK.TKT.01`)."
§4.1 (non-payment): "the MD decides whether to withdraw workers (deployment `end`, reason
*client non-payment*, notice per the contract)."
§7.1: "A deployment entered in error is `cancel`led while `planned`; once active it is ended with
reason *entered in error* and its attendance rejected (no `void`)."

#### Edge cases decided
- **Client rejected:** the client's note is required; the end date defaults to today.
  Replacement request: headcount 1, same role, site, shift, start tomorrow, end = the rejected
  deployment's end date.
- **Conduct involved:** a checkbox; when ticked, a complaint ticket (category conduct) is opened
  linked to the worker and the client.
- **Client non-payment:** only a user with the MD role may choose this reason; the end date must
  be at least the contract's notice period after today, unless the MD attaches a waiver.
- **Entered in error:** attendance records of the deployment are rejected; hours not billed and
  not paid (unless already in a locked timesheet, then an adjustment timesheet is required:
  BL-ATT-07).
- **Who records a rejection** coming from the client portal: staff; the portal has no rejection
  action in the MVP.

#### Must not
- Must not end without a reason.
- Must not open a replacement when the contract's cover obligation is off.

#### Done when
- [ ] Given D for W at client C (cover on), when account manager A ends D with reason *client
      rejected* and note "late three times", D is `ended`, a replacement request for 1 worker
      from tomorrow is `open`, W is on the bench, and the client is notified.
- [ ] With "conduct involved" ticked, a conduct complaint ticket exists for W and C.
- [ ] With cover off, no request is created.
- [ ] A non-MD user cannot choose *client non-payment*; MD user B ending D with an end date 10
      days ahead on a 30-day notice contract is refused without a waiver.
- [ ] Ending D with *entered in error* rejects D's 3 attendance records.

#### Out of scope
System ends (BL-MPW-06, BL-MPW-13); `MPW.RUL.13` (BL-MPW-16).

---

### BL-MPW-15 — 09 Manpower: apply a client block to deployments and the contract (`continue`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-PTY-01 · BL-PTY-03 (block effect per agreement kind) · **Blocks:** None
**Review items:** X-18

#### Goal
When a client company is blocked as a party, workers already on site continue (the agency's
obligations do not stop), but no new deployment starts and nothing is extended or renewed. The
MD decides any termination. Staff without the permission see only *On hold* as the reason.

#### Files to read
- `docs/verticals/09-manpower.md` §4.1 row "The client is blocked as a party", §7.1 *Client
  contract* (paragraph after the diagram)
- `docs/modules/wl-parties.md` §5.6

#### Terms
- **`PartyBlocked` / `PartyUnblocked`:** wl-parties events.
- **Block effect `continue`:** the agreement runs to its normal end; renewal and extension are
  refused.
- **`parties.block_reason.view`:** permission to see the real block reason (`PTY.ACL.02`).

#### Spec excerpt (authoritative for this issue)

§4.1: "Block effect for deployments: `continue` for active ones; planned ones are not activated
(wl-parties §5.6). The MD decides any ending, through `terminate`; the reason shown to staff
without `parties.block_reason.view` is *On hold*."

§7.1: "A blocked client (`PartyBlocked`, wl-parties §5.6) does not change the contract state:
deployments follow their own block effect (*continue*), and the MD decides any `terminate`."

wl-parties §5.6 table row: "Deployment | 09 vertical | continue for `active`; `planned` ones are
not activated | the vertical's decider (09: MD), per the client contract".

#### Edge cases decided
- **Refused while blocked:** deployment `activate` and `extend`; contract `renew`; request
  `submit`.
- **Suspended deployments:** stay suspended and may `resume` (resume is not an extension).
- **Messages:** no automatic message to the client about the block; the MD and the account
  manager get a task listing the client's deployments.
- **Unblock:** nothing to undo; the guards stop refusing.

#### Must not
- Must not change the contract state or end any deployment on `PartyBlocked`.
- Must not show the real reason to staff without `parties.block_reason.view`.

#### Done when
- [ ] Given client C blocked with reason `fraud_suspected` (restricted), active D continues and
      W can check in; planned D2 `activate` is refused with reason *On hold* for account manager A
      (no permission).
- [ ] `extend` of D and `renew` of C's contract are refused.
- [ ] The MD gets a task listing C's deployments.
- [ ] After `PartyUnblocked`, D2 can be activated.

#### Out of scope
Block and unblock (BL-PTY-01, BL-PTY-02); the MD's `terminate` (BL-MPW-04).

---

### BL-MPW-16 — 09 Manpower: warn when cover is not found in time (`MPW.RUL.13`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:medium` `blocked:open-item` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: 09 Replacements (`MPW.DEP.02`, `MPW.RUL.06`) · **Blocks:** None
**Review items:** 09 MEDIUM "no cover available"

#### Goal
When the contract requires cover (leave, suspension, rejection) and nobody has been found 24
hours before the cover period starts, the client and the account manager are told. Uncovered
days are not billed. Contract penalties for missing cover are not calculated (open item).

#### Files to read
- `docs/verticals/09-manpower.md` §4.1 row "No replacement can be found", §4.6, §7.2
  (`MPW.RUL.13`)
- `docs/OPEN-ITEMS.md` C-03

#### Terms
- **Replacement request:** a manpower request created for cover (`MPW.DEP.02`).
- **Cover period start:** the replacement request's start date and the shift start time.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MPW.RUL.13` | N hours before the start of a cover period (the replacement request's start date and shift start time); at once for a request created less than N hours before it | replacement request still `open` or `partially_filled` | notify the client and the account manager once that cover is not yet found; no penalty line | N = 24 | on | MVP |

§4.1: "The replacement request stays `open`; `MPW.RUL.13` tells the client and the account
manager N hours before the cover start (default 24). Uncovered days are not billed. Penalties
for missing cover follow the contract `⚠ open: client (C-03)`."

#### Edge cases decided
- **Request created less than 24 h before start** (e.g. a same-day suspension): notify at once.
- **One notice per request:** not repeated daily.
- **`partially_filled`:** notified like `open` (some cover is still missing); the notice says
  how many of the headcount are still missing.
- **Uncovered days:** nothing to bill (no deployment, no hours); no penalty line is generated.
- **Time zone:** Asia/Dubai; shift start 07:00 on 2026-11-21 → notice at 07:00 on 2026-11-20.

#### Must not
- Must not create penalty charges or credit notes for missing cover (C-03 open).

#### Done when
- [ ] Given replacement request R `open`, start 2026-11-21 07:00, at 2026-11-20 07:00 the client
      and the account manager are notified once.
- [ ] Given R `filled` by then, nothing is sent.
- [ ] Given R with headcount 2 and one cover deployment active (`partially_filled`) at
      2026-11-20 07:00, the client and the account manager are notified once.
- [ ] Given R created 2026-11-20 18:00 for 2026-11-21 07:00, the notice goes at once.

#### Out of scope
Creating replacement requests (base `MPW.RUL.06`; BL-MPW-11; BL-MPW-14).

---

### BL-MPW-17 — 09 Manpower: open and close an absence case (`MPW.RUL.09`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` · **Milestone:** Build 1 · **Size:** m
**Depends on:** Base: wl-ticketing Ticket lifecycle · BL-HRM-07 (`report_absconding`) · BL-HRM-18 (`HRM.RUL.12` releases held lines) · BL-KRN-08 (compliance-critical alert) · **Blocks:** None
**Review items:** 09-N12-c, X-15

#### Goal
When a deployed worker has not turned up for 7 consecutive calendar days without approved leave,
HR must act: the rule opens an absence case, asks for cover and tells the client, holds the next
salary payment, and reminds HR to record contact attempts and to file the MoHRE absence report
and record its number. Once both are recorded HR may report the worker as absconded. If the worker returns, the case
closes with a reason and the held pay is released.

#### Files to read
- `docs/verticals/09-manpower.md` §4.7 row "The worker absconds", §7.2 (`MPW.RUL.09`, compliance-
  critical list), §13 question 8
- `docs/modules/wl-hr.md` §5.4 "Absconding", §6 *Consumes* (apps 09 absence case)

#### Terms
- **Absence case:** a ticket of category *absence* for HR, linked to the employee and deployment;
  while it is open, payroll lines of the employee are held (wl-hr).
- **MoHRE absence report:** filed by the PRO outside the system; its number and date are
  recorded on the case.
- **Contact attempt:** recorded on the employee (date, method, by whom, outcome);
  `report_absconding` needs at least two failed attempts on different days of the absence.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MPW.RUL.09` | daily: N consecutive calendar days without attendance or approved leave, counted from the last day with either (or from the deployment's `activate` or `resume` if later); that day is day 0, so the case opens at the first run after day N | employee `active` with an `active` deployment (a worker on the bench, or whose deployments are all suspended, has no attendance expected); no absence case already open | open an *absence* case (ticket category) for HR; flag the deployment for cover (a replacement request when the contract requires it) and tell the client the worker is absent (no reason); hold the next salary payment pending the case; remind HR daily, until the case closes, to record contact attempts on the employee (at least two failed attempts on different days) and to file the MoHRE absence report and record its number; once both are recorded, HR may run `report_absconding` (`HRM.ACL.07`); the deployment is not stopped by the case; if the worker returns, HR closes the case with a reason, held pay is released (`HRM.RUL.12` proposes an off-cycle run, prepared and approved like any run, `HRM.ACL.04`) and any cover deployment runs to its end date | N = 7: the employer may report work abandonment once absence exceeds 7 consecutive days and the worker cannot be reached (ER Art. 28) ✔ [S5]; a reported worker is outside WPS while the report is valid (MR 340/2026 Art. 4) ✔ [S17] | on | MVP |

Compliance-critical (`PLT.RUL.04`): `MPW.RUL.09` — a failure alerts the MD and the admin at once.

#### Edge cases decided
- **Days:** calendar days, counted from the last day with attendance or approved leave, or
  from the deployment's `activate` or `resume` if later (aligned with `report_absconding`,
  ER Art. 28, BL-HRM-07). That day is day 0: the next day is day 1, and the case opens at the
  02:00 run after day 7 (a resume on 2026-11-03: day 7 is 2026-11-10, the case opens on the
  2026-11-11 run).
- **No active deployment (bench):** the rule does not run; there is no attendance expectation.
- **One open case per employee:** a second trigger while a case is open does nothing.
- **Cover:** a replacement request when the contract requires cover; the client is told the
  worker is absent (no further reason).
- **Reminder to record the report:** daily to HR until the number is recorded or the case
  closed.
- **Worker returns:** HR closes the case with reason (e.g. "hospitalised"); wl-hr releases
  held lines via `HRM.RUL.12`: it proposes an off-cycle run, which HR prepares and another user
  approves like any run (`HRM.ACL.04`); nothing is paid without that approval. The cover
  deployment runs to its end date.
- **Run time:** daily at 02:00 Asia/Dubai.

#### Must not
- Must not run `report_absconding` automatically.
- Must not stop the worker's deployment on opening the case.

#### Done when
- [ ] Given W deployed Mon–Sat with last attendance on Saturday 2026-10-31 and no leave, the 7th
      consecutive calendar day without attendance is Saturday 2026-11-07; at the 02:00 run on
      2026-11-08 an absence case opens for HR, a cover request opens (cover on), the client is
      told, and W's line in the next payroll run is held.
- [ ] At the 02:00 run on 2026-11-07 (6 days) nothing happens.
- [ ] Given W's deployment was suspended until a `resume` on 2026-11-03 and W never attends,
      2026-11-03 is day 0: no case opens at the runs of 2026-11-08 and 2026-11-10; the case
      opens at the 02:00 run on 2026-11-11.
- [ ] HR receives a daily reminder until two failed contact attempts (2026-11-08 by phone,
      2026-11-09 by SMS) and the report number "MOHRE-777" are recorded; then
      `report_absconding` becomes available. With one attempt only, it stays unavailable.
- [ ] When W returns on 2026-11-12 and HR closes the case with reason "hospitalised", an
      off-cycle run with W's held line is proposed in `draft`; it is paid only after a second
      user approves it.
- [ ] A failed run alerts the MD and admin.

#### Out of scope
`report_absconding` and `reinstate` (BL-HRM-07, BL-HRM-08); holding payroll lines (wl-hr base).

---

### BL-MPW-18 — 09 Manpower: escalate late visa case steps (`MPW.RUL.14`)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:medium` `blocked:open-item` · **Milestone:** Build 1 · **Size:** s
**Depends on:** Base: wl-hr Visa case lifecycle (step templates with target days) · **Blocks:** None
**Review items:** 09 MEDIUM "visa step deadlines"

#### Goal
Each visa step has a target number of days. When a step passes its target, the PRO is told;
seven days later HR and the MD are told. Statutory time limits per step are not yet verified,
so only the configured targets are used.

#### Files to read
- `docs/verticals/09-manpower.md` §4.2 row "A visa step is late", §7.1 *Visa case* (intro),
  §7.2 (`MPW.RUL.14`)
- `docs/OPEN-ITEMS.md` AU-21

#### Terms
- **Step template:** the configured checklist of steps per visa case type, each with target days.
- **Step start:** the date the previous mandatory step was completed, or the case creation date
  for the first step.
- **Passes its target:** today minus the step start > target days (day 5 of a 5-day target is
  still on time; day 6 is late).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `MPW.RUL.14` | daily: a visa case step passes its target days (today minus the step start > target, the step start being the completion of the previous mandatory step, or the case's creation); again after M more days | case `open`, `in_progress` or `on_hold`; step not done; step has a target | first: notify the PRO; second: notify HR and the MD; once each per step | targets per step in the step template (`⚠ open: authority (AU-21)` for statutory limits); M = 7 | on | MVP |

#### Edge cases decided
- **Step start and lateness:** as defined in Terms. A step started 2026-11-01 with target 5 is
  on time through 2026-11-06 (5 days) and late on 2026-11-07 (6 days); the second notice comes
  M = 7 days after the first.
- **Days:** calendar days, Asia/Dubai, daily run at 02:00.
- **`on_hold` cases** still count (the rule lists them).
- **Once each:** one first and one second notice per step.
- **Target empty** on a step: no notices for it.

#### Must not
- Must not present targets as legal deadlines (AU-21 open).

#### Done when
- [ ] Given a *new* case whose medical test step (target 5 days) started 2026-11-01 and is not
      done, nothing is sent on 2026-11-06, the PRO is notified on 2026-11-07 and HR and the MD on
      2026-11-14.
- [ ] Given the step done on 2026-11-10, no second notice is sent.
- [ ] Given the case `cancelled`, nothing is sent.

#### Out of scope
Visa case transitions (wl-hr; BL-HRM-03).

---

### BL-MPW-19 — 09 Manpower: give wl-attendance the client approver, escalation contact and deemed-approval days

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:medium` · **Milestone:** Build 1 · **Size:** s
**Depends on:** BL-ATT-10 · BL-ATT-11 · **Blocks:** None
**Review items:** 09-N5-b

#### Goal
wl-attendance escalates and may deem timesheets approved, but it reads who the approver is, who
to escalate to and after how many days from the client contract. The contract gets these fields
and the vertical implements the `ApproverResolver` provider for deployment timesheets.

#### Files to read
- `docs/verticals/09-manpower.md` §4.3 row "The client does not approve the timesheet", §6
  (Client contract fields), §7 (Timesheet paragraph), §7.2 (`HRM.RUL.15`)
- `docs/modules/wl-attendance.md` §6 (apps provide `ApproverResolver`)

#### Terms
- **`ApproverResolver`:** wl-attendance provider that returns, for a timesheet context, the
  approver kind, the escalation contact and the deemed-approval days.
- **Deemed approval:** `deem_approve` after N days if the contract has the clause.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) |
|---|---|---|---|---|
| `HRM.RUL.15` | E days after a timesheet is submitted; N days after | still `submitted` to the client | at E: client escalation contact and the account manager's manager, task to collect a signed sheet; at N: `deem_approve` and tell the client | E = 6; N = the client contract's deemed-approval days (none by default) |

§6 Client contract fields: "… cover obligation, deemed-approval days (none by default),
escalation contact, notice period, hold reason, status".

§4.3 (escalation): "… to the client's escalation contact (from the contract; skipped when the
field is empty) and the account manager's manager (the account manager's line manager, found by
wl-attendance §5; without one, the users with `timesheets.approve_on_behalf` in the client's
scope)".

§4.3: "If the contract has a deemed-approval clause, the timesheet is `deem_approve`d after its
N days … N is a contract field; without the clause nothing is deemed."

#### Edge cases decided
- **Approver:** the client approver contacts of the client company (portal), or the account
  manager with a signed sheet (`MPW.ACL.09`).
- **Who the manager is:** not this issue. The resolver returns only the contract's values
  (approver kind, escalation contact, deemed-approval days). The account manager's manager is
  resolved by wl-attendance (BL-ATT-10): the account manager's line manager in wl-hr (the
  users holding the *manager* role in the account manager's scope); without one, the users
  with `timesheets.approve_on_behalf` in the client's scope, and the task says no manager was
  found.
- **Escalation contact empty:** the resolver returns no contact; BL-ATT-10 then notifies the
  manager only.
- **Deemed-approval days:** whole days ≥ 1 or empty; empty = never deemed. The value in force
  when the timesheet was submitted applies.

#### Must not
- Must not deem approval when the field is empty.

#### Done when
- [ ] Given contract K with deemed days 10 and escalation contact "Finance Director", the
      resolver returns both for a timesheet of a K deployment.
- [ ] Given K without deemed days, the resolver returns none, and BL-ATT-11 never deems.
- [ ] Given K without an escalation contact, the resolver returns no contact, and the BL-ATT-10
      escalation at day 6 goes to the account manager's line manager only.
- [ ] Changing K's deemed days from 10 to 5 does not change a timesheet already submitted.

#### Out of scope
Escalation and deemed approval themselves (BL-ATT-10, BL-ATT-11).

---

### BL-MPW-20 — 09 Manpower: configure shared negative-flow settings (documents, parties, notifications, alerts, integrations)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-DOC-01 · BL-DOC-02 · BL-DOC-06 · BL-PTY-03 · BL-NTF-06 · BL-KRN-05 · BL-KRN-08 · BL-TKT-11 (*labour case* category) · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None
**Review items:** X-13, X-18, X-19, X-20

#### Goal
Seed the 09 starter configuration for the shared mechanisms that are not about money or
payroll: which document kinds block which deployment and attendance transitions and how they
can be overridden, the block effect for deployments, message give-up reactions, which rules are
compliance-critical and who the owner is, who sees confidential tickets (labour cases), and the
manual fallbacks when an integration is down.
It also seeds the 09 values for lead sources and campaigns (`SAL.CRM.02`), onboarding and SOPs
(`HRM.ONB.01`) and registration kinds (`DOC.EXP.01`), added on 2026-10-16: `SAL.ACL.07`,
`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`. Onboarding
checklist steps use `WRK.CHK.01` (MVP in 09 since 2026-10-16).

#### Files to read
- `docs/verticals/09-manpower.md` §3.2 (ticket visibility paragraph), §3.3 (`MPW.ACL.07`),
  §4.1 row "The client is blocked", §7.2
  (intro paragraphs), §8 (last paragraph), §9 "Documents that block work"
- `docs/modules/wl-documents.md` §5.6, §7 · `docs/modules/wl-parties.md` §5.6, §7 ·
  `docs/modules/wl-notify.md` §5.4, §7 · `docs/modules/wl-kernel.md` §7 (owner alert, integration
  down)
- `docs/modules/wl-sales.md` §4.6, §5.8 and §7 (campaigns, sources); `docs/modules/wl-hr.md`
  §5.8 and §7 (onboarding); `docs/modules/wl-documents.md` §3, §5.7 and §7 (SOPs, registration
  kinds)
- `docs/OPEN-ITEMS.md` L-19, AU-11, AU-12

#### Terms
- **Mode `block` / `flag`:** refuse the transition / let it run and flag the record.
- **Override mode:** `never` (locked) or `with_approval` (`DOC.ACL.02`).
- **Give-up reaction:** what `NTF.RUL.01` does when a message could not be delivered.

#### Spec excerpt (authoritative for this issue)

| Setting | Value (09) | Spec reference |
|---|---|---|
| `blocks` — residence visa, work permit (employee) | `deployment.activate`, `deployment.extend`, `deployment.resume` (block); `attendance_record.accept` (flag); override `never` | §9 table; `⚠ open: lawyer (L-19)` |
| `blocks` — SIRA licence (security roles) | as above; override `never` | §9 table; issuing body outside Dubai `⚠ open: authority (AU-11)` |
| `blocks` — Emirates ID, health insurance, health card, site pass | as above; override `with_approval`, approver MD (`DOC.ACL.02`) | §9 table; health card for cleaners `⚠ open: authority (AU-12)` |
| Responsible role per document kind | PRO / visa officer: residence visa, work permit, SIRA licence, Emirates ID, health insurance, health card, passport, ILOE; HR / payroll officer: labour contract; account manager: site pass, client contract, rate card acceptance, signed timesheet. For a worker's document the account manager is taken in the client companies of the affected deployments | §9 table and paragraph; wl-documents §5.2 |
| Look-ahead window for these kinds | 30 days (`MPW.ACL.07` N = 30; `DOC.RUL.02`) | §3.3, §4.5 |
| Override validity | one use, 72 hours (module default) | wl-documents §7 |
| Block effect — Deployment | `continue` for `active`; `planned` not activated; decider MD | §4.1; wl-parties §5.6 |
| Neutral reason shown | *On hold* to staff without `parties.block_reason.view` | §4.1 |
| Notification type groups (`register_type`) | termination confirmation and non-payment withdrawal notice: *Statutory notices* (marked statutory; proof task to the account manager); invoices, statements, reminders: *Invoices, statements, payment reminders*; timesheet approval requests, reminders, escalations, deemed-approval notice, signature requests: *Portal invitations, document requests, signature requests* (task to the account manager); deployment notices to the client: no group, `mark_contact_invalid` + `staff_task` to the account manager (cover not found is `urgent`); SMS to workers: no group, `mark_contact_invalid` + `staff_task` to HR; staff-only types: no group, `none` | §7.2 *Notification types* |
| Give-up reaction — SMS to workers | `mark_contact_invalid` + `staff_task` to HR (reach the worker through the site supervisor) | §7.2 (`NTF.RUL.01`, *Notification types*), §8 |
| Give-up reaction — invoices | `mark_contact_invalid` + `alternative_recipient` (other finance contact of the client), then `staff_task` to the accountant | §7.2 (`NTF.RUL.01`, *Notification types*) |
| Confidential tickets (`tickets.confidential.view`, `WRK.ACL.03`) | granted in the 09 default roles to HR / payroll officer, accountant and company admin / MD only; the *labour case* category (BL-TKT-11) and *grievance* are confidential | §3.2; wl-ticketing §7 |
| Compliance-critical rules | `HRM.RUL.03`, `HRM.RUL.04`, `HRM.RUL.09`, `HRM.RUL.10`, `HRM.RUL.11`, `HRM.RUL.14`, `MPW.RUL.09`, `MPW.RUL.12`, `DOC.RUL.03` | §7.2 |
| Owner alert recipients (`PLT.RUL.04`) | MD + company admin | §7.2 |
| Integration fallback — WPS bank or exchange house | SIF uploaded by hand in the bank's portal; bank result file attached | §8 |
| Integration fallback — SMS | give-up → task to HR via site supervisor | §8 |
| Integration fallback — attendance | paper, entered late (`HRM.ACL.01` 1 day back, late-entry flag) | §8 |
| Integration fallback — ASP (e-invoicing) | invoices still issued and sent as PDF while the ASP is down (`FIN.RUL.16`) | §8 |
| Lead sources used (`SAL.CRM.02`) | referral, website, paid ads, partner, event, other; on prospective client leads (`SAL.CRM.01`); no source requires a campaign | §5; §7.2 settings |
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | LinkedIn, search ads, email, industry events; converted value: the first client contract's first invoice; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.2 settings |
| Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for worker (deployment induction, PPE, site pass), recruiter, site supervisor and account manager, started by `HRM.RUL.16` when an employee is created in `onboarding` (`HRM.EMP.01`); induction and PPE steps use `WRK.CHK.01` checklists with photos and the worker's signature; tasks to the HR / payroll officer, the account manager and the employee; the employee's own steps recorded only by them, in the worker portal or in person with a drawn signature and a staff witness (`HRM.ACL.10`); `HRM.RUL.17` reminds overdue steps every 2 days and tells the HR / payroll officer and the line manager after 5 days; visa steps stay in visa cases; completion blocks nothing (deployment stays guarded by documents) | §5; §7.2 settings |
| SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: HR / payroll officer (HR and payroll SOPs) and account manager (client-site rules per contract); required readers by role; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the manager and the owner role after 7 days; workers acknowledge in the worker portal or in person with a drawn signature and a staff witness, never by someone else (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.2 settings |
| Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Trademark registration* and *Other certification* (subject: legal entity); responsible role MD; `DOC.RUL.01` at 90, 30 and 7 days; no `blocks` | §5; §7.2 settings |

#### Edge cases decided
- **Legal blocks (L-19):** `never` is seeded and locked; when L-19 is answered the seed changes,
  not the code.
- **Client edits:** a client may add `blocks` entries, not remove a locked one (wl-documents §7).
- **Compliance-critical list:** set by us; not client-editable (wl-kernel §7).
- **Seeding is idempotent:** re-running the seed does not duplicate entries or overwrite values
  the client changed where editing is allowed.

#### Must not
- Must not let a client switch visa, work permit or SIRA to `with_approval`.
- Must not hard-code any of these values in the deployment code.

#### Done when
- [ ] **Documents:** after seeding, the kinds carry the `blocks` entries (including
      `deployment.extend`) and override modes above; switching the work permit override to
      `with_approval` is refused.
- [ ] **Responsible role:** the work permit kind has responsible role PRO; a site pass expiring
      for worker W deployed at client C notifies C's account manager.
- [ ] **Look-ahead:** `DOC.RUL.02` uses 30 days for these kinds.
- [ ] **Parties:** a 09 deployment shows block effect `continue`, decider MD.
- [ ] **Notifications:** a failed SMS to worker W marks the number invalid and creates an HR
      task to reach W through the site supervisor; a failed invoice e-mail marks the contact
      invalid, goes to the client's other finance contact, and when that also fails creates a
      `staff_task` for the accountant; a failed termination confirmation creates a
      `statutory_notice` task for the account manager.
- [ ] **Notification types:** the registered 09 types carry these groups: contract termination
      confirmation and non-payment withdrawal notice → *Statutory notices* (statutory);
      invoice, statement, overdue reminder → *Invoices, statements, payment reminders*;
      timesheet approval request, reminder, escalation, deemed-approval notice, rate card and
      renewal signature request → *Portal invitations, document requests, signature requests*;
      deployment notices (activated, extended, suspended, resumed, ended, cancelled, worker
      absent, cover not found) → no group, `mark_contact_invalid` + `staff_task` to the account
      manager (cover not found `urgent`, with the in-app alert); worker SMS (payslip published,
      document expiry, leave decision, shift change) → no group, `mark_contact_invalid` +
      `staff_task` to HR; staff-only types → no group, `none`. A failed "deployment suspended"
      e-mail creates an account manager task.
- [ ] **Tickets:** a *labour case* ticket is visible to HR, the accountant and the MD, and not
      to an account manager or a site supervisor.
- [ ] **Alerts:** a failed `MPW.RUL.12` run alerts the MD and the admin; the 9 rules show as
      compliance-critical.
- [ ] **Integrations:** with the SMS connection `down`, the HR task path is used; with the ASP
      `down`, invoices are issued and e-mailed as PDF; for WPS, the SIF of an approved run can
      be downloaded and the bank's result file attached to the run (no bank connection exists
      in the MVP); for attendance, a supervisor enters yesterday's paper attendance, which is
      saved with the late-entry flag, while an entry two days back is refused (`HRM.ACL.01`,
      1 day back).
- [ ] **Lead sources:** a new lead or party offers the sources referral, website, paid ads,
      partner, event and other; a campaign with channel *LinkedIn*, a budget and cost entries
      reports leads, conversion, cost per lead, CAC and ROAS; a cost entry over the budget waits
      for a manager (`SAL.ACL.07`).
- [ ] **Onboarding and SOPs:** hiring a worker creates the worker checklist with tasks to the HR /
      payroll officer, the account manager and the employee (`HRM.RUL.16`), its PPE step is a
      `WRK.CHK.01` checklist signed by the worker, and the role is active before it is complete;
      a step open 2 days past its due date reminds its assignee (`HRM.RUL.17`); an HR user
      cannot mark the worker's own step done without the worker's signature and a witness
      (`HRM.ACL.10`); a new version of an SOP owned by the HR / payroll officer asks every
      required reader to acknowledge again within 7 days (`DOC.RUL.04`).
- [ ] **Registrations:** a fresh 09 deployment lists *Trademark registration* and *Other
      certification* with responsible role MD; a trademark registration expiring in 90 days
      notifies the MD.

#### Out of scope
Finance, attendance and payroll values (BL-MPW-21); the mechanisms (module epics).

---

### BL-MPW-21 — 09 Manpower: configure shared negative-flow settings (finance, attendance, payroll and HR)

**Labels:** `vertical:09-manpower` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Build 1 · **Size:** m
**Depends on:** BL-INV-01 · BL-INV-04 · BL-INV-10 · BL-PAY-09 · BL-ATT-02 · BL-ATT-04 · BL-ATT-10 · BL-ATT-12 · BL-HRM-04 · BL-HRM-14 · BL-HRM-15 · BL-HRM-19 · BL-HRM-25 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · **Blocks:** None
**Review items:** X-01, X-02, X-04, X-05, X-15, X-16, 09-N4-a, 09-N5-b, 09-N5-c, 09-N5-d, 09-N7-a, 09-N7-b, 09-N11-b

#### Goal
Seed the 09 values for the money, attendance and payroll mechanisms built in the module epics:
invoice write-off, disputes from the portal, cheque bounce handling, e-invoice rejection,
attendance flags and escalation, provisional payroll at the cut-off, WPS alerts, recovery caps,
pay account change control, exit approvals and the settlement deadline.
It also seeds the 09 values for the cash-flow forecast alert (`FIN.CFL.01`) and the
company budget with owner drawings (`FIN.BUD.01`), added on 2026-10-16: `FIN.RUL.23`,
`FIN.RUL.24` and `FIN.ACL.21`.

#### Files to read
- `docs/verticals/09-manpower.md` §3.3 (`HRM.ACL.05`–`09`), §4.3, §4.4, §4.7 "When it goes wrong"
  tables, §7 (reused machines paragraph), §7.1 *Visa case* intro and *Pay account* note, §7.2, §9
- Configuration tables (§7) of `wl-invoicing`, `wl-payments`, `wl-attendance`, `wl-hr`
- `docs/modules/wl-invoicing.md` §5.14–§5.16 and §7 (cash-flow forecast, company budget, owner
  drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments)
- `docs/OPEN-ITEMS.md` L-23, L-24, L-25, L-26, O-01

#### Terms
- **Policy `attendance`:** payroll uses accepted attendance hours, marked *provisional*, when a
  timesheet is not approved at the cut-off.

#### Spec excerpt (authoritative for this issue)

| Setting | Value (09) | Spec reference |
|---|---|---|
| Bad-debt write-off limit (`FIN.ACL.15`) | AED 0: every write-off approved | §4.3, §7, §9 |
| Client approver may raise an invoice dispute in the portal | on | §4.3, §7 |
| Dispute escalation (`FIN.RUL.15`) | 14 days | §4.3 |
| Credited dispute → adjustment timesheet effect | origin *client dispute*, default `billing`; `both` only with evidence that attendance was wrong | §4.3 |
| Adjustment of already-invoiced hours (+) | billed on the next invoice; (−) by credit note | §4.3 |
| E-invoice rejection (`FIN.RUL.16`) | task to the accountant with error codes | §4.3 |
| Bounce fee | none | §7 |
| Cheque paths after `bounced` | `represent`, `refer_to_legal`, `write_off` | §7 |
| Timesheet escalation (`HRM.RUL.15`) | E = 6 days; deemed days from the contract (none by default) | §4.3, §7.2 |
| Overtime cap (`HRM.RUL.13`) | 2 h a day, 144 h per 3 weeks | §4.3, §7.2 |
| Midday break flag (`HRM.RUL.13`) | outdoor sites, 12:30–15:00, 15 Jun–15 Sep | §2, §7.2 |
| Permit check (`HRM.RUL.14`) | work permit, residence visa, plus the site's required qualifications; hours paid, not billed until HR reviews | §4.3, §7.2 |
| Flag review (`HRM.ACL.09`) | `attendance.review` with a reason, not the marking supervisor | §3.3 |
| Payroll basis at the cut-off | `attendance` (fixed for external approvers) | §4.4, §9 |
| WPS deadline alert (`HRM.RUL.03`) | 3 days before the 1st; day 2 and day 5 after | §7.2 |
| WPS rejection escalation (`HRM.RUL.11`) | 1 day before the WPS due date | §7.2 |
| Recovery caps (`HRM.ACL.06`) | 20% a month; 50% in total | §3.3 |
| Pay account cooling (`HRM.ACL.05`) | 5 days; SMS (`NTF.MSG.03`) to the number on record before the change; workers may request a change in the portal; waiver needs the MD | §3.3, §7.1 *Pay account* |
| Non-standard exits approver (`HRM.ACL.07`) | MD | §3.3 |
| Settlement alert (`HRM.RUL.09`) | HR and the accountant 10 days after the last working day; the MD on the legal deadline, 14 days after the end of the contract | §4.7, §7.2 |
| Payroll cut-off (`HRM.RUL.02`, wl-hr `PayrollCalendar`) | day 25 of each month, for every legal entity (seeded calendar) | §7.2 |
| Absence days before `report_absconding` | more than 7 consecutive calendar days, and at least two failed contact attempts on different days | §7.1 *Employee*, §7.2 |
| Visa step templates | per emirate, with the *cancellation* variants *standard*, *absconding* (starts from the MoHRE absence report, no worker signature) and *death* (starts from the death certificate) | §7.1 *Visa case* |
| Payroll and settlement SoD (`HRM.ACL.04`) | on | §3.3 |
| Salary change approval (`MPW.ACL.06`) | above 10%, MD | §3.3 |
| Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 250,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the MD and the accountant, checked daily at 06:00; payroll by the WPS due date; visa and insurance costs by their schedule; due dates adjusted by each client's average days late: on; disputed amounts counted: off | §5; §7.2 settings |
| Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories (payroll, visa and insurance costs, accommodation, transport, office) owned by the accountant and mapped to the export accounts; approved by the MD; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.2 settings |
| Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of operating cost, margin and profit; every drawing (AED 0) approved by the MD, never by the requester (`FIN.ACL.21`) | §5; §7.2 settings |

#### Edge cases decided
- **Open legal items:** negative settlement offset (L-23), withholding from absconded workers
  (L-24), estate payee (L-25), notice withdrawal without agreement (L-26) keep the module
  defaults; nothing here decides them.
- **SIF layout per bank (O-01):** not a setting of this issue; set per client at onboarding.
- **Fixed values:** payroll basis `attendance` cannot be changed to `wait` for 09 (external
  approvers).
- **Settlement dates:** the end of the contract is the last working day recorded at
  `give_notice` (the employment ends that day), so both alerts count from the same date:
  last working day + 10 and + 14 calendar days.
- **Cut-off:** the seed writes D = 25 into the `PayrollCalendar` of each legal entity; a
  client may change it per entity.
- **Seeding is idempotent.**

#### Must not
- Must not allow a bounce fee by default.
- Must not let payroll wait for client approval.

#### Done when
- [ ] **Invoices:** a write-off of AED 500 on a 09 invoice needs approval; the client approver can
      flag an invoice `disputed` in the portal; an open dispute escalates after 14 days.
- [ ] **Cheques:** a bounced client cheque offers re-present, legal case and write-off; no bounce
      fee charge is created.
- [ ] **Attendance:** 3 overtime hours on a day are flagged `overtime_cap`; a supervisor's own
      flag cannot be accepted by that supervisor; escalation goes at day 6.
- [ ] **Payroll:** after seeding, each legal entity's `PayrollCalendar` has cut-off day 25; at
      the 2026-11-25 cut-off a deployment with an unapproved timesheet is paid on provisional
      attendance; switching to `wait` is refused; the WPS alerts fire at 3 days
      before, day 2 and day 5.
- [ ] **HR:** a new pay account activates after 5 days with SMS to the old number; a recovery of
      25% of the wage in one month is refused; `report_absconding` is approved by the MD and
      opens the *cancellation* case from the *absconding* template variant.
- [ ] **Settlement:** given a last working day (end of the contract) of 2026-11-30 and the
      settlement not `paid`, HR and the accountant are alerted on 2026-12-10 and the MD on
      2026-12-14.
- [ ] **Cash-flow forecast:** in a fresh 09 deployment, a projected balance of AED 249,999 in week
      5 alerts the MD and the accountant once (`FIN.RUL.23`, 06:00 run); AED 250,000 or more in
      every week alerts no one; the forecast shows 13 weekly buckets from Monday.
- [ ] **Budget and drawings:** a monthly budget for a legal entity prepared by the accountant
      needs approval by the MD; an expense with category *owner drawings* is left out of operating
      cost and profit reports, and the user who records it cannot approve it (`FIN.ACL.21`); an
      expense line reaching 100% of its month's budget line warns the category's owner role and
      the accountant once (`FIN.RUL.24`).

#### Out of scope
Document, party, notification and integration values (BL-MPW-20); the mechanisms (module epics).
