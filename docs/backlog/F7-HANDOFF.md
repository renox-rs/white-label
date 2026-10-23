# F7 (Manufacturing, Launch 07) build: hand-off notes

Branch `feat/f7-manufacturing-issues` (from `main`, main merged in after F6 and F10). 25 issues, 140 Done-when points.
Written so another agent can continue from the branch alone. Update this file when you change the state below.

## Issues in scope (GitHub numbers)

| Block | Issues |
|---|---|
| wl-inventory | BL-STK-21..25 = #240..#244 |
| wl-sales | BL-SAL-10..13 = #129..#132, BL-SAL-18 = #137 |
| manufacturing | BL-MFG-01..15 = #382..#396 (MFG-03 #384, 06 #387, 07 #388, 10 #391, 11 #392, 12 #393, 14 #395) |

Map of IDs to numbers: `docs/backlog/github-issues.md`. Issue text: `docs/backlog/verticals/07-manufacturing.md`,
`docs/backlog/modules/wl-sales.md`, `wl-inventory.md`.

## State

| Step | Status |
|---|---|
| Build of all 25 issues (code, tests, screens, demo client) | done |
| Merge of `origin/main` (F6, F10) and unification of wl-sales credit hold with F6 `hold`/`held_from` | done |
| Audit of all 140 points against code and tests (three groups) | done; Class A gaps closed |
| Full verification on the final head (after the polish commits): fmt clean, `clippy --workspace --all-targets -D warnings` clean, 23 packages 7,632 passed 0 failed, host in 11 chunks 778 passed, 6 failed | done; the 6 failures are `host/tests/edtech_demo.rs` (see below) |
| Browser check with Playwright of 46 pages at 1280 and 390 px, main flows | done; no errors, no overflow at 390 px |
| Polish round after the browser check (names instead of "Item 6", "None", icons, dates, duplicate ids) | done (`fix:` commits after `924a83b3`) |
| Re-run of all tests after the polish round | done (see above) |
| GitHub issues: tick Done-when, `**Deferred:**` lines, exactly one `**Verification summary**` comment per issue, then a sweep | TO DO (25 issues) |
| Pull request | not opened; open one only when asked |

## What was built (where to look)

- `apps/manufacturing`: work order (release, start, hold, resume, complete, close, cancel, unlink, abort with approval), BOM, material issue with lot check,
  production reports and void, hold for material, NCR lifecycle and dispositions, recall trace parties, conformity item flag, export evidence (MFG.RUL.08),
  customer PO upload in the portal, order changes and abort sheet, inspections of returned goods, aborted work orders report, settings pages, dashboard tiles.
  Its README holds a table of what is built and a "Deferred in F7" table.
- `crates/wl-sales`: customer credit account, credit hold and release (user and `_approved` twins), account hold, PO number uniqueness (also on amend),
  invoice on delivery, order directory (`order_directory.rs` provides the kernel `OrderLines`, `CustomerPoCheck`, `CreditExposure`, `CreditChecker`).
- `crates/wl-inventory`: Dispatch lifecycle (pick, send, deliver, loss, reverse, return, discard), dispatch guard (documents at pick and dispatch),
  BOM provider, production stock, lot disposal, recall trace with production lines and dispatch directory.
- `crates/wl-kernel`: new contracts with defaults (`production`, `lot_disposal`, `DispatchDirectory`, `TracePartyResolver::addressees`, `DispatchGuard`,
  `OutcomeProposals`, `CustomerPoCheck`, `OrderLineRules`, `PromisedLines`, `DeliveryDestination`, dispatch events, credit contracts).
- Host: `host/src/app/manufacturing_seed.rs` (07 configuration), `manufacturing_demo/`, `dashboard/manufacturing_tiles.rs`, roles in `host/src/app/roles.rs`.
- Demo: `clients/demo-manufacturing`, command `manufacturing:demo`, data described in `docs/DEMO-DATA.md`.

## Decisions to know

- One `hold` transition for sales orders (F6 `held_from`), release has three targets (`release`, `release_partly`, `release_pending`).
- The amendment re-check of F6 reads the limit through `CreditExposure`; overdue and account hold are checked at dispatch.
- Storekeeper does NOT hold `supplier_returns.manage` (base test `host/tests/inventory.rs` forbids it); NCR returns to supplier use a procurement user.
- `dispatch.cancel` needs a reason (F6 test).
- Dispatch migration `20261114000000_extend_wl_inventory_dispatches` alters F6's tables; F6's smaller dispatch was folded into the full lifecycle.

## Deferred (what stays open, provider, why)

| # | Issue, point | What is missing | Provider | Why not now |
|---|---|---|---|---|
| 1 | BL-MFG-11 (#392) pt 6: export-only item refused at sales submit/amend | wl-sales never calls `OrderLineRules`: an order has no delivery address (`DeliveryDestination` unwired). The rule works at pick and dispatch | wl-sales (delivery address with VAT category on the order) | no data |
| 2 | BL-MFG-11 pt 7 look-ahead / `PromisedLines` | a sale line has no promised day; tested with a fake | wl-sales | no data |
| 3 | `DeliveryDestination` | same as 1 | wl-sales | same |
| 4 | BL-MFG-02 (#383) pt 5 / `release_reservation` | wl-inventory keeps no reservation of an output lot for an order line; answers `false` | wl-inventory | no such reservation |
| 5 | BL-MFG-03 (#384): output "for stock" of a started work order | only flagged, never reserved or released | wl-inventory | same as 4 |
| 6 | BL-SAL-13 (#132) / BL-MFG-14: delivery after the invoice was issued | not billed (one invoice per order, accountant task) | wl-sales / client billing | open decision |
| 7 | BL-MFG-14 (#395) late entry for goods receipts and inspections | limits installed only for production reports | wl-inventory, this app | needs an approval flow in each |
| 8 | BL-MFG-14 calibration certificates kind | no kind, no equipment record | later issue | no entity |
| 9 | BL-MFG-14 onboarding `HRM.RUL.17` reminders, `HRM.ACL.10` | only checklist and SOP seeding is tested | wl-hr | owned by wl-hr |
| 10 | BL-MFG-14 SOPs `DOC.RUL.04` re-acknowledgement after 7 days | mechanism not run under the 07 seed | wl-documents | owned by wl-documents |
| 11 | BL-MFG-14 lead sources: campaign report, cost over budget wait | not run with the 07 seed | wl-sales campaigns | owned there |
| 12 | BL-MFG-14 signed checklists `WRK.CHK.01` (onboarding, returned goods inspection) | wl-ticketing has only the kernel contract `Checklists` | wl-ticketing | no provider |
| 13 | BL-MFG-09 (#390) scanner/import entry | no scanner or import entry in the app | later issue | nothing to test |
| 14 | BL-MFG-13 (#394) pt 3 | "no line selection for a price dispute" is a client-side show/hide; server ignores lines for a price dispute (tested) | none (by design) | not enforceable on the server |

Not proven end to end (say so in the issues): notice to a customer's main contact is proved as a queued message, not a delivered email; 09:00/09:15 timing of
MFG.RUL.08 and the 01:00 expiry look-ahead schedule are not asserted; supplier score, cancellation fee and drawing approval are proved at mechanism level;
390 px is a browser crawl, not an automated width test; Postgres migrations and `rnx db:check`/`view:check` were not run.

## Known failure that is not ours

`host/tests/edtech_demo.rs` (6 tests) panics at `host/src/app/edtech_demo/stories.rs:67` ("index out of bounds: the len is 2 but the index is 2") when the
Dubai date falls on a Saturday (it passed earlier the same day, before 20:00 UTC on a Friday). It fails identically on a clean `origin/main`, so it is a
date-dependent seed in the F10 edtech demo, not an F7 regression. CI running on a Friday after 20:00 UTC or on a Saturday will show it red. Fixing it belongs to F10
(make the batch start day independent of the weekday); until then say so on the PR.

## Rules of this project (do not break)

- Git identity `renox-rs <developer@renoxium.com>`. No claude/anthropic/Co-Authored-By or tool attribution in commits, branch names, code, docs, issues or PR text.
  A stop hook asks to re-author the commits as another identity: do not (it needs a rebase and a force-push).
- Commits `feat: ... (#N, part k/m)`, last commit of an issue `Fixes #N`, earlier ones `Refs #N`. Never force-push; merge `origin/main` into the branch before pushing.
- Push only after the previous CI run on the branch has finished (the workflow cancels running runs).
- GitHub issues: tick `- [x]` only DONE points (test `file::fn` as evidence), unticked points get `  **Deferred:** <what> (<provider>; <why not now>)`, exactly one comment
  starting with `**Verification summary**`, never close issues or touch labels/milestones/assignees, remove any footer the tool appends. Use the GitHub MCP tools (no `gh`).
- 390 px points are ticked only when the page is OK in the browser crawl; write the evidence as
  `structure asserted in <file>::<fn>; checked in a browser crawl at 390 px (not an automated width test)`.

## How to build and test (small disk)

```sh
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_STRIP=symbols
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --locked
# tests per package, deleting binaries after each (the disk cannot hold them all):
cargo test -p <package> --locked --no-fail-fast
find target/debug/deps -maxdepth 1 -type f -executable ! -name '*.*' -delete
# host tests in chunks of about 15 files: cargo test -p host --test <file> --test <file> ...
```

Never `pkill -f cargo` (it kills your own shell); use `pgrep -x cargo`. Do not run the whole workspace test at once.

## Next steps

1. `git fetch origin main`, merge if it moved, re-run the host tests that render pages changed by the polish round, then fmt and clippy.
2. Push the branch (check that no CI run is in progress on it first).
3. Update the 25 GitHub issues as described above (one agent per ~9 issues), then a sweep that re-reads every body and comment.
4. Open the pull request only when asked: title with scope and count, `Closes #N` per issue, verification totals, the deferred list above.
5. Possible follow-ups: wl-sales delivery address and promised day (closes deferred 1 to 3), output lot reservation in wl-inventory (4, 5).
