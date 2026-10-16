# 05 — Private Clinics: Go-to-Market Pack

> Status: draft · Last updated: 2026-10-15 · Source:
> [05-clinics.md](../verticals/05-clinics.md) (accepted)

## 1. Positioning

**One line:** Bookings, reminders, packages, front-desk billing and insurance claim
follow-up for UAE aesthetics, dental, physiotherapy and wellness clinics, next to your EMR,
not instead of it.

**For:** private outpatient clinics with 1–5 branches whose revenue depends on bookings,
packages and insurance: aesthetics and cosmetic dermatology, physiotherapy, dental,
wellness and IV therapy. Clinical records are light or already in a licensed EMR.

**Not for (say so early):**
- Clinics looking for an EMR. We hold no clinical notes, prescriptions, results or treatment
  plans, and do not submit to NABIDH, Malaffi or Riayati. That stays in the clinic's EMR.
- Large multi-specialty polyclinics and hospitals that want one all-in-one product.
- Clinics that need e-prescribing or pharmacy dispensing.
- Clinics that need claims sent straight to eClaimLink or Shafafiya from the system today
  (integrated submission is on the roadmap; until then claims go through the existing
  channel).

## 2. Ideal client profile

| | Fits | Weak fit |
|---|---|---|
| Size | 1–5 branches, 3–40 practitioners, 10–150 staff, 30–500 visits a day | one practitioner, a few visits a day |
| Speciality | aesthetics, cosmetic dermatology, physiotherapy, dental, wellness, IV therapy | hospitals, multi-specialty polyclinics wanting all-in-one |
| Location | Dubai (DHA), Abu Dhabi (DoH), northern emirates | — |
| Revenue | a mix of self-pay, packages and insurance with copays | — |
| Today | an EMR or front-desk tool, WhatsApp and phone bookings, insurer portals for claims, Excel for packages and licence expiry | an EMR whose built-in billing and booking they are happy with |

**Signals that a company fits**
- Job ads for an "insurance coordinator", "claims officer", "RCM executive" or "front desk
  receptionist" (manual work we remove).
- Packages promoted on Instagram or the website (e.g. "6 laser sessions", "10 physio
  sessions").
- Online reviews about waiting times, booking by phone only, or billing surprises.
- A new branch opening, or new practitioners joining.

**Who decides and who uses it**

| Role | Cares about | Message angle |
|---|---|---|
| Owner / medical director | Revenue per practitioner, denials, regulator problems | Utilisation, no-shows and revenue by payer; licences flagged before they expire |
| Clinic manager | Full schedule, no-shows, complaints, packages | Self-booking with reminders, a waiting list, packages tracked to the session |
| Insurance / billing coordinator | Prior authorisations, denials, remittances | Every claim and denial tracked, with the resubmission deadline as a task |
| Receptionist | Queue at check-in, copay arguments | Eligibility recorded, patient share calculated from the payer contract |

## 3. Lead sources

Build the list by hand: look up each clinic, check its website and LinkedIn, and record
where it came from. Do not scrape or bulk-copy any of the sources below.

| Source | What it gives | Type | How to use | Terms |
|---|---|---|---|---|
| DHA Dubai Medical Registry on Sheryan ([search](https://services.dha.gov.ae/sheryan/wps/portal/home/medical-directory)) [S1] | Medical professionals and facilities registered with DHA | Official | Look up a clinic to confirm its licence and speciality, one at a time | Terms not checked |
| DoH "Find a Healthcare Facility" on TAMM ([search](https://www.tamm.abudhabi/wb/doh/find-a-facility?lang=en)) [S2] | Licensed facilities in Abu Dhabi | Official | Look up a clinic to confirm it, one at a time | DoH site: personal, non-commercial use only, no reproduction without consent [S3]; TAMM terms not checked |
| MOHAP (northern emirates) | No public facility search found; MOHAP app reportedly has a directory | Official | Confirm a clinic's licence on request | Not verified |
| Insurer and TPA provider network lists | Clinics in an insurer's network, by speciality and area | Commercial | Spot clinics that bill insurance; look up each one | Terms not checked |
| Events: AEEDC Dubai (dental), 19–21 January 2027 [S4]; WHX Dubai, 25–28 January 2027 [S5]; Dubai Derma (aesthetics and dermatology), 6–8 April 2027 [S6] | Clinic owners and managers in one place | Commercial | Visit, meet, collect cards with consent to follow up | Event terms |
| LinkedIn | Clinic managers, practice owners, RCM and insurance leads | Commercial | Manual search or Sales Navigator; no automation | User agreement forbids scraping |
| Google Maps and Instagram ("aesthetic clinic Dubai", "physiotherapy Abu Dhabi") | Local clinics with reviews and promoted packages | Commercial | Manual research; reviews show pain points | Terms forbid bulk extraction |
| Partners: clinic licensing and setup consultants, medical billing (RCM) outsourcing firms, EMR vendors without a business module | Introductions to new and growing clinics | Commercial | Referral arrangement | Agreement per partner |

## 4. Messages

Health data rule for all copy: describe how data is handled, never claim a certification
or that the clinic becomes compliant by using the system. No outcome numbers (e.g. "cut
no-shows by X%").

### 4.1 Landing page

**Headline:** A full schedule, paid packages and claims that get paid.

**Sub-headline:** Bookings, packages, front-desk billing and insurance follow-up for UAE
clinics, under your brand, next to your EMR, hosted in the UAE.

**Three problems we hear about:**
1. Bookings come by phone and WhatsApp; no-shows leave gaps nobody fills.
2. Packages are tracked in Excel. Nobody knows how many sessions a patient has left, or how
   much unused value sits on the books.
3. Denied claim lines come back on the remittance and are found weeks later, if at all.

**Three outcomes:**
1. **Patients book themselves.** Online booking that respects each practitioner's roster,
   SMS, WhatsApp and email reminders, and freed slots offered to the waiting list.
2. **Packages to the session.** Sold, used per visit, reminded before expiry, and reported
   as a liability.
3. **Denials followed up.** Claims prepared from charge capture, remittances reconciled per
   line, each denial a task with its resubmission deadline.

**Also included:** patients with guardians and consents, payer contracts with tariffs and
copay rules, eligibility and prior authorisation tracking, patient-share calculation, forms
signed on a tablet, receipts and tax invoices with VAT per service, online payment, optional
booking deposits, practitioner licences that block bookings when expired, recalls,
complaints, a patient portal, consumables with lots and expiry, dashboards, migration from
Excel.

**How patient data is handled:** every deployment is treated as holding health data: hosted
in the UAE, access limited by role (reception does not see visit reasons or codes), every
view logged, patient data kept for 25 years.

**Call to action:** See a 10-minute demo with a sample clinic.

**Footer facts:** hosted in the UAE · your own branded deployment · priced per
practitioner.

### 4.2 First email (to a named owner or clinic manager)

> **Subject:** Bookings and packages at {Clinic}
>
> Hi {First name},
>
> I saw that {Clinic} offers {packages / services seen on their site}. Many clinics we speak
> to take bookings on the phone and WhatsApp, track packages in Excel, and find denied
> claim lines late.
>
> We built a system for the business side of the clinic: online booking with reminders and
> a waiting list, packages tracked per session, and claims and denials followed up. It
> works next to your EMR, runs under your brand and is hosted in the UAE.
>
> Would a 10-minute demo with a sample clinic be useful? If not, reply "no" and I won't
> write again.
>
> {Name} · {Company} · {phone} · {website}
> You received this because {Clinic}'s details are published on {source}. Unsubscribe: {link}

### 4.3 LinkedIn connection note (under 300 characters)

> Hi {First name}, I work on software for the business side of UAE clinics (bookings,
> packages and insurance claims, next to the EMR). I'd like to connect and share what we're
> building.

### 4.4 Follow-up (once, 5–7 working days later, same thread)

> Hi {First name}, a short follow-up. The part clinic managers like most is the waiting
> list: when a patient cancels, the slot is offered to the next patient waiting, without
> anyone calling round. Happy to show it in 10 minutes. If now is not the time, just say so.

No further follow-up without a reply.

## 5. Demo storyline (10 minutes)

Uses *Shifa Family & Dental Clinics* from [DEMO-DATA.md](../DEMO-DATA.md) §05, focusing on
its dental, physiotherapy and aesthetics services. Tell it as the clinic manager's day. No clinical
notes or diagnoses beyond claim codes are shown.

1. **08:00, the clinic manager opens the dashboard:** today's schedule, a dentist's licence
   expiring in 20 days, 5 packages expiring this week, denied claims awaiting
   resubmission.
2. **The patient's phone:** a patient books online, picks a dentist and a free slot, and
   gets a WhatsApp reminder (shown in the outbox). *Wow 1: self-booking that respects each
   dentist's roster.*
3. **Reception:** check-in, the eligibility result recorded, a consent form signed on the
   tablet, the copay calculated from the payer contract and collected; a receipt issued.
4. **A cancellation:** the slot is released and offered to the patient on the waiting list,
   who accepts it.
5. **Physiotherapy:** a patient's package session is used at the visit; the remaining
   sessions show at the desk and in the patient portal.
6. **The insurance coordinator:** a claim batch prepared from charge capture and submitted
   through the existing channel; a denial with its reason, corrected and resubmitted.
   *Wow 2: denials tracked to money recovered.*
7. **The medical director:** utilisation per practitioner, no-show rate, revenue by payer;
   the licence about to expire is flagged and bookings will be blocked after expiry.
   *Wow 3: a regulator problem caught before it happens.*

## 6. Objections

| Objection | Answer |
|---|---|
| "We already have an EMR." | Keep it for clinical records. We cover bookings, packages, front-desk billing and claim follow-up. Charge capture is picked from your coded service list or imported from the EMR by CSV; EMR integration is on the roadmap. If your EMR already does all of this well, we are probably not for you. |
| "We need an EMR." | Then we are not for you. We do not hold clinical notes, prescriptions or results. |
| "Does it submit claims to eClaimLink / Shafafiya?" | Not directly yet. Claims are prepared and checked in the system, submitted through your existing channel, and their status, remittances and denials are tracked. Integrated submission is on the roadmap. |
| "Where is patient data, and who sees it?" | Hosted in the UAE. Every deployment is treated as health data: access by role and branch, reception sees no visit reasons or codes, every view is logged, patient data is kept 25 years. For Abu Dhabi clinics we go through your ADHICS requirements with you; we do not claim a certification. |
| "Our patients book on WhatsApp." | They can keep messaging; staff book them in the same calendar. Reminders go by WhatsApp, SMS and email. |
| "Can we send promotions to our patients?" | Not in the first version: messages are reminders and transactional only. Campaigns to consenting patients are on the roadmap, and promotions need a regulator permit. |
| "We sell memberships." | Prepaid packages are included; recurring memberships are on the roadmap. |
| "How long does it take to start?" | Patients, practitioners, price lists and open packages are imported from Excel; then the team is trained by role. |
| "What does it cost?" | Priced per practitioner, with plans by clinic size. We quote after the demo. |

## 7. What we can claim

| Claim | Capability (MVP) |
|---|---|
| Patients with guardians, consents, Emirates ID and insurance cards | `PTY.PTY.01`, `PTY.REL.01`, `PTY.CNS.01`, `PTY.KYC.01`, `CLN.INS.02` |
| Online and staff booking, waiting list, rosters, rooms | `SCH.SLF.01`, `SCH.BKG.01`, `SCH.RES.01`, `HRM.ROS.01` |
| SMS, WhatsApp and email reminders, no-show handling, optional deposits | `SCH.RMD.01`, `NTF.MSG.01`, `NTF.MSG.02`, `NTF.MSG.03` |
| Practitioner licences with a booking guard | `HRM.EMP.01`, `HRM.QUA.01`, `DOC.EXP.01` |
| Check-in, eligibility, forms signed on a tablet | `CLN.ENC.01`, `CLN.INS.02`, `DOC.SIG.01` |
| Payer contracts, charge capture and patient-share calculation | `CLN.INS.01`, `CLN.ENC.01`, `CLN.ENC.02` |
| Prior authorisation tracking | `CLN.INS.03` |
| Claim preparation, remittance reconciliation, denial management | `CLN.CLM.01`, `FIN.TPP.01`, `CLN.CLM.03` |
| Patient feedback after a visit, with consent; low ratings open a complaint | `WRK.FBK.01`, `WRK.TKT.01` |
| Receipts and tax invoices with VAT per service, online payment | `FIN.INV.01`, `FIN.PAY.01`, `FIN.PAY.02`, `SAL.CAT.01` |
| Prepaid packages | `SAL.PKG.01` |
| Complaints | `WRK.TKT.01`, `WRK.TKT.02` |
| Patient portal | `POR.SLF.01` |
| Consumables with lots and expiry | `STK.ITM.01`, `STK.LOC.01`, `STK.MOV.01`, `STK.LOT.01` |
| Recalls and clinic dashboards | `CLN.RCL.01`, `CLN.RPT.01`, `RPT.DSH.01` |
| Health data handling: role access, audited views, 25-year retention | `PLT.USR.01`, `PLT.AUD.01` |
| Migration from Excel | `PLT.IMP.01` |

**On the roadmap only (do not promise dates):** integrated claim submission and remittance
download, EMR integration, telehealth via a licensed provider, memberships, leads and
campaigns, portal document exchange, Emirates ID reader, purchasing and goods received,
staff leave, accounting export, bank statement matching.

**Never claim:** an EMR or any clinical record (notes, diagnoses as a record, prescriptions,
results, treatment plans), HIE submission (NABIDH, Malaffi, Riayati), e-prescribing or
dispensing, a regulator certification or ADHICS compliance on the clinic's behalf, clinical
or outcome statistics.

## Sources

Checked 2026-10-15. *Official* = government or legislation; others are secondary.

| # | Source | Type | Used for |
|---|---|---|---|
| S1 | DHA, Sheryan, "Dubai Medical Registry", https://services.dha.gov.ae/sheryan/ | Official | Registered professionals and facilities in Dubai |
| S2 | DoH, "Lists and tools" (Find a Healthcare Facility), https://www.doh.gov.ae/en/resources/Lists-and-Tools | Official | Facility search in Abu Dhabi |
| S3 | DoH, "Terms of use" (section B, copyright), https://www.doh.gov.ae/en/term-of-use | Official | Personal, non-commercial use of DoH content |
| S4 | AEEDC Dubai, https://www.aeedc.com | Secondary | Dental event dates |
| S5 | World Health Expo, "General event FAQs", https://www.worldhealthexpo.com/events/healthcare/dubai/overview/general-event-faqs/ | Secondary | WHX Dubai dates |
| S6 | Dubai Derma, https://www.dubaiderma.com | Secondary | Aesthetics and dermatology event dates |
