# 0005 — Build phases follow the vertical numbers

- **Status:** accepted
- **Date:** 2026-10-20
- **Supersedes:** the build order of [0001](0001-first-two-verticals.md) (Property, then
  Manpower) and the roadmap rows F2–F4+ of [ARCHITECTURE.md](../ARCHITECTURE.md) §9.

## Context

[README.md](../../README.md) and [README.md](../README.md) list the ten verticals as 01–10, all
`accepted`. The roadmap still named phase F2 "Manpower" (vertical 09) and left F3 for fleet
rollout, so the phase names and the vertical numbers disagreed.

## Decision

1. **Phase F*n* builds vertical *n*** for n = 1…10: F1 Property, F2 Construction, F3 Logistics,
   F4 Accounting, F5 Clinics, F6 F&B, F7 Manufacturing, F8 Auto, F9 Manpower, F10 EdTech.
2. **F0 stays Renox foundations.** The former F3 (fleet rollout) is no longer a phase number.
   It is the **rollout track**: it starts after F1 and is finished before the second paying
   client goes live.
3. **Each phase builds only the modules its vertical needs first**, and measures reuse of the
   modules built before it. Which module comes with which phase is in
   [ARCHITECTURE.md](../ARCHITECTURE.md) §9.
4. **A real prospect still overrides the order** (the override condition of 0001); record it
   in a new decision.

## Consequences

- `wl-attendance` and `wl-inventory` are built in F2 (Construction), not for Manpower.
  The rest of `wl-hr` (visas, leave, payroll, WPS, final settlement) moves to F9.
- **Build 1** now means phase F1 only. The 21 issues of 09 Manpower and the module issues only
  09 needs (44 in all) moved to the milestone **Launch 09**; 11 issues first needed by 02
  (attendance flags and adjustment timesheets, qualification checks, absconding report, the
  adjustment billing) moved to **Launch 02**, and one to **Launch 05**. Each issue's phase is the
  earliest vertical whose issues depend on it, directly or through `Depends on`.
