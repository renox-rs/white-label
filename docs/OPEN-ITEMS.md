# Open Items Register

> Status: draft · Last updated: 2026-10-15

Every fact the documents could not settle from public sources (`⚠ open: <owner>`) and every
business decision still to take (`⚠ decide`), in one place. The marker in the source document
carries the item's ID, e.g. `⚠ open: lawyer (L-01)`. When an item is answered, update the
source document first (replace the marker with the fact and a source), then strike the row
here as `~~question~~ Answered (date): ...`.

**Gates** say when an answer is needed:

| Gate | Meaning |
|---|---|
| **Build 1** | Changes the data model or a rule in the first build (01 Property, 09 Manpower and the modules they use); answer before coding those parts |
| **Client 1** | Needed for the contract, data processing agreement or security pack before the first production client of any vertical |
| **Launch NN** | Needed before selling vertical NN (or before selling it outside Dubai, where noted) |
| **Onboarding** | Collected per client during onboarding; the app already has a setting or a manual path |
| **Outreach** | Needed before contacting leads at scale ([go-to-market](go-to-market/README.md)) |
| **Later** | Only for a capability that is Later |

## 1. Summary

| Owner | Items | Build 1 | Client 1 | Outreach | Launch | Onboarding | Later |
|---|---|---|---|---|---|---|---|
| Owner decision (`D`) | 5 (all decided) | — | 0 | — | — | — | — |
| Lawyer (`L`) | 38 | 2 | 5 | 1 | 28 | 1 | 1 |
| Accountant (`A`) | 16 | — | — | — | 15 | 1 | — |
| Authority (`AU`) | 25 | — | — | — | 21 | 2 | 2 |
| Client (`C`) | 3 | — | — | — | 1 | 2 | — |
| Onboarding (`O`) | 4 | — | 2 | — | — | 2 | — |
| **Total open** | **86** | **2** | **7** | **1** | **65** | **8** | **3** |

**Before Build 1:** L-05 and L-10. **Before outreach at scale:** L-18. **Before Client 1:**
L-01, L-02, L-03, L-11, L-13, O-01, O-02. The owner decisions D-01 to D-05 were taken on 2026-10-15.
The items added from the negative-flow pass (L-19 to L-37, A-04 to A-14, AU-20 to AU-25, C-03)
and from the SME gaps (L-38, A-15, A-16)
are not needed before Build 1: each has a safe default in its document (the stricter
choice, off by default, or a configurable parameter).

## 2. Owner decisions

| ID | Decision | Where | Gate |
|---|---|---|---|
| D-01 | ~~Hosting region: AWS `me-central-1`, Azure UAE North or Core42~~ Decided (2026-10-15): our own VPS in the UAE; backups on a second UAE VPS at another provider (S3-compatible storage) | [OPERATIONS.md](OPERATIONS.md) §2 | Client 1 |
| D-02 | ~~Loss of the region~~ Decided (2026-10-15): if the primary provider is down, all clients are restored on the backup VPS (or a new UAE VPS), highest service level first; RTO ≤ 24 hours; data stays in the UAE | [OPERATIONS.md](OPERATIONS.md) §5 | Client 1 |
| D-03 | ~~Minimum contract term~~ Decided (2026-10-15): 12 months, then monthly with 30 days' notice | [PRICING.md](PRICING.md) §6 | Client 1 |
| D-04 | ~~Discount for yearly prepayment~~ Decided (2026-10-15): 2 months free (pay 10 for 12) | [PRICING.md](PRICING.md) §6 | Client 1 |
| D-05 | ~~Notice period for price changes~~ Decided (2026-10-15): 60 days, applied from the next renewal; prices locked during a running term | [PRICING.md](PRICING.md) §6 | Client 1 |

The prices themselves are filled in [PRICING.md](PRICING.md) by the owner.

## 3. Lawyer

| ID | Question | Where | Verticals | Gate |
|---|---|---|---|---|
| L-01 | Are the PDPL (Decree-Law 45 of 2021) executive regulations published, and what do they require (deadlines, formats, DPO, cross-border transfers)? | [SECURITY.md](SECURITY.md) §6; [wl-parties](modules/wl-parties.md), [wl-notify](modules/wl-notify.md), [wl-documents](modules/wl-documents.md) §10 | all | Client 1 |
| L-02 | What is the breach notification deadline under PDPL Art. 9 ("immediately upon becoming aware")? | [SECURITY.md](SECURITY.md) §7 | all | Client 1 |
| L-03 | How long may login and security logs be kept? | [wl-portal](modules/wl-portal.md) §10 | all | Client 1 |
| L-04 | Do health regulators use a consent age other than 18 for patients and their data? | [wl-portal](modules/wl-portal.md), [wl-parties](modules/wl-parties.md) §10 | 05, 10 | Launch 05 |
| L-05 | Is a plain check-in photo, reviewed by a supervisor, biometric (sensitive) data under the PDPL? | [wl-attendance](modules/wl-attendance.md) §10 | 02, 03, 09 | **Build 1** |
| L-06 | Does Cabinet Resolution 66 of 2023 add rules for marketing messages beyond consent and opt-out? | [wl-notify](modules/wl-notify.md) §10 | 05, 06, 08, 10 | Launch 05 |
| L-07 | Do feedback requests after a service count as direct marketing? | [wl-ticketing](modules/wl-ticketing.md) §10 | all with `WRK.FBK.01` | Launch 05 |
| L-08 | Is an expiry term on prepaid services, packages or vouchers enforceable? | [wl-sales](modules/wl-sales.md) §10 | 05, 08, 10 | Launch 05 |
| L-09 | Are there rules on no-show fees and forfeited booking deposits? | [wl-scheduling](modules/wl-scheduling.md) §10 | 05, 10 | Launch 05 |
| L-10 | Which documents need more than a simple electronic signature (tenancy contracts for Ejari, employment contracts on MoHRE, engagement letters)? | [wl-documents](modules/wl-documents.md) §10 | 01, 04, 09 | **Build 1** |
| L-11 | Which sector duties require keeping an archive beyond deletion 30 days after a client leaves? | [wl-operator](modules/wl-operator.md) §7; [OPERATIONS.md](OPERATIONS.md) §10 | all, mainly 05 | Client 1 |
| L-12 | Is a support session from outside the UAE "processing health data outside the UAE" (Law 2 of 2019, Art. 13)? | [wl-operator](modules/wl-operator.md) §10 | 05 | Launch 05 |
| L-13 | Is the platform, which connects each client's own merchant account and never holds funds, excluded from the CBUAE Retail Payment Services Regulation (Circular 15/2021)? | [wl-payments](modules/wl-payments.md) §10 | all with online payments | Client 1 |
| L-14 | Does collecting cash on delivery for merchants need a CBUAE licence? | [03-logistics.md](verticals/03-logistics.md) §2, §13 q11 | 03 | Launch 03 |
| L-15 | Does a client delivering only its own merchants' parcels need the Emirates Post courier licence? | [03-logistics.md](verticals/03-logistics.md) §2, §13 q10 | 03 | Launch 03 |
| L-16 | Is an outside firm filing for a client through the client's own EmaraTax login, for a fee, unregistered tax agent practice? | [04-accounting.md](verticals/04-accounting.md) §2 | 04 | Launch 04 |
| L-17 | Is UAE PASS on TDRA's UAE Trust List (qualified signatures)? | [04-accounting.md](verticals/04-accounting.md) §8 | 04 | Later |
| L-18 | May we send first-contact B2B emails to named managers at UAE companies (PDPL consent for direct marketing, TDRA electronic messages), and does the telemarketing resolution (Cabinet Resolution 56 of 2024) apply to B2B calls? | [go-to-market/README.md](go-to-market/README.md) | all | Outreach |
| L-19 | Which documents are legal blocks, i.e. the law forbids the act without a valid one (practitioner licence and branch privileges, driving licence and vehicle permits, work permit, tutor and course approvals, product conformity certificates, Civil Defence approval of a fire-safety vendor)? Is a one-off approved override lawful for any of the other document kinds? | [wl-documents](modules/wl-documents.md) §5, §10; [01-property.md](verticals/01-property.md) §9; [03-logistics.md](verticals/03-logistics.md) §9; [05-clinics.md](verticals/05-clinics.md) §4; [07-manufacturing.md](verticals/07-manufacturing.md) §7, §9; [09-manpower.md](verticals/09-manpower.md) §9; [10-edtech.md](verticals/10-edtech.md) §9 | 01, 03, 05, 07, 09, 10 | Launch 01 |
| L-20 | May an employer deduct a cash shortage (a cashier's or driver's cash count that is short) from wages without a finding that the worker caused it, and under which category of Labour Law Art. 25? | [wl-payments](modules/wl-payments.md) §7, §10; [03-logistics.md](verticals/03-logistics.md) §3; [05-clinics.md](verticals/05-clinics.md) §4; [06-fnb.md](verticals/06-fnb.md) §4 | 03, 05, 06 | Launch 03 |
| L-21 | Returned cheques: which bank return reasons still support a criminal complaint (account closed, payment stopped, signature or alteration irregular)? What is the deadline to start execution on a returned cheque, may a partly honoured cheque be presented again for the rest, and what must the formal notice to the drawer contain? | [wl-payments](modules/wl-payments.md) §7, §10 | 01, all taking cheques | Launch 01 |
| L-22 | Tenancy negative paths in Dubai and Abu Dhabi: the demand period before an eviction claim for non-payment and the other grounds for eviction; how a landlord may repossess an abandoned unit and what evidence is needed; may early termination penalties and a returned cheque fee be charged without a contract clause; the holding-over rule in Abu Dhabi; may a deposit be kept when a prospect backs out before signing? | [01-property.md](verticals/01-property.md) §4, §7, §9, §13 q12 | 01 | Launch 01 |
| L-23 | When a final settlement is negative (advances, loans or damage exceed the dues), may the employer set off more than the Art. 25 caps against end-of-service dues, and how may it recover the rest? | [wl-hr](modules/wl-hr.md) §4, §10; [09-manpower.md](verticals/09-manpower.md) §4 | 09 | Launch 09 |
| L-24 | What may lawfully be withheld from the dues of a worker reported as absconding, and how and when must the approved settlement be paid? | [wl-hr](modules/wl-hr.md) §5, §10; [09-manpower.md](verticals/09-manpower.md) §9 | 09 | Launch 09 |
| L-25 | When a worker dies in service, to whom is the final settlement paid (heirs, a court, an embassy), which documents are needed, and does the employer bear the repatriation costs? | [wl-hr](modules/wl-hr.md) §5, §10; [09-manpower.md](verticals/09-manpower.md) §4, §9 | 09 | Launch 09 |
| L-26 | May the employer or the worker withdraw a notice of termination without the other party's written agreement? | [wl-hr](modules/wl-hr.md) §4, §10; [09-manpower.md](verticals/09-manpower.md) §7 | 09 | Launch 09 |
| L-27 | Which exceptions to the daily overtime cap apply, and what is the penalty for exceeding it? | [wl-attendance](modules/wl-attendance.md) §10 | 02, 03, 09 | Launch 09 |
| L-28 | When a worker works without a valid work permit or visa, what is the employer's exposure, and must the hours still be paid in full? | [wl-attendance](modules/wl-attendance.md) §10 | 02, 03, 09 | Launch 09 |
| L-29 | Is a timesheet the client approved by silence (deemed approval under the client contract) enough evidence in a billing dispute? | [wl-attendance](modules/wl-attendance.md) §10 | 09 | Launch 09 |
| L-30 | Sanctions freeze: do the 24 hours to freeze run from the list update or from the moment the business finds the match? What form and deadline apply to the report to the supervisor after freezing? | [wl-parties](modules/wl-parties.md) §5, §7, §10 | 04, all with screening | Launch 04 |
| L-31 | May an accounting firm still file a tax return for a client whose funds it has frozen after a sanctions match? | [04-accounting.md](verticals/04-accounting.md) §9 | 04 | Launch 04 |
| L-32 | When the business cancels a booking or cannot deliver it, is the customer owed more than the full refund of the deposit? | [wl-scheduling](modules/wl-scheduling.md) §10 | 05, 08, 10 | Launch 05 |
| L-33 | Under consumer protection law, may a restaurant refuse to refund a prepaid delivery order that failed for a customer-side reason (customer unreachable, wrong address, order refused)? | [06-fnb.md](verticals/06-fnb.md) §3, §9 | 06 | Launch 06 |
| L-34 | When a clinic misses the insurer's first-submission window, may it bill the payer's share to the patient, and does that depend on the payer contract or the emirate's rules? | [05-clinics.md](verticals/05-clinics.md) §3, §9 | 05 | Launch 05 |
| L-35 | Which legal parts supply period (7, 14 or 30 days) applies to which part, by demand and source? | [08-auto.md](verticals/08-auto.md) §9 | 08 | Onboarding |
| L-36 | What may a workshop do with a vehicle the customer never collects, and does a storage fee need more than the customer's agreement at check-in? | [08-auto.md](verticals/08-auto.md) §4, §9 | 08 | Launch 08 |
| L-37 | Construction: does retention become due when a contract is terminated (unless the contract says otherwise)? May a subcontractor suspend work for non-payment? How does a subcontractor claim against an insolvent main contractor? | [02-construction.md](verticals/02-construction.md) §4, §9 | 02 | Launch 02 |
| L-38 | Clinics: do health regulators limit loyalty rewards or discounts on medical services, and does campaign content for a clinic need their approval? | [05-clinics.md](verticals/05-clinics.md) §7.2 | 05 | Launch 05 |

## 4. Accountant

| ID | Question | Where | Verticals | Gate |
|---|---|---|---|---|
| A-01 | Who is the VAT recipient under each payer agreement: the patient (zero-rated healthcare) or the insurer (5%) (VATP016)? | [wl-invoicing](modules/wl-invoicing.md) §10; [05-clinics.md](verticals/05-clinics.md) §2 | 05 | Launch 05 |
| A-02 | How do insurance claims (eClaimLink, Shafafiya) relate to e-invoices from 2027? | [05-clinics.md](verticals/05-clinics.md) §9 | 05 | Launch 05 |
| A-03 | Which VAT treatment applies to each sale into a designated zone? | [08-auto.md](verticals/08-auto.md) §9 | 08 | Onboarding |
| A-04 | Is a cancellation fee kept from a customer's prepayment consideration for a supply (5% VAT) or compensation outside the scope of VAT? | [wl-sales](modules/wl-sales.md) §10; [07-manufacturing.md](verticals/07-manufacturing.md) §7, §9 | 06, 07, 08, all with order cancellation | Launch 07 |
| A-05 | Is a returned cheque (bounce) fee consideration for a supply (5% VAT) or compensation outside the scope of VAT (VATP001)? | [wl-payments](modules/wl-payments.md) §10; [01-property.md](verticals/01-property.md) §9 | 01, all taking cheques | Launch 01 |
| A-06 | What is the VAT treatment of a lease early termination penalty, and of a damages charge invoiced to a tenant for the part that exceeds the deposit? | [01-property.md](verticals/01-property.md) §4, §9 | 01 | Launch 01 |
| A-07 | Do liquidated damages deducted by the client from a payment certificate reduce the price (credit note) or are they compensation outside the scope of VAT? | [02-construction.md](verticals/02-construction.md) §4, §9 | 02 | Launch 02 |
| A-08 | When a property management company invoices a negative owner balance (vacancy, maintenance contract costs, repairs) to the owner, what VAT applies to the recharged costs? | [01-property.md](verticals/01-property.md) §4 | 01 | Launch 01 |
| A-09 | Bad-debt relief (Decree-Law Art. 64): what must the notice to the customer contain and when is it sent, in which period is the relief claimed, how is it computed on a partly paid invoice, and how is output VAT handled when a written-off debt is later paid? | [wl-invoicing](modules/wl-invoicing.md) §4, §10, §12 q11 | all | Launch 01 |
| A-10 | E-invoicing: is a document valid for VAT before the accredited service provider accepts it, how long may transmission take, which data fields are required, and must an ASP rejection be corrected by a credit note and a new invoice? | [wl-invoicing](modules/wl-invoicing.md) §4, §10, §12 q4, q12 | all | Launch 01 |
| A-11 | Supplier debit notes: is input VAT reduced when the business issues its debit note or when the supplier's tax credit note arrives? | [wl-invoicing](modules/wl-invoicing.md) §10, §12 q13 | all | Launch 01 |
| A-12 | After an export's zero rate was corrected to 5% because exit evidence did not arrive within 90 days, can the zero rate be restored when the evidence arrives later? | [07-manufacturing.md](verticals/07-manufacturing.md) §9; [08-auto.md](verticals/08-auto.md) §9 | 07, 08 | Launch 07 |
| A-13 | Does VAT become due on construction retention at the one-year limit even when the client still holds it, and how is a retention the client keeps for good treated? | [02-construction.md](verticals/02-construction.md) §9 | 02 | Launch 02 |
| A-14 | Can a small corporate tax error be corrected in a later return without a voluntary disclosure, as for VAT errors up to AED 10,000? | [04-accounting.md](verticals/04-accounting.md) §3, §9 | 04 | Launch 04 |
| A-15 | Loyalty points: does redeeming points reduce the taxable amount of the sale it is used on, how are free points and their expiry treated for VAT, and is the outstanding balance a liability or a provision? | [wl-sales](modules/wl-sales.md) §10; verticals 05, 06, 08, 10 §7.2 | 05, 06, 08, 10 | Launch 05 |
| A-16 | Owner drawings: how are drawings treated for VAT (input VAT on goods taken for private use) and for corporate tax? | [wl-invoicing](modules/wl-invoicing.md) §10 | all | Launch 01 |

## 5. Authority

| ID | Question | Where | Verticals | Gate |
|---|---|---|---|---|
| AU-01 | Are there reserved test ranges for Emirates ID numbers, TRNs and UAE phone numbers? | [DEMO-DATA.md](DEMO-DATA.md) §1 | demo | Later |
| AU-02 | Must clinic data sit on a dedicated host? | [OPERATIONS.md](OPERATIONS.md) §2 | 05 | Launch 05 |
| AU-03 | Does a patient portal or a non-EMR clinic system need its own DHA or DoH certification? | [wl-portal](modules/wl-portal.md) §10; [SECURITY.md](SECURITY.md) §6 | 05 | Launch 05 |
| AU-04 | Must a health data incident also be reported to DHA or DoH, and when? | [SECURITY.md](SECURITY.md) §7 | 05 | Launch 05 |
| AU-05 | Which traceability and record periods do food and health regulators require for stock lots? | [wl-inventory](modules/wl-inventory.md) §10 | 05, 06 | Launch 06 |
| AU-06 | Binding retention periods for inspection records (food 4 years reported; HSE; quality) | [wl-ticketing](modules/wl-ticketing.md) §10; [06-fnb.md](verticals/06-fnb.md) §9 | 06, 07 | Launch 06 |
| AU-07 | Do DHA or DoH set response times for patient complaints? | [wl-ticketing](modules/wl-ticketing.md) §10 | 05 | Launch 05 |
| AU-08 | Fire and life-safety rules outside Dubai: maintenance contracts, certificates per building type, alarm platform (Hassantuk or DCD), records to keep | [01-property.md](verticals/01-property.md) §2, §8, §9; [wl-ticketing](modules/wl-ticketing.md) §10 | 01 | Launch 01 outside Dubai |
| AU-09 | Does the 12-month lift inspection apply to residential passenger lifts? | [01-property.md](verticals/01-property.md) §2 | 01 | Launch 01 |
| AU-10 | Does Tawtheeq or DARI offer an API for property managers? | [01-property.md](verticals/01-property.md) §8 | 01 | Later |
| AU-11 | Who licenses security guards outside Dubai, and how? | [09-manpower.md](verticals/09-manpower.md) §2 | 09 | Launch 09 outside Dubai |
| AU-12 | Do cleaners in food establishments need an occupational health card? | [09-manpower.md](verticals/09-manpower.md) §2 | 09 | Onboarding |
| AU-13 | Who licenses training centres in Ajman, Umm Al Quwain and Fujairah? | [10-edtech.md](verticals/10-edtech.md) §2 | 10 | Launch 10 outside Dubai |
| AU-14 | Does ADAFSA require a person in charge on each shift, like Dubai? | [06-fnb.md](verticals/06-fnb.md) §2 | 06 | Launch 06 outside Dubai |
| AU-15 | How long is a food handler's occupational health card valid? | [06-fnb.md](verticals/06-fnb.md) §2 | 06 | Launch 06 |
| AU-16 | What licence terms do DoH and MOHAP use for practitioners? | [05-clinics.md](verticals/05-clinics.md) §2 | 05 | Launch 05 |
| AU-17 | First-submission windows for claims: does Dubai's 2025 directive keep "by the 15th of next month", and does DoH's 24-hour rule still apply? | [05-clinics.md](verticals/05-clinics.md) §2, §13 q4 | 05 | Launch 05 |
| AU-18 | Do DHA or DoH restrict what an appointment reminder may say? | [wl-scheduling](modules/wl-scheduling.md) §10 | 05 | Launch 05 |
| AU-19 | Which DET activity codes apply to each workshop type? | [08-auto.md](verticals/08-auto.md) §2 | 08 | Onboarding |
| AU-20 | Must a business notify an authority (food safety, health regulator, MoIAT, consumer protection) of a product recall, by when and in which form? | [wl-inventory](modules/wl-inventory.md) §10; [05-clinics.md](verticals/05-clinics.md) §4, §9; [06-fnb.md](verticals/06-fnb.md) §4, §9; [07-manufacturing.md](verticals/07-manufacturing.md) §4, §9 | 05, 06, 07 | Launch 06 |
| AU-21 | What are the statutory time limits for each visa step (e.g. entering the country on an entry permit, medical test, Emirates ID, residence visa)? | [wl-hr](modules/wl-hr.md) §7; [09-manpower.md](verticals/09-manpower.md) §4, §7 | 09 | Launch 09 |
| AU-22 | On what conditions, and how, can an absconding (work abandonment) report be withdrawn or cancelled when the worker returns? | [wl-hr](modules/wl-hr.md) §4, §10; [09-manpower.md](verticals/09-manpower.md) §7 | 09 | Launch 09 |
| AU-23 | After a visa renewal is refused, what is the worker's status, and how long may the worker stay after the visa expires? | [wl-hr](modules/wl-hr.md) §10 | 09 | Launch 09 |
| AU-24 | When a unit is sold or the management agreement ends mid-lease, does Ejari or Tawtheeq need a new registration, or only a change of landlord or manager? | [01-property.md](verticals/01-property.md) §4, §13 q13 | 01 | Launch 01 |
| AU-25 | May a running course batch finish after the course approval or centre permit expires? | [10-edtech.md](verticals/10-edtech.md) §4, §9 | 10 | Launch 10 |

## 6. Client

| ID | Question | Where | Verticals | Gate |
|---|---|---|---|---|
| C-01 | What penetration-test evidence does each clinic client require? | [SECURITY.md](SECURITY.md) §8 | 05 | Launch 05 |
| C-02 | Which of the client's products are on MoIAT's regulated list? | [07-manufacturing.md](verticals/07-manufacturing.md) §2 | 07 | Onboarding |
| C-03 | Which penalties does each manpower client contract set for missing replacement cover? | [09-manpower.md](verticals/09-manpower.md) §4 | 09 | Onboarding |

## 7. Onboarding

| ID | Question | Where | Verticals | Gate |
|---|---|---|---|---|
| O-01 | SIF layout of the first WPS agents (bank or exchange house) our first clients use | [09-manpower.md](verticals/09-manpower.md) §8 | 09 (02 later) | Client 1 |
| O-02 | Bank statement formats (MT940, CSV, Excel) of the first clients' banks | [01-property.md](verticals/01-property.md) §8; [wl-payments](modules/wl-payments.md) §12; [GLOSSARY.md](GLOSSARY.md) | 01, all | Client 1 |
| O-03 | Exact visa step names per emirate and channel (Work Bundle, typing centre) | [wl-hr](modules/wl-hr.md) §7 | 09 | Onboarding |
| O-04 | Invoice wording each insurer requires (approval reference, excess line, claim number) | [08-auto.md](verticals/08-auto.md) §9 | 08 | Onboarding |

## 8. Question packs

Ready to send. Each pack lists the items for one adviser, with the context they need.

### 8.1 For a UAE lawyer (data protection, payments, commercial)

We are building a hosted business software platform for UAE SMEs (one deployment per client,
data hosted in the UAE). Our clients are the data controllers; we are their processor.

1. **PDPL** (L-01, L-02, L-03): Are the executive regulations of Decree-Law 45 of 2021 in
   force? What do they require of a processor: breach notification deadline, log retention,
   records of processing, cross-border rules?
2. **Check-in photos** (L-05): Staff check in on a phone with a photo that a supervisor looks
   at; no face matching. Is that biometric data under the PDPL?
3. **Electronic signatures** (L-10): Which documents need an approved or qualified signature
   rather than a simple one (click-to-accept with OTP): tenancy contracts registered on Ejari,
   MoHRE employment contracts, engagement letters, delivery notes?
4. **Exit** (L-11): After a client leaves we delete data within 30 days. Which sector duties
   (health, AML, tax) would require us to keep an archive instead?
5. **Payments** (L-13): The platform connects each client's own merchant account (Stripe,
   N-Genius) and never holds funds. Are we within the technical service provider exclusion
   of CBUAE Circular 15/2021?
6. **Health data** (L-04, L-12): Is there a health-specific consent age other than 18? Is a
   support session by our staff from outside the UAE "processing outside the UAE"?
7. **Consumer rules** (L-06 to L-09): Marketing messages under Cabinet Resolution 66 of 2023;
   whether feedback requests are marketing; expiry of prepaid packages and vouchers;
   no-show fees and forfeited deposits.
8. **Our own outreach** (L-18): May we send a first, short business email to a named manager
   at a UAE company whose details are published, with an opt-out? Does Cabinet Resolution
   56 of 2024 on telemarketing apply to calls between businesses?
9. **Vertical items** (L-14 to L-17): cash on delivery and the CBUAE; courier licence for
   delivery of a client's own merchants' parcels; outside firms filing via a client's
   EmaraTax login; UAE PASS on the UAE Trust List.
10. **Documents that block work** (L-19): The app refuses an act while a required document
    is expired (booking a patient with a practitioner, dispatching a driver, deploying a
    worker, a tutor's session, shipping a regulated product, a fire-safety vendor's visit).
    For which documents does the law itself forbid the act, so that no override may be
    offered? Is a one-off, second-person approved override lawful for the others?
11. **Wages and cheques** (L-20, L-21): May a cash shortage be deducted from wages without a
    finding of fault, and under which Art. 25 category? For returned cheques: which return
    reasons still support a criminal complaint, the deadline to start execution, whether a
    partly honoured cheque may be presented again, and what the notice to the drawer says.
12. **Tenancy** (L-22): Demand period and grounds for eviction in Dubai and Abu Dhabi;
    repossessing an abandoned unit; early termination penalties and returned cheque fees
    without a contract clause; holding over in Abu Dhabi; keeping a deposit when a prospect
    backs out before signing.
13. **Employment** (L-23 to L-29): Set-off beyond the Art. 25 caps on a negative final
    settlement; what may be withheld from an absconding worker; settlement and repatriation
    after a death in service; withdrawing a notice without agreement; overtime cap
    exceptions and penalty; pay and exposure for work without a valid permit; whether a
    deemed-approved timesheet is evidence in a billing dispute.
14. **Sanctions** (L-30, L-31): Do the 24 hours to freeze run from the list update or from
    the match, and what form and deadline apply to the report? May an accounting firm still
    file a tax return for a client whose funds are frozen?
15. **Customers** (L-32 to L-36): What a customer is owed beyond the deposit when the
    business cancels a booking; refusing a refund for a failed delivery caused by the
    customer; billing a patient for a payer share the clinic submitted late; which legal
    parts supply period applies to which part; vehicles never collected and storage fees.
16. **Construction** (L-37): Retention on termination; a subcontractor's right to suspend
    work for non-payment; claims against an insolvent main contractor.
17. **Clinic marketing** (L-38): Limits on loyalty rewards or discounts for medical services,
    and whether a regulator must approve campaign content.

### 8.2 For a UAE tax accountant

1. **Healthcare and insurers** (A-01): Under VATP016, when is the insurer the recipient of a
   clinic's supply (5%) rather than the patient? Which contract terms decide it?
2. **Claims and e-invoicing** (A-02): Are claims submitted through eClaimLink or Shafafiya
   within the e-invoicing mandate from 2027, and does a claim need a separate e-invoice?
3. **Designated zones** (A-03): For a parts dealer selling into designated zones, which
   facts per customer decide the treatment under Executive Regulation Art. 51?
4. **Fees and compensation** (A-04 to A-07): Is each of these consideration (5%) or
   compensation outside the scope of VAT (VATP001): a cancellation fee kept from a
   prepayment, a returned cheque fee, a lease early termination penalty, a damages charge
   above the deposit, liquidated damages deducted from a payment certificate?
5. **Recharges** (A-08): A property manager invoices a negative owner balance (vacancy,
   maintenance, repairs) to the owner. What VAT applies to the recharged costs?
6. **Bad debts, e-invoicing and debit notes** (A-09 to A-11): Content and timing of the
   bad-debt notice, the period of the relief, partly paid invoices and later recovery; is a
   document valid before the ASP accepts it, the transmission time, data fields and how a
   rejection is corrected; when input VAT is reduced for a supplier debit note.
7. **Late export evidence** (A-12): After the zero rate was corrected at 90 days, can it be
   restored when the exit evidence arrives later?
8. **Construction retention** (A-13): Is VAT due on retention at the one-year limit while
   the client still holds it, and how is retention the client keeps treated?
9. **Corporate tax corrections** (A-14): Can a small corporate tax error be corrected
   without a voluntary disclosure?
10. **Loyalty points** (A-15): VAT on a redemption, on free points and their expiry, and how
    the outstanding points are booked.
11. **Owner drawings** (A-16): VAT and corporate tax treatment of drawings, including goods
    taken for private use.
