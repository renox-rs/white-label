# UX Specification

> Status: specification, screens built for F1-F10 (not every screen of this document) · Last updated: 2026-10-24
>
> **Build status (2026-10-24):** Built: the shell, the standard list grid and the page-fill rule (§2.3). Target, not built: the `wl-*` components named below (`wl-status`, `wl-money`): today the screens use `rx-badge` kinds and the kit's money formatting. Individual screens are marked built or not in the progress notes and the module specs.

How people use the platform, screen by screen. [DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) defines
the look, and this document defines the layouts, navigation and behaviour. Screens are built
from Renox kit components (`rx-*`), platform components (`wl-*`) and vertical components
([DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) §7). Every behaviour points to a capability ID, a state
machine or an access rule, so an agent can implement it without guessing.

## 1. Four surfaces

| # | Surface | Who | Layout | Style | Built with |
|---|---|---|---|---|---|
| 1 | **Back office** | staff | `rx-shell`: `rx-sidebar` + full-width `rx-navbar` + content | dense; rounded-rectangle buttons; no hero, no photos | `rx-grid`, `rx-table`, `rx-infolist`, `rx-tabs`, `rx-form`, `rx-wizard`, `rx-kanban`, `rx-month-calendar`, `rx-week-calendar` (R5), charts |
| 2 | **Field PWA** | field roles: technician, driver, supervisor, storekeeper, line operator, cook | single column, bottom action bar, no sidebar | large targets (≥ 44 px, `--rx-target`), one task per screen | `rx-page`, `rx-card`, `rx-camera-capture`, `rx-signature-pad`, `rx-geolocate`, `rx-barcode-scan` (R5), `rx-sheet` |
| 3 | **Portal** | external users: tenants, clients, patients, guardians, students, buyers | `rx-navbar` + centred page | editorial: pill buttons, `--rx-type-section`, client branding | `wl-portal` layout, `rx-card-grid`, `rx-infolist`, `rx-form` |
| 4 | **Public pages** | anyone, no login | marketing page + one task | editorial with hero and photos | self-booking (`SCH.SLF.01`), direct ordering (`FNB.DTC.01`), tracking by signed link (`POR.SLF.02` in 03), payment link (`FIN.PAY.02`) |

The field PWA is the back office at phone width with a different layout
(`layouts/field.html`). It uses the same routes, the same permissions and the same state
machines. A role is a *field* role when its default layout is `field`. A field user who
opens a page with no field layout sees the back-office page.

## 2. Platform-wide patterns

### 2.1 Work inbox: "Needs your action"

The first block of every staff home screen is a list of items **this user can act on now**,
in this scope. Each item has one line, a count and a link to a pre-filtered list. The
sources are fixed:

| Source | Item | Fed by |
|---|---|---|
| Approvals | "2 deposit refunds waiting for your approval" | `PLT.APR.01` approval requests where the user holds the current step |
| Available actions | "5 cheques to deposit today" | records whose state machine offers the user a transition today (`PLT.WFL.01`) |
| Assignments | "3 tickets assigned to you" | `ASG` rules: tickets, tasks, deliveries, bookings |
| Time triggers | "1 ticket breached SLA", "4 leases expire in 30 days" | automation runs (`WRK.RUL.01`, `DOC.RUL.01`, `FIN.RUL.02`, …) that flag a record |
| Mentions | "Sara mentioned you on Lease L-0142" | `PLT.NOT.01` |

Rules:

- Items appear only if the user holds the permission in that scope ([ACCESS-CONTROL.md](ACCESS-CONTROL.md) §4).
- An empty inbox shows "Nothing needs your action", followed by the next upcoming item.
- Counts update live through Renox `event-stream`.
- The notification bell (`rx-notification-bell`) shows the same items as notifications. The
  inbox is the list to work through; the bell only tells the user that something is new.

### 2.2 Record page

Every record with a state machine uses one layout:

```
┌──────────────────────────────────────────────────────────────────────┐
│ ‹ Leases                                                              │
│ Lease L-0142 · Unit 1204, Tower A            [ACTIVE]                 │
│ Ahmed Al Mansoori · 01 Jan 2026 – 31 Dec 2026                         │
│                          [ Record notice ] [ Renew ] [ ⋯ More ]       │
├──────────────────────────────────────────────────────────────────────┤
│ Overview │ Charges │ Cheques │ Documents │ Tickets │ Activity         │
├──────────────────────────────────────────────────────────────────────┤
│ rx-infolist (key facts)                  │ Side panel:                │
│                                          │  Balance AED 12,500        │
│                                          │  Next cheque 01 Feb        │
│                                          │  Deposit held AED 6,000    │
└──────────────────────────────────────────────────────────────────────┘
```

| Part | Rule |
|---|---|
| Title | number + the subject's natural name (`Lease L-0142 · Unit 1204, Tower A`) |
| Status | status badge (`rx-badge` kind; `wl-status` is a target, not built); tone from [DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) §6 |
| Actions | exactly the transitions available to this user now (WORKFLOW §3.2 point 5). Primary action = the transition that moves the record forward; destructive ones (`cancel`, `void`, `credit`) go under **More** with an `rx-confirm` |
| Blocked action | shown disabled with a tooltip naming the missing guard ("Signed contract not attached"); a `LIM`/`SOD` action shows "Request approval" instead |
| Tabs | `rx-tabs`: **Overview** (`rx-infolist`) · related records (one tab per related list) · **Documents** (`DOC.FIL.01`, expiry badges) · **Activity** |
| Activity | Renox `history` block merging state transitions, field changes (audit), notes, comments and @mentions (`PLT.NOT.01`), messages sent (`wl-notify`) |
| Hidden fields | `FLD`/`SEN` fields are not rendered; a `SEN` field the user may see shows a "Reveal" button whose click is audited (`PLT.ACL.06`) |
| Side panel | 3–5 figures that matter (`--rx-type-display`), with amounts in AED (`wl-money` is a target, not built) |

### 2.3 Lists

- **Every list is the standard grid**, `renox::grid::Grid` rendered as `<rx-grid>`, with the
  baseline of the project skill [standard-grid](../.claude/skills/standard-grid/SKILL.md): search,
  sort, filters, pagination, dynamic columns, remembered state, export, an empty state and summaries on
  money columns; the status column is an `rx-badge` (tone from DESIGN-SYSTEM §6). `rx-table` is
  only for a few lines inside a detail page, never for a list ([host/AGENTS.md](../host/AGENTS.md)).
- **Pages fill the desktop screen** (`<rx-shell fill>`): the document does not scroll, the grid is a
  direct child of `<rx-page>` and takes the remaining height (`rx-fill`, `rx-panes` for side-by-side
  panels), each panel scrolls on its own; tablets and phones scroll as a document. Check with
  `node .claude/skills/standard-grid/check-fill.js` ([DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) §11).
- **Saved filters**: per user, plus defaults per role ("My open tickets", "Overdue invoices").
- **Scope selector** in the navbar (building, project, branch, outlet…), as in the bike
  shop's store switcher. Lists, inbox and dashboards follow it. "All" appears only for users
  with global roles.
- **Entity selector**: shown only when the deployment has more than one legal entity
  ([decision 0003](decisions/0003-legal-entities.md), `PLT.ENT.01`). It filters financial
  lists and reports.
- **Bulk actions**: only transitions that are allowed on every selected row. Each row runs
  the transition separately, and a failure on one row does not stop the others.
- **Export**: Excel/CSV of the current filtered list (`RPT.LIB.01`), permission
  `reports.export`. `FLD`/`SEN` columns are excluded.

### 2.4 Global search

`PLT.SRC.01`, in the navbar, opened with `/` or the search icon.

- Searches parties (names in English and Arabic, phone, Emirates ID last digits for users
  allowed to see them), records by number (`INV-…`, `L-…`), units, vehicles (plate, VIN) and
  documents by name.
- Results are grouped by type, 5 per group, and filtered by scope and permission. Results
  never reveal records the user cannot open.
- Arabic normalisation and transliteration: R22.

### 2.5 Dashboards

`RPT.DSH.01`: role-based tiles. Each tile is a KPI from the vertical's §10, with a sparkline
when it has a trend. Clicking a tile opens the filtered list or report behind it. Tiles
come after the inbox, never before it.

### 2.6 Forms

- `rx-form` with inline validation; long creations use `rx-wizard` (a lease, a project, an
  enrolment).
- Custom fields (R14) are rendered in a "More details" section, in the order configured by
  the client.
- Arabic fields sit next to their English counterpart (`name_en` | `name_ar`), and the Arabic
  input is `dir="rtl"`.
- Amounts are shown as AED (`wl-money` is a target, not built); dates use `rx-date-picker` in `dd/mm/yyyy` format.

### 2.7 Empty states

Every list and tab has an empty state: an icon circle, one sentence, and one primary action
when the user may create a record ("No cheques yet · Record cheques"). First-run empty
states link to the import (`PLT.IMP.01`): "Import your units from Excel".

### 2.8 RTL

With `ar` active, the layout mirrors: the sidebar is on the right, chevrons flip, tables
read right to left. Digits, amounts, codes, phone numbers, plates and charts do not mirror
([DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) §5). Every screen in this document must be checked in
RTL before release.

### 2.9 Field PWA patterns

```
┌───────────────────────────┐
│ Today · Tue 14 Oct    ⟳ ● │
│ 6 jobs · 2 done           │
├───────────────────────────┤
│ ▸ 09:00  Unit 1204        │
│   AC not cooling · HIGH   │
│   [ IN PROGRESS ]         │
├───────────────────────────┤
│ ▸ 11:30  Unit 803         │
│   Leaking tap             │
│   [ ASSIGNED ]            │
├───────────────────────────┤
│ ▸ 14:00  Villa 12         │
│   Water heater            │
└───────────────────────────┘
│ Today │ Scan │ Me         │
└───────────────────────────┘
```

```
┌───────────────────────────┐
│ ‹ Ticket T-2291           │
│ Unit 1204 · Tower A       │
│ AC not cooling · HIGH     │
│ SLA: 2 h 10 min left      │
│ Tenant: Ahmed · 📞 Call   │
├───────────────────────────┤
│ Before photos  [＋ 📷]    │
│ After photos   [＋ 📷]    │
│ Notes                     │
├───────────────────────────┤
│ [   Put on hold   ]       │
│ [ ███  Resolve  ███ ]     │
└───────────────────────────┘
```

| Pattern | Rule |
|---|---|
| Today list | The home screen of every field role: the user's assigned work for today, in time or route order, with a status badge and the one key fact (address, SLA, item count) |
| Big actions | Up to two actions at the bottom: the primary transition full-width, the secondary one outlined. Other transitions go under ⋯ |
| Capture | `rx-camera-capture` for photos (before/after, POD, damage), `rx-signature-pad` for sign-off, `rx-geolocate` only at check-in/out and at proof of delivery ([wl-attendance](modules/wl-attendance.md)), `rx-barcode-scan` for items, lots, parcels, assets |
| Guards visible | When a guard requires something ("after photo required"), the field is marked, and the action stays disabled until it is filled |
| Contact | Tap-to-call and WhatsApp deep links. The phone number is shown only while the job is open (`WRK.ACL.02`) |
| Connectivity | MVP: online only, with a clear banner when offline. Captured photos wait in the browser and upload when the connection is back. **Later:** an offline queue for attendance, POD outcomes and POS sales (R25, noted in [wl-delivery](modules/wl-delivery.md) and [wl-sales](modules/wl-sales.md)) |
| Install | PWA install prompt after the first login (R6) |

### 2.10 Portal patterns

```
┌──────────────────────────────────────────────────────────────┐
│ [Logo] Acme Properties          Home  Payments  Requests  ⓐ │
├──────────────────────────────────────────────────────────────┤
│  Welcome back, Ahmed                                          │
│  Unit 1204, Tower A · Lease until 31 Dec 2026                 │
│                                                               │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐          │
│  │ Next payment │ │ Open requests│ │ Documents    │          │
│  │ AED 12,500   │ │ 1 in progress│ │ Lease, Ejari │          │
│  │ 01 Feb · Pay │ │ View         │ │ View         │          │
│  └──────────────┘ └──────────────┘ └──────────────┘          │
│                                                               │
│  [  Report a problem  ]                                       │
│                                                               │
│  Recent: Invoice INV-ACM-2026-0113 issued · Ticket resolved  │
└──────────────────────────────────────────────────────────────┘
```

- Home = 3 cards (what I owe, what is in progress, my documents) + one primary action +
  a recent-activity line. The cards depend on the audience ([wl-portal](modules/wl-portal.md)).
- Party switcher when the user acts for several parties (company, dependants): `PTY.REL.01`,
  [wl-portal](modules/wl-portal.md) access grants.
- No staff vocabulary: "Report a problem", not "Create ticket"; state labels use the portal
  label set ([wl-kernel](modules/wl-kernel.md) settings).
- Read-only mode after the relationship ends shows a banner with the closing date (`POR.ACL.01`).
- Branding per client and per brand domain (R9).

### 2.11 Public pages

One task per page, no login before it is needed. Login (magic link / OTP, R15) is asked only
at the step that needs identity (confirm booking, pay, track history). Every public page has
the client's branding, EN/AR toggle, and consent checkboxes where marketing is involved
(`PTY.CNS.01`).

## 3. Navigation conventions

- The sidebar is grouped into sections ordered by frequency: **Home**, front-line work,
  money, operations, **Manage** (reports, team, settings), in that order. The sections
  per vertical are listed in §4.
- Each link is guarded by `can('…')`; a section heading shows only if one of its links does
  (as in the bike shop's staff layout). Modules contribute their links through R10.
- **Settings** (company admin only) is the Renox admin (`renox-admin`): users and roles,
  entities (`PLT.ENT.01`), branding, numbering, custom fields, automation toggles
  (`PLT.ATM.02`), notification templates, import (`PLT.IMP.01`).
- **Reports** (`RPT.LIB.01`) is one link in **Manage**: a library page grouped by area, each
  report with filters and Excel/CSV/PDF export; scheduling (`RPT.SCH.01`, Later).

## 4. Per vertical

Each vertical table lists: role → home (inbox items, then tiles) · sidebar sections · the
three key screens · field PWA screens. Inbox items always come from §2.1 sources; only the
vertical examples are listed.

### 4.1 01 Property

| Role | Home: inbox → tiles |
|---|---|
| Property manager | approvals (deposits `PRP.ACL.05`, waivers, payouts) · leases in renewal window (`PRP.RUL.01`) · leases without Ejari (`PRP.RUL.02`) · SLA breaches → occupancy %, collections this month, arrears 90+, expiring 30/60/90 |
| Accountant | cheques to deposit today (`FIN.RUL.03`) · bounced cheques · overdue invoices · bank lines to match (`FIN.BNK.01`) · draft owner statements (`PRP.RUL.05`) → collected vs due, arrears ageing |
| Leasing agent | drafts waiting for signature · units available → vacant units, days vacant |
| Maintenance coordinator | open tickets to triage · SLA at risk/breached · tickets on hold → open by category, avg resolution time |

**Sidebar:** Home · Front desk (Leases, Units, Tenants, Owners) · Money (Invoices, Cheques,
Payments, Deposits, Owner statements) · Maintenance (Tickets, Inspections, Vendors, Service
contracts `WRK.SVC.01`) · Manage (Reports, Team, Settings).

**Key screens:** (1) Property manager home (wireframe §5.1). (2) Lease record page (§2.2):
tabs Overview · Charges · Cheques · Documents · Tickets · Activity. Actions follow the
*Lease* state machine (`submit`, `activate`, `record_notice`, `renew`, `terminate`,
`cancel`). (3) Cheque deposit list: `rx-grid` of cheques due, with a bulk `deposit`
and then `clear`/`bounce` per row.

**Field PWA:** technician: Today list → ticket (§2.9) · move-in/out inspection
(`WRK.CHK.01` checklist with photos and tenant signature).

**Portal:** tenant home (wireframe §2.10) · owner home: units, occupancy, statements
(`POR.SLF.02`, Later).

### 4.2 02 Construction

| Role | Home: inbox → tiles |
|---|---|
| Project manager | purchase requests to approve (`STK.ACL.03`) · timesheets to approve · IPAs to submit or awaiting certification (`CON.RUL.04`) · variations to submit · budget lines ≥ 90% (`FIN.RUL.05`) → margin forecast, claimed vs certified vs received, retention receivable |
| QS | draft IPAs at cut-off (`CON.RUL.03`) · quotations expiring (`FIN.RUL.07`) · variations to price |
| Procurement | approved requests to order · POs overdue for delivery (`STK.RUL.01`) |
| Accountant | supplier bills failing 3-way match (`FIN.ACL.03`) · expense claims to reimburse (`FIN.EXP.01`) · guarantees expiring (`DOC.RUL.01`) |

**Sidebar:** Home · Projects (Projects, Daily reports, Variations, IPAs) · Site (Attendance,
Timesheets, Material requests, Stores) · Purchasing (Purchase orders, Suppliers, Supplier
bills) · Money (Invoices, Cheques, Retentions, Guarantees, Expenses) · Manage.

**Key screens:** (1) Project record: tabs Overview (contract value, margin) · BOQ & progress
(`rx-grid` editable progress per line) · IPAs · Variations · Costs (`FIN.CST.01` budget vs
committed vs actual) · Team · Documents · Activity. (2) IPA editor: cumulative progress grid
→ calculated summary (work done + variations − retention − advance recovery − previous) →
`submit`. (3) Material request → PO flow.

**Field PWA:** supervisor: Today (crews, attendance) · crew attendance marking (tick list,
`HRM.ATT.01`) · daily site report (`CON.DSR.01`: manpower, work done, photos) · material
request · snag capture. Storekeeper: receive against PO (`STK.GRN.01`, scan) · issue to cost
code. Worker: self check-in with geofence.

### 4.3 03 Logistics

| Role | Home: inbox → tiles |
|---|---|
| Dispatcher | unassigned shipments for today · runs not started · failed attempts to reschedule (`WRK.RUL.08`) → shipments today by status, on-time % |
| Operations manager | exceptions (delays, returns) · cash shortages to approve (`FIN.ACL.06`) → delivered today, success rate, COD outstanding |
| Accountant | cash handovers to count (`FIN.ACL.05`) · COD remittances due · usage invoices to issue (`FIN.USG.01`) |
| Fleet manager | vehicle documents expiring · services due by meter (`WRK.RUL.03`) · fines to allocate |

**Sidebar:** Home · Operations (Shipments, Runs, Hubs, Exceptions) · Freight (Jobs, Customs
documents) · Fleet (Vehicles, Drivers, Fines & tolls) · Money (COD, Cash handovers,
Invoices, Rate cards) · Manage.

**Key screens:** (1) Dispatch board: unassigned shipments on the left, runs per driver on
the right, assignment by selection (`WRK.DSP.01`), map view Later (R20). (2) Shipment record
with tracking timeline. (3) End-of-day cash handover (`FIN.CSH.01`).

**Field PWA:** driver: Today run (stops in order) → stop: deliver (POD photo, signature or
OTP, `WRK.POD.01`) or fail (reason) · COD collected · end-of-day cash handover. Hub staff:
scan in/out (`rx-barcode-scan`).

**Public:** tracking page by signed link (`POR.SLF.02`). **Portal:** client: book shipments,
CSV upload, invoices, COD statements.

### 4.4 04 Accounting

| Role | Home: inbox → tiles |
|---|---|
| Manager | returns to review · tasks overdue (`WRK.RUL.05`) · document requests overdue (`POR.RUL.01`) · engagement approvals (`ACC.ACL.06`) → deadlines this month, WIP, realisation |
| Accountant / associate | my tasks due this week · documents received to review · time to log |
| Compliance officer | CDD files to review · re-screening hits (`PTY.RUL.01`) · AML cases |
| Billing | retainers to invoice · overdue client invoices |

**Sidebar:** Home · Clients (Clients, Engagements, CDD files) · Work (Tasks, Deadlines
calendar, Document requests, Returns) · Time & billing (Time, Invoices, Retainers) ·
Compliance (AML cases, Screening) · Manage.

**Key screens:** (1) Deadline calendar (`rx-month-calendar`, `WRK.TSK.01`). (2) Client record:
engagements, tasks, document requests, returns, KPI pack, invoices. (3) Return workflow page
(*Return* state machine, client approval via `DOC.SIG.01`).

**Portal:** client: document requests checklist with upload (`POR.DOC.01`) · returns to approve
· invoices · KPI pack.

### 4.5 05 Clinics

| Role | Home: inbox → tiles |
|---|---|
| Receptionist | today's bookings to check in · requested bookings to confirm · waiting list offers (`SCH.RUL.03`) · eligibility checks pending → today's appointments, no-shows |
| Practitioner | my schedule today · consent forms missing → sessions today |
| Insurance coordinator | prior authorisations pending · claims to submit · denials to resubmit · remittances to reconcile → claims ageing, denial rate |
| Clinic manager | low ratings (`WRK.FBK.01`) · complaints · licences expiring (`HRM.QUA.01`) → utilisation, revenue by service |

**Sidebar:** Home · Front desk (Calendar, Bookings, Patients, Waiting list) · Billing
(Invoices, Packages, Payments) · Insurance (Pre-authorisations, Claims, Remittances) ·
Clinic (Practitioners, Rosters, Rooms, Stock) · Manage.

**Key screens:** (1) Resource calendar (`rx-week-calendar` by practitioner and room, drag to
reschedule, *Booking* state machine). (2) Patient record: bookings, packages, invoices,
claims, consents, documents (health class, `SEN`). (3) Check-in → billing at the counter.

**Public / portal:** self-booking (`SCH.SLF.01`) · patient portal: bookings, packages,
invoices, guardian switcher for dependants (`PTY.REL.01`).

### 4.6 06 F&B

| Role | Home: inbox → tiles |
|---|---|
| Outlet manager | orders pending acceptance · day closing not done (`SAL.POS.01` till session) · stock count due · wastage to approve (`STK.ACL.02`) → sales today by channel, food cost % |
| Head chef | production batches today · items below par → theoretical vs actual |
| Accountant | payout statements to reconcile (`FNB.REC.01`) · petty cash top-ups (`FIN.EXP.01`) |

**Sidebar:** Home · Orders (Live orders, Aggregator entry, Direct orders) · Kitchen (KDS,
Production, Recipes) · Stock (Items, Counts, Purchase orders) · Menu (Brands, Menus, Promo
codes) · Money (Closing, Payouts, Expenses) · Manage.

**Key screens:** (1) Live orders board (`rx-kanban` by kitchen fulfilment state). (2) KDS
station screen (full screen, large tickets, bump). (3) Aggregator quick entry (one form,
item search, under 20 seconds per order).

**Field:** line cook: KDS station · rider: deliveries (wl-delivery PWA, Later).
**Public:** direct ordering site per brand (`FNB.DTC.01`), table reservation (see the
vertical document).

### 4.7 07 Manufacturing

| Role | Home: inbox → tiles |
|---|---|
| Production planner | confirmed orders without work orders · material shortages · work orders late → output vs plan, OTIF |
| QC inspector | lots awaiting inspection · open NCRs |
| Sales / customer service | orders on credit hold (`SAL.RUL.01`) · dispatches due → orders by status |

**Sidebar:** Home · Sales (Customers, Quotations, Sales orders, Returns) · Production (Work
orders, BOMs, Lines) · Quality (Inspections, NCRs, Lots, Certificates) · Stock & purchasing
· Dispatch · Manage.

**Key screens:** (1) Work order record (BOM version, materials issued, output, QC). (2) Lot
traceability (lot → inputs → orders). (3) Sales order with fulfilment progress.

**Field PWA (line tablet):** work order steps · record output and scrap · QC checklist
(`WRK.CHK.01`) · scan lots.

**Portal:** customer buyer: place and track orders, certificates of analysis.

### 4.8 08 Auto

| Role | Home: inbox → tiles |
|---|---|
| Service advisor | today's bookings · job cards awaiting customer approval · insurance approvals pending (see vertical) · vehicles ready for pickup |
| Workshop manager | job cards by status · technician load → bay utilisation |
| Counter sales | orders to pick · returns to inspect (`SAL.RET.01`) |
| Accountant | credit holds · fleet usage invoices (`FIN.USG.01`) · insurer receivables |

**Sidebar:** Home · Parts (Catalogue, Sales orders, Counter, Returns) · Workshop (Bookings,
Job cards, Warranty, Insurance claims) · Fleet (Contracts, Vehicles) · Stock & purchasing ·
Deliveries · Money · Manage.

**Key screens:** (1) Parts search by fitment, part number or cross-reference. (2) Job card
`rx-kanban` (check-in → diagnosis → approval → in progress → QC → ready). (3) Counter sale
(`SAL.POS.01`).

**Field PWA:** technician: my job cards · clock on/off · photos · parts request. Driver:
deliveries with POD.

**Portal:** B2B buyer ordering · fleet contact (vehicles, services, invoices) · car owner
booking (`SCH.SLF.01`, Later).

### 4.9 09 Manpower

| Role | Home: inbox → tiles |
|---|---|
| Account manager | manpower requests to fill · timesheets awaiting client approval (`HRM.RUL.08`) · contract renewals → deployed headcount, bench, margin per client |
| PRO / visa officer | visa cases by step · visas expiring (`HRM.RUL.04`) · documents expiring |
| HR / payroll | payroll run to prepare or approve (`HRM.ACL.04`) · final settlements · leave requests |
| Accountant | usage invoices to issue · bank lines to match (`FIN.BNK.01`) |

**Sidebar:** Home · Clients (Clients, Contracts & rate cards, Requests) · Workers
(Employees, Deployments, Visa cases, Accommodation) · Time (Attendance, Timesheets, Leave)
· Payroll (Runs, WPS files, Settlements) · Money · Manage.

**Key screens:** (1) Employee record: deployments, visa case, documents with expiry,
timesheets, payslips (`SEN` salary). (2) Visa case step tracker (`rx-status-stepper`).
(3) Payroll run review grid.

**Field PWA:** site supervisor: attendance for deployed workers (`HRM.ACL.01`), no-shows.
**Portal:** client approver: approve timesheets (signed approval), invoices; worker: payslips,
leave requests.

### 4.10 10 EdTech

| Role | Home: inbox → tiles |
|---|---|
| Admissions counsellor | new leads · trials booked today · enrolments to complete → conversion rate |
| Academic coordinator | sessions without a tutor · make-ups to schedule · submissions to grade → attendance % |
| Tutor | my sessions today · attendance to mark · submissions to grade |
| Centre manager | subscriptions past due (`FIN.RUL.06`) · refunds to approve (`FIN.ACL.04`) · low ratings |

**Sidebar:** Home · Admissions (Leads, Trials, Enrolments) · Academics (Courses, Batches,
Timetable, Sessions, Assessments, Certificates) · Students (Students, Guardians, Packages) ·
Money (Invoices, Subscriptions) · Manage.

**Key screens:** (1) Timetable (`rx-week-calendar` by room and tutor). (2) Session attendance
(tick list, consumes package credits `SAL.PKG.01`). (3) Student record with progress report.

**Portal:** student and guardian: timetable, attendance, materials, payments, certificates
(guardian switcher).

## 5. Key wireframes

### 5.1 Property manager home (back office)

```
┌────────────┬──────────────────────────────────────────────────────────┐
│ ACME Props │  Building: Tower A ▾   Entity: Acme LLC ▾  🔍  🔔 3  Sara ▾│
├────────────┼──────────────────────────────────────────────────────────┤
│ Home       │  Needs your action                                        │
│ FRONT DESK │  ● 2 deposit refunds waiting for your approval      ›     │
│  Leases    │  ● 4 leases in renewal window (90 days)             ›     │
│  Units     │  ● 1 lease active 14 days without Ejari             ›     │
│  Tenants   │  ● 1 ticket breached SLA · AC Unit 1204             ›     │
│  Owners    │                                                           │
│ MONEY      │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐     │
│  Invoices  │  │Occupancy │ │Collected │ │Arrears   │ │Expiring  │     │
│  Cheques   │  │  94%  ▁▃▅│ │  87%     │ │AED 142k  │ │ 30d: 6   │     │
│ MAINTENANCE│  └──────────┘ └──────────┘ └──────────┘ └──────────┘     │
│  Tickets   │                                                           │
│ MANAGE     │  Collections, last 6 months (chart)                       │
│  Reports   │                                                           │
└────────────┴──────────────────────────────────────────────────────────┘
```

The entity selector appears only because this client has two legal entities. The other
key wireframes are in §2.2 (record page), §2.9 (technician PWA) and §2.10 (tenant portal).

## 6. Acceptance checklist per screen

- [ ] Actions come from the state machine's available actions; no hard-coded buttons
- [ ] The inbox and lists respect scope, entity and permissions; no count leaks hidden rows
- [ ] `FLD`/`SEN` fields hidden or revealed with audit
- [ ] Empty state with one action
- [ ] Works at 360 px (field roles) and in RTL
- [ ] Keyboard path for every action; visible focus
- [ ] Every list exports (`RPT.LIB.01`) without hidden columns
- [ ] A list is the standard grid and fills the screen (`check-fill.js` passes)

## 7. Screens still to specify

Found while classifying the backlog by surface (2026-10-16). The behaviour is specified; the
screen is not yet described in §2 or §4. Each is designed before its backlog issue is built.

| Surface | Screen | Needed by |
|---|---|---|
| Portal | Raise an invoice dispute (party side); needs a `wl-portal` action and permission | BL-INV-04 |
| Portal | Cancel a subscription or membership, where the client allows it | BL-INV-21 |
| Portal | Employee: own onboarding steps and SOP acknowledgements | BL-HRM-27 |
| Portal | 09 client: sign a contract renewal or a new rate card version | 09 §4.9 (BL-MPW-05, BL-MPW-07) |
| Portal | 10 guardian or student: enrol (full batch, alternatives, waiting list) | BL-EDU-02 |
| Portal | 10 membership pause and resume | BL-EDU-05 |
| Portal | 10 notice of an absence | BL-EDU-15 |
| Portal, public | Loyalty: join, see balance, redeem points with a one-time code | BL-SAL-22, BL-SAL-23 |
| Public | Lead form opened from a campaign link | BL-SAL-20 |
| Public | A payment link that was cancelled after an enrolment lapsed | BL-EDU-01 |
| Public | Client confirms a change of its remittance bank account (signed link) | BL-LOG-17 |
| Field PWA | 08 technician: record a test drive and raise a vehicle incident with photos | BL-AUT-06 |
| Field PWA or back office | Cash holder acknowledges a salary deduction for a shortage | BL-PAY-17 |
| Back office | 06 turn cash on delivery back on for a customer; resume an outlet's channels after an outage | BL-FNB-03, BL-FNB-14 |
| Back office | Log of refused portal actions for blocked parties, visible to compliance only | BL-POR-01 |
