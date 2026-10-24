# Demo Data

> Status: draft · Last updated: 2026-10-15

Each vertical ships one **demo company**: a fictional but realistic UAE business, with enough
history that dashboards, reports and automations look alive the moment a prospect logs in.
This is the platform's main sales asset. The approach follows the Renox bike shop: the
instance resets every night, demo logins are listed on the login page, and every screen has
data in it.

## 1. Common rules

### 1.1 Generation principles

1. **Fictional, never copied.** Company, people, buildings, products and suppliers are
   invented. No real person, company, building or brand appears, even as a joke.
2. **Bilingual.** Every party has an English and an Arabic name. Roughly a third of people
   use Arabic names, the rest reflect the UAE's expatriate mix (South Asian, Filipino,
   Arab, European, African). Addresses use real emirates and generic community names
   ("Business Bay", "Al Quoz Industrial 3"), never real building names.
3. **Money looks real.** AED amounts at plausible market levels, VAT applied correctly per
   line (standard, zero-rated, exempt), round numbers avoided.
4. **History.** **15 months** of activity ending "today": dashboards show trends, year-on-year
   comparisons and seasonality (Ramadan, summer slowdown, Q4 peak).
5. **Every state of every state machine** has at least two records, so each status badge,
   filter and action button can be shown.
6. **Exceptions on purpose.** Each demo has a few records that make automations visible:
   overdue invoices, expiring documents, bounced cheques, SLA breaches, pending approvals,
   low stock. These are listed per vertical as *triggers*.
7. **Relative dates.** Data is generated relative to the reset date, so "expires in 7 days"
   is always true on the day of the demo.
8. **Deterministic.** The same seed produces the same data, so scripts and screenshots stay
   valid. A different seed is used only for load tests.
9. **Multi-entity where natural.** At least one entity per demo; verticals 06 and 09 show two
   legal entities ([decision 0003](decisions/0003-legal-entities.md)).

### 1.2 Fake identifiers

| Identifier | Demo format | Rule |
|---|---|---|
| Emirates ID | `784-1990-0000NNN-C` (7-digit block starting `0000`) | Format-valid; no published ICP test range was found, so the `0000` block is our choice, not a reserved one `⚠ open: authority (AU-01)`. Demo records carry a visible **DEMO** badge |
| TRN | `100000000NNNNNN` with a `DEMO` note on documents | 15 digits so validation passes; documents print "Demo — not a tax invoice". No published FTA test range was found `⚠ open: authority (AU-01)` |
| Trade licence | `DEMO-NNNNNN` | Never a real authority format |
| Passport | `X0000NNN` | — |
| Phone | `+971 50 000 NNNN` | No TDRA range reserved for fictional use was found `⚠ open: authority (AU-01)`; outbound SMS and WhatsApp are disabled in demo instances anyway |
| Email | `name@<company>.example` | RFC 2606 reserved domain; outbound email goes to a catch-all mailbox |
| IBAN | `AE00 0000 …` | Checksum deliberately invalid so it can never be used |
| Vehicle plates, VINs | `DEMO 12345`, VIN with `DEMO` in positions 4–7 | — |

### 1.3 Reset policy

- **Nightly reset at 03:00 Gulf time:** the database is replaced with a freshly generated one.
  A banner on every page says "Demo data, reset every night".
- **Per-prospect instance on request** (`OPS.DMO.01`): a copy that does not reset for 14 days,
  so a prospect can try it with a colleague. It expires and is deleted automatically.
- Uploaded files, sent messages and created users disappear at reset.

### 1.4 Privacy and safety

- **No outbound messages.** Email goes to a catch-all, SMS and WhatsApp are simulated and
  shown in an in-app "outbox" so the demo still shows them.
- **No real payments.** Gateways run in test mode; card fields accept only the provider's
  published test numbers.
- **No real personal data**, ever, including in uploaded sample documents. Sample passports,
  Emirates IDs and trade licences are clearly watermarked "SPECIMEN — DEMO".
- Prospects must not upload real client data. The login page and upload dialogs say so.

### 1.5 Demo logins

The login page lists one account per main role with a one-tap login, like the bike shop
(`BIKESHOP_DEMO_LOGINS`). The password is the same for all and is shown on the page. Staff
two-factor login is turned off in demo instances only. Portal users (tenant, patient,
parent…) have their own one-tap logins on the portal login page.

### 1.6 How demo instances are created

The operator console ([`wl-operator`](modules/wl-operator.md), `OPS.DMO.01`):

1. creates a demo client from the vertical's `client.toml` preset, with the demo company's
   branding;
2. runs the vertical's demo generator with the seed and the reset date;
3. registers the nightly reset, or the 14-day expiry for a per-prospect copy;
4. lists the demo URL and logins for the sales team.

Each vertical's demo generator is versioned with the vertical app, so the data always matches
the current state machines and rules. A demo that fails to generate blocks the release.

### 1.7 Per-vertical template

Each section below has: **company profile**, **demo users**, **volumes**, **triggers**
(records that make automations visible), the **10-minute demo script** with its three *wow*
moments, and **must not appear**.

---

## 01 — Property: *Sahraa Property Management LLC*

**Profile.** Dubai, RERA-licensed manager. 32 staff, 4 buildings (2 residential towers in
"Jumeirah Village", 1 mixed-use in "Business Bay", 1 warehouse block in "Al Quoz"), 412 units.
Manages for 38 owners. One legal entity.

**Demo users.** Company admin · Property manager (Tower A and B) · Property manager (Business
Bay) · Leasing agent · Accountant · Maintenance coordinator · Technician (PWA) · Vendor
(AC contractor) · Tenant primary contact (individual) · Tenant finance contact (corporate) ·
Owner.

| Entity | Volume |
|---|---|
| Units | 412 (93% occupied, 11 notice given, 6 under maintenance, 8 off-market) |
| Leases | 380 active, 140 historical, 12 drafts, 6 pending signature |
| Cheques | ~2,100 over 15 months; 140 due in the next 30 days |
| Invoices | ~4,800; ageing spread across all buckets |
| Tickets | ~1,600; 45 open |
| Owner statements | 15 monthly runs |

**Triggers.** 9 leases in the 90-day renewal window · 2 leases expired without renewal ·
3 bounced cheques (one replaced) · 4 overdue invoices > 60 days · 1 Ejari number missing
14 days after activation · 2 tickets breaching SLA · 1 deposit refund above AED 5,000
waiting for approval · 3 tenant Emirates IDs expiring within 30 days.

**10-minute script.**
1. Log in as the property manager: the "needs action" list and the occupancy dashboard.
2. Open a unit, then its lease: history, documents, cheques, invoices on one page.
3. Renew a lease from the renewal list: new version, new cheques, contract PDF in English
   and Arabic. **Wow 1:** a bilingual tenancy contract generated in one click.
4. Switch to the tenant portal on a phone: invoices and the cheque schedule, then raise an AC
   ticket with a photo. (Online payment is Later; do not show a pay button.)
5. Back office: the ticket appears, is assigned to the vendor, and the vendor closes it from
   the PWA with before/after photos. **Wow 2:** the tenant gets the update without anyone
   calling.
6. Accountant: today's cheque deposit list, mark one bounced. The tenant and manager are
   notified and a follow-up task appears. **Wow 3:** an automation the prospect does today
   by memory.

**Must not appear.** Real building or community developer names, real owners, real Ejari
numbers.

**The local demo (`property:demo`).** The host's `property:demo` command, run by `db:seed`
when `APP_ENV=local` (and by the command itself in `testing`; never in production), makes a
smaller *Palm Grove Properties LLC* (entities PGP and PGFZ): 6 buildings in four emirates, 62
units, 49 lease versions in every state, ~160 cheques, ~110 invoices with payments, deposits
held, refunded, forfeited and handed to a new owner, owner statements in every state, tickets
with a vendor and an SLA breach, leads, campaigns, staff and 15 months of history. It is made
through the modules' own services and routes as the demo users (so permissions, guards and
state machines are the real ones) with the simulated clock moved to each step's day; the
only states it does not make are *reserved* units (the module refuses to reserve until
booking deposits exist) and *archived* documents (no page archives). Seeding twice changes
nothing (an audit entry `demo.seeded` marks it done). It takes about 12 s in a release build.
Logins, all `@palmgrove.example` with the password of `seed.rs`: `admin`,
`property_manager` (JVT-A and JVT-B), `property_manager.bay` (BBY, AQZ),
`property_manager.north` (ADR, SHJ), `property_manager.all` (global: every building),
`leasing_agent`, `accountant`, `receivables`, `finance_manager`, `owner`, `coordinator`,
`facility_manager`, `technician`, `compliance`, `marketing`, `marketing_manager`, `hr`. The
login page lists them in local. The building-scoped managers see the module lists (invoices,
cheques, deposits, parties, tickets, ...) of their buildings only ([ACCESS-CONTROL.md](ACCESS-CONTROL.md) §10).

**Walking the lease path in the browser** (each step by the role that does it): the leasing agent
adds the party (`/parties/new`) and its documents (button *Documents* on the party), the
`compliance` role accepts them (`/documents/review`; nobody accepts their own upload), the
agent drafts and submits the lease on the unit page, the property manager adds the signed
contract to the lease's documents, compliance accepts it, the accountant records the deposit
received (button on the lease), the property manager activates the lease and records the Ejari
number (after adding the certificate), the agent records the cheques, the nightly run (or
`host property:invoice-charges`) drafts the invoice of a due instalment, a receivables clerk
issues it, the accountant deposits and clears the cheque (the invoice is paid) or records its
bounce, and the property manager records the notice, the inspection and the move-out.

---

## 02 — Construction: *Al Waha Interiors & Fit-out LLC*

**Profile.** Dubai fit-out and MEP subcontractor. 210 workers, 18 office staff, 7 active
projects (offices, a mall unit, a clinic, a restaurant), contract values AED 0.6m–9m. One
legal entity.

**Demo users.** MD · Project manager · Quantity surveyor · Site supervisor (PWA) ·
Storekeeper · Procurement officer · Accountant · HR/PRO · Client contact (Later portal).

| Entity | Volume |
|---|---|
| Projects | 7 active, 9 completed (4 in defects liability), 2 in setup |
| BOQ lines | ~3,500 across projects |
| Attendance | 15 months × ~180 workers per day |
| Purchase requests / POs / GRNs | ~900 / ~650 / ~800 |
| IPAs | ~95, incl. revisions and certification gaps |
| Variations | 60 (approved, rejected, pending) |
| Guarantees | 14 (advance and performance) |

**Triggers.** A project at 92% of a cost code budget · 2 IPAs submitted > 14 days without
certification · a performance bond expiring in 25 days · a material request above AED 50,000
awaiting MD approval · 3 workers checked in during the midday break (summer data) · a site
with no daily report yesterday.

**10-minute script.**
1. MD dashboard: margin forecast per project, claimed vs certified vs received.
2. Site supervisor on a phone: crew attendance in two taps, daily report with photos.
   **Wow 1:** attendance on site inside the geofence, no paper.
3. Raise a material request; it goes to the PM, then the MD (two-step approval); procurement
   turns it into a PO.
4. Month-end: the draft IPA is already created; the QS enters progress, retention and
   advance recovery calculate themselves. **Wow 2:** the payment application that takes
   them two days in Excel.
5. Project cost report: the budget warning on the over-spending cost code.
   **Wow 3:** they see the loss before the project ends.

**Must not appear.** Real main contractors, consultants or malls; real worker passports.

---

## 03 — Logistics: *Najm Express Cargo LLC*

**Profile.** Dubai last-mile courier with a freight forwarding desk in Jebel Ali. 140 staff,
65 vans and 8 trucks, 2 hubs. B2B clients are online shops. One legal entity.

**Demo users.** Operations manager · Dispatcher · Hub staff · Driver (PWA) · Documentation
officer · Fleet manager · Accountant · Customer service · Client user (online shop) ·
Recipient (tracking link).

| Entity | Volume |
|---|---|
| Shipments | ~420,000 over 15 months (~900 per day), 3% failed first attempt |
| Runs | ~25,000 |
| COD | ~40% of shipments; weekly remittances to 60 clients |
| Freight jobs | ~600 (sea, air, road) |
| Vehicles | 73, with fines and tolls |

**Triggers.** 2 drivers with cash shortages at handover · 1 COD remittance overdue to a
client · 3 vehicle registrations expiring · a van past its service kilometres · 40 shipments
awaiting reattempt · a vehicle grounded by an expired registration.

**10-minute script.**
1. Client portal: upload 50 shipments by CSV, print labels.
2. Dispatcher builds runs; driver app shows the day's stops. **Wow 1:** proof of delivery
   with photo, signature and location appears live in the client portal.
3. A failed attempt: reason, reattempt scheduled, recipient notified by SMS (outbox).
4. End of day: cash handover counted by a second person; shortage flagged.
   **Wow 2:** COD money they can finally trace.
5. Monthly invoice from the rate card, with COD fees and surcharges.
   **Wow 3:** one invoice from 6,000 transactions.

**Must not appear.** Real courier brands, real shop names, real recipient addresses.

---

## 04 — Accounting: *Bayan Accounting & Tax Consultants*

**Profile.** Dubai firm, FTA-registered tax agency, 22 staff, 180 SME clients on monthly
retainers. One legal entity.

**Demo users.** Partner · Manager · Accountant · Tax specialist · MLRO · Billing admin ·
Client owner (portal) · Client finance contact (portal).

| Entity | Volume |
|---|---|
| Client parties | 180 with UBO structures; 12 high-risk |
| Engagements | 220 active |
| Tasks (deadlines) | ~7,500 over 15 months; quarterly VAT peaks |
| Document requests | ~2,000 |
| Returns | ~900 VAT, ~150 corporate tax |
| Time entries | ~40,000 |

**Triggers.** 6 VAT returns due in 5 days with documents still missing · 3 CDD files due for
review · 1 sanctions re-screening hit pending · 8 retainers past due · an engagement with low
realisation.

**10-minute script.**
1. Manager's deadline calendar: this month's VAT returns by status.
2. Client portal: the client sees a request list and uploads invoices from a phone.
   **Wow 1:** no more chasing documents on WhatsApp.
3. Return prepared, client approves in the portal (e-signature); filing evidence attached.
4. MLRO: a high-risk client's EDD file, re-screening result. **Wow 2:** AML evidence ready
   for an inspection.
5. Partner dashboard: WIP, realisation and retainers by client. **Wow 3:** which clients lose
   money.

**Must not appear.** Real client companies, real TRNs, real sanctions-list names (screening
uses a fictional list).

---

## 05 — Clinics: *Shifa Family & Dental Clinics*

**Profile.** Two DHA-licensed branches in Dubai (dental, physiotherapy, aesthetics),
14 practitioners, 30 other staff. One legal entity.

**Implemented as** `clients/demo-clinics/client.toml` and `host/src/app/clinics_demo/`
(*Palm Grove Clinic LLC*, two branches, three practitioners, twelve patients, three payers; the
story is the one of the table below at a smaller size). `APP_ENV=local
CLIENT_CONFIG=clients/demo-clinics/client.toml cargo run -p host -- migrate`, then
`-- clinics:demo` (or `-- db:seed`), then `-- serve`; the logins are on the login page, password
`password123`. Every date is relative to today. Bookings, visits, charges, claims, stock,
sales and payments are made through the real services and the app's own pages as the user who
does them; roles are held in all branches (the clinics pages check a permission outside any
branch), and the branch is the employee's scope in HR.

**Demo users.** Medical director · Clinic manager · Receptionist · Call-centre agent ·
Practitioner (dentist) · Nurse · Insurance coordinator · Accountant · Patient · Guardian.

| Entity | Volume |
|---|---|
| Patients | 9,500 (25% minors with guardians) |
| Appointments | ~52,000; 6% no-show |
| Packages | 700 (physio, aesthetics) |
| Prior authorisations / claims | ~2,500 / ~30,000; 7% denied, resubmissions |
| Insurers / TPAs | 12 fictional |

**Triggers.** A practitioner licence expiring in 20 days (blocks bookings after expiry) ·
claims with denials awaiting resubmission · 5 packages expiring this week · a full day with
a waiting list · a low-rating feedback opening a complaint ticket.

**10-minute script.**
1. Patient books online on a phone, picks a dentist and a slot; gets a WhatsApp reminder
   (outbox). **Wow 1:** self-booking that respects each dentist's roster.
2. Reception: check-in, insurance eligibility recorded, copay collected.
3. A cancellation frees a slot; the waiting-list patient is offered it automatically.
4. Insurance coordinator: claim batch, a denial with reason, resubmission.
   **Wow 2:** denials tracked to money recovered.
5. Medical director dashboard: utilisation per practitioner, no-show rate, revenue by payer.
   **Wow 3:** the licence about to expire, flagged before it becomes a regulator problem.

**Must not appear.** Any clinical notes or diagnoses beyond claim codes, real insurers, real
DHA licence numbers.

---

## 06 — F&B: *Zaatar & Co. Kitchens*

**Profile.** A cloud kitchen group in Dubai and Sharjah: 3 kitchens, 5 virtual brands, 1
central production kitchen, 1 takeaway counter (dine-in is Later). **Two legal entities** (Dubai LLC and Sharjah
LLC), each with its own TRN and numbering.

**Demo users.** Operations director · Area manager · Kitchen manager · Head chef · Line cook
(KDS) · Cashier · Storekeeper · Menu manager · Accountant · Rider · Customer.

| Entity | Volume |
|---|---|
| Orders | ~310,000 (aggregators 70%, direct 25%, counter 5%) |
| Recipes / items | 180 recipes, 420 stock items with lots |
| Stock counts | weekly per kitchen |
| Payout statements | weekly per aggregator, with variances |
| Daily closings | per outlet per day |

**Triggers.** A payout statement with unmatched AED 1,240 · items below par in 2 kitchens ·
a lot expiring tomorrow · theoretical vs actual food cost gap of 4 points in one kitchen ·
a fryer fault ticket open for 2 days.

**10-minute script.**
1. Kitchen screen: orders from three aggregators and the direct site on one KDS.
   **Wow 1:** one screen instead of four tablets.
2. Direct ordering site on a phone with a promo code; order flows to the KDS.
3. Stock count in the kitchen, variance approved.
4. Food cost report: theoretical vs actual per kitchen. **Wow 2:** where the money leaks.
5. Aggregator payout reconciliation with the missing amount highlighted.
   **Wow 3:** commissions and deductions they never checked.

**Must not appear.** Real aggregator logos or brand names on screens (use "Aggregator A/B/C"
in the demo), real restaurant brands.

---

## 07 — Manufacturing: *Emirates Pack Industries LLC*

**Profile.** Packaging manufacturer in Sharjah (corrugated boxes, printed sleeves), 2
production lines, 85 staff, B2B customers in F&B and retail. One legal entity, with an ICV
certificate on file.

**Implemented as** `clients/demo-manufacturing/client.toml` and `host/src/app/manufacturing_demo/`
(the story at a smaller size: one plant with two lines, nine items with their lots, three released
BOMs, five customers with credit accounts, five sales orders, seven work orders, two NCRs, one
supplier recall, five dispatches). `APP_ENV=local CLIENT_CONFIG=clients/demo-manufacturing/client.toml
cargo run -p host -- migrate`, then `-- manufacturing:demo` (or `-- db:seed`), then `-- serve`; the
logins are on the login page, password `password123`. Every date is relative to today. The work is
done through the app's own routes as the person who does it (BOMs, work orders, issues, reports, NCRs,
the recall, the credit accounts, the sales orders, the picks); the opening stock, the lots, the
certificates and the delivery that came back in part are inserted as the modules' services and
records leave them.

| Customer | What it shows |
|---|---|
| Al Noor Trading LLC | a work order running for its order (one report entered late from a paper job card), a dispatch that left and came back in part (returned goods for QC and sales), a portal buyer |
| Gulf Fresh Foods LLC | food boxes (food-contact certificate), a planned work order, a dispatch picked |
| Emirates Print House LLC | printed sleeves made for it alone: the cancel sheet proposes *scrap* |
| Sahara Retail LLC | an invoice 31 days overdue, so its next order is on credit hold |
| Atlas Gulf Export FZE | two zero-rated exports, one with its customs exit certificate and one 65 days out without it |

**Demo users.** General manager / owner · Company admin · Production manager · Production planner ·
Line supervisor · Line operator · QC inspector · Storekeeper · Sales · Sales manager · Accountant ·
Customer buyer (portal).

**Screens.** `manufacturing:demo` ends by printing the screen map (URL, the login that owns it, one
line on what it shows), with the ids read from the database; the code is
`host/src/app/manufacturing_demo/screens.rs` (`host::manufacturing_demo::screen_map`) and
`host/tests/manufacturing_demo_screens.rs` opens every URL as its owner. The groups: **Start**
(`/dashboard` with the tiles of the plant, `/menu`, `/approvals`), **Work orders**, **Bills of
materials**, **Quality** (NCRs, inspections of returned lots, conformity, settings), **Sales** (orders,
credit accounts, returned goods), **Stock and shipping** (stock, dispatches, holds, recalls, export
evidence), **Money**, **People and documents** and the **customer portal**.

| Entity | Volume |
|---|---|
| Sales orders | ~3,200 |
| Work orders | ~4,000 |
| BOMs | 260 with versions |
| Lots | ~9,000 (raw and finished) |
| NCRs | ~150 |
| Dispatches / deliveries | ~3,000 |

**Triggers.** A customer on credit hold with a new order · 2 NCRs open on one supplier's
board · a work order behind its planned start · a raw-material lot expiring · an overdue PO.

**10-minute script.**
1. Customer buyer places a repeat order in the portal.
2. The order goes on credit hold; the accountant releases it.
3. Work order issued to the line tablet; QC checklist with a failed measurement opens an NCR.
   **Wow 1:** quality issues recorded at the machine, not on paper.
4. Lot traceability from a finished pallet back to the supplier's board lot.
   **Wow 2:** answer a customer complaint in seconds.
5. Dispatch with proof of delivery; invoice issued. **Wow 3:** order-to-cash on one timeline.

**Must not appear.** Real customers' brands on printed products, real ICV certificate numbers.

---

## 08 — Auto: *Deira Auto Spares & Service Centre*

**Profile.** Parts trading in Deira with a workshop in Al Quoz. 3 branches, 55 staff, 18,000
SKUs, 400 B2B workshop customers, 6 fleet contracts. One legal entity.

**Demo users.** Owner · Branch manager · Counter sales · Sales rep · Purchaser · Storekeeper ·
Accountant · Service advisor · Workshop manager · Technician · Delivery driver · B2B buyer ·
Fleet contact · Car owner.

| Entity | Volume |
|---|---|
| Sales orders / counter sales | ~45,000 / ~60,000 |
| Job cards | ~9,000, incl. insurance repairs |
| Bookings | ~7,000 |
| Warranty claims | ~300 |
| Returns | ~2,500 |
| Vehicles | ~6,500 customer vehicles |

**Triggers.** 4 workshops over their credit limit · dead stock worth AED 180,000 · a reorder
list ready this morning · an insurance repair waiting for the surveyor's approval · an order
ready but not collected for 5 days.

**10-minute script.**
1. Counter: search a part by cross-reference or make, model and year, sell with a VAT
   receipt (VIN search is Later).
   **Wow 1:** find the right part across brands in seconds.
2. B2B buyer orders online; the driver delivers with proof of delivery and a collected cheque.
3. Service advisor books a car, opens a job card, adds parts from stock.
4. An insurance repair: estimate approved by the insurer, excess collected from the owner,
   invoice to the insurer. **Wow 2:** insurer and customer billed correctly from one job.
5. Owner dashboard: margin by category, dead stock, reorder suggestions.
   **Wow 3:** cash tied up in parts that do not move.

**Must not appear.** Real car-brand logos, real insurer names, real VINs or plates.

**Implemented as** `clients/demo-auto/client.toml` (a fictional Deira / Sharjah parts trader with a
workshop; parties, documents, notify, invoicing, ticketing, payments, sales, hr, scheduling,
inventory and portal) and the seeds of `host/src/app/auto_seed.rs`, `auto_settings.rs` and
`auto_money.rs`: a fresh deployment starts with the workshop service types, the ticket categories,
the document kinds (police accident report, claim documents, trade licence, fleet contract), the
08 values for the shared modules (back-orders on the counter, phone and sales rep channels, the
cheque, cash and stock limits, the loyalty programme, the onboarding checklists and the SOP
library) and the two settings pages. **There is no demo company yet: the command `auto:demo` is
future work and not built**, so the 10-minute script above is walked by hand on an empty
deployment (the tests of `apps/auto` and `host/tests/auto_*.rs` hold the fixtures).

---

## 09 — Manpower: *Khaleej Workforce Solutions*

**Profile.** Manpower supplier (cleaning, security, hospitality, IT) in Dubai with a
free-zone IT arm. **Two legal entities** (mainland LLC and free-zone FZ-LLC). 650 employees,
45 client companies.

**Implemented as** `clients/demo-manpower/client.toml` and `host/src/app/manpower_demo.rs`
(*Desert Star Manpower Services LLC*, three clients, 14 workers; the story is the one of the
table below at a smaller size). `APP_ENV=local CLIENT_CONFIG=clients/demo-manpower/client.toml
cargo run -p host -- migrate`, then `-- manpower:demo` (or `-- db:seed`), then `-- serve`; the
logins are on the login page, password `password123`. Every date is relative to today.

**Demo users.** MD · Account manager · PRO · HR/payroll officer · Site supervisor ·
Accountant · Worker (PWA) · Client approver (portal) · Client viewer.

| Entity | Volume |
|---|---|
| Employees | 650 active, 300 left (with final settlements) |
| Visa cases | ~1,400 in all steps |
| Deployments | ~1,100 |
| Timesheets | ~14,000 approved by clients |
| Payroll runs | 15 per entity, with WPS files |
| Client invoices | ~800 from rate cards |

**Triggers.** 12 visas expiring within 60 days · a client approver who has not approved last
week's timesheets · a payroll run waiting for second approval · an employee on notice with a
draft final settlement · a deployment where no one checked in.

**10-minute script.**
1. Client requests 10 cleaners in the portal; account manager deploys from the bench.
2. Workers check in on site; the client approves the week's timesheets in the portal.
   **Wow 1:** client approval without signed paper sheets.
3. Usage invoice from the rate card (overtime, Friday multiplier).
4. Payroll run per entity and the WPS file. **Wow 2:** billing and payroll from the same
   approved hours.
5. PRO dashboard: visas and Emirates IDs by expiry. **Wow 3:** no more fines for missed
   renewals.

**Screens.** `manpower:demo` ends by printing the screen map (URL, the login that owns it, one
line on what it shows), with the ids read from the database; the code is
`host/src/app/manpower_demo/screens.rs` (`host::manpower_demo::screen_map`) and
`host/tests/manpower_demo_screens.rs` opens every URL as its owner. The groups:

- **Start:** `/dashboard` (admin, account manager, HR: the tiles differ), `/menu`, `/approvals` (MD).
- **Contracts:** the list (and its two tile filters), the running contract, the one ending in
  45 days, the one on credit hold (Al Quoz: the seed runs the hold job on its overdue invoice).
- **Rate cards:** a card and a version. **Requests:** the list, a part-filled and an open request.
- **Deployments:** the list (account manager and site supervisor), an active and a suspended worker.
- **Absence:** the list and a case (HR). **Timesheets:** the list, one waiting for the client, one locked and billed.
- **Payroll:** runs, a run, settlements, a settlement, a pay account (HR).
- **Workers:** the list, a worker's page (HR, and the account manager without salary), the salary
  after Reveal, the worker's documents.
- **Client portal:** the approver's home, requests, deployments, timesheets and a timesheet page.

**Must not appear.** Real client companies, real worker identities or salaries of real
people.

---

## 10 — EdTech: *Noor Learning Centre*

**Profile.** KHDA-licensed training and tutoring centre in Dubai with a satellite in Abu
Dhabi. 35 tutors, 1,200 active students (60% minors), corporate training contracts. One
legal entity.

**Demo users.** Owner · Centre manager · Admissions counsellor · Academic coordinator ·
Tutor · Accountant · Adult student · Parent · Corporate client contact.

| Entity | Volume |
|---|---|
| Leads | ~6,000 (trials, conversions) |
| Batches / sessions | ~450 / ~28,000 |
| Enrolments | ~5,000 |
| Subscriptions / packages | 900 / 1,100 |
| Submissions / certificates | ~22,000 / ~2,400 |

**Triggers.** Failed subscription payments in retry · packages expiring with credits left ·
a tutor permit expiring · 3 leads with no follow-up for a week · a batch below minimum size.

**10-minute script.**
1. The counsellor books a trial for a parent's enquiry, then converts the lead to an
   enrolment with a sibling discount (online self-booking is Later).
2. Tutor marks attendance on a phone; a package credit is consumed. **Wow 1:** the parent
   sees attendance and published assessment results in the portal the same day.
3. Monthly subscription charged by card; a failed payment retried.
4. Certificate issued with a verification link. **Wow 2:** verifiable certificates.
5. Manager dashboard: batch fill rates, lead conversion, revenue per course.
   **Wow 3:** which courses actually make money.

**Implemented as** `clients/demo-edtech/client.toml` (*Gulf Skills Training Centre LLC*, one legal
entity, the modules parties, documents, notify, invoicing, ticketing, payments, sales, hr, attendance,
scheduling and portal) and the seeds of `host/src/app/edtech_seed.rs`, `edtech_settings.rs` and
`edtech_money.rs`: a fresh deployment starts with the three service types (trial class, placement test,
one-to-one), the ticket categories, the document kinds (centre permit, course approval, police clearance,
tutor permit or freelance visa, student ID, guardian consent, trademark registration, other certification),
the lead sources, the loyalty programme, the onboarding checklists, the SOP library and the budget
categories. `APP_ENV=local CLIENT_CONFIG=clients/demo-edtech/client.toml cargo run -p host -- migrate`,
then `-- serve`.

**The demo company (`edtech:demo`).** `host/src/app/edtech_demo/` makes *Gulf Skills Training Centre LLC*
(one legal entity and no campuses in the client file, so one site) at a smaller size than the table above,
with the same stories. `APP_ENV=local CLIENT_CONFIG=clients/demo-edtech/client.toml cargo run -p host --
migrate`, then `-- edtech:demo` (or `-- db:seed`, which runs it in `local` only), then `-- serve`; the logins
are on the login page, password `password123`: `admin@`, `owner@`, `manager@` (centre manager), `counsellor@`,
`coordinator@` (academic coordinator), `accountant@` and five tutors (`tutor@`, `omar@`, `layla@`, `ravi@`,
`hind@`), all `@gulfskills.example`. Every date is relative to the latest Friday up to today (the batches meet on fixed weekdays, so the story is the same whatever day it is seeded), money is in fils, names are in English
and Arabic, and the seed is deterministic and idempotent (an audit entry marks it done). It takes about 7 s
in a debug build.

| Screen | What the seed puts there |
|---|---|
| Courses | eight courses in four programmes (language, IT, exam preparation, kids), two with a KHDA approval on file |
| Batches | sixteen: planned (2), open (3, one full with a waiting list), running (5), completed (2), cancelled (2), terminated (2) |
| Sessions | scheduled, held and cancelled (one with a make-up); held ones carry present, late, absent and excused marks, an excused notice earns a make-up right, two missed sessions in a row start the absence rule |
| Tutors (wl-hr) | three employed and two freelance, with police clearance and permit as documents: one clearance ends in 20 days, one ended 10 days ago, one freelance permit ends in 34 days; the sessions after the date are flagged and the coordinator has the task |
| Enrolments | pending payment (a discount waiting for the centre manager, a seat held 48 hours), active, frozen (by the student; by a block), withdrawn with a refund, lapsed, completed, transferred, a package of private sessions with credits used, instalments |
| Students | nineteen students (children and a sixteen-year-old with guardians, one blocked) and a corporate payer |
| Money | invoices issued by the accountant, cash, card and transfer payments, one online card payment charged back (the counsellor has the task to collect), refunds requested, approved and rejected |
| Certificates | draft, issued (public page `/verify/certificate/{code}`), revoked, superseded and reissued |
| Documents | the centre permit ends in 45 days, so the batches that run later stay planned and cannot be opened |
| Leads, campaigns, tickets | leads in every state (three untouched for a week), four campaigns (draft, running, ended), a complaint, a fee query, a classroom issue and tasks |

Everything a person does at a screen is a request to the app's own router as the demo user who does it,
so the permissions, guards and machines are the real ones; a batch of the past is planned and opened on the
seed's clock of that day and each session is marked on the evening it was taught. The only record inserted
in a state is the online card payment (`insert_in_state`), because no gateway adapter is installed in the demo
(as in the clinics demo). Not seeded: a membership enrolment with its subscription (the freeze with the cause
*linked*), online self-booking of trial classes, assessments and submissions (Later in the spec), and the
Abu Dhabi satellite, the 35 tutors and the volumes of the table above.

**Must not appear.** Real students or minors' photos, real school names, real KHDA permit
numbers.
