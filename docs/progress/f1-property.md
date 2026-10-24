# F1 (vertical 01, Property) and the shared base: progress

Milestone "Build 1": 132 issues (the shared modules every vertical uses). Merged 2026-10-18 to 2026-10-21.

## Pull requests
- #455 brought the shared crates (`wl-kernel`, `wl-parties`, `wl-documents`, `wl-notify`, `wl-invoicing`, `wl-ticketing`, `wl-payments`, `wl-sales`) and `apps/property`.
- #456 (Build 1 issues, `wl-hr`, `wl-portal`), #481 (docs aligned after F1), #482 (demo and deploy).

## Built
- `apps/property`: Unit, Lease, Management agreement and Owner statement machines, charge schedule, holding over, notice and termination, deposit settlement, owner statements with payout approval ([apps/property/README.md](../../apps/property/README.md)).
- Shared modules for parties and KYC, documents with expiry, notices, invoices, cheques, deposits, tickets, sales enquiries, staff and portal sections, bank statement import (CSV, MT940), cash-flow forecast, budget vs actual.
- Demo: `clients/demo-properties`, command `property:demo` ([DEMO-DATA.md](../DEMO-DATA.md) 01); public sign-up closed, admins invite users.

## Known gaps
- Spec MVP items not built: tenant portal, rent roll report (`PRP.RPT.01`), EN/AR contract PDF, Excel migration (`PLT.IMP.01`).
- Open Build 1 follow-ups `#458`-`#479` (22 issues: BL-KRN-10/11, PTY-11, POR-02/03, DOC-11, NTF-10, INV-31, PAY-27..30, SAL-27/28, TKT-12..14, PRP-30..34) have no implementing commit.
- Open leftovers with a `**Deferred:**` line: `#207` (BL-HRM-17), `#213` (BL-HRM-23), `#278`, `#279` (BL-PRP-28/29).
