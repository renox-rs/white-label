# White-Label UAE SME Platform — Documentation

A platform of reusable business modules on [Renox](https://renox.rs) that covers ten UAE SME
verticals. Documentation came first ([decision 0002](decisions/0002-document-all-ten-verticals-first.md));
the build started with F1 (Property) ([decision 0004](decisions/0004-start-f1-build.md)), phase F*n* builds vertical *n* ([decision 0005](decisions/0005-phases-follow-vertical-numbers.md)), and F1–F10 are built.

## How to read

| If you want to… | Read |
|---|---|
| See the whole picture: what is shared and what is specific | [CAPABILITY-MAP.md](CAPABILITY-MAP.md) |
| Understand one business | [verticals/](#verticals) |
| Understand one module | [Modules](#modules) |
| Know how the platform is layered and deployed | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Know who may do what (RBAC + ABAC) | [ACCESS-CONTROL.md](ACCESS-CONTROL.md) |
| Know how work moves: statuses, approvals, automations | [WORKFLOW.md](WORKFLOW.md) |
| Know how it looks | [DESIGN-SYSTEM.md](DESIGN-SYSTEM.md) |
| Know how each role's screens work | [UX.md](UX.md) |
| Prepare a sales demo | [DEMO-DATA.md](DEMO-DATA.md) |
| Know how we package and price it | [PRICING.md](PRICING.md) |
| Run it: environments, releases, backups, support, onboarding | [OPERATIONS.md](OPERATIONS.md) |
| Know the threats and security controls | [SECURITY.md](SECURITY.md) |
| Know how it is tested | [TESTING.md](TESTING.md) |
| Look up a term (TRN, Ejari, PDC, WPS…) | [GLOSSARY.md](GLOSSARY.md) |
| Know why something was decided | [decisions/](decisions/) |
| See what is still open: questions for lawyers, accountants, authorities, and owner decisions | [OPEN-ITEMS.md](OPEN-ITEMS.md) |
| Find and approach leads for a vertical | [go-to-market/](go-to-market/README.md) |
| See review findings and how they were closed (negative flows, SME problem map) | [reviews/](reviews/) |
| Read the notes of a finished build phase | [progress/](progress/) |
| Rules for coding agents: grid, fill, build workflow | [../host/AGENTS.md](../host/AGENTS.md), [../.claude/skills/](../.claude/skills/) |
| Pick up build work: draft issues per module and vertical, written for developers and coding models | [backlog/](backlog/README.md) |

## Verticals

| # | Vertical | Document | Status |
|---|---|---|---|
| 01 | Property & building management | [01-property.md](verticals/01-property.md) | accepted (2026-10-15) |
| 02 | Construction & fit-out subcontractors | [02-construction.md](verticals/02-construction.md) | accepted (2026-10-15) |
| 03 | Logistics & freight forwarding | [03-logistics.md](verticals/03-logistics.md) | accepted (2026-10-15) |
| 04 | Accounting firms & fractional CFO | [04-accounting.md](verticals/04-accounting.md) | accepted (2026-10-15) |
| 05 | Private clinics | [05-clinics.md](verticals/05-clinics.md) | accepted (2026-10-15) |
| 06 | Cloud kitchens & F&B chains | [06-fnb.md](verticals/06-fnb.md) | accepted (2026-10-15) |
| 07 | Light manufacturing & assembly | [07-manufacturing.md](verticals/07-manufacturing.md) | accepted (2026-10-15) |
| 08 | Auto parts & fleet services | [08-auto.md](verticals/08-auto.md) | accepted (2026-10-15) |
| 09 | Manpower supply & outsourcing | [09-manpower.md](verticals/09-manpower.md) | accepted (2026-10-15) |
| 10 | Training centres & EdTech | [10-edtech.md](verticals/10-edtech.md) | accepted (2026-10-15) |

## Modules

| Module | Covers | Status |
|---|---|---|
| [`wl-kernel`](modules/wl-kernel.md) | Platform services: settings, branding, legal entities, sequences, custom fields, approvals, automations, business calendar, import, search, notes, reports | built (F1-F10) |
| [`wl-parties`](modules/wl-parties.md) | Companies and people in any role, identity, relationships, consents, UBO, screening | built (F1-F10) |
| [`wl-portal`](modules/wl-portal.md) | External users: access grants, self-service, document requests | built (F1-F10) |
| [`wl-documents`](modules/wl-documents.md) | Files, expiry reminders, generated PDFs, e-signature, SOP library with acknowledgements | built (F1-F10) |
| [`wl-notify`](modules/wl-notify.md) | Email, WhatsApp, SMS, marketing broadcasts with consent | built (F1-F10) |
| [`wl-invoicing`](modules/wl-invoicing.md) | Quotations, invoices, subscriptions, rate cards, usage billing, costing, receivables, supplier bills, VAT summary, company budget, cash-flow forecast | built (F1-F10) |
| [`wl-payments`](modules/wl-payments.md) | Payments, payment links, cheques, deposits, cash custody, expense claims and petty cash, bank statement matching | built (F1-F10) |
| [`wl-sales`](modules/wl-sales.md) | Leads, campaigns and acquisition cost, catalogue, orders, POS, packages, discounts, loyalty, returns, credit limits | built (F1-F10) |
| [`wl-ticketing`](modules/wl-ticketing.md) | Tickets, checklists, planned maintenance, recurring tasks, customer feedback, vendor service contracts | built (F1-F10) |
| [`wl-delivery`](modules/wl-delivery.md) | Dispatch, runs, proof of delivery | built (F1-F10) |
| [`wl-scheduling`](modules/wl-scheduling.md) | Resources, bookings, self-booking, reminders, waiting list | built (F1-F10) |
| [`wl-attendance`](modules/wl-attendance.md) | Attendance, timesheets, rosters | built (F1-F10) |
| [`wl-hr`](modules/wl-hr.md) | Employees, onboarding, qualifications, visa cases, leave, payroll, WPS, final settlement | built (F1-F10) |
| [`wl-inventory`](modules/wl-inventory.md) | Items, stock, purchasing, supplier scorecards, lots, BOMs, dispatch notes, assets | built (F1-F10) |
| [`wl-operator`](modules/wl-operator.md) | **Ours, not the clients':** clients, provisioning, plans and entitlements, billing our clients, rollout, monitoring, support, demo instances | built (F1-F10) |

## Reconciliation

The ten verticals were reconciled into shared capabilities across 14 client-side modules,
plus `wl-operator` for us (2026-10-14). A re-analysis the same day added legal entities
([decision 0003](decisions/0003-legal-entities.md)), data import, search, notes, reports, VAT
summary, expenses, bank matching, feedback, service contracts, the operator control plane,
insurance repairs (08), dine-in (06) and AMCs (01). Decisions are in [CAPABILITY-MAP.md](CAPABILITY-MAP.md) §5; overlaps still
open are in §6. Rules repeated across verticals are collected in
[ACCESS-CONTROL.md](ACCESS-CONTROL.md) §9 and [WORKFLOW.md](WORKFLOW.md) §7; their *proposed*
entries replace the vertical rules when each vertical is reviewed.

## Conventions

These rules apply to every document, for human and agent readers alike.

**Language.** All documents are written in English.

**Templates.** Every vertical follows [templates/vertical.md](templates/vertical.md) and every
module follows [templates/module.md](templates/module.md). Keep the headings exactly as in
the template, so that the same question always has its answer under the same heading.

**Capability IDs.** Every capability has the ID `DOMAIN.AREA.NN`, for example `FIN.BIL.01`.
`DOMAIN` and `AREA` are three uppercase letters; `NN` is two digits.
An ID never changes and is never reused. Issues, PRs and other documents refer to
capabilities by ID.

| Domain | Meaning | Kind |
|---|---|---|
| `PLT` | Platform: tenancy of the deployment, branding, settings, users, roles, audit | shared |
| `PTY` | Parties: companies, people, contacts | shared |
| `POR` | External portals and self-service | shared |
| `DOC` | Documents, contracts, expiry | shared |
| `FIN` | Billing, invoicing, payments, receivables, payables | shared |
| `WRK` | Work orders, tickets, tasks, SLAs | shared |
| `NTF` | Notifications and messaging | shared |
| `HRM` | Workforce: attendance, timesheets, payroll exports | shared |
| `SCH` | Scheduling and booking | shared |
| `STK` | Stock, items, purchasing | shared |
| `SAL` | Sales: leads, catalogue and price lists, sales orders, counter sales | shared |
| `OPS` | Operator: our control plane for clients, plans, billing, rollout ([wl-operator](modules/wl-operator.md)) | ours, not clients' |
| `RPT` | Reporting and dashboards | shared |
| `PRP` `CON` `LOG` `ACC` `CLN` `FNB` `MFG` `AUT` `MPW` `EDU` | Specific to one vertical (01–10) | specific |

A capability that starts as vertical-specific moves to a shared domain once a second vertical
needs it. It keeps its old ID in the *Formerly* column of the capability map.

**Access rule IDs.** Access rules use the area `ACL` within a domain: `PRP.ACL.01` for a
vertical's rule, `FIN.ACL.01` for a shared module's rule, `PLT.ACL.01` for platform-wide rules
([ACCESS-CONTROL.md](ACCESS-CONTROL.md) §8).

**Markers.** `MVP` = needed for the first sellable version · `Later` = after the MVP ·
`⚠ verify` = a fact in a draft that must be checked against an official source before it
is relied on · `✔ [Sn]` = checked against source `Sn` in the document's **Sources** table
(date checked, and whether the source is official, press or secondary) · `⚠ open: <owner>`
= checked as far as public sources allow; the rest needs a `lawyer`, `accountant`,
`authority`, `client` or `onboarding` answer, tracked by ID in [OPEN-ITEMS.md](OPEN-ITEMS.md) · `⚠ check` = to be checked in the Renox code ·
`⚠ decide` = a business decision for the owner.
A verified document ends with a `## Sources` section after §13.

**Diagrams.** Flows are written as numbered steps or tables, not images. They stay readable
by agents and in diffs. A state machine also gets a Mermaid `stateDiagram-v2` under its
transitions table, so people see it at a glance (GitHub renders it). **The table is
authoritative.** The diagram is redrawn from it whenever the table changes.
