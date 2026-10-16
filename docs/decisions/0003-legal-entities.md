# 0003 — Several legal entities in one client deployment

- **Status:** accepted
- **Date:** 2026-10-14
- **Related:** [wl-kernel](../modules/wl-kernel.md), [wl-invoicing](../modules/wl-invoicing.md),
  [ACCESS-CONTROL.md](../ACCESS-CONTROL.md) §3

## Context

One deployment serves one client ([ARCHITECTURE.md](../ARCHITECTURE.md) §1). In the UAE, one
client is often a **group** of legal entities: for example a mainland LLC and a free-zone
company, or one company per brand or emirate. Each entity has its own trade licence, its
own TRN (or none, if it is not VAT-registered), its own bank accounts and its own invoice
numbering. Until now the platform assumed one issuer per deployment, taken from the
settings.

## Decision

1. **A legal entity is a first-class kernel concept** (`PLT.ENT.01`, `wl-kernel`): legal
   names in English and Arabic, trade licence, licensing authority (mainland emirate or free
   zone), TRN and VAT registration status, address, bank accounts, logo and document footer.
2. **Every financial or legal document has an issuing entity:** quotations, invoices, credit
   notes, receipts, supplier bills, payslips and contracts. Its TRN, legal name and bank
   details are copied onto the document when it is issued and never change afterwards.
3. **Sequences are per entity and per document type** (`INV-ACME-2026-0001`).
4. **A deployment always has at least one entity.** A single-entity client never sees the
   choice; the entity selector appears only when there is a second one.
5. **Entities are not a security boundary by themselves.** Access is still scoped by the
   vertical's scope unit (building, project, branch…). Each scope belongs to exactly one
   entity, so reports and numbering follow from it. A client that needs entity-level access
   control assigns roles in an entity scope.
6. **Intercompany transactions** (one entity invoicing another) are recorded as ordinary
   invoices between parties that represent the entities. Consolidation and elimination stay
   in the accounting software.

## Why

- A tax invoice must carry the correct supplier TRN. Retrofitting an issuer onto existing
  invoices later is a data migration on legal records.
- Mainland and free-zone entities are common among the target SMEs (manpower agencies,
  F&B groups, trading companies).
- Designing it in from day one costs one foreign key per document. A single-entity client
  pays nothing in complexity.

## Consequences

- `wl-kernel` owns the Legal entity and per-entity sequences.
- `wl-invoicing`, `wl-payments`, `wl-hr` and `wl-documents` copy issuer details at issue time.
- Reports (VAT summary `FIN.VAT.01`, receivables, dashboards) can be filtered by entity.
- `wl-operator` counts entities per client for pricing.
