# Epic: 04 Accounting — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows of the 04 Accounting app (`apps/accounting`): engagement
activation guards, the unsigned letter, suspension with the statutory deadline notice, resumption,
ending mid-period and neutral reasons; CDD stale-list guards and exit; the automatic AML case on a
confirmed sanctions match; overdue tasks; tax profile change control and calendar regeneration;
return approval withdrawal, cancellation, corrections and the tax agent check on filing; KPI pack
versions and withdrawal; and the 04 values for the shared mechanisms. Module behaviour (party
block, screening, document guards, held and waived tasks, retainer pause and cancel, invoice
disputes and write-offs, give-up reactions) is built in the module epics and only consumed here.
Review items: D-09, D-10, X-13, X-18, X-20, 04-N5-c, 04-N6-a, 04-N7-b, 04-N8-a, 04-N8-b, 04-N8-d,
04-N11-a and the 04 MEDIUM items.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-ACC-01 | Refuse engagement activation on an expired KYC document, a stale screening, an open match or a blocked client | high | m | Launch 04 | Base: 04 Engagement lifecycle; BL-DOC-01; BL-DOC-06; BL-PTY-08; BL-PTY-04 |
| BL-ACC-02 | Chase an unsigned engagement letter and decline it (`ACC.RUL.11`) | medium | s | Launch 04 | Base: 04 Engagement lifecycle; Base: wl-documents e-signature (`DOC.SIG.01`) |
| BL-ACC-03 | Suspend an engagement and hold its tasks, requests and retainer | high | m | Launch 04 | BL-TKT-10; BL-INV-20; BL-PTY-03; BL-ACC-04; BL-ACC-07 |
| BL-ACC-04 | Send the client the statutory deadlines on suspend or end (`ACC.RUL.09`) | high | m | Launch 04 | BL-NTF-05; Base: wl-portal acknowledgements |
| BL-ACC-05 | Resume a suspended engagement | medium | s | Launch 04 | BL-ACC-03; BL-PTY-02 |
| BL-ACC-06 | End an engagement mid-period | high | m | Launch 04 | BL-ACC-03; BL-ACC-04; BL-ACC-17; BL-INV-21; BL-TKT-09; BL-PTY-04 |
| BL-ACC-07 | Show only neutral reasons for a restricted suspension or end (`ACC.ACL.14`) | high | s | Launch 04 | BL-PTY-01 |
| BL-ACC-08 | Refuse CDD submit and approve on a stale list or an open possible match | high | s | Launch 04 | Base: 04 CDD file lifecycle; BL-PTY-07 |
| BL-ACC-09 | Exit a client through CDD and hand the end to the partner | high | m | Launch 04 | Base: 04 CDD file lifecycle; BL-ACC-06; BL-ACC-07 |
| BL-ACC-10 | Open an AML case on a confirmed sanctions match (`ACC.RUL.12`) and close it | high | m | Launch 04 | BL-PTY-06; BL-ACC-03 |
| BL-ACC-11 | Apply a goAML report to the CDD file and the engagement | high | s | Launch 04 | Base: 04 AML case lifecycle; BL-ACC-09 |
| BL-ACC-12 | Open a CDD update task when an active client's KYC document expires | medium | s | Launch 04 | BL-DOC-05; Base: 04 CDD file lifecycle |
| BL-ACC-13 | Mark deadline tasks overdue, complete them late or close them missed (`ACC.RUL.04`) | high | s | Launch 04 | Base: wl-ticketing Task lifecycle |
| BL-ACC-14 | Approve tax profile changes with an effective date (`ACC.ACL.13`) | high | s | Launch 04 | Base: 04 Tax profile; BL-KRN-01 |
| BL-ACC-15 | Regenerate the compliance calendar after a tax profile change (`ACC.RUL.10`) | high | m | Launch 04 | BL-ACC-14; BL-TKT-09 |
| BL-ACC-16 | Withdraw a client's return approval before filing | medium | s | Launch 04 | Base: 04 Return lifecycle |
| BL-ACC-17 | Cancel a return the firm will not file | medium | s | Launch 04 | Base: 04 Return lifecycle; BL-TKT-09 |
| BL-ACC-18 | Correct a filed return by voluntary disclosure or in the next return (`ACC.ACL.04`) | high | m | Launch 04 | Base: 04 Return lifecycle; Base: wl-kernel business calendar |
| BL-ACC-19 | File a return only through a valid tax agent, flag expired KYC, ask the MLRO for a blocked client (`ACC.ACL.15`) | high | m | Launch 04 | BL-HRM-01; BL-DOC-01; BL-DOC-02; BL-KRN-01; BL-ACC-13 |
| BL-ACC-20 | Version a KPI pack on re-import and supersede the published one | medium | s | Launch 04 | Base: 04 KPI pack (`ACC.KPI.01`) |
| BL-ACC-21 | Withdraw a published KPI pack | medium | s | Launch 04 | BL-ACC-20 |
| BL-ACC-22 | Suspend an engagement for overdue invoices, never for disputed ones (`ACC.RUL.07`) | medium | s | Later | BL-ACC-03; BL-INV-04 |
| BL-ACC-23 | 04 Accounting: configure shared negative-flow settings — compliance, documents, blocks and integrations | high | m | Launch 04 | BL-DOC-01; BL-DOC-02; BL-DOC-04; BL-DOC-06; BL-HRM-01; BL-PTY-01; BL-PTY-03; BL-PTY-04; BL-PTY-06; BL-PTY-07; BL-PTY-08; BL-PTY-09; BL-INV-09; BL-INV-20; BL-KRN-05; BL-KRN-08; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |
| BL-ACC-24 | 04 Accounting: configure shared negative-flow settings — notices, receivables, tasks and portal | high | m | Launch 04 | BL-NTF-01; BL-NTF-02; BL-NTF-03; BL-NTF-04; BL-NTF-05; BL-NTF-06; BL-INV-01; BL-INV-02; BL-INV-04; BL-INV-05; BL-INV-19; BL-INV-21; BL-PAY-09; BL-PAY-14; BL-TKT-09; BL-TKT-10; BL-KRN-04; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26 |

---

### BL-ACC-01 — 04 Accounting: refuse engagement activation on an expired KYC document, a stale screening, an open match or a blocked client

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** Base: 04 Engagement lifecycle (`activate`) · BL-DOC-01 (document guard) · BL-DOC-06 (one-use override) · BL-PTY-08 (stale list guard, block mode) · BL-PTY-04 (no new agreements for a blocked party) · **Blocks:** None.
**Review items:** X-13, X-18, 04-N7-b

#### Goal
A firm must not start serving a client whose identity documents have expired, whose sanctions
screening ran on an out-of-date list, who has an undecided possible match, or who is blocked.
`activate` checks all four. The MLRO may approve a one-use override for an expired KYC document,
valid 72 hours; there is no override for the screening or the block.

#### Files to read
- `docs/verticals/04-accounting.md` §3.3 `ACC.ACL.07`, `DOC.ACL.02`, `PTY.ACL.03`, §4.1 "When it goes wrong" (rows *A KYC document expires before activation*, *Sanctions lists are stale or a possible match is open*), §7.1 *Engagement* (row `activate`), §7.2 `PTY.RUL.02`, §9 *Document blocks*
- `docs/modules/wl-documents.md` §5.6; `docs/modules/wl-parties.md` §5.4 step 7, service `Screening`

#### Terms
- **KYC documents:** the client's trade licence and the passports and Emirates IDs of its UBOs.
- **UBO:** ultimate beneficial owner (25% or more, otherwise control, otherwise senior management).
- **Stale list:** a sanctions list whose last download failed or is older than 24 hours (`PTY.RUL.02`).
- **Possible match:** a screening result in `possible_match`, not yet confirmed or dismissed.
- **MLRO:** the compliance officer (money laundering reporting officer).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `activate` | onboarding | active | `engagements.manage` | CDD `approved` (`ACC.ACL.07`); engagement letter signed; tax profile complete; client KYC documents valid (`document_valid`, block; override `DOC.ACL.02`); latest screening of each screened party of the CDD file (the client company, its UBOs, directors and signatories, as for CDD `approve`) not run on a stale list and no open possible match (`PTY.RUL.02`); client not blocked (no exception: the `PTY.ACL.03` exception covers money out only) | compliance calendar generated; retainer subscription started |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `DOC.ACL.02` *(shared)* | An expired client KYC document blocks engagement activation; a one-use override needs a reason and the MLRO's approval, valid 72 hours | `SOD`, `TIM` | manager, MLRO | 72 hours |
| `PTY.ACL.03` *(shared)* | While a client is blocked, no engagement or retainer is started, renewed or extended (no exception), and no money is paid out to it (refund of a prepaid retainer, credit balance) unless the MLRO approves an exception for that payment | `STA`, `LIM` | partner, billing | — |

`PTY.RUL.02` (part): "while stale, engagement `activate` and CDD `submit` and `approve` are refused".

§9 *Document blocks* row: "Client trade licence; UBO passport and Emirates ID (party) —
`engagement.activate` (block); `return.file` (flag) — with approval by the MLRO (`DOC.ACL.02`) —
30 days". "`BlockSubjects`: an engagement → the client company and its UBOs."

§4.6 step 6: "screenings run on a stale list are re-run after the next update, and onboarding cannot
finish meanwhile."

#### Edge cases decided
- **Order of checks and message:** all failing guards are listed in one refusal, each with what to do (renew document X, wait for list update, MLRO to decide the match). For a blocked client the message shows only "On hold" to users without `parties.block_reason.view` (`PTY.ACL.02`).
- **Which documents:** the client company's trade licence and each current UBO's passport and Emirates ID, valid on the activation date (Asia/Dubai). A missing document fails like an expired one.
- **Override:** requested by the manager with a reason, approved by the MLRO (not the requester); valid for one `activate` within 72 hours of approval; used or expired, it is gone. A rejected request leaves the engagement in `onboarding` (BL-KRN-01).
- **Screened parties:** the same set as CDD `approve` (BL-ACC-08): the client company and every UBO, director and signatory on the CDD file. (Document checks use `BlockSubjects`, client company and UBOs only; screening checks use this wider set.)
- **Stale screening:** the latest screening of any screened party carries the stale list flag, or a list is stale now → refused. It passes again after the next list update and a `clear` re-run.
- **Possible match:** an open `possible_match` on any screened party (client company, UBO, director or signatory) → refused, with no detail to users without `parties.block_reason.view` (they see "screening pending").
- **Blocked:** no exception path for `activate`: `PTY.ACL.03` allows no exception for starting an engagement or retainer (its MLRO exception is for a payment out only), so activation of a blocked client is simply refused.
- **Concurrency:** the guards are evaluated in the same transaction as the state change.

#### Must not
- Must not activate on an expired or missing KYC document without a valid override.
- Must not offer an override for a stale screening, an open match or a block.
- Must not reveal a possible match or a restricted block reason in the refusal.

#### Done when
- [ ] Given an `onboarding` engagement whose UBO passport expired 2026-11-01, `activate` on 2026-11-03 is refused naming the passport and its expiry.
- [ ] Manager A requests an override with a reason; MLRO M approves at 2026-11-03 10:00; `activate` at 2026-11-05 09:00 succeeds; a second engagement for the same client cannot reuse it; an unused override is refused at 2026-11-06 10:01.
- [ ] A cannot approve the override.
- [ ] Given the UN list stale since 2026-11-02 06:00, `activate` is refused with the list name and date; after the update and a `clear` re-run it succeeds.
- [ ] Given an open possible match on a UBO, `activate` is refused; a manager without `parties.block_reason.view` sees "screening pending" only. The same holds for an open possible match on a director or a signatory.
- [ ] Given the client blocked, `activate` is refused and a manager sees "On hold".

#### Out of scope
The guard, override and screening mechanics (BL-DOC-01, BL-DOC-06, BL-PTY-07, BL-PTY-08). The
document kind settings (BL-ACC-23).

---

### BL-ACC-02 — 04 Accounting: chase an unsigned engagement letter and decline it (`ACC.RUL.11`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: 04 Engagement lifecycle · Base: wl-documents e-signature (`DOC.SIG.01`) · **Blocks:** None.
**Review items:** 04 MEDIUM "engagement letter never signed"

#### Goal
A proposal is accepted but the engagement letter is never signed. The client approver is reminded
at 7 and 14 days; at 30 days the manager gets a task to chase or decline with the reason *letter
not signed*. A declined engagement disables the client's portal users and tells the client with a
neutral template.

#### Files to read
- `docs/verticals/04-accounting.md` §4.1 "When it goes wrong" (rows *Engagement letter never signed*, *CDD rejected*), §7.1 *Engagement* (state `declined`, row `decline`), §7.2 `ACC.RUL.11`

#### Terms
- **Sent date:** the date the letter was sent for acceptance in the portal (`DOC.SIG.01`).
- **Neutral template:** a client message that states the outcome without the reason.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `declined` | neutral | yes | Not accepted (CDD failed, letter not signed, or client withdrew) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `decline` | onboarding | declined | `engagements.manage` | reason given | client users disabled; client told with a neutral template |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `ACC.RUL.11` | N days after the engagement letter was sent (a new version re-sent restarts the count from its sent date) | not signed; engagement `onboarding` | remind the client approver; at M days, task for the manager to chase or `decline` with the reason *letter not signed* | N = 7, 14; M = 30 | on | MVP |

§4.1 row *CDD rejected*: "The engagement is `decline`d; the reason stays visible to the MLRO only;
the client is told with a neutral template."

#### Edge cases decided
- **Days:** calendar days from the sent date (Asia/Dubai); reminders at 09:00 on day 7 and day 14, the task on day 30.
- **Letter re-sent:** a new version of the letter restarts the count from its sent date; reminders already sent are not repeated for the old version.
- **Signed meanwhile:** no further reminder; an open task closes automatically.
- **Decline reasons:** *letter not signed*, *CDD rejected*, *client withdrew*, *other* with text. For *CDD rejected* the reason is visible to the MLRO only; everyone else sees "Declined".
- **Neutral template:** one template per reason group, chosen by the user; for *CDD rejected* the text never mentions due diligence.
- **Client users disabled:** their portal grants for this client company are revoked; users with grants for other companies of the group keep those.
- **Proposal and retainer:** nothing to undo (no retainer starts before `activate`).

#### Must not
- Must not decline automatically at 30 days.
- Must not show the *CDD rejected* reason to non-MLRO staff or the client.

#### Done when
- [ ] Given a letter sent 2026-11-01 and not signed, the client approver is reminded on 2026-11-08 and 2026-11-15, and the manager gets a task on 2026-12-01.
- [ ] Given the letter signed on 2026-11-10, no reminder is sent on 2026-11-15 and no task on 2026-12-01.
- [ ] When the manager declines with *letter not signed*, the engagement is `declined`, the client's portal users for that company are disabled, and the client receives the chosen neutral template.
- [ ] Given a decline with *CDD rejected*, a partner without the MLRO permission sees "Declined" and no reason.

#### Out of scope
CDD `reject` itself (base). Engagement activation guards (BL-ACC-01).

---

### BL-ACC-03 — 04 Accounting: suspend an engagement and hold its tasks, requests and retainer

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-TKT-10 (held tasks) · BL-INV-20 (retainer pause and resume) · BL-PTY-03 (block effect *pause*) · BL-ACC-04 (deadline notice) · BL-ACC-07 (neutral reasons) · **Blocks:** BL-ACC-05, BL-ACC-06, BL-ACC-10, BL-ACC-22
**Review items:** 04-N8-a, 04 MEDIUM "suspension and retainer"

#### Goal
An engagement is paused for non-payment, at the client's request, or because the client was
blocked. On `suspend` its tasks are held, no new request lists are created, open requests stop
reminding the client, the retainer pauses, and the client gets the statutory deadline notice unless
the reason is restricted. A client block suspends it automatically with the neutral reason *on hold*.

#### Files to read
- `docs/verticals/04-accounting.md` §4.2 "When it goes wrong" (row *Engagement suspended*), §4.5 (row *Engagement suspended*), §7.1 *Engagement* (state `suspended`, row `suspend`, paragraph *Block effect*), §7.1 *Task* (paragraph after the diagram), §7.1 *Subscription* (paragraph after the diagram), §7.2 `ACC.RUL.02`, `POR.RUL.01`

#### Terms
- **Held task:** a task with the *held* flag: state and due date kept, client reminders stopped (wl-ticketing).
- **Restricted reason:** sanctions or AML concern; visible only to the MLRO and deputy.
- **Request list:** the period's document request list (`ACC.RUL.02`).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `suspended` | warning | no | Paused for non-payment, at the client's request, or because the client is blocked; tasks held |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `suspend` | active | suspended | `engagements.manage`, or system (`ACC.RUL.07`; `PartyBlocked` with the neutral reason *on hold*) | reason given | open tasks held (no client reminders, no new request lists); retainer `pause`; for a reason that is not restricted, `ACC.RUL.09` sends the client the statutory deadlines due in the next 90 days and asks for acknowledgement; for a restricted reason nothing is sent (`ACC.ACL.14`). A block on an engagement already suspended runs no second transition: the shown reason becomes *on hold* and the original reason stays in the history for staff with access |

`suspend` effect (part, 04 §7.1): "for a suspension from `PartyBlocked` nothing is sent
automatically (wl-parties §5.6) and, when the block reason is not restricted, the partner gets a
task with the deadline list and the neutral template (`ACC.RUL.09`)".

*Block effect* (§7.1): "*pause*. A blocked client's engagement is suspended with the neutral reason;
ending it is the MLRO's decision, carried out by the partner through `end`. The retainer pauses;
money out stops; money in continues and, for a restricted reason, is neither refunded nor paid on
without the MLRO."

§4.2 row: "Engagement suspended — No new request lists are created; open requests stop reminding
the client." §4.5 row: "The retainer is paused (no cycle invoices) and resumes with the engagement;
time already logged stays billable."

Task paragraph: "While the engagement is suspended, its tasks are *held*: they keep their state and
due date, client reminders stop, and `mark_overdue` still runs because the due date is a fact."

#### Edge cases decided
- **Reasons (user):** *non-payment*, *client request*, *other* with text. **System:** *on hold* from `PartyBlocked` (the real block reason stays on the block record); *non-payment* from `ACC.RUL.07` (Later, BL-ACC-22).
- **Already suspended when the client is blocked:** no second transition; the suspension reason is replaced by *on hold* only for display, and the history notes the block. The original reason stays in the history, visible to staff with access.
- **Request lists:** `ACC.RUL.02` skips suspended engagements; the skipped periods are recorded so `resume` can create them (BL-ACC-05). Open requests keep their state; `POR.RUL.01` does not remind the client while suspended.
- **Time entries:** may still be logged against a suspended engagement (work already done); they stay billable.
- **Money in:** receipts are recorded as usual. For a restricted block, a receipt is not refunded or paid on without the MLRO (wl-payments; this issue only labels it).
- **Retainer:** paused through wl-invoicing with reason `on_hold` for a block (BL-INV-20); a cycle invoice already issued stays.
- **Notice:** a user suspension with a reason that is not restricted (*non-payment*, *client request*, *other*) is handed to `ACC.RUL.09` (BL-ACC-04), which sends the notice. A suspension from `PartyBlocked` (*on hold*) sends nothing automatically, whatever the block reason (wl-parties §5.6 step 3): when the real block reason is not restricted (e.g. `non_payment`, `legal_dispute`), `ACC.RUL.09` gives the partner a task with the deadline list and the neutral template; when it is restricted (`sanctions`, `aml_concern`), no task.

#### Must not
- Must not send any automatic client message for a suspension caused by a block (the partner's task for a non-restricted block is sent only when the partner decides).
- Must not stop `mark_overdue` on held tasks.
- Must not end the engagement automatically.

#### Done when
- [ ] Given an `active` engagement with 3 open tasks and a monthly retainer, when the manager suspends with *client request*, then it is `suspended`, the 3 tasks are held, the retainer is `paused`, and `ACC.RUL.09` runs.
- [ ] On the next period end, no request list is created for it, and an open request due 2026-11-05 sends no client reminder.
- [ ] Given `PartyBlocked` with reason `sanctions` for the client, the engagement is `suspended` with reason *on hold*, the retainer is paused with `on_hold`, no message goes to the client and no task is created.
- [ ] Given `PartyBlocked` with reason `legal_dispute`, the engagement is `suspended` with *on hold*, no message goes to the client, and the partner has a task with the deadline list and the neutral template.
- [ ] A held task due 2026-11-28 still becomes `overdue` on 2026-11-29.
- [ ] Time can still be logged on the suspended engagement.

#### Out of scope
The deadline notice (BL-ACC-04). Resume (BL-ACC-05). Neutral reason visibility (BL-ACC-07). The
held flag and retainer pause mechanics (BL-TKT-10, BL-INV-20).

---

### BL-ACC-04 — 04 Accounting: send the client the statutory deadlines on suspend or end (`ACC.RUL.09`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-NTF-05 (statutory notice deadline held until proof) · Base: wl-portal acknowledgements · **Blocks:** BL-ACC-03, BL-ACC-06
**Review items:** 04-N5-c

#### Goal
When the firm stops acting for a client, the client must know which tax deadlines are coming and
that the firm will not act on them. On `suspend` or `end` for a reason that is not restricted, the
client approver receives the statutory deadlines due in the next 90 days and is asked to acknowledge
in the portal; a reminder follows after 3 days and the partner gets a task after 7 days without an
acknowledgement. The notice is statutory: if it cannot be delivered, staff deliver it another way.

#### Files to read
- `docs/verticals/04-accounting.md` §4.3 "When it goes wrong" (rows *Deadlines fall inside a suspension*, *Deadline notice not delivered*), §6 *Engagement* (deadline notice and client acknowledgement), §7.2 `ACC.RUL.09` and the compliance-critical paragraph, §9 *Notices*
- `docs/modules/wl-notify.md` §5.4 (`statutory_notice`)

#### Terms
- **Statutory deadlines:** the client's compliance calendar tasks of statutory types (VAT return and payment, corporate tax registration and return, UBO updates, trade licence renewal) with a due date in the window.
- **Acknowledgement:** the client approver confirms in the portal that they read the notice; time and user recorded.
- **Statutory notice:** a notification type whose delivery failure creates a staff task to deliver it by other means and record proof.
- **Working day:** a day of the firm's kernel business calendar (Asia/Dubai; weekends and UAE public holidays excluded); the same as *business day* (04 §7). "Due in 1 working day" is the next such day.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `ACC.RUL.09` | engagement `suspend` or `end` | reason not restricted (an `end` after a CDD `exit` or a rejected review counts as restricted); for `end` with a restricted reason and for a suspension from `PartyBlocked`, see Then | send the client approver the statutory deadlines due in the next N days, stating the firm will not act on them, and ask for acknowledgement in the portal; remind after 3 days; no acknowledgement after 7 days: task for the partner. Statutory notice (give-up reaction `statutory_notice`). `end` with a restricted reason: nothing sent; task for the MLRO with the deadline list and the neutral template, sent when the MLRO decides. Suspension from `PartyBlocked`: nothing sent automatically (wl-parties §5.6); for a block reason that is not restricted (e.g. `non_payment`, `legal_dispute`), task for the partner with the deadline list and the neutral template, sent when the partner decides; for a restricted block reason, no task. Restricted `suspend`: nothing sent, no task | N = 90 | on | MVP |

§4.3 row: "On `suspend`, `ACC.RUL.09` sends the client the statutory deadlines due in the next 90
days, says the firm will not act on them while suspended, and asks the client approver to
acknowledge in the portal. No acknowledgement within 7 days: task for the partner. Not sent for a
restricted reason (`ACC.ACL.14`); on `end` for a restricted reason the MLRO gets a task with the
deadline list instead (`ACC.RUL.09`)."

§7.1 `end` effect (part): "the client gets the statutory deadlines due in the next 90 days
(`ACC.RUL.09`); for a restricted reason nothing is sent automatically and the MLRO gets a task with
the deadline list and the neutral template".

§7.2: `ACC.RUL.09` is compliance-critical (owner alert to the partner role and the company admin).

#### Edge cases decided
- **Window:** due dates from the transition date to transition date + 90 days inclusive (Asia/Dubai).
- **Wording:** suspend → "the firm will not act on these while the engagement is suspended"; end → "the firm no longer acts on these".
- **No deadline in the window:** the notice is still sent, saying there are none in the next 90 days, and still asks for acknowledgement.
- **Several client approvers:** sent to all; one acknowledgement is enough.
- **Suspension from a block:** nothing is sent automatically (wl-parties §5.6 step 3). Block reason not restricted (e.g. `non_payment`): the partner gets a task with the deadline list pre-filled and the neutral template, and sends it when they decide; the 3-day and 7-day clocks start only once it is sent. Restricted block reason: no task.
- **Restricted reason on `end`** (including every `end` after a CDD `exit` or a rejected review, BL-ACC-09): nothing is sent automatically (`ACC.ACL.14`); the MLRO gets a task with the deadline list pre-filled and the neutral template, and sends it when they decide. Restricted `suspend`: nothing is sent and no task is created.
- **Delivery failure:** handled by wl-notify `statutory_notice`: the contact point is marked invalid and a staff task (1 working day) asks for delivery by other means with proof; the 3-day and 7-day clocks start only when the notice was delivered or proof recorded (BL-NTF-05).
- **Resumed before acknowledgement:** reminders and the partner task are cancelled.
- **Compliance-critical:** a failed or missed run of the rule alerts the partner role and the company admin (BL-KRN-08).

#### Must not
- Must not send the notice automatically for a restricted reason.
- Must not start the 3-day or 7-day clock before delivery or proof of delivery.

#### Done when
- [ ] Given an engagement suspended on 2026-11-02 with *client request* and VAT return tasks due 2026-11-28 and 2027-02-28, the client approver receives a notice listing 2026-11-28 only (2027-02-28 is beyond 90 days), with an acknowledge button in the portal.
- [ ] Without acknowledgement, a reminder is sent on 2026-11-05 and the partner gets a task on 2026-11-09.
- [ ] Given the e-mail bounces on Friday 2026-11-06, the contact is marked invalid, a staff task is due Monday 2026-11-09 (the next working day after the Saturday–Sunday weekend), and no reminder is sent until proof is recorded.
- [ ] Given a suspension with reason *on hold* from a `sanctions` block, nothing is sent and no task is created; from a `non_payment` block, nothing is sent and the partner has a task with the deadline list.
- [ ] Given `end` with a restricted reason, nothing is sent and the MLRO has a task with the deadline list.

#### Out of scope
The give-up reaction mechanics (BL-NTF-01, BL-NTF-05). Suspend and end transitions (BL-ACC-03,
BL-ACC-06).

---

### BL-ACC-05 — 04 Accounting: resume a suspended engagement

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** BL-ACC-03 · BL-PTY-02 (unblock) · **Blocks:** None.
**Review items:** 04-N8-a, 04 MEDIUM "suspension and retainer"

#### Goal
When the reason for the suspension is gone, a manager resumes the engagement: tasks are released
(past-due ones are `overdue`), the retainer resumes, and the request lists for the periods skipped
while suspended are created. Whether the reason is gone is the manager's decision; the system only
checks that the client is not blocked. After a block, it resumes only once the party is unblocked
and the MLRO agrees.

#### Files to read
- `docs/verticals/04-accounting.md` §7.1 *Engagement* (row `resume`, paragraph *Block effect*), §7.1 *Task* (paragraph after the diagram)

#### Terms
- **Skipped period:** a tax period or month whose end fell while the engagement was suspended, so `ACC.RUL.02` created no request list.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `resume` | suspended | active | `engagements.manage`; after a block, only once the party is unblocked, through an approval request to the MLRO or deputy (rejected or returned: stays `suspended`) | client not blocked | tasks released, past-due ones `overdue`; retainer `resume`; request lists for skipped periods created at once, due today plus the template's lead days, with one client notice for all; pending deadline notice reminders cancelled |

#### Edge cases decided
- **Who decides the reason is gone:** a user with `engagements.manage`, recording a resume note. The only system guard is "client not blocked". For *non-payment*, the resume screen shows the client's overdue invoices and their total as information (not a guard: the firm may resume on a payment plan). For *client request* and *other*, the note records the client's instruction or what changed.
- **"The MLRO agrees":** when the suspension came from a block, `resume` creates an approval request to the MLRO (or deputy); the engagement resumes on approval. Rejected: stays `suspended` (BL-KRN-01).
- **Retainer catch-up:** none; the retainer resumes from the next cycle with the next invoice date recalculated (wl-invoicing). Cycles skipped while paused are not invoiced.
- **Past-due tasks:** already `overdue` through `mark_overdue` while held; release clears the held flag and the manager gets the list of tasks that went overdue meanwhile (BL-TKT-10).
- **Request lists:** one per skipped period, created at once with due dates from the template relative to today (due today + the template's usual lead days), and the client is notified once for all.
- **Deadline notice reminders** pending from the suspension are cancelled (BL-ACC-04).

#### Must not
- Must not resume while the client is blocked.
- Must not resume a block-caused suspension without the MLRO.

#### Done when
- [ ] Given an engagement suspended 2026-10-01 (*client request*) with monthly periods, when the manager resumes on 2026-12-05, it is `active`, held tasks are released, the retainer is `active` with its next invoice date recalculated, and request lists for October and November are created.
- [ ] Given a suspension for *non-payment* with AED 6,000 still overdue, the resume screen shows the AED 6,000 and `resume` with a note succeeds; without a note it is refused.
- [ ] Given a suspension with reason *on hold* and the party still blocked, `resume` is refused.
- [ ] Given the party unblocked, `resume` needs the MLRO's approval; on approval it is `active`.

#### Out of scope
Unblock (BL-PTY-02). Held task release mechanics (BL-TKT-10).

---

### BL-ACC-06 — 04 Accounting: end an engagement mid-period

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-ACC-03 · BL-ACC-04 · BL-ACC-17 (cancel returns) · BL-INV-21 (retainer cancel) · BL-TKT-09 (waive tasks on end) · BL-PTY-04 (money out to a blocked party) · **Blocks:** BL-ACC-09
**Review items:** 04-N8-b, 04 MEDIUM "ending mid-period"

#### Goal
An engagement can end at any point in a period, by agreement or on the MLRO's decision. Before
`end`, every open return is filed or cancelled, unbilled time is billed or written off, and a final
invoice or credit is issued. On `end` the retainer is cancelled with a pro-rata final invoice or
credit, upcoming tasks are waived, the client receives the statutory deadlines due in the next 90
days, and any refund to a blocked client waits.

#### Files to read
- `docs/verticals/04-accounting.md` §3.3 `ACC.ACL.09`, `POR.ACL.01`, §4.5 "When it goes wrong" (row *Engagement ends mid-period*), §4.6 step 5, §7.1 *Engagement* (state `ended`, row `end`), §7.1 *Task* (row `waive`, paragraph after the diagram), §7.1 *Subscription* (paragraph after the diagram)

#### Terms
- **WIP:** unbilled time (work in progress) on the engagement.
- **Pro-rata:** the retainer amount for the days of the cycle up to the end date.
- **Upcoming task:** a task in `upcoming` (not started).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `ended` | neutral | yes | Services stopped; portal read-only (`POR.ACL.01`) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `end` | active, suspended | ended | `engagements.end` (partner); after a CDD `exit`, on the MLRO's task | end date today or later (a future date keeps the engagement `active` until a scheduled `end` on that date, with the guards checked again then; if one fails, the end does not run, the partner gets a task listing the open items, and the scheduled `end` is tried again each day until it passes or the partner changes or removes the date); every open return filed or cancelled; unbilled WIP billed or written off (`ACC.ACL.09`); final invoice or credit issued | retainer `cancel` with pro-rata final invoice or credit; upcoming tasks waived; the client gets the statutory deadlines due in the next 90 days (`ACC.RUL.09`); for a restricted reason (an `end` after a CDD `exit` or a rejected review always counts as restricted) nothing is sent automatically and the MLRO gets a task with the deadline list and the neutral template; refunds to a blocked client wait (`PTY.ACL.03`) |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `ACC.ACL.09` | Writing off unbilled time above a share of the engagement's WIP needs a partner | `LIM` | manager | 10% |

§4.5 row: "The retainer is cancelled with a pro-rata final invoice or credit; unbilled WIP is billed
or written off (`ACC.ACL.09`); returns not filed are cancelled or finished first; the client gets
the list of deadlines due in the next 90 days (`ACC.RUL.09`)."

Task paragraph: "A started task that no longer applies (tax profile changed, engagement ended) is
listed for the manager, who completes it or closes it as `missed` once overdue."

#### Edge cases decided
- **Guards are checked, not performed:** `end` shows a checklist (open returns, WIP, final invoice) and is refused while any item is open; the partner resolves each through its own action.
- **Retainer pro-rata:** computed by wl-invoicing from the end date (BL-INV-21): a cycle already invoiced in advance gives a credit for the days after the end date; a cycle in arrears gives a final invoice for the days up to it. Amounts in AED, 2 decimals, half-up.
- **"Final invoice or credit issued":** satisfied by the retainer's pro-rata document, or a WIP invoice, or a recorded "nothing to bill" confirmation by the partner when both are zero.
- **Started tasks:** not waived (04 keeps *waive a started task* off); listed for the manager.
- **Refund to a blocked client:** the credit note is issued, its refund is not paid (`PTY.ACL.03`), and the MLRO gets the exception request.
- **Portal:** client users become read-only for 90 days (`POR.ACL.01`), then the portal closes.
- **End date in the past:** not allowed; the end date is today or later. A future end date keeps the engagement `active` until a scheduled `end` on that date, with the guards re-checked then.
- **Scheduled end with a failing guard:** the end does not run and the engagement stays `active`; the partner gets one task listing the open items (for example the open return); the scheduled `end` is tried again daily at the same time until the guards pass (then it ends with the scheduled date as end date, and the task closes) or the partner changes or removes the date. The pro-rata is computed from the scheduled date.

#### Must not
- Must not end with an open return.
- Must not waive a started task automatically.
- Must not pay a refund to a blocked client without an approved exception.

#### Done when
- [ ] Given an `active` engagement with a VAT return in `in_review`, `end` is refused naming the return.
- [ ] Given unbilled WIP of AED 10,000, a write-off of AED 1,500 by the manager needs a partner (above 10%); AED 800 does not.
- [ ] Given a monthly retainer of AED 3,000 invoiced in advance for November and an end date of 2026-11-15, `end` cancels the retainer with a credit of AED 1,500.00 (15 of 30 days), waives the 2 upcoming tasks, lists the 1 started task for the manager, and `ACC.RUL.09` sends the deadline notice.
- [ ] Given the client blocked, the AED 1,500 credit is issued and its refund waits for an MLRO exception.
- [ ] Client users have read-only access after `end`.
- [ ] Given an `end` scheduled for 2026-12-31 and a VAT return still `preparing` that day, the engagement stays `active` and the partner has a task naming the return; after the return is cancelled on 2027-01-04, the next daily try ends it with end date 2026-12-31.

#### Out of scope
Retainer cancel mechanics (BL-INV-21). Return cancel (BL-ACC-17). Deadline notice (BL-ACC-04). CDD
exit (BL-ACC-09).

---

### BL-ACC-07 — 04 Accounting: show only neutral reasons for a restricted suspension or end (`ACC.ACL.14`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** BL-PTY-01 (restricted reasons, `parties.block_reason.view`) · **Blocks:** BL-ACC-03, BL-ACC-09
**Review items:** 04-N11-a, X-18

#### Goal
Telling a client, even indirectly, that it is under a sanctions or AML review is tipping off, which
the law forbids. When an engagement is suspended or ended for a restricted reason, everyone except
the MLRO and deputy sees only "On hold" or "Ended", the portal shows nothing about a block, no
automatic message goes to the client, and the MLRO decides what the client is told with a neutral
template.

#### Files to read
- `docs/verticals/04-accounting.md` §3.3 `ACC.ACL.14`, `PTY.ACL.02`, `ACC.ACL.05`, §4.6 step 7, §6 *Engagement* (suspension or end reason), §9 *Block effects and no tipping off*

#### Terms
- **Restricted reason:** sanctions, AML concern.
- **Neutral code:** the reason code stored on the engagement and shown to staff without the permission: `on_hold` for a suspension, displayed "On hold"; `relationship_ended` for an end, displayed "Ended" ("relationship ended" in the text is this code; "Ended" is its label in EN, with the AR label as its translation). The portal shows the same labels without a reason line.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `ACC.ACL.14` | When an engagement is suspended or ended for a restricted reason (sanctions, AML concern), staff without `parties.block_reason.view` and the portal see only "On hold" or "Ended"; no automatic message goes to the client; the MLRO decides what the client is told, with a neutral template | `SEN`, `FLD` | all except MLRO and deputy | — |
| `PTY.ACL.02` *(shared)* | Restricted block reasons (sanctions, AML concern) and block records are visible only to the MLRO and deputy (`parties.block_reason.view`); everyone else sees "On hold"; the portal sees no block | `SEN`, `FLD` | all staff, client users | — |

§6 Engagement fields: "suspension or end reason (neutral code shown to staff)".

§4.6 step 7: "suspension, end and block reasons show as "On hold" to everyone except the MLRO and
deputy; the portal shows no block and gets no automatic message (`PTY.ACL.02`, `ACC.ACL.14`)."

#### Edge cases decided
- **Where the reason appears:** engagement page, lists, history rows, audit views, exports, reports, notifications, search results and the API. Each shows the neutral code to users without `parties.block_reason.view`.
- **Restricted reason storage:** the engagement stores only the neutral code; the real reason lives on the party's restricted Block record or the CDD file's exit reason (MLRO only).
- **Portal:** for a suspension, client users see "On hold" on the engagement; for an end, "Ended", with no reason, as `ACC.ACL.14` states.
- **Automatic messages suppressed:** the `ACC.RUL.09` notice (BL-ACC-04), the decline/end notices, retainer pause notices from wl-invoicing, request reminders. The MLRO's template choice is recorded on the engagement.
- **Neutral templates:** at least "Your engagement is on hold, please contact your partner" and "Our engagement has ended"; EN and AR.
- **Deputy:** holds the same permission.

#### Must not
- Must not store the restricted reason on the engagement.
- Must not send any automatic client message for a restricted reason.
- Must not show any block, count or badge in the portal.

#### Done when
- [ ] Given an engagement suspended by a `sanctions` block, a partner without `parties.block_reason.view` sees "On hold" on the page, in the history and in the export; the MLRO sees "sanctions" on the party's block record.
- [ ] The client user in the portal sees "On hold" and receives no e-mail.
- [ ] When the MLRO sends the neutral "on hold" template, it is recorded on the engagement.
- [ ] Given an end after a CDD exit, non-MLRO staff and the portal see "Ended".

#### Out of scope
Block and restricted reasons in wl-parties (BL-PTY-01). AML case visibility (`ACC.ACL.05`, base).

---

### BL-ACC-08 — 04 Accounting: refuse CDD submit and approve on a stale list or an open possible match

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: 04 CDD file lifecycle · BL-PTY-07 (stale list flag and re-run) · **Blocks:** None.
**Review items:** 04-N7-b

#### Goal
Due diligence must rest on a current screening. A CDD file cannot be submitted for approval if its
screening ran on a stale sanctions list, and the MLRO cannot approve it while a list is stale or a
possible match on the client or a UBO is undecided.

#### Files to read
- `docs/verticals/04-accounting.md` §4.6 step 6, §7.1 *CDD file* (rows `submit`, `approve`), checklist row *CDD file*, §7.2 `PTY.RUL.02`

#### Terms
- **CDD file:** the due diligence record of a client (risk factors, rating, EDD notes, approvals, next review date).
- **Screening done:** each of the client company and its UBOs, directors and signatories has a screening result.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | in_progress | pending_approval | `cdd.prepare` | UBOs identified; identity documents attached; screening done and not on a stale list; risk factors rated | approvers notified |
| `approve` | pending_approval | approved | `cdd.approve` (MLRO) | `ACC.ACL.06` for high risk; no open possible match; no list stale and the latest screenings not run on a stale list (`PTY.RUL.02`) | next review date set from the rating |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `PTY.RUL.02` *(shared)* | a sanctions list download fails, or a list has no successful update for N hours | screening on | alert the MLRO at once (in-app and e-mail); retry hourly; mark screenings run on the stale list and re-run them after the next update; while stale, engagement `activate` and CDD `submit` and `approve` are refused | N = 24 | on | MVP |

Checklist row *CDD file*, document guards: "screening not stale on `submit`, `approve`".

#### Edge cases decided
- **`approve` and stale lists:** `approve` checks both the open possible match and the stale list, as the transition guard, `PTY.RUL.02` and the checklist state.
- **Stale at `approve` time but not at `submit`:** refused; the file stays `pending_approval` until the list is current and the re-run is `clear`.
- **Manual screening while the download is down** (wl-parties §5.4 step 8, BL-PTY-09): an uploaded official file whose list date is not older than the current version becomes the list's current version (source *manual upload*) and clears the stale status; screenings re-run against it are not stale. The list goes stale again 24 hours after that upload if no newer download or upload comes (`PTY.RUL.02`, measured from the last successful update). A manual check against a PDF list counts as screening done.
- **Who is screened:** the client company and every UBO, director and signatory on the file; the guard reads the latest screening of each. Engagement `activate` checks the same set (BL-ACC-01).
- **Message:** names the stale list and its last update date; a possible match is shown to the MLRO only; the preparer sees "screening pending".

#### Must not
- Must not let a file be submitted or approved on a stale screening.
- Must not reveal a possible match to the preparer.

#### Done when
- [ ] Given a file whose UBO screening carries the stale flag (UN list of 2026-11-02), `submit` is refused naming the list and date.
- [ ] Given a file in `pending_approval` and the UAE list stale since 2026-11-04 09:00, `approve` by the MLRO is refused; after the update and a `clear` re-run it succeeds.
- [ ] Given an open possible match on a director, `approve` is refused; the preparer sees "screening pending".

#### Out of scope
List freshness, alerts and re-runs (BL-PTY-07, BL-PTY-09). Engagement activation (BL-ACC-01).

---

### BL-ACC-09 — 04 Accounting: exit a client through CDD and hand the end to the partner

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** Base: 04 CDD file lifecycle · BL-ACC-06 (end) · BL-ACC-07 (neutral reasons) · **Blocks:** BL-ACC-11
**Review items:** 04-N8-b

#### Goal
The MLRO may decide that the firm must stop serving an existing client: at a periodic review, after
a report, or at any time. `exit` closes the CDD file with a reason only the MLRO sees, gives the
partner a task to end the engagement with the neutral reason *relationship ended*, stops new work,
and the client is told only what the MLRO chooses, with a neutral template. A file rejected at a
review of an existing client is treated the same way.

#### Files to read
- `docs/verticals/04-accounting.md` §4.6 step 5, §6 *CDD file* (exit reason, MLRO only), §7.1 *CDD file* (states `rejected`, `exited`; rows `reject`, `exit`), §7.1 *Engagement* (row `end`, By)

#### Terms
- **Existing client:** a client with an engagement `active` or `suspended` (not `onboarding`).
- **New work:** a new engagement, service or proposal for the client.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `rejected` | error | yes | Client not accepted at onboarding or at a review |
| `exited` | error | yes | Existing relationship ended by the MLRO |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reject` | pending_approval | rejected | `cdd.approve` | reason recorded (visible to MLRO only) | onboarding: engagement `decline`; existing client: as `exit` |
| `exit` | approved, review_due, in_progress | exited | `cdd.approve` (MLRO) | reason recorded (visible to MLRO only) | task for the partner, due in 5 working days, to `end` the engagement with the neutral reason *relationship ended* (one task listing every `active` or `suspended` engagement); no new work accepted; client told only with the neutral template the MLRO chooses (`ACC.ACL.14`) |

`ACC.RUL.09` If (part): "reason not restricted (an `end` after a CDD `exit` or a rejected review
counts as restricted)". Return `cancel` effect (part): "client told who files it, except for a
restricted end reason, including any end after a CDD `exit` (`ACC.ACL.14`; the MLRO's template
tells the client instead)".

§4.6 step 5: "**MLRO exits a client:** CDD `exit` (reason visible to the MLRO only). The partner gets a
task to `end` the engagement with the neutral reason *relationship ended*; returns not filed are
cancelled. Nothing goes to the client automatically: on `end` the MLRO gets a task with the deadline
list and chooses the neutral template that tells the client which deadlines it must now meet itself
(`ACC.RUL.09`)."

#### Edge cases decided
- **"As `exit`" for a rejected review:** the file ends `rejected` (not `exited`) but all `exit` effects run.
- **Several engagements for the client:** one partner task listing each `active` or `suspended` engagement.
- **Partner task content:** "End engagement — relationship ended", without the reason; due in 5 working days.
- **No new work:** a new proposal or engagement for the client is refused with "On hold" (the CDD file is not `approved`, so `ACC.ACL.07` refuses activation anyway); existing work continues until `end`.
- **Returns not filed:** cancelled by the manager before the partner runs `end`: `end` is refused while any return is open (its guard, BL-ACC-06), so the partner's task lists the open returns and each is cancelled with the reason *engagement ended* (BL-ACC-17). No automatic "who files it" message is sent for these: an end after a CDD `exit` always counts as a restricted reason (`ACC.ACL.14`, 04 §7.1 `cancel`), and the MLRO's neutral template sent on `end` tells the client which deadlines it must now meet itself.
- **Client message:** none automatic, from the exit, the return cancellations or the `end`. On `end`, `ACC.RUL.09` treats the end as restricted whatever the exit reason, so it sends nothing and gives the MLRO a task with the deadline list; the MLRO picks the neutral template and sends it when they decide (BL-ACC-04).
- **AML case:** an exit does not open or close an AML case on its own.

#### Must not
- Must not show the exit reason to anyone but the MLRO and deputy.
- Must not end the engagement automatically.
- Must not send any automatic client message.

#### Done when
- [ ] Given an `approved` CDD file and an `active` engagement, when the MLRO runs `exit` with a reason, the file is `exited`, the partner has a task "End engagement — relationship ended", and a new proposal for the client is refused.
- [ ] A manager without the MLRO permission sees "Exited" and no reason.
- [ ] Given a `pending_approval` file of an existing client rejected at review, the file is `rejected` and the partner has the same task.
- [ ] Given a `pending_approval` file at onboarding rejected, the engagement is `declined` (no partner task).
- [ ] Given an open VAT return, the partner's `end` is refused until the manager cancels it with *engagement ended*; that cancellation sends no client message.
- [ ] No message reaches the client automatically: on `end`, `ACC.RUL.09` sends nothing and the MLRO has a task with the deadline list; the client gets the neutral template only when the MLRO sends it.

#### Out of scope
`end` itself (BL-ACC-06). Neutral reasons (BL-ACC-07). AML reporting (BL-ACC-11).

---

### BL-ACC-10 — 04 Accounting: open an AML case on a confirmed sanctions match (`ACC.RUL.12`) and close it

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-PTY-06 (confirmed match: block, freeze and report tasks) · BL-ACC-03 (suspend on block) · **Blocks:** None.
**Review items:** 04-N8-a

#### Goal
When a sanctions match on the client, its UBO, director or signatory is confirmed, the firm has 24
hours to freeze and must report. The system checks that the client company is blocked with the
restricted reason `sanctions`, opens an AML case linked to the screening and to the freeze and
report tasks, and notifies the MLRO; the block suspends the engagement and pauses the retainer, and
no message goes to the client. The case can close only once the freeze and report tasks are done.

#### Files to read
- `docs/verticals/04-accounting.md` §2 (*Targeted financial sanctions*), §4.6 step 4, §7.1 *AML case* (state `open`, rows `open`, `close`), §7.2 `ACC.RUL.12` and the compliance-critical paragraph
- `docs/modules/wl-parties.md` §5.4 steps 6 and 9
- `docs/OPEN-ITEMS.md` L-30

#### Terms
- **AML case:** the MLRO's internal case record (facts, decision, goAML reference); visible to the MLRO and deputy only (`ACC.ACL.05`).
- **Freeze task / report task:** compliance-only tasks created by wl-parties on a confirmed match, due 24 hours after the possible match was flagged.
- **`ScreeningMatchConfirmed`:** wl-parties event, delivered only to compliance listeners.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `ACC.RUL.12` | `ScreeningMatchConfirmed` for a client party or its UBO, director or signatory | — | check the client company is blocked too (wl-parties related-person block, reason `sanctions`, restricted); if it is not (related-person block off, or the person is linked only through the CDD file), block it with that reason and record this on the case; open an AML case linked to the screening, or add to the client company's open sanctions case (one per company); the block suspends the engagement (neutral reason) and pauses the retainer; no client message; MLRO notified | — | on | MVP |

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `open` | warning | no | Concern raised internally, or a confirmed sanctions match |
| `under_review` | warning | no | MLRO investigating |
| `closed_no_action` | neutral | yes | Reviewed; no report needed, reason recorded |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `open` | — | open | `aml.raise` (any staff), or system (`ACC.RUL.12` on `ScreeningMatchConfirmed`) | facts given | MLRO notified; for a sanctions match the screening and the freeze and report tasks are linked |
| `review` | open | under_review | `aml.manage` (MLRO) | — | — |
| `close` | under_review | closed_no_action | `aml.manage` | reason recorded; for a sanctions match, freeze and report tasks done | if the client was blocked with `aml_concern`, the MLRO decides whether to unblock |

§4.6 step 4: "`confirm_match` blocks the party with the restricted reason `sanctions`; wl-parties also
blocks the related client company (related-person block, on; if it did not, `ACC.RUL.12` blocks it
itself), and `ACC.RUL.12` opens an AML case linked to the screening, and the freeze and report tasks (due 24 hours after the flag) go to the
MLRO. The block suspends the engagement with the neutral reason *on hold*, pauses the retainer,
holds open tasks and stops money out (`PTY.ACL.03`). No message goes to the client (`ACC.ACL.14`).
The MLRO then decides to keep the engagement suspended or exit."

#### Edge cases decided
- **"Check the client company is blocked too":** if wl-parties already blocked it (related-person block on), nothing more; if it is not blocked (setting off, or the matched person links to the client only through the CDD file), the rule blocks the client company with reason `sanctions` (restricted) itself and records that on the case.
- **Facts on a system case:** pre-filled: matched party, role (client, UBO, director, signatory), screening, list and entry.
- **One case per confirmation:** a second `ScreeningMatchConfirmed` for the same screening opens nothing; a confirmed match on another UBO of the same client adds to the open case: one open sanctions case per client company.
- **Several engagements:** all are suspended by the block (BL-ACC-03).
- **Path to close:** a system case starts `open`; the MLRO takes it with `review` (`under_review`), then either `close` (`closed_no_action`) or `report` (BL-ACC-11). `close` is not offered from `open`.
- **Close guard:** both linked tasks `completed`; otherwise refused.
- **`close` effect for a sanctions block:** unblocking a `sanctions` block needs the sanctions evidence (wl-parties); `close` does not unblock. The `aml_concern` unblock decision is a task for the MLRO.
- **24 hours:** counted by wl-parties from the flag (L-30 open); this issue does not change the due times.
- **Compliance-critical:** a failed run of `ACC.RUL.12` alerts the partner role and the company admin (BL-KRN-08).

#### Must not
- Must not send any message to the client or show the case to client-facing staff.
- Must not close a sanctions case before the freeze and report tasks are done.
- Must not unblock the client on `close`.

#### Done when
- [ ] Given a screening of UBO "Omar K." of "Falcon Trading LLC" flagged 2026-11-03 14:00 and confirmed 2026-11-04 09:00, an AML case is `open` with the screening, the freeze task and the report task (both due 2026-11-04 14:00) linked, and the MLRO is notified.
- [ ] Falcon Trading's engagement is `suspended` with *on hold*, its retainer is `paused`, and no e-mail goes to the client.
- [ ] Given the related-company setting off, the rule blocks Falcon Trading with `sanctions` and notes it on the case.
- [ ] The MLRO runs `review` and the case is `under_review`; `close` with the report task still open is refused; with both done and a reason it is `closed_no_action` and the client stays blocked.
- [ ] A partner without the MLRO permission cannot see the case.

#### Out of scope
Block, freeze and report tasks (BL-PTY-06). Report effects (BL-ACC-11). goAML filing (manual).

---

### BL-ACC-11 — 04 Accounting: apply a goAML report to the CDD file and the engagement

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: 04 AML case lifecycle (`review`, `report`) · BL-ACC-09 (exit) · **Blocks:** None.
**Review items:** 04-N8-a, 04-N8-b

#### Goal
When the MLRO files a suspicious transaction report on goAML and records the reference on a case
`under_review`, the CDD file goes to `review_due` if it is `approved` (otherwise the compliance
officer gets a "review CDD after report" task), the partner is informed without client-facing
staff, and the MLRO decides what happens to the engagement: continue, suspend with a neutral
reason, or exit.

#### Files to read
- `docs/verticals/04-accounting.md` §3.3 `ACC.ACL.05`, §4.6 step 3, §7.1 *AML case* (row `report`), §7.1 *CDD file* (row `flag_review`)

#### Terms
- **goAML:** the UAE Financial Intelligence Unit's reporting system; filed by hand in the MVP.
- **Engagement decision:** the MLRO's recorded choice: *continue*, *suspend* or *exit*.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `under_review` | warning | no | MLRO investigating |
| `reported` | error | yes | Suspicious transaction report filed on goAML |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `review` | open | under_review | `aml.manage` (MLRO) | — | — |
| `report` | under_review | reported | `aml.manage` | goAML reference and date recorded | CDD `flag_review` when the file is `approved`, otherwise a task for the compliance officer "review CDD after report"; partner told only that the MLRO needs to discuss the client, with no access to the case (`ACC.ACL.05`); task for the MLRO, due 1 working day, to record the engagement decision: continue, suspend (neutral reason) or CDD `exit` |
| `flag_review` | approved | review_due | system | next review date reached, or an AML case `report` (from `approved` only; a file in another state gets the task without a transition) | task for the compliance officer |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `ACC.ACL.05` | AML cases and suspicious-activity reports are visible only to the MLRO and their deputy, never to client-facing staff or the portal (no tipping off) | `SEN`, `FLD` | all except MLRO | — |

#### Edge cases decided
- **CDD not `approved` at report time** (already `review_due`, `in_progress` or `pending_approval`): `flag_review` cannot run: no transition; a task for the compliance officer "review CDD after report" is created instead.
- **"Partner informed":** a notification to the engagement's partner that says "The MLRO needs to discuss client X with you", with no case detail; the partner does not get access to the case.
- **Engagement decision:** recorded on the case. *Suspend* runs engagement `suspend` with the neutral reason *on hold* and the restricted reason `aml_concern` on a party block (BL-PTY-01); *exit* runs CDD `exit` (BL-ACC-09); *continue* changes nothing.
- **Decision required:** the MLRO gets a task to record the decision, due 1 working day after `report`.

#### Must not
- Must not show the report, its reference or its date to the partner or client-facing staff.
- Must not suspend or exit without the MLRO's recorded decision.

#### Done when
- [ ] Given a case `under_review` for a client with an `approved` CDD file, `report` with reference "GOAML-2026-0042" and date makes the case `reported`, the file `review_due`, the compliance officer gets a task, and the partner gets a notification without case detail.
- [ ] `report` without a reference is refused.
- [ ] When the MLRO records *suspend*, the engagement is `suspended` with *on hold* and the client gets no message.
- [ ] Given the file already `review_due`, `report` creates the "review CDD after report" task instead.

#### Out of scope
Opening and closing cases (BL-ACC-10). Exit (BL-ACC-09).

---

### BL-ACC-12 — 04 Accounting: open a CDD update task when an active client's KYC document expires

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** BL-DOC-05 (`DocumentExpired` with blocked records; lifting on renewal) · Base: 04 CDD file lifecycle · **Blocks:** None.
**Review items:** X-13

#### Goal
An expired trade licence or UBO passport of a client that is already being served does not suspend
the engagement. Instead the CDD file gets an update task, and the client's returns are flagged at
filing (BL-ACC-19). When the renewed document is accepted, the task closes.

#### Files to read
- `docs/verticals/04-accounting.md` §9 *Document blocks* (last paragraph)
- `docs/modules/wl-documents.md` §5.6 (*Lifting the block*), event `DocumentExpired`, `DocumentAccepted`

#### Terms
- **CDD update task:** a task for the compliance officer (and the engagement's manager) to collect the renewed document and update the CDD file.

#### Spec excerpt (authoritative for this issue)

§9: "`BlockSubjects`: an engagement → the client company and its UBOs; a return → its client and its
filer. `FutureWork`: tasks and returns due after the expiry date. Expired KYC documents of an active
client do not suspend the engagement on their own; they flag returns and open a CDD update task for
the compliance officer (the manager as watcher), due 14 days after the expiry, one per document and
client, completed when the renewal is accepted."

#### Edge cases decided
- **Trigger:** `DocumentExpired` for a KYC kind whose subject is a client company or a UBO of a client with an `active` or `suspended` engagement.
- **Assignee:** the compliance officer; the engagement's manager is a watcher (to ask the client). Due 14 days after the expiry.
- **One task per document:** a second event for the same document does nothing; a UBO of several clients gives one task per client.
- **Closing:** `DocumentAccepted` for a renewal of that kind and subject completes the task automatically.
- **Onboarding engagements:** no task (activation is blocked instead, BL-ACC-01).
- **No state change** on the engagement or the CDD file.

#### Must not
- Must not suspend the engagement because a KYC document expired.

#### Done when
- [ ] Given an `active` engagement and the client's trade licence expiring 2026-11-10, on 2026-11-11 the compliance officer has a CDD update task due 2026-11-24 and the engagement stays `active`.
- [ ] When the renewed licence is accepted, the task is completed.
- [ ] A second `DocumentExpired` for the same document creates no second task.

#### Out of scope
Return flag at filing (BL-ACC-19). Activation block (BL-ACC-01).

---

### BL-ACC-13 — 04 Accounting: mark deadline tasks overdue, complete them late or close them missed (`ACC.RUL.04`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: wl-ticketing Task lifecycle (`mark_overdue`, `close_missed`) · **Blocks:** BL-ACC-19
**Review items:** D-09, 04 MEDIUM

#### Goal
A missed filing date is a fact, not the end of the work: the return can still be filed late. When a
deadline task's due date passes, `ACC.RUL.04` marks it `overdue` and tells the assignee, manager and
partner; it can still be completed and is then flagged *late*. A manager closes it as `missed` only
with a reason.

#### Files to read
- `docs/verticals/04-accounting.md` §4.2 (row *The client never uploads*), §4.3 "When it goes wrong" (row *Due date passes*), §7.1 *Task* (states `overdue`, `completed`, `missed`; rows `start`, `mark_overdue`, `close_missed`), §7.2 `ACC.RUL.04` and the compliance-critical paragraph

#### Terms
- ***late* flag:** set on a task completed after passing through `overdue`; shown in reports "completed late".

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `overdue` | error | no | Due date passed, not completed; can still be completed |
| `completed` | success | yes | Done, with evidence; flagged *late* if it passed through `overdue` |
| `missed` | error | yes | Closed as not done, with a reason |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `start` | upcoming, overdue | in_progress | `tasks.work` | — | — |
| `mark_overdue` | upcoming, in_progress, waiting_on_client, in_review | overdue | system (`ACC.RUL.04`) | due date passed | assignee, manager and partner notified |
| `close_missed` | overdue | missed | `tasks.manage` | reason given | next occurrence generated |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `ACC.RUL.04` | due date passed | task not `completed`, `missed` or `not_required` | task `mark_overdue` (the task can still be completed and is flagged *late*); notify assignee, manager and partner | — | on | MVP |

`ACC.RUL.04` is compliance-critical (owner alert to the partner role and the company admin).

#### Edge cases decided
- **"Due date passed":** the run at 00:30 Asia/Dubai on the day after the due date. A task due 2026-11-28 becomes `overdue` on 2026-11-29.
- **Already `overdue`:** skipped (idempotent).
- **Completing from `overdue`:** `start` returns it to `in_progress` with the stored position (wl-ticketing keeps where the work stood); `complete` then sets *late*.
- **Held tasks** (suspended engagement): still marked overdue; the client is not notified.
- **`close_missed` reason list:** client did not provide documents, client filed itself, engagement ended, other with text.
- **Partner:** the engagement's partner; if none, the partner role of the firm.

#### Must not
- Must not make `overdue` terminal or lock the task.
- Must not close a task as `missed` automatically.

#### Done when
- [ ] Given a VAT return task due 2026-11-28 in `waiting_on_client`, on 2026-11-29 it is `overdue` and the assignee, manager and partner are notified.
- [ ] Starting and completing it on 2026-12-02 with evidence makes it `completed` with *late*.
- [ ] `close_missed` without a reason is refused; with *client filed itself* it is `missed` and the next occurrence exists.
- [ ] When `ACC.RUL.04` fails to run, the partner role and the company admin get the owner alert.

#### Out of scope
Held tasks (BL-TKT-10). Owner alert mechanics (BL-KRN-08).

---

### BL-ACC-14 — 04 Accounting: approve tax profile changes with an effective date (`ACC.ACL.13`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: 04 Tax profile (`ACC.TAX.01`) · BL-KRN-01 (approval returned) · **Blocks:** BL-ACC-15
**Review items:** 04-N6-a

#### Goal
The tax profile drives the whole compliance calendar; a wrong stagger or year end means missed
deadlines. A change to any calendar-driving field takes effect only after a manager other than the
requester approves it, with an effective date. The profile keeps every version.

#### Files to read
- `docs/verticals/04-accounting.md` §3.3 `ACC.ACL.13`, §4.3 "When it goes wrong" (row *Tax profile changes*), §6 *Tax profile* (versioned by effective date)

#### Terms
- **Calendar-driving fields:** VAT period and stagger, financial year end, VAT or corporate tax registration or deregistration, Small Business Relief, QFZP status.
- **Effective date:** the date from which the new values apply.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `ACC.ACL.13` | A change to the tax profile fields that drive the calendar (VAT period and stagger, financial year end, VAT or corporate tax registration or deregistration, Small Business Relief, QFZP status) takes effect only after a manager other than the requester approves it, with an effective date; one pending change per profile at a time | `SOD` | accountant, tax specialist, manager | on |

§6 Tax profile: "Client's tax facts, versioned by effective date — TRN, VAT period and stagger,
financial year end, CT registration no., small business relief, QFZP status, effective date,
approved by".

#### Edge cases decided
- **Other fields** (contacts, notes): no approval.
- **Pending change:** shown on the profile as "pending approval"; the calendar uses the current version until approval.
- **Two pending changes:** only one pending change per profile; a second request is refused until the first is decided.
- **Effective date in the past:** allowed (the FTA often confirms stagger changes late); the approver sees the past date highlighted; `ACC.RUL.10` regenerates from that date.
- **Approval rejected or returned:** the current version stays (BL-KRN-01).
- **Approver:** a user with the manager role on the engagement or global; not the requester even if a manager.

#### Must not
- Must not change the calendar-driving fields without approval.
- Must not overwrite a version: each approval creates a new one.

#### Done when
- [ ] Given accountant A changes the VAT stagger from "quarter ending Jan" to "quarter ending Feb" effective 2027-02-01, the profile shows the change pending and the calendar is unchanged.
- [ ] Manager B approves; the profile has a new version effective 2027-02-01 approved by B, and `ACC.RUL.10` runs.
- [ ] Manager A cannot approve their own change.
- [ ] Changing a contact e-mail needs no approval.

#### Out of scope
Calendar regeneration (BL-ACC-15).

---

### BL-ACC-15 — 04 Accounting: regenerate the compliance calendar after a tax profile change (`ACC.RUL.10`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-ACC-14 · BL-TKT-09 (waive with reason; started tasks listed) · **Blocks:** None.
**Review items:** 04-N6-a

#### Goal
After an approved tax profile change, the calendar is rebuilt from the effective date: tasks for
periods that end before the effective date are left as they are; for the rest, upcoming tasks that
no longer apply are waived with the reason *tax profile changed*, the new tasks are
created, and anything already started or overdue that no longer matches, plus returns in progress
for affected periods, is listed for the manager to decide.

#### Files to read
- `docs/verticals/04-accounting.md` §4.3 "When it goes wrong" (row *Tax profile changes*), §7.1 *Task* (row `waive`, paragraph after the diagram), §7.2 `ACC.RUL.10`

#### Terms
- **No longer applies:** a task generated from the old profile for a period ending on or after the effective date, whose period, type or due date does not exist under the new profile.
- **Compared tasks:** only tasks for periods ending on or after the effective date. A period that ended before it belongs to the old profile and its task is kept as it is, whatever its state.
- **First new period:** starts on the effective date and ends on the new profile's first period end on or after it (a transitional period when the stagger changes, as the FTA notice sets it).
- **Manager list:** a task for the engagement manager listing items to decide.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `ACC.RUL.10` | tax profile change approved (`ACC.ACL.13`) | engagement `active` or `suspended` | regenerate the calendar from the effective date (tasks for periods that end before the effective date are not compared and stay as they are; the new profile's first period starts on the effective date and ends on the new profile's first period end on or after it): upcoming tasks that no longer apply `waive` with the reason *tax profile changed*; an upcoming task of the same type and period whose due date changed gets the new due date (audited) instead of being waived and recreated; new tasks created (no duplicate of a task that still matches); started or overdue tasks that no longer match, and returns in progress for affected periods, listed for the manager | — | on | MVP |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `waive` | upcoming | not_required | `tasks.manage` or system (`ACC.RUL.10`, engagement `end`) | reason given | next occurrence generated, unless the engagement ended or the waive comes from `ACC.RUL.10` (regeneration creates the new tasks) |

#### Edge cases decided
- **Matching:** a task matches when the new profile produces a task of the same type and period with the same due date; it is kept unchanged. Same type and period with a different due date: an `upcoming` task gets the new due date (audited) instead of waive + create; a started one is listed.
- **"Next occurrence generated" on `waive`:** suppressed for waives by `ACC.RUL.10`, because regeneration creates the new tasks; no duplicate occurrence.
- **Deregistration:** every upcoming task of that tax from the effective date is waived; the final return task required on deregistration is created by the template.
- **Suspended engagement:** new tasks are created held.
- **Idempotency:** running twice for the same version changes nothing the second time.
- **Effective date in the past:** tasks for periods ending on or after the effective date, with due dates before today, that no longer apply and are not started are waived; overdue ones are listed (never waived automatically).

#### Must not
- Must not waive a started or overdue task automatically.
- Must not delete tasks.
- Must not create duplicates of tasks that still match.

#### Done when
- [ ] Given quarterly VAT tasks for quarters ending 2027-01-31 and 2027-04-30 (`upcoming`) and a change to quarters ending Feb effective 2027-02-01, the 2027-01-31 task is kept unchanged (its period ends before the effective date), the 2027-04-30 task is waived with *tax profile changed*, and tasks are created for the first new period 2027-02-01 to 2027-02-28 and for the quarter ending 2027-05-31.
- [ ] Given the 2027-04-30 task already `in_progress` and no longer matching, it is listed for the manager and not waived; the 2027-01-31 task is not listed, whatever its state.
- [ ] Given a VAT return in `preparing` for an affected period, it is on the manager list.
- [ ] Given an `upcoming` task whose period stays but whose due date moves from 2027-05-28 to 2027-06-28, it keeps its ID with the new due date (audited) and is not waived.
- [ ] Running the rule twice creates no extra tasks.

#### Out of scope
Profile approval (BL-ACC-14). Waive mechanics and the "waive a started task" setting (BL-TKT-09).

---

### BL-ACC-16 — 04 Accounting: withdraw a client's return approval before filing

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: 04 Return lifecycle (`approve`, `DOC.SIG.01`) · **Blocks:** None.
**Review items:** 04 MEDIUM "withdraw approval before filing"

#### Goal
A client who approved a return may find an error before the firm files it. The client approver (or
a reviewer on the client's written instruction) withdraws the approval with a reason; the return
goes back to `preparing`, the signature is kept and marked withdrawn, the preparer and the
reviewer are notified, and the preparer gets a task to rework the return and bring the linked task
back to work.

#### Files to read
- `docs/verticals/04-accounting.md` §4.4 "When it goes wrong" (row *Client withdraws its approval before filing*), §7.1 *Return* (row `withdraw_approval`), §7.1 *Task* (rows `start`, `send_back`)

#### Terms
- **Signature:** the in-portal acceptance record (name, time, IP address, document hash) from `DOC.SIG.01`.
- **Linked task:** the compliance calendar task for this return.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `withdraw_approval` | approved | preparing | client approver, or `returns.review` on the client's written instruction | reason given; a reviewer attaches the written instruction | signature kept and marked withdrawn; preparer and reviewer notified; the linked task is not moved back to work by the system (`mark_overdue` may still move it to `overdue`): the preparer gets a task (due 1 working day, reason "client withdrew approval") to rework the return and bring the linked task back to work through its own transitions (`send_back` by the reviewer from `in_review`, `start` from `overdue`) |

§4.4 row: "`withdraw_approval` returns it to `preparing`; the signature is kept and marked withdrawn;
the preparer gets a task to rework the return and bring the linked task back to work."

#### Edge cases decided
- **Which client approver:** any user of the client with the *approver* portal role (`ACC.ACL.11`), not only the one who approved.
- **Written instruction:** attached to the transition when a reviewer runs it; refused without an attachment.
- **Linked task:** not moved back to work by the system: wl-ticketing has no system transition into `in_progress`. The only system move is `mark_overdue` (`ACC.RUL.04`, BL-ACC-13), which still takes a linked task from `in_review` (or `in_progress`) to `overdue` when its due date passes, before or after the withdrawal. The preparer's task names the linked task and its current state: from `in_review` the reviewer runs `send_back`, from `overdue` the preparer runs `start`, from `in_progress` nothing is needed. The task closes when the return is submitted again.
- **After `filed`:** not offered; a filed return is corrected (BL-ACC-18).
- **Resubmission:** the return goes through `submit`, `review` and a new client `approve`; the withdrawn signature stays in the history.

#### Must not
- Must not delete the withdrawn signature.
- Must not allow withdrawal after filing.

#### Done when
- [ ] Given an `approved` VAT return with linked task in `in_review`, when the client approver withdraws with reason "sales figure wrong", the return is `preparing`, the signature is marked withdrawn, the preparer and reviewer are notified, the linked task stays `in_review`, and the preparer has a task "client withdrew approval" due the next working day.
- [ ] A reviewer withdrawing without an attached instruction is refused.
- [ ] Given the linked task still `in_review` when its due date 2026-11-28 passes after the withdrawal, `ACC.RUL.04` makes it `overdue` on 2026-11-29 and the preparer's task now says to `start` it.
- [ ] A `filed` return offers no withdrawal.

#### Out of scope
`send_back` (base). Corrections (BL-ACC-18).

---

### BL-ACC-17 — 04 Accounting: cancel a return the firm will not file

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: 04 Return lifecycle · BL-TKT-09 (waive or hand over) · **Blocks:** BL-ACC-06
**Review items:** 04-N8-b, 04 MEDIUM "ending mid-period"

#### Goal
A return in progress may no longer be the firm's to file: the engagement ends, the client decides to
file itself, or the return is not required. A manager cancels it with a reason; the linked task is
waived or handed over, and the client is told who files it now.

#### Files to read
- `docs/verticals/04-accounting.md` §4.4 "When it goes wrong" (row *Return no longer needed*), §7.1 *Return* (state `cancelled`, row `cancel`)

#### Terms
- **Handed over:** the linked task stays with the client as "client files" instead of being waived, so the deadline remains visible in the portal.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `cancelled` | neutral | yes | Not filed by the firm (engagement ended, client files itself, not required) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | preparing, in_review, client_approval, approved | cancelled | `returns.review` (manager) | reason given (*engagement ended*, *client files itself*, *not required*) | linked task: *not required* → waived when `upcoming`, otherwise listed for the manager; *client files itself* or *engagement ended* → handed over (completed with the evidence "handed over to client"; the portal shows the deadline as the client's); client told who files it, except for a restricted end reason (`ACC.ACL.14`) |

#### Edge cases decided
- **Reasons:** *engagement ended*, *client files itself*, *not required*.
- **Waived or handed over:** *not required* → task waived (`not_required`) when `upcoming`; otherwise listed for the manager (04 keeps "waive a started task" off). *client files itself* and *engagement ended* → handed over: the task is completed with evidence "handed over to client" and the portal shows the deadline as the client's.
- **Client message:** names who files and the due date; for a restricted end reason it is not sent automatically (`ACC.ACL.14`; the MLRO decides).
- **Approved return:** the signature is kept; the return is never filed by the firm.

#### Must not
- Must not cancel a `filed` return.
- Must not cancel without a reason.

#### Done when
- [ ] Given a VAT return in `client_approval`, when the manager cancels with *client files itself*, it is `cancelled`, the linked task is handed over, and the client is told it files the return due 2026-11-28 itself.
- [ ] Given *not required* and an `upcoming` task, the task is `not_required`.
- [ ] A `filed` return offers no `cancel`.

#### Out of scope
Engagement end (BL-ACC-06). Task waive mechanics (BL-TKT-09).

---

### BL-ACC-18 — 04 Accounting: correct a filed return by voluntary disclosure or in the next return (`ACC.ACL.04`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** Base: 04 Return lifecycle · Base: wl-kernel business calendar · **Blocks:** None.
**Review items:** D-10

#### Goal
A filed return is locked; a correction is a new version. For VAT, an error above AED 10,000 of tax
needs a voluntary disclosure within 20 business days of finding it; an error of AED 10,000 or less is
corrected in the next period's return, which the new version names. Corporate tax corrections always
need a voluntary disclosure until an accountant confirms otherwise (A-14).

#### Files to read
- `docs/verticals/04-accounting.md` §3.3 `ACC.ACL.04`, §4.4 step 7, §6 *Return* (correction route), §7.1 *Return* (state `filed`, `superseded`; row `correct`; paragraph after the diagram), §9 *Corrections of filed returns*
- `docs/OPEN-ITEMS.md` A-14

#### Terms
- **Voluntary disclosure:** the taxpayer's correction filed with the FTA; its reference is recorded.
- **Error amount:** the absolute difference in tax between the filed figures and the correct figures.
- **Business day:** a working day in the firm's kernel business calendar (Asia/Dubai; weekends and UAE public holidays excluded).

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `ACC.ACL.04` | A filed return is locked; a correction is a new version. For VAT, an error above AED 10,000 of tax needs a voluntary disclosure reference; a smaller error is corrected in the next period's return, which the new version names ✔ [S18]. Corporate tax corrections need a voluntary disclosure reference until an accountant confirms otherwise (`⚠ open: accountant (A-14)`) | `STA` | all staff | AED 10,000 |

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `filed` | success | no | Filed on EmaraTax; acknowledgement attached; locked (`ACC.ACL.04`) |
| `superseded` | neutral | yes | Replaced by a corrected version |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `correct` | filed | superseded | `returns.prepare` | reason; error amount and the date it was found; route, fixed at `correct`: *voluntary disclosure* (always for VAT errors above AED 10,000 of tax, and for corporate tax; the reference may be entered later, but the disclosure task cannot be completed without it) or *next return* (VAT errors of AED 10,000 or less; a later VAT return of the client not yet `filed` is named, or only its period when it does not exist yet) (`ACC.ACL.04`) | new `preparing` version created; for a voluntary disclosure, a task due 20 business days after the error was found ✔ [S18]; for the next-return route, the adjustment is attached to that return (when it is opened, if named by period), and the new version, marked *corrected via next return* and never filed itself, is `cancel`led with the reason *not required* once the named return is filed |

"`filed` is no longer terminal only because `correct` leaves it; the filed version itself is never
edited."

#### Edge cases decided
- **Inputs on `correct`:** reason, error amount (AED, 2 decimals), date the error was found, route, and the reference or the named return.
- **Threshold:** AED 10,000.00 exactly → *next return* allowed; AED 10,000.01 → *voluntary disclosure* only. The threshold is the `ACC.ACL.04` parameter.
- **Reference "required" when the disclosure is not yet filed:** the reference may be entered later on the task; the task (due 20 business days after the error was found) cannot be completed without it. The route is fixed at `correct`.
- **Due date:** counted from the found date (day 0) over business days; for an error found on Thursday 2026-11-05, with the kernel calendar holding public holidays on 2026-12-02 and 2026-12-03, the task is due Monday 2026-12-07.
- **Named next return:** a VAT return of the same client for a later period, not yet `filed`. If none exists yet, the user names the period and the adjustment attaches when that return is opened.
- **Corporate tax:** only *voluntary disclosure* is offered (A-14 open, `blocked:open-item`).
- **New version:** copies figures and workpapers; it goes through review and client approval again; for the next-return route, the corrected figures are recorded but the new version is not filed separately. The new version carries the marker "corrected via next return" (no new state) and is cancelled with the reason *not required* once the named return is filed.
- **Rounding:** none; amounts as entered.

#### Must not
- Must not edit a filed version.
- Must not offer *next return* for corporate tax or for a VAT error above AED 10,000.

#### Done when
- [ ] Given a filed VAT return and an error of AED 12,500 found on 2026-11-05, `correct` offers only *voluntary disclosure*; the old version is `superseded`, a new `preparing` version exists, and a task is due 2026-12-07 (public holidays 2026-12-02 and 2026-12-03 in the calendar).
- [ ] Given an error of AED 10,000.00, *next return* is allowed; naming the Q4 2026 return attaches the adjustment to it; when the Q4 2026 return is filed, the new version is `cancelled` with *not required*.
- [ ] Given a corporate tax return, only *voluntary disclosure* is offered.
- [ ] The disclosure task cannot be completed without the reference.
- [ ] The superseded version cannot be edited.

#### Out of scope
Filing guards (BL-ACC-19).

---

### BL-ACC-19 — 04 Accounting: file a return only through a valid tax agent, flag expired KYC, ask the MLRO for a blocked client (`ACC.ACL.15`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-HRM-01 (`qualification_valid`) · BL-DOC-01 (document guard) · BL-DOC-02 (flag mode) · BL-KRN-01 (approval returned) · BL-ACC-13 (*late* flag on tasks completed from `overdue`) · **Blocks:** None.
**Review items:** 04-N8-d, X-13, 04-N8-a

#### Goal
Only a registered tax agent may file for a client. `file` records who filed: a staff member whose FTA
tax agent registration is valid on the filing date and linked to the firm's tax agency, or the client
itself through its own EmaraTax user. If a client KYC document has expired, filing still runs but the
return is flagged for the MLRO. If the client is blocked, filing needs the MLRO's approval until a
lawyer answers L-31.

#### Files to read
- `docs/verticals/04-accounting.md` §2 (*Tax agents*), §3.3 `ACC.ACL.15`, §4.4 "When it goes wrong" (rows *Filer is not a registered tax agent*, *A client KYC document has expired*), §5 `HRM.QUA.01`, §6 *Return* (filed by), §7.1 *Return* (row `file`), §9 *Tax agent status*, *Document blocks*, *Block effects and no tipping off*
- `docs/OPEN-ITEMS.md` L-16, L-31

#### Terms
- **Tax agent registration:** the staff member's FTA registration (number, expiry), linked to the firm's tax agency; a `HRM.QUA.01` qualification.
- **Filing date:** the date on the FTA acknowledgement.
- **Flag mode:** the guarded transition runs and the record is flagged for review with the invalid document.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `ACC.ACL.15` | A return is filed only by a staff member whose FTA tax agent registration is valid on the filing date and linked to the firm's tax agency (`qualification_valid`, `HRM.QUA.01`), or recorded as filed by the client through its own EmaraTax user ("filed by the client's own staff"; the acknowledgement shows the client's user, noted on the return) | `STA` | returns.file holders | — |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `file` | approved | filed | `returns.file` | FTA acknowledgement attached; filer's tax agent registration valid on the filing date, or filed by the client (`ACC.ACL.15`); client not blocked, or the MLRO approves (an approval request valid for this return only) | linked task `complete`; payment reminder if tax payable; if a client KYC document has expired, the return is flagged for the MLRO (flag mode) |

§9: "Whether the firm may still file a return for a client whose funds are frozen is `⚠ open: lawyer
(L-31)`; until answered `file` needs the MLRO's approval." Document blocks: client KYC kinds →
`return.file` (flag); FTA tax agent registration (staff) → `return.file` (block), override never.
"`BlockSubjects`: … a return → its client and its filer."

§2: "Whether an outside firm filing that way [through the client's own EmaraTax user] for a fee
counts as unregistered practice is `⚠ open: lawyer (L-16)`; the app assumes filing by a registered
agent or by the client's own staff."

#### Edge cases decided
- **Filer:** the user chooses *filed by tax agent* (a staff member, default the current user) or *filed by the client*. For *tax agent*, `qualification_valid(filer, FTA tax agent registration, filing date)` must pass and the registration must be linked to the firm's agency. For *client*, the filer recorded is the client's own EmaraTax user as shown on the acknowledgement (user name, required; the return is refused without it), as `ACC.ACL.15` states; the client contact who told the firm it was filed may be added as a note but is not the filer. The option label says "filed by the client's own staff" (L-16).
- **Filing date:** taken from the acknowledgement, entered by the user; not today if the acknowledgement is older.
- **No override** for the tax agent check.
- **KYC flag:** the return is `filed` and carries a flag "KYC document expired: <kind>, <expiry>" visible to the MLRO; it does not stop the payment reminder.
- **Blocked client:** `file` creates an approval request to the MLRO; staff without `parties.block_reason.view` see "On hold — MLRO approval needed". Rejected or returned: the return stays `approved` (BL-KRN-01). The approval is valid for this return only.
- **Late filing:** allowed; the linked task completes with *late* if it was `overdue` (BL-ACC-13).

#### Must not
- Must not file with a filer whose registration is expired, missing or not linked to the firm's agency.
- Must not offer an override for the tax agent check.
- Must not refuse filing because of an expired KYC document (flag only).
- Must not reveal the block reason to the filer.

#### Done when
- [ ] Given tax specialist T with a registration expiring 2026-11-30 and an acknowledgement dated 2026-11-28, `file` succeeds; with an acknowledgement dated 2026-12-01 it is refused naming the registration.
- [ ] Given *filed by the client* and the client's acknowledgement showing EmaraTax user "falcon.finance", `file` succeeds without a tax agent check and records "falcon.finance" as filer; without the EmaraTax user name it is refused.
- [ ] Given the client's trade licence expired, `file` succeeds and the return is flagged for the MLRO.
- [ ] Given the client blocked, `file` waits for the MLRO's approval; the filer sees "On hold — MLRO approval needed"; on approval the return is `filed` and the linked task `completed`.
- [ ] Given a payable return, the client gets the payment reminder after filing.

#### Out of scope
`qualification_valid` (BL-HRM-01). Document kind settings (BL-ACC-23).

---

### BL-ACC-20 — 04 Accounting: version a KPI pack on re-import and supersede the published one

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** Base: 04 KPI pack (`ACC.KPI.01`, import and calculation) · **Blocks:** BL-ACC-21
**Review items:** 04 MEDIUM "KPI pack lifecycle"

#### Goal
A client's trial balance is often re-exported after late entries. Importing it again for a period
that already has a published pack creates a new draft version; the published one shows "being
revised" until the new version is published, which supersedes the old one. The client always sees
one current pack per period.

#### Files to read
- `docs/verticals/04-accounting.md` §4.2 "When it goes wrong" (row *Trial balance re-imported after publication*), §6 *KPI pack* (version), §7.1 *KPI pack* (states, rows `publish`, `supersede`, paragraph after the diagram), §7.2 `ACC.RUL.06`

#### Terms
- **Version:** 1, 2, 3… per client and period.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Figures imported; KPIs calculated; commentary in progress |
| `published` | success | no | Visible in the portal |
| `superseded` | neutral | yes | Replaced by a newer version of the same period |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `publish` | draft | published | `kpi.publish` (manager) | every account mapped to a KPI category; commentary added when the template requires it | portal updated; client notified; a previous published version of the period `supersede`s |
| `supersede` | published | superseded | system (`publish` of a newer version) | — | the old version stays readable by staff, marked superseded |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `ACC.RUL.06` | trial balance imported for a period | — | recalculate the KPI pack; notify the manager to add commentary | — | on | MVP |

"A trial balance imported again for a period that has a published pack creates a new `draft` version
(`ACC.RUL.06`); the published one shows "being revised" until the new one is published. The client
is not notified at re-import, only at the new `publish`."

#### Edge cases decided
- **Re-import while a draft exists** (no published pack): the draft is recalculated in place (no new version).
- **Re-import while a published pack and a newer draft exist:** the newer draft is recalculated; no third version.
- **"Being revised":** shown to client users on the published pack in the portal; no notification to the client at re-import, only at the new `publish`.
- **Commentary:** copied from the published version into the new draft for the manager to update.
- **Portal after supersede:** client users see only the latest published version; staff see all.
- **Unmapped account in the new import:** `publish` is refused naming the accounts.

#### Must not
- Must not change a published pack's figures.
- Must not show two published versions of one period to the client.

#### Done when
- [ ] Given October's pack v1 `published`, importing a new trial balance for October creates v2 `draft` with v1's commentary, and the portal shows v1 with "being revised".
- [ ] When the manager publishes v2, v1 is `superseded`, the portal shows v2 only, and the client is notified.
- [ ] Importing again while v2 is `draft` recalculates v2; no v3 exists.
- [ ] `publish` with an unmapped account is refused.

#### Out of scope
Withdrawal (BL-ACC-21). Live ledger connectors (Later).

---

### BL-ACC-21 — 04 Accounting: withdraw a published KPI pack

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 04 · **Size:** s
**Depends on:** BL-ACC-20 · **Blocks:** None.
**Review items:** 04 MEDIUM "KPI pack lifecycle"

#### Goal
A pack published to the wrong client or with wrong figures is withdrawn from the portal at once.
The client users are told it was withdrawn; for a wrong-client release, a privacy incident task is
opened for the company admin, because another company's financial data was exposed.

#### Files to read
- `docs/verticals/04-accounting.md` §4.2 "When it goes wrong" (row *Pack published to the wrong client or with wrong figures*), §7.1 *KPI pack* (state `withdrawn`, row `withdraw`), §9 *PDPL*, *Professional confidentiality*

#### Terms
- **Privacy incident task:** a task for the company admin to assess the exposure (who opened it, when) and decide notifications.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `withdrawn` | error | yes | Removed from the portal (wrong client, wrong figures) |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `withdraw` | published | withdrawn | `kpi.publish` | reason given (*wrong client*, *wrong figures*) | removed from the portal; client users told it was withdrawn (for a wrong client, without naming the other company); for a wrong client, a privacy incident task for the company admin |

#### Edge cases decided
- **Reasons:** *wrong client*, *wrong figures*.
- **Who is told:** the users of the company that received the pack. For *wrong client*, the message says only that a report was withdrawn and must be disregarded, without naming the other company.
- **Incident task content:** the pack, the receiving company, the portal view log of that pack (users and times) from the audit trail.
- **After withdraw:** the period has no published pack; the manager publishes a corrected version as a new draft, either by re-importing the trial balance (BL-ACC-20) or by *copy*: a new `draft` version created from the withdrawn pack's imported figures, KPI mapping and commentary, for the right client and the same period (for *wrong client*, the client the figures belong to). The withdrawn pack is not changed by the copy.
- **Superseded versions** cannot be withdrawn (they are not in the portal).

#### Must not
- Must not delete the withdrawn pack; it stays readable by staff.
- Must not name the other company in the client message.

#### Done when
- [ ] Given October's pack for client A published to client B by mistake, `withdraw` with *wrong client* removes it from B's portal, tells B's users without naming A, and opens a privacy incident task for the company admin listing who in B opened it.
- [ ] Given *wrong figures*, no incident task is opened.
- [ ] `withdraw` without a reason is refused.
- [ ] After a *wrong client* withdraw, *copy* for client A creates a `draft` for A's October with the same figures and commentary; the withdrawn pack stays `withdrawn` and readable by staff.

#### Out of scope
Versions and supersede (BL-ACC-20).

---

### BL-ACC-22 — 04 Accounting: suspend an engagement for overdue invoices, never for disputed ones (`ACC.RUL.07`)

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Later · **Size:** s
**Depends on:** BL-ACC-03 · BL-INV-04 (invoice dispute flag) · **Blocks:** None.
**Review items:** X-02, 04 MEDIUM "suspension and retainer"

#### Goal
When the firm turns it on, an invoice overdue by 60 days suspends the engagement and tells the
partner. A disputed invoice never does: the dispute pauses the rule, as it pauses reminders.

#### Files to read
- `docs/verticals/04-accounting.md` §4.5 step 4 and "When it goes wrong" (row *Client disputes an invoice*), §7.2 `ACC.RUL.07`

#### Terms
- **Disputed invoice:** an invoice with the wl-invoicing `disputed` flag set.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `ACC.RUL.07` | an invoice overdue by N days (counted from the due date; days in dispute are not excluded) | engagement `active`; invoice not `disputed` | engagement `suspend`; notify partner | N = 60 | off | Later |

§4.5 row: "the invoice is `disputed`, reminders and `ACC.RUL.07` pause, and a ticket goes to the
engagement manager".

#### Edge cases decided
- **Days overdue:** days since the invoice due date (Asia/Dubai); the rule runs daily.
- **Disputed then resolved *upheld*:** the count continues from the original due date; the days in dispute are not excluded, so an invoice may suspend the engagement on the day after resolution.
- **Reason:** *non-payment* (not restricted), so `ACC.RUL.09` sends the deadline notice (BL-ACC-04).
- **Already suspended:** nothing.

#### Must not
- Must not suspend for a disputed invoice.
- Must not run while the rule is off (default).

#### Done when
- [ ] With the rule on, an invoice due 2026-09-01 and unpaid suspends the engagement on 2026-10-31 with *non-payment* and notifies the partner.
- [ ] The same invoice with `disputed` set does not suspend it.
- [ ] With the rule off, nothing happens.

#### Out of scope
Invoice disputes (BL-INV-04). Suspend effects (BL-ACC-03).

---

### BL-ACC-23 — 04 Accounting: configure shared negative-flow settings — compliance, documents, blocks and integrations

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-DOC-01 · BL-DOC-02 · BL-DOC-04 · BL-DOC-06 · BL-HRM-01 · BL-PTY-01 · BL-PTY-03 · BL-PTY-04 · BL-PTY-06 · BL-PTY-07 · BL-PTY-08 · BL-PTY-09 · BL-INV-09 · BL-INV-20 · BL-KRN-05 · BL-KRN-08 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None.
**Review items:** X-13, X-18, X-19, 04-N7-b, 04-N8-a, 04-N11-a

#### Goal
Seed the 04 deployment's values for document blocks, sanctions screening, party blocks and their
effects, compliance-critical rules and integration fallbacks. No new behaviour: each value
configures a module mechanism.
It also seeds the 04 values for lead sources and campaigns (`SAL.CRM.02`), onboarding and SOPs
(`HRM.ONB.01`) and registration kinds (`DOC.EXP.01`), added on 2026-10-16: `SAL.ACL.07`,
`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`.

#### Files to read
- `docs/verticals/04-accounting.md` §3.3 (`PTY.ACL.02`, `PTY.ACL.03`, `DOC.ACL.02`), §4.6, §5 `HRM.QUA.01`, §7.1 *Engagement* (paragraph *Block effect*), §7.1 checklist, §7.2 (compliance-critical paragraph), §8 (*Integration down*), §9 *Document blocks*, *Block effects and no tipping off*
- `docs/modules/wl-parties.md` §7; `docs/modules/wl-documents.md` §5.2, §5.6; `docs/modules/wl-kernel.md` §5.14
- `docs/modules/wl-sales.md` §4.6, §5.8 and §7 (campaigns, sources); `docs/modules/wl-hr.md` §5.8 and §7 (onboarding); `docs/modules/wl-documents.md` §3, §5.7 and §7 (SOPs, registration kinds)
- `docs/OPEN-ITEMS.md` L-30

#### Terms
- **Owner role:** the role that receives the owner alert (`PLT.RUL.04`) when a compliance-critical rule fails or misses its schedule.
- **Block effect:** what a party block does to each kind of agreement (pause, continue, review) and who decides to end it.

#### Spec excerpt (authoritative for this issue)

§9 *Document blocks*:

| Document kind (subject) | `blocks` (mode) | Override | Look-ahead | Responsible role |
|---|---|---|---|---|
| Client trade licence; UBO passport and Emirates ID (party) | `engagement.activate` (block); `return.file` (flag) | with approval by the MLRO (`DOC.ACL.02`) | 30 days | compliance officer (MLRO) |
| FTA tax agent registration (staff, `HRM.QUA.01`) | `return.file` (block) | never (legal block) [S5], [S16] | 30 days | partner |

"`BlockSubjects`: an engagement → the client company and its UBOs; a return → its client and its
filer. `FutureWork`: tasks and returns due after the expiry date. … The responsible role
(wl-documents §5.2) gets the kind's expiry reminders (`DOC.RUL.01`) and renewal task (`DOC.RUL.03`)
in the subject's scope."

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `DOC.ACL.02` *(shared)* | An expired client KYC document blocks engagement activation; a one-use override needs a reason and the MLRO's approval, valid 72 hours | `SOD`, `TIM` | manager, MLRO | 72 hours |
| `PTY.ACL.02` *(shared)* | Restricted block reasons (sanctions, AML concern) and block records are visible only to the MLRO and deputy (`parties.block_reason.view`); everyone else sees "On hold"; the portal sees no block | `SEN`, `FLD` | all staff, client users | — |
| `PTY.ACL.03` *(shared)* | While a client is blocked, no engagement or retainer is started, renewed or extended (no exception), and no money is paid out to it (refund of a prepaid retainer, credit balance) unless the MLRO approves an exception for that payment | `STA`, `LIM` | partner, billing | — |

§7.1 *Block effect*: "*pause*. A blocked client's engagement is suspended with the neutral reason;
ending it is the MLRO's decision, carried out by the partner through `end`. The retainer pauses;
money out stops; money in continues and, for a restricted reason, is neither refunded nor paid on
without the MLRO."

§7.2: "**Compliance-critical** (owner alert `PLT.RUL.04` to the partner role, which is this
vertical's owner role, and the company admin): `ACC.RUL.04`, `ACC.RUL.05`, `ACC.RUL.09`,
`ACC.RUL.12`, `PTY.RUL.01`, `PTY.RUL.02`, and `DOC.RUL.03` for the blocking kinds in §9."

§8 *Integration down*: "Connections registered with a manual fallback: sanctions list download (the
MLRO screens new clients by hand against the official lists and records the result; onboarding
stays blocked until the lists are current), e-mail (notices stay visible in the portal; statutory
notices are delivered by other means, `NTF.RUL.01`), e-invoicing ASP (transmissions queued and sent
on recovery). EmaraTax and goAML are manual in the MVP, so their outages only delay the filing
task."

`PTY.RUL.02`: N = 24 hours; "while stale, engagement `activate` and CDD `submit` and `approve` are
refused". §4.6 step 4: related-person block "on"; freeze and report tasks "due 24 hours after the
flag".

**Settings (setting → value → spec reference)**

| Setting | 04 value | Spec reference |
|---|---|---|
| KYC kinds (trade licence; UBO passport; UBO Emirates ID): `blocks` | `engagement.activate` block (override with MLRO approval); `return.file` flag; look-ahead 30 days | §9 |
| FTA tax agent registration: `blocks` | `return.file` block; override never; look-ahead 30 days | §9 |
| Responsible role per kind | KYC kinds: compliance officer (MLRO); FTA tax agent registration: partner | §9 |
| `BlockSubjects` | engagement → client company and its UBOs; return → its client and its filer | §9 |
| `FutureWork` | tasks and returns due after the expiry date | §9 |
| `HRM.QUA.01` qualification kind | FTA tax agent registration, linked to the firm's tax agency | §5 |
| Override approval (`DOC.ACL.02`) | MLRO; one use; valid 72 hours; reason required | §3.3 |
| Screening | on; lists UN Consolidated List (XML) and UAE Local Terrorist List (Excel; PDF checked by hand) | §2, §8 |
| List freshness limit (`PTY.RUL.02`) | 24 hours; retry hourly | §7.2 |
| Stale list at onboarding | block (`engagement.activate`, CDD `submit`, CDD `approve`) | §7.2 |
| Manual list upload | allowed, MLRO | §8, wl-parties |
| Confirmed match on a UBO, director or signatory blocks the company | on | §4.6 |
| Second reviewer for a confirmed match | on (module default) | wl-parties §7 |
| Freeze and report tasks due | 24 hours after the possible match was flagged (L-30) | §4.6 |
| `parties.block_reason.view` holders | MLRO and deputy | §3.3 |
| Block effect: engagement | pause (`suspend`, reason *on hold*); end decided by the MLRO, carried out by the partner | §7.1 |
| Block effect: retainer | pause with the engagement | §7.1 |
| Money out to a blocked client (`PTY.ACL.03`) | stopped; exception approved by the MLRO | §3.3 |
| Money in for a restricted block | recorded; not refunded or paid on without the MLRO | §7.1 |
| Compliance-critical rules | `ACC.RUL.04`, `ACC.RUL.05`, `ACC.RUL.09`, `ACC.RUL.12`, `PTY.RUL.01`, `PTY.RUL.02`, `DOC.RUL.03` (blocking kinds) | §7.2 |
| Owner role (`PLT.RUL.04`) | partner role, and the company admin | §7.2 |
| Integration connections with manual fallback | sanctions list download; e-mail; e-invoicing ASP | §8 |
| Sanctions download fallback | MLRO screens new clients by hand and records it; onboarding stays blocked until lists are current | §8 |
| E-mail fallback | notices stay visible in the portal; statutory notices delivered by other means | §8 |
| E-invoicing ASP fallback | transmissions queued and sent on recovery; `FIN.RUL.16` task | §8 |
| EmaraTax, goAML | not registered as connections (manual) | §8 |
| Lead sources used (`SAL.CRM.02`) | referral, website, social, paid ads, partner (banks, business setup consultants), event, other; recorded on the lead (`SAL.CRM.01`) and carried to the client party when the engagement letter is signed; no source requires a campaign | §5; §7.2 settings |
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | search ads, LinkedIn, email newsletters, seminars and webinars; converted value: the first engagement's fees; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.2 settings |
| Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for accountant / associate, tax specialist and billing / admin, started by `HRM.RUL.16` when an employee record (`HRM.EMP.01`) is created; checklist steps use `WRK.CHK.01`; tasks to the manager and the employee; the employee's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds overdue steps every 2 days and tells billing / admin (HR) and the manager after 5 days; completion blocks nothing (role activation not blocked) | §5; §7.2 settings |
| SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: MLRO (the firm's AML and CDD procedures, read by every client-facing role) and manager (engagement, return and month-end close procedures); `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the manager and the owner role after 7 days; acknowledged only by the reader (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.2 settings |
| Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Trademark registration* and *Other certification* (subject: the firm's legal entity); responsible role partner; `DOC.RUL.01` at 90, 30 and 7 days; no `blocks` | §5; §7.2 settings |

#### Edge cases decided
- **PTY.RUL.02 compliance-critical:** listed by the 04 spec (the wl-parties epic chose it as a default too, BL-PTY-07).
- **Freeze and report due times:** stay a parameter until L-30 is answered (`blocked:open-item`).
- **Owner alert to two roles:** both the partner role and the company admin receive it, once each.
- **Seed, not code:** values come from the 04 deployment template. The override mode of the FTA tax agent registration stays `never` and is not editable: it is a legal block confirmed by the sources in 04 §9 ([S5], [S16]: only a registered tax agent may act for a client), so no open item governs it (L-19 does not list 04). Only L-30 (freeze timing) keeps this issue `blocked:open-item`.

#### Must not
- Must not hard-code these values in module code.
- Must not give `parties.block_reason.view` to any role but the MLRO and deputy by default.

#### Done when
- [ ] Documents: activating with an expired UBO passport is refused with an MLRO override offered; filing with an expired tax agent registration is refused with no override; filing with an expired trade licence succeeds with a flag; the expiry reminder of a client trade licence goes to the MLRO and that of a tax agent registration to the partners.
- [ ] Screening: with the UN list 25 hours old, CDD `submit` is refused; the MLRO can upload the list by hand.
- [ ] Blocks: a confirmed match on a UBO blocks the client company; a partner sees "On hold"; a refund of a prepaid retainer waits for the MLRO's exception.
- [ ] Rules: given a list download failure that triggers `PTY.RUL.02`, and the rule run itself made to end `failed` in the test (its action raises an error, so no MLRO alert or stale marking is done), the partner role and the company admin each get one owner alert (`PLT.RUL.04`, BL-KRN-08); the same for a run that is `stop`ped. A failed list download with a successful rule run gives only the MLRO alert, no owner alert.
- [ ] Integrations: with the e-mail connection `down`, notices stay visible in the portal and statutory notices create staff delivery tasks.
- [ ] Lead sources: a new lead or party offers the sources referral, website, social, paid ads, partner, event and other; a campaign with channel *search ads*, a budget and cost entries reports leads, conversion, cost per lead, CAC and ROAS; a cost entry over the budget waits for a manager (`SAL.ACL.07`).
- [ ] Onboarding and SOPs: hiring a tax specialist creates the tax specialist checklist with tasks to the manager, the line manager and the employee, and the role is active before it is complete; a step open 2 days past its due date reminds its assignee (`HRM.RUL.17`); a new version of an SOP owned by the MLRO asks every required reader to acknowledge again within 7 days (`DOC.RUL.04`), and no one else can acknowledge for them (`HRM.ACL.10`).
- [ ] Registrations: a fresh 04 deployment lists *Trademark registration* and *Other certification* with responsible role partner; a trademark registration expiring in 90 days notifies the partner.

#### Out of scope
The mechanisms (module epics listed in Depends on). Notices, receivables, tasks and portal settings
(BL-ACC-24).

---

### BL-ACC-24 — 04 Accounting: configure shared negative-flow settings — notices, receivables, tasks and portal

**Labels:** `vertical:04-accounting` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 04 · **Size:** m
**Depends on:** BL-NTF-01 · BL-NTF-02 · BL-NTF-03 · BL-NTF-04 · BL-NTF-05 · BL-NTF-06 · BL-INV-01 · BL-INV-02 · BL-INV-04 · BL-INV-05 · BL-INV-19 · BL-INV-21 · BL-PAY-09 · BL-PAY-14 · BL-TKT-09 · BL-TKT-10 · BL-KRN-04 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · **Blocks:** None.
**Review items:** X-01, X-02, X-05, X-20, 04 MEDIUM "e-mail bounces"

#### Goal
Seed the 04 deployment's values for give-up reactions of client messages, receivables (write-offs,
disputes, bounced cheques, retainers), deadline tasks, the portal after an engagement ends, and late
time entries.
It also seeds the 04 values for the cash-flow forecast alert (`FIN.CFL.01`) and the
company budget with owner drawings (`FIN.BUD.01`), added on 2026-10-16: `FIN.RUL.23`,
`FIN.RUL.24` and `FIN.ACL.21`.

#### Files to read
- `docs/verticals/04-accounting.md` §3.3 (`FIN.ACL.15`, `POR.ACL.01`), §4.2 "When it goes wrong" (row *Request e-mail bounces*), §4.5 "When it goes wrong", §7.1 *Task* (paragraph about `waiting_on_client`), §7.1 *Subscription* (paragraph after the diagram), §7.1 checklist (row *Time entry*), §7.2 (reused rules), §9 *Notices*, *Bad debts*
- `docs/modules/wl-notify.md` §5.4; `docs/modules/wl-invoicing.md` §5.9, §5.10; `docs/modules/wl-ticketing.md` §4.4
- `docs/modules/wl-invoicing.md` §5.14–§5.16 and §7 (cash-flow forecast, company budget, owner drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments)

#### Terms
- **Give-up reaction:** what wl-notify does when a message could not be delivered after all retries.
- **Statutory notice type:** a notification type whose undelivered message needs delivery by other means with proof before dependent deadlines start.

#### Spec excerpt (authoritative for this issue)

§9 *Notices*: "deadline notices, document requests, suspension and end notices (`ACC.RUL.09`) are
statutory notification types: on give-up the contact point is marked invalid and a staff task asks
for delivery by another means with proof; dependent deadlines do not start until then. Return
approval requests go to another client approver, then a staff task. Invoices and statements go to
the other finance contact, then the billing role. Each type 04 registers names its wl-notify group
(§5.4 there): deadline notices, document requests and their reminders, suspension and end notices →
statutory; return approval requests → *portal and signatures*, with the reaction changed to another
client approver, then a staff task; engagement letters, KPI pack and query notices → *portal and
signatures* (staff task to the engagement manager); invoices, statements, retainer and write-off
notices → *invoices* (collector role: billing). MLRO alerts on lists and matches are staff-only,
with urgency `urgent` and no group."

§4.2 row *Request e-mail bounces*: "The notice is statutory: the contact point is marked invalid and a
staff task (1 working day) asks for delivery by another means with proof; until then the request
shows "notice not delivered" and its reminder deadlines do not start (`NTF.RUL.01`)."

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `FIN.ACL.15` *(shared)* | Writing off an invoice balance as a bad debt needs a partner's approval; the requester cannot approve | `LIM`, `SOD` | billing, manager | AED 0 (every write-off) |
| `POR.ACL.01` *(shared, formerly `ACC.ACL.12`)* | After an engagement ends, client users keep read-only access to their filed returns and documents, then the portal closes | `TIM` | client users | closes after 90 days |

§4.5 rows: "Client disputes an invoice — Client users with the approver role may raise it in the
portal; the invoice is `disputed`, reminders and `ACC.RUL.07` pause, and a ticket goes to the
engagement manager; resolved as *upheld* or *credited*." "Cheque bounces — wl-payments cheque path
(re-present, settle, legal case or write-off); the client goes on credit hold until resolved: in 04
the hold is only a *credit hold* badge shown to billing staff and partners (there are no sales orders
to hold) and blocks nothing; no bounce fee by default." "Invoice uncollectable — `write_off` with a partner's approval
(`FIN.ACL.15`); the write-off notice goes to the client; VAT bad-debt relief is proposed by
`FIN.RUL.14` when its conditions are met."

§7.1 *Subscription*: "A cycle invoice that is `disputed` does not make the retainer `past_due`."
§7.1 *Task*: "`waiting_on_client` is the module's `waiting` state with this vertical's label." Task
paragraph: a started task that no longer applies "is listed for the manager". Checklist *Time
entry*: "into a locked timesheet period only with approval (`PLT.ACL.10`)".

**Settings (setting → value → spec reference)**

| Setting | 04 value | Spec reference |
|---|---|---|
| Statutory notification types | deadline notices; document requests (and their reminders); suspension and end notices (`ACC.RUL.09`) → reaction `mark_contact_invalid` + `statutory_notice`, task due 1 working day | §9, §4.2 |
| Document request while its notice is undelivered | shows "notice not delivered"; `POR.RUL.01` reminder deadlines do not start | §4.2 |
| Return approval requests | give-up → another client approver of the company; then a staff task | §9 |
| Invoices and statements | give-up → the other finance contact; then a task for the billing role | §9 |
| Notification type groups | statutory: deadline notices, document requests, suspension and end notices; *portal and signatures*: return approval requests (reaction: another approver, then task), engagement letters, KPI pack and query notices (task to the engagement manager); *invoices*: invoices, statements, retainer and write-off notices (collector role billing); MLRO alerts: urgency `urgent`, no group | §9 |
| Invoice write-off (`FIN.ACL.15`) | limit AED 0; approver partner; requester cannot approve | §3.3 |
| Write-off notice to the client | sent | §4.5, §9 |
| Bad-debt relief proposal (`FIN.RUL.14`) | on | §4.5 |
| Invoice dispute from the portal | allowed for client users with the *approver* role; ticket to the engagement manager; outcomes *upheld* or *credited*; escalation `FIN.RUL.15` | §4.5 |
| Disputed cycle invoice and the retainer | does not make it `past_due` | §7.1 |
| Retainer with the engagement | paused on `suspend`, resumed on `resume`, cancelled on `end` with pro-rata final invoice or credit | §7.1 |
| Bounced cheque | wl-payments path; `FIN.RUL.18` follow-up on; credit hold until resolved, shown only as a badge to billing staff and partners, blocking nothing; bounce fee off | §4.5, §7.2 |
| Task label | `waiting` shown as *waiting on client* | §7.1 |
| Waive a started task | off | §7.1 |
| Tasks held while the engagement is suspended | on | §7.1 |
| Portal after engagement end (`POR.ACL.01`) | read-only, closes after 90 days | §3.3 |
| Late time entries (`PLT.ACL.10`) | into a locked timesheet period only with a manager's approval | §7.1 checklist |
| Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | the firm's own cash: projected balance below AED 25,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the partner and billing / admin, checked daily at 06:00; retainers by cycle date; due dates adjusted by each client's average days late: on; disputed amounts counted: off | §5; §7.2 settings |
| Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories owned by billing / admin and mapped to the export accounts; one budget per legal entity and month, approved by a partner; `FIN.RUL.24` warns the category's owner role and billing / admin when a month's actual reaches 100% of its line | §5; §7.2 settings |
| Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings* (partner drawings), kept out of operating cost and profit; every drawing (AED 0) approved by a partner other than the requester (`FIN.ACL.21`) | §5; §7.2 settings |

#### Edge cases decided
- **Credit hold in 04:** there are no sales orders, so the client shows a *credit hold* badge to billing staff and partners until the cheque is resolved; nothing else is blocked (not activation, filing or time entries).
- **Restricted reasons:** statutory notices that `ACC.ACL.14` suppresses are never sent, so their give-up reaction never runs.
- **Group contacts:** the "other finance contact" is another finance contact of the same company, not of another company in the group.
- **Seed, not code:** values come from the 04 deployment template and are editable by the company admin within the module ranges.

#### Must not
- Must not let the requester approve a write-off.
- Must not start reminder deadlines of a request whose notice was not delivered.
- Must not hard-code these values in module code.

#### Done when
- [ ] Notices: a document request e-mail that gives up marks the contact invalid, creates a staff task due the next working day, and the request shows "notice not delivered" with no reminder sent until proof is recorded.
- [ ] Notices: a return approval request that gives up goes to the second client approver, then creates a staff task; a KPI pack notice that gives up marks the contact invalid and creates a task for the engagement manager.
- [ ] Receivables: every write-off needs a partner other than the requester; the client gets the write-off notice; a client approver can dispute an invoice in the portal and the engagement manager gets the ticket; a disputed cycle invoice leaves the retainer `active`; a bounced cheque adds no fee and shows the credit hold badge.
- [ ] Tasks: an `in_progress` task is never waived automatically; the waiting state shows *waiting on client*.
- [ ] Portal: an engagement ended 2026-11-01 gives read-only access until 2027-01-30 and none from 2027-01-31.
- [ ] Cash-flow forecast: in a fresh 04 deployment, a projected balance of AED 24,999 in week 5 alerts the partner and the billing / admin once (`FIN.RUL.23`, 06:00 run); AED 25,000 or more in every week alerts no one; the forecast shows 13 weekly buckets from Monday.
- [ ] Budget and drawings: a monthly budget for a legal entity prepared by the billing / admin needs approval by a partner other than the requester; an expense with category *owner drawings* is left out of operating cost and profit reports, and the user who records it cannot approve it (`FIN.ACL.21`); an expense line reaching 100% of its month's budget line warns the category's owner role and billing / admin once (`FIN.RUL.24`).

#### Out of scope
The mechanisms (module epics listed in Depends on). Compliance, documents, blocks and integrations
(BL-ACC-23).
