# Pricing and Packaging

> Status: draft for the owner · Last updated: 2026-10-14

This document defines **what we sell and how it is measured**. It does not fix prices: every
amount is an open question for the owner (§7). Plans and add-ons become entitlements in
[wl-operator](modules/wl-operator.md) (`OPS.ENT.01`), which each client deployment enforces.

## 1. Principles

1. **One vertical app per client**, priced by plan plus one pricing metric that grows with
   the client's business.
2. **Plans differ by limits and breadth, not by crippling the core.** Every plan can run the
   vertical's MVP.
3. **Setup is paid.** Provisioning, branding and data migration are real work and are billed
   once.
4. **Prices are in AED, excluding VAT**, billed monthly, or yearly with a discount.

## 2. Plans

| | **Starter** | **Business** | **Group** |
|---|---|---|---|
| For | A single small business | An established SME | A group with several entities or sites |
| Vertical MVP modules | ✓ | ✓ | ✓ |
| Staff users included | 5 | 20 | 50 |
| Legal entities ([0003](decisions/0003-legal-entities.md)) | 1 | 1 | up to 5 |
| Scopes (buildings, projects, branches…) | 1 | up to 5 | unlimited |
| Portal for external parties | basic (view, pay, request) | full | full, several brands |
| Email notifications | ✓ | ✓ | ✓ |
| SMS / WhatsApp | add-on | included quota | included quota |
| Later-scope modules of the vertical | — | selected | all |
| Report library and exports | ✓ | ✓ | ✓ + scheduled reports |
| Approvals and automations | defaults | defaults + parameters | defaults + parameters |
| Custom fields | 10 | 50 | unlimited |
| Single sign-on (Microsoft / Google) | — | — | ✓ |
| Support | email, business hours | priority | priority + named account manager |
| Backups | daily retention 30 days | 90 days | 1 year |

Limits in this table are proposals; confirm with the owner.

## 3. Add-ons

| Add-on | Unit | Notes |
|---|---|---|
| Extra staff users | per 5 users | |
| Extra legal entity | per entity | Group only above 5 |
| WhatsApp / SMS bundles | per 1,000 messages | provider cost passed through + margin |
| Online payments | per client | gateway fees are the client's own contract |
| Extra module | per module | a module outside the plan (e.g. `wl-hr` payroll for a property client) |
| Accounting export connector | per client | Zoho Books, Xero, QuickBooks |
| Client-specific extension | quote | extension crate ([ARCHITECTURE.md](ARCHITECTURE.md) §7) |
| Extra storage | per 50 GB | documents and photos |

## 4. One-time fees

| Fee | Covers |
|---|---|
| Setup | provisioning, domain, branding, first entity, admin training |
| Data migration | import of the client's Excel data (`PLT.IMP.01`), dry run and sign-off; priced by volume or number of sources |
| Training | per session, on site or remote |
| Custom document templates | invoice, contract or report layouts beyond the defaults |

## 5. Pricing metric per vertical

The metric is counted by each deployment and reported monthly to the operator (`OPS.USG.01`).

| # | Vertical | Metric | Why |
|---|---|---|---|
| 01 | Property | managed units | grows with the portfolio |
| 02 | Construction | active projects, or workers on attendance | grows with site activity |
| 03 | Logistics | shipments per month, or vehicles | volume business |
| 04 | Accounting | client companies (engagements) | the firm's own pricing unit |
| 05 | Clinics | practitioners | capacity of the clinic |
| 06 | F&B | outlets / kitchens | each outlet is a site |
| 07 | Manufacturing | production lines, or plants | capacity |
| 08 | Auto | branches / workshops | each branch is a site |
| 09 | Manpower | deployed workers | revenue driver of the agency |
| 10 | EdTech | active students | revenue driver of the centre |

Each plan includes a band of the metric (e.g. Starter up to N units); above the band, a
per-unit price applies.

## 6. Commercial terms

- Minimum term: **12 months, then monthly** with 30 days' notice (D-03, decided 2026-10-15).
- Yearly prepayment: **2 months free** (pay 10 months for 12, about 17% off) (D-04, decided 2026-10-15).
- Non-payment: notice, then read-only after the grace period (`OPS.RUL.02`), never deletion.
- Exit: full export included, then verified deletion within 30 days
  ([wl-operator](modules/wl-operator.md) §5.6, [OPERATIONS.md](OPERATIONS.md) §10).
- Price changes are announced **60 days ahead** and apply from the next renewal (the next
  year for yearly contracts, the next month for monthly ones); prices are locked during a
  running term (D-05, decided 2026-10-15).

## 7. Open questions for the owner

1. Monthly price of each plan, and the metric band each includes, per vertical.
2. Setup and migration fees: fixed per plan, or quoted per client?
3. ~~Discount for yearly prepayment, and the minimum term.~~ Decided (2026-10-15): D-03,
   D-04.
4. Should clinics and other regulated verticals carry a compliance surcharge (stricter
   hosting, audits)?
5. Is a free pilot (e.g. 30 days on a demo copy with their data) offered?
6. Message bundle sizes and margin on SMS / WhatsApp.
7. Do resellers or partners get a margin, and is there a partner plan?
