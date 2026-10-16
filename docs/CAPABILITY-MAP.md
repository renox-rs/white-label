# Capability Map

> Status: draft · Last updated: 2026-10-15 · Built from the capability tables (§5) of the
> [vertical documents](verticals/). Change a vertical first, then update this map to match.

The helicopter view: every capability the ten verticals need, which module provides it, and
who uses it. ID conventions are in [README.md](README.md).

## 1. Summary

| # | Vertical | Capabilities | Shared | Specific | Reuse |
|---|---|---|---|---|---|
| 01 | [Property](verticals/01-property.md) | 55 | 43 | 12 | 78% |
| 02 | [Construction](verticals/02-construction.md) | 69 | 53 | 16 | 77% |
| 03 | [Logistics](verticals/03-logistics.md) | 72 | 57 | 15 | 79% |
| 04 | [Accounting](verticals/04-accounting.md) | 63 | 48 | 15 | 76% |
| 05 | [Clinics](verticals/05-clinics.md) | 73 | 61 | 12 | 84% |
| 06 | [F&B](verticals/06-fnb.md) | 92 | 67 | 25 | 73% |
| 07 | [Manufacturing](verticals/07-manufacturing.md) | 74 | 62 | 12 | 84% |
| 08 | [Auto](verticals/08-auto.md) | 88 | 70 | 18 | 80% |
| 09 | [Manpower](verticals/09-manpower.md) | 61 | 52 | 9 | 85% |
| 10 | [EdTech](verticals/10-edtech.md) | 71 | 56 | 15 | 79% |
| | **Distinct** | **249** | **100** | **149** | |

Every vertical gets 73–85% of its capabilities from shared modules. Building the shared
modules once covers 100 capabilities; each vertical then adds 9–25 of its own.

## 2. Modules

| Module | Shared capabilities | Used by | Notes |
|---|---|---|---|
| `wl-kernel + Renox` | 18 | 10 verticals | Branding, legal entities, approvals, automations, import, search, notes, reports, API. With Renox: users, SSO, audit, state machines, webhooks, dashboards, scanning |
| `wl-parties` | 6 | 10 verticals | Companies and people in any role, identity, relationships, consents, UBO, screening |
| `wl-portal` | 3 | 10 verticals | External users' self-service and document exchange |
| `wl-documents` | 4 | 10 verticals | Files, expiry reminders, generated PDFs, e-signature |
| `wl-notify` | 3 | 10 verticals | Email, WhatsApp, SMS |
| `wl-invoicing` | 14 | 10 verticals | Quotations, invoices, subscriptions, rate cards, usage billing, costing, receivables, supplier bills, VAT summary |
| `wl-payments` | 7 | 10 verticals | Payments, online payment, cheques, deposits, cash custody, expenses, bank statement matching |
| `wl-sales` | 8 | 8 verticals | **New.** Leads, catalogue, orders, POS, packages, discounts, returns, credit limits |
| `wl-ticketing` | 8 | 10 verticals | Tickets, checklists, planned maintenance, recurring tasks, feedback, service contracts |
| `wl-delivery` | 2 | 4 verticals | **New** ([§5](#5-resolved-overlaps)). Dispatch and proof of delivery |
| `wl-scheduling` | 4 | 4 verticals | Resources, bookings, self-booking, reminders |
| `wl-attendance` | 3 | 9 verticals | Attendance, timesheets, rosters |
| `wl-hr` | 7 | 9 verticals | **New.** Employees, visas, leave, payroll, WPS, final settlement |
| `wl-inventory` | 13 | 6 verticals | Items, stock, purchasing, lots, BOMs, dispatch notes, assets |

Not in this table: [`wl-operator`](modules/wl-operator.md), our own control plane (domain `OPS`). Its capabilities serve us, not the clients.

## 3. Shared capabilities

● MVP · ○ Later · blank = not listed. *Expected also* = verticals that will probably need it
although their document does not list it yet; confirm when that vertical is reviewed.

### wl-kernel + Renox

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `PLT.API.01` | API for external parties to push records with their own tokens | wl-kernel + Renox tokens |  |  | ○ |  |  |  |  |  |  |  |  |
| `PLT.APR.01` | Approval requests for limit and segregation-of-duties rules | wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.ATM.01` | Default automation rules per vertical | wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.ATM.02` | Automation rules the client can toggle and parameterise | wl-kernel |  |  |  |  |  |  |  |  |  |  | all (Later, see WORKFLOW §6) |
| `PLT.AUD.01` | Audit trail of changes and of sensitive-data views | Renox Audit | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.ENT.01` | Legal entities: several trade licences and TRNs in one deployment; issuer on every document | wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.HOK.01` | Outbound webhooks for the client's own integrations | Renox (R11) | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ |  |
| `PLT.IMP.01` | Data import and migration: Excel templates per entity, validation, dry run, rollback | wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.NOT.01` | Notes, comments and @mentions on any record, with an activity timeline | wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.ORG.01` | Branded deployment: logo, colours, domain, languages (EN/AR) | wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.SCN.01` | Scan barcodes and QR codes with the device camera | Renox (R5) |  |  | ● |  |  |  |  |  |  |  |  |
| `PLT.SRC.01` | Global search across records, including Arabic names | wl-kernel + Renox search | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.SSO.01` | Staff single sign-on with Microsoft 365 or Google Workspace | Renox OAuth (renox-oauth) | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ |  |
| `PLT.USR.01` | Staff users and roles, scoped and time-bound | Renox Permissions | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PLT.WFL.01` | State machines with transition history and available actions | Renox (R12) + wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `RPT.DSH.01` | Role-based dashboards with KPI tiles and charts | Renox charts + wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `RPT.LIB.01` | Report library: filtered reports with export to Excel, CSV and PDF | wl-kernel | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `RPT.SCH.01` | Scheduled reports sent by email | wl-kernel | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ |  |

### wl-parties

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `PTY.CNS.01` | Recorded consents per party and purpose, with withdrawal | wl-parties |  |  |  |  | ● | ● |  |  |  | ● |  |
| `PTY.KYC.01` | Identity fields: Emirates ID, passport, visa, trade licence, TRN | wl-parties | ● | ● | ● | ● | ● |  | ● | ● | ● | ● |  |
| `PTY.PTY.01` | Register of parties: companies and individuals in any role | wl-parties | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `PTY.REL.01` | Links between parties: guardian and dependant, family, group | wl-parties |  |  |  | ● | ● |  |  | ● |  | ● |  |
| `PTY.SCR.01` | Sanctions and watch-list screening with re-screening log | wl-parties |  |  |  | ● |  |  |  |  |  |  |  |
| `PTY.UBO.01` | Ownership structure and ultimate beneficial owners | wl-parties |  |  |  | ● |  |  |  |  |  |  |  |

### wl-portal

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `POR.DOC.01` | Secure document exchange through the portal: request lists, uploads | wl-portal |  |  |  | ● | ○ |  |  |  | ○ |  |  |
| `POR.SLF.01` | Self-service portal for the main external party | wl-portal | ● | ○ | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `POR.SLF.02` | Portal for a second external audience | wl-portal | ○ |  | ● |  |  |  |  | ○ | ● | ○ |  |

### wl-documents

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `DOC.EXP.01` | Expiry tracking with tiered reminders | wl-documents | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `DOC.FIL.01` | Attach documents to any record | wl-documents | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `DOC.GEN.01` | Generate documents from templates in EN/AR (PDF) | wl-documents | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `DOC.SIG.01` | E-signature and acceptance capture | wl-documents |  |  |  | ● | ● |  |  | ● | ● | ● |  |

### wl-notify

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `NTF.MSG.01` | Email notifications from templates | wl-notify | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `NTF.MSG.02` | WhatsApp notifications | wl-notify | ○ | ○ | ○ | ○ | ● | ○ | ○ | ○ | ○ | ● |  |
| `NTF.MSG.03` | SMS notifications | wl-notify |  |  | ● |  | ● | ● |  | ● | ● | ○ |  |

### wl-invoicing

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `FIN.BIL.01` | Charge schedule from an agreement: instalments, recurring fees | wl-invoicing | ● |  |  | ● |  |  |  | ● |  | ● |  |
| `FIN.CST.01` | Job costing: budget vs committed vs actual per cost object | wl-invoicing |  | ● | ● |  |  |  | ● |  | ● |  |  |
| `FIN.CUR.01` | Multi-currency amounts with exchange rates | wl-invoicing |  | ● | ● |  |  |  | ● | ● |  |  |  |
| `FIN.DSB.01` | Disbursements: costs paid on a client's behalf, recharged at cost | wl-invoicing |  |  | ● |  |  |  |  |  |  |  |  |
| `FIN.EXT.01` | Export to accounting software (Zoho Books, Xero, QuickBooks) | wl-invoicing | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ | ○ |  |
| `FIN.INV.01` | Tax invoices and credit notes with per-line VAT category | wl-invoicing | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `FIN.PAB.01` | Supplier bills and expenses with 3-way match, allocated to cost objects | wl-invoicing | ● | ● | ● |  | ○ | ● | ● | ● |  | ● |  |
| `FIN.QUO.01` | Quotations with validity and revisions; accepted → order or project | wl-invoicing |  | ● | ● | ● |  |  | ● | ● | ● | ○ |  |
| `FIN.RAT.01` | Rate cards: prices per service, role, zone or vehicle with multipliers and minimums (formerly `LOG.RAT.01`) | wl-invoicing |  |  | ● |  |  |  |  | ● | ● |  |  |
| `FIN.REC.01` | Receivables ageing and payment reminders | wl-invoicing | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `FIN.SUB.01` | Recurring subscriptions, memberships and retainers | wl-invoicing (+ renox-billing via wl-payments) |  |  |  | ● | ○ | ○ |  |  |  | ● |  |
| `FIN.TPP.01` | Third-party payer billing: split a bill between the customer and a payer (insurer), invoice the payer, reconcile remittances per line (paid, denied, partial) and follow up payer receivables (formerly `AUT.INS.05`) | wl-invoicing |  |  |  |  | ● |  |  | ● |  |  |  |
| `FIN.USG.01` | Usage billing: periodic invoice from priced transactions (rate card × usage) (formerly `AUT.FLT.02`) | wl-invoicing |  |  | ● |  |  |  |  | ● | ● |  |  |
| `FIN.VAT.01` | VAT summary per period and entity: output and input VAT, for the accountant | wl-invoicing | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |

### wl-payments

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `FIN.BNK.01` | Bank statement import and matching of receipts to open invoices | wl-payments | ● | ○ | ○ | ● | ○ | ○ | ○ | ○ | ● | ○ |  |
| `FIN.CHQ.01` | Post-dated cheque register, received and issued | wl-payments | ● | ● | ● | ● |  |  | ● | ● | ● |  |  |
| `FIN.CSH.01` | Cash custody: cash held by staff, end-of-day handover, shortages | wl-payments |  |  | ● |  | ● | ● |  | ● |  | ● |  |
| `FIN.DEP.01` | Deposits held, deductions and refunds | wl-payments | ● |  |  |  |  |  |  |  |  |  |  |
| `FIN.EXP.01` | Expense claims and petty cash: submit with receipt, approve, reimburse, top up | wl-payments | ○ | ● | ● | ○ | ○ | ● | ○ | ○ | ○ | ○ |  |
| `FIN.PAY.01` | Record payments by any method and allocate them to invoices | wl-payments | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● |  |
| `FIN.PAY.02` | Online payment by link or card | wl-payments | ○ |  | ○ | ○ | ● | ● |  | ○ |  | ● |  |

### wl-sales

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `SAL.CAT.01` | Sellable catalogue and price lists | wl-sales |  |  |  |  | ● | ● | ● | ● |  | ● |  |
| `SAL.CRD.01` | Customer credit limits, credit check and credit hold | wl-sales |  |  |  |  |  |  | ● | ● |  |  |  |
| `SAL.CRM.01` | Leads and enquiries pipeline | wl-sales |  |  | ○ | ○ | ○ |  | ○ | ○ | ○ | ● | 01 (viewings) |
| `SAL.DSC.01` | Discounts and promo codes (formerly `FNB.PRM.01`) | wl-sales |  |  |  |  |  | ● |  |  |  | ● |  |
| `SAL.ORD.01` | Sales orders from every channel | wl-sales |  |  |  |  |  | ● | ● | ● |  |  |  |
| `SAL.PKG.01` | Prepaid packages of sessions or units: sell, consume, expire, refund (formerly `EDU.PKG.01`) | wl-sales |  |  |  |  | ● |  |  |  |  | ● |  |
| `SAL.POS.01` | Counter sale / point of sale with receipt and day closing | wl-sales |  |  |  |  |  | ● |  | ● |  |  |  |
| `SAL.RET.01` | Sales returns with inspection, credit note or refund, restocking (formerly `MFG.RTN.01`) | wl-sales |  |  |  |  |  |  | ○ | ● |  |  |  |

### wl-ticketing

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `WRK.CHK.01` | Checklists with photos and signature | wl-ticketing | ● | ● | ● | ● |  | ● | ● | ● |  |  |  |
| `WRK.FBK.01` | Customer feedback and ratings after a completed service; low ratings open a ticket | wl-ticketing | ○ |  | ○ |  | ● | ○ |  | ● |  | ○ |  |
| `WRK.PPM.01` | Planned preventive maintenance by date or meter | wl-ticketing | ● |  | ● |  |  | ○ | ○ | ● |  |  |  |
| `WRK.SVC.01` | Vendor service contracts (AMC): scope, assets, planned visits, SLA, cost | wl-ticketing | ● |  |  |  | ○ | ○ | ○ |  |  |  |  |
| `WRK.TKT.01` | Raise a ticket from the portal or by staff, with category and photos (formerly `AUT.WAR.01`) | wl-ticketing | ● | ● | ● | ● | ● | ● | ● | ● | ● | ○ |  |
| `WRK.TKT.02` | Triage: priority, SLA, assignment to staff or vendor | wl-ticketing | ● | ● | ● | ● | ● | ● | ● |  | ● |  |  |
| `WRK.TKT.03` | Field updates, before/after photos, requester sign-off | wl-ticketing | ● | ● | ● |  |  | ● | ● |  |  |  |  |
| `WRK.TSK.01` | Recurring tasks and deadline calendar | wl-ticketing |  |  |  | ● |  |  |  |  |  |  |  |

### wl-delivery

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `WRK.DSP.01` | Dispatch: assign deliveries to drivers and runs; driver's day list (formerly `LOG.DSP.01`) | wl-delivery |  |  | ● |  |  | ● | ● | ● |  |  |  |
| `WRK.POD.01` | Proof of delivery or completion: photo, signature or OTP, location | wl-delivery |  |  | ● |  |  | ● | ● | ● |  |  |  |

### wl-scheduling

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `SCH.BKG.01` | Bookings by staff, with waiting list | wl-scheduling |  |  |  |  | ● | ○ |  | ● |  | ● |  |
| `SCH.RES.01` | Resources and availability: people, rooms, bays, equipment | wl-scheduling |  |  |  |  | ● | ○ |  | ● |  | ● |  |
| `SCH.RMD.01` | Booking reminders, confirmations and no-show handling | wl-scheduling |  |  |  |  | ● | ○ |  | ● |  | ● |  |
| `SCH.SLF.01` | Online self-booking from the website and portal | wl-scheduling |  |  |  |  | ● | ○ |  | ○ |  | ○ |  |

### wl-attendance

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `HRM.ATT.01` | Attendance: supervisor marking or self check-in within a geofence | wl-attendance |  | ● | ● |  |  | ○ | ○ |  | ● |  |  |
| `HRM.ROS.01` | Shift rosters: patterns, assignment, gaps; feed bookable availability (formerly `CLN.ROS.01`) | wl-attendance |  |  |  |  | ● |  |  |  | ○ |  |  |
| `HRM.TMS.01` | Timesheets allocated to cost objects, overtime, approval, export | wl-attendance |  | ● |  | ● |  |  | ○ | ○ | ● | ● |  |

### wl-hr

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `HRM.EMP.01` | Employee records with documents and professional licences | wl-hr |  | ● | ● | ● | ● | ● | ○ | ○ | ● | ● |  |
| `HRM.EOS.01` | Final settlement: gratuity, leave encashment, recoveries | wl-hr |  |  |  |  |  |  |  |  | ● |  |  |
| `HRM.LEV.01` | Leave and absence, blocking calendars | wl-hr |  |  |  |  | ○ |  |  |  | ● | ○ |  |
| `HRM.PAY.01` | Payroll calculation: salary structure, overtime, allowances, deductions | wl-hr |  | ○ |  |  |  | ○ |  |  | ● | ○ |  |
| `HRM.QUA.01` | Qualifications and licences per role and scope, with a validity guard (formerly `CLN.LIC.01`) | wl-hr |  |  | ● | ● | ● |  |  |  | ● | ● |  |
| `HRM.VIS.01` | Visa cases (new, renewal, cancellation) as tracked steps | wl-hr |  |  |  |  |  |  |  |  | ● |  |  |
| `HRM.WPS.01` | WPS salary file (SIF) export | wl-hr |  | ○ |  |  |  | ○ |  |  | ● |  |  |

### wl-inventory

| ID | Capability | Module | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | Expected also |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `STK.AST.01` | Asset register: equipment, vehicles, tools; location, assignment, service due | wl-inventory |  | ○ | ● |  |  | ○ | ○ |  |  |  |  |
| `STK.BOM.01` | Bills of materials and recipes, versioned, with yields | wl-inventory |  |  |  |  |  | ● | ● |  |  |  |  |
| `STK.CNT.01` | Stock counts with variance and approved adjustment | wl-inventory |  | ● |  |  |  | ● |  | ● |  |  |  |
| `STK.DSP.01` | Pick, pack and dispatch with delivery notes | wl-inventory |  |  |  |  |  |  | ● |  |  |  |  |
| `STK.GRN.01` | Goods received against a purchase order | wl-inventory |  | ● |  |  | ○ | ● | ● | ● |  |  |  |
| `STK.ITM.01` | Item catalogue with units of measure | wl-inventory |  | ● |  |  | ● | ● | ● | ● |  |  |  |
| `STK.LND.01` | Landed cost spread over received items | wl-inventory |  | ● |  |  |  |  | ● | ● |  |  |  |
| `STK.LOC.01` | Stock per location | wl-inventory |  | ● |  |  | ● | ● | ● | ● |  |  |  |
| `STK.LOT.01` | Batch/lot and expiry tracking | wl-inventory |  |  |  |  | ● | ● | ● |  |  |  |  |
| `STK.MOV.01` | Stock movements: receive, issue, transfer, adjust | wl-inventory |  | ● |  |  | ● | ● | ● | ● |  |  |  |
| `STK.PRQ.01` | Purchase requests with approval | wl-inventory |  | ● |  |  |  | ● | ● |  |  |  |  |
| `STK.PUR.01` | Purchase orders to suppliers | wl-inventory |  | ● |  |  | ○ | ● | ● | ● |  |  |  |
| `STK.ROP.01` | Reorder points and suggested purchase lists | wl-inventory |  |  |  |  |  | ● |  | ● |  |  |  |

## 4. Vertical-specific capabilities

The full rows are in each vertical's §5.

<details><summary><b>01 Property</b>: 12 specific capabilities</summary>

| ID | Capability |
|---|---|
| `PRP.PRT.01` | Portfolio: buildings, floors, units and their attributes (type, size, use) |
| `PRP.PRT.02` | Unit availability and status |
| `PRP.PRT.03` | Ownership and management agreements (owner, fee %) |
| `PRP.LEA.01` | Enquiries and viewings |
| `PRP.LEA.02` | Reservation with booking deposit |
| `PRP.LSE.01` | Lease: parties, unit, term, rent, payment plan, deposit, clauses, versions |
| `PRP.LSE.02` | Ejari/Tawtheeq registration tracking (number, dates, certificate) |
| `PRP.LSE.03` | Rent increase check against the rental index |
| `PRP.LSE.04` | Renewal workflow with notice deadlines |
| `PRP.LSE.05` | Move-out: notice, inspection, deposit settlement |
| `PRP.OWN.01` | Owner statements (collected − expenses − fee) and payouts |
| `PRP.RPT.01` | Rent roll, occupancy and vacancy reports |

</details>

<details><summary><b>02 Construction</b>: 16 specific capabilities</summary>

| ID | Capability |
|---|---|
| `CON.TND.01` | BOQ import from Excel and pricing per line |
| `CON.PRJ.01` | Project: contract value, terms, retention %, advance %, sites, geofence |
| `CON.PRJ.02` | Cost codes from BOQ lines |
| `CON.PRG.01` | Progress measurement per BOQ line |
| `CON.IPA.01` | Interim payment applications: cumulative claim, variations, retention, advance recovery (pro rata by default, or a fixed % per IPA from IPA N; manual override with a reason) |
| `CON.IPA.02` | Certification: certified vs claimed |
| `CON.VAR.01` | Variations: record, price, submit, approve |
| `CON.RET.01` | Retention tracking and release |
| `CON.GUA.01` | Bank guarantees and bonds register |
| `CON.PMT.01` | Authority and landlord permits / NOCs |
| `CON.DSR.01` | Daily site report |
| `CON.HND.01` | Practical completion, handover certificate, defects liability period |
| `CON.SUB.01` | Submittals and RFIs with the consultant (transmittals) |
| `CON.SBC.01` | Payment applications from own subcontractors (payable side) |
| `CON.HSE.01` | HSE: toolbox talks, incidents, permits to work |
| `CON.RPT.01` | Project cost report and margin forecast; cash: claimed vs certified vs received |

</details>

<details><summary><b>03 Logistics</b>: 15 specific capabilities</summary>

| ID | Capability |
|---|---|
| `LOG.SHP.01` | Shipment booking: recipient, address, parcels, weight, service, COD amount |
| `LOG.SHP.02` | Labels and waybills with barcode |
| `LOG.HUB.01` | Hub scanning: in, sort, out, exceptions |
| `LOG.RTE.01` | Runs: group shipments by area and sequence stops |
| `LOG.RTE.02` | Route optimisation |
| `LOG.TRK.01` | Tracking timeline from events, with location at each event; public tracking link |
| `LOG.TRK.02` | Live vehicle location from telematics devices |
| `LOG.COD.01` | COD collection and weekly remittance to clients |
| `LOG.JOB.01` | Freight jobs: mode, route, BL/AWB, containers, milestones |
| `LOG.CUS.01` | Customs and free-zone documentation: declaration numbers, duties, delivery orders |
| `LOG.FLT.01` | Fleet details: odometer, fuel logs, vehicle documents |
| `LOG.FIN.01` | Fines and tolls (Salik, Darb) imported and allocated to driver, vehicle or job |
| `LOG.CLM.01` | Claims for lost or damaged shipments, valued against liability limits |
| `LOG.SUB.01` | Subcontracted carriers and owner-drivers: assign runs, settle payments |
| `LOG.RPT.01` | Operations reports: first-attempt success, on-time, returns, cost per drop |

</details>

<details><summary><b>04 Accounting</b>: 15 specific capabilities</summary>

| ID | Capability |
|---|---|
| `ACC.KYC.01` | CDD risk assessment and rating, enhanced due diligence, approval |
| `ACC.KYC.02` | Periodic CDD reviews scheduled by risk rating |
| `ACC.AML.01` | AML case log for the MLRO, with goAML report references |
| `ACC.ENG.01` | Engagements: services, team, fees, terms, period templates |
| `ACC.TAX.01` | Client tax profile: TRN, VAT period, financial year, corporate tax registration and status |
| `ACC.CMP.01` | Compliance calendar generated from the tax profile |
| `ACC.REQ.01` | Period document request lists (PBC) from templates |
| `ACC.VAT.01` | VAT return workflow: prepare, review, client approval, filing evidence |
| `ACC.CTX.01` | Corporate tax return workflow |
| `ACC.MCL.01` | Month-end close checklist templates per client |
| `ACC.KPI.01` | Client KPI packs from imported trial balance or P&L (CSV/Excel; presets for Zoho Books, Xero, QuickBooks and Tally; saved account mapping per client) |
| `ACC.KPI.02` | Live connectors to Zoho Books, Xero, QuickBooks |
| `ACC.TIM.01` | Billing of work in progress from time, with write-offs |
| `ACC.CAP.01` | Staff capacity and workload planning |
| `ACC.RPT.01` | Firm reports: deadline status, WIP, realisation, profitability per client |

</details>

<details><summary><b>05 Clinics</b>: 12 specific capabilities</summary>

| ID | Capability |
|---|---|
| `CLN.INS.01` | Payer contracts: plans, networks, tariffs, copay and deductible rules, coverage exclusions |
| `CLN.INS.02` | Patient insurance policies and eligibility check results |
| `CLN.INS.03` | Prior authorisation requests and their validity |
| `CLN.ENC.01` | Visit check-in/out and charge capture (ICD-10, CPT, consumables) |
| `CLN.ENC.02` | Patient-share calculation (copay, deductible, non-covered) at the front desk |
| `CLN.CLM.01` | Claim preparation and validation from charge capture |
| `CLN.CLM.02` | Integrated claim submission and remittance download (eClaimLink/DHPO, Shafafiya, MOHAP) |
| `CLN.CLM.03` | Denial management with denial codes, resubmission, write-off |
| `CLN.RCL.01` | Recalls and follow-up schedules (e.g. 6-month dental check-up) |
| `CLN.TEL.01` | Telehealth appointments via a licensed provider |
| `CLN.HIE.01` | EMR integration: import charge capture, export bookings and demographics (HL7/FHIR) |
| `CLN.RPT.01` | Clinic KPIs: utilisation, no-shows, revenue per practitioner, denial rate, package liability |

</details>

<details><summary><b>06 F&B</b>: 25 specific capabilities</summary>

| ID | Capability |
|---|---|
| `FNB.BRD.01` | Virtual brands: several brands per kitchen, each with menu, branding and ordering site |
| `FNB.CHN.01` | Sales channels (each aggregator, D2C, phone, walk-in) with commission % and price list |
| `FNB.AGG.01` | Quick entry of aggregator orders with the aggregator order number |
| `FNB.AGG.02` | Automatic aggregator order import through middleware (Deliverect in the MVP; Grubtech Later) |
| `FNB.AGG.03` | Menu, price and availability sync to aggregators through middleware (Deliverect in the MVP) |
| `FNB.REC.01` | Aggregator payout statement import and reconciliation |
| `FNB.KDS.01` | Kitchen display: tickets by station, bump, recall, live updates |
| `FNB.KDS.02` | Prep time targets and late-order alerts per brand and channel |
| `FNB.DTC.01` | Branded direct ordering site (PWA) per brand |
| `FNB.DTC.02` | Delivery areas, fees, minimum order, opening hours per outlet |
| `FNB.DTC.03` | Delivery for direct orders: own riders (MVP) through `wl-delivery`; third-party couriers such as Quiqup or Jeebly Later |
| `FNB.DIN.01` | Floor plan per outlet: areas, tables, seats, table status at a glance |
| `FNB.DIN.02` | Table sessions: seat, covers, move, merge and transfer tables |
| `FNB.DIN.03` | Orders by seat and course, fired to the kitchen display by course |
| `FNB.DIN.04` | Bill presentation and splitting (by seat, by item, equal shares), several payments per bill, tips |
| `FNB.DIN.05` | Service charge (hotel- or tourism-licensed outlets only, up to 10%) and Dubai municipality fee (7%) lines per outlet configuration |
| `FNB.DIN.06` | Reservation table allocation by party size and turn time; walk-in waiting list |
| `FNB.LOY.01` | Loyalty points for direct customers |
| `FNB.CST.01` | Theoretical vs actual food cost and variance per item and outlet |
| `FNB.WST.01` | Wastage logging with reasons and value |
| `FNB.PRD.01` | Central kitchen production planning and batches |
| `FNB.LBL.01` | Prep labels: item, production date, expiry, allergens |
| `FNB.ALG.01` | Allergen and calorie information per item |
| `FNB.FSF.01` | Food safety logs: temperature readings and opening/closing checks (configured on `WRK.CHK.01`) |
| `FNB.RPT.01` | Sales mix, channel profitability after commission, prep time and menu engineering reports |

</details>

<details><summary><b>07 Manufacturing</b>: 12 specific capabilities</summary>

| ID | Capability |
|---|---|
| `MFG.PRD.01` | Work orders from sales orders (make to order, the default) or for stock from minimum levels or the plan |
| `MFG.PRD.02` | Material issue to work orders and output reporting with scrap, from line tablets |
| `MFG.PRD.03` | Routings: operations, work centres, standard times |
| `MFG.PLN.01` | Production schedule board per line (manual placement, no optimisation) |
| `MFG.QCL.01` | QC plans per product: inspection points and limits |
| `MFG.QCL.02` | Non-conformance reports and disposition |
| `MFG.QCL.03` | Certificates of analysis / conformity per lot |
| `MFG.TRC.01` | Lot traceability: raw lots → finished lots → customer deliveries |
| `MFG.CST.01` | Standard cost roll-up: BOM materials at landed cost plus standard hours × an hourly rate per line (labour and overhead); actual cost from materials issued and the work order's start/stop hours |
| `MFG.ICV.01` | ICV data pack: manufacturing cost, local spend, Emirati headcount, assets, export revenue for the certifying body (the certificate itself is a document with expiry in the MVP) |
| `MFG.CMP.01` | Product conformity register: schemes, test reports, certifying bodies, links to lots (the *regulated* and *export only* item flags and certificates as documents are in the MVP) |
| `MFG.RPT.01` | Production reports: output vs plan, yield and scrap, on-time delivery, WIP |

</details>

<details><summary><b>08 Auto</b>: 18 specific capabilities</summary>

| ID | Capability |
|---|---|
| `AUT.CAT.01` | Part master: OEM numbers, brand, quality grade, cross-references |
| `AUT.CAT.02` | Vehicle fitment: parts by make, model, year, engine |
| `AUT.CAT.03` | VIN decoding and licensed parts data import |
| `AUT.VEH.01` | Vehicle register: plate, VIN, make/model/year, mileage, owner or fleet |
| `AUT.JOB.01` | Job card: check-in, complaints, labour, parts issued, technician, QC |
| `AUT.JOB.02` | Additional work authorisation during a job |
| `AUT.JOB.03` | Labour operations catalogue (menu and flat-rate pricing) |
| `AUT.JOB.04` | Vehicle service history and next-service reminder |
| `AUT.FLT.01` | Fleet service contracts: vehicles, rates, inclusions, schedules |
| `AUT.WAR.02` | Supplier warranty recovery, with warranty terms per brand or supplier |
| `AUT.INS.01` | Insurance claim registration: insurer, policy and claim number, police report, agency or non-agency repair, excess |
| `AUT.INS.02` | Estimates to insurers and surveyor approvals (LPO) with the approved amount per line |
| `AUT.INS.03` | Supplementary estimates and approvals for hidden damage |
| `AUT.INS.04` | Split billing: approved amount to the insurer; excess and private items to the customer |
| `AUT.INS.06` | Replacement car allocation and recharge to the insurer |
| `AUT.INS.07` | Insurer and surveyor portal: claims, estimates, approvals, repair status |
| `AUT.EXP.01` | Export sales: zero-rated VAT invoice, packing list and commercial invoice in USD; exit certificate and bill of lading tracked within 90 days |
| `AUT.RPT.01` | Reports: sales and margin by brand and part, dead stock, bay utilisation, technician productivity |

</details>

<details><summary><b>09 Manpower</b>: 9 specific capabilities</summary>

| ID | Capability |
|---|---|
| `MPW.CTR.01` | Client contracts and rate cards (roles, shifts, multipliers, minimums, billing cycle) |
| `MPW.ORD.01` | Manpower requests from clients |
| `MPW.REC.01` | Candidate pipeline: sourcing, screening, interview, offer |
| `MPW.DEP.01` | Deployments: worker → client site, role, shift, dates, bill rate |
| `MPW.DEP.02` | Replacements and cover during leave or absence |
| `MPW.CMP.01` | Site requirements: licences, passes and inductions a site requires |
| `MPW.ACM.01` | Accommodation and transport allocation and cost |
| `MPW.EMR.01` | Emiratisation quota tracking |
| `MPW.RPT.01` | Margin per client, deployment and worker; utilisation and bench |

</details>

<details><summary><b>10 EdTech</b>: 15 specific capabilities</summary>

| ID | Capability |
|---|---|
| `EDU.CRS.01` | Course structure: programme, levels, modules, hours, prerequisites, delivery mode |
| `EDU.CRS.02` | Regulator approvals for courses and trainers (permit numbers, expiry) |
| `EDU.BAT.01` | Batches with capacity, weekly pattern, tutor, room; sessions generated around holidays |
| `EDU.ENR.01` | Enrolments in batches, packages or memberships, with completion criteria |
| `EDU.ATT.01` | Student attendance per session |
| `EDU.MKP.01` | Make-up sessions and transfers between batches |
| `EDU.TRL.01` | Trial classes and placement tests (required per catalogue item; score or level entered by the tutor, suggested batch) with conversion tracking |
| `EDU.REF.01` | Withdrawal and refund calculation per policy (tiered default template, editable per catalogue item) |
| `EDU.LMS.01` | Learning materials per course and session |
| `EDU.LMS.02` | Assignments, submissions, grading and feedback |
| `EDU.ASM.01` | Assessments, exams and gradebook |
| `EDU.CRT.01` | Certificates with verification code, QR and public verification page |
| `EDU.PRG.01` | Progress reports for students, guardians and corporate clients |
| `EDU.ONL.01` | Online class links per session (Zoom, Teams, Meet) |
| `EDU.RPT.01` | Reports: enrolments, conversion, utilisation, attendance, retention, revenue per course |

</details>

## 5. Resolved overlaps

Decisions taken when the ten verticals were reconciled (2026-10-14). The module documents in
[modules/](modules/) hold the canonical definitions.

| # | Overlap | Decision |
|---|---|---|
| 1 | *Appointment* (05), *Booking* (08, 10) state machines | One **Booking** state machine in `wl-scheduling`. Verticals add fields and automations, not states |
| 2 | *Sales order* state machine in 07 and 08 | One **Sales order** state machine in `wl-sales`. F&B keeps its kitchen fulfilment machine as vertical-specific; it starts from a confirmed sales order |
| 3 | *Subscription* in 04 (retainer, wl-invoicing) and 10 (wl-payments) | One **Subscription** state machine in `wl-invoicing`. Automatic card charging goes through `wl-payments`, which uses renox-billing's gateways |
| 4 | `CON.ACL.03` and `STK.ACL.01`; `CON.ACL.10` and `FIN.ACL.03` | The shared IDs replace the construction ones |
| 5 | Dispatch: `STK.DSP.01`, `WRK.DSP.01`, Logistics' own dispatch | New module **`wl-delivery`** owns dispatch to drivers (`WRK.DSP.01`) and proof of delivery (`WRK.POD.01`). `wl-inventory` keeps pick, pack and delivery notes (`STK.DSP.01`) and hands over to `wl-delivery`. Logistics builds shipments, hubs and routes on top. Cash collected on delivery stays in `wl-payments` (`FIN.CSH.01`) |
| 6 | Staff records spread over `wl-attendance` | New module **`wl-hr`** (employees, visas, leave, payroll, WPS, final settlement). `wl-attendance` keeps attendance, timesheets and rosters |
| 7 | Sales features proposed by six verticals | New module **`wl-sales`** (leads, catalogue, orders, POS, packages, discounts, returns, credit limits) |
| 8 | Session packages `EDU.PKG.01` and prepaid packages `SAL.PKG.01` | Merged into `SAL.PKG.01` |
| 9 | `FNB.PRM.01` promo codes, `MFG.RTN.01` returns, `LOG.DSP.01` dispatch | Merged into `SAL.DSC.01`, `SAL.RET.01` and `WRK.DSP.01` |
| 10 | Promotion candidates confirmed by the module specifications | `LOG.RAT.01` → `FIN.RAT.01` (new shared rate cards, also used by 08 and 09); `AUT.FLT.02` → `FIN.USG.01`; `CLN.ROS.01` → `HRM.ROS.01`; `CLN.LIC.01` → `HRM.QUA.01` (new shared qualifications); `MPW.TSA.01` absorbed by `HRM.TMS.01`; `MFG.MNT.01` absorbed by `STK.AST.01`; `FNB.CSH.01` absorbed by `SAL.POS.01` (till session); `LOG.ATT.01` absorbed by `WRK.POD.01` |
| 11 | Capability IDs `PLT.RUL.01`–`02` used the area reserved for automation rules | Renamed `PLT.ATM.01`–`02`; kernel automations are `PLT.RUL.01`–`03` |
| 12 | Working calendars defined in several modules | One business calendar in `wl-kernel` |
| 13 | Rules repeated across verticals | Collected in the shared registers: [ACCESS-CONTROL.md](ACCESS-CONTROL.md) §9 and [WORKFLOW.md](WORKFLOW.md) §7. *Proposed* entries replace the listed vertical rules when each vertical is reviewed |
| 14 | Gap analysis (2026-10-14): platform capabilities nobody had listed | Added `PLT.ENT.01` legal entities ([decision 0003](decisions/0003-legal-entities.md)), `PLT.IMP.01` data import, `PLT.SRC.01` search, `PLT.NOT.01` notes and mentions, `PLT.SSO.01` staff SSO, `RPT.LIB.01` report library, `RPT.SCH.01` scheduled reports, `FIN.VAT.01` VAT summary, `FIN.EXP.01` expenses and petty cash, `FIN.BNK.01` bank statement matching, `WRK.FBK.01` feedback (absorbs `FNB.FDB.01`), `WRK.SVC.01` vendor service contracts |
| 15 | Gap analysis: our own business had no module | New [`wl-operator`](modules/wl-operator.md) (domain `OPS`): clients, provisioning, plans and entitlements, billing our clients, rollout, monitoring, support, demo instances, offboarding. Its capabilities are ours, not the clients', so they are not in the matrix above |
| 16 | Gap analysis: vertical gaps | 08 insurance accident repairs (`AUT.INS.*`), 06 dine-in and table reservations (`FNB.DIN.*`, Later), 01 vendor AMCs with fire, life safety and lift compliance (`WRK.SVC.01`, `WRK.PPM.01` now MVP) |
| 17 | `AUT.WAR.01` warranty claims and `WRK.TKT.01` (review of 08, 2026-10-15) | Customer warranty claims are tickets of category *warranty* in `wl-ticketing`; supplier recovery stays in Auto (`AUT.WAR.02`) |
| 18 | `AUT.INS.05` insurer receivables and `CLN.CLM.*` insurance claims (review of 08, 2026-10-15) | New shared `FIN.TPP.01` third-party payer billing in `wl-invoicing`: split billing, payer invoices, remittance reconciliation and payer receivables. Clinical coding, claim channels and denial codes stay in Clinics; surveys, estimates and supplementaries stay in Auto |

## 6. Promotion candidates

Partial overlaps still open after the module specifications. Candidates that were confirmed
moved to §5, row 10.

| Specific | Overlaps with | Status |
|---|---|---|
| `LOG.FLT.01` fleet details | `STK.AST.01` | Partly: vehicle details, documents and meter readings belong to the Asset in `wl-inventory`; fuel logs stay in Logistics |
| `AUT.FLT.01` fleet service contracts | `FIN.RAT.01`, `FIN.USG.01` | Partly: rates use shared rate cards; inclusions and service schedules stay in Auto |
| `CLN.TEL.01` telehealth, `EDU.ONL.01` online classes | `SCH.BKG.01` | Partly: the meeting link and provider hook are on the shared Booking; clinical telehealth rules stay in Clinics |
| `AUT.VEH.01` customer vehicles | `STK.AST.01` | Rejected: customer vehicles are not company assets |
| `FNB.LOY.01` loyalty points | — | Rejected for now: becomes a shared `SAL` capability when a second vertical asks for it |

## 7. Keeping this map current

1. A capability changes in a vertical's §5 first; this map follows.
2. A new shared capability gets the next free number in its domain and area, a row in §3, and
   a line in the owning module's document.
3. A specific capability that a second vertical needs moves to a shared domain. The row keeps
   the old ID as *formerly*, and every reference in the vertical documents is updated.
4. A shared capability used by only one vertical must name at least one vertical in
   *Expected also*. Otherwise it moves back to that vertical (rule of three).
   Exceptions, decided 2026-10-15: platform features that every deployment can switch on
   (`PLT.API.01` client API, `PLT.SCN.01` camera scanning) and compliance duties that apply
   to any regulated client (`PTY.UBO.01` beneficial owners, `PTY.SCR.01` sanctions
   screening) stay shared even with one vertical.
