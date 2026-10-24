# Vertical build status

**As of 2026-10-24** (main at `021526bb`, phases F1-F10 merged). A spec marked "accepted" is a specification, not a statement
that everything in it is built. Each spec carries a **Build status** line under its title; this page puts the ten side by side.

How the status was found: the state sets of §7.1 were compared by script with every `impl StateMachine` in `apps/*` (they are
identical for all app-owned machines), the `XXX.RUL.nn` ids of §7.2 were searched in `apps/`, `host/` and `crates/`, and §12 and
each app README's "not built" list were read against the code. "Open" means an MVP item of §12 with no code behind it.

| Vertical | App | Client | Demo command | Machines in code | MVP built | Open (MVP items not built) |
|---|---|---|---|---|---|---|
| 01 Property | `apps/property` | `demo-properties` | `property:demo` | Unit, Lease, ManagementAgreement, OwnerStatement | all §7.1 machines; leases and every negative path, charges, invoices, cheques, deposits, owner statements | tenant portal; rent roll report (`PRP.RPT.01`); EN/AR contract PDF; Excel migration (`PLT.IMP.01`) |
| 02 Construction | `apps/construction` | `demo-construction` (no `sales`) | `construction:demo` | Project, Variation, Ipa, Retention, Guarantee; **extra:** IpaDeduction, RetentionDeduction | project to close, termination and final account, variations (omission, time), deductions, guarantees, absence, stock and labour on shared modules | sites and geofences; BOQ import and IPA calculation, `revise`, `CON.RUL.04`; advance recovery; daily site reports (`CON.RUL.02`); snag lists; `CON.RUL.05`; crews; project pages and client portal |
| 03 Logistics | `apps/logistics` | `demo-logistics` | `logistics:demo` | Shipment, Remittance, FreightJob; **extra:** AdjustmentLine, AccountChange | shipments and all negative flows, COD remittance, freight jobs with free time, runs and driver PWA (wl-delivery) | vehicle register (services, checks, fines, tolls, odometer CSV); public tracking link; `LOG.RUL.04`, `LOG.RUL.05`; per-client freight switch; rate cards still a local stand-in |
| 04 Accounting | `apps/accounting` | `demo-accounting` | **none** (F4 has no demo) | Engagement, CddFile, Return, AmlCase, KpiPack; **extra:** TaxProfileChange, WipWriteOff | onboarding with CDD and sanctions, calendar, retainer, returns with review and client approval, AML, KPI packs (generic CSV) | time entries and WIP billing (`ACC.TIM.01`; no provider of `UnbilledTime`); KPI presets (`ACC.KPI.01`) |
| 05 Clinics | `apps/clinics` | `demo-clinics` | `clinics:demo` | PriorAuthorisation, Claim; **extra:** Visit, RecallNotice (spec has Appointment and Package on shared modules) | check-in with eligibility and coverage outcomes, claims with rejections and void, remittance recording, recalls | patient portal (only card requests); patient feedback; waiting list; bank-side matching of remittances |
| 06 F&B | `apps/fnb` | `demo-fnb` | `fnb:demo` | FnbOrder, KitchenTicket, ProductionBatch, PayoutStatement, DailyClosing; **extra:** Outage | orders, kitchen display, production, payout reconciliation, closing, recalls, outage and paper orders | Deliverect; D2C ordering site; `FNB.RUL.02`, `06`, `07`, `09`; authority notification (AU-20) |
| 07 Manufacturing | `apps/manufacturing` | `demo-manufacturing` | `manufacturing:demo` | WorkOrder, Ncr; **extra:** ProductionReport | work orders, BOM and lots, reports, NCR, credit hold, dispatch, recall, export evidence, portal PO | schedule board and shortage list; `MFG.RUL.01`, `02`, `03`, `05`, `06`; deferred rows (calibration, checklists, scanner, post-invoice delivery) in [f7-manufacturing](../progress/f7-manufacturing.md) |
| 08 Auto | `apps/auto` | `demo-auto` | `auto:demo` | JobCard, SupplierClaim, FleetContract, InsuranceClaim | job cards, estimates by link, insurance claims, fleet contracts, supplier claims, storage, recalls | part master with cross-references, fitment, price tiers; Excel parts import; B2B portal; counter POS; `AUT.RUL.10`, `14` (and the messages of `11`, `13`); see [f8-auto-deferred](../progress/f8-auto-deferred.md) |
| 09 Manpower | `apps/manpower` | `demo-manpower` | `manpower:demo` | ClientContract, ManpowerRequest, Deployment | contracts, requests, deployments, rate cards, timesheet approval, payroll on wl-hr, exits | worker portal (`POR.SLF.02`); candidates are Later |
| 10 EdTech | `apps/edtech` | `demo-edtech` | `edtech:demo` | Batch, Enrolment, Session, Certificate (spec also lists Submission: **Later**) | batches, seats, enrolment, withdrawal and refund approval, freeze, transfer, absence credits, attendance, certificates | leads and trial classes (shared modules); e-signature (`DOC.SIG.01`); certificate QR and PDF; `EDU.RUL.01`, `05`, `08`, `09`, `10`; make-up booking; student and guardian portal; loyalty segments. Materials, assessments, gradebook and Submission: **Later** (spec §12 reconciled) |

## Open issues on GitHub (not covered by the phases above)

- Build 1 follow-ups `#458`-`#479` (22 issues, F1 work the closed F1 issues deferred).
- Search and agents `BL-AGT-01`..`13`: `#541`-`#554` (nothing built).
- Deferred leftovers: `#207`, `#213` (wl-hr), `#278`, `#279` (property), `#298` (construction), `#346` (accounting), `#434`, `#435` and the screens `#502`, `#503`, `#505`-`#507`, `#509`, `#510`, `#513` (manpower).

The phase notes are in [docs/progress/](../progress/README.md).
