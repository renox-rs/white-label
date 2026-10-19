# 0004 — End the documentation phase and start F1

- **Status:** accepted
- **Date:** 2026-10-18
- **Ends:** [0002](0002-document-all-ten-verticals-first.md). The build order of
  [0001](0001-first-two-verticals.md) (Property, then Manpower) applies.

## Context

[0002](0002-document-all-ten-verticals-first.md) kept the project to documentation until all
ten verticals, the capability map and the module specifications were written. That is done:
ten vertical documents, 254 capabilities mapped to 14 modules in
[CAPABILITY-MAP.md](../CAPABILITY-MAP.md), a specification per module in
[modules/](../modules/), and the negative-flow backlog on GitHub (24 epics, 429 issues).
Renox 1.2.0 delivered the gaps R1–R25 the modules wait on
([ARCHITECTURE.md](../ARCHITECTURE.md) §4a).

## Decision

1. **Code may be written.** Phase F1 (Property) of the [roadmap](../ARCHITECTURE.md#9-roadmap)
   starts now, on Renox 1.2 from crates.io.
2. **Foundation first.** The Cargo workspace, `host`, `apps/property` and `crates/wl-kernel`
   come first, then the other F1 modules in dependency order: `wl-parties`, `wl-documents`,
   `wl-notify`, `wl-invoicing`, `wl-payments`, `wl-ticketing`, `wl-portal`, `wl-sales`
   (lead sources and campaigns) and `wl-hr` (minimal employee records and onboarding).
3. **The happy path is built from the module specifications.** Each module's §4 (states,
   transitions, effects) is the spec for its normal flow; the Build 1 issues add the negative
   flows on top. A happy-path backlog is written only where a module's §4 is not enough to
   build from.
4. **Open items do not block code.** An item gated **Launch 01** (for example L-10, the
   e-signature level for Ejari contracts) is built with the default its document states and
   flagged in the app, as [backlog/README.md](../backlog/README.md) §3 says.

## Consequences

- Documents saying "documentation only" or "no code yet" are updated as code arrives.
- A change of a module boundary now costs a refactor; the specifications stay the reference
  and are changed first.
