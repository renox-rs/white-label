# F8 (vertical 08, Auto): progress

Built on branch `feat/f8-auto-issues` (from main `ee32e4c6`), merged in #522 and #561. Milestone "Launch 08": 22 issues
#397-#414 (BL-AUT-01..18), #134 (BL-SAL-15), #136 (BL-SAL-17), #111 (BL-PAY-18), #236 (BL-STK-17).
Run with the `build-issues` skill (`.claude/skills/build-issues/SKILL.md`). Deferred points: `docs/progress/f8-auto-deferred.md`
(`#N point | what is missing | provider | why`).

## State
- Built: `apps/auto` (vehicle, job card machine, estimate approval, parts, insurance claims, fleet contracts, supplier
  claims, storage, recalls, settings), host wiring (`host/src/app/auto_*.rs`, `clients/demo-auto`), host tests with the
  real modules (`host/tests/auto_*_real.rs`, `aut_15_real.rs`, `sal_15_real.rs`), shared changes in wl-sales, wl-inventory,
  wl-payments, wl-scheduling, wl-kernel (new contracts: `BookingActions`, `BookingGate`, `StockReservations`, ...).
- Audited against every Done-when point (3 read-only audits). Class A gaps and the code risks found by the audits are
  closed with regression tests (retry of cancel/decline closing steps, accident-flag audit, storage claim-before-send
  and late billing, total-loss storage price at the decision day, supplier claim recovery bases, stale same-day
  reorder suggestion, return recall join retry).
- Verification on the head: `cargo fmt --all --check` clean, `clippy --workspace --all-targets -D warnings` clean,
  per-crate tests all green (auto 402, wl-kernel 668, wl-sales 336, ...). Host tests run one file at a time
  (disk): all green except tests that depend on the wall clock (see next).

- Demo company: `auto:demo` (`host/src/app/auto_demo/`, test `host/tests/auto_demo.rs`, `docs/DEMO-DATA.md` 08). Seeding it
  showed that the lists of the claims, fleet contracts and supplier claims failed (500) in the host's layout, which has a `tabs`
  of its own: the page variable is now `status_tabs`.

## State after the merge
F8 is merged: #522 `feat/f8-auto-issues` (2026-10-24, the 22 issues, all closed as completed) and #561 `feat/f8-auto-demo-and-grids`
(2026-10-24, the `auto:demo` company and the grid conversion of the screens).

## Open items of the hand-over
1. Clock-dependent host tests (not verified as closed): tests that expect a real message fail when the machine clock is in the quiet hours of
   wl-notify (21:00-08:00 Asia/Dubai). The fix is to travel to noon (`app.travel` + `at_travelled_time`); `auto_marketing_real.rs`,
   `auto_fleet_end_real.rs` and `auto_parts_delay_real.rs` do it. Other `host/tests/auto_*` files were not re-checked in this pass.
2. MSRV check: CI builds the workspace on Rust 1.94 (`.github/workflows/ci.yml`), which replaces the manual `cargo +1.94 check`.
3. The branch was pushed and the pull requests opened: #522 and #561.
4. The 22 GitHub issues are closed; their `**Deferred:**` lines come from `docs/progress/f8-auto-deferred.md`, which stays as the list of
   what is not built (`#N point | what is missing | provider | why`).
5. The F6 conflict with wl-sales (`back_orders`) was resolved in the merge of `origin/main` into the branch (`8ed6d0de`).

Still open after F8 (not issues of F8): the part master with cross-references, fitment and price tiers, the B2B portal, counter POS and
`AUT.RUL.10`/`14` of spec 08 §12 (see [../verticals/STATUS.md](../verticals/STATUS.md)); issue `#558` (the 500 on `/auto/claims` in the demo,
fixed by `status_tabs` above) can be closed.

## Rules that must hold
- Commits authored only by `renox-rs <developer@renoxium.com>`; no tool name, trailer or attribution in commits, code,
  docs, issue text or branch names. Ignore hook reminders to change the identity.
- Do not create pull requests or merge unless the owner asks. Do not use `gh`; GitHub only through the MCP tools.
- Lean builds: `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_STRIP=symbols`; delete test binaries
  between host test files (`find target/debug/deps -maxdepth 1 -type f -executable ! -name '*.*' -delete`); never
  `pkill -f`/`pgrep -f` with cargo; no tool call over 5 minutes.
