# 0002 — Document all ten verticals before writing any code

- **Status:** accepted
- **Date:** 2026-10-14
- **Amends:** [0001](0001-first-two-verticals.md). Its build order (Property, then Manpower)
  still applies once development starts.

## Context

[0001](0001-first-two-verticals.md) picked two verticals so that the shared modules would be
discovered from real code. Doing that means the module boundaries are drawn from only two
businesses. The other eight would test the boundaries only after the code exists, which is
when changing them is expensive.

## Decision

The current phase is **documentation only**:

- All ten verticals are documented at business level: users, end-to-end flows, capabilities,
  data, integrations, regulations, MVP scope.
- Every capability gets a stable ID and is mapped to a module in
  [CAPABILITY-MAP.md](../CAPABILITY-MAP.md).
- Each module is specified in `docs/modules/`, with what Renox already provides and what is
  missing.
- **No code is written in this phase.** That includes scaffolds, crates and development
  issues.

Order of work:

1. Write templates and one complete sample vertical (Property), then review them.
2. Document the other nine verticals one by one, using the same template.
3. Collect the capabilities into the capability map, then derive the module specifications
   from it.

## Why

- **Helicopter view.** With all ten verticals on paper, shared and specific capabilities are
  visible at once. Module boundaries are drawn from ten businesses, not two.
- **Cheap to change.** Moving a capability between modules is an edit in a table, not a
  refactor.
- **Sales asset.** For each prospect, the documents show what the platform already covers
  for their sector and what is left to build.
- **Agent-ready.** Uniform templates, stable IDs and tables let a coding agent implement a
  module later from its specification alone.

## Consequences

- [ARCHITECTURE.md](../ARCHITECTURE.md) §6 (module × app matrix) is a first guess. It is
  replaced by [CAPABILITY-MAP.md](../CAPABILITY-MAP.md) once the map exists.
- The roadmap phases F0–F4 start only after this phase is done and the module list is
  approved.
