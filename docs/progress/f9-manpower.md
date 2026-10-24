# F9 (vertical 09, Manpower): progress

Milestone "Launch 09": 61 issues, the largest phase after Build 1. Merged 2026-10-22 and 2026-10-23.

## Pull requests
- #500 `feat/F9-manpower` (first merge), #501 (follow-up), #516 (screens), #519 (wording of the screen issues).

## Built
- `apps/manpower`: Client contract, Manpower request and Deployment machines, cover requests, document blocks and suspension, exits (absconding, death, notice withdrawn), blocked client company ([apps/manpower/README.md](../../apps/manpower/README.md)).
- Rate cards and usage billing in `wl-invoicing`, leave, payroll, WPS and final settlements in `wl-hr`.
- Demo: `clients/demo-manpower`, command `manpower:demo` ([DEMO-DATA.md](../DEMO-DATA.md) 09).

## Known gaps
- Worker portal with payslips and leave (`POR.SLF.02`) is not built; the portal routes are for the client approver.
- The candidate pipeline (`MPW.REC.01`) is Later as written.
- Open on GitHub: `#434`, `#435` (BL-MPW-20/21, deferred points) and screens `#502`, `#503`, `#505`-`#507`, `#509`, `#510`; `#513` (BL-MPW-33, Arabic translation) is created and not built.
