# F5 (vertical 05, Clinics): progress

Milestone "Launch 05": 52 issues. Merged 2026-10-22.

## Pull requests
- #499 `feat/f5-clinics-issues` (the app and `wl-scheduling`), #514 (demo data and screen fixes), #515 (staff screens).

## Built
- `apps/clinics`: Prior authorisation and Claim machines (technical rejection, recoupment, late closing, void), the Visit opened by check-in, RecallNotice, practitioner licences as booking guards, recalls traced to patients, remittance recording per claim ([apps/clinics/README.md](../../apps/clinics/README.md)).
- `crates/wl-scheduling` (bookings, resources, check-in guard).
- Demo: `clients/demo-clinics`, command `clinics:demo` ([DEMO-DATA.md](../DEMO-DATA.md) 05).

## Known gaps
- The spec §7.1 has Appointment and Package only (both on shared modules); the app's `Visit` and `RecallNotice` machines are not in the spec.
- Not built (spec §12): patient portal (only a card-request route exists), patient feedback after a visit, waiting list, bank-side matching of remittances.
