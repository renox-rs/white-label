# F10 (vertical 10, EdTech): progress

Milestone "Launch 10": 18 issues. Merged 2026-10-23.

## Pull requests
- #517 `feat/f10-edtech-issues`.

## Built
- `apps/edtech`: Batch, Enrolment, Session and Certificate machines; seats held 48 hours, withdrawal with a tiered refund that needs a second person, freeze and unfreeze, transfer, absence policy and credits, attendance, certificates with public verification ([apps/edtech/README.md](../../apps/edtech/README.md)).
- Host wiring: `edtech_seed.rs`, `edtech_settings.rs`, `edtech_money.rs`; tests `host/tests/edtech_wiring.rs`, `edtech_settings.rs`, `edtech_money.rs`.
- Demo: `clients/demo-edtech`, command `edtech:demo` ([DEMO-DATA.md](../DEMO-DATA.md) 10). The demo counts its dates from the latest Friday.

## Known gaps
- Not built (README): leads and trial classes, e-signature of terms and guardian consent (`DOC.SIG.01`), certificate QR image and PDF (`DOC.GEN.01`), `EDU.RUL.01`, `05`, `08`, `09`, tutor timesheets (`EDU.RUL.10`), make-up booking, student and guardian portal screens, loyalty segments.
- Submission machine, materials, assessments and gradebook: Later (spec 10 §7.1 and §12 reconciled on 2026-10-24).
- The deferred points of each issue are the `**Deferred:**` lines on the GitHub issues; there is no separate deferred file for F10.
