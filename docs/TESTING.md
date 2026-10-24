# Testing

> Status: §0 describes the practice as built (F1-F10); the rest is the target where marked · Last updated: 2026-10-24

How the platform is tested, derived from the documentation. The documents already define
the behaviour precisely, with IDs: capabilities, access rules, automations and state machine
transitions. Every one of them becomes at least one test, so a reviewer can check coverage by
ID instead of by reading code.

## 0. How tests are written and run today (F1-F10)

- **Where:** integration tests in `crates/<crate>/tests/*.rs`, `apps/<app>/tests/*.rs` (every app has them) and
  `host/tests/*.rs`, each crate with a `tests/support` module that boots a `TestApp` with the
  modules it needs (fakes for the other modules' kernel contracts). Tests named `*_real.rs`
  and those in `host/tests` boot the real modules together; some apps add `*_demo.rs` and
  `*_screens.rs` files (auto, manufacturing, manpower) for the demo data and the rendered screens. Unit tests sit next to the code in
  `#[cfg(test)]` modules.
- **Named after the backlog issue, not the capability ID.** Each test file starts with a
  `//!` comment naming the issue it covers (`BL-PRP-02 (#252)`) and the rule or capability IDs
  (`PRP.RUL.12`); 891 of 894 top-level test files do (183 in `host/tests`, 711 in `crates/*/tests` and `apps/*/tests`; counted 2026-10-24, the number dates fast). Each test's `///` comment names the Done-when
  point it checks (`AC1:`, `AC2:` …), and the function name says the behaviour in words
  (`a_second_adjustment_waits_until_the_first_is_settled`). Some files prefix the functions
  with the issue (`bl_prp_24_...`); a few with a capability or rule ID (`prp_lse_01_...`, `prp_acl_04_...`).
- **Run:** `cargo test -p <crate> --test <file>` while working, `cargo test --workspace --locked`
  before a pull request.
- **CI** (`.github/workflows/ci.yml`, on every pull request and push to `main`):
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked`,
  `cargo test --workspace --locked` (all with `RUSTFLAGS=-D warnings`, a lean dev profile
  `CARGO_PROFILE_DEV_DEBUG=0` and free runner disk first), and in the `msrv` job `cargo check
  --workspace --locked` on Rust 1.94 (the minimum version). `release.yml` (tags) and
  `release-main.yml` (every push to main) build binaries; they are not test workflows.
- **Not built yet:** the ID coverage report (§1), generated state machine conformance
  matrices (§3), browser, RTL, dark-mode and PDF comparison checks (§8), `cargo deny`, and the
  `rnx db:check` / `rnx view:check` steps in CI. They are the target below.

## 1. Traceability (target)

| Documented item | Where it is defined | Test it becomes | Named after |
|---|---|---|---|
| Capability `DOMAIN.AREA.NN` | Vertical §5, module §2 | Feature tests for its main flow | the capability ID |
| Access rule `*.ACL.NN` | Vertical §3.3, module §6, [ACCESS-CONTROL.md](ACCESS-CONTROL.md) §9 | Allowed and denied cases per pattern (§4) | the rule ID |
| Automation `*.RUL.NN` | Vertical §7.2, module §6, [WORKFLOW.md](WORKFLOW.md) §7 | Trigger, condition, action, idempotency (§5) | the rule ID |
| State machine transition | Module §4, vertical §7.1 | Conformance tests (§3) | entity + transition |
| Value stream | Vertical §4 | Acceptance scenario (§7) | vertical + stream number |
| Platform rule `PLT.ACL.*` | [ACCESS-CONTROL.md](ACCESS-CONTROL.md) §8 | Platform-wide tests run for every module | the rule ID |

**Coverage report** (target, not built). A check lists every ID found in the documents and
every ID found in tests (names or the file comment, §0), and reports IDs without tests. Once it
exists, a module is not done while its IDs are uncovered (§9). Until then, coverage is checked
per issue against its Done-when points (the `AC` comments).

## 2. Tools

All tests use Renox's test helpers (`renox/docs/testing.md`):

| Need | Renox helper |
|---|---|
| Boot the app, send requests, check responses | `TestApp`, `get`/`post`, `assert_ok`, `assert_forbidden`, `assert_invalid`, `assert_see` |
| Log in as a role in a scope | `acting_as`, plus permission scope helpers |
| Time-based rules (reminders, expiry, SLA, overdue) | `travel` (moves the clock for the database, sessions, signed URLs, queue and scheduler), `run_scheduled` |
| Jobs, events, notifications, mail | `run_jobs`, `run_all_jobs`, `fake_events` + `assert_emitted`, `fake_notifications` + `assert_notified`, `sent_mail` |
| External services (gateways, BSPs, accounting exports) | `fake_http` with `assert_sent`; nothing reaches the network |
| Data | Factories per model; demo datasets as fixtures (§6) |
| Schema and templates | `assert_models_match_schema`, `assert_views_compile`, `rnx db:check`, `rnx view:check` |
| Browser behaviour | `app.serve()` + the CDP harness from `renox/tests/browser` |

## 3. State machine conformance

Each state machine is documented as a States table and a Transitions table
([WORKFLOW.md](WORKFLOW.md) §3.5). The tests are derived from those tables:

1. **Every allowed transition** succeeds from every listed *From* state for a user with the
   *By* permission, and lands in *To*.
2. **Every other pair** (state, transition) is refused. The matrix is complete: states ×
   transitions minus the allowed cells.
3. **Permission:** a user without the *By* permission is refused; `system` transitions cannot
   be run by any user.
4. **Guards:** each guard is tested failing on its own, with the error shown.
5. **Effects:** each effect is observed: events emitted, documents generated, other
   transitions run, notifications queued after commit.
6. **History:** every successful transition writes one history row with actor, from, to and
   reason.
7. **Atomicity:** if an effect that writes data fails, the status and history are unchanged.
8. **Available actions:** for each state and role, the listed actions equal the allowed
   transitions.

Because the tables follow one format, the expected matrix can be produced from the module
document. When a table changes, the conformance tests change with it, and a failing test
shows which row of the document disagrees with the code.

## 4. Access rules by pattern

| Pattern | Minimum tests |
|---|---|
| `OWN` | Owner allowed; another party of the same type refused; list shows only own rows; direct URL to another's record refused (not just hidden) |
| `ASG` | Assignee allowed; another field user refused; reassignment moves access |
| `STA` | Action allowed in the listed statuses; refused in every other status |
| `LIM` | Below the limit: done directly. At and above: an approval request is created, not a refusal. Limit changed in settings: behaviour follows |
| `SOD` | Same user creating and approving refused; different user allowed; switching off (where configurable) allows it |
| `TIM` | Before, during and after the window, using `travel` |
| `SEN` | Without the permission: field or file hidden; with it: visible and one audit row per view |
| `FLD` | Field absent from page, export and API for the restricted role |
| Scope | Role in scope A allowed in A, refused in B; dated assignment allowed only within its dates |

Platform rules `PLT.ACL.01`–`08` run against every module: for example `PLT.ACL.01` checks
that every route in the module refuses an anonymous user and a user without roles.

## 5. Automations

For each `*.RUL.NN`:

1. **Fires:** with the trigger met and the condition true, the action happens (travel to the
   date, run the scheduler).
2. **Does not fire** when the condition is false or the rule is switched off.
3. **Idempotent:** running the scheduler twice in the same period produces one action
   (WORKFLOW §4.3).
4. **Parameters:** a changed parameter (days, time, percentage) changes the result.
5. **Chains stop:** a rule chain deeper than 3 is stopped and reported.
6. **Failure:** a failing action retries, is logged, and appears in the `PLT.RUL.03` digest.

## 6. Data for tests

- **Factories** for every model, producing valid records with sensible defaults.
- **Demo datasets** ([DEMO-DATA.md](DEMO-DATA.md)) are loaded in acceptance tests, so the
  scenarios run on the same data the sales demo shows. A demo that breaks fails the build: the demo commands are run in tests (`host/tests/demo_seed.rs` for `property:demo`, `*_demo.rs` for construction, clinics, manpower, manufacturing, edtech, auto and accounting, `logistics.rs` and `fnb.rs` for those two).
- **No production data** in tests, ever.
- **Edge data:** Arabic names, mixed scripts, very long names, zero-rated and exempt VAT lines,
  amounts with fils, dates around month end, Ramadan hours, public holidays.

## 7. Acceptance scenarios per vertical

Each value stream in a vertical's §4 becomes one end-to-end scenario on the demo dataset,
written as the numbered steps of the document. Example for Property 4.1 *Lease to cash*:

1. A unit is available; a lease is drafted, submitted, signed and activated.
2. The charge schedule is created; the first invoice is issued with exempt VAT.
3. Cheques are logged; one is deposited and cleared, one bounces.
4. Travel past the due date: reminders are sent, the bounced cheque creates a task.
5. The portal shows the tenant exactly their lease, invoices and cheques.

Scenarios are reviewed with the vertical document: a step in the document without a step in
the scenario is a gap.

## 8. Interface checks

- **Browser tests** for every UI change that htmx or Alpine drives: sheets, live validation,
  wizards, signature pad, camera capture, geolocation (mocked), offline banners.
- **RTL:** every page is checked with `dir="rtl"` and Arabic content, layout without overlap or
  clipped text.
- **Dark mode:** every page in light and dark.
- **Phone width:** PWA screens at 360 px.
- **Accessibility:** focus order, visible focus ring, labels, 44 px targets, contrast of
  client brand colours (the R9 contrast guard).
- **Documents:** generated PDFs in English and Arabic compared against reference files
  (text and layout regions).

## 9. Definition of Done for a module (target)

A module is done when (the items that need a tool not built yet, §0, are checked by review
until it exists):

- [ ] Every capability, access rule, automation and transition in its document has tests
      named by ID, and the coverage report shows no gaps.
- [ ] State machine conformance tests pass for every machine in §4 of the module.
- [ ] Platform rules `PLT.ACL.*` pass for every route of the module.
- [ ] It works with each vertical listed in its "Used by" column, using that vertical's demo
      dataset.
- [ ] RTL, dark mode, phone width and browser tests pass for its pages.
- [ ] `rnx db:check`, `rnx view:check`, `cargo deny`, formatting and lints pass.
- [ ] Its module document is updated to match what was built, and every deviation is
      recorded in the document's open questions or a decision record.
- [ ] Migrations run forward on a copy of the previous release's schema.

## 10. Open questions

1. Should the coverage report run in CI and fail the build on uncovered IDs from the start, or
   only once a module is marked accepted? *(us)*
2. Which CDP client do we standardise on for browser tests in the white-label repository?
   *(us)*
