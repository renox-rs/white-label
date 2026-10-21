# Epic: 05 Clinics — negative flows

> Status: draft · Last updated: 2026-10-16 · Spec version: same commit as this file

This epic builds the negative flows the review fixes added to
[05-clinics.md](../../verticals/05-clinics.md): coverage outcomes at check-in and the self-pay
consent (`CLN.ACL.15`), prior authorisation and insurance card look-aheads (`CLN.RUL.09`,
`CLN.RUL.10`), billing of work done when a visit stops after check-in, the claim negatives
(technical rejection, overpayment and recoupment, late close, void; `CLN.ACL.14`, `CLN.ACL.16`,
`CLN.RUL.03`, `CLN.RUL.05`), licence expiry (`CLN.RUL.01`), lots on consumables and recalls traced
to patients, and 05's values for the shared mechanisms. It closes 05-N3-c, 05-N5-a, 05-N5-b,
05-N5-c, 05-N7-a, 05-N8-a, 05-N10-a, 05-N11-a and the 05 MEDIUM items (walk-out after check-in,
clinic-side cancellation and deposit refund, billing disputes, void a claim, practitioner sick
on the day, negative stock), on top of the module epics for X-02, X-03, X-06, X-07, X-09, X-10,
X-13, X-17, X-19 and X-20.

| ID | Title | Priority | Size | Milestone | Depends on |
|---|---|---|---|---|---|
| BL-CLN-01 | Record a coverage outcome at check-in (`CLN.ACL.15`) | high | m | Launch 05 | Base: 05 Visit and Insurance policy; BL-SCH-01; BL-SCH-02; BL-SCH-08 |
| BL-CLN-02 | Require a valid prior authorisation with units left at check-in | high | s | Launch 05 | BL-CLN-01; Base: 05 Prior authorisation lifecycle |
| BL-CLN-03 | Flag appointments when a prior authorisation is denied, used up or ends (`CLN.RUL.09`) | high | m | Launch 05 | Base: 05 Prior authorisation lifecycle; Base: 05 `CLN.RUL.07` |
| BL-CLN-04 | Ask for a new insurance card before an insured appointment (`CLN.RUL.10`) | medium | s | Launch 05 | Base: 05 Insurance policy; Base: wl-portal uploads |
| BL-CLN-05 | Bill work done when a visit is cancelled after check-in | high | m | Launch 05 | BL-SCH-02; BL-SCH-03; Base: 05 charge capture |
| BL-CLN-06 | Charge only the consultation when the planned procedure is not suitable | medium | s | Launch 05 | BL-SCH-02; Base: 05 charge capture |
| BL-CLN-07 | Return a claim to draft after a technical rejection (`reject_submission`) | high | m | Launch 05 | Base: 05 Claim lifecycle; BL-INV-16 |
| BL-CLN-08 | Hold a remittance overpayment and record a payer recoupment on a claim | high | m | Launch 05 | Base: 05 Claim lifecycle; BL-INV-17; BL-KRN-01 |
| BL-CLN-09 | Close a claim that missed the first-submission window (`CLN.ACL.16`) | high | m | Launch 05 | Base: 05 Claim lifecycle; BL-INV-18; BL-KRN-01 |
| BL-CLN-10 | Void a claim created in error (`CLN.ACL.14`) | medium | s | Launch 05 | Base: 05 Claim lifecycle; BL-INV-18; BL-KRN-03; BL-CLN-07 |
| BL-CLN-11 | Warn before the first-submission window ends (`CLN.RUL.03`) | high | s | Launch 05 | Base: 05 Claim lifecycle; BL-KRN-08 |
| BL-CLN-12 | Block bookings and alert the owner when a practitioner licence expires (`CLN.RUL.01`) | high | s | Launch 05 | BL-HRM-01; BL-DOC-05; BL-SCH-07; BL-KRN-08 |
| BL-CLN-13 | Record the lot of every consumable issued to a visit | high | s | Launch 05 | Base: wl-inventory Lot and stock movements; BL-STK-11; BL-STK-18 |
| BL-CLN-14 | Trace a recalled lot to patients and send neutral notices | high | m | Launch 05 | BL-CLN-13; BL-STK-14; BL-STK-15; BL-STK-16; BL-NTF-02 |
| BL-CLN-15 | 05: configure shared negative-flow settings (bookings, licences, alerts, connections, messages) | high | m | Launch 05 | BL-SCH-01 to BL-SCH-10; BL-DOC-01; BL-DOC-06; BL-KRN-05; BL-KRN-08; BL-NTF-01; BL-SAL-19; BL-SAL-20; BL-SAL-21; BL-SAL-22; BL-SAL-23; BL-SAL-24; BL-SAL-25; BL-SAL-26; BL-NTF-07; BL-NTF-08; BL-NTF-09; BL-HRM-26; BL-HRM-27; BL-HRM-28; BL-DOC-08; BL-DOC-09; BL-DOC-10 |
| BL-CLN-16 | 05: configure shared negative-flow settings (money and stock) | high | m | Launch 05 | BL-INV-04; BL-INV-05; BL-INV-18; BL-PAY-02 to BL-PAY-07; BL-PAY-17; BL-SAL-16; BL-STK-11 to BL-STK-20; BL-INV-25; BL-INV-26; BL-INV-27; BL-INV-28; BL-INV-29; BL-INV-30; BL-PAY-25; BL-PAY-26 |

---

### BL-CLN-01 — 05 Clinics: record a coverage outcome at check-in (`CLN.ACL.15`)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: 05 Visit and Insurance policy (eligibility result) · Base: wl-documents signatures (`DOC.SIG.01`) · BL-SCH-01 (reason codes; vertical codes) · BL-SCH-02 (deposit carried on reschedule, refunded on business cancel) · BL-SCH-08 (licence re-check at `check_in`) · **Blocks:** BL-CLN-02
**Review items:** 05-N5-c, X-17

#### Goal
When the patient's cover fails at check-in (eligibility negative, card or policy expired), the
receptionist records the result and one coverage outcome on the visit before `check_in` can
run: re-split to another valid policy, self-pay with a signed self-pay consent showing the
estimated price, or reschedule. A patient who refuses self-pay and does not reschedule is
cancelled with the 05 reason `coverage_not_confirmed`, which refunds the deposit in full. This
stops a clinic from treating a patient and then finding nobody will pay, and stops a patient
being billed a price they never saw.

#### Surfaces
- **Back office:** on the pre-check-in screen the receptionist records the eligibility
  result and one coverage outcome (re-split to another policy, self-pay, reschedule) or
  cancels with `coverage_not_confirmed`; the patient signs the self-pay consent with the
  estimated price in person on the front-desk tablet (`DOC.SIG.01`, not in the portal); Check in is refused with "coverage outcome required" until
  both are set, and charge capture refuses a service not on the consent.
- **System:** the pending check-in data moves to the visit at Check in and is discarded at
  the end of the appointment day; `coverage_not_confirmed` refunds the deposit in full.

#### Files to read
- `docs/verticals/05-clinics.md` §3.3 (`CLN.ACL.15`), §4.2 "When it goes wrong" (row
  "Eligibility negative, or the insurance card or policy expired, at check-in"), §6 (Visit),
  §7.1 *Appointment* (`check_in` row and the paragraph on reason codes), §9 (Self-pay consent)
- `docs/modules/wl-scheduling.md` §4.1 (`check_in`, `reschedule`, reason codes)

#### Terms
- **Eligibility check:** the receptionist asks the payer (portal or phone in the MVP) whether
  the policy covers the visit today and records the result (positive or negative) on the visit.
- **Coverage outcome:** one value on the visit: `insured` (cover confirmed), `re_split`
  (another valid policy or an updated card is used), `self_pay` (patient pays the whole visit,
  with consent reference).
- **Re-split:** the visit's charge split (`CLN.ENC.02`) is recalculated with another policy;
  the patient share changes.
- **Self-pay consent:** a consent form (wl-documents, `DOC.SIG.01`) signed by the patient on the
  tablet, showing the services and their estimated self-pay price from the self-pay price list
  (`SAL.CAT.01`; the estimate is each service's self-pay price on the visit date). Kept with
  the visit like other consent forms.
- **Insured visit:** an appointment booked with an insurance policy attached.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `check_in` | booked, confirmed | checked_in | `appointments.checkin` | identity checked; practitioner licensed today (`SCH.ACL.04`); for insured visits: eligibility recorded and a coverage outcome set (insured, re-split, or self-pay with consent, `CLN.ACL.15`); for services that need prior authorisation: a valid authorisation with units left, or self-pay with consent | visit created |
| `reschedule` | requested, booked, confirmed | requested, booked | patient (until N hours before) or `appointments.manage` | new slot free; practitioner licensed at the new time | old slot released; notification sent; a held deposit moves to the new appointment |
| `cancel` | requested, booked, confirmed, checked_in, in_progress | cancelled | patient (until N hours before; never after check-in) or `appointments.manage` | reason code and *initiated by* recorded; after check-in: staff only, with the visit's work done captured or marked none (`SCH.ACL.06`) | slot released; waiting list offered; deposit per `SCH.ACL.05` (clinic-initiated: refunded in full; late patient cancel: forfeited; walk-out: applied to work done, the remainder refunded); work done billed through charge capture |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `CLN.ACL.15` | When coverage fails at check-in or before the service (eligibility negative, card or policy expired, prior authorisation missing, expired or used up), the service starts as self-pay only after the patient signs a self-pay consent showing the estimated price; otherwise the visit is re-split to a valid policy or the appointment is rescheduled | `STA` | receptionist, practitioner | on |

§7.1: "Reason codes are the module's list (wl-scheduling §4.1). 05 adds `coverage_not_confirmed`
(insurance or authorisation failed at check-in and the patient declined self-pay). It counts
as clinic-initiated for the deposit, so the deposit is refunded in full or carried to the new
booking."

§4.2 row: "The receptionist records the result and one coverage outcome on the visit:
**re-split** to another valid policy or an updated card (the patient share is recalculated),
**self-pay** with the patient's signed self-pay consent showing the estimated price
(`CLN.ACL.15`), or **reschedule** so the patient can sort out cover (deposit carried to the new
booking). On a self-pay visit, a service added that is not on the signed consent needs an
updated consent (new signature) before charge capture accepts it; the bill uses the self-pay
price list on the visit date, not the estimate. If eligibility turns positive before `start`,
the receptionist may change the outcome to *insured*; the signed consent stays on file marked
*superseded*. After `start` the outcome is fixed."

Visit fields (§6): "coverage outcome (insured, re-split, self-pay with consent reference)".

#### Edge cases decided
- **Order of steps:** the eligibility result and coverage outcome are recorded on a pre-check-in
  screen; the visit record itself is created by `check_in` and receives them. Nothing is stored
  on the appointment except a link to the pending check-in data.
- **Pending check-in data:** an apps/clinics draft for one appointment holding the eligibility
  result, the outcome, the policy chosen for a re-split and the consent reference. It is moved
  to the visit by `check_in`, and discarded when the appointment is cancelled or rescheduled or
  is not checked in by the end of the appointment day (Asia/Dubai). A consent already signed
  stays in wl-documents on the patient's record, marked *superseded*.
- **Service without a self-pay price:** a service with no self-pay price on the visit date
  cannot be put on the consent; `self_pay` is refused with "no self-pay price for <service>"
  until the clinic manager sets one in the self-pay price list.
- **Card or policy expired:** "expired" means the policy's *valid to* date (or the card's expiry)
  is before the visit date in Asia/Dubai; a policy valid to the visit date is valid all day.
- **Re-split target:** only a policy of the same patient whose dates cover the visit date and
  whose eligibility is recorded positive today; an updated card replaces the old one on the same
  policy (new card image, new valid-to) and needs `patients.sensitive.view` to see (`CLN.ACL.12`).
- **Estimated price changes later:** the consent lists the services expected; a service added during the visit that is not on the signed consent needs an updated
  consent (new signature) before charge capture accepts it on a `self_pay` visit. The final bill
  uses the self-pay price list at the visit date, not the estimate.
- **Eligibility becomes positive after self-pay was chosen (same day, before `start`):**
  the receptionist may change the outcome to `insured` before the
  appointment is `in_progress`; the signed consent stays on file marked *superseded*. After
  `start` the outcome is fixed.
- **Patient declines self-pay at the desk:** two paths only: `reschedule` (business-initiated,
  deposit carried, `SCH.ACL.05`) or `cancel` with `coverage_not_confirmed` (initiated by
  `business`, deposit refunded in full).
- **Non-insured visit:** no eligibility or outcome is asked; the guard applies only when the
  appointment has a policy attached.
- **Concurrent check-in by two receptionists:** the second `check_in` is refused because the
  appointment is no longer `booked`/`confirmed`.
- **Reason and messages:** the patient-facing message for a reschedule or cancel shows no reason
  (§9 "Patient messages after a change").

#### Must not
- Must not run `check_in` on an insured visit without an eligibility result and an outcome.
- Must not accept `self_pay` without a signed consent reference.
- Must not forfeit the deposit on `coverage_not_confirmed`.
- Must not show the visit reason or codes to the receptionist while doing this (`CLN.ACL.02`).

#### Done when
- [x] Given an insured appointment for 2026-11-10 09:00 and eligibility recorded *negative*, when
      receptionist A tries `check_in` without an outcome, it is refused with "coverage outcome
      required".
- [x] Given policy P1 expired 2026-11-09 and policy P2 valid to 2027-03-31 with positive
      eligibility, when A records `re_split` to P2, `check_in` succeeds, the visit shows outcome
      `re_split` with P2 and the patient share is recalculated from P2's contract.
- [x] Given eligibility negative and the patient signs a self-pay consent showing estimated
      price AED 450.00, `check_in` succeeds with outcome `self_pay` and the consent reference.
- [x] `self_pay` without a signed consent is refused.
- [x] Given a service with no self-pay price on 2026-11-10, `self_pay` is refused with "no
      self-pay price for <service>".
- [x] Pending check-in data for an appointment not checked in by 23:59 on its day is discarded.
- [x] Given a deposit of AED 100.00, when A cancels with `coverage_not_confirmed`, initiated by is
      `business` and AED 100.00 is refunded; when A reschedules instead, the AED 100.00 is linked
      to the new appointment.
- [x] On a `self_pay` visit, adding a service not on the consent in charge capture is refused
      until an updated consent is signed.
- [x] Given a policy valid to 2026-11-10 and a visit on 2026-11-10, no outcome other than
      `insured` is asked after a positive eligibility result.

#### Out of scope
Prior authorisation at check-in (BL-CLN-02); the card look-ahead before the day (BL-CLN-04);
licence check at check-in (BL-SCH-08); deposit mechanics (BL-SCH-02).

---

### BL-CLN-02 — 05 Clinics: require a valid prior authorisation with units left at check-in

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-CLN-01 · Base: 05 Prior authorisation lifecycle · **Blocks:** None.
**Review items:** 05-N5-b

#### Goal
A service that needs prior authorisation must not start on an insured visit unless an
authorisation is valid on the visit date and still has units left. Otherwise that service goes
self-pay with consent, or the appointment is rescheduled while the billing coordinator requests
a new authorisation. This prevents claims that are certain to be denied.

#### Surfaces
- **Back office:** at check-in the receptionist enters the units needed (default 1); Check
  in is refused with "prior authorisation required: self-pay with consent or reschedule";
  the self-pay consent lists only the unauthorised service; on reschedule the billing
  coordinator gets a task to request a new authorisation.
- **System:** units left are approved − consumed − units held by other checked-in visits;
  nothing is consumed at check-in.

#### Files to read
- `docs/verticals/05-clinics.md` §3.3 (`CLN.ACL.15`), §4.2 "When it goes wrong" (row "Prior
  authorisation missing, expired or used up before the visit"), §7.1 *Appointment* (`check_in`)
  and *Prior authorisation* (states, `consume`)

#### Terms
- **Prior authorisation:** a payer's approval for named services and a number of units, valid to
  a date; states `approved` or `partially_approved` are usable.
- **Units left:** approved units minus units consumed (`consume` runs from visit charge capture).
- **Units needed:** the units of an authorised service a visit will use, entered by the
  receptionist at check-in (default 1); a checked-in visit holds them against the authorisation.
- **Service that needs authorisation:** the payer contract's prior-auth list contains the
  service code.

#### Spec excerpt (authoritative for this issue)

`check_in` guard (part): "for services that need prior authorisation: a valid authorisation with
units left, or self-pay with consent".

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `consume` | approved, partially_approved | approved, partially_approved, used | system (visit charge capture) | units remaining | `used` when zero; on `used`, later appointments that relied on it are flagged (`CLN.RUL.09`) |

§4.2 row: "Flagged ahead by `CLN.RUL.07` and `CLN.RUL.09`. At check-in, a service that needs
authorisation goes ahead only with a valid authorisation with units left on the visit date, or as
self-pay with consent (`CLN.ACL.15`), or the appointment is rescheduled while a new authorisation
is requested. Units left at check-in = approved − consumed − units of the same authorisation on
other visits already checked in and not yet completed or cancelled. The units a visit needs
for a service are entered at check-in (default 1); a checked-in visit counts those units against
the authorisation."

#### Edge cases decided
- **Refusal:** check-in is refused with "prior authorisation required: self-pay with consent or
  reschedule"; the appointment stays `booked` or `confirmed` and nothing is consumed.
- **Units needed above units left:** the units left are used and the rest goes self-pay with
  consent (as `partially_approved` below), or the appointment is rescheduled.
- **Per service, not per visit:** a visit with a covered consultation and a procedure needing
  authorisation keeps the consultation insured; only the procedure becomes self-pay (consent
  lists only that service).
- **Valid on the visit date:** the authorisation's *valid until* is inclusive, Asia/Dubai.
- **Two visits the same day against one unit:** units left at check-in =
  approved − consumed − units of the same authorisation on other visits already checked in and
  not yet completed or cancelled. The second check-in sees 0 left.
- **Authorisation `requested` but not answered:** not valid; same choice (self-pay or reschedule).
- **`partially_approved` with fewer units than booked:** the units approved are used; extra units
  go self-pay with consent.
- **Reschedule here:** business-initiated reschedule; deposit carried; a task "request new
  authorisation" goes to the billing coordinator (same task type as `CLN.RUL.09`).

#### Must not
- Must not let check-in pass with an expired, denied, used or cancelled authorisation.
- Must not consume units at check-in (consumption stays at charge capture).

#### Done when
- [x] Given a procedure needing authorisation and authorisation A1 `approved` for 3 units, 3
      consumed, when receptionist A checks in with 1 unit needed and no outcome, it is refused
      with "prior authorisation required: self-pay with consent or reschedule" and the
      appointment stays `confirmed`; with a signed self-pay consent for the procedure, check-in
      succeeds.
- [x] Given A1 valid until 2026-11-10 and a visit on 2026-11-11, check-in is refused the same way.
- [x] Given A1 with 1 unit left and visit V1 already checked in with 1 unit needed, a second visit
      V2 the same day sees 0 units left.
- [x] Given A1 with 2 units left and 3 units needed, 2 units stay in the payer split and 1 unit
      goes self-pay with consent.
- [x] With a covered consultation and an unauthorised procedure, the consent lists only the
      procedure and the consultation stays in the payer split.
- [x] On reschedule, the billing coordinator gets a task to request a new authorisation.

#### Out of scope
Look-ahead flags (BL-CLN-03); the coverage outcome mechanics and consent (BL-CLN-01).

---

### BL-CLN-03 — 05 Clinics: flag appointments when a prior authorisation is denied, used up or ends (`CLN.RUL.09`)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: 05 Prior authorisation lifecycle · Base: 05 `CLN.RUL.07` (appointment flag) · **Blocks:** None.
**Review items:** 05-N5-b

#### Goal
Future appointments that rely on a prior authorisation must be flagged as soon as it stops
covering them: when it is denied, used up or expires, and 3 days before its validity ends. The
billing coordinator gets a task to request a new one, and the front desk sees the flag at
check-in. This gives the clinic time to get cover before the patient arrives.

#### Surfaces
- **Back office:** the billing coordinator gets one task per authorisation and trigger
  listing the flagged appointments; the front desk sees the "needs authorisation" flag on
  the appointment at check-in.
- **System:** `CLN.RUL.09` runs on `deny`, `used` and `expire` and daily at 02:00 for the
  3-day look-ahead, flags only appointments with no other valid authorisation, and does
  nothing new on a re-run; a new approval, reschedule or cancel clears or re-evaluates the
  flag.

#### Files to read
- `docs/verticals/05-clinics.md` §7.1 *Prior authorisation* (rows `approve`, `deny`, `consume`,
  `expire`), §7.2 (`CLN.RUL.07`, `CLN.RUL.09`)

#### Terms
- **Relied on it:** a future appointment of the same patient for a service the authorisation
  covers. On `deny` and `used`: every such appointment from today on. On `expire` and on the
  D-days look-ahead: only those dated after *valid until* (appointments inside the validity are
  still covered). In every case only when no other valid authorisation covers its date.
- **Flag:** the same appointment flag `CLN.RUL.07` sets ("needs authorisation"), with a cause.

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `approve` | requested | approved | `insurance.manage` | approval reference and validity recorded | linked appointments unflagged |
| `deny` | requested | denied | `insurance.manage` | reason recorded | patient informed; self-pay option offered; linked future appointments flagged (`CLN.RUL.09`) |
| `consume` | approved, partially_approved | approved, partially_approved, used | system (visit charge capture) | units remaining | `used` when zero; on `used`, later appointments that relied on it are flagged (`CLN.RUL.09`) |
| `expire` | approved, partially_approved | expired | system | validity date passed | appointments after the date that relied on it are flagged (`CLN.RUL.09`) |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CLN.RUL.07` | appointment booked for a service that needs prior authorisation | no `approved` authorisation valid on that date | flag the appointment; notify the billing coordinator | — | on | MVP |
| `CLN.RUL.09` | prior authorisation → `expired`, `used` or `denied`; and D days before its validity ends | a future appointment of the patient for a service it covered, with no other valid authorisation on its date | flag the appointment; task for the billing coordinator to request a new authorisation; the front desk sees the flag at check-in | D = 3 | on | MVP |

#### Edge cases decided
- **D days before:** runs daily at 02:00 Asia/Dubai; fires once per authorisation when today =
  *valid until* − 3 days. It flags only appointments dated after *valid until*.
- **"No other valid authorisation on its date":** another `approved` or `partially_approved`
  authorisation of the same patient and payer covering the service, valid on that date, with
  units left.
- **One task per trigger:** one task per authorisation and trigger, listing all flagged
  appointments; a re-run does not create a second task or flag (idempotent).
- **Unflag:** a new authorisation `approve` unflags the appointments it covers (existing effect);
  a reschedule re-evaluates the flag for the new date; a cancel removes it.
- **Self-pay appointments:** an appointment already marked self-pay for that service is not
  flagged.
- **Message to the patient:** none from this rule; `deny` keeps its existing "patient informed"
  effect, without the clinical reason (§9).

#### Must not
- Must not flag appointments of other patients or for services the authorisation does not cover.
- Must not cancel or reschedule appointments automatically.

#### Done when
- [x] Given authorisation A1 (patient P, laser, valid until 2026-11-30) and P's appointments on
      2026-11-20 and 2026-12-05, when A1 is denied, both are flagged and the coordinator gets one task.
- [x] Given A1 valid until 2026-11-30, on 2026-11-27 at 02:00 the 2026-12-05 appointment is
      flagged; the 2026-11-20 one is not.
- [x] When A1's last unit is consumed on 2026-11-20, A1 is `used` and the 2026-12-05 appointment
      is flagged.
- [x] Given another authorisation A2 valid on 2026-12-05 with units left, nothing is flagged.
- [x] Running the rule twice creates one flag and one task.
- [x] When A3 is approved covering 2026-12-05, the flag is cleared.

#### Out of scope
`CLN.RUL.07` itself (base); the check at check-in (BL-CLN-02).

---

### BL-CLN-04 — 05 Clinics: ask for a new insurance card before an insured appointment (`CLN.RUL.10`)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** Base: 05 Insurance policy · Base: wl-portal document upload · Base: wl-notify messages · **Blocks:** None.
**Review items:** 05-N5-c

#### Goal
Two days before an insured appointment, if the policy on the appointment ends before the visit
date or the card on file has expired, the patient gets a message with a portal link to upload
the new card, and the appointment is flagged for the front desk. Fewer patients arrive with an
expired card.

#### Surfaces
- **Back office:** the front desk sees the `insurance_card` flag on the day list and accepts
  an uploaded card valid on the appointment date, which clears the flag; card images need
  `patients.sensitive.view`.
- **Portal:** the patient (or guardian) follows the link in the message and uploads the new
  insurance card.
- **System:** `CLN.RUL.10` runs daily at 02:00 for appointments in 2 days (at booking for
  later bookings inside 2 days); the message waits for the end of quiet hours.

#### Files to read
- `docs/verticals/05-clinics.md` §7.2 (`CLN.RUL.10`), §9 (Patient messages after a change),
  §3.3 (`CLN.ACL.12`)

#### Terms
- **Card on file:** the latest insurance card image and its expiry date on the policy.
- **Flag:** an appointment flag with cause `insurance_card` shown on the front-desk day list.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CLN.RUL.10` | N days before an insured appointment | the patient's policy ends before the appointment date, or the card on file has expired | send the patient a message with a portal link to upload the new card; flag the appointment for the front desk; the flag clears when staff accept an uploaded card valid on the appointment date (an upload alone does not clear it) | N = 2 | on | MVP |

#### Edge cases decided
- **When:** daily at 02:00 Asia/Dubai for appointments dated today + 2; an appointment booked
  later than that (inside 2 days) is checked at booking.
- **Which policy:** the policy attached to the appointment. Card "expired" = card expiry date
  before the appointment date.
- **Dependants:** the message goes to the guardian when the patient is a linked dependant
  (`PTY.REL.01`).
- **Clearing the flag:** the flag clears when staff accept the uploaded
  card (new valid-to on or after the appointment date). An upload alone does not clear it.
- **Message content:** clinic name, appointment date and time, upload link; no service or
  clinical detail.
- **Quiet hours:** the flag is set at 02:00; the message has normal urgency, so wl-notify defers
  it to the end of quiet hours (08:00 Asia/Dubai by default) and then sends it; it is never
  dropped.
- **Idempotency:** one message and one flag per appointment; rescheduling re-runs the check.

#### Must not
- Must not send the message for self-pay appointments.
- Must not show card images to roles without `patients.sensitive.view` (`CLN.ACL.12`).

#### Done when
- [x] Given an insured appointment on 2026-11-12 and policy valid to 2026-11-10, on 2026-11-10 at
      02:00 the appointment is flagged and the upload message is deferred and sent at 08:00.
- [x] Given a card expiring 2026-11-11 and a policy valid to 2027-06-30, the same happens.
- [x] Given a valid policy and card, nothing happens.
- [x] After staff accept an uploaded card valid to 2027-11-11, the flag is cleared.
- [x] The message text contains no service name.

#### Out of scope
What happens at the desk if the card is still expired (BL-CLN-01).

---

### BL-CLN-05 — 05 Clinics: bill work done when a visit is cancelled after check-in

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SCH-02 · BL-SCH-03 (staff cancel after check-in, work done captured or *none*) · Base: 05 charge capture (`CLN.ENC.01`) · **Blocks:** None.
**Review items:** X-17, 05 MEDIUM "walk-out after check-in", "clinic-side cancellation and deposit refund"

#### Goal
wl-scheduling lets staff cancel a checked-in appointment only after the visit records its work
done. This issue builds the visit side for 05: capture the work done (consumables used, a
service the practitioner records as delivered, or *none*), price it, settle a held deposit
against it, and bill the rest. A walk-out (`walked_out`) is billed as a normal visit and the
deposit remainder is refunded; when the clinic stops the visit, nothing is charged unless the
patient accepts the work done, and the deposit is refunded in full.

#### Surfaces
- **Back office:** before a checked-in appointment is cancelled, the practitioner marks
  services delivered and the receptionist or nurse records consumables issued, or sets
  *none*; the receptionist cancels with `walked_out` or a clinic reason and records the
  patient's acceptance of the work done on a clinic stop.
- **System:** the work done is priced and billed through charge capture; a held deposit
  settles it (walk-out) or is refunded in full (clinic stop); a draft claim is created for a
  payer share.

#### Files to read
- `docs/verticals/05-clinics.md` §3.3 (`SCH.ACL.05`, `SCH.ACL.06`), §4.2 "When it goes wrong"
  (rows "Patient walks out after check-in", "Clinic stops the visit after check-in"), §6
  (Visit: work done), §7.1 *Appointment* (`cancel`), §7.2 settings (walk-out deposit, partial
  price), §13 question 8
- `docs/modules/wl-scheduling.md` §4.1 "Cancel after check-in"

#### Terms
- **Work done:** what the visit records when it stops: consumables issued to the visit and
  services the practitioner marks *delivered*; or *none*.
- **Walk-out:** the patient leaves after check-in; reason `walked_out` (party-initiated).
- **Clinic stop:** a business-initiated reason (`resource_unavailable`, `equipment_failure`,
  `site_closed`, `other` with business initiator).
- **Patient accepts the work done:** the patient agrees, at the desk, to pay for the work done
  on a clinic stop (recorded with the receptionist's name and time).

#### Spec excerpt (authoritative for this issue)

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `SCH.ACL.05` *(shared)* | A booking deposit is forfeited only on a patient's late cancellation or a no-show (a walk-out after check-in applies it to work done and refunds the remainder, §13 question 8); a clinic-initiated cancel or reschedule refunds it in full; waiving a forfeiture needs `bookings.waive_deposit` | `STA` | receptionist, clinic manager | waiver: clinic manager |
| `SCH.ACL.06` *(shared)* | An appointment in `checked_in` or `in_progress` is cancelled only by staff with a reason code, after the visit's work done is captured or marked none; the patient cannot cancel after check-in | `STA` | receptionist, clinic manager | — |

`cancel` row (Appointment): see BL-CLN-01; effects "deposit per `SCH.ACL.05` (clinic-initiated:
refunded in full; late patient cancel: forfeited; walk-out: applied to work done, the remainder
refunded); work done billed through charge capture".

| Situation | What happens |
|---|---|
| Patient walks out after check-in | Staff cancel with reason `walked_out`. Work done (consumables used, a service already delivered) is recorded in charge capture and billed as a normal visit; a held deposit settles it first and the remainder is refunded (`SCH.ACL.05`, `SCH.ACL.06`; §13 question 8: forfeit only on a no-show or late cancellation) |
| Clinic stops the visit after check-in (emergency, equipment failure) | Staff cancel with a business-initiated reason; nothing is charged unless the patient accepts the work done; the deposit is refunded in full. Consumables already issued stay issued (the stock was used); without acceptance the visit records them at price 0 with reason *clinic stop*, and neither the patient nor the payer is charged (no claim is created). With acceptance the work done is billed as a normal visit: the payer split applies and a draft claim is created for a payer share |

| Setting | 05 value |
|---|---|
| Walk-out after check-in: deposit | applied to work done; the remainder refunded (forfeit only on no-show or late cancellation, §13 question 8) |
| Partial price for work done when cancelled after check-in | consumables used at their price; a service only if the practitioner records it as delivered, at its normal price; a package session is consumed only when the package's service is recorded as delivered |

#### Edge cases decided
- **Who captures:** the practitioner marks services delivered (charge capture permission); the
  receptionist or nurse records consumables already issued. *None* can be set by the
  receptionist only when no consumable was issued and no service was marked.
- **Insured walk-out:** "billed as a normal visit": the payer split applies to the delivered
  services and consumables as usual, and a claim draft is created when there is a payer share.
- **Insured clinic stop:** without acceptance nothing is billed to the patient or the payer and
  no claim is created; with acceptance the payer split applies as on a walk-out (claim draft
  for a payer share), and the deposit is still refunded in full.
- **Deposit larger than the patient share:** the patient share is settled from the deposit and
  the remainder is refunded (wl-payments refund, `FIN.ACL.04` SoD). Smaller: the patient pays the
  balance; normal receivable.
- **Clinic stop without acceptance:** consumables issued stay issued from stock (the stock is
  used) but are not charged; the visit records them with price 0 and reason *clinic stop*, so
  stock and cost reports stay correct.
- **Packages:** a package session is consumed only when the practitioner
  records the package's service as delivered.
- **VAT and rounding:** each charged line keeps the service's VAT category; amounts in fils.
- **Appointment already cancelled by another user:** the second cancel is refused; the work done
  is not captured twice.

#### Must not
- Must not forfeit any part of the deposit on a walk-out (05 setting: remainder refunded).
- Must not charge anything on a clinic stop without the recorded patient acceptance.
- Must not bill an insurer for a deposit forfeiture (§13 question 8).
- Must not let the patient cancel after check-in.

#### Done when
- [ ] Given a checked-in appointment with deposit AED 200.00, a filler syringe issued (price AED
      150.00) and no service delivered, when staff cancel with `walked_out`, the visit bills AED
      150.00, the deposit settles it and AED 50.00 is refunded.
- [ ] Given the practitioner marked the consultation (AED 300.00) delivered, a walk-out bills AED
      300.00; the deposit AED 200.00 settles part and AED 100.00 is due from the patient.
- [ ] Given an `in_progress` visit stopped for `equipment_failure` with a consumable issued and no
      acceptance, nothing is charged, the consumable shows price 0, and AED 200.00 is refunded.
- [ ] With acceptance recorded, the same stop bills the consumable and still refunds the deposit
      in full.
- [ ] An insured walk-out with a delivered consultation creates a draft claim for the payer share.
- [ ] An insured visit stopped for `equipment_failure` with a delivered consultation and no
      acceptance creates no claim and no patient charge; with acceptance a draft claim is
      created for the payer share.
- [ ] Setting *none* while a consumable was issued is refused.

#### Out of scope
The booking `cancel` mechanics and deposit engine (BL-SCH-02, BL-SCH-03); procedure not
suitable (BL-CLN-06).

---

### BL-CLN-06 — 05 Clinics: charge only the consultation when the planned procedure is not suitable

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-SCH-02 · Base: 05 charge capture · **Blocks:** None.
**Review items:** X-17 (05 §4.2 row "procedure is not suitable")

#### Goal
When the practitioner sees the patient and decides the booked procedure is not suitable, the
appointment completes normally with only the consultation in charge capture. The procedure is
not charged, and a deposit taken for the procedure is applied to the consultation or refunded.

#### Surfaces
- **Back office:** in charge capture the practitioner ticks "procedure not performed: not
  suitable" and must pick the consultation service given; the receptionist may refund the
  deposit in full instead of applying it.
- **System:** the procedure is not charged, no authorisation unit or package session is
  consumed, and the deposit is applied to the consultation's patient share with the rest
  refunded.

#### Files to read
- `docs/verticals/05-clinics.md` §4.2 "When it goes wrong" (row "Practitioner sees the patient
  and decides the planned procedure is not suitable"), §7.1 *Appointment* (`complete`)
- `docs/modules/wl-scheduling.md` §4.1 reason code `service_not_suitable`

#### Terms
- **Procedure deposit:** the booking deposit of the booked service type.
- **Not suitable:** a clinical decision by the practitioner; the app records only the outcome,
  not the clinical reason.

#### Spec excerpt (authoritative for this issue)

| Situation | What happens |
|---|---|
| Practitioner sees the patient and decides the planned procedure is not suitable | The appointment completes with the consultation in charge capture (the practitioner picks the consultation service they gave from the service catalogue); the procedure is not charged. A procedure deposit is applied to the consultation's patient share first and the remainder refunded; the receptionist may refund it in full instead (consultation covered by the payer, or waived by the clinic). It is never forfeited |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `complete` | in_progress | completed | practitioner | — | charge capture opened; follow-up offered |

#### Edge cases decided
- **Recording:** the practitioner ticks "procedure not performed: not suitable" in charge capture;
  the procedure line is removed from charge capture and the practitioner picks the
  consultation service they gave from the service catalogue (required; none is proposed). Its
  price, VAT category and payer split follow that service as on any visit.
- **Deposit:** the deposit is applied to the consultation's patient share
  first and any remainder is refunded; the receptionist may refund it in full instead (when the
  consultation is covered by the payer or the clinic waives it).
- **Not a cancellation and not a no-show:** the appointment is `completed`; no-show and
  cancellation KPIs are not affected.
- **Prior authorisation:** no unit is consumed for the procedure (consumption follows charge
  capture).
- **Package:** a package session for the procedure is not consumed.

#### Must not
- Must not store the clinical reason in this app.
- Must not forfeit the procedure deposit.

#### Done when
- [ ] Given a booked laser procedure (AED 1,200.00) with deposit AED 300.00, when the practitioner
      records "not suitable" and a consultation (AED 250.00, self-pay), the visit bills AED 250.00,
      the deposit covers it and AED 50.00 is refunded.
- [ ] Recording "not suitable" without picking a consultation service is refused.
- [ ] The appointment is `completed`, not `cancelled`.
- [ ] No prior-authorisation unit and no package session is consumed for the laser.

#### Out of scope
Cancellation after check-in (BL-CLN-05).

---

### BL-CLN-07 — 05 Clinics: return a claim to draft after a technical rejection (`reject_submission`)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: 05 Claim lifecycle (`validate`, `submit`, `resubmit`) · BL-INV-16 (payer bill `submission_rejected`) · **Blocks:** BL-CLN-10
**Review items:** 05-N7-a

#### Goal
The claims channel (eClaimLink/DHPO, Shafafiya, MOHAP) can reject a submission before the payer
looks at it: bad format, missing field, wrong member ID. The billing coordinator records the
rejection, the claim goes back to `draft` with the channel's error codes, is fixed, validated
and submitted again with the same number. This is not a resubmission and does not use one of
the payer's limited resubmissions. In the MVP the coordinator records it by hand; integration
is Later.

#### Surfaces
- **Back office:** the billing coordinator records the channel's rejection on a `submitted`
  or `resubmitted` claim with at least one error code and message, gets a task with the
  errors and the submission deadline, fixes the claim and runs Validate and Submit again
  with the same number.
- **System:** the claim returns to `draft` with the attempt's errors stored, the
  resubmission count unchanged and the payer bill `submission_rejected`; Validate checks the
  window of the rejected attempt.

#### Files to read
- `docs/verticals/05-clinics.md` §4.3 "When it goes wrong" (row "The channel rejects the
  submission for technical reasons"), §6 (Claim), §7.1 *Claim* (states, `submit`,
  `reject_submission`, `resubmit`), §3.3 (`CLN.ACL.09`)
- `docs/modules/wl-invoicing.md` §4 *Payer bill* (`submission_rejected`, vertical mapping)

#### Terms
- **Technical rejection:** the channel refuses the file before adjudication; no remittance.
- **Attempt:** one submission of the claim; each attempt keeps its own errors.
- **Resubmission count / limit:** number of `resubmit` runs; the payer contract limits it (Dubai:
  at most 2).
- **Payer bill:** the money side of the claim in wl-invoicing.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `draft` | neutral | no | Created from charge capture, or returned after a technical rejection |
| `submitted` | info | no | Sent to the payer; locked (`CLN.ACL.09`) |
| `resubmitted` | info | no | Corrected and sent again |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `validate` | draft | ready | `claims.manage` | codes present; prior authorisation linked where required; within the window of the attempt being prepared (resubmission count 0: the first-submission window from the visit date; count above 0, after a technical rejection of a resubmission: the resubmission window from the remittance that denied it), or the payer contract accepts late claims and a late reason is recorded | — |
| `submit` | ready | submitted | `claims.manage` | — | submission reference and date recorded; payer bill `issue` (same number when issued again) |
| `reject_submission` | submitted, resubmitted | draft | `claims.manage` or system (integration, Later) | rejected by the channel before adjudication (format, missing field, member ID); no remittance recorded | channel error codes and messages stored with the attempt; resubmission count unchanged; payer bill `submission_rejected`; task for the billing coordinator with the errors and the submission deadline |
| `resubmit` | partially_paid, denied | resubmitted | `claims.manage` | corrections recorded; within resubmission window and limit | payer bill `resubmit` (count + 1) |

§4.3 row: "Claim `reject_submission` back to `draft` (never to `ready`: it must be validated
again) with the channel's error codes (payer bill `submission_rejected`). It is fixed, validated
and submitted again with the same number; this is not a resubmission and does not count against
the payer's limit. A rejected resubmission is validated against the resubmission window, not the
first-submission window."

#### Edge cases decided
- **Recording by hand (MVP):** the coordinator enters at least one error code and message from
  the channel; free text is allowed for the message.
- **Deadline shown on the task:** for a rejected first submission, the end of the payer's
  first-submission window counted from the visit date; for a rejected resubmission, the end of
  the resubmission window counted from the remittance date that denied it. Asia/Dubai, inclusive.
- **Target state:** always `draft`, never `ready`. The wl-invoicing 05 mapping row already says
  `draft` (wl-invoicing §4, *Vertical mapping*); no alignment step is needed.
- **Window guard after a rejected resubmission:** `validate` checks the
  window of the rejected attempt (resubmission window for a claim whose resubmission count > 0),
  not the first-submission window, so a rejected resubmission can be fixed and sent again.
- **State after sending again:** `submit` always leads to `submitted` (spec row); the
  resubmission count keeps its value, so a claim with count 1 shows `submitted` with count 1.
- **Remittance already recorded:** `reject_submission` is refused.
- **Repeated rejections:** each attempt keeps its errors; the count does not change.
- **Number:** the claim and its payer bill keep their numbers.

#### Must not
- Must not increase the resubmission count on a technical rejection.
- Must not assign a new claim or payer bill number.
- Must not allow `resubmit` beyond the payer's limit or window.

#### Done when
- [x] Given claim CLM-0101 `submitted` on 2026-11-03 for a visit on 2026-11-01, when coordinator A
      records rejection "E104 invalid member ID", the claim is `draft`, the attempt shows E104, the
      count stays 0, payer bill `submission_rejected` ran, and A gets a task with the
      first-submission deadline.
- [x] After fixing, `validate` and `submit` send it again as CLM-0101 with the same payer bill
      number.
- [x] Given a claim with count 1 in `resubmitted`, a technical rejection returns it to `draft`
      with count 1; it can be validated within the resubmission window and submitted again.
- [x] Given a payer limit of 2 and count 2, `resubmit` is refused.
- [x] `reject_submission` on a claim with a recorded remittance is refused.

#### Out of scope
Integrated channel responses (`CLN.CLM.02`, Later); payer bill effects (BL-INV-16).

---

### BL-CLN-08 — 05 Clinics: hold a remittance overpayment and record a payer recoupment on a claim

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: 05 Claim lifecycle (`record_remittance`) · BL-INV-17 (payer credit balance, payer bill `record_recoupment`) · BL-KRN-01 (approval ends without a yes) · **Blocks:** None.
**Review items:** 05-N3-c, X-03

#### Goal
A remittance that pays more than is open on a claim line is held until a second person approves
moving the excess to the payer credit balance. A payer that takes back money paid earlier
(recoupment) reopens the recouped lines of the earlier claim as denied with the payer's code, so
the claim can be resubmitted, moved to the patient or written off. `paid` and `written_off`
claims are no longer final: only `record_recoupment` leaves them. The denial task
(`CLN.RUL.05`) also fires after a recoupment.

#### Surfaces
- **Back office:** the billing coordinator records a remittance or a recoupment and a second
  user approves it from "Needs your action"; the recorder cannot approve; the claim shows
  the recouped lines as denied with the payer's code, and the coordinator gets the denial
  task with the resubmission deadline.
- **System:** a held remittance records nothing until approval; a `PaymentReversed` from
  wl-payments runs `record_recoupment` without approval; the recoupment is set off from the
  payer credit balance first and `CLN.RUL.05` creates the task.

#### Files to read
- `docs/verticals/05-clinics.md` §3.3 (`FIN.ACL.16`), §4.3 "When it goes wrong" (rows
  "Remittance pays more than is open on a line", "Payer recoups money paid earlier"), §7.1
  *Claim* (states `paid`, `partially_paid`, `denied`, `written_off`; rows `record_remittance`,
  `record_recoupment`), §7.2 (`CLN.RUL.05`)
- `docs/modules/wl-invoicing.md` §4 *Payer bill* (`record_recoupment`), §5.8 step 7

#### Terms
- **Remittance advice:** the payer's statement of paid and denied lines.
- **Recoupment:** a negative remittance line taking back money paid on an earlier claim line.
- **Payer credit balance:** money a payer paid beyond what it owes (wl-invoicing / wl-payments).
- **Second person:** any user with the approve permission other than the one recording.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `paid` | success | no | All lines paid as claimed; only `record_recoupment` leaves it |
| `partially_paid` | warning | no | Some lines paid, some denied, reduced or recouped |
| `denied` | error | no | All lines denied or recouped |
| `written_off` | neutral | no | Remaining balance written off or moved to the patient; only `record_recoupment` leaves it |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `record_remittance` | submitted, resubmitted | paid, partially_paid, denied | `claims.manage` or system (integration, Later) | remittance lines matched; an amount above the open line goes to the payer credit balance only with approval (`FIN.ACL.16`) | payments allocated; denial reasons stored; recoupment lines on the same advice run `record_recoupment` on the earlier claims |
| `record_recoupment` | paid, partially_paid, written_off | partially_paid, denied | `claims.manage` or system (remittance import; wl-payments `PaymentReversed`) | recouped amount not above the paid amount; payer's recoupment code; approval by a second person (`FIN.ACL.16`, not for a reversal from wl-payments) | payer bill `record_recoupment`; recouped lines reopen as denied with the payer's code; set off from the payer credit balance first; denial task (`CLN.RUL.05`) |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `FIN.ACL.16` *(shared)* | Moving a payer overpayment to the payer credit balance, or recording a payer recoupment, needs a second person (a reversal from wl-payments is exempt) | `SOD`, `LIM` | billing coordinator, accountant | any amount |

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CLN.RUL.05` | claim → `denied` or `partially_paid`, including after a recoupment (`PayerRecoupmentRecorded`) | — | create a task with the resubmission deadline | — | on | MVP |

#### Edge cases decided
- **Held remittance:** the whole remittance advice waits (nothing recorded) until the approval;
  rejected → nothing recorded and the coordinator gets the comment (BL-KRN-01, as BL-INV-17).
- **Resulting state:** `denied` when every line's paid amount is zero after the recoupment,
  otherwise `partially_paid`.
- **Recoupment of a `written_off` claim:** the recouped lines reopen as denied; the earlier
  write-off amount stays recorded and visible.
- **Resubmission deadline on the task:** counted from the date of the remittance advice that
  carried the recoupment (payer contract resubmission window), Asia/Dubai.
- **Recoupment for a claim not found:** the remittance line is unmatched; the advice cannot be
  recorded until it is matched (guard "remittance lines matched").
- **Same advice imported twice:** refused by remittance reference per payer (as BL-INV-17).
- **Patient balance:** a recoupment never changes the patient share automatically; moving to the
  patient is a later `write_off` decision (BL-CLN-09 rules on transfer).

#### Must not
- Must not let the recorder approve their own overpayment or recoupment.
- Must not recoup more than was paid on the lines.
- Must not leave `paid` or `written_off` by any transition other than `record_recoupment`.

#### Done when
- [x] Given claim line open AED 400.00 and a remittance paying AED 450.00, the advice is held;
      user B approves; AED 400.00 is allocated and AED 50.00 goes to the payer credit balance;
      user A (recorder) cannot approve.
- [x] Given claim CLM-0200 `paid` (one line AED 400.00) and a later advice recouping AED 400.00
      with code "R07" on an advice dated 2026-12-10, and a payer resubmission window of 30
      days, after user B approves CLM-0200 is `denied`, the line shows R07, and the coordinator
      has a task with resubmission deadline 2027-01-09.
- [x] Given a `written_off` claim with a line paid AED 300.00, a recoupment of AED 100.00 makes it
      `partially_paid` and the write-off amount is still shown.
- [x] A `PaymentReversed` for a payment allocated to the claim's payer bill runs
      `record_recoupment` without approval.
- [x] A recoupment of AED 500.00 on lines paid AED 400.00 is refused.
- [x] From `paid`, `write_off` and `resubmit` are not offered.

#### Out of scope
Payer credit balance mechanics and refunds to the payer (BL-INV-17, BL-PAY-08).

---

### BL-CLN-09 — 05 Clinics: close a claim that missed the first-submission window (`CLN.ACL.16`)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** Base: 05 Claim lifecycle · BL-INV-18 (payer bill `write_off` and `transfer_to_customer` from draft) · BL-KRN-01 · **Blocks:** None.
**Review items:** 05-N5-a

#### Goal
A claim still in `draft` or `ready` after the payer's first-submission window is closed with
`write_off` and a reason, with the owner's approval above AED 1,000. The payer share moves to the
patient only when the payer contract allows it, never by default. A payer that accepts late
claims gets the claim with a late reason instead, through `validate`. The amount is reported as
*not submitted in time*.

#### Surfaces
- **Back office:** the clinic manager runs Write off on a `draft` or `ready` claim with a
  reason after the window passed; above AED 1,000 the owner / medical director approves; for
  a payer that accepts late claims Validate asks for a late reason instead; the owner sets
  the payer contract flag "missed-window share may be billed to the patient"; the denial KPI
  and the payer receivables report show *not submitted in time*.
- **System:** the write-off runs payer bill `write_off`, or `transfer_to_customer` only with
  the contract flag on; an amount change while the approval waits fails the request.

#### Files to read
- `docs/verticals/05-clinics.md` §3.3 (`FIN.ACL.14`, `CLN.ACL.16`), §4.3 "When it goes wrong"
  (row "First-submission window missed"), §7.1 *Claim* (`validate`, `write_off`), §9 (Missed
  submission windows), §2 (claim deadlines)
- `docs/modules/wl-invoicing.md` §4 *Payer bill* (`write_off`, `transfer_to_customer`)
- `docs/OPEN-ITEMS.md` L-34, AU-17

#### Terms
- **First-submission window:** days after the visit in which the payer accepts a first claim; a
  parameter per payer contract.
- **Accepts late claims:** a payer contract flag.
- **Late reason:** text recorded on the claim when sending it after the window to a payer that
  accepts late claims.
- **Move to the patient:** payer bill `transfer_to_customer`: the payer share becomes patient
  share on the same tax invoice (05: the patient is the VAT recipient).

#### Spec excerpt (authoritative for this issue)

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `validate` | draft | ready | `claims.manage` | codes present; prior authorisation linked where required; within the window of the attempt being prepared (resubmission count 0: the first-submission window from the visit date; count above 0, after a technical rejection of a resubmission: the resubmission window from the remittance that denied it), or the payer contract accepts late claims and a late reason is recorded | — |
| `write_off` | draft, ready, partially_paid, denied | written_off | `claims.writeoff` (clinic manager) | reason given; above the limit needs the owner (`FIN.ACL.14`); from `draft` or `ready` only after the first-submission window passed and the payer does not accept late claims (`CLN.ACL.16`) | balance moved to the patient where the contract allows, otherwise written off (payer bill `transfer_to_customer` or `write_off`); a missed window is reported in the denial KPI as *not submitted in time* |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `FIN.ACL.14` *(shared)* | Writing off the open amount of a claim above a limit needs the owner's approval | `LIM` | clinic manager | AED 1,000 |
| `CLN.ACL.16` | A claim that missed the payer's first-submission window is closed (`write_off`) with a reason; its payer share is moved to the patient only when the payer contract allows it, never by default | `STA`, `LIM` | billing coordinator, clinic manager | moving to the patient: off `⚠ open: lawyer (L-34)` |

§9: "Whether a patient may be billed for a payer share the clinic did not submit in time depends
on the payer contract and the emirate's rules; off by default (`CLN.ACL.16`) `⚠ open: lawyer
(L-34)`." §2: first-submission windows `⚠ open: authority (AU-17)`; "Windows are parameters per
payer."

#### Edge cases decided
- **Window passed:** counted in days from the visit date, Asia/Dubai; "passed" means today is
  after the last day (as BL-INV-18).
- **Contract flag for moving to the patient:** a payer contract setting "missed-window share may
  be billed to the patient", default off and editable only by the owner; with it off, `write_off`
  always writes off. The open part of L-34 is flagged; the default (off) may be built.
- **Approval:** above AED 1,000 an approval request goes to the owner / medical director; the
  claim stays in its state while waiting; a rejection leaves it unchanged (BL-KRN-01). If the
  amount changes while waiting, the kernel fails the request.
- **Late claim to a payer that accepts late claims:** `write_off` from `draft`/`ready` is refused;
  `validate` asks for the late reason and passes.
- **KPI:** *not submitted in time* is a reason value on the denial KPI and on the payer
  receivables report, filterable.
- **Patient told:** when the share moves to the patient, the patient is notified by the payer bill
  effect (module); the message shows the amount, not the codes.

#### Must not
- Must not move the payer share to the patient unless the contract flag is on.
- Must not write off from `draft`/`ready` while the window is open, or when the payer accepts late
  claims.
- Must not let the clinic manager approve above AED 1,000.

#### Done when
- [x] Given a draft claim AED 600.00 for a visit on 2026-09-01, a 30-day window (ended 2026-10-01),
      a payer that does not accept late claims and the flag off, when clinic manager A writes it
      off with reason "missed window" on 2026-10-05, it is `written_off`, payer bill `write_off`
      ran and the KPI shows AED 600.00 *not submitted in time*.
- [x] With the flag on, the same action runs `transfer_to_customer` and the patient share rises by
      AED 600.00.
- [x] AED 1,500.00 creates an approval request to the owner; the claim stays `draft` until approved.
- [x] On 2026-09-30 (window open) `write_off` from `draft` is refused.
- [x] For a payer that accepts late claims, `write_off` from `draft` is refused and `validate`
      without a late reason is refused; with "system outage" it passes.

#### Out of scope
Alerts before the window ends (BL-CLN-11); payer bill mechanics (BL-INV-18).

---

### BL-CLN-10 — 05 Clinics: void a claim created in error (`CLN.ACL.14`)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:medium` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** Base: 05 Claim lifecycle · BL-INV-18 (payer bill `void`) · BL-KRN-03 (void pattern) · BL-CLN-07 · **Blocks:** None.
**Review items:** 05 MEDIUM "void a claim", X-19

#### Goal
A claim created by mistake (duplicate visit, wrong payer or policy) is voided from `draft` or
`ready`, before it was ever sent. The number is kept and shown as void, its payer bill is voided
too (number kept), the visit's payer split is removed, and the visit is re-split to the right
policy, which creates a new claim (or, with no right policy, is flagged *coverage to resolve*).
A claim that was ever submitted is never voided.

#### Surfaces
- **Back office:** the billing coordinator runs Void on a `draft` or `ready` claim with a
  reason code; *duplicate* asks for the duplicated claim from a list of the same patient's
  other claims; the voided claim stays in lists marked void; with no right policy the
  coordinator gets a "coverage to resolve" task.
- **System:** Void is refused for a claim with any stored submission attempt; the payer bill
  is voided, the payer split removed and the visit re-split to the right policy with a new
  claim.

#### Files to read
- `docs/verticals/05-clinics.md` §3.3 (`CLN.ACL.14`), §4.3 "When it goes wrong" (row "Claim
  created in error"), §7.1 *Claim* (`voided`, `void`)
- `docs/modules/wl-kernel.md` §5.12 (void), `PLT.ACL.09`

#### Terms
- **Void:** the platform pattern for a record entered in error: number kept, record shown as
  void, never deleted.

#### Spec excerpt (authoritative for this issue)

| State | Tone | Terminal | Meaning |
|---|---|---|---|
| `voided` | neutral | yes | Created in error before submission; never sent |

| Transition | From | To | By | Guards | Effects |
|---|---|---|---|---|---|
| `void` | draft, ready | voided | `claims.manage` | reason given (duplicate, wrong payer or policy); never after `submit`: a claim with any recorded submission attempt is refused (`CLN.ACL.14`, `PLT.ACL.09`) | payer split removed from the visit; its payer bill runs `void` (`voided`, number kept); the visit is re-split to the right policy when there is one (new claim), otherwise flagged *coverage to resolve* with a task for the billing coordinator; number kept, shown as void |

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|
| `CLN.ACL.14` | A claim is voided only from `draft` or `ready` (entered in error: duplicate visit, wrong payer or policy); a submitted claim is never voided, only resubmitted or written off (`PLT.ACL.09`) | `STA` | billing coordinator | — |

#### Edge cases decided
- **Payer bill:** runs its own `void` → `voided` (number kept), as the wl-invoicing module
  defines; it is never deleted.
- **Claim back in `draft` after a technical rejection:** it was submitted, so `void` is refused
  (stored attempts prove it).
- **No right policy:** the visit is left without a payer split and flagged
  "coverage to resolve" with a task for the billing coordinator; no amount moves to the patient
  automatically.
- **Duplicate visit:** the void reason `duplicate` needs the claim it duplicates, picked from a
  list of the same patient's other claims that are not `voided`; the link is shown on both
  claims. A claim of another patient cannot be picked.
- **Reason codes:** `duplicate`, `wrong_payer`, `wrong_policy`; free text optional.

#### Must not
- Must not delete a claim or reuse its number.
- Must not void a claim with any recorded submission attempt.

#### Done when
- [x] Given claim CLM-0300 `ready` for visit V1 with payer X, when coordinator A voids it with
      `wrong_payer` and V1 is re-split to payer Y, CLM-0300 is `voided`, its payer bill is
      `voided`, and a new claim exists for payer Y.
- [x] Given CLM-0301 back in `draft` after a technical rejection, `void` is refused.
- [x] Given no other policy, V1 has no payer split and the coordinator gets a "coverage to
      resolve" task.
- [x] CLM-0300 still appears in lists, marked void.
- [x] Voiding CLM-0302 with `duplicate` without picking a claim is refused; the picker lists only
      the same patient's claims that are not `voided`.

#### Out of scope
Voiding payments, receipts and appointments (their modules).

---

### BL-CLN-11 — 05 Clinics: warn before the first-submission window ends (`CLN.RUL.03`)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** Base: 05 Claim lifecycle · BL-KRN-08 (owner alert for compliance-critical rules) · **Blocks:** None.
**Review items:** 05-N5-a

#### Goal
Claims left in `draft` or `ready` are chased twice: the billing coordinator is told 7 days after
the visit, and the clinic manager gets a task 2 days before the payer's window ends. The rule is
compliance-critical: if it fails or misses its run, the owner / medical director is alerted.

#### Surfaces
- **Back office:** the clinic manager gets a task 2 days before the payer's window ends;
  saving a payer contract with N not below its window is refused.
- **System:** `CLN.RUL.03` runs daily at 02:00, notifies the billing coordinator at N days
  and fires once per claim and step; a failed or missed run alerts the owner / medical
  director, and the client cannot switch it off.

#### Files to read
- `docs/verticals/05-clinics.md` §4.3 "When it goes wrong" (row "First-submission window about to
  pass"), §7.2 (`CLN.RUL.03`, compliance-critical list)
- `docs/modules/wl-kernel.md` `PLT.RUL.04`

#### Terms
- **Compliance-critical rule:** a rule whose failure or missed run alerts the business owner
  (`PLT.RUL.04`).

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CLN.RUL.03` | N days after a visit with an insurer share; and D days before the payer's first-submission window ends | claim still `draft` or `ready` | at N: notify the billing coordinator; at D: task for the clinic manager | N = 7 (parameter per payer; a payer contract with N not below its submission window is refused; N = 0 is allowed, for windows of 1 day, and notifies on the first run after the visit); D = 2 | on | MVP |

"Compliance-critical (owner alert through `PLT.RUL.04` to the owner / medical director when they
fail or miss their schedule): `CLN.RUL.01`, `SCH.RUL.05`, `CLN.RUL.03` (payer submission windows)
and `DOC.RUL.03` for practitioner licences and the facility licence."

#### Edge cases decided
- **Run time:** daily at 02:00 Asia/Dubai. At N: visit date + 7 = today. At D: window last day − 2
  = today.
- **N not below the window:** saving a payer contract with N ≥ the window
  is refused with "N must be below the submission window". For a 1-day window the contract sets
  N = 0; at N = 0 the notification goes on the first run after the visit date.
- **Missing window on a payer contract:** the window is a required field; a contract without
  it cannot be activated (§6 Payer contract).
- **Window shorter than D (e.g. DoH 24-hour rule, AU-17):** the D task fires on the first run on or
  after visit date when window − 2 ≤ 0, i.e. at once.
- **Claim back in `draft` after a technical rejection:** still counted; the D task can fire.
- **Idempotency:** one notification at N and one task at D per claim; reruns skip.

#### Must not
- Must not fire for claims in `submitted` or later.
- Must not be switchable off by the client (compliance-critical, set by us).

#### Done when
- [x] Given a visit on 2026-11-01 with a draft claim and a 30-day window, on 2026-11-08 the
      coordinator is notified; on 2026-11-29 the clinic manager gets a task.
- [x] A claim submitted on 2026-11-05 gets neither.
- [x] Saving a payer contract with a 30-day window and N = 30 is refused.
- [x] Given a payer contract with a 1-day window and N = 0, and a visit on 2026-11-01, the
      2026-11-02 02:00 run notifies the coordinator and creates the clinic manager task.
- [x] Given the 2026-11-08 02:00 run ends `failed` (test: a forced error in the rule job), the owner
      / medical director gets the owner alert at once; given no run exists for 02:00 (test: the
      scheduler paused until 03:00), the alert comes at the 03:00 missed-run check (BL-KRN-08).

#### Out of scope
Closing after the window (BL-CLN-09).

---

### BL-CLN-12 — 05 Clinics: block bookings and alert the owner when a practitioner licence expires (`CLN.RUL.01`)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:s` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** BL-HRM-01 (`qualification_valid`, `QualificationExpired`) · BL-DOC-05 (`DocumentExpired`) · BL-SCH-07 (`SCH.RUL.05`) · BL-KRN-08 · **Blocks:** None.
**Review items:** 05-N8-a, X-13

#### Goal
On the day after a practitioner's licence expires, new bookings for that practitioner and
branch are refused, the clinic manager and the owner / medical director are told, and every
appointment already booked is in conflict through `SCH.RUL.05`. The rule is compliance-critical.

#### Surfaces
- **Back office:** a new booking for the practitioner at that branch is refused with no
  override; the appointments already booked show a conflict.
- **System:** 05 reacts to `QualificationExpired` with one combined message per recipient
  (clinic manager of the branch, owner / medical director) naming the number of affected
  appointments, and `SCH.RUL.05` raises the conflicts; a failed run raises the owner alert.

#### Files to read
- `docs/verticals/05-clinics.md` §4.5 (paragraph on blocking document kinds), §7.2 (`CLN.RUL.01`,
  compliance-critical list)
- `docs/modules/wl-hr.md` `HRM.RUL.10`; `docs/modules/wl-documents.md` `DOC.RUL.03`
- `docs/OPEN-ITEMS.md` L-19

#### Terms
- **Practitioner licence / branch privileges:** document kinds of the practitioner (wl-hr
  `HRM.QUA.01`) that block `booking.book`, `booking.check_in` and `booking.start`.
- **Expiry day:** the document is valid through its expiry date; it is expired from the next day.

#### Spec excerpt (authoritative for this issue)

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|
| `CLN.RUL.01` *(generalised by `HRM.RUL.10`)* | `QualificationExpired` (wl-hr `HRM.RUL.10`) for a practitioner licence or branch privileges | — | no second expiry scan: the block on new bookings for that practitioner and branch is `HRM.RUL.10`'s (`qualification_valid` = no); 05 sends one combined message per recipient (clinic manager of the branch, owner / medical director) naming the practitioner, the branch and the number of affected appointments, and no second owner message for the same expiry (`DOC.RUL.03`); appointments already booked are in conflict through `SCH.RUL.05` | — | on | MVP |

§4.5: "The practitioner licence and branch privileges are document kinds that block
`booking.book`, `booking.check_in` and `booking.start` (mode `block`) with override mode `never`,
as `SCH.ACL.04` already requires; recording them as legal blocks in the platform is `⚠ open:
lawyer (L-19)` (wl-documents §10). They are compliance-critical: the owner / medical director is
alerted when they expire with appointments affected (`DOC.RUL.03`)."

#### Edge cases decided
- **Implementation:** 05 reacts to `QualificationExpired` (wl-hr `HRM.RUL.10`, from `CLN.RUL.01`);
  it does not run a second expiry scan. The block itself is `qualification_valid` = no.
- **One message per person:** `DOC.RUL.03` also tells the owner for compliance-critical kinds;
  05 sends one combined message per recipient (clinic manager of the
  branch, owner / medical director) listing the practitioner, branch and number of affected
  appointments; no second owner message for the same expiry.
- **Privileges at one branch only:** the block is per branch; other branches stay bookable.
- **Renewal accepted:** bookings open again from the new valid date (BL-DOC-05); conflicts clear
  through `SCH.RUL.05`.
- **L-19 open:** build override mode `never` as specified; the open item only concerns whether
  the law makes it a legal block.

#### Must not
- Must not cancel booked appointments automatically.
- Must not offer an override of the block.

#### Done when
- [x] Given Dr K's DHA licence expiring 2026-11-30 and 4 appointments in December, on 2026-12-01
      a new booking for Dr K at Branch 1 is refused, the clinic manager of Branch 1 and the owner
      each get one message listing 4 affected appointments, and the 4 appointments have a conflict.
- [x] On 2026-11-30 Dr K can still be checked in.
- [x] Given privileges expired only at Branch 2, Branch 1 bookings still work.
- [x] If the run fails, the owner alert (`PLT.RUL.04`) is raised.

#### Out of scope
Look-ahead 30 days before expiry (BL-SCH-07); check-in re-check (BL-SCH-08).

---

### BL-CLN-13 — 05 Clinics: record the lot of every consumable issued to a visit

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:s` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** s
**Depends on:** Base: wl-inventory Lot and stock movements (`STK.MOV.01`, `STK.LOT.01`) · BL-STK-11 (held stock cannot be issued) · BL-STK-18 (pending issues) · **Blocks:** BL-CLN-14
**Review items:** 05-N10-a, X-09

#### Goal
A recall can only find patients if every lot-tracked consumable issued to a visit records its
lot. The issue screen proposes the lot with the earliest expiry, the user scans or picks it, and
the visit cannot be checked out with a lot-tracked line that has no lot, except a pending issue
whose lot is assigned when stock arrives.

#### Surfaces
- **Back office:** the issue screen proposes the lot with the earliest expiry and the user
  scans or picks it; check-out is refused with "lot required" for a lot-tracked line without
  a lot; a pending issue shows "lot uncertain"; a wrong lot is edited before check-out.
- **System:** a quantity larger than one lot is split per lot, earliest expiry first, and
  only the remaining short quantity becomes a pending issue; held or expired lots are
  refused.

#### Files to read
- `docs/verticals/05-clinics.md` §4.6 (step 2 and "When it goes wrong" row "Not enough stock"),
  §6 (Visit: consumables with lot)
- `docs/modules/wl-inventory.md` §3 (pending issue), §4 *Lot*

#### Terms
- **Lot-tracked item:** an item with lot tracking on (injectables, fillers, dental materials).
- **Pending issue:** the short quantity of an automatic consumption at a `block` location; posts
  when stock arrives, lot *assigned on arrival*.

#### Spec excerpt (authoritative for this issue)

§4.6 step 2: "Items used in a visit are issued from the branch store (`STK.MOV.01`). Every issue
of a lot-tracked item to a visit records its lot (scanned or picked; the earliest expiry is
proposed). The visit cannot be completed with a lot-tracked line that has no lot, except as a
pending issue (below). A wrong lot is corrected by editing the line before check-out; after
check-out, by a reversing movement and a new issue (both audited), never by editing the posted
movement."

| Situation | What happens |
|---|---|
| Not enough stock at the branch store for a visit's consumables | Every branch store uses the negative stock policy `block`. The visit is not blocked: the short quantity becomes a *pending issue* that posts when stock arrives, with the lot *assigned on arrival*; `STK.RUL.06` tells the storekeeper and, after 24 hours, the clinic manager |

#### Edge cases decided
- **"Visit completed":** the visit check-out (and the appointment `complete`) is refused while a
  lot-tracked line has neither a lot nor a pending issue.
- **Proposed lot:** earliest expiry among lots at the branch store that are not expired, not on
  hold and have quantity; an expired or held lot scanned by hand is refused.
- **Split across lots:** a quantity larger than one lot's balance is split into one line per lot,
  earliest expiry first; every available lot is used before any short quantity goes pending.
- **Correction of a wrong lot:** before check-out, the line is edited; after
  check-out, a reversing movement and a new issue (both audited), never an edit of the posted
  movement.
- **Pending issue:** the line shows "lot uncertain" until the lot is assigned on arrival.

#### Must not
- Must not issue from a held or expired lot.
- Must not allow check-out with a lot-tracked line without lot or pending issue.

#### Done when
- [ ] Given filler lots L1 (expiry 2027-01-31) and L2 (2027-03-31) at Branch 1, the issue screen
      proposes L1.
- [ ] A line for filler without a lot blocks check-out with "lot required".
- [ ] Given 1 unit at the store and 2 used, one line posts with L1 and 1 unit becomes a pending
      issue shown as "lot uncertain"; check-out succeeds.
- [ ] Given L1 with 1 unit and L2 with 1 unit and 3 used, lines post 1 unit from L1 and 1 unit
      from L2, and 1 unit becomes a pending issue.
- [ ] Scanning held lot L3 is refused.

#### Out of scope
Pending-issue mechanics and alerts (BL-STK-18, BL-STK-20); recalls (BL-CLN-14).

---

### BL-CLN-14 — 05 Clinics: trace a recalled lot to patients and send neutral notices

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-CLN-13 · BL-STK-14 · BL-STK-15 (`TracePartyResolver`) · BL-STK-16 (notices, close) · BL-NTF-02 (give-up task) · **Blocks:** None.
**Review items:** 05-N10-a, X-09, X-20

#### Goal
When a supplier, manufacturer or regulator recalls an injectable, filler or dental material,
the clinic must find every patient who received it. 05 provides the recall trace's party
resolver (visit → patient), the patient notice that names the clinic and asks the patient to
call without clinical detail, and the call task for the front desk when a notice cannot be
delivered. Whether the health authority must be told is open (AU-20).

#### Surfaces
- **Back office:** the clinic manager or medical director sees the recall trace with patient
  name, visit date and branch only (each view audited), including *possible* lines marked
  "lot uncertain"; the clinic manager sends the notices; the front desk of the visit's
  branch gets a call task when a notice gives up and records the call outcome.
- **System:** the `TracePartyResolver` maps visit → patient (the guardian for a dependant);
  one notice per patient per recall, with no product name or clinical detail.

#### Files to read
- `docs/verticals/05-clinics.md` §4.6 "When it goes wrong" (row "Supplier, manufacturer or
  regulator recalls a lot"), §7.2 settings (Recall owner), §8 (last paragraph: give-up), §9
  (Patient messages after a change; Recalls), §3.3 (`CLN.ACL.01`, `STK.ACL.05`)
- `docs/modules/wl-inventory.md` §5.7 (recall), `TracePartyResolver`
- `docs/OPEN-ITEMS.md` AU-20

#### Terms
- **`TracePartyResolver`:** extension a vertical implements to name the party behind a subject in
  a recall trace (05: visit → patient).
- **Affected line:** one subject + party in the recall trace, with a notice status.
- **Give-up:** wl-notify could not deliver the message on any channel (`MessageGaveUp`).

#### Spec excerpt (authoritative for this issue)

| Situation | What happens |
|---|---|
| Supplier, manufacturer or regulator recalls a lot (an injectable, a filler, a dental material) | The clinic manager or medical director opens a recall (wl-inventory §5.7): every lot in scope is held at every branch. The trace lists each visit that used the lot and its patient (`TracePartyResolver`: visit → patient); pending issues show "lot uncertain" (a visit's pending issue of the recalled item whose lot is not yet assigned is listed as *possible* when it was recorded while a lot in scope had a balance at that branch store, from the lot's first receipt or transfer into the store until its balance there reached zero or it was held). Notices to patients name the clinic and ask them to call, never the product's clinical details in the message body; a notice that cannot be delivered becomes a call task for the front desk. Held lots are discarded, returned to the supplier or released (second person, `STK.ACL.05`). Whether the clinic must notify DHA, DoH or MOHAP, and by when, is `⚠ open: authority (AU-20)` (wl-inventory §10) |

| Setting | 05 value |
|---|---|
| Recall owner | owner / medical director; notices sent by the clinic manager |

§8: "A reminder that gives up (`MessageGaveUp`) needs no action; a recall notice that gives up
becomes a call task for the front desk."

#### Edge cases decided
- **Resolver:** visit → patient; when the patient is a dependant, the notice goes to the guardian
  (`PTY.REL.01`) and the line names the patient.
- **Pending issues:** a visit with an open pending issue of the recalled item (lot not yet
  assigned) is listed as *possible* with "lot uncertain" when the pending issue was recorded at a
  branch store while a lot in scope had a balance there: from the lot's first receipt or
  transfer into that store until its balance there reached zero or it was held. A pending issue
  later assigned a lot in scope is listed like any other issue of that lot.
- **Health data:** the trace list shows patient name, visit date and branch only, never visit
  reasons or codes; each view is audited (`CLN.ACL.01`).
- **Notice text:** EN/AR patient template: clinic name, phone number, "please call us about a
  product used in your visit on <date>"; no product name, no clinical detail.
- **Call task:** due 1 working day (wl-notify default), assigned to the front desk of the visit's
  branch; closing the task records the call outcome and marks the line `sent`.
- **Duplicates:** one notice per patient per recall, listing all visit dates.
- **AU-20 open:** no authority notification is built; the recall report can be exported.

#### Must not
- Must not put the product name or clinical detail in the message body.
- Must not show the trace list to roles without health data access.

#### Done when
- [ ] Given lot L9 issued to visits V1 (patient P1, 2026-10-02) and V2 (P2, 2026-10-09), opening a
      recall lists V1/P1 and V2/P2.
- [ ] Given L9 received at Branch 1 on 2026-10-01 and held by the recall on 2026-10-12, a pending
      issue of the item at Branch 1 on 2026-10-05 for P3 is listed *possible*, "lot uncertain";
      one on 2026-09-28 for P5 is not listed.
- [ ] P1 gets one notice with the clinic name and phone and no product name.
- [ ] When P2's notice gives up, the Branch 1 front desk gets a call task due next working day.
- [ ] For dependant P4 the notice goes to the guardian.
- [ ] A receptionist without health data access cannot open the trace list.

#### Out of scope
Recall opening, holds and decisions (BL-STK-14, BL-STK-12, BL-STK-16); authority notification
(AU-20).

---

### BL-CLN-15 — 05 Clinics: configure shared negative-flow settings (bookings, licences, alerts, connections, messages)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` `surface:back-office` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-SCH-01 to BL-SCH-10 · BL-DOC-01 · BL-DOC-06 · BL-KRN-05 · BL-KRN-08 · BL-NTF-01 · BL-NTF-02 · BL-SAL-19 · BL-SAL-20 · BL-SAL-21 · BL-SAL-22 · BL-SAL-23 · BL-SAL-24 · BL-SAL-25 · BL-SAL-26 · BL-NTF-07 · BL-NTF-08 · BL-NTF-09 · BL-HRM-26 · BL-HRM-27 · BL-HRM-28 · BL-DOC-08 · BL-DOC-09 · BL-DOC-10 · **Blocks:** None.
**Review items:** X-13, X-17, X-19, X-20, 05-N8-a, 05 MEDIUM "practitioner sick on the day", "clinic-side cancellation and deposit refund"

#### Goal
Seed 05's values for the shared booking, document, kernel and notification mechanisms, so that
the module behaviour runs with the clinic rules: practitioner licences as blocking kinds with no
override, the eligibility look-ahead and conflict escalation, deposit outcomes, the
compliance-critical rules, the connection fallbacks, the give-up reactions, and patient change
messages without reasons.
It also seeds the 05 values for lead sources and campaigns (`SAL.CRM.02`), loyalty
(`SAL.LOY.01`), onboarding and SOPs (`HRM.ONB.01`) and registration kinds (`DOC.EXP.01`), added
on 2026-10-16: `SAL.RUL.07`, `SAL.RUL.08`, `SAL.ACL.07`, `NTF.RUL.02`, `NTF.ACL.03`,
`HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`, `DOC.RUL.04` and `DOC.RUL.01`. Campaigns that promote
services keep the advertising permit guard (`CLN.ACL.13`).

#### Surfaces
- **Back office:** in the 05 deployment the licence kinds show no override action; with SMS
  `down` the front desk sees the 24-hour call list per branch with a *called* tick; with the
  claims channel `down` the claim screen shows "submit through the payer portal and record
  the reference"; check-in, check-out, cash payments and consumables accept a business time
  with the late-entry flag.
- **System:** values are seeded from the 05 deployment template, with the override mode
  `never` and the compliance-critical rules locked; give-up reactions, `SCH.RUL.05`,
  `SCH.RUL.06` and the loyalty and marketing rules run with the 05 values.

#### Files to read
- `docs/verticals/05-clinics.md` §3.3 (`SCH.ACL.04`, `SCH.ACL.05`, `SCH.ACL.06`), §4.1 "When it
  goes wrong", §4.5, §7.2 (compliance-critical list and settings table), §8 ("When a connection
  is down"), §9 (Patient messages after a change)
- `docs/modules/wl-scheduling.md` §7; `docs/modules/wl-documents.md` §7; `docs/modules/wl-kernel.md`
  §7; `docs/modules/wl-notify.md` §5.3, §5.4, §7
- `docs/modules/wl-sales.md` §4.6, §4.7, §5.8, §5.10 and §7 (campaigns, sources, loyalty,
  segments); `docs/modules/wl-hr.md` §5.8 and §7 (onboarding); `docs/modules/wl-documents.md` §3,
  §5.7 and §7 (SOPs, registration kinds)

#### Terms
- **Seeded setting:** the value installed for a 05 deployment; the module provides the mechanism.

#### Spec excerpt (authoritative for this issue)

| Setting | 05 value | Spec reference |
|---|---|---|
| Licence kinds per service type checked by `SCH.RUL.05` and at `check_in` / `start` | practitioner licence (DHA, DoH, Sharjah Health Authority or MOHAP) and branch privileges for every service type | §7.2 settings |
| `blocks` for those kinds | `booking.book`, `booking.check_in`, `booking.start`, mode `block` | §4.5 |
| Override mode for those kinds | `never` (`⚠ open: lawyer (L-19)`) | §4.5 |
| Compliance-critical document kinds (`DOC.RUL.03`) | practitioner licences, facility licence | §7.2 |
| Eligibility look-ahead D (`SCH.RUL.05`) / conflict escalation H (`SCH.RUL.06`) | 30 days / 2 hours | §7.2 settings |
| Walk-out after check-in: deposit | applied to work done; the remainder refunded | §7.2 settings |
| Partial price for work done when cancelled after check-in | consumables used at their price; a service only if the practitioner records it as delivered, at its normal price | §7.2 settings |
| Deposit waiver permission (`SCH.ACL.05`) | clinic manager (`bookings.waive_deposit`) | §3.3 |
| Reason code added | `coverage_not_confirmed`, initiated by `business` | §7.1 |
| Reason code for lapsed licence or privileges | `resource_ineligible` | §7.1 |
| Booking deposit per service type | off by default; per service type when the clinic enables it; forfeit window N hours | §13 question 8 |
| Compliance-critical rules (owner alert, `PLT.RUL.04`) | `CLN.RUL.01`, `SCH.RUL.05`, `CLN.RUL.03`, `DOC.RUL.03` (practitioner licences, facility licence) | §7.2 |
| Owner alert recipients | owner / medical director | §7.2 |
| Records that support `void` | claim (`draft`, `ready`); payments, receipts and appointments follow their module | §7.2 settings |
| Late entry after an outage (business time) | check-in and check-out time, cash payments, consumables issued (paper sheets entered later) | §7.2 settings |
| Connection fallback: payment gateway | take the deposit or payment on the card terminal or in cash and record it | §8 |
| Connection fallback: SMS or WhatsApp | the front desk calls patients from a list per branch of `booked` and `confirmed` appointments starting in the next 24 hours from the moment the connection went `down`, with a *called* tick per row | §8 |
| Connection fallback: claims channel | submit through the payer's portal and record the reference | §8 |
| Give-up reaction: appointment reminder | none | §8 |
| Give-up reaction: recall notice | call task for the front desk | §8 |
| Patient change messages (reassignment, clinic cancellation, recall) | clinic, time and link or phone only; never the reason or clinical detail | §9 |
| Responsible role per document kind (wl-documents §5.2) | practitioner licence and branch privileges: clinic manager of the branch; facility licence: owner / medical director; patient Emirates ID and insurance card: receptionist, with the patient (or guardian) notified as the party | §7.2 settings |
| Notification types and groups (wl-notify §5.4) | `clinic.card_upload_request`: *Portal invitations, document requests*, task to the receptionist; `clinic.recall_invite` and `clinic.prior_auth_denied`: *Booking confirmations and reminders*; `clinic.product_recall_notice`: no group, urgent (call task for the front desk); dependants: other guardian first | §7.2 |
| Lead sources used (`SAL.CRM.02`) | referral, walk-in, website, social, paid ads, aggregator (booking platforms), partner (insurers, corporates), event, other; on leads and patients; a source is required, and a campaign is required for *paid ads* and *social* | §5; §7.2 settings |
| Campaign channels (`SAL.CRM.02`, `SAL.ACL.07`, `CLN.ACL.13`) | Instagram, TikTok, search ads, SMS, WhatsApp, email; converted value: the patient's first paid visit; a cost entry that takes a campaign over its budget needs a manager (`SAL.ACL.07`: on); a campaign that promotes services needs the advertising permit before it is activated and before a retention broadcast starts (`CLN.ACL.13`, `NTF.ACL.04`), and is ended by the system on the day the permit's validity ends; whether campaign content needs health authority approval beyond the permit is outside the platform (`⚠ open: lawyer (L-38)`) | §5; §7.2 settings |
| Loyalty earn and redeem (`SAL.LOY.01`, `SAL.RUL.07`) | `SAL.RUL.07` earns 1 point per AED 10 paid by the patient for self-pay services and products; none on the insurer's share or on package sessions; 100 points = an AED 10 discount line on a later self-pay invoice, confirmed by a one-time code to the patient (whether health authority rules limit rewards on medical services: `⚠ open: lawyer (L-38)`; VAT on the discounted supply: `⚠ open: accountant (A-15)`) | §5; §7.2 settings |
| Loyalty expiry, segments and retention campaigns (`SAL.LOY.01`, `SAL.RUL.08`, `SAL.ACL.07`, `NTF.RUL.02`, `NTF.ACL.03`) | points expire 12 months after they are earned, reminder 30 days before (`SAL.RUL.08`); a manual adjustment above 500 points needs a manager (`SAL.ACL.07`); segments: no visit in 6 months, package with sessions left and no booking in 30 days, birthday month (segment rules on clinical services off: visit dates, spend and non-clinical items only); campaigns only to patients with marketing consent (`NTF.ACL.01`), with opt-out and the advertising permit (`CLN.ACL.13`); sent 07:00–21:00, at most 2 marketing messages per patient in 7 days (`NTF.RUL.02`); a broadcast to more than 200 patients needs a second approver (`NTF.ACL.03`) | §5; §7.2 settings |
| Onboarding checklists (`HRM.ONB.01`, `HRM.RUL.16`, `HRM.RUL.17`, `HRM.ACL.10`) | templates for receptionist, call-centre agent, nurse / assistant, practitioner, billing coordinator and storekeeper, started by `HRM.RUL.16` when the employee record (`HRM.EMP.01`) is created in `onboarding` (none for a record created directly `active`, e.g. by a data import); checklist steps use `WRK.CHK.01` (photos and signature); tasks to the clinic manager, the line manager and the employee; the employee's own steps recorded only by them (`HRM.ACL.10`); `HRM.RUL.17` reminds the assignees of an overdue step on the first run after its due date, then every 2 days, and tells the clinic manager and the line manager after 5 days; completion blocks nothing (bookings stay guarded by licences, `SCH.ACL.04`) | §5; §7.2 settings |
| SOP library (`HRM.ONB.01`, `DOC.RUL.04`, `HRM.ACL.10`) | SOP owners: clinic manager (front desk, billing, complaints) and owner / medical director (infection control and clinical SOPs); required readers by role per branch; `DOC.RUL.04` asks each reader to acknowledge within 7 days, reminds every 3 days and tells the reader's manager and the SOP's owner role (the SOP owner named above) after 7 days overdue; acknowledged only by the reader (`HRM.ACL.10`); a new version asks for a new acknowledgement | §5; §7.2 settings |
| Registration and certification kinds (`DOC.EXP.01`, `DOC.RUL.01`) | *Trademark registration* and *Other certification* (subject: legal entity); responsible role owner / medical director; `DOC.RUL.01` at 180, 90 and 30 days for trademarks and 90, 30 and 7 days for the others; no `blocks` | §5; §7.2 settings |

#### Edge cases decided
- **Loyalty VAT:** a redeemed points discount reduces the VAT of the discounted supply only
  (`⚠ open: accountant (A-15)`); the rest of the loyalty settings can be built.
- **Loyalty on medical services:** whether health authority rules limit rewards on medical
  services, and whether campaign content needs their approval, is `⚠ open: lawyer (L-38)`; the
  seed earns points on self-pay services until answered.
- **SMS/WhatsApp down list:** the fallback screen lists `booked` and `confirmed` appointments
  starting in the next 24 hours from the moment the connection went `down`, per branch, with a
  *called* tick per row.
- **Give-up for other 05 message types:** module defaults (wl-notify §5.4).
- **Responsible role scope:** the clinic manager of the branch in the licence or privilege's
  scope; for a licence valid at several branches, the clinic manager of each. The client may
  change the role per kind (wl-documents §7).
- **Locked values:** override mode `never` and compliance-critical rules cannot be changed by the
  client (set by us).

#### Must not
- Must not let a client switch the licence block to `with_approval`.
- Must not include a cancellation reason in any patient template.

#### Done when
- Licences and bookings
  - [ ] A 05 deployment has practitioner licence and branch privileges kinds with the three
        `blocks` entries and override `never`; the override action is not offered.
  - [ ] `SCH.RUL.05` runs daily with D = 30; `SCH.RUL.06` with H = 2.
  - [ ] The practitioner licence kind has responsible role clinic manager; the facility licence
        kind, owner / medical director; the insurance card kind, receptionist with *notify party*.
- Deposits and reason codes
  - [ ] A walk-out refunds the deposit remainder; a waiver needs `bookings.waive_deposit` held by
        the clinic manager.
  - [ ] `coverage_not_confirmed` exists with initiator `business`.
- Alerts and void
  - [ ] Failing `CLN.RUL.03` alerts the owner / medical director; the client cannot switch it off.
  - [ ] `void` is offered on a draft claim and not on an appointment.
- Connections and messages
  - [ ] With the SMS connection `down`, the front desk sees the 24-hour call list.
  - [ ] With the claims channel `down`, the claim screen shows "submit through the payer portal
        and record the reference".
  - [ ] A reminder that gives up creates no task; a recall notice that gives up creates a call task.
  - [ ] The reassignment, clinic-cancel and recall templates (EN/AR) contain no reason field.
  - [ ] The four 05 notification types are registered with the groups and urgency of §7.2; a
        card upload request that gives up creates a task for the receptionist.
- Late entry
  - [ ] Check-in time, check-out time, cash payments and consumables issued accept a business time
        with the late-entry flag.
- Sales, finance, staff and registrations
  - [ ] a new lead or party offers the sources referral, walk-in, website, social, paid ads,
        aggregator, partner, event and other; a campaign with channel *Instagram*, a budget and
        cost entries reports leads, conversion, cost per lead, CAC and ROAS; a lead with source
        *paid ads* and no campaign is refused; a cost entry over the budget waits for a manager
        (`SAL.ACL.07`); a campaign promoting services without a permit number cannot be
        activated (`CLN.ACL.13`); one whose permit is valid to 2026-12-31 is ended on that day,
        and its retention broadcast scheduled for 2027-01-02 is cancelled (`NTF.ACL.04`).
  - [ ] a self-pay invoice of AED 200 paid by the patient earns 20 points and the insurer's share
        earns none; redeeming 100 points adds an AED 10 discount line; points earned on 2026-11-01
        expire on 2027-11-01 with a reminder on 2027-10-02 (`SAL.RUL.08`); an adjustment of 600
        points needs a manager (`SAL.ACL.07`); a patient without marketing consent is left out of
        a campaign; a retention message queued at 22:00 is deferred to 07:00, a third marketing
        message in 7 days is suppressed (`NTF.RUL.02`); a send to 201 patients needs a second
        approver (`NTF.ACL.03`); a segment cannot use a clinical service as a rule.
  - [ ] hiring a receptionist creates the receptionist checklist with tasks to the clinic manager,
        the line manager and the employee, and the role is active before it is complete; a step
        still open on the first run after its due date reminds its assignees, and again 2 days later
        (`HRM.RUL.17`); a new version of an SOP owned by the clinic manager asks every required
        reader to acknowledge again within 7 days (`DOC.RUL.04`), and no one else can acknowledge
        for them (`HRM.ACL.10`).
  - [ ] a fresh 05 deployment lists *Trademark registration* and *Other certification* with
        responsible role owner / medical director; a trademark registration expiring in 90 days
        notifies the owner / medical director.

#### Out of scope
Module mechanisms (their epics); money and stock settings (BL-CLN-16).

---

### BL-CLN-16 — 05 Clinics: configure shared negative-flow settings (money and stock)

**Labels:** `vertical:05-clinics` `area:negative-flow` `priority:high` `size:m` `blocked:open-item` `surface:back-office` `surface:portal` `surface:system` · **Milestone:** Launch 05 · **Size:** m
**Depends on:** BL-INV-04 · BL-INV-05 · BL-INV-18 · BL-PAY-02 to BL-PAY-07 · BL-PAY-17 · BL-SAL-16 · BL-STK-11 to BL-STK-20 · BL-INV-25 · BL-INV-26 · BL-INV-27 · BL-INV-28 · BL-INV-29 · BL-INV-30 · BL-PAY-25 · BL-PAY-26 · **Blocks:** None.
**Review items:** X-02, X-03, X-06, X-07, X-09, X-10, 05-N11-a, 05 MEDIUM "billing disputes", "negative stock"

#### Goal
Seed 05's values for the shared money and stock mechanisms: patient invoice write-off and
disputes, claim write-off limit, payer overpayment approval, failed and pending payments,
chargebacks and failed refunds, front-desk cash custody per receptionist and day, packages after a
payment reversal, negative stock, stock holds and recalls.
It also seeds the 05 values for the cash-flow forecast alert (`FIN.CFL.01`) and the
company budget with owner drawings (`FIN.BUD.01`), added on 2026-10-16: `FIN.RUL.23`,
`FIN.RUL.24` and `FIN.ACL.21`.

#### Surfaces
- **Back office:** patient invoice write-offs, cash shortage write-offs and owner drawings
  wait for approval; the front desk gets a collect task for a package bought with a reversed
  payment; a receptionist's handover is counted by someone else and salary recovery is not
  offered.
- **Portal:** the patient raises a billing dispute through the complaint form, category
  *billing*, picking one of their open invoices.
- **System:** values are seeded from the 05 deployment template; `FIN.RUL.19`, `FIN.RUL.21`,
  `STK.RUL.06`, `FIN.RUL.23` and `FIN.RUL.24` run with the 05 values.

#### Files to read
- `docs/verticals/05-clinics.md` §3.3 (`FIN.ACL.14`, `FIN.ACL.16`, `FIN.ACL.05`, `FIN.ACL.06`,
  `FIN.ACL.19`, `STK.ACL.05`), §4.1 and §4.2 "When it goes wrong", §4.6, §7.2 settings
- `docs/modules/wl-invoicing.md` §7; `docs/modules/wl-payments.md` §7; `docs/modules/wl-sales.md`
  §7; `docs/modules/wl-inventory.md` §7
- `docs/modules/wl-invoicing.md` §5.14–§5.16 (cash-flow forecast, company budget, owner
  drawings); `docs/modules/wl-payments.md` (open cheques, owner-drawings payments)
- `docs/OPEN-ITEMS.md` L-20

#### Terms
- **Cash holder:** a person who collects cash and must hand it over for counting.
- **Handover:** the cash count record in wl-payments; `short` when counted cash is below expected.

#### Spec excerpt (authoritative for this issue)

| Setting | 05 value | Spec reference |
|---|---|---|
| Invoice write-off limit (`FIN.ACL.15`) for patient balances | AED 0 (always approval) | §7.2 settings |
| Patients may raise a billing dispute in the portal | yes, through the complaint form (category *billing*) | §7.2 settings |
| Dispute escalation (`FIN.RUL.15`) | 14 days | §4.2 |
| Dispute outcome credited above the limit | `FIN.ACL.07` | §4.2 |
| Claim write-off limit (`FIN.ACL.14`) | AED 1,000; above: owner | §3.3 |
| Payer overpayment and recoupment approval (`FIN.ACL.16`) | second person, any amount | §3.3 |
| Payer follow-up after submission (`FIN.RUL.13`) | 45 days | §7.2 |
| Pending online payment check (`FIN.RUL.19`) | 30 minutes | §4.1 |
| Late confirmation after a deposit hold expired | refunded automatically | §4.1 |
| Chargeback acceptance (`FIN.ACL.18`) and evidence task (`FIN.RUL.20`) | second person | §4.2 |
| Refund retry (`FIN.RUL.21`) | once after 24 hours, then another method or patient credit | §4.2 |
| Package bought with a reversed payment | stays active; the front desk is told to collect before the next session (suspend consumption: off) | §4.2; wl-sales §7 |
| Cash holders | each receptionist, per person, per day at the branch (one handover per receptionist, branch and day) | §7.2 settings |
| Counter of front-desk cash (`FIN.ACL.05`) | someone other than the holder (clinic manager or another user) | §3.3, §4.2 step 9 |
| Cash shortage write-off (`FIN.ACL.06`) | AED 0 (always approval) | §3.3 |
| Shortage recovery by salary deduction (`FIN.ACL.19`) | off `⚠ open: lawyer (L-20)` | §3.3 |
| Negative stock policy | `block` at every branch store; no `allow_consumption` (consumables are lot-tracked) | §7.2 settings |
| Pending issue alert (`STK.RUL.06`) | storekeeper; clinic manager after 24 hours | §4.6 |
| Stock hold reason for a suspect consumable | `quality`; deciders storekeeper or clinic manager; reminder every 24 hours (`STK.RUL.07`) | §4.6 |
| Recall hold release (`STK.ACL.05`) | second holder of `lots.release` | §3.3 |
| Recall owner | owner / medical director; notices sent by the clinic manager | §7.2 settings |
| Recall notice template | *patient* (BL-CLN-14) | §4.6 |
| Cash-flow forecast alert (`FIN.CFL.01`, `FIN.RUL.23`) | projected balance below AED 50,000 in any of the next 13 weeks (weeks start Monday): `FIN.RUL.23` alerts the owner / medical director and the accountant, checked daily at 06:00; insurer remittances by claim due date, adjusted by each payer's average days late: on; disputed amounts counted: off | §5; §7.2 settings |
| Budget categories and approval (`FIN.BUD.01`, `FIN.RUL.24`) | categories owned by the accountant and mapped to the export accounts; one budget per legal entity and fiscal year, by month (budget lines have no branch; the seed has no branch categories, and a client who wants branch lines has the accountant map branch-specific export accounts to categories of their own); approved by the owner / medical director; `FIN.RUL.24` warns the category's owner role and the accountant when a month's actual reaches 100% of its line | §5; §7.2 settings |
| Owner drawings (`FIN.BUD.01`, `FIN.ACL.21`) | expense category and payment purpose *owner drawings*, kept out of operating cost and profit; every drawing approved, whatever its amount (`FIN.ACL.21` limit AED 0), by the owner / medical director, never by the requester (`FIN.ACL.21`) | §5; §7.2 settings |

#### Edge cases decided
- **Handover unit:** one handover per receptionist, branch and day (Asia/Dubai); a receptionist
  who works at two branches on one day has two handovers.
- **Who counts if the clinic manager is the holder:** any other user with the count permission;
  the holder is always refused (`FIN.ACL.05`).
- **Package collection task:** goes to the front desk of the branch where the package was sold.
- **Dispute from the portal:** the complaint form category *billing* lets the patient pick one of
  their open invoices; submitting runs `dispute` on it and links the complaint ticket.

#### Must not
- Must not enable salary deduction for shortages while L-20 is open.
- Must not set `allow_consumption` on any branch store.

#### Done when
- Invoices and disputes
  - [ ] Every patient invoice write-off creates an approval request (AED 0 limit).
  - [ ] A patient submits a *billing* complaint in the portal for invoice INV-0042: the invoice is
        `disputed`, reminders stop, and after 14 days the escalation runs.
- Claims
  - [ ] A claim write-off of AED 1,000.01 needs the owner; AED 1,000.00 does not.
- Payments
  - [ ] A deposit payment pending 30 minutes is checked with the gateway.
  - [ ] A failed refund is retried once after 24 hours.
  - [ ] A reversed payment on package PKG-7 leaves it `active` and gives the front desk a collect task.
- Cash
  - [ ] Receptionist A has one handover for 2026-11-10 at Branch 1; it cannot be counted by A; a AED 10.00 shortage needs
        approval to write off; salary recovery is not offered.
- Stock
  - [ ] An issue to a visit beyond stock at a branch store creates a pending issue; after 24 hours
        the clinic manager is told.
  - [ ] Releasing a recall-held lot needs a second `lots.release` holder.
- Cash-flow forecast and budget
  - [ ] in a fresh 05 deployment, a projected balance of AED 49,999 in week 5 alerts the owner /
        medical director and the accountant once (`FIN.RUL.23`, 06:00 run); AED 50,000 or more in
        every week alerts no one; the forecast shows 13 weekly buckets from Monday.
  - [ ] a monthly budget for a legal entity prepared by the accountant needs approval by the owner
        / medical director; an expense with category *owner drawings* is left out of operating
        cost and profit reports, and the user who records it cannot approve it (`FIN.ACL.21`); an
        expense line reaching 100% of its month's budget line warns the category's owner role and
        the accountant once (`FIN.RUL.24`).

#### Out of scope
Module mechanisms (their epics); bookings, licences and messages (BL-CLN-15).
