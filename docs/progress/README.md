# Build progress

One note per phase of the build (F1-F10, [decision 0005](../decisions/0005-phases-follow-vertical-numbers.md)). A note says what was built,
in which pull request, and what is known to be missing. The status of the ten verticals side by side is in
[../verticals/STATUS.md](../verticals/STATUS.md); the issue texts are in [../backlog/](../backlog/README.md); what a demo company contains is in
[../DEMO-DATA.md](../DEMO-DATA.md). The state of every Done-when point lives in the GitHub issue (ticked boxes, `**Deferred:**` lines and one
`**Verification summary**` comment).

All phases F1-F10 are merged (the merge dates are from `git log --merges`).

| Phase | Vertical | Pull requests | Merged | Notes |
|---|---|---|---|---|
| F1 | 01 Property, with the shared base | #455, #456, #481, #482 | 2026-10-19 to 2026-10-21 | [f1-property.md](f1-property.md) |
| F2 | 02 Construction | #486, #487, #488 (and #457) | 2026-10-21 | [f2-construction.md](f2-construction.md) |
| F3 | 03 Logistics | #498 | 2026-10-22 | [f3-logistics.md](f3-logistics.md) |
| F4 | 04 Accounting | #484, #485, #497 | 2026-10-21 | [f4-accounting.md](f4-accounting.md) |
| F5 | 05 Clinics | #499, #514, #515 | 2026-10-22 | [f5-clinics.md](f5-clinics.md) |
| F6 | 06 F&B | #520 | 2026-10-23 | [f6-fnb.md](f6-fnb.md) |
| F7 | 07 Manufacturing | #521 | 2026-10-24 | [f7-manufacturing.md](f7-manufacturing.md) (with the deferred table) |
| F8 | 08 Auto | #522, #561 | 2026-10-24 | [f8-auto.md](f8-auto.md), deferred points: [f8-auto-deferred.md](f8-auto-deferred.md) |
| F9 | 09 Manpower | #500, #501, #516, #519 | 2026-10-22 to 2026-10-23 | [f9-manpower.md](f9-manpower.md) |
| F10 | 10 EdTech | #517 | 2026-10-23 | [f10-edtech.md](f10-edtech.md) |

## Issues still open

- **Build 1 follow-ups `#458`-`#479`** (22 issues, the work the closed F1 issues deferred): no implementing commit yet.
- **Search and agents `BL-AGT-01`..`13`, `#541`-`#554`**: nothing built (decision 0006).
- Issues with a `**Deferred:**` line, built in their phase and kept open: `#207`, `#213` (wl-hr), `#278`, `#279` (property), `#298` (construction),
  `#346` (accounting), `#434`, `#435` and the screens `#502`, `#503`, `#505`-`#507`, `#509`, `#510`, `#513` (manpower).
- The standard-grid issues `#523`-`#538`, `#556`, `#557` are not backlog issues; they track the conversion of the lists to the standard grid.
