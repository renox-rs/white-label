# NN — <Vertical name>

> Status: draft | reviewed | accepted · Last updated: YYYY-MM-DD

## 1. Summary

Two to four sentences: who the customer is, what they struggle with today, what the app does
for them.

## 2. Business context (UAE)

- Market and typical customer size (staff, sites, volume).
- How they work today (Excel, WhatsApp, paper, legacy software).
- Rules and authorities that shape the work (mark uncertain facts `⚠ verify`).

## 3. Users and access

Follows [ACCESS-CONTROL.md](../ACCESS-CONTROL.md). Platform-wide rules (`PLT.ACL.*`) are not
repeated here.

### 3.1 Roles

Default roles shipped with this vertical.

| Role | Actor type (staff / field / external) | What they do | Permissions (summary) | Typical scope |
|---|---|---|---|---|

### 3.2 Scope

The scope unit for this vertical, and which roles are usually global or scoped.

### 3.3 Access rules

| ID | Rule | Pattern | Applies to | Parameter (default) |
|---|---|---|---|---|

*Pattern* is one of `OWN`, `ASG`, `STA`, `LIM`, `SOD`, `TIM`, `SEN`, `FLD`. Rules owned by a
shared module use that module's domain (e.g. `FIN.ACL.01`) and are marked *(shared)*.

## 4. Value streams

One subsection per end-to-end flow, as numbered steps. Each step lists the capability IDs it
uses.

### 4.1 <Flow name>

1. Step … (`XXX.YYY.NN`)

## 5. Capabilities

| ID | Capability | Shared or specific | Proposed module | Scope |
|---|---|---|---|---|

*Shared* = also needed by another vertical. *Scope* = `MVP` or `Later`.

## 6. Key entities

| Entity | Description | Important fields | Owned by |
|---|---|---|---|

## 7. Workflow

Follows [WORKFLOW.md](../WORKFLOW.md). An entity owned by a shared module is marked
*(shared — wl-…)* and links to its canonical definition in [modules/](../modules/); the
vertical lists only its requirements on it.

### 7.1 State machines

For each entity with a lifecycle: a **States** table, a **Transitions** table and a Mermaid
diagram drawn from it ([WORKFLOW.md](../WORKFLOW.md) §3.5).

### 7.2 Automations

| ID | When | If | Then | Parameters (default) | Default | Scope |
|---|---|---|---|---|---|---|

IDs use the area `RUL`: `PRP.RUL.01` for this vertical, `FIN.RUL.01` for a shared module.

## 8. Integrations

| System | Purpose | Required for MVP | Access / notes |
|---|---|---|---|

## 9. Compliance and data

Tax (VAT, corporate tax, e-invoicing), data protection (PDPL), sector rules, data residency,
retention.

## 10. Reports and KPIs

What the owner or manager looks at every week.

## 11. Platform fit

- What Renox already provides for this vertical.
- What the shared `wl-*` modules provide.
- What is specific to this vertical (`apps/<name>`).
- Estimated reuse: share of capabilities covered by shared modules.

## 12. MVP scope

What the first sellable version includes and explicitly excludes.

## 13. Open questions

Numbered. Each one names who can answer it (client, accountant, authority, us).
