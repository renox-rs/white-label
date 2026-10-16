# Epic: 02 Construction — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows that the negative-flow review added to
[02 Construction](../../verticals/02-construction.md): project termination with a final account,
overlapping IPAs, typed deductions on certificates, the new retention and guarantee machines,
rejected, omission and time variations with their reminders, absence cases, the subcontractor
registration guard on purchase orders, and 02's values for the shared mechanisms (receipt
rejection, returns to suppliers, PO revision, attendance flags, adjustment timesheets,
absconding, write-off, cheques). Review items: D-04, D-05, 02-N1-a, 02-N2-a, 02-N4-a, 02-N4-b,
02-N5-d, 02-N8-a, 02-N10-a, 02-N12-b, 02-N13-a and the 02 MEDIUM items of review §6.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-CON-01 | Terminate a project mid-way (`terminate`, `CON.ACL.15`) | high | m | Launch 02 | Base: 02 Project lifecycle; BL-KRN-01; BL-CON-07; BL-CON-09 |
| BL-CON-02 | Settle the final account of a terminated project; cancel only before work | high | s | Launch 02 | BL-CON-01; BL-INV-01 |
| BL-CON-03 | Keep a blocked client's project running and refuse new projects for it | high | s | Launch 02 | BL-PTY-03; BL-PTY-04; BL-CON-06; BL-PAY-24 |
| BL-CON-04 | Handle an IPA still waiting for its certificate at the next cut-off (`CON.RUL.03`, `supersede`) | high | m | Launch 02 | Base: 02 IPA lifecycle |
| BL-CON-05 | Record typed deductions on a certificate and accept or contest each (`CON.ACL.14`) | high | m | Launch 02 | Base: 02 IPA lifecycle; BL-KRN-01 |
| BL-CON-06 | Invoice a certificate with accepted and contested deductions | high | m | Launch 02 | BL-CON-05; BL-INV-04; BL-PAY-24; Base: wl-invoicing Credit note, Supplier bill |
| BL-CON-07 | Make a retention part due and invoice it (`make_due`, `invoice`, `settle`, `CON.RUL.07`) | high | m | Launch 02 | Base: 02 IPA `invoice`; Base: wl-invoicing Invoice lifecycle |
| BL-CON-08 | Deduct retention the client keeps (`deduct`) | high | s | Launch 02 | BL-CON-07; BL-CON-05; BL-INV-04 |
| BL-CON-09 | Register, reduce, extend, release and expire a guarantee (`CON.RUL.11`) | medium | m | Launch 02 | Base: wl-documents Document expiry (`DOC.RUL.01`) |
| BL-CON-10 | Record a guarantee call and its settlement | high | s | Launch 02 | BL-CON-09 |
| BL-CON-11 | Resubmit, claim, settle or abandon a rejected variation (`CON.ACL.16`) | high | m | Launch 02 | Base: 02 Variation lifecycle; BL-KRN-01 |
| BL-CON-12 | Price omission and time variations | medium | s | Launch 02 | Base: 02 Variation lifecycle |
| BL-CON-13 | Chase an unanswered variation (`CON.RUL.09`) | medium | s | Launch 02 | Base: 02 Variation lifecycle |
| BL-CON-14 | Warn before a contract notice deadline for a delay event (`CON.RUL.10`) | medium | s | Launch 02 | BL-CON-12 |
| BL-CON-15 | Open an absence case for a worker who stops coming (`CON.RUL.08`) | medium | s | Launch 02 | Base: wl-attendance Attendance record lifecycle; BL-HRM-07; BL-TKT-11 |
| BL-CON-16 | Hold a PO to an unregistered subcontractor on a Dubai project (`CON.ACL.13`) | medium | s | Launch 02 | Base: wl-inventory Purchase order lifecycle; BL-KRN-01 |
| BL-CON-17 | 02: configure shared negative-flow settings — money and procurement | high | m | Launch 02 | BL-INV-01; BL-INV-04; BL-INV-10; BL-INV-13; BL-PAY-09; BL-PAY-24; BL-STK-01; BL-STK-03; BL-STK-07; BL-STK-08; BL-STK-27; BL-CON-03; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26; BL-STK-28; BL-STK-29; BL-STK-30; BL-STK-31 |
| BL-CON-18 | 02: configure shared negative-flow settings — labour and compliance | high | m | Launch 02 | BL-ATT-01 to BL-ATT-08; BL-HRM-01; BL-HRM-04; BL-HRM-07; BL-DOC-02; BL-DOC-04; BL-KRN-04; BL-KRN-08; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |
| BL-CON-19 | Make the client portal read-only after the final account | medium | s | Later | BL-CON-02; Base: wl-portal client portal (`POR.SLF.01`, Later in 02) |

---

### BL-CON-01 — 02 Construction: terminate a project mid-way (`terminate`, `CON.ACL.15`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: 02 Project lifecycle · BL-KRN-01 · BL-CON-07 · BL-CON-09 · Base: wl-inventory Stock count (`STK.CNT.01`), Purchase request and Purchase order lifecycle · **Blocks:** BL-CON-02
**Review items:** 02-N2-a, 02 MEDIUM "site stock counts" (review §6)

#### Goal
When the client terminates the subcontract, the main contractor becomes insolvent, or both sides
agree to stop, the project manager records the termination with the notice and the effective date
and the MD approves. The project moves to `terminated` (not an end): attendance stops after the
effective date, crews are released, purchasing is wound down, the site store is counted, a final
account IPA is drafted, retention becomes due, guarantees are followed up, open snags are cancelled
and the client is told.

#### Files to read
- `docs/verticals/02-construction.md` §4.8, §3.3 `CON.ACL.15`, §7.1 *Project* (states,
  `terminate`, diagram), *Retention* (`make_due`), *Guarantee*, *Purchase request* (`cancel`),
  *Purchase order* (`cancel`, `short_close`), §9 "Termination and guarantees", §13 question 11
- `docs/modules/wl-kernel.md` §4.1 (approval request)
- `docs/OPEN-ITEMS.md` L-37

#### Terms
- **Effective date:** the date the termination takes effect (from the notice); it is the last
  working day on the project.
- **Final account IPA:** an IPA of kind `final_account` valuing work to the effective date,
  materials on site, approved variations, open claims and termination costs.
- **Materials on site:** materials left in the site store, listed for the final account.
- **Termination reason:** `terminated_by_client`, `our_default`, `main_contractor_insolvent`,
  `agreed`.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `terminated` | warning | no | Stopped before completion; final account being agreed |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `terminate` | in_progress, on_hold | terminated | `projects.cancel` | `CON.ACL.15`; reason (terminated by the client, for our default, main contractor insolvent, agreed), termination notice and effective date recorded | the effective date is the last working day: attendance blocked from the day after it (an effective date in the past is allowed: attendance already recorded after it is kept, flagged for the project manager's review and stays costed until accepted or adjusted); crew assignments end on the effective date, so crews work until then (task to reassign created on approval); open purchase requests without a PO cancelled; one task to procurement to cancel or short-close open POs; site store count opened (`STK.CNT.01`); final account IPA drafted (a `draft` IPA of the project becomes it, valued to the effective date; an IPA still `submitted` stays and is certified normally); both retention parts `make_due` unless `retention_due_on_termination` is off; guarantee follow-up tasks; open snags and defects cancelled with the reason; client contact told; event `ProjectTerminated` |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `CON.ACL.15` | Terminating a project mid-way is requested by the project manager and approved by the MD | `SOD` | project manager | on |

§4.8 steps 3–5: "The site store is counted. Each material is transferred to another site or the
central store, returned to the supplier, or listed as *materials on site* for the final account
… Both retention parts become due, unless the contract says otherwise (`⚠ open: lawyer
(L-37)`). … the advance guarantee reduced to the unrecovered advance, the performance bond
released or called."

#### Edge cases decided
- **Approval:** the request stores the reason, notice document and effective date; the project
  keeps its state until the MD approves (BL-KRN-01). The MD cannot be the requester.
- **Effective date in the past:** allowed (notices arrive late). Attendance already recorded after the effective date is not deleted; it is flagged for review by the project
  manager and stays costed until accepted or adjusted.
- **Effective date in the future:** the transition runs on approval; the crew assignments get
  the effective date as their end date (not ended at once), and the reassign task is created on
  approval. Attendance is accepted up to and including the effective date and refused from the
  day after, so crews work until then.
- **"Unless the contract says otherwise":** the project field `retention_due_on_termination`
  (default yes, §6). With *no*, retention parts stay `held`. The legal
  default is open (L-37, `blocked:open-item`).
- **Purchase requests:** requests in `draft`, `submitted` or `approved` with no PO issued are
  cancelled; requests with some lines on POs are listed in the PO task.
- **Open POs:** not cancelled automatically (the supplier must be informed); procurement gets one
  task listing them, each to `cancel` (nothing received) or `short_close`.
- **Existing IPA draft:** a `draft` IPA of the project is turned into the
  final account IPA (kind changed, valuation date = effective date); an IPA still `submitted` stays
  and its certificate is recorded normally.
- **Guarantee tasks:** one task per `active` guarantee to the accountant: advance → reduce to the
  unrecovered advance; performance → release or respond to a call; retention bond → release.
- **Snags and defects:** open tickets are cancelled with the reason "project terminated".
- **`CON.RUL.03`:** stops creating monthly drafts (the project is no longer `in_progress`).
- **Client told:** an e-mail to the client contacts with the effective date (portal is Later).

#### Must not
- Must not delete attendance, receipts or IPAs.
- Must not cancel POs automatically.
- Must not make the project terminal; `settle_final_account` closes it (BL-CON-02).

#### Done when
- [ ] Given an `in_progress` project, when project manager A requests `terminate` with reason
      `main_contractor_insolvent`, the notice and effective date 2027-05-31, then an approval
      request goes to the MD and the project stays `in_progress`.
- [ ] A cannot approve. When the MD approves, the project is `terminated` and `ProjectTerminated`
      is emitted.
- [ ] Attendance for 2027-06-01 on the project is refused; attendance for 2027-05-31 is accepted.
- [ ] With the MD approving on 2027-05-20 (effective date 2027-05-31), the crew assignments show
      end date 2027-05-31 and stay valid until then; the project manager has the reassign task
      from 2027-05-20; attendance for 2027-05-25 is accepted.
- [ ] 2 open purchase requests without POs are `cancelled`; procurement has one task listing the
      3 open POs; a stock count is open for the site store.
- [ ] A final account IPA in `draft` exists; both retention parts of AED 50,000 each are `due`;
      with `retention_due_on_termination = no` they stay `held`.
- [ ] The accountant has one task per active guarantee; 4 open snag tickets are `cancelled` with
      the reason; the client contacts get the e-mail.

#### Out of scope
Closing the terminated project (BL-CON-02); retention invoicing (BL-CON-07); guarantee
transitions (BL-CON-09, BL-CON-10); returns to suppliers (Epic wl-inventory: BL-STK-03).

---

### BL-CON-02 — 02 Construction: settle the final account of a terminated project; cancel only before work

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `blocked:open-item` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-CON-01 · BL-INV-01 · **Blocks:** BL-CON-19
**Review items:** 02-N2-a

#### Goal
A terminated project closes when its final account is certified (or decided by a court or
arbitration) and the final account IPA is invoiced or its balance written off. `cancel` is now
only possible from `setup`, before any attendance, goods receipt or IPA, so a project with work
done always ends through `terminate`.

#### Files to read
- `docs/verticals/02-construction.md` §4.8 steps 6–7, §7.1 *Project* (`settle_final_account`,
  `cancel`, `closed` meaning), §9 "Termination and guarantees"
- `docs/OPEN-ITEMS.md` L-37

#### Terms
- **Final account certified:** the client's certificate for the final account IPA recorded
  (`certify`).
- **Decision recorded:** a court judgment or arbitration award attached with the decided amount.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `closed` | success | yes | Defects liability period ended and final retention invoiced, or the final account after a termination agreed and invoiced |
| `cancelled` | neutral | yes | Cancelled before work started |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `settle_final_account` | terminated | closed | `projects.manage` | final account certified, or decided by a court or arbitration and recorded (as the final account IPA's certificate, with the judgment attached); final account IPA invoiced, or its balance written off; open retention parts, guarantees and a site store count still open are warnings, not guards (they keep their own lifecycles) | client portal read-only (`POR.ACL.01`) |
| `close` | practical_completion | closed | `projects.manage` | defects liability end date passed; no open defect tickets; a site store count still open is a warning, not a guard | final retention part `make_due`; performance bond release task (one per `active` performance or retention bond) |
| `cancel` | setup | cancelled | `projects.cancel` (MD) | reason given; no attendance, goods receipt or IPA recorded | open POs flagged for cancellation |

§4.8 step 7: "If the main contractor is insolvent, open invoices are disputed or written off with
the MD's approval; filing a claim with the liquidator is outside the platform (`⚠ open: lawyer
(L-37)`)."

#### Edge cases decided
- **Court decision:** recorded on the final account IPA as a certificate with the decided amount and
  the judgment attached; then invoiced as usual.
- **Balance written off:** the final account invoice is `written_off` (`FIN.ACL.15`, MD); this
  satisfies the guard.
- **Retention and guarantees still open:** not a guard; retention parts and
  guarantees keep their own lifecycles after `closed`, and the closing screen warns about them.
- **Stock count still open:** a warning, not a guard (§4.3 step 8; `close` and
  `settle_final_account` guards).
- **Cancel with POs:** POs are flagged, not cancelled.
- **Portal read-only:** the client portal is Later in 02 (BL-CON-19).

#### Must not
- Must not cancel a project that has attendance, a goods receipt or an IPA.
- Must not close a terminated project before its final account is invoiced or written off.

#### Done when
- [ ] Given a `terminated` project whose final account IPA is `invoiced`, `settle_final_account`
      makes it `closed`.
- [ ] With the final account IPA only `certified`, it is refused.
- [ ] With the final account invoice `written_off` after MD approval, it is allowed.
- [ ] Given a `setup` project with one goods receipt, `cancel` is refused; without any, the MD
      cancels it with a reason and its 2 POs are flagged for cancellation.
- [ ] `cancel` from `in_progress` is not offered.
- [ ] `settle_final_account` with the site store count still open succeeds after a warning;
      `close` of a completed project with an open count also only warns.

#### Out of scope
The termination (BL-CON-01); write-off mechanics (BL-INV-01); the portal (BL-CON-19).

---

### BL-CON-03 — 02 Construction: keep a blocked client's project running and refuse new projects for it

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-PTY-03 · BL-PTY-04 · BL-CON-06 (supplier bill from the client and its set-off) · BL-PAY-24 (`set_off` with `BlockGuard`) · **Blocks:** BL-CON-17
**Review items:** X-18

#### Goal
While a client is blocked, its running projects continue (block effect `continue`, decided by the
MD): work, IPAs and variations go on. No new project is started for it, and no money goes out to
it unless an exception is approved. Stopping a project is the MD's decision through `terminate`.

#### Files to read
- `docs/verticals/02-construction.md` §7.2 "Negative-flow settings" (row Project), §7.1
  *Quotation* (`accept`), *Project*
- `docs/modules/wl-parties.md` §5.6 (agreement kind Project)

#### Terms
- **Client:** the party the project's IPAs are invoiced to (client or main contractor).
- **Money out to the client:** a refund, or a set-off of a supplier bill from the client.

#### Spec excerpt (authoritative for this issue)

Negative-flow settings, row Project, column Party blocked: "client blocked: block effect
`continue` (decided by the MD); IPAs and variations still prepared; quotations may still be sent,
but `accept` is refused, so no new project starts; money out to the client stops (`PTY.ACL.03`),
including a `set_off` of a supplier bill from the client, which waits with the IPA invoice open
for that amount".

wl-parties §5.6: "Project | 02 vertical | continue: work, IPAs and variations go on; no new
project is started for the party; money out stops (`PTY.ACL.03`) | the vertical's decider (02:
MD), through the project's `terminate`".

| Transition (Quotation) | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `accept` | sent | accepted | `quotations.accept` | award document (LPO) attached | event `QuotationAccepted` → project created |

#### Edge cases decided
- **Where the guard runs:** quotation `accept` (action kind `start`) is refused for a blocked
  party, so no project is created. A quotation can still be prepared and sent (sending is not
  starting an agreement; settings table row Project).
- **Set-off of a supplier bill from the client** (BL-CON-06): a payment of method `set_off`
  (wl-payments, BL-PAY-24) is money out (`BlockGuard::check(party, money_out)`), so it is
  refused until unblock or an approved exception; the IPA invoice stays open for that amount
  meanwhile. This issue needs BL-CON-06 (which creates the bill) and BL-PAY-24 (which runs the
  guard) to be testable; it adds only the 02 test.
- **Receipts from the client:** recorded normally.
- **No review task:** effect `continue` creates none; the MD is notified once per block.
- **Refusal text:** neutral for users without `PTY.ACL.02`.

#### Must not
- Must not hold or terminate the project automatically.
- Must not show a restricted reason.

#### Done when
- [ ] Given client C blocked, accepting a quotation for C is refused with the neutral text.
- [ ] C's `in_progress` project still accepts attendance, IPA submission and variation approval.
- [ ] A set-off of a supplier bill from C waits for an exception approved by the compliance role.
- [ ] A cheque received from C is recorded and cleared normally.

#### Out of scope
The guard and exceptions (BL-PTY-04); the effect registration (BL-PTY-03).

---

### BL-CON-04 — 02 Construction: handle an IPA still waiting for its certificate at the next cut-off (`CON.RUL.03`, `supersede`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: 02 IPA lifecycle · **Blocks:** None.
**Review items:** D-05, 02-N13-a

#### Goal
Clients often certify late. At the next cut-off the monthly draft IPA is still created, marked
*previous pending*, and "previous certified" is always the latest IPA that is `certified` or
`invoiced`. The new IPA can be submitted only as a replacement of the pending one, which becomes
`superseded` (the client is told); a certificate that arrives later for the superseded IPA is
recorded on the one that replaced it.

#### Files to read
- `docs/verticals/02-construction.md` §4.4 step 3 and row "The previous IPA is not certified at
  the next cut-off", §7.1 *IPA* (states, `submit`, `supersede`, `revise`, paragraph "Previous
  certified"), §7.2 `CON.RUL.03`, `CON.RUL.04`, §3.3 `CON.ACL.05`

#### Terms
- **IPA:** interim payment application; cumulative (each IPA values all work to date).
- **Previous certified:** the cumulative certified amount of the latest IPA in `certified` or
  `invoiced`.
- **Pending IPA:** an IPA in `submitted` with no certificate yet.
- **Replacement:** a new IPA whose field `replaces` points to the pending one.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `superseded` | neutral | yes | Replaced by a new revision, or by a later IPA, before certification |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | submitted | `ipas.submit` (project manager) | progress recorded for the period; no earlier IPA of the project still `submitted`, unless this one replaces it | PDF with photos generated; a replaced IPA `supersede`, and the client told |
| `supersede` | submitted | superseded | system (a later IPA `submit` that replaces it) | — | a certificate arriving later for it is recorded on the replacing IPA; when that one is already `certified`, it is attached for reference only, no amounts change, and the QS is told |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CON.RUL.03` | schedule: monthly on cut-off day D (the last day of the month when D does not exist in it) | project `in_progress`; no IPA of the project in `draft` | create a draft IPA; when an earlier IPA is still `submitted`, mark the draft *previous pending* and notify the QS and project manager; otherwise notify the QS | D = 25 | on | MVP |
| `CON.RUL.04` | N days after IPA → `submitted` | not `certified` | notify QS and project manager | N = 14 | on | MVP |

"*Previous certified* in the IPA calculation is the cumulative certified amount of the latest IPA
in `certified` or `invoiced`."

#### Edge cases decided
- **Latest:** by IPA sequence number within the project (not by date entered).
- **Replacement is explicit:** when an earlier IPA is `submitted`, the submit screen requires
  choosing "replaces IPA n"; only the most recent pending IPA can be replaced. Submitting without
  it is refused with the reason.
- **Late certificate for a superseded IPA:** recorded as the `certify` of the replacing IPA when
  that one is still `submitted` (the certified amount is what the client certified; the gap to the
  replacing claim is tracked as usual). If the replacing IPA is already `certified`, the late
  certificate is attached to it for reference only and the QS is told; no amounts change
  (`supersede` effects).
- **Previous pending flag:** cleared on the draft when the pending IPA is certified before the
  draft is submitted; then the draft submits normally and its previous certified updates.
- **Calculation refresh:** the draft's "previous certified" is recomputed whenever an earlier IPA
  is certified.
- **Cut-off on a non-existent day** (D = 31 in February): the last day of the month
  (`CON.RUL.03`). Run at 02:00 Asia/Dubai (§7.2 intro).
- **Idempotency:** one draft per project per cut-off; a re-run finds the draft and does nothing.

#### Must not
- Must not count a `submitted` IPA in previous certified.
- Must not let two IPAs of a project be `submitted` at once.
- Must not change a submitted IPA (`CON.ACL.05`); revise or replace it.

#### Done when
- [ ] Given IPA 4 `invoiced` (cumulative certified AED 400,000) and IPA 5 `submitted` on
      2027-01-26 without certificate, on 2027-02-25 `CON.RUL.03` creates draft IPA 6 marked
      *previous pending*, notifies the QS and project manager, and its previous certified is AED
      400,000.
- [ ] Submitting IPA 6 without "replaces IPA 5" is refused; with it, IPA 5 is `superseded`, the
      client is told, IPA 6 is `submitted`.
- [ ] The client's certificate for IPA 5 (AED 520,000 cumulative) arriving on 2027-03-05 is
      recorded as IPA 6's certificate; IPA 6's claim gap is shown.
- [ ] If IPA 5 is certified on 2027-03-01 before IPA 6 is submitted, the *previous pending* flag
      clears and IPA 6's previous certified becomes IPA 5's amount.
- [ ] A second run of `CON.RUL.03` on 2027-02-25 creates nothing.

#### Out of scope
Certificate deductions (BL-CON-05); invoicing (BL-CON-06).

---

### BL-CON-05 — 02 Construction: record typed deductions on a certificate and accept or contest each (`CON.ACL.14`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: 02 IPA lifecycle · BL-KRN-01 · **Blocks:** BL-CON-06, BL-CON-08
**Review items:** 02-N4-a

#### Goal
Clients deduct back-charges, contra-charges and liquidated damages from certificates. When the
certificate is recorded, each deduction is a typed line with its reference. The QS records the
lines; the project manager accepts or contests each one, and accepting one above AED 25,000 needs
the MD. The IPA is certified only when every line is typed and accepted or contested; contested
lines are listed for the final account.

#### Files to read
- `docs/verticals/02-construction.md` §4.4 step 5 and row "The client deducts back-charges,
  contra-charges or liquidated damages", §3.3 `CON.ACL.14`, §6 *IPA* (deduction lines), §7.1 *IPA*
  (`certify`)

#### Terms
- **Back-charge / contra-charge:** an amount the client charges for goods, services or labour it
  supplied (e.g. cleaning, scaffolding).
- **Liquidated damages (LD):** an agreed amount for late completion.
- **Contested:** the project manager does not accept the line; it stays on the IPA as a claim.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `certify` | submitted | certified | `ipas.certify` | certified amount (the work, before deductions) and certificate attached; each deduction line typed and accepted or contested (`CON.ACL.14`); lines editable until `certify` | difference from claim recorded; contested deductions listed for the final account |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `CON.ACL.14` | Deductions on a certificate (back-charge, contra-charge, liquidated damages) and deductions from retention are recorded by the QS; the project manager accepts or disputes each one (never the user who recorded it); accepting one above a value, checked per line excluding VAT, needs the MD | `LIM`, `SOD` | QS, project manager | AED 25,000 |

Deduction line fields (§6): type (`back_charge`, `contra_charge`, `liquidated_damages`, `other`),
reference, amount, accepted or contested.

#### Edge cases decided
- **Certified amount:** recorded as the amount the client certifies for the work before the
  typed deductions (§4.4 step 5); the deductions are separate lines. The difference from the
  claim compares claim and certified work, not the net paid.
- **Limit per line:** AED 25,000 is checked per line, excluding VAT (`CON.ACL.14`).
  Accepting a line of AED 30,000 creates an approval request to the MD; the IPA cannot be certified
  until it is approved, or the line is contested instead. On reject or return the line goes back to
  *undecided* (BL-KRN-01).
- **SoD:** the user who recorded a line cannot accept or contest it.
- **Line type `other`:** needs a description; it is treated like a price reduction (credit note)
  when accepted (§4.4 row; IPA `invoice` effects).
- **Corrections:** lines are editable until `certify`; after that, only through the final account.
- **No deductions:** `certify` needs only the amount and certificate.

#### Must not
- Must not certify with an undecided line.
- Must not let the QS accept their own line.
- Must not net deductions into the certified work amount.

#### Done when
- [ ] Given IPA 6 `submitted`, the QS records a certificate of AED 180,000 with lines back-charge
      AED 5,000 (ref BC-12), LD AED 30,000 (ref LD-1) and contra-charge AED 12,000 (ref CC-3).
- [ ] `certify` is refused while any line is undecided.
- [ ] The project manager accepts BC-12 directly; accepting LD-1 creates an MD approval request;
      the project manager contests CC-3.
- [ ] After the MD approves, `certify` succeeds; the contested CC-3 is listed in the project's
      final account items.
- [ ] The QS cannot accept the lines they recorded.

#### Out of scope
What invoicing does with the lines (BL-CON-06); retention deductions (BL-CON-08).

---

### BL-CON-06 — 02 Construction: invoice a certificate with accepted and contested deductions

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-CON-05 · BL-INV-04 · BL-PAY-24 (payment method `set_off`) · Base: wl-invoicing Credit note, Supplier bill · **Blocks:** BL-CON-03
**Review items:** 02-N4-a

#### Goal
When a certified IPA is invoiced, the tax invoice covers the certified work. Accepted back- and
contra-charges become a supplier bill from the client, set off against the invoice; accepted
liquidated damages and price reductions become a credit note against the invoice; the unpaid part
for contested deductions is flagged `disputed`. Retention and advance recovery are booked as
before.

#### Files to read
- `docs/verticals/02-construction.md` §4.4 steps 5–6 and row "The client deducts…", §7.1 *IPA*
  (`invoice`), §9 "Certificate deductions"
- `docs/modules/wl-invoicing.md` §4 *Invoice* (`dispute`), *Credit note*, *Supplier bill*
- `docs/modules/wl-payments.md` §4 *Payment* (paragraph "Set-off"), §7 "Payment methods enabled"
  (set-off on for 02)
- `docs/OPEN-ITEMS.md` A-07

#### Terms
- **Set-off:** settling our invoice to the client and the client's bill to us against each other
  for the same amount, without money moving.
- **Price reduction:** a deduction that lowers the price of our supply (credit note).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `invoice` | certified | invoiced | `invoices.issue` | — | tax invoice created for the certified work (`FIN.INV.01`); retention added to the held retention parts; advance recovery booked; accepted back- and contra-charges recorded as a supplier bill from the client (no PO match; approved by the accountant) and, once it is approved, set off by the accountant with a payment of method `set_off` (wl-payments); accepted liquidated damages, price reductions and `other` lines issued as a credit note against the invoice; the unpaid part for contested deductions flagged `disputed` |

§9: "back- and contra-charges for goods, services or labour the client supplied are the client's
supplies to us, recorded as supplier bills and set off; liquidated damages and agreed price
reductions are credit notes against the IPA invoice by default. Whether liquidated damages reduce
the price or are compensation outside the scope of VAT is `⚠ open: accountant (A-07)`."

#### Edge cases decided
- **Supplier bill from the client:** one bill per IPA with one line per accepted back- or
  contra-charge, supplier = the client party, VAT as on the client's document (5% by default);
  it skips the 3-way match (no PO) and is approved by the accountant.
- **Set-off mechanism:** a payment of method `set_off` (wl-payments, BL-PAY-24) for the bill
  amount, allocated to the IPA invoice and to the supplier bill at once; it moves no money and
  needs no bank account. The method is enabled in the 02 seed (BL-PAY-24 setting "Payment
  methods enabled": set-off on for 02). Because the supplier bill must be `approved` first, the
  order is: supplier bill created and approved, then the set-off. A correction is only `reverse`
  of the set-off (reason *entered in error*), which reopens both documents.
- **LD credit note:** VAT on the credit note follows the invoice line (5%) by default; the A-07
  answer may change this (`blocked:open-item`). Each accepted `liquidated_damages` or `other` line
  is a credit note line with its reference.
- **Contested part:** the invoice is put in dispute (`dispute`, reason *contested certificate
  deduction*) with disputed amount = the contested lines plus their VAT; this pauses reminders on
  the invoice (module behaviour).
- **Order:** in one transaction on `invoice`: the tax invoice is issued, then the credit note,
  then the dispute flag, and the supplier bill from the client is created and submitted. The
  set-off is a second step: the accountant approves the supplier bill (a task), then records the
  set-off from it; until then the invoice balance includes the back-charge amount.
- **Rounding:** VAT per line, half up to 0.01 AED.
- **Blocked client:** the set-off waits (BL-CON-03).

#### Must not
- Must not reduce the tax invoice amount by the deductions.
- Must not issue a credit note for back- or contra-charges.
- Must not issue a tax document for retention here.

#### Done when
- [ ] Given IPA 6 certified with work AED 180,000, accepted back-charge AED 5,000, accepted LD AED
      30,000 and contested contra-charge AED 12,000, `invoice` issues a tax invoice of AED
      180,000 + VAT 9,000.
- [ ] A supplier bill from the client of AED 5,000 + 250 VAT is created and the accountant has a
      task to approve it; after approval, the set-off of AED 5,250 drops the invoice balance by
      AED 5,250 and the bill is `paid`, with no bank movement and no bank account set.
- [ ] `reverse` of that set-off with reason *entered in error* reopens the invoice balance and the
      supplier bill.
- [ ] A credit note of AED 30,000 + 1,500 VAT is issued against the invoice.
- [ ] The invoice is `disputed` with disputed amount AED 12,600; reminders do not run for it.
- [ ] Retention for the period is added to the held parts; no retention invoice exists.

#### Out of scope
Recording the lines (BL-CON-05); dispute resolution (BL-INV-04); retention (BL-CON-07).

---

### BL-CON-07 — 02 Construction: make a retention part due and invoice it (`make_due`, `invoice`, `settle`, `CON.RUL.07`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: 02 IPA `invoice` · Base: wl-invoicing Invoice lifecycle · BL-CON-09 · **Blocks:** BL-CON-01, BL-CON-08
**Review items:** D-04, 02-N4-b

#### Goal
Retention withheld from certified IPAs is not invoiced on the IPA invoice. Each release part
(first, final) is a record that is `held` until its release condition is met, then `due`, then
invoiced on its own 5% tax invoice, and `settled` when that invoice is paid, credited or written
off. The accountant is warned before VAT on retention becomes due at the one-year limit.

#### Files to read
- `docs/verticals/02-construction.md` §4.6 steps 3–6 and rows "The client disputes or delays the
  retention release" and "Retention is replaced by a bond", §3.3 `CON.ACL.11`, §6 *Retention*,
  §7.1 *Retention*, *Project* (`complete`, `close`, `terminate`), §7.2 `CON.RUL.07`, §9 "VAT"
- `docs/OPEN-ITEMS.md` A-13, L-37

#### Terms
- **Release part:** `first` (due at practical completion) or `final` (due at the end of the
  defects liability period), split per the project's release split (default 50:50).
- **Retention bond:** a guarantee of type *retention bond* that replaces cash retention.
- **One-year limit:** VAT on a supply becomes due at the latest one year after the work; here
  counted from the completion certificate.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `held` | info | no | Withheld on certified IPAs; no tax document yet |
| `due` | warning | no | Release condition met; to be invoiced |
| `invoiced` | info | no | Retention tax invoice issued; follows the invoice lifecycle |
| `settled` | success | yes | Retention invoice paid, credited or written off |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `make_due` | held | due | system (project `complete` for the first part; `close` or the end of the defects liability period for the final part; `terminate` for both) or `retentions.manage` (replaced by a retention bond) | `CON.ACL.11`; for a bond, a guarantee of type *retention bond* `active` and linked to the part (a bond amount below the part's held amount is a warning) | accountant notified |
| `invoice` | due | invoiced | `invoices.issue` | `CON.ACL.11` | retention tax invoice at 5% for the remaining amount (`FIN.INV.01`); `CON.RUL.07` stops |
| `settle` | invoiced | settled | system (retention invoice `paid`, `credited` or `written_off`) | — | a deduction made after invoicing goes through invoice `dispute` and ends as a credit note (outcome `credited`); `settled` stays terminal if the invoice is later reopened, and the retention report reads the invoice's state |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `CON.ACL.11` | A retention release is invoiced only after practical completion (first part) or the end of the defects liability period (rest) is recorded, per the project's release split; a part is also invoiced once an `active` retention bond linked to it replaces it, or once the project's termination effective date is reached (both parts, unless `retention_due_on_termination` is off) | `STA` | accountant | 50:50 |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CON.RUL.07` | N days before 12 months after the completion certificate, or after the termination effective date for a terminated project | a retention part `held` or `due` | notify the accountant that VAT on the retention becomes due; the accountant may dismiss the notice with a reason, which changes nothing else | N = 30 | on | MVP |

Project `complete` effect: "first retention part `make_due`"; `close` effect: "final retention part
`make_due`". §4.6: "The part stays `due`; the accountant still issues the retention invoice before
the one-year limit (`CON.RUL.07`) unless the accountant decides otherwise".

#### Edge cases decided
- **Amounts:** each IPA `invoice` adds retention to the parts per the release split (AED 10,000
  retained → AED 5,000 to each part at 50:50), up to the project's retention cap; rounding half up
  to 0.01 AED, the remainder cent to the final part.
- **Cap reached** (§7.1 *Retention* intro): the cap is an amount in AED on the project (empty =
  no cap). The IPA whose retention would pass the cap withholds only the rest up to the cap;
  later IPAs withhold nothing and their invoice has no retention line. The rest withheld is split
  per the release split like any amount, so at the cap each part holds cap × its share. The
  accountant is notified once when the cap is reached.
- **Bond and termination (`CON.ACL.11`):** an active retention bond linked to the part, or the
  project's termination effective date (unless `retention_due_on_termination` is off), also
  allows invoicing that part.
- **Bond:** the user links the bond to one or both parts; a warning (not a refusal) when the bond
  amount is below the part's held amount (`make_due` guard).
- **`CON.RUL.07` without a completion certificate** (terminated project): counted from the
  termination effective date (`CON.RUL.07`).
- **Accountant decides otherwise:** the accountant may dismiss the `CON.RUL.07` notice with a
  reason; nothing else changes. Whether VAT is due at the limit on
  retention the client keeps is `⚠ open: accountant (A-13)` (`blocked:open-item`).
- **Partial credit after dispute:** the part becomes `settled` when its invoice reaches `paid`,
  `credited` or `written_off`; a partial credit note plus payment ends in `paid`.
- **Retention invoice reopened** (payment reversed): `settled` is terminal (`settle` effects);
  the invoice follows its own lifecycle and the retention report reads the invoice state.
- **Run time:** daily at 02:00 Asia/Dubai (§7.2 intro).

#### Must not
- Must not issue any tax document for a part while `held` or `due`.
- Must not invoice a part before its release condition is recorded.

#### Done when
- [ ] Given IPAs that withheld AED 100,000 at 50:50, project `complete` with the handover
      certificate dated 2027-06-15 makes the first part (AED 50,000) `due` and notifies the
      accountant.
- [ ] `invoice` on the first part issues AED 50,000 + VAT 2,500 and the part is `invoiced`; on the
      final part (still `held`) it is not offered.
- [ ] When the retention invoice is paid, the part is `settled`.
- [ ] Given retention 10%, cap AED 60,000 at 50:50 and AED 55,000 already withheld, an IPA
      certifying AED 100,000 of work withholds AED 5,000 (not 10,000): each part gets AED 2,500
      and holds AED 30,000; the accountant is notified that the cap is reached; the next IPA
      withholds nothing.
- [ ] With the final part still `held`, the accountant is notified on 2028-05-16.
- [ ] A retention bond `active` linked to the final part lets `retentions.manage` make it `due` and
      invoice it before the defects liability period ends.
- [ ] On a project terminated with effective date 2027-05-31, both parts are `due` and can be
      invoiced; `CON.RUL.07` notifies the accountant on 2028-05-01.

#### Out of scope
Retention deductions (BL-CON-08); the bond's own lifecycle (BL-CON-09); termination (BL-CON-01).

---

### BL-CON-08 — 02 Construction: deduct retention the client keeps (`deduct`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `blocked:open-item` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-CON-07 · BL-CON-05 · BL-INV-04 · **Blocks:** None.
**Review items:** D-04, 02-N4-b

#### Goal
The client may keep part or all of the retention, for example for defects it fixed itself. Before
the part is invoiced, `deduct` reduces it with the client's notice, books the amount to the
project's back-charge cost code and issues no tax document; when nothing remains the part is
`kept`. After invoicing, the reduction is a dispute on the retention invoice ending in a credit
note.

#### Files to read
- `docs/verticals/02-construction.md` §4.6 row "The client keeps part or all of the retention",
  §7.1 *Retention* (`deduct`, `kept`, `settle`), §3.3 `CON.ACL.14`, §9 "VAT"
- `docs/OPEN-ITEMS.md` A-13

#### Terms
- **Back-charge cost code:** the project cost code for amounts the client charges us.
- **Kept:** the client keeps the whole part; terminal.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `kept` | neutral | yes | Kept in full by the client before it was invoiced |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `deduct` | held, due | held, due, kept | `retentions.manage` | reason and the client's notice or contra-charge reference; `CON.ACL.14` | amount reduced; deducted amount booked to the project's back-charge cost code; no tax document; `kept` when nothing remains |

§4.6 row: "After it was invoiced: the invoice is disputed and the agreed amount is a credit note
(`credited`). Each deduction is accepted or disputed by the project manager (`CON.ACL.14`)."

#### Edge cases decided
- **`CON.ACL.14` applied:** the QS records the deduction; the project manager accepts it (above
  AED 25,000 the MD) or contests it. A contested deduction is not applied; it is listed for the
  final account.
- **Amount:** cannot exceed the part's remaining amount; equal → `kept`.
- **State after deduct:** stays `held` or `due` when an amount remains.
- **After invoicing:** `deduct` is not offered on `invoiced`; the user opens invoice `dispute`
  on the retention invoice.
- **VAT:** no tax document while not invoiced (spec); whether VAT is still due at the one-year
  limit on a kept part is `⚠ open: accountant (A-13)` (`blocked:open-item`); `CON.RUL.07` still
  warns while any amount is `held` or `due`.

#### Must not
- Must not issue a credit note for a deduction before the part is invoiced.
- Must not apply a deduction before it is accepted.

#### Done when
- [ ] Given the final part `held` at AED 50,000, a deduction of AED 8,000 with the client's notice,
      accepted by the project manager, leaves AED 42,000 `held` and books AED 8,000 to the
      back-charge cost code; no tax document exists.
- [ ] A deduction of AED 42,000 accepted by the MD (above AED 25,000) makes the part `kept`.
- [ ] On an `invoiced` part, `deduct` is not offered; disputing the retention invoice and resolving
      it `credited` for AED 8,000 issues a credit note of AED 8,000 + VAT 400.

#### Out of scope
Due and invoicing (BL-CON-07); invoice disputes (BL-INV-04).

---

### BL-CON-09 — 02 Construction: register, reduce, extend, release and expire a guarantee (`CON.RUL.11`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:medium` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: wl-documents Document expiry (`DOC.RUL.01`) · **Blocks:** BL-CON-01, BL-CON-07, BL-CON-10
**Review items:** 02 MEDIUM "guarantee lifecycle" (review §6), 02-N12-b

#### Goal
Advance guarantees, performance bonds and retention bonds get a lifecycle with their bank
documents. A guarantee is issued, reduced (the advance guarantee never below the unrecovered
advance), extended, released, or expires. 60 days before expiry, if the project has not reached the
guarantee's release point, the accountant gets a task to ask the bank for an extension, because the
client may call a guarantee that is not extended.

#### Files to read
- `docs/verticals/02-construction.md` §2 (advance payments, performance bond), §4.6 rows "A
  guarantee is about to expire before it may be released", "Retention is replaced by a bond",
  §6 *Guarantee*, §7.1 *Guarantee*, *Project* (`close`), §7.2 `CON.RUL.11`

#### Terms
- **Beneficiary:** the client or main contractor who holds the guarantee.
- **Unrecovered advance:** advance paid − advance recovered on invoiced IPAs.
- **Release point:** completion for the advance guarantee; end of the defects liability period for
  performance and retention bonds (see edge cases).

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Requested from the bank |
| `active` | success | no | Issued and held by the beneficiary |
| `released` | success | yes | Returned, cancelled by the bank, or ended after a call was paid |
| `expired` | neutral | yes | Expiry date passed without extension |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `issue` | draft | active | `guarantees.manage` | bank document attached; type, amount, beneficiary and expiry set | expiry reminders (`DOC.RUL.01`, `CON.RUL.11`) |
| `reduce` | active | active | `guarantees.manage` | bank amendment or the beneficiary's consent attached; advance guarantee: not below the unrecovered advance | amount updated |
| `extend` | active | active | `guarantees.manage` | bank amendment with the new expiry attached | reminders reset |
| `release` | active | released | `guarantees.manage` | original returned or the bank's cancellation letter attached | — |
| `expire` | active | expired | system | expiry date passed; no extension recorded | accountant and project manager notified |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CON.RUL.11` | N days before a guarantee's expiry | guarantee `active`; the project has not reached the guarantee's release point (completion for the advance guarantee, end of the defects liability period for performance and retention bonds) | task to the accountant to request an extension from the bank; notify the project manager | N = 60 | on | MVP |

Project `close` effect: "performance bond release task".

#### Edge cases decided
- **Performance bond release point:** the end of the defects liability period (`CON.RUL.11`;
  §4.6 step 6 releases the performance bond then, and project `close` creates its release task).
- **Expiry day:** `expire` runs the day after the expiry date at 02:00 Asia/Dubai (§7.2
  intro).
- **Extend:** the new expiry must be later than the current one; `CON.RUL.11` and `DOC.RUL.01`
  restart from it.
- **Reduce below the unrecovered advance:** refused with the current unrecovered amount shown.
- **Release task on `close`:** one task to the accountant per `active` performance or retention
  bond.
- **Documents:** each transition stores its bank document as a document of the guarantee.

#### Must not
- Must not change amount or expiry without the bank document.
- Must not let a guarantee expire silently: the notification is required.

#### Done when
- [ ] Given a performance bond AED 500,000 expiring 2027-09-30 on a project whose defects
      liability period ends 2028-06-15, the accountant has a task on 2027-08-01 and the project
      manager is notified.
- [ ] `extend` to 2028-07-31 with the bank amendment resets the reminders; the next task comes on
      2028-06-01.
- [ ] An advance guarantee of AED 150,000 with unrecovered advance AED 90,000 cannot be reduced to
      AED 80,000; AED 90,000 is allowed with the amendment.
- [ ] With no extension, on 2027-10-01 the bond is `expired` and the accountant and project
      manager are notified.
- [ ] `release` without the original or a cancellation letter is refused.

#### Out of scope
Calls (BL-CON-10); retention bond use (BL-CON-07).

---

### BL-CON-10 — 02 Construction: record a guarantee call and its settlement

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-CON-09 · **Blocks:** None.
**Review items:** 02-N12-b

#### Goal
When the beneficiary demands payment under a guarantee, the call is recorded with the demand
notice and amount; the MD, project manager and accountant are told at once and a task to respond
is created. When the bank pays, the amount is booked to the project; a disputed call is carried as
a claim in the final account; a partial call leaves the guarantee `active` for the rest.

#### Files to read
- `docs/verticals/02-construction.md` §4.6 row "The client calls a guarantee", §7.1 *Guarantee*
  (`call`, `settle_call`), §9 "Termination and guarantees"

#### Terms
- **Demand notice:** the beneficiary's written demand to the bank.
- **Debit advice:** the bank's notice that it paid the beneficiary and debited the company.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `called` | error | no | The beneficiary demanded payment |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `call` | active | called | `guarantees.manage` | the beneficiary's demand notice and amount recorded | MD, project manager and accountant told at once; task to respond to the call |
| `settle_call` | called | released, active | `guarantees.manage` | bank's debit advice attached with the amount paid; or, for a call withdrawn before payment, amount paid 0 with the beneficiary's withdrawal letter | amount paid booked to the project's cost code for called guarantees; when disputed, carried as a claim in the final account; `active` with the remaining amount when the call was partial, or with the full amount when it was withdrawn |

§4.6 row: "A call the beneficiary withdraws before payment is closed by `settle_call` with amount
paid 0 and the withdrawal letter, and the guarantee is `active` again."

§9: "the platform records the termination notice, the final account, guarantee calls and the
evidence; it does not judge whether a termination or a call is valid."

§7.1 *Guarantee* (after the table): "A `called` guarantee does not expire: a demand made before
the expiry date stays valid with the bank, so the guarantee stays `called` past its expiry date
until `settle_call`. If `settle_call` returns it to `active` (partial or withdrawn call) after
the expiry date, `expire` runs on the next daily run; `CON.RUL.11` sends no extension task for a
guarantee that is `called`."

#### Edge cases decided
- **Called amount:** ≤ the guarantee amount; partial when lower.
- **After a partial call:** `active` with amount = previous − paid; the expiry stays.
- **Disputed:** a flag set by the project manager on settle; the paid amount is listed in the
  project's final account claims.
- **Call withdrawn before payment:** `settle_call` with amount paid 0 and the beneficiary's
  withdrawal letter attached in place of the debit advice returns it to `active` with the full
  amount; nothing is booked.
- **"At once":** in-app and e-mail immediately on `call`, not in a digest.
- **Expiry date passes while `called`:** nothing happens; `expire` runs only from `active`, and
  the guarantee stays `called` until `settle_call`. A settle that returns it to `active` after
  its expiry date is followed by `expire` on the next daily run (02:00 Asia/Dubai). No
  `CON.RUL.11` extension task while `called`.

#### Must not
- Must not judge or block a call.
- Must not book the cost before the bank's debit advice.

#### Done when
- [ ] Given an `active` performance bond of AED 500,000, `call` for AED 200,000 with the demand
      notice makes it `called` and notifies the MD, project manager and accountant at once, with a
      task to respond.
- [ ] `settle_call` with the debit advice for AED 200,000 books AED 200,000 to the project's called
      guarantees cost code and leaves the bond `active` at AED 300,000.
- [ ] A full call settled for AED 500,000 makes it `released`.
- [ ] Marked disputed, the AED 200,000 appears in the final account claims.
- [ ] Given a bond expiring 2027-08-31 and `called` on 2027-08-20, on 2027-09-01 it is still
      `called`; after a partial `settle_call` on 2027-09-10 it is `active` and becomes `expired`
      on 2027-09-11, with the accountant and project manager notified.
- [ ] A call withdrawn before payment, settled with amount 0 and the withdrawal letter, returns
      the bond to `active` at AED 500,000 and books nothing; without the letter it is refused.

#### Out of scope
The other guarantee transitions (BL-CON-09).

---

### BL-CON-11 — 02 Construction: resubmit, claim, settle or abandon a rejected variation (`CON.ACL.16`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** Base: 02 Variation lifecycle · BL-KRN-01 · **Blocks:** None.
**Review items:** 02-N1-a

#### Goal
A client may reject a variation whose work was already executed on a site instruction. `rejected`
is no longer an end: the cost stays on the variation's own cost code marked unrecovered, and the
project manager resubmits it with more evidence or a new price, or turns it into a claim for the
final account. A claim can be settled at an agreed amount. Giving up (abandon) needs the MD.

#### Files to read
- `docs/verticals/02-construction.md` §4.5 step 1 and row "The client rejects a variation already
  executed on a site instruction", §3.3 `CON.ACL.16`, §6 *Variation*, §7.1 *Variation*

#### Terms
- **Site instruction:** the client's or consultant's instruction to do the work, with a reference.
- **Executed before approval:** variation field set when the work was done before the client
  approved.
- **Unrecovered:** cost booked on the variation's cost code with no approved amount to cover it.
- **Claim:** an item pursued in the final account.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `rejected` | error | no | Not accepted; if the work was executed, its cost is unrecovered until resubmitted, claimed or abandoned |
| `claimed` | warning | no | Pursued as a claim in the final account |
| `abandoned` | neutral | yes | Given up; its cost stays unrecovered |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `reject` | submitted | rejected | `variations.record` | reason recorded | executed work: its cost stays on the variation's cost code, marked unrecovered; QS and project manager notified |
| `resubmit` | rejected | draft | `variations.submit` | — | new revision; the rejected one kept read-only, shown as "resubmitted as rev n+1", and can no longer be claimed or abandoned |
| `claim` | rejected | claimed | `variations.submit` | work executed on a site instruction | listed in the project's claims for the final account |
| `settle_claim` | claimed | approved | `variations.record` | agreed amount and client reference recorded | as `approve` |
| `abandon` | rejected, claimed | abandoned | `variations.submit` | `CON.ACL.16` (MD) | cost written into the project's unrecovered variations |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `CON.ACL.16` | Abandoning a rejected variation or an unsettled claim (its cost stays unrecovered) needs MD approval | `LIM` | project manager | any amount |

`approve` effects: "contract value, budget and next IPA updated (an omission reduces them); time:
completion date moved".

#### Edge cases decided
- **Revision:** `resubmit` creates revision n+1 in `draft` copying the rejected one; the rejected
  revision stays read-only for history. The rejected revision's state shows
  "resubmitted as rev n+1" and cannot be claimed or abandoned again.
- **Not executed:** `claim` is refused; `resubmit` or `abandon` remain. Abandon still needs the MD
  (any amount).
- **Approval:** `abandon` creates an approval request to the MD; on reject or return the variation
  stays in its state (BL-KRN-01).
- **Settle claim:** the agreed amount flows into the next IPA, or into the final account IPA when
  the project is `terminated`.
- **Cost after rejection:** further costs booked to the variation's cost code keep counting as
  unrecovered.

#### Must not
- Must not delete or edit a rejected revision.
- Must not abandon without MD approval.
- Must not move the cost of a rejected variation to another cost code automatically.

#### Done when
- [ ] Given VO-07 executed on site instruction SI-22 with cost AED 40,000 booked, `reject` with
      reason "not instructed by employer" makes it `rejected`, marks AED 40,000 unrecovered and
      notifies the QS and project manager.
- [ ] `resubmit` creates revision 2 in `draft`; revision 1 stays `rejected` and read-only.
- [ ] `claim` lists VO-07 in the project's claims; `settle_claim` at AED 32,000 with client
      reference makes it `approved` and adds AED 32,000 to the contract value and the next IPA.
- [ ] `abandon` requests MD approval; once approved it is `abandoned` and AED 40,000 is in the
      unrecovered variations report.
- [ ] `claim` on a variation not executed is refused.

#### Out of scope
Omission and time variations (BL-CON-12); reminders (BL-CON-13, BL-CON-14).

---

### BL-CON-12 — 02 Construction: price omission and time variations

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** Base: 02 Variation lifecycle · **Blocks:** BL-CON-14
**Review items:** 02 MEDIUM "omission variations", "extension-of-time notices" (review §6)

#### Goal
A variation has a kind: `addition`, `omission` or `time` (extension of time). An omission cannot
omit more than the work not yet done and reduces the contract value when approved. A time variation
states the days asked and the delay event; when approved with days granted it moves the completion
date.

#### Files to read
- `docs/verticals/02-construction.md` §4.5 steps 1–4 and rows "The client approves less than
  priced" and "An omission covers work already done", §6 *Variation*, §7.1 *Variation* (`submit`,
  `approve`)

#### Terms
- **Quantity not yet done:** BOQ line quantity − cumulative progress quantity on the latest IPA.
- **Delay event:** the cause and date of a delay (variation field "event date").
- **Days granted:** the extension the client approves.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `submit` | draft | submitted | `variations.submit` (project manager) | priced (time: days asked and delay event); `CON.ACL.06`; omission: omitted quantity not above the quantity not yet done (from the latest recorded progress: the draft IPA if any, else the latest submitted or certified IPA) | PDF generated; notice date set; when after the contract's notice deadline (`CON.RUL.10`), the variation is marked *notice late* (not refused) |
| `approve` | submitted | approved | `variations.record` | approved amount (or days) and client reference recorded | contract value, budget and next IPA updated (an omission reduces them); time: completion date moved; an omission that now exceeds the quantity not yet done is still approved and the QS is warned to record the excess as a claim |

§4.5 row: "An omission covers work already done | The omitted quantity cannot exceed what is not
yet done; work already done stays in the valuation, or becomes a claim". Row: "The client approves
less than priced | The approved amount is recorded; the gap shows in the variation report".

#### Edge cases decided
- **Progress source:** the quantity not yet done uses the latest recorded progress (draft IPA if
  any, else the latest submitted or certified IPA; `submit` guard).
- **Re-check at approve:** if progress moved since submit and the omission now exceeds the
  quantity not yet done, `approve` is allowed (the client decided) but the
  QS is warned to record the excess as a claim.
- **Omission amount:** stored negative; the contract value and budget decrease.
- **Time variation:** no amount required; days granted may be 0 (rejected in substance) or fewer
  than asked; completion date + days granted.
- **Mixed variation:** one kind per variation (§6); addition and time are two
  variations linked by the same instruction.

#### Must not
- Must not let an omission reduce valued work already done.

#### Done when
- [ ] Given BOQ line 4.2 of 1,000 m² with 700 m² done, an omission of 400 m² is refused at
      `submit`; 300 m² at AED 120 is submitted.
- [ ] Approving it at AED −36,000 reduces the contract value, budget and next IPA by AED 36,000.
- [ ] A time variation asking 30 days for delay event "late drawings, 2027-04-01", approved for 20
      days, moves the completion date from 2027-10-31 to 2027-11-20.
- [ ] An addition priced AED 50,000 approved at AED 42,000 shows a gap of AED 8,000 in the
      variation report.

#### Out of scope
Notice deadlines for delay events (BL-CON-14); rejected variations (BL-CON-11).

---

### BL-CON-13 — 02 Construction: chase an unanswered variation (`CON.RUL.09`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** Base: 02 Variation lifecycle · **Blocks:** None.
**Review items:** 02 MEDIUM "variation response reminder" (review §6)

#### Goal
A submitted variation the client does not answer is chased: after 14 days the project manager is
reminded to chase the client, after 30 days the MD is notified.

#### Files to read
- `docs/verticals/02-construction.md` §4.5 row "The client does not answer", §7.2 `CON.RUL.09`

#### Terms
- **Answered:** the variation left `submitted` (approved, rejected or withdrawn).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CON.RUL.09` | N and M days after variation → `submitted` | not approved or rejected | at N: remind the project manager to chase the client; at M: notify the MD | N = 14; M = 30 | on | MVP |

#### Edge cases decided
- **Days:** calendar days in Asia/Dubai from the latest entry into `submitted`; run daily at 02:00
  (§7.2 intro).
- **Resubmitted revision:** its own clock from its own `submit`.
- **Withdrawn:** no reminder.
- **Idempotency:** one reminder at N and one notice at M per submission.

#### Must not
- Must not send anything to the client automatically.

#### Done when
- [ ] Given VO-09 submitted 2027-03-01 and not answered, the project manager is reminded on
      2027-03-15 and the MD is notified on 2027-03-31.
- [ ] VO-09 approved on 2027-03-20: no MD notice.

#### Out of scope
Notice deadlines (BL-CON-14).

---

### BL-CON-14 — 02 Construction: warn before a contract notice deadline for a delay event (`CON.RUL.10`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** BL-CON-12 · **Blocks:** None.
**Review items:** 02 MEDIUM "extension-of-time notices" (review §6)

#### Goal
Many contracts require notice of a delay event or variation within a period, or the right to claim
is lost. When a project sets that period, the project manager and QS are warned 7 and 2 days before
the deadline while the variation for the event is still a draft; a notice sent late is recorded
with its date.

#### Files to read
- `docs/verticals/02-construction.md` §4.5 step 3 and row "The notice period for an extension of
  time is about to lapse", §6 *Variation* (event date, notice date), §7.2 `CON.RUL.10`

#### Terms
- **Notice period:** a project setting in days (none by default).
- **Notice deadline:** the variation's event date + the project's notice period.
- **Notice date:** the date the variation was submitted to the client.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CON.RUL.10` | N days before the contract's notice deadline for a delay event or variation | variation of the project in `draft` for that event | notify the project manager and QS | N = 7, 2; deadline per project (none by default) | on | MVP |

§4.5 row: "`CON.RUL.10` warns the project manager before the contract's period ends; a notice
sent late is recorded with its date."

#### Edge cases decided
- **No notice period on the project:** the rule does nothing.
- **Notice date:** set on `submit`; when it is after the deadline, the variation shows "notice
  late" (a marker, `submit` is not refused; variation `submit` effects).
- **Event date required:** for kind `time`; for other kinds optional; without it the rule skips the
  variation.
- **Run:** daily at 02:00 Asia/Dubai (§7.2 intro).

#### Must not
- Must not refuse a late submission.

#### Done when
- [ ] Given a project notice period of 28 days and a time variation in `draft` with event date
      2027-04-01, the project manager and QS are notified on 2027-04-22 and 2027-04-27.
- [ ] Submitted on 2027-05-02, the variation shows "notice late" with notice date 2027-05-02.
- [ ] With no notice period set, no warning.

#### Out of scope
Variation kinds (BL-CON-12).

---

### BL-CON-15 — 02 Construction: open an absence case for a worker who stops coming (`CON.RUL.08`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** Base: wl-attendance Attendance record lifecycle · BL-HRM-07 · BL-TKT-11 · **Blocks:** None.
**Review items:** 02 MEDIUM "worker absconding" (review §6)

#### Goal
A crew worker with no attendance and no approved leave for 3 consecutive working days gets an
absence case ticket for HR, and the supervisor is told. At 7 days HR is reminded that an absconding
report may be requested (approved by the MD in wl-hr).

#### Files to read
- `docs/verticals/02-construction.md` §4.2 row "A worker stops coming", §7.2 `CON.RUL.08`, §3.3
  `HRM.ACL.07`

#### Terms
- **Working day:** a day the worker's site works (site calendar), excluding public holidays.
- **Absence case:** a ticket of category *absence* for HR.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CON.RUL.08` | daily | a worker assigned to a crew has no attendance and no approved leave for N consecutive working days of the crew's site (days when the project is `on_hold` or `terminated` do not count) | open an absence case ticket for HR (category *absence*), one per absence spell; notify the supervisor; at M consecutive working days, counted the same way, remind HR that `report_absconding` may be requested; attendance recorded again stops the count | N = 3; M = 7 | on | MVP |

§4.2 row: "After 7 days HR may report absconding (`report_absconding`, approved by the MD,
`HRM.ACL.07`): the worker leaves the crew, payroll lines are held, and the cancellation case starts
in wl-hr".

#### Edge cases decided
- **Counting:** consecutive working days of the worker's crew site; days when the project is
  `on_hold` or `terminated` (attendance blocked) do not count (`CON.RUL.08`).
- **7 days:** M = 7 consecutive working days, counted the same way (`CON.RUL.08`).
- **One case per absence spell:** attendance recorded again stops the count; the open ticket stays
  for HR to close with a note.
- **Run time:** daily at 02:00 Asia/Dubai for the previous days (§7.2 intro), so late
  entries made within the late-entry window are seen.
- **Absconding:** this rule never runs `report_absconding`; HR requests it (BL-HRM-07).

#### Must not
- Must not report a worker as absconded automatically.

#### Done when
- [ ] Given worker W in crew C with no attendance on Mon 2027-03-01, Tue 2027-03-02 and Wed
      2027-03-03 (all working days) and no leave, on 2027-03-04 an *absence* ticket opens for HR
      and the supervisor is notified.
- [ ] On a Monday–Saturday site calendar, with W still absent through 2027-03-08 (7 working
      days; Sunday 2027-03-07 not counted), HR is reminded on 2027-03-09.
- [ ] W with approved leave on those days gets no case.
- [ ] W marked present on 2027-03-05 stops the count; no reminder at 7 days.

#### Out of scope
`report_absconding` and its effects (BL-HRM-07).

---

### BL-CON-16 — 02 Construction: hold a PO to an unregistered subcontractor on a Dubai project (`CON.ACL.13`)

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Launch 02 · **Size:** s
**Depends on:** Base: wl-inventory Purchase order lifecycle · BL-KRN-01 · **Blocks:** None.
**Review items:** X-13 (02 values)

#### Goal
Under Dubai Law 7/2025 a contractor must check that its subcontractors are registered with Dubai
Municipality. A PO to a subcontractor on a Dubai project, when the subcontractor has no valid
registration on file, needs approval before it is issued.

#### Files to read
- `docs/verticals/02-construction.md` §2 "Contractor registration", §3.3 `CON.ACL.13`, §7.1
  *Purchase order* (`issue`), §7.2 "Expired documents", §9 "Contractor registration"

#### Terms
- **Subcontractor:** a supplier party marked as a subcontractor (not a material supplier).
- **Dubai project:** a project whose site is in the emirate of Dubai.
- **Registration:** a document of kind *Dubai Municipality contractor registration* on the
  supplier party, with an expiry date.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `issue` | draft | issued | `purchase_orders.issue` | `STK.ACL.01` (SoD); supplier, prices and delivery site set; Dubai subcontractor registration (`CON.ACL.13`) | PDF sent to supplier; committed cost added to budget |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `CON.ACL.13` | A purchase order to a subcontractor (a supplier party with the role `subcontractor`) on a Dubai project needs approval when the subcontractor has no valid Dubai Municipality contractor registration on file on the PO date; checked on `issue` and again on each `revise`; the approver is the project manager of the PO's project, never the PO's issuer | `LIM` | procurement, project manager | on for Dubai projects |

§7.2: "Guarantees, permits and the contractor registration only remind (`DOC.RUL.01`); the
subcontractor registration guards PO `issue` through `CON.ACL.13`."

#### Edge cases decided
- **Subcontractor marker:** the party role `subcontractor` on the supplier (`CON.ACL.13`); POs to
  other suppliers skip the check.
- **Valid:** `document_valid(supplier, Dubai Municipality contractor registration, PO date)`.
- **Approver:** the project manager of the PO's project (`CON.ACL.13`), never the PO's
  issuer; on reject or return the PO stays `draft` with the reason (BL-KRN-01).
- **PO revise:** a revision re-runs the check (`CON.ACL.13`).
- **Not a document block:** wl-documents does not block; this is an approval rule.

#### Must not
- Must not refuse the PO outright; it goes to approval.

#### Done when
- [ ] Given a Dubai project and subcontractor S with an expired registration, issuing a PO to S
      creates an approval request to the project manager and the PO stays `draft`.
- [ ] After approval the PO is `issued`.
- [ ] With S's registration valid, the PO is issued without approval.
- [ ] On an Abu Dhabi project, no check.

#### Out of scope
PO SoD and revision (Epic wl-inventory: BL-STK-08).

---

### BL-CON-17 — 02 Construction: configure shared negative-flow settings — money and procurement

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `blocked:open-item` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-INV-01 · BL-INV-04 · BL-INV-05 · BL-INV-10 · BL-INV-13 · BL-PAY-09 · BL-PAY-10 · BL-PAY-14 · BL-PAY-24 · BL-STK-01 · BL-STK-03 · BL-STK-04 · BL-STK-05 · BL-STK-07 · BL-STK-08 · BL-STK-27 · BL-PTY-03 · BL-KRN-01 · BL-CON-03 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · BL-STK-28 · BL-STK-29 · BL-STK-30 · BL-STK-31 · **Blocks:** None.
**Review items:** X-01, X-02, X-04, X-05, X-08, X-18, 02-N10-a, 02 MEDIUM "PO revision", "site stock counts" (review §6)

#### Goal
02 seeds its values for the shared money and procurement mechanisms built in the module epics:
write-off, invoice disputes, e-invoice rejection, bounced client cheques, rejected deliveries and
returns to suppliers, PO revision, stock count variances, party block effects for suppliers, and
non-chargeable snag tickets. Nothing new is built here; each value is configured in the 02 seed and
checked by a test.
It also seeds the 02 values for the cash-flow forecast alert (`FIN.CFL.01`), the company
budget with owner drawings (`FIN.BUD.01`) and the supplier scorecard (`STK.SUP.01`), added on
2026-10-16: `FIN.RUL.23`, `FIN.RUL.24`, `FIN.ACL.21`, `STK.RUL.09` and `STK.ACL.07`. The
supplier scorecard moved here from BL-CON-18 on 2026-10-16, with the procurement settings.

#### Files to read
- `docs/verticals/02-construction.md` §3.3, §4.3, §4.4 (tables "When it goes wrong"), §7
  (intro: 02 settings for invoice, cheque and ticket), §7.2 (rules marked *shared*, "Negative-flow
  settings")
- The module settings sections (§7) of `docs/modules/wl-invoicing.md`, `wl-payments.md`,
  `wl-inventory.md`, `wl-parties.md`
- `docs/modules/wl-invoicing.md` §5.14–§5.16 and §7 (cash-flow forecast, company budget, owner
  drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments);
  `docs/modules/wl-inventory.md` §5.9 (supplier scorecard and alternative suppliers)
- `docs/OPEN-ITEMS.md` L-37

#### Terms
- **Seed:** the vertical's default configuration for every new 02 deployment.
- **MD:** managing director (company admin / MD role).

#### Spec excerpt (authoritative for this issue)

| # | Setting | 02 value | Spec reference |
|---|---|---|---|
| **Invoices** | | | |
| 1 | Write-off limit and approver (`FIN.ACL.15`) | AED 0 (always approval); approver the MD, not the requester | §3.3 `FIN.ACL.15`; §7 intro |
| 2 | Invoice dispute | recorded by staff; party portal dispute off (portal Later) | §7 intro |
| 3 | `FIN.RUL.15` dispute escalation | 14 days; remind the accountant, then escalate to the project manager | §7.2 `FIN.RUL.15` |
| 4 | Bad-debt relief (`FIN.RUL.14`) | on (standard-rated 5% invoices) | §4.4 row "A debt is uncollectable" |
| 5 | E-invoice rejected by the ASP (`FIN.RUL.16`) | the invoice stands; task to correct and resend; content errors corrected by credit note and a new invoice | §4.4 row "An e-invoice is rejected" |
| **Cheques** | | | |
| 6 | Bounce fee | none in 02 by default | §7 intro; §7.2 `FIN.RUL.18` |
| 7 | Maximum presentations | 2 | §7 intro |
| 8 | `FIN.RUL.18` chase | 3 days: remind the client and the project manager; 14 days: task to re-present, refer to legal or write off | §7.2 `FIN.RUL.18` |
| 9 | Who is told on a client cheque bounce | the project manager, who decides whether to slow or suspend work under the contract (suspension rights `⚠ open: lawyer (L-37)`) | §4.4 row "A client cheque bounces" |
| **Receiving and returns** | | | |
| 10 | Rejection reasons at goods receipt | `damaged`, `wrong_item`, `quality`, `excess`; photos | §4.3 row "Damaged or wrong materials delivered" |
| 11 | Refused at the door / kept for collection | Return to supplier dispatched at once / created as a draft; procurement and the supplier told | §4.3 row "Damaged or wrong materials delivered" |
| 12 | Return to supplier approval (`STK.ACL.06`) | above AED 10,000; approver project manager; not the creator | §3.3 `STK.ACL.06` |
| 13 | `STK.RUL.08` returns not credited | task to procurement 30 days after `dispatched` | §7.2 `STK.RUL.08` |
| 14 | Receipt cost object | site store receipts charged to the project on receipt; central store when issued or transferred | §4.3 step 4 |
| 15 | Goods receipt late entry | business time = the delivery date | §7.2 settings table row Goods receipt |
| **Purchasing** | | | |
| 16 | PO `revise` re-approval | a higher total is approved again with the `STK.ACL.03` limits AED 10,000 / AED 50,000 (project manager / MD) | §4.3 row "The PO must change"; §3.3 `STK.ACL.03` |
| 17 | Purchase request or PO approval rejected or returned | back to `submitted` or `draft` with the reason | §7.2 settings table row Purchase request, PO |
| 18 | Stock count variance (`STK.ACL.02`) | above AED 1,000 approved by someone who did not count; losses to the project's wastage cost code | §4.3 row "The site count differs" |
| **Party blocked and tickets** | | | |
| 19 | Supplier blocked | no new PO and no revision that raises the total or moves the delivery date later (BL-STK-27); payments stop (`PTY.ACL.03`) | §7.2 settings table row Purchase request, PO |
| 20 | Client blocked | project block effect `continue`, decider MD (BL-CON-03) | §7.2 settings table row Project |
| 21 | Snag and defect tickets | never chargeable | §7 intro |
| **Payments** | | | |
| 22 | Payment method *set-off* (BL-PAY-24) | on (back- and contra-charges set off against an IPA invoice, BL-CON-06) | §7 intro; §7.1 *IPA* `invoice` |
| **Cash-flow forecast and budget** | | | |
| 23 | Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 200,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the MD and the accountant, checked daily at 06:00; certified IPAs by due date; retention releases on their expected release date; payroll by the WPS due date; due dates adjusted by each client's average days late: on; disputed amounts counted: off | §5; §7.2 settings |
| 24 | Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | company budget (overheads: office, yard, plant, staff) per legal entity and month; categories owned by the accountant and mapped to cost codes; approved by the MD; project budgets stay in job costing; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.2 settings |
| 25 | Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of operating cost, project cost and profit; every drawing (AED 0) approved by the MD, never by the requester (`FIN.ACL.21`) | §5; §7.2 settings |
| **Supplier scorecard** | | | |
| 26 | Supplier scorecard review (`STK.SUP.01`, `STK.RUL.09`, `STK.ACL.07`) | score on a 0–100 scale over a 3-month window (grace 1 day, at least 3 receipts, module weights); below 70 `STK.RUL.09` gives the procurement officer a review task and tells the project manager; preferred and alternative suppliers per item offered on purchase requests and reorder suggestions; making another supplier an item's preferred one needs a second person (`STK.ACL.07`: on) | §5; §7.2 settings |

#### Edge cases decided
- **Item 9:** a notification to the project manager with the cheque and client; no automatic
  project `hold` (`blocked:open-item` L-37 for any rule on suspension).
- **Stock count before a project closes or stops:** §4.3 step 8: no rule enforces the quarterly
  count; the count opened by `terminate` (BL-CON-01) is the only automatic one, and closing with a
  count open only warns (BL-CON-02).
- **Client changes:** values 1, 7, 12, 16 and 18 can be changed per client within the module's
  limits.

#### Must not
- Must not hard-code any of these values in 02 code.
- Must not seed a bounce fee.

#### Done when
- [ ] **Invoices:** a write-off of AED 1 creates an approval request to the MD; no dispute button
      exists for client contacts; a dispute open 14 days reminds the accountant.
- [ ] **Invoices (items 4, 5):** a written-off 5% invoice that meets the relief conditions gets a
      `FIN.RUL.14` VAT adjustment proposal for the accountant (4); an e-invoice rejected by the
      ASP keeps the invoice `issued` and gives the accountant a task to correct and resend; a
      content error is corrected by a credit note and a new invoice, not by editing (5).
- [ ] **Cheques:** a bounced client cheque creates no fee, notifies the project manager, and a third
      presentation is refused.
- [ ] **Cheques (item 8):** given a client cheque bounced on 2027-04-01 and still open, on
      2027-04-04 the client and the project manager are reminded, and on 2027-04-15 the
      accountant has the task to re-present, refer to legal or write off.
- [ ] **Receiving and returns:** a receipt line can be split into accepted and rejected quantity
      with reason `damaged`; a return of AED 12,000 needs the project manager; one dispatched 30
      days without credit creates a procurement task.
- [ ] **Receiving (items 11, 14, 15):** a delivery refused at the door creates a return to
      supplier that is `dispatched` at once; one kept for collection creates a `draft` return;
      procurement and the supplier are told in both cases (11). A receipt into a site store
      posts its cost to the project on receipt; a receipt into the central store posts no project
      cost until it is issued or transferred to a site (14). A receipt entered on 2027-05-12 with
      delivery date 2027-05-10 has business time 2027-05-10 (15).
- [ ] **Purchasing:** revising a PO from AED 8,000 to AED 15,000 needs project manager approval; a
      count variance of AED 1,500 cannot be approved by the counter.
- [ ] **Purchasing (item 17):** a purchase request above AED 10,000 whose approval request is
      returned stays `submitted` with the reason; a PO whose `issue` approval is rejected stays
      `draft` with the reason; both requesters are told.
- [ ] **Party blocked and tickets:** a PO to a blocked supplier is refused; a revision of an issued
      PO to that supplier that raises its total is refused; a snag ticket cannot be marked
      chargeable.
- [ ] **Client blocked (item 20):** the project block effect is `continue` with the MD as decider:
      blocking client C creates no review task and C's `in_progress` project keeps accepting
      attendance (the full behaviour is tested in BL-CON-03).
- [ ] **Payments (item 22):** method *set-off* is offered to the accountant in a fresh 02
      deployment.
- [ ] **Cash-flow forecast:** in a fresh 02 deployment, a projected balance of AED 199,999 in week
      5 alerts the MD and the accountant once (`FIN.RUL.23`, 06:00 run); AED 200,000 or more in
      every week alerts no one; the forecast shows 13 weekly buckets from Monday.
- [ ] **Budget and drawings:** a monthly budget for a legal entity prepared by the accountant
      needs approval by the MD; an expense with category *owner drawings* is left out of operating
      cost and profit reports, and the user who records it cannot approve it (`FIN.ACL.21`); an
      expense line reaching 100% of its month's budget line warns the category's owner role and
      the accountant once (`FIN.RUL.24`).
- [ ] **Supplier scorecard (item 26):** a supplier whose monthly snapshot scores 69 of 100 over
      the 3-month window gives the procurement officer one review task and tells the project
      manager (`STK.RUL.09`); 70 creates none; a supplier with fewer than 3 receipts gets no
      score; an item's alternative supplier is offered on a purchase request for the item; making
      it the preferred supplier waits for a second person (`STK.ACL.07`).

#### Out of scope
The mechanisms themselves (module epics in Depends on); labour and compliance (BL-CON-18).

---

### BL-CON-18 — 02 Construction: configure shared negative-flow settings — labour and compliance

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:high` `size:m` · **Milestone:** Launch 02 · **Size:** m
**Depends on:** BL-ATT-01 · BL-ATT-02 · BL-ATT-03 · BL-ATT-04 · BL-ATT-05 · BL-ATT-06 · BL-ATT-07 · BL-ATT-08 · BL-HRM-01 · BL-HRM-04 · BL-HRM-07 · BL-DOC-02 · BL-DOC-04 · BL-KRN-04 · BL-KRN-08 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None.
**Review items:** X-13, X-15, X-16, X-19, 02-N5-d, 02-N8-a

#### Goal
02 seeds its values for the shared labour-law mechanisms: the owner role and compliance-critical
rules, the overtime cap and midday-break flags, the expired visa or permit flag, the reviewers of
flagged attendance, adjustment timesheets, absconding approval and late entry of attendance.
It also seeds the 02 values for lead sources and campaigns (`SAL.CRM.02`), onboarding and SOPs
(`HRM.ONB.01`) and registration kinds (`DOC.EXP.01`), added on 2026-10-16: `SAL.ACL.07`,
`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`. The supplier
scorecard (`STK.SUP.01`) is seeded in BL-CON-17.

#### Files to read
- `docs/verticals/02-construction.md` §2 "Labour", §3.3 (`HRM.ACL.01`, `HRM.ACL.07`–`09`), §4.2
  (table "When it goes wrong"), §7.1 *Timesheet*, §7.2 (`HRM.RUL.01`, `HRM.RUL.13`, `HRM.RUL.14`,
  paragraphs "Compliance-critical rules and owner alerts", "Expired documents", "Negative-flow
  settings")
- The module settings sections (§7) of `docs/modules/wl-attendance.md`, `wl-hr.md`,
  `wl-documents.md`, `wl-kernel.md`
- `docs/modules/wl-sales.md` §4.6, §5.8 and §7 (campaigns, sources); `docs/modules/wl-hr.md` §5.8
  and §7 (onboarding); `docs/modules/wl-documents.md` §3, §5.7 and §7 (SOPs, registration kinds)

#### Terms
- **Flag:** a marker on an attendance record that holds it for review (`overtime_cap`,
  `midday_break`, `permit_invalid`, `overlap`).
- **Outdoor site:** a site marked outdoor; the midday break applies there in the season.

#### Spec excerpt (authoritative for this issue)

| # | Setting | 02 value | Spec reference |
|---|---|---|---|
| **Owner alerts** | | | |
| 1 | Owner role (`PLT.RUL.04`) | the MD, and the company admin | §7.2 "Compliance-critical rules and owner alerts" |
| 2 | Compliance-critical rules | `HRM.RUL.01`, `HRM.RUL.13`, `HRM.RUL.14` | §7.2 rules table |
| **Hours** | | | |
| 3 | Overtime caps (`HRM.RUL.13`) | 8 normal + 2 overtime hours a day across all sites; overtime above 144 hours in three weeks → flag `overtime_cap` | §4.2 row "Hours inside the midday break…"; §7.2 `HRM.RUL.13` |
| 4 | Midday break (`HRM.RUL.01`, `HRM.RUL.13`) | 12:30–15:00, 15 June–15 September, outdoor sites; break field pre-filled; removing it or hours crossing it → flag `midday_break`; HSE officer notified | §2; §7.2 |
| 5 | Who is told on an hours flag | the supervisor, the HSE officer and the project manager | §4.2 row; §7.2 `HRM.RUL.13` |
| **Permits** | | | |
| 6 | Kinds checked by `HRM.RUL.14` | work permit, residence visa; flag `permit_invalid`; HR and the supervisor told the same day | §7.2 `HRM.RUL.14`; §4.2 row "A worker with an expired visa…" |
| 7 | Document kinds work permit and residence visa (`blocks`) | `attendance_record.accept`, mode `flag` (no override needed; the record is reviewed); hours paid and costed meanwhile | §7.2 "Expired documents" |
| 8 | Look-ahead (`DOC.RUL.02`) | 30 days before expiry, list the crews affected | §4.2 row "A worker with an expired visa…" |
| 9 | Kinds that only remind (`DOC.RUL.01`) | guarantees, permits, the client's contractor registration | §7.2 "Expired documents" |
| **Review and corrections** | | | |
| 10 | Reviewers of flagged attendance (`HRM.ACL.09`) | project manager for `overtime_cap`, `midday_break` and `overlap`; HR for `permit_invalid`; never the supervisor who marked it; reason required | §3.3 `HRM.ACL.09` |
| 11 | Adjustment timesheet (`HRM.ACL.08`) | origins `attendance_error`, `payroll_error`; approved by a project manager who did not create it; cost delta to the cost codes; payroll export carries the difference | §4.2 row "A locked timesheet is wrong"; §7.1 *Timesheet* |
| 12 | Timesheet `submit` | refused while attendance is left flagged; refused for hours above the attended hours (`overlap`) | §7.1 *Timesheet*; §4.2 row "The same worker is marked on two sites" |
| 13 | Absconding report (`HRM.ACL.07`) | requested by HR, approved by the MD | §3.3 `HRM.ACL.07` |
| **Late entry** | | | |
| 14 | Attendance by the supervisor (`HRM.ACL.01`) | today or 1 day back; later only by the project manager with the late-entry flag and a reason (`PLT.ACL.10`) | §3.3 `HRM.ACL.01`; §4.2 row "Attendance entered after the late-entry window" |
| **Sales, finance, staff and registrations** | | | |
| 15 | Lead sources used (`SAL.CRM.02`) | referral, website, event, partner (consultants and main contractors), other; tenders and enquiries from main contractors are leads (`SAL.CRM.01`); no source requires a campaign | §5; §7.2 settings |
| 16 | Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`) | tender invitations, exhibitions, LinkedIn, email, company profile mailings; converted value: the first accepted quotation; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on) | §5; §7.2 settings |
| 17 | Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for worker (site induction, PPE issue, visa and ID documents), supervisor, storekeeper, procurement officer and QS, started by `HRM.RUL.16` when the employee record (`HRM.EMP.01`) is created; checklist steps use `WRK.CHK.01`; tasks to HR / PRO, the project manager and the employee; the employee's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds overdue steps every 2 days and tells HR / PRO and the line manager after 5 days; completion blocks nothing (role activation not blocked) | §5; §7.2 settings |
| 18 | SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: HSE officer (safety SOPs, method statements) and project manager (site SOPs); required readers: supervisors and the workers of the project's crews; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the manager and the owner role (MD) after 7 days; acknowledged only by the reader, in person with a drawn signature and a staff witness for workers without a session (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.2 settings |
| 19 | Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *ISO certificate* (subject: legal entity; responsible role HSE officer); *Trademark registration* and *Other certification* (legal entity; MD); `DOC.RUL.01` at 90, 30 and 7 days; no `blocks` | §5; §7.2 settings |

#### Edge cases decided
- **`midday_break` reviewer:** `HRM.ACL.09` covers `midday_break` too; it is reviewed by the
  project manager (holder of `attendance.review`) like `overtime_cap`.
- **Three weeks:** a rolling 21-day window ending on the attendance day (`HRM.RUL.13`).
- **Client changes:** the caps, break window and season are legal values; the seed marks them
  read-only for clients (§7.2 "Compliance-critical rules and owner alerts").

#### Must not
- Must not let the supervisor who marked the hours accept a flag.
- Must not refuse attendance because a permit expired; flag it.

#### Done when
- [ ] **Owner alerts:** a failed `HRM.RUL.14` run alerts the MD and the company admin.
- [ ] **Hours:** a worker marked 11 hours on 2027-03-10 is flagged `overtime_cap`; on an outdoor
      site on 2027-07-10 hours from 11:00 to 16:00 are flagged `midday_break` and the HSE officer
      is notified.
- [ ] **Permits:** a worker whose work permit expired on 2027-05-31, marked present on 2027-06-01,
      is flagged `permit_invalid`, HR and the supervisor are told that day, and the hours are
      costed; on 2027-05-01 the crews affected are listed.
- [ ] **Review and corrections:** the supervisor cannot accept the flag; the project manager accepts
      `overtime_cap` with reason "emergency slab pour" and a `midday_break` flag with reason
      "exempt emergency job"; only HR can accept `permit_invalid`; an
      adjustment timesheet created by project manager A is approved only by project manager B.
- [ ] **Timesheet submit (item 12):** a timesheet with one attendance record still flagged
      `overtime_cap` is refused on `submit` ("flagged attendance not reviewed"); after the flag is
      accepted it is submitted. A timesheet allocating 9 hours to cost codes for a day with 8
      attended hours is refused (`overlap`).
- [ ] **Absconding (item 13):** for worker W with an open absence case, HR user H requests
      `report_absconding`; an approval request goes to the MD and nothing changes for W until it
      is approved; H cannot approve it; a project manager cannot request it; when the MD approves,
      W leaves the crew and W's payroll lines are held.
- [ ] **Remind only (item 9):** a performance bond expiring in 30 days and the client's
      contractor registration expiring in 30 days remind their responsible roles (`DOC.RUL.01`);
      on the day after either expires, no transition is refused or flagged because of it.
- [ ] **Late entry:** a supervisor entering attendance for 3 days ago is refused; the project
      manager enters it with a reason and a late-entry flag.
- [ ] **Lead sources:** a new lead or party offers the sources referral, website, event, partner
      and other; a campaign with channel *tender invitations*, a budget and cost entries reports
      leads, conversion, cost per lead, CAC and ROAS; a cost entry over the budget waits for a
      manager (`SAL.ACL.07`).
- [ ] **Onboarding and SOPs:** hiring a worker creates the worker checklist with tasks to HR /
      PRO, the line manager and the employee, and the role is active before it is complete; a step
      open 2 days past its due date reminds its assignee (`HRM.RUL.17`); a new version of an SOP
      owned by the HSE officer asks every required reader to acknowledge again within 7 days
      (`DOC.RUL.04`); a worker without a session acknowledges in person with a drawn signature and
      a staff witness, and no one else can acknowledge for them (`HRM.ACL.10`).
- [ ] **Registrations:** a fresh 02 deployment lists *ISO certificate*, *Trademark registration*
      and *Other certification* with responsible role HSE officer for ISO, MD for the others; a
      trademark registration expiring in 90 days notifies the MD.

#### Out of scope
The mechanisms themselves (module epics in Depends on); the absence case (BL-CON-15); the supplier
scorecard (BL-CON-17).

---

### BL-CON-19 — 02 Construction: make the client portal read-only after the final account

**Labels:** `vertical:02-construction` `area:negative-flow` `priority:medium` `size:s` · **Milestone:** Later · **Size:** s
**Depends on:** BL-CON-02 · Base: wl-portal client portal (`POR.SLF.01`, Later in 02) · **Blocks:** None.
**Review items:** 02-N2-a

#### Goal
When the client portal exists for 02 (Later), client contacts keep read-only access to a project's
documents after its final account is closed (by `close` or `settle_final_account`), then the portal
closes after 90 days.

#### Files to read
- `docs/verticals/02-construction.md` §3.3 `POR.ACL.01`, `CON.ACL.12`, §4.8 step 6, §7.1
  *Project* (`settle_final_account`), §5 `POR.SLF.01` (Later)

#### Terms
- **Read-only:** documents visible, nothing can be uploaded or answered.

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `POR.ACL.01` *(shared)* | After a project's final account is closed, client contacts keep read-only access to its documents, then the portal closes | `TIM` | client contact | closes after 90 days |

`settle_final_account` effect: "client portal read-only (`POR.ACL.01`)".

#### Edge cases decided
- **Days:** 90 calendar days from the date the project became `closed` (Asia/Dubai).
- **Other projects of the same client:** unaffected.

#### Must not
- Must not close access to other projects of the same contact.

#### Done when
- [ ] Given project P closed on 2028-01-31, its client contact can read P's documents until
      2028-04-30 and cannot upload; on 2028-05-01 P is no longer visible.

#### Out of scope
The client portal itself (`POR.SLF.01`, Later).
