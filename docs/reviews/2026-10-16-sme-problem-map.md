# Review: Common SME Problems Mapped to the Platform

> Status: decided · Date: 2026-10-16 · All seven gaps accepted as MVP on 2026-10-16 · Scope: the shared modules and the ten verticals

A list of 50 problems that small and medium businesses commonly face, grouped into 10
business areas, was checked against the platform. For each problem the review asks: does the
platform already solve it, solve part of it, or is it outside what software can do? The
problems are generic. Where they name a country-specific item, the UAE equivalent is used:
tax reporting means VAT and corporate tax with the FTA; product certification means MoIAT,
ESMA or Dubai Municipality schemes; a business licence means the trade licence.

The result also serves sales: each ✅ row is a "problem we solve" statement for
[go-to-market](../go-to-market/README.md).

## 1. Result

| | Count | Meaning |
|---|---|---|
| ✅ | 20 | Solved by capabilities in the current MVP scope |
| 🟡 | 22 | Partly solved: the data or a vertical-specific feature exists, a shared piece is missing |
| ⬜ | 8 | Outside what software can do (strategy, recruiting, partnerships, research); consulting, not features |

Strongest areas: error-proofing (guards on every transition, expired documents that block
work, credit re-checks, duplicate checks, segregation of duties), integration (one app, one
database per client), receivables and margin visibility. Weakest: marketing measurement,
company-level planning, supplier performance and retention campaigns.

## 2. Mapping

### 2.1 Marketing and sales

| # | Problem | | Platform today | Missing |
|---|---|---|---|---|
| 1 | No systematic funnel; leads come by word of mouth | 🟡 | Leads pipeline (`SAL.CRM.01`); enquiries and viewings (`PRP.LEA.01`) | Lead source and conversion by source |
| 2 | Online channels tried at random | 🟡 | Branded ordering site (`FNB.DTC.01`), self-booking (`SCH.SLF.01`), payment links (`FIN.PAY.02`) | Conversion by channel |
| 3 | Dependence on one market segment | ⬜ | Channels and legal entities (`FNB.CHN.01`, `PLT.ENT.01`) help only indirectly | A strategy choice |
| 4 | Advertising ROI and customer acquisition cost unknown | ⬜ | None | Campaign cost and CAC report (gap G1) |
| 5 | Distribution and retail partners | 🟡 | Dispatch and delivery (`WRK.DSP.01`), orders from every channel (`SAL.ORD.01`), aggregators (06) | A retailer or reseller network |

### 2.2 Operations and production

| # | Problem | | Platform today |
|---|---|---|---|
| 6 | Repeated manual steps | ✅ | Automation rules (`PLT.ATM.01`), Excel import (`PLT.IMP.01`), barcode scanning (`PLT.SCN.01`) |
| 7 | No written SOPs; results vary by employee | 🟡 | State machines enforce the order of work; checklists with photos (`WRK.CHK.01`); QC plans (`MFG.QCL.01`). No SOP library (gap G6) |
| 8 | Poor facility layout | 🟡 | Separate stock locations, quarantine for held goods (`STK.LOC.01`). The physical layout stays with the client |
| 9 | Non-value-added steps, double forms | ✅ | One record used across modules; no re-keying between apps |
| 10 | Capacity bottlenecks when demand spikes | 🟡 | Production board (`MFG.PLN.01`), rosters (`HRM.ROS.01`), staff capacity (`ACC.CAP.01`), late-order alerts (`FNB.KDS.02`). No capacity forecast |

### 2.3 Finance and accounting

| # | Problem | | Platform today | Missing |
|---|---|---|---|---|
| 11 | Personal and business money mixed | 🟡 | Expense claims and petty cash (`FIN.EXP.01`), bank matching (`FIN.BNK.01`) | Owner drawings account (gap G5) |
| 12 | Late collections while supplier bills fall due | ✅ | Reminders and ageing (`FIN.REC.01`), payment links, PDC register (`FIN.CHQ.01`), credit limits and holds (`SAL.CRD.01`), disputes and write-off, supplier bill due dates (`FIN.PAB.01`) | |
| 13 | Books not bankable | 🟡 | VAT tax invoices, audit trail, export to Zoho Books, Xero, QuickBooks (`FIN.EXT.01`) | The platform is not a general ledger by design |
| 14 | No budgeting or forecasting | 🟡 | Budget vs committed vs actual per cost object (`FIN.CST.01`), project margin forecast (`CON.RPT.01`) | Company budget and cash-flow forecast (gaps G2, G5) |
| 15 | Hidden overhead; profit on paper, no cash | ✅ | Job costing, theoretical vs actual food cost (`FNB.CST.01`), landed cost (`STK.LND.01`), channel profit after commission (`FNB.RPT.01`), margin per worker (`MPW.RPT.01`) | |

### 2.4 Human resources

| # | Problem | | Platform today |
|---|---|---|---|
| 16 | Overlapping roles, no job descriptions | 🟡 | Scoped roles, segregation of duties and approval limits ([ACCESS-CONTROL.md](../ACCESS-CONTROL.md)). No job descriptions |
| 17 | Hard to recruit | ⬜ | Candidate pipeline only in 09 (`MPW.REC.01`) |
| 18 | No onboarding or training | 🟡 | Employee onboarding state, required qualifications (`HRM.QUA.01`). No training checklist (gap G6) |
| 19 | Business stops when the founder is away | ✅ | Delegated, multi-step approvals (`PLT.APR.01`), time-bound roles (`PLT.USR.01`), owner alerts (`PLT.RUL.04`), role dashboards (`RPT.DSH.01`) |
| 20 | High turnover | 🟡 | Payroll, leave and final settlement are handled correctly (wl-hr). No career paths or compensation planning |

### 2.5 Technology and digitalisation

| # | Problem | | Platform today |
|---|---|---|---|
| 21 | Data scattered across laptops | ✅ | One database per client, global search (`PLT.SRC.01`), daily backups and restore drills ([OPERATIONS.md](../OPERATIONS.md)) |
| 22 | Software bought but not adopted | 🟡 | Excel migration with dry run, role-based screens ([UX.md](../UX.md)), demo data, onboarding service |
| 23 | Many free apps, nothing connected | ✅ | All modules in one app; API (`PLT.API.01`), webhooks (`PLT.HOK.01`), accounting export |
| 24 | Manual recaps invite human error | ✅ | Scheduled reports (`RPT.SCH.01`), WPS file generation (`HRM.WPS.01`), bank matching |
| 25 | Weak connectivity and security | ✅ | Hosting in the UAE, two-factor login, audit trail ([SECURITY.md](../SECURITY.md)); late entry after an outage (`PLT.ACL.10`) |

### 2.6 Supply chain and logistics

| # | Problem | | Platform today |
|---|---|---|---|
| 26 | Stockouts and dead stock | ✅ | Reorder points and suggested purchases (`STK.ROP.01`), dead-stock report (`STK.RUL.05`), lot expiry warnings (`STK.RUL.03`) |
| 27 | One supplier, no alternatives or audits | 🟡 | Supplier register. No alternative suppliers per item and no supplier audit (gap G3) |
| 28 | No quality check on goods received | ✅ | Accepted and rejected quantity at receipt, temperature checks, incoming inspection, return to supplier and debit note (wl-inventory, wl-invoicing) |
| 29 | Outbound shipping cost not optimised | 🟡 | Runs by area, rate cards (`FIN.RAT.01`), cost per drop (`LOG.RPT.01`) in 03. No courier rate comparison for other verticals |
| 30 | Physical stock differs from the books | ✅ | Barcode scanning, stock counts with variance (`STK.CNT.01`), asset register (`STK.AST.01`) |

### 2.7 Customer service and relationship management

| # | Problem | | Platform today |
|---|---|---|---|
| 31 | No central customer database | ✅ | Party register (`PTY.PTY.01`), notes and timeline on every record (`PLT.NOT.01`) |
| 32 | Existing customers neglected after the sale | 🟡 | Patient recalls (`CLN.RCL.01`), next-service reminders (`AUT.JOB.04`), package expiry reminders (`SAL.RUL.02`), lease renewals (`PRP.LSE.04`). No general campaigns (gap G4) |
| 33 | Feedback not used | ✅ | Ratings after a completed service; a low rating opens a ticket (`WRK.FBK.01`) |
| 34 | Slow, unmeasured responses | ✅ | Ticket SLAs and escalation (`WRK.TKT.02`, `WRK.RUL.01`) |
| 35 | No loyalty scheme | 🟡 | Loyalty points in 06 only (`FNB.LOY.01`); discounts and prepaid packages everywhere (`SAL.DSC.01`, `SAL.PKG.01`) (gap G4) |

### 2.8 Legal and compliance

| # | Problem | | Platform today |
|---|---|---|---|
| 36 | Licences obtained late | 🟡 | Trade licence and permit expiry tracking; expired documents block work (`DOC.EXP.01`, wl-documents §5.6). Obtaining the licence stays outside |
| 37 | Trademarks and IP not registered | ⬜ | Not covered. Tracking registrations is cheap as document kinds with expiry (gap G7) |
| 38 | B2B agreements on trust only | ✅ | Templates in EN/AR (`DOC.GEN.01`), e-signature (`DOC.SIG.01`), versioned agreements in each vertical |
| 39 | Books not ready for tax returns | ✅ | Per-line VAT category (`FIN.INV.01`), VAT summary (`FIN.VAT.01`), e-invoicing from 2027, VAT and corporate tax workflows for accounting firms (04) |
| 40 | Certification delays sales | 🟡 | Product conformity register (`MFG.CMP.01`), certificates per lot (`MFG.QCL.03`), construction permits (`CON.PMT.01`), course approvals (`EDU.CRS.02`). Halal and ISO not named (gap G7) |

### 2.9 Strategy and planning

| # | Problem | | Platform today |
|---|---|---|---|
| 41 | No business plan or 3–5 year roadmap | ⬜ | Outside software |
| 42 | Scaling before the basics work | ⬜ | Branch and entity data help to see it; the decision stays outside |
| 43 | Decisions without data | ✅ | Role dashboards (`RPT.DSH.01`), report library (`RPT.LIB.01`), KPI reports in every vertical |
| 44 | Narrow partner network | ⬜ | Outside software |
| 45 | Slow to adapt to market change | 🟡 | Catalogue, prices, channels and rule parameters are configuration; client-editable rules are Later (`PLT.ATM.02`) |

### 2.10 Quality and innovation

| # | Problem | | Platform today |
|---|---|---|---|
| 46 | Inconsistent product or service quality | ✅ | Versioned recipes and BOMs (`STK.BOM.01`), QC plans, checklists |
| 47 | No error-proofing, only end inspection | ✅ | Guards on every transition ([WORKFLOW.md](../WORKFLOW.md) §3, §8), document blocks, credit re-check at dispatch, duplicate checks, import validation |
| 48 | No product research or variants | ⬜ | Menu engineering and sales mix in 06 (`FNB.RPT.01`) only |
| 49 | No vendor quality metrics | 🟡 | Rejected quantities, NCRs and returns per supplier are recorded; no scorecard (gap G3) |
| 50 | No continuous improvement | 🟡 | Reports and audit trail give the data; no internal audit workflow |

## 3. Gaps worth adding

Ordered by how many 🟡 rows they turn into ✅ and by fit with the lead-generation goal of the
ten verticals. Each is a candidate for a decision; none is in scope yet.

| # | Gap | Fills | Shape | Likely home |
|---|---|---|---|---|
| G1 | Lead source, campaign cost, conversion by source and channel, CAC and ROI report | 1, 2, 4 | Source and campaign fields on leads and parties; campaign cost entries; report | wl-sales (`SAL.CRM.01` extension) |
| G2 | Cash-flow forecast: receivables by due date, supplier bills, scheduled charges, payroll, PDCs | 12, 14 | Report over existing data | wl-invoicing + wl-payments |
| G3 | Supplier scorecard and alternative suppliers per item | 27, 49 | On-time delivery, rejected %, NCRs, price variance; preferred and alternative supplier per item | wl-inventory |
| G4 | Retention campaigns and shared loyalty | 32, 35 | Promote `FNB.LOY.01` to a shared capability; segment and send campaigns with consent (`NTF.ACL.01`) | wl-sales + wl-notify |
| G5 | Company budget vs actual, owner drawings account | 11, 14 | Budget lines per period and category; drawings as an expense category kept out of operating cost | wl-invoicing |
| G6 | Staff onboarding checklist and SOP library | 7, 18 | Checklist templates on onboarding (`WRK.CHK.01`); SOP documents per role (wl-documents) | wl-hr + wl-documents |
| G7 | Trademark, Halal, ISO and similar registrations | 37, 40 | Document kinds with expiry and reminders; configuration only | wl-documents |

G6 and G7 are mostly configuration of existing modules. G1 to G5 add data and reports but no
new state machines.

## 4. Decision (2026-10-16)

The owner accepted all seven gaps, all in the MVP.

| Gap | Capability | Module | Verticals |
|---|---|---|---|
| G1 | `SAL.CRM.02` | wl-sales | all; `SAL.CRM.01` (leads) moves to MVP where it was Later, and 01 `PRP.LEA.01` (enquiries) too |
| G2 | `FIN.CFL.01` | wl-invoicing | all |
| G3 | `STK.SUP.01` | wl-inventory | 02, 06, 07, 08 (05 Later, with its purchasing) |
| G4 | `SAL.LOY.01` (formerly `FNB.LOY.01`) | wl-sales + wl-notify | 05, 06, 08, 10 |
| G5 | `FIN.BUD.01` | wl-invoicing | all |
| G6 | `HRM.ONB.01` | wl-hr + wl-documents | all; the minimal employee record (`HRM.EMP.01`) moves to MVP in 01, 07 and 08, and checklists (`WRK.CHK.01`) are added to 05, 09 and 10 |
| G7 | `DOC.EXP.01` seeded kinds (no new ID) | wl-documents | per vertical setting |

New rules: `SAL.RUL.07`–`08`, `NTF.RUL.02`, `FIN.RUL.23`–`24`, `STK.RUL.09`, `HRM.RUL.16`–`17`,
`DOC.RUL.04`. New access rules: `SAL.ACL.07`, `NTF.ACL.03`, `FIN.ACL.21`, `STK.ACL.07`,
`HRM.ACL.10`. Open questions: L-38, A-15, A-16. Backlog issues carry the label `area:sme-gap`.

## 5. Where it is written

Each gap has its capability row in [CAPABILITY-MAP.md](../CAPABILITY-MAP.md), its module
specification, the vertical settings, and backlog issues
([backlog/README.md](../backlog/README.md)).
