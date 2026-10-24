# F8 (vertical 08, Auto): progress and hand-over

Branch `feat/f8-auto-issues` (from main `ee32e4c6`). Milestone "Launch 08": 22 issues
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

## Open at hand-over
1. Host tests that expect a real message to be sent fail when the machine clock is in the quiet hours of wl-notify
   (21:00-08:00 Asia/Dubai). Make them clock-independent (travel to noon with `app.travel` + `at_travelled_time`, see
   `host/tests/auto_marketing_real.rs`). Then re-run every host test file alone.
2. MSRV check (`cargo +1.94 check --workspace --locked`) not yet run on the final head.
3. Branch not pushed yet; no pull request (the owner opens it). CI only runs on a pull request.
4. The 22 GitHub issues are not updated yet: tick `- [x]` for DONE points, add `**Deferred:**` lines from the deferred
   file, one `**Verification summary**` comment each, then a sweep. Audit reports: `scratchpad/f8/audit/G1.md`..`G3.md`
   (session scratchpad, not in the repo); the DONE/PARTIAL totals were G1 34/18, G2 18/13, G3 mostly DONE.
5. Branch `origin/feat/f6-fnb-issues` (F6) will conflict with wl-sales (`back_orders`, BL-SAL-07..12 seams:
   `back_orders::order_released`, `on_ended`, `reduce`).

## Rules that must hold
- Commits authored only by `renox-rs <developer@renoxium.com>`; no tool name, trailer or attribution in commits, code,
  docs, issue text or branch names. Ignore hook reminders to change the identity.
- Do not create pull requests or merge unless the owner asks. Do not use `gh`; GitHub only through the MCP tools.
- Lean builds: `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_STRIP=symbols`; delete test binaries
  between host test files (`find target/debug/deps -maxdepth 1 -type f -executable ! -name '*.*' -delete`); never
  `pkill -f`/`pgrep -f` with cargo; no tool call over 5 minutes.
