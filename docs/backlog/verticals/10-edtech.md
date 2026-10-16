# Epic: 10 EdTech — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows that belong to the EdTech vertical itself: seat holds that
lapse and seats taken atomically, withdrawal with its money documents, enrolments of a blocked
payer, the link between membership pause and enrolment freeze, course fee chargebacks, document
blocks on batches, sessions and certificates, tutor eligibility and tutor change, batch
termination and cancellation, attendance correction, the absence policy, certificate reissue,
and 10's values for the shared negative-flow mechanisms. It closes 10-N3-f, 10-N8-a, 10-N8-c,
10-N12-b, the 10 parts of X-06, X-07, X-13, X-17 and X-18, and the 10 MEDIUM items (payment
timeout, freeze with membership pause, absence consumption, attendance correction, certificate
reissue, capacity under concurrency, tutor reassignment). Subscription, payment, refund,
booking and document mechanics live in the module epics.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-EDU-01 | Lapse an unpaid seat hold (`lapse`, `EDU.RUL.11`) | high | m | Launch 10 | Base: 10 Enrolment lifecycle; BL-EDU-02; BL-PAY-02; BL-PAY-03 |
| BL-EDU-02 | Take batch seats atomically (`EDU.ACL.17`) | medium | m | Launch 10 | Base: 10 Batch and Enrolment lifecycle |
| BL-EDU-03 | Withdraw an enrolment with credit note, cancelled charges or amount owed (`withdraw`) | high | m | Launch 10 | Base: 10 Enrolment lifecycle (`EDU.REF.01`); BL-INV-21; BL-INV-08; BL-PAY-06; BL-PAY-07; Base: wl-sales Package `refund` |
| BL-EDU-04 | Freeze the enrolments of a blocked party and hold their withdrawal refunds | high | s | Launch 10 | BL-PTY-01; BL-PTY-03; BL-PTY-04; BL-EDU-03 |
| BL-EDU-05 | Keep membership pause and enrolment freeze in step (`EDU.RUL.14`) | medium | s | Launch 10 | BL-INV-20; Base: 10 Enrolment lifecycle (`freeze`, `unfreeze`) |
| BL-EDU-06 | Give the counsellor a task when a course fee is charged back | high | s | Launch 10 | BL-PAY-04; BL-INV-07; BL-INV-19 |
| BL-EDU-07 | Block batch `open` and certificate `issue` on an expired centre permit or course approval (`EDU.RUL.13`) | high | m | Launch 10 | BL-DOC-01; BL-DOC-04; BL-DOC-05; BL-KRN-08 |
| BL-EDU-08 | Guard tutor assignment on every session date (`EDU.ACL.14`) | high | m | Launch 10 | BL-HRM-01; BL-DOC-01; BL-DOC-04; BL-DOC-06 |
| BL-EDU-09 | Change the tutor of a batch or a session (`change_tutor`) | medium | m | Launch 10 | BL-EDU-08 |
| BL-EDU-10 | Flag sessions of a tutor who will not be eligible or available (`EDU.RUL.12`) | high | m | Launch 10 | BL-EDU-08; BL-EDU-09; BL-HRM-01; BL-SCH-05; BL-KRN-08 |
| BL-EDU-11 | Flag a session taught by an ineligible tutor (`submit_attendance`) | high | s | Launch 10 | BL-EDU-08; BL-DOC-02 |
| BL-EDU-12 | Terminate a running batch (`terminate`, `EDU.ACL.15`) | high | m | Launch 10 | BL-EDU-02; BL-EDU-03; BL-KRN-01 |
| BL-EDU-13 | Cancel a batch before it runs with a full refund (`cancel`) | high | s | Launch 10 | BL-EDU-03; BL-EDU-02 |
| BL-EDU-14 | Correct attendance of a held session (`correct_attendance`, `EDU.ACL.16`) | medium | m | Launch 10 | BL-EDU-15; BL-ATT-07; BL-ATT-08; Base: wl-sales Package `reverse` |
| BL-EDU-15 | Consume package credits by the absence policy | medium | m | Launch 10 | Base: wl-sales Package `consume`; Base: 10 Session lifecycle |
| BL-EDU-16 | Reissue a certificate (`reissue`) and print a copy | medium | m | Launch 10 | Base: 10 Certificate lifecycle; BL-EDU-07 |
| BL-EDU-17 | 10: configure shared negative-flow settings (documents, scheduling, notifications, alerts, integrations) | high | m | Launch 10 | BL-DOC-01; BL-DOC-02; BL-DOC-06; BL-SCH-02; BL-SCH-05; BL-SCH-07; BL-PTY-03; BL-NTF-06; BL-KRN-05; BL-KRN-08; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-SAL-22; BL-SAL-23; BL-SAL-24; BL-SAL-25; BL-SAL-26; BL-NTF-07; BL-NTF-08; BL-NTF-09; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |
| BL-EDU-18 | 10: configure shared negative-flow settings (fees, refunds, payments, cash) | high | m | Launch 10 | BL-INV-01; BL-INV-04; BL-INV-10; BL-INV-20; BL-PAY-02; BL-PAY-04; BL-PAY-07; BL-PAY-17; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26 |

---

### BL-EDU-01 — 10 EdTech: lapse an unpaid seat hold (`lapse`, `EDU.RUL.11`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** Base: 10 Enrolment lifecycle (`activate`) · BL-EDU-02 (seat count) · BL-PAY-02 (`FIN.RUL.19` pending timeout, `PaymentFailed`) · BL-PAY-03 (`late_confirm` refund) · **Blocks:** None
**Review items:** X-06, 10 MEDIUM "enrolment payment timeout"

#### Goal
An enrolment waiting for its first payment holds a seat. If no payment arrives within 48 hours
the enrolment lapses: the seat is released for others, the payment link is cancelled and the
student or guardian and the counsellor are told. A failed online payment does not end the hold
early. A payment that arrives after the lapse is refunded automatically, also when the counsellor
has already re-enrolled the student.

#### Files to read
- `docs/verticals/10-edtech.md` §4.1 "When it goes wrong" rows 1–2, §6 (Enrolment: seat hold
  until), §7.1 *Enrolment*, §7.2 (`EDU.RUL.11`)
- `docs/modules/wl-payments.md` §4 *Payment* (`fail`, `late_confirm`), §4 *Payment link*,
  `FIN.RUL.19`

#### Terms
- **Seat hold:** the seat counted for an enrolment in `pending_payment` until *seat hold until*.
- **Payment `pending` at the gateway:** an online payment started but not yet confirmed or
  failed; settled by `FIN.RUL.19` (timeout, default 30 minutes).
- **`late_confirm`:** wl-payments transition for a gateway confirmation that arrives after the
  payment was failed. If the payment's targets are still open and its prepaid subject is still
  active, the money is allocated as usual; otherwise it becomes the party's credit and a refund
  is requested automatically. A `lapsed` enrolment is never an active subject, so a late
  payment for it is always refunded.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `pending_payment` | warning | no | Seat held; first payment or approval outstanding |
| `lapsed` | neutral | yes | Seat hold expired without payment |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `lapse` | pending_payment | lapsed | system (`EDU.RUL.11`) | no payment N hours after the hold | seat released; open payment link cancelled; guardian told; a later payment is refunded automatically |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `EDU.RUL.11` | N hours after an enrolment is created; a gateway payment of a held enrolment failing (`PaymentFailed`) after that time | still `pending_payment`; nothing paid; no payment `pending` at the gateway | `lapse` the enrolment; release the seat; cancel the payment link; tell the student or guardian and the counsellor | N = 48 | on | MVP |

§4.1: "An online payment still `pending` is settled by `FIN.RUL.19`, and the enrolment lapses at
once if it fails. A payment that arrives after the lapse is refunded automatically (wl-payments
`late_confirm`: the lapsed enrolment is no longer an active subject), also when the counsellor
has meanwhile re-enrolled the student; the new enrolment is paid through its own payment
link." · "The online payment fails: The enrolment stays
`pending_payment`; the guardian gets a new payment link; the seat hold keeps running."

#### Edge cases decided
- **Seat hold until** = enrolment created at + 48 hours, stored on the enrolment; Asia/Dubai for
  display.
- **Payment pending at the 48-hour mark:** no lapse yet; when the payment fails or times out
  (`PaymentFailed`), the rule lapses the enrolment at once.
- **Failed payment:** a new payment link is sent to the payer; *seat hold until* is not
  extended.
- **Late payment and re-enrolment:** the late payment is refunded automatically by wl-payments
  (`late_confirm`, BL-PAY-03) in every case. A re-enrolment E2 is a separate enrolment that
  took its own seat under `EDU.ACL.17` when it was created (no second seat check later) and is
  paid through its own payment link; the counsellor sees the refund on the party. The late
  payment is never moved to E2 (wl-payments allocates a late payment only to its own open
  targets).
- **Re-enrolling a lapsed student:** a new enrolment (a `lapsed` one never reopens), subject to
  `EDU.ACL.17`.
- **Partial first payment:** not "no payment"; the enrolment does not lapse; the counsellor sees
  the balance.
- **Discount approval pending:** counts as "approval outstanding"; the hold still lapses at 48
  hours.

#### Must not
- Must not lapse while a gateway payment is `pending`.
- Must not keep the seat after `lapse`.
- Must not reopen a `lapsed` enrolment.

#### Done when
- [ ] Given enrolment E created 2026-11-01 10:00 in `pending_payment` with no payment, at
      2026-11-03 10:00 E is `lapsed`, the batch has one more free seat, the payment link is
      cancelled, and the guardian and counsellor are told.
- [ ] Given a gateway payment `pending` at 10:00 on 2026-11-03, E stays `pending_payment`; when it
      times out at 10:30, E lapses.
- [ ] Given a failed card payment on 2026-11-02, E stays `pending_payment`, a new link is sent,
      and E still lapses on 2026-11-03 10:00.
- [ ] Given E `lapsed` and AED 1,050 confirmed late on 2026-11-03 12:00, a refund of AED 1,050
      is requested and approved automatically.
- [ ] Given the counsellor re-enrolled the student (E2 `pending_payment`, seat taken at
      creation) before that payment, the AED 1,050 is still refunded automatically, E2 stays
      `pending_payment` with its own payment link, and the counsellor is told of the refund.
- [ ] Given batch B full when the counsellor tries to re-enrol the student, no E2 is created
      (`EDU.ACL.17`).

#### Out of scope
Seat counting (BL-EDU-02); pending timeout and `late_confirm` (BL-PAY-02, BL-PAY-03).

---

### BL-EDU-02 — 10 EdTech: take batch seats atomically (`EDU.ACL.17`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:medium` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** Base: 10 Batch and Enrolment lifecycle · **Blocks:** BL-EDU-01, BL-EDU-12, BL-EDU-13
**Review items:** X-17, 10 MEDIUM "capacity under concurrency"

#### Goal
Two counsellors, or a counsellor and a parent in the portal, must never sell the last seat
twice. A seat is taken in one atomic step that counts active, frozen and held enrolments
against the batch capacity; the second attempt is refused and offered another batch or the
waiting list. Only the centre manager may go over capacity, and never beyond the room's limit.

#### Files to read
- `docs/verticals/10-edtech.md` §3.3 (`EDU.ACL.17`), §4.1 row "Two people take the last seat",
  §6 (Batch: waiting list), §7.1 *Enrolment* (`transfer`), *Batch*

#### Terms
- **Seats taken:** enrolments of the batch in `active`, `frozen` or `pending_payment`.
- **Room limit:** the maximum persons of the batch's room (resource capacity).

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `EDU.ACL.17` | Seats are taken atomically: an enrolment or seat hold is refused when active, frozen and held enrolments reach the batch capacity; only the centre manager may exceed capacity, up to the room's limit (for a room without a limit, the capacity is the hard limit) | `STA`, `LIM` | counsellor, portal | — |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `transfer` | active, frozen | transferred | `enrolments.manage`, or system (batch `terminate`) | target batch has a seat (`EDU.ACL.17`) | new `active` enrolment with remaining credits or balance |

§4.1: "Seats are taken atomically (`EDU.ACL.17`); the second attempt is refused and offered
another batch or the waiting list."

#### Edge cases decided
- **Atomicity:** the count and the insert run in one transaction with a lock on the batch row;
  the second request waits and then sees the full count.
- **Which actions take a seat:** creating an enrolment (`pending_payment`), `transfer` into the
  batch, and re-enrolment. `unfreeze` does not (a frozen enrolment keeps its seat).
- **Over capacity:** the centre manager may confirm an enrolment above capacity up to the room
  limit; the room limit is never exceeded. A room without a limit: capacity is the hard limit.
- **Waiting list:** a waiting entry on the batch (student, payer, created at) visible to
  counsellors; no automatic offer when a seat frees.
- **Alternatives offered:** other `open` batches of the same course and level with free seats.
- **Packages and memberships without a batch:** not affected.

#### Must not
- Must not count `lapsed`, `withdrawn`, `transferred` or `completed` enrolments.
- Must not let anyone exceed the room limit.

#### Done when
- [ ] Given batch B with capacity 12 and 11 seats taken, when counsellor A and guardian G try to
      enrol at the same moment, exactly one succeeds and the other is refused with alternatives
      and the waiting list option.
- [ ] Given 12 taken and room limit 14, the centre manager can enrol a 13th and 14th, not a 15th.
- [ ] A counsellor cannot exceed 12.
- [ ] `transfer` into a full batch is refused.
- [ ] After a `lapse`, a new enrolment into B succeeds.

#### Out of scope
Lapse (BL-EDU-01); batch termination transfers (BL-EDU-12).

---

### BL-EDU-03 — 10 EdTech: withdraw an enrolment with credit note, cancelled charges or amount owed (`withdraw`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** Base: 10 Enrolment lifecycle and refund calculation (`EDU.REF.01`) · BL-INV-21 (subscription cancel on withdrawal) · BL-INV-08 (credit note reopened on failed refund) · BL-PAY-06, BL-PAY-07 (failed refund) · Base: wl-sales Package `refund` · **Blocks:** BL-EDU-04, BL-EDU-12, BL-EDU-13
**Review items:** 10-N3-f

#### Goal
When a student leaves early, the money must be settled with proper tax documents: a credit note
for the paid amount not used (per the refund policy accepted at enrolment), cancellation of
future instalments, an immediate cancellation of a membership, a refund of a package, and, if the
student used more than was paid, an invoice for the amount owed. The refund is approved by a
different user. If the refund later fails, the enrolment stays withdrawn.

#### Files to read
- `docs/verticals/10-edtech.md` §4.3 "When it goes wrong" rows 1–2, §6 (Enrolment: withdrawal
  refund or amount owed), §7.1 *Enrolment* (`withdraw`, paragraph after the diagram), §9 (refund
  policy), §13 question 5
- `docs/modules/wl-invoicing.md` §4 *Credit note*, *Subscription* (`cancel`), charge schedules
- `docs/modules/wl-sales.md` §4 *Package* (`refund`)

#### Terms
- **Refund policy:** the policy accepted at enrolment and stored with it. Default template:
  before the start, 100% minus the registration fee; after the start, unused sessions pro rata
  minus an admin fee; after 50% of the course, no refund. Packages: unused sessions at the
  single-session price, without the package discount.
- **Scheduled instalments:** charges of the enrolment's instalment schedule not yet invoiced.
- **Amount owed:** value of used sessions plus fees minus what was paid, when positive.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `withdrawn` | neutral | yes | Left early; refund or amount owed settled |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `withdraw` | pending_payment, active, frozen | withdrawn | `enrolments.manage` (the centre manager while the party is blocked), or system (batch `terminate` or `cancel` when no transfer is chosen) | refund calculated per policy (`EDU.REF.01`), or per batch `terminate` or `cancel`; approval per `FIN.ACL.04` (the state changes only on approval); refund paid only when the party is not blocked, or with a compliance exception (`PTY.ACL.03`) | credit note for the refund the policy gives (the admin fee and a kept registration fee stay revenue; session value = price ÷ sessions; *used* = sessions `held` up to the withdrawal date, attended or not); issued but unpaid instalment invoices credited in full and future instalments cancelled; membership `cancel`led at once (its own pro rata credit, not repeated); package `refund`ed (its own valuation); refund paid (`wl-payments`); if used sessions and fees exceed what was paid, one invoice for the amount owed; from `pending_payment`: no credit note, payment link cancelled; seat released |

§4.3: "A student withdraws: `withdraw` calculates the refund per the accepted policy. A credit
note covers the paid amount not used; scheduled instalments are cancelled; a membership is
`cancel`led; a package is `refund`ed. If the sessions used plus fees exceed what was paid, an
invoice for the amount owed is issued instead. The refund is approved by another user
(`FIN.ACL.04`)." · "The refund fails: `FIN.RUL.21` retries once, then the accountant pays by
another method; the credit note balance is restored meanwhile."

§7.1: "A refund that fails (`RefundFailed`) leaves the enrolment `withdrawn` and restores the
credit note balance."

`FIN.ACL.04`: the user who records a refund cannot approve it (`SOD`, on).

#### Edge cases decided
- **Credit note amount** = the refund calculated by the policy (paid not used minus any admin
  fee); the admin fee and the registration fee kept are not credited. Amounts include 5% VAT;
  the credit note shows the VAT share; half-up to fils.
- **Session value:** course price ÷ number of sessions. "Used" = sessions `held` up to the
  withdrawal date, whether attended or not (unexcused absence counts as used).
- **Issued but unpaid instalment invoices:** credited in full; the amount owed, if any, is
  invoiced once.
- **Amount owed** = used sessions + admin fee − paid, never including unused future sessions.
- **Membership:** the subscription `cancel` (system) issues its own pro rata credit (BL-INV-21);
  the enrolment's credit note does not include the membership again.
- **Package:** wl-sales `refund` values the unused credits and issues the one credit note
  through wl-invoicing (wl-sales §4 *Package* `refund`); this vertical issues no second one, so
  "Must not refund without a credit note" is met by that credit note.
- **From `pending_payment`:** nothing paid; no credit note; the payment link is cancelled and the
  seat released.
- **Order of effects:** the state becomes `withdrawn` only when the refund approval is given; on
  reject or return (BL-KRN-01) the enrolment stays as it was. Effects then run in one
  transaction; the refund payment follows in wl-payments.
- **Refund fails:** the enrolment stays `withdrawn`; nothing in this vertical changes. The
  credit note balance is reopened by wl-invoicing on `RefundFailed` (BL-INV-08) and the retry
  is `FIN.RUL.21` (BL-PAY-06, BL-PAY-07); this issue only checks the result.

#### Must not
- Must not refund without a credit note.
- Must not leave scheduled instalments active after withdrawal.
- Must not let the recorder approve the refund.
- Must not credit a membership twice.

#### Done when
- [ ] Given a 12-session course at AED 2,520 (incl. VAT) paid upfront, admin fee AED 105, and 3
      sessions held, when counsellor A withdraws with policy "after start", a refund of
      2,520 − 3 × 210 − 105 = AED 1,785 waits for approval; A cannot approve; user B approves;
      the enrolment is `withdrawn`, a credit note of AED 1,785 (VAT AED 85.00) exists, the seat
      is free and a refund of AED 1,785 is requested.
- [ ] Given the same course in two instalments of AED 1,260 (second not yet invoiced) and 8
      sessions held, the second instalment is cancelled and an invoice for
      8 × 210 + 105 − 1,260 = AED 525 is issued; no credit note.
- [ ] Given 7 of 12 sessions held (after 50%) and paid upfront, no refund and no invoice.
- [ ] Given a membership enrolment, the subscription is `cancelled` at once with its pro rata
      credit, and no other credit note is issued.
- [ ] Given a 10-credit package with 4 unused and a single-session price of AED 150, the package
      is `refunded` for AED 600 and one credit note of AED 600 exists (issued through the
      package `refund`).
- [ ] Given the refund fails, the enrolment stays `withdrawn`, and (through BL-INV-08) the
      credit note balance of AED 1,785 is open again.

#### Out of scope
Blocked payer (BL-EDU-04); withdrawal from batch `terminate` (BL-EDU-12) or `cancel`
(BL-EDU-13); refund retries (BL-PAY-07).

---

### BL-EDU-04 — 10 EdTech: freeze the enrolments of a blocked party and hold their withdrawal refunds

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** s
**Depends on:** BL-PTY-01 · BL-PTY-03 (block effect per agreement kind) · BL-PTY-04 (money out exception) · BL-EDU-03 · **Blocks:** None
**Review items:** X-18

#### Goal
When the payer (or the student) is blocked as a party, their `active` enrolments pause through
`freeze` (`frozen` ones keep their freeze and get the block as a second cause; `pending_payment`
ones are left to lapse)
without the usual freeze-day limit, the seat is kept, and the centre manager decides any
withdrawal. A withdrawal refund to a blocked party waits unless a compliance exception is
approved. On unblock, enrolments frozen by the block are unfrozen.

#### Files to read
- `docs/verticals/10-edtech.md` §4.1 row "The payer is blocked", §7.1 *Enrolment* (`freeze`,
  `unfreeze`, `withdraw`)
- `docs/modules/wl-parties.md` §5.6

#### Terms
- **Block effect `pause`:** the agreement stays, service and billing stop, and it resumes on
  `unblock`.
- **`PTY.ACL.03`:** money out to a blocked party waits for unblock or an approved exception.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `freeze` | active | frozen | `enrolments.manage`, or system (subscription `pause`, `PartyBlocked`) | within the allowance of 30 days a year, shared with membership `pause` (not counted for a block or a mirrored system freeze) | each freeze cause stored (a freeze on a `frozen` enrolment only adds its cause); package `extend` by the frozen days; membership `pause` (`EDU.RUL.14`) |
| `unfreeze` | frozen | active | `enrolments.manage` or system (freeze end, subscription `resume`, `PartyUnblocked`) | each trigger removes only its own cause; unfreezes when none remains | membership `resume` |
| `withdraw` | pending_payment, active, frozen | withdrawn | `enrolments.manage` (the centre manager while the party is blocked), or system (batch `terminate` or `cancel` when no transfer is chosen) | refund calculated per policy (`EDU.REF.01`), or per batch `terminate` or `cancel`; approval per `FIN.ACL.04` (the state changes only on approval); refund paid only when the party is not blocked, or with a compliance exception (`PTY.ACL.03`) | credit note for the refund the policy gives (the admin fee and a kept registration fee stay revenue; session value = price ÷ sessions; *used* = sessions `held` up to the withdrawal date, attended or not); issued but unpaid instalment invoices credited in full and future instalments cancelled; membership `cancel`led at once (its own pro rata credit, not repeated); package `refund`ed (its own valuation); refund paid (`wl-payments`); if used sessions and fees exceed what was paid, one invoice for the amount owed; from `pending_payment`: no credit note, payment link cancelled; seat released |

§4.1: "Enrolment block effect, for enrolments whose payer or student is the blocked party:
pause through `freeze` without the freeze-day limit; the seat is kept; nothing is sent to the
blocked party, and staff without `parties.block_reason.view` see *On hold*. An enrolment already
frozen by the student keeps its freeze and gets the block as a second cause; `PartyUnblocked`
removes only the block's cause. `pending_payment` enrolments are left to lapse. Only the centre
manager may `withdraw`; the credit note is issued and the refund waits (`PTY.ACL.03`)."

wl-parties §5.6 row: "Enrolment | 10 vertical | pause (`freeze` without the freeze-day limit;
seat kept) | centre manager | 10".

#### Edge cases decided
- **Which enrolments:** those where the blocked party is the payer or the student.
- **Already frozen by the student:** stays frozen; the block is stored as a second freeze cause;
  `PartyUnblocked` removes only its cause.
- **`pending_payment` enrolments:** left alone; they lapse normally (no payment can arrive from a
  blocked payer through new links).
- **Package expiry:** extended by the frozen days (the `freeze` effect).
- **Withdrawal while blocked:** only the centre manager may run it; the credit note is issued; the
  refund waits for unblock or an approved exception.
- **Messages:** none to the blocked party about the freeze; reason *On hold* to staff without
  `parties.block_reason.view`.

#### Must not
- Must not count block days against the freeze allowance.
- Must not release the seat on freeze.
- Must not pay a refund to a blocked party without the exception.

#### Done when
- [ ] Given guardian G paying for active enrolments E1 and E2, when G is blocked, both are
      `frozen` with reason *On hold*, seats kept, and no message goes to G.
- [ ] A counsellor cannot withdraw E1; the centre manager can; the credit note is issued and the
      refund stays waiting.
- [ ] Given E3 frozen by the student until 2026-12-01, after `PartyUnblocked` on 2026-11-20 E3
      stays frozen; E1 and E2 are `active`.
- [ ] A package of E2 with expiry 2026-12-31 frozen 10 days now expires 2027-01-10.
- [ ] Given E4 of G in `pending_payment`, the block leaves it `pending_payment` and it lapses at
      its *seat hold until*.

#### Out of scope
Membership pause on the block (BL-INV-20); block and unblock (BL-PTY-01, BL-PTY-02).

---

### BL-EDU-05 — 10 EdTech: keep membership pause and enrolment freeze in step (`EDU.RUL.14`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:medium` · **Milestone:** Launch 10 · **Size:** s
**Depends on:** BL-INV-20 (subscription pause causes) · Base: 10 Enrolment lifecycle (`freeze`, `unfreeze`) · **Blocks:** None
**Review items:** 10 MEDIUM "freeze linked to membership pause"

#### Goal
A membership and its enrolment must not drift apart: pausing the subscription freezes the
enrolment and freezing the enrolment pauses the subscription, and the same for resume and
unfreeze, without loops and without running a transition that is already done.

#### Files to read
- `docs/verticals/10-edtech.md` §4.3 row "The membership is paused", §7.1 *Subscription*
  (`pause`, `resume`), *Enrolment* (`freeze`, `unfreeze`), §7.2 (`EDU.RUL.14`)

#### Terms
- **Belong together:** the subscription was created by the enrolment (membership catalogue item).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `EDU.RUL.14` | subscription `pause` or `resume`; enrolment `freeze` or `unfreeze` | the membership and the enrolment belong together (the subscription was created by the enrolment) and the other side is not already in that state | run the matching transition on the other side, with cause *linked* (it neither triggers this rule again nor counts against the allowance); a subscription resumed into `past_due` still unfreezes the enrolment; `suspended` is not a pause | — | on | MVP |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `pause` (subscription) | active, past_due | paused | `subscriptions.manage`, or system (enrolment `freeze`, `PartyBlocked`) | user pause within the allowance: 30 days a year, shared with enrolment `freeze` (10 overrides the wl-invoicing default of 0); a system pause is not counted | billing stops; end date extended; enrolment `freeze` (`EDU.RUL.14`) |
| `resume` (subscription) | paused | active, past_due | `subscriptions.manage` or system (pause end, enrolment `unfreeze`, `PartyUnblocked`) | each trigger removes only its own pause cause; resumes when none remains | billing resumes; back to `past_due` when cycle invoices from before the pause are still overdue; enrolment `unfreeze` either way (session access is decided by `suspend`) |
| `freeze` (enrolment) | active | frozen | `enrolments.manage`, or system (subscription `pause`, `PartyBlocked`) | within the allowance of 30 days a year, shared with membership `pause` (not counted for a block or a mirrored system freeze) | each freeze cause stored (a freeze on a `frozen` enrolment only adds its cause); package `extend` by the frozen days; membership `pause` (`EDU.RUL.14`) |
| `unfreeze` (enrolment) | frozen | active | `enrolments.manage` or system (freeze end, subscription `resume`, `PartyUnblocked`) | each trigger removes only its own cause; unfreezes when none remains | membership `resume` |

#### Edge cases decided
- **Allowance:** checked once, on the side the user acted on; the mirrored system transition does
  not check or count it again.
- **Loop guard:** the rule skips when the other side is already in the target state (spec); the
  mirrored transition carries the cause "linked" so it does not trigger the rule again.
- **Subscription `resume` lands in `past_due`** (wl-invoicing allows it): the enrolment still
  unfreezes.
- **Block:** `PartyBlocked` pauses both sides directly (BL-EDU-04, BL-INV-20); the rule skips
  because the other side is already frozen or paused.
- **Subscription `suspended`** (retries exhausted) is not `pause`; the rule does nothing.

#### Must not
- Must not run a transition twice or loop.

#### Done when
- [ ] Given membership M `active` and enrolment E `active`, when the guardian pauses M for 14
      days, E is `frozen`; when M resumes, E is `active`.
- [ ] When staff freeze E, M is `paused`; the allowance is counted once.
- [ ] Given M already `paused` by a block, freezing E does not run a second `pause`.
- [ ] No transition fires more than once in the history for one user action.
- [ ] Given M paused while a cycle invoice from before the pause is overdue, on resume M is
      `past_due` and E is `active`.

#### Out of scope
Pause causes and billing (BL-INV-20); blocked parties (BL-EDU-04).

---

### BL-EDU-06 — 10 EdTech: give the counsellor a task when a course fee is charged back

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** s
**Depends on:** BL-PAY-04 (chargeback) · BL-INV-07 (`InvoiceReopened`) · BL-INV-19 (membership goes `past_due`) · **Blocks:** None
**Review items:** X-06

#### Goal
When a card payment for a course fee is charged back, the invoice reopens. The enrolment does
not change by itself; the counsellor gets a task to collect the money again or withdraw the
student. (A membership goes `past_due` in the subscription module.)

#### Files to read
- `docs/verticals/10-edtech.md` §4.3 row "A card payment is charged back", §7.1 *Enrolment*
  (paragraph after the diagram)

#### Terms
- **`InvoiceReopened`:** wl-invoicing event when a paid invoice gets an open balance again
  because its payment was reversed.

#### Spec excerpt (authoritative for this issue)

§4.3: "The payment is reversed and the invoice reopened (`InvoiceReopened`); a membership goes
`past_due`; for a course fee the counsellor gets a task. Evidence is collected under
`FIN.RUL.20`."

§7.1: "A paid course fee reopened by a chargeback (`InvoiceReopened`) does not change the
enrolment by itself; the enrolment's counsellor (or, when that user is no longer active, the
counsellors of the enrolment's campus) gets a task to collect or `withdraw`."

§6 Enrolment fields include "campus, counsellor (the user who enrolled the student;
reassignable)".

#### Edge cases decided
- **Which invoices:** invoices of an enrolment's course fee or instalments (not membership cycle
  invoices). Package invoices too.
- **Task:** to the enrolment's counsellor (its *counsellor* field); when that user is no longer
  active, to the counsellors of the enrolment's campus. Due in 2 working days, with the amount,
  the payer and the links "send payment link" and "withdraw".
- **Several reopened invoices of one enrolment:** one open task, updated.
- **Chargeback won later** (invoice paid again): the task closes automatically.
- **Withdraw from this task:** normal `withdraw` (BL-EDU-03); amounts use what is actually paid
  after the reversal.

#### Must not
- Must not freeze, withdraw or block the student automatically.

#### Done when
- [ ] Given enrolment E with course fee invoice INV-5 of AED 2,520 paid by card, when the payment
      is reversed for a chargeback, INV-5 reopens, E stays `active`, and the counsellor has a
      task with AED 2,520 and both links.
- [ ] When the chargeback is won and INV-5 is paid again, the task closes.
- [ ] A membership cycle invoice reopened creates no counsellor task.
- [ ] Given E's counsellor C1 deactivated, the task goes to the counsellors of E's campus.

#### Out of scope
Chargeback lifecycle (BL-PAY-04, BL-PAY-05); membership `past_due` (BL-INV-19).

---

### BL-EDU-07 — 10 EdTech: block batch `open` and certificate `issue` on an expired centre permit or course approval (`EDU.RUL.13`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** BL-DOC-01 · BL-DOC-04 · BL-DOC-05 · BL-KRN-08 · **Blocks:** BL-EDU-16
**Review items:** X-13, 10-N8-a

#### Goal
A centre must not open a batch or issue a certificate under an expired centre permit or course
approval. Both transitions are blocked with no override. Sixty days before expiry, and on
expiry, the coordinator, the centre manager and the owner are told which batches are affected,
and a renewal task goes to the kind's responsible role. Running batches are not stopped by the system; whether they may
finish is an open question.

#### Files to read
- `docs/verticals/10-edtech.md` §2 (KHDA, ACTVET, SPEA), §4.2 row "The course approval or centre
  permit expires", §7.1 *Batch* (`open`), *Certificate* (`issue`), §7.2 (`EDU.RUL.13`), §9
  "Documents that block work"
- `docs/modules/wl-documents.md` §5.6
- `docs/OPEN-ITEMS.md` AU-25, L-19

#### Terms
- **Centre permit:** the regulator's permit for the legal entity or campus (KHDA, ACTVET, SPEA).
- **Course approval:** the regulator's approval of a programme, where the course is marked as
  requiring one.
- **States it:** the certificate template prints the approval or attestation.

#### Spec excerpt (authoritative for this issue)

| Document kind (subject) | `blocks` (mode) | Override | Responsible role |
|---|---|---|---|
| Centre permit (legal entity, campus) | `batch.open`, `certificate.issue` (block, always) | never | centre manager |
| Course approval (course, where required) | `batch.open` (block); `certificate.issue` (block, condition: the certificate states the approval) | never | academic coordinator |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `open` (batch) | planned | open | `batches.manage` | tutor and room available for every session; `document_valid` for the centre permit and the course approval where required (§9) on the batch's last session date; tutor passes `EDU.ACL.14` on every session date | sessions generated; listed in the catalogue |
| `issue` (certificate) | draft | issued | `certificates.issue` | `EDU.ACL.06` criteria met; `document_valid` for the centre permit, and for the course approval when the certificate states it (§9); for a reissued draft both checks use the enrolment's completion date | number and verification code assigned; PDF in the portal; a superseded certificate's page now links this one |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `EDU.RUL.13` | `DocumentExpiring` or `DocumentExpired` for the centre permit or a course approval | — | list the `open` and `running` batches of that course (or campus): no new batch `open`, and certificates blocked as in §9; tell the coordinator, the centre manager and the owner in one message; renewal task to the kind's responsible role (§9: centre permit → centre manager; course approval → academic coordinator), due on the expiry date; running batches are not stopped | look-ahead 60 days | on | MVP |

§9: "A running batch whose course approval expires is not stopped by the system; the centre
manager decides (`EDU.RUL.13`) `⚠ open: authority (AU-25)`." Compliance-critical: `EDU.RUL.13`,
`DOC.RUL.03`.

#### Edge cases decided
- **`on_date` for `batch.open`:** the last session date of the batch (the permit and approval
  must cover the whole batch).
- **Centre permit and `certificate.issue`:** blocks always (§9 table); the course approval blocks
  only when the certificate states it (§7.1 guard, as the entry's condition).
- **Course without "approval required":** no course approval check.
- **Look-ahead 60 days:** the kinds' look-ahead window is 60 days, so `DocumentExpiring` arrives
  60 days ahead (BL-EDU-17).
- **Running batches:** listed in the message; nothing is stopped; the centre manager may
  `terminate` (BL-EDU-12). Certificates of a running batch cannot be issued while the approval
  is expired.
- **Renewal task:** to the kind's responsible role (§9 table): centre permit → centre manager
  of the campus; course approval → academic coordinator (of the course, or of the campuses with
  its batches). Due on the expiry date. The centre manager is still in the message.
- **Renewal accepted:** the guard answers yes again; drafts waiting can be issued.

#### Must not
- Must not offer an override.
- Must not stop or terminate running batches automatically.

#### Done when
- [ ] Given course C's approval expiring 2027-01-31 and planned batch B with sessions to
      2027-03-15, `open` on 2026-11-10 is refused naming the approval.
- [ ] Given B ending 2027-01-20, `open` succeeds.
- [ ] Given the campus permit expired 2026-11-05, issuing a certificate draft on 2026-11-10 is
      refused even if the template does not print the approval.
- [ ] On 2026-12-02 (60 days before 2027-01-31), the coordinator, the centre manager and the owner
      get one message listing C's open and running batches, and the academic coordinator has a
      renewal task due 2027-01-31.
- [ ] For a campus permit expiring 2027-02-28, the campus's centre manager has the renewal task.
- [ ] A failed `EDU.RUL.13` run alerts the owner and the admin.

#### Out of scope
Tutor eligibility (BL-EDU-08); batch termination (BL-EDU-12).

---

### BL-EDU-08 — 10 EdTech: guard tutor assignment on every session date (`EDU.ACL.14`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** BL-HRM-01 (`qualification_valid`) · BL-DOC-01 · BL-DOC-04 · BL-DOC-06 · **Blocks:** BL-EDU-09, BL-EDU-10, BL-EDU-11
**Review items:** X-13, 10-N8-c

#### Goal
A tutor may be assigned to a batch, a session or a one-to-one booking only if their required
documents are valid on every date they will teach: a police clearance when minors are enrolled,
and a MoHRE private tutor permit or freelance visa for freelance tutors. This issue builds the
guard for batches and sessions. For one-to-one bookings the same rule is enforced by
wl-scheduling (`ResourceEligibility`, `booking.book` and `booking.reassign`) with the kinds that
BL-EDU-17 sets on the service types; here bookings appear only in the `FutureWork` provider. The
batch and session records register the providers that let wl-documents find what an expiring
document affects.

#### Files to read
- `docs/verticals/10-edtech.md` §2 (private tutors, minors), §3.3 (`EDU.ACL.14`), §7.1 *Batch*
  (`open`), *Session* (`reschedule`), *Booking* (paragraph after the diagram), §9
- `docs/modules/wl-hr.md` §2 (standard guard) · `docs/modules/wl-documents.md` §5.6
- `docs/OPEN-ITEMS.md` L-19

#### Terms
- **Required kinds:** police clearance (when minors are enrolled); for tutors with employment
  type `freelance`, a MoHRE private tutor permit or a freelance visa (either one).
- **`BlockSubjects` / `FutureWork`:** see wl-documents §5.6.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `EDU.ACL.14` | A tutor is assigned to a batch, a session or a one-to-one booking only if `qualification_valid` passes on every session date for the kinds the course requires (police clearance when minors are enrolled: the course is marked *for minors*, or the batch has a `pending_payment`, `active` or `frozen` enrolment of a student under 18 on the session date; MoHRE private tutor permit or freelance visa, either one, for freelance tutors). Override per §9 | `STA` | coordinator, counsellor | — |

| Document kind (subject) | `blocks` (mode) | Override | Responsible role |
|---|---|---|---|
| Police clearance (tutor; required for courses with minors) | `batch.open`, `batch.change_tutor`, `session.change_tutor`, `session.reschedule`, `booking.book`, `booking.reassign` (block); `session.submit_attendance` (flag) | never by default (best practice, not a legal block; the client may switch it to approval by the owner, one use, 72 hours) | academic coordinator |
| MoHRE private tutor permit or freelance visa (freelance tutor) | as police clearance (block and flag) | never | academic coordinator |

| Transition | From | To | By | Guards |
|---|---|---|---|---|
| `reschedule` (session) | scheduled | scheduled | `batches.manage` | tutor and room free at the new time; tutor passes `EDU.ACL.14` on the new date |

§9: "The batch and session records register `BlockSubjects` (batch → course, campus, tutor;
session → tutor) and `FutureWork` (sessions and one-to-one bookings after the expiry date)."

#### Edge cases decided
- **"Minors are enrolled":** the course is marked *for minors*, or the batch has an enrolment
  (`pending_payment`, `active`, `frozen`) of a student under 18 on the session date.
- **A minor enrols into a batch whose tutor has no valid clearance:** the enrolment is allowed
  (enrolment is not in the `blocks` list), the batch's sessions are flagged and the coordinator
  gets a task (`EDU.RUL.12`, BL-EDU-10).
- **Every session date:** `qualification_valid(tutor, kind, session date)` for each remaining
  session the tutor would teach.
- **Freelance kinds:** passes if either the permit or the freelance visa is valid.
- **Bookings:** the `booking.book` and `booking.reassign` blocks of the §9 table are enforced by
  wl-scheduling through `ResourceEligibility` with the kinds BL-EDU-17 sets on the one-to-one
  service types; this issue builds no booking guard. It only includes the tutor's one-to-one
  bookings in `FutureWork` (so the expiry message lists them).
- **Override:** none by default; if the client switches police clearance to `with_approval`,
  the approver is the owner (one use, 72 h). The freelance permit stays `never`.

#### Must not
- Must not check only the first session date.
- Must not show minors' data in the refusal message.

#### Done when
- [ ] Given tutor T with a police clearance valid to 2026-12-15 and planned batch B (minors) with
      sessions to 2027-02-28, `open` with T is refused naming the clearance and the first
      failing date 2026-12-16.
- [ ] Given an adult course without minor enrolments, the same `open` succeeds.
- [ ] Given freelance tutor F with an expired permit but a valid freelance visa, F can be assigned.
- [ ] Rescheduling a session of B to 2026-12-20 with T is refused; to 2026-12-10 succeeds.
- [ ] `FutureWork(T, 2026-12-15)` returns B's sessions after 2026-12-15 and T's one-to-one
      bookings after that date.

#### Out of scope
`change_tutor` (BL-EDU-09); look-ahead flags (BL-EDU-10); the flag at `submit_attendance`
(BL-EDU-11); booking guards (wl-scheduling).

---

### BL-EDU-09 — 10 EdTech: change the tutor of a batch or a session (`change_tutor`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:medium` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** BL-EDU-08 · **Blocks:** BL-EDU-10
**Review items:** 10 MEDIUM "tutor reassignment", 10-N8-c

#### Goal
When a tutor becomes unavailable or ineligible, the coordinator moves the remaining sessions of a
batch, or a single session, to another tutor who is free and eligible on every date. Students and
guardians are told and the conflicts on the old tutor are cleared.

#### Files to read
- `docs/verticals/10-edtech.md` §4.2 row "A tutor is sick or leaves", §7.1 *Batch*
  (`change_tutor`), *Session* (`change_tutor`)

#### Terms
- **Remaining sessions:** `scheduled` sessions of the batch dated on or after the *from* date.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `change_tutor` (batch) | open, running | same state | `batches.manage` | from a date; the new tutor is free and passes `EDU.ACL.14` on every remaining session date (all or nothing: the failing dates are shown) | remaining sessions of the batch's current tutor reassigned (sessions already moved to another tutor keep theirs); batch tutor set from that date; each student or guardian told once; conflicts on the old tutor cleared |
| `change_tutor` (session) | scheduled | scheduled | `batches.manage`, or system (batch `change_tutor`) | new tutor free and passes `EDU.ACL.14` on the date | students and guardians told (no reason given); conflict cleared |

#### Edge cases decided
- **Sessions already moved one by one to a third tutor:** keep their tutor; the batch change
  moves only sessions taught by the batch's current tutor.
- **Batch tutor field:** set to the new tutor from the *from* date.
- **One message per student for a batch change** (not one per session), via WhatsApp with email
  fallback.
- **Partial failure:** if the new tutor fails on any remaining date, nothing is changed and the
  failing dates are shown (all or nothing).
- **Pay:** sessions are paid to the tutor who submits attendance (`EDU.RUL.10` collects by
  session tutor).
- **Concurrency:** the availability check and assignment run under a lock on the new tutor's
  calendar.

#### Must not
- Must not change `held` or `cancelled` sessions.
- Must not tell students why the tutor changed.

#### Done when
- [ ] Given running batch B (tutor T) with 10 remaining sessions from 2026-11-16, when the
      coordinator changes to tutor U from 2026-11-16 and U is free and eligible, all 10 are U's,
      each student or guardian gets one message, and T's conflicts on B are cleared.
- [ ] Given U busy on 2026-11-23, nothing changes and 2026-11-23 is shown.
- [ ] Changing one session on 2026-11-20 to tutor V affects only that session.
- [ ] A session moved to V earlier keeps V after a batch change to U.

#### Out of scope
Eligibility guard (BL-EDU-08); flags and tasks (BL-EDU-10).

---

### BL-EDU-10 — 10 EdTech: flag sessions of a tutor who will not be eligible or available (`EDU.RUL.12`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** BL-EDU-08 · BL-EDU-09 · BL-HRM-01 · BL-SCH-05 (booking conflicts) · BL-KRN-08 · **Blocks:** None
**Review items:** X-13, X-17, 10-N8-c, 10 MEDIUM "tutor reassignment"

#### Goal
Before a tutor's police clearance or permit expires, when the tutor is marked unavailable (a
manual availability block in the MVP; tutor leave is Later for 10), or when the tutor gives
notice, the coordinator must see which sessions are affected and change the tutor in time. The
rule flags those sessions, gives the coordinator a task, and tells the centre manager if a flagged
session is still unresolved 24 hours before it starts. It is compliance-critical.

#### Files to read
- `docs/verticals/10-edtech.md` §4.2 rows "A tutor's police clearance or permit will expire", "A
  tutor is sick or leaves", §7.1 *Booking* (paragraph after the diagram), §7.2 (`EDU.RUL.12`,
  compliance-critical list)

#### Terms
- **Flagged session:** a session showing "tutor not available or not eligible" until resolved.
- **Resolved:** the session's tutor changed, rescheduled to an eligible date, cancelled, or the
  document renewed or leave cancelled.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `EDU.RUL.12` | `DocumentExpiring`, `DocumentExpired`, `QualificationExpired`, `EmployeeNoticeGiven` for a tutor, or a manual availability block on the tutor (wl-scheduling) created or changed; daily. `LeaveApproved` is a trigger too but stays inactive until tutor leave (`HRM.LEV.01`, Later for 10) is enabled | the tutor is assigned to sessions or one-to-one bookings dated on or after the date and no more than D days after the later of the date and today (the window moves with the daily run): the date is the day after the expiry date for document events, the leave or block start, the day after the last working day for a notice (for leave and blocks, only sessions inside the period) | flag the sessions; one task per tutor and trigger to the coordinator to `change_tutor` or reschedule, updated by the daily run as sessions enter the window; one-to-one bookings are listed in the task but flagged and escalated only by wl-scheduling (`BookingConflict`, `SCH.RUL.05`–`06`); if a flagged session is still unresolved H hours before it, notify the centre manager once | D = 30; H = 24 | on | MVP |

§4.2: "`DOC.RUL.02` looks 30 days ahead; `EDU.RUL.12` lists the tutor's batches, sessions and
one-to-one bookings after the expiry date and gives the coordinator a task to change the tutor.
On expiry the tutor cannot be assigned or teach (`EDU.ACL.14`); open conflicts H hours before a
session go to the centre manager."

§7.1 *Booking*: "`SCH.RUL.05` checks bookings D = 30 days ahead and raises `BookingConflict`
(cause `eligibility`); `SCH.RUL.06` escalates an open conflict H = 2 hours before the start."

#### Edge cases decided
- **"The date":** expiry date + 1 day for document events; leave or block start date; last
  working day + 1 for `EmployeeNoticeGiven`.
- **Window:** sessions dated from the date up to max(the date, today) + 30 days. Before the date
  (e.g. at `DocumentExpiring` 30 days ahead) the window is the date to date + 30; after it, the
  daily run moves the end forward each day, so later sessions enter one by one (for a clearance
  expiring 2026-12-15: the run on 2026-12-21 covers 2026-12-16 to 2027-01-20).
- **Leave:** only sessions inside the leave period are flagged.
- **Bookings:** listed in the coordinator task; their flag and escalation are wl-scheduling's
  `BookingConflict` (`SCH.RUL.05`, `SCH.RUL.06` at 2 h), not a second flag here.
- **One task per tutor per trigger,** updated by the daily run; the daily run also catches
  sessions entering the 30-day window.
- **Tutor leave in 10:** `HRM.LEV.01` is Later for 10, so the `LeaveApproved` trigger stays
  inactive until tutor leave is enabled; it is built now. The MVP trigger for absence is a
  manual availability block on the tutor (wl-scheduling), created or changed; only sessions
  inside the block are flagged.
- **Escalation at 24 h:** one message to the centre manager per flagged session.

#### Must not
- Must not change tutors automatically.
- Must not send a second escalation for one-to-one bookings.

#### Done when
- [ ] Given T's police clearance expiring 2026-12-15 and T teaching batch B sessions on 2026-12-16,
      2026-12-18 and 2027-01-20, on `DocumentExpiring` the sessions of 2026-12-16 and 2026-12-18
      are flagged (within 30 days of 2026-12-16) and the coordinator has one task.
- [ ] The daily run on 2026-12-20 does not flag the 2027-01-20 session (window ends 2027-01-19);
      the run on 2026-12-21 flags it (2026-12-21 + 30 days) and updates the task.
- [ ] At 2026-12-15 10:00 (24 h before a 10:00 session on 2026-12-16), with the session still
      flagged, the centre manager is notified.
- [ ] After `change_tutor` to an eligible tutor, the flags clear.
- [ ] A manual block on T for 2026-11-20 to 2026-11-22 flags T's sessions on those dates only and
      gives the coordinator a task.
- [ ] A failed run alerts the owner and the admin.

#### Out of scope
The guard (BL-EDU-08); booking conflicts (BL-SCH-05 to BL-SCH-07).

---

### BL-EDU-11 — 10 EdTech: flag a session taught by an ineligible tutor (`submit_attendance`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** s
**Depends on:** BL-EDU-08 · BL-DOC-02 (flag mode) · **Blocks:** None
**Review items:** X-13, 10-N8-c

#### Goal
A session that took place cannot be undone, so attendance is always accepted. If the tutor did
not pass `EDU.ACL.14` that day, the session is flagged for the centre manager to review.

#### Files to read
- `docs/verticals/10-edtech.md` §7.1 *Session* (`submit_attendance`), §9 table

#### Terms
- **Flag mode:** the transition runs and the record is flagged for review (wl-documents §5.6).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit_attendance` | scheduled | held | `sessions.teach` (`EDU.ACL.03`, `EDU.ACL.04`) | start time passed; every enrolled student marked | package credits consumed per the absence policy; absences evaluated (`EDU.RUL.02`); session flagged for the centre manager if the session's tutor failed `EDU.ACL.14` that day (police clearance or freelance permit; §9 `flag`), whoever submitted; the centre manager closes the flag with a note; credits and tutor pay are not changed by it |

§9: police clearance — "`session.submit_attendance` (flag)"; freelance permit — "as police
clearance (block and flag)".

#### Edge cases decided
- **Freelance permit:** flags too (same entries as police clearance, §9 "block and flag").
- **Review:** the centre manager closes the flag with a note; credits and tutor pay are not
  changed by the flag.
- **Who submitted:** if the coordinator submits for the tutor, the check is still on the
  session's tutor.

#### Must not
- Must not refuse the attendance.

#### Done when
- [ ] Given T's clearance expired 2026-12-15 and T's session on 2026-12-16 (minors), when T submits
      attendance, the session is `held`, credits are consumed, and the centre manager sees a flag.
- [ ] The centre manager closes the flag with a note; the history shows it.
- [ ] Given T valid that day, no flag.

#### Out of scope
Credit consumption rules (BL-EDU-15).

---

### BL-EDU-12 — 10 EdTech: terminate a running batch (`terminate`, `EDU.ACL.15`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** BL-EDU-02 · BL-EDU-03 · BL-KRN-01 · **Blocks:** None
**Review items:** 10-N12-b

#### Goal
A running batch sometimes has to stop (tutor unavailable, approval expired or revoked, venue lost,
too few students). The coordinator requests termination and the centre manager approves. The
remaining sessions are cancelled; each student is either moved to another batch or withdrawn
with the unused sessions refunded pro rata and no admin fee; the tutor is paid only for sessions
taught; students and guardians are told; for a regulatory reason the owner is told too.

#### Files to read
- `docs/verticals/10-edtech.md` §2 (fees and refunds: learners settled if an authorisation is
  revoked), §3.3 (`EDU.ACL.15`), §4.2 row "A running batch must stop", §7.1 *Batch*
  (`terminate`), *Enrolment* (`withdraw`, `transfer`)

#### Terms
- **Regulatory reason:** approval expired or revoked.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `terminated` | error | yes | Stopped after it started; remaining sessions cancelled |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `terminate` | running | terminated | `batches.manage`, approved by the centre manager (`EDU.ACL.15`) | reason (tutor unavailable, approval expired or revoked, venue, too few students); per enrolment the choice *transfer to batch X* or *withdraw* (default withdraw) | sessions held before the approval count as used; remaining sessions cancelled; each `active` or `frozen` enrolment transferred (seat in the target batch at approval, else withdraw and the coordinator told; the unused value moves to the new enrolment, price differences invoiced or credited there) or withdrawn with the unused sessions refunded pro rata (price paid ÷ sessions × unused, capped at the amount paid) without admin fee or the no-refund tier, the registration fee not refunded; `pending_payment` enrolments withdrawn with nothing to refund; tutor paid only for sessions taught; students and guardians told; centre manager and owner told when the reason is regulatory |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `EDU.ACL.15` | Terminating a running batch is requested by the coordinator and approved by the centre manager; refunds from it follow `FIN.ACL.04` | `SOD` | coordinator, centre manager | — |

#### Edge cases decided
- **Per-enrolment choice:** the request lists each enrolment with "transfer to batch X" or
  "withdraw"; default withdraw. Transfers need a free seat at approval time (`EDU.ACL.17`); if the
  seat is gone, that enrolment falls back to withdraw and the coordinator is told.
- **Refund:** price paid ÷ total sessions × unused sessions, capped at the amount paid; no admin
  fee; the 50% no-refund tier does not apply; the registration fee is not refunded (only a batch
  `cancel` refunds it).
- **Transfer balance:** the unused value moves to the new enrolment; differences in price are
  invoiced or credited there.
- **`pending_payment` enrolments:** withdrawn with nothing to refund; payment link cancelled.
- **Approvals:** the termination is one approval (centre manager); each refund then follows
  `FIN.ACL.04` (recorder ≠ approver).
- **Tutor pay:** no entry for cancelled sessions.
- **Approval pending:** sessions keep running; sessions held before approval count as used.

#### Must not
- Must not deduct an admin fee or apply the no-refund tier.
- Must not let the coordinator approve the termination.

#### Done when
- [ ] Given running batch B (12 sessions, 5 held) with enrolments E1 (paid AED 2,520) and E2, when
      the coordinator requests `terminate` with reason "venue", E1 → withdraw, E2 → transfer to
      batch B2, and the centre manager approves: 7 sessions are `cancelled`, E1 is `withdrawn` with
      a refund of 2,520 × 7/12 = AED 1,470 awaiting approval, E2 is `transferred` with a new active
      enrolment in B2, and students and guardians are told.
- [ ] With reason "approval revoked" (or "approval expired"), the centre manager and the owner
      each get the regulatory notice as well; with reason "venue" they do not.
- [ ] The coordinator cannot approve their own request.
- [ ] B2 full at approval: E2 is withdrawn instead and the coordinator is told.
- [ ] The tutor's timesheet has 5 sessions, not 12.

#### Out of scope
Withdrawal money documents (BL-EDU-03); batch `cancel` (BL-EDU-13).

---

### BL-EDU-13 — 10 EdTech: cancel a batch before it runs with a full refund (`cancel`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** s
**Depends on:** BL-EDU-03 · BL-EDU-02 (seat check for transfers, `EDU.ACL.17`) · **Blocks:** None
**Review items:** 10-N3-f, 10-N12-b

#### Goal
When the centre cancels a batch before its first session, the cancellation is business-initiated:
each student is moved to another batch or withdrawn with everything refunded, including the
registration fee.

#### Files to read
- `docs/verticals/10-edtech.md` §7.1 *Batch* (`cancel`), §7.2 (`EDU.RUL.05`)

#### Terms
- **Business-initiated:** the centre, not the student, ends the enrolment; no policy deductions apply.
- **Registration fee:** the one-off fee charged at enrolment, kept on a student withdrawal before the start.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `cancel` | planned, open | cancelled | `batches.manage` | reason given; per enrolment the choice *transfer to batch X* or *withdraw* (default withdraw) | enrolments transferred, or withdrawn with a full refund including the registration fee (business-initiated); a transfer whose target seat is gone at that moment falls back to withdraw and the coordinator is told; `pending_payment` enrolments withdrawn with nothing to refund and the link cancelled; parties notified |

#### Edge cases decided
- **Per-enrolment choice:** the cancel lists each enrolment with "transfer to batch X" or
  "withdraw"; default withdraw.
- **Seat fallback:** a transfer takes a seat in the target batch at the moment of the cancel,
  through the atomic seat check of BL-EDU-02 (`EDU.ACL.17`); when the seat is gone, that
  enrolment is withdrawn with the full refund instead and the coordinator is told.
- **Full refund:** everything paid for the enrolment (course fee, instalments, registration fee);
  the credit note covers it; scheduled instalments are cancelled.
- **`pending_payment` enrolments:** withdrawn with nothing to refund; link cancelled.
- **Refund approval:** `FIN.ACL.04` still applies.

#### Must not
- Must not apply the refund policy tiers or deduct any fee.

#### Done when
- [ ] Given open batch B with E1 paid AED 2,520 + registration AED 210, when the coordinator
      cancels B with reason "too few students" and E1 → withdraw, E1 is `withdrawn`, a credit note
      of AED 2,730 exists and a refund of AED 2,730 awaits approval; the guardian is told.
- [ ] E2 → transfer to B2 creates an active enrolment in B2.
- [ ] Given B2 full at the cancel, E2 is withdrawn with a full refund instead and the
      coordinator is told.
- [ ] `cancel` without a reason is refused; from `running` it is not offered.

#### Out of scope
`terminate` (BL-EDU-12).

---

### BL-EDU-14 — 10 EdTech: correct attendance of a held session (`correct_attendance`, `EDU.ACL.16`)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:medium` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** BL-EDU-15 · BL-ATT-07, BL-ATT-08 (adjustment timesheets) · Base: wl-sales Package `reverse` and `consume` · **Blocks:** None
**Review items:** 10 MEDIUM "attendance correction after a session"

#### Goal
Attendance marked wrongly must be correctable without breaking credits, completion or pay. The
tutor may correct within 24 hours after the session ends; later only the coordinator, with a
reason. Credits are reversed or consumed, completion is re-evaluated, the guardian is told when an
absence alert had been sent, and a locked tutor timesheet is corrected by an adjustment. If an
issued certificate is affected, it goes through reissue or revoke.

#### Files to read
- `docs/verticals/10-edtech.md` §3.3 (`EDU.ACL.04`, `EDU.ACL.16`), §4.2 row "Attendance was marked
  wrong", §4.5 step 4, §7.1 *Session* (`held`, `correct_attendance`), §8 (paper attendance)

#### Terms
- **`held`:** the session took place and attendance was submitted; it can still be corrected (not
  terminal).
- **Completion criteria:** attendance ≥ 80% and assessment passed (`EDU.ACL.06`).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `held` | success | no | Took place; attendance submitted; can be corrected |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `correct_attendance` | held | held | `sessions.teach` within 24 hours of the scheduled end, then the coordinator (`EDU.ACL.16`) | reason given | credits `reverse`d or consumed (with no credit left, the correction is saved and the counsellor gets a task to sell credits); completion re-evaluated (a draft certificate whose criteria now fail is discarded; an issued one gets a coordinator task to `reissue` or `revoke`); guardian told when an absence alert was sent; tutor timesheet adjusted only when a timesheet entry changes (adjustment when locked) |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `EDU.ACL.04` | A tutor can mark or change attendance only until a time after the session ends; later changes need the coordinator | `TIM` | tutor | 24 hours |
| `EDU.ACL.16` | Correcting attendance of a `held` session after the tutor's window (`EDU.ACL.04`) needs the coordinator and a reason; a change that affects an issued certificate goes through certificate `reissue` or `revoke` | `STA` | coordinator | — |

#### Edge cases decided
- **Window:** 24 hours from the session's scheduled end time, Asia/Dubai.
- **Credits:** the corrected mark is priced again by the stored absence policy (BL-EDU-15) and
  only the difference moves. A present student always consumes a credit; an unexcused absence
  consumes one when the policy says so (default yes); an excused absence never does. So, with
  the default policy: unexcused absent → present: no change (consumed before and after);
  present → unexcused absent: no change; excused → present or unexcused: one credit consumed;
  present or unexcused → excused: the credit is `reverse`d. With "unexcused absence consumes" =
  no: unexcused → present consumes one, present → unexcused reverses one. A package already
  `completed` at zero that gets a credit back returns to `active` through `reverse`.
- **No credits left** for a newly consumed credit: the correction is saved and the counsellor gets
  a task to sell credits.
- **Completion:** re-evaluated; an enrolment already `completed` stays completed (terminal); if a
  certificate was issued and the criteria are no longer met, the coordinator gets a task to
  `reissue` or `revoke` (never automatic). If a draft certificate exists and criteria fail, the
  draft is discarded.
- **Guardian message:** only when the correction changes an absence that had triggered an
  `EDU.RUL.02` alert.
- **Tutor timesheet:** changed only when the correction changes a timesheet entry (sessions are
  paid per session taught, so student marks usually do not change it); when the timesheet is
  `locked`, an adjustment timesheet (BL-ATT-07).
- **Paper attendance entered late** (integration down) by the tutor or coordinator follows the
  same windows.

#### Must not
- Must not edit a locked timesheet.
- Must not change an issued certificate directly.

#### Done when
- [ ] Given session S held on 2026-11-10 ending 18:00 with student P marked absent (unexcused,
      credit consumed), tutor T marks P present at 2026-11-11 10:00 with reason "marked wrong
      row": no credit change, P's absence streak resets, and the guardian, who got an absence
      alert, is told.
- [ ] At 2026-11-11 18:30 T cannot correct; the coordinator can, with a reason; without a reason
      it is refused.
- [ ] Marking P as excused reverses the credit (balance +1).
- [ ] Given P marked excused (no credit) and corrected to present, one credit is consumed.
- [ ] Given P's certificate issued and attendance falling to 75%, the coordinator gets a task to
      reissue or revoke; the certificate is unchanged.

#### Out of scope
Credit rules (BL-EDU-15); certificate reissue (BL-EDU-16); adjustment timesheets (BL-ATT-07,
BL-ATT-08).

---

### BL-EDU-15 — 10 EdTech: consume package credits by the absence policy

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:medium` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** Base: wl-sales Package `consume` · Base: 10 Session lifecycle (`submit_attendance`, `cancel`) · **Blocks:** BL-EDU-14
**Review items:** 10 MEDIUM "absence consumption policy"

#### Goal
Whether a missed session costs a credit must follow the catalogue item's absence policy, not a
fixed rule. By default an unexcused absence consumes a credit, an absence notified at least 24
hours before is excused (no credit, and a make-up right), memberships consume nothing, and
nothing is consumed when the centre cancels a session or a one-to-one booking.

#### Files to read
- `docs/verticals/10-edtech.md` §4.2 row "A student is absent", §6 (Catalogue item: absence
  policy), §7.1 *Session* (`submit_attendance`, `cancel`), *Booking* (`cancel`)

#### Terms
- **Absence policy (per catalogue item):** *unexcused absence consumes a credit* (default yes);
  *notice for an excused absence* (default 24 h).
- **Make-up right:** an entitlement to book a make-up session in another batch (`EDU.MKP.01`).

#### Spec excerpt (authoritative for this issue)

§4.2: "The catalogue item's absence policy decides: by default an unexcused absence consumes a
package credit; an absence notified at least N hours before (default 24) is *excused*, consumes
nothing and gives a make-up right. Memberships consume nothing."

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit_attendance` | scheduled | held | `sessions.teach` (`EDU.ACL.03`, `EDU.ACL.04`) | start time passed; every enrolled student marked | package credits consumed per the absence policy; absences evaluated (`EDU.RUL.02`); session flagged for the centre manager if the session's tutor failed `EDU.ACL.14` that day (police clearance or freelance permit; §9 `flag`), whoever submitted; the centre manager closes the flag with a note; credits and tutor pay are not changed by it |
| `cancel` (session) | scheduled | cancelled | `batches.manage` | reason given | students notified; no credits consumed; optional make-up session added to the batch |
| `cancel` (booking) | requested, confirmed | cancelled | `bookings.manage` or the party | … | …; business-initiated (tutor sick, tutor ineligible): any deposit refunded in full (`SCH.ACL.05`); a package credit is not consumed |

#### Edge cases decided
- **Recording a notice:** the guardian or student notifies through the portal or the counsellor
  records it with a timestamp; the student is pre-marked *excused* when the notice is at least
  24 h before the session start.
- **Late notice (< 24 h):** unexcused unless the coordinator marks it excused with a reason.
- **Late arrival:** counts as present.
- **Batch enrolments without a package** (course fee): nothing to consume; excused absences still
  give a make-up right.
- **Make-up right:** one per excused absence, valid until the enrolment ends.
- **Policy used:** the one stored with the enrolment at enrolment time.

#### Must not
- Must not consume a credit for a cancelled session or a business-cancelled booking.
- Must not consume credits for memberships.

#### Done when
- [ ] Given a 10-credit package (default policy) and P absent without notice, `submit_attendance`
      consumes 1 credit.
- [ ] Given a notice on 2026-11-09 09:00 for a session on 2026-11-10 10:00, P is excused, no credit
      is consumed and P has one make-up right.
- [ ] A notice at 2026-11-10 08:00 for the same session leaves P unexcused.
- [ ] Given a catalogue item with "unexcused absence consumes" = no, nothing is consumed.
- [ ] A cancelled session and a tutor-sick booking cancellation consume nothing.

#### Out of scope
Corrections after the session (BL-EDU-14); make-up booking itself (base `EDU.MKP.01`).

---

### BL-EDU-16 — 10 EdTech: reissue a certificate (`reissue`) and print a copy

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:medium` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** Base: 10 Certificate lifecycle (`issue`, `revoke`, public verification page) · BL-EDU-07 · **Blocks:** None
**Review items:** 10 MEDIUM "certificate reissue"

#### Goal
A certificate with a wrong name or date, or whose result changed, is never edited. It is reissued:
the old one becomes `superseded`, the public verification page points to the new number, and a
new draft with a new number is created and then issued. A lost copy is simply printed again
without any change.

#### Files to read
- `docs/verticals/10-edtech.md` §4.4 "When it goes wrong", §6 (Certificate: supersedes /
  superseded by), §7.1 *Certificate*

#### Terms
- **Verification code:** printed on the certificate with a QR; the public page shows the
  certificate's status.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `superseded` | neutral | yes | Replaced by a reissued certificate; the public page points to the new one |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reissue` | issued | superseded | `certificates.issue` | reason (name or date correction, changed result that still meets the criteria; otherwise `revoke`); the corrected data saved | new draft created, linked to the old one, numbered when issued; the old page shows "superseded, replacement pending" until then; a recorded KHDA attestation is copied only if the coordinator confirms; student notified when the new one is issued |

§4.4: "A copy is lost: A copy of the same certificate is generated; nothing changes in its state."

#### Edge cases decided
- **Number:** the new number is assigned when the draft is issued (as for any certificate); the
  old page shows "superseded" and links the new one only after the new one is issued; until then
  it shows "superseded, replacement pending".
- **Issue guard for the new draft:** `EDU.ACL.06` and the approval check run with `on_date` = the
  enrolment's completion date, so a correct certificate earned under a valid approval can be
  reissued after the approval expired.
- **Changed result failing criteria:** not a reissue; the coordinator uses `revoke`.
- **Copy:** regenerates the same PDF (same number and code), marked nowhere; logged in history.
- **KHDA attestation recorded on the old certificate:** copied to the new one only if the
  coordinator confirms.

#### Must not
- Must not edit an issued certificate.
- Must not reuse the old number.

#### Done when
- [ ] Given issued certificate CRT-0101 with name "Ahmad" instead of "Ahmed", when the coordinator
      reissues with reason "name correction" and the corrected name, CRT-0101 is `superseded` and a
      linked draft exists.
- [ ] When the draft is issued as CRT-0150, the public page of CRT-0101 points to CRT-0150 and the
      student is notified.
- [ ] Reissue without a reason is refused; from `revoked` or `superseded` it is not offered.
- [ ] Generating a copy of CRT-0150 leaves its state and number unchanged.

#### Out of scope
`revoke` (base); attendance corrections that trigger it (BL-EDU-14).

---

### BL-EDU-17 — 10 EdTech: configure shared negative-flow settings (documents, scheduling, notifications, alerts, integrations)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` `blocked:open-item` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** BL-DOC-01 · BL-DOC-02 · BL-DOC-06 · BL-SCH-02 · BL-SCH-05 · BL-SCH-07 · BL-PTY-03 · BL-NTF-06 · BL-KRN-05 · BL-KRN-08 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-SAL-22 · BL-SAL-23 · BL-SAL-24 · BL-SAL-25 · BL-SAL-26 · BL-NTF-07 · BL-NTF-08 · BL-NTF-09 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None
**Review items:** X-13, X-17, X-18, X-19, X-20

#### Goal
Seed the 10 starter configuration for the shared mechanisms that are not about fees: document
blocks and overrides, look-ahead windows, one-to-one booking eligibility and conflicts, the block
effect for enrolments, guardian message give-up, compliance-critical rules and the owner, and the
manual fallbacks when an integration is down.
It also seeds the 10 values for lead sources and campaigns (`SAL.CRM.02`), loyalty
(`SAL.LOY.01`), onboarding and SOPs (`HRM.ONB.01`) and registration kinds (`DOC.EXP.01`), added
on 2026-10-16: `SAL.RUL.07`, `SAL.RUL.08`, `SAL.ACL.07`, `NTF.RUL.02`, `NTF.ACL.03`,
`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`.

#### Files to read
- `docs/verticals/10-edtech.md` §4.1 rows "The payer is blocked", "A message to a guardian cannot
  be delivered", §7.1 *Booking* paragraph, §7.2 (intro), §8 (last paragraph), §9
- Configuration tables (§7) of `wl-documents`, `wl-scheduling`, `wl-parties`, `wl-notify`,
  `wl-kernel`
- `docs/modules/wl-sales.md` §4.6, §4.7, §5.8, §5.10 and §7 (campaigns, loyalty, segments);
  `docs/modules/wl-notify.md` §4.3, §5.3 (marketing broadcasts); `docs/modules/wl-hr.md` §5.8
  and §7 (onboarding); `docs/modules/wl-documents.md` §3, §5.7 and §7 (SOPs, registration kinds)
- `docs/OPEN-ITEMS.md` L-19, AU-13, AU-25

#### Terms
- **Mode `block` / `flag`:** refuse the transition / let it run and flag the record (wl-documents §5.6).
- **Override mode:** `never` (locked) or `with_approval` (`DOC.ACL.02`, one use, 72 h).
- **Give-up reaction:** what `NTF.RUL.01` does when a message could not be delivered.
- **Owner:** the business owner role alerted by `PLT.RUL.04`; in 10 the company owner.

#### Spec excerpt (authoritative for this issue)

| Setting | Value (10) | Spec reference |
|---|---|---|
| `blocks` — centre permit (legal entity, campus) | `batch.open`, `certificate.issue` (block); override `never` | §9 |
| `blocks` — course approval (course, where required) | `batch.open`, `certificate.issue` (block), condition: the certificate states it for `certificate.issue`; override `never` | §7.1, §9 |
| `blocks` — police clearance (tutor; courses with minors) | `batch.open`, `batch.change_tutor`, `session.change_tutor`, `session.reschedule`, `booking.book`, `booking.reassign` (block); `session.submit_attendance` (flag); override `never` by default, client may switch to `with_approval` (owner) | §9 |
| `blocks` — MoHRE private tutor permit or freelance visa | as police clearance (block and flag); override `never` | §9 |
| Look-ahead window — tutor kinds | 30 days (`DOC.RUL.02`) | §4.2, §7.2 |
| Look-ahead window — centre permit, course approval | 60 days (`EDU.RUL.13`) | §7.2 |
| One-to-one service types: required tutor kinds | police clearance when the student is a minor; private tutor permit or freelance visa for freelance tutors | §7.1 *Booking* |
| Eligibility look-ahead (`SCH.RUL.05`) | D = 30 days | §7.1 *Booking*, §7.2 |
| Conflict escalation (`SCH.RUL.06`) | H = 2 hours | §7.1 *Booking*, §7.2 |
| Booking deposit | none by default; `SCH.ACL.05` where a service type takes one | §7.1 *Booking* |
| Check-in | not used | §7.1 *Booking* |
| `EDU.RUL.12` | D = 30; H = 24 | §7.2 |
| `EDU.RUL.11` | N = 48 hours | §7.2 |
| Block effect — Enrolment | pause through `freeze` without the freeze-day limit; seat kept; decider centre manager | §4.1; wl-parties §5.6 |
| Block effect — Subscription (membership) | pause | §7.1 *Subscription* |
| Give-up reaction — guardian messages | `alternative_recipient` (the other guardian), then `staff_task` to the counsellor | §4.1, §7.2 |
| Notification type groups (`register_type`) | messages to students and guardians: *Guardian messages*; invoices, membership charges, failed payments, payment links, reminders: *Invoices, statements, payment reminders* (task to the accountant); terms, refund policy and consent requests: *Portal invitations, document requests, signature requests* (task to the counsellor); booking messages keep the wl-scheduling group; staff-only types: no group, `none`; no type statutory | §7.2 *Notification types* |
| Responsible role per document kind | centre permit: centre manager; course approval, police clearance, freelance permit or visa: academic coordinator; for a tutor's document, in the campuses of the affected batches and bookings | §9 table and paragraph; wl-documents §5.2 |
| Compliance-critical rules | `DOC.RUL.03`, `EDU.RUL.12`, `EDU.RUL.13` | §7.2 |
| Owner alert recipients (`PLT.RUL.04`) | company owner + company admin | §7.2 |
| Integration fallback — payment gateway | desk card terminal or cash with a receipt; payment links resent on recovery | §8 |
| Integration fallback — WhatsApp | email | §8 |
| Integration fallback — attendance | paper, entered later by the tutor or coordinator (`EDU.ACL.04`, `EDU.ACL.16`) | §8 |
| Integration fallback — ASP | `FIN.RUL.16` (corporate invoices) | §8 |
| Lead sources used (`SAL.CRM.02`) | referral, walk-in, website, social, paid ads, event (school fairs), partner (schools, corporates), other; recorded on the lead (`SAL.CRM.01`) and on the student or guardian party; no source requires a campaign | §5; §7.2 settings |
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | Instagram, TikTok, search ads, WhatsApp, email, school and community events; converted value: the first enrolment invoice; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.2 settings |
| Loyalty earn and redeem (`SAL.LOY.01`, `SAL.RUL.07`) | `SAL.RUL.07` earns 1 point per AED 10 of fees paid by the student or guardian (none on corporate-paid fees) and reverses in proportion on a refund or credit note; 100 points = an AED 10 discount line on a later enrolment or package, at most 10% of its fee, confirmed by a one-time code to the member (VAT on the discounted supply: `⚠ open: accountant (A-15)`) | §5; §7.2 settings |
| Loyalty expiry, adjustments and segments (`SAL.LOY.01`, `SAL.RUL.08`, `SAL.ACL.07`, `NTF.RUL.02`, `NTF.ACL.03`) | points expire 12 months after they are earned, with a reminder 30 days before (`SAL.RUL.08`); a manual adjustment above 500 points, or closing an account with points at staff's initiative (never at the member's request), needs a manager (`SAL.ACL.07`); segments: package with 2 credits or fewer, course completed without re-enrolment in 30 days, trial attended and not enrolled, guardians with two or more students; campaigns only with marketing consent (`NTF.ACL.01`), the guardian's for a minor, with opt-out; sent 07:00–21:00 and at most 2 marketing messages per recipient in 7 days (`NTF.RUL.02`); a broadcast to more than 200 recipients needs a second approver (`NTF.ACL.03`) | §5; §7.2 settings |
| Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for tutor / trainer, admissions counsellor and academic coordinator, started by `HRM.RUL.16` when the employee record (`HRM.EMP.01`) is created in `onboarding` (none for a record created directly `active`, e.g. by a data import); checklist steps use `WRK.CHK.01`; tasks to the centre manager, the academic coordinator and the employee; the employee's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds the assignees of an overdue step on the first run after its due date, then every 2 days, and tells the centre manager (HR) and the line manager after 5 days; completion blocks nothing (sessions stay guarded by tutor permits and police clearance) | §5; §7.2 settings |
| SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: academic coordinator (teaching, assessment, safeguarding) and centre manager (front desk, refunds); required readers by role per campus; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP's owner role (the SOP owner named above) after 7 days overdue; acknowledged only by the reader (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.2 settings |
| Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Trademark registration* and *Other certification* (subject: legal entity); responsible role company owner; `DOC.RUL.01` at 180, 90 and 30 days for trademarks and 90, 30 and 7 days for the others; no `blocks` | §5; §7.2 settings |

#### Edge cases decided
- **Loyalty VAT:** a redeemed points discount reduces the VAT of the discounted supply only
  (`⚠ open: accountant (A-15)`); the rest of the loyalty settings can be built.
- **Open items:** which kinds are legal blocks (L-19), the licensing body outside Dubai, Abu Dhabi
  and Sharjah (AU-13), running batches after expiry (AU-25) do not change the seeded values; only
  the data changes when answered.
- **Client edits:** may switch police clearance to `with_approval`; may not switch the centre
  permit, course approval or freelance permit.
- **Seeding is idempotent.**

#### Must not
- Must not hard-code these values.

#### Done when
- [ ] **Documents:** the four kinds carry the entries above; switching the centre permit override
      is refused; switching police clearance to `with_approval` with the owner as approver works.
- [ ] **Look-ahead:** a course approval expiring in 60 days produces `DocumentExpiring`; a police
      clearance at 30 days.
- [ ] **Scheduling:** booking a minor's one-to-one with a tutor without clearance is refused; an
      open conflict escalates 2 h before start; no deposit is taken.
- [ ] **Parties:** a blocked payer's enrolment shows effect pause, decider centre manager.
- [ ] **Notifications:** a failed WhatsApp to guardian 1 goes to guardian 2, then a counsellor
      task; for an adult student without a guardian it goes straight to the counsellor task; a
      failed invoice e-mail marks the contact invalid, goes to the other guardian or payer
      contact, then ends in an accountant task.
- [ ] **Notification types:** the registered 10 types carry these groups: session reminder,
      session change, absence alert, tutor change, seat hold and lapse notice, package
      reminder, certificate notice → *Guardian messages*; invoice, membership charge, failed
      payment notice, payment link, overdue reminder → *Invoices, statements, payment
      reminders*; terms, refund policy and guardian consent request → *Portal invitations,
      document requests, signature requests* (task to the counsellor); trial and one-to-one
      booking messages → the wl-scheduling group *Booking confirmations and reminders*;
      staff-only types → no group, `none`; none is marked statutory.
- [ ] **Responsible role:** a centre permit expiring notifies the campus's centre manager; a
      tutor's police clearance expiring notifies the coordinator of each campus where the tutor
      has affected sessions.
- [ ] **Alerts:** a failed `EDU.RUL.12` run alerts the owner and admin.
- [ ] **Integrations:** with the gateway `down`, the desk shows the card-terminal fallback and
      payment links are resent on recovery; with WhatsApp `down`, a session reminder to a
      guardian goes by email; with the ASP `down`, a corporate invoice is still issued and sent
      as PDF (`FIN.RUL.16`); paper attendance entered by the coordinator after the tutor's
      24-hour window needs a reason (`EDU.ACL.16`).
- [ ] **Lead sources:** a new lead or party offers the sources referral, walk-in, website, social,
      paid ads, event, partner and other; a campaign with channel *Instagram*, a budget and cost
      entries reports leads, conversion, cost per lead, CAC and ROAS; a cost entry over the
      budget waits for a manager (`SAL.ACL.07`).
- [ ] **Loyalty:** fees of AED 1,000 paid by a guardian earn 100 points and corporate-paid fees
      earn none; redeeming 100 points adds an AED 10 discount line, refused above 10% of the fee;
      points earned on 2026-11-01 expire on 2027-11-01, with a reminder on 2027-10-02
      (`SAL.RUL.07`, `SAL.RUL.08`); a manual adjustment of 600 points waits for a manager
      (`SAL.ACL.07`); a guardian without marketing consent is left out of a campaign; a
      campaign message queued at 22:00 is deferred to 07:00, a third marketing message in 7
      days is suppressed (`NTF.RUL.02`), and a campaign to 201 guardians needs a second
      approver (`NTF.ACL.03`).
- [ ] **Onboarding and SOPs:** hiring a tutor / trainer creates the tutor / trainer checklist with
      tasks to the centre manager, the academic coordinator and the employee, and the role is active
      before it is complete; a step still open on the first run after its due date reminds its
      assignees, and again 2 days later (`HRM.RUL.17`); a new version of an SOP owned by the
      academic coordinator asks every required reader to acknowledge again within 7 days
      (`DOC.RUL.04`), and no one else can acknowledge for them (`HRM.ACL.10`).
- [ ] **Registrations:** a fresh 10 deployment lists *Trademark registration* and *Other
      certification* with responsible role company owner; a trademark registration expiring in 90
      days notifies the company owner.

#### Out of scope
Fees and payments (BL-EDU-18).

---

### BL-EDU-18 — 10 EdTech: configure shared negative-flow settings (fees, refunds, payments, cash)

**Labels:** `vertical:10-edtech` `area:negative-flow` `priority:high` · **Milestone:** Launch 10 · **Size:** m
**Depends on:** BL-INV-01 · BL-INV-04 · BL-INV-10 · BL-INV-20 · BL-PAY-02 · BL-PAY-04 · BL-PAY-07 · BL-PAY-17 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · **Blocks:** None
**Review items:** X-01, X-02, X-04, X-06, X-07, 10-N3-f

#### Goal
Seed the 10 values for the money mechanisms: refund approval and failed refunds, pending
payments and chargebacks, membership retries and pauses, packages, desk cash, corporate invoice
disputes, e-invoice rejection and write-offs, plus the catalogue defaults for the refund and
absence policies.
It also seeds the 10 values for the cash-flow forecast alert (`FIN.CFL.01`) and the
company budget with owner drawings (`FIN.BUD.01`), added on 2026-10-16: `FIN.RUL.23`,
`FIN.RUL.24` and `FIN.ACL.21`.

#### Files to read
- `docs/verticals/10-edtech.md` §3.3 (`FIN.ACL.04`, `FIN.ACL.06`), §4.3 "When it goes wrong",
  §6 (Catalogue item), §7.1 *Subscription*, §7.2, §9 (refund policy), §13 question 5
- Configuration tables (§7) of `wl-invoicing`, `wl-payments`, `wl-sales`
- `docs/modules/wl-invoicing.md` §5.14–§5.16 (cash-flow forecast, company budget, owner
  drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments)

#### Terms
- **`FIN.ACL.04`:** the user who records a refund cannot approve it.
- **`late_confirm`:** wl-payments transition for a gateway confirmation that arrives after the
  payment was failed. If the payment's targets are still open and its prepaid subject is still
  active, the money is allocated as usual; otherwise it becomes the party's credit and a refund
  is requested automatically. A `lapsed` enrolment is never an active subject, so a late
  payment for it is always refunded (same definition as BL-EDU-01).
- **Catalogue default:** the value a new catalogue item starts with; editable per item.

#### Spec excerpt (authoritative for this issue)

| Setting | Value (10) | Spec reference |
|---|---|---|
| Refund approval (`FIN.ACL.04`) | recorder ≠ approver; always | §3.3, §4.3 |
| Failed refund (`FIN.RUL.21`) | retry once after 24 hours (wl-payments default, N = 24, maximum 1 retry), then a task to the accountant to pay by another method; credit note balance restored by wl-invoicing | §4.3; wl-payments §7 |
| Pending online payment (`FIN.RUL.19`) | module default (30 minutes) | §4.1, §7.2 |
| Late payment after lapse | refunded automatically (`late_confirm`) | §4.1 |
| Chargebacks (`FIN.RUL.20`, `FIN.ACL.18`) | module defaults; course fee → counsellor task; membership → `past_due` | §4.3 |
| Membership retries (`FIN.RUL.06`) | 1, 3, 7 days; `suspend` after the last retry | §7.2 |
| Membership pause and enrolment freeze allowance | 30 days a year, shared (overrides the wl-invoicing default of 0); linked (`EDU.RUL.14`) | §4.3, §6, §7.1 |
| Package refund valuation | unused sessions at the single-session price, without the package discount | §13 q5; wl-sales |
| Package after a reopened invoice | task to collect only (no suspension of consumption) | wl-sales §7 |
| Desk cash (`FIN.CSH.01`) | per counsellor per day, counted by another user | §4.3 |
| Cash shortage write-off (`FIN.ACL.06`) | AED 0 (always approval); salary recovery off | §3.3, §4.3 |
| Corporate invoice dispute from the portal | off until the corporate portal ships | §4.3 |
| Dispute escalation (`FIN.RUL.15`) | module default (14 days) | §7.2 |
| E-invoice rejection (`FIN.RUL.16`) | corporate (B2B) invoices only | §7.2, §9 |
| Bad-debt write-off (`FIN.ACL.15`) | AED 0 (every write-off approved) | §4.3 |
| Refund policy template (catalogue default) | before the start 100% minus the registration fee; after the start unused sessions pro rata minus an admin fee; after 50% no refund | §9, §13 q5 |
| Absence policy (catalogue default) | unexcused absence consumes a credit: yes; notice for an excused absence: 24 h | §6 |
| Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 25,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the company owner and the accountant, checked daily at 06:00; instalments by due date; due dates adjusted by each payer's average days late: on; disputed amounts counted: off | §5; §7.2 settings |
| Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories (tutor costs, rent, marketing, materials, staff) owned by the accountant and mapped to the export accounts; one budget per legal entity and fiscal year, by month, approved by the company owner; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.2 settings |
| Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of operating cost and profit; every drawing approved, whatever its amount (`FIN.ACL.21` limit AED 0), by the company owner, never by the requester (`FIN.ACL.21`) | §5; §7.2 settings |

#### Edge cases decided
- **Pause and freeze allowance:** 30 days per year, one shared value per catalogue item for
  freeze and membership pause; it overrides the wl-invoicing default of 0 days.
- **Admin fee and registration fee amounts:** per catalogue item, no platform default (set at
  onboarding).
- **Seeding is idempotent;** catalogue defaults apply to new catalogue items only.

#### Must not
- Must not allow refunds without a second approver.
- Must not turn on salary recovery for cash shortages.

#### Done when
- [ ] **Refunds:** a refund recorded by A needs B's approval; a gateway refund failing on
      2026-11-10 10:00 is retried at 2026-11-11 10:00; when the retry fails too, the accountant
      gets a task to pay by another method (bank transfer or cash) and records that payment.
- [ ] **Payments:** a gateway payment pending for 30 minutes fails; a late confirmation after
      lapse is refunded automatically.
- [ ] **Memberships:** a failed charge retries on days 1, 3 and 7, then `suspended`; a 30-day
      pause per year is allowed, a 31st day is refused.
- [ ] **Packages:** a refund of 4 unused credits at a single-session price of AED 150 is AED 600.
- [ ] **Cash:** a desk shortage of AED 20 needs approval to write off; salary recovery is not
      offered.
- [ ] **Corporate:** corporate contacts cannot dispute in the portal; a write-off of AED 100 needs
      approval.
- [ ] **Catalogue:** a new course item gets the tiered refund policy and the absence policy above.
- [ ] **Cash-flow forecast:** in a fresh 10 deployment, a projected balance of AED 24,999 in week
      5 alerts the company owner and the accountant once (`FIN.RUL.23`, 06:00 run); AED 25,000
      or more in every week alerts no one; the forecast shows 13 weekly buckets from Monday.
- [ ] **Budget and drawings:** a monthly budget for a legal entity prepared by the accountant
      needs approval by the company owner; an expense with category *owner drawings* is left out
      of operating cost and profit reports, and the user who records it cannot approve it
      (`FIN.ACL.21`); an expense line reaching 100% of its month's budget line warns the
      category's owner role and the accountant once (`FIN.RUL.24`).

#### Out of scope
Documents, scheduling, notifications and integrations (BL-EDU-17); the mechanisms (module epics).
